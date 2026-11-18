//! Startup-only scheme loading. Each candidate has an isolated manager.

use std::fs;
use std::ops::Deref;
use std::path::Path;
use std::rc::Rc;

use super::theme_palette::{self, Palette};
use crate::core::{
    fsutil::atomic_write_if_changed,
    themes::{self, Theme},
};

const SCHEMES: [(&str, &str); 2] = [
    (
        "notediff-dark",
        include_str!("../../assets/notediff-dark.xml"),
    ),
    (
        "notediff-light",
        include_str!("../../assets/notediff-light.xml"),
    ),
];

pub struct LoadedTheme {
    pub definition: Theme,
    pub scheme: Option<sourceview5::StyleScheme>,
    pub palette: Palette,
    // Retain the manager (and parent schemes) for the catalog's lifetime.
    _manager: sourceview5::StyleSchemeManager,
}

impl Deref for LoadedTheme {
    type Target = Theme;
    fn deref(&self) -> &Theme {
        &self.definition
    }
}

pub struct Catalog {
    pub entries: Vec<Rc<LoadedTheme>>,
    pub diagnostics: Vec<String>,
}

impl Catalog {
    pub fn load(user_dir: &Path, styles_dir: &Path) -> Self {
        let mut result = Self {
            entries: Vec::new(),
            diagnostics: Vec::new(),
        };
        for (def, (id, xml)) in themes::builtins().into_iter().zip(SCHEMES) {
            let path = styles_dir.join(format!("{id}.xml"));
            let (xml, line_number_alpha) = prepare_scheme_xml(xml);
            let manager = sourceview5::StyleSchemeManager::new();
            let scheme = match atomic_write_if_changed(&path, &xml, true) {
                Ok(_) => load_file(&manager, &path, id, &path),
                Err(err) => Err(format!("{}: {err}", path.display())),
            };
            let scheme = match scheme {
                Ok(scheme) => Some(scheme),
                Err(err) => {
                    result
                        .diagnostics
                        .push(format!("{}: {err}; using system fallback", def.name));
                    manager.scheme(if def.dark { "Adwaita-dark" } else { "Adwaita" })
                }
            };
            if scheme.is_none() {
                result
                    .diagnostics
                    .push(format!("{}: no fallback scheme available", def.name));
            }
            result.push(def, scheme, manager, line_number_alpha);
        }
        let (candidates, diagnostics) = themes::discover(user_dir);
        result.diagnostics.extend(diagnostics);
        for (def, path) in candidates {
            // A broken earlier candidate must not reserve its ID.
            if result.entries.iter().any(|entry| entry.id == def.id) {
                result.diagnostics.push(format!(
                    "{}: duplicate theme id {}; ignored",
                    path.display(),
                    def.id
                ));
                continue;
            }
            let xml = match fs::read_to_string(&path) {
                Ok(xml) => xml,
                Err(err) => {
                    result
                        .diagnostics
                        .push(format!("{}: {err}", path.display()));
                    continue;
                }
            };
            let (sanitized, line_number_alpha) = prepare_scheme_xml(&xml);
            let load_path = if sanitized != xml {
                let cached = styles_dir.join(format!("{}.xml", def.scheme_id));
                match atomic_write_if_changed(&cached, &sanitized, true) {
                    Ok(_) => cached,
                    Err(err) => {
                        result
                            .diagnostics
                            .push(format!("{}: {err}", path.display()));
                        continue;
                    }
                }
            } else {
                path.clone()
            };
            let manager = sourceview5::StyleSchemeManager::new();
            match load_file(&manager, &load_path, &def.scheme_id, &path) {
                Ok(scheme) => result.push(def, Some(scheme), manager, line_number_alpha),
                Err(err) => result.diagnostics.push(err),
            }
        }
        result
    }

    fn push(
        &mut self,
        def: Theme,
        scheme: Option<sourceview5::StyleScheme>,
        manager: sourceview5::StyleSchemeManager,
        line_number_alpha: Option<f32>,
    ) {
        let (palette, warnings) =
            theme_palette::resolve(scheme.as_ref(), def.dark, line_number_alpha);
        self.diagnostics
            .extend(warnings.into_iter().map(|w| format!("{}: {w}", def.name)));
        self.entries.push(Rc::new(LoadedTheme {
            definition: def,
            scheme,
            palette,
            _manager: manager,
        }));
    }

    pub fn by_id(&self, id: &str) -> Option<Rc<LoadedTheme>> {
        self.entries.iter().find(|entry| entry.id == id).cloned()
    }
}

