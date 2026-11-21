//! Diff Mode session state, free of any toolkit types.
//!
//! A session holds the baseline (`S`), the live text (`D`), the current diff,
//! the focused hunk, inspection state and the undo stack. Accept and undo
//! update the baseline, re-diff and move the focus. Accepts stay in memory; the window writes the accepted
//! baseline to the snapshot store only when the user confirms.

use std::ops::Range;

use super::diff::{self, DiffResult};
use super::inline::{self, InlineChange};
use super::undo::UndoStack;

/// Undo history is bounded by both entry count and retained baseline bytes.
/// At the 16 MiB document cap this keeps four full-size undo steps, while
/// ordinary documents retain the full 100-entry history.
const MAX_UNDO_ENTRIES: usize = 100;
const MAX_UNDO_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct DiffSession {
    baseline: String,
    live: String,
    diff: DiffResult,
    focus: Option<usize>,
    inspecting: bool,
    inline_changes: Vec<InlineChange>,
    inline_revision: Option<u64>,
    inline_focus: Option<usize>,
    remembered_inline: Option<InlineChange>,
    undo: UndoStack<SessionSnapshot>,
    accepted_hunks: usize,
    original_baseline: String,
    content_revision: u64,
}

#[derive(Debug)]
struct SessionSnapshot {
    baseline: String,
    focus: Option<usize>,
    accepted_hunks: usize,
    inline_focus: Option<usize>,
}

fn new_undo() -> UndoStack<SessionSnapshot> {
    UndoStack::new(MAX_UNDO_ENTRIES, MAX_UNDO_BYTES, |snapshot| {
        snapshot.baseline.len()
    })
}

impl DiffSession {
    /// Start a session. No hunk is focused yet.
    pub fn new(baseline: String, live: String) -> Self {
        let diff = diff::diff_texts(&baseline, &live);
        Self {
            original_baseline: baseline.clone(),
            baseline,
            live,
            diff,
            focus: None,
            inspecting: false,
            inline_changes: Vec::new(),
            inline_revision: None,
            inline_focus: None,
            remembered_inline: None,
            undo: new_undo(),
            accepted_hunks: 0,
            content_revision: 0,
        }
    }

    pub fn baseline(&self) -> &str {
        &self.baseline
    }

    pub fn live(&self) -> &str {
        &self.live
    }

    pub fn diff(&self) -> &DiffResult {
        &self.diff
    }

    /// Changes only when Accept/Undo changes the texts and recomputes the diff.
    pub fn content_revision(&self) -> u64 {
        self.content_revision
    }

    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    pub fn is_inspecting(&self) -> bool {
        self.inspecting
    }

    /// Switch between the full diff and the focused hunk.
    pub fn toggle_inspection(&mut self) -> bool {
        if self.focus.is_none() {
            return false;
        }
        self.inspecting = !self.inspecting;
        if self.inspecting {
            self.refresh_inline();
            self.select_inline_for_hunk();
            if let Some(index) = self.inline_changes.iter().position(|change| {
                Some(change.hunk) == self.focus && Some(change) == self.remembered_inline.as_ref()
            }) {
                self.inline_focus = Some(index);
            }
        } else {
            self.remembered_inline = self.selected_inline().cloned();
            self.inline_focus = None;
        }
        true
    }

    pub fn selected_inline(&self) -> Option<&InlineChange> {
        self.inline_focus
            .and_then(|index| self.inline_changes.get(index))
    }

    pub fn inline_focus(&self) -> Option<usize> {
        self.inline_focus
    }

    fn refresh_inline(&mut self) {
        if self.inline_revision != Some(self.content_revision) {
            self.inline_changes = inline::changes(&self.baseline, &self.live, &self.diff);
            self.inline_revision = Some(self.content_revision);
        }
    }

    fn select_inline_for_hunk(&mut self) {
        self.inline_focus = self
            .inline_changes
            .iter()
            .position(|change| Some(change.hunk) == self.focus);
    }

    /// Accepted review units: whole hunks in full diff, individual changes in
    /// inspection, or pending hunks for Accept All. Reset by Confirm.
    pub fn accepted_hunks(&self) -> usize {
        self.accepted_hunks
    }

    /// Whether confirming must persist an updated baseline, independently of
    /// the progress counter and the bounded undo history.
    pub fn baseline_changed(&self) -> bool {
        self.baseline != self.original_baseline
    }

