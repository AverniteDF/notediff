//! Display-dependent window smoke scenarios, run by one GTK test.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use adw::prelude::*;
use gtk::gio;
use gtk::glib::variant::ToVariant;
use sourceview5::prelude::*;

use crate::core::indent::IndentOperation;
use crate::core::inline::ChangeKind;
use crate::core::recent::RecentDocuments;
use crate::core::snapshot_store::{LinkMode, SnapshotStore};
use crate::core::textstats::LineEnding;
use crate::ui::status_bar::EditMode;
use crate::ui::theme;
use crate::ui::toast::ToastKind;

use super::file_actions::save_action_label;
use super::{
    AppWindow, CREATE_LABEL, REVIEW_LABEL, SAVE_AS_LABEL, initialize, review_enabled,
    save_as_suggested, title_markup, title_text,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "notediff-ui-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn untitled_save_prompt_offers_save_as() {
    assert_eq!(save_action_label(true), "Save");
    assert_eq!(save_action_label(false), "Save As");
}

#[test]
fn window_title_puts_the_dirty_marker_first() {
    assert_eq!(title_text("notes.md", false, false), "notes.md");
    assert_eq!(title_text("notes.md", true, false), "●  notes.md");
    assert_eq!(title_text("Untitled", false, false), "Untitled");
    assert_eq!(
        title_text("notes.txt", false, true),
        "notes.txt (Read-Only)"
    );
    assert_eq!(
        title_text("notes.txt", true, true),
        "●  notes.txt (Read-Only)"
    );
}

#[test]
fn header_title_scales_the_dirty_marker_and_escapes_the_name() {
    assert_eq!(title_markup("notes.md", false, false), "notes.md");
    assert_eq!(
        title_markup("notes.md", true, false),
        "<span size=\"80%\" rise=\"1.5pt\">●</span>  notes.md"
    );
    assert_eq!(
        title_markup("a&b.md", true, true),
        "<span size=\"80%\" rise=\"1.5pt\">●</span>  a&amp;b.md (Read-Only)"
    );
}

#[test]
fn review_button_is_enabled_only_for_pending_changes() {
    assert!(save_as_suggested(false));
    assert!(!save_as_suggested(true));

    assert!(review_enabled(false, false, false, false));
    assert!(review_enabled(true, false, false, false));
    assert!(review_enabled(true, true, true, false));
    assert!(!review_enabled(true, true, false, false));
    assert!(!review_enabled(true, true, true, true));
}

#[test]
#[ignore = "requires a display server"]
fn gtk_window_smoke() {
    gtk::init().unwrap();
    adw::init().unwrap();
    let theme_dir = test_dir();
    crate::ui::theme_catalog::smoke_at(&theme_dir.join("catalog"));
    let cola_dir = theme_dir.join("themes/cola-n-cream");
    std::fs::create_dir_all(&cola_dir).unwrap();
    std::fs::write(
        cola_dir.join("theme.conf"),
        include_str!("../../../examples/themes/cola-n-cream/theme.conf"),
    )
    .unwrap();
    std::fs::write(
        cola_dir.join("scheme.xml"),
        include_str!("../../../examples/themes/cola-n-cream/scheme.xml"),
    )
    .unwrap();
    theme::install_at(&theme_dir.join("themes"), None, &theme_dir.join("styles"));
    let state_path = theme_dir.join("selection");
    std::fs::write(&state_path, "theme = cola-n-cream\n").unwrap();
    theme::install_at(
        &theme_dir.join("themes"),
        Some(&state_path),
        &theme_dir.join("styles"),
    );
    assert_eq!(theme::current().id, "cola-n-cream");
    std::fs::write(&state_path, "theme = removed-theme\n").unwrap();
    theme::install_at(
        &theme_dir.join("themes"),
        Some(&state_path),
        &theme_dir.join("styles"),
    );
    assert_eq!(theme::current().id, "dark");
    assert!(
        theme::diagnostics()
            .iter()
            .any(|d| d.contains("removed-theme"))
    );
    assert_eq!(
        std::fs::read_to_string(&state_path).unwrap(),
        "theme = removed-theme\n"
    );
    theme::install_css();
    let app = adw::Application::builder()
        .application_id("io.github.AverniteDF.Test")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(initialize);
    app.register(None::<&gio::Cancellable>).unwrap();

    assert_eq!(
        app.accels_for_action("win.word-wrap").as_slice(),
        ["<Alt>z"]
    );
    assert_eq!(
        app.accels_for_action("win.close").as_slice(),
        ["<Control>w"]
    );
    assert_eq!(app.accels_for_action("win.quit").as_slice(), ["<Control>q"]);
    assert_eq!(app.accels_for_action("win.new").as_slice(), ["<Control>n"]);
    assert_eq!(
        app.accels_for_action("win.new-window").as_slice(),
        ["<Shift><Control>n"]
    );
    assert_eq!(app.accels_for_action("win.enter-diff").as_slice(), ["F5"]);
    assert_eq!(
        app.accels_for_action("win.indent-increase-include-empty")
            .as_slice(),
        ["<Control>bracketright"]
    );
    assert_eq!(
        app.accels_for_action("win.indent-decrease-no-flattening")
            .as_slice(),
        ["<Control>bracketleft"]
    );
    assert_eq!(
        app.accels_for_action("win.indent-double").as_slice(),
        ["<Shift><Control>bracketright", "<Shift><Control>braceright"]
    );
    assert_eq!(
        app.accels_for_action("win.indent-halve").as_slice(),
        ["<Shift><Control>bracketleft", "<Shift><Control>braceleft"]
    );
    let convert_tabs = super::file_actions::convert_tabs_dialog();
    assert_eq!(convert_tabs.default_response().as_deref(), Some("no"));
    assert_eq!(convert_tabs.close_response().as_str(), "cancel");
    assert_eq!(
        convert_tabs.response_appearance("no"),
        adw::ResponseAppearance::Suggested
    );
    assert_eq!(
        convert_tabs.response_appearance("yes"),
        adw::ResponseAppearance::Destructive
    );
    assert_eq!(
        convert_tabs.response_appearance("cancel"),
        adw::ResponseAppearance::Default
    );

    let dir = test_dir();
    // Baselines go to a temporary store, never the real one.
    let store = SnapshotStore::at(dir.join("store"));
    crate::ui::app_dialog::snapshots_tab::link_buttons_smoke(&store, &dir);
    transient_focus_scenario(&app, &store);
    context_menu_focus_scenario(&app, &store);
    trim_whitespace_scenario(&app, &store);
    gutter_selection_scenario(&app, store.clone());
    editing_scenario(&app, &dir, &store);
    indentation_scenario(&app, &dir, &store);
    trailing_whitespace_scenario(&app, &dir, &store);
    clipboard_scenario(&app, &dir, &store);
    title_menu_scenario(&app, &dir, &store);
    search_replace_scenario(&app, &dir, &store);
    file_actions_scenario(&app, &dir, &store);
    duplicate_open_scenario(&app, &dir, &store);
    save_as_duplicate_scenario(&app, &dir, &store);
    plain_text_scenario(&app, &dir, &store);
    document_state_scenario(&app, &dir, &store);
    save_external_scenario(&app, &dir, &store);
    quit_scenario(&app, store.clone());
    quit_review_scenario(&app, &dir, &store);
    open_file_save_prompt_scenario(&app, &dir, &store);
    recent_menu_scenario(&app, &dir, &store);
    recent_trash_scenario(&app, &dir, &store);
    new_file_scenario(&app, &dir, &store);
    diff_file_actions_disabled_scenario(&app, &dir, &store);
    theme_scenario(&app, &store);
    font_appearance_scenario(&app, &store);
    restore_appearance_scenario(&app, &store);
    external_change_in_diff_scenario(&app, &dir, &store);
    diff_actions_scenario(&app, &dir, &store);
    review_exit_scenario(&app, &dir, &store);
    partial_confirm_scenario(&app, &dir, &store);
    inspection_scenario(&app, &dir, &store);
    inline_inspection_scenario(&app, &dir, &store);
    whitespace_highlight_scenario(&app, &dir, &store);
    caret_navigation_scenario(&app, &dir, &store);
    blank_marker_scenario(&app, &dir, &store);
    full_boundary_marker_scenario(&app, &dir, &store);
    full_boundary_marker_after_accept_scenario(&app, &dir, &store);
    full_boundary_marker_scroll_scenario(&app, &dir, &store);
    focused_line_number_scenario(&app, &dir, &store);
    trailing_blank_marker_scenario(&app, &dir, &store);
    long_diff_scroll_scenario(&app, &dir, &store);
    assert!(app.windows().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(theme_dir).unwrap();
}

fn gutter_selection_scenario(app: &adw::Application, store: SnapshotStore) {
    let window = AppWindow::with_store(app, store);
    window.set_editor_text("one\ntwo\nthree\nfour\n");
    window.window.present();
    gtk::test_widget_wait_for_draw(&window.window);

    let view = &window.editor_view;
    let buffer = view.buffer();
    let gutter = sourceview5::prelude::ViewExt::gutter(view, gtk::TextWindowType::Left);
    let bounds = gutter.compute_bounds(view).expect("gutter bounds");
    let x = f64::from(bounds.x()) + f64::from(bounds.width()) / 2.0;
    let line_y = |line: i32| {
        let (y, height) = view.line_yrange(&buffer.iter_at_line(line).unwrap());
        f64::from(y) + f64::from(height) / 2.0
    };
    let controllers = view.observe_controllers();
    let click = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|object| object.downcast::<gtk::GestureClick>().ok())
        .expect("gutter click gesture");
    let motion = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|object| object.downcast::<gtk::EventControllerMotion>().ok())
        .expect("gutter motion controller");

    click.emit_by_name::<()>("pressed", &[&1i32, &x, &line_y(1)]);
    assert_eq!(selected_text(&buffer).as_deref(), Some("two\n"));
    motion.emit_by_name::<()>("motion", &[&x, &line_y(2)]);
    assert_eq!(selected_text(&buffer).as_deref(), Some("two\nthree\n"));
    click.emit_by_name::<()>("released", &[&1i32, &x, &line_y(2)]);

    // Gesture events use widget coordinates, so a scrolled document must
    // resolve the line under the pointer, not the same y from the buffer top.
    let text: String = (1..=200).map(|n| format!("line {n}\n")).collect();
    window.set_editor_text(&text);
    gtk::test_widget_wait_for_draw(&window.window);

    let adjustment = view.vadjustment().expect("vertical adjustment");
    // The text view refreshes its scroll range on idle after a text change.
    wait_for(|| adjustment.upper() > adjustment.page_size() + 100.0);
    let target = 100;
    let desired = line_y(target) - 20.0;
    adjustment.set_value(desired);
    wait_for(|| (adjustment.value() - desired).abs() < 0.5);
    assert!(adjustment.value() > 0.0);
    let visible_y = |line: i32| line_y(line) - adjustment.value();
    let pointer_y = visible_y(target);
    assert!(
        pointer_y >= 0.0 && pointer_y < adjustment.page_size(),
        "target line should be inside the viewport"
    );

    click.emit_by_name::<()>("pressed", &[&1i32, &x, &pointer_y]);
    assert_eq!(selected_text(&buffer).as_deref(), Some("line 101\n"));
    motion.emit_by_name::<()>("motion", &[&x, &visible_y(target + 2)]);
    assert_eq!(
        selected_text(&buffer).as_deref(),
        Some("line 101\nline 102\nline 103\n")
    );
    click.emit_by_name::<()>("released", &[&1i32, &x, &visible_y(target + 2)]);
    window.window.destroy();
}

fn selected_text(buffer: &gtk::TextBuffer) -> Option<String> {
    let (start, end) = buffer.selection_bounds()?;
    Some(buffer.text(&start, &end, true).to_string())
}

fn assert_toast(window: &AppWindow, kind: ToastKind, text: &str) {
    let toast = window.last_toast.borrow().as_ref().unwrap().clone();
    assert_eq!(
        toast.title().as_deref(),
        Some(crate::ui::toast::title(kind, text).as_str())
    );
    toast.dismiss();
}

fn assert_timestamp(text: &str) {
    assert_eq!(text.chars().count(), 16, "{text}");
    for (index, ch) in text.chars().enumerate() {
        match index {
            4 | 7 => assert_eq!(ch, '-', "{text}"),
            10 => assert_eq!(ch, ' ', "{text}"),
            13 => assert_eq!(ch, ':', "{text}"),
            _ => assert!(ch.is_ascii_digit(), "{text}"),
        }
    }
}

fn clipboard_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut widgets = vec![root.clone()];
        let mut index = 0;
        while index < widgets.len() {
            let mut child = widgets[index].first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                widgets.push(widget);
            }
            index += 1;
        }
        widgets
    }
    fn select(view: &sourceview5::View, start: i32, end: i32) {
        view.grab_focus();
        let buffer = view.buffer();
        buffer.select_range(&buffer.iter_at_offset(start), &buffer.iter_at_offset(end));
    }
    fn clipboard_text(window: &AppWindow) -> String {
        gtk::glib::MainContext::default()
            .block_on(window.window.clipboard().read_text_future())
            .unwrap()
            .unwrap()
            .to_string()
    }
    let path = dir.join("clipboard.txt");
    std::fs::write(&path, "editor αβ\nsecond line\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("changed γδ\nsecond line\n");
    window.persist(None).unwrap();
    window.window.present();
    let buttons: Vec<gtk::Button> = descendants(window.window.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast().ok())
        .collect();
    let copy = buttons
        .iter()
        .find(|button| button.icon_name().as_deref() == Some("edit-copy-symbolic"))
        .unwrap();
    let open = &window.open_button;
    assert_eq!(
        open.next_sibling().as_ref(),
        Some(copy.upcast_ref::<gtk::Widget>())
    );
    assert_eq!(
        copy.tooltip_text().as_deref(),
        Some("Copy Selected Text (Ctrl+C)")
    );
    assert!(!copy.gets_focus_on_click());
    let clock = std::time::Instant::now();
    window.status_state.note_edit(
        clock - std::time::Duration::from_secs(5),
        std::time::SystemTime::now(),
    );

    select(&window.editor_view, 8, 10);
    // Keyboard activation can focus the button without losing the source pane.
    copy.grab_focus();
    copy.emit_clicked();
    assert_eq!(copy.icon_name().as_deref(), Some("object-select-symbolic"));
    assert_eq!(clipboard_text(&window), "γδ");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 2 characters to clipboard",
    );
    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));
    assert_eq!(
        window
            .editor_buffer
            .selection_bounds()
            .map(|(a, b)| (a.offset(), b.offset())),
        Some((8, 10))
    );
    assert!(!window.flags.dirty.get());

    // Copying the title path uses the same checkmark feedback and no toast.
    window.last_toast.take();
    window.copy_file_name();
    assert_eq!(copy.icon_name().as_deref(), Some("object-select-symbolic"));
    assert_eq!(clipboard_text(&window), "clipboard.txt");
    assert!(window.last_toast.borrow().is_none());
    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));

    // Outside Diff Mode an empty editor selection selects and copies the whole
    // document, leaving it selected so the copied range is visible.
    select(&window.editor_view, 0, 0);
    // Let the queued focus move settle before the button runs; otherwise the
    // focus change can clear the selection the button installs.
    wait_for(|| window.editor_view.is_focus());
    copy.emit_clicked();
    assert_eq!(copy.icon_name().as_deref(), Some("object-select-symbolic"));
    assert_eq!(clipboard_text(&window), "changed γδ\nsecond line\n");
    assert_eq!(
        window
            .editor_buffer
            .selection_bounds()
            .map(|(a, b)| (a.offset(), b.offset())),
        Some((0, 23))
    );
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 23 characters (2 lines) to clipboard",
    );

    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));
    // An empty document selects nothing, so the warning, clipboard and icon
    // stay as they are.
    window.set_editor_text("");
    select(&window.editor_view, 0, 0);
    window.window.clipboard().set_text("keep me");
    copy.emit_clicked();
    assert_toast(&window, ToastKind::Warning, "No selected text to copy!");
    assert_eq!(clipboard_text(&window), "keep me");
    assert_eq!(copy.icon_name().as_deref(), Some("edit-copy-symbolic"));
    window.set_editor_text("changed γδ\nsecond line\n");

    select(&window.editor_view, 0, 1);
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "c");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 1 character to clipboard",
    );
    select(&window.editor_view, 0, 11);
    copy.emit_clicked();
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 11 characters to clipboard",
    );
    select(&window.editor_view, 0, window.editor_buffer.char_count());
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "changed γδ\nsecond line\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 23 characters (2 lines) to clipboard",
    );

    window.set_read_only(true);
    assert!(
        window
            .window
            .title()
            .is_some_and(|title| title.contains("(Read-Only)"))
    );
    select(&window.editor_view, 8, 10);
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "γδ");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 2 characters to clipboard",
    );
    // Read-only editors still select all when nothing is selected.
    select(&window.editor_view, 0, 0);
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "changed γδ\nsecond line\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 23 characters (2 lines) to clipboard",
    );
    select(&window.editor_view, 8, 10);
    window.set_read_only(false);
    assert!(
        window
            .window
            .title()
            .is_none_or(|title| !title.contains("(Read-Only)"))
    );
    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));
    window.last_toast.take();
    window.editor_view.emit_by_name::<()>("copy-clipboard", &[]);
    assert_eq!(copy.icon_name().as_deref(), Some("object-select-symbolic"));
    assert_eq!(clipboard_text(&window), "γδ");
    assert!(
        window.last_toast.borrow().is_none(),
        "native Ctrl+C must not gain a toast"
    );
    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));

    window.enter_diff();
    select(window.diff.active_left_view(), 0, 7);
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "changed");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 7 characters to clipboard",
    );
    select(window.diff.active_right_view(), 0, 6);
    copy.grab_focus();
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "editor");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 6 characters to clipboard",
    );
    // Do not silently copy from the other pane's stale selection.
    select(window.diff.active_right_view(), 0, 0);
    copy.emit_clicked();
    assert_toast(&window, ToastKind::Warning, "No selected text to copy!");
    assert_eq!(clipboard_text(&window), "editor");

    // Native pane copies omit display-only markers and flash the icon too.
    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));
    let left = window.diff.active_left_view();
    select(left, 0, 7);
    left.emit_by_name::<()>("copy-clipboard", &[]);
    assert_eq!(copy.icon_name().as_deref(), Some("object-select-symbolic"));
    assert_eq!(clipboard_text(&window), "changed");
    wait_for(|| copy.icon_name().as_deref() == Some("edit-copy-symbolic"));

    window.focus_relative(1);
    window.diff.inspect_button.emit_clicked();
    // Inline review highlighting is not a user text selection, nor should a
    // selection in a hidden full-document view be copied.
    let hidden = window.diff.left_view.buffer();
    hidden.select_range(&hidden.start_iter(), &hidden.iter_at_offset(7));
    copy.emit_clicked();
    assert_toast(&window, ToastKind::Warning, "No selected text to copy!");
    select(window.diff.active_left_view(), 8, 10);
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "γδ");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 2 characters to clipboard",
    );
    select(window.diff.active_right_view(), 7, 9);
    copy.grab_focus();
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "αβ");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 2 characters to clipboard",
    );
    window.diff.exit_button.emit_clicked();
    assert!(window.diff_active());
    assert!(!window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Close"));
    assert!(window.diff.exit_button.tooltip_text().is_none());
    window.diff.exit_button.emit_clicked();
    assert!(!window.diff_active());
    window.editor_buffer.select_range(
        &window.editor_buffer.iter_at_offset(8),
        &window.editor_buffer.iter_at_offset(10),
    );
    copy.emit_clicked();
    assert_eq!(clipboard_text(&window), "γδ");
    assert_toast(
        &window,
        ToastKind::Success,
        "Copied 2 characters to clipboard",
    );
    assert_eq!(window.current_text(), "changed γδ\nsecond line\n");
    assert!(!window.flags.dirty.get());
    assert_eq!(
        window.status_state.last_edit_text(clock).as_deref(),
        Some("5 seconds ago")
    );
    window.window.destroy();
}

