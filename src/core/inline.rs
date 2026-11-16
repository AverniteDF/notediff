//! Independently acceptable changes within line hunks. Byte ranges refer to
//! the complete, immutable live text and the current baseline, never GTK rows.

use std::ops::Range;
use std::time::Instant;

use similar::{ChangeTag, TextDiffConfig};

use super::diff::{DIFF_TIMEOUT, DiffResult, Hunk, split_lines};
use super::textlines;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Text,
    Block,
    Ending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineChange {
    pub hunk: usize,
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub kind: ChangeKind,
}

pub fn changes(baseline: &str, live: &str, diff: &DiffResult) -> Vec<InlineChange> {
    let deadline = Instant::now() + DIFF_TIMEOUT;
    let old_lines = split_lines(baseline);
    let new_lines = split_lines(live);
    let offsets = |lines: &[&str]| {
        let mut result = vec![0];
        for line in lines {
            result.push(result.last().unwrap() + line.len());
        }
        result
    };
    let old_offsets = offsets(&old_lines);
    let new_offsets = offsets(&new_lines);
    let mut result = Vec::new();
    for (hunk, region) in diff.hunks.iter().enumerate() {
        if Instant::now() >= deadline {
            result.push(InlineChange {
                hunk,
                old: old_offsets[region.old_start]..old_offsets[region.old_end()],
                new: new_offsets[region.new_start]..new_offsets[region.new_end()],
                kind: ChangeKind::Block,
            });
            continue;
        }
        if region.old_len == region.new_len {
            for offset in 0..region.old_len {
                let old_line = region.old_start + offset;
                let new_line = region.new_start + offset;
                let old = body(old_lines[old_line]);
                let new = body(new_lines[new_line]);
                let mut old_pos = old_offsets[old_line];
                let mut new_pos = new_offsets[new_line];
                let mut current: Option<InlineChange> = None;
                let mut bridged = false;
                let mut config = TextDiffConfig::new();
                config.deadline(deadline);
                for change in config.diff_unicode_words(old, new).iter_all_changes() {
                    let len = change.value().len();
                    if change.tag() == ChangeTag::Equal {
                        // Equal whitespace bridges adjacent changes into one
                        // phrase-level replacement; unchanged words or
                        // punctuation still separate review units.
                        if !change.value().chars().all(char::is_whitespace) {
                            bridged = false;
                            if let Some(change) = current.take() {
                                push_text_change(&mut result, change, baseline, live);
                            }
                        } else if current.is_some() {
                            bridged = true;
                        }
                        old_pos += len;
                        new_pos += len;
                    } else {
                        // If another change follows bridged whitespace, both
                        // ranges must cross it. Extending only the changed side
                        // leaves the other side's whitespace in place, which
                        // duplicates spaces when the change is applied.
                        if bridged {
                            if let Some(item) = current.as_mut() {
                                item.old.end = old_pos;
                                item.new.end = new_pos;
                            }
                            bridged = false;
                        }
                        let item = current.get_or_insert(InlineChange {
                            hunk,
                            old: old_pos..old_pos,
                            new: new_pos..new_pos,
                            kind: ChangeKind::Text,
                        });
                        if change.tag() == ChangeTag::Delete {
                            old_pos += len;
                            item.old.end = old_pos;
                        } else {
                            new_pos += len;
                            item.new.end = new_pos;
                        }
                    }
                }
                if let Some(change) = current {
                    push_text_change(&mut result, change, baseline, live);
                }
                let old_end = old_offsets[old_line] + old.len();
                let new_end = new_offsets[new_line] + new.len();
                if baseline[old_end..old_offsets[old_line + 1]]
                    != live[new_end..new_offsets[new_line + 1]]
                {
                    result.push(InlineChange {
                        hunk,
                        old: old_end..old_offsets[old_line + 1],
                        new: new_end..new_offsets[new_line + 1],
                        kind: ChangeKind::Ending,
                    });
                }
            }
        } else {
            // A wholly inserted/deleted region is one atomic block. Unequal
            // replacements also stay atomic: they have no reliable pairing.
            result.push(InlineChange {
                hunk,
                old: old_offsets[region.old_start]..old_offsets[region.old_end()],
                new: new_offsets[region.new_start]..new_offsets[region.new_end()],
                kind: ChangeKind::Block,
            });
        }
    }
    result
}

