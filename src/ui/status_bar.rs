//! The status bar shown under the editor and under each diff pane.
//!
//! Presentation only: the numbers come from `core::textstats`.

use gtk::prelude::*;
use gtk::{gdk, gio};

use crate::core::textstats::{CursorPosition, SelectionStats, TextStats};

const ENCODING_LABEL: &str = "UTF-8";

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
    /// Editor bar only.
    cursor: Option<gtk::Label>,
    /// Editor bar only: hidden until the first edit.
    last_edit_separator: Option<gtk::Separator>,
    last_edit: Option<gtk::Label>,
    /// Editor bar only: green when clean, yellow when trailing whitespace
    /// exists.
    trailing: Option<gtk::Label>,
    ending: gtk::Label,
    /// Editor bar only.
    mode: Option<gtk::Label>,
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
            cursor,
            last_edit_separator,
            last_edit,
            trailing,
            ending,
            mode,
        }
    }

    /// Update byte and character counts, line count and line-ending style.
    pub fn set_stats(&self, stats: &TextStats) {
        self.stats.set_text(&stats.summary());
        self.ending.set_text(stats.line_ending.label());
        if let Some(dot) = &self.trailing {
            if stats.has_trailing_whitespace {
                dot.remove_css_class("trailing-clean");
                dot.add_css_class("trailing-dirty");
                dot.set_tooltip_text(Some("Trailing whitespace"));
                dot.set_cursor_from_name(Some("pointer"));
            } else {
                dot.remove_css_class("trailing-dirty");
                dot.add_css_class("trailing-clean");
                dot.set_tooltip_text(Some("No trailing whitespace"));
                dot.set_cursor_from_name(None);
            }
        }
    }

    /// Make the trailing-whitespace dot activate `action` on a primary click.
    /// The action is enabled only while the dot is yellow, so a green dot is
    /// inert.
    pub fn set_trailing_action(&self, action: &str) {
        let Some(dot) = &self.trailing else {
            return;
        };
        let action = action.to_string();
        let label = dot.clone();
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.connect_released(move |_, _, _, _| {
            let _ = label.activate_action(&action, None);
        });
        dot.add_controller(click);
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
            Self::attach_menu(label, model, "Read-only mode");
        }
    }

    fn attach_menu(label: &gtk::Label, model: &gio::Menu, tooltip: &str) {
        let popover = gtk::PopoverMenu::from_model(Some(model));
        popover.set_parent(label);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::End);

        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        {
            let popover = popover.clone();
            click.connect_released(move |_, _, _, _| popover.popup());
        }
        label.add_controller(click);
        label.set_cursor_from_name(Some("pointer"));
        label.set_tooltip_text(Some(tooltip));

        // A parented popover must be unparented before its parent goes away.
        label.connect_destroy(move |_| popover.unparent());
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