fn load_file(
    manager: &sourceview5::StyleSchemeManager,
    path: &Path,
    id: &str,
    source: &Path,
) -> Result<sourceview5::StyleScheme, String> {
    // Pass the exact file, not its whole directory. System defaults remain available
    // for parent-scheme lookup; other user bundles never enter this manager.
    let absolute = fs::canonicalize(path).map_err(|e| format!("{}: {e}", source.display()))?;
    let filename = absolute
        .to_str()
        .ok_or_else(|| format!("{}: non-UTF-8 path", source.display()))?;
    manager.prepend_search_path(filename);
    let scheme = manager.scheme(id).ok_or_else(|| {
        format!(
            "{}: missing/invalid scheme {id} (including parent resolution)",
            source.display()
        )
    })?;
    let actual = scheme
        .filename()
        .and_then(|p| fs::canonicalize(p.as_str()).ok());
    if actual.as_ref() != Some(&absolute) {
        return Err(format!(
            "{}: scheme {id} resolved from another file; ignored",
            source.display()
        ));
    }
    Ok(scheme)
}

/// Remove a notediff line-number opacity sentinel from the XML before
/// GtkSourceView sees it, and return the requested alpha.
///
/// `auto` and a blank value mean 50%. `NN%` means NN% opacity. Invalid
/// percentages are treated as `auto` so they are never passed to
/// GtkSourceView as invalid colours.
fn prepare_scheme_xml(xml: &str) -> (String, Option<f32>) {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    let mut alpha = None;
    while let Some(start) = find_style_start(rest) {
        let (before, after) = rest.split_at(start);
        out.push_str(before);
        let Some(end) = after.find('>') else {
            out.push_str(after);
            return (out, alpha);
        };
        let (tag, remaining) = after.split_at(end + 1);
        let is_line_numbers =
            attribute_value(tag, "name").is_some_and(|(value, _)| value == "line-numbers");
        let sentinel = is_line_numbers
            .then(|| attribute_value(tag, "foreground"))
            .flatten()
            .and_then(|(value, range)| parse_line_number_alpha(value).map(|a| (a, range)));
        if let Some((requested, (start, end))) = sentinel {
            alpha = Some(requested);
            out.push_str(&tag[..start]);
            out.push_str(&tag[end..]);
        } else {
            out.push_str(tag);
        }
        rest = remaining;
    }
    out.push_str(rest);
    (out, alpha)
}

fn find_style_start(xml: &str) -> Option<usize> {
    let mut search = 0;
    while let Some(relative) = xml[search..].find("<style") {
        let index = search + relative;
        if xml
            .as_bytes()
            .get(index + "<style".len())
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            return Some(index);
        }
        search = index + "<style".len();
    }
    None
}

/// Find a quoted XML attribute and its full span, including the whitespace
/// immediately before it. The theme schema is simple enough for this targeted
/// scan; style values are not entity-decoded.
fn attribute_value<'a>(tag: &'a str, name: &str) -> Option<(&'a str, (usize, usize))> {
    let bytes = tag.as_bytes();
    let mut search = 0;
    while let Some(relative) = tag[search..].find(name) {
        let index = search + relative;
        let before_ok = index == 0 || bytes[index - 1].is_ascii_whitespace();
        let mut after_name = index + name.len();
        if before_ok {
            while after_name < bytes.len() && bytes[after_name].is_ascii_whitespace() {
                after_name += 1;
            }
            if after_name < bytes.len() && bytes[after_name] == b'=' {
                let mut value_start = after_name + 1;
                while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
                    value_start += 1;
                }
                if value_start < bytes.len()
                    && (bytes[value_start] == b'"' || bytes[value_start] == b'\'')
                {
                    let quote = bytes[value_start];
                    let content_start = value_start + 1;
                    if let Some(content_len) = tag[content_start..].find(quote as char) {
                        let content_end = content_start + content_len;
                        let mut attribute_start = index;
                        while attribute_start > 0
                            && bytes[attribute_start - 1].is_ascii_whitespace()
                        {
                            attribute_start -= 1;
                        }
                        return Some((
                            &tag[content_start..content_end],
                            (attribute_start, content_end + 1),
                        ));
                    }
                }
            }
        }
        search = index + name.len();
    }
    None
}

fn parse_line_number_alpha(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        return Some(0.5);
    }
    let percent = value.strip_suffix('%')?;
    let alpha = percent
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|percent| *percent <= 100)
        .map_or(0.5, |percent| f32::from(percent) / 100.0);
    Some(alpha)
}

