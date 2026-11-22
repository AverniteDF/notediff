//! Baseline Snapshots tab: orphan cleanup and baseline linking.
//!
//! The dialog is told which document the window has open (`LinkTarget`). When
//! that document has a path and no baseline, orphaned baselines can be moved to
//! it and baselines of other existing files can be copied to it. Linking never
//! replaces an existing baseline. Link and Copy act immediately; deletion is confirmed.
//!
//! Widget closures hold only a weak reference to `State`, and `State` holds the
//! dialog weakly, so nothing keeps the dialog alive after it closes.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::SystemTime;

use adw::prelude::*;
use gtk::glib;

use super::cleanup_dialog::{
    CleanupPage, Lead, build_row, connect_weak, empty_label, note_label, section_label, show_error,
};
use crate::core::relink::{self, Candidate};
use crate::core::snapshot_store::{LinkMode, LivePathState, SnapshotEntry, SnapshotStore};
use crate::ui::toast::{self, ToastKind};

/// The document that linking would give a baseline to.
pub struct LinkTarget {
    /// Canonical path of the open document.
    pub path: PathBuf,
    /// The document's current editor text, used to rank candidates.
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LinkState {
    /// No document path (untitled document, or no target at all).
    NoPath,
    /// The document already has a baseline; linking never replaces it.
    HasBaseline,
    /// Linking is possible.
    Ready,
}

struct State {
    dialog: glib::WeakRef<adw::Dialog>,
    store: SnapshotStore,
    target: Option<LinkTarget>,
    toasts: adw::ToastOverlay,
    page: CleanupPage,
    /// Lower-cased live path and row, for the copy-section filter.
    copy_rows: RefCell<Vec<(String, gtk::Widget)>>,
    filter: RefCell<String>,
    copy_expanded: Cell<bool>,
}

/// The tab widget and state, kept alive by the dialog shell.
pub struct SnapshotsTab {
    pub root: gtk::Box,
    _state: Rc<State>,
}

impl SnapshotsTab {
    pub fn new(
        dialog: &adw::Dialog,
        toasts: &adw::ToastOverlay,
        store: SnapshotStore,
        target: Option<LinkTarget>,
    ) -> Self {
        let page = CleanupPage::new(
            "A file that is renamed or moved outside of notediff will dissociate from its \
             baseline. Use this tool to re-link or delete orphaned snapshots.",
        );
        page.connect_close(dialog);
        let root = page.root.clone();
        let delete_button = page.delete_button.clone();
        let state = Rc::new(State {
            dialog: dialog.downgrade(),
            store,
            target,
            toasts: toasts.clone(),
            page,
            copy_rows: RefCell::new(Vec::new()),
            filter: RefCell::new(String::new()),
            copy_expanded: Cell::new(false),
        });
        state.rebuild();
        connect_weak(&delete_button, &state, |state| state.confirm_delete());
        Self {
            root,
            _state: state,
        }
    }
}

impl State {
    fn link_state(&self) -> LinkState {
        match &self.target {
            None => LinkState::NoPath,
            Some(target) if self.store.has_for_canonical(&target.path) => LinkState::HasBaseline,
            Some(_) => LinkState::Ready,
        }
    }

    /// Snapshots in the given state, ranked when linking is possible.
    fn candidates(
        &self,
        entries: &[SnapshotEntry],
        wanted: LivePathState,
        link: LinkState,
    ) -> Vec<Candidate> {
        let pool: Vec<SnapshotEntry> = entries
            .iter()
            .filter(|entry| entry.state == wanted)
            .cloned()
            .collect();
        match (&self.target, link) {
            (Some(target), LinkState::Ready) => {
                relink::rank(&pool, &target.path, &target.text, relink::read_limited)
            }
            _ => pool.into_iter().map(Candidate::unranked).collect(),
        }
    }

