//! The central baseline store.
//!
//! A document's identity is its canonical absolute path. The store mirrors
//! that path beneath `$XDG_DATA_HOME/notediff/snapshots`, so a document at
//! `/home/you/notes/todo.md` has its baseline at
//! `.../notediff/snapshots/home/you/notes/todo.md`.
//!
//! Baselines are plain byte-for-byte text files with no metadata. Existence
//! and modification time are enough.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use super::MAX_OPEN_BYTES;
use super::fsutil::{ReadError, atomic_write, data_home, prune_empty_dirs, read_utf8_limited};

#[derive(Debug, Clone)]
pub struct SnapshotStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivePathState {
    /// The live file exists.
    Present,
    /// The live file is missing, but its location is reachable.
    Orphan,
    /// An ancestor mount (for example an unshared `/mnt/chromeos` folder) is
    /// absent, so the app cannot know whether the file still exists.
    Unavailable,
}

/// How a baseline is linked to another document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMode {
    /// The baseline changes owner: the old snapshot is removed. Used for
    /// orphans, whose old live file is gone.
    Move,
    /// The baseline is duplicated: the original stays with its own file.
    Copy,
}

#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    pub snapshot_path: PathBuf,
    pub live_path: PathBuf,
    pub modified: Option<SystemTime>,
    pub state: LivePathState,
}

impl Default for SnapshotStore {
    fn default() -> Self {
        Self::at(data_home().join("notediff").join("snapshots"))
    }
}

impl SnapshotStore {
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Map an already-canonical live path into the store.
    pub fn snapshot_path_for_canonical(&self, canonical: &Path) -> PathBuf {
        let mut relative = PathBuf::new();
        for component in canonical.components() {
            if let Component::Normal(value) = component {
                relative.push(value);
            }
        }
        self.root.join(relative)
    }

    pub fn read_for_canonical(&self, canonical: &Path) -> Result<Option<String>, ReadError> {
        let path = self.snapshot_path_for_canonical(canonical);
        match read_utf8_limited(&path, MAX_OPEN_BYTES) {
            Ok(text) => Ok(Some(text)),
            Err(ReadError::Io(err)) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// True when a baseline file exists for the canonical live path.
    pub fn has_for_canonical(&self, canonical: &Path) -> bool {
        self.snapshot_path_for_canonical(canonical).is_file()
    }

    /// The baseline file's modification time, for the Diff Mode header.
    pub fn mtime_for_canonical(&self, canonical: &Path) -> Option<SystemTime> {
        fs::metadata(self.snapshot_path_for_canonical(canonical))
            .ok()?
            .modified()
            .ok()
    }

    pub fn write_for_canonical(&self, canonical: &Path, text: &str) -> io::Result<()> {
        if text.len() as u64 > MAX_OPEN_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the baseline is larger than the supported size limit",
            ));
        }
        let path = self.snapshot_path_for_canonical(canonical);
        atomic_write(&path, text, true)
    }

    #[cfg(test)]
    pub fn delete_for_canonical(&self, canonical: &Path) -> io::Result<()> {
        let path = self.snapshot_path_for_canonical(canonical);
        delete_file_and_prune(&path, &self.root)
    }

    /// Delete a snapshot using the mirrored snapshot path directly. This is
    /// how orphaned snapshots are removed, because their live path cannot be
    /// canonicalized any more.
    pub fn delete_snapshot_path(&self, snapshot_path: &Path) -> io::Result<()> {
        if !within_store(&self.root, snapshot_path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to delete a path outside the snapshot store",
            ));
        }
        delete_file_and_prune(snapshot_path, &self.root)
    }

    /// Give the document at `target_canonical` a baseline taken from
    /// `source_snapshot` (a path inside the store). Linking never replaces an
    /// existing baseline. The target is written first and the source removed
    /// afterwards (for `Move`), so a failure never loses the baseline. If only
    /// the final removal fails, the error says the transfer itself succeeded.
    pub fn link(
        &self,
        source_snapshot: &Path,
        target_canonical: &Path,
        mode: LinkMode,
    ) -> io::Result<()> {
        if !within_store(&self.root, source_snapshot) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to link a baseline from outside the snapshot store",
            ));
        }
        let destination = self.snapshot_path_for_canonical(target_canonical);
        if destination == source_snapshot {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a baseline cannot be linked to its own document",
            ));
        }
        let source_meta = match fs::symlink_metadata(source_snapshot) {
            Ok(meta) => meta,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the baseline to link no longer exists",
                ));
            }
        };
        if !source_meta.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "refusing to link a baseline that is not a regular file inside the store",
            ));
        }
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the document already has a baseline; linking never replaces one",
            ));
        }
        self.transfer(source_snapshot, &destination, mode == LinkMode::Move)
    }

    /// Move the baseline of `from` to `to` after the live file was renamed.
    /// Returns false when `from` had no baseline. A baseline already stored
    /// under `to` is replaced: `to` did not exist as a file, so it was an
    /// orphan.
    pub fn move_baseline(&self, from: &Path, to: &Path) -> io::Result<bool> {
        let source = self.snapshot_path_for_canonical(from);
        let destination = self.snapshot_path_for_canonical(to);
        if source == destination || !source.is_file() {
            return Ok(false);
        }
        self.transfer(&source, &destination, true)?;
        Ok(true)
    }

    fn transfer(&self, source: &Path, destination: &Path, remove_source: bool) -> io::Result<()> {
        let text = read_utf8_limited(source, MAX_OPEN_BYTES).map_err(read_error_to_io)?;
        atomic_write(destination, &text, true)?;
        if remove_source {
            delete_file_and_prune(source, &self.root).map_err(|err| {
                io::Error::new(
                    err.kind(),
                    format!(
                        "the baseline was moved, but the old snapshot could not be removed: {err}"
                    ),
                )
            })?;
        }
        Ok(())
    }

    /// Walk the whole store and report every snapshot, including orphans and
    /// snapshots on unavailable mounts.
    pub fn scan(&self) -> io::Result<Vec<SnapshotEntry>> {
        let mut entries = Vec::new();
        if self.root.exists() {
            walk(&self.root, &self.root, &mut entries)?;
        }
        entries.sort_by(|a, b| a.live_path.cmp(&b.live_path));
        Ok(entries)
    }
}