fn long_diff_scroll_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    fn settle() {
        let context = gtk::glib::MainContext::default();
        for _ in 0..40 {
            for _ in 0..100 {
                if !context.pending() {
                    break;
                }
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    fn assert_focus_visible(window: &AppWindow) {
        let state = window.diff_state.borrow();
        let session = state.as_ref().unwrap();
        let hunk = &session.diff().hunks[session.focus().unwrap()];
        for (view, line) in [
            (&window.diff.left_view, hunk.new_start),
            (&window.diff.right_view, hunk.old_start),
        ] {
            let iter = view.buffer().iter_at_line(line as i32).unwrap();
            let location = view.iter_location(&iter);
            let visible = view.visible_rect();
            assert!(
                location.y() >= visible.y() && location.y() < visible.y() + visible.height(),
                "hunk at line {line} (y={}) outside viewport {}..{}",
                location.y(),
                visible.y(),
                visible.y() + visible.height()
            );
        }
    }
    let baseline: String = (0..8000)
        .map(|i| {
            if i == 2000 {
                format!("line {i:05} {}\n", "x".repeat(400))
            } else {
                format!("line {i:05}\n")
            }
        })
        .collect();
    let live = baseline
        .replace(
            "line 02500\n",
            &format!("changed first\n{}", "extra line\n".repeat(80)),
        )
        .replace("line 06000\n", "changed second\n");
    let path = dir.join("long-scroll.md");
    std::fs::write(&path, &baseline).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text(&live);
    window.window.present();
    window.enter_diff();
    settle();
    window.focus_relative(1);
    settle();
    assert_focus_visible(&window);

    // Hunk navigation realigns both panes vertically and resets horizontal.
    let left_horizontal = window.diff.left_view.hadjustment().unwrap();
    let right_horizontal = window.diff.right_view.hadjustment().unwrap();
    left_horizontal.set_value(64.0);
    right_horizontal.set_value(48.0);
    assert!(left_horizontal.value() > 0.0);
    assert!(right_horizontal.value() > 0.0);
    window.focus_relative(1);
    settle();
    assert_eq!(left_horizontal.value(), 0.0);
    assert_eq!(right_horizontal.value(), 0.0);
    assert_focus_visible(&window);
    window.focus_relative(-1);
    settle();
    assert_focus_visible(&window);

    let live_changes = Rc::new(std::cell::Cell::new(0));
    let changes = live_changes.clone();
    window
        .diff
        .left_view
        .buffer()
        .connect_changed(move |_| changes.set(changes.get() + 1));
    let largest_baseline_edit = Rc::new(std::cell::Cell::new(0usize));
    let largest = largest_baseline_edit.clone();
    window
        .diff
        .right_view
        .buffer()
        .connect_delete_range(move |_, start, end| {
            largest.set(largest.get().max((end.offset() - start.offset()) as usize));
        });
    let largest = largest_baseline_edit.clone();
    window
        .diff
        .right_view
        .buffer()
        .connect_insert_text(move |_, _, text| {
            largest.set(largest.get().max(text.chars().count()));
        });
    for _ in 0..3 {
        window.accept_focused();
        settle();
        assert_focus_visible(&window);
        window.undo_accept();
        settle();
        assert_focus_visible(&window);
    }
    // A newer Undo request must also win when Accept has not reached a frame yet.
    window.accept_focused();
    window.undo_accept();
    settle();
    assert_focus_visible(&window);
    assert_eq!(
        live_changes.get(),
        0,
        "Accept/Undo must not replace unchanged live text"
    );
    assert!(
        largest_baseline_edit.get() < 2000,
        "Accept/Undo must edit the changed baseline range, not replace the whole document"
    );
    // Full panes scroll independently after programmatic placement settles.
    let left = window.diff.left_view.vadjustment().unwrap();
    let right = window.diff.right_view.vadjustment().unwrap();
    let right_before = right.value();
    left.set_value(left.value() + 100.0);
    assert!(left.value() > right_before);
    assert!((right.value() - right_before).abs() < 1.0);
    let left_horizontal = window.diff.left_view.hadjustment().unwrap();
    let right_horizontal = window.diff.right_view.hadjustment().unwrap();
    let right_horizontal_before = right_horizontal.value();
    left_horizontal.set_value(left_horizontal.value() + 64.0);
    assert!(left_horizontal.value() > 0.0);
    assert!((right_horizontal.value() - right_horizontal_before).abs() < 1.0);
    window.diff.accept_all_button.emit_clicked();
    settle();
    window.undo_accept();
    settle();
    assert_focus_visible(&window);

    // The full views stay instantiated, populated and positioned throughout
    // inspection. A plain toggle must not edit their buffers or queue a scroll.
    let keys = (0..window.window.observe_controllers().n_items())
        .filter_map(|index| window.window.observe_controllers().item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-diff-keys"))
        .unwrap();
    let press = |key: gtk::gdk::Key| {
        keys.emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
        )
    };
    window.focus_relative(1);
    settle();
    assert_focus_visible(&window);
    let full_buffers = [
        window.diff.left_view.buffer(),
        window.diff.right_view.buffer(),
    ];
    let toggle_edits = Rc::new(std::cell::Cell::new(0));
    for buffer in &full_buffers {
        let edits = toggle_edits.clone();
        buffer.connect_changed(move |_| edits.set(edits.get() + 1));
    }
    let visible_frames = Rc::new(std::cell::Cell::new(0));
    let frames = visible_frames.clone();
    let weak = Rc::downgrade(&window);
    window.diff.root.add_tick_callback(move |_, _| {
        let Some(window) = weak.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        let full = window
            .diff_state
            .borrow()
            .as_ref()
            .is_some_and(|session| !session.is_inspecting() && session.focus().is_some());
        if full && window.diff.panes_presented() {
            assert_focus_visible(&window);
            frames.set(frames.get() + 1);
        }
        gtk::glib::ControlFlow::Continue
    });
    let mut return_time = std::time::Duration::ZERO;
    for _ in 0..4 {
        let positions = [left.value(), right.value()];
        let scroll_requests = window.diff.full_scroll_requests();
        assert!(press(gtk::gdk::Key::Return));
        assert!(window.diff.panes_presented());
        assert_ne!(window.diff.active_left_view().buffer(), full_buffers[0]);
        assert_ne!(window.diff.active_right_view().buffer(), full_buffers[1]);
        assert_eq!(window.diff.left_view.buffer(), full_buffers[0]);
        assert_eq!(window.diff.right_view.buffer(), full_buffers[1]);
        assert!((left.value() - positions[0]).abs() < 1.0);
        assert!((right.value() - positions[1]).abs() < 1.0);
        wait_for(|| window.diff.panes_ready());
        let start = std::time::Instant::now();
        assert!(press(gtk::gdk::Key::Escape));
        return_time += start.elapsed();
        assert!(window.diff.panes_presented());
        assert!(
            window.diff.panes_ready(),
            "an unchanged return must not wait for placement frames"
        );
        assert_eq!(window.diff.active_left_view().buffer(), full_buffers[0]);
        assert_eq!(window.diff.active_right_view().buffer(), full_buffers[1]);
        assert_eq!(window.diff.full_scroll_requests(), scroll_requests);
        assert_eq!(toggle_edits.get(), 0);
        assert!((left.value() - positions[0]).abs() < 1.0);
        assert!((right.value() - positions[1]).abs() < 1.0);
        assert_focus_visible(&window);
        let frames_before = visible_frames.get();
        wait_for(|| visible_frames.get() > frames_before);
    }
    eprintln!(
        "8,000-line full-diff return: {:?} average; no buffer edits or scroll requests",
        return_time / 4
    );
    assert!(visible_frames.get() > 0);

    // Inspection navigation changes the return target, rather than restoring
    // the viewport saved for the previous hunk.
    assert!(press(gtk::gdk::Key::Return));
    assert!(press(gtk::gdk::Key::Up));
    assert!(press(gtk::gdk::Key::Escape));
    assert_focus_visible(&window);
    wait_for(|| window.diff.panes_ready());
    assert_focus_visible(&window);

    // Keep an intentionally off-centre viewport, including horizontal scroll,
    // rather than merely centring the same hunk again on every return.
    left.set_value(left.value() + 37.0);
    let horizontal = window.diff.left_view.hadjustment().unwrap();
    horizontal.set_value(64.0);
    assert!(horizontal.value() > 0.0);
    let positions = [left.value(), right.value(), horizontal.value()];
    assert_focus_visible(&window);
    assert!(press(gtk::gdk::Key::Return));
    wait_for(|| window.diff.panes_ready());
    assert!(press(gtk::gdk::Key::Escape));
    wait_for(|| window.diff.panes_ready());
    assert!((left.value() - positions[0]).abs() < 1.0);
    assert!((right.value() - positions[1]).abs() < 1.0);
    assert!((horizontal.value() - positions[2]).abs() < 1.0);

    // Accept and Undo can change baseline line coordinates during inspection.
    largest_baseline_edit.set(0);
    assert!(press(gtk::gdk::Key::Return));
    assert!(press(gtk::gdk::Key::Right));
    assert_eq!(
        live_changes.get(),
        0,
        "inspection accepts must retain the full live buffer"
    );
    assert!(press(gtk::gdk::Key::Escape));
    wait_for(|| window.diff.panes_ready());
    assert_focus_visible(&window);
    assert!(press(gtk::gdk::Key::Return));
    assert!(press(gtk::gdk::Key::Left));
    assert!(press(gtk::gdk::Key::Escape));
    wait_for(|| window.diff.panes_ready());
    assert_focus_visible(&window);

    // Superseded transition callbacks must not expose an obsolete viewport.
    assert!(
        largest_baseline_edit.get() < 2000,
        "inspection Accept/Undo must incrementally update the retained baseline"
    );
    for _ in 0..3 {
        assert!(press(gtk::gdk::Key::Return));
        assert!(press(gtk::gdk::Key::Escape));
    }
    wait_for(|| window.diff.panes_ready());
    assert_focus_visible(&window);
    window.window.destroy();
}

fn app_data_stack(dialog: &adw::Dialog) -> gtk::Stack {
    assert_eq!(dialog.title(), "Manage App");
    dialog
        .child()
        .unwrap()
        .downcast::<adw::ToastOverlay>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<adw::ToolbarView>()
        .unwrap()
        .content()
        .unwrap()
        .downcast::<gtk::Stack>()
        .unwrap()
}

fn find_response_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button.label().as_deref() == Some(label)
    {
        return Some(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(button) = find_response_button(&widget, label) {
            return Some(button);
        }
        child = widget.next_sibling();
    }
    None
}

fn wait_for(condition: impl Fn() -> bool) {
    use std::time::{Duration, Instant};

    let context = gtk::glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        for _ in 0..100 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for UI state");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The text view parents its lazily created context-menu popover directly to
/// itself.
fn editor_context_menu(view: &sourceview5::View) -> Option<gtk::PopoverMenu> {
    let mut child = view.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(popover) = widget.downcast::<gtk::PopoverMenu>() {
            return Some(popover);
        }
        child = next;
    }
    None
}

fn context_menu_model_button(root: &gtk::Widget, text: &str) -> Option<gtk::Widget> {
    let mut widgets = vec![root.clone()];
    while let Some(widget) = widgets.pop() {
        if widget.is_mapped()
            && widget.type_().name() == "GtkModelButton"
            && widget.property::<String>("text") == text
        {
            return Some(widget);
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            widgets.push(widget);
        }
    }
    None
}

fn open_editor_context_menu(window: &AppWindow) {
    assert!(
        window
            .editor_view
            .activate_action("menu.popup", None)
            .is_ok()
    );
    wait_for(|| {
        editor_context_menu(&window.editor_view).is_some_and(|popover| popover.is_mapped())
    });
}

fn close_editor_context_menu(window: &AppWindow) {
    let popover = editor_context_menu(&window.editor_view).unwrap();
    popover.popdown();
    wait_for(|| !popover.is_visible());
}

fn transient_focus_scenario(app: &adw::Application, store: &SnapshotStore) {
    fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut widgets = vec![root.clone()];
        let mut index = 0;
        while index < widgets.len() {
            let mut child = widgets[index].first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                widgets.push(widget);
            }
            index += 1;
        }
        widgets
    }

    let window = AppWindow::with_store(app, store.clone());
    window.window.present();
    // Wait for the toplevel's initial Wayland configure and first frames
    // before opening a child popup programmatically.
    let frames = Rc::new(std::cell::Cell::new(0));
    let drawn = frames.clone();
    window.window.add_tick_callback(move |_, _| {
        drawn.set(drawn.get() + 1);
        if drawn.get() >= 3 {
            gtk::glib::ControlFlow::Break
        } else {
            gtk::glib::ControlFlow::Continue
        }
    });
    wait_for(|| frames.get() >= 3);
    let editor_focused = || {
        gtk::prelude::GtkWindowExt::focus(&window.window)
            == Some(window.editor_view.clone().upcast::<gtk::Widget>())
    };
    let menu = descendants(window.window.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::MenuButton>().ok())
        .find(|button| button.icon_name().as_deref() == Some("open-menu-symbolic"))
        .unwrap();
    assert!(menu.tooltip_text().is_none());
    // Automated popup calls have no real pointer/keyboard serial. Avoid a
    // Wayland input grab with serial 0; production menus retain autohide.
    menu.popover().unwrap().set_autohide(false);

    // Closing the menu with no selection restores editing focus.
    window.editor_view.grab_focus();
    menu.popup();
    wait_for(|| menu.popover().unwrap().is_mapped());
    menu.popdown();
    wait_for(editor_focused);

    window.set_editor_text("");
    window.flags.dirty.set(false);

    // Dismissing the unsaved-changes prompt restores editing focus. The
    // prompt has no Cancel button; Esc and close abort the pending New File.
    window.editor_buffer.insert_at_cursor("unsaved text");
    menu.grab_focus();
    window.window.lookup_action("new").unwrap().activate(None);
    let dialog = window.window.visible_dialog().unwrap();
    dialog.close();
    wait_for(|| window.window.visible_dialog().is_none() && editor_focused());
    assert_eq!(window.current_text(), "unsaved text");
    assert!(window.flags.dirty.get());

    // A menu action opening a dialog must keep that dialog's focus. On close,
    // Manage App returns focus to the editor.
    menu.popup();
    wait_for(|| menu.popover().unwrap().is_mapped());
    window
        .window
        .lookup_action("manage-app")
        .unwrap()
        .activate(None);
    menu.popdown();
    let snapshots = window.window.visible_dialog().unwrap();
    let stack = app_data_stack(&snapshots);
    assert!(stack.child_by_name("snapshots").is_some());
    assert!(stack.child_by_name("languages").is_none());
    assert!(stack.child_by_name("appearance").is_some());
    assert_eq!(stack.pages().n_items(), 2);
    let page = stack.page(&stack.child_by_name("appearance").unwrap());
    assert_eq!(page.title().as_deref(), Some("Appearance"));
    assert!(page.icon_name().is_none());
    wait_for(|| {
        gtk::prelude::GtkWindowExt::focus(&window.window)
            .is_some_and(|focus| focus.is_ancestor(&snapshots))
    });
    snapshots.close();
    wait_for(|| window.window.visible_dialog().is_none() && editor_focused());

    // Find chosen from the menu keeps its explicit focus target.
    menu.popup();
    wait_for(|| menu.popover().unwrap().is_mapped());
    window.window.lookup_action("find").unwrap().activate(None);
    menu.popdown();
    wait_for(|| {
        gtk::prelude::GtkWindowExt::focus(&window.window).is_some_and(|focus| {
            focus == window.search_entry || focus.is_ancestor(&window.search_entry)
        })
    });
    assert!(window.search_bar.is_search_mode());
    window.search_bar.set_search_mode(false);

    // Diff Mode returns to a visible pane, never to the hidden editor.
    window.stack.set_visible_child_name("diff");
    menu.popup();
    wait_for(|| menu.popover().unwrap().is_mapped());
    menu.popdown();
    wait_for(|| {
        gtk::prelude::GtkWindowExt::focus(&window.window)
            == Some(window.diff.left_view.clone().upcast::<gtk::Widget>())
    });
    window
        .window
        .lookup_action("manage-app")
        .unwrap()
        .activate(None);
    window.window.visible_dialog().unwrap().close();
    wait_for(|| {
        window.window.visible_dialog().is_none()
            && gtk::prelude::GtkWindowExt::focus(&window.window)
                == Some(window.diff.left_view.clone().upcast::<gtk::Widget>())
    });
    window.window.destroy();
}

fn context_menu_focus_scenario(app: &adw::Application, store: &SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());
    window.window.present();
    wait_for(|| window.window.is_mapped());
    let editor_focused = || {
        gtk::prelude::GtkWindowExt::focus(&window.window)
            == Some(window.editor_view.clone().upcast::<gtk::Widget>())
    };

    assert!(
        window
            .editor_view
            .activate_action("menu.popup", None)
            .is_ok()
    );
    wait_for(|| {
        editor_context_menu(&window.editor_view).is_some_and(|popover| popover.is_mapped())
    });
    let context = editor_context_menu(&window.editor_view).unwrap();
    context.popdown();
    // Keep focus outside the editor until the deferred restoration runs, as a
    // click-away leaves it. Without the hook it would stay on this button.
    window.open_button.grab_focus();
    assert!(!editor_focused());
    wait_for(editor_focused);
    window.window.destroy();
}

fn trim_whitespace_scenario(app: &adw::Application, store: &SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());
    window.window.present();
    wait_for(|| window.window.is_mapped());
    let buffer = window.editor_buffer.clone();
    let leading = window
        .window
        .lookup_action("trim-leading-whitespace")
        .unwrap();
    let trailing = window
        .window
        .lookup_action("trim-trailing-whitespace")
        .unwrap();
    let blank_remove = window
        .window
        .lookup_action("trim-blank-lines-remove")
        .unwrap();
    let blank_strip = window
        .window
        .lookup_action("trim-blank-lines-strip")
        .unwrap();
    let submenu = window.window.lookup_action("trim-whitespace-menu").unwrap();

    // A clean document disables all four items and the submenu heading.
    window.set_editor_text("a\nb\n");
    open_editor_context_menu(&window);
    assert!(!leading.is_enabled());
    assert!(!trailing.is_enabled());
    assert!(!blank_remove.is_enabled());
    assert!(!blank_strip.is_enabled());
    assert!(!submenu.is_enabled());
    let context = editor_context_menu(&window.editor_view).unwrap();
    assert!(
        !context_menu_model_button(context.upcast_ref(), "Trim Whitespace")
            .unwrap()
            .is_sensitive()
    );
    close_editor_context_menu(&window);

    // A 0-byte document disables every trim item and the heading.
    window.set_editor_text("");
    open_editor_context_menu(&window);
    assert!(!leading.is_enabled());
    assert!(!trailing.is_enabled());
    assert!(!blank_remove.is_enabled());
    assert!(!blank_strip.is_enabled());
    assert!(!submenu.is_enabled());
    let context = editor_context_menu(&window.editor_view).unwrap();
    assert!(
        !context_menu_model_button(context.upcast_ref(), "Trim Whitespace")
            .unwrap()
            .is_sensitive()
    );
    close_editor_context_menu(&window);
    window.trim_blank_lines_remove();
    assert_eq!(window.current_text(), "");
    assert_toast(&window, ToastKind::Info, "No blank lines to remove");
    window.trim_blank_lines_strip();
    assert_toast(&window, ToastKind::Info, "No blank lines to strip");

    // A single-space document is one real blank line.
    window.set_editor_text(" ");
    open_editor_context_menu(&window);
    assert!(blank_remove.is_enabled());
    assert!(blank_strip.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);
    blank_remove.activate(None);
    assert_eq!(window.current_text(), "");
    assert_toast(&window, ToastKind::Success, "1 blank line was removed");

    window.set_editor_text(" ");
    blank_strip.activate(None);
    assert_eq!(window.current_text(), "");
    assert_toast(
        &window,
        ToastKind::Success,
        "1 blank line was stripped of 1 whitespace character",
    );

    // Whole-document leading trim, success toast and one undo step.
    window.set_editor_text(" a\n  b\n");
    open_editor_context_menu(&window);
    assert!(leading.is_enabled());
    assert!(!trailing.is_enabled());
    assert!(submenu.is_enabled());
    let context = editor_context_menu(&window.editor_view).unwrap();
    assert!(
        context_menu_model_button(context.upcast_ref(), "Trim Whitespace")
            .unwrap()
            .is_sensitive()
    );
    close_editor_context_menu(&window);
    leading.activate(None);
    assert_eq!(window.current_text(), "a\nb\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "3 leading whitespace characters were removed",
    );
    buffer.undo();
    assert_eq!(window.current_text(), " a\n  b\n");

    // Whole-document trailing trim, success toast and one undo step.
    window.set_editor_text("a \nb \n");
    open_editor_context_menu(&window);
    assert!(!leading.is_enabled());
    assert!(trailing.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);
    trailing.activate(None);
    assert_eq!(window.current_text(), "a\nb\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "2 trailing whitespace characters were removed",
    );
    buffer.undo();
    assert_eq!(window.current_text(), "a \nb \n");

    // Singular wording for each side.
    window.set_editor_text(" a\n");
    window.trim_leading_whitespace();
    assert_eq!(window.current_text(), "a\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "1 leading whitespace character was removed",
    );
    buffer.undo();
    assert_eq!(window.current_text(), " a\n");

    window.set_editor_text("a \n");
    window.trim_trailing_whitespace();
    assert_eq!(window.current_text(), "a\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "1 trailing whitespace character was removed",
    );
    buffer.undo();
    assert_eq!(window.current_text(), "a \n");

    // Blank lines can be removed entirely.
    window.set_editor_text("a\n\n \n b\n\nc\n");
    open_editor_context_menu(&window);
    assert!(blank_remove.is_enabled());
    assert!(blank_strip.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);
    blank_remove.activate(None);
    assert_eq!(window.current_text(), "a\n b\nc\n");
    assert_toast(&window, ToastKind::Success, "3 blank lines were removed");
    buffer.undo();
    assert_eq!(window.current_text(), "a\n\n \n b\n\nc\n");

    // Blank lines can be stripped instead.
    window.set_editor_text("a\n   \n \nb\n");
    open_editor_context_menu(&window);
    assert!(blank_remove.is_enabled());
    assert!(blank_strip.is_enabled());
    close_editor_context_menu(&window);
    blank_strip.activate(None);
    assert_eq!(window.current_text(), "a\n\n\nb\n");
    assert_toast(
        &window,
        ToastKind::Success,
        "2 blank lines were stripped of 4 whitespace characters",
    );
    buffer.undo();
    assert_eq!(window.current_text(), "a\n   \n \nb\n");

    // Empty blank lines enable removal but not stripping.
    window.set_editor_text("a\n\nb\n");
    open_editor_context_menu(&window);
    assert!(blank_remove.is_enabled());
    assert!(!blank_strip.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);

    // A selection limits the target lines for both sides.
    window.set_editor_text(" a \n b \n c \n");
    buffer.select_range(
        &buffer.iter_at_line(0).unwrap(),
        &buffer.iter_at_line(1).unwrap(),
    );
    open_editor_context_menu(&window);
    assert!(leading.is_enabled());
    assert!(trailing.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);
    leading.activate(None);
    assert_eq!(window.current_text(), "a \n b \n c \n");
    buffer.undo();
    assert_eq!(window.current_text(), " a \n b \n c \n");

    buffer.select_range(
        &buffer.iter_at_line(0).unwrap(),
        &buffer.iter_at_line(1).unwrap(),
    );
    open_editor_context_menu(&window);
    close_editor_context_menu(&window);
    trailing.activate(None);
    assert_eq!(window.current_text(), " a\n b \n c \n");

    // A selection also limits blank-line removal.
    window.set_editor_text("a\n\nb\n\nc\n");
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    open_editor_context_menu(&window);
    assert!(blank_remove.is_enabled());
    assert!(!blank_strip.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    blank_remove.activate(None);
    assert_eq!(window.current_text(), "a\nb\n\nc\n");
    buffer.undo();
    assert_eq!(window.current_text(), "a\n\nb\n\nc\n");

    // …and blank-line stripping.
    window.set_editor_text("a\n  \nb\n \nc\n");
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    open_editor_context_menu(&window);
    assert!(blank_remove.is_enabled());
    assert!(blank_strip.is_enabled());
    close_editor_context_menu(&window);
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    blank_strip.activate(None);
    assert_eq!(window.current_text(), "a\n\nb\n \nc\n");

    // Trailing-only selection keeps the submenu enabled.
    window.set_editor_text("a \nb\nc \n");
    buffer.select_range(
        &buffer.iter_at_line(0).unwrap(),
        &buffer.iter_at_line(1).unwrap(),
    );
    open_editor_context_menu(&window);
    assert!(!leading.is_enabled());
    assert!(trailing.is_enabled());
    assert!(submenu.is_enabled());
    close_editor_context_menu(&window);

    // A selection with no leading, trailing or blank-line whitespace disables
    // every item and the heading.
    window.set_editor_text("a \nb\nc \n");
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    open_editor_context_menu(&window);
    assert!(!leading.is_enabled());
    assert!(!trailing.is_enabled());
    assert!(!blank_remove.is_enabled());
    assert!(!blank_strip.is_enabled());
    assert!(!submenu.is_enabled());
    close_editor_context_menu(&window);

    // Direct invocation still reports each no-op. Restore the selection
    // because closing a programmatically opened popup may move focus.
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    window.trim_leading_whitespace();
    assert_toast(&window, ToastKind::Info, "No leading whitespace to remove");
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    window.trim_trailing_whitespace();
    assert_eq!(window.current_text(), "a \nb\nc \n");
    assert_toast(&window, ToastKind::Info, "No trailing whitespace to remove");
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    window.trim_blank_lines_remove();
    assert_toast(&window, ToastKind::Info, "No blank lines to remove");
    buffer.select_range(
        &buffer.iter_at_line(1).unwrap(),
        &buffer.iter_at_line(2).unwrap(),
    );
    window.trim_blank_lines_strip();
    assert_toast(&window, ToastKind::Info, "No blank lines to strip");

    // Read-only disables both actions and the heading, and does nothing.
    window.set_editor_text(" a \n");
    window.set_read_only(true);
    open_editor_context_menu(&window);
    assert!(!leading.is_enabled());
    assert!(!trailing.is_enabled());
    assert!(!blank_remove.is_enabled());
    assert!(!blank_strip.is_enabled());
    assert!(!submenu.is_enabled());
    close_editor_context_menu(&window);
    window.trim_leading_whitespace();
    window.trim_trailing_whitespace();
    window.trim_blank_lines_remove();
    window.trim_blank_lines_strip();
    assert_eq!(window.current_text(), " a \n");
    window.set_read_only(false);

    // Diff Mode does nothing.
    window.stack.set_visible_child_name("diff");
    window.trim_leading_whitespace();
    window.trim_trailing_whitespace();
    window.trim_blank_lines_remove();
    window.trim_blank_lines_strip();
    assert_eq!(window.current_text(), " a \n");
    window.stack.set_visible_child_name("editor");

    window.window.destroy();
}

