//! Restore in-window editing focus after transient UI closes. This never
//! presents or activates a toplevel window; desktop activation remains the
//! compositor's responsibility.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::AppWindow;

impl AppWindow {
    pub(super) fn connect_transient_focus(
        this: &Rc<Self>,
        popovers: impl IntoIterator<Item = gtk::Popover>,
    ) {
        // A menu closing because an action was chosen must not override that
        // action's focus (a new dialog, a file chooser, etc.). Find/Replace
        // are restored explicitly if GTK hands focus back to the menu button.
        // Check on idle because closure can precede action activation.
        let action_chosen = Rc::new(Cell::new(false));
        // These actions do not claim focus. Choosing one must still restore the
        // editor. The Trim Whitespace submenu action only controls the heading's
        // sensitivity.
        const FOCUS_NEUTRAL_ACTIONS: [&str; 3] =
            ["copy-file-name", "copy-full-path", "trim-whitespace-menu"];
        for name in this.window.list_actions() {
            if FOCUS_NEUTRAL_ACTIONS.contains(&name.as_str()) {
                continue;
            }
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
        for popover in popovers {
            Self::connect_popover_focus(this, &popover, action_chosen.clone());
        }
        Self::watch_editor_context_menu(this, action_chosen.clone());

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

    /// The text view creates its context-menu popover lazily as a child.
    /// Watch for that child and hook it into the same focus restoration as
    /// the window-owned popovers.
    fn watch_editor_context_menu(this: &Rc<Self>, action_chosen: Rc<Cell<bool>>) {
        let weak = Rc::downgrade(this);
        let chosen = action_chosen.clone();
        this.editor_children
            .connect_items_changed(move |_, _, _, _| {
                if let Some(this) = weak.upgrade() {
                    Self::hook_editor_context_menu(&this, &chosen);
                }
            });
        Self::hook_editor_context_menu(this, &action_chosen);
    }

    fn hook_editor_context_menu(this: &Rc<Self>, action_chosen: &Rc<Cell<bool>>) {
        if this.context_menu_hooked.get() {
            return;
        }
        let mut child = this.editor_view.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Ok(popover) = widget.downcast::<gtk::PopoverMenu>() {
                this.context_menu_hooked.set(true);
                let weak = Rc::downgrade(this);
                popover.connect_show(move |_| {
                    if let Some(this) = weak.upgrade() {
                        this.update_context_menu_actions();
                    }
                });
                Self::connect_popover_focus(this, popover.upcast_ref(), action_chosen.clone());
                return;
            }
            child = next;
        }
    }

    /// Restore content focus when `popover` closes, unless the action that
    /// closed it has its own focus target.
    fn connect_popover_focus(
        this: &Rc<Self>,
        popover: &gtk::Popover,
        action_chosen: Rc<Cell<bool>>,
    ) {
        let shown = action_chosen.clone();
        popover.connect_show(move |_| shown.set(false));
        let weak = Rc::downgrade(this);
        let chosen = action_chosen;
        popover.connect_closed(move |_| {
            let weak = weak.clone();
            let chosen = chosen.clone();
            glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade()
                    && (!chosen.get() || this.search_bar.is_search_mode())
                {
                    this.restore_content_focus();
                }
            });
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