/// Called on the existing GTK smoke-test thread, with a temporary root.
#[cfg(test)]
pub(crate) fn smoke_at(root: &Path) {
    use sourceview5::prelude::*;
    let users = root.join("users");
    let bundle = |folder: &str, id: &str, xml: Option<&str>| {
        let dir = users.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("theme.conf"),
            format!("id = {id}\nname = {folder}\ndescription = Test\nmode = dark\n"),
        )
        .unwrap();
        if let Some(xml) = xml {
            fs::write(dir.join("scheme.xml"), xml).unwrap();
        }
    };
    let cola = include_str!("../../examples/themes/cola-n-cream/scheme.xml");
    bundle("00-missing", "cola-n-cream", None);
    bundle("01-good", "cola-n-cream", Some(cola));
    bundle("02-duplicate", "cola-n-cream", Some(cola));
    bundle("03-invalid", "invalid", Some("<not-xml"));
    bundle("04-wrong-id", "wrong", Some(cola));
    bundle(
        "05-inherited",
        "inherited",
        Some(
            r##"<?xml version="1.0"?>
<style-scheme id="user-inherited" name="Inherited" version="1.0" parent-scheme="Adwaita-dark">
<color name="sample" value="#ffcc99"/>
<style name="line-numbers" foreground="sample" background="#101010"/>
<style name="notediff:accent" foreground="sample"/>
</style-scheme>"##,
        ),
    );
    bundle("06-reserved", "dark", Some(cola));
    bundle(
        "07-parent",
        "parent",
        Some(
            r#"<style-scheme id="user-parent" name="Broken parent" version="1.0" parent-scheme="not-a-real-parent"/>"#,
        ),
    );
    bundle(
        "08-review",
        "review",
        Some(
            r##"<style-scheme id="user-review" name="Review overrides" version="1.0">
<style name="text" foreground="#000000" background="#ffffff"/>
<style name="custom:review" foreground="#eeeeaa" background="#123456"/>
<style name="notediff:hunk" use-style="custom:review"/>
<style name="notediff:focused-hunk" foreground="#ffffff"/>
<style name="notediff:focused-word" foreground="#ffffff" background="#ffffff"/>
</style-scheme>"##,
        ),
    );
    bundle(
        "09-auto",
        "auto",
        Some(
            r##"<style-scheme id="user-auto" name="Auto line numbers" version="1.0">
<style name="text" foreground="#ffffff" background="#000000"/>
<style name="current-line-number" foreground="#ffcc99"/>
<style name="line-numbers" foreground="auto" background="#101010"/>
</style-scheme>"##,
        ),
    );
    bundle(
        "10-percent",
        "percent",
        Some(
            r##"<style-scheme id="user-percent" name="Percent line numbers" version="1.0">
<style name="text" foreground="#ffffff" background="#000000"/>
<style name="current-line-number" foreground="#ffcc99"/>
<style name="line-numbers" foreground="70%" background="#101010"/>
</style-scheme>"##,
        ),
    );
    let catalog = Catalog::load(&users, &root.join("styles"));
    assert_eq!(
        catalog
            .entries
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        [
            "dark",
            "light",
            "cola-n-cream",
            "inherited",
            "review",
            "auto",
            "percent"
        ]
    );
    let user = catalog.by_id("cola-n-cream").unwrap();
    assert_eq!(user.name, "01-good");
    assert_eq!(user.palette.preview.background, "#f6ead8");
    assert_eq!(user.scheme.as_ref().unwrap().id(), "user-cola-n-cream");
    let inherited = catalog.by_id("inherited").unwrap();
    let parent = sourceview5::StyleSchemeManager::new()
        .scheme("Adwaita-dark")
        .unwrap();
    let (parent_palette, _) = theme_palette::resolve(Some(&parent), true, None);
    assert_eq!(
        inherited.palette.preview.background,
        parent_palette.preview.background
    );
    assert_eq!(inherited.palette.preview.gutter_background, "#101010");
    assert_eq!(inherited.palette.preview.gutter_text, "#ffcc99");
    assert!(inherited.palette.dirty_header_bg.contains("#ffcc99"));
    let auto = catalog.by_id("auto").unwrap();
    assert_eq!(auto.palette.preview.gutter_background, "#101010");
    assert_eq!(
        auto.palette.preview.gutter_text,
        "rgba(255, 204, 153, 0.50)"
    );
    assert_eq!(
        auto.palette.gutter_fallback.as_deref(),
        Some("rgba(255, 204, 153, 0.50)")
    );
    let percent = catalog.by_id("percent").unwrap();
    assert_eq!(
        percent.palette.preview.gutter_text,
        "rgba(255, 204, 153, 0.70)"
    );
    assert_eq!(
        percent.palette.gutter_fallback.as_deref(),
        Some("rgba(255, 204, 153, 0.70)")
    );
    let review = catalog.by_id("review").unwrap();
    // Effective use-style values are kept exactly, even over a white editor.
    assert_eq!(
        super::theme::rgba(review.palette.hunk.background),
        gtk::gdk::RGBA::parse("#123456").unwrap()
    );
    assert_eq!(
        super::theme::rgba(review.palette.hunk.foreground),
        gtk::gdk::RGBA::parse("#eeeeaa").unwrap()
    );
    for (style, expected) in [
        (&review.palette.word, "#8a5a00"),
        (&review.palette.focused_hunk, "#702020"),
        (&review.palette.focused_word, "#ad3030"),
    ] {
        assert_eq!(
            super::theme::rgba(style.background),
            gtk::gdk::RGBA::parse(expected).unwrap()
        );
        assert_eq!(style.foreground, [1.0; 4]);
    }
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|m| m.contains("notediff:focused-hunk") && m.contains("unset"))
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|m| m.contains("notediff:focused-word") && m.contains("contrast"))
    );
    let messages = catalog.diagnostics.join("\n");
    let (light_review, warnings) = theme_palette::resolve(review.scheme.as_ref(), false, None);
    // Valid explicit XML overrides remain exact even in a different declared mode.
    assert_eq!(light_review.hunk.foreground, review.palette.hunk.foreground);
    assert_eq!(light_review.hunk.background, review.palette.hunk.background);
    // Missing fields/styles and low-contrast pairs use the light standards.
    for (style, expected) in [
        (&light_review.word, "#ffd477"),
        (&light_review.focused_hunk, "#fad6d6"),
        (&light_review.focused_word, "#f39a9a"),
    ] {
        assert_eq!(style.foreground, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            super::theme::rgba(style.background),
            gtk::gdk::RGBA::parse(expected).unwrap()
        );
    }
    assert!(warnings.iter().any(|m| m.contains("contrast")));
    let light = catalog.by_id("light").unwrap();
    for (style, expected) in [
        (&light.palette.hunk, "#fff0c2"),
        (&light.palette.word, "#ffd477"),
        (&light.palette.focused_hunk, "#fad6d6"),
        (&light.palette.focused_word, "#f39a9a"),
    ] {
        assert_eq!(style.foreground, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            super::theme::rgba(style.background),
            gtk::gdk::RGBA::parse(expected).unwrap()
        );
    }
    for fragment in [
        "00-missing",
        "duplicate theme id",
        "03-invalid",
        "04-wrong-id",
        "reserved id",
        "07-parent",
    ] {
        assert!(messages.contains(fragment), "{messages}");
    }
    for theme in &catalog.entries {
        for style in [
            &theme.palette.hunk,
            &theme.palette.word,
            &theme.palette.focused_hunk,
            &theme.palette.focused_word,
        ] {
            let fg = super::theme::rgba(style.foreground);
            let bg = super::theme::rgba(style.background);
            assert!(theme_palette::contrast(&fg, &bg) >= 4.5);
        }
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_style_scheme(theme.scheme.as_ref());
        assert_eq!(buffer.style_scheme(), theme.scheme);
    }
    // The live catalog owns its loaded objects: disk changes wait until restart.
    fs::write(users.join("01-good/scheme.xml"), "broken now").unwrap();
    assert_eq!(user.palette.preview.background, "#f6ead8");
    assert_eq!(user.scheme.as_ref().unwrap().id(), "user-cola-n-cream");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_number_sentinels_are_removed_and_parsed() {
        for (value, alpha) in [
            ("auto", 0.5),
            ("70%", 0.7),
            ("0%", 0.0),
            ("100%", 1.0),
            ("101%", 0.5),
            ("bad%", 0.5),
            ("", 0.5),
        ] {
            let xml = format!(
                r##"<style name="line-numbers" foreground="{value}" background="#101010"/>"##
            );
            let (prepared, parsed) = prepare_scheme_xml(&xml);
            assert_eq!(parsed, Some(alpha), "{value}");
            assert!(!prepared.contains("foreground="), "{value}: {prepared}");
            assert!(prepared.contains(r##"background="#101010""##));
        }
    }

    #[test]
    fn builtin_schemes_are_unchanged() {
        for (_, xml) in SCHEMES {
            let (prepared, alpha) = prepare_scheme_xml(xml);
            assert_eq!(prepared, xml);
            assert_eq!(alpha, None);
        }
    }

    #[test]
    fn explicit_line_number_colours_and_other_styles_are_untouched() {
        let xml = r##"<style name="line-numbers" foreground="#ffcc99" background="#101010"/>
<style name="current-line-number" foreground="auto"/>"##;
        let (prepared, alpha) = prepare_scheme_xml(xml);
        assert_eq!(alpha, None);
        assert_eq!(prepared, xml);
    }

    #[test]
    fn sentinel_scan_handles_attribute_order_quotes_and_line_breaks() {
        let xml = r##"<style
    name="line-numbers"
    background="#101010"
    foreground='70%'
/>"##;
        let (prepared, alpha) = prepare_scheme_xml(xml);
        assert_eq!(alpha, Some(0.7));
        assert!(!prepared.contains("foreground="));
        assert!(prepared.contains("background=\"#101010\""));
        assert!(prepared.contains("name=\"line-numbers\""));
    }
}
