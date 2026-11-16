//! Toolkit-independent view state for the Diff Mode toolbar.

use crate::core::session::DiffSession;

/// The toolbar controls derived from a review session.
pub(super) struct DiffToolbarState {
    pub(super) has_pending: bool,
    pub(super) can_accept: bool,
    pub(super) can_undo: bool,
    pub(super) inspection: Option<usize>,
    pub(super) accepted_hunks: usize,
    pub(super) exit: ExitState,
}

impl DiffToolbarState {
    pub(super) fn for_session(session: &DiffSession) -> Self {
        let has_pending = !session.diff().is_empty();
        Self {
            has_pending,
            can_accept: has_pending && session.focus().is_some(),
            can_undo: session.can_undo(),
            inspection: session.focus().filter(|_| session.is_inspecting()),
            accepted_hunks: session.accepted_hunks(),
            exit: ExitState::for_session(session),
        }
    }
}

/// What the toolbar's leave button says, based on review progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExitState {
    /// Nothing accepted yet.
    Close,
    /// Inspection has no uncommitted accepts: return to the full diff.
    Back,
    /// Some, but not all, hunks accepted.
    Confirm,
    /// Every hunk accepted; the label turns green.
    ConfirmAll,
}

impl ExitState {
    fn for_session(session: &DiffSession) -> Self {
        if !session.baseline_changed() {
            if session.is_inspecting() {
                Self::Back
            } else {
                Self::Close
            }
        } else if session.diff().is_empty() {
            Self::ConfirmAll
        } else {
            Self::Confirm
        }
    }

    pub(super) fn label(self, accepted: usize) -> String {
        match self {
            Self::Close => "Close".to_string(),
            Self::Back => "Back".to_string(),
            Self::Confirm | Self::ConfirmAll => {
                let noun = if accepted == 1 { "change" } else { "changes" };
                format!("Confirm ({accepted} {noun})")
            }
        }
    }

    pub(super) fn tooltip(self) -> Option<&'static str> {
        match self {
            Self::Close => None,
            Self::Back => Some("Return to full view (Enter)"),
            Self::Confirm => Some("Update baseline"),
            Self::ConfirmAll => Some("Sync baseline"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_follows_focus_accept_and_undo() {
        let mut session = DiffSession::new("one\ntwo\n".into(), "one\nTWO\n".into());

        let initial = DiffToolbarState::for_session(&session);
        assert!(initial.has_pending);
        assert!(!initial.can_accept);
        assert!(!initial.can_undo);
        assert_eq!(initial.exit, ExitState::Close);
        assert_eq!(initial.accepted_hunks, 0);

        assert!(session.focus_relative(1));
        assert!(DiffToolbarState::for_session(&session).can_accept);

        assert!(session.accept_focused().unwrap());
        let synced = DiffToolbarState::for_session(&session);
        assert!(!synced.has_pending);
        assert!(!synced.can_accept);
        assert!(synced.can_undo);
        assert_eq!(synced.exit, ExitState::ConfirmAll);
        assert_eq!(synced.accepted_hunks, 1);

        assert!(session.undo());
        let undone = DiffToolbarState::for_session(&session);
        assert!(undone.has_pending);
        assert!(undone.can_accept);
        assert!(!undone.can_undo);
        assert_eq!(undone.exit, ExitState::Close);
        assert_eq!(undone.accepted_hunks, 0);
    }

    #[test]
    fn exit_state_follows_progress() {
        let mut session = DiffSession::new("a\nb\nc\nd\ne\n".into(), "A\nb\nC\nd\nE\n".into());
        assert_eq!(ExitState::for_session(&session), ExitState::Close);
        session.focus_relative(1);
        session.accept_focused().unwrap();
        assert_eq!(ExitState::for_session(&session), ExitState::Confirm);
        session.mark_committed();
        let checkpoint = DiffToolbarState::for_session(&session);
        assert_eq!(checkpoint.exit, ExitState::Close);
        assert!(checkpoint.has_pending);
        assert!(checkpoint.can_accept);
        assert!(!checkpoint.can_undo);
        assert_eq!(checkpoint.accepted_hunks, 0);
        session.accept_focused().unwrap();
        assert_eq!(ExitState::for_session(&session), ExitState::Confirm);
        session.accept_focused().unwrap();
        assert_eq!(ExitState::for_session(&session), ExitState::ConfirmAll);
        let unchanged = DiffSession::new("".into(), "".into());
        assert_eq!(ExitState::for_session(&unchanged), ExitState::Close);
    }

    #[test]
    fn exit_state_labels() {
        assert_eq!(ExitState::Close.label(0), "Close");
        assert_eq!(ExitState::Back.label(0), "Back");
        for state in [ExitState::Confirm, ExitState::ConfirmAll] {
            assert_eq!(state.label(1), "Confirm (1 change)");
            assert_eq!(state.label(2), "Confirm (2 changes)");
        }
    }

    #[test]
    fn inspection_exit_tracks_accept_undo_and_commit() {
        let mut session = DiffSession::new("a\nb\nc\nd\ne\n".into(), "A\nb\nC\nd\nE\n".into());
        session.focus_relative(1);
        session.toggle_inspection();
        assert_eq!(ExitState::for_session(&session), ExitState::Back);
        session.accept_focused().unwrap();
        assert_eq!(ExitState::for_session(&session), ExitState::Confirm);
        session.undo();
        assert_eq!(ExitState::for_session(&session), ExitState::Back);
        session.accept_focused().unwrap();
        session.mark_committed();
        assert_eq!(ExitState::for_session(&session), ExitState::Back);
        session.toggle_inspection();
        assert_eq!(ExitState::for_session(&session), ExitState::Close);
        session.toggle_inspection();
        assert_eq!(ExitState::for_session(&session), ExitState::Back);
        session.accept_all();
        assert_eq!(ExitState::for_session(&session), ExitState::ConfirmAll);
    }

    #[test]
    fn confirm_tooltip_describes_the_pending_write() {
        assert_eq!(ExitState::Close.tooltip(), None);
        assert_eq!(
            ExitState::Back.tooltip(),
            Some("Return to full view (Enter)")
        );
        assert_eq!(ExitState::Confirm.tooltip(), Some("Update baseline"));
        assert_eq!(ExitState::ConfirmAll.tooltip(), Some("Sync baseline"));
    }
}
