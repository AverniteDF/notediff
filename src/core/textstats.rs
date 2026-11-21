//! Text statistics for the status bars, free of any toolkit types.
//!
//! Everything here is computed from plain `&str`, so it can be unit-tested
//! headlessly. Line breaks use the same definition as GtkTextBuffer: `\n`,
//! `\r\n`, a lone `\r`, and U+2029 PARAGRAPH SEPARATOR.

use super::{textlines, whitespace};

/// The line-ending style of a text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    /// Only `\n` breaks, or no breaks at all.
    #[default]
    Lf,
    /// Only `\r\n` breaks.
    CrLf,
    /// A mixture of styles, including lone `\r` or U+2029 breaks.
    Mixed,
}

impl LineEnding {
    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "Unix (LF)",
            LineEnding::CrLf => "Windows (CRLF)",
            LineEnding::Mixed => "Mixed",
        }
    }
}

/// Whole-text statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStats {
    /// Size of the text in UTF-8 bytes.
    pub bytes: usize,
    /// Number of characters (Unicode scalar values).
    pub chars: usize,
    /// Number of lines. An empty text has one (empty) line, and a trailing
    /// line break starts a new, empty last line.
    pub lines: usize,
    pub line_ending: LineEnding,
    /// True when any line ends in ASCII spaces or tabs.
    pub has_trailing_whitespace: bool,
    /// True when any line's content is non-empty and contains only ASCII
    /// spaces and tabs.
    pub has_non_empty_blank_line: bool,
}

impl TextStats {
    /// `bytes: <bytes>  chars: <chars>  lines: <lines>`, with thousands separators.
    pub fn summary(&self) -> String {
        format!(
            "bytes: {}  chars: {}  lines: {}",
            format_thousands(self.bytes),
            format_thousands(self.chars),
            format_thousands(self.lines)
        )
    }
}

/// A cursor location. All three fields are 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPosition {
    pub line: usize,
    /// Column in characters.
    pub col: usize,
    /// Position in bytes from the start of the text, plus one.
    pub pos: usize,
}

impl CursorPosition {
    /// `Ln: <line>  Col: <col>  Pos: <pos>`, or, when something is selected,
    /// `Ln: <line>  Col: <col>  Sel: <bytes> bytes, <lines> lines`, or
    /// `Sel: All` when the whole text is selected. Numbers use thousands
    /// separators.
    pub fn summary(&self, selection: Option<&SelectionStats>) -> String {
        match selection {
            Some(selection) => format!(
                "Ln: {}  Col: {}  {}",
                format_thousands(self.line),
                format_thousands(self.col),
                selection.summary()
            ),
            None => format!(
                "Ln: {}  Col: {}  Pos: {}",
                format_thousands(self.line),
                format_thousands(self.col),
                format_thousands(self.pos)
            ),
        }
    }
}

/// The size of a non-empty selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionStats {
    /// Selected text size in UTF-8 bytes.
    pub bytes: usize,
    /// Number of lines the selection touches. A selection that ends exactly
    /// at the start of a line does not count that last line, because none of
    /// its characters are selected.
    pub lines: usize,
    /// 0-based index of the first line the selection touches.
    pub first_line: usize,
    /// True when the selection covers the whole text.
    pub whole: bool,
}

impl SelectionStats {
    /// `Sel: All`, or `Sel: <bytes> bytes, <lines> lines`.
    pub fn summary(&self) -> String {
        if self.whole {
            return "Sel: All".to_string();
        }
        format!(
            "Sel: {} {}, {} {}",
            format_thousands(self.bytes),
            plural(self.bytes, "byte", "bytes"),
            format_thousands(self.lines),
            plural(self.lines, "line", "lines")
        )
    }

    /// 0-based, inclusive range of the lines the selection touches.
    pub fn line_range(&self) -> std::ops::RangeInclusive<usize> {
        self.first_line..=self.first_line + self.lines - 1
    }
}

