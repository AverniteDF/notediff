//! Live document state: path, text and last-known modification time.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::MAX_OPEN_BYTES;
use super::fsutil::{
    ReadError, atomic_write, canonical_target, file_mtime, move_file, read_utf8_limited,
    read_utf8_limited_with_mtime,
};
use super::textstats::{LineEnding, detect_line_ending};

#[derive(Debug, Default)]
pub struct Document {
    path: Option<PathBuf>,
    text: String,
    saved_chars: usize,
    last_mtime: Option<SystemTime>,
    line_ending: LineEnding,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(path: &Path) -> Result<Self, ReadError> {
        let canonical = fs::canonicalize(path).map_err(ReadError::Io)?;
        let (text, last_mtime) = read_utf8_limited_with_mtime(&canonical, MAX_OPEN_BYTES)?;
        Ok(Self {
            path: Some(canonical),
            line_ending: detect_line_ending(&text),
            saved_chars: text.chars().count(),
            text,
            last_mtime: Some(last_mtime),
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of Unicode scalar values in the last read or written text.
    pub fn saved_chars(&self) -> usize {
        self.saved_chars
    }

    /// The document's dominant line ending, used when inserting new breaks.
    /// It is derived from the last open, save or reload and updated by
    /// explicit line-ending conversion.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.line_ending = line_ending;
    }

    /// True when `text` is not the text last read from or written to disk.
    /// This is the "is dirty" predicate of a file-backed document. String
    /// comparison checks the length first, so differing sizes are cheap.
    pub fn differs_from(&self, text: &str) -> bool {
        self.text != text
    }

    /// Force a remembered timestamp for deterministic external-change scenarios.
    #[cfg(test)]
    pub fn set_last_mtime(&mut self, mtime: Option<SystemTime>) {
        self.last_mtime = mtime;
    }

    /// Save the draft to the current path.
    pub fn save_live(&mut self, text: String) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "document has no path"))?;
        ensure_save_size(&text)?;
        atomic_write(&path, &text, false)?;
        self.line_ending = detect_line_ending(&text);
        self.saved_chars = text.chars().count();
        self.text = text;
        self.last_mtime = file_mtime(&path).ok();
        Ok(())
    }

    /// Save the draft to a new path, then canonicalize it. Existing symlinks
    /// are resolved so future writes replace the target rather than the link.
    pub fn save_as(&mut self, path: &Path, text: String) -> io::Result<()> {
        ensure_save_size(&text)?;
        let canonical = canonical_target(path)?;
        atomic_write(&canonical, &text, false)?;
        self.path = Some(canonical.clone());
        self.line_ending = detect_line_ending(&text);
        self.saved_chars = text.chars().count();
        self.text = text;
        self.last_mtime = file_mtime(&canonical).ok();
        Ok(())
    }

    /// Rename the file on disk to `new_path` and point the document at it.
    /// The text and the last-known modification time are left alone: a rename
    /// changes neither the content nor (on one filesystem) the mtime, and an
    /// external change that has not been noticed yet must still be noticed
    /// afterwards. The caller has checked that `new_path` does not exist.
    pub fn rename_to(&mut self, new_path: &Path) -> io::Result<()> {
        let old = self
            .path
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "document has no path"))?;
        let copied = move_file(&old, new_path)?;
        self.path = Some(canonical_target(new_path).unwrap_or_else(|_| new_path.to_path_buf()));
        if copied {
            // A cross-filesystem move is a copy, so the mtime is new. Adopt it
            // so no false "changed on disk" prompt follows.
            self.last_mtime = file_mtime(new_path).ok();
        }
        Ok(())
    }

    pub fn reload(&mut self) -> Result<(), ReadError> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| ReadError::Io(io::Error::from(io::ErrorKind::NotFound)))?;
        let (text, last_mtime) = read_utf8_limited_with_mtime(&path, MAX_OPEN_BYTES)?;
        self.line_ending = detect_line_ending(&text);
        self.saved_chars = text.chars().count();
        self.text = text;
        self.last_mtime = Some(last_mtime);
        Ok(())
    }

    /// Returns true when the file on disk has changed since this document was
    /// read or last saved.
    pub fn externally_modified(&self) -> bool {
        let Some(path) = &self.path else {
            return false;
        };
        let Ok(disk_mtime) = file_mtime(path) else {
            return false;
        };
        match self.last_mtime {
            Some(known) => known != disk_mtime,
            None => true,
        }
    }

    pub fn disk_mtime(&self) -> Option<SystemTime> {
        self.path.as_deref().and_then(|path| file_mtime(path).ok())
    }

    /// Adopt the file's current modification time without replacing the text.
    /// Used when an explicit reload finds identical disk content: the document
    /// stays exactly as it is, but a later focus check must not report the
    /// same modification again.
    pub fn adopt_disk_mtime(&mut self) -> io::Result<()> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        self.last_mtime = Some(file_mtime(path)?);
        Ok(())
    }

    /// Read the file as it is on disk right now, without changing this
    /// document. Used to inspect an external change before deciding whether
    /// to reload it.
    pub fn read_disk(&self) -> Result<String, ReadError> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| ReadError::Io(io::Error::from(io::ErrorKind::NotFound)))?;
        read_utf8_limited(path, MAX_OPEN_BYTES)
    }
}

