//! Editing behaviour of the main window: the no-tab paste filter, the Tab
//! key, indentation editing, trailing-whitespace find and trim, EOL
//! conversion, word wrap and the read-only state.

use std::ops::RangeInclusive;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::variant::ToVariant;
use gtk::{gdk, gio, glib};

use crate::core::eol;
use crate::core::indent::{self, IndentError, IndentOperation};
use crate::core::insert::{self, InsertError};
use crate::core::tabs;
use crate::core::textlines;
use crate::core::textstats::{self, LineEnding};
use crate::core::whitespace;
use crate::ui::dialogs::{self, Response};
use crate::ui::toast::ToastKind;

use super::{AppWindow, add_action};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrimSide {
    Leading,
    Trailing,
}

impl AppWindow {
    /// Connect the editing signals and actions. Called once from
    /// `connect_signals`; every handler holds the window weakly.
    pub(super) fn connect_editing(this: &Rc<Self>) {
        // No-tab policy and line-ending consistency. Tabs in pasted, dropped
        // or otherwise inserted text become spaces. New line breaks match the
        // document's dominant ending, so Enter in a CRLF file inserts CRLF and
        // a pasted LF does not silently make the document mixed. The handler
        // replaces the insertion with a converted one; the nested insert has
        // nothing left to convert and passes straight through. The
        // programmatic-load guard bypasses both rules.
        {
            let weak = Rc::downgrade(this);
            this.editor_buffer
                .connect_insert_text(move |buffer, iter, text| {
                    let Some(this) = weak.upgrade() else {
                        return;
                    };
                    if this.flags.is_loading() || this.flags.in_undo_redo.get() {
                        return;
                    }
                    let target = this.doc.borrow().line_ending();
                    let convert_breaks = target != LineEnding::Mixed
                        && !this.flags.keep_inserted_line_endings.get()
                        && textlines::has_break(text);
                    let convert_tabs = tabs::contains_tab(text);
                    if !convert_breaks && !convert_tabs {
                        return;
                    }

                    let mut converted = text.to_string();
                    if convert_tabs {
                        converted = tabs::convert_tabs(&converted);
                    }
                    if convert_breaks {
                        converted = eol::convert_line_endings(&converted, target);
                    }
                    if converted == text {
                        return;
                    }
                    buffer.stop_signal_emission_by_name("insert-text");
                    buffer.insert(iter, &converted);
                    if convert_tabs {
                        this.toast(ToastKind::Info, "Tabs were converted to spaces");
                    }
                });
        }

        // Bypass the insert filter while GtkTextBuffer replays an undo or redo
        // step, then re-sync the document's preferred line ending so undoing an
        // EOL conversion restores both the buffer text and future Enter breaks.
        for signal in ["undo", "redo"] {
            let weak = Rc::downgrade(this);
            this.editor_buffer.connect_closure(
                signal,
                false,
                glib::closure_local!(move |_buffer: sourceview5::Buffer| {
                    if let Some(this) = weak.upgrade() {
                        this.flags.in_undo_redo.set(true);
                    }
                }),
            );
            let weak = Rc::downgrade(this);
            this.editor_buffer.connect_closure(
                signal,
                true,
                glib::closure_local!(move |_buffer: sourceview5::Buffer| {
                    if let Some(this) = weak.upgrade() {
                        this.flags.in_undo_redo.set(false);
                        let ending = textstats::detect_line_ending(&this.current_text());
                        this.doc.borrow_mut().set_line_ending(ending);
                    }
                }),
            );
        }

        // No-tab policy: the editor's Tab key never inserts a tab character.
        // The controller runs in the capture phase so GtkSourceView's own Tab
        // handling never sees the key.
        let tab_keys = gtk::EventControllerKey::new();
        tab_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(this);
            tab_keys.connect_key_pressed(move |_, keyval, _, state| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let other_modifiers = gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK
                    | gdk::ModifierType::META_MASK;
                if state.intersects(other_modifiers) || !this.editor_view.is_editable() {
                    return glib::Propagation::Proceed;
                }
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                match keyval {
                    gdk::Key::Tab => this.press_tab(shift),
                    gdk::Key::ISO_Left_Tab => this.press_tab(true),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
        }
        this.editor_view.add_controller(tab_keys);

        let window = &this.window;
        let wrap_enabled = this.editor_view.wrap_mode() != gtk::WrapMode::None;
        let word_wrap =
            gio::SimpleAction::new_stateful("word-wrap", None, &wrap_enabled.to_variant());
        let weak = Rc::downgrade(this);
        // GIO toggles boolean actions by requesting a state change on activation.
        word_wrap.connect_change_state(move |action, state| {
            let (Some(this), Some(state)) = (weak.upgrade(), state) else {
                return;
            };
            let Some(enabled) = state.get::<bool>() else {
                return;
            };
            this.editor_view.set_wrap_mode(if enabled {
                gtk::WrapMode::WordChar
            } else {
                gtk::WrapMode::None
            });
            action.set_state(state);
        });
        window.add_action(&word_wrap);
        this.stack.connect_visible_child_name_notify(move |stack| {
            word_wrap.set_enabled(stack.visible_child_name().as_deref() == Some("editor"));
        });

        // Only set_read_only changes the state, including after a refused unlock.
        let read_only_action =
            gio::SimpleAction::new_stateful("read-only", None, &false.to_variant());
        let weak = Rc::downgrade(this);
        read_only_action.connect_change_state(move |_, state| {
            let (Some(this), Some(wanted)) =
                (weak.upgrade(), state.and_then(|state| state.get::<bool>()))
            else {
                return;
            };
            this.request_read_only(wanted);
        });
        window.add_action(&read_only_action);
        this.stack.connect_visible_child_name_notify(move |stack| {
            read_only_action.set_enabled(stack.visible_child_name().as_deref() == Some("editor"));
        });

        add_action(window, this, "convert-eol-crlf", |this| {
            this.convert_line_endings(LineEnding::CrLf)
        });
        add_action(window, this, "convert-eol-lf", |this| {
            this.convert_line_endings(LineEnding::Lf)
        });
        add_action(window, this, "indent-increase-include-empty", |this| {
            this.edit_indentation(IndentOperation::IncreaseIncludeEmpty)
        });
        add_action(window, this, "indent-decrease-no-flattening", |this| {
            this.edit_indentation(IndentOperation::DecreaseNoFlattening)
        });
        add_action(window, this, "indent-double", |this| {
            this.edit_indentation(IndentOperation::Double)
        });
        add_action(window, this, "indent-halve", |this| {
            this.edit_indentation(IndentOperation::Halve)
        });
        add_action(window, this, "find-next-trailing-whitespace", |this| {
            this.find_next_trailing_whitespace()
        });
        add_action(window, this, "find-next-non-empty-blank-line", |this| {
            this.find_next_non_empty_blank_line()
        });
        add_action(window, this, "trim-leading-whitespace", |this| {
            this.trim_leading_whitespace()
        });
        add_action(window, this, "trim-trailing-whitespace", |this| {
            this.trim_trailing_whitespace()
        });
        add_action(window, this, "trim-blank-lines-remove", |this| {
            this.trim_blank_lines_remove()
        });
        add_action(window, this, "trim-blank-lines-strip", |this| {
            this.trim_blank_lines_strip()
        });
        // Sensitivity-only action for the Trim Whitespace submenu heading.
        add_action(window, this, "trim-whitespace-menu", |_| {});

        add_action(window, this, "insert-path", |this| {
            this.request_insert_path()
        });
        add_action(window, this, "insert-folder-path", |this| {
            this.request_insert_folder_path()
        });
        add_action(window, this, "insert-file", |this| {
            this.request_insert_file()
        });
    }

    /// Lock or unlock the editor for the current document.
    pub(super) fn set_read_only(&self, read_only: bool) {
        self.flags.read_only.set(read_only);
        self.editor_view.set_editable(!read_only);
        if let Some(action) = self.lookup_simple_action("read-only") {
            action.set_state(&read_only.to_variant());
        }
        self.set_edit_actions_enabled(!read_only);
        self.update_search_actions();
        self.update_status();
        self.update_title();
    }

    /// Enable or disable the context-menu actions that edit the text
    /// (the indentation operations, Insert Path and Add Text from File).
    fn set_edit_actions_enabled(&self, enabled: bool) {
        for name in [
            "indent-increase-include-empty",
            "indent-decrease-no-flattening",
            "indent-double",
            "indent-halve",
            "trim-leading-whitespace",
            "trim-trailing-whitespace",
            "trim-blank-lines-remove",
            "trim-blank-lines-strip",
            "trim-whitespace-menu",
            "insert-path",
            "insert-folder-path",
            "insert-file",
        ] {
            self.set_action_enabled(name, enabled);
        }
    }

    /// Insert one space at the cursor. In overwrite mode the space replaces
    /// the next character on the line, as typed text would, unless the cursor
    /// is at the end of the line. One undo step.
    fn insert_space_at_cursor(&self) {
        let buffer = &self.editor_buffer;
        buffer.begin_user_action();
        if self.editor_view.overwrites() {
            let mut start = buffer.iter_at_mark(&buffer.get_insert());
            if !start.ends_line() {
                let mut end = start;
                end.forward_char();
                buffer.delete(&mut start, &mut end);
            }
        }
        buffer.insert_at_cursor(" ");
        buffer.end_user_action();
    }

    /// The editor's Tab key under the no-tab policy. Tab never inserts a tab
    /// character. With no selection it inserts one space (overwriting the next
    /// character in overwrite mode). With a selection it indents every touched
    /// line by one space. Shift+Tab removes one leading space from the touched
    /// lines, or from the cursor's line when nothing is selected. Line edits
    /// form one undo step.
    pub(super) fn press_tab(&self, outdent: bool) {
        let buffer = &self.editor_buffer;
        let selection = self.selection_offsets();

        if selection.is_none() && !outdent {
            self.insert_space_at_cursor();
        } else {
            let text = self.current_text();
            let lines = selection
                .and_then(|(start, end)| textstats::selection_stats(&text, start, end))
                .map(|selection| selection.line_range())
                .unwrap_or_else(|| {
                    let line = buffer
                        .iter_at_offset(buffer.cursor_position())
                        .line()
                        .max(0) as usize;
                    line..=line
                });

            buffer.begin_user_action();
            for line in lines {
                self.shift_line(line, outdent);
            }
            buffer.end_user_action();
        }

        self.editor_view.scroll_mark_onscreen(&buffer.get_insert());
    }

    /// Add or remove one leading space on a 0-based line. Blank lines are not
    /// indented, so no trailing-whitespace-only lines appear.
    fn shift_line(&self, line: usize, outdent: bool) {
        let buffer = &self.editor_buffer;
        let Some(mut start) = buffer.iter_at_line(line as i32) else {
            return;
        };
        if outdent {
            if start.char() == ' ' {
                let mut end = start;
                end.forward_char();
                buffer.delete(&mut start, &mut end);
            }
        } else if !start.ends_line() {
            buffer.insert(&mut start, " ");
        }
    }

    /// Convert every line ending in the editor to `target`. It is one undo
    /// step and leaves the document dirty. The cursor keeps its line and
    /// column. Does nothing in Diff Mode and is refused for read-only
    /// documents.
    pub(super) fn convert_line_endings(&self, target: LineEnding) {
        if self.diff_active() {
            return;
        }

        // The status-bar EOL popover is anchored to a label, which cannot take
        // focus. When it closes, GTK would otherwise fall back to focusing the
        // new toast's close button, and AdwToast deliberately suspends its
        // timeout while it has keyboard focus. Return focus to the editor
        // before showing any toast, so the toast can expire normally.
        self.editor_view.grab_focus();

        if self.flags.read_only.get() {
            self.toast(
                ToastKind::Warning,
                "This document is read-only, so its line endings cannot be converted",
            );
            return;
        }

        let text = self.current_text();
        let converted = eol::convert_line_endings(&text, target);
        if converted == text {
            if textlines::has_break(&text) {
                self.toast(
                    ToastKind::Info,
                    &format!("Line endings are already {}", target.label()),
                );
            } else {
                self.toast(
                    ToastKind::Info,
                    "The document has no line breaks to convert",
                );
            }
            return;
        }

        let buffer = &self.editor_buffer;
        let cursor = buffer.iter_at_offset(buffer.cursor_position());
        let (line, column) = (cursor.line(), cursor.line_offset());

        // Update the preferred ending before touching the buffer: the insert
        // filter sees the converted text and must not normalize it back.
        self.doc.borrow_mut().set_line_ending(target);
        buffer.begin_user_action();
        let mut start = buffer.start_iter();
        let mut end = buffer.end_iter();
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, &converted);
        buffer.end_user_action();

        let restored = buffer
            .iter_at_line_offset(line, column)
            .unwrap_or_else(|| buffer.end_iter());
        buffer.place_cursor(&restored);
        self.editor_view.scroll_mark_onscreen(&buffer.get_insert());
        self.toast(
            ToastKind::Success,
            &format!("Converted line endings to {}", target.label()),
        );
    }