fn plain_text_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let assert_plain_text = |window: &AppWindow| {
        assert!(window.editor_buffer.language().is_none());
        assert!(!window.editor_buffer.is_highlight_syntax());
        for view in window.diff.copy_views() {
            let buffer = view.buffer().downcast::<sourceview5::Buffer>().unwrap();
            assert!(buffer.language().is_none());
            assert!(!buffer.is_highlight_syntax());
        }
    };

    // Untitled: no language and no syntax highlighting.
    let window = AppWindow::with_store(app, store.clone());
    assert_plain_text(&window);

    // An extension that GtkSourceView could recognize still opens as plain text.
    let rust = dir.join("plain.rs");
    std::fs::write(&rust, "fn main() { println!(\"plain\"); }\n").unwrap();
    AppWindow::open_path(&window, &rust);
    assert_plain_text(&window);
    assert_eq!(
        window.current_text(),
        "fn main() { println!(\"plain\"); }\n"
    );

    // Markdown is plain text too; headings and emphasis are not styled.
    let markdown = dir.join("plain.md");
    std::fs::write(&markdown, "# Title\n\n* emphasis * and `code`\n").unwrap();
    AppWindow::open_path(&window, &markdown);
    assert_plain_text(&window);
    window.window.destroy();
}

fn document_state_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());

    // An emptied untitled document has nothing to save.
    window.editor_buffer.insert_at_cursor("x");
    assert!(window.flags.dirty.get());
    assert!(window.window.has_css_class(theme::DIRTY_CLASS));
    window.editor_buffer.set_text("");
    assert!(!window.flags.dirty.get());
    assert!(!window.window.has_css_class(theme::DIRTY_CLASS));

    // Loading a document clears the last-edit stamp; typing sets it.
    let path = dir.join("state.md");
    std::fs::write(&path, "alpha beta gamma\n").unwrap();
    AppWindow::open_path(&window, &path);
    assert!(!window.status_state.has_edit());
    window.editor_buffer.insert_at_cursor("x");
    assert!(window.status_state.has_edit());
    window.update_last_edit();

    // Save As preserves the last edit.
    window.persist(Some(&dir.join("state-saved.md"))).unwrap();
    assert!(window.status_state.has_edit());
    window.persist(None).unwrap();
    assert!(window.status_state.has_edit());
    AppWindow::open_path(&window, &dir.join("state-saved.md"));
    assert!(!window.status_state.has_edit());

    // Undoing back to the saved text makes a file-backed document clean.
    assert!(!window.flags.dirty.get());
    window.editor_buffer.insert_at_cursor("y");
    window.settle_dirty();
    assert!(window.flags.dirty.get());
    window.editor_buffer.undo();
    window.settle_dirty();
    assert!(!window.flags.dirty.get());

    // Equal character counts still require exact text comparison.
    let original = window.current_text();
    let replacement = original.replacen("alpha", "omega", 1);
    assert_ne!(original, replacement);
    assert_eq!(original.chars().count(), replacement.chars().count());
    window.editor_buffer.set_text(&replacement);
    window.settle_dirty();
    assert!(window.flags.dirty.get());
    window.editor_buffer.set_text(&original);
    window.settle_dirty();
    assert!(!window.flags.dirty.get());

    // The actual idle callback coalesces edits and clears the pending flag.
    window.editor_buffer.insert_at_cursor("z");
    window.editor_buffer.undo();
    assert!(window.flags.dirty.get());
    assert!(window.flags.dirty_check_pending.get());
    let context = gtk::glib::MainContext::default();
    while window.flags.dirty_check_pending.get() {
        context.iteration(false);
    }
    assert!(!window.flags.dirty.get());
    window.window.destroy();
}

fn save_external_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    use std::time::SystemTime;

    let path = dir.join("save-external.txt");
    std::fs::write(&path, "disk one\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.set_editor_text("local edit\n");
    std::fs::write(&path, "disk two\n").unwrap();
    window
        .doc
        .borrow_mut()
        .set_last_mtime(Some(SystemTime::UNIX_EPOCH));
    window.window.present();

    window.save_then(None);
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(dialog.body().contains("Saving now"));
    assert!(dialog.has_response("cancel"));
    assert!(dialog.has_response("overwrite"));
    find_response_button(dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(!window.flags.external_dialog_open.get());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "disk two\n");
    assert!(window.doc.borrow().externally_modified());

    window.save_then(None);
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Overwrite")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "local edit\n");
    assert!(!window.doc.borrow().externally_modified());
    window.window.destroy();
}
fn quit_scenario(app: &adw::Application, store: SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());
    let clean = AppWindow::with_store(app, store);
    window.window.present();
    clean.window.present();
    window.editor_buffer.insert_at_cursor("dirty");
    assert!(window.flags.dirty.get());

    // Quit must route a dirty window through its normal close prompt instead
    // of calling application quit directly.
    assert!(gtk::prelude::WidgetExt::activate_action(&window.window, "win.quit", None).is_ok());
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(window.window.is_visible());
    assert!(clean.window.is_visible());
    assert!(
        app.windows()
            .iter()
            .any(|candidate| { candidate == window.window.upcast_ref::<gtk::Window>() })
    );

    // Discarding the dirty window continues the quit and closes the clean one.
    assert!(gtk::prelude::WidgetExt::activate_action(&window.window, "win.quit", None).is_ok());
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Discard Changes")
        .unwrap()
        .emit_clicked();
    wait_for(|| app.windows().is_empty());
}

fn quit_review_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("quit-review.txt");
    std::fs::write(&path, "one\ntwo\n").unwrap();

    let window = AppWindow::with_store(app, store.clone());
    let clean = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("one\nTWO\n");
    window.flags.dirty.set(true);
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    window.accept_focused();
    assert!(window.has_accepted_diff());
    window.window.present();
    clean.window.present();

    assert!(gtk::prelude::WidgetExt::activate_action(&window.window, "win.quit", None).is_ok());
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(
        dialog.heading().as_deref(),
        Some("Discard accepted changes?")
    );
    assert!(dialog.body().contains("continue quitting?"));
    assert!(find_response_button(dialog.upcast_ref(), "Discard and Quit").is_some());
    find_response_button(dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(window.window.is_visible());
    assert!(clean.window.is_visible());
    assert!(window.diff_active());

    assert!(gtk::prelude::WidgetExt::activate_action(&window.window, "win.quit", None).is_ok());
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Discard and Quit")
        .unwrap()
        .emit_clicked();
    wait_for(|| app.windows().is_empty());
}

fn open_file_save_prompt_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let first = dir.join("open-prompt-first.txt");
    let second = dir.join("open-prompt-second.txt");
    let third = dir.join("open-prompt-third.txt");
    std::fs::write(&first, "first\n").unwrap();
    std::fs::write(&second, "second\n").unwrap();
    std::fs::write(&third, "third\n").unwrap();

    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &first);
    window.window.present();
    window
        .editor_buffer
        .place_cursor(&window.editor_buffer.end_iter());
    window.editor_buffer.insert_at_cursor("dirty");
    assert!(window.flags.dirty.get());
    let first_canonical = first.canonicalize().unwrap();

    // Simulate the file chooser returning the second file. The save prompt
    // comes after the chooser and has no Cancel button.
    AppWindow::finish_open_dialog(&window, Ok(gio::File::for_path(&second)));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(dialog.heading().as_deref(), Some("Save changes?"));
    assert_eq!(dialog.body().as_str(), "The document has unsaved changes.");
    assert!(find_response_button(dialog.upcast_ref(), "Cancel").is_none());
    assert!(find_response_button(dialog.upcast_ref(), "Save").is_some());
    assert!(find_response_button(dialog.upcast_ref(), "Discard Changes").is_some());
    assert!(find_response_button(dialog.upcast_ref(), "Open New Window").is_some());
    assert_eq!(
        dialog.response_appearance("save"),
        adw::ResponseAppearance::Suggested
    );
    assert_eq!(
        dialog.response_appearance("discard"),
        adw::ResponseAppearance::Destructive
    );
    assert_eq!(
        dialog.response_appearance("new-window"),
        adw::ResponseAppearance::Default
    );
    assert_eq!(dialog.close_response().as_str(), "cancel");

    // Dismissing the prompt aborts the pending Open.
    dialog.close();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(window.doc.borrow().path(), Some(first_canonical.as_path()));
    assert_eq!(window.current_text(), "first\ndirty");
    assert!(window.flags.dirty.get());

    // Opening in a new window preserves the current dirty document.
    let before = super::WINDOWS.with_borrow(|windows| windows.len());
    AppWindow::finish_open_dialog(&window, Ok(gio::File::for_path(&second)));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Open New Window")
        .unwrap()
        .emit_clicked();
    wait_for(|| super::WINDOWS.with_borrow(|windows| windows.len() == before + 1));
    let created = super::WINDOWS
        .with_borrow(|windows| {
            windows
                .iter()
                .find(|candidate| !Rc::ptr_eq(candidate, &window))
                .cloned()
        })
        .expect("new window");
    assert_eq!(window.doc.borrow().path(), Some(first_canonical.as_path()));
    assert_eq!(window.current_text(), "first\ndirty");
    assert!(window.flags.dirty.get());
    let second_canonical = second.canonicalize().unwrap();
    assert_eq!(
        created.doc.borrow().path(),
        Some(second_canonical.as_path())
    );
    assert_eq!(created.current_text(), "second\n");
    assert!(!created.diff_active());
    created.window.destroy();
    wait_for(|| {
        super::WINDOWS.with_borrow(|windows| {
            !windows
                .iter()
                .any(|candidate| Rc::ptr_eq(candidate, &created))
        })
    });

    // Discard Changes opens the selected file in this window.
    AppWindow::finish_open_dialog(&window, Ok(gio::File::for_path(&second)));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Discard Changes")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(window.doc.borrow().path(), Some(second_canonical.as_path()));
    assert_eq!(window.current_text(), "second\n");
    assert!(!window.flags.dirty.get());

    // Save first, then open the third file in this window.
    window
        .editor_buffer
        .place_cursor(&window.editor_buffer.end_iter());
    window.editor_buffer.insert_at_cursor("dirty");
    assert!(window.flags.dirty.get());
    AppWindow::finish_open_dialog(&window, Ok(gio::File::for_path(&third)));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Save")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second\ndirty");
    let third_canonical = third.canonicalize().unwrap();
    assert_eq!(window.doc.borrow().path(), Some(third_canonical.as_path()));
    assert_eq!(window.current_text(), "third\n");
    assert!(!window.flags.dirty.get());
    window.window.destroy();
}

fn new_file_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("new-file-first.txt");
    std::fs::write(&path, "first file\n").unwrap();

    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.window.present();
    let canonical = path.canonicalize().unwrap();

    // New File prompts before replacing a dirty document. The neutral button
    // opens an empty window instead; there is no Cancel button.
    window
        .editor_buffer
        .place_cursor(&window.editor_buffer.end_iter());
    window.editor_buffer.insert_at_cursor("unsaved");
    assert_eq!(window.current_text(), "first file\nunsaved");
    assert!(window.flags.dirty.get());
    window.new_button.emit_clicked();
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(dialog.heading().as_deref(), Some("Save changes?"));
    assert_eq!(dialog.body().as_str(), "The document has unsaved changes.");
    assert!(find_response_button(dialog.upcast_ref(), "Cancel").is_none());
    assert!(find_response_button(dialog.upcast_ref(), "Save").is_some());
    assert!(find_response_button(dialog.upcast_ref(), "Discard Changes").is_some());
    assert!(find_response_button(dialog.upcast_ref(), "Open New Window").is_some());
    assert_eq!(
        dialog.response_appearance("save"),
        adw::ResponseAppearance::Suggested
    );
    assert_eq!(
        dialog.response_appearance("discard"),
        adw::ResponseAppearance::Destructive
    );
    assert_eq!(
        dialog.response_appearance("new-window"),
        adw::ResponseAppearance::Default
    );

    // Dismissing the prompt leaves the dirty document in place.
    dialog.close();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(window.doc.borrow().path(), Some(canonical.as_path()));
    assert_eq!(window.current_text(), "first file\nunsaved");
    assert!(window.flags.dirty.get());

    // Open New Window opens an empty window and keeps this draft.
    let before = super::WINDOWS.with_borrow(|windows| windows.len());
    window.window.lookup_action("new").unwrap().activate(None);
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Open New Window")
        .unwrap()
        .emit_clicked();
    wait_for(|| super::WINDOWS.with_borrow(|windows| windows.len() == before + 1));
    let created = super::WINDOWS
        .with_borrow(|windows| {
            windows
                .iter()
                .find(|candidate| !Rc::ptr_eq(candidate, &window))
                .cloned()
        })
        .expect("new window");
    assert!(created.doc.borrow().path().is_none());
    assert_eq!(created.current_text(), "");
    assert_eq!(created.window.title().as_deref(), Some("Untitled"));
    assert!(!created.flags.dirty.get());
    assert_eq!(window.doc.borrow().path(), Some(canonical.as_path()));
    assert_eq!(window.current_text(), "first file\nunsaved");
    assert!(window.flags.dirty.get());
    created.window.destroy();
    wait_for(|| {
        super::WINDOWS.with_borrow(|windows| {
            !windows
                .iter()
                .any(|candidate| Rc::ptr_eq(candidate, &created))
        })
    });

    // Discard Changes resets this window to an untitled, editable document.
    window.window.lookup_action("new").unwrap().activate(None);
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Discard Changes")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(window.doc.borrow().path().is_none());
    assert_eq!(window.current_text(), "");
    assert_eq!(window.window.title().as_deref(), Some("Untitled"));
    assert!(!window.flags.dirty.get());
    assert!(!window.flags.read_only.get());
    assert!(!window.diff_active());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first file\n");
    // Replacing the document leaves the old file's baseline on disk.
    assert!(store.has_for_canonical(&canonical));

    // Save writes the draft first, then starts the new document.
    AppWindow::open_path(&window, &path);
    window
        .editor_buffer
        .place_cursor(&window.editor_buffer.end_iter());
    window.editor_buffer.insert_at_cursor("saved");
    assert!(window.flags.dirty.get());
    window.window.lookup_action("new").unwrap().activate(None);
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Save")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first file\nsaved");
    assert!(window.doc.borrow().path().is_none());
    assert_eq!(window.current_text(), "");
    assert!(!window.flags.dirty.get());

    // New Window opens another empty window and leaves this one untouched.
    let open_before = super::WINDOWS.with_borrow(|windows| windows.len());
    window
        .window
        .lookup_action("new-window")
        .unwrap()
        .activate(None);
    let created = super::WINDOWS
        .with_borrow(|windows| {
            assert_eq!(windows.len(), open_before + 1);
            windows.last().cloned()
        })
        .expect("new window");
    assert!(!Rc::ptr_eq(&created, &window));
    assert!(created.doc.borrow().path().is_none());
    assert_eq!(created.current_text(), "");
    assert_eq!(created.window.title().as_deref(), Some("Untitled"));
    assert_eq!(window.current_text(), "");

    created.window.destroy();
    wait_for(|| {
        super::WINDOWS.with_borrow(|windows| {
            !windows
                .iter()
                .any(|candidate| Rc::ptr_eq(candidate, &created))
        })
    });
    window.window.destroy();
    wait_for(|| {
        super::WINDOWS.with_borrow(|windows| {
            !windows
                .iter()
                .any(|candidate| Rc::ptr_eq(candidate, &window))
        })
    });
}

fn title_menu_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("title-menu.md");
    std::fs::write(&path, "title\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    let canonical = path.canonicalize().unwrap();
    let canonical_text = canonical.display().to_string();

    assert_eq!(
        window.title_label.tooltip_text().as_deref(),
        Some(canonical_text.as_str())
    );

    // The full-path tooltip is suppressed while the copy menu is showing.
    // Dismissing the menu also returns focus to the editor.
    window.window.present();
    gtk::test_widget_wait_for_draw(&window.window);
    window.title_popover.set_autohide(false);
    let editor_focused = || {
        gtk::prelude::GtkWindowExt::focus(&window.window)
            == Some(window.editor_view.clone().upcast::<gtk::Widget>())
    };
    window.editor_view.grab_focus();
    assert!(editor_focused());
    window.title_popover.popup();
    wait_for(|| window.title_popover.is_visible());
    assert!(window.title_label.tooltip_text().is_none());
    window.title_popover.popdown();
    wait_for(|| !window.title_popover.is_visible());
    wait_for(editor_focused);
    assert_eq!(
        window.title_label.tooltip_text().as_deref(),
        Some(canonical_text.as_str())
    );

    for name in ["copy-file-name", "copy-full-path"] {
        assert!(window.window.lookup_action(name).unwrap().is_enabled());
    }

    window.editor_view.grab_focus();
    window.last_toast.take();
    window.title_popover.popup();
    wait_for(|| window.title_popover.is_visible());
    window
        .window
        .lookup_action("copy-file-name")
        .unwrap()
        .activate(None);
    window.title_popover.popdown();
    wait_for(|| !window.title_popover.is_visible());
    wait_for(editor_focused);
    let copied = gtk::glib::MainContext::default()
        .block_on(window.window.clipboard().read_text_future())
        .unwrap()
        .unwrap();
    assert_eq!(copied, "title-menu.md");
    assert!(
        window.last_toast.borrow().is_none(),
        "copying the file name must not show a toast"
    );

    window.editor_view.grab_focus();
    window.last_toast.take();
    window.title_popover.popup();
    wait_for(|| window.title_popover.is_visible());
    window
        .window
        .lookup_action("copy-full-path")
        .unwrap()
        .activate(None);
    window.title_popover.popdown();
    wait_for(|| !window.title_popover.is_visible());
    wait_for(editor_focused);
    let copied = gtk::glib::MainContext::default()
        .block_on(window.window.clipboard().read_text_future())
        .unwrap()
        .unwrap();
    assert_eq!(copied, canonical_text);
    assert!(
        window.last_toast.borrow().is_none(),
        "copying the full path must not show a toast"
    );

    // Untitled documents have no title tooltip or copy actions.
    window.window.lookup_action("new").unwrap().activate(None);
    assert!(window.doc.borrow().path().is_none());
    assert!(window.title_label.tooltip_text().is_none());
    assert_eq!(window.title_label.label().as_str(), "Untitled");
    for name in ["copy-file-name", "copy-full-path"] {
        assert!(!window.window.lookup_action(name).unwrap().is_enabled());
    }

    window.window.destroy();
}

