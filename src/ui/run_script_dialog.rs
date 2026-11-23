//! Run Script dialog: discovers user scripts, builds a dynamic parameter form
//! from each script's metadata comment block, runs the script asynchronously
//! (optionally inside a bubblewrap sandbox), and delivers stdout to a new window.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::core::script_state::{
    self, SavedFieldValue, ScriptDialogState, ScriptParams, ScriptStateStore,
};
use crate::core::scripts::{
    self, CancelHandle, DiscoveredScript, FieldSpec, FieldType, FieldValue, InvocationContext,
    SandboxDefault, SandboxMode, ScriptMetadata, ScriptOutput, ScriptRunError,
};
use crate::ui::dialogs::{self, Response};
use crate::ui::display_path;
use crate::ui::toast::{self, ToastKind};

const SANDBOX_RO: u32 = 0;
const SANDBOX_WRITABLE: u32 = 1;
const SANDBOX_NONE: u32 = 2;

type OutputCallback = Box<dyn Fn(String, ScriptOutput)>;
type OpenDocsCallback = Box<dyn Fn()>;

/// Present the Run Script dialog over `parent`.
pub fn present(
    parent: &adw::ApplicationWindow,
    current_file: Option<PathBuf>,
    on_open_docs: impl Fn() + 'static,
    on_output: impl Fn(String, ScriptOutput) + 'static,
) {
    let state = Rc::new(DialogState::new(
        parent,
        current_file,
        ScriptStateStore::default(),
        Box::new(on_open_docs),
        Box::new(on_output),
    ));
    state.wire_signals();
    state.load_initial_scripts();
    state.dialog.present(Some(parent));
}

