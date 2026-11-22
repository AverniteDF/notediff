//! Editor Find, Replace and Go to Line.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use sourceview5::prelude::*;

use crate::core::tabs;
use crate::core::textlines;
use crate::core::textstats::format_thousands;
use crate::ui::dialogs::{self, Response};
use crate::ui::theme;
use crate::ui::toast::ToastKind;

use super::{AppWindow, add_action};

/// Maximum length of a single-line selection that automatically pre-populates
/// the Find entry when Find or Replace is invoked.
const MAX_PREPOPULATE_CHARS: usize = 256;
const NO_MATCH_CLASS: &str = "notediff-search-no-match";

pub(super) struct SearchControls {
    pub(super) replacement: gtk::Entry,
    pub(super) match_case: gtk::ToggleButton,
    pub(super) whole_word: gtk::ToggleButton,
    pub(super) replace_toggle: gtk::ToggleButton,
    pub(super) match_count: gtk::Label,
    replace_row: gtk::Box,
    context: sourceview5::SearchContext,
    settings: sourceview5::SearchSettings,
    /// Character offset where the current incremental search began.
    anchor_offset: Cell<i32>,
    /// Suppresses one incremental `buffer.select_range` after pre-populating
    /// `search_entry` from the editor selection so X11 primary selection stays
    /// on `search_entry` and typing replaces it cleanly.
    skip_next_incremental_select: Cell<bool>,
}

pub(super) fn build_search_bar(
    buffer: &sourceview5::Buffer,
) -> (gtk::SearchBar, gtk::SearchEntry, SearchControls) {
    let settings = sourceview5::SearchSettings::new();
    settings.set_wrap_around(true);
    settings.set_regex_enabled(false);
    settings.set_case_sensitive(false);
    settings.set_at_word_boundaries(false);

    let context = sourceview5::SearchContext::new(buffer, Some(&settings));
    context.set_highlight(false);

    let replace_toggle = gtk::ToggleButton::new();
    replace_toggle.set_icon_name("pan-end-symbolic");
    replace_toggle.add_css_class("flat");
    replace_toggle.set_tooltip_text(Some("Toggle Replace (Ctrl+H)"));

    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Find in document"));
    search_entry.set_hexpand(true);

    let match_count = gtk::Label::new(None);
    match_count.add_css_class("dim-label");
    match_count.add_css_class("search-match-count");
    match_count.set_visible(false);

    let nav_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    nav_box.add_css_class("linked");
    let prev_button = gtk::Button::from_icon_name("go-up-symbolic");
    prev_button.set_tooltip_text(Some("Previous Match (Shift+Enter / Shift+F3)"));
    prev_button.set_action_name(Some("win.find-previous"));
    let next_button = gtk::Button::from_icon_name("go-down-symbolic");
    next_button.set_tooltip_text(Some("Next Match (Enter / F3)"));
    next_button.set_action_name(Some("win.find-next"));
    nav_box.append(&prev_button);
    nav_box.append(&next_button);

    let options_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    options_box.add_css_class("linked");
    let match_case = gtk::ToggleButton::with_label("Aa");
    match_case.set_tooltip_text(Some("Match Case"));
    let whole_word = gtk::ToggleButton::with_label("W");
    whole_word.set_tooltip_text(Some("Whole Word"));
    options_box.append(&match_case);
    options_box.append(&whole_word);

    let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    find_row.append(&replace_toggle);
    find_row.append(&search_entry);
    find_row.append(&match_count);
    find_row.append(&nav_box);
    find_row.append(&options_box);

    let replacement = gtk::Entry::new();
    replacement.set_placeholder_text(Some("Replace with"));
    replacement.set_hexpand(true);
    let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    replace_row.append(&replacement);
    for (label, tooltip, action) in [
        (
            "Replace",
            "Replace current match (Enter)",
            "win.replace-one",
        ),
        (
            "Replace All",
            "Replace all matches (Ctrl+Enter)",
            "win.replace-all",
        ),
    ] {
        let button = gtk::Button::with_label(label);
        button.set_tooltip_text(Some(tooltip));
        button.set_action_name(Some(action));
        replace_row.append(&button);
    }
    replace_row.set_visible(false);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&find_row);
    content.append(&replace_row);

    let search_bar = gtk::SearchBar::new();
    search_bar.set_child(Some(&content));
    // Do not call `search_bar.connect_entry(&search_entry)`: GTK clears a
    // connected entry when the bar closes, which would wipe the last query
    // needed by F3 / Shift+F3 and by reopening Find without a selection.
    // Find is opened explicitly, never by typing while a toolbar button or
    // another non-editable widget has focus (for example after a file dialog).
    search_bar.set_search_mode(false);
    search_bar.set_show_close_button(true);

    (
        search_bar,
        search_entry,
        SearchControls {
            replacement,
            match_case,
            whole_word,
            replace_toggle,
            match_count,
            replace_row,
            context,
            settings,
            anchor_offset: Cell::new(0),
            skip_next_incremental_select: Cell::new(false),
        },
    )
}

