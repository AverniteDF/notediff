//! Hidden Alt+T / Alt+Shift+T theme cycling.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::core::themes;
use crate::ui::{theme, toast::ToastKind};

use super::AppWindow;

/// Return whether to cycle backward, ignoring lock modifiers.
fn direction(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<bool> {
    direction_with_mask(key, modifiers, gtk::accelerator_get_default_mod_mask())
}

/// Keep shortcut matching testable without initializing GTK or a display.
fn direction_with_mask(
    key: gdk::Key,
    modifiers: gdk::ModifierType,
    mask: gdk::ModifierType,
) -> Option<bool> {
    if key != gdk::Key::t && key != gdk::Key::T {
        return None;
    }
    let modifiers = modifiers & mask;
    if modifiers == gdk::ModifierType::ALT_MASK {
        Some(false)
    } else if modifiers == (gdk::ModifierType::ALT_MASK | gdk::ModifierType::SHIFT_MASK) {
        Some(true)
    } else {
        None
    }
}

impl AppWindow {
    pub(super) fn connect_theme_keys(this: &Rc<Self>) {
        let state = Rc::new(Cell::new(false));
        let keys = gtk::EventControllerKey::new();
        keys.set_name(Some("notediff-theme-keys"));
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let pressed = state.clone();
        let weak = Rc::downgrade(this);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(backward) = direction(key, modifiers) else {
                return glib::Propagation::Proceed;
            };
            if !pressed.replace(true)
                && let Some(this) = weak.upgrade()
            {
                this.cycle_theme(backward);
            }
            glib::Propagation::Stop
        });
        let released = state.clone();
        keys.connect_key_released(move |_, key, _, _| {
            if key == gdk::Key::t || key == gdk::Key::T {
                released.set(false);
            }
        });
        // Releases can be delivered elsewhere after switching applications.
        this.window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                state.set(false);
            }
        });
        this.window.add_controller(keys);
    }

    fn cycle_theme(&self, backward: bool) {
        let catalog = theme::catalog();
        let index = catalog
            .iter()
            .position(|def| def.id == theme::current().id)
            .unwrap_or(0);
        let count = catalog.len();
        let next_index = if backward {
            (index + count - 1) % count
        } else {
            (index + 1) % count
        };
        let next = &catalog[next_index];
        theme::apply(next.clone());
        if self.persist_settings
            && let Err(err) = themes::save_selected_to(&themes::default_path(), &next.definition)
        {
            self.toast(
                ToastKind::Warning,
                &format!("{} applied, but could not be remembered: {err}", next.name),
            );
            return;
        }
        self.toast(ToastKind::Success, &format!("Theme: {}", next.name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_choose_direction_without_matching_other_combinations() {
        use gdk::{Key, ModifierType as Mod};
        let direction = |key, modifiers| {
            direction_with_mask(
                key,
                modifiers,
                Mod::SHIFT_MASK
                    | Mod::CONTROL_MASK
                    | Mod::ALT_MASK
                    | Mod::SUPER_MASK
                    | Mod::HYPER_MASK
                    | Mod::META_MASK,
            )
        };
        assert_eq!(direction(Key::t, Mod::ALT_MASK), Some(false));
        assert_eq!(
            direction(Key::T, Mod::ALT_MASK | Mod::SHIFT_MASK),
            Some(true)
        );
        assert_eq!(
            direction(Key::T, Mod::ALT_MASK | Mod::LOCK_MASK),
            Some(false)
        );
        assert_eq!(direction(Key::t, Mod::empty()), None);
        assert_eq!(direction(Key::t, Mod::ALT_MASK | Mod::CONTROL_MASK), None);
        assert_eq!(direction(Key::z, Mod::ALT_MASK), None);
        assert_eq!(direction(Key::Alt_R, Mod::ALT_MASK), None);
    }
}
