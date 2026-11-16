//! Line-based diffing and hunk application.
//!
//! A line is represented as its full text including its line terminator. This
//! is important: it lets the app preserve CRLF vs LF exactly, and it makes a
//! change in the trailing newline an ordinary line replacement that can be
//! accepted like any other hunk.

use std::ops::Range;
use std::time::{Duration, Instant};

use similar::{ChangeTag, DiffOp, TextDiffConfig};

use super::textlines;

/// Maximum time spent on one line diff and its inline refinement. A timed-out
/// diff is still a valid, if less minimal, diff: hunks retain valid ranges and
/// accepting all of them still reaches the live text.
pub(crate) const DIFF_TIMEOUT: Duration = Duration::from_secs(1);

/// A contiguous changed region, with line indexes into the baseline (old) and
/// live (new) texts. Ranges are half-open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

impl Hunk {
    pub fn old_end(&self) -> usize {
        self.old_start + self.old_len
    }

    pub fn new_end(&self) -> usize {
        self.new_start + self.new_len
    }
}

/// A character range within one logical line, used for word-level highlights.
/// Offsets are character counts (not bytes) into the line content with its
/// line terminator removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineSpan {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Default)]
pub struct DiffResult {
    pub hunks: Vec<Hunk>,
    pub baseline_inline: Vec<InlineSpan>,
    pub live_inline: Vec<InlineSpan>,
}