/// Refuse to create a file that the editor would refuse to open again.
fn ensure_save_size(text: &str) -> io::Result<()> {
    if text.len() as u64 > MAX_OPEN_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "The document is {} bytes, which is more than the {} byte limit for saving. \
                 Remove some text before saving.",
                text.len(),
                MAX_OPEN_BYTES
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-doc-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn open_save_and_save_as() {
        let dir = test_dir();
        let first = dir.join("first.md");
        let second = dir.join("second.md");
        fs::write(&first, "hello\r\nworld\r\n").unwrap();

        let mut doc = Document::open(&first).unwrap();
        assert_eq!(doc.text(), "hello\r\nworld\r\n");
        assert!(doc.path().unwrap().is_absolute());

        doc.save_live("hello\r\nthere\r\n".into()).unwrap();
        assert_eq!(fs::read_to_string(&first).unwrap(), "hello\r\nthere\r\n");

        doc.save_as(&second, "new\n".into()).unwrap();
        assert_eq!(fs::read_to_string(&second).unwrap(), "new\n");
        assert_eq!(doc.path(), Some(second.canonicalize().unwrap().as_path()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_disk_leaves_the_document_unchanged() {
        let dir = test_dir();
        let path = dir.join("peek.md");
        fs::write(&path, "old\n").unwrap();
        let doc = Document::open(&path).unwrap();
        fs::write(&path, "new\n").unwrap();
        assert_eq!(doc.read_disk().unwrap(), "new\n");
        assert_eq!(doc.text(), "old\n");
        assert!(Document::new().read_disk().is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn adopting_disk_mtime_silences_an_identical_change() {
        let dir = test_dir();
        let path = dir.join("same.md");
        fs::write(&path, "same\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.set_last_mtime(None);
        assert!(doc.externally_modified());
        doc.adopt_disk_mtime().unwrap();
        assert!(!doc.externally_modified());
        assert_eq!(doc.text(), "same\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn differs_from_compares_with_the_saved_text() {
        let dir = test_dir();
        let path = dir.join("d.md");
        fs::write(&path, "abc\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert!(!doc.differs_from("abc\n"));
        assert!(doc.differs_from("abc"));
        assert!(doc.differs_from("abd\n"));
        assert!(doc.differs_from(""));

        doc.save_live("new\n".into()).unwrap();
        assert!(!doc.differs_from("new\n"));
        assert!(doc.differs_from("abc\n"));

        assert!(!Document::new().differs_from(""));
        assert!(Document::new().differs_from("x"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_character_count_follows_open_save_as_and_reload() {
        let dir = test_dir();
        let path = dir.join("a.md");
        fs::write(&path, "é猫\r\n").unwrap();
        assert_eq!(Document::new().saved_chars(), 0);
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.saved_chars(), 4);
        doc.save_live("é".into()).unwrap();
        assert_eq!(doc.saved_chars(), 1);
        let other = dir.join("b.md");
        doc.save_as(&other, "🦀é".into()).unwrap();
        assert_eq!(doc.saved_chars(), 2);
        fs::write(&other, "abc\r\n").unwrap();
        doc.reload().unwrap();
        assert_eq!(doc.saved_chars(), 5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_utf8_is_reported() {
        let dir = test_dir();
        let path = dir.join("bad.md");
        fs::write(&path, [0xff, 0x00, 0xfe]).unwrap();
        assert!(matches!(Document::open(&path), Err(ReadError::NotUtf8)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn nul_bytes_are_refused() {
        let dir = test_dir();
        let path = dir.join("nul.md");
        fs::write(&path, b"a\0b").unwrap();
        assert!(matches!(Document::open(&path), Err(ReadError::NulByte)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_files_are_refused() {
        let dir = test_dir();
        let path = dir.join("large.md");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_OPEN_BYTES + 1).unwrap();
        assert!(matches!(
            Document::open(&path),
            Err(ReadError::TooLarge { .. })
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_documents_cannot_be_saved() {
        let dir = test_dir();
        let path = dir.join("small.md");
        fs::write(&path, "keep").unwrap();
        let mut document = Document::open(&path).unwrap();
        let too_large = "x".repeat(MAX_OPEN_BYTES as usize + 1);

        assert!(document.save_live(too_large.clone()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep");

        let new_path = dir.join("new.md");
        assert!(document.save_as(&new_path, too_large).is_err());
        assert!(!new_path.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn line_ending_tracks_open_save_reload_and_conversion() {
        let dir = test_dir();
        let path = dir.join("eol.md");
        fs::write(&path, "a\r\nb\r\n").unwrap();
        let mut doc = Document::open(&path).unwrap();
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
        doc.save_live("a\nb\n".into()).unwrap();
        assert_eq!(doc.line_ending(), LineEnding::Lf);
        doc.set_line_ending(LineEnding::CrLf);
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
        fs::write(&path, "a\rb").unwrap();
        doc.reload().unwrap();
        assert_eq!(doc.line_ending(), LineEnding::Mixed);
        let _ = fs::remove_dir_all(dir);
    }
}
