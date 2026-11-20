//! Diff Mode wiring of the main window: starting a session, applying session
//! results to the snapshot store and the UI, the Diff Mode keys, Create
//! Baseline, inspection and Confirm.
//!
//! The session logic itself is `core::session`; presentation is `diff_page`.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::core::pending::{ExternalCheck, PendingAction};
use crate::core::session::DiffSession;
use crate::ui::dialogs::{self, Response};
use crate::ui::diff_toolbar::DiffToolbarState;
use crate::ui::toast::ToastKind;

use super::{AppWindow, NO_CHANGES_LABEL, add_action, connect_button};

/// What should happen after the user confirms discarding accepted changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AfterDiscardReview {
    ReturnToEditor,
    CloseWindow,
    Quit,
}

fn discard_review_body(target: AfterDiscardReview) -> String {
    let action = match target {
        AfterDiscardReview::ReturnToEditor => "return to editor",
        AfterDiscardReview::CloseWindow => "close this window",
        AfterDiscardReview::Quit => "continue quitting",
    };
    format!("Accepted changes aren't confirmed yet. Discard them and {action}?")
}

fn discard_review_button(target: AfterDiscardReview) -> &'static str {
    match target {
        AfterDiscardReview::ReturnToEditor => "Discard and End Review",
        AfterDiscardReview::CloseWindow => "Discard and Close",
        AfterDiscardReview::Quit => "Discard and Quit",
    }
}

impl AppWindow {
    /// Connect the Diff Mode buttons, keys and actions. Called once from
    /// `connect_signals`; every handler holds the window weakly.
    pub(super) fn connect_diff_actions(this: &Rc<Self>, diff_button: &gtk::Button) {
        connect_button(diff_button, this, |this| this.on_diff_button_clicked());
        connect_button(&this.diff.exit_button, this, |this| {
            this.on_diff_exit_clicked()
        });
        connect_button(&this.diff.inspect_button, this, |this| {
            this.toggle_inspection();
        });
        connect_button(&this.diff.accept_button, this, |this| this.accept_focused());
        connect_button(&this.diff.accept_all_button, this, |this| this.accept_all());
        connect_button(&this.diff.undo_button, this, |this| this.undo_accept());
        connect_button(&this.diff.previous_button, this, |this| {
            this.focus_relative(-1)
        });
        connect_button(&this.diff.next_button, this, |this| this.focus_relative(1));

        add_action(&this.window, this, "enter-diff", |this| this.enter_diff());

        // Capture the review keys before read-only text views can consume them.
        let keys = gtk::EventControllerKey::new();
        keys.set_name(Some("notediff-diff-keys"));
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(this);
            keys.connect_key_pressed(move |_, keyval, _, modifiers| {
                let Some(this) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                // Dialogs and popover menus own their own keyboard navigation.
                if !this.diff_active()
                    || this.window.visible_dialog().is_some()
                    || gtk::prelude::GtkWindowExt::focus(&this.window)
                        .is_some_and(|focus| focus.ancestor(gtk::Popover::static_type()).is_some())
                {
                    return glib::Propagation::Proceed;
                }

                // Review keys are plain shortcuts. Leave modified combinations
                // to the focused pane and the application accelerators. Caps
                // Lock and Num Lock are not significant modifiers.
                if modifiers.intersects(gtk::accelerator_get_default_mod_mask()) {
                    return glib::Propagation::Proceed;
                }

                match keyval {
                    gdk::Key::Up => this.focus_relative(-1),
                    gdk::Key::Down => this.focus_relative(1),
                    gdk::Key::Right => this.accept_focused(),
                    gdk::Key::Left => this.undo_accept(),
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        if !this.toggle_inspection() {
                            return glib::Propagation::Proceed;
                        }
                    }
                    gdk::Key::Escape => {
                        if this.is_inspecting() {
                            this.toggle_inspection();
                        } else {
                            this.confirm_discard_review(AfterDiscardReview::ReturnToEditor);
                        }
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
        }
        this.window.add_controller(keys);
    }

    pub(super) fn enter_diff(self: &Rc<Self>) {
        if self.diff_active() {
            return;
        }
        // An untitled document has no path to key a baseline on. The header
        // button is disabled, and the menu item and shortcuts do nothing.
        if self.doc.borrow().path().is_none() {
            return;
        }

        // Saving is deferred until the user tries to accept something (see
        // `require_saved`), so unsaved edits can be reviewed.
        self.check_external(ExternalCheck::EnterDiff);
    }

    /// Show `text` on the header button for one second, then let
    /// `update_diff_button` restore the label that fits the current state. A
    /// generation counter stops an older timer from ending a newer flash.
    fn flash_button_label(self: &Rc<Self>, text: &str) {
        let generation = self.flash.begin();
        self.diff_button.set_label(text);

        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(1), move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.flash.end(generation) {
                this.update_diff_button();
            }
        });
    }

