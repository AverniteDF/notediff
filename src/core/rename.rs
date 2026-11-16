//! Renaming or moving the open document's file, carrying its baseline along,
//! free of any toolkit types.
//!
//! The user supplies either a new file name (rename in place) or a path
//! (move). A path may be absolute, relative to the file's current folder, or
//! start with `~/`. A path that names an existing folder, or ends in `/`,
//! moves the file into that folder under its current name. The destination
//! folder must already exist and nothing is overwritten.
//!
//! The live file is moved first because it is the source of truth; the
//! baseline follows. If the baseline cannot be moved, the file stays where it
//! went and the old baseline remains an orphan that Manage Snapshots can link.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::document::Document;
use super::snapshot_store::SnapshotStore;

#[derive(Debug)]
pub enum RenameError {
    /// The document is untitled, so there is no file to rename.
    NoPath,
    /// The input is empty (after trimming).
    Empty,
    /// The input contains NUL or does not end in a usable file name.
    InvalidName,
    /// The destination is the file's current path.
    Unchanged,
    /// The destination folder does not exist.
    MissingFolder(PathBuf),
    /// Another file already has the destination path.
    TargetExists(PathBuf),
    Io(io::Error),
}

impl fmt::Display for RenameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenameError::NoPath => write!(f, "The document has no file to rename."),
            RenameError::Empty => write!(f, "That is not a valid file name or path"),
            RenameError::InvalidName => write!(f, "That is not a valid file name or path"),
            RenameError::Unchanged => write!(f, "The file is already there."),
            RenameError::MissingFolder(path) => {
                write!(f, "The folder {} does not exist", path.display())
            }
            RenameError::TargetExists(path) => {
                write!(f, "{} already exists.", path.display())
            }
            RenameError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for RenameError {}

/// What happened to the baseline during a successful rename or move.
#[derive(Debug)]
pub enum BaselineMove {
    /// The old path had no baseline.
    None,
    /// The baseline now lives under the new path.
    Moved,
    /// The file was moved but the baseline could not be moved.
    Failed(io::Error),
}

/// Expand a leading `~` or `~/` to the home folder, when `HOME` is set.
fn expand_home(input: &str) -> PathBuf {
    if (input == "~" || input.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty())
    {
        let rest = input.trim_start_matches('~').trim_start_matches('/');
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(input)
}

/// Work out the new canonical path for `current` from the user's `input`.
/// The destination folder is canonicalized, so it must exist.
pub fn plan(current: &Path, input: &str) -> Result<PathBuf, RenameError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(RenameError::Empty);
    }
    if input.contains('\0') {
        return Err(RenameError::InvalidName);
    }

    let current_name = current.file_name().ok_or(RenameError::InvalidName)?;
    let base = current.parent().unwrap_or_else(|| Path::new("/"));
    // An absolute input replaces the base when joined.
    let requested = base.join(expand_home(input));

    let target = if input.ends_with('/') || requested.is_dir() {
        requested.join(current_name)
    } else {
        requested
    };

    let name = target
        .file_name()
        .ok_or(RenameError::InvalidName)?
        .to_owned();
    let folder = target.parent().unwrap_or_else(|| Path::new("/"));
    let canonical_folder =
        fs::canonicalize(folder).map_err(|_| RenameError::MissingFolder(folder.to_path_buf()))?;

    let planned = canonical_folder.join(name);
    if planned == current {
        Err(RenameError::Unchanged)
    } else {
        Ok(planned)
    }
}

