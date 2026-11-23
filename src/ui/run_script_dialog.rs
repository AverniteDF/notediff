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
        Box::new(on_open_docs),
        Box::new(on_output),
    ));
    state.wire_signals();
    state.load_initial_scripts();
    state.dialog.present(Some(parent));
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
    writable_dir: RefCell<PathBuf>,
    bwrap_ok: bool,
    scripts: RefCell<Vec<DiscoveredScript>>,
    selected_index: Cell<Option<usize>>,
    controls: RefCell<Vec<FieldControl>>,
    running_cancel: RefCell<Option<CancelHandle>>,
    on_open_docs: OpenDocsCallback,
    on_output: OutputCallback,
}

impl DialogState {
    fn new(
        parent: &adw::ApplicationWindow,
        current_file: Option<PathBuf>,
        on_open_docs: OpenDocsCallback,
        on_output: OutputCallback,
    ) -> Self {
        let current_dir = current_file
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let default_writable = current_dir.clone().unwrap_or_else(glib::home_dir);
        let bwrap_ok = scripts::bwrap_available();

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
        info_button.set_tooltip_text(Some(
            "See docs/scripts.md for the full script authoring guide.",
        ));
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
        header.pack_end(&run_button);

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

        // Footer bar: sandbox controls + running indicator.
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.add_css_class("script-footer");
        footer.set_margin_top(8);
        footer.set_margin_bottom(8);
        footer.set_margin_start(12);
        footer.set_margin_end(12);

        let sandbox_label = gtk::Label::new(Some("Safety:"));
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

        if !bwrap_ok {
            let hint = gtk::Label::new(Some("Install bubblewrap (bwrap) to enable sandboxing"));
            hint.add_css_class("caption");
            hint.add_css_class("dim-label");
            footer.append(&hint);
        }

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let spinner = gtk::Spinner::new();
        let running_label = gtk::Label::new(Some("Running…"));
        running_label.add_css_class("dim-label");
        let running_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        running_box.append(&spinner);
        running_box.append(&running_label);
        running_box.set_visible(false);
        footer.append(&running_box);

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
            writable_dir: RefCell::new(default_writable),
            bwrap_ok,
            scripts: RefCell::new(Vec::new()),
            selected_index: Cell::new(None),
            controls: RefCell::new(Vec::new()),
            running_cancel: RefCell::new(None),
            on_open_docs,
            on_output,
        }
    }

    fn load_initial_scripts(self: &Rc<Self>) {
        let dir = scripts::scripts_directory();
        let (found, _diags) = scripts::discover(&dir);
        *self.scripts.borrow_mut() = found;
        self.rebuild_sidebar(Some(0));
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
                        this.start_run();
                    }
                }
            });
        }

        // Sole strong owner of `DialogState`; released when the dialog closes.
        let keep_alive = self.clone();
        self.dialog.connect_closed(move |_| {
            if let Some(cancel) = keep_alive.running_cancel.borrow().as_ref() {
                cancel.cancel();
            }
        });
    }

    fn rebuild_sidebar(self: &Rc<Self>, select_idx: Option<usize>) {
        while let Some(child) = self.sidebar_list.first_child() {
            self.sidebar_list.remove(&child);
        }

        let scripts = self.scripts.borrow();
        if scripts.is_empty() {
            drop(scripts);
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

        if let Some(idx) = select_idx.filter(|&i| i < count)
            && let Some(row) = self.sidebar_list.row_at_index(idx as i32)
        {
            self.sidebar_list.select_row(Some(&row));
        }
    }

    fn clear_form(&self) {
        self.controls.borrow_mut().clear();
        while let Some(child) = self.form_box.first_child() {
            self.form_box.remove(&child);
        }
    }

    fn select_script(self: &Rc<Self>, index: Option<usize>) {
        self.selected_index.set(index);
        self.clear_form();

        let Some(idx) = index else {
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

        let home = glib::home_dir();
        let heading = gtk::Label::new(Some(&script.display_name()));
        heading.set_xalign(0.0);
        heading.set_wrap(true);
        heading.add_css_class("title-2");
        self.form_box.append(&heading);

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

                if self.bwrap_ok {
                    let sandbox_idx = match meta.sandbox {
                        SandboxDefault::ReadOnly => SANDBOX_RO,
                        SandboxDefault::WritableDir => SANDBOX_WRITABLE,
                        SandboxDefault::None => SANDBOX_NONE,
                    };
                    self.sandbox_dropdown.set_selected(sandbox_idx);
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
                        let control = self.build_field_row(spec);
                        controls.push(control);
                    }
                    *self.controls.borrow_mut() = controls;
                }
            }
        }

        self.update_run_sensitivity();
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

    fn build_field_row(self: &Rc<Self>, spec: &FieldSpec) -> FieldControl {
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

        let widget = match spec.field_type {
            FieldType::Text => {
                let entry = gtk::Entry::new();
                if let Some(ref def) = spec.default {
                    entry.set_text(def);
                }
                if let Some(ref ph) = spec.placeholder {
                    entry.set_placeholder_text(Some(ph));
                }
                let weak = Rc::downgrade(self);
                entry.connect_changed(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_run_sensitivity();
                    }
                });
                container.append(&entry);
                FieldWidget::Text(entry)
            }
            FieldType::Multiline | FieldType::List => {
                let buffer = gtk::TextBuffer::new(None);
                if let Some(ref def) = spec.default {
                    buffer.set_text(def);
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
                let lo = spec.min.unwrap_or(i32::MIN as i64) as f64;
                let hi = spec.max.unwrap_or(i32::MAX as i64) as f64;
                let initial = spec
                    .default
                    .as_deref()
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0) as f64;
                let adj = gtk::Adjustment::new(initial.clamp(lo, hi), lo, hi, 1.0, 10.0, 0.0);
                let spin = gtk::SpinButton::new(Some(&adj), 1.0, 0);
                spin.set_halign(gtk::Align::Start);
                container.append(&spin);
                FieldWidget::Int(spin)
            }
            FieldType::Bool => {
                let check = gtk::CheckButton::with_label(&spec.label);
                let active = matches!(spec.default.as_deref(), Some("true" | "1"));
                check.set_active(active);
                container.append(&check);
                FieldWidget::Bool(check)
            }
            FieldType::Choice => {
                let refs: Vec<&str> = spec.options.iter().map(String::as_str).collect();
                let dropdown = gtk::DropDown::from_strings(&refs);
                dropdown.set_halign(gtk::Align::Start);
                if let Some(ref def) = spec.default
                    && let Some(pos) = spec.options.iter().position(|o| o == def)
                {
                    dropdown.set_selected(pos as u32);
                }
                container.append(&dropdown);
                FieldWidget::Choice {
                    dropdown,
                    options: spec.options.clone(),
                }
            }
            FieldType::Slider => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let max_idx = spec.options.len().saturating_sub(1);
                let initial = spec
                    .default
                    .as_deref()
                    .and_then(|def| spec.options.iter().position(|o| o == def))
                    .unwrap_or(0)
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
                scale.connect_value_changed(move |scale| {
                    let idx = (scale.value().round().max(0.0) as usize)
                        .min(options_for_label.len().saturating_sub(1));
                    if let Some(text) = options_for_label.get(idx) {
                        label_ref.set_text(text);
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
                if let Some(ref def) = spec.default {
                    entry.set_text(def);
                }
                if let Some(ref ph) = spec.placeholder {
                    entry.set_placeholder_text(Some(ph));
                }
                let weak = Rc::downgrade(self);
                entry.connect_changed(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_run_sensitivity();
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
                let paths = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
                let list = gtk::ListBox::new();
                list.set_selection_mode(gtk::SelectionMode::None);
                list.add_css_class("boxed-list");
                list.set_visible(false);

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

    fn choose_writable_dir(self: &Rc<Self>) {
        let current = self.writable_dir.borrow().clone();
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
                    let home = glib::home_dir();
                    this.writable_dir_button
                        .set_label(&display_path(&path, &home));
                    this.writable_dir_button
                        .set_tooltip_text(Some(&path.display().to_string()));
                    *this.writable_dir.borrow_mut() = path;
                }
            },
        );
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
            SANDBOX_WRITABLE => SandboxMode::WritableDir(self.writable_dir.borrow().clone()),
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
    }

    fn start_run(self: &Rc<Self>) {
        let Some(script) = self.selected_script() else {
            return;
        };
        let Ok(meta): Result<ScriptMetadata, _> = script.metadata.clone() else {
            return;
        };

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
