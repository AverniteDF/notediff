//! Preparing the text of a file for insertion into the editor, free of any
//! toolkit types.
//!
//! "Add Text from File" puts the whole contents of a chosen file at the
//! cursor. This module decides whether a file may be inserted (a regular file
//! below a hard size limit), reads it as UTF-8, drops a leading byte-order
//! mark and adapts its line endings to the document it goes into. Tabs are
//! not handled here: the editor's insert filter converts them like any other
//! inserted text.

use std::fmt;
use std::fs;
use std::path::Path;

use super::LARGE_FILE_WARNING_BYTES;
use super::MAX_OPEN_BYTES;
use super::eol::convert_line_endings;
use super::fsutil::{ReadError, read_utf8_limited};
use super::textlines;
use super::textstats::{LineEnding, detect_line_ending, format_thousands};

#[derive(Debug)]
pub enum InsertError {
    /// The path is not a regular file (for example a folder).
    NotAFile,
    /// The file is above `MAX_OPEN_BYTES`.
    TooLarge { bytes: u64 },
    /// The file could not be read or is not valid UTF-8.
    Read(ReadError),
}

impl fmt::Display for InsertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InsertError::NotAFile => write!(f, "That is not a regular file."),
            InsertError::TooLarge { bytes } => write!(
                f,
                "The file is {} bytes, which is more than the {} byte limit for inserting.",
                format_thousands(*bytes as usize),
                format_thousands(MAX_OPEN_BYTES as usize)
            ),
            InsertError::Read(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for InsertError {}

/// Check that `path` is a regular file within the hard size limit and return
/// its size in bytes.
pub fn inspect(path: &Path) -> Result<u64, InsertError> {
    let metadata = fs::metadata(path).map_err(|err| InsertError::Read(ReadError::Io(err)))?;
    if !metadata.is_file() {
        return Err(InsertError::NotAFile);
    }
    if metadata.len() > MAX_OPEN_BYTES {
        return Err(InsertError::TooLarge {
            bytes: metadata.len(),
        });
    }
    Ok(metadata.len())
}

/// True when a file of `bytes` bytes should be confirmed before inserting.
pub fn needs_confirmation(bytes: u64) -> bool {
    bytes > LARGE_FILE_WARNING_BYTES
}

/// Read the file for insertion: the same checks as [`inspect`], then UTF-8
/// text without a leading byte-order mark.
pub fn load(path: &Path) -> Result<String, InsertError> {
    let text = read_utf8_limited(path, MAX_OPEN_BYTES).map_err(|err| match err {
        ReadError::NotRegularFile => InsertError::NotAFile,
        ReadError::TooLarge { bytes, .. } => InsertError::TooLarge { bytes },
        other => InsertError::Read(other),
    })?;
    Ok(strip_bom(text))
}

/// Remove one leading U+FEFF (the UTF-8 byte-order mark), if present.
pub fn strip_bom(mut text: String) -> String {
    if text.starts_with('\u{feff}') {
        text.drain(..'\u{feff}'.len_utf8());
    }
    text
}

/// Convert the line endings of `inserted` to the style of `document`, so the
/// insertion does not make the document mixed. Nothing changes when the
/// document has no line breaks yet (the file's own style is as good as any) or
/// is already mixed.
pub fn match_line_endings(inserted: &str, document: &str) -> String {
    if !textlines::has_break(document) {
        return inserted.to_string();
    }
    match detect_line_ending(document) {
        LineEnding::Mixed => inserted.to_string(),
        target => convert_line_endings(inserted, target),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-insert-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inspect_accepts_regular_files() {
        let dir = test_dir();
        let path = dir.join("a.txt");
        fs::write(&path, "hello").unwrap();
        assert_eq!(inspect(&path).unwrap(), 5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn inspect_refuses_folders_and_missing_files() {
        let dir = test_dir();
        assert!(matches!(inspect(&dir), Err(InsertError::NotAFile)));
        assert!(matches!(
            inspect(&dir.join("missing.txt")),
            Err(InsertError::Read(ReadError::Io(_)))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn inspect_refuses_files_over_the_hard_limit() {
        let dir = test_dir();
        let path = dir.join("huge.bin");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_OPEN_BYTES + 1).unwrap();
        assert!(matches!(
            inspect(&path),
            Err(InsertError::TooLarge { bytes }) if bytes == MAX_OPEN_BYTES + 1
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn confirmation_starts_above_the_warning_size() {
        assert!(!needs_confirmation(0));
        assert!(!needs_confirmation(LARGE_FILE_WARNING_BYTES));
        assert!(needs_confirmation(LARGE_FILE_WARNING_BYTES + 1));
    }

    #[test]
    fn load_reads_text_and_strips_a_bom() {
        let dir = test_dir();
        let path = dir.join("bom.txt");
        fs::write(&path, "\u{feff}héllo\r\n").unwrap();
        assert_eq!(load(&path).unwrap(), "héllo\r\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_rejects_invalid_utf8() {
        let dir = test_dir();
        let path = dir.join("bad.bin");
        fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(
            load(&path),
            Err(InsertError::Read(ReadError::NotUtf8))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn strip_bom_only_removes_one_leading_mark() {
        assert_eq!(strip_bom("\u{feff}a".to_string()), "a");
        assert_eq!(strip_bom("a\u{feff}".to_string()), "a\u{feff}");
        assert_eq!(strip_bom(String::new()), "");
        assert_eq!(strip_bom("\u{feff}\u{feff}a".to_string()), "\u{feff}a");
    }

    #[test]
    fn line_endings_follow_a_uniform_document() {
        assert_eq!(match_line_endings("a\r\nb\r\n", "x\ny\n"), "a\nb\n");
        assert_eq!(match_line_endings("a\nb\n", "x\r\ny\r\n"), "a\r\nb\r\n");
        assert_eq!(match_line_endings("a\nb\r\n", "x\ny\n"), "a\nb\n");
    }

    #[test]
    fn line_endings_are_kept_when_the_document_has_no_style() {
        // No line breaks yet: the file's own endings are kept.
        assert_eq!(match_line_endings("a\r\nb\r\n", ""), "a\r\nb\r\n");
        assert_eq!(match_line_endings("a\r\nb\r\n", "abc"), "a\r\nb\r\n");
        // Already mixed: nothing to match.
        assert_eq!(match_line_endings("a\r\nb\n", "x\r\ny\nz"), "a\r\nb\n");
    }
}
