//! Menu models, shared menu actions and application-wide accelerators.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib::variant::ToVariant;
use gtk::{gio, glib};

use crate::core::pending::PendingAction;
use crate::ui::{about_dialog, app_dialog, display_path, find_in_files_dialog, run_script_dialog};

use super::diff_actions::AfterDiscardReview;
use super::{AppWindow, add_action};

pub(super) fn build_main_menu_button() -> gtk::MenuButton {
    let menu = build_main_menu();
    let button = gtk::MenuButton::new();
    button.set_icon_name("open-menu-symbolic");
    button.set_menu_model(Some(&menu));
    button
}

impl AppWindow {
    pub(super) fn connect_menu_actions(this: &Rc<Self>) {
        let window = &this.window;
        add_action(window, this, "manage-app", |this| {
            let weak = Rc::downgrade(this);
            app_dialog::present(
                &this.window,
                this.store.clone(),
                this.link_target(),
                this.persist_settings,
                move || {
                    if let Some(this) = weak.upgrade() {
                        this.invalidate_baseline_cache();
                        this.update_diff_button();
                    }
                },
            );
        });
        add_action(window, this, "find-in-files", |this| {
            let folder = find_in_files_dialog::default_folder_for(this.doc.borrow().path());
            let query = this.short_editor_selection();
            let weak = Rc::downgrade(this);
            find_in_files_dialog::present(
                &this.window,
                folder,
                query,
                super::dirty_open_buffers,
                move |path, jump| {
                    weak.upgrade()
                        .is_some_and(|this| this.open_search_match(path, jump))
                },
            );
        });
        add_action(window, this, "run-script", |this| {
            let current_file = this.doc.borrow().path().map(Path::to_path_buf);
            let weak_docs = Rc::downgrade(this);
            let weak_out = Rc::downgrade(this);
            run_script_dialog::present(
                &this.window,
                current_file,
                move || {
                    if let Some(this) = weak_docs.upgrade() {
                        this.open_reference_window(
                            "scripts.md",
                            include_str!("../../../docs/scripts.md"),
                        );
                    }
                },
                move |title, output| {
                    if let Some(this) = weak_out.upgrade() {
                        this.open_script_output_window(title, output);
                    }
                },
            );
        });
        add_action(window, this, "about", |this| {
            about_dialog::present(&this.window)
        });
        add_action(window, this, "copy-file-name", |this| this.copy_file_name());
        add_action(window, this, "copy-full-path", |this| this.copy_full_path());
        add_action(window, this, "close", |this| this.window.close());
        add_action(window, this, "quit", |this| this.quit_application());
    }

    /// Quit by handling every window at once through the normal close path:
    /// clean windows close immediately, while each dirty window and each
    /// window with accepted review changes shows its own prompt.
    fn quit_application(self: &Rc<Self>) {
        let Some(app) = self
            .window
            .application()
            .and_then(|app| app.downcast::<adw::Application>().ok())
        else {
            return;
        };
        finish_quit(&app);
    }
}

/// Start a quit pass over all open windows. Windows that are clean and have no
/// accepted review changes close immediately; the others show their prompts
/// without waiting for each other. Cancelling a prompt leaves that window open,
/// so the quit only completes once every window has closed.
pub(super) fn finish_quit(app: &adw::Application) {
    for window in super::WINDOWS.with_borrow(Clone::clone) {
        if window.flags.allow_close.get() || window.window.visible_dialog().is_some() {
            continue;
        }
        if window.flags.dirty.get() {
            window.require_saved(PendingAction::QuitApplication);
        } else if window.has_accepted_diff() {
            window.confirm_discard_review(AfterDiscardReview::Quit);
        } else {
            window.window.close();
        }
    }
    schedule_quit(app.clone());
}

