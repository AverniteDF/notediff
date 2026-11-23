//! The status bar shown under the editor and under each diff pane.
//!
//! Presentation only: the numbers come from `core::textstats`.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

use crate::core::textstats::{CursorPosition, SelectionStats, TextStats};

const ENCODING_LABEL: &str = "UTF-8";
pub(super) const READ_ONLY_WARNING_CLASS: &str = "notediff-read-only-warning";

#[derive(Default)]
struct WarningTimer(RefCell<Option<glib::SourceId>>);

impl WarningTimer {
    fn cancel(&self) {
        if let Some(source) = self.0.take() {
            source.remove();
        }
    }
}

struct ModeMenu {
    popover: gtk::PopoverMenu,
    timer: Rc<WarningTimer>,
}

impl Drop for ModeMenu {
    fn drop(&mut self) {
        self.timer.cancel();
    }
}

/// What the editor status dot is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DotState {
    Clean,
    Trailing,
    Blank,
}

/// Red (non-empty blank line) takes precedence unless Alt is held while
/// trailing whitespace also exists.
fn dot_state(trailing: bool, blank: bool, alt: bool) -> DotState {
    if blank && !(trailing && alt) {
        DotState::Blank
    } else if trailing {
        DotState::Trailing
    } else {
        DotState::Clean
    }
}

/// What the right-most field of the editor bar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    Insert,
    Overwrite,
    /// The document was opened read-only (for example, it contains tabs).
    ReadOnly,
}

impl EditMode {
    fn label(self) -> &'static str {
        match self {
            EditMode::Insert => "INS",
            EditMode::Overwrite => "OVR",
            EditMode::ReadOnly => "RO",
        }
    }
}

pub struct StatusBar {
    pub root: gtk::Box,
    stats: gtk::Label,
    /// The last measurement applied to the widgets, so cursor and selection
    /// moves that leave the text unchanged do not repeat label or dot updates.
    last_stats: Cell<Option<TextStats>>,
    /// Editor bar only.
    cursor: Option<gtk::Label>,
    /// Editor bar only: hidden until the first edit.
    last_edit_separator: Option<gtk::Separator>,
    last_edit: Option<gtk::Label>,
    /// Editor bar only: green when clean, yellow for trailing whitespace and
    /// red for non-empty blank lines.
    trailing: Option<gtk::Label>,
    dot_state: Rc<Cell<DotState>>,
    alt_held: Cell<bool>,
    ending: gtk::Label,
    /// Editor bar only.
    mode: Option<gtk::Label>,
    mode_menu: OnceCell<ModeMenu>,
}

impl StatusBar {
    /// Full bar for the editor:
    /// `bytes chars lines | Ln Col Pos | last edit ... | ● | line ending | UTF-8 | INS`.
    pub fn editor() -> Self {
        Self::build(true)
    }

    /// Reduced bar for a read-only diff pane:
    /// `bytes chars lines ... line ending | UTF-8`.
    pub fn pane() -> Self {
        Self::build(false)
    }

    fn build(editor: bool) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("status-bar");

        let stats = field("");
        root.append(&stats);

        let (cursor, last_edit_separator, last_edit) = if editor {
            root.append(&separator());
            let label = field("");
            root.append(&label);
            let last_edit_separator = separator();
            last_edit_separator.set_visible(false);
            root.append(&last_edit_separator);
            let last_edit = field("");
            last_edit.set_visible(false);
            root.append(&last_edit);
            (Some(label), Some(last_edit_separator), Some(last_edit))
        } else {
            (None, None, None)
        };

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        root.append(&spacer);

        let trailing = if editor {
            let label = field("●");
            label.add_css_class("trailing-dot");
            label.add_css_class("trailing-clean");
            label.set_tooltip_text(Some("No trailing whitespace"));
            root.append(&label);
            root.append(&separator());
            Some(label)
        } else {
            None
        };

        let ending = field("");
        root.append(&ending);
        root.append(&separator());
        root.append(&field(ENCODING_LABEL));

        let mode = if editor {
            root.append(&separator());
            let label = field(EditMode::Insert.label());
            root.append(&label);
            Some(label)
        } else {
            None
        };