    /// Run one indentation operation on the lines the selection touches, or
    /// on the whole document when nothing is selected.
    ///
    /// Double and Halve abort with a toast (changing nothing) when the
    /// operation is not possible; the Increase and Decrease variants never
    /// report success or failure. The edits are one undo step and do nothing
    /// in Diff Mode or for read-only documents.
    pub(super) fn edit_indentation(&self, operation: IndentOperation) {
        if self.diff_active() || self.flags.read_only.get() {
            return;
        }

        let text = self.current_text();
        let changes = match indent::plan(&text, operation, self.target_lines(&text)) {
            Ok(changes) => changes,
            Err(err @ (IndentError::NothingToScale | IndentError::OddIndent { .. })) => {
                self.toast(ToastKind::Warning, &err.message(operation));
                return;
            }
        };
        if changes.is_empty() {
            return;
        }

        let buffer = &self.editor_buffer;
        buffer.begin_user_action();
        for change in &changes {
            // Line numbers do not shift: only spaces are added or removed.
            let Some(mut start) = buffer.iter_at_line(change.line as i32) else {
                continue;
            };
            if change.new > change.old {
                buffer.insert(&mut start, &" ".repeat(change.new - change.old));
            } else {
                let mut end = start;
                end.forward_chars((change.old - change.new) as i32);
                buffer.delete(&mut start, &mut end);
            }
        }
        buffer.end_user_action();
    }