    /// Start a fresh accept/undo checkpoint after a successful disk write.
    /// The diff, focus and texts are unchanged.
    pub fn mark_committed(&mut self) {
        self.original_baseline.clone_from(&self.baseline);
        self.accepted_hunks = 0;
        self.undo = new_undo();
    }

    pub fn can_undo(&self) -> bool {
        self.undo.peek().is_some()
    }

    /// Move between whole hunks in full diff or individual changes during
    /// inspection (no wrap). With nothing focused, select the first hunk.
    pub fn focus_relative(&mut self, direction: i32) -> bool {
        if self.inspecting {
            if self.inline_changes.is_empty() {
                return false;
            }
            let last = self.inline_changes.len() - 1;
            let current = self.inline_focus.unwrap_or(0);
            let index = (current as i64 + i64::from(direction)).clamp(0, last as i64) as usize;
            self.inline_focus = Some(index);
            self.focus = Some(self.inline_changes[index].hunk);
            return true;
        }
        let count = self.diff.hunks.len();
        if count == 0 {
            return false;
        }
        self.focus = Some(match self.focus {
            Some(current) => {
                (current as i64 + i64::from(direction)).clamp(0, count as i64 - 1) as usize
            }
            None => 0,
        });
        true
    }

    /// Accept the focused hunk, or only the selected change during inspection,
    /// as one undoable step. Re-diff and advance to the next pending unit.
    /// Returns `Ok(false)` (changing nothing) when no hunk is focused.
    pub fn accept_focused(&mut self) -> Result<bool, String> {
        if self.inspecting {
            return self.accept_inline();
        }
        let Some(index) = self.focus else {
            return Ok(false);
        };
        let Some(&hunk) = self.diff.hunks.get(index) else {
            return Ok(false);
        };
        let after = diff::apply_hunk(&self.baseline, &self.live, &hunk)?;
        self.replace_baseline(after, Some(index));
        self.accepted_hunks += 1;
        self.focus = if self.diff.hunks.is_empty() {
            self.inspecting = false;
            None
        } else {
            Some(index.min(self.diff.hunks.len() - 1))
        };
        Ok(true)
    }

    fn accept_inline(&mut self) -> Result<bool, String> {
        let Some(change) = self.selected_inline().cloned() else {
            return Ok(false);
        };
        let region = self
            .diff
            .hunks
            .get(change.hunk)
            .map(|hunk| inline::hunk_live_span(&self.live, hunk));
        let after = inline::apply(&self.baseline, &self.live, &change)?;
        self.replace_baseline(after, self.focus);
        self.accepted_hunks += 1;
        if self.diff.is_empty() {
            self.inspecting = false;
            self.focus = None;
            self.inline_changes.clear();
            self.inline_revision = None;
            self.inline_focus = None;
        } else {
            self.refresh_inline();
            // Live-text coordinates never move during a review. Follow the
            // accepted change, not a volatile hunk or inline ordinal.
            let successor = self
                .inline_changes
                .iter()
                .position(|item| item.new.start >= change.new.end);
            // A re-diff may split or merge the inspected hunk, so use its
            // stable live-text span instead of an inline or hunk index.
            let successor_in_region = successor.filter(|&index| {
                region
                    .as_ref()
                    .is_some_and(|region| inside_live_region(&self.inline_changes[index], region))
            });
            let nearest_above = region.as_ref().and_then(|region| {
                self.inline_changes.iter().rposition(|item| {
                    inside_live_region(item, region) && item.new.end <= change.new.start
                })
            });
            let index = successor_in_region
                .or(nearest_above)
                .or(successor)
                .unwrap_or(self.inline_changes.len() - 1);
            self.inline_focus = Some(index);
            self.focus = Some(self.inline_changes[index].hunk);
        }
        Ok(true)
    }

    /// Accept every pending hunk as a single undoable step. Returns false
    /// (changing nothing) when nothing is pending.
    pub fn accept_all(&mut self) -> bool {
        if self.diff.is_empty() {
            return false;
        }
        // Apply every hunk rather than copying the live text directly. This
        // exercises the same path as individual accepts and preserves the core
        // invariant: accepting all hunks equals D.
        let after = diff::apply_all_hunks(&self.baseline, &self.live);
        let focus = self.focus;
        let pending = self.diff.hunks.len();
        self.replace_baseline(after, focus);
        self.accepted_hunks += pending;
        self.focus = None;
        self.inspecting = false;
        self.inline_changes.clear();
        self.inline_revision = None;
        self.inline_focus = None;
        true
    }