fn recent_menu_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let state = dir.join("recent-state");
    let first_file = dir.join("recent-first.txt");
    let second_file = dir.join("recent-second.txt");
    std::fs::write(&first_file, "first\n").unwrap();
    std::fs::write(&second_file, "second\n").unwrap();

    let window = AppWindow::with_parts(
        app,
        store.clone(),
        false,
        RecentDocuments::at(state.clone()),
    );
    window.window.present();
    AppWindow::open_path(&window, &first_file);
    AppWindow::open_path(&window, &second_file);
    let first = first_file.canonicalize().unwrap();
    let second = second_file.canonicalize().unwrap();

    assert_eq!(
        window.recent.pruned_entries(),
        vec![second.clone(), first.clone()]
    );
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        format!("{}\n{}\n", second.display(), first.display())
    );

    // Clicking the button opens the menu when there are recent paths. The
    // button's shortcut tooltip is suppressed while the menu is showing.
    assert_eq!(
        window.open_button.tooltip_text().as_deref(),
        Some("Open File (Ctrl+O)")
    );
    window.open_button.emit_clicked();
    assert!(window.open_popover.is_visible());
    assert!(window.open_button.tooltip_text().is_none());
    let model = window.open_popover.menu_model().expect("open menu model");
    assert_eq!(model.n_items(), 2);
    let label = |menu: &gio::MenuModel, index: i32| {
        menu.item_attribute_value(index, gio::MENU_ATTRIBUTE_LABEL, None)
            .and_then(|value| value.get::<String>())
    };
    assert_eq!(label(&model, 0).as_deref(), Some("Browse…"));
    let recents = model.item_link(1, gio::MENU_LINK_SECTION).unwrap();
    assert_eq!(recents.n_items(), 2);
    assert_eq!(label(&recents, 0).as_deref(), Some("recent-second.txt"));
    assert_eq!(label(&recents, 1).as_deref(), Some("recent-first.txt"));
    let custom = |menu: &gio::MenuModel, index: i32| {
        menu.item_attribute_value(index, "custom", None)
            .and_then(|value| value.get::<String>())
    };
    assert_eq!(custom(&recents, 0).as_deref(), Some("recent-0"));
    assert_eq!(custom(&recents, 1).as_deref(), Some("recent-1"));

    let rows = window.open_recent_rows.borrow().clone();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].root.parent().is_some());
    let content = rows[0]
        .open
        .child()
        .unwrap()
        .downcast::<gtk::Box>()
        .unwrap();
    assert_eq!(content.orientation(), gtk::Orientation::Vertical);
    let name_label = content
        .first_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(name_label.label().as_str(), "recent-second.txt");
    assert!(name_label.attributes().is_some());
    assert!(rows[0].open.tooltip_text().is_none());
    assert!(rows[0].trash.tooltip_text().is_none());
    let directory_label = content
        .last_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(
        directory_label.label().as_str(),
        format!("{}/", second.parent().unwrap().to_string_lossy())
    );
    assert!(directory_label.has_css_class("dim-label"));
    assert!(directory_label.has_css_class("caption"));
    assert_eq!(
        rows[0].trash.icon_name().as_deref(),
        Some("view-conceal-symbolic")
    );
    assert_eq!(
        rows[0].root.last_child().as_ref(),
        Some(rows[0].trash.upcast_ref::<gtk::Widget>())
    );
    window.open_popover.popdown();
    assert_eq!(
        window.open_button.tooltip_text().as_deref(),
        Some("Open File (Ctrl+O)")
    );

    // Reopening the menu replaces the custom rows instead of reusing stale
    // ones.
    window.open_button.emit_clicked();
    assert!(window.open_button.tooltip_text().is_none());
    let rows = window.open_recent_rows.borrow().clone();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.root.parent().is_some()));
    window.open_popover.popdown();
    assert_eq!(
        window.open_button.tooltip_text().as_deref(),
        Some("Open File (Ctrl+O)")
    );

    // Clicking a two-line recent row goes through the normal open flow and
    // moves the path to the front of the list.
    let first_row = rows[1].open.clone();
    first_row.emit_clicked();
    assert_eq!(window.doc.borrow().path(), Some(first.as_path()));
    assert_eq!(
        window.recent.pruned_entries(),
        vec![first.clone(), second.clone()]
    );

    // A dirty document gets the same save prompt as the chooser.
    window
        .editor_buffer
        .place_cursor(&window.editor_buffer.end_iter());
    window.editor_buffer.insert_at_cursor("dirty");
    let parameter = second.to_str().unwrap().to_variant();
    window
        .window
        .lookup_action("open-recent")
        .unwrap()
        .activate(Some(&parameter));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(dialog.heading().as_deref(), Some("Save changes?"));
    dialog.close();
    wait_for(|| window.window.visible_dialog().is_none());
    assert_eq!(window.doc.borrow().path(), Some(first.as_path()));
    assert!(window.flags.dirty.get());

    // Save As and Rename or Move record the new canonical path; the stale old
    // path is pruned when the list is next used.
    let third_file = dir.join("recent-third.txt");
    window.persist(Some(&third_file)).unwrap();
    let third = third_file.canonicalize().unwrap();
    assert_eq!(window.doc.borrow().path(), Some(third.as_path()));
    assert_eq!(
        window.recent.pruned_entries(),
        vec![third.clone(), first.clone(), second.clone()]
    );

    window.rename_file("recent-renamed.txt");
    let renamed = dir.join("recent-renamed.txt").canonicalize().unwrap();
    assert_eq!(window.doc.borrow().path(), Some(renamed.as_path()));
    assert_eq!(
        window.recent.pruned_entries(),
        vec![renamed.clone(), first.clone(), second.clone()]
    );

    // A missing file disappears the next time the list is used.
    std::fs::remove_file(&second_file).unwrap();
    assert_eq!(
        window.recent.pruned_entries(),
        vec![renamed.clone(), first.clone()]
    );
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        format!("{}\n{}\n", renamed.display(), first.display())
    );

    assert!(
        window.window.lookup_action("clear-recent").is_none(),
        "Clear Recent was removed"
    );

    window.window.destroy();
}

fn recent_trash_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let state = dir.join("recent-trash-state");
    let first_file = dir.join("recent-trash-first.txt");
    let second_file = dir.join("recent-trash-second.txt");
    std::fs::write(&first_file, "first\n").unwrap();
    std::fs::write(&second_file, "second\n").unwrap();

    let window = AppWindow::with_parts(
        app,
        store.clone(),
        false,
        RecentDocuments::at(state.clone()),
    );
    window.window.present();
    AppWindow::open_path(&window, &first_file);
    AppWindow::open_path(&window, &second_file);
    let first = first_file.canonicalize().unwrap();
    let second = second_file.canonicalize().unwrap();
    assert_eq!(
        window.recent.pruned_entries(),
        vec![second.clone(), first.clone()]
    );

    window.open_button.emit_clicked();
    let rows = window.open_recent_rows.borrow().clone();
    assert_eq!(rows.len(), 2);

    rows[0].trash.emit_clicked();
    assert_eq!(window.recent.pruned_entries(), vec![first.clone()]);
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        format!("{}\n", first.display())
    );
    assert!(window.open_popover.is_visible());
    let rows = window.open_recent_rows.borrow().clone();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].open.is_focusable());

    rows[0].trash.emit_clicked();
    assert!(!window.open_popover.is_visible());
    assert!(window.recent.pruned_entries().is_empty());
    assert!(!state.exists());

    window.window.destroy();
}

fn diff_file_actions_disabled_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("diff-locked-actions.txt");
    std::fs::write(&path, "base\n").unwrap();

    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("changed\n");
    window.persist(None).unwrap();
    assert_eq!(
        window.open_button.tooltip_text().as_deref(),
        Some("Open File (Ctrl+O)")
    );
    assert_eq!(
        window.new_button.tooltip_text().as_deref(),
        Some("New File (Ctrl+N)")
    );
    window.enter_diff();
    window.focus_relative(1);
    window.accept_focused();
    assert!(window.has_accepted_diff());
    window.window.present();

    for name in ["new", "open", "open-recent", "save-as", "rename"] {
        assert!(
            !window.window.lookup_action(name).unwrap().is_enabled(),
            "{name} must be disabled in Diff Mode"
        );
    }
    assert!(
        window
            .window
            .lookup_action("new-window")
            .unwrap()
            .is_enabled()
    );
    assert!(!window.open_button.is_sensitive());
    assert!(window.open_button.tooltip_text().is_none());
    assert!(!window.new_button.is_sensitive());
    assert!(window.new_button.tooltip_text().is_none());

    // Disabled actions must not open a prompt or disturb the review session.
    window.window.lookup_action("new").unwrap().activate(None);
    window.window.lookup_action("open").unwrap().activate(None);
    assert!(window.window.visible_dialog().is_none());
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());
    assert_eq!(window.current_text(), "changed\n");

    // Leaving Diff Mode restores the actions and their tooltips.
    window.undo_accept();
    assert!(!window.has_accepted_diff());
    window.exit_diff();
    assert!(!window.diff_active());
    for name in ["new", "open", "open-recent", "save-as", "rename"] {
        assert!(window.window.lookup_action(name).unwrap().is_enabled());
    }
    assert!(window.open_button.is_sensitive());
    assert_eq!(
        window.open_button.tooltip_text().as_deref(),
        Some("Open File (Ctrl+O)")
    );
    assert_eq!(
        window.new_button.tooltip_text().as_deref(),
        Some("New File (Ctrl+N)")
    );
    assert!(window.new_button.is_sensitive());
    window.window.destroy();
}

fn external_change_in_diff_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    use crate::core::pending::ExternalCheck;
    use std::time::SystemTime;

    fn response_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(button) = response_button(&widget, label) {
                return Some(button);
            }
            child = widget.next_sibling();
        }
        None
    }

    let path = dir.join("external-change-in-diff.txt");
    let baseline = "original\n";
    let live = "reviewed text\n";
    let disk = "external replacement\n";
    std::fs::write(&path, baseline).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text(live);
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    window.accept_focused();
    assert!(window.has_accepted_diff());
    window.window.present();

    std::fs::write(&path, disk).unwrap();
    // Force a distinct remembered time, independent of filesystem resolution.
    window
        .doc
        .borrow_mut()
        .set_last_mtime(Some(SystemTime::UNIX_EPOCH));
    // Use the focus check without depending on the compositor activating a
    // test window (which is not guaranteed under ChromeOS).
    window.check_external(ExternalCheck::Focus);
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert!(dialog.body().contains("unconfirmed review changes"));
    assert!(!dialog.can_close());
    assert!(!dialog.has_response("cancel"));
    assert!(dialog.has_response("keep"));
    assert!(dialog.has_response("reload"));
    assert!(!dialog.close());
    assert!(window.flags.external_dialog_open.get());
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());
    assert_eq!(window.current_text(), live);
    response_button(dialog.upcast_ref(), "Keep Current Text")
        .unwrap()
        .emit_clicked();
    wait_for(|| !window.flags.external_dialog_open.get());
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());
    assert_eq!(window.current_text(), live);
    assert!(window.doc.borrow().externally_modified());

    // The same disk change must prompt again after Keep, without a new write.
    window.check_external(ExternalCheck::Focus);
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    response_button(dialog.upcast_ref(), "Reload from Disk")
        .unwrap()
        .emit_clicked();
    wait_for(|| !window.flags.external_dialog_open.get());
    assert!(!window.diff_active());
    assert!(window.diff_state.borrow().is_none());
    assert_eq!(window.current_text(), disk);
    assert!(!window.flags.dirty.get());
    let canonical = window.doc.borrow().path().unwrap().to_path_buf();
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some(baseline)
    );
    // Keep also continues a pending request to enter Diff Mode.
    std::fs::write(&path, "another external change\n").unwrap();
    window
        .doc
        .borrow_mut()
        .set_last_mtime(Some(SystemTime::UNIX_EPOCH));
    window.enter_diff();
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    response_button(dialog.upcast_ref(), "Keep Current Text")
        .unwrap()
        .emit_clicked();
    wait_for(|| !window.flags.external_dialog_open.get());
    assert!(window.diff_active());
    assert_eq!(window.current_text(), disk);
    assert!(window.doc.borrow().externally_modified());
    window.window.destroy();
}

fn theme_scenario(app: &adw::Application, store: &SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());
    let hunk_red = |window: &AppWindow| {
        window
            .diff
            .left_view
            .buffer()
            .tag_table()
            .lookup("notediff-hunk")
            .unwrap()
            .property::<Option<gtk::gdk::RGBA>>("background-rgba")
            .unwrap()
            .red()
    };
    for selected in theme::catalog() {
        theme::apply(selected.clone());
        assert_eq!(window.editor_buffer.style_scheme(), selected.scheme);
        window.diff.assert_theme_smoke(&selected);
        let future = AppWindow::with_store(app, store.clone());
        assert_eq!(future.editor_buffer.style_scheme(), selected.scheme);
        future.diff.assert_theme_smoke(&selected);
        future.window.destroy();
    }
    let dark = theme::by_id("dark").unwrap();
    let light = theme::by_id("light").unwrap();

    theme::apply(light.clone());
    assert_eq!(theme::current().id, "light");
    assert_eq!(
        adw::StyleManager::default().color_scheme(),
        adw::ColorScheme::ForceLight
    );
    assert!((hunk_red(&window) - light.palette.hunk.background[0]).abs() < 0.01);

    theme::apply(dark.clone());
    assert_eq!(theme::current().id, dark.id);
    assert_eq!(
        adw::StyleManager::default().color_scheme(),
        adw::ColorScheme::ForceDark
    );
    assert!((hunk_red(&window) - dark.palette.hunk.background[0]).abs() < 0.01);
    // The hidden shortcuts cycle in both directions, wrap, and ignore repeats.
    let controllers = window.window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-theme-keys"))
        .unwrap();
    use gtk::gdk::{Key, ModifierType};
    let press = |key: Key, modifiers: ModifierType| {
        keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
    };
    let release =
        |key: Key| keys.emit_by_name::<()>("key-released", &[&key, &0u32, &ModifierType::ALT_MASK]);
    let forward = ModifierType::ALT_MASK;
    let backward = forward | ModifierType::SHIFT_MASK;
    assert!(!press(Key::Alt_R, forward));
    assert!(press(Key::t, forward));
    assert_eq!(theme::current().id, "light");
    assert!(
        window
            .last_toast
            .borrow()
            .as_ref()
            .unwrap()
            .title()
            .unwrap()
            .contains("notediff Light")
    );
    assert!(press(Key::t, forward));
    assert_eq!(theme::current().id, "light");
    release(Key::t);
    assert!(press(Key::t, forward));
    assert_eq!(theme::current().id, "cola-n-cream");
    release(Key::t);
    assert!(press(Key::t, forward));
    assert_eq!(theme::current().id, "dark");
    release(Key::t);
    assert!(press(Key::T, backward));
    assert_eq!(theme::current().id, "cola-n-cream");
    release(Key::T);
    theme::apply(dark.clone());
    window.window.destroy();
}

fn search_replace_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let editor = AppWindow::with_store(app, store.clone());
    let buffer = &editor.editor_buffer;
    // Ordinary typing must not implicitly open Find when editor focus is lost.
    assert!(editor.search_bar.key_capture_widget().is_none());
    editor.window.present();
    gtk::prelude::GtkWindowExt::set_focus(&editor.window, None::<&gtk::Widget>);
    assert!(gtk::prelude::GtkWindowExt::focus(&editor.window).is_none());
    let before = editor.current_text();
    AppWindow::finish_open_dialog(
        &editor,
        Err(gtk::glib::Error::new(
            gtk::DialogError::Dismissed,
            "Cancelled",
        )),
    );
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&editor.window),
        Some(editor.editor_view.clone().upcast::<gtk::Widget>())
    );
    assert_eq!(editor.current_text(), before);
    assert!(!editor.search_bar.is_search_mode());
    let replace = editor.window.lookup_action("replace").unwrap();
    let one = editor.window.lookup_action("replace-one").unwrap();
    let all = editor.window.lookup_action("replace-all").unwrap();
    assert!(replace.is_enabled());
    assert!(!all.is_enabled());
    replace.activate(None);
    assert!(editor.search_bar.is_search_mode());

    // Escape from Replace with must close the whole bar and restore editor
    // focus, just like Escape from Find. Exercise the installed key handler.
    editor.window.present();
    editor.search_controls.replacement.grab_focus();
    let controllers = editor.search_controls.replacement.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .unwrap();
    assert_eq!(keys.propagation_phase(), gtk::PropagationPhase::Capture);
    assert!(!keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gtk::gdk::Key::a, &0u32, &gtk::gdk::ModifierType::empty()],
    ));
    assert!(editor.search_bar.is_search_mode());
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Escape,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ],
    ));
    assert!(!editor.search_bar.is_search_mode());
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&editor.window),
        Some(editor.editor_view.clone().upcast::<gtk::Widget>())
    );
    replace.activate(None);
    assert!(editor.search_bar.is_search_mode());

    editor.set_editor_text("Cat cat CAT\né 猫 é\n");
    editor.search_entry.set_text("cat");
    editor
        .search_entry
        .emit_by_name::<()>("search-changed", &[]);
    editor.search_controls.replacement.set_text("dog");
    assert!(one.is_enabled());
    assert_eq!(editor.selection_offsets(), Some((0, 3)));
    one.activate(None);
    assert_eq!(editor.current_text(), "dog cat CAT\né 猫 é\n");
    assert_eq!(editor.selection_offsets(), Some((4, 7)));
    buffer.undo();
    assert_eq!(editor.current_text(), "Cat cat CAT\né 猫 é\n");

    // Replace All is one undo step, case-insensitive by default.
    all.activate(None);
    assert_eq!(editor.current_text(), "dog dog dog\né 猫 é\n");
    buffer.undo();
    assert_eq!(editor.current_text(), "Cat cat CAT\né 猫 é\n");
    editor.search_controls.match_case.set_active(true);
    all.activate(None);
    assert_eq!(editor.current_text(), "Cat dog CAT\né 猫 é\n");
    buffer.undo();

    // Character offsets, tab conversion and replacement containing the query.
    editor.search_entry.set_text("é");
    editor.search_controls.replacement.set_text("é\t猫");
    all.activate(None);
    assert_eq!(editor.current_text(), "Cat cat CAT\né 猫 猫 é 猫\n");
    buffer.undo();
    assert_eq!(editor.current_text(), "Cat cat CAT\né 猫 é\n");

    // Empty replacement deletes; a non-match selection is never replaced.
    editor.search_controls.replacement.set_text("");
    all.activate(None);
    assert_eq!(editor.current_text(), "Cat cat CAT\n 猫 \n");
    buffer.undo();
    buffer.select_range(&buffer.iter_at_offset(0), &buffer.iter_at_offset(3));
    one.activate(None);
    assert_eq!(editor.current_text(), "Cat cat CAT\né 猫 é\n");
    assert_eq!(editor.selection_offsets(), Some((12, 13)));

    // Literal matching, non-overlapping matches and no-match no-ops.
    editor.set_editor_text(".* .* aaaa\r\n");
    editor.search_entry.set_text(".*");
    editor.search_controls.replacement.set_text("x");
    all.activate(None);
    assert_eq!(editor.current_text(), "x x aaaa\r\n");
    buffer.undo();
    editor.search_entry.set_text("aa");
    editor.search_controls.replacement.set_text("aaa");
    all.activate(None);
    assert_eq!(editor.current_text(), ".* .* aaaaaa\r\n");
    buffer.undo();
    editor.search_entry.set_text("missing");
    editor.flags.dirty.set(false);
    all.activate(None);
    assert_eq!(editor.current_text(), ".* .* aaaa\r\n");
    assert!(!editor.flags.dirty.get());

    // Find still wraps and shares Match Case with Replace.
    editor.search_entry.set_text(".*");
    buffer.place_cursor(&buffer.end_iter());
    editor.search_entry.emit_by_name::<()>("activate", &[]);
    assert_eq!(editor.selection_offsets(), Some((0, 2)));
    editor.search_entry.emit_by_name::<()>("activate", &[]);
    assert_eq!(editor.selection_offsets(), Some((3, 5)));
    editor.search_entry.emit_by_name::<()>("activate", &[]);
    assert_eq!(editor.selection_offsets(), Some((0, 2)));

    editor.set_read_only(true);
    assert!(!replace.is_enabled());
    assert!(!one.is_enabled());
    assert!(!all.is_enabled());
    assert!(editor.window.lookup_action("find").unwrap().is_enabled());
    all.activate(None);
    assert_eq!(editor.current_text(), ".* .* aaaa\r\n");
    editor.set_read_only(false);
    assert!(all.is_enabled());
    editor.search_entry.set_text("");
    editor
        .search_entry
        .emit_by_name::<()>("search-changed", &[]);
    assert!(!one.is_enabled());
    assert!(!all.is_enabled());

    let path = dir.join("replace.md");
    std::fs::write(&path, "baseline\n").unwrap();
    AppWindow::open_path(&editor, &path);
    editor.create_baseline();
    editor.set_editor_text("changed\n");
    editor.search_entry.set_text("changed");
    editor.enter_diff();
    assert!(editor.diff_active());
    assert!(!replace.is_enabled());
    assert!(!all.is_enabled());
    assert!(!editor.search_bar.is_search_mode());
    editor.exit_diff();
    assert!(replace.is_enabled());
    assert!(all.is_enabled());
    editor.window.destroy();
}

