//! Indentation scaling (double / halve), free of any toolkit types.
//!
//! Indentation means the run of ASCII space characters at the start of a line.
//! Tabs and other whitespace are not indentation. Blank and whitespace-only
//! lines are treated literally: their leading spaces count. Lines are numbered
//! exactly as GtkTextBuffer numbers them, including the empty line after a
//! trailing break.

use std::ops::RangeInclusive;

use super::textlines;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentScale {
    Double,
    Halve,
}

impl IndentScale {
    /// Toast shown when every target line has zero indentation.
    pub fn nothing_message(self) -> &'static str {
        match self {
            IndentScale::Double => "No indentation to double",
            IndentScale::Halve => "No indentation to halve",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentError {
    /// Every target line has zero indentation.
    NothingToScale,
    /// Halving is impossible: `line` (1-based) has an odd number of leading
    /// spaces. It is the first such line.
    OddIndent { line: usize },
}

impl IndentError {
    /// The toast text for this error.
    pub fn message(self, scale: IndentScale) -> String {
        match self {
            IndentError::NothingToScale => scale.nothing_message().to_string(),
            IndentError::OddIndent { line } => {
                format!("Cannot halve indentation: line {line} has an odd number of leading spaces")
            }
        }
    }
}

/// One line whose indentation changes. `line` is 0-based; `old` and `new` are
/// counts of leading spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndentChange {
    pub line: usize,
    pub old: usize,
    pub new: usize,
}

/// Number of leading ASCII space characters.
pub fn leading_spaces(line: &str) -> usize {
    line.bytes().take_while(|&byte| byte == b' ').count()
}

/// Work out the indentation changes for `scale`. `lines` is a 0-based,
/// inclusive range of target lines, or `None` for the whole text. Only lines
/// that actually change are returned. Nothing is returned on error, so the
/// operation is all-or-nothing.
pub fn plan(
    text: &str,
    scale: IndentScale,
    lines: Option<RangeInclusive<usize>>,
) -> Result<Vec<IndentChange>, IndentError> {
    let targets: Vec<(usize, usize)> = textlines::split_all(text)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .map(|(index, line)| (index, leading_spaces(line)))
        .collect();

    if targets.iter().all(|&(_, spaces)| spaces == 0) {
        return Err(IndentError::NothingToScale);
    }

    if scale == IndentScale::Halve
        && let Some(&(index, _)) = targets.iter().find(|&&(_, spaces)| spaces % 2 == 1)
    {
        return Err(IndentError::OddIndent { line: index + 1 });
    }

    Ok(targets
        .into_iter()
        .filter(|&(_, spaces)| spaces > 0)
        .map(|(line, old)| IndentChange {
            line,
            old,
            new: match scale {
                IndentScale::Double => old * 2,
                IndentScale::Halve => old / 2,
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply a plan to text, the way the window applies it to the buffer.
    fn apply(text: &str, changes: &[IndentChange]) -> String {
        let mut out = String::new();
        for (index, line) in textlines::split_all(text).into_iter().enumerate() {
            match changes.iter().find(|c| c.line == index) {
                Some(change) => {
                    out.push_str(&" ".repeat(change.new));
                    out.push_str(&line[change.old..]);
                }
                None => out.push_str(line),
            }
        }
        out
    }

    fn run(text: &str, scale: IndentScale, lines: Option<RangeInclusive<usize>>) -> String {
        apply(text, &plan(text, scale, lines).unwrap())
    }

    #[test]
    fn counts_ascii_spaces_only() {
        assert_eq!(leading_spaces(""), 0);
        assert_eq!(leading_spaces("abc"), 0);
        assert_eq!(leading_spaces("   abc"), 3);
        assert_eq!(leading_spaces("   "), 3);
        assert_eq!(leading_spaces("\t  x"), 0);
        assert_eq!(leading_spaces(" \t x"), 1);
        assert_eq!(leading_spaces("\u{a0} x"), 0);
    }

    #[test]
    fn doubles_the_whole_text() {
        assert_eq!(
            run("a\n  b\n    c\n", IndentScale::Double, None),
            "a\n    b\n        c\n"
        );
    }

    #[test]
    fn halves_the_whole_text() {
        assert_eq!(
            run("a\n    b\n  c\n", IndentScale::Halve, None),
            "a\n  b\n c\n"
        );
    }

    #[test]
    fn selection_limits_the_target_lines() {
        // Odd indentation outside the range does not block halving.
        let text = " x\n  y\n    z\n   w\n";
        assert_eq!(
            run(text, IndentScale::Halve, Some(1..=2)),
            " x\n y\n  z\n   w\n"
        );
        assert_eq!(
            run(text, IndentScale::Double, Some(2..=2)),
            " x\n  y\n        z\n   w\n"
        );
    }

    #[test]
    fn all_zero_indentation_aborts() {
        for scale in [IndentScale::Double, IndentScale::Halve] {
            assert_eq!(
                plan("a\nb\n", scale, None),
                Err(IndentError::NothingToScale)
            );
            assert_eq!(plan("", scale, None), Err(IndentError::NothingToScale));
        }
        assert_eq!(
            plan("a\n  b\n", IndentScale::Double, Some(0..=0)),
            Err(IndentError::NothingToScale)
        );
        assert_eq!(
            plan("a\n  b\n", IndentScale::Halve, Some(0..=0)),
            Err(IndentError::NothingToScale)
        );
    }

    #[test]
    fn halve_reports_first_odd_line_one_based() {
        assert_eq!(
            plan("  a\n   b\n c\n", IndentScale::Halve, None),
            Err(IndentError::OddIndent { line: 2 })
        );
        assert_eq!(
            IndentError::OddIndent { line: 2 }.message(IndentScale::Halve),
            "Cannot halve indentation: line 2 has an odd number of leading spaces"
        );
    }

    #[test]
    fn whitespace_only_lines_count_literally() {
        // A whitespace-only line with an odd number of spaces blocks Halve.
        assert_eq!(
            plan("    a\n   \n", IndentScale::Halve, None),
            Err(IndentError::OddIndent { line: 2 })
        );
        // An empty line is zero indentation and is left alone.
        assert_eq!(
            run("  a\n\n  b\n", IndentScale::Double, None),
            "    a\n\n    b\n"
        );
        // Whitespace-only lines with an even count are scaled.
        assert_eq!(run("  \n", IndentScale::Double, None), "    \n");
        assert_eq!(run("    \n", IndentScale::Halve, None), "  \n");
    }

    #[test]
    fn crlf_and_trailing_line_are_handled() {
        assert_eq!(
            run("  a\r\n  b\r\n", IndentScale::Double, None),
            "    a\r\n    b\r\n"
        );
        // The empty last line (after a trailing break) is a zero-indent line.
        let changes = plan("  a\n", IndentScale::Double, Some(0..=1)).unwrap();
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn every_gtk_line_break_splits_target_lines() {
        for (text, expected) in [
            ("  a\r  b\r", "    a\r    b\r"),
            ("  a\u{2029}  b\u{2029}", "    a\u{2029}    b\u{2029}"),
        ] {
            assert_eq!(run(text, IndentScale::Double, None), expected);
        }
        // The line after a trailing break is empty and has no indentation.
        assert_eq!(run("  a\r", IndentScale::Double, None), "    a\r");
        assert_eq!(
            plan("  a\r", IndentScale::Double, Some(1..=1)),
            Err(IndentError::NothingToScale)
        );
    }

    #[test]
    fn only_changing_lines_are_reported() {
        let changes = plan("a\n  b\n", IndentScale::Double, None).unwrap();
        assert_eq!(
            changes,
            vec![IndentChange {
                line: 1,
                old: 2,
                new: 4
            }]
        );
    }

    #[test]
    fn halve_then_double_roundtrips_even_indentation() {
        let text = "a\n    b\n        c\n  d\n";
        let halved = run(text, IndentScale::Halve, None);
        assert_eq!(run(&halved, IndentScale::Double, None), text);
    }
}