    /// 0-based inclusive lines the selection touches, or `None` for the whole
    /// document.
    fn target_lines(&self, text: &str) -> Option<RangeInclusive<usize>> {
        self.selection_offsets()
            .and_then(|(start, end)| textstats::selection_stats(text, start, end))
            .map(|selection| selection.line_range())
    }

    /// Remove leading ASCII spaces and tabs from the target lines. The edit is
    /// one undo step and reports the number of removed characters. Does
    /// nothing in Diff Mode or for read-only documents.
    pub(super) fn trim_leading_whitespace(&self) {
        self.trim_whitespace(TrimSide::Leading);
    }

    /// Remove trailing ASCII spaces and tabs from the target lines. The edit is
    /// one undo step and reports the number of removed characters. Does
    /// nothing in Diff Mode or for read-only documents.
    pub(super) fn trim_trailing_whitespace(&self) {
        self.trim_whitespace(TrimSide::Trailing);
    }

    fn trim_whitespace(&self, side: TrimSide) {
        if self.diff_active() || self.flags.read_only.get() {
            return;
        }

        let text = self.current_text();
        let lines = self.target_lines(&text);
        let changes = match side {
            TrimSide::Leading => whitespace::trim_leading_plan(&text, lines),
            TrimSide::Trailing => whitespace::trim_trailing_plan(&text, lines),
        };
        if changes.is_empty() {
            let message = match side {
                TrimSide::Leading => "No leading whitespace to remove",
                TrimSide::Trailing => "No trailing whitespace to remove",
            };
            self.toast(ToastKind::Info, message);
            return;
        }
        let removed: usize = changes.iter().map(|change| change.len).sum();
        self.apply_trim_changes(&changes);

        let side_name = match side {
            TrimSide::Leading => "leading",
            TrimSide::Trailing => "trailing",
        };
        let message = if removed == 1 {
            format!("1 {side_name} whitespace character was removed")
        } else {
            format!(
                "{} {side_name} whitespace characters were removed",
                textstats::format_thousands(removed)
            )
        };
        self.toast(ToastKind::Success, &message);
    }