fn editing_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    // No-tab policy. Inserted tabs become spaces, Tab edits with spaces
    // (overwriting in overwrite mode), selections indent and outdent, and
    // programmatic loads keep tabs for read-only documents.
    let editor = AppWindow::with_store(app, store.clone());
    let buffer = editor.editor_buffer.clone();
    editor.set_editor_text("ab\ncd\n");
    buffer.place_cursor(&buffer.start_iter());
    buffer.insert_at_cursor("x\ty");
    assert_eq!(editor.current_text(), "x yab\ncd\n");

    buffer.place_cursor(&buffer.start_iter());
    editor.press_tab(false);
    assert_eq!(editor.current_text(), " x yab\ncd\n");
    editor.editor_view.set_overwrite(true);
    editor.press_tab(false);
    assert_eq!(editor.current_text(), "   yab\ncd\n");
    editor.editor_view.set_overwrite(false);

    editor.set_editor_text("a\n\nb\n");
    buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
    editor.press_tab(false);
    assert_eq!(editor.current_text(), " a\n\n b\n");
    buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
    editor.press_tab(true);
    assert_eq!(editor.current_text(), "a\n\nb\n");

    editor.set_editor_text("a\tb\n");
    assert_eq!(editor.current_text(), "a\tb\n");
    editor.set_read_only(true);
    assert!(!editor.editor_view.is_editable());
    assert_eq!(editor.edit_mode(), EditMode::ReadOnly);
    editor.set_read_only(false);
    assert!(editor.editor_view.is_editable());

    // The status-bar Read Only toggle locks and unlocks a tab-free document.
    let toggle = editor.window.lookup_action("read-only").unwrap();
    editor.set_editor_text("plain\n");
    assert_eq!(toggle.state().unwrap().get::<bool>(), Some(false));
    toggle.activate(None);
    assert!(!editor.editor_view.is_editable());
    assert_eq!(toggle.state().unwrap().get::<bool>(), Some(true));
    toggle.activate(None);
    assert!(editor.editor_view.is_editable());
    assert_eq!(toggle.state().unwrap().get::<bool>(), Some(false));

    // Unlocking tabs only succeeds after Yes; No and Cancel keep the lock.
    for response in ["no", "cancel", "yes"] {
        editor.set_editor_text("a\tb\n");
        buffer.place_cursor(&buffer.iter_at_offset(2));
        editor.set_read_only(true);
        toggle.activate(None);
        let dialog = editor
            .window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        dialog.emit_by_name::<()>("response", &[&response]);
        let context = gtk::glib::MainContext::default();
        for _ in 0..100 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        let unlocked = response == "yes";
        assert_eq!(editor.editor_view.is_editable(), unlocked);
        assert_eq!(toggle.state().unwrap().get::<bool>(), Some(!unlocked));
        assert_eq!(
            editor.current_text(),
            if unlocked { "a b\n" } else { "a\tb\n" }
        );
        if unlocked {
            assert_eq!(buffer.cursor_position(), 2);
            assert!(editor.flags.dirty.get());
            assert!(buffer.can_undo());
        }
    }

    // EOL conversion: mixed text becomes uniform, as one undo step, and
    // a read-only document is refused.
    editor.set_editor_text("a\nb\r\nc\n");
    editor.convert_line_endings(LineEnding::CrLf);
    assert_eq!(editor.current_text(), "a\r\nb\r\nc\r\n");
    editor.convert_line_endings(LineEnding::Lf);
    assert_eq!(editor.current_text(), "a\nb\nc\n");
    editor.set_read_only(true);
    editor.convert_line_endings(LineEnding::CrLf);
    assert_eq!(editor.current_text(), "a\nb\nc\n");
    editor.set_read_only(false);

    // New line breaks follow the document. This covers Enter and pasted text.
    editor.set_editor_text("a\r\nb\r\n");
    editor.doc.borrow_mut().set_line_ending(LineEnding::CrLf);
    buffer.place_cursor(&buffer.end_iter());
    buffer.insert_at_cursor("\n");
    assert_eq!(editor.current_text(), "a\r\nb\r\n\r\n");
    buffer.place_cursor(&buffer.end_iter());
    buffer.insert_at_cursor("x\ny");
    assert_eq!(editor.current_text(), "a\r\nb\r\n\r\nx\r\ny");
    editor.set_editor_text("a\nb\n");
    editor.doc.borrow_mut().set_line_ending(LineEnding::Lf);
    buffer.place_cursor(&buffer.end_iter());
    buffer.insert_at_cursor("\r\n");
    assert_eq!(editor.current_text(), "a\nb\n\n");
    editor.convert_line_endings(LineEnding::CrLf);
    assert_eq!(editor.current_text(), "a\r\nb\r\n\r\n");
    buffer.place_cursor(&buffer.end_iter());
    buffer.insert_at_cursor("\n");
    assert_eq!(editor.current_text(), "a\r\nb\r\n\r\n\r\n");

    // Indentation editing: whole document, abort cases, read-only guard.
    let scaler = AppWindow::with_store(app, store.clone());
    scaler.set_editor_text("a\n  b\n    c\n");
    scaler.edit_indentation(IndentOperation::Halve);
    assert_eq!(scaler.current_text(), "a\n b\n  c\n");
    scaler.edit_indentation(IndentOperation::Double);
    assert_eq!(scaler.current_text(), "a\n  b\n    c\n");
    scaler.set_editor_text(" odd\n");
    scaler.edit_indentation(IndentOperation::Halve);
    assert_eq!(scaler.current_text(), " odd\n");
    scaler.set_editor_text("  a\n  b\n");
    scaler.set_read_only(true);
    scaler.edit_indentation(IndentOperation::Double);
    assert_eq!(scaler.current_text(), "  a\n  b\n");

    // Add Text from File: at the cursor, BOM stripped, line endings
    // matched, one undo step, inserted text selected, tabs converted,
    // and read-only documents refused.
    let inserter = AppWindow::with_store(app, store.clone());
    let ibuf = inserter.editor_buffer.clone();
    let source = dir.join("insert-me.txt");
    inserter.set_editor_text("a\nb\n");
    std::fs::write(&source, "\u{feff}X\r\nY\r\n").unwrap();
    ibuf.place_cursor(&ibuf.iter_at_line(1).unwrap());
    inserter.insert_file(&source);
    assert_eq!(inserter.current_text(), "a\nX\nY\nb\n");
    let (start, end) = ibuf.selection_bounds().unwrap();
    assert_eq!(ibuf.text(&start, &end, true).as_str(), "X\nY\n");
    assert_eq!(ibuf.cursor_position(), end.offset());
    assert!(inserter.flags.dirty.get());
    ibuf.undo();
    assert_eq!(inserter.current_text(), "a\nb\n");

    // A document with no line breaks keeps the inserted file's endings.
    inserter.set_editor_text("abc");
    ibuf.place_cursor(&ibuf.end_iter());
    std::fs::write(&source, "X\r\nY\r\n").unwrap();
    inserter.insert_file(&source);
    assert_eq!(inserter.current_text(), "abcX\r\nY\r\n");
    ibuf.undo();
    assert_eq!(inserter.current_text(), "abc");

    inserter.set_editor_text("keep SELECTED keep\n");
    ibuf.select_range(&ibuf.iter_at_offset(5), &ibuf.iter_at_offset(13));
    std::fs::write(&source, "é\t猫").unwrap();
    inserter.insert_file(&source);
    assert_eq!(inserter.current_text(), "keep é 猫 keep\n");
    let (start, end) = ibuf.selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (5, 8));
    assert_eq!(ibuf.text(&start, &end, true).as_str(), "é 猫");
    assert_eq!(ibuf.cursor_position(), end.offset());
    ibuf.undo();
    assert_eq!(inserter.current_text(), "keep SELECTED keep\n");

    // Insert Path's File… inserts the chosen absolute path like a paste,
    // replacing the selection in one undo step.
    inserter.set_editor_text("keep SELECTED keep\n");
    ibuf.select_range(&ibuf.iter_at_offset(5), &ibuf.iter_at_offset(13));
    inserter.insert_path(&source);
    let path_text = source.display().to_string();
    assert_eq!(inserter.current_text(), format!("keep {path_text} keep\n"));
    let (start, end) = ibuf.selection_bounds().unwrap();
    assert_eq!(ibuf.text(&start, &end, true).as_str(), path_text);
    assert!(inserter.flags.dirty.get());
    ibuf.undo();
    assert_eq!(inserter.current_text(), "keep SELECTED keep\n");

    // The same insertion works unchanged for a directory path.
    inserter.set_editor_text("keep");
    ibuf.place_cursor(&ibuf.end_iter());
    inserter.insert_path(dir);
    let dir_text = dir.display().to_string();
    assert_eq!(inserter.current_text(), format!("keep{dir_text}"));
    ibuf.undo();
    assert_eq!(inserter.current_text(), "keep");

    let insert_path_action = inserter.window.lookup_action("insert-path").unwrap();
    let insert_folder_action = inserter.window.lookup_action("insert-folder-path").unwrap();
    assert!(insert_path_action.is_enabled());
    assert!(insert_folder_action.is_enabled());
    inserter.set_editor_text("same\n");
    inserter.set_read_only(true);
    assert!(!insert_path_action.is_enabled());
    assert!(!insert_folder_action.is_enabled());
    inserter.insert_path(&source);
    assert_eq!(inserter.current_text(), "same\n");
    inserter.insert_file(&source);
    assert_eq!(inserter.current_text(), "same\n");
    inserter.set_read_only(false);
    assert!(insert_path_action.is_enabled());
    assert!(insert_folder_action.is_enabled());

    inserter.window.destroy();
    scaler.window.destroy();
    editor.window.destroy();
}

fn click_trailing_dot(dot: &gtk::Label) {
    let controllers = dot.observe_controllers();
    let gesture = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::GestureClick>().ok())
        .expect("the dot has a click controller");
    let args: [&dyn gtk::glib::value::ToValue; 3] = [&1i32, &0f64, &0f64];
    gesture.emit_by_name::<()>("released", &args);
}

fn trailing_whitespace_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());
    let buffer = window.editor_buffer.clone();
    let dot = window.status.trailing_label().unwrap().clone();
    let find = window
        .window
        .lookup_action("find-next-trailing-whitespace")
        .unwrap();
    let find_blank = window
        .window
        .lookup_action("find-next-non-empty-blank-line")
        .unwrap();

    // A clean document has a green, inert dot and no find action.
    window.set_editor_text("a\nb\n");
    window.update_status();
    assert!(dot.has_css_class("trailing-clean"));
    assert!(!dot.has_css_class("trailing-dirty"));
    assert_eq!(
        dot.tooltip_text().as_deref(),
        Some("No trailing whitespace")
    );
    assert!(!find.is_enabled());
    assert!(!find_blank.is_enabled());
    buffer.place_cursor(&buffer.start_iter());
    click_trailing_dot(&dot);
    find.activate(None);
    find_blank.activate(None);
    assert!(buffer.selection_bounds().is_none());

    // A yellow dot enables find. Clicking selects the next run and wraps.
    window.set_editor_text("one \ntwo \t\n");
    window.update_status();
    assert!(dot.has_css_class("trailing-dirty"));
    assert!(!dot.has_css_class("trailing-clean"));
    assert_eq!(
        dot.tooltip_text().as_deref(),
        Some("Find trailing whitespace")
    );
    assert!(find.is_enabled());
    assert!(!find_blank.is_enabled());

    buffer.place_cursor(&buffer.start_iter());
    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (3, 4));
    assert_eq!(buffer.text(&start, &end, true).as_str(), " ");

    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!(buffer.text(&start, &end, true).as_str(), " \t");

    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!(buffer.text(&start, &end, true).as_str(), " ");

    // Read-only documents keep find available.
    window.set_read_only(true);
    assert!(find.is_enabled());
    buffer.place_cursor(&buffer.start_iter());
    click_trailing_dot(&dot);
    assert!(buffer.selection_bounds().is_some());
    window.set_read_only(false);

    // A non-empty blank line turns the dot red. Clicking it finds blank lines;
    // Alt shows the yellow state and finds trailing whitespace instead.
    window.set_editor_text("one \n \ntwo\n\t\n");
    window.update_status();
    assert!(dot.has_css_class("trailing-blank"));
    assert!(!dot.has_css_class("trailing-dirty"));
    assert!(!dot.has_css_class("trailing-clean"));
    assert_eq!(
        dot.tooltip_text().as_deref(),
        Some("Find non-empty blank line")
    );
    assert!(find.is_enabled());
    assert!(find_blank.is_enabled());

    buffer.place_cursor(&buffer.start_iter());
    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (5, 6));
    assert_eq!(buffer.text(&start, &end, true).as_str(), " ");
    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (11, 12));
    assert_eq!(buffer.text(&start, &end, true).as_str(), "\t");
    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (5, 6));

    let controllers = window.window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-alt-indicator"))
        .unwrap();
    let _ = keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Alt_L,
            &0u32,
            &gtk::gdk::ModifierType::ALT_MASK,
        ],
    );
    assert!(dot.has_css_class("trailing-dirty"));
    assert!(!dot.has_css_class("trailing-blank"));
    assert_eq!(
        dot.tooltip_text().as_deref(),
        Some("Find trailing whitespace")
    );
    buffer.place_cursor(&buffer.start_iter());
    click_trailing_dot(&dot);
    let (start, end) = buffer.selection_bounds().unwrap();
    assert_eq!((start.offset(), end.offset()), (3, 4));
    assert_eq!(buffer.text(&start, &end, true).as_str(), " ");

    keys.emit_by_name::<()>(
        "key-released",
        &[
            &gtk::gdk::Key::Alt_L,
            &0u32,
            &gtk::gdk::ModifierType::ALT_MASK,
        ],
    );
    assert!(dot.has_css_class("trailing-blank"));

    // Read-only documents keep the red find available.
    window.set_read_only(true);
    assert!(find.is_enabled());
    assert!(find_blank.is_enabled());
    buffer.place_cursor(&buffer.start_iter());
    click_trailing_dot(&dot);
    assert!(buffer.selection_bounds().is_some());
    window.set_read_only(false);

    // Diff Mode does nothing for either find.
    let path = dir.join("trailing-diff.txt");
    std::fs::write(&path, "base \n").unwrap();
    let review = AppWindow::with_store(app, store.clone());
    let review_buffer = review.editor_buffer.clone();
    AppWindow::open_path(&review, &path);
    review.create_baseline();
    review.set_editor_text("changed \n \n");
    review.enter_diff();
    review_buffer.place_cursor(&review_buffer.start_iter());
    let review_find = review
        .window
        .lookup_action("find-next-trailing-whitespace")
        .unwrap();
    let review_blank = review
        .window
        .lookup_action("find-next-non-empty-blank-line")
        .unwrap();
    review_find.activate(None);
    review_blank.activate(None);
    assert!(review_buffer.selection_bounds().is_none());
    review.exit_diff();
    review.window.destroy();

    window.window.destroy();
}

fn indentation_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let window = AppWindow::with_store(app, store.clone());
    let buffer = window.editor_buffer.clone();
    let increase = window
        .window
        .lookup_action("indent-increase-include-empty")
        .unwrap();
    let decrease = window
        .window
        .lookup_action("indent-decrease-no-flattening")
        .unwrap();
    let double = window.window.lookup_action("indent-double").unwrap();
    let halve = window.window.lookup_action("indent-halve").unwrap();
    let all = [&increase, &decrease, &double, &halve];

    // Increase touches every target line, including the empty line after a
    // terminating break.
    window.set_editor_text("a\n  b\n");
    increase.activate(None);
    assert_eq!(window.current_text(), " a\n   b\n ");
    buffer.undo();
    assert_eq!(window.current_text(), "a\n  b\n");

    // A multi-line operation is one undo step.
    window.set_editor_text("a\n b\nc");
    increase.activate(None);
    assert_eq!(window.current_text(), " a\n  b\n c");
    buffer.undo();
    assert_eq!(window.current_text(), "a\n b\nc");

    // Decrease ignores the terminating line.
    window.set_editor_text(" a\n b\n");
    decrease.activate(None);
    assert_eq!(window.current_text(), "a\nb\n");
    buffer.undo();
    assert_eq!(window.current_text(), " a\n b\n");

    // A deliberate middle empty line still blocks it.
    window.set_editor_text(" a\n\n b\n");
    decrease.activate(None);
    assert_eq!(window.current_text(), " a\n\n b\n");

    // An unindented non-empty line blocks it too.
    window.set_editor_text("a\n b\n");
    decrease.activate(None);
    assert_eq!(window.current_text(), "a\n b\n");

    // Empty lines indented by Increase can then be decreased normally.
    window.set_editor_text("a\n\nb");
    increase.activate(None);
    assert_eq!(window.current_text(), " a\n \n b");
    decrease.activate(None);
    assert_eq!(window.current_text(), "a\n\nb");

    // Double and Halve keep their warnings and leave the text unchanged.
    window.set_editor_text("a\nb\n");
    double.activate(None);
    assert_eq!(window.current_text(), "a\nb\n");
    assert_toast(&window, ToastKind::Warning, "No indentation to double");
    window.set_editor_text(" odd\n");
    halve.activate(None);
    assert_eq!(window.current_text(), " odd\n");
    assert_toast(
        &window,
        ToastKind::Warning,
        "Cannot halve indentation: line 1 has an odd number of leading spaces",
    );

    // Read-only disables all four operations.
    window.set_editor_text("  a\n  b\n");
    window.set_read_only(true);
    for action in &all {
        assert!(!action.is_enabled());
    }
    increase.activate(None);
    double.activate(None);
    assert_eq!(window.current_text(), "  a\n  b\n");
    window.set_read_only(false);
    for action in &all {
        assert!(action.is_enabled());
    }

    // Diff Mode does nothing.
    let path = dir.join("indent-diff.txt");
    std::fs::write(&path, "base\n").unwrap();
    let review = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&review, &path);
    review.create_baseline();
    review.set_editor_text("  changed\n");
    review.enter_diff();
    let before = review.current_text();
    review
        .window
        .lookup_action("indent-double")
        .unwrap()
        .activate(None);
    assert_eq!(review.current_text(), before);
    review.exit_diff();
    review.window.destroy();

    window.window.destroy();
}

fn file_actions_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    // Rename, move to a folder, and the baseline following the file.
    let renamer = AppWindow::with_store(app, store.clone());
    let before = dir.join("before.md");
    std::fs::write(&before, "alpha\n").unwrap();
    AppWindow::open_path(&renamer, &before);
    renamer.create_baseline();
    let before_canonical = renamer.doc.borrow().path().unwrap().to_path_buf();
    assert!(renamer.store.has_for_canonical(&before_canonical));

    renamer.rename_file("after.md");
    let after = before_canonical.with_file_name("after.md");
    assert_eq!(renamer.doc.borrow().path(), Some(after.as_path()));
    assert!(!renamer.store.has_for_canonical(&before_canonical));
    assert!(renamer.store.has_for_canonical(&after));
    assert_eq!(renamer.diff_button.label().as_deref(), Some(REVIEW_LABEL));

    let sub = after.parent().unwrap().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    renamer.rename_file("sub/");
    let moved = sub.join("after.md");
    assert_eq!(renamer.doc.borrow().path(), Some(moved.as_path()));
    assert!(moved.exists() && !after.exists());
    assert!(!renamer.store.has_for_canonical(&after));
    assert_eq!(
        renamer.store.read_for_canonical(&moved).unwrap().as_deref(),
        Some("alpha\n")
    );

    // A refused rename changes nothing.
    renamer.rename_file("nope/x.md");
    assert_eq!(renamer.doc.borrow().path(), Some(moved.as_path()));

    // Link round trip: the dialog's target, then an orphan moved onto a
    // document that has no baseline.
    let linker = AppWindow::with_store(app, store.clone());
    let fresh = dir.join("fresh.md");
    std::fs::write(&fresh, "fresh\n").unwrap();
    AppWindow::open_path(&linker, &fresh);
    let fresh_canonical = linker.doc.borrow().path().unwrap().to_path_buf();
    let target = linker.link_target().unwrap();
    assert_eq!(target.path, fresh_canonical);
    assert_eq!(target.text, "fresh\n");
    assert_eq!(linker.diff_button.label().as_deref(), Some(CREATE_LABEL));

    let orphan = Path::new("/notediff-test-gone/orphan.md");
    linker
        .store
        .write_for_canonical(orphan, "orphan base\n")
        .unwrap();
    let orphan_snapshot = linker.store.snapshot_path_for_canonical(orphan);
    linker
        .store
        .link(&orphan_snapshot, &fresh_canonical, LinkMode::Move)
        .unwrap();
    linker.update_diff_button();
    assert_eq!(linker.diff_button.label().as_deref(), Some(REVIEW_LABEL));
    assert!(!orphan_snapshot.exists());
    assert!(linker.link_target().is_some());

    // Save As carry-over: copy the baseline of the moved file.
    let copier = AppWindow::with_store(app, store.clone());
    let copy_path = dir.join("copy.md");
    std::fs::write(&copy_path, "alpha\n").unwrap();
    AppWindow::open_path(&copier, &copy_path);
    let copy_canonical = copier.doc.borrow().path().unwrap().to_path_buf();
    copier.copy_baseline_from(&moved);
    assert_eq!(
        copier
            .store
            .read_for_canonical(&copy_canonical)
            .unwrap()
            .as_deref(),
        Some("alpha\n")
    );
    assert!(copier.store.has_for_canonical(&moved));
    assert_eq!(copier.diff_button.label().as_deref(), Some(REVIEW_LABEL));

    copier.window.destroy();
    linker.window.destroy();
    renamer.window.destroy();
}

