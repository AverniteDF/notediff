//! Resolve only effective GtkSourceView styles, never reparse XML colour aliases.

use gtk::gdk;

/// Minimum contrast ratio for readable review colours.
const MIN_CONTRAST: f32 = 4.5;

/// Colours for the Appearance tab's two-segment sample: the gutter with a
/// normal line number, and one non-selected line of text.
#[derive(Clone, Debug)]
pub struct Preview {
    pub background: String,
    pub text: String,
    pub gutter_background: String,
    pub gutter_text: String,
}

#[derive(Clone, Debug)]
pub struct Highlight {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub hunk: Highlight,
    pub word: Highlight,
    pub focused_hunk: Highlight,
    pub focused_word: Highlight,
    pub dirty_header_bg: String,
    /// A CSS colour used to override the gutter when the theme derives its
    /// normal line-number colour from `current-line-number`.
    pub gutter_fallback: Option<String>,
    /// Opaque line-number colours: normal rows and focused review rows.
    pub line_number: String,
    pub line_number_strong: String,
    pub line_number_strong_bold: bool,
    pub preview: Preview,
}

fn parse_any(value: Option<&str>, set: bool) -> Option<gdk::RGBA> {
    if !set {
        return None;
    }
    value.and_then(|v| gdk::RGBA::parse(v).ok())
}

#[cfg(test)]
fn parse(value: Option<&str>, set: bool) -> Option<gdk::RGBA> {
    parse_any(value, set).filter(|c| c.alpha() == 1.0) // Translucency has no unambiguous base colour.
}

fn color_any(
    scheme: Option<&sourceview5::StyleScheme>,
    name: &str,
    background: bool,
) -> Option<gdk::RGBA> {
    // get_style resolves parent-scheme and use-style inheritance in GtkSourceView.
    let style = scheme?.style(name)?;
    if background {
        parse_any(style.background().as_deref(), style.is_background_set())
    } else {
        parse_any(style.foreground().as_deref(), style.is_foreground_set())
    }
}

fn color(
    scheme: Option<&sourceview5::StyleScheme>,
    name: &str,
    background: bool,
) -> Option<gdk::RGBA> {
    color_any(scheme, name, background).filter(|c| c.alpha() == 1.0)
}

fn rgb(r: f32, g: f32, b: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, 1.0)
}

fn array(c: &gdk::RGBA) -> [f32; 4] {
    [c.red(), c.green(), c.blue(), 1.0]
}

fn hex(c: &gdk::RGBA) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.red() * 255.0).round() as u8,
        (c.green() * 255.0).round() as u8,
        (c.blue() * 255.0).round() as u8
    )
}

/// Composite a possibly translucent colour over an opaque background.
fn flatten(fg: &gdk::RGBA, bg: &gdk::RGBA) -> gdk::RGBA {
    let alpha = fg.alpha();
    gdk::RGBA::new(
        fg.red() * alpha + bg.red() * (1.0 - alpha),
        fg.green() * alpha + bg.green() * (1.0 - alpha),
        fg.blue() * alpha + bg.blue() * (1.0 - alpha),
        1.0,
    )
}

fn css_color(c: &gdk::RGBA) -> String {
    if (c.alpha() - 1.0).abs() < f32::EPSILON {
        hex(c)
    } else {
        format!(
            "rgba({}, {}, {}, {:.2})",
            (c.red() * 255.0).round() as u8,
            (c.green() * 255.0).round() as u8,
            (c.blue() * 255.0).round() as u8,
            c.alpha()
        )
    }
}

fn with_alpha(c: &gdk::RGBA, alpha: f32) -> gdk::RGBA {
    gdk::RGBA::new(c.red(), c.green(), c.blue(), c.alpha() * alpha)
}

fn luminance(c: &gdk::RGBA) -> f32 {
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.red()) + 0.7152 * linear(c.green()) + 0.0722 * linear(c.blue())
}

pub(super) fn contrast(a: &gdk::RGBA, b: &gdk::RGBA) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn readable(fg: &gdk::RGBA, bg: &gdk::RGBA) -> gdk::RGBA {
    if contrast(fg, bg) >= MIN_CONTRAST {
        return *fg;
    }
    let (black, white) = (rgb(0.0, 0.0, 0.0), rgb(1.0, 1.0, 1.0));
    if contrast(&black, bg) >= contrast(&white, bg) {
        black
    } else {
        white
    }
}

