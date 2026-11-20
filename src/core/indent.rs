//! Indentation editing, free of any toolkit types.
//!
//! Indentation means the run of ASCII space characters at the start of a line.
//! Tabs and other whitespace are not indentation. Blank and whitespace-only
//! lines are treated literally: their leading spaces count. Lines are numbered
//! exactly as GtkTextBuffer numbers them, including the empty line after a
//! trailing break.
//!
//! Increase adds one space to every target line, including truly empty ones.
//! Decrease requires every participating line to have at least one leading
//! space and silently changes nothing otherwise. It ignores the empty line
//! after a terminating line break, so a normal newline-terminated file does not
//! block the operation; other empty lines still participate.

use std::ops::RangeInclusive;

use super::textlines;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentOperation {
    IncreaseIncludeEmpty,
    DecreaseNoFlattening,
    Double,
    Halve,
}

impl IndentOperation {
    /// Toast shown when every target line has zero indentation. Increase and
    /// Decrease never fail, so they have no message.
    pub fn nothing_message(self) -> Option<&'static str> {
        match self {
            IndentOperation::Double => Some("No indentation to double"),
            IndentOperation::Halve => Some("No indentation to halve"),
            IndentOperation::IncreaseIncludeEmpty | IndentOperation::DecreaseNoFlattening => None,
        }
    }

    /// The new leading-space count for a line with `old` spaces, or `None` when
    /// the line does not change.
    fn changed(self, old: usize) -> Option<usize> {
        match self {
            IndentOperation::IncreaseIncludeEmpty => Some(old + 1),
            IndentOperation::DecreaseNoFlattening => (old > 0).then(|| old - 1),
            IndentOperation::Double => (old > 0).then_some(old * 2),
            IndentOperation::Halve => (old > 0).then_some(old / 2),
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
    pub fn message(self, operation: IndentOperation) -> String {
        match self {
            IndentError::NothingToScale => {
                operation.nothing_message().unwrap_or_default().to_string()
            }
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

/// Work out the indentation changes for `operation`. `lines` is a 0-based,
/// inclusive range of target lines, or `None` for the whole text. Only lines
/// that actually change are returned.
///
/// Increase and Decrease never fail. Double and Halve are all-or-nothing and
/// return `NothingToScale` or `OddIndent` without a change list.
pub fn plan(
    text: &str,
    operation: IndentOperation,
    lines: Option<RangeInclusive<usize>>,
) -> Result<Vec<IndentChange>, IndentError> {
    let text_lines = textlines::split_all(text);
    let last_line = text_lines.len().saturating_sub(1);
    let ignore_terminating_line =
        operation == IndentOperation::DecreaseNoFlattening && textlines::ends_with_break(text);

    let targets: Vec<(usize, usize)> = text_lines
        .into_iter()
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .filter(|(index, _)| !(ignore_terminating_line && *index == last_line))
        .map(|(index, line)| (index, leading_spaces(line)))
        .collect();

    // Decrease only works when every participating line can lose a space. It
    // fails silently, without a toast.
    if operation == IndentOperation::DecreaseNoFlattening
        && targets.iter().any(|&(_, spaces)| spaces == 0)
    {
        return Ok(Vec::new());
    }

    match operation {
        IndentOperation::Double | IndentOperation::Halve => {
            if targets.iter().all(|&(_, spaces)| spaces == 0) {
                return Err(IndentError::NothingToScale);
            }

            if operation == IndentOperation::Halve
                && let Some(&(index, _)) = targets.iter().find(|&&(_, spaces)| spaces % 2 == 1)
            {
                return Err(IndentError::OddIndent { line: index + 1 });
            }

            Ok(changes(targets, operation))
        }
        IndentOperation::IncreaseIncludeEmpty | IndentOperation::DecreaseNoFlattening => {
            Ok(changes(targets, operation))
        }
    }
}

fn changes(targets: Vec<(usize, usize)>, operation: IndentOperation) -> Vec<IndentChange> {
    targets
        .into_iter()
        .filter_map(|(line, old)| {
            operation
                .changed(old)
                .map(|new| IndentChange { line, old, new })
        })
        .collect()
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

    fn run(text: &str, operation: IndentOperation, lines: Option<RangeInclusive<usize>>) -> String {
        apply(text, &plan(text, operation, lines).unwrap())
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
    fn increase_include_empty_adds_to_every_target_line() {
        assert_eq!(
            run("a\n  b\n", IndentOperation::IncreaseIncludeEmpty, None),
            " a\n   b\n "
        );
        assert_eq!(
            run("a\n  b", IndentOperation::IncreaseIncludeEmpty, None),
            " a\n   b"
        );
    }

    #[test]
    fn strict_decrease_ignores_the_terminating_line() {
        assert_eq!(
            run(" a\n   b\n", IndentOperation::DecreaseNoFlattening, None),
            "a\n  b\n"
        );
        assert_eq!(
            run(" a\n   b", IndentOperation::DecreaseNoFlattening, None),
            "a\n  b"
        );
        assert_eq!(
            run(" a\r\n b\r\n", IndentOperation::DecreaseNoFlattening, None),
            "a\r\nb\r\n"
        );
        assert_eq!(
            run(
                " a\u{2029} b\u{2029}",
                IndentOperation::DecreaseNoFlattening,
                None
            ),
            "a\u{2029}b\u{2029}"
        );
    }

    #[test]
    fn strict_decrease_fails_silently_on_any_zero_indent_line() {
        assert_eq!(
            plan("a\n  b\n", IndentOperation::DecreaseNoFlattening, None),
            Ok(Vec::new())
        );
        // The terminating line is ignored, but a deliberate middle empty line
        // still participates and blocks the operation.
        assert_eq!(
            plan("  a\n\n  b\n", IndentOperation::DecreaseNoFlattening, None),
            Ok(Vec::new())
        );
        // With only indented lines, the terminating line is no obstacle.
        assert_eq!(
            plan("  a\n", IndentOperation::DecreaseNoFlattening, None),
            Ok(vec![IndentChange {
                line: 0,
                old: 2,
                new: 1
            }])
        );
        // A selection that includes only the terminating empty line does
        // nothing because that line is ignored.
        assert_eq!(
            plan(" a\n", IndentOperation::DecreaseNoFlattening, Some(1..=1)),
            Ok(Vec::new())
        );
        // A selection containing an unindented non-empty line fails silently.
        assert_eq!(
            plan(
                " a\nb\n c\n",
                IndentOperation::DecreaseNoFlattening,
                Some(0..=1)
            ),
            Ok(Vec::new())
        );
    }

    #[test]
    fn strict_decrease_works_across_selected_indented_lines() {
        assert_eq!(
            run(
                " a\n b\n c\n",
                IndentOperation::DecreaseNoFlattening,
                Some(1..=2)
            ),
            " a\nb\nc\n"
        );
    }

    #[test]
    fn increase_never_fails() {
        assert_eq!(
            plan("", IndentOperation::IncreaseIncludeEmpty, None),
            Ok(vec![IndentChange {
                line: 0,
                old: 0,
                new: 1
            }])
        );
        assert_eq!(
            plan("a\n", IndentOperation::IncreaseIncludeEmpty, None),
            Ok(vec![
                IndentChange {
                    line: 0,
                    old: 0,
                    new: 1
                },
                IndentChange {
                    line: 1,
                    old: 0,
                    new: 1
                }
            ])
        );
    }

    #[test]
    fn doubles_the_whole_text() {
        assert_eq!(
            run("a\n  b\n    c\n", IndentOperation::Double, None),
            "a\n    b\n        c\n"
        );
    }

    #[test]
    fn halves_the_whole_text() {
        assert_eq!(
            run("a\n    b\n  c\n", IndentOperation::Halve, None),
            "a\n  b\n c\n"
        );
    }

    #[test]
    fn selection_limits_the_target_lines() {
        // Odd indentation outside the range does not block halving.
        let text = " x\n  y\n    z\n   w\n";
        assert_eq!(
            run(text, IndentOperation::Halve, Some(1..=2)),
            " x\n y\n  z\n   w\n"
        );
        assert_eq!(
            run(text, IndentOperation::Double, Some(2..=2)),
            " x\n  y\n        z\n   w\n"
        );
        assert_eq!(
            run(text, IndentOperation::IncreaseIncludeEmpty, Some(1..=1)),
            " x\n   y\n    z\n   w\n"
        );
    }

    #[test]
    fn all_zero_indentation_aborts() {
        for operation in [IndentOperation::Double, IndentOperation::Halve] {
            assert_eq!(
                plan("a\nb\n", operation, None),
                Err(IndentError::NothingToScale)
            );
            assert_eq!(plan("", operation, None), Err(IndentError::NothingToScale));
        }
        assert_eq!(
            plan("a\n  b\n", IndentOperation::Double, Some(0..=0)),
            Err(IndentError::NothingToScale)
        );
        assert_eq!(
            plan("a\n  b\n", IndentOperation::Halve, Some(0..=0)),
            Err(IndentError::NothingToScale)
        );
    }

    #[test]
    fn halve_reports_first_odd_line_one_based() {
        assert_eq!(
            plan("  a\n   b\n c\n", IndentOperation::Halve, None),
            Err(IndentError::OddIndent { line: 2 })
        );
        assert_eq!(
            IndentError::OddIndent { line: 2 }.message(IndentOperation::Halve),
            "Cannot halve indentation: line 2 has an odd number of leading spaces"
        );
        assert_eq!(
            IndentError::NothingToScale.message(IndentOperation::Double),
            "No indentation to double"
        );
        assert_eq!(
            IndentError::NothingToScale.message(IndentOperation::Halve),
            "No indentation to halve"
        );
    }

    #[test]
    fn whitespace_only_lines_count_literally() {
        // A whitespace-only line with an odd number of spaces blocks Halve.
        assert_eq!(
            plan("    a\n   \n", IndentOperation::Halve, None),
            Err(IndentError::OddIndent { line: 2 })
        );
        // An empty line is zero indentation and is left alone.
        assert_eq!(
            run("  a\n\n  b\n", IndentOperation::Double, None),
            "    a\n\n    b\n"
        );
        // Whitespace-only lines with an even count are scaled.
        assert_eq!(run("  \n", IndentOperation::Double, None), "    \n");
        assert_eq!(run("    \n", IndentOperation::Halve, None), "  \n");
    }

    #[test]
    fn crlf_and_trailing_line_are_handled() {
        assert_eq!(
            run("  a\r\n  b\r\n", IndentOperation::Double, None),
            "    a\r\n    b\r\n"
        );
        // The empty last line (after a trailing break) is a zero-indent line.
        let changes = plan("  a\n", IndentOperation::Double, Some(0..=1)).unwrap();
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn every_gtk_line_break_splits_target_lines() {
        for (text, expected) in [
            ("  a\r  b\r", "    a\r    b\r"),
            ("  a\u{2029}  b\u{2029}", "    a\u{2029}    b\u{2029}"),
        ] {
            assert_eq!(run(text, IndentOperation::Double, None), expected);
        }
        // The line after a trailing break is empty and has no indentation.
        assert_eq!(run("  a\r", IndentOperation::Double, None), "    a\r");
        assert_eq!(
            plan("  a\r", IndentOperation::Double, Some(1..=1)),
            Err(IndentError::NothingToScale)
        );
    }

    #[test]
    fn only_changing_lines_are_reported() {
        let changes = plan("a\n  b\n", IndentOperation::Double, None).unwrap();
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
        let halved = run(text, IndentOperation::Halve, None);
        assert_eq!(run(&halved, IndentOperation::Double, None), text);
    }
}
