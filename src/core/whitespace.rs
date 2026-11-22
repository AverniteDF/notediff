//! Leading and trailing whitespace detection and trimming, free of any
//! toolkit types.
//!
//! Whitespace means ASCII spaces and tabs at the start or end of a line's
//! content. Lines use the GTK line model from `textlines`, so `\r\n`, a lone
//! `\r` and U+2029 PARAGRAPH SEPARATOR are terminators. A whitespace-only line
//! counts; a truly empty line does not.

use std::ops::{Range, RangeInclusive};

use super::textlines;

/// True when any line's content ends in an ASCII space or tab.
#[cfg(test)]
pub fn has_trailing(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if byte != b' ' && byte != b'\t' {
            continue;
        }
        // The next character is a line break, or the text ends here.
        let next = index + 1;
        if next == bytes.len() || textlines::break_len(text, next).is_some() {
            return true;
        }
    }
    false
}

/// True when any line's content is non-empty and contains only ASCII spaces
/// and tabs. The synthetic empty line after a final break does not count.
#[cfg(test)]
pub fn has_non_empty_blank_line(text: &str) -> bool {
    let mut index = 0;
    while index < text.len() {
        let line_start = index;
        let mut content_end = index;
        while content_end < text.len() {
            match text.as_bytes()[content_end] {
                b' ' | b'\t' => content_end += 1,
                _ => break,
            }
        }
        if content_end > line_start
            && (content_end == text.len() || textlines::break_len(text, content_end).is_some())
        {
            return true;
        }
        // Skip the rest of this line.
        match textlines::next_break(text, content_end) {
            Some((pos, len)) => index = pos + len,
            None => return false,
        }
    }
    false
}

/// True when any line in `lines` (0-based, inclusive) ends in ASCII spaces or
/// tabs. `None` checks the whole text.
pub fn has_trailing_in(text: &str, lines: Option<RangeInclusive<usize>>) -> bool {
    textlines::iter_all(text)
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .any(|(_, line)| trailing_len(textlines::content(line)) > 0)
}

/// True when any line in `lines` (0-based, inclusive) begins with ASCII spaces
/// or tabs. `None` checks the whole text.
pub fn has_leading_in(text: &str, lines: Option<RangeInclusive<usize>>) -> bool {
    textlines::iter_all(text)
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .any(|(_, line)| leading_len(textlines::content(line)) > 0)
}

/// One line whose leading- or trailing-whitespace run is removed. `line` is
/// 0-based; `start` and `len` are character offsets and counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrimChange {
    pub line: usize,
    pub start: usize,
    pub len: usize,
}