    /// Remove every blank target line. The edit is one undo step and does
    /// nothing in Diff Mode or for read-only documents.
    pub(super) fn trim_blank_lines_remove(&self) {
        if self.diff_active() || self.flags.read_only.get() {
            return;
        }

        let text = self.current_text();
        let lines = whitespace::blank_line_remove_plan(&text, self.target_lines(&text));
        if lines.is_empty() {
            self.toast(ToastKind::Info, "No blank lines to remove");
            return;
        }
        let count = lines.len();

        let buffer = &self.editor_buffer;
        buffer.begin_user_action();
        for line in lines.iter().rev() {
            let Some(mut start) = buffer.iter_at_line(*line as i32) else {
                continue;
            };
            let mut end = buffer
                .iter_at_line(*line as i32 + 1)
                .unwrap_or_else(|| buffer.end_iter());
            buffer.delete(&mut start, &mut end);
        }
        buffer.end_user_action();

        let message = if count == 1 {
            "1 blank line was removed".to_string()
        } else {
            format!(
                "{} blank lines were removed",
                textstats::format_thousands(count)
            )
        };
        self.toast(ToastKind::Success, &message);
    }

    /// Remove all spaces and tabs from blank target lines while keeping the
    /// lines. The edit is one undo step and does nothing in Diff Mode or for
    /// read-only documents.
    pub(super) fn trim_blank_lines_strip(&self) {
        if self.diff_active() || self.flags.read_only.get() {
            return;
        }

        let text = self.current_text();
        let changes = whitespace::blank_line_strip_plan(&text, self.target_lines(&text));
        if changes.is_empty() {
            self.toast(ToastKind::Info, "No blank lines to strip");
            return;
        }
        let lines = changes.len();
        let characters: usize = changes.iter().map(|change| change.len).sum();
        self.apply_trim_changes(&changes);

        let character_text = if characters == 1 {
            "whitespace character"
        } else {
            "whitespace characters"
        };
        let message = if lines == 1 {
            format!(
                "1 blank line was stripped of {} {character_text}",
                textstats::format_thousands(characters)
            )
        } else {
            format!(
                "{} blank lines were stripped of {} {character_text}",
                textstats::format_thousands(lines),
                textstats::format_thousands(characters)
            )
        };
        self.toast(ToastKind::Success, &message);
    }