fn duplicate_open_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let shared = dir.join("duplicate-shared.txt");
    std::fs::write(&shared, "shared\n").unwrap();
    let owner = AppWindow::with_store(app, store.clone());
    owner.window.present();
    AppWindow::open_path(&owner, &shared);
    let canonical = shared.canonicalize().unwrap();
    assert_eq!(owner.doc.borrow().path(), Some(canonical.as_path()));

    // Another window's open request raises the owner and leaves both
    // documents alone.
    let requester = AppWindow::with_store(app, store.clone());
    requester.window.present();
    requester.set_editor_text("draft\n");
    let before = super::WINDOWS.with_borrow(|windows| windows.len());
    AppWindow::finish_open_dialog(&requester, Ok(gio::File::for_path(&shared)));
    assert_eq!(super::WINDOWS.with_borrow(|windows| windows.len()), before);
    assert!(requester.doc.borrow().path().is_none());
    assert_eq!(requester.current_text(), "draft\n");
    assert_toast(
        &owner,
        ToastKind::Info,
        "File is already open in this window",
    );
    assert_toast(
        &requester,
        ToastKind::Info,
        "File is already open in another window",
    );

    // Opening the current window's own path is a reload, not a duplicate.
    // A clean document follows the disk text without a prompt.
    std::fs::write(&shared, "changed on disk\n").unwrap();
    assert_eq!(owner.current_text(), "shared\n");
    AppWindow::finish_open_dialog(&owner, Ok(gio::File::for_path(&shared)));
    assert_eq!(owner.current_text(), "changed on disk\n");
    assert!(!owner.flags.dirty.get());
    assert_eq!(owner.doc.borrow().path(), Some(canonical.as_path()));

    // Identical content is left untouched, preserving cursor, scroll and
    // undo history.
    let before_text = owner.current_text();
    AppWindow::finish_open_dialog(&owner, Ok(gio::File::for_path(&shared)));
    assert_eq!(owner.current_text(), before_text);
    assert_toast(
        &owner,
        ToastKind::Info,
        "Document is identical with its file on disk",
    );

    // A changed mtime with identical text is acknowledged without replacing
    // the buffer, so the external-change prompt does not return.
    owner.doc.borrow_mut().set_last_mtime(None);
    assert!(owner.doc.borrow().externally_modified());
    AppWindow::finish_open_dialog(&owner, Ok(gio::File::for_path(&shared)));
    assert_eq!(owner.current_text(), before_text);
    assert!(!owner.doc.borrow().externally_modified());
    assert_toast(
        &owner,
        ToastKind::Info,
        "Document is identical with its file on disk",
    );

    // Unsaved edits require confirmation. Cancel keeps them.
    owner
        .editor_buffer
        .place_cursor(&owner.editor_buffer.end_iter());
    owner.editor_buffer.insert_at_cursor("dirty");
    assert!(owner.flags.dirty.get());
    AppWindow::finish_open_dialog(&owner, Ok(gio::File::for_path(&shared)));
    wait_for(|| {
        owner
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = owner
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(dialog.heading().as_deref(), Some("Reload from Disk?"));
    assert_eq!(dialog.body().as_str(), "The document has unsaved changes.");
    dialog.close();
    wait_for(|| owner.window.visible_dialog().is_none());
    assert_eq!(owner.current_text(), "changed on disk\ndirty");
    assert!(owner.flags.dirty.get());

    // Confirming reloads the saved disk text and clears the dirty state.
    AppWindow::finish_open_dialog(&owner, Ok(gio::File::for_path(&shared)));
    wait_for(|| {
        owner
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = owner
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Reload from Disk")
        .unwrap()
        .emit_clicked();
    wait_for(|| owner.window.visible_dialog().is_none());
    assert_eq!(owner.current_text(), "changed on disk\n");
    assert!(!owner.flags.dirty.get());

    // A new-window request for the same path does not create a window.
    let before = super::WINDOWS.with_borrow(|windows| windows.len());
    super::open_window_for_path_with(
        app,
        &shared,
        store.clone(),
        false,
        RecentDocuments::disabled(),
        Some(&requester),
    );
    assert_eq!(super::WINDOWS.with_borrow(|windows| windows.len()), before);
    assert_toast(
        &owner,
        ToastKind::Info,
        "File is already open in this window",
    );
    assert_toast(
        &requester,
        ToastKind::Info,
        "File is already open in another window",
    );

    // A pending Convert Tabs prompt reserves the path as well.
    let tabs_file = dir.join("duplicate-tabs.txt");
    std::fs::write(&tabs_file, "a\tb\n").unwrap();
    AppWindow::open_path(&owner, &tabs_file);
    wait_for(|| {
        owner
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let before = super::WINDOWS.with_borrow(|windows| windows.len());
    AppWindow::finish_open_dialog(&requester, Ok(gio::File::for_path(&tabs_file)));
    assert_eq!(super::WINDOWS.with_borrow(|windows| windows.len()), before);
    assert!(requester.doc.borrow().path().is_none());
    assert_toast(
        &owner,
        ToastKind::Info,
        "File is already open in this window",
    );
    assert_toast(
        &requester,
        ToastKind::Info,
        "File is already open in another window",
    );
    owner.window.visible_dialog().unwrap().close();
    wait_for(|| owner.window.visible_dialog().is_none());
    assert_eq!(owner.doc.borrow().path(), Some(canonical.as_path()));

    // Rename or Move updates the path that duplicate detection follows.
    owner.rename_file("duplicate-renamed.txt");
    let renamed = dir.join("duplicate-renamed.txt").canonicalize().unwrap();
    assert_eq!(owner.doc.borrow().path(), Some(renamed.as_path()));
    let before = super::WINDOWS.with_borrow(|windows| windows.len());
    super::open_window_for_path_with(
        app,
        &renamed,
        store.clone(),
        false,
        RecentDocuments::disabled(),
        Some(&requester),
    );
    assert_eq!(super::WINDOWS.with_borrow(|windows| windows.len()), before);
    assert_toast(
        &owner,
        ToastKind::Info,
        "File is already open in this window",
    );
    assert_toast(
        &requester,
        ToastKind::Info,
        "File is already open in another window",
    );

    requester.window.destroy();
    owner.window.destroy();
}

fn save_as_duplicate_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let owned = dir.join("save-as-owned.txt");
    std::fs::write(&owned, "owner\n").unwrap();
    let owner = AppWindow::with_store(app, store.clone());
    owner.window.present();
    AppWindow::open_path(&owner, &owned);
    let owner_path = owner.doc.borrow().path().map(Path::to_path_buf);
    assert!(owner_path.is_some());

    let draft = AppWindow::with_store(app, store.clone());
    draft.window.present();
    draft.set_editor_text("draft\n");

    let expect_refused = |window: &Rc<AppWindow>| {
        wait_for(|| {
            window
                .window
                .visible_dialog()
                .is_some_and(|dialog| dialog.is_mapped())
        });
        let dialog = window
            .window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        assert_eq!(dialog.heading().as_deref(), Some("Could not save"));
        assert_eq!(
            dialog.body().as_str(),
            "That file is already open in another window."
        );
        dialog.close();
        wait_for(|| window.window.visible_dialog().is_none());
    };

    // Save As must not write over a file owned by another window.
    draft.save_as_path(&owned, None);
    expect_refused(&draft);
    assert!(draft.doc.borrow().path().is_none());
    assert_eq!(draft.current_text(), "draft\n");
    assert_eq!(std::fs::read_to_string(&owned).unwrap(), "owner\n");
    assert_eq!(owner.doc.borrow().path(), owner_path.as_deref());
    assert_eq!(owner.current_text(), "owner\n");

    // A symlink resolves to the same canonical target.
    let link = dir.join("save-as-owned-link.txt");
    std::os::unix::fs::symlink(&owned, &link).unwrap();
    draft.save_as_path(&link, None);
    expect_refused(&draft);
    assert!(draft.doc.borrow().path().is_none());

    // A file deleted from disk is still owned by the window that has it open.
    let removed = dir.join("save-as-removed.txt");
    std::fs::write(&removed, "removed\n").unwrap();
    let removed_window = AppWindow::with_store(app, store.clone());
    removed_window.window.present();
    AppWindow::open_path(&removed_window, &removed);
    std::fs::remove_file(&removed).unwrap();
    draft.save_as_path(&removed, None);
    expect_refused(&draft);
    assert!(draft.doc.borrow().path().is_none());
    assert!(!removed.exists());

    // A path with no other owner still saves normally.
    let fresh = dir.join("save-as-fresh.txt");
    draft.save_as_path(&fresh, None);
    let canonical = fresh.canonicalize().unwrap();
    assert_eq!(draft.doc.borrow().path(), Some(canonical.as_path()));
    assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "draft\n");
    assert_toast(&draft, ToastKind::Success, "Saved");

    removed_window.window.destroy();
    draft.window.destroy();
    owner.window.destroy();
}

fn inline_inspection_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    use gtk::gdk::{Key, ModifierType};

    let path = dir.join("inline-inspection.txt");
    let old = "The quick brown fox jumps.\r\n";
    let live = "The fast brown fox sleeps.\n";
    std::fs::write(&path, old).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    let canonical = window.doc.borrow().path().unwrap().to_path_buf();
    window.set_editor_text(live);
    window.persist(None).unwrap();
    window.window.present();
    let controllers = window.window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-diff-keys"))
        .unwrap();
    let press =
        |key: Key| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::empty()]);
    window.enter_diff();
    assert!(press(Key::Down));
    assert!(press(Key::Return));
    assert_eq!(window.diff.accept_button.label().as_deref(), Some("Accept"));
    let selected_tag = window
        .diff
        .active_left_view()
        .buffer()
        .tag_table()
        .lookup("notediff-inline-selected")
        .unwrap();
    assert_eq!(selected_tag.underline(), gtk::pango::Underline::None);
    for view in [
        window.diff.active_left_view(),
        window.diff.active_right_view(),
    ] {
        let buffer = view.buffer();
        // The hunk is already isolated: unchanged context needs no background.
        assert!(buffer.start_iter().tags().is_empty());
        for name in ["notediff-hunk", "notediff-focus"] {
            let tag = buffer.tag_table().lookup(name).unwrap();
            for offset in 0..buffer.char_count() {
                assert!(!buffer.iter_at_offset(offset).has_tag(&tag));
            }
        }
    }
    let buffer = window.diff.active_left_view().buffer();
    let pending = buffer.tag_table().lookup("notediff-inline-normal").unwrap();
    assert!(buffer.iter_at_offset(19).has_tag(&pending));
    assert_eq!(
        pending.property::<Option<gtk::gdk::RGBA>>("background-rgba"),
        Some(theme::rgba(
            theme::current().palette.focused_hunk.background
        ))
    );
    assert_eq!(pending.underline(), gtk::pango::Underline::None);
    assert!(!pending.property::<bool>("underline-set"));
    assert_eq!(
        pending.property::<Option<gtk::gdk::RGBA>>("foreground-rgba"),
        Some(theme::rgba(
            theme::current().palette.focused_hunk.foreground
        ))
    );
    assert!(!pending.property::<bool>("weight-set"));
    assert_eq!(
        selected_tag.property::<Option<gtk::gdk::RGBA>>("background-rgba"),
        Some(theme::rgba(
            theme::current().palette.focused_word.background
        ))
    );
    // The persistent full-diff views retain whole-hunk highlighting.
    let full = window.diff.left_view.buffer();
    let hunk_tag = full.tag_table().lookup("notediff-hunk").unwrap();
    assert!(full.start_iter().has_tag(&hunk_tag));
    let word_tag = full.tag_table().lookup("notediff-word").unwrap();
    assert_eq!(
        word_tag.property::<Option<gtk::gdk::RGBA>>("background-rgba"),
        Some(theme::rgba(theme::current().palette.word.background))
    );
    assert!(
        window
            .diff
            .active_left_view()
            .buffer()
            .iter_at_offset(4)
            .has_tag(&selected_tag)
    );
    assert!(
        !window
            .diff
            .active_left_view()
            .buffer()
            .iter_at_offset(19)
            .has_tag(&selected_tag)
    );
    assert!(press(Key::Down));
    assert!(
        !window
            .diff
            .active_left_view()
            .buffer()
            .iter_at_offset(4)
            .has_tag(&selected_tag)
    );
    assert!(
        window
            .diff
            .active_left_view()
            .buffer()
            .iter_at_offset(19)
            .has_tag(&selected_tag)
    );
    assert!(press(Key::Right));
    let accepted = "The quick brown fox sleeps.\r\n";
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().baseline(),
        accepted
    );
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .selected_inline()
            .unwrap()
            .kind,
        ChangeKind::Ending
    );
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some(old)
    );
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (1 change)")
    );
    assert!(press(Key::Left));
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().inline_focus(),
        Some(1)
    );
    assert_eq!(window.diff_state.borrow().as_ref().unwrap().baseline(), old);
    assert!(press(Key::Right));

    // Failed Confirm preserves the inline selection and its undo history.
    let snapshot = store.snapshot_path_for_canonical(&canonical);
    let backup = snapshot.with_extension("backup");
    std::fs::rename(&snapshot, &backup).unwrap();
    std::fs::create_dir(&snapshot).unwrap();
    window.diff.exit_button.emit_clicked();
    assert!(window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .selected_inline()
            .unwrap()
            .kind,
        ChangeKind::Ending
    );
    assert!(window.diff.undo_button.is_sensitive());
    window.window.visible_dialog().unwrap().close();
    wait_for(|| window.window.visible_dialog().is_none());
    std::fs::remove_dir(&snapshot).unwrap();
    std::fs::rename(&backup, &snapshot).unwrap();

    window.diff.exit_button.emit_clicked();
    assert!(window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Back"));
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some(accepted)
    );
    assert!(!window.diff.undo_button.is_sensitive());
    assert!(press(Key::Right));
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().baseline(),
        "The quick brown fox sleeps.\n"
    );
    assert!(press(Key::Escape));
    assert_eq!(window.diff.accept_button.label().as_deref(), Some("Accept"));
    assert!(press(Key::Return));
    assert!(press(Key::Right));
    assert!(!window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().baseline(),
        live
    );
    assert!(window.diff.exit_button.has_css_class("success"));
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (2 changes)")
    );
    window.diff.exit_button.emit_clicked();
    assert!(!window.diff_active());
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some(live)
    );
    // A zero-width side gets a caret overlay at the exact boundary; no
    // neighbouring character is tagged as if it were part of the edit.
    let insertion = "The fast brown fox quietly sleeps.\n";
    window.set_editor_text(insertion);
    window.enter_diff();
    window.focus_relative(1);
    window.diff.inspect_button.emit_clicked();
    assert_eq!(window.diff.inspection_caret(true), (true, Some(19)));
    assert!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .selected_inline()
            .unwrap()
            .old
            .is_empty()
    );
    window.accept_focused();
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().baseline(),
        insertion
    );
    window.undo_accept();
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().baseline(),
        live
    );
    window.window.destroy();

    // Navigation within one wrapped paragraph must reveal the selected words,
    // not repeatedly reset the excerpt to its top.
    let path = dir.join("wrapped-inline.txt");
    let old = format!("before {}oldend\n", "same ".repeat(1500));
    let live = format!("after {}newend\n", "same ".repeat(1500));
    std::fs::write(&path, &old).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text(&live);
    window.window.present();
    window.enter_diff();
    window.focus_relative(1);
    window.diff.inspect_button.emit_clicked();
    wait_for(|| window.diff.panes_ready());
    let view = window.diff.active_left_view();
    assert_eq!(view.vadjustment().unwrap().value(), 0.0);
    window.focus_relative(1);
    wait_for(|| window.diff.panes_ready());
    let offset = live.find("newend").unwrap() as i32;
    let location = view.iter_location(&view.buffer().iter_at_offset(offset));
    let visible = view.visible_rect();
    assert!(view.vadjustment().unwrap().value() > 0.0);
    assert!(location.y() >= visible.y() && location.y() < visible.y() + visible.height());
    window.accept_focused();
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().baseline(),
        old.replace("oldend", "newend")
    );
    window.undo_accept();
    assert_eq!(window.diff_state.borrow().as_ref().unwrap().baseline(), old);
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().inline_focus(),
        Some(1)
    );
    window.window.destroy();
}

fn whitespace_highlight_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("whitespace-highlight.txt");
    let old = "- Font size and per-window zoom (Ctrl+scroll, Ctrl+plus/minus), with the default size in the config file.\n";
    let live = "- Per-window zoom (Ctrl+scroll, Ctrl+plus/minus) relative to the persisted application-wide font size, with a reset-to-default action.\n";
    std::fs::write(&path, old).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text(live);
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();
    window.focus_relative(1);
    window.diff.inspect_button.emit_clicked();
    wait_for(|| window.diff.inspection_active() && window.diff.panes_ready());
    window.focus_relative(1);

    let assert_tight = |window: &AppWindow| {
        let buffer = window.diff.active_left_view().buffer();
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let relative = text.find("relative").unwrap() as i32;
        let after_to = relative + "relative to".len() as i32;
        let selected = buffer
            .tag_table()
            .lookup("notediff-inline-selected")
            .unwrap();
        let normal = buffer.tag_table().lookup("notediff-inline-normal").unwrap();
        for tag in [&selected, &normal] {
            assert!(buffer.iter_at_offset(relative).has_tag(tag));
            assert!(!buffer.iter_at_offset(relative - 1).has_tag(tag));
            assert!(!buffer.iter_at_offset(after_to).has_tag(tag));
        }
    };
    assert_tight(&window);

    // Accepting the first change re-diffs the line and must not make the
    // selected successor absorb either boundary space.
    window.focus_relative(-1);
    window.accept_focused();
    assert_tight(&window);
    window.window.destroy();
}

