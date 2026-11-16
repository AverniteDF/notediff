//! Editor Find, Replace and Go to Line.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::core::tabs;
use crate::ui::dialogs::{self, Response};
use crate::ui::toast::ToastKind;

use super::{AppWindow, add_action};

pub(super) struct SearchControls {
    pub(super) replacement: gtk::Entry,
    pub(super) match_case: gtk::CheckButton,
    replace_row: gtk::Box,
}

pub(super) fn build_search_bar() -> (gtk::SearchBar, gtk::SearchEntry, SearchControls) {
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Find in document"));
    search_entry.set_hexpand(true);
    let match_case = gtk::CheckButton::with_label("Match Case");
    let find_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    find_row.append(&search_entry);
    find_row.append(&match_case);

    let replacement = gtk::Entry::new();
    replacement.set_placeholder_text(Some("Replace with"));
    replacement.set_hexpand(true);
    let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    replace_row.append(&replacement);
    for (label, action) in [
        ("Replace", "win.replace-one"),
        ("Replace All", "win.replace-all"),
    ] {
        let button = gtk::Button::with_label(label);
        button.set_action_name(Some(action));
        replace_row.append(&button);
    }
    replace_row.set_visible(false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&find_row);
    content.append(&replace_row);
    let search_bar = gtk::SearchBar::new();
    search_bar.set_child(Some(&content));
    search_bar.connect_entry(&search_entry);
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
            replace_row,
        },
    )
}

impl AppWindow {
    pub(super) fn connect_search(this: &Rc<Self>) {
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_changed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.update_search_actions();
                }
            });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_search_changed(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.update_search_actions();
                    this.find(true);
                }
            });
        }
        {
            let weak = Rc::downgrade(this);
            this.search_entry.connect_activate(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.find(false);
                }
            });
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
                    if bar.is_search_mode()
                        && let Some(this) = weak.upgrade()
                    {
                        this.search_entry.grab_focus();
                    }
                });
        }

        let window = &this.window;
        add_action(window, this, "find", |this| {
            this.search_controls.replace_row.set_visible(false);
            this.search_bar.set_search_mode(true);
            this.search_entry.grab_focus();
            this.search_entry.select_region(0, -1);
        });
        add_action(window, this, "replace", |this| {
            if this.diff_active() || this.flags.read_only.get() {
                return;
            }
            this.search_controls.replace_row.set_visible(true);
            this.search_bar.set_search_mode(true);
            this.search_entry.grab_focus();
            this.search_entry.select_region(0, -1);
        });
        add_action(window, this, "replace-one", |this| this.replace_one());
        add_action(window, this, "replace-all", |this| this.replace_all());
        {
            let weak = Rc::downgrade(this);
            this.search_controls.match_case.connect_toggled(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.find(true);
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
        // SearchBar connects Escape handling to the SearchEntry, not to the
        // plain replacement Entry. Catch it before that entry consumes it and
        // reuse Find's stop-search handler to close the bar and restore focus.
        let replacement_keys = gtk::EventControllerKey::new();
        replacement_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(this);
            replacement_keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape
                    && let Some(this) = weak.upgrade()
                    && this.search_bar.is_search_mode()
                {
                    this.search_entry.emit_by_name::<()>("stop-search", &[]);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        this.search_controls
            .replacement
            .add_controller(replacement_keys);
        {
            let weak = Rc::downgrade(this);
            this.stack.connect_visible_child_name_notify(move |_| {
                if let Some(this) = weak.upgrade() {
                    if this.diff_active() {
                        this.search_bar.set_search_mode(false);
                    }
                    this.update_search_actions();
                }
            });
        }
        add_action(window, this, "goto-line", |this| this.show_goto_line());
        this.update_search_actions();
    }

    pub(super) fn update_search_actions(&self) {
        let editable = !self.diff_active() && !self.flags.read_only.get();
        let has_query = !self.search_entry.text().is_empty();
        for (name, enabled) in [
            ("find", !self.diff_active()),
            ("replace", editable),
            ("replace-one", editable && has_query),
            ("replace-all", editable && has_query),
            ("goto-line", !self.diff_active()),
        ] {
            if let Some(action) = self
                .window
                .lookup_action(name)
                .and_then(|action| action.downcast::<gio::SimpleAction>().ok())
            {
                action.set_enabled(enabled);
            }
        }
        self.search_controls.replacement.set_sensitive(editable);
    }

    fn search_flags(&self) -> gtk::TextSearchFlags {
        if self.search_controls.match_case.is_active() {
            gtk::TextSearchFlags::VISIBLE_ONLY
        } else {
            gtk::TextSearchFlags::VISIBLE_ONLY | gtk::TextSearchFlags::CASE_INSENSITIVE
        }
    }

    /// Replace only a selection that is exactly a match under the current
    /// search options. Otherwise select the next match without changing text.
    fn replace_one(&self) {
        if self.diff_active() || self.flags.read_only.get() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        let matched = buffer.selection_bounds().filter(|(start, end)| {
            start
                .forward_search(&self.search_entry.text(), self.search_flags(), Some(end))
                .is_some_and(|(a, b)| a.offset() == start.offset() && b.offset() == end.offset())
        });
        if matched.is_some() {
            let replacement = tabs::convert_tabs(&self.search_controls.replacement.text());
            buffer.begin_user_action();
            buffer.delete_selection(true, true);
            buffer.insert_at_cursor(&replacement);
            buffer.end_user_action();
        }
        self.find(false);
    }

    /// Snapshot non-overlapping matches before editing, so replacement text is
    /// never searched again. Reverse edits keep the original character offsets.
    fn replace_all(&self) {
        if self.diff_active() || self.flags.read_only.get() || self.search_entry.text().is_empty() {
            return;
        }
        let buffer = &self.editor_buffer;
        let needle = self.search_entry.text();
        // Convert before inserting so the buffer filter does not fire one
        // "Tabs were converted" toast per match.
        let replacement = tabs::convert_tabs(&self.search_controls.replacement.text());
        let mut matches = Vec::new();
        let mut iter = buffer.start_iter();
        while let Some((start, end)) = iter.forward_search(&needle, self.search_flags(), None) {
            matches.push((start.offset(), end.offset()));
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
                "Replaced {count} {}",
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
            match entry.text().trim().parse::<i32>() {
                Ok(line) => this.go_to_line(line),
                Err(_) => {
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

    fn find(&self, from_start: bool) {
        let needle = self.search_entry.text().to_string();
        if self.diff_active() || needle.is_empty() {
            return;
        }

        let buffer = &self.editor_buffer;
        let flags = self.search_flags();

        let start = if from_start {
            buffer.start_iter()
        } else {
            buffer
                .selection_bounds()
                .map(|(_, end)| end)
                .unwrap_or_else(|| buffer.iter_at_offset(buffer.cursor_position()))
        };

        let found = start
            .forward_search(&needle, flags, None::<&gtk::TextIter>)
            .or_else(|| {
                buffer
                    .start_iter()
                    .forward_search(&needle, flags, None::<&gtk::TextIter>)
            });

        if let Some((match_start, match_end)) = found {
            buffer.select_range(&match_start, &match_end);
            let mut iter = match_start;
            self.editor_view
                .scroll_to_iter(&mut iter, 0.1, false, 0.0, 0.3);
        }
    }
}