    /// Undo the last accept, restoring the previous baseline and focus.
    /// Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        let restored_focus = previous.focus;
        self.baseline = previous.baseline;
        self.accepted_hunks = previous.accepted_hunks;
        self.rediff();
        let count = self.diff.hunks.len();
        self.focus = restored_focus
            .filter(|_| count > 0)
            .map(|index| index.min(count - 1));
        if self.inspecting {
            self.refresh_inline();
            self.inline_focus = previous
                .inline_focus
                .filter(|index| *index < self.inline_changes.len());
            if self.inline_focus.is_none() {
                self.select_inline_for_hunk();
            }
            if let Some(change) = self.selected_inline() {
                self.focus = Some(change.hunk);
            }
        }
        true
    }

    /// Swap in a new baseline, remembering the old one (and the focus to
    /// restore) for undo, then re-diff.
    fn replace_baseline(&mut self, after: String, focus: Option<usize>) {
        let before = std::mem::replace(&mut self.baseline, after);
        self.undo.push(SessionSnapshot {
            baseline: before,
            focus,
            accepted_hunks: self.accepted_hunks,
            inline_focus: self.inline_focus,
        });
        self.rediff();
    }

    fn rediff(&mut self) {
        self.diff = diff::diff_texts(&self.baseline, &self.live);
        self.content_revision = self.content_revision.wrapping_add(1);
    }
}