fn inspection_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    use gtk::gdk::{Key, ModifierType};

    let path = dir.join("inspection.txt");
    let old = format!("text {}\n", "α".repeat(4_200));
    let new = format!("text {}\n", "β".repeat(6_500));
    let added = "added first\n\nadded last\n";
    let removed = "removed first\n\nremoved at EOF";
    let baseline = format!("header\n{old}anchor one\nanchor two\n{removed}");
    let live = format!("header\n{new}anchor one\n{added}anchor two\n");
    let first_accepted = format!("header\n{new}anchor one\nanchor two\n{removed}");
    std::fs::write(&path, &baseline).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    let canonical = window.doc.borrow().path().unwrap().to_path_buf();
    window.set_editor_text(&live);
    window.persist(None).unwrap();
    window.window.present();

    let controllers = window.window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-diff-keys"))
        .unwrap();
    let press =
        |key: Key| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::empty()]);
    let pane = |view: &sourceview5::View| {
        let buffer = view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), true)
    };
    let assert_panes = |left: &str, right: &str, wrap| {
        // Blank lines inside a hunk carry the display-only pilcrow. Compare
        // the document text and leave marker-specific assertions to the
        // dedicated blank-marker scenario.
        assert_eq!(
            pane(window.diff.active_left_view()).replace('\u{b6}', ""),
            left
        );
        assert_eq!(
            pane(window.diff.active_right_view()).replace('\u{b6}', ""),
            right
        );
        assert_eq!(window.diff.active_left_view().wrap_mode(), wrap);
        assert_eq!(window.diff.active_right_view().wrap_mode(), wrap);
    };
    let assert_snapshot = |text: &str| {
        assert_eq!(
            store.read_for_canonical(&canonical).unwrap().as_deref(),
            Some(text)
        );
    };
    let assert_selected_block = |view: &sourceview5::View| {
        let buffer = view.buffer();
        let selected = buffer
            .tag_table()
            .lookup("notediff-inline-selected")
            .unwrap();
        for offset in 0..buffer.char_count() {
            assert!(buffer.iter_at_offset(offset).has_tag(&selected));
        }
    };
    let wrapped = gtk::WrapMode::WordChar;
    let full = gtk::WrapMode::None;

    window.enter_diff();
    assert_eq!(
        window.diff.header_texts().0,
        crate::ui::display_path(&canonical, &gtk::glib::home_dir())
    );
    assert_eq!(
        window.diff.document_path_tooltip().as_deref(),
        Some(canonical.to_string_lossy().as_ref())
    );
    let (left_time, right_time) = window.diff.header_times();
    assert_timestamp(&left_time);
    assert_timestamp(&right_time);
    assert_eq!(
        window.diff.header_tooltips(),
        (
            Some("Last saved".to_string()),
            Some("Last saved".to_string())
        )
    );
    assert!(!window.diff.inspect_button.is_sensitive());
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Close"));
    // The toolbar is immediately followed by the panes, with no information
    // row (including hidden/reserved labels) between them.
    let toolbar = window.diff.root.first_child().unwrap();
    assert!(toolbar.has_css_class("diff-toolbar"));
    assert!(toolbar.next_sibling().unwrap().next_sibling().is_none());
    assert_eq!(
        window.diff.inspect_button.tooltip_text().as_deref(),
        Some("Inspect selected hunk (Enter)")
    );
    assert_eq!(
        window.diff.accept_button.tooltip_text().as_deref(),
        Some("Accept selected hunk (Right)")
    );
    assert_eq!(
        window.diff.undo_button.tooltip_text().as_deref(),
        Some("Undo last accept (Left)")
    );
    assert!(window.diff.exit_button.tooltip_text().is_none());
    assert!(!press(Key::Return));
    assert!(press(Key::Down));
    assert!(window.diff.inspect_button.is_sensitive());
    window.diff.inspect_button.emit_clicked();
    wait_for(|| window.diff.inspection_active() && window.diff.panes_ready());
    assert_panes(&new, &old, wrapped);
    // Full-diff blank lines inside a hunk carry the display-only pilcrow.
    let left = pane(&window.diff.left_view).replace('\u{b6}', "");
    let right = pane(&window.diff.right_view).replace('\u{b6}', "");
    assert_eq!(left, live);
    assert_eq!(right, baseline);
    assert!(!window.diff.active_left_view().is_editable());
    assert!(!window.diff.active_right_view().is_editable());
    assert!(!window.diff.inspect_button.is_visible());
    assert!(!window.diff.accept_all_button.is_visible());
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Back"));
    assert_eq!(
        window.diff.exit_button.tooltip_text().as_deref(),
        Some("Return to full view (Enter)")
    );
    assert_eq!(
        window.diff.inspect_button.next_sibling().as_ref(),
        Some(window.diff.accept_button.upcast_ref::<gtk::Widget>())
    );
    assert_eq!(window.diff.accept_button.label().as_deref(), Some("Accept"));
    assert_eq!(
        toolbar.last_child().as_ref(),
        Some(window.diff.exit_button.upcast_ref::<gtk::Widget>())
    );
    assert_eq!(
        window.diff.previous_button.tooltip_text().as_deref(),
        Some("Previous change (Up)")
    );
    assert_eq!(
        window.diff.next_button.tooltip_text().as_deref(),
        Some("Next change (Down)")
    );
    assert_eq!(
        window.diff.accept_button.tooltip_text().as_deref(),
        Some("Accept selected change (Right)")
    );
    let initial_focus = window.diff_state.borrow().as_ref().unwrap().focus();
    window.diff.exit_button.emit_clicked();
    assert!(window.diff_active());
    assert_panes(&live, &baseline, full);
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().focus(),
        initial_focus
    );
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Close"));
    assert!(window.diff.exit_button.tooltip_text().is_none());
    assert_snapshot(&baseline);
    assert!(press(Key::Return));
    assert_panes(&new, &old, wrapped);

    // Both excerpts overflow after wrapping. Wait for the existing scroll
    // placement callback to settle before checking pane independence.
    let left = window.diff.active_left_view().vadjustment().unwrap();
    let right = window.diff.active_right_view().vadjustment().unwrap();
    let frames = Rc::new(std::cell::Cell::new(0));
    let ticks = frames.clone();
    window.diff.root.add_tick_callback(move |_, _| {
        ticks.set(ticks.get() + 1);
        if ticks.get() == 4 {
            gtk::glib::ControlFlow::Break
        } else {
            gtk::glib::ControlFlow::Continue
        }
    });
    wait_for(|| frames.get() >= 4);
    assert!(left.upper() > left.page_size());
    assert!(right.upper() > right.page_size());
    left.set_value(48.0);
    assert!(left.value() > 0.0);
    assert_eq!(right.value(), 0.0);

    assert!(press(Key::Right));
    assert_panes(added, "", wrapped);
    assert_selected_block(window.diff.active_left_view());
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .selected_inline()
            .unwrap()
            .kind,
        ChangeKind::Block
    );
    assert_eq!(pane(&window.diff.left_view).replace('\u{b6}', ""), live);
    assert_eq!(
        pane(&window.diff.right_view).replace('\u{b6}', ""),
        first_accepted
    );
    assert_eq!(window.diff.header_texts().1, "Baseline");
    assert_snapshot(&baseline);

    assert!(press(Key::Down));
    assert_panes("", removed, wrapped);
    assert_selected_block(window.diff.active_right_view());
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .selected_inline()
            .unwrap()
            .kind,
        ChangeKind::Block
    );
    assert_eq!(
        window.diff.header_texts().0,
        crate::ui::display_path(&canonical, &gtk::glib::home_dir())
    );
    assert_eq!(
        window.diff.document_path_tooltip().as_deref(),
        Some(canonical.to_string_lossy().as_ref())
    );
    assert!(press(Key::Up));
    assert_panes(added, "", wrapped);

    let focus = window.diff_state.borrow().as_ref().unwrap().focus();
    let selected = window
        .diff_state
        .borrow()
        .as_ref()
        .unwrap()
        .selected_inline()
        .cloned();
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (1 change)")
    );
    assert!(press(Key::Escape));
    assert_panes(&live, &first_accepted, full);
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (1 change)")
    );
    assert_eq!(window.diff_state.borrow().as_ref().unwrap().focus(), focus);
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .accepted_hunks(),
        1
    );
    assert_snapshot(&baseline);
    assert!(window.diff.undo_button.is_sensitive());
    assert!(window.diff.accept_all_button.is_visible());
    assert!(press(Key::KP_Enter));
    assert_panes(added, "", wrapped);
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .selected_inline(),
        selected.as_ref()
    );
    assert!(press(Key::Left));
    assert_panes(&new, &old, wrapped);
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Back"));
    assert!(press(Key::Return));
    assert_panes(&live, &baseline, full);
    window.diff.inspect_button.emit_clicked();
    assert!(press(Key::Escape));
    assert_panes(&live, &baseline, full);

    assert!(press(Key::Return));
    window.diff.accept_button.emit_clicked();

    window.flags.dirty.set(true);
    window.diff.exit_button.emit_clicked();
    assert!(!press(Key::Return));
    assert!(!press(Key::Escape));
    window.window.visible_dialog().unwrap().close();
    wait_for(|| window.window.visible_dialog().is_none());
    window.flags.dirty.set(false);
    assert_panes(added, "", wrapped);
    assert_snapshot(&baseline);

    // A partial Confirm checkpoints the baseline without leaving inspection.
    assert_eq!(
        window.diff.exit_button.tooltip_text().as_deref(),
        Some("Update baseline")
    );
    window.diff.exit_button.emit_clicked();
    assert_panes(added, "", wrapped);
    assert!(!window.diff.undo_button.is_sensitive());
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Back"));
    assert_eq!(
        window.diff.exit_button.tooltip_text().as_deref(),
        Some("Return to full view (Enter)")
    );
    assert_snapshot(&first_accepted);
    window.diff.exit_button.emit_clicked();
    assert!(window.diff_active());
    assert_panes(&live, &first_accepted, full);
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Close"));
    assert!(press(Key::Return));
    assert_panes(added, "", wrapped);
    window.diff.accept_button.emit_clicked();
    assert_panes("", removed, wrapped);
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (1 change)")
    );
    window.diff.accept_button.emit_clicked();
    assert_panes(&live, &live, full);
    assert!(window.diff.exit_button.has_css_class("success"));
    assert_eq!(
        window.diff.exit_button.tooltip_text().as_deref(),
        Some("Sync baseline")
    );
    assert!(!window.diff.inspect_button.is_sensitive());

    // Undoing the final accept restores a selection in the full diff.
    assert!(press(Key::Left));
    assert_eq!(window.diff.left_view.wrap_mode(), full);
    assert!(window.diff.inspect_button.is_sensitive());
    assert!(press(Key::Return));
    assert_panes("", removed, wrapped);
    assert_selected_block(window.diff.active_right_view());
    assert!(press(Key::Escape));
    window.diff.accept_all_button.emit_clicked();
    window.diff.exit_button.emit_clicked();
    assert!(!window.diff_active());
    assert_snapshot(&live);
    window.window.destroy();
}

fn partial_confirm_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("partial-confirm.txt");
    std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    let canonical = window.doc.borrow().path().unwrap().to_path_buf();
    window.set_editor_text("A\nb\nC\nd\nE\n");
    window.flags.dirty.set(true);
    window.persist(None).unwrap();
    window.enter_diff();
    let save_as = window.window.lookup_action("save-as").unwrap();
    let rename = window.window.lookup_action("rename").unwrap();
    let goto_line = window.window.lookup_action("goto-line").unwrap();
    assert!(!save_as.is_enabled());
    assert!(!rename.is_enabled());
    assert!(!goto_line.is_enabled());
    window.focus_relative(1);
    window.accept_focused();
    let focus = window.diff_state.borrow().as_ref().unwrap().focus();
    let left = window.diff.left_view.vadjustment().unwrap();
    let right = window.diff.right_view.vadjustment().unwrap();
    let left_scroll = left.value();
    let right_scroll = right.value();

    // Cancelling the save-first prompt must not reset pending accepts.
    window.flags.dirty.set(true);
    window.diff.exit_button.emit_clicked();
    window.window.visible_dialog().unwrap().close();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());
    assert!(window.diff.undo_button.is_sensitive());
    window.flags.dirty.set(false);

    // A failed snapshot write must also preserve the session for retry.
    let snapshot = store.snapshot_path_for_canonical(&canonical);
    let backup = snapshot.with_extension("backup");
    std::fs::rename(&snapshot, &backup).unwrap();
    std::fs::create_dir(&snapshot).unwrap();
    window.diff.exit_button.emit_clicked();
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());
    assert!(window.diff.undo_button.is_sensitive());
    assert_eq!(window.diff.header_texts().1, "Baseline");
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (1 change)")
    );
    window.window.visible_dialog().unwrap().close();
    wait_for(|| window.window.visible_dialog().is_none());
    std::fs::remove_dir(&snapshot).unwrap();
    std::fs::rename(&backup, &snapshot).unwrap();

    window.diff.exit_button.emit_clicked();
    assert!(window.diff_active());
    assert_eq!(window.diff.header_texts().1, "Baseline");
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Close"));
    assert!(!window.diff.exit_button.has_css_class("success"));
    assert!(!window.diff.undo_button.is_sensitive());
    assert_eq!(window.diff_state.borrow().as_ref().unwrap().focus(), focus);
    assert_eq!(left.value(), left_scroll);
    assert_eq!(right.value(), right_scroll);
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some("A\nb\nc\nd\ne\n")
    );
    window.undo_accept();
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .accepted_hunks(),
        0
    );

    // Close preserves the checkpoint without accepting the remaining hunks.
    window.diff.exit_button.emit_clicked();
    assert!(!window.diff_active());
    assert!(save_as.is_enabled());
    assert!(rename.is_enabled());
    assert!(goto_line.is_enabled());
    window.enter_diff();
    window.focus_relative(1);
    window.accept_focused();
    window.diff.exit_button.emit_clicked();
    assert!(window.diff_active());
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some("A\nb\nC\nd\ne\n")
    );
    window.accept_focused();
    assert!(window.diff.exit_button.has_css_class("success"));
    window.diff.exit_button.emit_clicked();
    assert!(!window.diff_active());
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some("A\nb\nC\nd\nE\n")
    );
    window.window.destroy();
}

fn diff_actions_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("doc.md");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();

    let untitled = AppWindow::with_store(app, store.clone());
    assert!(untitled.diff_button.is_sensitive());
    assert_eq!(untitled.diff_button.label().as_deref(), Some(SAVE_AS_LABEL));
    assert!(untitled.diff_button.has_css_class("suggested-action"));
    assert!(!untitled.diff_button.has_css_class("flat"));
    assert!(untitled.diff_button.tooltip_text().is_none());
    untitled.enter_diff();
    assert!(!untitled.diff_active());

    // Saving an untitled document turns the button into Create Baseline.
    untitled.set_editor_text("saved\n");
    untitled
        .persist(Some(&dir.join("saved.md")))
        .expect("saving an untitled document");
    assert_eq!(untitled.diff_button.label().as_deref(), Some(CREATE_LABEL));
    assert!(untitled.diff_button.is_sensitive());
    assert!(!untitled.diff_button.has_css_class("suggested-action"));
    assert!(untitled.diff_button.has_css_class("flat"));
    assert!(
        untitled
            .diff_button
            .tooltip_text()
            .is_some_and(|text| text == "Save current text as a baseline for later comparison")
    );

    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    assert!(window.diff_button.is_sensitive());
    assert_eq!(window.doc.borrow().text(), "one\ntwo\nthree\n");
    assert!(window.editor_view.has_css_class(theme::CONTENT_CLASS));
    assert!(window.diff.left_view.has_css_class(theme::CONTENT_CLASS));
    assert!(window.diff.right_view.has_css_class(theme::CONTENT_CLASS));

    let wrap = window.window.lookup_action("word-wrap").unwrap();
    assert!(wrap.is_enabled());
    assert_eq!(wrap.state().unwrap().get::<bool>(), Some(true));
    assert_eq!(window.editor_view.wrap_mode(), gtk::WrapMode::WordChar);
    wrap.activate(None);
    assert_eq!(wrap.state().unwrap().get::<bool>(), Some(false));
    assert_eq!(window.editor_view.wrap_mode(), gtk::WrapMode::None);
    assert_eq!(untitled.editor_view.wrap_mode(), gtk::WrapMode::WordChar);
    assert_eq!(window.current_text(), "one\ntwo\nthree\n");
    assert!(!window.flags.dirty.get());

    assert_eq!(window.diff_button.label().as_deref(), Some(CREATE_LABEL));
    assert!(!window.diff_button.has_css_class("suggested-action"));
    assert!(window.diff_button.has_css_class("flat"));
    assert!(
        window
            .diff_button
            .tooltip_text()
            .is_some_and(|text| text == "Save current text as a baseline for later comparison")
    );

    // Without a baseline, entering Diff Mode does nothing.
    window.enter_diff();
    assert!(!window.diff_active());

    window.create_baseline();
    let canonical = window.doc.borrow().path().unwrap().to_path_buf();
    assert_eq!(
        window
            .store
            .read_for_canonical(&canonical)
            .unwrap()
            .as_deref(),
        Some("one\ntwo\nthree\n")
    );
    assert!(!window.diff_active());
    assert_eq!(window.diff_button.label().as_deref(), Some(REVIEW_LABEL));
    assert!(!window.diff_button.is_sensitive());
    assert!(!window.diff_button.has_css_class("suggested-action"));
    assert!(
        window
            .diff_button
            .tooltip_text()
            .is_some_and(|text| text == "Compare with baseline")
    );
    window.set_editor_text("one\nTWO changed\nthree\n");
    window.flags.dirty.set(true);
    window.persist(None).unwrap();
    assert!(window.diff_button.is_sensitive());

    window.enter_diff();
    assert!(!window.diff_button.has_css_class("suggested-action"));
    assert!(!window.diff_button.is_sensitive());
    assert!(window.diff_button.tooltip_text().is_none());
    assert!(!wrap.is_enabled());
    assert_eq!(window.diff.left_view.wrap_mode(), gtk::WrapMode::None);
    assert_eq!(window.diff.right_view.wrap_mode(), gtk::WrapMode::None);
    let hunks = window
        .diff_state
        .borrow()
        .as_ref()
        .unwrap()
        .diff()
        .hunks
        .clone();
    assert_eq!(hunks.len(), 1);
    assert_eq!(window.diff.header_texts().1, "Baseline");
    assert!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .focus()
            .is_none()
    );
    assert!(!window.diff.accept_button.is_sensitive());

    // Accepting with nothing focused does nothing.
    window.accept_focused();
    assert_eq!(
        window
            .diff_state
            .borrow()
            .as_ref()
            .unwrap()
            .diff()
            .hunks
            .len(),
        1
    );

    // The first navigation input focuses the first hunk, whichever direction.
    window.focus_relative(-1);
    assert_eq!(
        window.diff_state.borrow().as_ref().unwrap().focus(),
        Some(0)
    );
    assert!(window.diff.accept_button.is_sensitive());

    window.accept_focused();
    {
        let state = window.diff_state.borrow();
        assert!(state.as_ref().unwrap().diff().is_empty());
    }
    assert_eq!(window.diff.header_texts().1, "Baseline");
    // Accepts stay in memory until Confirm.
    assert_eq!(
        window
            .store
            .read_for_canonical(&canonical)
            .unwrap()
            .as_deref(),
        Some("one\ntwo\nthree\n")
    );
    assert_eq!(
        window.diff.exit_button.label().as_deref(),
        Some("Confirm (1 change)")
    );
    assert!(window.diff.exit_button.has_css_class("success"));

    window.undo_accept();
    assert_eq!(
        window
            .store
            .read_for_canonical(&canonical)
            .unwrap()
            .as_deref(),
        Some("one\ntwo\nthree\n")
    );
    assert_eq!(window.diff.header_texts().1, "Baseline");
    assert_eq!(window.diff.exit_button.label().as_deref(), Some("Close"));
    assert!(!window.diff.exit_button.has_css_class("success"));

    // Confirm commits the accepted baseline in one write, then leaves.
    window.focus_relative(1);
    window.accept_focused();
    window.exit_diff();
    assert!(!window.diff_active());
    assert!(!window.diff_button.is_sensitive());
    assert!(
        window
            .diff_button
            .tooltip_text()
            .is_some_and(|text| text == "Compare with baseline")
    );
    assert!(wrap.is_enabled());
    assert_eq!(window.editor_view.wrap_mode(), gtk::WrapMode::None);
    wrap.activate(None);
    assert_eq!(wrap.state().unwrap().get::<bool>(), Some(true));
    assert_eq!(window.editor_view.wrap_mode(), gtk::WrapMode::WordChar);
    assert_eq!(
        window
            .store
            .read_for_canonical(&canonical)
            .unwrap()
            .as_deref(),
        Some("one\nTWO changed\nthree\n")
    );

    // Closing the window confirms before discarding accepted hunks. The
    // snapshot keeps the last confirmed value.
    window.set_editor_text("one\nTWO changed\nTHREE changed\n");
    window.flags.dirty.set(true);
    window.persist(None).unwrap();
    assert!(window.diff_button.is_sensitive());
    window.enter_diff();
    assert!(!window.diff_button.is_sensitive());
    window.focus_relative(1);
    window.accept_focused();
    assert!(window.has_accepted_diff());
    assert!(!window.flags.dirty.get());
    // GTK only handles close requests for realized windows.
    window.window.present();
    assert!(window.window.is_realized());
    let weak = Rc::downgrade(&window);

    window.window.close();
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|dialog| dialog.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(
        dialog.heading().as_deref(),
        Some("Discard accepted changes?")
    );
    assert!(dialog.body().contains("close this window?"));
    find_response_button(dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(window.window.is_visible());
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());

    window.window.close();
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Discard and Close")
        .unwrap()
        .emit_clicked();
    wait_for(|| !window.window.is_visible());
    assert_eq!(
        window
            .store
            .read_for_canonical(&canonical)
            .unwrap()
            .as_deref(),
        Some("one\nTWO changed\nthree\n")
    );

    drop(window);
    assert!(
        weak.upgrade().is_none(),
        "closing must release the AppWindow"
    );
    untitled.window.destroy();
}