fn walk(root: &Path, directory: &Path, entries: &mut Vec<SnapshotEntry>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk(root, &path, entries)?;
        } else if file_type.is_file() {
            let Some(relative) = path.strip_prefix(root).ok() else {
                continue;
            };
            if relative.as_os_str().is_empty() {
                continue;
            }
            let live_path = Path::new("/").join(relative);
            let state = live_path_state(&live_path);
            let modified = entry.metadata().ok().and_then(|meta| meta.modified().ok());
            entries.push(SnapshotEntry {
                snapshot_path: path,
                live_path,
                modified,
                state,
            });
        }
    }
    Ok(())
}

/// Classify a live path: present, missing (orphan) or on an unavailable
/// mount. Used by the baseline snapshot cleanup.
pub fn live_path_state(path: &Path) -> LivePathState {
    if path.exists() {
        return LivePathState::Present;
    }

    if location_unavailable(path) {
        return LivePathState::Unavailable;
    }

    LivePathState::Orphan
}

/// The expected mount point for a path on a shared-volume root. A missing
/// nested folder below this point is deleted content (an orphan), while a
/// missing mount point means the volume itself is unavailable.
fn mount_root(path: &Path) -> Option<PathBuf> {
    let components: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect();

    let first = components.first().and_then(|value| value.to_str())?;
    let second = components.get(1).and_then(|value| value.to_str());
    let third = components.get(2).and_then(|value| value.to_str());
    let root_len = match (first, second, third) {
        ("mnt", Some("chromeos"), Some("removable")) => 4,
        ("mnt", Some("chromeos"), _) => 3,
        ("mnt", _, _) => 2,
        ("media", _, _) => 3,
        ("run", Some("media"), _) => 4,
        _ => return None,
    };
    if components.len() <= root_len {
        return None;
    }

    let mut root = PathBuf::from("/");
    for component in &components[..root_len] {
        root.push(component);
    }
    Some(root)
}

/// ChromeOS shared folders live under `/mnt/chromeos` (and occasionally other
/// mount roots). If the volume mount point is absent at container start,
/// report "location unavailable" rather than an orphan.
fn location_unavailable(path: &Path) -> bool {
    match mount_root(path) {
        Some(root) => !root.exists(),
        None => false,
    }
}

fn read_error_to_io(err: ReadError) -> io::Error {
    match err {
        ReadError::Io(err) => err,
        ReadError::NotUtf8 => io::Error::new(
            io::ErrorKind::InvalidData,
            "the baseline is not valid UTF-8",
        ),
        ReadError::NotRegularFile => io::Error::new(
            io::ErrorKind::InvalidData,
            "the baseline is not a regular file",
        ),
        ReadError::NulByte => io::Error::new(
            io::ErrorKind::InvalidData,
            "the baseline contains a NUL byte",
        ),
        ReadError::TooLarge { .. } => io::Error::new(
            io::ErrorKind::InvalidData,
            "the baseline is larger than the supported size limit",
        ),
    }
}

