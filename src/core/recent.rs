//! Recently opened document paths with bounded storage and lazy pruning.
//!
//! The state is app-written plain text under `$XDG_DATA_HOME/notediff/recent`:
//! one absolute UTF-8 path per line, newest first. Nothing is read or stat-ed
//! at startup; the list is loaded when a path is recorded or when the Open menu
//! is about to be shown. Missing files are dropped at that point, so a stale
//! entry costs nothing until the menu is used.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::fsutil::{atomic_write_if_changed, data_home, read_utf8_limited};
use super::snapshot_store::{LivePathState, live_path_state};

/// More entries are never remembered or shown.
pub const MAX_RECENT: usize = 10;

/// The state file is a short list; refuse to read an unexpectedly large file.
const MAX_STATE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug)]
pub struct RecentDocuments {
    path: Option<PathBuf>,
}

impl Default for RecentDocuments {
    fn default() -> Self {
        Self::at(Self::default_path())
    }
}

impl RecentDocuments {
    pub fn default_path() -> PathBuf {
        data_home().join("notediff").join("recent")
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    /// Test windows keep no recent state instead of touching the real file.
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self { path: None }
    }

    /// Put `path` at the front of the list, removing any earlier occurrence and
    /// dropping the oldest entry beyond [`MAX_RECENT`]. A path that cannot be
    /// stored as a single UTF-8 line is ignored. Write failures are silent:
    /// the recent list is disposable state.
    pub fn record(&self, path: &Path) {
        let Some(state) = &self.path else {
            return;
        };
        if !rememberable(path) {
            return;
        }
        let mut paths = self.load();
        paths.retain(|existing| existing != path);
        paths.insert(0, path.to_path_buf());
        paths.truncate(MAX_RECENT);
        let _ = atomic_write_if_changed(state, &render(&paths), true);
    }

    /// The remembered paths, newest first, with missing files removed. The
    /// filesystem is checked only here, never at startup. The state file is
    /// rewritten only when pruning changed the list or normalized its text.
    pub fn pruned_entries(&self) -> Vec<PathBuf> {
        let Some(state) = &self.path else {
            return Vec::new();
        };
        let text = self.read_text();
        let remembered = parse(&text);
        let mut seen = HashSet::new();
        let pruned: Vec<PathBuf> = remembered
            .into_iter()
            .filter(|path| rememberable(path) && present_or_unavailable(path))
            .filter(|path| seen.insert(path.clone()))
            .take(MAX_RECENT)
            .collect();

        if pruned.is_empty() {
            let _ = fs::remove_file(state);
        } else {
            let rendered = render(&pruned);
            if rendered != text {
                let _ = atomic_write_if_changed(state, &rendered, true);
            }
        }
        pruned
    }

