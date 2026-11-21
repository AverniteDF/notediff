use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replace `path` with `contents`.
///
/// The normal path writes a sibling temporary file and renames it over the
/// destination. Some shared filesystems (notably ChromeOS shared folders)
/// reject the rename; only then does the function fall back to a direct write.
/// A failure while creating or writing the temporary file never touches the
/// original destination.
///
/// When `create_parent` is true, missing parent directories are created first.
pub fn atomic_write(path: &Path, contents: &str, create_parent: bool) -> io::Result<()> {
    if create_parent && let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let existing_permissions = fs::metadata(path).ok().map(|meta| meta.permissions());

    let mut last_rename_error = None;
    for attempt in 0..4 {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        // Keep the temporary name short. Including the destination file name
        // can exceed NAME_MAX for otherwise valid long names and would force
        // every save of such a file through the non-atomic path.
        let temp_path = parent.join(format!(
            ".notediff-{}-{stamp}-{nonce}.tmp",
            std::process::id()
        ));

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Existing private files must not become readable while the temporary
        // file is being written. The mode is masked by the process umask; the
        // exact destination permissions are applied after the write. New files
        // keep the normal umask-derived mode.
        #[cfg(unix)]
        if let Some(permissions) = &existing_permissions {
            options.mode(permissions.mode() & 0o777);
        }
        let mut file = match options.open(&temp_path) {
            Ok(file) => file,
            Err(err) => {
                let retry = matches!(
                    err.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::Interrupted
                ) && attempt < 3;
                if retry {
                    continue;
                }
                return Err(err);
            }
        };

        let written = file
            .write_all(contents.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(err) = written {
            let _ = fs::remove_file(&temp_path);
            return Err(err);
        }

        // Some filesystems do not support chmod. Leave the umask-masked mode
        // in that case, which is never wider than the intended destination
        // permissions for an existing file.
        if let Some(permissions) = &existing_permissions {
            let _ = fs::set_permissions(&temp_path, permissions.clone());
        }

        match fs::rename(&temp_path, path) {
            Ok(()) => {
                sync_parent_dir(parent);
                return Ok(());
            }
            Err(err) => {
                let _ = fs::remove_file(&temp_path);
                let retry = matches!(
                    err.kind(),
                    io::ErrorKind::AlreadyExists | io::ErrorKind::Interrupted
                ) && attempt < 3;
                last_rename_error = Some(err);
                if retry {
                    continue;
                }
                break;
            }
        }
    }

    let Some(rename_error) = last_rename_error else {
        return Err(io::Error::other("atomic write failed without an error"));
    };
    if !may_fall_back_to_direct_write(&rename_error) {
        return Err(rename_error);
    }

    // The temporary file was written successfully and the rename was rejected
    // by the mount. A direct write is not atomic, but it is the only option for
    // those filesystems. Any failure is reported as a normal write failure.
    direct_write(path, contents)?;
    sync_parent_dir(parent);
    Ok(())
}

/// Like [`atomic_write`], but skip the write when `path` already holds exactly
/// `contents`, and report whether a write happened.
///
/// Startup code uses this so cache files (the installed scheme XMLs) are not
/// rewritten with an fsync on every launch. A read failure falls back to a
/// normal write, so a missing, unreadable or corrupt file still recovers.
pub fn atomic_write_if_changed(
    path: &Path,
    contents: &str,
    create_parent: bool,
) -> io::Result<bool> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    atomic_write(path, contents, create_parent)?;
    Ok(true)
}

/// Whether a rename failure may indicate a filesystem that does not support
/// sibling renames. Other failures (notably disk-full and generic I/O errors)
/// must leave the original file untouched.
fn may_fall_back_to_direct_write(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied
            | io::ErrorKind::Unsupported
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::CrossesDevices
    )
}

/// Best-effort directory sync so a completed rename survives a crash. Failure
/// is not fatal: the file and rename have already succeeded.
fn sync_parent_dir(parent: &Path) {
    let _ = fs::File::open(parent).and_then(|directory| directory.sync_all());
}