fn review_exit_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("review-exit.txt");
    std::fs::write(&path, "one\ntwo\n").unwrap();

    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    let canonical = window.doc.borrow().path().unwrap().to_path_buf();
    window.set_editor_text("one\nTWO\n");
    window.flags.dirty.set(true);
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    window.accept_focused();
    assert!(window.has_accepted_diff());
    window.window.present();

    let controllers = window.window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-diff-keys"))
        .unwrap();
    let press = |key: gtk::gdk::Key| {
        keys.emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0u32, &gtk::gdk::ModifierType::empty()],
        )
    };

    // Left still undoes without leaving Diff Mode.
    assert!(press(gtk::gdk::Key::Left));
    assert!(window.diff_active());
    assert!(!window.has_accepted_diff());
    window.focus_relative(1);
    window.accept_focused();
    assert!(window.has_accepted_diff());

    // Esc asks before discarding accepted changes.
    assert!(press(gtk::gdk::Key::Escape));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    assert_eq!(
        dialog.heading().as_deref(),
        Some("Discard accepted changes?")
    );
    assert!(
        dialog
            .body()
            .contains("Accepted changes aren't confirmed yet. Discard them and return to editor?")
    );
    assert!(find_response_button(dialog.upcast_ref(), "Discard and End Review").is_some());
    find_response_button(dialog.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(window.diff_active());
    assert!(window.has_accepted_diff());

    assert!(press(gtk::gdk::Key::Escape));
    wait_for(|| {
        window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    });
    let dialog = window
        .window
        .visible_dialog()
        .unwrap()
        .downcast::<adw::AlertDialog>()
        .unwrap();
    find_response_button(dialog.upcast_ref(), "Discard and End Review")
        .unwrap()
        .emit_clicked();
    wait_for(|| window.window.visible_dialog().is_none());
    assert!(!window.diff_active());
    assert_eq!(
        store.read_for_canonical(&canonical).unwrap().as_deref(),
        Some("one\ntwo\n")
    );

    // With nothing accepted, Esc leaves without a prompt.
    window.enter_diff();
    assert!(window.diff_active());
    assert!(press(gtk::gdk::Key::Escape));
    assert!(!window.diff_active());
    assert!(window.window.visible_dialog().is_none());

    // Esc in inspection still returns to the full diff first.
    window.enter_diff();
    window.focus_relative(1);
    assert!(press(gtk::gdk::Key::Return));
    assert!(window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert!(press(gtk::gdk::Key::Escape));
    assert!(
        !window
            .window
            .visible_dialog()
            .is_some_and(|d| d.is_mapped())
    );
    assert!(window.diff_active());
    assert!(!window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert!(press(gtk::gdk::Key::Escape));
    assert!(!window.diff_active());
    window.window.destroy();
}

fn font_appearance_scenario(app: &adw::Application, store: &SnapshotStore) {
    use crate::core::appearance::Appearance;

    let previous_font = theme::appearance();
    let previous_theme = theme::current();
    let first = AppWindow::with_store(app, store.clone());
    first.set_editor_text("font changes preserve this draft");
    first.flags.dirty.set(true);
    let second = AppWindow::with_store(app, store.clone());
    let chosen = Appearance::new("monospace", 20).unwrap();
    theme::apply_appearance(chosen.clone());
    theme::apply(theme::by_id("light").unwrap());
    assert_eq!(theme::appearance(), chosen);
    assert_eq!(first.current_text(), "font changes preserve this draft");
    assert!(first.flags.dirty.get());
    assert_eq!(second.current_text(), "");
    assert!(!second.flags.dirty.get());
    let third = AppWindow::with_store(app, store.clone());
    for window in [&first, &second, &third] {
        for view in [
            &window.editor_view,
            &window.diff.left_view,
            &window.diff.right_view,
        ] {
            assert!(view.has_css_class(theme::CONTENT_CLASS));
        }
    }
    theme::apply(previous_theme);
    theme::apply_appearance(previous_font);
    first.flags.dirty.set(false);
    for window in [&first, &second, &third] {
        window.window.destroy();
    }
}

fn restore_appearance_scenario(app: &adw::Application, store: &SnapshotStore) {
    use crate::core::appearance::Appearance;

    fn widgets(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut result = vec![root.clone()];
        let mut index = 0;
        while index < result.len() {
            let mut child = result[index].first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                result.push(widget);
            }
            index += 1;
        }
        result
    }

    let previous_theme = theme::current();
    let previous_font = theme::appearance();
    theme::apply(theme::by_id("cola-n-cream").unwrap());
    theme::apply_appearance(Appearance::new("monospace", 20).unwrap());
    let catalog_ids: Vec<_> = theme::catalog().iter().map(|t| t.id.clone()).collect();
    let first = AppWindow::with_store(app, store.clone());
    first
        .editor_buffer
        .insert_at_cursor("keep this draft and undo history");
    let second = AppWindow::with_store(app, store.clone());
    let form = |window: &AppWindow| {
        window
            .window
            .lookup_action("manage-app")
            .unwrap()
            .activate(None);
        let dialog = window.window.visible_dialog().unwrap();
        let stack = app_data_stack(&dialog);
        stack.set_visible_child_name("appearance");
        let page = stack.child_by_name("appearance").unwrap();
        let descendants = widgets(&page);
        let label = |text: &str| {
            descendants.iter().find_map(|widget| {
                let label = widget.clone().downcast::<gtk::Label>().ok()?;
                (label.label().as_str() == text).then_some(label)
            })
        };
        let explanation =
            label("Appearance changes affect every open window and are remembered for next time.")
                .expect("appearance explanation");
        assert!(explanation.has_css_class("dim-label"));
        let policy = label(
            "User themes are loaded at startup. Restart notediff after adding or editing a theme.",
        )
        .expect("user-theme policy");
        assert!(policy.has_css_class("dim-label"));
        let family = descendants
            .iter()
            .find_map(|w| w.clone().downcast::<gtk::Entry>().ok())
            .unwrap();
        let size = descendants
            .iter()
            .find_map(|w| w.clone().downcast::<gtk::SpinButton>().ok())
            .unwrap();
        let restore = descendants
            .iter()
            .filter_map(|w| w.clone().downcast::<gtk::Button>().ok())
            .find(|b| b.label().as_deref() == Some("Restore Defaults"))
            .unwrap();
        (dialog, family, size, restore, page)
    };
    let (dialog, family, size, restore, _) = form(&first);
    let (other_dialog, other_family, other_size, _, other_page) = form(&second);
    family.set_text("unapplied draft");
    // Uncommitted text must reset even when the adjustment already equals 12.
    size.set_value(12.0);
    size.set_text("36");
    other_family.set_text("another independent draft");
    other_size.set_value(18.0);
    assert!(restore.is_sensitive());
    assert!(!restore.has_css_class("destructive-action"));
    restore.emit_clicked();
    assert_eq!(theme::current().id, "dark");
    assert_eq!(theme::appearance(), Appearance::default());
    assert_eq!(family.text(), Appearance::default().font_family());
    assert_eq!(size.value_as_int(), 12);
    assert_eq!(size.text(), "12");
    assert_eq!(other_family.text(), "another independent draft");
    assert_eq!(other_size.value_as_int(), 18);
    let dark_row = widgets(&other_page)
        .into_iter()
        .filter_map(|w| w.downcast::<adw::ActionRow>().ok())
        .find(|row| row.title() == "notediff Dark")
        .unwrap();
    let dark_check = widgets(dark_row.upcast_ref())
        .into_iter()
        .find_map(|w| w.downcast::<gtk::CheckButton>().ok())
        .unwrap();
    assert!(dark_check.is_active());
    assert_eq!(first.current_text(), "keep this draft and undo history");
    assert!(first.flags.dirty.get());
    assert!(first.editor_buffer.can_undo());
    assert_eq!(
        theme::catalog()
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        catalog_ids
    );
    dialog.force_close();
    other_dialog.force_close();
    // A fresh dialog starts from applied defaults, not either discarded draft.
    let (reopened, family, size, _, _) = form(&first);
    assert_eq!(family.text(), Appearance::default().font_family());
    assert_eq!(size.value_as_int(), 12);
    reopened.force_close();
    first.flags.dirty.set(false);
    first.window.destroy();
    second.window.destroy();
    theme::apply(previous_theme);
    theme::apply_appearance(previous_font);
}

fn blank_marker_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let path = dir.join("blank-marker.txt");
    std::fs::write(&path, "alpha\nbeta\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("alpha\n\nbeta\n");
    window.persist(None).unwrap();
    window.enter_diff();

    let has = |tags: &[String], name: &str| tags.iter().any(|tag| tag == name);
    let unfocused = window.diff.full_marker_tags(false);
    assert!(
        !unfocused.is_empty(),
        "the inserted blank line needs a marker"
    );
    for (_, tags) in &unfocused {
        assert!(
            has(tags, "notediff-word"),
            "unfocused marker must use the strong amber layer: {tags:?}"
        );
        assert!(!has(tags, "notediff-word-focus"));
    }

    window.focus_relative(1);
    let focused = window.diff.full_marker_tags(false);
    assert!(
        focused
            .iter()
            .any(|(_, tags)| has(tags, "notediff-word-focus")),
        "the focused marker must use the strong red layer: {focused:?}"
    );

    window.exit_diff();
    window.window.destroy();
}

fn full_boundary_marker_after_accept_scenario(
    app: &adw::Application,
    dir: &Path,
    store: &SnapshotStore,
) {
    // Accepting the first insertion moves the marker to the later insertion's
    // boundary; the row must follow the rewritten pane, not the old layout.
    let path = dir.join("full-boundary-accept.txt");
    std::fs::write(&path, "alpha\nbeta\ngamma\ndelta\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("top\nalpha\nbeta\ngamma\ndelta\ntail\n");
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();

    let expected = |offset: i32| {
        let view = window.diff.active_right_view();
        let buffer = view.buffer();
        let iter = buffer.iter_at_offset(offset);
        let (strong, _) = view.cursor_locations(Some(&iter));
        (strong.x(), strong.y(), strong.height())
    };

    window.focus_relative(1);
    wait_for(|| window.diff.panes_ready());
    assert_eq!(window.diff.full_boundary_marker(true), (true, Some(0)));
    assert_eq!(
        window.diff.full_boundary_marker_geometry(true),
        Some(expected(0))
    );

    window.accept_focused();
    wait_for(|| window.diff.panes_ready());
    assert_eq!(
        window.diff.full_boundary_marker(true),
        (true, Some(27)),
        "the marker must move to the remaining insertion"
    );
    assert_eq!(
        window.diff.full_boundary_marker_geometry(true),
        Some(expected(27)),
        "the marker row must match the rewritten pane"
    );

    window.undo_accept();
    wait_for(|| window.diff.panes_ready());
    assert_eq!(window.diff.full_boundary_marker(true), (true, Some(0)));
    assert_eq!(
        window.diff.full_boundary_marker_geometry(true),
        Some(expected(0)),
        "undo must restore the marker row"
    );
    window.window.destroy();
}

fn full_boundary_marker_scroll_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    // The marker is an overlay in buffer coordinates. Scrolling the pane must
    // move its allocation by the same amount, on both axes.
    let baseline: String = (0..60)
        .map(|index| format!("line {index:02} {}\n", "x".repeat(300)))
        .collect();
    let mut live = String::new();
    for index in 0..60 {
        if index == 12 {
            live.push_str("inserted\n");
        }
        live.push_str(&format!("line {index:02} {}\n", "x".repeat(300)));
    }
    let path = dir.join("full-boundary-scroll.txt");
    std::fs::write(&path, &baseline).unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text(&live);
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();
    window.focus_relative(1);
    wait_for(|| window.diff.panes_ready());
    let expected_offset = window
        .diff
        .active_right_view()
        .buffer()
        .iter_at_line(12)
        .unwrap()
        .offset();
    assert_eq!(
        window.diff.full_boundary_marker(true),
        (true, Some(expected_offset))
    );

    let scroller = window.diff.full_scrollers()[1];
    let vertical = scroller.vadjustment();
    let before_value = vertical.value();
    let before = window.diff.full_boundary_marker_allocation(true).unwrap();
    vertical.set_value(before_value + 42.0);
    wait_for(|| window.diff.full_boundary_marker_allocation(true) != Some(before));
    let after = window.diff.full_boundary_marker_allocation(true).unwrap();
    let delta = vertical.value() - before_value;
    assert!(
        (f64::from(before.1 - after.1) - delta).abs() <= 1.0,
        "vertical scroll {delta} moved the marker by {}",
        before.1 - after.1
    );
    assert_eq!(
        before.0, after.0,
        "vertical scroll must not move the marker x"
    );

    vertical.set_value(before_value);
    wait_for(|| window.diff.full_boundary_marker_allocation(true) == Some(before));
    let horizontal = scroller.hadjustment();
    let before_value = horizontal.value();
    let before = window.diff.full_boundary_marker_allocation(true).unwrap();
    horizontal.set_value(before_value + 40.0);
    wait_for(|| window.diff.full_boundary_marker_allocation(true) != Some(before));
    let after = window.diff.full_boundary_marker_allocation(true).unwrap();
    let delta = horizontal.value() - before_value;
    assert!(
        (f64::from(before.0 - after.0) - delta).abs() <= 1.0,
        "horizontal scroll {delta} moved the marker by {}",
        before.0 - after.0
    );
    assert_eq!(
        before.1, after.1,
        "horizontal scroll must not move the marker y"
    );

    window.window.destroy();
}

fn focused_line_number_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    // Every display row of a focused hunk uses the strong line-number colour.
    let path = dir.join("focused-line-numbers.txt");
    std::fs::write(&path, "a\nb\nc\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("a\nB\nC\n");
    window.persist(None).unwrap();
    window.enter_diff();
    assert_eq!(window.diff.full_line_number_strong(false), None);
    assert_eq!(window.diff.full_line_number_strong(true), None);
    window.focus_relative(1);
    assert_eq!(window.diff.full_line_number_strong(false), Some((1, 3)));
    assert_eq!(window.diff.full_line_number_strong(true), Some((1, 3)));

    window.accept_focused();
    assert_eq!(window.diff.full_line_number_strong(false), None);
    assert_eq!(window.diff.full_line_number_strong(true), None);
    window.window.destroy();

    // An empty hunk side emphasises its single boundary row on both panes.
    let path = dir.join("focused-line-numbers-empty.txt");
    std::fs::write(&path, "a\nb\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("a\nb\nc\n");
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    assert_eq!(window.diff.full_line_number_strong(false), Some((2, 3)));
    assert_eq!(window.diff.full_line_number_strong(true), Some((2, 3)));
    window.window.destroy();
}

fn trailing_blank_marker_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    let has = |tags: &[String], name: &str| tags.iter().any(|tag| tag == name);

    // Adding a final newline creates a display-only blank line after the last
    // real line. It belongs to the hunk when the other side lacks that newline.
    let path = dir.join("trailing-blank-add.txt");
    std::fs::write(&path, "sentence").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("\nsentence\n");
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();
    window.focus_relative(1);
    wait_for(|| window.diff.panes_ready());

    let unfocused = window.diff.full_marker_tags(false);
    assert_eq!(
        unfocused.len(),
        2,
        "the leading and trailing blank lines both need markers: {unfocused:?}"
    );
    assert!(
        unfocused.iter().all(|(_, tags)| has(tags, "notediff-word")),
        "the trailing marker must use the normal word layer: {unfocused:?}"
    );
    assert!(window.diff.full_marker_tags(true).is_empty());

    let focused = window.diff.full_marker_tags(false);
    assert!(
        focused
            .iter()
            .all(|(_, tags)| has(tags, "notediff-word-focus")),
        "the focused trailing marker must use the strong red layer: {focused:?}"
    );

    window.diff.inspect_button.emit_clicked();
    assert!(window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    assert_eq!(
        window.diff.inspection_marker_tags(false).len(),
        2,
        "inspection must mark both blank lines of the final hunk"
    );
    assert!(window.diff.inspection_marker_tags(true).is_empty());
    window.window.destroy();

    // Removing a final newline marks the baseline pane instead.
    let path = dir.join("trailing-blank-remove.txt");
    std::fs::write(&path, "\nsentence\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("sentence");
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    assert_eq!(window.diff.full_marker_tags(true).len(), 2);
    assert!(window.diff.full_marker_tags(false).is_empty());
    window.window.destroy();

    // No trailing marker when both sides already end with a newline.
    let path = dir.join("trailing-blank-stable.txt");
    std::fs::write(&path, "sentence\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("\nsentence\n");
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    let markers = window.diff.full_marker_tags(false);
    assert_eq!(
        markers.len(),
        1,
        "only the inserted leading blank line is new: {markers:?}"
    );
    window.window.destroy();

    // One blank line above and two below: the last real line is itself blank,
    // and the display-only row after the final newline is the second blank
    // line below.
    let path = dir.join("trailing-blank-two-lines.txt");
    std::fs::write(&path, "sentence").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("\nsentence\n\n");
    window.persist(None).unwrap();
    window.enter_diff();
    window.focus_relative(1);
    let markers = window.diff.full_marker_tags(false);
    assert_eq!(
        markers.len(),
        3,
        "one leading and two trailing blank lines need markers: {markers:?}"
    );
    window.diff.inspect_button.emit_clicked();
    let markers = window.diff.inspection_marker_tags(false);
    assert_eq!(
        markers.len(),
        3,
        "inspection must mark every blank line of the final hunk"
    );
    assert!(
        markers
            .iter()
            .all(|(_, tags)| has(tags, "notediff-inline-selected")),
        "the selected atomic change must include the trailing marker: {markers:?}"
    );
    window.window.destroy();
}

fn full_boundary_marker_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    // A selected hunk with an empty side marks the exact insertion/deletion
    // boundary in that side's full pane, like inspection's caret.
    let path = dir.join("full-boundary-insert.txt");
    std::fs::write(&path, "alpha\nbeta\ngamma\ndelta\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("alpha\nBETA\ngamma\ndelta\nepsilon\n");
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();

    // The replacement hunk has content on both sides: no boundary marker.
    window.focus_relative(1);
    assert_eq!(window.diff.full_boundary_marker(false), (false, None));
    assert_eq!(window.diff.full_boundary_marker(true), (false, None));

    // The insertion leaves the baseline side empty; mark its line boundary
    // (the empty line after delta, at character offset 23).
    window.focus_relative(1);
    assert_eq!(window.diff.full_boundary_marker(false), (false, None));
    assert_eq!(
        window.diff.full_boundary_marker(true),
        (true, Some(23)),
        "the insertion boundary must be marked on the baseline side"
    );
    gtk::test_widget_wait_for_draw(&window.window);
    assert!(
        window.diff.full_boundary_marker_ready(true),
        "the boundary marker must be mapped and sized"
    );
    assert_eq!(
        window.diff.full_boundary_marker_width(true),
        3,
        "the boundary marker must be three pixels wide"
    );

    // Moving back to the replacement clears it again.
    window.focus_relative(-1);
    assert_eq!(window.diff.full_boundary_marker(true), (false, None));
    window.window.destroy();

    // A pure deletion marks the live pane instead.
    let path = dir.join("full-boundary-delete.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("one\nthree\n");
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();
    window.focus_relative(1);
    assert_eq!(
        window.diff.full_boundary_marker(false),
        (true, Some(4)),
        "the deletion boundary must be marked on the live side"
    );
    assert_eq!(window.diff.full_boundary_marker(true), (false, None));
    gtk::test_widget_wait_for_draw(&window.window);
    assert!(window.diff.full_boundary_marker_ready(false));
    assert_eq!(window.diff.full_boundary_marker_width(false), 3);
    // Returning from inspection restores the marker in the retained full view.
    window.diff.inspect_button.emit_clicked();
    assert!(window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    window.diff.exit_button.emit_clicked();
    assert!(!window.diff_state.borrow().as_ref().unwrap().is_inspecting());
    gtk::test_widget_wait_for_draw(&window.window);
    assert_eq!(window.diff.full_boundary_marker(false), (true, Some(4)));
    assert!(window.diff.full_boundary_marker_ready(false));
    window.window.destroy();

    // An empty baseline still gets a boundary marker at offset zero.
    let path = dir.join("full-boundary-empty.txt");
    std::fs::write(&path, "").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("alpha\n");
    window.persist(None).unwrap();
    window.window.present();
    window.enter_diff();
    window.focus_relative(1);
    assert_eq!(window.diff.full_boundary_marker(false), (false, None));
    assert_eq!(
        window.diff.full_boundary_marker(true),
        (true, Some(0)),
        "an insertion into an empty baseline must mark offset zero"
    );
    window.window.destroy();
}

fn caret_navigation_scenario(app: &adw::Application, dir: &Path, store: &SnapshotStore) {
    use gtk::gdk::{Key, ModifierType};

    // Five lines: two blank lines, a character, two blank lines. The live
    // edit puts a character on the first and fifth lines, so both baseline
    // excerpts are the same one-line "\n" with the caret at offset 0. This
    // used to leave the right caret invisible after navigating hunks.
    let path = dir.join("caret-navigation.txt");
    std::fs::write(&path, "\n\nx\n\n\n").unwrap();
    let window = AppWindow::with_store(app, store.clone());
    AppWindow::open_path(&window, &path);
    window.create_baseline();
    window.set_editor_text("a\n\nx\n\nb\n");
    window.persist(None).unwrap();
    window.window.present();
    let controllers = window.window.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|controller| controller.name().as_deref() == Some("notediff-diff-keys"))
        .unwrap();
    let press =
        |key: Key| keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &ModifierType::empty()]);

    // The diff engine resolves the repeated blank lines as three hunks:
    // replace (right caret), delete (left caret), insert (right caret). The
    // right-hand caret used to go missing after navigating to the later ones.
    let assert_carets = |label: &str, right: Option<i32>, left: Option<i32>| {
        gtk::test_widget_wait_for_draw(&window.window);
        assert_eq!(
            window.diff.inspection_caret(true),
            (right.is_some(), right),
            "{label}: right caret"
        );
        assert_eq!(
            window.diff.inspection_caret(false),
            (left.is_some(), left),
            "{label}: left caret"
        );
        for (side, offset) in [("right", right), ("left", left)] {
            if offset.is_some() {
                let is_right = side == "right";
                assert!(
                    window.diff.inspection_caret_ready(is_right),
                    "{label}: {side} caret was not mapped and sized"
                );
                assert_eq!(
                    window.diff.inspection_caret_width(is_right),
                    3,
                    "{label}: {side} caret must be three pixels wide"
                );
            }
        }
    };

    window.enter_diff();
    assert!(press(Key::Down));
    assert!(press(Key::Return));
    assert_carets("first hunk", Some(0), None);

    assert!(press(Key::Down));
    assert_carets("next diff via key", None, Some(0));
    assert!(press(Key::Down));
    assert_carets("next diff via key again", Some(0), None);
    assert!(press(Key::Down));
    assert_carets("last change", Some(0), None);
    assert!(press(Key::Up));
    assert_carets("previous diff via key", None, Some(0));

    window.diff.previous_button.emit_clicked();
    assert_carets("previous diff via button", Some(0), None);
    window.diff.next_button.emit_clicked();
    assert_carets("next diff via button", None, Some(0));
    window.diff.next_button.emit_clicked();
    assert_carets("next diff via button again", Some(0), None);

    window.window.destroy();
}
