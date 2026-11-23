//! Find in Files dialog (`Ctrl+Shift+F`): recursive literal search across a
//! folder with Match Case, Whole Word, include/exclude globs, async execution
//! with cancellation, and click-to-jump into the matching file and line.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::core::find_in_files::{
    self, DEFAULT_MAX_MATCHES, FileMatch, SearchOptions, SearchReport,
};
use crate::core::textstats::format_thousands;
use crate::ui::dialogs::{self, Response};
use crate::ui::display_path;

type MatchCallback = Box<dyn Fn(PathBuf, usize, usize, usize) -> bool>;
type DirtyBuffersCallback = Box<dyn Fn() -> HashMap<PathBuf, String>>;

#[derive(Clone, Debug)]
struct SavedInputs {
    query: String,
    match_case: bool,
    whole_word: bool,
    folder: String,
    include: String,
    exclude: String,
}

thread_local! {
    /// Persists the Find in Files input fields across close/reopen within a session.
    static SAVED_INPUTS: RefCell<Option<SavedInputs>> = const { RefCell::new(None) };
}

/// Present the Find in Files dialog over `parent`.
pub fn present(
    parent: &adw::ApplicationWindow,
    initial_folder: PathBuf,
    initial_query: Option<String>,
    snapshot_dirty: impl Fn() -> HashMap<PathBuf, String> + 'static,
    on_open_match: impl Fn(PathBuf, usize, usize, usize) -> bool + 'static,
) {
    let state = Rc::new(DialogState::new(
        parent,
        initial_folder,
        initial_query,
        Box::new(snapshot_dirty),
        Box::new(on_open_match),
    ));
    state.wire_signals();
    state.dialog.present(Some(parent));
    state.query_entry.grab_focus();
    if !state.query_entry.text().is_empty() {
        state.query_entry.select_region(0, -1);
    }
}

#[cfg(test)]
pub(super) fn close_for_smoke(parent: &adw::ApplicationWindow) -> impl Fn() -> bool + use<> {
    let state = Rc::new(DialogState::new(
        parent,
        glib::home_dir(),
        None,
        Box::new(HashMap::new),
        Box::new(|_, _, _, _| false),
    ));
    let weak = Rc::downgrade(&state);
    state.wire_signals();
    state.dialog.present(Some(parent));
    state.dialog.close();
    move || weak.upgrade().is_none()
}

struct DialogState {
    parent: adw::ApplicationWindow,
    dialog: adw::Dialog,
    query_entry: gtk::SearchEntry,
    match_case: gtk::ToggleButton,
    whole_word: gtk::ToggleButton,
    folder_entry: gtk::Entry,
    browse_button: gtk::Button,
    include_entry: gtk::Entry,
    exclude_entry: gtk::Entry,
    search_button: gtk::Button,
    results_stack: gtk::Stack,
    results_scroll: gtk::ScrolledWindow,
    results_box: gtk::Box,
    status_label: gtk::Label,
    spinner: gtk::Spinner,
    running_cancel: RefCell<Option<Arc<AtomicBool>>>,
    snapshot_dirty: DirtyBuffersCallback,
    on_open_match: MatchCallback,
}

