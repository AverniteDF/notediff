//! Toolkit-free, app-written content font preferences.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::fsutil::{atomic_write, data_home};

pub const DEFAULT_FONT_FAMILY: &str = "JetBrains Mono, monospace";
pub const DEFAULT_FONT_SIZE_PT: u32 = 12;
pub const MIN_FONT_SIZE_PT: u32 = 6;
pub const MAX_FONT_SIZE_PT: u32 = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Appearance {
    font_family: String,
    font_size_pt: u32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_family: DEFAULT_FONT_FAMILY.into(),
            font_size_pt: DEFAULT_FONT_SIZE_PT,
        }
    }
}

impl Appearance {
    /// A comma-separated list of unquoted family names, safe for CSS and state files.
    pub fn new(font_family: &str, font_size_pt: u32) -> Result<Self, &'static str> {
        let font_family = font_family.trim();
        if !valid_font_family(font_family) {
            return Err(
                "Use comma-separated font names containing letters, numbers, spaces, hyphens or underscores.",
            );
        }
        if !(MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&font_size_pt) {
            return Err("Font size must be a whole number from 6 to 48 pt.");
        }
        Ok(Self {
            font_family: font_family.into(),
            font_size_pt,
        })
    }

    pub fn font_family(&self) -> &str {
        &self.font_family
    }

    pub fn font_size_pt(&self) -> u32 {
        self.font_size_pt
    }

    pub fn default_path() -> PathBuf {
        data_home().join("notediff").join("appearance")
    }

    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Invalid lines do not erase a previous valid value of either field.
    pub fn parse(text: &str) -> Self {
        let mut value = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, raw)) = line.split_once('=') else {
                continue;
            };
            let raw = raw.trim();
            match key.trim() {
                "font_family" if valid_font_family(raw) => value.font_family = raw.into(),
                "font_size_pt" => {
                    if let Ok(size) = raw.parse::<u32>()
                        && (MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&size)
                    {
                        value.font_size_pt = size;
                    }
                }
                _ => {}
            }
        }
        value
    }

    pub fn render(&self) -> String {
        format!(
            "# notediff appearance (written by notediff)\nfont_family = {}\nfont_size_pt = {}\n",
            self.font_family, self.font_size_pt
        )
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        atomic_write(path, &self.render(), true)
    }
}

fn valid_font_family(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | ','))
        && value.split(',').all(|name| {
            name.trim()
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn malformed_values_do_not_erase_valid_values() {
        let value = Appearance::parse(
            "font_family = Fira Code, monospace\nfont_size_pt = 18\nfont_family = a; color: red\nfont_size_pt = 999\nunknown = x\n",
        );
        assert_eq!(value, Appearance::new("Fira Code, monospace", 18).unwrap());
        for bad in [
            "",
            "a,b,",
            "x; color:red",
            "'Hack'",
            "A/*x*/",
            "A\nB",
            "123",
            "A\\B",
        ] {
            assert!(Appearance::new(bad, 12).is_err(), "{bad:?}");
        }
        for bad in [0, 5, 49, u32::MAX] {
            assert!(Appearance::new("monospace", bad).is_err());
        }
        assert_eq!(
            Appearance::parse("font_size_pt = 12.5\n"),
            Appearance::default()
        );
        assert_eq!(
            Appearance::parse("# comment\ngarbage\n"),
            Appearance::default()
        );
    }

    #[test]
    fn roundtrip_and_file_errors() {
        let dir = std::env::temp_dir().join(format!(
            "notediff-appearance-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = dir.join("nested/appearance");
        assert_eq!(Appearance::load_from(&path), Appearance::default());
        let value = Appearance::new("Hack, monospace", 48).unwrap();
        assert_eq!(Appearance::parse(&value.render()), value);
        value.save_to(&path).unwrap();
        assert_eq!(Appearance::load_from(&path), value);
        assert!(value.save_to(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
