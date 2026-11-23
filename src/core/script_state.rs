//! Persistent state for the Run Script dialog (`$XDG_DATA_HOME/notediff/script-state`).
//!
//! Stores the last-selected script path and per-script parameter drafts,
//! overridden sandbox mode, and chosen writable folder as plain text. Nothing
//! is read at startup; state is loaded when the Run Script dialog opens and
//! written atomically only when changed.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::fsutil::{atomic_write_if_changed, data_home, read_utf8_limited};
use super::scripts::SandboxDefault;

/// Refuse unexpectedly large state files.
const MAX_STATE_BYTES: u64 = 1024 * 1024;

/// Maximum number of scripts retained in the state file.
pub const MAX_SAVED_SCRIPTS: usize = 64;

/// A persisted field value.
///
/// Scalar, multiline, list, integer, boolean, choice and slider fields are
/// stored as [`SavedFieldValue::Text`] (preserving raw buffer text and
/// distinguishing an explicitly cleared field from an unset default). Multi-path
/// fields (`type = paths`) are stored as [`SavedFieldValue::List`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavedFieldValue {
    Text(String),
    List(Vec<String>),
}

/// Persisted overrides for a single script.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptParams {
    pub sandbox: Option<SandboxDefault>,
    pub writable_dir: Option<PathBuf>,
    pub fields: BTreeMap<String, SavedFieldValue>,
}

impl ScriptParams {
    pub fn is_empty(&self) -> bool {
        self.sandbox.is_none() && self.writable_dir.is_none() && self.fields.is_empty()
    }
}

/// Top-level Run Script dialog state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptDialogState {
    pub selected_script: Option<PathBuf>,
    pub scripts: BTreeMap<PathBuf, ScriptParams>,
}

impl ScriptDialogState {
    /// Remove any script entries that hold no overrides.
    pub fn prune_empty(&mut self) {
        self.scripts.retain(|_, params| !params.is_empty());
    }

    /// True when nothing remains to persist.
    pub fn is_empty(&self) -> bool {
        self.selected_script
            .as_deref()
            .is_none_or(|p| !rememberable_path(p))
            && self.scripts.values().all(ScriptParams::is_empty)
    }

    /// Update or remove a script's saved parameters while keeping the map bounded.
    pub fn set_script_params(&mut self, key: PathBuf, params: ScriptParams) {
        if params.is_empty() || !rememberable_path(&key) {
            self.scripts.remove(&key);
            return;
        }
        if !self.scripts.contains_key(&key) && self.scripts.len() >= MAX_SAVED_SCRIPTS {
            // Drop the first non-selected key to stay within the cap.
            if let Some(evict) = self
                .scripts
                .keys()
                .find(|k| self.selected_script.as_ref() != Some(*k))
                .cloned()
            {
                self.scripts.remove(&evict);
            }
        }
        self.scripts.insert(key, params);
    }
}

/// Resolve a script path to its canonical key, falling back to the original path
/// if canonicalization fails (for example, if a file was removed).
pub fn canonical_script_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Handle for reading and writing the Run Script state file.
#[derive(Clone, Debug)]
pub struct ScriptStateStore {
    path: Option<PathBuf>,
}

impl Default for ScriptStateStore {
    fn default() -> Self {
        Self::at(Self::default_path())
    }
}

impl ScriptStateStore {
    pub fn default_path() -> PathBuf {
        data_home().join("notediff").join("script-state")
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn load(&self) -> ScriptDialogState {
        let Some(path) = &self.path else {
            return ScriptDialogState::default();
        };
        read_utf8_limited(path, MAX_STATE_BYTES)
            .map(|text| parse(&text))
            .unwrap_or_default()
    }

    /// Persist `state`. Write failures are ignored because script parameter
    /// history is disposable state. When nothing remains to save, the state
    /// file is removed.
    pub fn save(&self, state: &ScriptDialogState) {
        let Some(path) = &self.path else {
            return;
        };
        if state.is_empty() {
            let _ = fs::remove_file(path);
            return;
        }
        let rendered = render(state);
        let _ = atomic_write_if_changed(path, &rendered, true);
    }
}

fn rememberable_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

fn sandbox_to_str(sandbox: SandboxDefault) -> &'static str {
    match sandbox {
        SandboxDefault::ReadOnly => "read-only",
        SandboxDefault::WritableDir => "writable-dir",
        SandboxDefault::None => "none",
    }
}