/// Rename or move the document's file according to `input` (see [`plan`]) and
/// move its baseline with it. Nothing is overwritten on disk: an existing file
/// at the destination is an error. A baseline already stored under the new
/// path belongs to a file that does not exist (otherwise the move would have
/// been refused), so it is an orphan and is replaced by the moved baseline.
pub fn rename_document(
    doc: &mut Document,
    store: &SnapshotStore,
    input: &str,
) -> Result<BaselineMove, RenameError> {
    let old = doc.path().ok_or(RenameError::NoPath)?.to_path_buf();
    let new = plan(&old, input)?;
    if fs::symlink_metadata(&new).is_ok() {
        return Err(RenameError::TargetExists(new));
    }

    doc.rename_to(&new).map_err(RenameError::Io)?;

    let renamed = doc.path().unwrap_or(&new).to_path_buf();
    Ok(match store.move_baseline(&old, &renamed) {
        Ok(true) => BaselineMove::Moved,
        Ok(false) => BaselineMove::None,
        Err(err) => BaselineMove::Failed(err),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn validation_toasts_have_no_trailing_period() {
        assert_eq!(
            RenameError::Empty.to_string(),
            RenameError::InvalidName.to_string()
        );
        assert_eq!(
            RenameError::InvalidName.to_string(),
            "That is not a valid file name or path"
        );
        assert_eq!(
            RenameError::MissingFolder(PathBuf::from("/missing")).to_string(),
            "The folder /missing does not exist"
        );
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-rename-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn open(dir: &Path, name: &str, text: &str) -> Document {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        Document::open(&path).unwrap()
    }

    #[test]
    fn plain_names_rename_in_place() {
        let dir = test_dir();
        let current = dir.join("a.md");
        assert_eq!(plan(&current, " b.md ").unwrap(), dir.join("b.md"));
        assert_eq!(plan(&current, "./b.md").unwrap(), dir.join("b.md"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_input_is_refused() {
        let dir = test_dir();
        let current = dir.join("a.md");
        assert!(matches!(plan(&current, "   "), Err(RenameError::Empty)));
        assert!(matches!(
            plan(&current, "a\0b"),
            Err(RenameError::InvalidName)
        ));
        assert!(matches!(
            plan(&current, "nope/.."),
            Err(RenameError::InvalidName)
        ));
        assert!(matches!(
            plan(&current, "a.md"),
            Err(RenameError::Unchanged)
        ));
        assert!(matches!(plan(&current, "."), Err(RenameError::Unchanged)));
        assert!(matches!(
            plan(&current, "nope/b.md"),
            Err(RenameError::MissingFolder(_))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn paths_and_folders_move_the_file() {
        let dir = test_dir();
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let current = dir.join("a.md");

        // A folder (with or without a trailing slash) keeps the file name.
        assert_eq!(plan(&current, "sub/").unwrap(), sub.join("a.md"));
        assert_eq!(plan(&current, "sub").unwrap(), sub.join("a.md"));
        // A path with a new name.
        assert_eq!(plan(&current, "sub/b.md").unwrap(), sub.join("b.md"));
        // Absolute and parent-relative paths.
        let absolute = sub.join("c.md");
        assert_eq!(
            plan(&current, absolute.to_str().unwrap()).unwrap(),
            absolute
        );
        let from_sub = sub.join("a.md");
        assert_eq!(plan(&from_sub, "../a.md").unwrap(), dir.join("a.md"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_moves_file_and_baseline() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let mut doc = open(&dir, "a.md", "live\r\n");
        let old = doc.path().unwrap().to_path_buf();
        store.write_for_canonical(&old, "base\r\n").unwrap();

        let outcome = rename_document(&mut doc, &store, "b.md").unwrap();
        assert!(matches!(outcome, BaselineMove::Moved));

        let new = dir.join("b.md");
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(&new).unwrap(), "live\r\n");
        assert_eq!(doc.path(), Some(new.as_path()));
        assert_eq!(doc.text(), "live\r\n");
        assert_eq!(
            store.read_for_canonical(&new).unwrap().as_deref(),
            Some("base\r\n")
        );
        assert!(!store.has_for_canonical(&old));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn moving_to_another_folder_carries_the_baseline() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let mut doc = open(&dir, "a.md", "live\n");
        let old = doc.path().unwrap().to_path_buf();
        store.write_for_canonical(&old, "base\n").unwrap();

        let outcome = rename_document(&mut doc, &store, "sub/").unwrap();
        assert!(matches!(outcome, BaselineMove::Moved));

        let new = sub.join("a.md");
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(&new).unwrap(), "live\n");
        assert_eq!(doc.path(), Some(new.as_path()));
        assert!(!store.has_for_canonical(&old));
        assert_eq!(
            store.read_for_canonical(&new).unwrap().as_deref(),
            Some("base\n")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rename_without_a_baseline_reports_none() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let mut doc = open(&dir, "a.md", "live\n");
        let outcome = rename_document(&mut doc, &store, "b.md").unwrap();
        assert!(matches!(outcome, BaselineMove::None));
        assert!(dir.join("b.md").exists());
        assert!(!store.has_for_canonical(&dir.join("b.md")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn existing_target_is_refused_and_nothing_changes() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let mut doc = open(&dir, "a.md", "live\n");
        fs::write(dir.join("b.md"), "other\n").unwrap();
        let old = doc.path().unwrap().to_path_buf();
        store.write_for_canonical(&old, "base\n").unwrap();

        let result = rename_document(&mut doc, &store, "b.md");
        assert!(matches!(result, Err(RenameError::TargetExists(_))));
        assert_eq!(doc.path(), Some(old.as_path()));
        assert_eq!(fs::read_to_string(dir.join("b.md")).unwrap(), "other\n");
        assert_eq!(fs::read_to_string(&old).unwrap(), "live\n");
        assert!(store.has_for_canonical(&old));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn leftover_baseline_at_the_new_path_is_replaced() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let mut doc = open(&dir, "a.md", "live\n");
        let old = doc.path().unwrap().to_path_buf();
        let new = dir.join("b.md");
        store.write_for_canonical(&old, "fresh\n").unwrap();
        store.write_for_canonical(&new, "stale\n").unwrap();

        let outcome = rename_document(&mut doc, &store, "b.md").unwrap();
        assert!(matches!(outcome, BaselineMove::Moved));
        assert_eq!(
            store.read_for_canonical(&new).unwrap().as_deref(),
            Some("fresh\n")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn untitled_documents_cannot_be_renamed() {
        let dir = test_dir();
        let store = SnapshotStore::at(dir.join("store"));
        let mut doc = Document::new();
        assert!(matches!(
            rename_document(&mut doc, &store, "b.md"),
            Err(RenameError::NoPath)
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
