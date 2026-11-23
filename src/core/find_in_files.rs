//! Pure-Rust recursive Find in Files engine (no GTK).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::MAX_OPEN_BYTES;
use super::fsutil::read_utf8_limited;
use super::textlines;

/// Default maximum number of matches collected before stopping.
pub const DEFAULT_MAX_MATCHES: usize = 1_000;

/// Directories always skipped during recursive folder traversal unless
/// explicitly passed as the search root.
const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "target",
    "dist",
    "build",
    "venv",
    ".venv",
    "__pycache__",
];

/// Maximum preview line length (in characters) stored per match.
const MAX_PREVIEW_CHARS: usize = 240;

#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub query: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub max_matches: usize,
    /// Live in-memory text of dirty open documents keyed by canonical path.
    /// Searched instead of the on-disk file so unsaved edits are reflected.
    pub open_buffers: HashMap<PathBuf, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    pub path: PathBuf,
    pub rel_path: String,
    /// 1-based line number matching GTK and `core::textlines`.
    pub line: usize,
    /// 0-based character offset of the match on `line`.
    pub column_char: usize,
    /// Character length of the matched query.
    pub match_chars: usize,
    /// Trimmed preview snippet of the line.
    pub preview: String,
    /// Byte range `(start, end)` of the match within `preview` for highlighting.
    pub preview_highlight: Option<(usize, usize)>,
    /// True when this match came from an unsaved in-memory buffer rather than disk.
    pub from_unsaved_buffer: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SearchReport {
    pub matches: Vec<FileMatch>,
    pub files_searched: usize,
    pub files_matched: usize,
    pub truncated: bool,
    pub cancelled: bool,
}

