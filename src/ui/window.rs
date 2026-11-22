//! The main application window.
//!
//! `AppWindow` holds shared state, builds the window and wires status updates.
//! Feature submodules wire their own signals and actions:
//!
//! - `clipboard`: header Copy Selected Text button, checkmark flash and
//!   last-focused-pane tracking.
//! - `editing`: insert filter for tabs and document line endings, Tab key,
//!   indentation editing, trailing-whitespace find and trim, EOL conversion,
//!   file insertion, word wrap and read-only state.
//! - `file_actions`: new file and new window, open, save, save as, rename or
//!   move, external-change handling and the save-then-continue flow.
//! - `flags`: per-window document flags and scoped loading guards.
//! - `flash`: generation-guarded temporary label/icon/error flashes.
//! - `focus`: in-window focus restoration after menus and dialogs close.
//! - `diff_actions`: the Diff Mode session wiring, Diff Mode keys, Create
//!   Baseline and Confirm.
//! - `menus`: menu construction, shared menu actions and accelerators.
//! - `search`: the search bar, Find, Replace and Go to Line.
//! - `status`: `StatusState` (measured text, last edit, refresh timer) and the
//!   status bar refresh methods.
//! - `theme_keys`: hidden Alt+T / Alt+Shift+T theme cycling.
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

use crate::core::LARGE_FILE_WARNING_BYTES;
use crate::core::document::Document;
use crate::core::pending::AfterOpen;
use crate::core::recent::RecentDocuments;
use crate::core::scripts::{self, ScriptOutput};
use crate::core::session::DiffSession;
use crate::core::snapshot_store::SnapshotStore;
use crate::core::tabs;
use crate::core::textstats::detect_line_ending;
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
/// `unreadable` is true when a baseline file exists on disk but failed to read,
/// so the Review button stays enabled and clicking it surfaces the read error.
#[derive(Default)]
struct BaselineCache {
    key: Option<(PathBuf, Option<SystemTime>)>,
    text: String,
    unreadable: bool,
}

/// Compose the window title: an optional dirty marker, the file name and an
/// optional read-only suffix. The title describes the document, not the view
/// mode, so entering Diff Mode does not change it.
fn title_text(name: &str, dirty: bool, read_only: bool) -> String {
    let dot = if dirty { "●  " } else { "" };
    let read_only = if read_only { " (Read-Only)" } else { "" };
    format!("{dot}{name}{read_only}")
}

