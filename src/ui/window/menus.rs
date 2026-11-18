//! Menu models, shared menu actions and application-wide accelerators.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::variant::ToVariant;
use gtk::{gio, glib};

use crate::core::pending::PendingAction;
use crate::ui::{about_dialog, app_dialog};

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
        add_action(window, this, "about", |this| {
            about_dialog::present(&this.window)
        });
        add_action(window, this, "copy-file-name", |this| this.copy_file_name());
        add_action(window, this, "copy-full-path", |this| this.copy_full_path());
        add_action(window, this, "close", |this| this.window.close());
        add_action(window, this, "quit", |this| this.quit_application());
    }

    /// Quit by closing every window through the normal close path. Dirty
    /// documents are prompted one at a time; clean windows are not closed until
    /// every prompt has been resolved.
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

/// Prompt the first dirty window, or close every remaining window and quit.
/// Called again after each dirty window's save prompt has been resolved.
pub(super) fn finish_quit(app: &adw::Application) {
    let windows = super::WINDOWS.with_borrow(Clone::clone);
    if let Some(window) = windows
        .iter()
        .find(|window| window.flags.dirty.get() && !window.flags.allow_close.get())
    {
        window.require_saved(PendingAction::QuitApplication);
        return;
    }
    if let Some(window) = windows
        .iter()
        .find(|window| window.has_accepted_diff() && !window.flags.allow_close.get())
    {
        window.confirm_discard_review(AfterDiscardReview::Quit);
        return;
    }
    for window in &windows {
        if window.flags.allow_close.get() {
            continue;
        }
        window.flags.allow_close.set(true);
        window.window.close();
    }
    app.quit();
}

/// Resume a quit after the current window has finished closing. Waiting for
/// idle lets GTK remove it from the window registry first.
pub(super) fn schedule_quit(app: adw::Application) {
    glib::idle_add_local_once(move || finish_quit(&app));
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
    tools.append(Some("Replace…"), Some("win.replace"));
    tools.append(Some("Go to Line…"), Some("win.goto-line"));
    tools.append(Some("Word Wrap"), Some("win.word-wrap"));
    menu.append_section(None, &tools);

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

/// With no recent paths the header Open button opens the chooser directly
/// instead of showing a one-item menu.
pub(super) fn open_menu_has_entries(recent: &[PathBuf]) -> bool {
    !recent.is_empty()
}

/// The id used for a recent entry's custom two-line row.
pub(super) fn recent_row_id(index: usize) -> String {
    format!("recent-{index}")
}

/// The header Open button's menu: Browse first, then recent paths (custom
/// two-line rows, newest first).
pub(super) fn build_open_menu(recent: &[PathBuf]) -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some("Browse…"), Some("win.open"));

    if !open_menu_has_entries(recent) {
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

/// The widgets of one custom recent entry: the clickable two-line open area and
/// its trailing hide/remove button.
#[derive(Clone)]
pub(super) struct RecentRow {
    pub root: gtk::Box,
    pub open: gtk::Button,
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
    open.set_action_name(Some("win.open-recent"));
    open.set_action_target_value(Some(&path.to_string_lossy().to_variant()));

    let home = glib::home_dir();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let directory = path
        .parent()
        .map(|parent| display_directory(parent, &home))
        .unwrap_or_default();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.set_hexpand(true);

    let name_label = gtk::Label::new(Some(name));
    name_label.set_xalign(0.0);
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name_label.set_max_width_chars(48);
    let attributes = gtk::pango::AttrList::new();
    attributes.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
    name_label.set_attributes(Some(&attributes));

    let directory_label = gtk::Label::new(Some(&directory));
    directory_label.set_xalign(0.0);
    directory_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    directory_label.set_max_width_chars(56);
    directory_label.add_css_class("caption");
    directory_label.add_css_class("dim-label");

    content.append(&name_label);
    content.append(&directory_label);
    open.set_child(Some(&content));

    let trash = gtk::Button::from_icon_name("view-conceal-symbolic");
    trash.add_css_class("flat");
    trash.set_valign(gtk::Align::Center);
    trash.set_focus_on_click(false);

    root.append(&open);
    root.append(&trash);
    RecentRow { root, open, trash }
}

/// Shorten a path below the user's home directory for display. The stored path
/// and the action target remain absolute; only the menu label uses `~`.
fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
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
    let scale = gio::Menu::new();
    scale.append(Some("Double"), Some("win.indent-double"));
    scale.append(Some("Halve"), Some("win.indent-halve"));
    let insert = gio::Menu::new();
    insert.append(Some("Insert Text from File…"), Some("win.insert-file"));
    let menu = gio::Menu::new();
    menu.append_submenu(Some("Scale Indentation"), &scale);
    menu.append_section(None, &insert);
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
    app.set_accels_for_action("win.find", &["<Ctrl>f"]);
    app.set_accels_for_action("win.replace", &["<Ctrl>h"]);
    app.set_accels_for_action("win.goto-line", &["<Ctrl>g"]);
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
        let app = menu.item_link(3, gio::MENU_LINK_SECTION).unwrap();
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
    fn open_menu_is_skipped_without_recents() {
        assert!(!open_menu_has_entries(&[]));
        assert!(open_menu_has_entries(&[PathBuf::from(
            "/home/you/notes/one.md"
        )]));
    }

    #[test]
    fn open_menu_offers_browse_and_recents() {
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
            Some("Browse…")
        );
        assert_eq!(
            attribute(
                menu.upcast_ref::<gio::MenuModel>(),
                0,
                gio::MENU_ATTRIBUTE_ACTION
            )
            .as_deref(),
            Some("win.open")
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

        let empty = build_open_menu(&[]);
        assert_eq!(empty.n_items(), 1);
        assert_eq!(
            attribute(
                empty.upcast_ref::<gio::MenuModel>(),
                0,
                gio::MENU_ATTRIBUTE_ACTION
            )
            .as_deref(),
            Some("win.open")
        );
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
