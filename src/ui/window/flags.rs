//! Per-window document flags. The scoped loading flag suppresses edit tracking
//! and lets tabs through during open/reload, which have their own tab policy.

use std::cell::Cell;
use std::time::SystemTime;

#[derive(Default)]
pub(super) struct DocumentFlags {
    /// Assumed dirty after an edit, then settled against saved text on idle.
    pub(super) dirty: Cell<bool>,
    pub(super) dirty_check_pending: Cell<bool>,
    pub(super) read_only: Cell<bool>,
    pub(super) allow_close: Cell<bool>,
    pub(super) external_dialog_open: Cell<bool>,
    /// Disk mtime for which the user cancelled the reload's tab prompt.
    pub(super) declined_reload_mtime: Cell<Option<SystemTime>>,
    /// Keep the line endings already chosen by Insert Text from File instead of
    /// normalizing them again in the generic insert filter.
    pub(super) keep_inserted_line_endings: Cell<bool>,
    loading: Cell<bool>,
}

impl DocumentFlags {
    pub(super) fn is_loading(&self) -> bool {
        self.loading.get()
    }

    /// Restore the loading flag even if the closure panics.
    pub(super) fn with_programmatic_load<R>(&self, load: impl FnOnce() -> R) -> R {
        struct Guard<'a>(&'a Cell<bool>);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }

        self.loading.set(true);
        let _guard = Guard(&self.loading);
        load()
    }

    /// Temporarily leave line endings alone while inserting text. Tabs are
    /// still converted by the insert filter.
    pub(super) fn with_inserted_line_endings<R>(&self, insert: impl FnOnce() -> R) -> R {
        struct Guard<'a>(&'a Cell<bool>, bool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }

        let previous = self.keep_inserted_line_endings.replace(true);
        let _guard = Guard(&self.keep_inserted_line_endings, previous);
        insert()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_flag_is_scoped() {
        let flags = DocumentFlags::default();
        assert!(!flags.is_loading());
        let seen = flags.with_programmatic_load(|| flags.is_loading());
        assert!(seen);
        assert!(!flags.is_loading());
    }

    #[test]
    fn loading_flag_is_cleared_after_a_panic() {
        let flags = DocumentFlags::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            flags.with_programmatic_load(|| panic!("boom"));
        }));
        assert!(result.is_err());
        assert!(!flags.is_loading());
    }

    #[test]
    fn inserted_line_endings_are_scoped() {
        let flags = DocumentFlags::default();
        assert!(!flags.keep_inserted_line_endings.get());
        let seen = flags.with_inserted_line_endings(|| flags.keep_inserted_line_endings.get());
        assert!(seen);
        assert!(!flags.keep_inserted_line_endings.get());
    }
}
