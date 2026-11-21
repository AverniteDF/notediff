//! Apply the current theme to all live buffers, CSS, change listeners and
//! font-dependent appearance listeners.
//! The owned catalog is loaded once at startup; user theme edits require restart.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use gtk::{gdk, glib};
use sourceview5::prelude::*;

use super::theme_catalog::Catalog;
pub use super::theme_catalog::LoadedTheme;
use super::theme_palette::Palette;
use crate::core::appearance::Appearance;
use crate::core::fsutil::data_home;
use crate::core::themes;

const STYLESHEET: &str = include_str!("../../assets/notediff.css");

pub const CONTENT_CLASS: &str = "notediff-content";
pub const PREVIEW_FONT_CLASS: &str = "notediff-preview-font";
pub const DIRTY_CLASS: &str = "notediff-dirty";
pub const MAIN_HEADER_CLASS: &str = "notediff-main-header";

thread_local! {
    static CATALOG: RefCell<Option<Catalog>> = const { RefCell::new(None) };
    static CURRENT: RefCell<Option<Rc<LoadedTheme>>> = const { RefCell::new(None) };
    static BUFFERS: RefCell<Vec<glib::WeakRef<sourceview5::Buffer>>> = const { RefCell::new(Vec::new()) };
    /// Returning false unregisters a listener.
    static LISTENERS: RefCell<Vec<Box<dyn Fn() -> bool>>> = const { RefCell::new(Vec::new()) };
    /// Appearance changes only. Kept separate from `LISTENERS` so a font change
    /// can re-measure font-dependent widgets without recolouring or repainting.
    static APPEARANCE_LISTENERS: RefCell<Vec<Box<dyn Fn() -> bool>>> =
        const { RefCell::new(Vec::new()) };
    static CSS: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static APPEARANCE: RefCell<Appearance> = RefCell::new(Appearance::default());
}

pub fn current() -> Rc<LoadedTheme> {
    CURRENT.with_borrow(|current| {
        current
            .clone()
            .expect("theme::install before UI construction")
    })
}

pub fn catalog() -> Vec<Rc<LoadedTheme>> {
    CATALOG.with_borrow(|catalog| {
        catalog
            .as_ref()
            .expect("theme catalog installed")
            .entries
            .clone()
    })
}

#[cfg(test)]
pub fn by_id(id: &str) -> Option<Rc<LoadedTheme>> {
    CATALOG.with_borrow(|catalog| catalog.as_ref().and_then(|c| c.by_id(id)))
}

pub fn diagnostics() -> Vec<String> {
    CATALOG.with_borrow(|catalog| {
        catalog
            .as_ref()
            .map(|c| c.diagnostics.clone())
            .unwrap_or_default()
    })
}

pub fn rgba(color: [f32; 4]) -> gdk::RGBA {
    gdk::RGBA::new(color[0], color[1], color[2], color[3])
}

/// Call once at startup, before creating windows or buffers.
pub fn install() {
    install_at(
        &themes::user_directory(),
        Some(&themes::default_path()),
        &data_home().join("notediff/styles"),
    );
}