impl DialogState {
    fn new(
        parent: &adw::ApplicationWindow,
        initial_folder: PathBuf,
        initial_query: Option<String>,
        snapshot_dirty: DirtyBuffersCallback,
        on_open_match: MatchCallback,
    ) -> Self {
        let dialog = adw::Dialog::builder()
            .title("Find in Files")
            .content_width(820)
            .content_height(620)
            .build();

        let search_button = gtk::Button::with_label("Search");
        search_button.add_css_class("suggested-action");

        let header = adw::HeaderBar::new();
        header.pack_end(&search_button);

        let saved = SAVED_INPUTS.with_borrow(Clone::clone);

        // Row 1: Query + Aa / W toggles.
        let query_entry = gtk::SearchEntry::new();
        query_entry.set_placeholder_text(Some("Search text"));
        query_entry.set_hexpand(true);
        if let Some(ref query) = initial_query {
            query_entry.set_text(query);
        } else if let Some(ref s) = saved {
            query_entry.set_text(&s.query);
        }

        let options_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        options_box.add_css_class("linked");
        let match_case = gtk::ToggleButton::with_label("Aa");
        match_case.set_tooltip_text(Some("Match Case"));
        let whole_word = gtk::ToggleButton::with_label("W");
        whole_word.set_tooltip_text(Some("Whole Word"));
        if let Some(ref s) = saved {
            match_case.set_active(s.match_case);
            whole_word.set_active(s.whole_word);
        }
        options_box.append(&match_case);
        options_box.append(&whole_word);

        let query_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        query_row.append(&query_entry);
        query_row.append(&options_box);

        // Row 2: Folder entry + Browse button.
        let folder_label = gtk::Label::new(Some("Folder:"));
        folder_label.add_css_class("dim-label");
        folder_label.set_width_chars(7);
        folder_label.set_xalign(0.0);

        let folder_entry = gtk::Entry::new();
        folder_entry.set_hexpand(true);
        if let Some(ref s) = saved
            && !s.folder.is_empty()
        {
            folder_entry.set_text(&s.folder);
        } else {
            folder_entry.set_text(&display_path(&initial_folder, &glib::home_dir()));
        }

        let browse_button = gtk::Button::with_label("Browse…");

        let folder_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        folder_row.append(&folder_label);
        folder_row.append(&folder_entry);
        folder_row.append(&browse_button);

        // Row 3: Include / Exclude glob filters.
        let include_label = gtk::Label::new(Some("Files:"));
        include_label.add_css_class("dim-label");
        include_label.set_width_chars(7);
        include_label.set_xalign(0.0);

        let include_entry = gtk::Entry::new();
        include_entry.set_hexpand(true);
        include_entry.set_placeholder_text(Some("*.rs, *.md (all files if blank)"));
        if let Some(ref s) = saved {
            include_entry.set_text(&s.include);
        }

        let exclude_label = gtk::Label::new(Some("Exclude:"));
        exclude_label.add_css_class("dim-label");

        let exclude_entry = gtk::Entry::new();
        exclude_entry.set_hexpand(true);
        exclude_entry.set_placeholder_text(Some("*.lock, *.log"));
        if let Some(ref s) = saved {
            exclude_entry.set_text(&s.exclude);
        }

        let filter_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        filter_row.append(&include_label);
        filter_row.append(&include_entry);
        filter_row.append(&exclude_label);
        filter_row.append(&exclude_entry);

        let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls.set_margin_top(12);
        controls.set_margin_bottom(12);
        controls.set_margin_start(14);
        controls.set_margin_end(14);
        controls.append(&query_row);
        controls.append(&folder_row);
        controls.append(&filter_row);

        // Results area.
        let empty_page = adw::StatusPage::builder()
            .icon_name("system-search-symbolic")
            .title("Find in Files")
            .description("Enter text to search across UTF-8 files in the chosen folder.")
            .vexpand(true)
            .build();

        let no_matches_page = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title("No Matches Found")
            .description("Try a different query, folder, or file filter.")
            .vexpand(true)
            .build();

        let results_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        results_box.set_margin_top(10);
        results_box.set_margin_bottom(10);
        results_box.set_margin_start(12);
        results_box.set_margin_end(12);
        let results_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&results_box)
            .build();

        let results_stack = gtk::Stack::new();
        results_stack.set_vexpand(true);
        results_stack.add_named(&empty_page, Some("empty"));
        results_stack.add_named(&no_matches_page, Some("no-matches"));
        results_stack.add_named(&results_scroll, Some("results"));
        results_stack.set_visible_child_name("empty");