#[cfg(test)]
pub(super) fn close_for_smoke(parent: &adw::ApplicationWindow) -> impl Fn() -> bool + use<> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let temp_root =
        std::env::temp_dir().join(format!("notediff-run-script-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_root);
    let scripts_dir = temp_root.join("scripts");
    fs::create_dir_all(&scripts_dir).unwrap();
    let state_path = temp_root.join("script-state");

    let script_a = scripts_dir.join("alpha.sh");
    fs::write(
        &script_a,
        "#!/bin/sh\n# --- notediff-script ---\n# name = Alpha\n# [field.query]\n# type = text\n# default = init\n# [field.flag]\n# type = bool\n# default = false\n# --- end-notediff-script ---\necho ok\n",
    )
    .unwrap();
    fs::set_permissions(&script_a, fs::Permissions::from_mode(0o755)).unwrap();

    let script_b = scripts_dir.join("beta.sh");
    fs::write(
        &script_b,
        "#!/bin/sh\n# --- notediff-script ---\n# name = Beta\n# [field.count]\n# type = int\n# default = 5\n# min = 1\n# max = 20\n# --- end-notediff-script ---\necho ok\n",
    )
    .unwrap();
    fs::set_permissions(&script_b, fs::Permissions::from_mode(0o755)).unwrap();

    // First session: edit fields on Alpha, switch to Beta, edit Beta, then close.
    let state1 = Rc::new(DialogState::new(
        parent,
        None,
        ScriptStateStore::at(state_path.clone()),
        Box::new(|| {}),
        Box::new(|_, _| {}),
    ));
    state1.wire_signals();
    state1.load_scripts_from(&scripts_dir);
    assert_eq!(state1.selected_index.get(), Some(0));
    assert!(!state1.has_non_default_state());
    {
        let controls = state1.controls.borrow();
        if let FieldWidget::Text(ref entry) = controls[0].widget {
            entry.set_text("saved-query");
        }
        if let FieldWidget::Bool(ref check) = controls[1].widget {
            check.set_active(true);
        }
    }
    assert!(state1.has_non_default_state());

    // Switch to Beta and modify its int field.
    if let Some(row) = state1.sidebar_list.row_at_index(1) {
        state1.sidebar_list.select_row(Some(&row));
    }
    assert_eq!(state1.selected_index.get(), Some(1));
    {
        let controls = state1.controls.borrow();
        if let FieldWidget::Int(ref spin) = controls[0].widget {
            spin.set_value(12.0);
        }
    }
    state1.capture_current_script_state();
    state1.flush_state();
    drop(state1);

    // Second session: Beta should be selected with count=12, and switching to Alpha restores its edits.
    let state = Rc::new(DialogState::new(
        parent,
        None,
        ScriptStateStore::at(state_path),
        Box::new(|| {}),
        Box::new(|_, _| {}),
    ));
    let weak = Rc::downgrade(&state);
    state.wire_signals();
    state.load_scripts_from(&scripts_dir);
    assert_eq!(state.selected_index.get(), Some(1));
    assert_eq!(state.controls.borrow()[0].value(), FieldValue::Int(12));
    assert!(state.has_non_default_state());

    // Resetting Beta restores default (5).
    state.reset_current_script();
    assert_eq!(state.controls.borrow()[0].value(), FieldValue::Int(5));
    assert!(!state.has_non_default_state());

    // Switching to Alpha restores saved-query and true.
    if let Some(row) = state.sidebar_list.row_at_index(0) {
        state.sidebar_list.select_row(Some(&row));
    }
    assert_eq!(
        state.controls.borrow()[0].value(),
        FieldValue::Text("saved-query".into())
    );
    assert_eq!(state.controls.borrow()[1].value(), FieldValue::Bool(true));

    state.dialog.present(Some(parent));
    state.dialog.close();
    let _ = fs::remove_dir_all(&temp_root);
    move || weak.upgrade().is_none()
}

enum FieldWidget {
    Text(gtk::Entry),
    Multiline(gtk::TextBuffer),
    Int(gtk::SpinButton),
    Bool(gtk::CheckButton),
    Choice {
        dropdown: gtk::DropDown,
        options: Vec<String>,
    },
    Slider {
        scale: gtk::Scale,
        options: Vec<String>,
    },
    PathEntry(gtk::Entry),
    Paths(Rc<RefCell<Vec<PathBuf>>>),
    List(gtk::TextBuffer),
}

struct FieldControl {
    spec: FieldSpec,
    widget: FieldWidget,
}

impl FieldControl {
    fn value(&self) -> FieldValue {
        match &self.widget {
            FieldWidget::Text(entry) | FieldWidget::PathEntry(entry) => {
                FieldValue::Text(entry.text().as_str().trim().to_string())
            }
            FieldWidget::Multiline(buffer) => {
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                FieldValue::Text(text.as_str().trim().to_string())
            }
            FieldWidget::Int(spin) => FieldValue::Int(spin.value_as_int() as i64),
            FieldWidget::Bool(check) => FieldValue::Bool(check.is_active()),
            FieldWidget::Choice { dropdown, options } => {
                let idx = dropdown.selected() as usize;
                let chosen = options.get(idx).cloned().unwrap_or_default();
                FieldValue::Text(chosen)
            }
            FieldWidget::Slider { scale, options } => {
                let max_idx = options.len().saturating_sub(1);
                let idx = (scale.value().round().max(0.0) as usize).min(max_idx);
                let chosen = options.get(idx).cloned().unwrap_or_default();
                FieldValue::Text(chosen)
            }
            FieldWidget::Paths(paths) => FieldValue::List(
                paths
                    .borrow()
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            ),
            FieldWidget::List(buffer) => {
                let raw = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                FieldValue::List(scripts::parse_list_items(raw.as_str()))
            }
        }
    }

    fn is_satisfied(&self) -> bool {
        !self.spec.required || !self.value().is_empty()
    }

    fn raw_saved_value(&self) -> SavedFieldValue {
        match &self.widget {
            FieldWidget::Text(entry) | FieldWidget::PathEntry(entry) => {
                SavedFieldValue::Text(entry.text().to_string())
            }
            FieldWidget::Multiline(buffer) | FieldWidget::List(buffer) => {
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                SavedFieldValue::Text(text.to_string())
            }
            FieldWidget::Int(spin) => {
                SavedFieldValue::Text((spin.value_as_int() as i64).to_string())
            }
            FieldWidget::Bool(check) => {
                SavedFieldValue::Text(if check.is_active() { "true" } else { "false" }.to_string())
            }
            FieldWidget::Choice { dropdown, options } => {
                let idx = dropdown.selected() as usize;
                SavedFieldValue::Text(options.get(idx).cloned().unwrap_or_default())
            }
            FieldWidget::Slider { scale, options } => {
                let max_idx = options.len().saturating_sub(1);
                let idx = (scale.value().round().max(0.0) as usize).min(max_idx);
                SavedFieldValue::Text(options.get(idx).cloned().unwrap_or_default())
            }
            FieldWidget::Paths(paths) => SavedFieldValue::List(
                paths
                    .borrow()
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
            ),
        }
    }

    fn is_default(&self) -> bool {
        match &self.widget {
            FieldWidget::Text(entry) | FieldWidget::PathEntry(entry) => {
                entry.text().as_str() == self.spec.default.as_deref().unwrap_or("")
            }
            FieldWidget::Multiline(buffer) | FieldWidget::List(buffer) => {
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                text.as_str() == self.spec.default.as_deref().unwrap_or("")
            }
            FieldWidget::Int(spin) => (spin.value_as_int() as i64) == default_int_value(&self.spec),
            FieldWidget::Bool(check) => check.is_active() == default_bool_value(&self.spec),
            FieldWidget::Choice { dropdown, options } => {
                dropdown.selected() as usize == default_option_index(&self.spec, options)
            }
            FieldWidget::Slider { scale, options } => {
                let max_idx = options.len().saturating_sub(1);
                let idx = (scale.value().round().max(0.0) as usize).min(max_idx);
                idx == default_option_index(&self.spec, options)
            }
            FieldWidget::Paths(paths) => paths.borrow().is_empty(),
        }
    }
}

fn default_int_value(spec: &FieldSpec) -> i64 {
    let lo = spec.min.unwrap_or(i32::MIN as i64);
    let hi = spec.max.unwrap_or(i32::MAX as i64);
    spec.default
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
        .clamp(lo, hi)
}

fn default_bool_value(spec: &FieldSpec) -> bool {
    matches!(spec.default.as_deref(), Some("true" | "1"))
}

fn default_option_index(spec: &FieldSpec, options: &[String]) -> usize {
    let max_idx = options.len().saturating_sub(1);
    spec.default
        .as_deref()
        .and_then(|def| options.iter().position(|o| o == def))
        .unwrap_or(0)
        .min(max_idx)
}

struct DialogState {
    parent: adw::ApplicationWindow,
    dialog: adw::Dialog,
    toasts: adw::ToastOverlay,
    browse_button: gtk::Button,
    info_button: gtk::Button,
    run_button: gtk::Button,
    sidebar_list: gtk::ListBox,
    form_box: gtk::Box,
    sandbox_dropdown: gtk::DropDown,
    writable_dir_button: gtk::Button,
    running_box: gtk::Box,
    spinner: gtk::Spinner,
    current_file: Option<PathBuf>,
    current_dir: Option<PathBuf>,
    writable_dir_override: RefCell<Option<PathBuf>>,
    bwrap_ok: bool,
    scripts: RefCell<Vec<DiscoveredScript>>,
    selected_index: Cell<Option<usize>>,
    controls: RefCell<Vec<FieldControl>>,
    reset_button: RefCell<Option<gtk::Button>>,
    state_store: ScriptStateStore,
    saved_state: RefCell<ScriptDialogState>,
    rebuilding_sidebar: Cell<bool>,
    suppress_state_events: Cell<bool>,
    running_cancel: RefCell<Option<CancelHandle>>,
    on_open_docs: OpenDocsCallback,
    on_output: OutputCallback,
}

impl DialogState {
    fn new(
        parent: &adw::ApplicationWindow,
        current_file: Option<PathBuf>,
        state_store: ScriptStateStore,
        on_open_docs: OpenDocsCallback,
        on_output: OutputCallback,
    ) -> Self {
        let current_dir = current_file
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let default_writable = current_dir.clone().unwrap_or_else(glib::home_dir);
        let bwrap_ok = scripts::bwrap_available();
        let saved_state = state_store.load();

        let dialog = adw::Dialog::builder()
            .title("Run Script")
            .content_width(820)
            .content_height(620)
            .build();

        let browse_button = gtk::Button::with_label("Browse…");
        let title_label = gtk::Label::new(Some("Run Script"));
        title_label.add_css_class("title");
        let info_button = gtk::Button::with_label("ℹ️");
        info_button.add_css_class("flat");
        info_button.add_css_class("circular");
        info_button.set_valign(gtk::Align::Center);
        info_button.set_tooltip_text(Some("Click to open the script authoring guide"));
        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        title_box.set_valign(gtk::Align::Center);
        title_box.append(&title_label);
        title_box.append(&info_button);

        let run_button = gtk::Button::with_label("Run");
        run_button.add_css_class("suggested-action");
        run_button.set_sensitive(false);

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title_box));
        header.pack_start(&browse_button);

        // Left sidebar: discovered scripts list.
        let sidebar_list = gtk::ListBox::new();
        sidebar_list.set_selection_mode(gtk::SelectionMode::Single);
        sidebar_list.add_css_class("navigation-sidebar");
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .width_request(230)
            .child(&sidebar_list)
            .build();

        // Right pane: dynamic form.
        let form_box = gtk::Box::new(gtk::Orientation::Vertical, 14);
        form_box.set_margin_top(16);
        form_box.set_margin_bottom(16);
        form_box.set_margin_start(18);
        form_box.set_margin_end(18);
        form_box.set_hexpand(true);
        let form_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .child(&form_box)
            .build();

        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.set_vexpand(true);
        split.append(&sidebar_scroll);
        split.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        split.append(&form_scroll);

        // Footer bar: running indicator on left; safety controls + Run right-aligned.
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.add_css_class("script-footer");
        footer.set_margin_top(8);
        footer.set_margin_bottom(8);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let spinner = gtk::Spinner::new();
        let running_label = gtk::Label::new(Some("Running…"));
        running_label.add_css_class("dim-label");
        let running_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        running_box.append(&spinner);
        running_box.append(&running_label);
        running_box.set_visible(false);
        footer.append(&running_box);

        if !bwrap_ok {
            let hint = gtk::Label::new(Some("Bubblewrap sandboxing is unavailable"));
            hint.add_css_class("caption");
            hint.add_css_class("dim-label");
            footer.append(&hint);
        }

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let sandbox_label = gtk::Label::new(Some("Safety Setting:"));
        sandbox_label.add_css_class("dim-label");

        let sandbox_dropdown = gtk::DropDown::from_strings(&[
            "Read-Only Sandbox",
            "Allow Writing to Folder…",
            "No Sandbox",
        ]);
        if !bwrap_ok {
            sandbox_dropdown.set_selected(SANDBOX_NONE);
            sandbox_dropdown.set_sensitive(false);
        }

        let home = glib::home_dir();
        let writable_dir_button = gtk::Button::with_label(&display_path(&default_writable, &home));
        writable_dir_button.set_tooltip_text(Some(&default_writable.display().to_string()));
        writable_dir_button.set_visible(false);

        footer.append(&sandbox_label);
        footer.append(&sandbox_dropdown);
        footer.append(&writable_dir_button);
        footer.append(&run_button);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&split);
        body.append(&footer);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&body));

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&toolbar));
        dialog.set_child(Some(&toasts));

        Self {
            parent: parent.clone(),
            dialog,
            toasts,
            browse_button,
            info_button,
            run_button,
            sidebar_list,
            form_box,
            sandbox_dropdown,
            writable_dir_button,
            running_box,
            spinner,
            current_file,
            current_dir,
            writable_dir_override: RefCell::new(None),
            bwrap_ok,
            scripts: RefCell::new(Vec::new()),
            selected_index: Cell::new(None),
            controls: RefCell::new(Vec::new()),
            reset_button: RefCell::new(None),
            state_store,
            saved_state: RefCell::new(saved_state),
            rebuilding_sidebar: Cell::new(false),
            suppress_state_events: Cell::new(false),
            running_cancel: RefCell::new(None),
            on_open_docs,
            on_output,
        }
    }

    fn load_initial_scripts(self: &Rc<Self>) {
        self.load_scripts_from(&scripts::scripts_directory());
    }

    fn load_scripts_from(self: &Rc<Self>, dir: &Path) {
        let (mut found, _diags) = scripts::discover(dir);
        let saved_selected = self.saved_state.borrow().selected_script.clone();
        let initial_idx = if let Some(ref target) = saved_selected {
            if let Some(pos) = found
                .iter()
                .position(|s| script_state::canonical_script_key(&s.path) == *target)
            {
                Some(pos)
            } else if target.is_file() {
                found.push(scripts::inspect_script(target));
                Some(found.len() - 1)
            } else {
                (!found.is_empty()).then_some(0)
            }
        } else {
            (!found.is_empty()).then_some(0)
        };
        *self.scripts.borrow_mut() = found;
        self.rebuild_sidebar(initial_idx);
    }

    fn wire_signals(self: &Rc<Self>) {
        // Sidebar selection.
        {
            let weak = Rc::downgrade(self);
            self.sidebar_list.connect_row_selected(move |_, row| {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                let index = row.map(|r| r.index() as usize);
                this.select_script(index);
            });
        }

        // Info button opens docs/scripts.md.
        {
            let weak = Rc::downgrade(self);
            self.info_button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    (this.on_open_docs)();
                }
            });
        }

        // Browse button.
        {
            let weak = Rc::downgrade(self);
            self.browse_button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.browse_for_script();
                }
            });
        }

        // Sandbox dropdown.
        {
            let weak = Rc::downgrade(self);
            self.sandbox_dropdown
                .connect_selected_notify(move |dropdown| {
                    if let Some(this) = weak.upgrade() {
                        this.writable_dir_button
                            .set_visible(this.bwrap_ok && dropdown.selected() == SANDBOX_WRITABLE);
                        if !this.suppress_state_events.get() {
                            this.update_reset_sensitivity();
                        }
                    }
                });
        }

        // Writable folder picker.
        {
            let weak = Rc::downgrade(self);
            self.writable_dir_button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.choose_writable_dir();
                }
            });
        }

        // Run / Cancel button.
        {
            let weak = Rc::downgrade(self);
            self.run_button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    if let Some(cancel) = this.running_cancel.borrow().as_ref() {
                        cancel.cancel();
                    } else {
                        this.request_run();
                    }
                }
            });
        }

        // Sole strong owner of `DialogState`; released when the dialog closes.
        let keep_alive = Cell::new(Some(self.clone()));
        self.dialog.connect_closed(move |_| {
            let Some(keep_alive) = keep_alive.take() else {
                return;
            };
            keep_alive.capture_current_script_state();
            keep_alive.flush_state();
            if let Some(cancel) = keep_alive.running_cancel.borrow().as_ref() {
                cancel.cancel();
            };
        });
    }

    fn rebuild_sidebar(self: &Rc<Self>, select_idx: Option<usize>) {
        self.rebuilding_sidebar.set(true);
        self.selected_index.set(None);
        while let Some(child) = self.sidebar_list.first_child() {
            self.sidebar_list.remove(&child);
        }

        let scripts = self.scripts.borrow();
        if scripts.is_empty() {
            drop(scripts);
            self.rebuilding_sidebar.set(false);
            self.select_script(None);
            return;
        }

        for script in scripts.iter() {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            row.set_margin_start(6);
            row.set_margin_end(6);

            let title = gtk::Label::new(Some(&script.display_name()));
            title.set_xalign(0.0);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.add_css_class("heading");

            let subtitle_text = if script.metadata.is_err() {
                format!("{} (header error)", script.file_name())
            } else if !script.executable {
                format!("{} (not executable)", script.file_name())
            } else {
                script.file_name()
            };
            let subtitle = gtk::Label::new(Some(&subtitle_text));
            subtitle.set_xalign(0.0);
            subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
            subtitle.add_css_class("caption");
            subtitle.add_css_class("dim-label");

            row.append(&title);
            row.append(&subtitle);
            self.sidebar_list.append(&row);
        }
        let count = scripts.len();
        drop(scripts);
        self.rebuilding_sidebar.set(false);

        if let Some(idx) = select_idx.filter(|&i| i < count)
            && let Some(row) = self.sidebar_list.row_at_index(idx as i32)
        {
            self.sidebar_list.select_row(Some(&row));
        }
    }

    fn clear_form(&self) {
        self.controls.borrow_mut().clear();
        *self.reset_button.borrow_mut() = None;
        while let Some(child) = self.form_box.first_child() {
            self.form_box.remove(&child);
        }
    }

    fn select_script(self: &Rc<Self>, index: Option<usize>) {
        if self.rebuilding_sidebar.get() {
            return;
        }
        self.capture_current_script_state();
        self.selected_index.set(index);
        self.clear_form();

        let Some(idx) = index else {
            if self.scripts.borrow().is_empty() {
                self.saved_state.borrow_mut().selected_script = None;
                self.flush_state();
            }
            self.show_empty_state();
            self.update_run_sensitivity();
            return;
        };

        let script = {
            let scripts = self.scripts.borrow();
            scripts.get(idx).cloned()
        };
        let Some(script) = script else {
            self.show_empty_state();
            self.update_run_sensitivity();
            return;
        };

        let key = script_state::canonical_script_key(&script.path);
        self.saved_state.borrow_mut().selected_script = Some(key.clone());
        self.flush_state();
        let saved_params = self.saved_state.borrow().scripts.get(&key).cloned();

        let home = glib::home_dir();
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let heading = gtk::Label::new(Some(&script.display_name()));
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        heading.set_wrap(true);
        heading.add_css_class("title-2");
        title_row.append(&heading);

        if script.metadata.is_ok() {
            let reset_btn = gtk::Button::with_label("Reset");
            reset_btn.set_valign(gtk::Align::Center);
            reset_btn.set_tooltip_text(Some("Reset parameters and safety mode to defaults"));
            let weak = Rc::downgrade(self);
            reset_btn.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.reset_current_script();
                }
            });
            title_row.append(&reset_btn);
            *self.reset_button.borrow_mut() = Some(reset_btn);
        }
        self.form_box.append(&title_row);

        let path_label = gtk::Label::new(Some(&display_path(&script.path, &home)));
        path_label.set_xalign(0.0);
        path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path_label.set_tooltip_text(Some(&script.path.display().to_string()));
        path_label.add_css_class("caption");
        path_label.add_css_class("dim-label");
        self.form_box.append(&path_label);

        if !script.executable {
            let warn = gtk::Label::new(Some(
                "This script is not executable. Run `chmod +x` on it before running.",
            ));
            warn.set_xalign(0.0);
            warn.set_wrap(true);
            warn.add_css_class("error");
            self.form_box.append(&warn);
        }

        match &script.metadata {
            Err(err) => {
                let diag = gtk::Label::new(Some(&format!("Metadata header error:\n{err}")));
                diag.set_xalign(0.0);
                diag.set_wrap(true);
                diag.set_selectable(true);
                diag.add_css_class("error");
                self.form_box.append(&diag);
            }
            Ok(meta) => {
                if !meta.description.is_empty() {
                    let desc = gtk::Label::new(Some(&meta.description));
                    desc.set_xalign(0.0);
                    desc.set_wrap(true);
                    self.form_box.append(&desc);
                }

                *self.writable_dir_override.borrow_mut() =
                    saved_params.as_ref().and_then(|p| p.writable_dir.clone());
                self.refresh_writable_dir_button();

                if self.bwrap_ok {
                    let effective_sandbox = saved_params
                        .as_ref()
                        .and_then(|p| p.sandbox)
                        .unwrap_or(meta.sandbox);
                    let sandbox_idx = match effective_sandbox {
                        SandboxDefault::ReadOnly => SANDBOX_RO,
                        SandboxDefault::WritableDir => SANDBOX_WRITABLE,
                        SandboxDefault::None => SANDBOX_NONE,
                    };
                    self.suppress_state_events.set(true);
                    self.sandbox_dropdown.set_selected(sandbox_idx);
                    self.suppress_state_events.set(false);
                    self.writable_dir_button
                        .set_visible(sandbox_idx == SANDBOX_WRITABLE);
                }

                if meta.fields.is_empty() {
                    let none_label = gtk::Label::new(Some(
                        "This script defines no input fields. Click Run to execute it.",
                    ));
                    none_label.set_xalign(0.0);
                    none_label.set_wrap(true);
                    none_label.add_css_class("dim-label");
                    self.form_box.append(&none_label);
                } else {
                    self.form_box
                        .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                    let mut controls = Vec::with_capacity(meta.fields.len());
                    for spec in &meta.fields {
                        let saved = saved_params.as_ref().and_then(|p| p.fields.get(&spec.id));
                        let control = self.build_field_row(spec, saved);
                        controls.push(control);
                    }
                    *self.controls.borrow_mut() = controls;
                }
            }
        }

        self.update_run_sensitivity();
        self.update_reset_sensitivity();
    }

    fn show_empty_state(&self) {
        let dir = display_path(&scripts::scripts_directory(), &glib::home_dir());
        let status = adw::StatusPage::builder()
            .icon_name("utilities-terminal-symbolic")
            .title("No Scripts Found")
            .description(format!(
                "Place executable scripts in {dir} or click Browse… to run any script on disk."
            ))
            .vexpand(true)
            .build();
        self.form_box.append(&status);
    }

    fn build_field_row(
        self: &Rc<Self>,
        spec: &FieldSpec,
        saved: Option<&SavedFieldValue>,
    ) -> FieldControl {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 4);

        if spec.field_type != FieldType::Bool {
            let label_text = if spec.required {
                format!("{} *", spec.label)
            } else {
                spec.label.clone()
            };
            let label = gtk::Label::new(Some(&label_text));
            label.set_xalign(0.0);
            label.add_css_class("heading");
            container.append(&label);
        }

        let saved_text = match saved {
            Some(SavedFieldValue::Text(s)) => Some(s.as_str()),
            _ => None,
        };

        let widget = match spec.field_type {
            FieldType::Text => {
                let entry = gtk::Entry::new();
                if let Some(val) = saved_text.or(spec.default.as_deref()) {
                    entry.set_text(val);
                }
                if let Some(ref ph) = spec.placeholder {
                    entry.set_placeholder_text(Some(ph));
                }
                let weak = Rc::downgrade(self);
                entry.connect_changed(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_run_sensitivity();
                        this.update_reset_sensitivity();
                    }
                });
                container.append(&entry);
                FieldWidget::Text(entry)
            }
            FieldType::Multiline | FieldType::List => {
                let buffer = gtk::TextBuffer::new(None);
                if let Some(val) = saved_text.or(spec.default.as_deref()) {
                    buffer.set_text(val);
                }
                let view = gtk::TextView::with_buffer(&buffer);
                view.set_wrap_mode(gtk::WrapMode::WordChar);
                view.set_top_margin(6);
                view.set_bottom_margin(6);
                view.set_left_margin(8);
                view.set_right_margin(8);
                let scroll = gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .min_content_height(if spec.field_type == FieldType::Multiline {
                        88
                    } else {
                        64
                    })
                    .child(&view)
                    .build();
                scroll.add_css_class("script-text-frame");
                let weak = Rc::downgrade(self);
                buffer.connect_changed(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_run_sensitivity();
                        this.update_reset_sensitivity();
                    }
                });
                container.append(&scroll);
                if let Some(ref ph) = spec.placeholder {
                    let ph_label = gtk::Label::new(Some(ph));
                    ph_label.set_xalign(0.0);
                    ph_label.set_wrap(true);
                    ph_label.add_css_class("caption");
                    ph_label.add_css_class("dim-label");
                    container.append(&ph_label);
                }
                if spec.field_type == FieldType::Multiline {
                    FieldWidget::Multiline(buffer)
                } else {
                    FieldWidget::List(buffer)
                }
            }
            FieldType::Int => {
                let lo = spec.min.unwrap_or(i32::MIN as i64);
                let hi = spec.max.unwrap_or(i32::MAX as i64);
                let initial = saved_text
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|n| n.clamp(lo, hi))
                    .unwrap_or_else(|| default_int_value(spec))
                    as f64;
                let adj = gtk::Adjustment::new(initial, lo as f64, hi as f64, 1.0, 10.0, 0.0);
                let spin = gtk::SpinButton::new(Some(&adj), 1.0, 0);
                spin.set_halign(gtk::Align::Start);
                let weak = Rc::downgrade(self);
                spin.connect_value_changed(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_reset_sensitivity();
                    }
                });
                container.append(&spin);
                FieldWidget::Int(spin)
            }
            FieldType::Bool => {
                let check = gtk::CheckButton::with_label(&spec.label);
                check.add_css_class("notediff-labeled-check");
                let active = match saved_text {
                    Some("true" | "1") => true,
                    Some("false" | "0") => false,
                    _ => default_bool_value(spec),
                };
                check.set_active(active);
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_reset_sensitivity();
                    }
                });
                container.append(&check);
                FieldWidget::Bool(check)
            }
            FieldType::Choice => {
                let refs: Vec<&str> = spec.options.iter().map(String::as_str).collect();
                let dropdown = gtk::DropDown::from_strings(&refs);
                dropdown.set_halign(gtk::Align::Start);
                let initial = saved_text
                    .and_then(|val| spec.options.iter().position(|o| o == val))
                    .unwrap_or_else(|| default_option_index(spec, &spec.options));
                if !spec.options.is_empty() {
                    dropdown.set_selected(initial as u32);
                }
                let weak = Rc::downgrade(self);
                dropdown.connect_selected_notify(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_reset_sensitivity();
                    }
                });
                container.append(&dropdown);
                FieldWidget::Choice {
                    dropdown,
                    options: spec.options.clone(),
                }
            }
            FieldType::Slider => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let max_idx = spec.options.len().saturating_sub(1);
                let initial = saved_text
                    .and_then(|val| spec.options.iter().position(|o| o == val))
                    .unwrap_or_else(|| default_option_index(spec, &spec.options))
                    .min(max_idx);
                let scale = gtk::Scale::with_range(
                    gtk::Orientation::Horizontal,
                    0.0,
                    max_idx.max(1) as f64,
                    1.0,
                );
                scale.set_hexpand(true);
                scale.set_draw_value(false);
                scale.set_digits(0);
                scale.set_round_digits(0);
                scale.adjustment().set_page_increment(1.0);
                if max_idx == 0 {
                    scale.set_sensitive(false);
                } else {
                    for i in 0..=max_idx {
                        scale.add_mark(i as f64, gtk::PositionType::Bottom, None);
                    }
                    let scroll = gtk::EventControllerScroll::new(
                        gtk::EventControllerScrollFlags::VERTICAL
                            | gtk::EventControllerScrollFlags::DISCRETE,
                    );
                    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
                    let scale_ref = scale.downgrade();
                    scroll.connect_scroll(move |_, _, dy| {
                        if let Some(scale) = scale_ref.upgrade() {
                            let step = if dy < 0.0 {
                                1.0
                            } else if dy > 0.0 {
                                -1.0
                            } else {
                                0.0
                            };
                            if step != 0.0 {
                                let next =
                                    (scale.value().round() + step).clamp(0.0, max_idx as f64);
                                scale.set_value(next);
                                return glib::Propagation::Stop;
                            }
                        }
                        glib::Propagation::Proceed
                    });
                    scale.add_controller(scroll);
                }
                scale.set_value(initial as f64);

                let value_label = gtk::Label::new(spec.options.get(initial).map(String::as_str));
                value_label.set_width_chars(6);
                value_label.set_xalign(1.0);
                value_label.add_css_class("script-slider-value");

                let options_for_label = spec.options.clone();
                let label_ref = value_label.clone();
                let weak = Rc::downgrade(self);
                scale.connect_value_changed(move |scale| {
                    let idx = (scale.value().round().max(0.0) as usize)
                        .min(options_for_label.len().saturating_sub(1));
                    if let Some(text) = options_for_label.get(idx) {
                        label_ref.set_text(text);
                    }
                    if let Some(this) = weak.upgrade() {
                        this.update_reset_sensitivity();
                    }
                });

                row.append(&scale);
                row.append(&value_label);
                container.append(&row);
                FieldWidget::Slider {
                    scale,
                    options: spec.options.clone(),
                }
            }
            FieldType::File | FieldType::Dir => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let entry = gtk::Entry::new();
                entry.set_hexpand(true);
                if let Some(val) = saved_text.or(spec.default.as_deref()) {
                    entry.set_text(val);
                }
                if let Some(ref ph) = spec.placeholder {
                    entry.set_placeholder_text(Some(ph));
                }
                let weak = Rc::downgrade(self);
                entry.connect_changed(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_run_sensitivity();
                        this.update_reset_sensitivity();
                    }
                });

                let browse = gtk::Button::with_label("Browse…");
                let is_dir = spec.field_type == FieldType::Dir;
                let weak = Rc::downgrade(self);
                let entry_ref = entry.clone();
                browse.connect_clicked(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.pick_single_path(&entry_ref, is_dir);
                    }
                });

                row.append(&entry);
                row.append(&browse);
                container.append(&row);
                FieldWidget::PathEntry(entry)
            }
            FieldType::Paths => {
                let mut initial_paths = Vec::<PathBuf>::new();
                if let Some(SavedFieldValue::List(items)) = saved {
                    for item in items {
                        let p = PathBuf::from(item);
                        if !initial_paths.contains(&p) {
                            initial_paths.push(p);
                        }
                    }
                }
                let paths = Rc::new(RefCell::new(initial_paths));
                let list = gtk::ListBox::new();
                list.set_selection_mode(gtk::SelectionMode::None);
                list.add_css_class("boxed-list");
                list.set_visible(false);
                if !paths.borrow().is_empty() {
                    self.rebuild_paths_list(&paths, &list);
                }

                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let add_files = gtk::Button::with_label("+ File…");
                let add_folder = gtk::Button::with_label("+ Folder…");
                actions.append(&add_files);
                actions.append(&add_folder);

                {
                    let weak = Rc::downgrade(self);
                    let paths = paths.clone();
                    let list = list.clone();
                    add_files.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.pick_multiple_files(paths.clone(), list.clone());
                        }
                    });
                }
                {
                    let weak = Rc::downgrade(self);
                    let paths = paths.clone();
                    let list = list.clone();
                    add_folder.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.pick_folder_into_list(paths.clone(), list.clone());
                        }
                    });
                }

                if let Some(ref current_file) = self.current_file {
                    let add_current = gtk::Button::with_label("+ Current File");
                    let weak = Rc::downgrade(self);
                    let paths = paths.clone();
                    let list = list.clone();
                    let path = current_file.clone();
                    add_current.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.append_path(&paths, &list, path.clone());
                        }
                    });
                    actions.append(&add_current);
                }
                if let Some(ref current_dir) = self.current_dir {
                    let add_dir = gtk::Button::with_label("+ Current Folder");
                    let weak = Rc::downgrade(self);
                    let paths = paths.clone();
                    let list = list.clone();
                    let path = current_dir.clone();
                    add_dir.connect_clicked(move |_| {
                        if let Some(this) = weak.upgrade() {
                            this.append_path(&paths, &list, path.clone());
                        }
                    });
                    actions.append(&add_dir);
                }

                container.append(&list);
                container.append(&actions);
                FieldWidget::Paths(paths)
            }
        };

        if let Some(ref help) = spec.help {
            let caption = gtk::Label::new(Some(help));
            caption.set_xalign(0.0);
            caption.set_wrap(true);
            caption.add_css_class("caption");
            caption.add_css_class("dim-label");
            container.append(&caption);
        }

        self.form_box.append(&container);
        FieldControl {
            spec: spec.clone(),
            widget,
        }
    }

    fn append_path(
        self: &Rc<Self>,
        paths: &Rc<RefCell<Vec<PathBuf>>>,
        list: &gtk::ListBox,
        path: PathBuf,
    ) {
        {
            let mut vec = paths.borrow_mut();
            if vec.contains(&path) {
                return;
            }
            vec.push(path);
        }
        self.rebuild_paths_list(paths, list);
        self.update_run_sensitivity();
        self.update_reset_sensitivity();
    }

    fn rebuild_paths_list(self: &Rc<Self>, paths: &Rc<RefCell<Vec<PathBuf>>>, list: &gtk::ListBox) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let home = glib::home_dir();
        let snapshot = paths.borrow().clone();
        list.set_visible(!snapshot.is_empty());
        for (idx, path) in snapshot.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.set_margin_start(8);
            row.set_margin_end(4);

            let label = gtk::Label::new(Some(&display_path(path, &home)));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            label.set_tooltip_text(Some(&path.display().to_string()));

            let remove = gtk::Button::from_icon_name("window-close-symbolic");
            remove.add_css_class("flat");
            remove.set_valign(gtk::Align::Center);
            let weak = Rc::downgrade(self);
            let paths_ref = paths.clone();
            let list_ref = list.clone();
            remove.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    if idx < paths_ref.borrow().len() {
                        paths_ref.borrow_mut().remove(idx);
                    }
                    this.rebuild_paths_list(&paths_ref, &list_ref);
                    this.update_run_sensitivity();
                    this.update_reset_sensitivity();
                }
            });

            row.append(&label);
            row.append(&remove);
            list.append(&row);
        }
    }

    fn initial_folder(&self) -> PathBuf {
        self.current_dir.clone().unwrap_or_else(glib::home_dir)
    }

    fn pick_single_path(&self, entry: &gtk::Entry, is_dir: bool) {
        let dialog = gtk::FileDialog::builder()
            .title(if is_dir {
                "Select Folder"
            } else {
                "Select File"
            })
            .modal(true)
            .initial_folder(&gio::File::for_path(self.initial_folder()))
            .build();
        let entry = entry.clone();
        if is_dir {
            dialog.select_folder(
                Some(&self.parent),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        entry.set_text(&path.display().to_string());
                    }
                },
            );
        } else {
            dialog.open(
                Some(&self.parent),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        entry.set_text(&path.display().to_string());
                    }
                },
            );
        }
    }

    fn pick_multiple_files(self: &Rc<Self>, paths: Rc<RefCell<Vec<PathBuf>>>, list: gtk::ListBox) {
        let dialog = gtk::FileDialog::builder()
            .title("Add Files")
            .modal(true)
            .initial_folder(&gio::File::for_path(self.initial_folder()))
            .build();
        let weak = Rc::downgrade(self);
        dialog.open_multiple(
            Some(&self.parent),
            None::<&gio::Cancellable>,
            move |result| {
                let (Some(this), Ok(model)) = (weak.upgrade(), result) else {
                    return;
                };
                for i in 0..model.n_items() {
                    if let Some(item) = model.item(i)
                        && let Ok(file) = item.downcast::<gio::File>()
                        && let Some(path) = file.path()
                    {
                        this.append_path(&paths, &list, path);
                    }
                }
            },
        );
    }

    fn pick_folder_into_list(
        self: &Rc<Self>,
        paths: Rc<RefCell<Vec<PathBuf>>>,
        list: gtk::ListBox,
    ) {
        let dialog = gtk::FileDialog::builder()
            .title("Add Folder")
            .modal(true)
            .initial_folder(&gio::File::for_path(self.initial_folder()))
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(
            Some(&self.parent),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(this), Ok(file)) = (weak.upgrade(), result)
                    && let Some(path) = file.path()
                {
                    this.append_path(&paths, &list, path);
                }
            },
        );
    }

    fn browse_for_script(self: &Rc<Self>) {
        let start = if scripts::scripts_directory().is_dir() {
            scripts::scripts_directory()
        } else {
            self.initial_folder()
        };
        let dialog = gtk::FileDialog::builder()
            .title("Choose Script")
            .modal(true)
            .initial_folder(&gio::File::for_path(start))
            .build();
        let weak = Rc::downgrade(self);
        dialog.open(
            Some(&self.parent),
            None::<&gio::Cancellable>,
            move |result| {
                let (Some(this), Ok(file)) = (weak.upgrade(), result) else {
                    return;
                };
                let Some(path) = file.path() else {
                    return;
                };
                this.capture_current_script_state();
                let inspected = scripts::inspect_script(&path);
                let idx = {
                    let mut scripts = this.scripts.borrow_mut();
                    if let Some(pos) = scripts.iter().position(|s| s.path == inspected.path) {
                        scripts[pos] = inspected;
                        pos
                    } else {
                        scripts.push(inspected);
                        scripts.len() - 1
                    }
                };
                this.rebuild_sidebar(Some(idx));
            },
        );
    }

    fn effective_writable_dir(&self) -> PathBuf {
        self.writable_dir_override
            .borrow()
            .clone()
            .unwrap_or_else(|| self.initial_folder())
    }

    fn refresh_writable_dir_button(&self) {
        let dir = self.effective_writable_dir();
        let home = glib::home_dir();
        self.writable_dir_button
            .set_label(&display_path(&dir, &home));
        self.writable_dir_button
            .set_tooltip_text(Some(&dir.display().to_string()));
    }

    fn choose_writable_dir(self: &Rc<Self>) {
        let current = self.effective_writable_dir();
        let dialog = gtk::FileDialog::builder()
            .title("Allow Writing to Folder")
            .modal(true)
            .initial_folder(&gio::File::for_path(current))
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(
            Some(&self.parent),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(this), Ok(file)) = (weak.upgrade(), result)
                    && let Some(path) = file.path()
                {
                    *this.writable_dir_override.borrow_mut() = Some(path);
                    this.refresh_writable_dir_button();
                    this.update_reset_sensitivity();
                }
            },
        );
    }

    fn selected_sandbox_default(&self) -> SandboxDefault {
        match self.sandbox_dropdown.selected() {
            SANDBOX_RO => SandboxDefault::ReadOnly,
            SANDBOX_WRITABLE => SandboxDefault::WritableDir,
            _ => SandboxDefault::None,
        }
    }

    fn capture_current_script_state(&self) {
        let Some(script) = self.selected_script() else {
            return;
        };
        let Ok(meta) = &script.metadata else {
            return;
        };
        let key = script_state::canonical_script_key(&script.path);
        let mut params = ScriptParams::default();

        if self.bwrap_ok {
            let selected = self.selected_sandbox_default();
            if selected != meta.sandbox {
                params.sandbox = Some(selected);
            }
        } else if let Some(prev) = self.saved_state.borrow().scripts.get(&key) {
            params.sandbox = prev.sandbox;
        }

        if let Some(ref dir) = *self.writable_dir_override.borrow() {
            params.writable_dir = Some(dir.clone());
        }

        for control in self.controls.borrow().iter() {
            if !control.is_default() {
                params
                    .fields
                    .insert(control.spec.id.clone(), control.raw_saved_value());
            }
        }

        self.saved_state.borrow_mut().set_script_params(key, params);
    }

    fn flush_state(&self) {
        self.state_store.save(&self.saved_state.borrow());
    }

    fn has_non_default_state(&self) -> bool {
        let Some(script) = self.selected_script() else {
            return false;
        };
        let Ok(meta) = &script.metadata else {
            return false;
        };
        if self.bwrap_ok && self.selected_sandbox_default() != meta.sandbox {
            return true;
        }
        if self.writable_dir_override.borrow().is_some() {
            return true;
        }
        self.controls.borrow().iter().any(|c| !c.is_default())
    }

    fn update_reset_sensitivity(&self) {
        if let Some(ref btn) = *self.reset_button.borrow() {
            let running = self.running_cancel.borrow().is_some();
            btn.set_sensitive(!running && self.has_non_default_state());
        }
    }

    fn reset_current_script(self: &Rc<Self>) {
        let Some(idx) = self.selected_index.get() else {
            return;
        };
        let Some(script) = self.selected_script() else {
            return;
        };
        let key = script_state::canonical_script_key(&script.path);
        self.saved_state.borrow_mut().scripts.remove(&key);
        self.flush_state();
        self.selected_index.set(None);
        self.select_script(Some(idx));
    }

    fn update_run_sensitivity(&self) {
        if self.running_cancel.borrow().is_some() {
            self.run_button.set_sensitive(true);
            return;
        }
        let runnable = self.selected_script().is_some_and(|script| {
            script.executable
                && script.metadata.is_ok()
                && self
                    .controls
                    .borrow()
                    .iter()
                    .all(FieldControl::is_satisfied)
        });
        self.run_button.set_sensitive(runnable);
    }

    fn selected_script(&self) -> Option<DiscoveredScript> {
        let idx = self.selected_index.get()?;
        self.scripts.borrow().get(idx).cloned()
    }

    fn selected_sandbox(&self) -> SandboxMode {
        if !self.bwrap_ok {
            return SandboxMode::None;
        }
        match self.sandbox_dropdown.selected() {
            SANDBOX_RO => SandboxMode::ReadOnly,
            SANDBOX_WRITABLE => SandboxMode::WritableDir(self.effective_writable_dir()),
            _ => SandboxMode::None,
        }
    }

    fn set_running(&self, cancel: Option<CancelHandle>) {
        let running = cancel.is_some();
        *self.running_cancel.borrow_mut() = cancel;
        self.sidebar_list.set_sensitive(!running);
        self.form_box.set_sensitive(!running);
        self.browse_button.set_sensitive(!running);
        self.sandbox_dropdown
            .set_sensitive(!running && self.bwrap_ok);
        self.writable_dir_button.set_sensitive(!running);
        self.running_box.set_visible(running);
        if running {
            self.spinner.start();
            self.run_button.set_label("Cancel");
            self.run_button.remove_css_class("suggested-action");
            self.run_button.add_css_class("destructive-action");
            self.run_button.set_sensitive(true);
        } else {
            self.spinner.stop();
            self.run_button.set_label("Run");
            self.run_button.remove_css_class("destructive-action");
            self.run_button.add_css_class("suggested-action");
            self.update_run_sensitivity();
        }
        self.update_reset_sensitivity();
    }

    fn start_run(self: &Rc<Self>) {
        let Some(script) = self.selected_script() else {
            return;
        };
        let Ok(meta): Result<ScriptMetadata, _> = script.metadata.clone() else {
            return;
        };

        self.capture_current_script_state();
        self.flush_state();

        let values: Vec<(String, FieldValue)> = self
            .controls
            .borrow()
            .iter()
            .map(|c| (c.spec.id.clone(), c.value()))
            .collect();

        let context = InvocationContext {
            current_file: self.current_file.clone(),
            current_dir: self.current_dir.clone(),
            sandbox: self.selected_sandbox(),
        };
        let (args, env_vars) = scripts::build_invocation(&meta, &values, &context);
        let cancel = CancelHandle::new();
        self.set_running(Some(cancel.clone()));

        let script_path = script.path.clone();
        let output_ext = meta.output_ext.clone();
        let (tx, rx) = mpsc::channel();
        let worker_path = script_path.clone();

        thread::spawn(move || {
            let result = scripts::run_script(&worker_path, &args, &env_vars, &context, &cancel);
            let _ = tx.send(result);
        });

        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || match rx.try_recv() {
            Ok(result) => {
                if let Some(this) = weak.upgrade() {
                    this.finish_run(&script_path, &output_ext, result);
                }
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => {
                if weak.strong_count() == 0 {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            }
            Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
    }

    fn request_run(self: &Rc<Self>) {
        if !matches!(self.selected_sandbox(), SandboxMode::None) {
            self.start_run();
            return;
        }
        let weak = Rc::downgrade(self);
        dialogs::ask(
            &self.dialog,
            "Run without a sandbox?",
            "This script will run with your normal permissions and can read or change your files. Only continue if you trust it.",
            &[
                Response::new("cancel", "Cancel"),
                Response::destructive("run", "Run Without Sandbox"),
            ],
            "cancel",
            "cancel",
            move |response| {
                if response == "run"
                    && let Some(this) = weak.upgrade()
                {
                    this.start_run();
                }
            },
        );
    }

    fn finish_run(
        self: &Rc<Self>,
        script_path: &Path,
        output_ext: &str,
        result: Result<ScriptOutput, ScriptRunError>,
    ) {
        self.set_running(None);
        match result {
            Ok(output) => {
                let timestamp = glib::DateTime::now_local()
                    .ok()
                    .and_then(|dt| dt.format("%Y-%m-%d-%H%M%S").ok())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "output".to_string());
                let filename = scripts::format_output_filename(script_path, output_ext, &timestamp);
                self.dialog.close();
                (self.on_output)(filename, output);
            }
            Err(ScriptRunError::Cancelled) => {
                let t = toast::new(ToastKind::Info, "Script cancelled");
                t.set_timeout(3);
                self.toasts.add_toast(t);
            }
            Err(err) => {
                dialogs::ask(
                    &self.dialog,
                    err.heading(),
                    &err.message(),
                    &[Response::new("close", "Close")],
                    "close",
                    "close",
                    |_| {},
                );
            }
        }
    }
}
