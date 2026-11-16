//! Manage App shell: cleanup and appearance tabs with one shared toast overlay.

mod appearance_tab;
mod cleanup_dialog;
mod font_controls;
pub(crate) mod snapshots_tab;

use adw::prelude::*;

use self::appearance_tab::AppearanceTab;
use crate::core::snapshot_store::SnapshotStore;
use crate::core::themes;
use crate::ui::app_dialog::snapshots_tab::{LinkTarget, SnapshotsTab};

/// Build Manage App; persist appearance settings only when enabled.
pub fn present(
    parent: &adw::ApplicationWindow,
    store: SnapshotStore,
    target: Option<LinkTarget>,
    persist_settings: bool,
    on_closed: impl Fn() + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title("Manage App")
        .content_width(780)
        .content_height(640)
        .build();
    let toasts = adw::ToastOverlay::new();
    let stack = gtk::Stack::new();
    let snapshots = SnapshotsTab::new(&dialog, &toasts, store, target);
    stack.add_titled(&snapshots.root, Some("snapshots"), "Baseline Snapshots");
    let appearance = AppearanceTab::new(
        &dialog,
        &toasts,
        persist_settings.then(themes::default_path),
        persist_settings,
    );
    stack.add_titled(&appearance.root, Some("appearance"), "Appearance");
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    toasts.set_child(Some(&toolbar));
    dialog.set_child(Some(&toasts));
    // Sole strong owner of the tab state, released with the dialog.
    dialog.connect_closed(move |_| {
        let _keep_alive = &snapshots;
        on_closed();
    });
    dialog.present(Some(parent));
}
