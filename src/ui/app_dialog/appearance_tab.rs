//! Appearance tab: live theme selection and explicit font application.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use adw::prelude::*;

use super::cleanup_dialog::CleanupPage;
use crate::core::{appearance::Appearance, themes};
use crate::ui::theme;
use crate::ui::toast::{self, ToastKind};

/// Distinguishes preview CSS classes when several Appearance dialogs are open.
static PREVIEW_ID: AtomicU64 = AtomicU64::new(0);

pub struct AppearanceTab {
    pub root: gtk::Box,
}

impl AppearanceTab {
    pub fn new(
        dialog: &adw::Dialog,
        toasts: &adw::ToastOverlay,
        state_path: Option<PathBuf>,
        persist_font: bool,
    ) -> Self {
        let page = CleanupPage::new(
            "Appearance changes affect every open window and are remembered for next time.",
        );
        let restore = &page.delete_button;
        restore.set_label("Restore Defaults");
        restore.remove_css_class("destructive-action");
        restore.set_sensitive(true);
        page.connect_close(dialog);

        let fonts = super::font_controls::build(toasts, persist_font);
        page.list.append(&fonts.root);
        let weak_toasts = toasts.downgrade();
        let restore_path = state_path.clone();
        restore.connect_clicked(move |_| {
            let default_theme = theme::catalog()
                .into_iter()
                .find(|theme| theme.id == "dark")
                .expect("built-in Dark is always available");
            let defaults = Appearance::default();
            theme::apply_appearance(defaults.clone());
            theme::apply(default_theme.clone());
            fonts.reset_fields();

            // Try both writes even if one fails; the defaults remain applied in memory.
            let mut errors = Vec::new();
            if let Some(path) = &restore_path
                && let Err(err) = themes::save_selected_to(path, &default_theme.definition)
            {
                errors.push(format!("theme: {err}"));
            }
            if persist_font && let Err(err) = defaults.save_to(&Appearance::default_path()) {
                errors.push(format!("font: {err}"));
            }
            if let Some(toasts) = weak_toasts.upgrade() {
                let notice = if errors.is_empty() {
                    toast::new(ToastKind::Success, "Appearance defaults restored")
                } else {
                    toast::new(
                        ToastKind::Warning,
                        &format!(
                            "Defaults applied, but could not be remembered: {}",
                            errors.join("; ")
                        ),
                    )
                };
                toasts.add_toast(notice);
            }
        });

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let messages = theme::diagnostics();
        if !messages.is_empty() {
            let diagnostics = gtk::Label::new(Some(&messages.join("\n")));
            diagnostics.set_wrap(true);
            diagnostics.set_selectable(true);
            diagnostics.set_xalign(0.0);
            page.list.append(&diagnostics);
        }
        let policy = gtk::Label::new(Some(
            "User themes are loaded at startup. Restart notediff after adding or editing a theme.",
        ));
        policy.set_wrap(true);
        policy.set_xalign(0.0);
        policy.add_css_class("dim-label");
        page.list.append(&policy);
        let selected = theme::current().id.clone();
        let mut group: Option<gtk::CheckButton> = None;
        let mut preview_css = String::new();
        for def in theme::catalog() {
            let check = gtk::CheckButton::new();
            check.set_valign(gtk::Align::Center);
            if let Some(first) = &group {
                check.set_group(Some(first));
            } else {
                group = Some(check.clone());
            }
            check.set_active(def.id == selected);
            // Keep open pickers in step with the hidden shortcut or another window.
            let weak = check.downgrade();
            let observed = def.clone();
            theme::on_change(move || {
                let Some(check) = weak.upgrade() else {
                    return false;
                };
                check.set_active(theme::current().id == observed.id);
                true
            });
            let toasts = toasts.downgrade();
            let state_path = state_path.clone();
            let chosen = def.clone();
            check.connect_toggled(move |check| {
                let def = &chosen;
                if !check.is_active() || theme::current().id == def.id {
                    return;
                }
                theme::apply(def.clone());
                let toast = match state_path
                    .as_deref()
                    .map(|path| themes::save_selected_to(path, &def.definition))
                {
                    Some(Err(err)) => toast::new(
                        ToastKind::Warning,
                        &format!("{} applied, but could not be remembered: {err}", def.name),
                    ),
                    _ => toast::new(ToastKind::Success, &format!("Theme: {}", def.name)),
                };
                if let Some(toasts) = toasts.upgrade() {
                    toasts.add_toast(toast);
                }
            });
            let row = adw::ActionRow::new();
            // Set this before the title: theme names are plain text and may
            // contain ampersands.
            row.set_use_markup(false);
            row.set_title(&def.name);
            row.set_subtitle(&def.description);
            row.add_prefix(&check);
            row.add_suffix(&preview_widget(&def, &mut preview_css));
            row.set_activatable_widget(Some(&check));
            list.append(&row);
        }
        page.list.append(&list);
        install_preview_css(page.root.upcast_ref(), &preview_css);
        Self {
            root: page.root.clone(),
        }
    }
}

/// Two coloured chips with real padding. Pango markup can draw a text-run
/// background, but it cannot add padding inside that background, so each part
/// is a separate label styled by a generated CSS class.
fn preview_widget(def: &theme::LoadedTheme, css: &mut String) -> gtk::Box {
    let p = &def.palette.preview;
    let id = PREVIEW_ID.fetch_add(1, Ordering::Relaxed);
    let gutter_class = format!("notediff-preview-gutter-{id}");
    let text_class = format!("notediff-preview-text-{id}");
    css.push_str(&format!(
        ".{gutter_class} {{ background-color: {}; color: {}; padding: 3px 6px 2px 6px; }}\n\
         .{text_class} {{ background-color: {}; color: {}; padding: 3px 6px 2px 6px; }}\n",
        p.gutter_background, p.gutter_text, p.background, p.text
    ));

    let gutter = gtk::Label::new(Some("1"));
    gutter.add_css_class(theme::PREVIEW_FONT_CLASS);
    gutter.add_css_class(&gutter_class);

    let text = gtk::Label::new(Some("Sample text"));
    text.add_css_class(theme::PREVIEW_FONT_CLASS);
    text.add_css_class(&text_class);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_valign(gtk::Align::Center);
    row.set_margin_end(6);
    row.append(&gutter);
    row.append(&text);
    row
}

fn install_preview_css(root: &gtk::Widget, css: &str) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(css);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    root.connect_destroy(move |_| {
        gtk::style_context_remove_provider_for_display(&display, &provider);
    });
}
