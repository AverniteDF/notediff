//! The main application window.
//!
//! `AppWindow` holds shared state, builds the window and wires status updates.
//! Feature submodules wire their own signals and actions:
//!
//! - `editing`: insert filter for tabs and document line endings, Tab key,
//!   Scale Indentation, EOL conversion, file insertion, word wrap and
//!   read-only state.
//! - `file_actions`: new file and new window, open, save, save as, rename or
//!   move, external-change handling and the save-then-continue flow.
//! - `focus`: in-window focus restoration after menus and dialogs close.
//! - `diff_actions`: the Diff Mode session wiring, Diff Mode keys, Create
//!   Baseline and Confirm.
//! - `menus`: menu construction, shared menu actions and accelerators.
//! - `search`: the search bar, Find, Replace and Go to Line.
//! - `status`: `StatusState` (measured text, last edit, refresh timer) and the
//!   status bar refresh methods.
//!
//! # Ownership
//!
//! The registry below is the deliberate owner of every window: it holds one
//! strong `Rc<AppWindow>` per open window and drops it when GTK removes that
//! window from the application. Window signal handlers, actions and controllers
//! capture a `Weak<AppWindow>` and upgrade it as needed, so the widgets and
//! handlers form no reference cycle and a closed window is freed. One-shot
//! dialog callbacks may hold a strong clone until the dialog is answered.

/// Wrap a unit-returning signal handler without keeping its window alive.
macro_rules! weak_handler {
    ($this:ident, |$($arg:pat_param),+| $body:block) => {{
        let weak = Rc::downgrade($this);
        move |$($arg),+| {
            let Some($this) = weak.upgrade() else {
                return;
            };
            $body
        }
    }};
}

mod clipboard;
mod diff_actions;
mod editing;
mod file_actions;
mod flags;
mod flash;
mod focus;
mod menus;
mod search;
mod status;
#[cfg(test)]
mod tests;
mod theme_keys;

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::{gio, glib};
use sourceview5::prelude::*;

use crate::core::document::Document;
use crate::core::pending::AfterOpen;
use crate::core::recent::RecentDocuments;
use crate::core::session::DiffSession;
use crate::core::snapshot_store::SnapshotStore;
use crate::ui::app_dialog::snapshots_tab::LinkTarget;
use crate::ui::dialogs::{self, Response};
use crate::ui::diff_page::DiffPage;
use crate::ui::gutter_selection;
use crate::ui::status_bar::{EditMode, StatusBar};
use crate::ui::theme;
use crate::ui::toast::{self, ToastKind};

const REVIEW_LABEL: &str = "Review Changes (F5)";
const CREATE_LABEL: &str = "Create Baseline";
const SAVE_AS_LABEL: &str = "Save As";
const NO_CHANGES_LABEL: &str = "No Changes";

/// Baseline text of the open document, cached so buffer changes can compare
/// against it without reading the snapshot file. `key` is the snapshot path
/// and its modification time; `None` means there is no readable baseline.
#[derive(Default)]
struct BaselineCache {
    key: Option<(PathBuf, Option<SystemTime>)>,
    text: String,
}

/// Compose the window title: an optional dirty marker, the file name and an
/// optional read-only suffix. The title describes the document, not the view
/// mode, so entering Diff Mode does not change it.
fn title_text(name: &str, dirty: bool, read_only: bool) -> String {
    let dot = if dirty { "• " } else { "" };
    let read_only = if read_only { " (Read-Only)" } else { "" };
    format!("{dot}{name}{read_only}")
}

/// The header button is suggested (blue) only for Save As on an untitled
/// document.
fn save_as_suggested(has_path: bool) -> bool {
    !has_path
}

/// Review Changes is actionable while a baseline exists and the live text
/// differs from it. It is disabled when the two match and while its own view
/// is showing. The untitled and no-baseline states stay enabled because their
/// button performs Save As or Create Baseline instead.
fn review_enabled(has_path: bool, has_baseline: bool, differs: bool, diff_active: bool) -> bool {
    !has_path || !has_baseline || (differs && !diff_active)
}

thread_local! {
    /// The strong owner of every open window (see the module docs).
    static WINDOWS: RefCell<Vec<Rc<AppWindow>>> = const { RefCell::new(Vec::new()) };
}