/// Finish the quit once every window has been removed from the registry.
/// Prompted windows keep the application alive until they are resolved, so a
/// cancelled prompt leaves the remaining windows open. Waiting for idle lets
/// GTK remove closed windows from the registry first.
pub(super) fn schedule_quit(app: adw::Application) {
    glib::idle_add_local_once(move || {
        if super::WINDOWS.with_borrow(Vec::is_empty) {
            app.quit();
        }
    });
}

fn build_main_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let file = gio::Menu::new();
    file.append(Some("New File"), Some("win.new"));
    file.append(Some("New Window"), Some("win.new-window"));
    file.append(Some("Open…"), Some("win.open"));
    file.append(Some("Save"), Some("win.save"));
    file.append(Some("Save As…"), Some("win.save-as"));
    file.append(Some("Rename or Move…"), Some("win.rename"));
    menu.append_section(None, &file);

    let data = gio::Menu::new();
    data.append(Some("Manage App…"), Some("win.manage-app"));
    menu.append_section(None, &data);

    let tools = gio::Menu::new();
    tools.append(Some("Find…"), Some("win.find"));
    tools.append(Some("Find in Files…"), Some("win.find-in-files"));
    tools.append(Some("Replace…"), Some("win.replace"));
    tools.append(Some("Go to Line…"), Some("win.goto-line"));
    tools.append(Some("Word Wrap"), Some("win.word-wrap"));
    menu.append_section(None, &tools);

    let scripts = gio::Menu::new();
    scripts.append(Some("Run Script…"), Some("win.run-script"));
    menu.append_section(None, &scripts);

    let app = gio::Menu::new();
    app.append(Some("About notediff"), Some("win.about"));
    app.append(Some("Close Window"), Some("win.close"));
    app.append(Some("Quit"), Some("win.quit"));
    menu.append_section(None, &app);

    menu
}

/// Context menu for the header document title.
pub(super) fn build_title_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Copy file name"), Some("win.copy-file-name"));
    menu.append(Some("Copy full path"), Some("win.copy-full-path"));
    menu
}

/// The id used for the Open menu's path-entry row.
pub(super) const OPEN_LOCATION_ID: &str = "open-location";

/// The id used for a recent entry's custom two-line row.
pub(super) fn recent_row_id(index: usize) -> String {
    format!("recent-{index}")
}

/// The header Open button's menu: a path-entry row first, then recent paths
/// (custom two-line rows, newest first).
pub(super) fn build_open_menu(recent: &[PathBuf]) -> gio::Menu {
    let menu = gio::Menu::new();
    let location = gio::MenuItem::new(Some("Open location"), None);
    location.set_attribute_value("custom", Some(&OPEN_LOCATION_ID.to_variant()));
    menu.append_item(&location);

    if recent.is_empty() {
        return menu;
    }

    let files = gio::Menu::new();
    for (index, path) in recent.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let target = path.to_string_lossy().into_owned();
        let item = gio::MenuItem::new(Some(name), None);
        item.set_action_and_target_value(Some("win.open-recent"), Some(&target.to_variant()));
        item.set_attribute_value("custom", Some(&recent_row_id(index).to_variant()));
        files.append_item(&item);
    }
    menu.append_section(None, &files);

    menu
}

/// The Open menu's inline path field and Browse button.
#[derive(Clone)]
pub(super) struct OpenLocationRow {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    pub browse: gtk::Button,
    error_flash: Rc<super::flash::FlashState>,
}

const OPEN_LOCATION_ERROR_CLASS: &str = "notediff-open-location-error";

impl OpenLocationRow {
    /// Flash the focus ring red for one second. The entry stays focused and
    /// the menu stays open so the user can correct the path in place.
    pub(super) fn flash_error(&self) {
        self.entry.add_css_class(OPEN_LOCATION_ERROR_CLASS);
        let generation = self.error_flash.begin();
        let entry = self.entry.downgrade();
        let error_flash = self.error_flash.clone();
        glib::timeout_add_local_once(Duration::from_secs(1), move || {
            if error_flash.end(generation)
                && let Some(entry) = entry.upgrade()
            {
                entry.remove_css_class(OPEN_LOCATION_ERROR_CLASS);
            }
        });
    }