        Self {
            root,
            stats,
            last_stats: Cell::new(None),
            cursor,
            last_edit_separator,
            last_edit,
            trailing,
            dot_state: Rc::new(Cell::new(DotState::Clean)),
            alt_held: Cell::new(false),
            ending,
            mode,
            mode_menu: OnceCell::new(),
        }
    }

    /// Update byte and character counts, line count and line-ending style.
    /// A measurement identical to the one already displayed is ignored, so
    /// cursor and selection refreshes do not re-format labels or restyle the
    /// whitespace dot.
    pub fn set_stats(&self, stats: &TextStats) {
        if self.last_stats.get() == Some(*stats) {
            return;
        }
        self.last_stats.set(Some(*stats));
        self.stats.set_text(&stats.summary());
        self.ending.set_text(stats.line_ending.label());
        self.refresh_dot();
    }

    /// Set whether an Alt key is currently held. While it is, a red dot with
    /// trailing whitespace shows the yellow state instead.
    pub fn set_alt_held(&self, held: bool) {
        if self.alt_held.replace(held) != held {
            self.refresh_dot();
        }
    }

    /// Make the dot activate the trailing-whitespace action (yellow) or the
    /// non-empty-blank-line action (red) on a primary click. The green state is
    /// inert.
    pub fn set_dot_actions(&self, trailing: &str, blank: &str) {
        let Some(dot) = &self.trailing else {
            return;
        };
        let trailing = trailing.to_string();
        let blank = blank.to_string();
        let state = self.dot_state.clone();
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.connect_released(move |gesture, _, _, _| {
            let Some(label) = gesture.widget() else {
                return;
            };
            let action = match state.get() {
                DotState::Clean => return,
                DotState::Trailing => &trailing,
                DotState::Blank => &blank,
            };
            let _ = label.activate_action(action, None);
        });
        dot.add_controller(click);
    }

    fn refresh_dot(&self) {
        let Some(dot) = &self.trailing else {
            return;
        };
        let stats = self.last_stats.get();
        let trailing = stats.is_some_and(|stats| stats.has_trailing_whitespace);
        let blank = stats.is_some_and(|stats| stats.has_non_empty_blank_line);
        let next = dot_state(trailing, blank, self.alt_held.get());
        if self.dot_state.get() == next {
            return;
        }
        self.dot_state.set(next);
        dot.remove_css_class("trailing-clean");
        dot.remove_css_class("trailing-dirty");
        dot.remove_css_class("trailing-blank");
        match next {
            DotState::Clean => {
                dot.add_css_class("trailing-clean");
                dot.set_tooltip_text(Some("No trailing whitespace"));
                dot.set_cursor_from_name(None);
            }
            DotState::Trailing => {
                dot.add_css_class("trailing-dirty");
                dot.set_tooltip_text(Some("Find trailing whitespace"));
                dot.set_cursor_from_name(Some("pointer"));
            }
            DotState::Blank => {
                dot.add_css_class("trailing-blank");
                dot.set_tooltip_text(Some("Find non-empty blank line"));
                dot.set_cursor_from_name(Some("pointer"));
            }
        }
    }

    /// The trailing-whitespace dot, for the UI smoke test.
    #[cfg(test)]
    pub(crate) fn trailing_label(&self) -> Option<&gtk::Label> {
        self.trailing.as_ref()
    }

    /// Update the cursor fields (no-op on a pane bar). With a selection, the
    /// `Pos` field is replaced by the selection size.
    pub fn set_cursor(&self, cursor: &CursorPosition, selection: Option<&SelectionStats>) {
        if let Some(label) = &self.cursor {
            label.set_text(&cursor.summary(selection));
        }
    }

    /// Show INS, OVR or RO (no-op on a pane bar).
    pub fn set_mode(&self, mode: EditMode) {
        if mode != EditMode::ReadOnly {
            self.dismiss_read_only_warning();
        }
        if let Some(label) = &self.mode {
            label.set_text(mode.label());
        }
    }

    /// Show the last-edit text, or hide the field for `None` (no-op on a
    /// pane bar).
    pub fn set_last_edit(&self, text: Option<&str>) {
        if let (Some(label), Some(separator)) = (&self.last_edit, &self.last_edit_separator) {
            label.set_text(text.unwrap_or(""));
            label.set_visible(text.is_some());
            separator.set_visible(text.is_some());
        }
    }

    /// Make the line-ending field clickable: a primary click pops up `model`
    /// (whose actions are resolved through the window the bar lives in).
    pub fn set_ending_menu(&self, model: &gio::Menu) {
        Self::attach_menu(&self.ending, model, "Convert line endings");
    }

    /// Make the INS/OVR/RO field clickable (no-op on a pane bar).
    pub fn set_mode_menu(&self, model: &gio::Menu) {
        if let Some(label) = &self.mode {
            let popover = Self::attach_menu(label, model, "Read-only mode");
            let timer = Rc::new(WarningTimer::default());
            let timer_for_close = timer.clone();
            popover.connect_closed(move |popover| {
                timer_for_close.cancel();
                popover.remove_css_class(READ_ONLY_WARNING_CLASS);
            });
            let weak = popover.downgrade();
            label.connect_unmap(move |_| {
                if let Some(popover) = weak.upgrade()
                    && popover.has_css_class(READ_ONLY_WARNING_CLASS)
                {
                    popover.popdown();
                }
            });
            assert!(self.mode_menu.set(ModeMenu { popover, timer }).is_ok());
        }
    }

    /// Briefly show the mode menu without making it modal or claiming focus.
    pub fn warn_read_only(&self) {
        let Some(ModeMenu { popover, timer }) = self.mode_menu.get() else {
            return;
        };
        if !self.root.is_mapped() {
            return;
        }
        if popover.is_visible() {
            // Do not replace a menu the user explicitly opened.
            if !popover.has_css_class(READ_ONLY_WARNING_CLASS) {
                return;
            }
        } else {
            popover.set_autohide(false);
            popover.set_can_focus(false);
            popover.add_css_class(READ_ONLY_WARNING_CLASS);
            popover.popup();
        }

        timer.cancel();
        let weak = popover.downgrade();
        let timer_for_callback = timer.clone();
        timer.0.replace(Some(glib::timeout_add_local_once(
            Duration::from_secs(2),
            move || {
                timer_for_callback.0.take();
                if let Some(popover) = weak.upgrade() {
                    popover.popdown();
                }
            },
        )));
    }

    pub fn dismiss_read_only_warning(&self) {
        if let Some(mode) = self.mode_menu.get()
            && mode.popover.has_css_class(READ_ONLY_WARNING_CLASS)
        {
            mode.popover.popdown();
        }
    }

    #[cfg(test)]
    pub(super) fn mode_popover(&self) -> &gtk::PopoverMenu {
        &self.mode_menu.get().unwrap().popover
    }

    fn attach_menu(label: &gtk::Label, model: &gio::Menu, tooltip: &str) -> gtk::PopoverMenu {
        let popover = gtk::PopoverMenu::from_model(Some(model));
        popover.set_parent(label);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::End);

        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        {
            let popover = popover.clone();
            click.connect_released(move |_, _, _, _| {
                if popover.has_css_class(READ_ONLY_WARNING_CLASS) {
                    popover.popdown();
                }
                if !popover.is_autohide() {
                    popover.set_can_focus(true);
                    popover.set_autohide(true);
                }
                popover.popup();
            });
        }
        label.add_controller(click);
        label.set_cursor_from_name(Some("pointer"));
        label.set_tooltip_text(Some(tooltip));

        // A parented popover must be unparented before its parent goes away.
        let weak = popover.downgrade();
        label.connect_destroy(move |_| {
            if let Some(popover) = weak.upgrade() {
                popover.unparent();
            }
        });
        popover
    }
}

fn field(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class("caption");
    label
}

fn separator() -> gtk::Separator {
    gtk::Separator::new(gtk::Orientation::Vertical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_state_prefers_blank_unless_alt_shows_trailing() {
        assert_eq!(dot_state(false, false, false), DotState::Clean);
        assert_eq!(dot_state(false, false, true), DotState::Clean);
        assert_eq!(dot_state(true, false, false), DotState::Trailing);
        assert_eq!(dot_state(true, false, true), DotState::Trailing);
        assert_eq!(dot_state(true, true, false), DotState::Blank);
        assert_eq!(dot_state(true, true, true), DotState::Trailing);
        // Red implies yellow, but keep the general rule explicit.
        assert_eq!(dot_state(false, true, false), DotState::Blank);
        assert_eq!(dot_state(false, true, true), DotState::Blank);
    }
}