    fn apply_trim_changes(&self, changes: &[whitespace::TrimChange]) {
        let buffer = &self.editor_buffer;
        buffer.begin_user_action();
        for change in changes.iter().rev() {
            let Some(mut start) =
                buffer.iter_at_line_offset(change.line as i32, change.start as i32)
            else {
                continue;
            };
            let mut end = start;
            end.forward_chars(change.len as i32);
            buffer.delete(&mut start, &mut end);
        }
        buffer.end_user_action();
    }

    /// Refresh context-menu actions whose availability depends on the current
    /// selection. Called when the editor context menu is about to open.
    pub(super) fn update_context_menu_actions(&self) {
        let selected = self.selection_offsets();
        let (has_leading, has_trailing, has_blank_remove, has_blank_strip) =
            self.status_state.measure(
                || self.buffer_text(),
                |text, stats| match selected {
                    Some((start, end)) => textstats::selection_stats(text, start, end).map_or(
                        (false, false, false, false),
                        |selection| {
                            let lines = selection.line_range();
                            (
                                whitespace::has_leading_in(text, Some(lines.clone())),
                                whitespace::has_trailing_in(text, Some(lines.clone())),
                                whitespace::has_blank_line_remove(text, Some(lines.clone())),
                                whitespace::has_blank_line_strip(text, Some(lines)),
                            )
                        },
                    ),
                    None => (
                        whitespace::has_leading_in(text, None),
                        stats.has_trailing_whitespace,
                        whitespace::has_blank_line_remove(text, None),
                        whitespace::has_blank_line_strip(text, None),
                    ),
                },
            );
        let editable = !self.diff_active() && !self.flags.read_only.get();
        let leading = has_leading && editable;
        let trailing = has_trailing && editable;
        let blank_remove = has_blank_remove && editable;
        let blank_strip = has_blank_strip && editable;
        for (name, enabled) in [
            ("trim-leading-whitespace", leading),
            ("trim-trailing-whitespace", trailing),
            ("trim-blank-lines-remove", blank_remove),
            ("trim-blank-lines-strip", blank_strip),
            (
                "trim-whitespace-menu",
                leading || trailing || blank_remove || blank_strip,
            ),
        ] {
            self.set_action_enabled(name, enabled);
        }
    }