/// Wire window ownership and accelerators once at application startup.
pub(super) fn initialize(app: &adw::Application) {
    // AppWindow owns the GTK widget, so release the registry entry when GTK
    // removes the window from the application during close.
    app.connect_window_removed(|_, removed_window| {
        let removed = WINDOWS.with_borrow_mut(|windows| {
            let index = windows
                .iter()
                .position(|window| window.window.upcast_ref::<gtk::Window>() == removed_window)?;
            Some(windows.remove(index))
        });
        // Drop outside the borrow in case widget teardown re-enters GTK.
        drop(removed);
    });
    menus::install_accels(app);
}

pub struct AppWindow {
    window: adw::ApplicationWindow,
    stack: gtk::Stack,
    toast_overlay: adw::ToastOverlay,
    editor_view: sourceview5::View,
    editor_buffer: sourceview5::Buffer,
    status: StatusBar,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_controls: search::SearchControls,
    diff: Rc<DiffPage>,
    new_button: gtk::Button,
    open_button: gtk::Button,
    open_popover: gtk::PopoverMenu,
    open_clear_button: gtk::Button,
    /// Custom two-line rows currently installed in the Open menu.
    open_recent_rows: RefCell<Vec<gtk::Button>>,
    diff_button: gtk::Button,
    /// The "No Changes" label flash on the header button (see `flash`).
    flash: flash::FlashState,
    doc: RefCell<Document>,
    /// Cached baseline text of the open document (see `BaselineCache`).
    baseline: RefCell<BaselineCache>,
    /// Whether the editor text currently differs from that baseline.
    baseline_differs: Cell<bool>,
    /// Dirty, read-only, close and external-change flags (see `flags`).
    flags: flags::DocumentFlags,
    /// Measured text, last edit and refresh timer (see `status`).
    status_state: status::StatusState,
    /// Whether theme and font settings are written to disk.
    persist_settings: bool,
    store: SnapshotStore,
    /// Remembered recent document paths (see `recent`).
    recent: RecentDocuments,
    diff_state: RefCell<Option<DiffSession>>,
    last_copy_view: glib::WeakRef<sourceview5::View>,
    #[cfg(test)]
    last_toast: RefCell<Option<adw::Toast>>,
}

impl AppWindow {
    /// Build a window that keeps its baselines in `store` and does not persist
    /// settings. Production windows use the default store under
    /// `$XDG_DATA_HOME`; the UI smoke test passes a temporary one so it never
    /// touches the real baselines.
    #[cfg(test)]
    fn with_store(app: &adw::Application, store: SnapshotStore) -> Rc<Self> {
        Self::with_parts(app, store, false, RecentDocuments::disabled())
    }