impl AppWindow {
    pub(super) fn connect_search(this: &Rc<Self>) {
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_changed(move |entry| {
                if let Some(this) = weak.upgrade() {
                    let text = entry.text();
                    let query = (!text.is_empty()).then_some(text.as_str());
                    this.search_controls.settings.set_search_text(query);
                    this.update_match_count();
                    this.update_search_actions();
                }
            });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_search_changed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.update_search_actions();
                    if this
                        .search_controls
                        .skip_next_incremental_select
                        .replace(false)
                    {
                        this.update_match_count();
                        return;
                    }
                    this.find_incremental();
                }
            });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_activate(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.find_next();
                }
            });
        }
        {
            let entry_keys = gtk::EventControllerKey::new();
            entry_keys.set_name(Some("notediff-search-entry-keys"));
            entry_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(this);
            entry_keys.connect_key_pressed(move |_, key, _, modifiers| {
                let active_mods = modifiers & gtk::accelerator_get_default_mod_mask();
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                    && active_mods == gdk::ModifierType::SHIFT_MASK
                    && let Some(this) = weak.upgrade()
                {
                    this.find_previous();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            this.search_entry.add_controller(entry_keys);
        }
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_stop_search(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.search_bar.set_search_mode(false);
                    this.editor_view.grab_focus();
                }
            });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_bar
                .connect_search_mode_enabled_notify(move |bar| {
                    if let Some(this) = weak.upgrade() {
                        let active = bar.is_search_mode();
                        this.search_controls.context.set_highlight(active);
                        if active {
                            this.update_match_count();
                        } else {
                            this.search_entry.remove_css_class(NO_MATCH_CLASS);
                        }
                    }
                });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_controls
                .context
                .connect_occurrences_count_notify(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_match_count();
                        this.update_search_actions();
                    }
                });
        }
        {
            let weak = Rc::downgrade(this);
            this.editor_buffer.connect_mark_set(move |_, _, mark| {
                if matches!(mark.name().as_deref(), Some("insert" | "selection_bound"))
                    && let Some(this) = weak.upgrade()
                    && this.search_bar.is_search_mode()
                {
                    this.update_match_count();
                }
            });
        }

        let window = &this.window;
        add_action(window, this, "find", |this| this.open_search_bar(false));
        add_action(window, this, "find-next", |this| this.find_next());
        add_action(window, this, "find-previous", |this| this.find_previous());
        add_action(window, this, "replace", |this| this.open_search_bar(true));
        add_action(window, this, "replace-one", |this| this.replace_one());
        add_action(window, this, "replace-all", |this| this.replace_all());

        {
            let weak = Rc::downgrade(this);
            this.search_controls
                .replace_toggle
                .connect_toggled(move |toggle| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    let show = toggle.is_active() && !this.flags.read_only.get();
                    this.search_controls.replace_row.set_visible(show);
                    toggle.set_icon_name(if show {
                        "pan-down-symbolic"
                    } else {
                        "pan-end-symbolic"
                    });
                    if !show && this.search_bar.is_search_mode() {
                        this.search_entry.grab_focus();
                    }
                });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_controls
                .match_case
                .connect_toggled(move |toggle| {
                    if let Some(this) = weak.upgrade() {
                        this.search_controls
                            .settings
                            .set_case_sensitive(toggle.is_active());
                        this.update_match_count();
                        this.update_search_actions();
                        this.find_incremental();
                    }
                });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_controls
                .whole_word
                .connect_toggled(move |toggle| {
                    if let Some(this) = weak.upgrade() {
                        this.search_controls
                            .settings
                            .set_at_word_boundaries(toggle.is_active());
                        this.update_match_count();
                        this.update_search_actions();
                        this.find_incremental();
                    }
                });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_controls.replacement.connect_activate(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.replace_one();
                }
            });
        }
        {
            let replace_keys = gtk::EventControllerKey::new();
            replace_keys.set_name(Some("notediff-replace-entry-keys"));
            replace_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(this);
            replace_keys.connect_key_pressed(move |_, key, _, modifiers| {
                let active_mods = modifiers & gtk::accelerator_get_default_mod_mask();
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                    && active_mods == gdk::ModifierType::CONTROL_MASK
                    && let Some(this) = weak.upgrade()
                {
                    this.replace_all();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            this.search_controls
                .replacement
                .add_controller(replace_keys);
        }

        // Escape closes the bar from any focus target in the window. Bubble
        // phase lets the focused widget handle it first (for example the
        // search entry's own stop-search) and only closes when nothing else
        // did. Dialogs and popover menus keep their own Escape handling.
        let keys = gtk::EventControllerKey::new();
        keys.set_name(Some("notediff-search-keys"));
        keys.set_propagation_phase(gtk::PropagationPhase::Bubble);
        {
            let weak = Rc::downgrade(this);
            keys.connect_key_pressed(move |_, key, _, _| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if key != gdk::Key::Escape
                    || !this.search_bar.is_search_mode()
                    || this.window.visible_dialog().is_some()
                    || gtk::prelude::GtkWindowExt::focus(&this.window)
                        .is_some_and(|focus| focus.ancestor(gtk::Popover::static_type()).is_some())
                {
                    return glib::Propagation::Proceed;
                }
                this.search_bar.set_search_mode(false);
                this.editor_view.grab_focus();
                glib::Propagation::Stop
            });
        }
        this.window.add_controller(keys);
        {
            let weak = Rc::downgrade(this);
            this.stack.connect_visible_child_name_notify(move |_| {
                if let Some(this) = weak.upgrade() {
                    if this.diff_active() {
                        this.search_bar.set_search_mode(false);
                        this.search_controls.context.set_highlight(false);
                    }
                    this.update_search_actions();
                }
            });
        }
        add_action(window, this, "goto-line", |this| this.show_goto_line());
        sync_search_match_style(&this.search_controls.context);
        {
            let weak = Rc::downgrade(this);
            theme::on_change(move || {
                let Some(this) = weak.upgrade() else {
                    return false;
                };
                sync_search_match_style(&this.search_controls.context);
                true
            });
        }
        this.update_search_actions();
    }

    fn open_search_bar(&self, show_replace: bool) {
        if self.diff_active() || (show_replace && self.flags.read_only.get()) {
            return;
        }
        let was_open = self.search_bar.is_search_mode();
        let prepopulated = self.prepopulate_from_selection();
        let focus_replace =
            show_replace && was_open && !prepopulated && !self.search_entry.text().is_empty();

        self.search_controls.replace_toggle.set_active(show_replace);
        self.search_controls.replace_row.set_visible(show_replace);
        self.search_bar.set_search_mode(true);
        self.search_controls.context.set_highlight(true);

        if focus_replace {
            self.search_controls.replacement.grab_focus();
            self.search_controls.replacement.select_region(0, -1);
        } else {
            self.search_entry.grab_focus();
            self.search_entry.select_region(0, -1);
        }
        self.update_match_count();
        self.update_search_actions();
    }

    /// If the editor has a short single-line selection, copy it into the Find
    /// entry and record its start as the search anchor. Multi-line selections
    /// leave the existing query untouched.
    fn prepopulate_from_selection(&self) -> bool {
        let buffer = &self.editor_buffer;
        if let Some((start, end)) = buffer.selection_bounds() {
            let selected = buffer.text(&start, &end, false);
            if is_prepopulatable_selection(&selected) {
                self.search_controls.anchor_offset.set(start.offset());
                if self.search_entry.text().as_str() != selected.as_str() {
                    self.search_controls.skip_next_incremental_select.set(true);
                    self.search_entry.set_text(&selected);
                }
                return true;
            }
            self.search_controls.anchor_offset.set(start.offset());
            return false;
        }
        self.search_controls
            .anchor_offset
            .set(buffer.cursor_position());
        false
    }

    pub(super) fn update_search_actions(&self) {
        let in_editor = !self.diff_active();
        let editable = in_editor && !self.flags.read_only.get();
        let has_query = !self.search_entry.text().is_empty();
        // `occurrences_count()` is `-1` while scanning asynchronously; only
        // disable match actions once the scan confirms zero matches.
        let has_matches = has_query && self.search_controls.context.occurrences_count() != 0;

        for (name, enabled) in [
            ("find", in_editor),
            ("find-next", in_editor && has_matches),
            ("find-previous", in_editor && has_matches),
            ("replace", editable),
            ("replace-one", editable && has_matches),
            ("replace-all", editable && has_matches),
            ("goto-line", in_editor),
        ] {
            self.set_action_enabled(name, enabled);
        }
        self.search_controls.replace_toggle.set_sensitive(editable);
        if !editable && self.search_controls.replace_toggle.is_active() {
            self.search_controls.replace_toggle.set_active(false);
        }
        self.search_controls.replacement.set_sensitive(editable);
    }

    fn update_match_count(&self) {
        let has_query = !self.search_entry.text().is_empty();
        if !has_query || !self.search_bar.is_search_mode() {
            self.search_controls.match_count.set_visible(false);
            self.search_entry.remove_css_class(NO_MATCH_CLASS);
            return;
        }
        let total = self.search_controls.context.occurrences_count();
        if total < 0 {
            // Asynchronous scan still in progress.
            self.search_entry.remove_css_class(NO_MATCH_CLASS);
            return;
        }
        if total == 0 {
            self.search_controls.match_count.set_text("No matches");
            self.search_controls.match_count.set_visible(true);
            self.search_entry.add_css_class(NO_MATCH_CLASS);
            return;
        }
        self.search_entry.remove_css_class(NO_MATCH_CLASS);
        let position = self
            .editor_buffer
            .selection_bounds()
            .map(|(start, end)| {
                self.search_controls
                    .context
                    .occurrence_position(&start, &end)
            })
            .filter(|&pos| pos > 0);
        let label = format_match_count(position, total);
        self.search_controls.match_count.set_text(&label);
        self.search_controls.match_count.set_visible(true);
    }

    /// Replace only a selection that is exactly a match under the current
    /// search options. Otherwise select the next match without changing text.
    fn replace_one(&self) {
        if self.diff_active() || self.flags.read_only.get() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        let matched = buffer.selection_bounds().filter(|(start, end)| {
            self.search_controls
                .context
                .forward(start)
                .is_some_and(|(a, b, wrapped)| {
                    !wrapped && a.offset() == start.offset() && b.offset() == end.offset()
                })
        });
        if matched.is_some() {
            let replacement = tabs::convert_tabs(&self.search_controls.replacement.text());
            buffer.begin_user_action();
            buffer.delete_selection(true, true);
            buffer.insert_at_cursor(&replacement);
            buffer.end_user_action();
        }
        self.find_next();
    }

    /// Snapshot non-overlapping matches before editing, so replacement text is
    /// never searched again. Reverse edits keep the original character offsets.
    fn replace_all(&self) {
        if self.diff_active() || self.flags.read_only.get() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        // Convert before inserting so the buffer filter does not fire one
        // "Tabs were converted" toast per match.
        let replacement = tabs::convert_tabs(&self.search_controls.replacement.text());
        let mut matches = Vec::new();
        let mut iter = buffer.start_iter();
        while let Some((start, end, wrapped)) = self.search_controls.context.forward(&iter) {
            if wrapped || start.offset() < iter.offset() {
                break;
            }
            matches.push((start.offset(), end.offset()));
            if end.offset() <= iter.offset() {
                break;
            }
            iter = end;
        }
        if !matches.is_empty() {
            buffer.begin_user_action();
            for &(start, end) in matches.iter().rev() {
                let mut start = buffer.iter_at_offset(start);
                let mut end = buffer.iter_at_offset(end);
                buffer.delete(&mut start, &mut end);
                buffer.place_cursor(&start);
                buffer.insert_at_cursor(&replacement);
            }
            buffer.end_user_action();
            self.editor_view.scroll_mark_onscreen(&buffer.get_insert());
        }
        let count = matches.len();
        self.toast(
            ToastKind::Info,
            &format!(
                "Replaced {} {}",
                format_thousands(count),
                if count == 1 { "match" } else { "matches" }
            ),
        );
    }

    fn show_goto_line(self: &Rc<Self>) {
        if self.diff_active() {
            return;
        }

        let line_count = self.editor_buffer.line_count().max(1);
        let current_line = self
            .editor_buffer
            .iter_at_offset(self.editor_buffer.cursor_position())
            .line()
            + 1;

        let entry = gtk::Entry::new();
        entry.set_input_purpose(gtk::InputPurpose::Digits);
        entry.set_text(&current_line.to_string());
        entry.set_activates_default(true);

        let dialog = dialogs::build(
            "Go to Line",
            &format!("Enter a line number from 1 to {line_count}."),
            &[
                Response::new("cancel", "Cancel"),
                Response::suggested("go", "Go"),
            ],
            "go",
            "cancel",
        );
        dialog.set_extra_child(Some(&entry));
        dialog.set_focus(Some(&entry));

        let this = self.clone();
        dialogs::choose(&dialog, &self.window, move |response| {
            if response != "go" {
                return;
            }
            match parse_goto_line(&entry.text(), line_count) {
                Some(line) => this.go_to_line(line),
                None => {
                    this.toast(
                        ToastKind::Error,
                        &format!("Enter a line number from 1 to {line_count}"),
                    );
                }
            }
        });
    }

    /// Move the cursor to the start of a 1-based line, clamped to the buffer.
    fn go_to_line(&self, line: i32) {
        let line_count = self.editor_buffer.line_count().max(1);
        let index = line.clamp(1, line_count) - 1;
        if let Some(mut iter) = self.editor_buffer.iter_at_line(index) {
            self.editor_buffer.place_cursor(&iter);
            self.editor_view
                .scroll_to_iter(&mut iter, 0.1, true, 0.0, 0.5);
        }
        self.editor_view.grab_focus();
    }

    /// Incremental search while typing or toggling options: start from the
    /// recorded search anchor rather than jumping to line 1.
    fn find_incremental(&self) {
        if self.diff_active() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        let clamped = self
            .search_controls
            .anchor_offset
            .get()
            .clamp(0, buffer.char_count());
        let start = buffer.iter_at_offset(clamped);
        if let Some((match_start, match_end, _)) = self.search_controls.context.forward(&start) {
            self.select_match(match_start, match_end);
        }
        self.update_match_count();
    }

    pub(super) fn find_next(&self) {
        if self.diff_active() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        let start = buffer
            .selection_bounds()
            .map(|(_, end)| end)
            .unwrap_or_else(|| buffer.iter_at_offset(buffer.cursor_position()));
        if let Some((match_start, match_end, _)) = self.search_controls.context.forward(&start) {
            self.search_controls.anchor_offset.set(match_start.offset());
            self.select_match(match_start, match_end);
        }
        self.update_match_count();
    }

    pub(super) fn find_previous(&self) {
        if self.diff_active() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        let start = buffer
            .selection_bounds()
            .map(|(start, _)| start)
            .unwrap_or_else(|| buffer.iter_at_offset(buffer.cursor_position()));
        if let Some((match_start, match_end, _)) = self.search_controls.context.backward(&start) {
            self.search_controls.anchor_offset.set(match_start.offset());
            self.select_match(match_start, match_end);
        }
        self.update_match_count();
    }

    fn select_match(&self, match_start: gtk::TextIter, match_end: gtk::TextIter) {
        self.editor_buffer.select_range(&match_start, &match_end);
        let mut iter = match_start;
        self.editor_view
            .scroll_to_iter(&mut iter, 0.1, false, 0.0, 0.3);
    }
}