    /// Select the next run of trailing spaces or tabs and scroll it onscreen.
    /// The search starts at the selection end, or at the cursor when there is
    /// no selection, and wraps at the end of the document. The insert mark is
    /// left at the end of the run, so repeated finds advance. Works for
    /// read-only documents and does nothing in Diff Mode.
    pub(super) fn find_next_trailing_whitespace(&self) {
        self.select_next_whitespace(whitespace::next_trailing);
    }

    /// Select the next non-empty blank line's whitespace, scroll it onscreen
    /// and wrap at the end. Works for read-only documents and does nothing in
    /// Diff Mode.
    pub(super) fn find_next_non_empty_blank_line(&self) {
        self.select_next_whitespace(whitespace::next_non_empty_blank);
    }

    fn select_next_whitespace(&self, find: fn(&str, usize) -> Option<std::ops::Range<usize>>) {
        if self.diff_active() {
            return;
        }

        let text = self.current_text();
        let from = self.selection_offsets().map_or_else(
            || self.editor_buffer.cursor_position().max(0) as usize,
            |(_, end)| end,
        );
        let Some(range) = find(&text, from) else {
            return;
        };

        let buffer = &self.editor_buffer;
        let start = buffer.iter_at_offset(range.start as i32);
        let end = buffer.iter_at_offset(range.end as i32);
        // The insert mark goes at the end so repeated finds advance.
        buffer.select_range(&end, &start);
        self.editor_view.scroll_mark_onscreen(&buffer.get_insert());
        self.editor_view.grab_focus();
    }

    /// True when text may be inserted now. Does nothing (silently) in Diff
    /// Mode and refuses, with a toast, for a read-only document.
    fn can_insert_text(&self) -> bool {
        if self.diff_active() {
            return false;
        }
        if self.flags.read_only.get() {
            self.toast(
                ToastKind::Warning,
                "This document is read-only, so text cannot be inserted",
            );
            return false;
        }
        true
    }

    fn build_insert_dialog(&self, title: &str) -> Option<gtk::FileDialog> {
        if !self.can_insert_text() {
            return None;
        }
        let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
        if let Some(folder) = self
            .path_for_snapshot()
            .and_then(|path| path.parent().map(Path::to_path_buf))
        {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        Some(dialog)
    }

    fn finish_insert_dialog(
        this: &Rc<Self>,
        result: Result<gio::File, glib::Error>,
        apply: impl FnOnce(&Rc<Self>, &Path),
    ) {
        if let Ok(file) = result
            && let Some(path) = file.path()
        {
            apply(this, &path);
        } else {
            this.editor_view.grab_focus();
        }
    }

    /// File Path: let the user pick a file, then insert its absolute path at
    /// the cursor (see `insert_path`).
    pub(super) fn request_insert_path(self: &Rc<Self>) {
        let Some(dialog) = self.build_insert_dialog("Insert File Path") else {
            return;
        };
        let this = self.clone();
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| Self::finish_insert_dialog(&this, result, |this, p| this.insert_path(p)),
        );
    }

