//! Indentation editing, free of any toolkit types.
//!
//! Indentation means the run of ASCII space characters at the start of a line.
//! Tabs and other whitespace are not indentation. Blank and whitespace-only
//! lines are treated literally: their leading spaces count. Lines are numbered
//! exactly as GtkTextBuffer numbers them, including the empty line after a
//! trailing break.
//!
//! `Skip empty lines` excludes only truly empty lines; whitespace-only lines
//! still count as targets.

use std::ops::RangeInclusive;

use super::textlines;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentOperation {
    Increase,
    Decrease,
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
            IndentOperation::Increase | IndentOperation::Decrease => None,
        }
    }

    /// The new leading-space count for a line with `old` spaces, or `None` when
    /// the line does not change.
    fn changed(self, old: usize) -> Option<usize> {
        match self {
            IndentOperation::Increase => Some(old + 1),
            IndentOperation::Decrease => (old > 0).then(|| old - 1),
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
/// `skip_empty` excludes lines whose content is truly empty before the target
/// list is checked. Whitespace-only lines are not empty and stay in.
///
/// Increase and Decrease never fail. Double and Halve are all-or-nothing and
/// return `NothingToScale` or `OddIndent` without a change list.
pub fn plan(
    text: &str,
    operation: IndentOperation,
    lines: Option<RangeInclusive<usize>>,
    skip_empty: bool,
) -> Result<Vec<IndentChange>, IndentError> {
    let targets: Vec<(usize, usize)> = textlines::split_all(text)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| lines.as_ref().is_none_or(|range| range.contains(index)))
        .filter(|(_, line)| !skip_empty || !textlines::content(line).is_empty())
        .map(|(index, line)| (index, leading_spaces(line)))
        .collect();

    match operation {
        IndentOperation::Increase | IndentOperation::Decrease => Ok(changes(targets, operation)),
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

    fn run(
        text: &str,
        operation: IndentOperation,
        lines: Option<RangeInclusive<usize>>,
        skip_empty: bool,
    ) -> String {
        apply(text, &plan(text, operation, lines, skip_empty).unwrap())
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
    fn increases_and_decreases_by_one_space() {
        assert_eq!(
            run("a\n  b\n", IndentOperation::Increase, None, true),
            " a\n   b\n"
        );
        assert_eq!(
            run(" a\n   b\n", IndentOperation::Decrease, None, true),
            "a\n  b\n"
        );
    }

    #[test]
    fn increase_includes_empty_lines_unless_skipped() {
        assert_eq!(
            run("  a\n\n  b\n", IndentOperation::Increase, None, false),
            "   a\n \n   b\n "
        );
        assert_eq!(
            run("  a\n\n  b\n", IndentOperation::Increase, None, true),
            "   a\n\n   b\n"
        );
    }

    #[test]
    fn skip_empty_keeps_whitespace_only_lines() {
        assert_eq!(
            run("a\n   \n", IndentOperation::Increase, None, true),
            " a\n    \n"
        );
        assert_eq!(
            run(" a\n    \n", IndentOperation::Decrease, None, true),
            "a\n   \n"
        );
    }

    #[test]
    fn decrease_is_a_silent_no_op_when_nothing_can_be_removed() {
        assert_eq!(
            plan("a\nb\n", IndentOperation::Decrease, None, true),
            Ok(Vec::new())
        );
        assert_eq!(
            plan("a\n  b\n", IndentOperation::Decrease, Some(0..=0), true),
            Ok(Vec::new())
        );
    }

    #[test]
    fn increase_never_fails() {
        assert_eq!(
            plan("", IndentOperation::Increase, None, true),
            Ok(Vec::new())
        );
        assert_eq!(
            plan("a\n", IndentOperation::Increase, None, true),
            Ok(vec![IndentChange {
                line: 0,
                old: 0,
                new: 1
            }])
        );
    }

    #[test]
    fn doubles_the_whole_text() {
        assert_eq!(
            run("a\n  b\n    c\n", IndentOperation::Double, None, true),
            "a\n    b\n        c\n"
        );
    }

    #[test]
    fn halves_the_whole_text() {
        assert_eq!(
            run("a\n    b\n  c\n", IndentOperation::Halve, None, true),
            "a\n  b\n c\n"
        );
    }

    #[test]
    fn selection_limits_the_target_lines() {
        // Odd indentation outside the range does not block halving.
        let text = " x\n  y\n    z\n   w\n";
        assert_eq!(
            run(text, IndentOperation::Halve, Some(1..=2), true),
            " x\n y\n  z\n   w\n"
        );
        assert_eq!(
            run(text, IndentOperation::Double, Some(2..=2), true),
            " x\n  y\n        z\n   w\n"
        );
        assert_eq!(
            run(text, IndentOperation::Increase, Some(1..=1), true),
            " x\n   y\n    z\n   w\n"
        );
    }

    #[test]
    fn skip_empty_is_applied_before_the_scaling_checks() {
        // The selected range contains only a truly empty line.
        assert_eq!(
            plan("  a\n\n", IndentOperation::Double, Some(1..=1), true),
            Err(IndentError::NothingToScale)
        );
        // A non-empty target still scales while empty lines stay filtered out.
        assert_eq!(
            run("  a\n\n", IndentOperation::Double, Some(0..=1), true),
            "    a\n\n"
        );
    }

    #[test]
    fn all_zero_indentation_aborts() {
        for operation in [IndentOperation::Double, IndentOperation::Halve] {
            assert_eq!(
                plan("a\nb\n", operation, None, true),
                Err(IndentError::NothingToScale)
            );
            assert_eq!(
                plan("", operation, None, true),
                Err(IndentError::NothingToScale)
            );
        }
        assert_eq!(
            plan("a\n  b\n", IndentOperation::Double, Some(0..=0), true),
            Err(IndentError::NothingToScale)
        );
        assert_eq!(
            plan("a\n  b\n", IndentOperation::Halve, Some(0..=0), true),
            Err(IndentError::NothingToScale)
        );
    }

    #[test]
    fn halve_reports_first_odd_line_one_based() {
        assert_eq!(
            plan("  a\n   b\n c\n", IndentOperation::Halve, None, true),
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
            plan("    a\n   \n", IndentOperation::Halve, None, true),
            Err(IndentError::OddIndent { line: 2 })
        );
        // An empty line is zero indentation and is left alone.
        assert_eq!(
            run("  a\n\n  b\n", IndentOperation::Double, None, true),
            "    a\n\n    b\n"
        );
        // Whitespace-only lines with an even count are scaled.
        assert_eq!(run("  \n", IndentOperation::Double, None, true), "    \n");
        assert_eq!(run("    \n", IndentOperation::Halve, None, true), "  \n");
    }

    #[test]
    fn crlf_and_trailing_line_are_handled() {
        assert_eq!(
            run("  a\r\n  b\r\n", IndentOperation::Double, None, true),
            "    a\r\n    b\r\n"
        );
        // The empty last line (after a trailing break) is a zero-indent line.
        let changes = plan("  a\n", IndentOperation::Double, Some(0..=1), true).unwrap();
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn every_gtk_line_break_splits_target_lines() {
        for (text, expected) in [
            ("  a\r  b\r", "    a\r    b\r"),
            ("  a\u{2029}  b\u{2029}", "    a\u{2029}    b\u{2029}"),
        ] {
            assert_eq!(run(text, IndentOperation::Double, None, true), expected);
        }
        // The line after a trailing break is empty and has no indentation.
        assert_eq!(run("  a\r", IndentOperation::Double, None, true), "    a\r");
        assert_eq!(
            plan("  a\r", IndentOperation::Double, Some(1..=1), true),
            Err(IndentError::NothingToScale)
        );
    }

    #[test]
    fn only_changing_lines_are_reported() {
        let changes = plan("a\n  b\n", IndentOperation::Double, None, true).unwrap();
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
        let halved = run(text, IndentOperation::Halve, None, true);
        assert_eq!(run(&halved, IndentOperation::Double, None, true), text);
    }
}
