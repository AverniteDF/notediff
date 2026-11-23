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

/// Maximum number of named presets retained per script.
pub const MAX_PRESETS_PER_SCRIPT: usize = 32;

/// Maximum length of a preset name in Unicode scalar values.
pub const MAX_PRESET_NAME_CHARS: usize = 64;

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
    pub presets: BTreeMap<PathBuf, BTreeMap<String, ScriptParams>>,
}

impl ScriptDialogState {
    /// Remove any script draft entries that hold no overrides, and any empty
    /// per-script preset maps.
    pub fn prune_empty(&mut self) {
        self.scripts.retain(|_, params| !params.is_empty());
        self.presets.retain(|_, map| !map.is_empty());
    }

    /// True when nothing remains to persist.
    pub fn is_empty(&self) -> bool {
        self.selected_script
            .as_deref()
            .is_none_or(|p| !rememberable_path(p))
            && self.scripts.values().all(ScriptParams::is_empty)
            && self.presets.values().all(BTreeMap::is_empty)
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

    /// Named presets saved for `key`, sorted alphabetically by name.
    pub fn presets_for(&self, key: &Path) -> Option<&BTreeMap<String, ScriptParams>> {
        self.presets.get(key).filter(|m| !m.is_empty())
    }

    /// Save (or overwrite) a named preset for `key`.
    /// Returns `Ok(true)` if an existing preset with the same name was overwritten,
    /// or `Ok(false)` if a new preset was created.
    pub fn save_preset(
        &mut self,
        key: PathBuf,
        raw_name: &str,
        params: ScriptParams,
    ) -> Result<bool, &'static str> {
        let Some(name) = validate_preset_name(raw_name) else {
            return Err("Enter a preset name (up to 64 characters).");
        };
        if !rememberable_path(&key) {
            return Err("Could not save preset for this script path.");
        }
        let map = self.presets.entry(key).or_default();
        let existed = map.contains_key(&name);
        if !existed && map.len() >= MAX_PRESETS_PER_SCRIPT {
            return Err("A script can have at most 32 saved presets.");
        }
        map.insert(name, params);
        Ok(existed)
    }

    /// Delete a named preset for `key`. Returns `true` if one was removed.
    pub fn delete_preset(&mut self, key: &Path, name: &str) -> bool {
        let Some(map) = self.presets.get_mut(key) else {
            return false;
        };
        let removed = map.remove(name).is_some();
        if map.is_empty() {
            self.presets.remove(key);
        }
        removed
    }
}

/// Normalize and validate a user-supplied preset name.
pub fn validate_preset_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_PRESET_NAME_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return None;
    }
    Some(trimmed.to_string())
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

enum ParseSection {
    Top,
    Ignored,
    Draft(PathBuf),
    Preset {
        script: PathBuf,
        name: Option<String>,
        params: ScriptParams,
    },
}

fn apply_param_entry(entry: &mut ScriptParams, key: &str, val: &str) {
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

fn flush_preset_section(state: &mut ScriptDialogState, section: ParseSection) {
    if let ParseSection::Preset {
        script,
        name: Some(name),
        params,
    } = section
    {
        let _ = state.save_preset(script, &name, params);
    }
}

pub fn parse(text: &str) -> ScriptDialogState {
    let mut state = ScriptDialogState::default();
    let mut section = ParseSection::Top;

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
            flush_preset_section(&mut state, section);
            let path = PathBuf::from(script_str);
            if rememberable_path(&path)
                && (state.scripts.contains_key(&path) || state.scripts.len() < MAX_SAVED_SCRIPTS)
            {
                section = ParseSection::Draft(path);
            } else {
                section = ParseSection::Ignored;
            }
            continue;
        }

        if let Some(script_str) = line
            .strip_prefix("[preset ")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            flush_preset_section(&mut state, section);
            let path = PathBuf::from(script_str);
            if rememberable_path(&path) {
                section = ParseSection::Preset {
                    script: path,
                    name: None,
                    params: ScriptParams::default(),
                };
            } else {
                section = ParseSection::Ignored;
            }
            continue;
        }

        let Some((key, val)) = split_key_value(line) else {
            continue;
        };

        match &mut section {
            ParseSection::Top => {
                if key == "selected" {
                    let path = PathBuf::from(val);
                    if rememberable_path(&path) {
                        state.selected_script = Some(path);
                    }
                }
            }
            ParseSection::Ignored => {}
            ParseSection::Draft(script_path) => {
                let entry = state.scripts.entry(script_path.clone()).or_default();
                apply_param_entry(entry, key, val);
            }
            ParseSection::Preset { name, params, .. } => {
                if key == "name" {
                    if let Some(valid) = validate_preset_name(&unescape_value(val)) {
                        *name = Some(valid);
                    }
                } else {
                    apply_param_entry(params, key, val);
                }
            }
        }
    }

    flush_preset_section(&mut state, section);
    state.prune_empty();
    state
}