    /// Directory Path: let the user pick a folder, then insert its absolute
    /// path at the cursor (see `insert_path`).
    pub(super) fn request_insert_folder_path(self: &Rc<Self>) {
        let Some(dialog) = self.build_insert_dialog("Insert Directory Path") else {
            return;
        };
        let this = self.clone();
        dialog.select_folder(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| Self::finish_insert_dialog(&this, result, |this, p| this.insert_path(p)),
        );
    }

    /// Add Text from File: let the user pick a file, then insert its whole
    /// contents at the cursor (see `insert_file`).
    pub(super) fn request_insert_file(self: &Rc<Self>) {
        let Some(dialog) = self.build_insert_dialog("Add Text from File") else {
            return;
        };
        let this = self.clone();
        dialog.open(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                Self::finish_insert_dialog(&this, result, |this, p| this.insert_file_checked(p))
            },
        );
    }

    /// Validate the chosen file and, for a large one, ask before inserting.
    fn insert_file_checked(self: &Rc<Self>, path: &Path) {
        // The state may have changed while the chooser was open.
        if !self.can_insert_text() {
            return;
        }
        let bytes = match insert::inspect(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.show_error("Could not insert file", &err.to_string());
                return;
            }
        };
        if !insert::needs_confirmation(bytes) {
            self.insert_file(path);
            return;
        }

        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("The file")
            .to_string();
        let this = self.clone();
        let path = path.to_path_buf();
        dialogs::ask(
            &self.window,
            "Insert a large file?",
            &format!(
                "{name} is {} bytes. Inserting it may make the editor slow.",
                textstats::format_thousands(bytes as usize)
            ),
            &[
                Response::new("cancel", "Cancel"),
                Response::suggested("insert", "Insert"),
            ],
            "cancel",
            "cancel",
            move |response| {
                if response == "insert" {
                    this.insert_file(&path);
                }
            },
        );
    }

    /// Insert the whole contents of `path` at the cursor, like a paste: a
    /// selection is replaced, the inserted text is selected and the edit is
    /// one undo step. A leading BOM is dropped, line endings are matched to
    /// the document, and tabs are converted by the insert filter. Failures
    /// are reported and change nothing.
    pub(super) fn insert_file(&self, path: &Path) {
        if !self.can_insert_text() {
            return;
        }

        let loaded = match insert::load(path) {
            Ok(text) => text,
            Err(
                err @ (InsertError::NotAFile | InsertError::TooLarge { .. } | InsertError::Read(_)),
            ) => {
                self.show_error("Could not insert file", &err.to_string());
                return;
            }
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        if loaded.is_empty() {
            self.editor_view.grab_focus();
            self.toast(
                ToastKind::Info,
                &format!("{name} is empty, so nothing was inserted"),
            );
            return;
        }

        let text = insert::match_line_endings(&loaded, &self.current_text());
        self.flags
            .with_inserted_line_endings(|| self.insert_text_like_paste(&text));
        self.toast(ToastKind::Success, &format!("Inserted {name}"));
    }

    /// Insert the selected file's absolute path at the cursor. The path is
    /// plain text: no quoting, canonicalization or file-content checks.
    pub(super) fn insert_path(&self, path: &Path) {
        if !self.can_insert_text() {
            return;
        }
        self.insert_text_like_paste(&path.display().to_string());
    }

    /// Insert `text` like a paste as one undo step: replace the selection,
    /// select the inserted text, scroll it onscreen and focus the editor.
    fn insert_text_like_paste(&self, text: &str) {
        let buffer = &self.editor_buffer;
        let insert_mark = {
            buffer.begin_user_action();
            buffer.delete_selection(true, true);
            let insert_mark = buffer.get_insert();
            let start_offset = buffer.iter_at_mark(&insert_mark).offset();
            buffer.insert_at_cursor(text);

            // Resolve both ends after the edit. The insert mark follows the
            // actual insertion even when the tab filter rewrites the text.
            let start = buffer.iter_at_offset(start_offset);
            let end = buffer.iter_at_mark(&insert_mark);
            buffer.select_range(&end, &start);
            buffer.end_user_action();
            insert_mark
        };

        self.editor_view.scroll_mark_onscreen(&insert_mark);
        self.editor_view.grab_focus();
    }
}