/// Startup only. Tests supply isolated paths and disable preference reads.
pub(crate) fn install_at(user_dir: &Path, state_path: Option<&Path>, styles_dir: &Path) {
    let mut catalog = Catalog::load(user_dir, styles_dir);
    let mut selected = catalog.entries[0].clone();
    if let Some(path) = state_path {
        match fs::read_to_string(path) {
            Ok(text) => {
                if let Some(id) = themes::selected_id(&text) {
                    if let Some(theme) = catalog.by_id(&id) {
                        selected = theme;
                    } else {
                        catalog.diagnostics.push(format!(
                            "Saved theme {id} is unavailable; using notediff Dark"
                        ));
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => catalog
                .diagnostics
                .push(format!("{}: {err}; using notediff Dark", path.display())),
        }
    }
    for message in &catalog.diagnostics {
        eprintln!("notediff themes: {message}");
    }
    CATALOG.with_borrow_mut(|value| *value = Some(catalog));
    CURRENT.with_borrow_mut(|value| *value = Some(selected.clone()));
    apply_color_scheme(&selected);
}

/// Switch every window without filesystem reads or persistence.
pub fn apply(theme: Rc<LoadedTheme>) {
    CURRENT.with_borrow_mut(|current| *current = Some(theme.clone()));
    apply_color_scheme(&theme);
    let buffers: Vec<sourceview5::Buffer> = BUFFERS.with_borrow_mut(|buffers| {
        buffers.retain(|weak| weak.upgrade().is_some());
        buffers.iter().filter_map(|weak| weak.upgrade()).collect()
    });
    for buffer in &buffers {
        set_scheme(buffer, &theme);
    }
    reload_css();
    // Move callbacks out before invoking them: signal callbacks may register more.
    let listeners = LISTENERS.with_borrow_mut(std::mem::take);
    let mut live: Vec<_> = listeners
        .into_iter()
        .filter(|listener| listener())
        .collect();
    LISTENERS.with_borrow_mut(|listeners| {
        live.append(listeners);
        *listeners = live;
    });
}

pub fn on_change(listener: impl Fn() -> bool + 'static) {
    LISTENERS.with_borrow_mut(|listeners| listeners.push(Box::new(listener)));
}

/// Listen for content font changes only. Returning false unregisters.
pub fn on_appearance(listener: impl Fn() -> bool + 'static) {
    APPEARANCE_LISTENERS.with_borrow_mut(|listeners| listeners.push(Box::new(listener)));
}

fn apply_color_scheme(theme: &LoadedTheme) {
    adw::StyleManager::default().set_color_scheme(if theme.dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
}

/// Fill stylesheet placeholders without requiring a display.
pub fn stylesheet(appearance: &Appearance, palette: &Palette) -> String {
    let gutter_fallback = palette
        .gutter_fallback
        .as_deref()
        .map(|color| format!("color: {color};"))
        .unwrap_or_default();
    STYLESHEET
        .replace("{{content_class}}", CONTENT_CLASS)
        .replace("{{preview_font_class}}", PREVIEW_FONT_CLASS)
        .replace(
            "{{font_family}}",
            &css_font_family(appearance.font_family()),
        )
        .replace("{{font_size}}", &appearance.font_size_pt().to_string())
        .replace("{{dirty_class}}", DIRTY_CLASS)
        .replace("{{main_header_class}}", MAIN_HEADER_CLASS)
        .replace("{{dirty_header_bg}}", &palette.dirty_header_bg)
        .replace("{{gutter_fallback}}", &gutter_fallback)
}

/// Render a comma-separated family list as CSS. Unquoted names with digit-led
/// tokens (for example `Foo 2`) are not valid CSS identifiers, so all
/// non-generic names are quoted. Generic families must stay unquoted.
fn css_font_family(families: &str) -> String {
    const GENERIC: &[&str] = &[
        "serif",
        "sans-serif",
        "cursive",
        "fantasy",
        "monospace",
        "system-ui",
        "ui-serif",
        "ui-sans-serif",
        "ui-monospace",
        "ui-rounded",
        "emoji",
        "math",
        "fangsong",
    ];
    families
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            if GENERIC
                .iter()
                .any(|generic| name.eq_ignore_ascii_case(generic))
            {
                name.to_string()
            } else {
                format!("\"{name}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn install_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&stylesheet(&appearance(), &current().palette));
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    CSS.with_borrow_mut(|css| *css = Some(provider));
}

/// Current application-wide font settings. Reading never accesses the filesystem.
pub fn appearance() -> Appearance {
    APPEARANCE.with_borrow(Clone::clone)
}

/// Apply fonts without changing theme, document text or persistence.
pub fn apply_appearance(value: Appearance) {
    APPEARANCE.with_borrow_mut(|current| *current = value);
    reload_css();
    notify_appearance();
}

fn notify_appearance() {
    // Move callbacks out before invoking them: they may register more.
    let listeners = APPEARANCE_LISTENERS.with_borrow_mut(std::mem::take);
    let mut live: Vec<_> = listeners
        .into_iter()
        .filter(|listener| listener())
        .collect();
    APPEARANCE_LISTENERS.with_borrow_mut(|listeners| {
        live.append(listeners);
        *listeners = live;
    });
}

fn reload_css() {
    CSS.with_borrow(|css| {
        if let Some(provider) = css {
            provider.load_from_string(&stylesheet(&appearance(), &current().palette));
        }
    });
}

pub fn apply_content_view(view: &sourceview5::View) {
    view.add_css_class(CONTENT_CLASS);
}

/// Register buffers weakly so future changes restyle them without keeping them alive.
pub fn apply_scheme(buffer: &sourceview5::Buffer) {
    BUFFERS.with_borrow_mut(|buffers| {
        buffers.retain(|weak| weak.upgrade().is_some());
        if !buffers
            .iter()
            .any(|weak| weak.upgrade().as_ref() == Some(buffer))
        {
            buffers.push(buffer.downgrade());
        }
    });
    set_scheme(buffer, &current());
}

fn set_scheme(buffer: &sourceview5::Buffer, theme: &LoadedTheme) {
    buffer.set_style_scheme(theme.scheme.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_families_are_safe_css_identifiers() {
        assert_eq!(
            css_font_family("Fira Code, monospace"),
            "\"Fira Code\", monospace"
        );
        assert_eq!(css_font_family("Foo 2, serif"), "\"Foo 2\", serif");
        assert_eq!(css_font_family("  "), "");
    }

    #[test]
    fn stylesheet_fills_every_placeholder_in_both_modes() {
        for dark in [false, true] {
            let (palette, _) = super::super::theme_palette::resolve(None, dark, None);
            let appearance = Appearance::new("Fira Code, monospace", 18).unwrap();
            let css = stylesheet(&appearance, &palette);
            assert!(!css.contains("{{"));
            assert!(!css.contains("}}"));
            assert!(css.contains("font-family: \"Fira Code\", monospace;"));
            assert!(css.contains("font-size: 18pt;"));
            assert!(css.contains(PREVIEW_FONT_CLASS));
            assert!(css.contains("border gutter"));
            assert!(css.contains("color: rgba("));
            assert!(css.contains(&format!("headerbar.{MAIN_HEADER_CLASS}")));
            assert!(css.contains(&palette.dirty_header_bg));
        }
    }
}