    /// Drop a stale error ring when the menu is reopened.
    pub(super) fn clear_error(&self) {
        self.error_flash.cancel();
        self.entry.remove_css_class(OPEN_LOCATION_ERROR_CLASS);
    }
}

pub(super) fn build_open_location_row() -> OpenLocationRow {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    root.set_margin_top(4);
    root.set_margin_bottom(4);
    root.set_margin_start(6);
    root.set_margin_end(6);

    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Type a path"));
    entry.set_width_chars(32);
    entry.set_hexpand(true);

    let browse = gtk::Button::with_label("Browse…");
    browse.add_css_class("flat");

    let entry_keys = gtk::EventControllerKey::new();
    entry_keys.set_name(Some("notediff-open-location-entry-keys"));
    entry_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let browse = browse.downgrade();
        let entry_ref = entry.downgrade();
        entry_keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Right
                && !modifiers.intersects(gtk::accelerator_get_default_mod_mask())
                && let (Some(entry), Some(browse)) = (entry_ref.upgrade(), browse.upgrade())
                && entry.text().is_empty()
            {
                browse.grab_focus();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
    }
    entry.add_controller(entry_keys);

    let browse_keys = gtk::EventControllerKey::new();
    browse_keys.set_name(Some("notediff-open-location-browse-keys"));
    browse_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let entry_ref = entry.downgrade();
        browse_keys.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Left
                && !modifiers.intersects(gtk::accelerator_get_default_mod_mask())
                && let Some(entry) = entry_ref.upgrade()
            {
                let _ = entry.child_focus(gtk::DirectionType::TabForward);
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
    }
    browse.add_controller(browse_keys);

    root.append(&entry);
    root.append(&browse);
    OpenLocationRow {
        root,
        entry,
        browse,
        error_flash: Rc::new(super::flash::FlashState::default()),
    }
}

/// The widgets of one custom recent entry: the clickable two-line open area,
/// its Shift-preview new-window icon, and its trailing hide/remove button.
#[derive(Clone)]
pub(super) struct RecentRow {
    pub root: gtk::Box,
    pub open: gtk::Button,
    pub new_window_icon: gtk::Image,
    pub trash: gtk::Button,
}

/// A custom two-line recent row: a bold file name and a smaller, dimmed
/// directory path. The directory is middle-ellipsized so a deep path cannot
/// widen the whole menu. The remove button fills the trailing edge.
pub(super) fn build_recent_row(path: &Path) -> RecentRow {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    root.set_hexpand(true);

    let open = gtk::Button::new();
    open.add_css_class("flat");
    open.set_halign(gtk::Align::Fill);
    open.set_hexpand(true);

    let home = glib::home_dir();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let directory = path
        .parent()
        .map(|parent| display_directory(parent, &home))
        .unwrap_or_default();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_hexpand(true);

    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    name_row.set_hexpand(true);

    let name_label = gtk::Label::new(Some(&name));
    name_label.set_xalign(0.0);
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name_label.set_max_width_chars(48);
    let attributes = gtk::pango::AttrList::new();
    attributes.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
    name_label.set_attributes(Some(&attributes));

    let new_window_icon = gtk::Image::from_icon_name("adw-external-link-symbolic");
    new_window_icon.add_css_class("open-new-window-icon");
    new_window_icon.set_valign(gtk::Align::Center);
    new_window_icon.set_visible(false);

    name_row.append(&name_label);
    name_row.append(&new_window_icon);

    let directory_label = gtk::Label::new(Some(&directory));
    directory_label.set_xalign(0.0);
    directory_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    directory_label.set_max_width_chars(56);
    directory_label.add_css_class("caption");
    directory_label.add_css_class("dim-label");

    content.append(&name_row);
    content.append(&directory_label);
    open.set_child(Some(&content));

    let trash = gtk::Button::from_icon_name("view-conceal-symbolic");
    trash.add_css_class("flat");
    trash.set_valign(gtk::Align::Center);
    trash.set_focus_on_click(false);

    root.append(&open);
    root.append(&trash);
    RecentRow {
        root,
        open,
        new_window_icon,
        trash,
    }
}