fn parse_sandbox(raw: &str) -> Option<SandboxDefault> {
    match raw {
        "read-only" => Some(SandboxDefault::ReadOnly),
        "writable-dir" => Some(SandboxDefault::WritableDir),
        "none" => Some(SandboxDefault::None),
        _ => None,
    }
}

fn escape_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => {}
            other => out.push(other),
        }
    }
    out
}

fn unescape_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Split `key = value`, stripping at most one space after `=` so empty values
/// and leading/trailing whitespace in saved field text round-trip losslessly.
fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let val = rest.strip_prefix(' ').unwrap_or(rest);
    Some((key, val))
}

pub fn parse(text: &str) -> ScriptDialogState {
    let mut state = ScriptDialogState::default();
    let mut current_script: Option<PathBuf> = None;

    for raw_line in text.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(script_str) = line
            .strip_prefix("[script ")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let path = PathBuf::from(script_str);
            if rememberable_path(&path)
                && (state.scripts.contains_key(&path) || state.scripts.len() < MAX_SAVED_SCRIPTS)
            {
                current_script = Some(path);
            } else {
                current_script = None;
            }
            continue;
        }

        let Some((key, val)) = split_key_value(line) else {
            continue;
        };

        let Some(ref script_path) = current_script else {
            if key == "selected" {
                let path = PathBuf::from(val);
                if rememberable_path(&path) {
                    state.selected_script = Some(path);
                }
            }
            continue;
        };

        let entry = state.scripts.entry(script_path.clone()).or_default();
        if key == "sandbox" {
            if let Some(mode) = parse_sandbox(val.trim()) {
                entry.sandbox = Some(mode);
            }
        } else if key == "writable_dir" {
            let path = PathBuf::from(val);
            if rememberable_path(&path) {
                entry.writable_dir = Some(path);
            }
        } else if let Some(field_id) = key.strip_prefix("field.")
            && !field_id.is_empty()
        {
            entry.fields.insert(
                field_id.to_string(),
                SavedFieldValue::Text(unescape_value(val)),
            );
        } else if let Some(field_id) = key.strip_prefix("item.")
            && !field_id.is_empty()
        {
            let item = unescape_value(val);
            if !item.is_empty() {
                match entry.fields.get_mut(field_id) {
                    Some(SavedFieldValue::List(items)) => items.push(item),
                    _ => {
                        entry
                            .fields
                            .insert(field_id.to_string(), SavedFieldValue::List(vec![item]));
                    }
                }
            }
        }
    }

    state.prune_empty();
    state
}