fn direct_write(path: &Path, contents: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

/// Read a UTF-8 text file, refusing files above `max_bytes`.
///
/// The read is bounded even when the file grows during the read, and special
/// files such as devices and FIFOs are rejected instead of blocking or
/// producing unbounded input.
pub fn read_utf8_limited(path: &Path, max_bytes: u64) -> Result<String, ReadError> {
    let metadata = fs::metadata(path).map_err(ReadError::Io)?;
    if !metadata.is_file() {
        return Err(ReadError::NotRegularFile);
    }
    let declared = metadata.len();
    if declared > max_bytes {
        return Err(ReadError::TooLarge {
            bytes: declared,
            limit: max_bytes,
        });
    }
    let file = fs::File::open(path).map_err(ReadError::Io)?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(ReadError::TooLarge {
            bytes: bytes.len() as u64,
            limit: max_bytes,
        });
    }
    let text = String::from_utf8(bytes).map_err(|_| ReadError::NotUtf8)?;
    if text.as_bytes().contains(&0) {
        return Err(ReadError::NulByte);
    }
    Ok(text)
}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    NotUtf8,
    /// The path is not a regular file (for example a folder or a device).
    NotRegularFile,
    /// The file is valid text but contains a NUL byte, which GtkTextBuffer
    /// cannot round-trip safely.
    NulByte,
    TooLarge {
        bytes: u64,
        limit: u64,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(err) => write!(f, "{err}"),
            ReadError::NotUtf8 => write!(
                f,
                "The file is not valid UTF-8, so it cannot be opened in notediff."
            ),
            ReadError::NotRegularFile => {
                write!(f, "That is not a regular file.")
            }
            ReadError::NulByte => write!(
                f,
                "The file contains a NUL byte, which notediff cannot safely edit."
            ),
            ReadError::TooLarge { bytes, limit } => write!(
                f,
                "The file is {bytes} bytes, which is more than the {limit} byte limit for opening."
            ),
        }
    }
}

impl ReadError {
    /// User-facing text for an open failure. OS error numbers are left out;
    /// the common path failures get a plain-language message instead.
    pub fn open_message(&self) -> String {
        match self {
            ReadError::Io(err) => match err.kind() {
                io::ErrorKind::NotFound => "No such file or directory.".into(),
                io::ErrorKind::PermissionDenied => "Permission denied.".into(),
                _ => "The file could not be read.".into(),
            },
            other => other.to_string(),
        }
    }
}

impl std::error::Error for ReadError {}

/// Resolve a path to the canonical target that will hold the document and its
/// baseline. For a path that does not exist yet (Save As), canonicalize the
/// parent directory and append the file name.
pub fn canonical_target(path: &Path) -> io::Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path);
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    Ok(canonical_parent.join(name))
}

pub fn file_mtime(path: &Path) -> io::Result<SystemTime> {
    fs::metadata(path)?.modified()
}

/// `$XDG_DATA_HOME`, falling back to `~/.local/share`.
pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

/// Move a file. A plain rename is used when possible; when `from` and `to` are
/// on different filesystems the file is copied and the original removed.
/// Returns true when the copy fallback was used (the new file then has a new
/// modification time). The caller has checked that `to` does not exist.
pub fn move_file(from: &Path, to: &Path) -> io::Result<bool> {
    match fs::rename(from, to) {
        Ok(()) => Ok(false),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            if let Err(err) = fs::copy(from, to) {
                let _ = fs::remove_file(to);
                return Err(err);
            }
            if let Err(err) = fs::remove_file(from) {
                let _ = fs::remove_file(to);
                return Err(err);
            }
            Ok(true)
        }
        Err(err) => Err(err),
    }
}

