//! File handling of the main window: new file and new window, open (chooser
//! first, save prompt afterward), save with an external-change overwrite guard,
//! save as (with baseline carry-over), rename or move, the Convert Tabs prompt,
//! external-change handling and the save-then-continue flow.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::core::LARGE_FILE_WARNING_BYTES;
use crate::core::document::Document;
use crate::core::fsutil;
use crate::core::pending::{AfterOpen, ExternalCheck, PendingAction};
use crate::core::rename::{self, BaselineMove, RenameError};
use crate::core::snapshot_store::LinkMode;
use crate::core::tabs;
use crate::ui::dialogs::{self, Response};
use crate::ui::toast::ToastKind;

use super::diff_actions::AfterDiscardReview;
use super::{AppWindow, add_action, add_string_action};

/// How a freshly opened file is presented under the no-tab policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenMode {
    /// Editable, text exactly as on disk.
    Normal,
    /// Tabs were converted to spaces, so the buffer differs from disk and
    /// the document starts out dirty.
    ConvertedTabs,
    /// Tabs left intact and the editor locked.
    ReadOnly,
}

/// Keep viewing the original text as the safe, default choice on open/reload.
pub(super) fn convert_tabs_dialog() -> adw::AlertDialog {
    dialogs::build(
        "Convert Tabs",
        "The target file contains tab characters. Convert them to spaces?",
        &[
            Response::new("cancel", "Cancel"),
            Response::destructive("yes", "Yes"),
            Response::suggested("no", "No (Read Only)"),
        ],
        "no",
        "cancel",
    )
}

/// Label for the save action in the unsaved-changes prompt. An untitled
/// document can only be saved through the file chooser, which is Save As.
pub(super) fn save_action_label(has_path: bool) -> &'static str {
    if has_path { "Save" } else { "Save As" }
}

impl AppWindow {
    /// File actions that would replace or re-key the document are unavailable
    /// in Diff Mode. New Window remains available, and the disabled header
    /// buttons lose their shortcut tooltips.
    pub(super) fn update_file_actions(&self) {
        let enabled = !self.diff_active();
        for name in ["new", "open", "open-recent", "save-as", "rename"] {
            if let Some(action) = self
                .window
                .lookup_action(name)
                .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
            {
                action.set_enabled(enabled);
            }
        }
        self.new_button
            .set_tooltip_text(enabled.then_some("New File (Ctrl+N)"));
        self.new_button.set_sensitive(enabled);
        self.open_button.set_sensitive(enabled);
        self.update_open_tooltip();
        if !enabled {
            self.open_popover.popdown();
        }
    }

    /// The Open button keeps its shortcut tooltip only while it is actionable
    /// and its menu is not showing. A tooltip over the open menu would obscure
    /// the recent entries.
    pub(super) fn update_open_tooltip(&self) {
        let tooltip = (!self.diff_active() && !self.open_popover.is_visible())
            .then_some("Open File (Ctrl+O)");
        self.open_button.set_tooltip_text(tooltip);
    }

