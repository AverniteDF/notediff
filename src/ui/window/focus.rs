//! Restore in-window editing focus after transient UI closes. This never
//! presents or activates a toplevel window; desktop activation remains the
//! compositor's responsibility.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::AppWindow;

impl AppWindow {
    pub(super) fn connect_transient_focus(this: &Rc<Self>, menu: &gtk::MenuButton) {
        // A menu closing because an action was chosen must not override that
        // action's focus (a new dialog, a file chooser, etc.). Find/Replace
        // are restored explicitly if GTK hands focus back to the menu button.
        // Check on idle because closure can precede action activation.
        let action_chosen = Rc::new(Cell::new(false));
        for name in this.window.list_actions() {
            if let Some(action) = this
                .window
                .lookup_action(&name)
                // Adding an activate handler to a stateful SimpleAction
                // disables its default toggle behavior. Stateful menu items
                // here are display toggles, not dialog-opening actions.
                .filter(|action| action.state().is_none())
                .and_then(|action| action.downcast::<gtk::gio::SimpleAction>().ok())
            {
                let chosen = action_chosen.clone();
                action.connect_activate(move |_, _| chosen.set(true));
            }
        }
        if let Some(popover) = menu.popover() {
            let chosen = action_chosen.clone();
            popover.connect_show(move |_| chosen.set(false));
            let weak = Rc::downgrade(this);
            popover.connect_closed(move |_| {
                let weak = weak.clone();
                let chosen = action_chosen.clone();
                glib::idle_add_local_once(move || {
                    if let Some(this) = weak.upgrade()
                        && (!chosen.get() || this.search_bar.is_search_mode())
                    {
                        this.restore_content_focus();
                    }
                });
            });
        }

        // Covers Cancel/Escape on save prompts, Manage Snapshots, About and
        // other Adwaita dialogs, after their own close handlers have run.
        let weak = Rc::downgrade(this);
        this.window.connect_visible_dialog_notify(move |window| {
            if window.visible_dialog().is_none() {
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(this) = weak.upgrade() {
                        this.restore_content_focus();
                    }
                });
            }
        });
    }

    fn focus_restore_allowed(&self) -> bool {
        if !self.window.is_visible() || self.window.visible_dialog().is_some() {
            return false;
        }
        // Do not reach past a separate modal window (e.g. a file chooser).
        if gtk::Window::list_toplevels()
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Window>())
            .any(|window| {
                window.is_visible()
                    && window.is_modal()
                    && window.transient_for().as_ref()
                        == Some(self.window.upcast_ref::<gtk::Window>())
            })
        {
            return false;
        }
        // A newly opened popover owns focus, even if its initial keyboard
        // placement has not run yet. Hidden/dismissed popovers do not block us.
        let mut widgets = vec![self.window.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = widgets.pop() {
            if widget.is::<gtk::Popover>() && widget.is_visible() {
                return false;
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                child = widget.next_sibling();
                widgets.push(widget);
            }
        }
        true
    }

    fn restore_content_focus(&self) {
        if !self.focus_restore_allowed() {
            return;
        }
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window);
        let within = |widget: &gtk::Widget| {
            focus
                .as_ref()
                .is_some_and(|focus| focus == widget || focus.is_ancestor(widget))
        };
        if self.diff_active() {
            if !within(self.diff.root.upcast_ref()) {
                self.diff.active_left_view().grab_focus();
            }
        } else if self.search_bar.is_search_mode() {
            if !within(self.search_bar.upcast_ref()) {
                self.search_entry.grab_focus();
            }
        } else {
            self.editor_view.grab_focus();
        }
    }
}