    fn rebuild(self: &Rc<Self>) {
        self.page.clear();
        self.copy_rows.borrow_mut().clear();
        let list = &self.page.list;

        let entries = match self.store.scan() {
            Ok(entries) => entries,
            Err(err) => {
                let label = note_label(&format!("Could not read the snapshot store: {err}"));
                label.remove_css_class("dim-label");
                list.append(&label);
                self.page.refresh_delete_button();
                return;
            }
        };

        let link = self.link_state();
        list.append(&self.context_banner(link));

        let orphans = self.candidates(&entries, LivePathState::Orphan, link);
        let existing = if link == LinkState::Ready {
            self.candidates(&entries, LivePathState::Present, link)
        } else {
            Vec::new()
        };
        let unavailable: Vec<&SnapshotEntry> = entries
            .iter()
            .filter(|entry| entry.state == LivePathState::Unavailable)
            .collect();

        if orphans.is_empty() && existing.is_empty() && unavailable.is_empty() {
            list.append(&empty_label(if link == LinkState::Ready {
                "No other snapshots exist to link."
            } else {
                "No orphaned snapshots."
            }));
        }

        if !orphans.is_empty() {
            list.append(&section_label(&format!(
                "Orphaned baselines ({})",
                orphans.len()
            )));
            for candidate in &orphans {
                let check = self.page.new_check(candidate.entry.snapshot_path.clone());

                let action = (link == LinkState::Ready)
                    .then(|| self.link_button(&candidate.entry, LinkMode::Move));
                let row = build_row(
                    Lead::Check(check),
                    &candidate.entry.live_path.display().to_string(),
                    &format!("Last modified {}", format_time(candidate.entry.modified)),
                    &badges(candidate),
                    action.as_ref(),
                );
                list.append(&row);
            }
        }

        if !existing.is_empty() {
            list.append(&self.copy_section(&existing));
        }

        if !unavailable.is_empty() {
            list.append(&section_label(&format!(
                "Location unavailable ({})",
                unavailable.len()
            )));

            list.append(&note_label(
                "These baselines refer to live files whose locations are currently unavailable. \
                 They cannot be confirmed as orphans until those locations are accessible again.",
            ));

            for entry in unavailable {
                let row = build_row(
                    Lead::Warning,
                    &entry.live_path.display().to_string(),
                    &format!("Last modified {}", format_time(entry.modified)),
                    &[],
                    None,
                );
                list.append(&row);
            }
        }

        self.page.refresh_delete_button();
    }

    /// The banner that says whether the open document can receive a baseline.
    fn context_banner(&self, link: LinkState) -> gtk::Box {
        let (heading, note) = match link {
            LinkState::NoPath => (
                "Linking unavailable",
                "The current document has no file path yet. Save it first, then a baseline \
                 can be associated.",
            ),
            LinkState::HasBaseline => (
                "Current document",
                "This document already has a baseline. Linking only applies to documents without one.",
            ),
            LinkState::Ready => (
                "Current document: no baseline",
                "Link an orphaned baseline below, or copy the baseline of another file.",
            ),
        };

        let inner = gtk::Box::new(gtk::Orientation::Vertical, 4);
        inner.set_margin_start(12);
        inner.set_margin_end(12);
        inner.set_margin_top(10);
        inner.set_margin_bottom(10);

        let heading_label = gtk::Label::new(Some(heading));
        heading_label.add_css_class("heading");
        heading_label.set_xalign(0.0);
        inner.append(&heading_label);

        if let Some(target) = &self.target {
            let path = target.path.display().to_string();
            let path_label = gtk::Label::new(Some(&path));
            path_label.set_selectable(true);
            path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            path_label.set_xalign(0.0);
            path_label.add_css_class("monospace");
            path_label.set_tooltip_text(Some(&path));
            inner.append(&path_label);
        }

        inner.append(&note_label(note));

        let banner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        banner.add_css_class("card");
        banner.set_margin_start(2);
        banner.set_margin_end(2);
        banner.append(&inner);
        banner
    }

    /// The collapsed "copy from another file" section, with a path filter.
    fn copy_section(self: &Rc<Self>, existing: &[Candidate]) -> gtk::Expander {
        let expander = gtk::Expander::new(Some(&format!(
            "Copy a baseline from another file ({})",
            existing.len()
        )));
        expander.set_margin_top(8);
        expander.set_expanded(self.copy_expanded.get());
        {
            let weak = Rc::downgrade(self);
            expander.connect_expanded_notify(move |expander| {
                if let Some(state) = weak.upgrade() {
                    state.copy_expanded.set(expander.is_expanded());
                }
            });
        }

        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.set_margin_top(8);

        body.append(&note_label(
            "Creates a separate copy of another file's baseline and links it to the current document.",
        ));

        let filter = gtk::SearchEntry::new();
        filter.set_placeholder_text(Some("Filter by path"));
        filter.set_text(&self.filter.borrow());
        {
            let weak = Rc::downgrade(self);
            filter.connect_search_changed(move |entry| {
                if let Some(state) = weak.upgrade() {
                    state.filter.replace(entry.text().to_string());
                    state.apply_filter();
                }
            });
        }
        body.append(&filter);

        for candidate in existing {
            let entry = &candidate.entry;
            let path = entry.live_path.display().to_string();
            let button = self.link_button(entry, LinkMode::Copy);
            let row = build_row(
                Lead::Plain,
                &path,
                &format!("Last modified {}", format_time(entry.modified)),
                &badges(candidate),
                Some(&button),
            );
            self.copy_rows
                .borrow_mut()
                .push((path.to_lowercase(), row.clone().upcast()));
            body.append(&row);
        }

        expander.set_child(Some(&body));
        self.apply_filter();
        expander
    }

