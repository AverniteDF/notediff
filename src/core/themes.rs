//! Owned theme metadata, strict user descriptors and remembered selection (no GTK).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::fsutil::{atomic_write, data_home};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub description: String,
    pub dark: bool,
    pub scheme_id: String,
}

pub fn builtins() -> Vec<Theme> {
    [
        (
            "dark",
            "notediff Dark",
            "Soft-contrast dark theme (default)",
            true,
        ),
        (
            "light",
            "notediff Light",
            "Warm off-white with soft, low-contrast accents",
            false,
        ),
    ]
    .into_iter()
    .map(|(id, name, description, dark)| Theme {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        dark,
        scheme_id: format!("notediff-{id}"),
    })
    .collect()
}

pub fn default_path() -> PathBuf {
    data_home().join("notediff/theme")
}

pub fn user_directory() -> PathBuf {
    data_home().join("notediff/themes")
}

/// Four required keys, once each; comments occupy whole lines. No quoting/escapes.
/// IDs are ASCII slugs; both built-in IDs and their namespace remain reserved.
pub fn descriptor(text: &str) -> Result<Theme, String> {
    let mut fields = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected key = value", index + 1))?;
        let (key, value) = (key.trim(), value.trim());
        if !["id", "name", "description", "mode"].contains(&key) {
            return Err(format!("unknown key: {key}"));
        }
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(format!("empty or control-containing value for {key}"));
        }
        if fields.insert(key, value).is_some() {
            return Err(format!("duplicate key: {key}"));
        }
    }
    let get = |key| {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing key: {key}"))
    };
    let id = get("id")?;
    if id.len() > 64
        || !id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || matches!(id, "dark" | "light")
        || id.starts_with("notediff-")
    {
        return Err(format!("invalid or reserved id: {id}"));
    }
    let dark = match get("mode")? {
        "dark" => true,
        "light" => false,
        _ => return Err("mode must be dark or light".into()),
    };
    Ok(Theme {
        id: id.into(),
        name: get("name")?.into(),
        description: get("description")?.into(),
        dark,
        scheme_id: format!("user-{id}"),
    })
}

/// Immediate subdirectories only, sorted by filename (case-sensitive OS ordering).
/// Missing root is normal. Surface all other scan/descriptor failures.
/// Duplicate IDs are decided later, after XML validation: first loadable wins.
pub fn discover(root: &Path) -> (Vec<(Theme, PathBuf)>, Vec<String>) {
    let mut diagnostics = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (Vec::new(), diagnostics),
        Err(err) => return (Vec::new(), vec![format!("{}: {err}", root.display())]),
    };
    let mut dirs = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => match entry.file_type() {
                Ok(kind) if kind.is_dir() => dirs.push(entry.path()),
                Ok(_) => {} // No symlink directories or loose files.
                Err(err) => diagnostics.push(format!("{}: {err}", entry.path().display())),
            },
            Err(err) => diagnostics.push(format!("{}: {err}", root.display())),
        }
    }
    dirs.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let mut themes = Vec::new();
    for dir in dirs {
        let path = dir.join("theme.conf");
        let result = fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| descriptor(&text));
        match result {
            Ok(theme) => themes.push((theme, dir.join("scheme.xml"))),
            Err(err) => diagnostics.push(format!("{}: {err}", path.display())),
        }
    }
    (themes, diagnostics)
}

/// Last syntactically valid theme line wins; resolve against the loaded catalog.
pub fn selected_id(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "theme" && !value.trim().is_empty()).then(|| value.trim().to_owned())
        })
        .next_back()
}

pub fn save_selected_to(path: &Path, theme: &Theme) -> io::Result<()> {
    atomic_write(
        path,
        &format!(
            "# notediff theme (written by notediff)\ntheme = {}\n",
            theme.id
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "id = midnight\nname = Midnight\ndescription = Deep blue\nmode = dark\n";

    #[test]
    fn descriptors_are_strict_and_owned() {
        let theme = descriptor(VALID).unwrap();
        assert_eq!(theme.scheme_id, "user-midnight");
        for invalid in [
            VALID.replace("midnight", "dark"),
            VALID.replace("midnight", "light"),
            VALID.replace("midnight", "notediff-custom"),
            VALID.replace("midnight", "../x"),
            VALID.replace("mode = dark", "mode = auto"),
            format!("{VALID}id = second\n"),
            format!("{VALID}palette = red\n"),
            VALID.replace("name = Midnight", "name = "),
            VALID.replace("mode = dark\n", ""),
        ] {
            assert!(descriptor(&invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            builtins().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["dark", "light"]
        );
        assert_eq!(
            selected_id("theme = light\ntheme = midnight\n").as_deref(),
            Some("midnight")
        );
    }

    #[test]
    fn scan_orders_candidates_and_reports_bad_descriptors() {
        let dir = std::env::temp_dir().join(format!("notediff-theme-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(discover(&dir).0.is_empty());
        for name in ["z-last", "a-first", "broken"] {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(
                dir.join(name).join("theme.conf"),
                if name == "broken" { "id = dark" } else { VALID },
            )
            .unwrap();
        }
        let (found, diagnostics) = discover(&dir);
        assert_eq!(found.len(), 2); // Keep duplicates until scheme validation.
        assert_eq!(found[0].1, dir.join("a-first/scheme.xml"));
        assert_eq!(found[1].1, dir.join("z-last/scheme.xml"));
        assert_eq!(diagnostics.len(), 1);
        let path = dir.join("state/theme");
        save_selected_to(&path, &found[0].0).unwrap();
        assert_eq!(
            selected_id(&fs::read_to_string(path).unwrap()).as_deref(),
            Some("midnight")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