    /// The header button saves an untitled document, creates a baseline when
    /// the file has none, and otherwise enters Diff Mode.
    fn on_diff_button_clicked(self: &Rc<Self>) {
        if self.doc.borrow().path().is_none() {
            self.save_as_then(None);
            return;
        }
        if self.baseline_exists() {
            self.enter_diff();
        } else {
            self.create_baseline();
        }
    }

    /// Create the first baseline from the saved document. A dirty document
    /// gets the usual Cancel / Save prompt. The user stays in the editor.
    pub(super) fn create_baseline(self: &Rc<Self>) {
        self.require_saved(PendingAction::CreateBaseline);
    }

    pub(super) fn write_baseline(&self) {
        let text = self.doc.borrow().text().to_string();
        if self.store_baseline("Could not create baseline", &text) {
            self.update_diff_button();
        }
    }

    /// Open the diff view. The live side is the current editor text, saved or
    /// not. If the baseline already matches the live text, flash "No Changes"
    /// instead of entering Diff Mode. Both panes start scrolled to zero and no
    /// hunk is focused; the first navigation input focuses the first hunk.
    pub(super) fn start_diff_session(self: &Rc<Self>) {
        let Some(path) = self.path_for_snapshot() else {
            return;
        };
        let live = self.current_text();

        match self.store.read_for_canonical(&path) {
            Ok(None) => {
                // The baseline vanished (or never existed): the header button
                // offers Create Baseline instead.
                self.update_diff_button();
                self.toast(ToastKind::Info, "Create a baseline first");
            }
            Ok(Some(baseline)) => {
                if baseline == live {
                    self.flash_button_label(NO_CHANGES_LABEL);
                    return;
                }
                *self.diff_state.borrow_mut() = Some(DiffSession::new(baseline, live));
                self.diff.set_document_path(Some(&path));
                self.diff.begin_review();
                self.stack.set_visible_child_name("diff");
                self.render_diff();
                self.diff.reset_scroll();
                self.update_diff_button();
            }
            Err(err) => {
                self.show_error("Could not read baseline", &err.to_string());
            }
        }
    }

    /// Re-render both panes. The focused hunk, if any, is scrolled into view.
    fn render_diff(&self) {
        let state = self.diff_state.borrow();
        let Some(session) = state.as_ref() else {
            return;
        };
        self.update_diff_timestamps();
        self.diff.render(session);
        self.diff.active_left_view().grab_focus();
    }

    /// Refresh the absolute times in both pane headers. The live side uses
    /// the last in-memory edit, or the file mtime before the first edit; the
    /// baseline side uses the snapshot file's mtime.
    pub(super) fn update_diff_timestamps(&self) {
        let edited = self.status_state.last_edit_wall();
        let left = edited.or_else(|| self.doc.borrow().disk_mtime());
        self.diff.set_document_time(left, edited.is_some());

        let right = self
            .path_for_snapshot()
            .and_then(|path| self.store.mtime_for_canonical(&path));
        self.diff.set_baseline_time(right);
    }

    fn is_inspecting(&self) -> bool {
        self.diff_state
            .borrow()
            .as_ref()
            .is_some_and(DiffSession::is_inspecting)
    }

    fn toggle_inspection(&self) -> bool {
        self.change_session(DiffSession::toggle_inspection)
    }

    pub(super) fn has_accepted_diff(&self) -> bool {
        self.diff_state
            .borrow()
            .as_ref()
            .is_some_and(DiffSession::baseline_changed)
    }

    /// Confirm discarding accepted changes before leaving, closing or quitting.
    /// A session with no accepted baseline change runs the target immediately.
    pub(super) fn confirm_discard_review(self: &Rc<Self>, target: AfterDiscardReview) {
        let changed = self
            .diff_state
            .borrow()
            .as_ref()
            .is_some_and(DiffSession::baseline_changed);
        if !changed {
            self.after_discard_review(target);
            return;
        }

        let body = discard_review_body(target);
        let this = self.clone();
        dialogs::ask(
            &self.window,
            "Discard accepted changes?",
            &body,
            &[
                Response::new("cancel", "Cancel"),
                Response::destructive("discard", discard_review_button(target)),
            ],
            "cancel",
            "cancel",
            move |response| {
                if response == "discard" {
                    this.after_discard_review(target);
                }
            },
        );
    }

    fn after_discard_review(self: &Rc<Self>, target: AfterDiscardReview) {
        match target {
            AfterDiscardReview::ReturnToEditor => self.close_diff_view(),
            AfterDiscardReview::CloseWindow => {
                self.flags.allow_close.set(true);
                self.window.close();
            }
            AfterDiscardReview::Quit => {
                self.flags.allow_close.set(true);
                let app = self
                    .window
                    .application()
                    .and_then(|app| app.downcast::<adw::Application>().ok());
                self.window.close();
                if let Some(app) = app {
                    super::menus::schedule_quit(app);
                }
            }
        }
    }