fn render_params(out: &mut String, params: &ScriptParams) {
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
        render_params(&mut out, params);
        written += 1;
    }

    for (path, preset_map) in &state.presets {
        if preset_map.is_empty() || !rememberable_path(path) {
            continue;
        }
        let Some(path_str) = path.to_str() else {
            continue;
        };
        for (name, params) in preset_map.iter().take(MAX_PRESETS_PER_SCRIPT) {
            let Some(valid_name) = validate_preset_name(name) else {
                continue;
            };
            out.push('\n');
            out.push_str("[preset ");
            out.push_str(path_str);
            out.push_str("]\n");
            out.push_str("name = ");
            out.push_str(&escape_value(&valid_name));
            out.push('\n');
            render_params(&mut out, params);
        }
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
            presets: BTreeMap::new(),
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
            presets: BTreeMap::new(),
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

    #[test]
    fn presets_roundtrip_isolate_per_script_overwrite_and_delete() {
        let mut state = ScriptDialogState::default();
        let script_a = PathBuf::from("/scripts/a.py");
        let script_b = PathBuf::from("/scripts/b.py");

        let mut p1 = ScriptParams::default();
        p1.fields
            .insert("q".into(), SavedFieldValue::Text("alpha".into()));
        assert_eq!(
            state.save_preset(script_a.clone(), "Work | [1]", p1.clone()),
            Ok(false)
        );
        // Empty (all-defaults) preset is preserved.
        assert_eq!(
            state.save_preset(script_a.clone(), "Defaults", ScriptParams::default()),
            Ok(false)
        );
        // Same preset name on another script is independent.
        let mut p2 = ScriptParams::default();
        p2.fields
            .insert("q".into(), SavedFieldValue::Text("beta".into()));
        assert_eq!(
            state.save_preset(script_b.clone(), "Work | [1]", p2.clone()),
            Ok(false)
        );

        let parsed = parse(&render(&state));
        assert_eq!(parsed, state);
        assert!(!parsed.is_empty());

        // Overwrite existing preset on script_a without touching script_b.
        let mut p1_updated = ScriptParams::default();
        p1_updated
            .fields
            .insert("q".into(), SavedFieldValue::Text("alpha-2".into()));
        assert_eq!(
            state.save_preset(script_a.clone(), "  Work | [1] ", p1_updated.clone()),
            Ok(true)
        );
        assert_eq!(
            state.presets_for(&script_a).unwrap().get("Work | [1]"),
            Some(&p1_updated)
        );
        assert_eq!(
            state.presets_for(&script_b).unwrap().get("Work | [1]"),
            Some(&p2)
        );

        // Delete presets and verify empty outer map is pruned.
        assert!(state.delete_preset(&script_b, "Work | [1]"));
        assert!(!state.delete_preset(&script_b, "Work | [1]"));
        assert!(state.presets_for(&script_b).is_none());
        assert!(!state.presets.contains_key(&script_b));

        // Invalid names and cap enforcement.
        assert!(
            state
                .save_preset(script_a.clone(), "   ", ScriptParams::default())
                .is_err()
        );
        assert!(
            state
                .save_preset(script_a.clone(), "bad\nname", ScriptParams::default())
                .is_err()
        );
        for i in 0..MAX_PRESETS_PER_SCRIPT {
            let _ = state.save_preset(
                script_b.clone(),
                &format!("preset-{i:02}"),
                ScriptParams::default(),
            );
        }
        assert!(
            state
                .save_preset(script_b.clone(), "overflow", ScriptParams::default())
                .is_err()
        );
    }
}