/// Measure the selection between two character offsets (in either order, as
/// reported by `GtkTextBuffer`). Returns `None` for an empty selection.
pub fn selection_stats(text: &str, start_char: usize, end_char: usize) -> Option<SelectionStats> {
    let (start_char, end_char) = if start_char <= end_char {
        (start_char, end_char)
    } else {
        (end_char, start_char)
    };
    let start = byte_offset(text, start_char);
    let end = byte_offset(text, end_char);
    if start == end {
        return None;
    }

    let selected = &text[start..end];
    let breaks = textlines::break_count(selected);
    // A selection ending with a complete line break ends at the start of the
    // next line. A selection containing only the CR of a CRLF pair still
    // touches one line, so a nonempty selection never reports zero lines.
    let lines = if breaks == 0 {
        1
    } else {
        breaks + 1 - usize::from(textlines::ends_with_break(selected))
    };
    let first_line_prefix = if start > 0
        && text.as_bytes()[start - 1] == b'\r'
        && text.as_bytes().get(start) == Some(&b'\n')
    {
        &text[..start - 1]
    } else {
        &text[..start]
    };
    let first_line = textlines::break_count(first_line_prefix);
    Some(SelectionStats {
        bytes: selected.len(),
        lines,
        first_line,
        whole: start == 0 && end == text.len(),
    })
}

/// Describe a copied selection using Unicode character counts and the same
/// touched-line convention as selection_stats (a trailing break adds no line).
pub fn copied_text_summary(text: &str) -> String {
    let chars = text.chars().count();
    let breaks = textlines::break_count(text);
    let lines = if breaks == 0 {
        1
    } else {
        breaks + 1 - usize::from(textlines::ends_with_break(text))
    };
    let count = format_thousands(chars);
    let noun = plural(chars, "character", "characters");
    if lines > 1 {
        format!(
            "Copied {count} {noun} ({} lines) to clipboard",
            format_thousands(lines)
        )
    } else {
        format!("Copied {count} {noun} to clipboard")
    }
}

/// Byte index of the character at `char_offset`, clamped to the text length.
fn byte_offset(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map_or(text.len(), |(index, _)| index)
}

/// Format a number with comma thousands separators: `12507` -> `12,507`.
pub fn format_thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

