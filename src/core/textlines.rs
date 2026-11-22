//! Logical lines as GtkTextBuffer sees them.
//!
//! GTK treats `\n`, `\r\n`, a lone `\r`, and U+2029 PARAGRAPH SEPARATOR as
//! paragraph breaks. The rest of the app must use the same definition or line
//! indexes computed in `core` will not match the editor buffer.

/// Byte length of the line break starting at `index`, if there is one.
///
/// `\r\n` is one two-byte break; a lone `\r` and U+2029 are one break each.
#[cfg(test)]
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

/// Find the next GTK line break (`\r\n`, `\n`, lone `\r`, or U+2029) at or
/// after `start`, returning `(break_start, break_len)`.
///
/// All break markers are self-synchronizing in UTF-8 (`0x0A`, `0x0D`, and
/// `0xE2 0x80 0xA9`), so scanning bytes directly is safe and avoids decoding
/// every character.
pub fn next_break(text: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => return Some((index, 1)),
            b'\r' => {
                let len = if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                return Some((index, len));
            }
            0xE2 if bytes.get(index + 1) == Some(&0x80) && bytes.get(index + 2) == Some(&0xA9) => {
                return Some((index, 3));
            }
            _ => index += 1,
        }
    }
    None
}

/// True when the text contains at least one line break.
pub fn has_break(text: &str) -> bool {
    next_break(text, 0).is_some()
}

/// True when the text ends with a line break.
pub fn ends_with_break(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r') || text.ends_with('\u{2029}')
}

/// Number of line breaks in `text`. `\r\n` counts once.
pub fn break_count(text: &str) -> usize {
    let mut count = 0;
    let mut index = 0;
    while let Some((pos, len)) = next_break(text, index) {
        count += 1;
        index = pos + len;
    }
    count
}

/// Zero-allocation iterator matching [`split_inclusive`].
pub struct LinesInclusive<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Iterator for LinesInclusive<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.text.len() {
            return None;
        }
        let start = self.pos;
        let end = match next_break(self.text, start) {
            Some((pos, len)) => pos + len,
            None => self.text.len(),
        };
        self.pos = end;
        Some(&self.text[start..end])
    }
}

/// Iterate over lines retaining their terminators without allocating a `Vec`.
pub fn iter_inclusive(text: &str) -> LinesInclusive<'_> {
    LinesInclusive { text, pos: 0 }
}

/// Zero-allocation iterator matching [`split_all`].
pub struct LinesAll<'a> {
    inner: LinesInclusive<'a>,
    trailing_empty: bool,
}

impl<'a> Iterator for LinesAll<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(line) = self.inner.next() {
            return Some(line);
        }
        if self.trailing_empty {
            self.trailing_empty = false;
            return Some("");
        }
        None
    }
}

/// Iterate over every GTK line, including the empty line after a trailing
/// terminator, without allocating a `Vec`.
pub fn iter_all(text: &str) -> LinesAll<'_> {
    LinesAll {
        inner: iter_inclusive(text),
        trailing_empty: text.is_empty() || ends_with_break(text),
    }
}

/// Split into lines that retain their terminators. A trailing terminator does
/// not create an extra empty item, so `""` produces no lines and `"a\n"`
/// produces one line. Used by the diff engine, where a final newline is part
/// of the preceding line rather than a separate empty line.
pub fn split_inclusive(text: &str) -> Vec<&str> {
    iter_inclusive(text).collect()
}

/// Split into every GTK line, including the empty line after a trailing
/// terminator. Used where line numbers must match `GtkTextIter::line`, such as
/// indentation editing and selection line ranges.
pub fn split_all(text: &str) -> Vec<&str> {
    iter_all(text).collect()
}

/// Byte offset of the start of `line` using `split_inclusive` numbering.
/// A line past the end maps to the text length.
pub fn line_start_byte(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut offset = 0;
    for (index, item) in iter_inclusive(text).enumerate() {
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
    let stop = start.saturating_add(len);
    let mut begin = None;
    let mut end = text.len();
    let mut offset = 0;
    for (index, line) in iter_inclusive(text).enumerate() {
        if index == start {
            begin = Some(offset);
        }
        if index == stop {
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