    /// Persist `text` as the baseline for the open document. Failures are
    /// reported to the user; the caller must not change session state when
    /// this returns false.
    fn store_baseline(&self, heading: &str, text: &str) -> bool {
        let Some(path) = self.path_for_snapshot() else {
            self.show_error(heading, "The document has no saved path.");
            return false;
        };
        match self.store.write_for_canonical(&path, text) {
            Ok(()) => {
                self.invalidate_baseline_cache();
                true
            }
            Err(err) => {
                self.show_error(heading, &err.to_string());
                false
            }
        }
    }

    /// Commit the session's accepted baseline to disk. Returns false on
    /// failure, leaving the session open so the user can retry or undo.
    fn commit_accepted_baseline(&self) -> bool {
        let baseline = {
            let state = self.diff_state.borrow();
            let Some(session) = state.as_ref() else {
                return true;
            };
            if !session.baseline_changed() {
                return true;
            }
            session.baseline().to_owned()
        };
        self.store_baseline("Could not write baseline", &baseline)
    }

    /// Run `change` on the Diff Mode session and re-render both panes when it
    /// reports that the session changed. Does nothing outside Diff Mode.
    fn change_session(&self, change: impl FnOnce(&mut DiffSession) -> bool) -> bool {
        let changed = self.diff_state.borrow_mut().as_mut().is_some_and(change);
        if changed {
            self.render_diff();
        }
        changed
    }

    pub(super) fn accept_focused(&self) {
        let mut error = None;
        self.change_session(|session| match session.accept_focused() {
            Ok(changed) => changed,
            Err(err) => {
                error = Some(err);
                false
            }
        });
        if let Some(err) = error {
            self.show_error("Could not accept change", &err);
        }
    }

    fn accept_all(&self) {
        self.change_session(DiffSession::accept_all);
    }

    pub(super) fn undo_accept(&self) {
        self.change_session(DiffSession::undo);
    }

    pub(super) fn focus_relative(self: &Rc<Self>, direction: i32) {
        let previous = {
            let mut state = self.diff_state.borrow_mut();
            let Some(session) = state.as_mut() else {
                return;
            };
            let previous = (session.focus(), session.inline_focus());
            if !session.focus_relative(direction) {
                return;
            }
            previous
        };

        let state = self.diff_state.borrow();
        let session = state.as_ref().unwrap();
        if session.is_inspecting() {
            if (session.focus(), session.inline_focus()) != previous {
                self.diff.render(session);
            }
        } else {
            let toolbar = DiffToolbarState::for_session(session);
            self.diff
                .set_focus(session.diff(), session.focus(), toolbar.can_accept);
        }
    }

    /// The toolbar's idle inspection button is Back, not Close. Keep this
    /// navigation separate from callers that explicitly leave Diff Mode.
    fn on_diff_exit_clicked(self: &Rc<Self>) {
        if self.is_inspecting() && !self.has_accepted_diff() {
            self.toggle_inspection();
        } else {
            self.exit_diff();
        }
    }

    /// Close an unchanged session, or commit pending accepts. A partial commit
    /// stays in Diff Mode; a complete one closes it. Save cancellation keeps
    /// all session state intact.
    pub(super) fn exit_diff(self: &Rc<Self>) {
        if !self.has_accepted_diff() {
            self.close_diff_view();
            return;
        }

        self.require_saved(PendingAction::ConfirmBaseline);
    }

    pub(super) fn confirm_diff(self: &Rc<Self>) {
        if !self.commit_accepted_baseline() {
            return;
        }
        let complete = self
            .diff_state
            .borrow()
            .as_ref()
            .is_none_or(|session| session.diff().is_empty());
        if complete {
            self.close_diff_view();
        } else {
            {
                let mut state = self.diff_state.borrow_mut();
                let session = state.as_mut().unwrap();
                session.mark_committed();
                self.diff
                    .update_toolbar(DiffToolbarState::for_session(session));
                self.diff.active_left_view().grab_focus();
            }
            self.update_diff_timestamps();
        }
    }

    fn close_diff_view(self: &Rc<Self>) {
        *self.diff_state.borrow_mut() = None;
        self.stack.set_visible_child_name("editor");
        self.update_diff_button();
        self.editor_view.grab_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discard_review_bodies_name_the_target_action() {
        for (target, action) in [
            (AfterDiscardReview::ReturnToEditor, "return to editor"),
            (AfterDiscardReview::CloseWindow, "close this window"),
            (AfterDiscardReview::Quit, "continue quitting"),
        ] {
            assert_eq!(
                discard_review_body(target),
                format!("Accepted changes aren't confirmed yet. Discard them and {action}?")
            );
        }
    }

    #[test]
    fn discard_review_buttons_are_specific() {
        assert_eq!(
            discard_review_button(AfterDiscardReview::ReturnToEditor),
            "Discard and End Review"
        );
        assert_eq!(
            discard_review_button(AfterDiscardReview::CloseWindow),
            "Discard and Close"
        );
        assert_eq!(
            discard_review_button(AfterDiscardReview::Quit),
            "Discard and Quit"
        );
    }
}
