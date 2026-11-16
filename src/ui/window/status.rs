//! Status bar bookkeeping of the main window.
//!
//! `StatusState` holds everything the editor status bar needs besides the
//! widgets: the measured buffer text, the idle-refresh flag, the time of the
//! last user edit and the timer that keeps the "Last edit" text current. It
//! has no widget dependencies apart from the timer's `SourceId`, so its logic
//! is unit-tested without a display. The `AppWindow` methods below connect it
//! to the buffer and the status bar.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use adw::prelude::*;
use gtk::glib;

use crate::core::elapsed;
use crate::core::textstats::{self, TextStats};

use super::AppWindow;

/// The buffer text last measured, with its whole-text stats.
struct Measured {
    text: glib::GString,
    stats: TextStats,
}

#[derive(Default)]
pub(super) struct StatusState {
    /// Dropped whenever the buffer changes, so cursor and selection moves
    /// reuse it instead of copying and rescanning the whole buffer.
    measured: RefCell<Option<Measured>>,
    /// When the user last edited the buffer since loading; `None` until the
    /// first edit.
    last_edit: Cell<Option<Instant>>,
    refresh_pending: Cell<bool>,
    /// The one-shot timer that refreshes the elapsed time.
    last_edit_timer: RefCell<Option<glib::SourceId>>,
}

impl StatusState {
    /// The buffer changed: forget the measured text.
    pub fn invalidate_text(&self) {
        self.measured.replace(None);
    }

    /// Run `use_measured` on the buffer text and its stats. The text is read
    /// with `read_text` only when nothing is cached.
    pub fn measure<R>(
        &self,
        read_text: impl FnOnce() -> glib::GString,
        use_measured: impl FnOnce(&str, &TextStats) -> R,
    ) -> R {
        let mut cache = self.measured.borrow_mut();
        let measured = cache.get_or_insert_with(|| {
            let text = read_text();
            let stats = textstats::text_stats(text.as_str());
            Measured { text, stats }
        });
        use_measured(measured.text.as_str(), &measured.stats)
    }

    /// Ask for an idle refresh. True when the caller must schedule it, false
    /// when one is already pending.
    pub fn begin_refresh(&self) -> bool {
        !self.refresh_pending.replace(true)
    }

    pub fn end_refresh(&self) {
        self.refresh_pending.set(false);
    }

    pub fn note_edit(&self, now: Instant) {
        self.last_edit.set(Some(now));
    }

    /// Forget the last edit and stop the refresh timer.
    pub fn clear_edit(&self) {
        self.last_edit.set(None);
        self.cancel_timer();
    }

    #[cfg(test)]
    pub fn has_edit(&self) -> bool {
        self.last_edit.get().is_some()
    }

    fn elapsed_secs(&self, now: Instant) -> Option<u64> {
        self.last_edit
            .get()
            .map(|at| now.saturating_duration_since(at).as_secs())
    }

    /// `31 seconds ago`, or `None` before the first edit.
    pub fn last_edit_text(&self, now: Instant) -> Option<String> {
        self.elapsed_secs(now).map(elapsed::format_elapsed)
    }

    /// Seconds until the "Last edit" text can next change, or `None` when
    /// there is no edit and so nothing to refresh.
    pub fn next_tick_secs(&self, now: Instant) -> Option<u32> {
        self.elapsed_secs(now).map(elapsed::next_refresh_secs)
    }

    /// Remember the pending timer, removing any earlier one.
    pub fn set_timer(&self, id: glib::SourceId) {
        if let Some(old) = self.last_edit_timer.replace(Some(id)) {
            old.remove();
        }
    }

    /// The timer has fired; forget its ID before scheduling another one.
    pub fn timer_fired(&self) {
        self.last_edit_timer.take();
    }

    fn cancel_timer(&self) {
        if let Some(id) = self.last_edit_timer.take() {
            id.remove();
        }
    }
}