/// Ensure `SearchContext` always has a valid match style, even when a user
/// theme's `scheme.xml` omits `search-match`.
fn sync_search_match_style(context: &sourceview5::SearchContext) {
    let current = theme::current();
    let has_own = current
        .scheme
        .as_ref()
        .and_then(|scheme| scheme.style("search-match"))
        .is_some();
    if has_own {
        context.set_match_style(None);
        return;
    }
    let fallback_id = if current.dark { "dark" } else { "light" };
    let fallback = theme::catalog()
        .into_iter()
        .find(|entry| entry.id == fallback_id)
        .and_then(|entry| entry.scheme.as_ref()?.style("search-match"));
    context.set_match_style(fallback.as_ref());
}

fn is_prepopulatable_selection(text: &str) -> bool {
    !text.is_empty() && !textlines::has_break(text) && text.chars().count() <= MAX_PREPOPULATE_CHARS
}

fn format_match_count(position: Option<i32>, total: i32) -> String {
    let total_text = format_thousands(total.max(0) as usize);
    match position {
        Some(pos) if pos > 0 => format!("{} of {total_text}", format_thousands(pos as usize)),
        _ => format!(
            "{total_text} {}",
            if total == 1 { "match" } else { "matches" }
        ),
    }
}

/// Parse the Go to Line entry. Non-digits and zero are rejected; values beyond
/// the document, including numbers too large for `u64`, clamp to the last line.
fn parse_goto_line(text: &str, line_count: i32) -> Option<i32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let line = text.parse::<u64>().unwrap_or(u64::MAX);
    (line > 0).then(|| line.min(line_count.max(1) as u64) as i32)
}

