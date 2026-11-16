//! Header-bar copying from the editor or last-focused visible review pane.

use std::rc::Rc;

use adw::prelude::*;

use crate::core::textstats::copied_text_summary;
use crate::ui::toast::ToastKind;

use super::{AppWindow, connect_button};

impl AppWindow {
    pub(super) fn connect_copy_button(this: &Rc<Self>, button: &gtk::Button) {
        this.last_copy_view.set(Some(&this.editor_view));
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
        connect_button(button, this, |this| this.copy_selected_text());
    }

    fn copy_selected_text(&self) {
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
        if let Some(selected) = crate::ui::diff_page::selected_text(buffer.upcast_ref()) {
            // Copying must not edit text, clear the selection or change the
            // last-edit timer. Display-only diff markers are left out.
            source.clipboard().set_text(&selected);
            self.toast(ToastKind::Success, &copied_text_summary(&selected));
        } else {
            self.toast(ToastKind::Warning, "No selected text to copy!");
        }
    }
}