fn within_store(root: &Path, path: &Path) -> bool {
    path != root
        && path.starts_with(root)
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

fn delete_file_and_prune(path: &Path, root: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                prune_empty_dirs(parent, root);
            }
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-store-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn maps_absolute_paths_under_the_store() {
        let store = SnapshotStore::at(PathBuf::from("/data/snapshots"));
        let mapped = store.snapshot_path_for_canonical(Path::new("/home/you/notes/todo.md"));
        assert_eq!(
            mapped,
            PathBuf::from("/data/snapshots/home/you/notes/todo.md")
        );
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let live = dir.join("live.md");
        fs::write(&live, "live\n").unwrap();
        let canonical = live.canonicalize().unwrap();

        assert_eq!(store.read_for_canonical(&canonical).unwrap(), None);
        assert!(!store.has_for_canonical(&canonical));
        store
            .write_for_canonical(&canonical, "baseline\r\n")
            .unwrap();
        assert!(store.has_for_canonical(&canonical));
        assert_eq!(
            store.read_for_canonical(&canonical).unwrap().as_deref(),
            Some("baseline\r\n")
        );
        assert!(store.snapshot_path_for_canonical(&canonical).exists());
        assert!(store.mtime_for_canonical(&canonical).is_some());

        store.delete_for_canonical(&canonical).unwrap();
        assert_eq!(store.read_for_canonical(&canonical).unwrap(), None);
        assert!(!store.snapshot_path_for_canonical(&canonical).exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_baseline_paths_are_errors_not_missing() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        fs::create_dir_all(dir.join("store")).unwrap();
        // A regular file where the snapshot's parent directory should be makes
        // lookup fail with NotADirectory; that must not look like "no baseline".
        fs::write(dir.join("store/blocked"), "not a directory").unwrap();
        let error = store
            .read_for_canonical(Path::new("/blocked/notes.md"))
            .unwrap_err();
        assert!(matches!(
            error,
            ReadError::Io(err) if err.kind() == io::ErrorKind::NotADirectory
        ));

        // A directory at the snapshot path is reported as a non-regular file.
        fs::create_dir_all(dir.join("store/ok/notes.md")).unwrap();
        let error = store
            .read_for_canonical(Path::new("/ok/notes.md"))
            .unwrap_err();
        assert!(matches!(error, ReadError::NotRegularFile));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_baselines_are_refused() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let live = dir.join("live.md");
        fs::write(&live, "live\n").unwrap();
        let canonical = live.canonicalize().unwrap();
        let text = "x".repeat(MAX_OPEN_BYTES as usize + 1);

        let error = store.write_for_canonical(&canonical, &text).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!store.has_for_canonical(&canonical));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_separates_orphans_and_unavailable() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let live = dir.join("present.md");
        fs::write(&live, "live\n").unwrap();
        store
            .write_for_canonical(&live.canonicalize().unwrap(), "baseline\n")
            .unwrap();

        let orphan = store.root().join("definitely/missing/orphan.md");
        atomic_write(&orphan, "orphan\n", true).unwrap();

        // Simulate a path under a missing mount. We cannot write through the
        // real /mnt, so invoke the helper directly.
        assert!(location_unavailable(Path::new(
            "/mnt/notediff-test-unavailable/notes/file.md"
        )));
        assert!(!location_unavailable(Path::new(
            "/home/you/missing/file.md"
        )));

        let entries = store.scan().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|entry| entry.state == LivePathState::Present)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.state == LivePathState::Orphan)
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn identifies_shared_volume_mount_roots() {
        assert_eq!(
            mount_root(Path::new("/mnt/chromeos/Downloads/notes/todo.md")),
            Some(PathBuf::from("/mnt/chromeos/Downloads"))
        );
        assert_eq!(
            mount_root(Path::new("/mnt/data/notes/todo.md")),
            Some(PathBuf::from("/mnt/data"))
        );
        assert_eq!(
            mount_root(Path::new("/media/you/DRIVE/notes/todo.md")),
            Some(PathBuf::from("/media/you/DRIVE"))
        );
        assert_eq!(
            mount_root(Path::new("/run/media/you/DRIVE/notes/todo.md")),
            Some(PathBuf::from("/run/media/you/DRIVE"))
        );
        assert_eq!(mount_root(Path::new("/home/you/notes/todo.md")), None);
        assert_eq!(mount_root(Path::new("/mnt/chromeos")), None);
    }

    #[test]
    fn refuses_to_delete_outside_store() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let outside = dir.join("outside.txt");
        fs::write(&outside, "keep\n").unwrap();
        assert!(store.delete_snapshot_path(&outside).is_err());
        assert!(outside.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_parent_dir_paths_in_the_store() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let outside = dir.join("outside.txt");
        fs::write(&outside, "keep\n").unwrap();
        let traversal = store.root().join("../outside.txt");

        assert!(store.delete_snapshot_path(&traversal).is_err());
        assert!(outside.exists());
        assert!(
            store
                .link(&traversal, &dir.join("target.md"), LinkMode::Copy)
                .is_err()
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn link_move_transfers_and_removes_the_source() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let old = Path::new("/gone/old/name.md");
        let new = dir.join("new.md");
        store.write_for_canonical(old, "baseline\r\n").unwrap();
        let source = store.snapshot_path_for_canonical(old);

        store.link(&source, &new, LinkMode::Move).unwrap();

        assert_eq!(
            store.read_for_canonical(&new).unwrap().as_deref(),
            Some("baseline\r\n")
        );
        assert!(!source.exists());
        assert!(!store.root().join("gone").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn link_copy_keeps_the_source() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let original = dir.join("original.md");
        let copy = dir.join("copy.md");
        store.write_for_canonical(&original, "baseline\n").unwrap();
        let source = store.snapshot_path_for_canonical(&original);

        store.link(&source, &copy, LinkMode::Copy).unwrap();

        assert_eq!(
            store.read_for_canonical(&original).unwrap().as_deref(),
            Some("baseline\n")
        );
        assert_eq!(
            store.read_for_canonical(&copy).unwrap().as_deref(),
            Some("baseline\n")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn link_never_replaces_an_existing_baseline() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let old = Path::new("/gone/old.md");
        let target = dir.join("target.md");
        store.write_for_canonical(old, "from the orphan\n").unwrap();
        store.write_for_canonical(&target, "mine\n").unwrap();
        let source = store.snapshot_path_for_canonical(old);

        let err = store.link(&source, &target, LinkMode::Move).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            store.read_for_canonical(&target).unwrap().as_deref(),
            Some("mine\n")
        );
        assert!(source.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn link_refuses_sources_outside_the_store_and_self_links() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let outside = dir.join("outside.txt");
        fs::write(&outside, "keep\n").unwrap();
        let target = dir.join("target.md");
        let err = store.link(&outside, &target, LinkMode::Move).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(outside.exists());

        store.write_for_canonical(&target, "own\n").unwrap();
        let own = store.snapshot_path_for_canonical(&target);
        let err = store.link(&own, &target, LinkMode::Copy).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let missing = store.root().join("nothing/here.md");
        let other = dir.join("other.md");
        let err = store.link(&missing, &other, LinkMode::Move).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn move_baseline_follows_a_rename() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let from = dir.join("a.md");
        let to = dir.join("b.md");

        assert!(!store.move_baseline(&from, &to).unwrap());

        store.write_for_canonical(&from, "base\n").unwrap();
        assert!(store.move_baseline(&from, &to).unwrap());
        assert!(!store.has_for_canonical(&from));
        assert_eq!(
            store.read_for_canonical(&to).unwrap().as_deref(),
            Some("base\n")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mount_root_requires_a_path_inside_the_mount_and_handles_chromeos_removable() {
        assert_eq!(mount_root(Path::new("/mnt/foo.md")), None);
        assert_eq!(
            mount_root(Path::new("/mnt/drive/foo.md")),
            Some(PathBuf::from("/mnt/drive"))
        );
        assert_eq!(mount_root(Path::new("/mnt/chromeos/foo.md")), None);
        assert_eq!(
            mount_root(Path::new("/mnt/chromeos/MyFiles/foo.md")),
            Some(PathBuf::from("/mnt/chromeos/MyFiles"))
        );
        assert_eq!(mount_root(Path::new("/mnt/chromeos/removable/USB")), None);
        assert_eq!(
            mount_root(Path::new("/mnt/chromeos/removable/USB/notes.md")),
            Some(PathBuf::from("/mnt/chromeos/removable/USB"))
        );
    }

    #[test]
    fn within_store_rejects_store_root_and_link_rejects_symlinks() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        assert!(!within_store(store.root(), store.root()));
        assert!(within_store(store.root(), &store.root().join("a.md")));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = dir.join("secret.txt");
            fs::write(&outside, "secret\n").unwrap();
            let link_in_store = store.root().join("gone/link.md");
            fs::create_dir_all(link_in_store.parent().unwrap()).unwrap();
            symlink(&outside, &link_in_store).unwrap();
            let target = dir.join("target.md");
            let err = store
                .link(&link_in_store, &target, LinkMode::Copy)
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        }

        let _ = fs::remove_dir_all(dir);
    }
}