#[cfg(test)]
mod tests {
    use super::{format_match_count, is_prepopulatable_selection, parse_goto_line};

    #[test]
    fn goto_line_parsing_clamps_and_rejects_invalid_input() {
        assert_eq!(parse_goto_line("3", 10), Some(3));
        assert_eq!(parse_goto_line(" 3 ", 10), Some(3));
        assert_eq!(parse_goto_line("10", 10), Some(10));
        assert_eq!(parse_goto_line("11", 10), Some(10));
        assert_eq!(parse_goto_line("999999999999999999999999", 10), Some(10));
        assert_eq!(parse_goto_line("0", 10), None);
        assert_eq!(parse_goto_line("-1", 10), None);
        assert_eq!(parse_goto_line("1.5", 10), None);
        assert_eq!(parse_goto_line("abc", 10), None);
        assert_eq!(parse_goto_line("", 10), None);
    }

    #[test]
    fn prepopulate_accepts_short_single_line_selections_only() {
        assert!(!is_prepopulatable_selection(""));
        assert!(is_prepopulatable_selection("hello world"));
        assert!(!is_prepopulatable_selection("line 1\nline 2"));
        assert!(!is_prepopulatable_selection("line 1\r\nline 2"));
        assert!(!is_prepopulatable_selection("line 1\u{2029}line 2"));
        assert!(is_prepopulatable_selection(&"a".repeat(256)));
        assert!(!is_prepopulatable_selection(&"a".repeat(257)));
    }

    #[test]
    fn match_count_formats_position_and_total() {
        assert_eq!(format_match_count(Some(3), 12), "3 of 12");
        assert_eq!(format_match_count(Some(1000), 2500), "1,000 of 2,500");
        assert_eq!(format_match_count(None, 1), "1 match");
        assert_eq!(format_match_count(None, 5), "5 matches");
    }
}