/// Parse a comma- or whitespace-separated glob list from a UI entry.
pub fn parse_globs(raw: &str) -> Vec<String> {
    raw.split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Simple glob matcher supporting `*` (any sequence) and `?` (any single char).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (None::<usize>, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = Some(pi);
            pi += 1;
            star_t = ti;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn matches_any_glob(globs: &[String], name: &str, rel_path: &str) -> bool {
    globs
        .iter()
        .any(|pat| glob_match(pat, name) || glob_match(pat, rel_path))
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Find all non-overlapping literal matches of `query` in `line`, returning
/// `(char_offset, char_len)` pairs.
pub fn find_line_matches(
    line: &str,
    query: &str,
    match_case: bool,
    whole_word: bool,
) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let line_chars: Vec<char> = line.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    let qlen = query_chars.len();
    if qlen > line_chars.len() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut i = 0usize;
    while i + qlen <= line_chars.len() {
        let matched = if match_case {
            line_chars[i..i + qlen] == query_chars[..]
        } else {
            line_chars[i..i + qlen]
                .iter()
                .zip(query_chars.iter())
                .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
        };
        if matched {
            let boundary_ok = !whole_word
                || ((i == 0 || !is_word_char(line_chars[i - 1]))
                    && (i + qlen == line_chars.len() || !is_word_char(line_chars[i + qlen])));
            if boundary_ok {
                out.push((i, qlen));
                i += qlen;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Build a compact preview snippet around the match at `[col_char .. col_char + match_chars]`,
/// returning `(preview_text, Some((hl_start_byte, hl_end_byte)))`.
fn build_preview(
    line: &str,
    col_char: usize,
    match_chars: usize,
) -> (String, Option<(usize, usize)>) {
    let chars: Vec<char> = line.chars().collect();
    let lead_trim = chars
        .iter()
        .take_while(|c| **c == ' ' || **c == '\t')
        .count();
    let tail_trim = chars
        .iter()
        .rev()
        .take_while(|c| **c == ' ' || **c == '\t')
        .count();
    let trim_end = chars.len().saturating_sub(tail_trim).max(lead_trim);

    let (slice_start, slice_end, prefix_ellipsis, suffix_ellipsis) =
        if trim_end - lead_trim <= MAX_PREVIEW_CHARS {
            (lead_trim, trim_end, false, false)
        } else {
            let center_start = col_char.saturating_sub(MAX_PREVIEW_CHARS / 3);
            let start = center_start.max(lead_trim);
            let end = (start + MAX_PREVIEW_CHARS).min(trim_end);
            (start, end, start > lead_trim, end < trim_end)
        };

    let mut preview = String::new();
    if prefix_ellipsis {
        preview.push('…');
    }
    let mut hl_start = None;
    let mut hl_end = None;
    let match_end = col_char + match_chars;
    for (idx, &ch) in chars[slice_start..slice_end].iter().enumerate() {
        let abs = slice_start + idx;
        if abs == col_char {
            hl_start = Some(preview.len());
        }
        if ch == '\t' {
            preview.push(' ');
        } else {
            preview.push(ch);
        }
        if abs + 1 == match_end {
            hl_end = Some(preview.len());
        }
    }
    if suffix_ellipsis {
        preview.push('…');
    }

    let highlight = match (hl_start, hl_end) {
        (Some(s), Some(e)) if s < e => Some((s, e)),
        _ => None,
    };
    (preview, highlight)
}

/// Recursively search `options.root` for `options.query`.
pub fn search(options: &SearchOptions, cancel: &Arc<AtomicBool>) -> Result<SearchReport, String> {
    if options.query.is_empty() {
        return Err("Search query cannot be empty.".to_string());
    }
    if !options.root.is_dir() {
        return Err(format!(
            "{} is not an existing folder.",
            options.root.display()
        ));
    }

    let mut report = SearchReport::default();
    let max_matches = options.max_matches.max(1);
    walk_dir(&options.root, "", options, max_matches, cancel, &mut report);
    Ok(report)
}

fn walk_dir(
    dir: &Path,
    rel_prefix: &str,
    options: &SearchOptions,
    max_matches: usize,
    cancel: &Arc<AtomicBool>,
    report: &mut SearchReport,
) {
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
        return;
    }
    if report.truncated {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name, entry.path())
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));

    let mut subdirs = Vec::new();
    for (name, path) in items {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            return;
        }
        if report.truncated {
            return;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if meta.is_dir() {
            if DEFAULT_IGNORED_DIRS.contains(&name.as_str())
                || matches_any_glob(&options.exclude_globs, &name, &rel)
            {
                continue;
            }
            subdirs.push((path, rel));
        } else if meta.is_file() {
            if matches_any_glob(&options.exclude_globs, &name, &rel) {
                continue;
            }
            if !options.include_globs.is_empty()
                && !matches_any_glob(&options.include_globs, &name, &rel)
            {
                continue;
            }
            search_file(&path, &rel, options, max_matches, report);
        }
    }

    for (subdir, rel) in subdirs {
        walk_dir(&subdir, &rel, options, max_matches, cancel, report);
        if report.cancelled || report.truncated {
            return;
        }
    }
}

fn search_file(
    path: &Path,
    rel_path: &str,
    options: &SearchOptions,
    max_matches: usize,
    report: &mut SearchReport,
) {
    let live_text = if options.open_buffers.is_empty() {
        None
    } else {
        fs::canonicalize(path)
            .ok()
            .and_then(|canonical| options.open_buffers.get(&canonical))
            .map(String::as_str)
    };

    let owned_disk;
    let (text, from_unsaved_buffer) = match live_text {
        Some(buf) => (buf, true),
        None => {
            let Ok(disk) = read_utf8_limited(path, MAX_OPEN_BYTES) else {
                return;
            };
            owned_disk = disk;
            (owned_disk.as_str(), false)
        }
    };
    report.files_searched += 1;

    let mut matched_this_file = false;
    for (idx, line_slice) in textlines::iter_inclusive(text).enumerate() {
        let line_no = idx + 1;
        let line = line_slice.trim_end_matches(['\n', '\r', '\u{2029}']);
        let hits = find_line_matches(line, &options.query, options.match_case, options.whole_word);
        if hits.is_empty() {
            continue;
        }
        if !matched_this_file {
            matched_this_file = true;
            report.files_matched += 1;
        }
        for (col_char, match_chars) in hits {
            if report.matches.len() >= max_matches {
                report.truncated = true;
                return;
            }
            let (preview, preview_highlight) = build_preview(line, col_char, match_chars);
            report.matches.push(FileMatch {
                path: path.to_path_buf(),
                rel_path: rel_path.to_string(),
                line: line_no,
                column_char: col_char,
                match_chars,
                preview,
                preview_highlight,
                from_unsaved_buffer,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-find-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn glob_matching_supports_star_and_question() {
        assert!(glob_match("*.rs", "window.rs"));
        assert!(!glob_match("*.rs", "window.md"));
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(!glob_match("file?.txt", "file12.txt"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn line_matching_respects_case_and_whole_word() {
        assert_eq!(
            find_line_matches("Foo foo foobar", "foo", false, false),
            vec![(0, 3), (4, 3), (8, 3)]
        );
        assert_eq!(
            find_line_matches("Foo foo foobar", "foo", true, false),
            vec![(4, 3), (8, 3)]
        );
        assert_eq!(
            find_line_matches("Foo foo foobar _foo", "foo", false, true),
            vec![(0, 3), (4, 3)]
        );
    }

    #[test]
    fn searches_recursively_with_filters_and_skips_ignored_dirs() {
        let dir = temp_dir("recursive");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("a.md"), "Hello World\nsecond line\n").unwrap();
        fs::write(dir.join("sub/b.rs"), "let world = 1;\n").unwrap();
        fs::write(dir.join("sub/skip.log"), "world in log\n").unwrap();
        fs::write(dir.join(".git/HEAD"), "world in git\n").unwrap();
        fs::write(dir.join("bin.dat"), b"world\0binary").unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let report = search(
            &SearchOptions {
                root: dir.clone(),
                query: "world".into(),
                match_case: false,
                whole_word: true,
                include_globs: vec![],
                exclude_globs: vec!["*.log".into()],
                max_matches: 100,
                open_buffers: HashMap::new(),
            },
            &cancel,
        )
        .unwrap();

        assert_eq!(report.files_matched, 2);
        assert_eq!(report.matches.len(), 2);
        assert_eq!(report.matches[0].rel_path, "a.md");
        assert_eq!(report.matches[0].line, 1);
        assert_eq!(report.matches[0].column_char, 6);
        assert_eq!(report.matches[1].rel_path, "sub/b.rs");
        assert_eq!(report.matches[1].line, 1);
        assert_eq!(report.matches[1].column_char, 4);

        // Include filter restricts to *.rs only.
        let rs_only = search(
            &SearchOptions {
                root: dir.clone(),
                query: "world".into(),
                match_case: false,
                whole_word: false,
                include_globs: vec!["*.rs".into()],
                exclude_globs: vec![],
                max_matches: 100,
                open_buffers: HashMap::new(),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(rs_only.matches.len(), 1);
        assert_eq!(rs_only.matches[0].rel_path, "sub/b.rs");

        // Dirty open buffer overrides on-disk content.
        let mut open_buffers = HashMap::new();
        open_buffers.insert(
            fs::canonicalize(dir.join("a.md")).unwrap(),
            "line1\nunsaved fresh_token here\n".to_string(),
        );
        let dirty_found = search(
            &SearchOptions {
                root: dir.clone(),
                query: "fresh_token".into(),
                match_case: false,
                whole_word: true,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: 100,
                open_buffers: open_buffers.clone(),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(dirty_found.matches.len(), 1);
        assert_eq!(dirty_found.matches[0].rel_path, "a.md");
        assert_eq!(dirty_found.matches[0].line, 2);
        assert!(dirty_found.matches[0].from_unsaved_buffer);

        // Text deleted in the dirty buffer no longer matches on-disk "world" in a.md.
        let disk_removed = search(
            &SearchOptions {
                root: dir.clone(),
                query: "world".into(),
                match_case: false,
                whole_word: true,
                include_globs: vec![],
                exclude_globs: vec!["*.log".into()],
                max_matches: 100,
                open_buffers,
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(disk_removed.matches.len(), 1);
        assert_eq!(disk_removed.matches[0].rel_path, "sub/b.rs");
        assert!(!disk_removed.matches[0].from_unsaved_buffer);

        let _ = fs::remove_dir_all(&dir);
    }
}