fn push_text_change(
    out: &mut Vec<InlineChange>,
    mut change: InlineChange,
    baseline: &str,
    live: &str,
) {
    trim_common_whitespace(baseline, live, &mut change);
    out.push(change);
}

/// Keep unchanged boundary whitespace out of a review unit. If both sides
/// carry the same leading or trailing whitespace, it is context rather than
/// part of the edit; leaving it in the range only widens the highlight and can
/// make the replacement include space that the other side already retains.
fn trim_common_whitespace(baseline: &str, live: &str, change: &mut InlineChange) {
    loop {
        let old_char = baseline[change.old.clone()].chars().next();
        let new_char = live[change.new.clone()].chars().next();
        match (old_char, new_char) {
            (Some(old), Some(new)) if old == new && old.is_whitespace() => {
                change.old.start += old.len_utf8();
                change.new.start += new.len_utf8();
            }
            _ => break,
        }
    }
    loop {
        let old_char = baseline[change.old.clone()].chars().next_back();
        let new_char = live[change.new.clone()].chars().next_back();
        match (old_char, new_char) {
            (Some(old), Some(new)) if old == new && old.is_whitespace() => {
                change.old.end -= old.len_utf8();
                change.new.end -= new.len_utf8();
            }
            _ => break,
        }
    }
}

/// Byte range of a hunk's lines in the complete live text. Live text never
/// changes during a review, so this range remains a stable identity for the
/// inspected region even when a re-diff splits or merges hunks.
pub(crate) fn hunk_live_span(live: &str, hunk: &Hunk) -> Range<usize> {
    let lines = split_lines(live);
    let start: usize = lines[..hunk.new_start].iter().map(|line| line.len()).sum();
    let end: usize = start
        + lines[hunk.new_start..hunk.new_end()]
            .iter()
            .map(|line| line.len())
            .sum::<usize>();
    start..end
}

fn body(line: &str) -> &str {
    textlines::content(line)
}