pub fn detect_line_ending(text: &str) -> LineEnding {
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let mut other = 0usize;
    let mut index = 0;
    while index < text.len() {
        match textlines::break_len(text, index) {
            Some(2) => {
                crlf += 1;
                index += 2;
            }
            Some(_) if text[index..].starts_with('\n') => {
                lf += 1;
                index += 1;
            }
            Some(len) => {
                other += 1;
                index += len;
            }
            None => {
                index += text[index..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    match (lf, crlf, other) {
        (_, _, other) if other > 0 => LineEnding::Mixed,
        (lf, crlf, _) if lf > 0 && crlf > 0 => LineEnding::Mixed,
        (0, crlf, _) if crlf > 0 => LineEnding::CrLf,
        _ => LineEnding::Lf,
    }
}

pub fn text_stats(text: &str) -> TextStats {
    TextStats {
        bytes: text.len(),
        chars: text.chars().count(),
        lines: textlines::break_count(text) + 1,
        line_ending: detect_line_ending(text),
        has_trailing_whitespace: whitespace::has_trailing(text),
        has_non_empty_blank_line: whitespace::has_non_empty_blank_line(text),
    }
}

/// Locate the cursor at `char_offset` (a character count from the start, as
/// reported by `GtkTextBuffer`). Offsets past the end clamp to the end.
pub fn cursor_position(text: &str, char_offset: usize) -> CursorPosition {
    let mut line = 1;
    let mut col = 1;
    let mut byte = text.len();
    let mut chars = text.char_indices().peekable();
    let mut index = 0;
    while let Some((byte_index, ch)) = chars.next() {
        if index == char_offset {
            byte = byte_index;
            break;
        }
        match ch {
            // A lone CR is a line break. In CRLF, the LF that follows is the
            // one that advances the line, so the cursor before the LF still
            // belongs to the preceding line.
            '\r' if !chars.peek().is_some_and(|(_, next)| *next == '\n') => {
                line += 1;
                col = 1;
            }
            '\n' | '\u{2029}' => {
                line += 1;
                col = 1;
            }
            _ => col += 1,
        }
        index += 1;
    }
    CursorPosition {
        line,
        col,
        pos: byte + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_summaries_count_characters_and_touched_lines() {
        for (text, expected, lines) in [
            ("x", "Copied 1 character to clipboard", 1),
            ("é🙂x", "Copied 3 characters to clipboard", 1),
            ("\n", "Copied 1 character to clipboard", 1),
            ("abc\n", "Copied 4 characters to clipboard", 1),
            ("a\nb", "Copied 3 characters (2 lines) to clipboard", 2),
            ("a\nb\n", "Copied 4 characters (2 lines) to clipboard", 2),
            (
                "a\r\nb\r\n",
                "Copied 6 characters (2 lines) to clipboard",
                2,
            ),
            ("\n\n", "Copied 2 characters (2 lines) to clipboard", 2),
        ] {
            assert_eq!(copied_text_summary(text), expected);
            assert_eq!(
                selection_stats(text, 0, text.chars().count())
                    .unwrap()
                    .lines,
                lines
            );
        }
        let text = "1234567\n".repeat(5) + "12345";
        assert_eq!(
            copied_text_summary(&text),
            "Copied 45 characters (6 lines) to clipboard"
        );
        assert_eq!(
            copied_text_summary(&"x".repeat(1000)),
            "Copied 1,000 characters to clipboard"
        );
    }

    #[test]
    fn stats_for_basic_texts() {
        let empty = text_stats("");
        assert_eq!((empty.bytes, empty.lines), (0, 1));
        assert_eq!(empty.line_ending, LineEnding::Lf);

        let stats = text_stats("ab\ncd");
        assert_eq!((stats.bytes, stats.lines), (5, 2));

        let trailing = text_stats("ab\n");
        assert_eq!((trailing.bytes, trailing.lines), (3, 2));
    }

    #[test]
    fn stats_include_trailing_whitespace_state() {
        assert!(!text_stats("").has_trailing_whitespace);
        assert!(!text_stats("a\nb\n").has_trailing_whitespace);
        assert!(text_stats("a \n").has_trailing_whitespace);
        assert!(text_stats("a\t\r\n").has_trailing_whitespace);
        assert!(text_stats("   \n").has_trailing_whitespace);
        assert!(!text_stats("a\u{a0}\n").has_trailing_whitespace);
    }

    #[test]
    fn stats_include_non_empty_blank_line_state() {
        assert!(!text_stats("").has_non_empty_blank_line);
        assert!(!text_stats("a\nb\n").has_non_empty_blank_line);
        assert!(!text_stats("a\n\n").has_non_empty_blank_line);
        assert!(text_stats(" \n").has_non_empty_blank_line);
        assert!(text_stats("a\n\t\n").has_non_empty_blank_line);
        assert!(text_stats("a\n   ").has_non_empty_blank_line);
        assert!(!text_stats("a\u{a0}\n").has_non_empty_blank_line);
    }

    #[test]
    fn bytes_count_utf8_not_chars() {
        let stats = text_stats("é🦀");
        assert_eq!(stats.bytes, 6);
        assert_eq!(stats.chars, 2);
    }

    #[test]
    fn line_ending_detection() {
        assert_eq!(detect_line_ending(""), LineEnding::Lf);
        assert_eq!(detect_line_ending("no breaks"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a\nb\n"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(detect_line_ending("a\r\nb\nc"), LineEnding::Mixed);
        assert_eq!(detect_line_ending("a\rb"), LineEnding::Mixed);
        assert_eq!(detect_line_ending("a\u{2029}b"), LineEnding::Mixed);
        assert_eq!(LineEnding::Lf.label(), "Unix (LF)");
        assert_eq!(LineEnding::CrLf.label(), "Windows (CRLF)");
        assert_eq!(LineEnding::Mixed.label(), "Mixed");
    }

    #[test]
    fn stats_count_every_gtk_line_break() {
        for (text, lines) in [
            ("a\rb", 2),
            ("a\r\nb", 2),
            ("a\u{2029}b", 2),
            ("a\r", 2),
            ("a\u{2029}", 2),
        ] {
            assert_eq!(text_stats(text).lines, lines, "{text:?}");
        }
        assert_eq!(text_stats("\r\n").lines, 2);
    }

    #[test]
    fn cursor_positions() {
        let start = cursor_position("", 0);
        assert_eq!((start.line, start.col, start.pos), (1, 1, 1));

        let p = cursor_position("ab\ncd", 4);
        assert_eq!((p.line, p.col, p.pos), (2, 2, 5));

        // 'é' is two bytes: after it the byte position advances by two.
        let p = cursor_position("éa", 1);
        assert_eq!((p.line, p.col, p.pos), (1, 2, 3));

        // CRLF: the cursor after the pair is at column 1 of line 2.
        let p = cursor_position("a\r\nb", 3);
        assert_eq!((p.line, p.col, p.pos), (2, 1, 4));

        // Past the end clamps to the end.
        let p = cursor_position("abc", 99);
        assert_eq!((p.line, p.col, p.pos), (1, 4, 4));

        // Lone CR and U+2029 advance to the next line.
        let p = cursor_position("a\rb", 2);
        assert_eq!((p.line, p.col, p.pos), (2, 1, 3));
        let p = cursor_position("a\u{2029}b", 2);
        assert_eq!((p.line, p.col, p.pos), (2, 1, 5));
    }

    #[test]
    fn selections_count_every_gtk_line_break() {
        let sel = selection_stats("a\rb", 0, 2).unwrap();
        assert_eq!((sel.lines, sel.first_line), (1, 0));
        let sel = selection_stats("a\rb", 0, 3).unwrap();
        assert_eq!((sel.lines, sel.first_line), (2, 0));
        let sel = selection_stats("a\u{2029}b", 0, 3).unwrap();
        assert_eq!((sel.lines, sel.first_line), (2, 0));
        // Half of a CRLF pair still belongs to one line.
        let sel = selection_stats("a\r\nb", 1, 2).unwrap();
        assert_eq!((sel.lines, sel.first_line), (1, 0));
        assert_eq!(
            copied_text_summary("a\rb"),
            "Copied 3 characters (2 lines) to clipboard"
        );
    }

    #[test]
    fn summaries() {
        assert_eq!(text_stats("a\nb").summary(), "bytes: 3  chars: 3  lines: 2");
        assert_eq!(
            text_stats("é🦀\n").summary(),
            "bytes: 7  chars: 3  lines: 2"
        );
        assert_eq!(
            cursor_position("a\nb", 2).summary(None),
            "Ln: 2  Col: 1  Pos: 3"
        );
        assert_eq!(
            text_stats(&"x".repeat(12507)).summary(),
            "bytes: 12,507  chars: 12,507  lines: 1"
        );
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(12507), "12,507");
        assert_eq!(format_thousands(100000), "100,000");
        assert_eq!(format_thousands(1234567), "1,234,567");
    }

    #[test]
    fn selection_measures_bytes_and_lines() {
        // "b\nc" spans two lines.
        let sel = selection_stats("ab\ncd", 1, 4).unwrap();
        assert_eq!((sel.bytes, sel.lines, sel.first_line), (3, 2, 0));

        // Either order gives the same answer.
        assert_eq!(selection_stats("ab\ncd", 4, 1), Some(sel));

        // Empty selection.
        assert_eq!(selection_stats("ab\ncd", 3, 3), None);

        // Ending exactly at the start of a line does not count that line.
        let sel = selection_stats("ab\ncd", 0, 3).unwrap();
        assert_eq!((sel.bytes, sel.lines), (3, 1));

        // Selection on a later line.
        let sel = selection_stats("a\nb\nc", 2, 5).unwrap();
        assert_eq!((sel.bytes, sel.lines, sel.first_line), (3, 2, 1));
        assert_eq!(sel.line_range(), 1..=2);
    }

    #[test]
    fn selection_counts_utf8_bytes() {
        let sel = selection_stats("é🦀x", 0, 2).unwrap();
        assert_eq!(sel.bytes, 6);
    }

    #[test]
    fn selection_summaries() {
        let partial = selection_stats("a\nb\nc", 0, 4).unwrap();
        assert_eq!(partial.summary(), "Sel: 4 bytes, 2 lines");
        assert_eq!(
            cursor_position("a\nb\nc", 5).summary(Some(&partial)),
            "Ln: 3  Col: 2  Sel: 4 bytes, 2 lines"
        );

        let whole = selection_stats("a\nb\nc", 0, 5).unwrap();
        assert_eq!(whole.summary(), "Sel: All");
        assert_eq!(
            cursor_position("a\nb\nc", 5).summary(Some(&whole)),
            "Ln: 3  Col: 2  Sel: All"
        );
        // Covering everything except the final line break is not whole.
        let without_final_break = selection_stats("abc\n", 0, 3).unwrap();
        assert_eq!(without_final_break.summary(), "Sel: 3 bytes, 1 line");

        let one = selection_stats("abc", 1, 2).unwrap();
        assert_eq!(one.summary(), "Sel: 1 byte, 1 line");
    }

    #[test]
    fn selection_starting_mid_crlf_agrees_with_cursor_line() {
        let text = "a\r\nb";
        assert_eq!(cursor_position(text, 2).line, 1);
        let sel = selection_stats(text, 2, 4).unwrap();
        assert_eq!(sel.first_line, 0);
        assert_eq!(sel.line_range(), 0..=1);
    }
}