/// A directory label always ends with `/`, without doubling the root separator.
fn display_directory(path: &Path, home: &Path) -> String {
    let mut text = display_path(path, home);
    if !text.ends_with('/') {
        text.push('/');
    }
    text
}

/// The EOL conversion choices for the status-bar popover.
pub(super) fn build_eol_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Windows (CRLF)"), Some("win.convert-eol-crlf"));
    menu.append(Some("Unix (LF)"), Some("win.convert-eol-lf"));
    menu
}

/// The single Read Only choice of the status-bar mode popover.
pub(super) fn build_mode_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Read Only"), Some("win.read-only"));
    menu
}

/// Extra entries for the editor's right-click menu.
pub(super) fn build_context_menu() -> gio::Menu {
    let indentation = gio::Menu::new();
    indentation.append(Some("Increase"), Some("win.indent-increase-include-empty"));
    indentation.append(Some("Decrease"), Some("win.indent-decrease-no-flattening"));
    indentation.append(Some("Double"), Some("win.indent-double"));
    indentation.append(Some("Halve"), Some("win.indent-halve"));

    let path = gio::Menu::new();
    path.append(Some("File…"), Some("win.insert-path"));
    path.append(Some("Directory…"), Some("win.insert-folder-path"));

    let trim = gio::Menu::new();
    trim.append(Some("Leading"), Some("win.trim-leading-whitespace"));
    trim.append(Some("Trailing"), Some("win.trim-trailing-whitespace"));
    trim.append(
        Some("Blank Lines (strip)"),
        Some("win.trim-blank-lines-strip"),
    );
    trim.append(
        Some("Blank Lines (remove)"),
        Some("win.trim-blank-lines-remove"),
    );

    let edit_section = gio::Menu::new();
    edit_section.append_submenu(Some("Edit Indentation"), &indentation);
    let trim_item = gio::MenuItem::new_submenu(Some("Trim Whitespace"), &trim);
    trim_item.set_action_and_target_value(Some("win.trim-whitespace-menu"), None);
    edit_section.append_item(&trim_item);

    let menu = gio::Menu::new();
    menu.append_submenu(Some("Insert Path"), &path);
    menu.append(Some("Add Text from File…"), Some("win.insert-file"));
    menu.append_section(None, &edit_section);
    menu
}