pub fn apply(baseline: &str, live: &str, change: &InlineChange) -> Result<String, String> {
    let Some(replacement) = live.get(change.new.clone()) else {
        return Err("Invalid live inline range".into());
    };
    if baseline.get(change.old.clone()).is_none() {
        return Err("Invalid baseline inline range".into());
    }
    let mut result = baseline.to_string();
    result.replace_range(change.old.clone(), replacement);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::diff::diff_texts;

    #[test]
    fn replacements_are_atomic_and_independent() {
        let old = "The quick brown fox jumps.\n";
        let new = "The fast brown fox sleeps.\n";
        let items = changes(old, new, &diff_texts(old, new));
        assert_eq!(items.len(), 2);
        assert_eq!(&old[items[0].old.clone()], "quick");
        assert_eq!(&new[items[0].new.clone()], "fast");
        assert_eq!(
            apply(old, new, &items[0]).unwrap(),
            "The fast brown fox jumps.\n"
        );
        assert_eq!(
            apply(old, new, &items[1]).unwrap(),
            "The quick brown fox sleeps.\n"
        );
    }

    #[test]
    fn whitespace_bridges_adjacent_inline_changes() {
        let old = "the quick fox jumped over the lazy dog\n";
        let new = "a stealthy cat snuck under a low bush\n";
        let items = changes(old, new, &diff_texts(old, new));
        assert_eq!(items.len(), 1);
        assert_eq!(
            &old[items[0].old.clone()],
            "the quick fox jumped over the lazy dog"
        );
        assert_eq!(
            &new[items[0].new.clone()],
            "a stealthy cat snuck under a low bush"
        );
        assert_eq!(apply(old, new, &items[0]).unwrap(), new);

        let old = "a  b\n";
        let new = "x  y\n";
        let items = changes(old, new, &diff_texts(old, new));
        assert_eq!(items.len(), 1);
        assert_eq!(&old[items[0].old.clone()], "a  b");
        assert_eq!(&new[items[0].new.clone()], "x  y");
        assert_eq!(apply(old, new, &items[0]).unwrap(), new);
    }

    #[test]
    fn accepting_a_change_keeps_bridged_whitespace_balanced() {
        let old = "- Font size and per-window zoom (Ctrl+scroll, Ctrl+plus/minus), with the default size in the config file.\n";
        let live = "- Per-window zoom (Ctrl+scroll, Ctrl+plus/minus) relative to the persisted application-wide font size, with a reset-to-default action.\n";
        let items = changes(old, live, &diff_texts(old, live));
        assert_eq!(&old[items[0].old.clone()], "Font size and per");
        assert_eq!(&live[items[0].new.clone()], "Per");
        assert_eq!(&live[items[1].new.clone()], " relative to");

        let after_first = apply(old, live, &items[0]).unwrap();
        let items = changes(&after_first, live, &diff_texts(&after_first, live));
        assert_eq!(&after_first[items[0].old.clone()], ", with");
        assert_eq!(&live[items[0].new.clone()], " relative to");

        let after_second = apply(&after_first, live, &items[0]).unwrap();
        assert_eq!(
            after_second,
            "- Per-window zoom (Ctrl+scroll, Ctrl+plus/minus) relative to the default size in the config file.\n"
        );
        assert!(!after_second.contains("  "));
    }

    #[test]
    fn non_whitespace_equal_tokens_still_separate_changes() {
        let old = "The quick brown fox jumps.\n";
        let new = "The fast brown fox sleeps.\n";
        let items = changes(old, new, &diff_texts(old, new));
        assert_eq!(items.len(), 2);
        assert_eq!(&old[items[0].old.clone()], "quick");
        assert_eq!(&new[items[0].new.clone()], "fast");
        assert_eq!(&old[items[1].old.clone()], "jumps");
        assert_eq!(&new[items[1].new.clone()], "sleeps");
    }

    #[test]
    fn endings_and_unicode_ranges_are_exact() {
        for (old, new) in [
            ("é old 🙂\r\n", "é new 🙂\n"),
            ("a\r", "a\r\n"),
            ("a\n", "a"),
            ("a", "a\n"),
            ("a  b!\n", "a b?\n"),
        ] {
            let mut current = old.to_string();
            for _ in 0..30 {
                if current == new {
                    break;
                }
                let items = changes(&current, new, &diff_texts(&current, new));
                assert!(!items.is_empty());
                let item = items.last().unwrap();
                assert!(current.is_char_boundary(item.old.start));
                assert!(new.is_char_boundary(item.new.end));
                current = apply(&current, new, item).unwrap();
            }
            assert_eq!(current, new);
        }
    }

    #[test]
    fn unpaired_lines_are_explicit_not_guessed() {
        let items = changes(
            "old\n",
            "new\nextra\n",
            &diff_texts("old\n", "new\nextra\n"),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ChangeKind::Block);
        let items = changes(
            "anchor\n",
            "one\ntwo\nanchor\n",
            &diff_texts("anchor\n", "one\ntwo\nanchor\n"),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ChangeKind::Block);
        assert!(items[0].old.is_empty());
    }

    #[test]
    fn wholly_inserted_and_deleted_blocks_are_atomic_and_byte_exact() {
        for (plain, inserted) in [
            ("anchor\n", "one\r\n\n🙂three\nanchor\n"),
            ("anchor\n", "anchor\n\nlast without newline"),
            ("", "α\r\n\nlast"),
            ("", "\n\n"),
        ] {
            for (old, new) in [(plain, inserted), (inserted, plain)] {
                let items = changes(old, new, &diff_texts(old, new));
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].kind, ChangeKind::Block);
                assert!(items[0].old.is_empty() || items[0].new.is_empty());
                assert_eq!(apply(old, new, &items[0]).unwrap(), new);
            }
        }
    }

    #[test]
    fn invalid_utf8_or_out_of_bounds_ranges_are_refused() {
        let mut change = InlineChange {
            hunk: 0,
            old: 1..2,
            new: 0..1,
            kind: ChangeKind::Text,
        };
        assert!(apply("é", "x", &change).is_err());
        change.old = 0..2;
        change.new = 0..2;
        assert!(apply("é", "x", &change).is_err());
    }

    #[test]
    fn text_insertions_and_deletions_keep_the_empty_side_empty() {
        for (old, new) in [
            ("one three\n", "one two three\n"),
            ("one two three\n", "one three\n"),
        ] {
            let items = changes(old, new, &diff_texts(old, new));
            assert_eq!(items.len(), 1);
            assert!(items[0].old.is_empty() || items[0].new.is_empty());
            assert_eq!(apply(old, new, &items[0]).unwrap(), new);
        }
    }
}
