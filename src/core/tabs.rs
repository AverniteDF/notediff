//! The "no tab characters" policy, free of any toolkit types.

/// True when the text contains at least one tab character.
pub fn contains_tab(text: &str) -> bool {
    text.contains('\t')
}

/// Replace every tab character with exactly one space.
pub fn convert_tabs(text: &str) -> String {
    text.replace('\t', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tabs() {
        assert!(!contains_tab(""));
        assert!(!contains_tab("no tabs here\n"));
        assert!(contains_tab("a\tb"));
        assert!(contains_tab("\t"));
    }

    #[test]
    fn each_tab_becomes_one_space() {
        assert_eq!(convert_tabs("a\tb"), "a b");
        assert_eq!(convert_tabs("\t\tx\t"), "  x ");
        assert_eq!(convert_tabs("plain"), "plain");
    }

    #[test]
    fn line_endings_are_untouched() {
        assert_eq!(convert_tabs("a\t\r\nb\t\n"), "a \r\nb \n");
    }
}