/// Fixed, opaque review colours. Never mix these with editor colours.
fn highlight(
    scheme: Option<&sourceview5::StyleScheme>,
    name: &str,
    default_foreground: gdk::RGBA,
    default_background: &str,
    warnings: &mut Vec<String>,
) -> Highlight {
    let default_bg = gdk::RGBA::parse(default_background).expect("valid standard review colour");
    let foreground = color(scheme, name, false);
    let background = color(scheme, name, true);
    if scheme.and_then(|scheme| scheme.style(name)).is_some()
        && (foreground.is_none() || background.is_none())
    {
        warnings.push(format!(
            "{name}: unset, invalid or translucent colour; using standard field fallback"
        ));
    }
    let (fg, bg) = (
        foreground.unwrap_or(default_foreground),
        background.unwrap_or(default_bg),
    );
    if contrast(&fg, &bg) < MIN_CONTRAST {
        warnings.push(format!(
            "{name}: contrast below 4.5:1; using standard review colours"
        ));
        return Highlight {
            foreground: array(&default_foreground),
            background: array(&default_bg),
        };
    }
    Highlight {
        foreground: array(&fg),
        background: array(&bg),
    }
}

/// GtkSourceView makes the current-line number bold unless the scheme
/// explicitly sets `bold="false"`. An absent style is also bold.
fn current_line_number_bold(style: Option<&sourceview5::Style>) -> bool {
    style.is_none_or(|style| !style.is_bold_set() || style.is_bold())
}