pub(super) fn install_accels(app: &adw::Application) {
    app.set_accels_for_action("win.new", &["<Ctrl>n"]);
    app.set_accels_for_action("win.new-window", &["<Ctrl><Shift>n"]);
    app.set_accels_for_action("win.open", &["<Ctrl>o"]);
    app.set_accels_for_action("win.save", &["<Ctrl>s"]);
    app.set_accels_for_action("win.save-as", &["<Ctrl><Shift>s"]);
    app.set_accels_for_action("win.rename", &["F2"]);
    app.set_accels_for_action("win.enter-diff", &["F5"]);
    app.set_accels_for_action("win.manage-app", &["F4"]);
    app.set_accels_for_action("win.run-script", &["F6"]);
    app.set_accels_for_action("win.find", &["<Ctrl>f"]);
    app.set_accels_for_action("win.find-in-files", &["<Ctrl><Shift>f"]);
    app.set_accels_for_action("win.find-next", &["F3"]);
    app.set_accels_for_action("win.find-previous", &["<Shift>F3"]);
    app.set_accels_for_action("win.replace", &["<Ctrl>h"]);
    app.set_accels_for_action("win.goto-line", &["<Ctrl>g"]);
    app.set_accels_for_action(
        "win.indent-increase-include-empty",
        &["<Control>bracketright"],
    );
    app.set_accels_for_action(
        "win.indent-decrease-no-flattening",
        &["<Control>bracketleft"],
    );
    app.set_accels_for_action(
        "win.indent-double",
        &["<Control><Shift>bracketright", "<Control><Shift>braceright"],
    );
    app.set_accels_for_action(
        "win.indent-halve",
        &["<Control><Shift>bracketleft", "<Control><Shift>braceleft"],
    );
    app.set_accels_for_action("win.word-wrap", &["<Alt>z"]);
    app.set_accels_for_action("win.close", &["<Ctrl>w"]);
    app.set_accels_for_action("win.quit", &["<Ctrl>q"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_menu_offers_copy_actions() {
        let attribute = |menu: &gio::MenuModel, index: i32, name: &str| {
            menu.item_attribute_value(index, name, None)
                .and_then(|value| value.get::<String>())
        };
        let menu = build_title_menu();
        assert_eq!(menu.n_items(), 2);
        assert_eq!(
            attribute(menu.upcast_ref(), 0, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("Copy file name")
        );
        assert_eq!(
            attribute(menu.upcast_ref(), 0, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
            Some("win.copy-file-name")
        );
        assert_eq!(
            attribute(menu.upcast_ref(), 1, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("Copy full path")
        );
        assert_eq!(
            attribute(menu.upcast_ref(), 1, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
            Some("win.copy-full-path")
        );
    }

    #[test]
    fn context_menu_orders_insert_edit_and_trim_actions() {
        let attribute = |menu: &gio::MenuModel, index: i32, name: &str| {
            menu.item_attribute_value(index, name, None)
                .and_then(|value| value.get::<String>())
        };

        let menu = build_context_menu();
        assert_eq!(menu.n_items(), 3);

        // The first items are the Insert Path submenu and Add Text from File.
        assert_eq!(
            attribute(menu.upcast_ref(), 0, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("Insert Path")
        );
        let path = menu.item_link(0, gio::MENU_LINK_SUBMENU).unwrap();
        assert_eq!(path.n_items(), 2);
        for (index, (label, action)) in [
            ("File…", "win.insert-path"),
            ("Directory…", "win.insert-folder-path"),
        ]
        .iter()
        .enumerate()
        {
            let index = index as i32;
            assert_eq!(
                attribute(&path, index, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
                Some(*label)
            );
            assert_eq!(
                attribute(&path, index, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
                Some(*action)
            );
        }
        assert_eq!(
            attribute(menu.upcast_ref(), 1, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("Add Text from File…")
        );
        assert_eq!(
            attribute(menu.upcast_ref(), 1, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
            Some("win.insert-file")
        );

        // The bottom section has Edit Indentation, then Trim Whitespace.
        let edit_section = menu.item_link(2, gio::MENU_LINK_SECTION).unwrap();
        assert_eq!(edit_section.n_items(), 2);
        assert_eq!(
            attribute(&edit_section, 0, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("Edit Indentation")
        );
        let indentation = edit_section.item_link(0, gio::MENU_LINK_SUBMENU).unwrap();

        assert_eq!(indentation.n_items(), 4);
        for (index, (label, action)) in [
            ("Increase", "win.indent-increase-include-empty"),
            ("Decrease", "win.indent-decrease-no-flattening"),
            ("Double", "win.indent-double"),
            ("Halve", "win.indent-halve"),
        ]
        .iter()
        .enumerate()
        {
            let index = index as i32;
            assert_eq!(
                attribute(&indentation, index, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
                Some(*label)
            );
            assert_eq!(
                attribute(&indentation, index, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
                Some(*action)
            );
        }

        assert_eq!(
            attribute(&edit_section, 1, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("Trim Whitespace")
        );
        assert_eq!(
            attribute(&edit_section, 1, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
            Some("win.trim-whitespace-menu")
        );
        let trim = edit_section.item_link(1, gio::MENU_LINK_SUBMENU).unwrap();
        assert_eq!(trim.n_items(), 4);
        for (index, (label, action)) in [
            ("Leading", "win.trim-leading-whitespace"),
            ("Trailing", "win.trim-trailing-whitespace"),
            ("Blank Lines (strip)", "win.trim-blank-lines-strip"),
            ("Blank Lines (remove)", "win.trim-blank-lines-remove"),
        ]
        .iter()
        .enumerate()
        {
            let index = index as i32;
            assert_eq!(
                attribute(&trim, index, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
                Some(*label)
            );
            assert_eq!(
                attribute(&trim, index, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
                Some(*action)
            );
        }
    }

    #[test]
    fn app_data_entry_and_read_only_menu() {
        let action = |menu: &gio::MenuModel, index: i32| {
            menu.item_attribute_value(index, gio::MENU_ATTRIBUTE_ACTION, None)
                .and_then(|value| value.get::<String>())
        };
        let menu = build_main_menu();
        let section = menu.item_link(1, gio::MENU_LINK_SECTION).unwrap();
        assert_eq!(section.n_items(), 1);
        assert_eq!(action(&section, 0).as_deref(), Some("win.manage-app"));
        assert!(section.item_link(0, gio::MENU_LINK_SUBMENU).is_none());
        let mode = build_mode_menu();
        assert_eq!(
            action(mode.upcast_ref::<gio::MenuModel>(), 0).as_deref(),
            Some("win.read-only")
        );
        let tools = menu.item_link(2, gio::MENU_LINK_SECTION).unwrap();
        assert_eq!(action(&tools, 1).as_deref(), Some("win.find-in-files"));

        let scripts = menu.item_link(3, gio::MENU_LINK_SECTION).unwrap();
        assert_eq!(scripts.n_items(), 1);
        assert_eq!(action(&scripts, 0).as_deref(), Some("win.run-script"));

        let app = menu.item_link(4, gio::MENU_LINK_SECTION).unwrap();
        assert!(app.n_items() >= 3);
        assert_eq!(action(&app, app.n_items() - 1).as_deref(), Some("win.quit"));
    }

    #[test]
    fn file_menu_offers_new_file_and_new_window() {
        let attribute = |menu: &gio::MenuModel, index: i32, name: &str| {
            menu.item_attribute_value(index, name, None)
                .and_then(|value| value.get::<String>())
        };
        let menu = build_main_menu();
        let file = menu.item_link(0, gio::MENU_LINK_SECTION).unwrap();
        assert_eq!(
            attribute(&file, 0, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("New File")
        );
        assert_eq!(
            attribute(&file, 0, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
            Some("win.new")
        );
        assert_eq!(
            attribute(&file, 1, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
            Some("New Window")
        );
        assert_eq!(
            attribute(&file, 1, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
            Some("win.new-window")
        );
    }

    #[test]
    fn home_paths_are_shortened_for_display_only() {
        let home = Path::new("/home/you");
        assert_eq!(
            display_path(Path::new("/home/you/notes/todo.md"), home),
            "~/notes/todo.md"
        );
        assert_eq!(display_path(home, home), "~");
        assert_eq!(
            display_path(Path::new("/home/you2/notes/todo.md"), home),
            "/home/you2/notes/todo.md"
        );
        assert_eq!(
            display_path(Path::new("/tmp/notes/todo.md"), home),
            "/tmp/notes/todo.md"
        );
    }

    #[test]
    fn directory_labels_end_with_a_slash() {
        let home = Path::new("/home/you");
        assert_eq!(
            display_directory(Path::new("/home/you/notes"), home),
            "~/notes/"
        );
        assert_eq!(display_directory(home, home), "~/");
        assert_eq!(display_directory(Path::new("/"), home), "/");
        assert_eq!(
            display_directory(Path::new("/tmp/notes"), home),
            "/tmp/notes/"
        );
    }

    #[test]
    fn open_menu_always_offers_a_path_row() {
        let attribute = |menu: &gio::MenuModel, index: i32, name: &str| {
            menu.item_attribute_value(index, name, None)
                .and_then(|value| value.get::<String>())
        };
        let empty = build_open_menu(&[]);
        assert_eq!(empty.n_items(), 1);
        assert_eq!(
            attribute(
                empty.upcast_ref::<gio::MenuModel>(),
                0,
                gio::MENU_ATTRIBUTE_LABEL
            )
            .as_deref(),
            Some("Open location")
        );
        assert_eq!(
            attribute(empty.upcast_ref::<gio::MenuModel>(), 0, "custom").as_deref(),
            Some(OPEN_LOCATION_ID)
        );
        assert!(
            attribute(
                empty.upcast_ref::<gio::MenuModel>(),
                0,
                gio::MENU_ATTRIBUTE_ACTION
            )
            .is_none(),
            "the path row is a custom child, not a plain action item"
        );
    }

    #[test]
    fn open_menu_offers_location_and_recents() {
        let attribute = |menu: &gio::MenuModel, index: i32, name: &str| {
            menu.item_attribute_value(index, name, None)
                .and_then(|value| value.get::<String>())
        };
        let home = glib::home_dir();
        let first = home.join("notes/first.md");
        let second = home.join("notes/second.md");
        let menu = build_open_menu(&[second.clone(), first.clone()]);

        assert_eq!(
            attribute(
                menu.upcast_ref::<gio::MenuModel>(),
                0,
                gio::MENU_ATTRIBUTE_LABEL
            )
            .as_deref(),
            Some("Open location")
        );
        assert_eq!(
            attribute(menu.upcast_ref::<gio::MenuModel>(), 0, "custom").as_deref(),
            Some(OPEN_LOCATION_ID)
        );

        let files = menu.item_link(1, gio::MENU_LINK_SECTION).unwrap();
        assert_eq!(files.n_items(), 2);
        for (index, path) in [second, first].iter().enumerate() {
            let index = index as i32;
            assert_eq!(
                attribute(&files, index, gio::MENU_ATTRIBUTE_LABEL).as_deref(),
                path.file_name().and_then(|name| name.to_str())
            );
            assert_eq!(
                attribute(&files, index, gio::MENU_ATTRIBUTE_ACTION).as_deref(),
                Some("win.open-recent")
            );
            assert_eq!(
                attribute(&files, index, gio::MENU_ATTRIBUTE_TARGET).as_deref(),
                Some(path.to_string_lossy().as_ref())
            );
            let custom = recent_row_id(index as usize);
            assert_eq!(
                attribute(&files, index, "custom").as_deref(),
                Some(custom.as_str())
            );
        }

        assert_eq!(menu.n_items(), 2);
    }

    #[test]
    fn eol_conversion_is_only_in_the_status_bar_menu() {
        let menu = build_main_menu();
        for section in 0..menu.n_items() {
            let section = menu.item_link(section, gio::MENU_LINK_SECTION).unwrap();
            for index in 0..section.n_items() {
                assert_ne!(
                    section
                        .item_attribute_value(index, gio::MENU_ATTRIBUTE_LABEL, None)
                        .and_then(|value| value.get::<String>())
                        .as_deref(),
                    Some("EOL Conversion")
                );
                assert_ne!(
                    section
                        .item_attribute_value(index, gio::MENU_ATTRIBUTE_ACTION, None)
                        .and_then(|value| value.get::<String>())
                        .as_deref(),
                    Some("win.enter-diff")
                );
            }
        }
        let eol = build_eol_menu();
        assert_eq!(eol.n_items(), 2);
        assert_eq!(
            eol.item_attribute_value(0, gio::MENU_ATTRIBUTE_ACTION, None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.convert-eol-crlf")
        );
        assert_eq!(
            eol.item_attribute_value(1, gio::MENU_ATTRIBUTE_ACTION, None)
                .unwrap()
                .get::<String>()
                .as_deref(),
            Some("win.convert-eol-lf")
        );
    }
}