        // Footer bar.
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        let status_label = gtk::Label::new(Some("Ready"));
        status_label.set_xalign(0.0);
        status_label.set_hexpand(true);
        status_label.add_css_class("caption");
        status_label.add_css_class("dim-label");

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("script-footer");
        footer.set_margin_top(6);
        footer.set_margin_bottom(6);
        footer.set_margin_start(14);
        footer.set_margin_end(14);
        footer.append(&spinner);
        footer.append(&status_label);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&controls);
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        body.append(&results_stack);
        body.append(&footer);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&body));
        dialog.set_child(Some(&toolbar));

        search_button.set_sensitive(!query_entry.text().is_empty());

        Self {
            parent: parent.clone(),
            dialog,
            query_entry,
            match_case,
            whole_word,
            folder_entry,
            browse_button,
            include_entry,
            exclude_entry,
            search_button,
            results_stack,
            results_scroll,
            results_box,
            status_label,
            spinner,
            running_cancel: RefCell::new(None),
            snapshot_dirty,
            on_open_match,
        }
    }

    fn wire_signals(self: &Rc<Self>) {
        // Query changes update Search button sensitivity.
        {
            let weak = Rc::downgrade(self);
            self.query_entry.connect_changed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.update_search_sensitivity();
                }
            });
        }

        // Pressing Enter in any input field triggers Search.
        {
            let weak = Rc::downgrade(self);
            self.query_entry.connect_activate(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.start_search();
                }
            });
        }
        for entry in [&self.folder_entry, &self.include_entry, &self.exclude_entry] {
            let weak = Rc::downgrade(self);
            entry.connect_activate(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.start_search();
                }
            });
        }

        // Browse folder button.
        {
            let weak = Rc::downgrade(self);
            self.browse_button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.choose_folder();
                }
            });
        }

        // Search / Cancel button.
        {
            let weak = Rc::downgrade(self);
            self.search_button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    if let Some(cancel) = this.running_cancel.borrow().as_ref() {
                        cancel.store(true, Ordering::Relaxed);
                    } else {
                        this.start_search();
                    }
                }
            });
        }

        // Sole strong owner of `DialogState`, released when the dialog closes.
        let keep_alive = Cell::new(Some(self.clone()));
        self.dialog.connect_closed(move |_| {
            let Some(keep_alive) = keep_alive.take() else {
                return;
            };
            keep_alive.save_inputs();
            if let Some(cancel) = keep_alive.running_cancel.borrow().as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
        });
    }

    fn save_inputs(&self) {
        SAVED_INPUTS.with_borrow_mut(|slot| {
            *slot = Some(SavedInputs {
                query: self.query_entry.text().to_string(),
                match_case: self.match_case.is_active(),
                whole_word: self.whole_word.is_active(),
                folder: self.folder_entry.text().to_string(),
                include: self.include_entry.text().to_string(),
                exclude: self.exclude_entry.text().to_string(),
            });
        });
    }

    fn update_search_sensitivity(&self) {
        if self.running_cancel.borrow().is_some() {
            self.search_button.set_sensitive(true);
            return;
        }
        self.search_button
            .set_sensitive(!self.query_entry.text().is_empty());
    }

    fn resolve_folder(&self) -> PathBuf {
        let raw = self.folder_entry.text().as_str().trim().to_string();
        if raw.is_empty() {
            return glib::home_dir();
        }
        if raw == "~" {
            return glib::home_dir();
        }
        if let Some(rest) = raw.strip_prefix("~/") {
            return glib::home_dir().join(rest);
        }
        PathBuf::from(raw)
    }

    fn choose_folder(self: &Rc<Self>) {
        let current = self.resolve_folder();
        let initial = if current.is_dir() {
            current
        } else {
            glib::home_dir()
        };
        let dialog = gtk::FileDialog::builder()
            .title("Search Folder")
            .modal(true)
            .initial_folder(&gio::File::for_path(initial))
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(Some(&self.parent), None::<&gio::Cancellable>, move |res| {
            if let (Some(this), Ok(file)) = (weak.upgrade(), res)
                && let Some(path) = file.path()
            {
                this.folder_entry
                    .set_text(&display_path(&path, &glib::home_dir()));
            }
        });
    }

    fn set_running(&self, cancel: Option<Arc<AtomicBool>>) {
        let running = cancel.is_some();
        *self.running_cancel.borrow_mut() = cancel;
        self.query_entry.set_sensitive(!running);
        self.match_case.set_sensitive(!running);
        self.whole_word.set_sensitive(!running);
        self.folder_entry.set_sensitive(!running);
        self.browse_button.set_sensitive(!running);
        self.include_entry.set_sensitive(!running);
        self.exclude_entry.set_sensitive(!running);
        self.spinner.set_visible(running);
        if running {
            self.spinner.start();
            self.status_label.set_text("Searching…");
            self.search_button.set_label("Cancel");
            self.search_button.remove_css_class("suggested-action");
            self.search_button.add_css_class("destructive-action");
            self.search_button.set_sensitive(true);
        } else {
            self.spinner.stop();
            self.search_button.set_label("Search");
            self.search_button.remove_css_class("destructive-action");
            self.search_button.add_css_class("suggested-action");
            self.update_search_sensitivity();
        }
    }

    fn start_search(self: &Rc<Self>) {
        if self.running_cancel.borrow().is_some() {
            return;
        }
        let query = self.query_entry.text().to_string();
        if query.is_empty() {
            return;
        }
        let root = self.resolve_folder();
        if !root.is_dir() {
            dialogs::ask(
                &self.dialog,
                "Folder not found",
                &format!("{} is not an existing folder.", root.display()),
                &[Response::new("close", "Close")],
                "close",
                "close",
                |_| {},
            );
            return;
        }

        let options = SearchOptions {
            root,
            query,
            match_case: self.match_case.is_active(),
            whole_word: self.whole_word.is_active(),
            include_globs: find_in_files::parse_globs(self.include_entry.text().as_str()),
            exclude_globs: find_in_files::parse_globs(self.exclude_entry.text().as_str()),
            max_matches: DEFAULT_MAX_MATCHES,
            open_buffers: (self.snapshot_dirty)(),
        };

        let cancel = Arc::new(AtomicBool::new(false));
        self.set_running(Some(cancel.clone()));
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let res = find_in_files::search(&options, &cancel);
            let _ = tx.send(res);
        });

        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || match rx.try_recv() {
            Ok(result) => {
                if let Some(this) = weak.upgrade() {
                    this.finish_search(result);
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

    fn finish_search(self: &Rc<Self>, result: Result<SearchReport, String>) {
        self.set_running(None);
        match result {
            Err(err) => {
                self.status_label.set_text("Search failed");
                dialogs::ask(
                    &self.dialog,
                    "Search failed",
                    &err,
                    &[Response::new("close", "Close")],
                    "close",
                    "close",
                    |_| {},
                );
            }
            Ok(report) => {
                if report.cancelled {
                    self.status_label.set_text("Search cancelled");
                    return;
                }
                self.populate_results(&report);
            }
        }
    }

    fn populate_results(self: &Rc<Self>, report: &SearchReport) {
        while let Some(child) = self.results_box.first_child() {
            self.results_box.remove(&child);
        }

        if report.matches.is_empty() {
            self.results_stack.set_visible_child_name("no-matches");
            self.status_label.set_text(&format!(
                "No matches ({} files searched)",
                format_thousands(report.files_searched)
            ));
            return;
        }

        // Matches from `find_in_files::search` are already grouped contiguously
        // per file in deterministic traversal order.
        let mut start = 0usize;
        while start < report.matches.len() {
            let path = &report.matches[start].path;
            let mut end = start + 1;
            while end < report.matches.len() && report.matches[end].path == *path {
                end += 1;
            }
            let group = &report.matches[start..end];
            self.results_box.append(&self.build_file_group(group));
            start = end;
        }
        self.results_stack.set_visible_child_name("results");

        let match_word = if report.total_matches == 1 {
            "match"
        } else {
            "matches"
        };
        let file_word = if report.files_matched == 1 {
            "file"
        } else {
            "files"
        };
        let mut summary = format!(
            "{} {match_word} in {} {file_word} ({} searched)",
            format_thousands(report.total_matches),
            format_thousands(report.files_matched),
            format_thousands(report.files_searched),
        );
        if report.truncated {
            summary.push_str(&format!(
                " — stopped at {}",
                format_thousands(DEFAULT_MAX_MATCHES)
            ));
        }
        self.status_label.set_text(&summary);
    }

    fn build_file_group(self: &Rc<Self>, hits: &[FileMatch]) -> gtk::Expander {
        let first = &hits[0];
        let file_match_count: usize = hits.iter().map(|h| h.hits.len()).sum();
        let mut count_text = if file_match_count == 1 {
            "1 match".to_string()
        } else {
            format!("{} matches", format_thousands(file_match_count))
        };
        if first.from_unsaved_buffer {
            count_text.push_str(" · unsaved");
        }

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let path_label = gtk::Label::new(Some(&first.rel_path));
        path_label.set_xalign(0.0);
        path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        path_label.set_tooltip_text(Some(&first.path.display().to_string()));
        path_label.add_css_class("heading");

        let count_label = gtk::Label::new(Some(&count_text));
        count_label.add_css_class("caption");
        count_label.add_css_class("dim-label");
        count_label.set_valign(gtk::Align::Center);

        header.append(&path_label);
        header.append(&count_label);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        list.set_margin_top(4);
        list.set_margin_start(16);

        let mut pre_scroll_targets: Vec<(glib::WeakRef<gtk::Adjustment>, f64)> =
            Vec::with_capacity(hits.len());
        for hit in hits {
            let (row_widget, hadj_weak, center_ratio) = self.build_match_row(hit);
            let list_row = gtk::ListBoxRow::new();
            list_row.set_activatable(false);
            list_row.set_selectable(false);
            list_row.add_css_class("find-result-row");
            list_row.set_child(Some(&row_widget));
            list.append(&list_row);
            if let Some(ratio) = center_ratio {
                pre_scroll_targets.push((hadj_weak, ratio));
            }
        }

        let expander = gtk::Expander::new(None);
        expander.set_label_widget(Some(&header));
        expander.set_expanded(false);
        expander.set_child(Some(&list));

        // Pre-scroll each result preview to center its highlighted match the
        // first time this file group is expanded and laid out.
        let centered_once = Cell::new(false);
        expander.connect_expanded_notify(move |exp| {
            if !exp.is_expanded() || centered_once.replace(true) {
                return;
            }
            let targets = pre_scroll_targets.clone();
            glib::idle_add_local_once(move || {
                for (hadj_weak, ratio) in targets {
                    if let Some(hadj) = hadj_weak.upgrade() {
                        center_preview(&hadj, ratio);
                    }
                }
            });
        });

        expander
    }

    fn build_match_row(
        self: &Rc<Self>,
        hit: &FileMatch,
    ) -> (gtk::Box, glib::WeakRef<gtk::Adjustment>, Option<f64>) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);

        let active_idx = Rc::new(Cell::new(0usize));
        let multi_match = hit.hits.len() > 1;

        let line_label = gtk::Label::new(None);
        if multi_match {
            line_label.set_markup(&format!("<b>{}</b>", hit.line));
        } else {
            line_label.set_text(&hit.line.to_string());
            line_label.add_css_class("dim-label");
        }
        line_label.set_width_chars(5);
        line_label.set_xalign(1.0);
        line_label.set_valign(gtk::Align::Center);
        line_label.set_margin_top(4);
        line_label.set_margin_bottom(4);
        line_label.set_margin_start(8);
        line_label.set_margin_end(10);
        line_label.add_css_class("monospace");
        line_label.add_css_class("caption");
        line_label.add_css_class("script-slider-value");

        // Wrap the line number in its own full-height segment so clicks across
        // the number and its entire background stay isolated from the text area.
        let line_segment = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line_segment.set_vexpand(true);
        line_segment.append(&line_label);

        let first = &hit.hits[0];
        let first_highlight = Some(first.byte_range);
        let preview_label = gtk::Label::new(None);
        preview_label.set_markup(&format_preview_markup(&first.preview, first_highlight));
        preview_label.set_xalign(0.0);
        preview_label.set_halign(gtk::Align::Start);
        preview_label.set_margin_top(4);
        preview_label.set_margin_bottom(4);
        preview_label.set_margin_end(8);
        preview_label.add_css_class("monospace");
        preview_label.add_css_class("caption");

        // Keep each result strictly single-line without widening the dialog,
        // and forward vertical mouse-wheel scrolling to the outer results list.
        let preview_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(false)
            .propagate_natural_height(true)
            .hexpand(true)
            .child(&preview_label)
            .build();
        let hadj = preview_scroll.hadjustment();

        let vscroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        vscroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let outer_vadj = self.results_scroll.vadjustment().downgrade();
        vscroll.connect_scroll(move |controller, _dx, dy| {
            if dy == 0.0 {
                return glib::Propagation::Proceed;
            }
            if let Some(vadj) = outer_vadj.upgrade() {
                let delta = match controller.unit() {
                    gtk::gdk::ScrollUnit::Surface => dy,
                    _ => dy * vadj.page_size().max(1.0).powf(2.0 / 3.0),
                };
                let max_val = (vadj.upper() - vadj.page_size()).max(vadj.lower());
                vadj.set_value((vadj.value() + delta).clamp(vadj.lower(), max_val));
            }
            glib::Propagation::Stop
        });
        preview_scroll.add_controller(vscroll);

        // Multi-match line number segment cycles which match is highlighted and
        // pans the single-line preview to center it. Single-match segments are
        // intentionally non-clickable.
        if multi_match {
            let pointer_cursor = gtk::gdk::Cursor::from_name("pointer", None);
            line_segment.set_cursor(pointer_cursor.as_ref());

            let line_click = gtk::GestureClick::new();
            line_click.set_button(1);
            let line_hits = hit.hits.clone();
            let active = active_idx.clone();
            let label_ref = preview_label.downgrade();
            let hadj_ref = hadj.downgrade();
            line_click.connect_released(move |_, _, _, _| {
                let next = (active.get() + 1) % line_hits.len();
                active.set(next);
                let hit = &line_hits[next];
                let range = Some(hit.byte_range);
                if let Some(label) = label_ref.upgrade() {
                    label.set_markup(&format_preview_markup(&hit.preview, range));
                }
                if let (Some(hadj), Some(ratio)) =
                    (hadj_ref.upgrade(), match_center_ratio(&hit.preview, range))
                {
                    center_preview(&hadj, ratio);
                    let hadj = hadj.downgrade();
                    let active = active.clone();
                    glib::idle_add_local_once(move || {
                        if active.get() == next
                            && let Some(hadj) = hadj.upgrade()
                        {
                            center_preview(&hadj, ratio);
                        }
                    });
                }
            });
            line_segment.add_controller(line_click);
        }

        // Only clicking the text area opens the currently highlighted match.
        let pointer_cursor = gtk::gdk::Cursor::from_name("pointer", None);
        preview_scroll.set_cursor(pointer_cursor.as_ref());
        let text_click = gtk::GestureClick::new();
        text_click.set_button(1);
        let weak = Rc::downgrade(self);
        let hit_path = hit.path.clone();
        let hit_line = hit.line;
        let line_hits = hit.hits.clone();
        text_click.connect_released(move |_, _, _, _| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            let idx = active_idx.get();
            let Some(target) = line_hits.get(idx).or_else(|| line_hits.first()) else {
                return;
            };
            let close_dialog = (this.on_open_match)(
                hit_path.clone(),
                hit_line,
                target.column_char,
                target.match_chars,
            );
            if close_dialog {
                this.dialog.close();
            }
        });
        preview_scroll.add_controller(text_click);

        let ratio = match_center_ratio(&first.preview, first_highlight);
        let hadj_weak = hadj.downgrade();

        row.append(&line_segment);
        row.append(&preview_scroll);
        (row, hadj_weak, ratio)
    }
}

