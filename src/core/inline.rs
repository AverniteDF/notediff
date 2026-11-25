//! Independently acceptable changes within line hunks. Byte ranges refer to
//! the complete, immutable live text and the current baseline, never GTK rows.

use std::ops::Range;
use std::time::Instant;

use similar::{ChangeTag, TextDiffConfig};

use super::diff::{DIFF_TIMEOUT, DiffResult, LineIndex, is_related_multiline_hunk, replace_bytes};
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

#[cfg(test)]
pub fn changes(baseline: &str, live: &str, diff: &DiffResult) -> Vec<InlineChange> {
    changes_indexed(
        baseline,
        live,
        diff,
        &LineIndex::new(baseline),
        &LineIndex::new(live),
    )
}

pub(crate) fn changes_indexed(
    baseline: &str,
    live: &str,
    diff: &DiffResult,
    old: &LineIndex,
    new: &LineIndex,
) -> Vec<InlineChange> {
    let deadline = Instant::now() + DIFF_TIMEOUT;
    let mut result = Vec::new();
    for (hunk, region) in diff.hunks.iter().enumerate() {
        if Instant::now() >= deadline {
            result.push(InlineChange {
                hunk,
                old: old.bytes(region.old_start..region.old_end()),
                new: new.bytes(region.new_start..region.new_end()),
                kind: ChangeKind::Block,
            });
            continue;
        }
        if region.old_len == region.new_len {
            let start_len = result.len();
            let mut total_non_ws = 0usize;
            let mut equal_non_ws = 0usize;
            let mut equal_alnum = 0usize;
            for offset in 0..region.old_len {
                let old_line = region.old_start + offset;
                let new_line = region.new_start + offset;
                let old_range = old.bytes(old_line..old_line + 1);
                let new_range = new.bytes(new_line..new_line + 1);
                let old = body(&baseline[old_range.clone()]);
                let new = body(&live[new_range.clone()]);
                let mut old_pos = old_range.start;
                let mut new_pos = new_range.start;
                let mut current: Option<InlineChange> = None;
                let mut bridged = false;
                let mut config = TextDiffConfig::new();
                config.deadline(deadline);
                for change in config.diff_unicode_words(old, new).iter_all_changes() {
                    let len = change.value().len();
                    let non_ws = change
                        .value()
                        .chars()
                        .filter(|ch| !ch.is_whitespace())
                        .count();
                    if change.tag() == ChangeTag::Equal {
                        total_non_ws += non_ws * 2;
                        equal_non_ws += non_ws;
                        equal_alnum += change
                            .value()
                            .chars()
                            .filter(|ch| ch.is_alphanumeric())
                            .count();
                        // Equal whitespace bridges adjacent changes into one
                        // phrase-level replacement; unchanged words or
                        // punctuation still separate review units.
                        if non_ws > 0 {
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
                        total_non_ws += non_ws;
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
                let old_end = old_range.start + old.len();
                let new_end = new_range.start + new.len();
                if baseline[old_end..old_range.end] != live[new_end..new_range.end] {
                    result.push(InlineChange {
                        hunk,
                        old: old_end..old_range.end,
                        new: new_end..new_range.end,
                        kind: ChangeKind::Ending,
                    });
                }
            }
            if region.old_len > 1
                && !is_related_multiline_hunk(total_non_ws, equal_non_ws, equal_alnum)
            {
                result.truncate(start_len);
                result.push(InlineChange {
                    hunk,
                    old: old.bytes(region.old_start..region.old_end()),
                    new: new.bytes(region.new_start..region.new_end()),
                    kind: ChangeKind::Block,
                });
            }
        } else {
            // A wholly inserted/deleted region is one atomic block. Unequal
            // replacements also stay atomic: they have no reliable pairing.
            result.push(InlineChange {
                hunk,
                old: old.bytes(region.old_start..region.old_end()),
                new: new.bytes(region.new_start..region.new_end()),
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
    if let Some(previous) = out.last_mut()
        && previous.hunk == change.hunk
        && previous.kind == ChangeKind::Text
        && change.kind == ChangeKind::Text
    {
        let old_gap = &baseline[previous.old.end..change.old.start];
        let new_gap = &live[previous.new.end..change.new.start];
        if old_gap == new_gap && old_gap.chars().all(char::is_whitespace) {
            previous.old.end = change.old.end;
            previous.new.end = change.new.end;
            trim_common_whitespace(baseline, live, previous);
            return;
        }
    }
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
    Ok(replace_bytes(baseline, change.old.clone(), replacement))
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

    #[test]
    fn whitespace_bridges_adjacent_changes_across_matching_newlines() {
        // Two consecutive changed lines separated only by "\n    " bridge into
        // one unit when the hunk shares enough unchanged content elsewhere.
        let old = "keep prefix old_one\n    old_two\nkeep suffix\n";
        let new = "keep prefix new_one\n    new_two\nkeep suffix\n";
        let items = changes(old, new, &diff_texts(old, new));
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(&old[items[0].old.clone()], "old_one\n    old_two");
        assert_eq!(&new[items[0].new.clone()], "new_one\n    new_two");
        assert_eq!(apply(old, new, &items[0]).unwrap(), new);

        // Differing line endings stay separate Ending units and do not bridge.
        let old = "keep one\r\nkeep two\n";
        let new = "keep ONE\nkeep TWO\n";
        let items = changes(old, new, &diff_texts(old, new));
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].kind, ChangeKind::Ending);
    }

    #[test]
    fn unrelated_equal_length_multiline_blocks_stay_atomic() {
        let old = "#[derive(Clone, Copy, PartialEq, Eq)]\nenum LinkState {\n    /// No document path.\n    NoPath,\n    /// Has baseline.\n    HasBaseline,\n    /// Ready.\n    Ready,\n";
        let new = "impl EditMode {\n    fn label(self) -> &'static str {\n        match self {\n            EditMode::Insert => \"INS\",\n            EditMode::Overwrite => \"OVR\",\n            EditMode::ReadOnly => \"RO\",\n        }\n    }\n";
        let diff = diff_texts(old, new);
        let items = changes(old, new, &diff);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].kind, ChangeKind::Block);
        assert_eq!(apply(old, new, &items[0]).unwrap(), new);
        // Full-diff inline highlights also fall back to whole lines.
        assert_eq!(diff.baseline_inline.len(), 8);
        assert_eq!(diff.live_inline.len(), 8);
    }
}
