//! Line-ending conversion, free of any toolkit types.

use super::textlines;
use super::textstats::LineEnding;

/// Rewrite every line break in `text` as `target`. GTK's line breaks (`\n`,
/// `\r\n`, a lone `\r`, and U+2029) are all normalised, so a mixed text
/// becomes uniform.
///
/// `LineEnding::Mixed` is not a valid target; the text is returned unchanged.
pub fn convert_line_endings(text: &str, target: LineEnding) -> String {
    let newline = match target {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
        LineEnding::Mixed => return text.to_string(),
    };

    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut index = 0;
    while index < text.len() {
        if let Some(len) = textlines::break_len(text, index) {
            out.push_str(newline);
            index += len;
        } else {
            let ch = text[index..].chars().next().unwrap();
            out.push(ch);
            index += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::textstats::detect_line_ending;

    #[test]
    fn lf_to_crlf() {
        assert_eq!(
            convert_line_endings("a\nb\n", LineEnding::CrLf),
            "a\r\nb\r\n"
        );
    }

    #[test]
    fn crlf_to_lf() {
        assert_eq!(convert_line_endings("a\r\nb\r\n", LineEnding::Lf), "a\nb\n");
    }

    #[test]
    fn mixed_becomes_uniform() {
        let mixed = "a\r\nb\nc\r\nd";
        let crlf = convert_line_endings(mixed, LineEnding::CrLf);
        assert_eq!(crlf, "a\r\nb\r\nc\r\nd");
        assert_eq!(detect_line_ending(&crlf), LineEnding::CrLf);
        let lf = convert_line_endings(mixed, LineEnding::Lf);
        assert_eq!(lf, "a\nb\nc\nd");
        assert_eq!(detect_line_ending(&lf), LineEnding::Lf);
    }

    #[test]
    fn conversion_is_idempotent() {
        let once = convert_line_endings("a\nb\r\nc\n", LineEnding::CrLf);
        assert_eq!(convert_line_endings(&once, LineEnding::CrLf), once);
    }

    #[test]
    fn no_breaks_and_empty_text_are_unchanged() {
        assert_eq!(convert_line_endings("", LineEnding::CrLf), "");
        assert_eq!(convert_line_endings("abc", LineEnding::CrLf), "abc");
    }

    #[test]
    fn trailing_newline_state_is_preserved() {
        assert_eq!(convert_line_endings("a\nb", LineEnding::CrLf), "a\r\nb");
        assert_eq!(convert_line_endings("a\r\nb", LineEnding::Lf), "a\nb");
    }

    #[test]
    fn lone_carriage_returns_and_paragraph_separators_are_breaks() {
        assert_eq!(
            convert_line_endings("a\rb\n", LineEnding::CrLf),
            "a\r\nb\r\n"
        );
        assert_eq!(convert_line_endings("a\r", LineEnding::Lf), "a\n");
        assert_eq!(convert_line_endings("a\u{2029}b", LineEnding::Lf), "a\nb");
        assert_eq!(
            convert_line_endings("a\u{2029}b", LineEnding::CrLf),
            "a\r\nb"
        );
    }

    #[test]
    fn mixed_target_is_a_no_op() {
        assert_eq!(
            convert_line_endings("a\r\nb\n", LineEnding::Mixed),
            "a\r\nb\n"
        );
    }

    #[test]
    fn unicode_is_untouched() {
        assert_eq!(
            convert_line_endings("é🦀\nx\n", LineEnding::CrLf),
            "é🦀\r\nx\r\n"
        );
    }
}
