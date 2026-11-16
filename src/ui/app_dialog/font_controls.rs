//! Staged font fields; Apply Font updates all content views via their CSS class.

use adw::prelude::*;

use crate::core::appearance::{Appearance, MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT};
use crate::ui::{
    theme,
    toast::{self, ToastKind},
};

pub(super) struct FontControls {
    pub root: gtk::Box,
    family: gtk::Entry,
    size: gtk::SpinButton,
}

impl FontControls {
    pub fn reset_fields(&self) {
        let defaults = Appearance::default();
        self.family.set_text(defaults.font_family());
        self.size.set_value(defaults.font_size_pt() as f64);
        // Also replace any uncommitted spin-button text when the value is unchanged.
        self.size.set_text(&defaults.font_size_pt().to_string());
    }
}

pub(super) fn build(toasts: &adw::ToastOverlay, persist: bool) -> FontControls {
    let current = theme::appearance();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);

    let family = gtk::Entry::new();
    family.set_text(current.font_family());
    family.set_hexpand(true);
    family.set_tooltip_text(Some(
        "Comma-separated family names, for example Fira Code, monospace",
    ));
    let family_row = adw::ActionRow::new();
    family_row.set_title("Font family");
    family_row.set_subtitle("First installed family is used");
    family_row.add_suffix(&family);
    family_row.set_activatable_widget(Some(&family));
    list.append(&family_row);

    let size = gtk::SpinButton::with_range(MIN_FONT_SIZE_PT as f64, MAX_FONT_SIZE_PT as f64, 1.0);
    size.set_digits(0);
    size.set_numeric(true);
    size.set_value(current.font_size_pt() as f64);
    size.set_valign(gtk::Align::Center);
    let size_row = adw::ActionRow::new();
    size_row.set_title("Font size (pt)");
    size_row.set_subtitle("Editor, diff panes and line numbers");
    size_row.add_suffix(&size);
    size_row.set_activatable_widget(Some(&size));
    list.append(&size_row);
    root.append(&list);

    let apply = gtk::Button::with_label("Apply Font");
    apply.add_css_class("suggested-action");
    apply.set_halign(gtk::Align::End);
    let weak_toasts = toasts.downgrade();
    let fields = FontControls {
        root: root.clone(),
        family: family.clone(),
        size: size.clone(),
    };
    apply.connect_clicked(move |_| {
        // Commit text that has not yet lost focus before reading the adjustment.
        size.update();
        let value = match Appearance::new(family.text().as_str(), size.value_as_int() as u32) {
            Ok(value) => value,
            Err(message) => {
                if let Some(toasts) = weak_toasts.upgrade() {
                    toasts.add_toast(toast::new(ToastKind::Warning, message));
                }
                return;
            }
        };
        theme::apply_appearance(value.clone());
        let result = if persist {
            value.save_to(&Appearance::default_path())
        } else {
            Ok(())
        };
        if let Some(toasts) = weak_toasts.upgrade() {
            let notice = match result {
                Ok(()) => toast::new(ToastKind::Success, "Font applied"),
                Err(err) => toast::new(
                    ToastKind::Warning,
                    &format!("Font applied, but could not be remembered: {err}"),
                ),
            };
            toasts.add_toast(notice);
        }
    });
    root.append(&apply);
    fields
}
