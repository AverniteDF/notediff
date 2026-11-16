//! Shared Manage App page scaffolding, rows, confirmation and weak helpers.
//! Tabs own their pages; widget callbacks hold state weakly and state holds the
//! dialog weakly, so closing the dialog releases its state.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::ui::dialogs::{self, Response};

pub enum Lead {
    Check(gtk::CheckButton),
    Warning,
    Plain,
}

type Checks = RefCell<Vec<(PathBuf, gtk::CheckButton)>>;

/// Explanation, scrolling list, counting delete button and Close.
pub struct CleanupPage {
    pub root: gtk::Box,
    pub list: gtk::Box,
    pub delete_button: gtk::Button,
    close_button: gtk::Button,
    checks: Rc<Checks>,
}

impl CleanupPage {
    pub fn new(explanation: &str) -> Self {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
        list.set_margin_start(6);
        list.set_margin_end(6);
        list.set_margin_top(6);
        list.set_margin_bottom(12);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&list));
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_start(18);
        content.set_margin_end(18);
        content.set_margin_top(12);
        content.set_margin_bottom(6);
        content.set_vexpand(true);
        content.append(&note_label(explanation));
        content.append(&scroller);
        let delete_button = gtk::Button::with_label("Delete Selected");
        delete_button.add_css_class("destructive-action");
        delete_button.set_sensitive(false);
        let close_button = gtk::Button::with_label("Close");
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bottom.set_margin_start(12);
        bottom.set_margin_end(12);
        bottom.set_margin_top(8);
        bottom.set_margin_bottom(8);
        bottom.append(&delete_button);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        bottom.append(&spacer);
        bottom.append(&close_button);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&content);
        root.append(&bottom);
        Self {
            root,
            list,
            delete_button,
            close_button,
            checks: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn connect_close(&self, dialog: &adw::Dialog) {
        let weak = dialog.downgrade();
        self.close_button.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.close();
            }
        });
    }

    pub fn clear(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.checks.borrow_mut().clear();
    }

    /// A tracked checkbox that updates the delete button's count.
    pub fn new_check(&self, path: PathBuf) -> gtk::CheckButton {
        let check = gtk::CheckButton::new();
        let checks = Rc::downgrade(&self.checks);
        let button = self.delete_button.clone();
        check.connect_toggled(move |_| {
            if let Some(checks) = checks.upgrade() {
                update_delete_button(&button, &checks);
            }
        });
        self.checks.borrow_mut().push((path, check.clone()));
        check
    }

    pub fn selected(&self) -> Vec<PathBuf> {
        self.checks
            .borrow()
            .iter()
            .filter(|(_, check)| check.is_active())
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn refresh_delete_button(&self) {
        update_delete_button(&self.delete_button, &self.checks);
    }

    /// Confirm removal of selected paths; do nothing for an empty selection.
    pub fn confirm_removal(
        &self,
        parent: &adw::Dialog,
        verb: &str,
        noun: &str,
        body: &str,
        button: &'static str,
        on_confirm: impl FnOnce(Vec<PathBuf>) + 'static,
    ) {
        let selected = self.selected();
        if selected.is_empty() {
            return;
        }
        let heading = format!(
            "{verb} {} {}?",
            selected.len(),
            counted(selected.len(), noun)
        );
        dialogs::ask(
            parent,
            &heading,
            body,
            &[
                Response::new("cancel", "Cancel"),
                Response::destructive("confirm", button),
            ],
            "cancel",
            "cancel",
            move |response| {
                if response == "confirm" {
                    on_confirm(selected);
                }
            },
        );
    }
}

fn update_delete_button(button: &gtk::Button, checks: &Checks) {
    let count = checks
        .borrow()
        .iter()
        .filter(|(_, check)| check.is_active())
        .count();
    button.set_sensitive(count > 0);
    if count == 0 {
        button.set_label("Delete Selected");
    } else {
        button.set_label(&format!("Delete Selected ({count})"));
    }
}

pub fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}

pub fn connect_weak<S: 'static>(
    button: &gtk::Button,
    state: &Rc<S>,
    action: impl Fn(&Rc<S>) + 'static,
) {
    let weak = Rc::downgrade(state);
    button.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            action(&state);
        }
    });
}

pub fn show_error(dialog: &glib::WeakRef<adw::Dialog>, heading: &str, body: &str) {
    let Some(dialog) = dialog.upgrade() else {
        return;
    };
    dialogs::ask(
        &dialog,
        heading,
        body,
        &[Response::new("close", "Close")],
        "close",
        "close",
        |_| {},
    );
}

pub fn note_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    label
}

pub fn empty_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dim-label");
    label.set_margin_top(18);
    label.set_halign(gtk::Align::Center);
    label
}

pub fn section_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("heading");
    label.set_halign(gtk::Align::Start);
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label
}

/// A card row with an optional lead/action, path, detail, badges and warning.
pub fn build_row(
    lead: Lead,
    path: &str,
    detail: &str,
    badges: &[(String, &'static str)],
    action: Option<&gtk::Button>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("card");
    row.set_margin_start(2);
    row.set_margin_end(2);
    row.set_margin_top(2);
    row.set_margin_bottom(2);
    let warning = matches!(lead, Lead::Warning);
    let plain = matches!(lead, Lead::Plain);
    match lead {
        Lead::Check(check) => {
            let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            holder.set_margin_start(10);
            holder.set_valign(gtk::Align::Center);
            holder.append(&check);
            row.append(&holder);
        }
        Lead::Warning => {
            let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
            icon.set_margin_start(10);
            icon.add_css_class("warning");
            icon.set_valign(gtk::Align::Center);
            row.append(&icon);
        }
        Lead::Plain => {}
    }
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.set_margin_top(8);
    details.set_margin_bottom(8);
    details.set_margin_end(12);
    details.set_hexpand(true);
    if plain {
        details.set_margin_start(12);
    }
    let path_label = gtk::Label::new(Some(path));
    path_label.set_selectable(true);
    path_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    path_label.set_xalign(0.0);
    path_label.add_css_class("monospace");
    path_label.set_tooltip_text(Some(path));
    details.append(&path_label);
    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let detail_label = gtk::Label::new(Some(detail));
    detail_label.set_xalign(0.0);
    detail_label.add_css_class("dim-label");
    detail_label.add_css_class("caption");
    meta.append(&detail_label);
    for (text, class) in badges {
        let badge = gtk::Label::new(Some(text));
        badge.add_css_class("caption");
        badge.add_css_class(class);
        meta.append(&badge);
    }
    details.append(&meta);
    if warning {
        let note = gtk::Label::new(Some("Location unavailable"));
        note.set_halign(gtk::Align::Start);
        note.add_css_class("caption");
        note.add_css_class("warning");
        details.append(&note);
    }
    row.append(&details);
    if let Some(button) = action {
        button.set_margin_end(12);
        row.append(button);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::counted;
    #[test]
    fn counted_pluralizes() {
        assert_eq!(counted(1, "snapshot"), "snapshot");
        assert_eq!(counted(0, "snapshot"), "snapshots");
        assert_eq!(counted(2, "baseline snapshot"), "baseline snapshots");
    }
}