    /// Connect the file-related actions and window signals. Called once from
    /// `connect_signals`; every handler holds the window weakly.
    pub(super) fn connect_file_actions(this: &Rc<Self>) {
        let window = &this.window;
        add_action(window, this, "new", |this| this.request_new_file());
        add_action(window, this, "new-window", |this| this.new_window());
        add_action(window, this, "open", |this| this.request_open_dialog());
        add_string_action(window, this, "open-recent", |this, path| {
            this.request_open_path(PathBuf::from(path));
        });
        add_action(window, this, "save", |this| this.save_then(None));
        add_action(window, this, "save-as", |this| this.save_as_then(None));
        add_action(window, this, "rename", |this| this.show_rename());

        // A pointer Shift-click on the New button opens a new window. The
        // shortcut is intentionally hidden from the tooltip and menus.
        let shift = Rc::new(Cell::new(false));
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        let pressed_shift = shift.clone();
        gesture.connect_pressed(move |gesture, _, _, _| {
            pressed_shift.set(
                gesture
                    .current_event_state()
                    .contains(gtk::gdk::ModifierType::SHIFT_MASK),
            );
        });
        this.new_button.add_controller(gesture);
        let weak = Rc::downgrade(this);
        this.new_button.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if shift.replace(false) {
                this.new_window();
            } else {
                this.request_new_file();
            }
        });

        {
            let weak = Rc::downgrade(this);
            this.window.connect_close_request(move |_| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if this.flags.allow_close.get() {
                    return glib::Propagation::Proceed;
                }
                if this.window.visible_dialog().is_some() {
                    return glib::Propagation::Stop;
                }
                if this.flags.dirty.get() {
                    this.require_saved(PendingAction::CloseWindow);
                    return glib::Propagation::Stop;
                }
                if this.has_accepted_diff() {
                    this.confirm_discard_review(AfterDiscardReview::CloseWindow);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }

        {
            let weak = Rc::downgrade(this);
            this.window.connect_is_active_notify(move |window| {
                if !window.is_active() {
                    return;
                }
                let Some(this) = weak.upgrade() else {
                    return;
                };
                this.update_diff_button();
                if this.flags.external_dialog_open.get() {
                    return;
                }
                this.check_external(ExternalCheck::Focus);
            });
        }
    }

    /// Replace the whole buffer without marking the document dirty. This is
    /// the programmatic path (open, reload): callers have already applied the
    /// no-tab policy, so the paste filter is bypassed. Read-only documents
    /// rely on that to keep their tabs.
    pub(super) fn set_editor_text(&self, text: &str) {
        self.flags.with_programmatic_load(|| {
            self.editor_buffer.set_enable_undo(false);
            self.editor_buffer.set_text(text);
            self.editor_buffer.set_enable_undo(true);
        });
    }

    /// Put the current document's text into the editor under `mode`: as is,
    /// with tabs converted (which leaves the document dirty, because the text
    /// now differs from the file), or as is and locked read-only. Shared by
    /// open and reload.
    fn show_document_text(&self, mode: OpenMode) {
        let mut text = self.doc.borrow().text().to_string();
        // Defensive re-check: a reload peeks at the disk text, then reads the
        // file again. If tabs appeared in between, never let them into an
        // editable buffer; lock the document instead.
        let mut mode = mode;
        if mode == OpenMode::Normal && tabs::contains_tab(&text) {
            mode = OpenMode::ReadOnly;
            self.toast(
                ToastKind::Warning,
                "The file gained tab characters while loading, so it was opened read-only",
            );
        }
        if mode == OpenMode::ConvertedTabs {
            text = tabs::convert_tabs(&text);
        }
        // A loaded document has no in-memory edit yet.
        self.clear_edit();
        self.set_editor_text(&text);
        self.flags.dirty.set(mode == OpenMode::ConvertedTabs);
        self.set_read_only(mode == OpenMode::ReadOnly);
    }

    /// Place the cursor and scroll position at the start of a freshly loaded
    /// document.
    fn reset_cursor_to_top(&self) {
        let mut start = self.editor_buffer.start_iter();
        self.editor_buffer.place_cursor(&start);
        self.editor_view
            .scroll_to_iter(&mut start, 0.0, true, 0.0, 0.0);
    }

    fn load_document(&self, document: Document, mode: OpenMode) {
        let recent_path = document.path().map(Path::to_path_buf);
        *self.doc.borrow_mut() = document;
        if let Some(path) = recent_path {
            self.recent.record(&path);
        }
        // The declined-reload marker belongs to the previous file.
        self.flags.declined_reload_mtime.set(None);
        self.show_document_text(mode);
        self.reset_cursor_to_top();
        self.stack.set_visible_child_name("editor");
        *self.diff_state.borrow_mut() = None;
        self.search_bar.set_search_mode(false);
        self.update_title();
        self.update_diff_button();
        self.editor_view.grab_focus();
    }

    /// Open `path` in this window, leaving the window alone if the user
    /// cancels the Convert Tabs prompt.
    pub(super) fn open_path(this: &Rc<Self>, path: &Path) {
        Self::open_path_after(this, path, AfterOpen::Stay);
    }

    /// Open `path` in this window. If the file contains tabs, ask what to do
    /// first; `after` says what happens when the user cancels that prompt, in
    /// which case nothing is loaded.
    pub(super) fn open_path_after(this: &Rc<Self>, path: &Path, after: AfterOpen) {
        let document = match Document::open(path) {
            Ok(document) => document,
            Err(err) => {
                this.show_error("Could not open file", &err.open_message());
                return;
            }
        };
        let large = std::fs::metadata(path)
            .map(|meta| meta.len() > LARGE_FILE_WARNING_BYTES)
            .unwrap_or(false);

        if !tabs::contains_tab(document.text()) {
            this.finish_open(document, OpenMode::Normal, large);
            return;
        }

        // Reserve the path while the prompt is open. Another open request must
        // raise this window instead of loading the same file twice.
        if let Some(canonical) = document.path() {
            this.pending_open.replace(Some(canonical.to_path_buf()));
        }
        let opener = this.clone();
        this.prompt_convert_tabs(move |mode| match mode {
            Some(mode) => opener.finish_open(document, mode, large),
            None => opener.abort_open(after),
        });
    }

    /// The user cancelled the Convert Tabs prompt of an open.
    fn abort_open(&self, after: AfterOpen) {
        self.pending_open.replace(None);
        match after {
            AfterOpen::Stay => {}
            AfterOpen::CloseWindowOnCancel => {
                self.flags.allow_close.set(true);
                self.window.close();
            }
        }
    }

    /// Ask what to do with a file that contains tab characters. `on_choice`
    /// gets `Some(ConvertedTabs)` for Yes, `Some(ReadOnly)` for No (Read
    /// Only) and `None` for Cancel or dismissal. Used for open, reload and unlock.
    fn prompt_convert_tabs(&self, on_choice: impl FnOnce(Option<OpenMode>) + 'static) {
        dialogs::choose(&convert_tabs_dialog(), &self.window, move |response| {
            on_choice(match response {
                "yes" => Some(OpenMode::ConvertedTabs),
                "no" => Some(OpenMode::ReadOnly),
                _ => None,
            })
        });
    }

    /// Lock immediately; unlocking tabs requires conversion under the no-tab policy.
    pub(super) fn request_read_only(self: &Rc<Self>, read_only: bool) {
        self.editor_view.grab_focus();
        if self.diff_active() || read_only == self.flags.read_only.get() {
            return;
        }
        if read_only {
            self.set_read_only(true);
            return;
        }
        if !tabs::contains_tab(&self.current_text()) {
            self.set_read_only(false);
            return;
        }
        let this = self.clone();
        self.prompt_convert_tabs(move |mode| {
            if mode == Some(OpenMode::ConvertedTabs) {
                this.convert_buffer_tabs();
                this.set_read_only(false);
            }
        });
    }

    /// Convert tabs without an undo step or the paste filters, preserving the
    /// cursor. This matches opening a tabbed file with **Yes**: the editable
    /// document starts with spaces and Ctrl+Z cannot bring the tabs back.
    fn convert_buffer_tabs(self: &Rc<Self>) {
        let buffer = &self.editor_buffer;
        let offset = buffer.cursor_position();
        let converted = tabs::convert_tabs(&self.current_text());
        self.set_editor_text(&converted);
        buffer.place_cursor(&buffer.iter_at_offset(offset));
        // `set_editor_text` treats the replacement as a load and suppresses
        // the normal edit bookkeeping; the unlock was requested by the user.
        self.record_user_edit();
    }

    fn finish_open(&self, document: Document, mode: OpenMode, large: bool) {
        self.pending_open.replace(None);
        self.load_document(document, mode);
        if large {
            self.toast(
                ToastKind::Warning,
                "Large file: diffing runs in-process and may be slow",
            );
        }
    }

    pub(super) fn request_open_dialog(self: &Rc<Self>) {
        self.request_open_dialog_in(false);
    }

    pub(super) fn request_open_dialog_in(self: &Rc<Self>, new_window: bool) {
        if self.diff_active() {
            return;
        }
        self.show_open_dialog_with(None, None, new_window);
    }

    /// Open a recent entry in this window or, when Shift was held, in a new
    /// window without disturbing the current draft.
    pub(super) fn request_open_recent(self: &Rc<Self>, path: PathBuf, new_window: bool) {
        if self.diff_active() {
            return;
        }
        if new_window {
            self.open_path_in_new_window(&path);
        } else {
            self.request_open_path(path);
        }
    }

    /// Open a path chosen from the chooser or the recent list. The path is
    /// committed first, so the usual save prompt runs before the document is
    /// replaced.
    fn request_open_path(self: &Rc<Self>, path: PathBuf) {
        if self.diff_active() {
            return;
        }
        // Check before the save prompt: an already-open file must not discard
        // or replace this document. The current window's own path becomes a
        // reload rather than a duplicate-open request.
        if self.handle_existing_open(&path) {
            return;
        }
        self.require_saved(PendingAction::OpenPath(path));
    }

    /// Open a path typed into the Open menu's path field. Relative paths
    /// resolve from the current document's folder, or from home when the
    /// document is untitled.
    pub(super) fn request_open_location(self: &Rc<Self>, text: &str) {
        self.request_open_location_in(text, false);
    }

    pub(super) fn request_open_location_in(self: &Rc<Self>, text: &str, new_window: bool) {
        if self.diff_active() {
            return;
        }
        let Some(path) = resolve_open_location(text, self.doc.borrow().path()) else {
            return;
        };

        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                self.reject_open_location();
                return;
            }
        };
        match location_kind(&metadata) {
            LocationKind::Folder => {
                self.open_popover.popdown();
                self.show_open_dialog_with(Some(&path), None, new_window);
                return;
            }
            LocationKind::Device => {
                self.open_popover.popdown();
                self.show_open_dialog_with(None, Some(&path), new_window);
                return;
            }
            LocationKind::Other => {
                self.reject_open_location();
                return;
            }
            LocationKind::Regular => {}
        }

        // Validate contents before a dirty document's save prompt. The path is
        // opened again after saving; a manually typed path is worth the second
        // read to keep the pending-action state simple.
        if let Err(err) = Document::open(&path) {
            match err {
                fsutil::ReadError::TooLarge { .. }
                | fsutil::ReadError::NotUtf8
                | fsutil::ReadError::NulByte => {
                    self.show_error("Could not open file", &err.open_message());
                }
                _ => self.reject_open_location(),
            }
            return;
        }
        self.open_popover.popdown();
        if new_window {
            self.open_path_in_new_window(&path);
        } else {
            self.request_open_path(path);
        }
    }

    /// Leave the Open menu showing and flash the path field so the user can try
    /// another path without reopening the menu.
    fn reject_open_location(&self) {
        if let Some(row) = self.open_location_row.borrow().as_ref() {
            row.flash_error();
        }
    }

    /// Handle an open request for a path that a window already owns. A
    /// same-window request reloads; another window gets the duplicate toasts.
    /// Returns false when no window owns the path.
    fn handle_existing_open(self: &Rc<Self>, path: &Path) -> bool {
        let Some(owner) = super::window_for_open_path(path) else {
            return false;
        };
        if Rc::ptr_eq(&owner, self) {
            self.request_reload();
        } else {
            super::present_duplicate_open(&owner, Some(self));
        }
        true
    }

    /// The user re-opened the document already loaded in this window. Treat
    /// that as an explicit reload from disk: confirm before discarding unsaved
    /// edits, but leave an identical clean document completely untouched so
    /// its cursor, scroll position and undo history survive.
    fn request_reload(self: &Rc<Self>) {
        if self.diff_active() {
            return;
        }
        if self.flags.dirty.get() {
            let this = self.clone();
            dialogs::ask(
                &self.window,
                "Reload from Disk?",
                "The document has unsaved changes.",
                &[
                    Response::new("cancel", "Cancel"),
                    Response::destructive("reload", "Reload from Disk"),
                ],
                "cancel",
                "cancel",
                move |response| {
                    if response == "reload" {
                        this.flags.external_dialog_open.set(true);
                        this.reload_from_disk(ExternalCheck::Focus);
                    }
                },
            );
            return;
        }

        let disk = self.doc.borrow().read_disk();
        match disk {
            Ok(text) if text == self.current_text() => {
                if let Err(err) = self.doc.borrow_mut().adopt_disk_mtime() {
                    self.show_error("Could not reload file", &err.to_string());
                    return;
                }
                self.toast(
                    ToastKind::Info,
                    "Document is identical with its file on disk",
                );
            }
            _ => {
                self.flags.external_dialog_open.set(true);
                self.reload_from_disk(ExternalCheck::Focus);
            }
        }
    }

    /// Open the chooser, optionally starting at a typed folder or file.
    fn show_open_dialog_with(
        self: &Rc<Self>,
        folder: Option<&Path>,
        file: Option<&Path>,
        new_window: bool,
    ) {
        let dialog = gtk::FileDialog::builder()
            .title("Open File")
            .modal(true)
            .build();
        if let Some(folder) = folder {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        if let Some(file) = file {
            dialog.set_initial_file(Some(&gio::File::for_path(file)));
        }
        let this = self.clone();
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| Self::finish_open_dialog_in(&this, result, new_window),
        );
    }

    /// A cancelled chooser must return keyboard input to the editor, not the
    /// Open button or another non-editable widget. Successful loads restore
    /// focus themselves, after any Convert Tabs prompt has been answered.
    #[cfg(test)]
    pub(super) fn finish_open_dialog(this: &Rc<Self>, result: Result<gio::File, glib::Error>) {
        Self::finish_open_dialog_in(this, result, false);
    }

    pub(super) fn finish_open_dialog_in(
        this: &Rc<Self>,
        result: Result<gio::File, glib::Error>,
        new_window: bool,
    ) {
        // File-replacing actions are disabled in Diff Mode; ignore a chooser
        // result that arrives after a review has started.
        if this.diff_active() {
            return;
        }
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            if new_window {
                this.open_path_in_new_window(&path);
            } else {
                // The chooser already committed to a path; the save prompt, if
                // any, comes now so cancelling the chooser costs nothing.
                this.request_open_path(path);
            }
        } else {
            this.editor_view.grab_focus();
        }
    }

    /// New File replaces the document in this window. A dirty document gets
    /// the usual save prompt first.
    fn request_new_file(self: &Rc<Self>) {
        if self.diff_active() {
            return;
        }
        self.require_saved(PendingAction::NewFile);
    }

    /// Reset this window to an empty, untitled, editable document.
    fn new_document(&self) {
        self.load_document(Document::new(), OpenMode::Normal);
    }

    pub(super) fn persist(&self, path: Option<&Path>) -> Result<(), String> {
        let text = self.current_text();
        let mut doc = self.doc.borrow_mut();
        let result = match path {
            Some(path) => doc.save_as(path, text),
            None => doc.save_live(text),
        };
        match result {
            Ok(()) => {
                drop(doc);
                if path.is_some()
                    && let Some(saved) = self.path_for_snapshot()
                {
                    self.recent.record(&saved);
                }
                self.flags.dirty.set(false);
                self.update_title();
                self.update_diff_button();
                Ok(())
            }
            Err(err) => Err(err.to_string()),
        }
    }

    /// Save to the current path (or ask for one), then run `then` if given.
    /// A file that changed on disk since it was opened or last saved gets an
    /// explicit overwrite confirmation first; focus checks alone are not
    /// enough because a sync client can write while the window is focused.
    pub(super) fn save_then(self: &Rc<Self>, then: Option<PendingAction>) {
        if self.doc.borrow().path().is_none() {
            self.save_as_then(then);
            return;
        }

        if self.doc.borrow().externally_modified() {
            self.confirm_overwrite_save(then);
            return;
        }

        self.save_now(then);
    }

    fn save_now(self: &Rc<Self>, then: Option<PendingAction>) {
        match self.persist(None) {
            Ok(()) => {
                self.toast(ToastKind::Success, "Saved");
                if self.diff_active() {
                    self.update_diff_timestamps();
                }
                if let Some(action) = then {
                    self.run_pending(action);
                }
            }
            Err(err) => self.show_error("Could not save", &err),
        }
    }

    fn confirm_overwrite_save(self: &Rc<Self>, then: Option<PendingAction>) {
        if self.flags.external_dialog_open.replace(true) {
            return;
        }
        let dialog = dialogs::build(
            "File changed on disk",
            "The file changed on disk since it was opened or last saved. Saving now \
             will replace the version on disk.",
            &[
                Response::new("cancel", "Cancel"),
                Response::destructive("overwrite", "Overwrite"),
            ],
            "cancel",
            "cancel",
        );
        let this = self.clone();
        dialogs::choose(&dialog, &self.window, move |response| {
            this.flags.external_dialog_open.set(false);
            if response == "overwrite" {
                this.save_now(then);
            }
        });
    }

    pub(super) fn save_as_then(self: &Rc<Self>, then: Option<PendingAction>) {
        if self.diff_active() {
            return;
        }

        let initial_name = self
            .doc
            .borrow()
            .path()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("untitled.md")
            .to_string();

        let dialog = gtk::FileDialog::builder()
            .title("Save As")
            .modal(true)
            .initial_name(&initial_name)
            .build();

        let this = self.clone();
        dialog.save(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                let file = match result {
                    Ok(file) => file,
                    Err(_) => {
                        this.editor_view.grab_focus();
                        return;
                    }
                };
                let Some(path) = file.path() else {
                    this.editor_view.grab_focus();
                    return;
                };
                this.save_as_path(&path, then);
            },
        );
    }

    /// Write the draft to `path` after checking that no other window owns the
    /// canonical target. The chooser callback and the smoke test both use this
    /// entry point.
    pub(super) fn save_as_path(self: &Rc<Self>, path: &Path, then: Option<PendingAction>) {
        if self.diff_active() {
            return;
        }

        // Compare the path `Document::save_as` will record. `canonical_target`
        // also resolves a path that is missing on disk, so a file deleted but
        // still open in another window cannot become a second owner.
        let canonical = match fsutil::canonical_target(path) {
            Ok(canonical) => canonical,
            Err(err) => {
                self.show_error("Could not save", &err.to_string());
                return;
            }
        };
        if let Some(owner) = super::window_for_canonical_path(&canonical)
            && !Rc::ptr_eq(&owner, self)
        {
            self.show_error(
                "Could not save",
                "That file is already open in another window.",
            );
            return;
        }

        let previous = self.path_for_snapshot();
        match self.persist(Some(path)) {
            Ok(()) => {
                self.toast(ToastKind::Success, "Saved");
                self.finish_save_as(previous, then);
            }
            Err(err) => self.show_error("Could not save", &err),
        }
    }

    /// After a successful Save As: when the document moved to a new path whose
    /// old path has a baseline and whose new path has none, offer to copy the
    /// baseline. A pending action runs once the offer is settled.
    fn finish_save_as(self: &Rc<Self>, previous: Option<PathBuf>, then: Option<PendingAction>) {
        let current = self.path_for_snapshot();
        let offer = match (previous, current) {
            (Some(previous), Some(current))
                if previous != current
                    && self.store.has_for_canonical(&previous)
                    && !self.store.has_for_canonical(&current) =>
            {
                Some(previous)
            }
            _ => None,
        };

        let Some(previous) = offer else {
            if let Some(action) = then {
                self.run_pending(action);
            }
            return;
        };

        let name = previous
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let this = self.clone();
        dialogs::ask(
            &self.window,
            "Copy the baseline too?",
            &format!(
                "{name} has a baseline. Copy it to the new file so Review Changes compares \
                 against the same save point? The original keeps its baseline."
            ),
            &[
                Response::new("no", "No Baseline"),
                Response::suggested("copy", "Copy Baseline"),
            ],
            "copy",
            "no",
            move |response| {
                if response == "copy" {
                    this.copy_baseline_from(&previous);
                }
                if let Some(action) = then {
                    this.run_pending(action);
                }
            },
        );
    }

    /// Give the open document a copy of the baseline of `previous` (a
    /// canonical live path). The original baseline is left in place, and an
    /// existing baseline is never replaced.
    pub(super) fn copy_baseline_from(&self, previous: &Path) {
        let Some(target) = self.path_for_snapshot() else {
            return;
        };
        let source = self.store.snapshot_path_for_canonical(previous);
        match self.store.link(&source, &target, LinkMode::Copy) {
            Ok(()) => self.toast(ToastKind::Success, "Baseline copied to the new file"),
            Err(err) => self.show_error("Could not copy baseline", &err.to_string()),
        }
        self.invalidate_baseline_cache();
        self.update_diff_button();
    }

    /// Carry out an action that was waiting for the document to be saved (or
    /// for its unsaved changes to be discarded).
    fn run_pending(self: &Rc<Self>, action: PendingAction) {
        match action {
            PendingAction::CreateBaseline => self.write_baseline(),
            PendingAction::ConfirmBaseline => self.confirm_diff(),
            PendingAction::OpenPath(path) => {
                // The file may have been opened elsewhere while the save
                // prompt was showing.
                if !self.handle_existing_open(&path) {
                    Self::open_path(self, &path);
                }
            }
            PendingAction::NewFile => self.new_document(),
            PendingAction::CloseWindow => {
                self.confirm_discard_review(AfterDiscardReview::CloseWindow);
            }
            PendingAction::QuitApplication => {
                self.confirm_discard_review(AfterDiscardReview::Quit);
            }
        }
    }

    /// Run `action` once the live document is saved. A clean document runs it
    /// at once. A dirty one triggers a prompt:
    ///
    /// - Baseline actions get Cancel / Save. Cancelling or dismissing drops
    ///   the action and leaves the baseline untouched.
    /// - Actions that may leave the document (OpenPath, New File) get a
    ///   neutral button that keeps the draft and resolves the request in a new
    ///   window, plus Discard Changes and Save. There is no Cancel button;
    ///   Esc and dismissal still abort the pending action.
    /// - Closing the window or quitting gets Cancel / Discard Changes / Save.
    ///
    /// For an untitled document the save button reads "Save As", because the
    /// document can only be saved through the file chooser.
    pub(super) fn require_saved(self: &Rc<Self>, action: PendingAction) {
        if !self.flags.dirty.get() {
            self.run_pending(action);
            return;
        }

        if let PendingAction::OpenPath(path) = &action {
            let path = path.clone();
            self.prompt_leave_document(action, "new-window", "Open New Window", move |this| {
                this.open_path_in_new_window(&path);
            });
            return;
        }

        if let PendingAction::NewFile = action {
            self.prompt_leave_document(action, "new-window", "Open New Window", |this| {
                this.new_window();
            });
            return;
        }

        let this = self.clone();
        let save_label = save_action_label(self.doc.borrow().path().is_some());
        match action.baseline_label() {
            Some(label) => dialogs::ask(
                &self.window,
                "Save the document first?",
                &format!(
                    "{label} updates the saved baseline, so the live document must be saved \
                     first. If you cancel, nothing changes and the baseline stays as it is."
                ),
                &[
                    Response::new("cancel", "Cancel"),
                    Response::suggested("save", save_label),
                ],
                "save",
                "cancel",
                move |response| {
                    if response == "save" {
                        this.save_then(Some(action));
                    }
                },
            ),
            None => dialogs::ask(
                &self.window,
                "Save changes?",
                "The document has unsaved changes.",
                &[
                    Response::new("cancel", "Cancel"),
                    Response::destructive("discard", "Discard Changes"),
                    Response::suggested("save", save_label),
                ],
                "save",
                "cancel",
                move |response| match response {
                    "save" => this.save_then(Some(action)),
                    "discard" if action.can_discard() => this.run_pending(action),
                    _ => {}
                },
            ),
        }
    }

    /// The shared "Save changes?" prompt for an action that may leave the
    /// document. The neutral button, whose id is `neutral_id` and whose label
    /// is `neutral_label`, keeps this document and lets `on_neutral` resolve
    /// the request elsewhere; Discard Changes and Save act on this window.
    /// Esc and dismissal abort the pending action.
    fn prompt_leave_document(
        self: &Rc<Self>,
        action: PendingAction,
        neutral_id: &'static str,
        neutral_label: &'static str,
        on_neutral: impl FnOnce(&Rc<Self>) + 'static,
    ) {
        let this = self.clone();
        let save_label = save_action_label(self.doc.borrow().path().is_some());
        dialogs::ask(
            &self.window,
            "Save changes?",
            "The document has unsaved changes.",
            &[
                Response::new(neutral_id, neutral_label),
                Response::destructive("discard", "Discard Changes"),
                Response::suggested("save", save_label),
            ],
            "save",
            "cancel",
            move |response| match response {
                "save" => this.save_then(Some(action)),
                "discard" => this.run_pending(action),
                id if id == neutral_id => on_neutral(&this),
                _ => {}
            },
        );
    }

    /// Open the selected path in a new window, leaving this document and its
    /// dirty state untouched.
    fn open_path_in_new_window(self: &Rc<Self>, path: &Path) {
        let Some(app) = self
            .window
            .application()
            .and_then(|app| app.downcast::<adw::Application>().ok())
        else {
            return;
        };
        super::open_window_for_path_with(
            &app,
            path,
            self.store.clone(),
            self.persist_settings,
            self.recent.clone(),
            Some(self),
        );
    }

    /// Ask for a new name or path for the open file. Does nothing in Diff
    /// Mode. An untitled document opens Save As instead.
    fn show_rename(self: &Rc<Self>) {
        if self.diff_active() {
            return;
        }
        let Some(path) = self.path_for_snapshot() else {
            self.save_as_then(None);
            return;
        };
        let current = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();

        let entry = gtk::Entry::new();
        entry.set_text(&current);
        entry.set_activates_default(true);
        entry.set_hexpand(true);

        let browse = gtk::Button::with_label("...");
        {
            let weak = Rc::downgrade(self);
            let entry = entry.clone();
            browse.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.browse_rename_target(&entry);
                }
            });
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&entry);
        row.append(&browse);

        let dialog = dialogs::build(
            "Rename or Move File",
            "Enter a new name and/or path.",
            &[
                Response::new("cancel", "Cancel"),
                Response::suggested("rename", "Rename"),
            ],
            "rename",
            "cancel",
        );
        dialog.set_extra_child(Some(&row));
        dialog.set_focus(Some(&entry));

        let this = self.clone();
        dialogs::choose(&dialog, &self.window, move |response| {
            if response == "rename" {
                this.rename_file(&entry.text());
            }
        });
    }

    /// Let the user pick the new location in a file browser and write the
    /// chosen absolute path into the rename dialog's `entry`. The chooser
    /// starts in the file's folder with its current name. Nothing is renamed
    /// until the user confirms the dialog, and the usual rename rules apply
    /// (a name already in use is refused there).
    fn browse_rename_target(&self, entry: &gtk::Entry) {
        let Some(current) = self.path_for_snapshot() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title("Choose New Location")
            .accept_label("Select")
            .modal(true)
            .build();
        if let Some(folder) = current.parent() {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        if let Some(name) = current.file_name().and_then(|name| name.to_str()) {
            dialog.set_initial_name(Some(name));
        }

        let entry = entry.clone();
        dialog.save(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    entry.set_text(&path.display().to_string());
                    entry.set_position(-1);
                    entry.grab_focus();
                }
            },
        );
    }

    /// Rename the open file and move its baseline along. Only the path
    /// changes: the editor text, dirty state and undo history are untouched.
    pub(super) fn rename_file(&self, input: &str) {
        let old_path = self.path_for_snapshot();
        let old_folder = old_path
            .as_deref()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        if let Some(old) = old_path.as_deref()
            && let Ok(planned) = rename::plan(old, input)
            && let Some(owner) = super::window_for_canonical_path(&planned)
            && !std::ptr::eq(self, owner.as_ref())
        {
            self.show_error(
                "Could not rename or move file",
                "That file is already open in another window.",
            );
            return;
        }
        let result = {
            let mut doc = self.doc.borrow_mut();
            rename::rename_document(&mut doc, &self.store, input)
        };
        match result {
            Ok(baseline) => {
                self.update_title();
                self.update_diff_button();
                let new_path = self.path_for_snapshot().unwrap_or_default();
                if !new_path.as_os_str().is_empty() {
                    self.recent.record(&new_path);
                }
                let name = new_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let moved = new_path.parent().map(Path::to_path_buf) != old_folder;
                let what = if moved {
                    format!("Moved to {}", new_path.display())
                } else {
                    format!("Renamed to {name}")
                };
                match baseline {
                    BaselineMove::None | BaselineMove::Moved => {
                        self.toast(ToastKind::Success, &what)
                    }
                    BaselineMove::Failed(err) => self.show_error(
                        "Baseline not moved",
                        &format!(
                            "The file is now at {}, but its baseline could not be moved: \
                             {err}\n\nThe old baseline is still in the snapshot store. \
                             Manage App… → Baseline Snapshots can link it to this \
                             document.",
                            new_path.display()
                        ),
                    ),
                }
            }
            Err(RenameError::Unchanged) => {}
            Err(
                err @ (RenameError::Empty
                | RenameError::InvalidName
                | RenameError::MissingFolder(_)),
            ) => {
                self.toast(ToastKind::Error, &err.to_string());
            }
            Err(err) => self.show_error("Could not rename or move file", &err.to_string()),
        }
    }

    /// Compare the on-disk modification time with the last read. This is the
    /// only reliable change-detection mechanism on shared ChromeOS folders.
    /// `after` says what follows once the check has settled with the document
    /// up to date; cancelling the dialogs drops it.
    pub(super) fn check_external(self: &Rc<Self>, after: ExternalCheck) {
        if self.flags.external_dialog_open.get() {
            return;
        }

        let modified = self.doc.borrow().externally_modified();
        let disk_mtime = self.doc.borrow().disk_mtime();
        // After Cancel on a reload's Convert Tabs prompt, stay quiet until the
        // file changes again.
        let declined = disk_mtime.is_some() && disk_mtime == self.flags.declined_reload_mtime.get();
        if !modified || declined {
            self.run_after_check(after);
            return;
        }

        let dirty = self.flags.dirty.get();
        let mut body = if dirty {
            "The file changed on disk since it was opened, and this window has unsaved edits. \
             Reloading discards those edits. Keeping them does not touch the disk until you save."
        } else {
            "The file changed on disk since it was opened. Reload it now, or keep the version in memory."
        }.to_string();
        if self.diff_active() {
            body.push_str(" Reloading leaves Diff Mode and discards any unconfirmed review changes. The saved baseline will not be changed.");
        }

        self.flags.external_dialog_open.set(true);
        let dialog = dialogs::build(
            "File changed on disk",
            &body,
            &[
                Response::new("keep", "Keep Current Text"),
                Response::destructive("reload", "Reload from Disk"),
            ],
            if dirty { "keep" } else { "reload" },
            "close",
        );
        dialog.set_can_close(false);
        let weak_dialog = dialog.downgrade();
        let this = self.clone();
        dialogs::choose(&dialog, &self.window, move |response| {
            // can-close also blocks AlertDialog's response buttons from closing
            // the dialog. Close explicitly only after a real choice, once the
            // choose handler has disconnected, avoiding a second close response.
            if matches!(response, "keep" | "reload")
                && let Some(dialog) = weak_dialog.upgrade()
            {
                dialog.force_close();
            }
            match response {
                // A reload may still ask about tabs, so the flag stays set
                // until `reload_from_disk` has settled.
                "reload" => this.reload_from_disk(after),
                "keep" => {
                    this.flags.external_dialog_open.set(false);
                    // Keep is temporary: do not acknowledge the disk change.
                    this.run_after_check(after);
                }
                _ => this.flags.external_dialog_open.set(false),
            }
        });
    }

    /// Carry out what was waiting for the external-change check.
    fn run_after_check(self: &Rc<Self>, after: ExternalCheck) {
        match after {
            ExternalCheck::Focus => {}
            ExternalCheck::EnterDiff => self.start_diff_session(),
        }
    }

    /// Reload the file from disk under the no-tab policy. The disk text is
    /// inspected first and the document is only touched once the user has
    /// chosen, so cancelling the Convert Tabs prompt leaves everything as it
    /// was (and the external-change prompt stays quiet until the file changes
    /// again).
    fn reload_from_disk(self: &Rc<Self>, after: ExternalCheck) {
        let disk = self.doc.borrow().read_disk();
        match disk {
            Err(err) => {
                self.flags.external_dialog_open.set(false);
                self.show_error("Could not reload file", &err.to_string());
            }
            Ok(text) if !tabs::contains_tab(&text) => {
                self.finish_reload(OpenMode::Normal, after);
            }
            Ok(_) => {
                let this = self.clone();
                let declined_mtime = self.doc.borrow().disk_mtime();
                self.prompt_convert_tabs(move |mode| match mode {
                    Some(mode) => this.finish_reload(mode, after),
                    None => {
                        this.flags.declined_reload_mtime.set(declined_mtime);
                        this.flags.external_dialog_open.set(false);
                    }
                });
            }
        }
    }

    fn finish_reload(self: &Rc<Self>, mode: OpenMode, after: ExternalCheck) {
        self.flags.external_dialog_open.set(false);
        let reloaded = self.doc.borrow_mut().reload();
        if let Err(err) = reloaded {
            self.show_error("Could not reload file", &err.to_string());
            return;
        }
        // Only leave review after a successful read. Failed or cancelled reloads
        // must preserve the session, including unconfirmed accepts and undo.
        if self.diff_active() {
            *self.diff_state.borrow_mut() = None;
            self.stack.set_visible_child_name("editor");
        }
        self.show_document_text(mode);
        self.reset_cursor_to_top();
        if self.doc.borrow().text().len() as u64 > LARGE_FILE_WARNING_BYTES {
            self.toast(
                ToastKind::Warning,
                "Large file: diffing runs in-process and may be slow",
            );
        }
        self.update_title();
        self.update_diff_button();
        self.run_after_check(after);
        if !self.diff_active() {
            self.editor_view.grab_focus();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LocationKind {
    Folder,
    Device,
    Other,
    Regular,
}

fn location_kind(metadata: &std::fs::Metadata) -> LocationKind {
    if metadata.is_dir() {
        LocationKind::Folder
    } else if is_device(metadata) {
        LocationKind::Device
    } else if metadata.is_file() {
        LocationKind::Regular
    } else {
        LocationKind::Other
    }
}

#[cfg(unix)]
fn is_device(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let kind = metadata.file_type();
    kind.is_block_device() || kind.is_char_device()
}

#[cfg(not(unix))]
fn is_device(_: &std::fs::Metadata) -> bool {
    false
}

/// Resolve a path typed into the Open menu. Surrounding quotes and whitespace
/// are ignored; `~` and `~/...` expand to the home folder, and relative paths
/// resolve from the current document's folder (or home when untitled).
fn resolve_open_location(text: &str, current: Option<&Path>) -> Option<PathBuf> {
    let text = text.trim();
    let text = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(text);
    if text.is_empty() {
        return None;
    }

    let home = glib::home_dir();
    let path = if text == "~" {
        home.clone()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(text)
    };
    if path.is_absolute() {
        Some(path)
    } else {
        let base = current
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or(home);
        Some(base.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::{LocationKind, location_kind, resolve_open_location};
    use gtk::glib;
    use std::path::{Path, PathBuf};

    #[test]
    fn open_location_paths_resolve_from_the_document_folder() {
        let current = Path::new("/home/you/notes/todo.md");
        assert_eq!(
            resolve_open_location(" other.md ", Some(current)),
            Some(PathBuf::from("/home/you/notes/other.md"))
        );
        assert_eq!(
            resolve_open_location("\"/tmp/x.md\"", Some(current)),
            Some(PathBuf::from("/tmp/x.md"))
        );
        assert_eq!(
            resolve_open_location("'/tmp/y.md'", Some(current)),
            Some(PathBuf::from("/tmp/y.md"))
        );
        let home = glib::home_dir();
        assert_eq!(
            resolve_open_location("~/notes/z.md", Some(current)),
            Some(home.join("notes/z.md"))
        );
        assert_eq!(
            resolve_open_location("~", Some(current)),
            Some(home.clone())
        );
        assert_eq!(resolve_open_location("a.md", None), Some(home.join("a.md")));
    }

    #[test]
    fn blank_open_locations_are_ignored() {
        for text in ["", "   ", "\"\"", "''"] {
            assert_eq!(resolve_open_location(text, None), None, "{text:?}");
        }
    }

    #[test]
    fn typed_paths_are_classified_before_opening() {
        let dir = std::env::temp_dir().join(format!("notediff-open-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file.txt");
        std::fs::write(&file, "text\n").unwrap();
        assert_eq!(
            location_kind(&std::fs::metadata(&file).unwrap()),
            LocationKind::Regular
        );
        assert_eq!(
            location_kind(&std::fs::metadata(&dir).unwrap()),
            LocationKind::Folder
        );
        #[cfg(unix)]
        {
            if let Ok(device) = std::fs::metadata("/dev/null") {
                assert_eq!(location_kind(&device), LocationKind::Device);
            }
            let socket = dir.join("socket");
            let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            assert_eq!(
                location_kind(&std::fs::metadata(&socket).unwrap()),
                LocationKind::Other
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