/// Remove empty directories from `start` upwards, stopping at `stop`
/// (exclusive). Used when an orphaned snapshot is deleted so the mirror does
/// not accumulate empty folders.
pub fn prune_empty_dirs(start: &Path, stop: &Path) {
    let mut current = start.to_path_buf();
    while current != stop {
        if fs::remove_dir(&current).is_err() {
            break;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notediff-fsutil-{}-{}-{}",
            std::process::id(),
            name,
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = test_dir("write");
        let path = dir.join("nested/file.txt");
        atomic_write(&path, "one", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        atomic_write(&path, "two", true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn atomic_write_if_changed_skips_identical_content() {
        let dir = test_dir("write-if-changed");
        let path = dir.join("nested/scheme.xml");
        assert!(atomic_write_if_changed(&path, "one", true).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        // Unchanged content is left alone, including its modification time.
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(!atomic_write_if_changed(&path, "one", true).unwrap());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        // Changed content is still written, and a missing file is recreated.
        assert!(atomic_write_if_changed(&path, "two", true).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        fs::remove_file(&path).unwrap();
        assert!(atomic_write_if_changed(&path, "two", true).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn atomic_write_handles_long_file_names() {
        let dir = test_dir("long-name");
        let path = dir.join(format!("{}.txt", "a".repeat(200)));
        atomic_write(&path, "one", false).unwrap();
        atomic_write(&path, "two", false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = test_dir("permissions");
        let path = dir.join("private.txt");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        atomic_write(&path, "new", false).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_temp_write_leaves_the_original_alone() {
        use std::os::unix::fs::PermissionsExt;

        let dir = test_dir("readonly-temp");
        let path = dir.join("keep.txt");
        fs::write(&path, "original").unwrap();

        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        permissions.set_mode(0o555);
        fs::set_permissions(&dir, permissions).unwrap();

        // Root can still create files in a 0555 directory; there is no useful
        // permission failure to test in that case.
        let probe = dir.join("probe");
        let writable = fs::write(&probe, "x").is_ok();
        if !writable {
            assert!(atomic_write(&path, "replacement", false).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        }

        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        permissions.set_mode(0o755);
        let _ = fs::set_permissions(&dir, permissions);
        let _ = fs::remove_file(&probe);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_rename_rejections_use_the_direct_write_fallback() {
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported,
            io::ErrorKind::InvalidInput,
            io::ErrorKind::CrossesDevices,
        ] {
            assert!(may_fall_back_to_direct_write(&io::Error::from(kind)));
        }
        for kind in [io::ErrorKind::Other, io::ErrorKind::NotFound] {
            assert!(!may_fall_back_to_direct_write(&io::Error::from(kind)));
        }
    }

    #[test]
    fn open_messages_are_friendly_and_hide_os_error_numbers() {
        assert_eq!(
            ReadError::Io(io::Error::from(io::ErrorKind::NotFound)).open_message(),
            "No such file or directory."
        );
        assert_eq!(
            ReadError::Io(io::Error::from(io::ErrorKind::PermissionDenied)).open_message(),
            "Permission denied."
        );
        assert_eq!(
            ReadError::Io(io::Error::from(io::ErrorKind::NotADirectory)).open_message(),
            "The file could not be read."
        );
        for error in [ReadError::NotUtf8, ReadError::NotRegularFile] {
            assert_eq!(error.open_message(), error.to_string());
            assert!(!error.open_message().contains("os error"));
        }
        let too_large = ReadError::TooLarge {
            bytes: 10,
            limit: 5,
        };
        assert_eq!(too_large.open_message(), too_large.to_string());
        assert!(!too_large.open_message().contains("os error"));
    }

    #[test]
    fn read_utf8_rejects_invalid_bytes() {
        let dir = test_dir("utf8");
        let path = dir.join("bad.txt");
        fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(matches!(
            read_utf8_limited(&path, u64::MAX),
            Err(ReadError::NotUtf8)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_utf8_rejects_nul_bytes() {
        let dir = test_dir("nul");
        let path = dir.join("nul.txt");
        fs::write(&path, b"before\0after").unwrap();
        assert!(matches!(
            read_utf8_limited(&path, u64::MAX),
            Err(ReadError::NulByte)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_utf8_limited_refuses_oversized_files() {
        let dir = test_dir("large");
        let path = dir.join("large.txt");
        fs::write(&path, "123456").unwrap();
        assert!(matches!(
            read_utf8_limited(&path, 5),
            Err(ReadError::TooLarge { bytes: 6, limit: 5 })
        ));
        assert_eq!(read_utf8_limited(&path, 6).unwrap(), "123456");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_utf8_rejects_non_regular_files() {
        let dir = test_dir("non-regular");
        assert!(matches!(
            read_utf8_limited(&dir, u64::MAX),
            Err(ReadError::NotRegularFile)
        ));
        #[cfg(unix)]
        assert!(matches!(
            read_utf8_limited(Path::new("/dev/null"), 16),
            Err(ReadError::NotRegularFile)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn move_file_renames_on_one_filesystem() {
        let dir = test_dir("move");
        let from = dir.join("a.txt");
        let to = dir.join("b.txt");
        fs::write(&from, "x").unwrap();
        assert!(!move_file(&from, &to).unwrap());
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "x");
        let _ = fs::remove_dir_all(dir);
    }
}