impl DiffResult {
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

/// Split text into lines that retain their terminators. `""` produces no
/// lines, `"a\n"` produces one line and `"a\nb"` produces two. GTK's lone CR
/// and U+2029 paragraph breaks are line breaks here too.
pub fn split_lines(text: &str) -> Vec<&str> {
    textlines::split_inclusive(text)
}

/// The visible part of a line, without any line terminator.
pub fn line_content(line: &str) -> &str {
    textlines::content(line)
}

pub fn diff_texts(baseline: &str, live: &str) -> DiffResult {
    diff_texts_with_timeout(baseline, live, DIFF_TIMEOUT)
}

fn diff_texts_with_timeout(baseline: &str, live: &str, timeout: Duration) -> DiffResult {
    let old_lines = split_lines(baseline);
    let new_lines = split_lines(live);
    let deadline = Instant::now() + timeout;
    let hunks = hunks_for(&old_lines, &new_lines, deadline);

    let mut result = DiffResult {
        hunks,
        baseline_inline: Vec::new(),
        live_inline: Vec::new(),
    };

    for hunk in &result.hunks {
        compute_inline_for_hunk(
            &old_lines,
            &new_lines,
            hunk,
            deadline,
            &mut result.baseline_inline,
            &mut result.live_inline,
        );
    }

    result
}

fn hunks_for(old_lines: &[&str], new_lines: &[&str], deadline: Instant) -> Vec<Hunk> {
    let mut config = TextDiffConfig::new();
    config.deadline(deadline);
    let diff = config.diff_slices(old_lines, new_lines);

    let mut hunks = Vec::new();
    let mut current: Option<Hunk> = None;

    for op in diff.ops() {
        match *op {
            DiffOp::Equal { len, .. } => {
                if len > 0
                    && let Some(hunk) = current.take()
                {
                    hunks.push(hunk);
                }
            }
            // Delete, Insert and Replace all describe a changed region. The
            // range accessors yield an empty range on the side with no lines.
            DiffOp::Delete { .. } | DiffOp::Insert { .. } | DiffOp::Replace { .. } => {
                let old = op.old_range();
                let new = op.new_range();
                let hunk = current.get_or_insert(Hunk {
                    old_start: old.start,
                    old_len: 0,
                    new_start: new.start,
                    new_len: 0,
                });
                extend(hunk, old.start, old.len(), new.start, new.len());
            }
        }
    }

    if let Some(hunk) = current {
        hunks.push(hunk);
    }
    coalesce_blank_separated_replacements(hunks, old_lines, new_lines)
}

/// Blank lines are interchangeable anchors: the line diff can split one
/// replacement into a deletion-only hunk and an insertion-only hunk with an
/// unchanged blank line between them. Join such runs when every gap is only
/// unchanged blank lines, but keep paragraph replacements and same-side
/// insert/delete runs separate.
fn coalesce_blank_separated_replacements(
    hunks: Vec<Hunk>,
    old_lines: &[&str],
    new_lines: &[&str],
) -> Vec<Hunk> {
    fn pure_deletion(hunk: &Hunk) -> bool {
        hunk.old_len > 0 && hunk.new_len == 0
    }
    fn pure_insertion(hunk: &Hunk) -> bool {
        hunk.old_len == 0 && hunk.new_len > 0
    }
    fn pure(hunk: &Hunk) -> bool {
        pure_deletion(hunk) || pure_insertion(hunk)
    }
    fn blank_gap(previous: &Hunk, next: &Hunk, old: &[&str], new: &[&str]) -> bool {
        let old_gap = &old[previous.old_end()..next.old_start];
        let new_gap = &new[previous.new_end()..next.new_start];
        !old_gap.is_empty()
            && old_gap.len() == new_gap.len()
            && old_gap
                .iter()
                .zip(new_gap)
                .all(|(old, new)| old == new && line_content(old).is_empty())
    }

    let mut merged = Vec::with_capacity(hunks.len());
    let mut index = 0;
    while index < hunks.len() {
        let first = hunks[index];
        if !pure(&first) {
            merged.push(first);
            index += 1;
            continue;
        }
        let mut end = index + 1;
        let mut saw_deletion = pure_deletion(&first);
        let mut saw_insertion = pure_insertion(&first);
        while end < hunks.len() {
            let next = hunks[end];
            if !pure(&next) || !blank_gap(&hunks[end - 1], &next, old_lines, new_lines) {
                break;
            }
            saw_deletion |= pure_deletion(&next);
            saw_insertion |= pure_insertion(&next);
            end += 1;
        }
        if end > index + 1 && saw_deletion && saw_insertion {
            let last = hunks[end - 1];
            let deleted = &old_lines[first.old_start..last.old_end()];
            let inserted = &new_lines[first.new_start..last.new_end()];
            // A blank deletion paired with an unrelated insertion is not a
            // fragmented replacement; it should stay navigable on its own.
            let changed_content = deleted.iter().any(|line| !line_content(line).is_empty())
                && inserted.iter().any(|line| !line_content(line).is_empty());
            if changed_content {
                merged.push(Hunk {
                    old_start: first.old_start,
                    old_len: last.old_end() - first.old_start,
                    new_start: first.new_start,
                    new_len: last.new_end() - first.new_start,
                });
                index = end;
                continue;
            }
        }
        merged.push(first);
        index += 1;
    }
    merged
}

fn extend(hunk: &mut Hunk, old_index: usize, old_len: usize, new_index: usize, new_len: usize) {
    let old_end = (hunk.old_start + hunk.old_len).max(old_index + old_len);
    let new_end = (hunk.new_start + hunk.new_len).max(new_index + new_len);
    hunk.old_start = hunk.old_start.min(old_index);
    hunk.new_start = hunk.new_start.min(new_index);
    hunk.old_len = old_end - hunk.old_start;
    hunk.new_len = new_end - hunk.new_start;
}

fn compute_inline_for_hunk(
    old_lines: &[&str],
    new_lines: &[&str],
    hunk: &Hunk,
    deadline: Instant,
    baseline_out: &mut Vec<InlineSpan>,
    live_out: &mut Vec<InlineSpan>,
) {
    // Pair lines only when the shape is the same. For unequal replacements it
    // is not clear which old line corresponds to which new line, so highlight
    // whole lines instead of inventing a misleading word alignment. Once the
    // time budget is spent, whole-line spans are also the safe fallback.
    if Instant::now() >= deadline {
        whole_line_spans(old_lines, hunk.old_start..hunk.old_end(), baseline_out);
        whole_line_spans(new_lines, hunk.new_start..hunk.new_end(), live_out);
        return;
    }
    if hunk.old_len == hunk.new_len && hunk.old_len > 0 {
        for offset in 0..hunk.old_len {
            let old_index = hunk.old_start + offset;
            let new_index = hunk.new_start + offset;
            inline_for_line_pair(
                line_content(old_lines[old_index]),
                line_content(new_lines[new_index]),
                old_index,
                new_index,
                deadline,
                baseline_out,
                live_out,
            );
        }
        return;
    }

    whole_line_spans(old_lines, hunk.old_start..hunk.old_end(), baseline_out);
    whole_line_spans(new_lines, hunk.new_start..hunk.new_end(), live_out);
}

/// Highlight every non-empty line in `range` from its first to its last
/// character (line terminator excluded).
fn whole_line_spans(lines: &[&str], range: Range<usize>, out: &mut Vec<InlineSpan>) {
    for index in range {
        let Some(line) = lines.get(index) else {
            continue;
        };
        push_span(out, index, 0, line_content(line).chars().count());
    }
}

fn inline_for_line_pair(
    old_line: &str,
    new_line: &str,
    old_index: usize,
    new_index: usize,
    deadline: Instant,
    baseline_out: &mut Vec<InlineSpan>,
    live_out: &mut Vec<InlineSpan>,
) {
    let mut config = TextDiffConfig::new();
    config.deadline(deadline);
    let diff = config.diff_unicode_words(old_line, new_line);
    let mut old_pos = 0;
    let mut new_pos = 0;
    let mut old_span: Option<Range<usize>> = None;
    let mut new_span: Option<Range<usize>> = None;
    let mut bridged = false;

    for change in diff.iter_all_changes() {
        let len = change.value().chars().count();
        match change.tag() {
            ChangeTag::Delete => {
                // Match the review units: when a change continues after equal
                // whitespace, both spans must cross that whitespace. Extending
                // only the changed side leaves the other side's whitespace
                // outside the highlight.
                if bridged {
                    extend_span(&mut old_span, old_pos);
                    extend_span(&mut new_span, new_pos);
                    bridged = false;
                }
                let span = old_span.get_or_insert(old_pos..old_pos);
                span.end = old_pos + len;
                old_pos += len;
            }
            ChangeTag::Insert => {
                if bridged {
                    extend_span(&mut old_span, old_pos);
                    extend_span(&mut new_span, new_pos);
                    bridged = false;
                }
                let span = new_span.get_or_insert(new_pos..new_pos);
                span.end = new_pos + len;
                new_pos += len;
            }
            ChangeTag::Equal => {
                // Whitespace bridges adjacent changes, while an unchanged word
                // or punctuation separates them.
                if !change.value().chars().all(char::is_whitespace) {
                    bridged = false;
                    flush_span_pair(
                        &mut old_span,
                        &mut new_span,
                        old_line,
                        new_line,
                        old_index,
                        new_index,
                        baseline_out,
                        live_out,
                    );
                } else if old_span.is_some() || new_span.is_some() {
                    bridged = true;
                }
                old_pos += len;
                new_pos += len;
            }
        }
    }
    flush_span_pair(
        &mut old_span,
        &mut new_span,
        old_line,
        new_line,
        old_index,
        new_index,
        baseline_out,
        live_out,
    );
}

fn extend_span(span: &mut Option<Range<usize>>, end: usize) {
    if let Some(span) = span.as_mut() {
        span.end = end;
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_span_pair(
    old_span: &mut Option<Range<usize>>,
    new_span: &mut Option<Range<usize>>,
    old_line: &str,
    new_line: &str,
    old_index: usize,
    new_index: usize,
    baseline_out: &mut Vec<InlineSpan>,
    live_out: &mut Vec<InlineSpan>,
) {
    let mut old_span = old_span.take();
    let mut new_span = new_span.take();
    if let (Some(old), Some(new)) = (old_span.as_mut(), new_span.as_mut()) {
        trim_common_span_whitespace(old_line, new_line, old, new);
    }
    if let Some(span) = old_span {
        push_span(baseline_out, old_index, span.start, span.end);
    }
    if let Some(span) = new_span {
        push_span(live_out, new_index, span.start, span.end);
    }
}

/// Remove identical boundary whitespace from a pair of word-level highlights,
/// matching the review units. Whitespace inside a bridged phrase stays.
fn trim_common_span_whitespace(
    old_line: &str,
    new_line: &str,
    old: &mut Range<usize>,
    new: &mut Range<usize>,
) {
    while old.start < old.end && new.start < new.end {
        let old_char = old_line.chars().nth(old.start);
        let new_char = new_line.chars().nth(new.start);
        match (old_char, new_char) {
            (Some(old_char), Some(new_char))
                if old_char == new_char && old_char.is_whitespace() =>
            {
                old.start += 1;
                new.start += 1;
            }
            _ => break,
        }
    }
    while old.start < old.end && new.start < new.end {
        let old_char = old_line.chars().nth(old.end - 1);
        let new_char = new_line.chars().nth(new.end - 1);
        match (old_char, new_char) {
            (Some(old_char), Some(new_char))
                if old_char == new_char && old_char.is_whitespace() =>
            {
                old.end -= 1;
                new.end -= 1;
            }
            _ => break,
        }
    }
}

fn push_span(out: &mut Vec<InlineSpan>, line: usize, start: usize, end: usize) {
    if end > start {
        out.push(InlineSpan { line, start, end });
    }
}

/// Replace the hunk's baseline range with the corresponding live range.
///
/// `baseline` must be the text the hunk was computed against. The returned
/// string preserves every line's original terminator from `live`.
pub fn apply_hunk(baseline: &str, live: &str, hunk: &Hunk) -> Result<String, String> {
    let old_lines = split_lines(baseline);
    let new_lines = split_lines(live);

    if hunk.old_end() > old_lines.len() || hunk.new_end() > new_lines.len() {
        return Err(format!(
            "hunk range out of bounds (baseline {} lines, live {} lines)",
            old_lines.len(),
            new_lines.len()
        ));
    }

    let mut result = String::with_capacity(baseline.len() + live.len());
    for line in &old_lines[..hunk.old_start] {
        result.push_str(line);
    }
    for line in &new_lines[hunk.new_start..hunk.new_end()] {
        result.push_str(line);
    }
    for line in &old_lines[hunk.old_end()..] {
        result.push_str(line);
    }
    Ok(result)
}

/// Apply every hunk at once. Accepting all hunks must produce exactly `live`;
/// this is the primary invariant of the application.
pub fn apply_all_hunks(baseline: &str, live: &str) -> String {
    let old_lines = split_lines(baseline);
    let new_lines = split_lines(live);
    let hunks = hunks_for(&old_lines, &new_lines, Instant::now() + DIFF_TIMEOUT);

    let mut out = String::with_capacity(live.len());
    let mut old_pos = 0;
    for hunk in &hunks {
        if hunk.old_start < old_pos
            || hunk.old_end() > old_lines.len()
            || hunk.new_end() > new_lines.len()
        {
            return live.to_string();
        }
        for line in &old_lines[old_pos..hunk.old_start] {
            out.push_str(line);
        }
        for line in &new_lines[hunk.new_start..hunk.new_end()] {
            out.push_str(line);
        }
        old_pos = hunk.old_end();
    }
    for line in &old_lines[old_pos..] {
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_roundtrip(text: &str) {
        let lines = split_lines(text);
        let joined: String = lines.concat();
        assert_eq!(joined, text);
    }

    #[test]
    fn split_lines_preserves_endings() {
        assert_roundtrip("");
        assert_roundtrip("a");
        assert_roundtrip("a\n");
        assert_roundtrip("a\r\nb\r\n");
        assert_roundtrip("a\r\nb\nc");
        assert_roundtrip("a\rb\u{2029}c");
        assert_eq!(split_lines("a\nb"), vec!["a\n", "b"]);
        assert_eq!(split_lines("a\n"), vec!["a\n"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("a\rb"), vec!["a\r", "b"]);
        assert_eq!(split_lines("a\u{2029}b"), vec!["a\u{2029}", "b"]);
    }

    #[test]
    fn identical_text_has_no_hunks() {
        let diff = diff_texts("hello\nworld\n", "hello\nworld\n");
        assert!(diff.hunks.is_empty());
        assert!(diff.baseline_inline.is_empty());
        assert!(diff.live_inline.is_empty());
    }

    #[test]
    fn detects_replacement_and_applies_it() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        let diff = diff_texts(old, new);
        assert_eq!(
            diff.hunks,
            vec![Hunk {
                old_start: 1,
                old_len: 1,
                new_start: 1,
                new_len: 1
            }]
        );
        let applied = apply_hunk(old, new, &diff.hunks[0]).unwrap();
        assert_eq!(applied, new);
    }

    #[test]
    fn detects_insertions_and_deletions() {
        let old = "a\nb\n";
        let new = "a\nx\ny\nb\n";
        let diff = diff_texts(old, new);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(apply_hunk(old, new, &diff.hunks[0]).unwrap(), new);
        assert_eq!(apply_all_hunks(old, new), new);

        let old = "a\nx\ny\nb\n";
        let new = "a\nb\n";
        let diff = diff_texts(old, new);
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(apply_hunk(old, new, &diff.hunks[0]).unwrap(), new);
        assert_eq!(apply_all_hunks(old, new), new);
    }

    #[test]
    fn blank_anchors_do_not_split_a_replacement() {
        let old = "hello world\n\n";
        let new = "\nhello world!\n\n";
        let diff = diff_texts(old, new);
        assert_eq!(diff.hunks.len(), 1, "{:?}", diff.hunks);
        assert_eq!(diff.hunks[0].old_start, 0);
        assert_eq!(diff.hunks[0].old_len, 2);
        assert_eq!(diff.hunks[0].new_start, 0);
        assert_eq!(diff.hunks[0].new_len, 3);
        assert_eq!(apply_all_hunks(old, new), new);

        // Paragraph replacements on both sides of a blank line stay separate.
        let old = "one\n\ntwo\n";
        let new = "ONE\n\nTWO\n";
        assert_eq!(diff_texts(old, new).hunks.len(), 2);

        // A blank-only deletion and an unrelated insertion stay separate.
        let old = "\n\nx\n\n\n";
        let new = "a\n\nx\n\nb\n";
        assert_eq!(diff_texts(old, new).hunks.len(), 3);
        assert_eq!(apply_all_hunks(old, new), new);
    }

    #[test]
    fn final_newline_can_be_added_and_removed() {
        let without = "a\nb";
        let with = "a\nb\n";

        let diff = diff_texts(without, with);
        assert!(!diff.hunks.is_empty());
        assert_eq!(apply_all_hunks(without, with), with);

        let diff = diff_texts(with, without);
        assert!(!diff.hunks.is_empty());
        assert_eq!(apply_all_hunks(with, without), without);
    }

    #[test]
    fn crlf_is_preserved() {
        let old = "one\r\ntwo\r\nthree\r\n";
        let new = "one\r\nTWO\r\nthree\r\n";
        let diff = diff_texts(old, new);
        let applied = apply_hunk(old, new, &diff.hunks[0]).unwrap();
        assert_eq!(applied, new);
        assert!(applied.contains("\r\n"));
        assert!(!applied.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn mixed_endings_roundtrip() {
        let old = "a\r\nb\nc\r\n";
        let new = "a\r\nB\nc\r\n";
        let diff = diff_texts(old, new);
        assert!(!diff.hunks.is_empty());
        assert_eq!(apply_all_hunks(old, new), new);
    }

    #[test]
    fn empty_files() {
        assert_eq!(apply_all_hunks("", ""), "");
        assert_eq!(apply_all_hunks("", "hello\n"), "hello\n");
        assert_eq!(apply_all_hunks("hello\n", ""), "");
        assert_eq!(apply_all_hunks("hello", "world\n"), "world\n");
    }

    #[test]
    fn word_level_spans_are_reported() {
        let old = "The quick brown fox\n";
        let new = "The quick red fox\n";
        let diff = diff_texts(old, new);
        assert!(!diff.baseline_inline.is_empty());
        assert!(!diff.live_inline.is_empty());
        let old_text: String = diff
            .baseline_inline
            .iter()
            .map(|span| {
                line_content(split_lines(old)[span.line])
                    .chars()
                    .skip(span.start)
                    .take(span.end - span.start)
                    .collect::<String>()
            })
            .collect();
        assert!(old_text.contains("brown"));
    }

    #[test]
    fn word_highlights_bridge_whitespace_like_review_units() {
        fn span_texts(line: &str, spans: &[InlineSpan]) -> Vec<String> {
            let body = line_content(split_lines(line)[0]);
            spans
                .iter()
                .map(|span| {
                    body.chars()
                        .skip(span.start)
                        .take(span.end - span.start)
                        .collect()
                })
                .collect()
        }

        let old = "the quick fox jumped over the lazy dog\n";
        let new = "a stealthy cat snuck under a low bush\n";
        let diff = diff_texts(old, new);
        assert_eq!(
            span_texts(old, &diff.baseline_inline),
            vec!["the quick fox jumped over the lazy dog"]
        );
        assert_eq!(
            span_texts(new, &diff.live_inline),
            vec!["a stealthy cat snuck under a low bush"]
        );

        let diff = diff_texts("a  b\n", "x  y\n");
        assert_eq!(span_texts("a  b\n", &diff.baseline_inline), vec!["a  b"]);
        assert_eq!(span_texts("x  y\n", &diff.live_inline), vec!["x  y"]);

        let diff = diff_texts(
            "The quick brown fox jumps.\n",
            "The fast brown fox sleeps.\n",
        );
        assert_eq!(
            span_texts("The quick brown fox jumps.\n", &diff.baseline_inline),
            vec!["quick", "jumps"]
        );
        assert_eq!(
            span_texts("The fast brown fox sleeps.\n", &diff.live_inline),
            vec!["fast", "sleeps"]
        );

        let diff = diff_texts(" a b \n", " x y \n");
        assert_eq!(span_texts(" a b \n", &diff.baseline_inline), vec!["a b"]);
        assert_eq!(span_texts(" x y \n", &diff.live_inline), vec!["x y"]);
    }

    #[test]
    fn word_spans_trim_bridging_whitespace_after_regrouping() {
        let old = "- Per-window zoom (Ctrl+scroll, Ctrl+plus/minus), with the default size in the config file.\n";
        let live = "- Per-window zoom (Ctrl+scroll, Ctrl+plus/minus) relative to the persisted application-wide font size, with a reset-to-default action.\n";
        let diff = diff_texts(old, live);
        let body = line_content(split_lines(live)[0]);
        let texts: Vec<String> = diff
            .live_inline
            .iter()
            .map(|span| {
                body.chars()
                    .skip(span.start)
                    .take(span.end - span.start)
                    .collect()
            })
            .collect();
        assert!(texts.iter().any(|text| text == "relative to"), "{texts:?}");
        assert!(
            !texts
                .iter()
                .any(|text| text.starts_with(' ') || text.ends_with(' ')),
            "{texts:?}"
        );
    }

    #[test]
    fn unequal_replacements_highlight_whole_lines() {
        let diff = diff_texts("a\nb\n", "a\nx\ny\n");
        assert_eq!(
            diff.baseline_inline,
            vec![InlineSpan {
                line: 1,
                start: 0,
                end: 1
            }]
        );
        assert_eq!(
            diff.live_inline,
            vec![
                InlineSpan {
                    line: 1,
                    start: 0,
                    end: 1
                },
                InlineSpan {
                    line: 2,
                    start: 0,
                    end: 1
                }
            ]
        );
    }

    // Tiny deterministic PRNG so the property test needs no extra dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn range(&mut self, max: usize) -> usize {
            if max == 0 {
                0
            } else {
                (self.next() as usize) % max
            }
        }
    }

    fn random_text(rng: &mut Rng) -> String {
        let line_count = rng.range(9);
        let mut text = String::new();
        let alphabet = ["a", "b", "c", "word", " ", "x", "é", "🦀"];
        for index in 0..line_count {
            let words = rng.range(6);
            for _ in 0..words {
                text.push_str(alphabet[rng.range(alphabet.len())]);
            }
            let last = index + 1 == line_count;
            match rng.range(5) {
                0 => text.push_str("\r\n"),
                1 => text.push('\n'),
                2 => text.push('\r'),
                3 => text.push('\u{2029}'),
                _ => {
                    if !last {
                        text.push('\n');
                    }
                }
            }
        }
        text
    }

    #[test]
    fn accepting_all_hunks_in_any_order_equals_live() {
        let mut rng = Rng(0xc0ff_ee12_3456_789a);
        for case in 0..500 {
            let baseline = random_text(&mut rng);
            let live = random_text(&mut rng);
            let mut current = baseline.clone();

            let mut steps = 0;
            loop {
                let diff = diff_texts(&current, &live);
                if diff.hunks.is_empty() {
                    break;
                }
                let index = rng.range(diff.hunks.len());
                current = apply_hunk(&current, &live, &diff.hunks[index]).unwrap();
                steps += 1;
                assert!(steps < 200, "did not converge in case {case}");
            }

            assert_eq!(current, live, "case {case} failed to reach live text");
        }
    }

    #[test]
    fn applying_all_hunks_equals_live() {
        let mut rng = Rng(0x1234_5678_9abc_def0);
        for _ in 0..300 {
            let baseline = random_text(&mut rng);
            let live = random_text(&mut rng);
            assert_eq!(apply_all_hunks(&baseline, &live), live);
        }
    }
}