    /// Forget every recent path. The file itself is removed, not blanked.
    pub fn clear(&self) -> io::Result<()> {
        let Some(state) = &self.path else {
            return Ok(());
        };
        match fs::remove_file(state) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn read_text(&self) -> String {
        let Some(state) = &self.path else {
            return String::new();
        };
        read_utf8_limited(state, MAX_STATE_BYTES).unwrap_or_default()
    }

    fn load(&self) -> Vec<PathBuf> {
        parse(&self.read_text())
    }
}

/// Absolute UTF-8 paths without control characters can be one state-file line
/// and one menu label.
fn rememberable(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

fn parse(text: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for line in text.lines() {
        let path = PathBuf::from(line);
        if rememberable(&path) && seen.insert(path.clone()) {
            paths.push(path);
            if paths.len() == MAX_RECENT {
                break;
            }
        }
    }
    paths
}

fn render(paths: &[PathBuf]) -> String {
    let mut text = String::new();
    for path in paths {
        if let Some(line) = path.to_str() {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// A missing file is pruned, but a file on a shared-volume mount that is not
/// available right now is kept: it can reappear when the volume is reconnected.
fn present_or_unavailable(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => metadata.is_file(),
        Err(_) => live_path_state(path) == LivePathState::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-recent-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "text\n").unwrap();
        path
    }

    #[test]
    fn record_prepends_dedupes_and_keeps_ten() {
        let dir = test_dir();
        let recent = RecentDocuments::at(dir.join("recent"));
        let paths: Vec<PathBuf> = (0..12)
            .map(|index| dir.join(format!("note-{index}.md")))
            .collect();

        for path in &paths {
            recent.record(path);
        }
        assert_eq!(
            recent.load(),
            paths[2..].iter().rev().cloned().collect::<Vec<_>>()
        );

        // Reopening an older path moves it to the front without duplicating it.
        recent.record(&paths[3]);
        let entries = recent.load();
        assert_eq!(entries[0], paths[3]);
        assert_eq!(entries.len(), MAX_RECENT);
        assert_eq!(entries.iter().filter(|path| *path == &paths[3]).count(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_ignores_relative_duplicate_and_control_paths() {
        let dir = test_dir();
        let path = dir.join("valid.md");
        let mut text = format!(
            "relative.md\n{}\n{}\n{}\n",
            path.display(),
            path.display(),
            dir.join("with\ttab.md").display()
        );
        text.push_str(&format!("{}\n", dir.join("more.md").display()));

        let parsed = parse(&text);
        assert_eq!(parsed, vec![path.clone(), dir.join("more.md")]);

        let many: String = (0..15)
            .map(|index| format!("{}\n", dir.join(format!("note-{index}.md")).display()))
            .collect();
        assert_eq!(parse(&many).len(), MAX_RECENT);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pruning_drops_missing_files_and_rewrites_the_state() {
        let dir = test_dir();
        let state = dir.join("recent");
        let first = touch(&dir, "first.md");
        let second = touch(&dir, "second.md");
        let third = touch(&dir, "third.md");
        let recent = RecentDocuments::at(state.clone());

        for path in [&first, &second, &third] {
            recent.record(path);
        }
        fs::remove_file(&second).unwrap();

        assert_eq!(recent.pruned_entries(), vec![third.clone(), first.clone()]);
        assert_eq!(
            fs::read_to_string(&state).unwrap(),
            format!("{}\n{}\n", third.display(), first.display())
        );

        // Nothing left: the state file itself disappears.
        fs::remove_file(&first).unwrap();
        fs::remove_file(&third).unwrap();
        assert!(recent.pruned_entries().is_empty());
        assert!(!state.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pruning_keeps_paths_on_unavailable_mounts() {
        let dir = test_dir();
        let state = dir.join("recent");
        let missing = PathBuf::from(format!(
            "/media/notediff-recent-test-{}-{}/volume/note.md",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&state, format!("{}\n", missing.display())).unwrap();
        let recent = RecentDocuments::at(state);

        assert_eq!(recent.pruned_entries(), vec![missing]);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_is_idempotent_and_removes_the_file() {
        let dir = test_dir();
        let state = dir.join("recent");
        let note = touch(&dir, "note.md");
        let recent = RecentDocuments::at(state.clone());
        recent.record(&note);
        assert!(state.exists());

        recent.clear().unwrap();
        assert!(!state.exists());
        recent.clear().unwrap();
        assert!(recent.pruned_entries().is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn disabled_state_does_nothing() {
        let recent = RecentDocuments::disabled();
        let path = PathBuf::from("/tmp/notediff-disabled.md");
        recent.record(&path);
        assert!(recent.pruned_entries().is_empty());
        recent.clear().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_not_recorded() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = test_dir();
        let recent = RecentDocuments::at(dir.join("recent"));
        let invalid = dir.join(OsStr::from_bytes(b"bad-\xff.md"));
        recent.record(&invalid);
        assert!(recent.load().is_empty());

        let _ = fs::remove_dir_all(dir);
    }
}