    /// Build a window with explicit storage: the baseline `store`, whether
    /// theme/font settings are written to disk, and the recent-document state.
    fn with_parts(
        app: &adw::Application,
        store: SnapshotStore,
        persist_settings: bool,
        recent: RecentDocuments,
    ) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1120)
            .default_height(780)
            .title("Untitled")
            .build();

        let header = adw::HeaderBar::new();
        header.add_css_class(theme::MAIN_HEADER_CLASS);
        header.set_show_end_title_buttons(true);

        // New File is wired through its action, so it needs no handler below.
        let new_button = gtk::Button::from_icon_name("document-new-symbolic");
        new_button.set_tooltip_text(Some("New File (Ctrl+N)"));
        new_button.add_css_class("flat");
        new_button.set_action_name(Some("win.new"));
        header.pack_start(&new_button);

        // The Open button shows recent files when there are any; otherwise its
        // handler opens the chooser directly. The popover is built on demand.
        let open_button = gtk::Button::from_icon_name("document-open-symbolic");
        open_button.set_tooltip_text(Some("Open File (Ctrl+O)"));
        open_button.add_css_class("flat");
        let open_popover = gtk::PopoverMenu::from_model(None::<&gio::Menu>);
        open_popover.set_parent(&open_button);
        let open_clear_button = menus::build_clear_recent_button();
        {
            let popover = open_popover.clone();
            open_button.connect_destroy(move |_| {
                if popover.parent().is_some() {
                    popover.unparent();
                }
            });
        }
        header.pack_start(&open_button);

        let copy_button = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_button.set_tooltip_text(Some("Copy Selected Text (Ctrl+C)"));
        copy_button.add_css_class("flat");
        copy_button.set_focus_on_click(false);
        header.pack_start(&copy_button);

        let diff_button = gtk::Button::with_label(REVIEW_LABEL);
        header.pack_end(&diff_button);

        let main_menu = menus::build_main_menu_button();
        header.pack_end(&main_menu);

        let editor_buffer = make_editor_buffer();
        let editor_view = sourceview5::View::with_buffer(&editor_buffer);
        theme::apply_content_view(&editor_view);
        editor_view.set_monospace(true);
        editor_view.set_show_line_numbers(true);
        gutter_selection::install_line_selection(&editor_view);
        editor_view.set_highlight_current_line(true);
        editor_view.set_auto_indent(true);
        // No-tab policy: the Tab key is handled by `press_tab`. The tab width
        // only matters for displaying read-only files that still hold tabs.
        editor_view.set_insert_spaces_instead_of_tabs(true);
        editor_view.set_indent_on_tab(false);
        editor_view.set_tab_width(4);
        editor_view.set_wrap_mode(gtk::WrapMode::WordChar);
        editor_view.set_extra_menu(Some(&menus::build_context_menu()));
        editor_view.set_left_margin(8);
        editor_view.set_top_margin(6);
        editor_view.set_bottom_margin(6);

        let editor_scroller = gtk::ScrolledWindow::new();
        editor_scroller.set_child(Some(&editor_view));
        editor_scroller.set_hexpand(true);
        editor_scroller.set_vexpand(true);
        editor_scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);

        let status = StatusBar::editor();
        status.set_ending_menu(&menus::build_eol_menu());
        status.set_mode_menu(&menus::build_mode_menu());

        let editor_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        editor_page.append(&editor_scroller);
        editor_page.append(&status.root);
        editor_page.set_hexpand(true);
        editor_page.set_vexpand(true);

        let diff = DiffPage::new();

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.add_named(&editor_page, Some("editor"));
        stack.add_named(&diff.root, Some("diff"));
        stack.set_visible_child_name("editor");
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&stack));
        toast_overlay.set_hexpand(true);
        toast_overlay.set_vexpand(true);

        let (search_bar, search_entry, search_controls) = search::build_search_bar();

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.add_top_bar(&search_bar);
        toolbar_view.set_content(Some(&toast_overlay));
        window.set_content(Some(&toolbar_view));

        let this = Rc::new(Self {
            window: window.clone(),
            stack,
            toast_overlay,
            editor_view,
            editor_buffer,
            status,
            search_bar,
            search_entry,
            search_controls,
            diff,
            new_button: new_button.clone(),
            open_button: open_button.clone(),
            open_popover: open_popover.clone(),
            open_clear_button: open_clear_button.clone(),
            open_recent_rows: RefCell::new(Vec::new()),
            diff_button: diff_button.clone(),
            flash: flash::FlashState::default(),
            doc: RefCell::new(Document::new()),
            baseline: RefCell::new(BaselineCache::default()),
            baseline_differs: Cell::new(false),
            flags: flags::DocumentFlags::default(),
            status_state: status::StatusState::default(),
            persist_settings,
            store,
            recent,
            diff_state: RefCell::new(None),
            last_copy_view: glib::WeakRef::new(),
            #[cfg(test)]
            last_toast: RefCell::new(None),
        });

        Self::connect_signals(&this, &diff_button);
        Self::connect_copy_button(&this, &copy_button);
        Self::connect_open_menu(&this);
        let mut popovers = Vec::new();
        if let Some(popover) = main_menu.popover() {
            popovers.push(popover);
        }
        popovers.push(this.open_popover.clone().upcast::<gtk::Popover>());
        Self::connect_transient_focus(&this, popovers);
        WINDOWS.with_borrow_mut(|windows| windows.push(this.clone()));
        this.update_title();
        this.update_diff_button();
        this.update_status();
        this
    }

    /// Wire buffer changes and status updates, then delegate feature wiring
    /// to the window submodules.
    fn connect_signals(this: &Rc<Self>, diff_button: &gtk::Button) {
        // One handler for every text change. The status bar follows programmatic
        // loads too, while only user edits make the document dirty.
        this.editor_buffer.connect_changed(weak_handler!(this, |_| {
            this.status_state.invalidate_text();
            this.schedule_status_update();
            if this.flags.is_loading() {
                return;
            }
            this.note_edit();
            this.flags.dirty.set(this.dirty_after_edit());
            this.update_title();
            this.enable_review_on_edit();
        }));

        // Cursor movement, selection changes and overwrite toggles can fire
        // together, so status refreshes are coalesced.
        this.editor_buffer
            .connect_cursor_position_notify(weak_handler!(this, |_| {
                this.schedule_status_update();
            }));
        this.editor_buffer
            .connect_has_selection_notify(weak_handler!(this, |_| {
                this.schedule_status_update();
            }));
        this.editor_view
            .connect_overwrite_notify(weak_handler!(this, |_| {
                this.schedule_status_update();
            }));

        Self::connect_search(this);
        Self::connect_menu_actions(this);
        Self::connect_editing(this);
        Self::connect_file_actions(this);
        Self::connect_diff_actions(this, diff_button);
        Self::connect_status_activity(this);
        Self::connect_theme_keys(this);
    }

    /// Open the chooser directly when there are no recent paths. Otherwise
    /// refill the menu from the pruned recent list. This is the only place the
    /// recent state is pruned, so startup and ordinary editing never scan the
    /// remembered paths.
    fn connect_open_menu(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.open_button.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.open_popover.is_visible() {
                this.open_popover.popdown();
                return;
            }
            let entries = this.recent.pruned_entries();
            if !menus::open_menu_has_entries(&entries) {
                this.request_open_dialog();
                return;
            }
            let old_rows = std::mem::take(&mut *this.open_recent_rows.borrow_mut());
            for row in old_rows {
                if row.parent().is_some() {
                    row.unparent();
                }
            }
            if this.open_clear_button.parent().is_some() {
                this.open_clear_button.unparent();
            }
            this.open_popover
                .set_menu_model(Some(&menus::build_open_menu(&entries)));

            let mut rows = Vec::with_capacity(entries.len());
            for (index, path) in entries.iter().enumerate() {
                let row = menus::build_recent_button(path);
                let _ = this
                    .open_popover
                    .add_child(&row, &menus::recent_row_id(index));
                let popover = this.open_popover.clone();
                row.connect_clicked(move |_| popover.popdown());
                rows.push(row);
            }
            *this.open_recent_rows.borrow_mut() = rows;

            let _ = this
                .open_popover
                .add_child(&this.open_clear_button, "clear-recent");
            this.open_popover.popup();
        });
    }

    fn diff_active(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("diff")
    }

    /// Refresh the header button for the current document. An untitled document
    /// gets "Save As" on the suggested (blue) background, a file with no
    /// baseline gets "Create Baseline", and a file with a baseline gets
    /// "Review Changes (F5)". Review is enabled only while the live text
    /// differs from the baseline and its own view is not showing. A flash in
    /// progress ("No Changes") keeps ownership of the label.
    fn update_diff_button(&self) {
        self.refresh_baseline_cache();

        let has_path = self.doc.borrow().path().is_some();
        let has_baseline = self.baseline_exists();
        let differs = has_baseline
            && self.status_state.measure(
                || self.buffer_text(),
                |text, _| self.baseline_differs_from(text),
            );
        self.baseline_differs.set(differs);

        let suggested = save_as_suggested(has_path);
        self.diff_button.set_sensitive(review_enabled(
            has_path,
            has_baseline,
            self.baseline_differs.get(),
            self.diff_active(),
        ));

        if suggested {
            self.diff_button.remove_css_class("flat");
            self.diff_button.add_css_class("suggested-action");
        } else {
            self.diff_button.add_css_class("flat");
            self.diff_button.remove_css_class("suggested-action");
        }

        let (label, tooltip) = if !has_path {
            (SAVE_AS_LABEL, None)
        } else if has_baseline {
            let tooltip = (!self.diff_active()).then_some("Compare with baseline");
            (REVIEW_LABEL, tooltip)
        } else {
            (
                CREATE_LABEL,
                Some("Save current text as a baseline for later comparison"),
            )
        };

        self.diff_button.set_tooltip_text(tooltip);

        if !self.flash.is_active() {
            self.diff_button.set_label(label);
        }

        self.update_file_actions();
    }

    /// Any user edit makes the live text differ from an existing baseline, so
    /// enable Review immediately instead of waiting for the idle comparison.
    /// The next settle disables it again if the text returns to the baseline.
    fn enable_review_on_edit(&self) {
        if !self.diff_active() && self.baseline.borrow().key.is_some() {
            self.diff_button.set_sensitive(true);
        }
    }

    /// Forget the cached baseline so the next update reads the snapshot again.
    /// Used after the app itself writes, links or deletes a baseline, where a
    /// same-timestamp rewrite could otherwise go unnoticed.
    fn invalidate_baseline_cache(&self) {
        let mut cache = self.baseline.borrow_mut();
        cache.key = None;
        cache.text.clear();
    }

    /// Reload the cached baseline when the open document's snapshot path or
    /// modification time changed. A missing or unreadable snapshot clears the
    /// cache, so it never drives a stale Review suggestion.
    fn refresh_baseline_cache(&self) {
        let path = self.doc.borrow().path().map(Path::to_path_buf);
        let key = path.as_ref().and_then(|path| {
            let snapshot = self.store.snapshot_path_for_canonical(path);
            let metadata = std::fs::metadata(&snapshot).ok()?;
            metadata
                .is_file()
                .then(|| (snapshot, metadata.modified().ok()))
        });

        let mut cache = self.baseline.borrow_mut();
        if cache.key == key {
            return;
        }
        let loaded = path
            .as_ref()
            .and_then(|path| self.store.read_for_canonical(path).ok().flatten());
        match loaded {
            Some(text) => {
                cache.key = key;
                cache.text = text;
            }
            None => {
                cache.key = None;
                cache.text.clear();
            }
        }
    }

    /// True when a readable baseline is cached and `text` differs from it.
    /// Rust's string equality checks the byte length first, so a length change
    /// is an immediate mismatch.
    fn baseline_differs_from(&self, text: &str) -> bool {
        let cache = self.baseline.borrow();
        cache.key.is_some() && cache.text != text
    }

    fn baseline_exists(&self) -> bool {
        self.doc
            .borrow()
            .path()
            .is_some_and(|path| self.store.has_for_canonical(path))
    }

    fn update_title(&self) {
        let name = self
            .doc
            .borrow()
            .path()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "Untitled".to_string());
        let is_dirty = self.flags.dirty.get();
        // Besides the dot, gently tint the header bar (`install_css`).
        if is_dirty {
            self.window.add_css_class(theme::DIRTY_CLASS);
        } else {
            self.window.remove_css_class(theme::DIRTY_CLASS);
        }
        self.window.set_title(Some(&title_text(
            &name,
            is_dirty,
            self.flags.read_only.get(),
        )));
    }

    /// Assume file-backed documents dirty immediately, then compare on idle.
    /// Coalescing avoids whole-buffer comparisons for every Replace All edit.
    /// An untitled document is dirty only while it contains text.
    fn dirty_after_edit(self: &Rc<Self>) -> bool {
        if self.doc.borrow().path().is_none() {
            return self.editor_buffer.char_count() > 0;
        }
        self.schedule_dirty_check();
        true
    }

    fn schedule_dirty_check(self: &Rc<Self>) {
        if self.flags.dirty_check_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        gtk::glib::idle_add_local_once(move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.flags.dirty_check_pending.set(false);
            this.settle_dirty();
        });
    }

    /// Compare the measured buffer text with both the saved text and the
    /// cached baseline. The status bar has already measured or will measure
    /// the same string, so cursor moves and cached comparisons add no copy.
    fn settle_dirty(&self) {
        let (dirty, differs) = {
            let doc = self.doc.borrow();
            if doc.path().is_none() {
                return;
            }
            self.status_state.measure(
                || self.buffer_text(),
                |text, _| {
                    (
                        self.editor_buffer.char_count() as usize != doc.saved_chars()
                            || doc.differs_from(text),
                        self.baseline_differs_from(text),
                    )
                },
            )
        };
        if self.flags.dirty.replace(dirty) != dirty {
            self.update_title();
        }
        if self.baseline_differs.replace(differs) != differs {
            self.update_diff_button();
        }
    }

    fn edit_mode(&self) -> EditMode {
        if self.flags.read_only.get() {
            EditMode::ReadOnly
        } else if self.editor_view.overwrites() {
            EditMode::Overwrite
        } else {
            EditMode::Insert
        }
    }

    /// Character offsets (start <= end) of a non-empty selection.
    fn selection_offsets(&self) -> Option<(usize, usize)> {
        let (start, end) = self.editor_buffer.selection_bounds()?;
        Some((start.offset().max(0) as usize, end.offset().max(0) as usize))
    }

    fn toast(&self, kind: ToastKind, message: &str) {
        let toast = toast::new(kind, message);
        toast.set_timeout(4);
        #[cfg(test)]
        self.last_toast.replace(Some(toast.clone()));
        self.toast_overlay.add_toast(toast);
    }

    fn show_error(&self, heading: &str, body: &str) {
        dialogs::ask(
            &self.window,
            heading,
            body,
            &[Response::new("close", "Close")],
            "close",
            "close",
            |_| {},
        );
    }

    fn current_text(&self) -> String {
        self.buffer_text().as_str().to_owned()
    }

    fn buffer_text(&self) -> gtk::glib::GString {
        let start = self.editor_buffer.start_iter();
        let end = self.editor_buffer.end_iter();
        self.editor_buffer.text(&start, &end, true)
    }

    fn path_for_snapshot(&self) -> Option<PathBuf> {
        self.doc.borrow().path().map(Path::to_path_buf)
    }

    /// The document Manage Snapshots may link a baseline to: its canonical
    /// path and current editor text (used to rank candidates). `None` for an
    /// untitled document.
    fn link_target(&self) -> Option<LinkTarget> {
        let path = self.path_for_snapshot()?;
        Some(LinkTarget {
            path,
            text: self.current_text(),
        })
    }

    /// Open a new, empty window that shares this window's snapshot store and
    /// settings persistence. The current document is not affected, so no save
    /// prompt is involved.
    fn new_window(&self) {
        if let Some(app) = self
            .window
            .application()
            .and_then(|app| app.downcast::<adw::Application>().ok())
        {
            open_empty_window_with(
                &app,
                self.store.clone(),
                self.persist_settings,
                self.recent.clone(),
            );
        }
    }
}