/// Whether a pending change belongs to the originally inspected live region.
/// An empty span cannot hold another change: a pure deletion hunk is one
/// atomic review unit.
fn inside_live_region(change: &InlineChange, region: &Range<usize>) -> bool {
    !region.is_empty() && change.new.start >= region.start && change.new.end <= region.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_accepts_inserted_or_deleted_blocks_in_one_undo_step() {
        for (old, live, checkpoint) in [
            (
                "anchor\nlast old\n",
                "one\r\n\n🙂three\nanchor\nlast new\n",
                "one\r\n\n🙂three\nanchor\nlast old\n",
            ),
            (
                "one\r\n\n🙂three\nanchor\nlast old\n",
                "anchor\nlast new\n",
                "anchor\nlast old\n",
            ),
        ] {
            let mut session = DiffSession::new(old.into(), live.into());
            session.focus_relative(1);
            session.toggle_inspection();
            let block = session.selected_inline().unwrap().clone();
            assert_eq!(block.kind, inline::ChangeKind::Block);
            assert_eq!(session.inline_changes.len(), 2);
            session.focus_relative(1);
            assert_eq!(session.focus(), Some(1));
            session.focus_relative(-1);
            assert_eq!(session.selected_inline(), Some(&block));
            session.accept_focused().unwrap();
            assert_eq!(session.baseline(), checkpoint);
            assert_eq!(session.accepted_hunks(), 1);
            assert!(session.is_inspecting());
            assert_eq!(session.focus(), Some(0));
            assert!(session.undo());
            assert_eq!(session.baseline(), old);
            assert_eq!(session.selected_inline(), Some(&block));
            assert!(!session.can_undo());
            session.accept_focused().unwrap();
            session.mark_committed();
            assert_eq!(session.accepted_hunks(), 0);
            assert!(!session.can_undo());
            session.accept_focused().unwrap();
            assert_eq!(session.baseline(), live);
            assert!(!session.is_inspecting());
        }
    }

    #[test]
    fn inspection_accepts_one_word_change_and_restores_exact_selection() {
        let old = "The quick brown fox jumps.\r\n";
        let live = "The fast brown fox sleeps.\n";
        let mut session = DiffSession::new(old.into(), live.into());
        session.focus_relative(1);
        session.toggle_inspection();
        assert_eq!(session.inline_changes.len(), 3);
        assert_eq!(
            &old[session.selected_inline().unwrap().old.clone()],
            "quick"
        );
        session.focus_relative(1);
        let selected = session.selected_inline().unwrap().clone();
        assert_eq!(&old[selected.old.clone()], "jumps");
        assert_eq!(session.accept_focused(), Ok(true));
        assert_eq!(session.baseline(), "The quick brown fox sleeps.\r\n");
        assert_eq!(session.accepted_hunks(), 1);
        assert_eq!(
            session.selected_inline().unwrap().kind,
            inline::ChangeKind::Ending
        );
        assert!(session.undo());
        assert_eq!(session.baseline(), old);
        assert_eq!(session.selected_inline(), Some(&selected));
        assert_eq!(session.accepted_hunks(), 0);
        session.accept_focused().unwrap();
        session.mark_committed();
        assert!(session.is_inspecting());
        assert!(!session.can_undo());
        assert_eq!(session.accepted_hunks(), 0);
        session.accept_focused().unwrap();
        assert_eq!(session.baseline(), "The quick brown fox sleeps.\n");
        assert_eq!(session.inline_focus(), Some(0));
        session.accept_focused().unwrap();
        assert_eq!(session.baseline(), live);
        assert!(!session.is_inspecting());
        assert!(session.undo());
        assert!(!session.is_inspecting());
        assert_eq!(session.baseline(), "The quick brown fox sleeps.\n");
    }

    #[test]
    fn inspection_accept_drains_the_inspected_hunk_before_advancing() {
        let old = "a one\nb two\nanchor\nd three\ne four\n";
        let live = "a ONE\nb TWO\nanchor\nd THREE\ne FOUR\n";
        let mut session = DiffSession::new(old.into(), live.into());
        assert_eq!(session.diff().hunks.len(), 2);
        session.focus_relative(1);
        session.toggle_inspection();
        assert_eq!(session.inline_changes.len(), 4);

        // Select the bottom-most change in the first hunk. Its live-position
        // successor is in the next hunk, but this hunk still has a change.
        session.focus_relative(1);
        assert_eq!(&live[session.selected_inline().unwrap().new.clone()], "TWO");
        assert_eq!(session.accept_focused(), Ok(true));
        assert!(session.is_inspecting());
        assert_eq!(session.focus(), Some(0));
        assert_eq!(&live[session.selected_inline().unwrap().new.clone()], "ONE");

        // Once the hunk is drained, inspection advances to the next hunk.
        assert_eq!(session.accept_focused(), Ok(true));
        assert_eq!(session.focus(), Some(0));
        assert_eq!(
            &live[session.selected_inline().unwrap().new.clone()],
            "THREE"
        );

        // The last hunk has no successor, so its bottom-most accept stays on
        // the nearest remaining change above it.
        session.focus_relative(1);
        assert_eq!(
            &live[session.selected_inline().unwrap().new.clone()],
            "FOUR"
        );
        assert_eq!(session.accept_focused(), Ok(true));
        assert_eq!(
            &live[session.selected_inline().unwrap().new.clone()],
            "THREE"
        );
        assert_eq!(session.accept_focused(), Ok(true));
        assert!(!session.is_inspecting());
    }

    #[test]
    fn inspection_accept_keeps_whitespace_units_stable() {
        let old = "- Font size and per-window zoom (Ctrl+scroll, Ctrl+plus/minus), with the default size in the config file.\n";
        let live = "- Per-window zoom (Ctrl+scroll, Ctrl+plus/minus) relative to the persisted application-wide font size, with a reset-to-default action.\n";
        let mut session = DiffSession::new(old.into(), live.into());
        session.focus_relative(1);
        session.toggle_inspection();
        assert_eq!(&live[session.selected_inline().unwrap().new.clone()], "Per");

        session.accept_focused().unwrap();
        let second = session.selected_inline().unwrap().new.clone();
        assert_eq!(&live[second.clone()], " relative to");
        assert!(!live[second].ends_with(' '));

        session.accept_focused().unwrap();
        assert!(session.baseline().contains("relative to the "));
        assert!(!session.baseline().contains("to  the"));
    }

    #[test]
    fn inspection_treats_a_split_hunk_as_one_live_region() {
        let old = "a one\nb two\nc three\n";
        let live = "a ONE\nb TWO2\nc THREE\n";
        let mut session = DiffSession::new(old.into(), live.into());
        assert_eq!(session.diff().hunks.len(), 1);
        session.focus_relative(1);
        session.toggle_inspection();
        assert_eq!(session.inline_changes.len(), 3);

        // Accepting the middle line makes it equal and splits the hunk.
        session.focus_relative(1);
        assert_eq!(
            &live[session.selected_inline().unwrap().new.clone()],
            "TWO2"
        );
        assert_eq!(session.accept_focused(), Ok(true));

        // The successor is in the second half, still inside the original
        // hunk's live-text span, so it is followed normally.
        assert_eq!(session.diff().hunks.len(), 2);
        assert_eq!(session.focus(), Some(1));
        assert_eq!(
            &live[session.selected_inline().unwrap().new.clone()],
            "THREE"
        );
    }

    #[test]
    fn inline_navigation_crosses_hunks_without_wrapping() {
        let mut session = DiffSession::new(
            "one old two old\nanchor\nold\n".into(),
            "one new two new\nanchor\nnew\n".into(),
        );
        session.focus_relative(1);
        session.toggle_inspection();
        session.focus_relative(-1);
        assert_eq!(session.inline_focus(), Some(0));
        session.focus_relative(1);
        assert_eq!(session.focus(), Some(0));
        assert_eq!(session.inline_focus(), Some(1));
        session.focus_relative(1);
        assert_eq!(session.focus(), Some(1));
        assert_eq!(session.inline_focus(), Some(2));
        session.focus_relative(1);
        assert_eq!(session.inline_focus(), Some(2));
        session.toggle_inspection();
        session.toggle_inspection();
        assert_eq!(session.inline_focus(), Some(2));
        session.toggle_inspection();
        session.focus_relative(-1);
        assert_eq!(session.focus(), Some(0));
        session.accept_focused().unwrap();
        assert_eq!(session.baseline(), "one new two new\nanchor\nold\n");
    }

    #[test]
    fn accepting_random_inline_changes_converges_and_undo_is_exact() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = |max: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize % max
        };
        for case in 0..500 {
            let mut text = || {
                let mut text = String::new();
                for _ in 0..next(12) {
                    for _ in 0..next(6) {
                        text.push_str(["a ", "b ", "é ", "🙂", " ", ","][next(6)]);
                    }
                    text.push_str(["\n", "\r\n"][next(2)]);
                }
                if next(2) == 0 {
                    text.pop();
                }
                text
            };
            let baseline = text();
            let live = text();
            let mut session = DiffSession::new(baseline, live.clone());
            if session.diff().is_empty() {
                continue;
            }
            session.focus_relative(1);
            session.toggle_inspection();
            let mut steps = 0;
            while session.is_inspecting() {
                assert!(steps < 500, "case {case} failed to converge");
                let index = next(session.inline_changes.len());
                session.inline_focus = Some(index);
                session.focus = Some(session.inline_changes[index].hunk);
                let before = session.baseline().to_string();
                let selected = session.selected_inline().cloned();
                let accepted = session.accepted_hunks();
                assert_eq!(session.accept_focused(), Ok(true));
                assert_ne!(session.baseline(), before);
                assert_eq!(session.accepted_hunks(), accepted + 1);
                if session.is_inspecting() && next(4) == 0 {
                    assert!(session.undo());
                    assert_eq!(session.baseline(), before);
                    assert_eq!(session.selected_inline(), selected.as_ref());
                    assert_eq!(session.accepted_hunks(), accepted);
                    session.accept_focused().unwrap();
                }
                steps += 1;
            }
            assert_eq!(session.baseline(), live, "case {case}");
        }
    }

    #[test]
    fn content_revision_changes_only_for_successful_text_operations() {
        let mut session = DiffSession::new("a\nb\nc\n".into(), "A\nb\nC\n".into());
        assert_eq!(session.content_revision(), 0);
        assert_eq!(session.accept_focused(), Ok(false));
        assert!(!session.undo());
        session.focus_relative(1);
        session.toggle_inspection();
        assert_eq!(session.content_revision(), 0);
        session.accept_focused().unwrap();
        assert_eq!(session.content_revision(), 1);
        assert!(session.undo());
        assert_eq!(session.content_revision(), 2);
        session.toggle_inspection();
        assert!(session.accept_all());
        assert_eq!(session.content_revision(), 3);
        session.mark_committed();
        assert!(!session.accept_all());
        assert!(!session.undo());
        assert_eq!(session.content_revision(), 3);
    }

    #[test]
    fn accept_focused_and_undo() {
        let mut session = DiffSession::new("one\ntwo\nthree\n".into(), "one\nTWO\nthree\n".into());
        assert_eq!(session.diff().hunks.len(), 1);
        assert_eq!(session.accepted_hunks(), 0);
        assert_eq!(session.focus(), None);
        assert_eq!(session.accept_focused(), Ok(false));

        assert!(session.focus_relative(-1));
        assert_eq!(session.focus(), Some(0));

        assert_eq!(session.accept_focused(), Ok(true));
        assert_eq!(session.baseline(), "one\nTWO\nthree\n");
        assert!(session.diff().is_empty());
        assert_eq!(session.accepted_hunks(), 1);
        assert_eq!(session.focus(), None);
        assert!(session.can_undo());

        assert!(session.undo());
        assert_eq!(session.baseline(), "one\ntwo\nthree\n");
        assert_eq!(session.accepted_hunks(), 0);
        assert_eq!(session.focus(), Some(0));
        assert!(!session.can_undo());
    }

    #[test]
    fn inspection_follows_focus_and_stops_when_no_hunks_remain() {
        let mut session = DiffSession::new("a\nb\nc\n".into(), "A\nb\nC\n".into());
        assert!(!session.toggle_inspection());
        assert!(session.focus_relative(1));
        assert!(session.toggle_inspection());
        assert!(session.is_inspecting());

        // Accepting the last pending position falls back to the previous hunk.
        assert!(session.focus_relative(1));
        assert_eq!(session.focus(), Some(1));
        assert_eq!(session.accept_focused(), Ok(true));
        assert_eq!(session.focus(), Some(0));
        assert!(session.is_inspecting());
        assert!(session.undo());
        assert_eq!(session.focus(), Some(1));
        assert!(session.is_inspecting());

        assert_eq!(session.accept_focused(), Ok(true));
        session.mark_committed();
        assert!(session.is_inspecting());
        assert!(!session.can_undo());
        assert_eq!(session.accept_focused(), Ok(true));
        assert!(!session.is_inspecting());
        assert!(!session.toggle_inspection());
        assert!(session.undo());
        assert_eq!(session.focus(), Some(0));
        assert!(!session.is_inspecting());
    }

    #[test]
    fn accept_all_is_one_undo_step() {
        let live = "a\nB\nc\nd\nE\n";
        let mut session = DiffSession::new("a\nb\nc\nd\ne\n".into(), live.into());
        assert_eq!(session.diff().hunks.len(), 2);

        assert!(session.accept_all());
        assert_eq!(session.baseline(), live);
        assert!(session.diff().is_empty());
        assert_eq!(session.accepted_hunks(), 2);
        assert!(!session.accept_all());

        assert!(session.undo());
        assert_eq!(session.baseline(), "a\nb\nc\nd\ne\n");
        assert_eq!(session.accepted_hunks(), 0);
        assert!(!session.can_undo());
    }

    #[test]
    fn partial_commit_resets_progress_without_changing_diff_or_focus() {
        let mut session = DiffSession::new("a\nb\nc\nd\ne\n".into(), "A\nb\nC\nd\nE\n".into());
        session.focus_relative(1);
        session.accept_focused().unwrap();
        let baseline = session.baseline().to_owned();
        let hunks = session.diff().hunks.clone();
        let focus = session.focus();
        session.mark_committed();
        assert_eq!(session.baseline(), baseline);
        assert_eq!(session.diff().hunks, hunks);
        assert_eq!(session.focus(), focus);
        assert_eq!(session.accepted_hunks(), 0);
        assert!(!session.baseline_changed());
        assert!(!session.undo());

        session.accept_focused().unwrap();
        assert_eq!(session.accepted_hunks(), 1);
        assert!(session.baseline_changed());
        assert!(session.undo());
        assert_eq!(session.baseline(), baseline);
        assert!(!session.baseline_changed());
        assert_eq!(session.accepted_hunks(), 0);
        assert!(!session.undo());

        assert!(session.accept_all());
        assert_eq!(session.accepted_hunks(), 2);
        session.mark_committed();
        assert_eq!(session.baseline(), session.live());
        assert!(!session.baseline_changed());
        assert!(!session.can_undo());
    }

    #[test]
    fn focus_does_not_wrap() {
        let mut session = DiffSession::new("a\nb\nc\nd\ne\n".into(), "a\nB\nc\nd\nE\n".into());
        assert!(session.focus_relative(1));
        assert_eq!(session.focus(), Some(0));
        assert!(session.focus_relative(1));
        assert_eq!(session.focus(), Some(1));
        assert!(session.focus_relative(1));
        assert_eq!(session.focus(), Some(1));
        assert!(session.focus_relative(-1));
        assert_eq!(session.focus(), Some(0));
        assert!(session.focus_relative(-1));
        assert_eq!(session.focus(), Some(0));
    }

    #[test]
    fn focus_advances_to_next_hunk_after_accept() {
        let mut session = DiffSession::new("a\nb\nc\nd\ne\n".into(), "a\nB\nc\nd\nE\n".into());
        session.focus_relative(1);
        assert_eq!(session.accept_focused(), Ok(true));
        assert_eq!(session.diff().hunks.len(), 1);
        assert_eq!(session.focus(), Some(0));
        assert_eq!(session.accepted_hunks(), 1);
    }

    #[test]
    fn no_hunks_means_no_focus_movement() {
        let mut session = DiffSession::new("same\n".into(), "same\n".into());
        assert!(!session.focus_relative(1));
        assert_eq!(session.focus(), None);
    }

    #[test]
    fn empty_operations_change_nothing() {
        let mut session = DiffSession::new("same\n".into(), "same\n".into());
        assert_eq!(session.accept_focused(), Ok(false));
        assert!(!session.accept_all());
        assert!(!session.undo());
        assert_eq!(session.baseline(), "same\n");
        assert_eq!(session.accepted_hunks(), 0);
        assert!(!session.baseline_changed());
    }

    #[test]
    fn repeated_lines_keep_accept_progress_and_undo_exact() {
        let mut seed = 0xc0ff_ee12_3456_789au64;
        let mut next = |max: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize % max
        };
        let mut saw_regrouping = false;
        for _ in 0..2000 {
            let mut text = || {
                let mut result = String::new();
                for _ in 0..next(24) {
                    result.push_str(["a\n", "b\n", "\n", "a\r\n"][next(4)]);
                }
                result
            };
            let baseline = text();
            let live = text();
            let mut session = DiffSession::new(baseline.clone(), live.clone());
            let mut steps = 0;
            while !session.diff().is_empty() {
                assert!(steps < 100, "review failed to converge");
                session.focus = Some(next(session.diff.hunks.len()));
                let before = session.baseline().to_owned();
                let focus = session.focus();
                let pending = session.diff.hunks.len();
                let accepted = session.accepted_hunks();
                assert_eq!(session.accept_focused(), Ok(true));
                let after = session.baseline().to_owned();
                saw_regrouping |= session.diff.hunks.len() + 1 != pending;
                assert_eq!(session.accepted_hunks(), accepted + 1);
                assert_eq!(session.baseline_changed(), session.baseline() != baseline);

                // Accept All counts the current pending hunks and is one undo.
                let remaining = session.diff.hunks.len();
                if remaining > 0 {
                    assert!(session.accept_all());
                    assert_eq!(session.accepted_hunks(), accepted + 1 + remaining);
                    assert_eq!(session.baseline(), live);
                    assert!(session.undo());
                    assert_eq!(session.accepted_hunks(), accepted + 1);
                    assert_eq!(session.baseline(), after);
                }

                assert!(session.undo());
                assert_eq!(session.baseline(), before);
                assert_eq!(session.focus(), focus);
                assert_eq!(session.accepted_hunks(), accepted);
                assert_eq!(session.baseline_changed(), before != baseline);
                assert_eq!(session.accept_focused(), Ok(true));
                steps += 1;
            }
            assert_eq!(session.baseline(), live);
        }
        assert!(saw_regrouping, "fixtures must exercise hunk regrouping");
    }

    #[test]
    fn progress_survives_bounded_undo_history() {
        let baseline: String = (0..105).map(|i| format!("old {i}\nanchor {i}\n")).collect();
        let live = baseline.replace("old", "new");
        let mut session = DiffSession::new(baseline, live);
        session.focus_relative(1);
        for count in 1..=105 {
            assert_eq!(session.accept_focused(), Ok(true));
            assert_eq!(session.accepted_hunks(), count);
        }
        for count in (5..105).rev() {
            assert!(session.undo());
            assert_eq!(session.accepted_hunks(), count);
            assert_eq!(session.focus(), Some(0));
        }
        assert!(!session.undo());
        assert!(session.baseline_changed());
        assert_eq!(session.accepted_hunks(), 5);
    }
}