pub fn render(state: &ScriptDialogState) -> String {
    let mut out = String::from("# notediff script state (written by notediff)\n");
    if let Some(ref selected) = state.selected_script
        && rememberable_path(selected)
        && let Some(s) = selected.to_str()
    {
        out.push_str("selected = ");
        out.push_str(s);
        out.push('\n');
    }

    let mut written = 0usize;
    for (path, params) in &state.scripts {
        if written >= MAX_SAVED_SCRIPTS {
            break;
        }
        if params.is_empty() || !rememberable_path(path) {
            continue;
        }
        let Some(path_str) = path.to_str() else {
            continue;
        };
        out.push('\n');
        out.push_str("[script ");
        out.push_str(path_str);
        out.push_str("]\n");

        if let Some(sandbox) = params.sandbox {
            out.push_str("sandbox = ");
            out.push_str(sandbox_to_str(sandbox));
            out.push('\n');
        }
        if let Some(ref dir) = params.writable_dir
            && rememberable_path(dir)
            && let Some(dir_str) = dir.to_str()
        {
            out.push_str("writable_dir = ");
            out.push_str(dir_str);
            out.push('\n');
        }
        for (id, value) in &params.fields {
            match value {
                SavedFieldValue::Text(text) => {
                    out.push_str("field.");
                    out.push_str(id);
                    out.push_str(" = ");
                    out.push_str(&escape_value(text));
                    out.push('\n');
                }
                SavedFieldValue::List(items) => {
                    for item in items {
                        if !item.is_empty() {
                            out.push_str("item.");
                            out.push_str(id);
                            out.push_str(" = ");
                            out.push_str(&escape_value(item));
                            out.push('\n');
                        }
                    }
                }
            }
        }
        written += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-script-state-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrips_all_value_shapes_losslessly() {
        let mut state = ScriptDialogState {
            selected_script: Some(PathBuf::from("/home/user/scripts/prompt [v2].py")),
            scripts: BTreeMap::new(),
        };

        let mut params = ScriptParams {
            sandbox: Some(SandboxDefault::WritableDir),
            writable_dir: Some(PathBuf::from("/home/user/work/my project")),
            fields: BTreeMap::new(),
        };
        params
            .fields
            .insert("empty".into(), SavedFieldValue::Text(String::new()));
        params.fields.insert(
            "padded".into(),
            SavedFieldValue::Text("  keep spaces ".into()),
        );
        params.fields.insert(
            "multiline".into(),
            SavedFieldValue::Text("line1\nline2\\slash\tindented\r\nend".into()),
        );
        params
            .fields
            .insert("verbose".into(), SavedFieldValue::Text("true".into()));
        params
            .fields
            .insert("count".into(), SavedFieldValue::Text("42".into()));
        params.fields.insert(
            "paths".into(),
            SavedFieldValue::List(vec![
                "/home/user/a.md".into(),
                "/home/user/dir with spaces/b.rs".into(),
            ]),
        );

        state.set_script_params(PathBuf::from("/home/user/scripts/prompt [v2].py"), params);

        let rendered = render(&state);
        let parsed = parse(&rendered);
        assert_eq!(parsed, state);
    }

    #[test]
    fn malformed_lines_do_not_drop_valid_entries() {
        let text = "\
# comment
garbage without equals
selected = relative/not-allowed.py
selected = /valid/selected.py

[script relative/bad.py]
field.x = ignored

[script /valid/selected.py]
sandbox = bogus
sandbox = none
writable_dir = relative/dir
writable_dir = /valid/out
field.name = hello
unknown_key = whatever
item.paths = /one
item.paths = /two
";
        let parsed = parse(text);
        assert_eq!(
            parsed.selected_script,
            Some(PathBuf::from("/valid/selected.py"))
        );
        let params = parsed.scripts.get(Path::new("/valid/selected.py")).unwrap();
        assert_eq!(params.sandbox, Some(SandboxDefault::None));
        assert_eq!(params.writable_dir, Some(PathBuf::from("/valid/out")));
        assert_eq!(
            params.fields.get("name"),
            Some(&SavedFieldValue::Text("hello".into()))
        );
        assert_eq!(
            params.fields.get("paths"),
            Some(&SavedFieldValue::List(vec!["/one".into(), "/two".into()]))
        );
    }

    #[test]
    fn store_saves_loads_prunes_and_removes_when_empty() {
        let dir = temp_dir();
        let path = dir.join("nested/script-state");
        let store = ScriptStateStore::at(path.clone());

        assert_eq!(store.load(), ScriptDialogState::default());

        let mut state = ScriptDialogState {
            selected_script: Some(PathBuf::from("/tmp/a.py")),
            scripts: BTreeMap::new(),
        };
        let mut params = ScriptParams::default();
        params
            .fields
            .insert("q".into(), SavedFieldValue::Text("needle".into()));
        state.set_script_params(PathBuf::from("/tmp/a.py"), params);
        store.save(&state);

        assert!(path.exists());
        assert_eq!(store.load(), state);

        // Clearing everything deletes the state file.
        store.save(&ScriptDialogState::default());
        assert!(!path.exists());

        // Oversized files are ignored safely.
        fs::write(&path, format!("# {}\n", "x".repeat(1100 * 1024))).unwrap();
        assert_eq!(store.load(), ScriptDialogState::default());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn enforces_max_saved_scripts_cap() {
        let mut state = ScriptDialogState::default();
        for i in 0..(MAX_SAVED_SCRIPTS + 10) {
            let mut params = ScriptParams::default();
            params
                .fields
                .insert("k".into(), SavedFieldValue::Text(i.to_string()));
            state.set_script_params(PathBuf::from(format!("/scripts/s-{i:03}.py")), params);
        }
        assert_eq!(state.scripts.len(), MAX_SAVED_SCRIPTS);
    }
}