fn make_editor_buffer() -> sourceview5::Buffer {
    // notediff renders every document as plain text: no language is attached.
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_highlight_syntax(false);
    theme::apply_scheme(&buffer);
    buffer
}

/// Connect a button's `clicked` signal to `action`. The handler holds the
/// window weakly, so the button does not keep it alive.
fn connect_button<F>(button: &gtk::Button, this: &Rc<AppWindow>, action: F)
where
    F: Fn(&Rc<AppWindow>) + 'static,
{
    button.connect_clicked(weak_handler!(this, |_| {
        action(&this);
    }));
}

/// Register a stateless window action that runs `handler` with the window.
/// The action holds the window weakly.
fn add_action<F>(window: &adw::ApplicationWindow, this: &Rc<AppWindow>, name: &str, handler: F)
where
    F: Fn(&Rc<AppWindow>) + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(weak_handler!(this, |_, _| {
        handler(&this);
    }));
    window.add_action(&action);
}

/// Register a window action with a single string parameter (used for recent
/// file paths). The action holds the window weakly.
fn add_string_action<F>(
    window: &adw::ApplicationWindow,
    this: &Rc<AppWindow>,
    name: &str,
    handler: F,
) where
    F: Fn(&Rc<AppWindow>, &str) + 'static,
{
    let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(this);
    action.connect_activate(move |_, parameter| {
        let Some(this) = weak.upgrade() else {
            return;
        };
        let Some(value) = parameter.and_then(|parameter| parameter.str()) else {
            return;
        };
        handler(&this, value);
    });
    window.add_action(&action);
}