/// Fractional center position (`0.0..=1.0`) of the highlighted match within
/// `preview` (accurate for the monospace preview label).
fn match_center_ratio(preview: &str, highlight: Option<(usize, usize)>) -> Option<f64> {
    let (start_b, end_b) = highlight?;
    let total_chars = preview.chars().count();
    if total_chars == 0 || start_b >= end_b || end_b > preview.len() {
        return None;
    }
    let start_c = preview[..start_b].chars().count();
    let end_c = preview[..end_b].chars().count();
    Some(((start_c + end_c) as f64 / 2.0) / total_chars as f64)
}

/// Pan a single-line result preview so its highlighted match is centered when
/// the line overflows the visible viewport.
fn center_preview(hadj: &gtk::Adjustment, match_ratio: f64) {
    let page = hadj.page_size();
    let upper = hadj.upper();
    let max_scroll = (upper - page).max(0.0);
    if max_scroll <= 0.0 || page <= 0.0 {
        return;
    }
    let match_center = match_ratio * upper;
    let target = (match_center - page * 0.45).clamp(hadj.lower(), max_scroll);
    hadj.set_value(target);
}

fn format_preview_markup(text: &str, highlight: Option<(usize, usize)>) -> String {
    match highlight {
        Some((start, end)) if start < end && end <= text.len() => {
            let before = glib::markup_escape_text(&text[..start]);
            let matched = glib::markup_escape_text(&text[start..end]);
            let after = glib::markup_escape_text(&text[end..]);
            format!("{before}<b><u>{matched}</u></b>{after}")
        }
        _ => glib::markup_escape_text(text).to_string(),
    }
}

/// Default starting folder for Find in Files: the open document's parent
/// directory, or the user's home directory for untitled documents.
pub fn default_folder_for(doc_path: Option<&Path>) -> PathBuf {
    doc_path
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(glib::home_dir)
}
