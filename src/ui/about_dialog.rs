//! The About dialog.

use adw::prelude::*;

use crate::ui::toast::{self, ToastKind};

// Name, version, author and description come from Cargo.toml so they cannot drift.
const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const RELEASE_DATE: &str = "November 18, 2026";
const LICENSE_NAME: &str = "MIT License";
const LICENSE_URL: &str = "https://opensource.org/licenses/MIT";

/// Show the About dialog. Everything is visible immediately; there are no
/// Details, Credits or Legal pages.
pub fn present(parent: &adw::ApplicationWindow) {
    let name = gtk::Label::new(Some(APP_NAME));
    name.add_css_class("title-1");

    let toast_overlay = adw::ToastOverlay::new();

    // The version is a blue pill button that copies itself to the clipboard.
    let version = gtk::Button::with_label(APP_VERSION);
    version.add_css_class("pill");
    version.add_css_class("suggested-action");
    version.add_css_class("version-pill");
    version.set_halign(gtk::Align::Center);
    {
        let toast_overlay = toast_overlay.clone();
        version.connect_clicked(move |button| {
            button.clipboard().set_text(APP_VERSION);
            let toast = toast::new(ToastKind::Success, "Version copied to clipboard");
            toast.set_timeout(2);
            toast_overlay.add_toast(toast);
        });
    }

    let description = centered_label(&format!(
        "{DESCRIPTION}. Review what has changed since your last save point, and \
         accept changes into the baseline one hunk at a time."
    ));

    let released = centered_label(&format!("Released {RELEASE_DATE}."));
    released.add_css_class("dim-label");

    let copyright = centered_label(&format!("© 2026 {AUTHOR}"));

    let license = centered_markup_label(&format!(
        "This application comes with absolutely no warranty. See the \
         <a href=\"{LICENSE_URL}\">{LICENSE_NAME}</a> for details."
    ));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_margin_top(0);
    content.set_margin_bottom(24);
    content.append(&name);
    content.append(&version);
    content.append(&description);
    content.append(&released);
    content.append(&copyright);
    content.append(&license);

    let header = adw::HeaderBar::new();
    header.set_show_title(false);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    toast_overlay.set_child(Some(&toolbar));

    let dialog = adw::Dialog::builder()
        .title("About notediff")
        .content_width(420)
        .build();
    dialog.set_child(Some(&toast_overlay));
    dialog.present(Some(parent));
}

fn centered_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_justify(gtk::Justification::Center);
    label
}

/// Markup is opt-in: Cargo metadata such as the author name is plain text and
/// must not be parsed as Pango markup.
fn centered_markup_label(markup: &str) -> gtk::Label {
    let label = centered_label("");
    label.set_use_markup(true);
    label.set_markup(markup);
    label
}
