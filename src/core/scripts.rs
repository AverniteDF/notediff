//! Script metadata parsing, discovery, invocation building and sandboxed execution (no GTK).

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::MAX_OPEN_BYTES;
use super::fsutil::data_home;

/// Metadata blocks are read from the top of the script file, bounded so large
/// scripts do not waste I/O during discovery.
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Cap captured stderr so a verbose script cannot exhaust memory.
const MAX_STDERR_BYTES: usize = 256 * 1024;

const START_MARKER: &str = "--- notediff-script ---";
const END_MARKER: &str = "--- end-notediff-script ---";

/// Default user scripts directory (`$XDG_DATA_HOME/notediff/scripts`).
pub fn scripts_directory() -> PathBuf {
    data_home().join("notediff/scripts")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SandboxDefault {
    #[default]
    ReadOnly,
    WritableDir,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxMode {
    ReadOnly,
    WritableDir(PathBuf),
    None,
}

impl SandboxMode {
    pub fn as_env_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WritableDir(_) => "writable-dir",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    Text,
    Multiline,
    Int,
    Bool,
    Choice,
    Slider,
    File,
    Dir,
    Paths,
    List,
}

impl FieldType {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "text" => Ok(Self::Text),
            "multiline" => Ok(Self::Multiline),
            "int" => Ok(Self::Int),
            "bool" => Ok(Self::Bool),
            "choice" => Ok(Self::Choice),
            "slider" => Ok(Self::Slider),
            "file" => Ok(Self::File),
            "dir" => Ok(Self::Dir),
            "paths" => Ok(Self::Paths),
            "list" => Ok(Self::List),
            other => Err(format!("unknown field type: {other}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub id: String,
    pub label: String,
    pub field_type: FieldType,
    pub default: Option<String>,
    pub placeholder: Option<String>,
    pub help: Option<String>,
    pub required: bool,
    pub arg: Option<String>,
    pub positional: bool,
    pub options: Vec<String>,
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptMetadata {
    pub name: String,
    pub description: String,
    pub sandbox: SandboxDefault,
    pub output_ext: String,
    pub fields: Vec<FieldSpec>,
}

#[derive(Clone, Debug)]
pub struct DiscoveredScript {
    pub path: PathBuf,
    pub executable: bool,
    pub metadata: Result<ScriptMetadata, String>,
}

impl DiscoveredScript {
    pub fn display_name(&self) -> String {
        match &self.metadata {
            Ok(meta) => meta.name.clone(),
            Err(_) => default_script_name(&self.path),
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Int(i64),
    Bool(bool),
    List(Vec<String>),
}

impl FieldValue {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(s) => s.trim().is_empty(),
            Self::Int(_) | Self::Bool(_) => false,
            Self::List(items) => items.iter().all(|item| item.trim().is_empty()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InvocationContext {
    pub current_file: Option<PathBuf>,
    pub current_dir: Option<PathBuf>,
    pub sandbox: SandboxMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptRunError {
    NotExecutable(PathBuf),
    MissingSandboxTool,
    InvalidWritableDir(PathBuf),
    SpawnFailed(String),
    Cancelled,
    OutputTooLarge,
    InvalidUtf8,
    ContainsNul,
    NonZeroExit { code: Option<i32>, stderr: String },
}

impl ScriptRunError {
    pub fn heading(&self) -> &'static str {
        match self {
            Self::NotExecutable(_) => "Script is not executable",
            Self::MissingSandboxTool => "Sandbox unavailable",
            Self::InvalidWritableDir(_) => "Invalid writable folder",
            Self::SpawnFailed(_) => "Could not run script",
            Self::Cancelled => "Script cancelled",
            Self::OutputTooLarge => "Script output too large",
            Self::InvalidUtf8 | Self::ContainsNul => "Unsupported script output",
            Self::NonZeroExit { .. } => "Script failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotExecutable(path) => format!(
                "{} does not have execute permission.\n\nRun chmod +x on it first.",
                path.display()
            ),
            Self::MissingSandboxTool => {
                "bubblewrap (bwrap) was not found in PATH. Install it or choose No Sandbox."
                    .to_string()
            }
            Self::InvalidWritableDir(path) => {
                format!("{} is not an existing directory.", path.display())
            }
            Self::SpawnFailed(err) => err.clone(),
            Self::Cancelled => "The script was cancelled.".to_string(),
            Self::OutputTooLarge => {
                "Script output exceeded the 16 MiB document limit and was stopped.".to_string()
            }
            Self::InvalidUtf8 => {
                "Script output is not valid UTF-8. notediff only opens UTF-8 plain text."
                    .to_string()
            }
            Self::ContainsNul => {
                "Script output contains a NUL byte (binary data) and cannot be opened.".to_string()
            }
            Self::NonZeroExit { code, stderr } => {
                let status = match code {
                    Some(c) => format!("The script exited with status {c}."),
                    None => "The script was terminated by a signal.".to_string(),
                };
                let trimmed = stderr.trim();
                if trimmed.is_empty() {
                    status
                } else {
                    format!("{status}\n\n{trimmed}")
                }
            }
        }
    }
}

/// Cooperative cancellation handle for a running script.
#[derive(Clone, Default)]
pub struct CancelHandle {
    cancelled: Arc<AtomicBool>,
}

impl CancelHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Default display name derived from a script's filename stem.
pub fn default_script_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "script".to_string())
}

/// Format the timestamped untitled document name for script output.
pub fn format_output_filename(script_path: &Path, output_ext: &str, timestamp: &str) -> String {
    let raw_stem = default_script_name(script_path);
    let stem: String = raw_stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    let stem = if stem.is_empty() { "script" } else { stem };
    let ext = output_ext.trim_start_matches('.');
    if ext.is_empty() {
        format!("{stem}-{timestamp}")
    } else {
        format!("{stem}-{timestamp}.{ext}")
    }
}

/// Extract a concise single-line summary from stderr for a success toast.
pub fn stderr_toast_summary(stderr: &str) -> Option<String> {
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut chars = line.chars();
    let truncated: String = chars.by_ref().take(140).collect();
    if chars.next().is_some() {
        Some(format!("{truncated}…"))
    } else {
        Some(truncated)
    }
}

/// Check whether `bwrap` (bubblewrap) is installed and able to create a
/// user/mount namespace in this environment. Cached once per process.
pub fn bwrap_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Some(bwrap) = find_in_path("bwrap") else {
            return false;
        };
        Command::new(bwrap)
            .args([
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--",
                "/bin/true",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    for dir in env::split_paths(&paths) {
        let candidate = dir.join(binary);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && (meta.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

/// Inspect a single script file on disk: checks executable bit and parses its
/// optional `--- notediff-script ---` metadata header.
pub fn inspect_script(path: &Path) -> DiscoveredScript {
    let executable = is_executable_file(path);
    let metadata = read_header_prefix(path)
        .map_err(|err| err.to_string())
        .and_then(|text| parse_metadata(&text, path));
    DiscoveredScript {
        path: path.to_path_buf(),
        executable,
        metadata,
    }
}

fn read_header_prefix(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.by_ref()
        .take(MAX_HEADER_BYTES as u64)
        .read_to_end(&mut buf)?;
    // Truncate at a valid UTF-8 boundary if the 64 KiB slice split a multibyte char.
    while !buf.is_empty() {
        match std::str::from_utf8(&buf) {
            Ok(s) => return Ok(s.to_owned()),
            Err(err) if err.error_len().is_none() => {
                let valid = err.valid_up_to();
                buf.truncate(valid);
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "script header is not valid UTF-8",
                ));
            }
        }
    }
    Ok(String::new())
}

/// Discover scripts in `root`. Missing `root` is normal and returns an empty list.
pub fn discover(root: &Path) -> (Vec<DiscoveredScript>, Vec<String>) {
    let mut diagnostics = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (Vec::new(), diagnostics),
        Err(err) => return (Vec::new(), vec![format!("{}: {err}", root.display())]),
    };

    let mut scripts = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                diagnostics.push(format!("{}: {err}", root.display()));
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let is_file = fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false);
        if !is_file {
            continue;
        }
        scripts.push(inspect_script(&path));
    }

    scripts.sort_by(|a, b| {
        a.display_name()
            .to_lowercase()
            .cmp(&b.display_name().to_lowercase())
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });
    (scripts, diagnostics)
}

/// Detect a supported comment prefix (`#`, `//`, or `--`) on a marker line.
fn detect_comment_prefix(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    for prefix in ["#", "//", "--"] {
        if let Some(rest) = trimmed.strip_prefix(prefix)
            && rest.trim() == START_MARKER
        {
            return Some(prefix);
        }
    }
    None
}

/// Parse an optional `--- notediff-script ---` metadata block from `text`.
/// A script without a marker block returns default metadata with no fields.
pub fn parse_metadata(text: &str, script_path: &Path) -> Result<ScriptMetadata, String> {
    let mut lines = text.lines().enumerate();
    let mut prefix: Option<&'static str> = None;
    let mut start_line = 0usize;

    for (index, line) in lines.by_ref() {
        if let Some(found) = detect_comment_prefix(line) {
            prefix = Some(found);
            start_line = index + 1;
            break;
        }
    }

    let Some(prefix) = prefix else {
        return Ok(ScriptMetadata {
            name: default_script_name(script_path),
            description: String::new(),
            sandbox: SandboxDefault::ReadOnly,
            output_ext: "txt".to_string(),
            fields: Vec::new(),
        });
    };

    let mut header_keys: BTreeMap<String, String> = BTreeMap::new();
    let mut field_order: Vec<String> = Vec::new();
    let mut field_maps: BTreeMap<String, (usize, BTreeMap<String, String>)> = BTreeMap::new();
    let mut field_env_keys = BTreeSet::new();
    let mut current_field: Option<String> = None;
    let mut closed = false;

    for (index, raw_line) in lines {
        let line_no = index + 1;
        let trimmed = raw_line.trim_start();
        let Some(body) = trimmed.strip_prefix(prefix) else {
            return Err(format!(
                "line {line_no}: expected comment prefix `{prefix}` inside metadata block"
            ));
        };
        let content = body.trim();
        if content == END_MARKER {
            closed = true;
            break;
        }
        if content.is_empty() || content.starts_with('#') {
            continue;
        }
        if let Some(section) = content.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let section = section.trim();
            let Some(id) = section.strip_prefix("field.") else {
                return Err(format!(
                    "line {line_no}: unknown section `[{section}]` (expected `[field.<id>]`)"
                ));
            };
            validate_field_id(id, line_no)?;
            if field_maps.contains_key(id) {
                return Err(format!("line {line_no}: duplicate field id `{id}`"));
            }
            let env_key = input_env_key(id);
            if !field_env_keys.insert(env_key.clone()) {
                return Err(format!(
                    "line {line_no}: field id `{id}` maps to duplicate environment variable `{env_key}`"
                ));
            }
            field_order.push(id.to_string());
            field_maps.insert(id.to_string(), (line_no, BTreeMap::new()));
            current_field = Some(id.to_string());
            continue;
        }

        let Some((key, value)) = content.split_once('=') else {
            return Err(format!("line {line_no}: expected `key = value`"));
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            return Err(format!("line {line_no}: empty key"));
        }
        if value.chars().any(char::is_control) {
            return Err(format!(
                "line {line_no}: control characters are not allowed in `{key}`"
            ));
        }

        if let Some(ref field_id) = current_field {
            const FIELD_KEYS: &[&str] = &[
                "label",
                "type",
                "default",
                "placeholder",
                "help",
                "required",
                "arg",
                "positional",
                "options",
                "min",
                "max",
            ];
            if !FIELD_KEYS.contains(&key) {
                return Err(format!(
                    "line {line_no}: unknown key `{key}` in [field.{field_id}]"
                ));
            }
            let (_, map) = field_maps.get_mut(field_id).unwrap();
            if map.insert(key.to_string(), value.to_string()).is_some() {
                return Err(format!(
                    "line {line_no}: duplicate key `{key}` in [field.{field_id}]"
                ));
            }
        } else {
            const HEADER_KEYS: &[&str] = &["name", "description", "sandbox", "output_ext"];
            if !HEADER_KEYS.contains(&key) {
                return Err(format!("line {line_no}: unknown header key `{key}`"));
            }
            if header_keys
                .insert(key.to_string(), value.to_string())
                .is_some()
            {
                return Err(format!("line {line_no}: duplicate header key `{key}`"));
            }
        }
    }

    if !closed {
        return Err(format!(
            "line {start_line}: unclosed `{START_MARKER}` block (missing `{END_MARKER}`)"
        ));
    }

    let name = header_keys
        .get("name")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| default_script_name(script_path));
    let description = header_keys.get("description").cloned().unwrap_or_default();
    let sandbox = match header_keys.get("sandbox").map(String::as_str) {
        None | Some("read-only") => SandboxDefault::ReadOnly,
        Some("writable-dir") => SandboxDefault::WritableDir,
        Some("none") => SandboxDefault::None,
        Some(other) => {
            return Err(format!(
                "invalid sandbox value `{other}` (expected read-only, writable-dir, or none)"
            ));
        }
    };
    let output_ext = match header_keys.get("output_ext") {
        None => "txt".to_string(),
        Some(ext) => {
            let ext = ext.trim_start_matches('.');
            if ext.len() > 16 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(format!("invalid output_ext `{ext}`"));
            }
            ext.to_string()
        }
    };

    let mut fields = Vec::with_capacity(field_order.len());
    for id in field_order {
        let (section_line, map) = field_maps.remove(&id).unwrap();
        let field_type = match map.get("type") {
            Some(raw) => FieldType::parse(raw)
                .map_err(|err| format!("line {section_line} ([field.{id}]): {err}"))?,
            None => {
                return Err(format!(
                    "line {section_line} ([field.{id}]): missing required key `type`"
                ));
            }
        };
        let label = map
            .get("label")
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| id.clone());
        let required = parse_bool_opt(map.get("required"), "required", section_line, &id)?;
        let positional = parse_bool_opt(map.get("positional"), "positional", section_line, &id)?;
        let arg = map.get("arg").filter(|s| !s.is_empty()).cloned();
        if positional && arg.is_some() {
            return Err(format!(
                "line {section_line} ([field.{id}]): `arg` and `positional = true` cannot both be set"
            ));
        }
        let min = parse_i64_opt(map.get("min"), "min", section_line, &id)?;
        let max = parse_i64_opt(map.get("max"), "max", section_line, &id)?;
        if let (Some(lo), Some(hi)) = (min, max)
            && lo > hi
        {
            return Err(format!(
                "line {section_line} ([field.{id}]): `min` ({lo}) is greater than `max` ({hi})"
            ));
        }
        let options = match map.get("options") {
            Some(raw) => parse_options(raw),
            None => Vec::new(),
        };
        if matches!(field_type, FieldType::Choice | FieldType::Slider) && options.is_empty() {
            return Err(format!(
                "line {section_line} ([field.{id}]): `{}` field requires non-empty `options`",
                if field_type == FieldType::Choice {
                    "choice"
                } else {
                    "slider"
                }
            ));
        }
        let default = map.get("default").cloned();
        if field_type == FieldType::Int
            && let Some(ref def) = default
            && !def.is_empty()
            && def.parse::<i64>().is_err()
        {
            return Err(format!(
                "line {section_line} ([field.{id}]): invalid integer default `{def}`"
            ));
        }
        if field_type == FieldType::Bool
            && let Some(ref def) = default
            && !matches!(def.as_str(), "true" | "false" | "1" | "0" | "")
        {
            return Err(format!(
                "line {section_line} ([field.{id}]): invalid boolean default `{def}`"
            ));
        }

        fields.push(FieldSpec {
            id,
            label,
            field_type,
            default,
            placeholder: map.get("placeholder").filter(|s| !s.is_empty()).cloned(),
            help: map.get("help").filter(|s| !s.is_empty()).cloned(),
            required,
            arg,
            positional,
            options,
            min,
            max,
        });
    }

    Ok(ScriptMetadata {
        name,
        description,
        sandbox,
        output_ext,
        fields,
    })
}

fn validate_field_id(id: &str, line_no: usize) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(format!("line {line_no}: invalid field id `{id}`"));
    }
    Ok(())
}