pub fn open_window_for_path(app: &adw::Application, path: &Path) {
    open_window_for_path_with(
        app,
        path,
        SnapshotStore::default(),
        true,
        RecentDocuments::default(),
    );
}

/// Open `path` in a new window backed by `store`. `persist_settings` says
/// whether theme and font settings are written to disk; test windows pass
/// false. Cancelling the Convert Tabs prompt closes this new window.
pub(super) fn open_window_for_path_with(
    app: &adw::Application,
    path: &Path,
    store: SnapshotStore,
    persist_settings: bool,
    recent: RecentDocuments,
) {
    let window = AppWindow::with_parts(app, store, persist_settings, recent);
    window.window.present();
    AppWindow::open_path_after(&window, path, AfterOpen::CloseWindowOnCancel);
    window.editor_view.grab_focus();
}

pub fn open_empty_window(app: &adw::Application) {
    open_empty_window_with(
        app,
        SnapshotStore::default(),
        true,
        RecentDocuments::default(),
    );
}

/// Open an empty window backed by `store`. `persist_settings` says whether
/// theme and font settings are written to disk; test windows pass false.
fn open_empty_window_with(
    app: &adw::Application,
    store: SnapshotStore,
    persist_settings: bool,
    recent: RecentDocuments,
) {
    let window = AppWindow::with_parts(app, store, persist_settings, recent);
    window.window.present();
    window.editor_view.grab_focus();
}