/// Review styles are exact XML colours or fixed dark/light-mode fallbacks.
/// Preview colours and the dirty header still follow the editor scheme.
/// `line_number_alpha` carries an explicit `auto`/percentage request from the
/// theme XML; `None` still enables the 50% fallback when `line-numbers` has no
/// foreground of its own.
pub fn resolve(
    scheme: Option<&sourceview5::StyleScheme>,
    dark: bool,
    line_number_alpha: Option<f32>,
) -> (Palette, Vec<String>) {
    let mut warnings = Vec::new();
    let bg = color(scheme, "text", true).unwrap_or_else(|| {
        warnings.push("text background unset, invalid or translucent; using mode fallback".into());
        if dark {
            rgb(0.12, 0.14, 0.16)
        } else {
            rgb(0.98, 0.98, 0.97)
        }
    });
    let fg = color(scheme, "text", false).unwrap_or_else(|| {
        warnings.push("text foreground unset, invalid or translucent; using mode fallback".into());
        if dark {
            rgb(0.8, 0.83, 0.87)
        } else {
            rgb(0.18, 0.21, 0.25)
        }
    });
    if contrast(&fg, &bg) < MIN_CONTRAST {
        warnings.push("scheme text contrast is below 4.5:1; original colours are retained".into());
    }
    let gutter_bg = color(scheme, "line-numbers", true).unwrap_or(bg);
    let explicit_gutter_fg = color_any(scheme, "line-numbers", false);
    let derived_alpha = line_number_alpha.or_else(|| explicit_gutter_fg.is_none().then_some(0.5));
    let (gutter_fg, gutter_fallback) = match derived_alpha {
        Some(alpha) => {
            let base = color_any(scheme, "current-line-number", false).unwrap_or(fg);
            let color = with_alpha(&base, alpha);
            let css = css_color(&color);
            (color, Some(css))
        }
        None => (explicit_gutter_fg.unwrap_or(fg), None),
    };
    let accent = color(scheme, "notediff:accent", false).unwrap_or_else(|| {
        if dark {
            // The built-in dark scheme's amber accent is the dark default.
            gdk::RGBA::parse("#e0b36b").expect("valid dark accent")
        } else {
            rgb(0.7, 0.4, 0.02)
        }
    });
    let line_number_strong_style = scheme.and_then(|scheme| scheme.style("current-line-number"));
    let line_number_strong_color = color_any(scheme, "current-line-number", false).unwrap_or(fg);
    let line_number_strong_bold = current_line_number_bold(line_number_strong_style.as_ref());
    let (review_fg, backgrounds) = if dark {
        (
            rgb(1.0, 1.0, 1.0),
            ["#664400", "#8A5A00", "#702020", "#AD3030"],
        )
    } else {
        (
            rgb(0.0, 0.0, 0.0),
            ["#FFF0C2", "#FFD477", "#FAD6D6", "#F39A9A"],
        )
    };
    (
        Palette {
            hunk: highlight(
                scheme,
                "notediff:hunk",
                review_fg,
                backgrounds[0],
                &mut warnings,
            ),
            word: highlight(
                scheme,
                "notediff:word",
                review_fg,
                backgrounds[1],
                &mut warnings,
            ),
            focused_hunk: highlight(
                scheme,
                "notediff:focused-hunk",
                review_fg,
                backgrounds[2],
                &mut warnings,
            ),
            focused_word: highlight(
                scheme,
                "notediff:focused-word",
                review_fg,
                backgrounds[3],
                &mut warnings,
            ),
            dirty_header_bg: format!("mix(@headerbar_bg_color, {}, 0.35)", hex(&accent)),
            gutter_fallback,
            line_number: hex(&flatten(&gutter_fg, &gutter_bg)),
            line_number_strong: hex(&flatten(&line_number_strong_color, &gutter_bg)),
            line_number_strong_bold,
            preview: Preview {
                background: hex(&bg),
                text: hex(&readable(&fg, &bg)),
                gutter_background: hex(&gutter_bg),
                gutter_text: css_color(&gutter_fg),
            },
        },
        warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_invalid_and_translucent_values_are_not_colours() {
        assert!(parse(Some("#ffffff"), false).is_none());
        assert!(parse(Some("nonsense"), true).is_none());
        assert!(parse(Some("rgba(0,0,0,0.5)"), true).is_none());
        assert!(parse(None, true).is_none());
        assert!(parse(Some("#ffffff"), true).is_some());
    }

    #[test]
    fn translucent_base_colours_keep_their_opacity() {
        let base = gdk::RGBA::parse("rgba(255, 255, 255, 0.8)").unwrap();
        let derived = with_alpha(&base, 0.5);
        assert!((derived.alpha() - 0.4).abs() < 1e-6);
        assert_eq!(
            (derived.red(), derived.green(), derived.blue()),
            (1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn fallback_overlays_and_sample_text_are_readable_in_both_modes() {
        for dark in [false, true] {
            let (p, warnings) = resolve(None, dark, None);
            assert_eq!(warnings.len(), 2);
            for style in [&p.hunk, &p.word, &p.focused_hunk, &p.focused_word] {
                let fg = super::super::theme::rgba(style.foreground);
                let bg = super::super::theme::rgba(style.background);
                assert!(contrast(&fg, &bg) >= MIN_CONTRAST);
                assert_eq!(
                    style.foreground,
                    if dark { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }
                );
                assert_eq!(style.background[3], 1.0);
            }
            assert!(p.gutter_fallback.is_some());
            assert!(
                p.dirty_header_bg
                    .contains(if dark { "#e0b36b" } else { "#b36605" })
            );
            let p = p.preview;
            let bg = gdk::RGBA::parse(&p.background).unwrap();
            assert!(contrast(&gdk::RGBA::parse(&p.text).unwrap(), &bg) >= MIN_CONTRAST);
            // The gutter deliberately keeps the theme's normal line-number
            // colour, even when that colour is dim.
        }
    }

    #[test]
    fn review_fallbacks_are_exact_for_each_theme_mode() {
        for dark in [false, true] {
            let (p, _) = resolve(None, dark, None);
            let backgrounds = if dark {
                ["#664400", "#8a5a00", "#702020", "#ad3030"]
            } else {
                ["#fff0c2", "#ffd477", "#fad6d6", "#f39a9a"]
            };
            for (style, expected) in [p.hunk, p.word, p.focused_hunk, p.focused_word]
                .into_iter()
                .zip(backgrounds)
            {
                assert_eq!(hex(&super::super::theme::rgba(style.background)), expected);
                assert_eq!(
                    style.foreground,
                    if dark { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }
                );
            }
        }
    }
}