fn parse_bool_opt(
    raw: Option<&String>,
    key: &str,
    line_no: usize,
    field_id: &str,
) -> Result<bool, String> {
    match raw.map(String::as_str) {
        None | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(other) => Err(format!(
            "line {line_no} ([field.{field_id}]): invalid boolean `{other}` for `{key}`"
        )),
    }
}

fn parse_i64_opt(
    raw: Option<&String>,
    key: &str,
    line_no: usize,
    field_id: &str,
) -> Result<Option<i64>, String> {
    match raw {
        None => Ok(None),
        Some(s) => s.parse::<i64>().map(Some).map_err(|_| {
            format!("line {line_no} ([field.{field_id}]): invalid integer `{s}` for `{key}`")
        }),
    }
}

fn parse_options(raw: &str) -> Vec<String> {
    let sep = if raw.contains('|') { '|' } else { ',' };
    let mut seen = BTreeSet::new();
    raw.split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty() && seen.insert((*s).to_string()))
        .map(ToOwned::to_owned)
        .collect()
}

/// Split a `list` field's raw text into individual items (newline or comma separated).
pub fn parse_list_items(raw: &str) -> Vec<String> {
    raw.split(['\n', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Build CLI arguments and `NOTEDIFF_*` environment variables for a script run.
pub fn build_invocation(
    metadata: &ScriptMetadata,
    values: &[(String, FieldValue)],
    context: &InvocationContext,
) -> (Vec<String>, Vec<(String, String)>) {
    let value_map: BTreeMap<&str, &FieldValue> =
        values.iter().map(|(k, v)| (k.as_str(), v)).collect();

    let mut flag_args = Vec::new();
    let mut positional_args = Vec::new();
    let mut env_vars = Vec::new();

    for field in &metadata.fields {
        let Some(value) = value_map.get(field.id.as_str()) else {
            continue;
        };

        let env_key = input_env_key(&field.id);
        match value {
            FieldValue::Text(text) => {
                env_vars.push((env_key, text.clone()));
                if !text.is_empty() {
                    if let Some(ref arg) = field.arg {
                        push_flag_value(&mut flag_args, arg, text);
                    } else if field.positional {
                        positional_args.push(text.clone());
                    }
                }
            }
            FieldValue::Int(n) => {
                let s = n.to_string();
                env_vars.push((env_key, s.clone()));
                if let Some(ref arg) = field.arg {
                    push_flag_value(&mut flag_args, arg, &s);
                } else if field.positional {
                    positional_args.push(s);
                }
            }
            FieldValue::Bool(b) => {
                env_vars.push((env_key, if *b { "1" } else { "0" }.to_string()));
                if *b {
                    if let Some(ref arg) = field.arg {
                        flag_args.push(arg.clone());
                    } else if field.positional {
                        positional_args.push("true".to_string());
                    }
                }
            }
            FieldValue::List(items) => {
                let cleaned: Vec<String> = items
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                env_vars.push((env_key, cleaned.join("\n")));
                for item in cleaned {
                    if let Some(ref arg) = field.arg {
                        push_flag_value(&mut flag_args, arg, &item);
                    } else if field.positional {
                        positional_args.push(item);
                    }
                }
            }
        }
    }

    if let Some(ref file) = context.current_file {
        env_vars.push((
            "NOTEDIFF_CURRENT_FILE".to_string(),
            file.to_string_lossy().into_owned(),
        ));
    }
    if let Some(ref dir) = context.current_dir {
        env_vars.push((
            "NOTEDIFF_CURRENT_DIR".to_string(),
            dir.to_string_lossy().into_owned(),
        ));
    }
    env_vars.push((
        "NOTEDIFF_SANDBOX".to_string(),
        context.sandbox.as_env_str().to_string(),
    ));
    if let SandboxMode::WritableDir(ref dir) = context.sandbox {
        env_vars.push((
            "NOTEDIFF_WRITABLE_DIR".to_string(),
            dir.to_string_lossy().into_owned(),
        ));
    }

    flag_args.extend(positional_args);
    (flag_args, env_vars)
}

fn push_flag_value(args: &mut Vec<String>, flag: &str, value: &str) {
    if flag.ends_with('=') {
        args.push(format!("{flag}{value}"));
    } else {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
}

fn input_env_key(id: &str) -> String {
    format!(
        "NOTEDIFF_INPUT_{}",
        id.to_ascii_uppercase().replace('-', "_")
    )
}

/// Build the `std::process::Command` for running `script_path` under `sandbox`.
pub fn build_command(
    script_path: &Path,
    args: &[String],
    env_vars: &[(String, String)],
    context: &InvocationContext,
) -> Result<Command, ScriptRunError> {
    if !is_executable_file(script_path) {
        return Err(ScriptRunError::NotExecutable(script_path.to_path_buf()));
    }

    let mut cmd = match &context.sandbox {
        SandboxMode::None => {
            let mut cmd = Command::new(script_path);
            cmd.args(args);
            cmd
        }
        SandboxMode::ReadOnly => {
            let bwrap = find_in_path("bwrap").ok_or(ScriptRunError::MissingSandboxTool)?;
            let mut cmd = Command::new(bwrap);
            cmd.args([
                "--die-with-parent",
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--",
            ]);
            cmd.arg(script_path);
            cmd.args(args);
            cmd
        }
        SandboxMode::WritableDir(dir) => {
            let bwrap = find_in_path("bwrap").ok_or(ScriptRunError::MissingSandboxTool)?;
            if !dir.is_dir() {
                return Err(ScriptRunError::InvalidWritableDir(dir.clone()));
            }
            let mut cmd = Command::new(bwrap);
            cmd.args([
                "--die-with-parent",
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--tmpfs",
                "/tmp",
                "--bind",
            ]);
            cmd.arg(dir);
            cmd.arg(dir);
            cmd.arg("--");
            cmd.arg(script_path);
            cmd.args(args);
            cmd
        }
    };

    if let Some(ref dir) = context.current_dir
        && dir.is_dir()
    {
        cmd.current_dir(dir);
    }
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    Ok(cmd)
}

/// Execute `script_path` synchronously (call from a background thread).
/// Enforces the 16 MiB stdout limit, UTF-8/NUL validation, and cooperative cancellation.
pub fn run_script(
    script_path: &Path,
    args: &[String],
    env_vars: &[(String, String)],
    context: &InvocationContext,
    cancel: &CancelHandle,
) -> Result<ScriptOutput, ScriptRunError> {
    if cancel.is_cancelled() {
        return Err(ScriptRunError::Cancelled);
    }
    let mut cmd = build_command(script_path, args, env_vars, context)?;
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|err| ScriptRunError::SpawnFailed(err.to_string()))?;

    let pid = child.id() as libc::pid_t;
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let mut out = Capture::new(MAX_OPEN_BYTES as usize);
    let mut err = Capture::new(MAX_STDERR_BYTES);
    let collected = collect_output(pid, &mut stdout, &mut stderr, &mut out, &mut err, cancel);
    if collected.is_err() || !out.closed || !err.closed {
        kill_group(pid);
        let _ = child.kill();
    }
    let status = child.wait();
    collected?;
    let status = status.map_err(run_io_error)?;

    if cancel.is_cancelled() {
        return Err(ScriptRunError::Cancelled);
    }

    let mut stderr = String::from_utf8_lossy(&err.bytes).replace('\0', "\u{fffd}");
    if err.truncated {
        stderr.push_str("\n… (stderr truncated)");
    }

    if !status.success() {
        return Err(ScriptRunError::NonZeroExit {
            code: status.code(),
            stderr,
        });
    }

    if out.bytes.contains(&0) {
        return Err(ScriptRunError::ContainsNul);
    }

    let stdout = String::from_utf8(out.bytes).map_err(|_| ScriptRunError::InvalidUtf8)?;
    Ok(ScriptOutput { stdout, stderr })
}

fn run_io_error(err: io::Error) -> ScriptRunError {
    ScriptRunError::SpawnFailed(err.to_string())
}

fn collect_output(
    pid: libc::pid_t,
    stdout: &mut (impl Read + AsRawFd),
    stderr: &mut (impl Read + AsRawFd),
    out: &mut Capture,
    err: &mut Capture,
    cancel: &CancelHandle,
) -> Result<(), ScriptRunError> {
    let stdout_fd = stdout.as_raw_fd();
    let stderr_fd = stderr.as_raw_fd();
    for fd in [stdout_fd, stderr_fd] {
        // SAFETY: the descriptors are owned by the live pipe objects above.
        if unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) } == -1 {
            return Err(run_io_error(io::Error::last_os_error()));
        }
    }
    let mut drain_deadline = None;
    loop {
        if cancel.is_cancelled() {
            return Err(ScriptRunError::Cancelled);
        }
        let read_stdout = out.drain(stdout).map_err(run_io_error)?;
        if out.truncated {
            return Err(ScriptRunError::OutputTooLarge);
        }
        let read_stderr = err.drain(stderr).map_err(run_io_error)?;
        if drain_deadline.is_none() && child_exited(pid).map_err(run_io_error)? {
            if out.closed && err.closed {
                return Ok(());
            }
            // Do not let a background descendant keep the run alive. Give
            // buffered output a final bounded drain even if a pipe stays open.
            kill_group(pid);
            drain_deadline = Some(std::time::Instant::now() + Duration::from_millis(250));
        }
        if let Some(deadline) = drain_deadline
            && ((out.closed && err.closed) || std::time::Instant::now() >= deadline)
        {
            return Ok(());
        }
        if !read_stdout && !read_stderr {
            let mut fds = [
                libc::pollfd {
                    fd: if out.closed { -1 } else { stdout_fd },
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if err.closed { -1 } else { stderr_fd },
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: poll receives a writable array of two descriptors tied
            // to the live pipes. The timeout also bounds cancellation latency.
            if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 20) } == -1 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(run_io_error(error));
                }
            }
        }
    }
}

