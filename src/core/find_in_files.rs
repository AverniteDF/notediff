//! Pure-Rust Find in Files engine (no GTK).

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
const MAX_QUERY_BYTES: usize = 16 * 1024;

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

/// Maximum preview window (in characters), excluding omission markers.
const MAX_PREVIEW_CHARS: usize = 240;

#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub query: MatchQuery,
    pub recursive: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub max_matches: usize,
    /// Live in-memory text of dirty open documents keyed by canonical path.
    /// Searched instead of the on-disk file so unsaved edits are reflected.
    pub open_buffers: HashMap<PathBuf, String>,
}

#[derive(Clone, Debug)]
pub struct MatchQuery {
    text: String,
    match_case: bool,
    whole_word: bool,
}

impl MatchQuery {
    pub fn new(text: String, match_case: bool, whole_word: bool) -> Self {
        Self {
            text,
            match_case,
            whole_word,
        }
    }

    /// Validate a result at its original character column in the current line.
    /// Return the exclusive end column only if the original query still matches.
    pub fn match_end_at(&self, line: &str, column: usize) -> Option<usize> {
        if self.text.is_empty() {
            return None;
        }
        let mut chars = line.chars();
        if column > 0 {
            let before = chars.nth(column - 1)?;
            if self.whole_word && is_word_char(before) {
                return None;
            }
        }
        let mut end = column;
        for expected in self.text.chars() {
            if char_key(chars.next()?, self.match_case) != char_key(expected, self.match_case) {
                return None;
            }
            end += 1;
        }
        if self.whole_word && chars.next().is_some_and(is_word_char) {
            return None;
        }
        Some(end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHit {
    /// 0-based character offset of the match on the raw line.
    pub column_char: usize,
    /// Bounded snippet around this match, with tabs displayed as spaces.
    pub preview: String,
    /// Byte range `(start, end)` of the visible match within `preview`.
    pub byte_range: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    pub path: PathBuf,
    pub rel_path: String,
    /// 1-based line number matching GTK and `core::textlines`.
    pub line: usize,
    /// All non-overlapping matches on this line, in left-to-right order.
    pub hits: Vec<LineHit>,
    /// True when this match came from an unsaved in-memory buffer rather than disk.
    pub from_unsaved_buffer: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SearchReport {
    pub matches: Vec<FileMatch>,
    pub total_matches: usize,
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

#[derive(Clone, Copy)]
struct RawHit {
    column: usize,
    start: usize,
    end: usize,
}

struct Matcher<'a> {
    query: Vec<[char; 3]>,
    prefix: Vec<usize>,
    ring: Vec<(usize, bool)>,
    match_case: bool,
    whole_word: bool,
    cancel: &'a AtomicBool,
}

impl<'a> Matcher<'a> {
    fn new(query: &str, match_case: bool, whole_word: bool, cancel: &'a AtomicBool) -> Self {
        let query: Vec<_> = query.chars().map(|c| char_key(c, match_case)).collect();
        let mut prefix = vec![0; query.len()];
        let mut matched = 0;
        for i in 1..query.len() {
            while matched > 0 && query[i] != query[matched] {
                matched = prefix[matched - 1];
            }
            if query[i] == query[matched] {
                matched += 1;
            }
            prefix[i] = matched;
        }
        let ring = vec![(0, false); query.len()];
        Self {
            query,
            prefix,
            ring,
            match_case,
            whole_word,
            cancel,
        }
    }

    /// Stream a line with a prefix table, retaining only the requested hits.
    /// The ring maps the start of a match back to byte and word boundaries.
    fn scan_line(&mut self, line: &str, limit: usize) -> Option<Vec<RawHit>> {
        let qlen = self.query.len();
        if qlen == 0 || qlen > line.len() || limit == 0 {
            return Some(Vec::new());
        }
        let mut matched = 0;
        let mut previous_word = false;
        let mut out = Vec::new();
        let mut chars = line.char_indices().enumerate().peekable();
        while let Some((column, (byte, ch))) = chars.next() {
            if column % 4096 == 0 && self.cancel.load(Ordering::Relaxed) {
                return None;
            }
            self.ring[column % qlen] = (byte, previous_word);
            previous_word = is_word_char(ch);
            let key = char_key(ch, self.match_case);
            while matched > 0 && key != self.query[matched] {
                matched = self.prefix[matched - 1];
            }
            if key == self.query[matched] {
                matched += 1;
            }
            if matched == qlen {
                let start_column = column + 1 - qlen;
                let (start_byte, preceded_by_word) = self.ring[start_column % qlen];
                if !self.whole_word
                    || (!preceded_by_word
                        && chars.peek().is_none_or(|(_, (_, c))| !is_word_char(*c)))
                {
                    out.push(RawHit {
                        column: start_column,
                        start: start_byte,
                        end: byte + ch.len_utf8(),
                    });
                    if out.len() == limit {
                        break;
                    }
                    matched = 0;
                } else {
                    matched = self.prefix[matched - 1];
                }
            }
        }
        Some(out)
    }
}

fn char_key(ch: char, match_case: bool) -> [char; 3] {
    let mut key = [ch, '\0', '\0'];
    if !match_case {
        for (out, c) in key.iter_mut().zip(ch.to_lowercase()) {
            *out = c;
        }
    }
    key
}

fn make_preview(line: &str, hit: RawHit) -> LineHit {
    let mut start = hit.start;
    let mut before = 0;
    for ch in line[..hit.start].chars().rev().take(MAX_PREVIEW_CHARS / 3) {
        start -= ch.len_utf8();
        before += 1;
    }
    let end = line[hit.start..]
        .char_indices()
        .nth(MAX_PREVIEW_CHARS - before)
        .map_or(line.len(), |(offset, _)| hit.start + offset);
    let mut preview = String::new();
    if start > 0 {
        preview.push('…');
    }
    // Replacing a tab with a space preserves byte offsets.
    let byte_range = (
        preview.len() + hit.start - start,
        preview.len() + hit.end.min(end) - start,
    );
    preview.push_str(&line[start..end].replace('\t', " "));
    if end < line.len() {
        preview.push('…');
    }
    LineHit {
        column_char: hit.column,
        preview,
        byte_range,
    }
}

/// Search `options.root` for `options.query`, optionally descending into subfolders.
pub fn search(options: &SearchOptions, cancel: &Arc<AtomicBool>) -> Result<SearchReport, String> {
    if options.query.text.is_empty() {
        return Err("Search query cannot be empty.".to_string());
    }
    if options.query.text.len() > MAX_QUERY_BYTES {
        return Err("Search query exceeds the 16 KiB limit.".to_string());
    }
    if !options.root.is_dir() {
        return Err(format!(
            "{} is not an existing folder.",
            options.root.display()
        ));
    }

    let mut report = SearchReport::default();
    let max_matches = options.max_matches.max(1);
    let mut matcher = Matcher::new(
        &options.query.text,
        options.query.match_case,
        options.query.whole_word,
        cancel,
    );
    walk_dir(
        &options.root,
        "",
        options,
        max_matches,
        &mut matcher,
        &mut report,
    );
    Ok(report)
}

fn walk_dir(
    dir: &Path,
    rel_prefix: &str,
    options: &SearchOptions,
    max_matches: usize,
    matcher: &mut Matcher<'_>,
    report: &mut SearchReport,
) {
    if matcher.cancel.load(Ordering::Relaxed) {
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
        if matcher.cancel.load(Ordering::Relaxed) {
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
            if !options.recursive
                || DEFAULT_IGNORED_DIRS.contains(&name.as_str())
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
            search_file(&path, &rel, options, max_matches, matcher, report);
            if report.cancelled || report.truncated {
                return;
            }
        }
    }

    for (subdir, rel) in subdirs {
        walk_dir(&subdir, &rel, options, max_matches, matcher, report);
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
    matcher: &mut Matcher<'_>,
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
        if matcher.cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            return;
        }
        let line_no = idx + 1;
        let line = line_slice.trim_end_matches(['\n', '\r', '\u{2029}']);
        let remaining = max_matches.saturating_sub(report.total_matches);
        let Some(mut hits) = matcher.scan_line(line, remaining.saturating_add(1)) else {
            report.cancelled = true;
            return;
        };
        if hits.is_empty() {
            continue;
        }
        if remaining == 0 {
            report.truncated = true;
            return;
        }
        if hits.len() > remaining {
            hits.truncate(remaining);
            report.truncated = true;
        }
        if !matched_this_file {
            matched_this_file = true;
            report.files_matched += 1;
        }
        let line_hits: Vec<_> = hits
            .into_iter()
            .map(|hit| make_preview(line, hit))
            .collect();
        report.total_matches += line_hits.len();
        report.matches.push(FileMatch {
            path: path.to_path_buf(),
            rel_path: rel_path.to_string(),
            line: line_no,
            hits: line_hits,
            from_unsaved_buffer,
        });
        if report.truncated {
            return;
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
        let find = |line, query, case, word| {
            Matcher::new(query, case, word, &AtomicBool::new(false))
                .scan_line(line, usize::MAX)
                .unwrap()
                .into_iter()
                .map(|hit| hit.column)
                .collect::<Vec<_>>()
        };
        assert_eq!(find("Foo foo foobar", "foo", false, false), vec![0, 4, 8]);
        assert_eq!(find("Foo foo foobar", "foo", true, false), vec![4, 8]);
        assert_eq!(find("Foo foo foobar _foo", "foo", false, true), vec![0, 4]);
        assert_eq!(find("a--- éÉ-é", "--", true, true), vec![2]);
        assert_eq!(find("a--- éÉ-é", "é", false, false), vec![5, 6, 8]);
        assert_eq!(find("aaaaa", "aa", true, false), vec![0, 2]);
    }

    #[test]
    fn matcher_honors_the_hit_budget_and_cancellation_on_a_long_line() {
        let cancel = AtomicBool::new(false);
        let mut matcher = Matcher::new("a", true, false, &cancel);
        let line = "a".repeat(100_000);
        assert_eq!(matcher.scan_line(&line, 3).unwrap().len(), 3);
        cancel.store(true, Ordering::Relaxed);
        assert!(matcher.scan_line(&line, 3).is_none());
    }

    #[test]
    fn revalidation_requires_the_original_column_case_and_word_boundaries() {
        let word = MatchQuery::new("café".into(), false, true);
        assert_eq!(word.match_end_at("▶ CAFÉ!", 2), Some(6));
        assert_eq!(word.match_end_at("▶ CAFÉ!", 1), None);
        assert_eq!(word.match_end_at("▶ CAFÉ!", 40), None);
        assert_eq!(word.match_end_at("▶ CAFÉ_suffix", 2), None);
        assert_eq!(word.match_end_at("▶xCAFÉ!", 2), None);
        assert_eq!(word.match_end_at("▶ CAF", 2), None);

        let exact = MatchQuery::new("café".into(), true, false);
        assert_eq!(exact.match_end_at("▶ CAFÉ!", 2), None);
        assert_eq!(exact.match_end_at("▶xcafé_suffix", 2), Some(6));
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
        let options = SearchOptions {
            root: dir.clone(),
            query: MatchQuery::new("world".into(), false, true),
            recursive: true,
            include_globs: vec![],
            exclude_globs: vec!["*.log".into()],
            max_matches: 100,
            open_buffers: HashMap::new(),
        };
        let report = search(&options, &cancel).unwrap();

        assert_eq!(report.files_matched, 2);
        assert_eq!(report.matches.len(), 2);
        assert_eq!(report.total_matches, 2);
        assert_eq!(report.matches[0].rel_path, "a.md");
        assert_eq!(report.matches[0].line, 1);
        assert_eq!(report.matches[0].hits[0].column_char, 6);
        assert_eq!(report.matches[1].rel_path, "sub/b.rs");
        assert_eq!(report.matches[1].line, 1);
        assert_eq!(report.matches[1].hits[0].column_char, 4);

        let flat = search(
            &SearchOptions {
                recursive: false,
                ..options
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(flat.files_searched, 1);
        assert_eq!(flat.total_matches, 1);
        assert_eq!(flat.matches[0].rel_path, "a.md");

        // Include filter restricts to *.rs only.
        let rs_only = search(
            &SearchOptions {
                root: dir.clone(),
                query: MatchQuery::new("world".into(), false, false),
                recursive: true,
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
                query: MatchQuery::new("fresh_token".into(), false, true),
                recursive: true,
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
                query: MatchQuery::new("world".into(), false, true),
                recursive: true,
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

        // Long lines keep one row, with a bounded preview for each match.
        let long_line = format!(
            "{}needle{}needle{}\n",
            "a".repeat(200),
            "b".repeat(150),
            "c".repeat(200)
        );
        let long_path = dir.join("long.txt");
        fs::write(&long_path, &long_line).unwrap();
        let long_report = search(
            &SearchOptions {
                root: dir.clone(),
                query: MatchQuery::new("needle".into(), true, false),
                recursive: true,
                include_globs: vec!["long.txt".into()],
                exclude_globs: vec![],
                max_matches: 10,
                open_buffers: HashMap::new(),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(long_report.matches.len(), 1);
        assert_eq!(long_report.total_matches, 2);
        let row = &long_report.matches[0];
        assert_eq!(row.hits.len(), 2);
        assert_eq!(row.hits[0].column_char, 200);
        assert_eq!(row.hits[1].column_char, 356);
        for hit in &row.hits {
            let (s, e) = hit.byte_range;
            assert!(hit.preview.starts_with('…'));
            assert!(hit.preview.ends_with('…'));
            assert!(hit.preview.chars().count() <= MAX_PREVIEW_CHARS + 2);
            assert_eq!(&hit.preview[s..e], "needle");
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