impl AppWindow {
    /// Background windows have no periodic elapsed-time wakeups.
    pub(super) fn connect_status_activity(this: &Rc<Self>) {
        let weak = Rc::downgrade(this);
        this.window.connect_is_active_notify(move |window| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if window.is_active() {
                this.resume_last_edit();
            } else {
                this.status_state.cancel_timer();
            }
        });
    }

    fn resume_last_edit(self: &Rc<Self>) {
        self.update_last_edit();
        if let Some(next) = self.status_state.next_tick_secs(Instant::now()) {
            self.schedule_last_edit_tick(next);
        }
    }

    /// Refresh the editor status bar once, on idle, however many signals
    /// asked for it since the last refresh.
    pub(super) fn schedule_status_update(self: &Rc<Self>) {
        if !self.status_state.begin_refresh() {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.status_state.end_refresh();
            this.update_status();
        });
    }

    /// Refresh the editor status bar from the buffer. The buffer text and its
    /// whole-text stats are measured only when the buffer has changed since
    /// the last refresh; cursor and selection moves reuse the cached text.
    pub(super) fn update_status(&self) {
        let offset = self.editor_buffer.cursor_position().max(0) as usize;
        let selected = self.selection_offsets();
        let (stats, cursor, selection) = self.status_state.measure(
            || self.buffer_text(),
            |text, stats| {
                (
                    *stats,
                    textstats::cursor_position(text, offset),
                    selected.and_then(|(start, end)| textstats::selection_stats(text, start, end)),
                )
            },
        );
        self.status.set_stats(&stats);
        self.status.set_cursor(&cursor, selection.as_ref());
        self.status.set_mode(self.edit_mode());
        self.update_last_edit();
    }

    /// Refresh the "Last edit" field from the monotonic elapsed time.
    pub(super) fn update_last_edit(&self) {
        let text = self.status_state.last_edit_text(Instant::now());
        self.status.set_last_edit(text.as_deref());
    }

    /// The user edited the buffer: restart the elapsed time and its timer.
    pub(super) fn note_edit(self: &Rc<Self>) {
        self.status_state.note_edit(Instant::now());
        // A background window has no timer; it refreshes on focus.
        if self.window.is_active() {
            self.schedule_last_edit_tick(1);
        }
    }

    /// A document was loaded, which is not an edit: hide the field.
    pub(super) fn clear_edit(&self) {
        self.status_state.clear_edit();
        self.update_last_edit();
    }

    /// Refresh the "Last edit" field after `secs` seconds, then schedule the
    /// next refresh for the moment its text can change again (every second at
    /// first, then once a minute, then once an hour). The timer holds the
    /// window weakly, so it does nothing once the window has been freed.
    fn schedule_last_edit_tick(self: &Rc<Self>, secs: u32) {
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local_once(secs, move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.status_state.timer_fired();
            this.update_last_edit();
            if let Some(next) = this.status_state.next_tick_secs(Instant::now()) {
                this.schedule_last_edit_tick(next);
            }
        });
        self.status_state.set_timer(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn no_edit_means_no_text_and_no_tick() {
        let state = StatusState::default();
        let now = Instant::now();
        assert_eq!(state.last_edit_text(now), None);
        assert_eq!(state.next_tick_secs(now), None);
        assert!(!state.has_edit());
    }

    #[test]
    fn last_edit_text_follows_the_elapsed_time() {
        let state = StatusState::default();
        let start = Instant::now();
        state.note_edit(start);
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(state.last_edit_text(start).as_deref(), Some("just now"));
        assert_eq!(
            state
                .last_edit_text(start + Duration::from_millis(999))
                .as_deref(),
            Some("just now")
        );
        assert_eq!(state.last_edit_text(at(1)).as_deref(), Some("1 second ago"));
        assert_eq!(
            state.last_edit_text(at(4)).as_deref(),
            Some("4 seconds ago")
        );
        assert_eq!(
            state.last_edit_text(at(300)).as_deref(),
            Some("5 minutes ago")
        );
        assert_eq!(state.next_tick_secs(at(4)), Some(1));
        assert_eq!(state.next_tick_secs(at(61)), Some(59));
    }

    #[test]
    fn a_new_edit_restarts_the_clock_and_clearing_removes_it() {
        let state = StatusState::default();
        let start = Instant::now();
        state.note_edit(start);
        let later = start + Duration::from_secs(120);
        state.note_edit(later);
        assert_eq!(state.last_edit_text(later).as_deref(), Some("just now"));
        state.clear_edit();
        assert!(!state.has_edit());
        assert_eq!(state.last_edit_text(later), None);
    }

    #[test]
    fn the_buffer_text_is_read_once_until_invalidated() {
        let state = StatusState::default();
        let reads = Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            glib::GString::from("ab\ncd")
        };
        assert_eq!(state.measure(read, |_, stats| stats.lines), 2);
        assert_eq!(state.measure(read, |text, _| text.len()), 5);
        assert_eq!(reads.get(), 1);
        state.invalidate_text();
        state.measure(read, |_, _| ());
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn refreshes_are_coalesced() {
        let state = StatusState::default();
        assert!(state.begin_refresh());
        assert!(!state.begin_refresh());
        state.end_refresh();
        assert!(state.begin_refresh());
    }
}