/// Iterate non-terminating target lines (`(0-based index, content)`).
/// `iter_inclusive` omits the synthetic empty line after a final break.
fn non_terminating_target_lines(
    text: &str,
    lines: Option<RangeInclusive<usize>>,
) -> impl Iterator<Item = (usize, &str)> {
    textlines::iter_inclusive(text)
        .enumerate()
        .filter(move |(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .map(|(index, line)| (index, textlines::content(line)))
}

/// True when any target line is blank: its content is empty or contains only
/// ASCII spaces and tabs. The synthetic empty line after a final break does
/// not count.
pub fn has_blank_line_remove(text: &str, lines: Option<RangeInclusive<usize>>) -> bool {
    non_terminating_target_lines(text, lines).any(|(_, content)| is_blank_line(content))
}

/// True when any target blank line contains at least one ASCII space or tab.
/// The synthetic empty line after a final break does not count.
pub fn has_blank_line_strip(text: &str, lines: Option<RangeInclusive<usize>>) -> bool {
    non_terminating_target_lines(text, lines)
        .any(|(_, content)| !content.is_empty() && is_blank_line(content))
}

/// Blank-line removals for `lines` (0-based, inclusive), or for the whole text
/// when `None`. Returns the 0-based line indices to delete, in ascending order.
pub fn blank_line_remove_plan(text: &str, lines: Option<RangeInclusive<usize>>) -> Vec<usize> {
    non_terminating_target_lines(text, lines)
        .filter(|(_, content)| is_blank_line(content))
        .map(|(index, _)| index)
        .collect()
}

/// Whitespace removals for blank lines in `lines` (0-based, inclusive), or for
/// the whole text when `None`. Only blank lines with at least one space or tab
/// are returned.
pub fn blank_line_strip_plan(text: &str, lines: Option<RangeInclusive<usize>>) -> Vec<TrimChange> {
    non_terminating_target_lines(text, lines)
        .filter_map(|(line, content)| {
            if content.is_empty() || !is_blank_line(content) {
                return None;
            }
            Some(TrimChange {
                line,
                start: 0,
                len: content.len(),
            })
        })
        .collect()
}

/// True when `content` is empty or contains only ASCII spaces and tabs.
fn is_blank_line(content: &str) -> bool {
    content.bytes().all(|byte| byte == b' ' || byte == b'\t')
}

/// Leading-whitespace removals for `lines` (0-based, inclusive), or for the
/// whole text when `None`. Only lines that change are returned.
pub fn trim_leading_plan(text: &str, lines: Option<RangeInclusive<usize>>) -> Vec<TrimChange> {
    textlines::split_all(text)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .filter_map(|(line, text)| {
            let len = leading_len(textlines::content(text));
            if len == 0 {
                return None;
            }
            Some(TrimChange {
                line,
                start: 0,
                len,
            })
        })
        .collect()
}

/// Trailing-whitespace removals for `lines` (0-based, inclusive), or for the
/// whole text when `None`. Only lines that change are returned.
pub fn trim_trailing_plan(text: &str, lines: Option<RangeInclusive<usize>>) -> Vec<TrimChange> {
    textlines::split_all(text)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .filter_map(|(line, text)| {
            let content = textlines::content(text);
            let len = trailing_len(content);
            if len == 0 {
                return None;
            }
            let content_chars = content.chars().count();
            Some(TrimChange {
                line,
                start: content_chars - len,
                len,
            })
        })
        .collect()
}

/// Character range of the next trailing-whitespace run at or after `from_char`,
/// wrapping to the first run when there is none. `None` when the text has no
/// trailing whitespace. Offsets are character positions as reported by
/// GtkTextBuffer; a run never includes its line terminator.
///
/// The first run whose end is after `from_char` is returned. A cursor inside a
/// run therefore selects that whole run, and a cursor left at the end of a
/// selected run advances to the next one.
pub fn next_trailing(text: &str, from_char: usize) -> Option<Range<usize>> {
    let mut first = None;
    let mut line_start = 0usize;
    for line in textlines::split_all(text) {
        let content = textlines::content(line);
        let trailing = trailing_len(content);
        if trailing > 0 {
            let content_chars = content.chars().count();
            let run = line_start + content_chars - trailing..line_start + content_chars;
            if first.is_none() {
                first = Some(run.clone());
            }
            if run.end > from_char {
                return Some(run);
            }
        }
        line_start += line.chars().count();
    }
    first
}

/// Character range of the next non-empty blank line's content at or after
/// `from_char`, wrapping to the first one when there is none. `None` when no
/// line contains only ASCII spaces and tabs.
pub fn next_non_empty_blank(text: &str, from_char: usize) -> Option<Range<usize>> {
    let mut first = None;
    let mut line_start = 0usize;
    for line in textlines::split_all(text) {
        let content = textlines::content(line);
        if !content.is_empty() && is_blank_line(content) {
            let content_chars = content.chars().count();
            let run = line_start..line_start + content_chars;
            if first.is_none() {
                first = Some(run.clone());
            }
            if run.end > from_char {
                return Some(run);
            }
        }
        line_start += line.chars().count();
    }
    first
}

/// Byte length of the run of ASCII spaces and tabs at the start of `content`.
/// Spaces and tabs are one byte each, so this is also the length in characters.
fn leading_len(content: &str) -> usize {
    content.len() - content.trim_start_matches([' ', '\t']).len()
}

/// Byte length of the run of ASCII spaces and tabs at the end of `content`.
/// Spaces and tabs are one byte each, so this is also the length in characters.
fn trailing_len(content: &str) -> usize {
    content.len() - content.trim_end_matches([' ', '\t']).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_spaces_and_tabs_before_every_terminator() {
        for text in [
            "a \n",
            "a\t\n",
            "a \r\n",
            "a\t\r",
            "a \u{2029}",
            "a ",
            "   \n",
            "\t\n",
            "\n \n",
        ] {
            assert!(has_trailing(text), "{text:?}");
        }
    }

    #[test]
    fn ignores_clean_text_and_empty_lines() {
        for text in ["", "\n", "a\n", "a\nb\n", "a\u{a0}\n", "a\u{2003}\n"] {
            assert!(!has_trailing(text), "{text:?}");
        }
    }

    #[test]
    fn detects_non_empty_blank_lines() {
        for text in [
            " \n",
            "\t\n",
            "   ",
            "a\n \n",
            "a\r\n\t\r\n",
            "a\u{2029} \u{2029}",
            " \n \n",
            "a\n \t",
        ] {
            assert!(has_non_empty_blank_line(text), "{text:?}");
        }
        for text in ["", "\n", "a\n", "a\n\n", "a\u{a0}\n", "a\n  b\n"] {
            assert!(!has_non_empty_blank_line(text), "{text:?}");
        }
    }

    #[test]
    fn line_limited_detection_follows_the_selection() {
        let text = "a \nb\nc\t\n";
        assert!(has_trailing_in(text, None));
        assert!(has_trailing_in(text, Some(0..=0)));
        assert!(!has_trailing_in(text, Some(1..=1)));
        assert!(has_trailing_in(text, Some(2..=2)));
        assert!(!has_trailing_in("a\nb\n", None));
    }

    #[test]
    fn line_limited_leading_detection_follows_the_selection() {
        let text = " a\nb\n\tc\n";
        assert!(has_leading_in(text, None));
        assert!(has_leading_in(text, Some(0..=0)));
        assert!(!has_leading_in(text, Some(1..=1)));
        assert!(has_leading_in(text, Some(2..=2)));
        assert!(!has_leading_in("a\nb\n", None));
    }

    #[test]
    fn trim_leading_plan_reports_every_leading_run() {
        assert_eq!(
            trim_leading_plan(" a\n\tb\t\nc\n", None),
            vec![
                TrimChange {
                    line: 0,
                    start: 0,
                    len: 1
                },
                TrimChange {
                    line: 1,
                    start: 0,
                    len: 1
                },
            ]
        );
        assert_eq!(
            trim_leading_plan(" a\n\tb\t\nc\n", Some(1..=1)),
            vec![TrimChange {
                line: 1,
                start: 0,
                len: 1
            }]
        );
        assert!(trim_leading_plan("a\nb\n", None).is_empty());
    }

    #[test]
    fn trim_leading_plan_handles_whitespace_only_unicode_and_breaks() {
        assert_eq!(
            trim_leading_plan("   \n", None),
            vec![TrimChange {
                line: 0,
                start: 0,
                len: 3
            }]
        );
        assert_eq!(
            trim_leading_plan(" \té\r\n", None),
            vec![TrimChange {
                line: 0,
                start: 0,
                len: 2
            }]
        );
        assert_eq!(
            trim_leading_plan("\t\r", None),
            vec![TrimChange {
                line: 0,
                start: 0,
                len: 1
            }]
        );
        assert_eq!(
            trim_leading_plan(" x\u{2029}\ty\u{2029}", None),
            vec![
                TrimChange {
                    line: 0,
                    start: 0,
                    len: 1
                },
                TrimChange {
                    line: 1,
                    start: 0,
                    len: 1
                },
            ]
        );
    }

    #[test]
    fn blank_line_plans_handle_remove_and_strip() {
        let text = "a\n\n \n b\n\nc\n";
        assert!(has_blank_line_remove(text, None));
        assert!(has_blank_line_strip(text, None));
        assert_eq!(blank_line_remove_plan(text, None), vec![1, 2, 4]);
        assert_eq!(
            blank_line_strip_plan(text, None),
            vec![TrimChange {
                line: 2,
                start: 0,
                len: 1
            }]
        );

        // The synthetic empty line after a final break is ignored.
        assert!(blank_line_remove_plan("a\n", None).is_empty());
        assert_eq!(blank_line_remove_plan("\n", None), vec![0]);

        // An empty document has no removable blank lines either.
        assert!(!has_blank_line_remove("", None));
        assert!(!has_blank_line_strip("", None));
        assert!(blank_line_remove_plan("", None).is_empty());
        assert!(blank_line_strip_plan("", None).is_empty());

        // A single space is a real blank line and is removable.
        assert!(has_blank_line_remove(" ", None));
        assert!(has_blank_line_strip(" ", None));
        assert_eq!(blank_line_remove_plan(" ", None), vec![0]);
        assert_eq!(
            blank_line_strip_plan(" ", None),
            vec![TrimChange {
                line: 0,
                start: 0,
                len: 1
            }]
        );

        // Selection limits both plans.
        assert_eq!(blank_line_remove_plan(text, Some(2..=2)), vec![2]);
        assert!(has_blank_line_strip(text, Some(2..=2)));
        assert!(!has_blank_line_strip(text, Some(1..=1)));
        assert!(has_blank_line_remove(text, Some(1..=1)));

        // Truly empty and whitespace-only lines count as blank for remove.
        assert_eq!(blank_line_remove_plan(" \n\t\n", None), vec![0, 1]);
        assert_eq!(
            blank_line_strip_plan(" \n\t\n", None),
            vec![
                TrimChange {
                    line: 0,
                    start: 0,
                    len: 1
                },
                TrimChange {
                    line: 1,
                    start: 0,
                    len: 1
                },
            ]
        );

        // Every GTK line terminator is a line boundary.
        assert_eq!(blank_line_remove_plan("a\r\n \r\nb\n", None), vec![1]);
        assert_eq!(
            blank_line_remove_plan("a\u{2029}\t\u{2029}b", None),
            vec![1]
        );
    }

    #[test]
    fn trim_plan_reports_every_trailing_run() {
        assert_eq!(
            trim_trailing_plan("a \n b\t\nc\n", None),
            vec![
                TrimChange {
                    line: 0,
                    start: 1,
                    len: 1
                },
                TrimChange {
                    line: 1,
                    start: 2,
                    len: 1
                },
            ]
        );
        assert_eq!(
            trim_trailing_plan("a \n b\t\nc\n", Some(1..=1)),
            vec![TrimChange {
                line: 1,
                start: 2,
                len: 1
            }]
        );
        assert!(trim_trailing_plan("a\nb\n", None).is_empty());
    }

    #[test]
    fn trim_plan_handles_whitespace_only_unicode_and_breaks() {
        assert_eq!(
            trim_trailing_plan("   \n", None),
            vec![TrimChange {
                line: 0,
                start: 0,
                len: 3
            }]
        );
        assert_eq!(
            trim_trailing_plan("é \r\n", None),
            vec![TrimChange {
                line: 0,
                start: 1,
                len: 1
            }]
        );
        assert_eq!(
            trim_trailing_plan("\t\r", None),
            vec![TrimChange {
                line: 0,
                start: 0,
                len: 1
            }]
        );
        assert_eq!(
            trim_trailing_plan("x \u{2029}y\t\u{2029}", None),
            vec![
                TrimChange {
                    line: 0,
                    start: 1,
                    len: 1
                },
                TrimChange {
                    line: 1,
                    start: 1,
                    len: 1
                },
            ]
        );
    }

    #[test]
    fn next_run_returns_character_offsets_without_terminators() {
        // Two trailing runs: line 0 has one space, line 1 has two tabs.
        let text = "a \n\t\tb \r\n";
        assert_eq!(next_trailing(text, 0), Some(1..2));
        assert_eq!(next_trailing(text, 2), Some(6..7));
        assert_eq!(next_trailing(text, 7), Some(1..2));
        // A cursor inside a run selects the whole run.
        assert_eq!(next_trailing(text, 6), Some(6..7));
        assert_eq!(next_trailing(text, 8), Some(1..2));
    }

    #[test]
    fn next_run_wraps_to_the_first_match() {
        let text = " one \n two \n";
        assert_eq!(next_trailing(text, 5), Some(10..11));
        assert_eq!(next_trailing(text, 11), Some(4..5));
        // Offsets past the end wrap as well.
        assert_eq!(next_trailing(text, 99), Some(4..5));
    }

    #[test]
    fn next_run_handles_unicode_and_every_line_break() {
        // 'é' is one character but two bytes.
        let text = "é \r猫 \u{2029}x ";
        assert_eq!(next_trailing(text, 0), Some(1..2));
        assert_eq!(next_trailing(text, 2), Some(4..5));
        assert_eq!(next_trailing(text, 6), Some(7..8));
        assert_eq!(next_trailing(text, 9), Some(1..2));
    }

    #[test]
    fn a_clean_text_has_no_next_run() {
        assert_eq!(next_trailing("", 0), None);
        assert_eq!(next_trailing("a\nb\n", 0), None);
        assert_eq!(next_trailing("a\u{a0}\n", 0), None);
    }

    #[test]
    fn next_non_empty_blank_skips_other_lines_and_wraps() {
        let text = "one \n \ntwo\n\t\n";
        assert_eq!(next_non_empty_blank(text, 0), Some(5..6));
        assert_eq!(next_non_empty_blank(text, 6), Some(11..12));
        assert_eq!(next_non_empty_blank(text, 12), Some(5..6));
        assert_eq!(next_non_empty_blank(text, 99), Some(5..6));
    }

    #[test]
    fn next_non_empty_blank_handles_breaks_unicode_and_the_final_line() {
        let text = "a\n \r\nb\u{2029}\t\u{2029}";
        assert_eq!(next_non_empty_blank(text, 0), Some(2..3));
        assert_eq!(next_non_empty_blank(text, 3), Some(7..8));

        // A whitespace-only final line without a terminator still counts.
        assert_eq!(next_non_empty_blank("a\n \t", 0), Some(2..4));
        // The synthetic empty line after a final break does not.
        assert_eq!(next_non_empty_blank("a\n\n", 0), None);
        assert_eq!(next_non_empty_blank("a\nb\n", 0), None);
        assert_eq!(next_non_empty_blank("", 0), None);
    }
}