    fn apply_filter(&self) {
        let needle = self.filter.borrow().to_lowercase();
        for (path, row) in self.copy_rows.borrow().iter() {
            row.set_visible(needle.is_empty() || path.contains(&needle));
        }
    }

    fn link_button(self: &Rc<Self>, entry: &SnapshotEntry, mode: LinkMode) -> gtk::Button {
        let label = match mode {
            LinkMode::Move => "Link",
            LinkMode::Copy => "Copy",
        };
        let button = gtk::Button::with_label(label);
        button.set_valign(gtk::Align::Center);

        let weak = Rc::downgrade(self);
        let entry = entry.clone();
        button.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.perform_link(&entry, mode);
            }
        });
        button
    }

    fn perform_link(self: &Rc<Self>, entry: &SnapshotEntry, mode: LinkMode) {
        let Some(target) = &self.target else {
            return;
        };
        let result = self.store.link(&entry.snapshot_path, &target.path, mode);
        // Rebuild either way: even a failed Move may have written the new
        // baseline before failing to remove the old one.
        self.rebuild();
        match result {
            Ok(()) => {
                let verb = match mode {
                    LinkMode::Move => "moved",
                    LinkMode::Copy => "copied",
                };
                let message = format!("Baseline {verb} to the current document");
                self.toasts
                    .add_toast(toast::new(ToastKind::Success, &message));
            }
            Err(err) => show_error(&self.dialog, "Could not link baseline", &err.to_string()),
        }
    }

    fn confirm_delete(self: &Rc<Self>) {
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let this = self.clone();
        self.page.confirm_removal(
            &dialog,
            "Delete",
            "snapshot",
            "Only the baselines will be removed. Live documents remain untouched. \
             This cannot be undone.",
            "Delete",
            move |selected| {
                let mut failures = Vec::new();
                for path in &selected {
                    if let Err(err) = this.store.delete_snapshot_path(path) {
                        failures.push(format!("{}: {err}", path.display()));
                    }
                }

                this.rebuild();

                if !failures.is_empty() {
                    show_error(
                        &this.dialog,
                        "Some snapshots could not be deleted",
                        &failures.join("\n"),
                    );
                }
            },
        );
    }
}

/// Badge texts (with a style class) for a ranked candidate.
fn badges(candidate: &Candidate) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    if candidate.same_name {
        out.push(("Same file name".to_string(), "success"));
    }
    if let Some(similarity) = candidate.similarity {
        out.push((
            format!("{}% similar", (similarity * 100.0).floor() as u32),
            "dim-label",
        ));
    }
    out
}

/// Part of the single-threaded GTK smoke test; button clicks must perform the
/// operation immediately, without waiting for a confirmation response.
#[cfg(test)]
pub(crate) fn link_buttons_smoke(store: &SnapshotStore, dir: &std::path::Path) {
    for (mode, name) in [(LinkMode::Move, "move"), (LinkMode::Copy, "copy")] {
        let source = dir.join(format!("snapshot-{name}-source.md"));
        let target = dir.join(format!("snapshot-{name}-target.md"));
        std::fs::write(&target, "live text\n").unwrap();
        let target = target.canonicalize().unwrap();
        store
            .write_for_canonical(&source, "baseline text\n")
            .unwrap();
        let entry = store
            .scan()
            .unwrap()
            .into_iter()
            .find(|entry| entry.live_path == source)
            .unwrap();
        let dialog = adw::Dialog::new();
        let state = Rc::new(State {
            dialog: dialog.downgrade(),
            store: store.clone(),
            target: Some(LinkTarget {
                path: target.clone(),
                text: "live text\n".into(),
            }),
            toasts: adw::ToastOverlay::new(),
            page: CleanupPage::new(""),
            copy_rows: RefCell::new(Vec::new()),
            filter: RefCell::new(String::new()),
            copy_expanded: Cell::new(false),
        });
        let button = state.link_button(&entry, mode);
        assert!(button.tooltip_text().is_none());
        button.emit_clicked();
        assert_eq!(
            store.read_for_canonical(&target).unwrap().as_deref(),
            Some("baseline text\n")
        );
        assert_eq!(store.has_for_canonical(&source), mode == LinkMode::Copy);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "live text\n");
    }
}

fn format_time(time: Option<SystemTime>) -> String {
    time.and_then(crate::ui::format_local_time)
        .unwrap_or_else(|| "unknown".to_string())
}
