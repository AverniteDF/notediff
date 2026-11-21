//! Logical lines as GtkTextBuffer sees them.
//!
//! GTK treats `\n`, `\r\n`, a lone `\r`, and U+2029 PARAGRAPH SEPARATOR as
//! paragraph breaks. The rest of the app must use the same definition or line
//! indexes computed in `core` will not match the editor buffer.

/// Byte length of the line break starting at `index`, if there is one.
///
/// `\r\n` is one two-byte break; a lone `\r` and U+2029 are one break each.
pub fn break_len(text: &str, index: usize) -> Option<usize> {
    let rest = text.get(index..)?;
    if rest.starts_with("\r\n") {
        Some(2)
    } else if rest.starts_with('\n') || rest.starts_with('\r') {
        Some(1)
    } else if rest.starts_with('\u{2029}') {
        Some('\u{2029}'.len_utf8())
    } else {
        None
    }
}

/// True when the text contains at least one line break.
pub fn has_break(text: &str) -> bool {
    let mut index = 0;
    while index < text.len() {
        if break_len(text, index).is_some() {
            return true;
        }
        index += next_char_len(text, index);
    }
    false
}

/// True when the text ends with a line break.
pub fn ends_with_break(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r') || text.ends_with('\u{2029}')
}

/// Number of line breaks in `text`. `\r\n` counts once.
pub fn break_count(text: &str) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < text.len() {
        if let Some(len) = break_len(text, index) {
            count += 1;
            index += len;
        } else {
            index += next_char_len(text, index);
        }
    }
    count
}

/// Split into lines that retain their terminators. A trailing terminator does
/// not create an extra empty item, so `""` produces no lines and `"a\n"`
/// produces one line. Used by the diff engine, where a final newline is part
/// of the preceding line rather than a separate empty line.
pub fn split_inclusive(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < text.len() {
        if let Some(len) = break_len(text, index) {
            index += len;
            lines.push(&text[start..index]);
            start = index;
        } else {
            index += next_char_len(text, index);
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Split into every GTK line, including the empty line after a trailing
/// terminator. Used where line numbers must match `GtkTextIter::line`, such as
/// indentation editing and selection line ranges.
pub fn split_all(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < text.len() {
        if let Some(len) = break_len(text, index) {
            index += len;
            lines.push(&text[start..index]);
            start = index;
        } else {
            index += next_char_len(text, index);
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    } else {
        lines.push("");
    }
    lines
}

/// Byte offset of the start of `line` using `split_inclusive` numbering.
/// A line past the end maps to the text length.
pub fn line_start_byte(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut offset = 0;
    for (index, item) in split_inclusive(text).into_iter().enumerate() {
        if index == line {
            return offset;
        }
        offset += item.len();
    }
    text.len()
}

/// Slice `len` lines starting at `start`, using `split_inclusive` numbering.
/// Terminators are retained. An out-of-range start returns an empty slice.
pub fn slice_lines(text: &str, start: usize, len: usize) -> &str {
    if len == 0 {
        return "";
    }
    let lines = split_inclusive(text);
    let mut begin = None;
    let mut end = text.len();
    let mut offset = 0;
    for (index, line) in lines.iter().enumerate() {
        if index == start {
            begin = Some(offset);
        }
        if index == start.saturating_add(len) {
            end = offset;
            break;
        }
        offset += line.len();
    }
    match begin {
        Some(begin) => &text[begin..end],
        None => "",
    }
}

/// The visible part of a line, without any line terminator.
pub fn content(line: &str) -> &str {
    for suffix in ["\r\n", "\n", "\r", "\u{2029}"] {
        if let Some(content) = line.strip_suffix(suffix) {
            return content;
        }
    }
    line
}

fn next_char_len(text: &str, index: usize) -> usize {
    text[index..].chars().next().map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_inclusive_retains_gtk_terminators() {
        assert_eq!(split_inclusive(""), Vec::<&str>::new());
        assert_eq!(split_inclusive("a"), vec!["a"]);
        assert_eq!(split_inclusive("a\n"), vec!["a\n"]);
        assert_eq!(split_inclusive("a\nb"), vec!["a\n", "b"]);
        assert_eq!(split_inclusive("a\r\nb\r"), vec!["a\r\n", "b\r"]);
        assert_eq!(
            split_inclusive("a\rb\u{2029}c"),
            vec!["a\r", "b\u{2029}", "c"]
        );
    }

    #[test]
    fn split_all_includes_the_trailing_empty_line() {
        assert_eq!(split_all(""), vec![""]);
        assert_eq!(split_all("a"), vec!["a"]);
        assert_eq!(split_all("a\n"), vec!["a\n", ""]);
        assert_eq!(split_all("a\r\n"), vec!["a\r\n", ""]);
        assert_eq!(split_all("a\r"), vec!["a\r", ""]);
        assert_eq!(split_all("a\u{2029}"), vec!["a\u{2029}", ""]);
    }

    #[test]
    fn break_counting_treats_crlf_as_one() {
        assert_eq!(break_count(""), 0);
        assert_eq!(break_count("a\nb\r\nc\rd\u{2029}e"), 4);
        assert!(has_break("a\r"));
        assert!(!has_break("abc"));
        assert!(ends_with_break("a\r\n"));
        assert!(ends_with_break("a\u{2029}"));
        assert!(!ends_with_break("abc"));
    }

    #[test]
    fn byte_offsets_and_slices_follow_every_break() {
        let text = "one\rtwo\r\nthree\u{2029}four";
        assert_eq!(line_start_byte(text, 0), 0);
        assert_eq!(line_start_byte(text, 1), 4);
        assert_eq!(line_start_byte(text, 2), 9);
        assert_eq!(line_start_byte(text, 3), 17);
        assert_eq!(slice_lines(text, 1, 2), "two\r\nthree\u{2029}");
        assert_eq!(slice_lines(text, 2, 1), "three\u{2029}");
        assert_eq!(slice_lines(text, 1, 0), "");
        assert_eq!(slice_lines(text, 9, 1), "");
    }

    #[test]
    fn content_strips_every_terminator() {
        assert_eq!(content("a\n"), "a");
        assert_eq!(content("a\r\n"), "a");
        assert_eq!(content("a\r"), "a");
        assert_eq!(content("a\u{2029}"), "a");
        assert_eq!(content("a"), "a");
    }
}