fn child_exited(pid: libc::pid_t) -> io::Result<bool> {
    // WNOWAIT keeps the leader's PID reserved until group cleanup is finished.
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: info is writable storage for waitid, and pid is our child.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == 0 {
        // SAFETY: on success waitid populates the child status (PID zero when
        // no child has exited); the storage was zero initialized.
        Ok(unsafe { info.assume_init().si_pid() } != 0)
    } else {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(err)
        }
    }
}

fn kill_group(pid: libc::pid_t) {
    // SAFETY: a positive PID from spawn was made its own process-group ID.
    unsafe { libc::kill(-pid, libc::SIGKILL) };
}

struct Capture {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
    closed: bool,
}

impl Capture {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
            closed: false,
        }
    }

    fn drain(&mut self, reader: &mut impl Read) -> io::Result<bool> {
        let mut progress = false;
        let mut chunk = [0u8; 8192];
        if !self.closed {
            // Keep stdout, stderr, child status, and cancellation all moving
            // even when one stream produces data continuously.
            for _ in 0..32 {
                match reader.read(&mut chunk) {
                    Ok(0) => {
                        self.closed = true;
                        break;
                    }
                    Ok(n) => {
                        progress = true;
                        let keep = n.min(self.limit - self.bytes.len());
                        self.bytes.extend_from_slice(&chunk[..keep]);
                        self.truncated |= keep < n;
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Err(err),
                }
            }
        }
        Ok(progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "notediff-scripts-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_exec(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn parses_all_field_types_and_comment_prefixes() {
        let script = r#"#!/usr/bin/env python3
# --- notediff-script ---
# name = Sample Tool
# description = Demonstrates all field types
# sandbox = writable-dir
# output_ext = md
#
# [field.title]
# label = Title
# type = text
# required = true
# arg = --title
# default = Hello
# placeholder = Enter title
#
# [field.notes]
# label = Notes
# type = multiline
# arg = --notes=
#
# [field.count]
# label = Count
# type = int
# arg = -n
# default = 10
# min = 1
# max = 50
#
# [field.verbose]
# label = Verbose
# type = bool
# arg = -v
# default = true
#
# [field.format]
# label = Format
# type = choice
# arg = --format
# options = markdown | json | plain
# default = markdown
#
# [field.limit]
# label = Limit
# type = slider
# arg = --limit
# options = 1K | 4K | 16K
# default = 4K
#
# [field.config]
# label = Config File
# type = file
# arg = --config
#
# [field.workdir]
# label = Work Dir
# type = dir
# arg = --dir
#
# [field.excludes]
# label = Excludes
# type = list
# arg = -x
#
# [field.targets]
# label = Targets
# type = paths
# required = true
# positional = true
# --- end-notediff-script ---
print("ok")
"#;
        let meta = parse_metadata(script, Path::new("/tmp/sample.py")).unwrap();
        assert_eq!(meta.name, "Sample Tool");
        assert_eq!(meta.description, "Demonstrates all field types");
        assert_eq!(meta.sandbox, SandboxDefault::WritableDir);
        assert_eq!(meta.output_ext, "md");
        assert_eq!(meta.fields.len(), 10);
        assert_eq!(meta.fields[4].options, vec!["markdown", "json", "plain"]);
        assert_eq!(meta.fields[5].field_type, FieldType::Slider);
        assert_eq!(meta.fields[5].options, vec!["1K", "4K", "16K"]);
        assert_eq!(meta.fields[2].min, Some(1));
        assert_eq!(meta.fields[2].max, Some(50));
        assert!(meta.fields[9].positional);
        assert!(meta.fields[9].required);

        // Also supports `//` and `--` prefixes.
        let js = "// --- notediff-script ---\n// name = JS Tool\n// --- end-notediff-script ---\n";
        assert_eq!(
            parse_metadata(js, Path::new("tool.js")).unwrap().name,
            "JS Tool"
        );
        let lua =
            "-- --- notediff-script ---\n-- name = Lua Tool\n-- --- end-notediff-script ---\n";
        assert_eq!(
            parse_metadata(lua, Path::new("tool.lua")).unwrap().name,
            "Lua Tool"
        );
    }

    #[test]
    fn script_without_metadata_uses_defaults() {
        let meta = parse_metadata("#!/bin/sh\necho hi\n", Path::new("/tmp/hello.sh")).unwrap();
        assert_eq!(meta.name, "hello");
        assert_eq!(meta.sandbox, SandboxDefault::ReadOnly);
        assert_eq!(meta.output_ext, "txt");
        assert!(meta.fields.is_empty());
    }

    #[test]
    fn rejects_invalid_metadata() {
        let p = Path::new("bad.py");
        assert!(parse_metadata("# --- notediff-script ---\n# name = X\n", p).is_err());
        assert!(
            parse_metadata(
                "# --- notediff-script ---\n# unknown = 1\n# --- end-notediff-script ---\n",
                p
            )
            .is_err()
        );
        assert!(
            parse_metadata(
                "# --- notediff-script ---\n# [field.a]\n# type = choice\n# --- end-notediff-script ---\n",
                p
            )
            .is_err()
        );
        assert!(
            parse_metadata(
                "# --- notediff-script ---\n# [field.a]\n# type = slider\n# --- end-notediff-script ---\n",
                p
            )
            .is_err()
        );
        assert!(
            parse_metadata(
                "# --- notediff-script ---\n# [field.a]\n# type = text\n# arg = -a\n# positional = true\n# --- end-notediff-script ---\n",
                p
            )
            .is_err()
        );
        assert!(
            parse_metadata(
                "# --- notediff-script ---\n# [field.a]\n# type = int\n# min = 10\n# max = 5\n# --- end-notediff-script ---\n",
                p
            )
            .is_err()
        );
    }

    #[test]
    fn builds_cli_args_and_env_vars() {
        let script = r#"
# --- notediff-script ---
# name = Test
# [field.prompt]
# type = text
# arg = --prompt
# [field.style]
# type = text
# arg = --style=
# [field.verbose]
# type = bool
# arg = -v
# [field.exclude]
# type = list
# arg = -x
# [field.paths]
# type = paths
# positional = true
# --- end-notediff-script ---
"#;
        let meta = parse_metadata(script, Path::new("t.py")).unwrap();
        let values = vec![
            ("prompt".into(), FieldValue::Text("hello".into())),
            ("style".into(), FieldValue::Text("short".into())),
            ("verbose".into(), FieldValue::Bool(true)),
            (
                "exclude".into(),
                FieldValue::List(vec!["*.tmp".into(), "dist".into()]),
            ),
            (
                "paths".into(),
                FieldValue::List(vec!["/a".into(), "/b".into()]),
            ),
        ];
        let ctx = InvocationContext {
            current_file: Some(PathBuf::from("/work/doc.md")),
            current_dir: Some(PathBuf::from("/work")),
            sandbox: SandboxMode::ReadOnly,
        };
        let (args, env_vars) = build_invocation(&meta, &values, &ctx);
        assert_eq!(
            args,
            vec![
                "--prompt",
                "hello",
                "--style=short",
                "-v",
                "-x",
                "*.tmp",
                "-x",
                "dist",
                "/a",
                "/b"
            ]
        );
        assert!(env_vars.contains(&("NOTEDIFF_INPUT_PROMPT".into(), "hello".into())));
        assert!(env_vars.contains(&("NOTEDIFF_INPUT_VERBOSE".into(), "1".into())));
        assert!(env_vars.contains(&("NOTEDIFF_CURRENT_FILE".into(), "/work/doc.md".into())));
        assert!(env_vars.contains(&("NOTEDIFF_SANDBOX".into(), "read-only".into())));
    }

    #[test]
    fn runs_script_captures_stdout_stderr_and_enforces_limits() {
        let dir = temp_dir("exec");
        let ok_script = dir.join("ok.sh");
        write_exec(
            &ok_script,
            "#!/bin/sh\necho \"hello $1\"\necho \"status line\" >&2\n",
        );
        let ctx = InvocationContext {
            current_file: None,
            current_dir: Some(dir.clone()),
            sandbox: SandboxMode::None,
        };
        let res = run_script(
            &ok_script,
            &["world".into()],
            &[],
            &ctx,
            &CancelHandle::new(),
        )
        .unwrap();
        assert_eq!(res.stdout, "hello world\n");
        assert_eq!(res.stderr, "status line\n");
        assert_eq!(
            stderr_toast_summary(&res.stderr).as_deref(),
            Some("status line")
        );

        // Non-zero exit returns stderr.
        let fail_script = dir.join("fail.sh");
        write_exec(&fail_script, "#!/bin/sh\necho 'bad input' >&2\nexit 2\n");
        let err = run_script(&fail_script, &[], &[], &ctx, &CancelHandle::new()).unwrap_err();
        assert_eq!(
            err,
            ScriptRunError::NonZeroExit {
                code: Some(2),
                stderr: "bad input\n".into()
            }
        );

        // NUL byte is refused.
        let nul_script = dir.join("nul.sh");
        write_exec(&nul_script, "#!/bin/sh\nprintf 'a\\0b'\n");
        assert_eq!(
            run_script(&nul_script, &[], &[], &ctx, &CancelHandle::new()).unwrap_err(),
            ScriptRunError::ContainsNul
        );

        let large_script = dir.join("large.sh");
        write_exec(
            &large_script,
            &format!("#!/bin/sh\nhead -c {} /dev/zero\n", MAX_OPEN_BYTES + 1),
        );
        assert_eq!(
            run_script(&large_script, &[], &[], &ctx, &CancelHandle::new()).unwrap_err(),
            ScriptRunError::OutputTooLarge
        );

        // Sandbox read-only test when bwrap is available.
        if bwrap_available() {
            let write_script = dir.join("try_write.sh");
            let target = dir.join("should_not_exist.txt");
            write_exec(
                &write_script,
                &format!("#!/bin/sh\necho hi > \"{}\"\n", target.display()),
            );
            let ro_ctx = InvocationContext {
                current_file: None,
                current_dir: Some(dir.clone()),
                sandbox: SandboxMode::ReadOnly,
            };
            let err = run_script(&write_script, &[], &[], &ro_ctx, &CancelHandle::new());
            assert!(err.is_err(), "read-only sandbox must block writes");
            assert!(!target.exists());

            let rw_ctx = InvocationContext {
                current_file: None,
                current_dir: Some(dir.clone()),
                sandbox: SandboxMode::WritableDir(dir.clone()),
            };
            run_script(&write_script, &[], &[], &rw_ctx, &CancelHandle::new()).unwrap();
            assert!(target.exists());
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn field_ids_must_map_to_distinct_environment_variables() {
        let header = "\
# --- notediff-script ---\n\
# [field.some-name]\n\
# type = text\n\
# [field.SOME_NAME]\n\
# type = text\n\
# --- end-notediff-script ---\n";
        let err = parse_metadata(header, Path::new("test.sh")).unwrap_err();
        assert!(err.contains("duplicate environment variable `NOTEDIFF_INPUT_SOME_NAME`"));
    }

    #[test]
    fn runner_finishes_when_a_descendant_holds_the_pipes_open() {
        let dir = temp_dir("inherited-pipes");
        let script = dir.join("test.sh");
        let ctx = InvocationContext {
            current_file: None,
            current_dir: Some(dir.clone()),
            sandbox: SandboxMode::None,
        };
        write_exec(&script, "#!/bin/sh\nsleep 5 &\nprintf 'complete\\n'\n");
        let start = std::time::Instant::now();
        let result = run_script(&script, &[], &[], &ctx, &CancelHandle::new()).unwrap();
        assert_eq!(result.stdout, "complete\n");
        assert!(start.elapsed() < Duration::from_secs(3));

        write_exec(&script, "#!/bin/sh\nsleep 5 &\ntouch started\nwait\n");
        let cancel = CancelHandle::new();
        let worker_cancel = cancel.clone();
        let worker = thread::spawn(move || run_script(&script, &[], &[], &ctx, &worker_cancel));
        let start = std::time::Instant::now();
        while !dir.join("started").exists() {
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(5));
        }
        let start = std::time::Instant::now();
        cancel.cancel();
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            ScriptRunError::Cancelled
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_and_sorts_scripts() {
        let dir = temp_dir("discover");
        write_exec(
            &dir.join("z.sh"),
            "#!/bin/sh\n# --- notediff-script ---\n# name = Alpha\n# --- end-notediff-script ---\n",
        );
        write_exec(
            &dir.join("a.sh"),
            "#!/bin/sh\n# --- notediff-script ---\n# name = Beta\n# --- end-notediff-script ---\n",
        );
        fs::write(dir.join(".hidden.sh"), "#!/bin/sh\n").unwrap();
        let (found, diags) = discover(&dir);
        assert!(diags.is_empty());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].display_name(), "Alpha");
        assert_eq!(found[1].display_name(), "Beta");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_example_scripts_parse_cleanly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/scripts");
        let (found, diags) = discover(&root);
        assert!(diags.is_empty(), "diagnostics: {diags:?}");
        assert!(
            found.len() >= 2,
            "expected shipped example scripts in {}",
            root.display()
        );
        for script in &found {
            assert!(
                script.executable,
                "{} should be executable",
                script.path.display()
            );
            let meta = script
                .metadata
                .as_ref()
                .unwrap_or_else(|err| panic!("{} failed to parse: {err}", script.path.display()));
            assert!(!meta.fields.is_empty());
        }
    }
}
