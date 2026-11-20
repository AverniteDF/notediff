//! Header-bar copying from the editor or last-focused visible review pane.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use crate::core::textstats::copied_text_summary;
use crate::ui::toast::ToastKind;

use super::{AppWindow, connect_button};

/// The header copy button's normal icon.
pub(super) const COPY_ICON: &str = "edit-copy-symbolic";
/// The temporary checkmark shown after a successful copy.
const COPIED_ICON: &str = "object-select-symbolic";
/// How long the checkmark stays visible after the newest copy.
const COPY_FLASH: Duration = Duration::from_secs(1);

impl AppWindow {
    pub(super) fn connect_copy_button(this: &Rc<Self>, button: &gtk::Button) {
        this.last_copy_view.set(Some(&this.editor_view));
        // Native editor copies (Ctrl+C and the context menu) emit this signal.
        // The header button copies directly and flashes itself.
        let weak = Rc::downgrade(this);
        this.editor_view.connect_copy_clipboard(move |view| {
            if view.buffer().has_selection()
                && let Some(this) = weak.upgrade()
            {
                this.flash_copy_button();
            }
        });
        for view in std::iter::once(&this.editor_view).chain(this.diff.copy_views()) {
            let focus = gtk::EventControllerFocus::new();
            let weak = Rc::downgrade(this);
            let source = view.downgrade();
            focus.connect_enter(move |_| {
                if let (Some(this), Some(source)) = (weak.upgrade(), source.upgrade()) {
                    this.last_copy_view.set(Some(&source));
                }
            });
            view.add_controller(focus);
        }
        // Diff panes copy their selection without the display-only markers.
        for view in this.diff.copy_views() {
            let weak = Rc::downgrade(this);
            view.connect_copy_clipboard(move |view| {
                view.stop_signal_emission_by_name("copy-clipboard");
                let Some(selected) =
                    crate::ui::diff_page::selected_text(view.buffer().upcast_ref())
                else {
                    return;
                };
                view.clipboard().set_text(&selected);
                if let Some(this) = weak.upgrade() {
                    this.flash_copy_button();
                }
            });
        }
        connect_button(button, this, |this| this.copy_selected_text());
    }

    /// Briefly show a checkmark on the header copy button after content reaches
    /// the clipboard. Each copy restarts the timer.
    pub(super) fn flash_copy_button(self: &Rc<Self>) {
        let generation = self.copy_flash.begin();
        self.copy_button.set_icon_name(COPIED_ICON);

        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(COPY_FLASH, move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.copy_flash.end(generation) {
                this.copy_button.set_icon_name(COPY_ICON);
            }
        });
    }

    pub(super) fn copy_file_name(self: &Rc<Self>) {
        let name = self
            .doc
            .borrow()
            .path()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let Some(name) = name else {
            return;
        };
        self.window.clipboard().set_text(&name);
        self.flash_copy_button();
    }

    pub(super) fn copy_full_path(self: &Rc<Self>) {
        let path = self
            .doc
            .borrow()
            .path()
            .map(|path| path.display().to_string());
        let Some(path) = path else {
            return;
        };
        self.window.clipboard().set_text(&path);
        self.flash_copy_button();
    }

    fn copy_selected_text(self: &Rc<Self>) {
        let source = if self.diff_active() {
            self.last_copy_view
                .upgrade()
                .filter(|view| {
                    view == self.diff.active_left_view() || view == self.diff.active_right_view()
                })
                .unwrap_or_else(|| self.diff.active_left_view().clone())
        } else {
            self.editor_view.clone()
        };
        let buffer = source.buffer();
        let selected = crate::ui::diff_page::selected_text(buffer.upcast_ref()).or_else(|| {
            // Outside Diff Mode an empty selection copies the whole document.
            // Selecting it first makes the copied range visible. An empty
            // buffer selects nothing and keeps the warning below.
            if self.diff_active() {
                return None;
            }
            buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
            crate::ui::diff_page::selected_text(buffer.upcast_ref())
        });
        if let Some(selected) = selected {
            // Copying must not edit text, clear the selection or change the
            // last-edit timer. Display-only diff markers are left out.
            source.clipboard().set_text(&selected);
            self.toast(ToastKind::Success, &copied_text_summary(&selected));
            self.flash_copy_button();
        } else {
            self.toast(ToastKind::Warning, "No selected text to copy!");
        }
    }
}