/// Markup for the header title. The dirty dot is scaled down, and the file name
/// is escaped because the label is populated with markup.
fn title_markup(name: &str, dirty: bool, read_only: bool) -> String {
    let dot = if dirty {
        "<span size=\"80%\" rise=\"1.5pt\">●</span>  "
    } else {
        ""
    };
    let read_only = if read_only { " (Read-Only)" } else { "" };
    let escaped = glib::markup_escape_text(name);
    format!("{dot}{}{read_only}", escaped.as_str())
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
    /// Keeps the editor view's child observer alive so the lazily created
    /// context-menu popover can be hooked for focus restoration.
    editor_children: gio::ListModel,
    /// True once the editor's context-menu popover has been hooked.
    context_menu_hooked: Cell<bool>,
    status: StatusBar,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    search_controls: search::SearchControls,
    diff: Rc<DiffPage>,
    new_button: gtk::Button,
    open_button: gtk::Button,
    open_popover: gtk::PopoverMenu,
    /// Custom two-line rows currently installed in the Open menu.
    open_recent_rows: RefCell<Vec<menus::RecentRow>>,
    /// The Open menu's path field and Browse button, installed while the
    /// popover is built.
    open_location_row: RefCell<Option<menus::OpenLocationRow>>,
    /// True while Shift is held in the Open menu, showing the new-window icon
    /// on recent entries and routing activations to a new window.
    open_menu_shift: Cell<bool>,
    diff_button: gtk::Button,
    copy_button: gtk::Button,
    title_label: gtk::Label,
    title_popover: gtk::PopoverMenu,
    /// The "No Changes" label flash on the header button (see `flash`).
    flash: flash::FlashState,
    /// The checkmark flash on the copy button (see `flash`).
    copy_flash: flash::FlashState,
    doc: RefCell<Document>,
    /// Optional display/suggested name for an untitled document (e.g., timestamped
    /// script output). Cleared when a real file is loaded or saved.
    untitled_name: RefCell<Option<String>>,
    /// Canonical path reserved while the asynchronous Convert Tabs prompt is
    /// open, before the document itself is loaded into `doc`.
    pending_open: RefCell<Option<PathBuf>>,
    /// One-shot `(1-based line, 0-based char col, match char len)` jump applied
    /// after `load_document` completes (used by Find in Files).
    pending_jump: Cell<Option<(usize, usize, usize)>>,
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
        let title_label = gtk::Label::new(None);
        title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.set_title_widget(Some(&title_label));
        let title_popover = gtk::PopoverMenu::from_model(Some(&menus::build_title_menu()));
        title_popover.set_parent(&title_label);
        {
            let popover = title_popover.clone();
            title_label.connect_destroy(move |_| {
                if popover.parent().is_some() {
                    popover.unparent();
                }
            });
        }

        // A normal click starts New File; Shift-click opens a new window.
        let new_button = gtk::Button::from_icon_name("document-new-symbolic");
        new_button.set_tooltip_text(Some("New File (Ctrl+N)"));
        new_button.add_css_class("flat");
        header.pack_start(&new_button);

        // The Open button always opens its popover menu (path field, Browse…
        // and up to ten recent paths), built on demand when clicked.
        let open_button = gtk::Button::from_icon_name("document-open-symbolic");
        open_button.set_tooltip_text(Some("Open File (Ctrl+O)"));
        open_button.add_css_class("flat");
        let open_popover = gtk::PopoverMenu::from_model(None::<&gio::Menu>);
        open_popover.set_parent(&open_button);
        {
            let popover = open_popover.clone();
            open_button.connect_destroy(move |_| {
                if popover.parent().is_some() {
                    popover.unparent();
                }
            });
        }
        header.pack_start(&open_button);

        let copy_button = gtk::Button::from_icon_name(clipboard::COPY_ICON);
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

        // The text view creates its context-menu popover lazily as a child.
        // Keep the observer alive so focus restoration can hook it when it
        // first appears.
        let editor_children = editor_view.observe_children();

        let editor_scroller = gtk::ScrolledWindow::new();
        editor_scroller.set_child(Some(&editor_view));
        editor_scroller.set_hexpand(true);
        editor_scroller.set_vexpand(true);
        editor_scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);

        let status = StatusBar::editor();
        status.set_ending_menu(&menus::build_eol_menu());
        status.set_mode_menu(&menus::build_mode_menu());
        status.set_dot_actions(
            "win.find-next-trailing-whitespace",
            "win.find-next-non-empty-blank-line",
        );

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

        let (search_bar, search_entry, search_controls) = search::build_search_bar(&editor_buffer);

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
            editor_children,
            context_menu_hooked: Cell::new(false),
            status,
            search_bar,
            search_entry,
            search_controls,
            diff,
            new_button: new_button.clone(),
            open_button: open_button.clone(),
            open_popover: open_popover.clone(),
            open_recent_rows: RefCell::new(Vec::new()),
            open_location_row: RefCell::new(None),
            open_menu_shift: Cell::new(false),
            diff_button: diff_button.clone(),
            copy_button: copy_button.clone(),
            title_label: title_label.clone(),
            title_popover: title_popover.clone(),
            flash: flash::FlashState::default(),
            copy_flash: flash::FlashState::default(),
            doc: RefCell::new(Document::new()),
            untitled_name: RefCell::new(None),
            pending_open: RefCell::new(None),
            pending_jump: Cell::new(None),
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
        Self::connect_title_menu(&this);
        let mut popovers = Vec::new();
        if let Some(popover) = main_menu.popover() {
            popovers.push(popover);
        }
        popovers.push(this.open_popover.clone().upcast::<gtk::Popover>());
        popovers.push(this.title_popover.clone().upcast::<gtk::Popover>());
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
            this.record_user_edit();
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
        Self::connect_alt_indicator(this);
        Self::connect_theme_keys(this);
    }

    /// Show or hide the new-window icon on every recent row while Shift is
    /// held in the Open menu.
    fn set_open_menu_shift(&self, held: bool) {
        if self.open_menu_shift.replace(held) == held {
            return;
        }
        for row in self.open_recent_rows.borrow().iter() {
            row.new_window_icon.set_visible(held);
        }
    }

    /// Wire the Open popover's path-entry row and refill the menu from the
    /// pruned recent list on demand. This is the only place the recent state is
    /// pruned, so startup and ordinary editing never scan the remembered paths.
    fn connect_open_menu(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.open_popover.connect_visible_notify(move |popover| {
            if let Some(this) = weak.upgrade() {
                this.update_open_tooltip();
                if !popover.is_visible() {
                    this.set_open_menu_shift(false);
                }
            }
        });

        let weak = Rc::downgrade(this);
        this.window.connect_is_active_notify(move |window| {
            if !window.is_active()
                && let Some(this) = weak.upgrade()
            {
                this.set_open_menu_shift(false);
            }
        });

        let popover_keys = gtk::EventControllerKey::new();
        popover_keys.set_name(Some("notediff-open-menu-shift"));
        popover_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(this);
            popover_keys.connect_key_pressed(move |_, key, _, modifiers| {
                if (matches!(key, gtk::gdk::Key::Shift_L | gtk::gdk::Key::Shift_R)
                    || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK))
                    && let Some(this) = weak.upgrade()
                {
                    this.set_open_menu_shift(true);
                }
                glib::Propagation::Proceed
            });
        }
        {
            let weak = Rc::downgrade(this);
            popover_keys.connect_key_released(move |_, key, _, _| {
                if matches!(key, gtk::gdk::Key::Shift_L | gtk::gdk::Key::Shift_R)
                    && let Some(this) = weak.upgrade()
                {
                    this.set_open_menu_shift(false);
                }
            });
        }
        this.open_popover.add_controller(popover_keys);

        let row = menus::build_open_location_row();
        {
            let shift_enter = gtk::EventControllerKey::new();
            shift_enter.set_name(Some("notediff-open-location-shift-enter"));
            shift_enter.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(this);
            let entry = row.entry.downgrade();
            shift_enter.connect_key_pressed(move |_, key, _, modifiers| {
                let active_mods = modifiers & gtk::accelerator_get_default_mod_mask();
                if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
                    && active_mods == gtk::gdk::ModifierType::SHIFT_MASK
                    && let (Some(this), Some(entry)) = (weak.upgrade(), entry.upgrade())
                {
                    this.request_open_location_in(entry.text().as_str(), true);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            row.entry.add_controller(shift_enter);
        }
        {
            let weak = Rc::downgrade(this);
            row.entry.connect_activate(move |entry| {
                if let Some(this) = weak.upgrade() {
                    this.request_open_location(entry.text().as_str());
                }
            });
        }
        {
            let browse_shift = Rc::new(Cell::new(false));
            let gesture = gtk::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let pressed_shift = browse_shift.clone();
            gesture.connect_pressed(move |gesture, _, _, _| {
                pressed_shift.set(
                    gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::SHIFT_MASK),
                );
            });
            row.browse.add_controller(gesture);

            let weak = Rc::downgrade(this);
            row.browse.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    let new_window = browse_shift.replace(false) || this.open_menu_shift.get();
                    this.open_popover.popdown();
                    this.request_open_dialog_in(new_window);
                }
            });
        }
        *this.open_location_row.borrow_mut() = Some(row);

        let open_shift = Rc::new(Cell::new(false));
        {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let pressed_shift = open_shift.clone();
            gesture.connect_pressed(move |gesture, _, _, _| {
                pressed_shift.set(
                    gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::SHIFT_MASK),
                );
            });
            this.open_button.add_controller(gesture);
        }
        let weak = Rc::downgrade(this);
        this.open_button.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.open_popover.is_visible() {
                this.open_popover.popdown();
                return;
            }
            this.open_menu_shift.set(open_shift.replace(false));
            let entries = this.recent.pruned_entries();
            this.fill_open_menu(&entries);
            if let Some(row) = this.open_location_row.borrow().as_ref() {
                row.entry.set_text("");
                row.clear_error();
            }
            this.open_popover.popup();
        });
    }

    /// Right-clicking the document title opens the copy menu for saved files.
    fn connect_title_menu(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.title_popover.connect_visible_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.update_title_tooltip();
            }
        });
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(this);
        gesture.connect_pressed(move |gesture, _, x, y| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.doc.borrow().path().is_none() {
                return;
            }
            if this.title_popover.is_visible() {
                this.title_popover.popdown();
                return;
            }
            let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            this.title_popover.set_pointing_to(Some(&rect));
            this.title_popover.popup();
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        this.title_label.add_controller(gesture);
    }

    /// Install the recent rows into a freshly built menu model. Existing rows
    /// are detached first so a rebuild while the menu is open does not leave
    /// stale widgets behind.
    fn fill_open_menu(self: &Rc<Self>, entries: &[PathBuf]) {
        let old_rows = std::mem::take(&mut *self.open_recent_rows.borrow_mut());
        for row in old_rows {
            if row.root.parent().is_some() {
                row.root.unparent();
            }
        }
        if let Some(row) = self.open_location_row.borrow().as_ref()
            && row.root.parent().is_some()
        {
            row.root.unparent();
        }
        self.open_popover
            .set_menu_model(Some(&menus::build_open_menu(entries)));

        if let Some(row) = self.open_location_row.borrow().as_ref() {
            let _ = self
                .open_popover
                .add_child(&row.root, menus::OPEN_LOCATION_ID);
        }

        let mut rows = Vec::with_capacity(entries.len());
        let shift_held = self.open_menu_shift.get();
        for (index, path) in entries.iter().enumerate() {
            let row = menus::build_recent_row(path);
            row.new_window_icon.set_visible(shift_held);
            let _ = self
                .open_popover
                .add_child(&row.root, &menus::recent_row_id(index));

            let row_shift = Rc::new(Cell::new(false));
            let gesture = gtk::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let pressed_shift = row_shift.clone();
            gesture.connect_pressed(move |gesture, _, _, _| {
                pressed_shift.set(
                    gesture
                        .current_event_state()
                        .contains(gtk::gdk::ModifierType::SHIFT_MASK),
                );
            });
            row.open.add_controller(gesture);

            let weak = Rc::downgrade(self);
            let open_path = path.clone();
            row.open.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    let new_window = row_shift.replace(false) || this.open_menu_shift.get();
                    this.open_popover.popdown();
                    this.request_open_recent(open_path.clone(), new_window);
                }
            });

            let weak = Rc::downgrade(self);
            let delete_path = path.clone();
            row.trash.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.remove_recent_entry(&delete_path, index);
                }
            });
            rows.push(row);
        }
        *self.open_recent_rows.borrow_mut() = rows;
    }

    /// Remove one recent entry from its hide/remove button without confirmation.
    /// The open menu is rebuilt in place and focus moves to the next row, the
    /// previous row when the last entry was deleted, or the path entry when no
    /// recent entries remain.
    fn remove_recent_entry(self: &Rc<Self>, path: &Path, index: usize) {
        self.recent.remove(path);
        if !self.open_popover.is_visible() {
            return;
        }
        let entries = self.recent.pruned_entries();
        self.fill_open_menu(&entries);
        let rows = self.open_recent_rows.borrow();
        let focus = index.min(rows.len().saturating_sub(1));
        if let Some(row) = rows.get(focus).cloned() {
            glib::idle_add_local_once(move || {
                row.open.grab_focus();
            });
        } else if let Some(location) = self.open_location_row.borrow().clone() {
            glib::idle_add_local_once(move || {
                let _ = location.entry.child_focus(gtk::DirectionType::TabForward);
            });
        }
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

    /// Bookkeeping for a user-caused text change after the buffer has already
    /// been updated. Programmatic loads skip it; the unlock tab conversion
    /// calls it explicitly because that replacement is still a user request.
    fn record_user_edit(self: &Rc<Self>) {
        self.note_edit();
        self.flags.dirty.set(self.dirty_after_edit());
        self.update_title();
        self.enable_review_on_edit();
    }

    /// Any user edit makes the live text differ from an existing baseline, so
    /// enable Review immediately instead of waiting for the idle comparison.
    /// The next settle disables it again if the text returns to the baseline.
    fn enable_review_on_edit(&self) {
        if !self.diff_active() && self.baseline.borrow().key.is_some() {
            self.baseline_differs.set(true);
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
        cache.unreadable = false;
    }

    /// Reload the cached baseline when the open document's snapshot path or
    /// modification time changed. A missing snapshot clears the cache; an
    /// unreadable existing snapshot sets `unreadable` so the Review button
    /// stays clickable to report the error.
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
        if cache.key == key && !cache.unreadable {
            return;
        }
        let loaded = path
            .as_ref()
            .map(|path| self.store.read_for_canonical(path));
        match loaded {
            Some(Ok(Some(text))) => {
                cache.key = key;
                cache.text = text;
                cache.unreadable = false;
            }
            Some(Err(_)) => {
                cache.key = None;
                cache.text.clear();
                cache.unreadable = true;
            }
            Some(Ok(None)) | None => {
                cache.key = None;
                cache.text.clear();
                cache.unreadable = false;
            }
        }
    }

    /// True when a readable baseline is cached and `text` differs from it, or
    /// when an existing baseline could not be read (so clicking Review surfaces
    /// the error). Rust's string equality checks the byte length first, so a
    /// length change is an immediate mismatch.
    fn baseline_differs_from(&self, text: &str) -> bool {
        let cache = self.baseline.borrow();
        cache.unreadable || (cache.key.is_some() && cache.text != text)
    }

    fn baseline_exists(&self) -> bool {
        self.doc
            .borrow()
            .path()
            .is_some_and(|path| self.store.has_for_canonical(path))
    }

    fn update_title(&self) {
        let path = self.doc.borrow().path().map(Path::to_path_buf);
        let name = path
            .as_deref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .or_else(|| self.untitled_name.borrow().clone())
            .unwrap_or_else(|| "Untitled".to_string());
        let is_dirty = self.flags.dirty.get();
        // Besides the dot, gently tint the header bar (`install_css`).
        if is_dirty {
            self.window.add_css_class(theme::DIRTY_CLASS);
        } else {
            self.window.remove_css_class(theme::DIRTY_CLASS);
        }
        self.title_label
            .set_markup(&title_markup(&name, is_dirty, self.flags.read_only.get()));
        self.update_title_tooltip();
        self.window.set_title(Some(&title_text(
            &name,
            is_dirty,
            self.flags.read_only.get(),
        )));

        let has_path = path.is_some();
        for name in ["copy-file-name", "copy-full-path"] {
            self.set_action_enabled(name, has_path);
        }
    }

    pub(super) fn lookup_simple_action(&self, name: &str) -> Option<gio::SimpleAction> {
        self.window
            .lookup_action(name)
            .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
    }

    pub(super) fn set_action_enabled(&self, name: &str, enabled: bool) {
        if let Some(action) = self.lookup_simple_action(name) {
            action.set_enabled(enabled);
        }
    }

    /// The header title shows the full path on hover, except while its copy
    /// menu is showing; a tooltip over the menu would obscure its entries.
    fn update_title_tooltip(&self) {
        let tooltip = if self.title_popover.is_visible() {
            None
        } else {
            self.doc
                .borrow()
                .path()
                .map(|path| path.display().to_string())
        };
        self.title_label.set_tooltip_text(tooltip.as_deref());
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

    /// True when this window owns `canonical`, either as its loaded document
    /// or as a path reserved for a pending Convert Tabs prompt.
    fn owns_open_path(&self, canonical: &Path) -> bool {
        self.doc.borrow().path() == Some(canonical)
            || self.pending_open.borrow().as_deref() == Some(canonical)
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

    /// True when this window holds an empty, untitled document outside Diff Mode
    /// and can be reused directly for script output without losing anything.
    fn is_empty_untitled(&self) -> bool {
        self.doc.borrow().path().is_none()
            && self.editor_buffer.char_count() == 0
            && !self.diff_active()
    }

    /// Populate this window with a script's stdout and timestamped untitled
    /// name. Opens editable unless `stdout` contains tab characters, in which
    /// case it opens read-only under the no-tab policy.
    fn load_script_output(&self, title: String, output: ScriptOutput) {
        let large = output.stdout.len() as u64 > LARGE_FILE_WARNING_BYTES;
        let has_tabs = tabs::contains_tab(&output.stdout);
        self.doc
            .borrow_mut()
            .set_line_ending(detect_line_ending(&output.stdout));
        *self.untitled_name.borrow_mut() = Some(title);
        self.clear_edit();
        self.set_editor_text(&output.stdout);
        self.flags.dirty.set(!output.stdout.is_empty());
        self.set_read_only(has_tabs);
        self.reset_cursor_to_top();
        self.search_bar.set_search_mode(false);
        self.update_title();
        self.update_diff_button();
        self.update_status();
        self.editor_view.grab_focus();
        if large {
            self.toast(
                ToastKind::Warning,
                "Large output (>1 MiB): Diff Mode may be slow",
            );
        } else if has_tabs {
            self.toast(
                ToastKind::Warning,
                "Script output contains tab characters, so it was opened read-only",
            );
        } else if let Some(summary) = scripts::stderr_toast_summary(&output.stderr) {
            self.toast(ToastKind::Info, &summary);
        }
    }

    /// Return a short single-line selection from the editor (at most 256
    /// characters) for pre-populating Find in Files.
    pub(super) fn short_editor_selection(&self) -> Option<String> {
        if self.diff_active() {
            return None;
        }
        let (start, end) = self.editor_buffer.selection_bounds()?;
        let text = self.editor_buffer.text(&start, &end, false);
        let s = text.as_str();
        if s.is_empty() || s.contains(['\n', '\r', '\u{2029}']) || s.chars().count() > 256 {
            return None;
        }
        Some(s.to_string())
    }

    /// Scroll to 1-based `line` and select `[col_char .. col_char + match_chars]`.
    pub(super) fn jump_to_match(&self, line: usize, col_char: usize, match_chars: usize) {
        let line_count = self.editor_buffer.line_count().max(1);
        let line_idx = (line.saturating_sub(1) as i32).clamp(0, line_count - 1);
        let Some(line_start) = self.editor_buffer.iter_at_line(line_idx) else {
            return;
        };
        let line_chars = line_start.chars_in_line().max(0) as usize;
        let start_col = col_char.min(line_chars) as i32;
        let end_col = (col_char + match_chars).min(line_chars) as i32;
        if let (Some(start), Some(end)) = (
            self.editor_buffer.iter_at_line_offset(line_idx, start_col),
            self.editor_buffer.iter_at_line_offset(line_idx, end_col),
        ) {
            self.editor_buffer.select_range(&end, &start);
        } else {
            self.editor_buffer.place_cursor(&line_start);
        }
        let mark = self.editor_buffer.get_insert();
        self.editor_view.scroll_to_mark(&mark, 0.2, true, 0.0, 0.35);
        // For a freshly opened window whose TextView has not yet completed its
        // first layout/line-height pass, re-scroll on idle so the match is
        // centered once heights are known.
        let view = self.editor_view.downgrade();
        let buffer = self.editor_buffer.downgrade();
        glib::idle_add_local_once(move || {
            if let (Some(view), Some(buffer)) = (view.upgrade(), buffer.upgrade()) {
                let mark = buffer.get_insert();
                view.scroll_to_mark(&mark, 0.2, true, 0.0, 0.35);
            }
        });
    }

    /// Open a Find in Files match: raise the owning window if already open,
    /// reuse this window if empty and untitled, otherwise open a new window,
    /// and jump to the matching line and selection.
    pub(super) fn open_search_match(
        self: &Rc<Self>,
        path: &Path,
        line: usize,
        col_char: usize,
        match_chars: usize,
    ) -> bool {
        if let Some(target) = window_for_open_path(path) {
            let same_window = Rc::ptr_eq(&target, self);
            target.window.present();
            if !target.diff_active() {
                target.jump_to_match(line, col_char, match_chars);
                target.editor_view.grab_focus();
            }
            return same_window;
        }
        if let Some(app) = self
            .window
            .application()
            .and_then(|app| app.downcast::<adw::Application>().ok())
        {
            let window = AppWindow::with_parts(
                &app,
                self.store.clone(),
                self.persist_settings,
                self.recent.clone(),
            );
            window.pending_jump.set(Some((line, col_char, match_chars)));
            window.window.present();
            AppWindow::open_path_after(&window, path, AfterOpen::CloseWindowOnCancel);
        }
        false
    }

    /// Show a script's stdout: reuse this window if it is empty and untitled,
    /// otherwise open a new window. Non-empty output starts dirty so closing
    /// prompts Save As, matching ordinary untitled documents.
    pub(super) fn open_script_output_window(&self, title: String, output: ScriptOutput) {
        if self.is_empty_untitled() {
            self.load_script_output(title, output);
            self.window.present();
            return;
        }
        if let Some(app) = self
            .window
            .application()
            .and_then(|app| app.downcast::<adw::Application>().ok())
        {
            open_script_output_window_with(
                &app,
                self.store.clone(),
                self.persist_settings,
                self.recent.clone(),
                title,
                output,
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
        None,
    );
}

/// Return the window that already owns `path`, if any. Canonicalization
/// failure means the normal open can report the error instead.
fn window_for_open_path(path: &Path) -> Option<Rc<AppWindow>> {
    let canonical = path.canonicalize().ok()?;
    window_for_canonical_path(&canonical)
}

/// Return the window that owns an already-canonical path. Save As uses this
/// so a path that does not exist yet can be compared without requiring it to
/// exist first.
fn window_for_canonical_path(canonical: &Path) -> Option<Rc<AppWindow>> {
    WINDOWS.with_borrow(|windows| {
        windows
            .iter()
            .find(|window| window.owns_open_path(canonical))
            .cloned()
    })
}

/// Raise the window that already owns a path and explain why nothing opened.
/// `requester` is the window that asked to open the file, when there is one;
/// it gets its own toast if the owner is a different window.
fn present_duplicate_open(target: &Rc<AppWindow>, requester: Option<&Rc<AppWindow>>) {
    target.window.present();
    target.toast(ToastKind::Info, "File is already open in this window");
    if let Some(requester) = requester
        && !Rc::ptr_eq(requester, target)
    {
        requester.toast(ToastKind::Info, "File is already open in another window");
    }
}

/// Open `path` in a new window backed by `store`. `persist_settings` says
/// whether theme and font settings are written to disk; test windows pass
/// false. Cancelling the Convert Tabs prompt closes this new window.
/// `requester` is passed through for duplicate-path feedback when an existing
/// window already owns the file.
pub(super) fn open_window_for_path_with(
    app: &adw::Application,
    path: &Path,
    store: SnapshotStore,
    persist_settings: bool,
    recent: RecentDocuments,
    requester: Option<&Rc<AppWindow>>,
) {
    if let Some(target) = window_for_open_path(path) {
        present_duplicate_open(&target, requester);
        return;
    }
    let window = AppWindow::with_parts(app, store, persist_settings, recent);
    window.window.present();
    AppWindow::open_path_after(&window, path, AfterOpen::CloseWindowOnCancel);
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

/// Open a new window containing a script's stdout.
fn open_script_output_window_with(
    app: &adw::Application,
    store: SnapshotStore,
    persist_settings: bool,
    recent: RecentDocuments,
    title: String,
    output: ScriptOutput,
) -> Rc<AppWindow> {
    let window = AppWindow::with_parts(app, store, persist_settings, recent);
    window.load_script_output(title, output);
    window.window.present();
    window
}
