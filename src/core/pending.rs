//! Actions that wait for the live document to be saved before they run.
//!
//! The window never passes closures around for the "save first, then
//! continue" flow. It passes one of these values, which keeps the flow
//! explicit and lets the prompt rules be tested without GTK.
//!
//! The two small enums at the end (`ExternalCheck`, `AfterOpen`) play the same
//! role for the external-change check and for an aborted open: they name what
//! happens next instead of carrying a closure.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    /// Write the first baseline for the saved document.
    CreateBaseline,
    /// Commit accepted hunks, leaving Diff Mode only if none remain pending.
    ConfirmBaseline,
    /// Open the selected path after the current document is saved or discarded.
    OpenPath(PathBuf),
    /// Replace the current document with an empty, untitled one.
    NewFile,
    /// Close the window.
    CloseWindow,
    /// Continue quitting after this dirty window's save prompt is resolved.
    QuitApplication,
}

impl PendingAction {
    /// For actions that write a baseline: the phrase that names the action in
    /// the save prompt. These actions need the document saved, so the prompt
    /// offers only Cancel and Save. `None` for actions that merely leave the
    /// document, which may also discard the unsaved changes.
    pub fn baseline_label(&self) -> Option<&'static str> {
        match self {
            PendingAction::CreateBaseline => Some("Creating a baseline"),
            PendingAction::ConfirmBaseline => Some("Confirming the baseline"),
            PendingAction::OpenPath(_)
            | PendingAction::NewFile
            | PendingAction::CloseWindow
            | PendingAction::QuitApplication => None,
        }
    }

    /// True when the user may choose to throw away unsaved changes and
    /// continue anyway.
    pub fn can_discard(&self) -> bool {
        self.baseline_label().is_none()
    }
}

/// What follows a check for external changes once it has settled with the
/// document up to date (reloaded, kept as is, or never changed on disk).
/// Cancelling the check's dialogs drops the follow-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalCheck {
    /// Nothing follows. The check ran because the window regained focus.
    Focus,
    /// Start a Diff Mode session.
    EnterDiff,
}

/// What happens when the user cancels the Convert Tabs prompt of an open.
/// Nothing is loaded in either case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterOpen {
    /// Leave the window as it is.
    Stay,
    /// Close the window. Used for a window created only for that file
    /// (command-line launch).
    CloseWindowOnCancel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_actions_require_a_save() {
        for action in [
            PendingAction::CreateBaseline,
            PendingAction::ConfirmBaseline,
        ] {
            assert!(action.baseline_label().is_some());
            assert!(!action.can_discard());
        }
    }

    #[test]
    fn leaving_actions_may_discard() {
        for action in [
            PendingAction::OpenPath(PathBuf::from("/tmp/example.md")),
            PendingAction::NewFile,
            PendingAction::CloseWindow,
            PendingAction::QuitApplication,
        ] {
            assert!(action.baseline_label().is_none());
            assert!(action.can_discard());
        }
    }

    #[test]
    fn baseline_labels_are_distinct() {
        assert_ne!(
            PendingAction::CreateBaseline.baseline_label(),
            PendingAction::ConfirmBaseline.baseline_label()
        );
    }
}
