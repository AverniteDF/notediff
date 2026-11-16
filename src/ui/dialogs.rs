//! Small helpers for building and showing `adw::AlertDialog`s.
//!
//! Every confirmation or error prompt in the app has the same shape: a
//! heading, a body, a few responses (some suggested or destructive), a default
//! response and a close response. These helpers keep that in one place.

use adw::prelude::*;
use gtk::gio;

/// One button of an alert dialog.
pub struct Response {
    id: &'static str,
    label: &'static str,
    appearance: adw::ResponseAppearance,
}

impl Response {
    /// A plain button.
    pub fn new(id: &'static str, label: &'static str) -> Self {
        Self::with(id, label, adw::ResponseAppearance::Default)
    }

    /// The blue, suggested button.
    pub fn suggested(id: &'static str, label: &'static str) -> Self {
        Self::with(id, label, adw::ResponseAppearance::Suggested)
    }

    /// The red, destructive button.
    pub fn destructive(id: &'static str, label: &'static str) -> Self {
        Self::with(id, label, adw::ResponseAppearance::Destructive)
    }

    fn with(id: &'static str, label: &'static str, appearance: adw::ResponseAppearance) -> Self {
        Self {
            id,
            label,
            appearance,
        }
    }
}

/// Build an alert dialog. `default` is the response activated by Enter and
/// `close` is the response reported when the dialog is dismissed (Esc).
pub fn build(
    heading: &str,
    body: &str,
    responses: &[Response],
    default: &str,
    close: &str,
) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    for response in responses {
        dialog.add_response(response.id, response.label);
        if response.appearance != adw::ResponseAppearance::Default {
            dialog.set_response_appearance(response.id, response.appearance);
        }
    }
    dialog.set_default_response(Some(default));
    dialog.set_close_response(close);
    dialog
}

/// Present `dialog` over `parent` and call `on_response` with the id of the
/// chosen response.
pub fn choose(
    dialog: &adw::AlertDialog,
    parent: &impl IsA<gtk::Widget>,
    on_response: impl FnOnce(&str) + 'static,
) {
    dialog
        .clone()
        .choose(Some(parent), None::<&gio::Cancellable>, move |response| {
            on_response(response.as_str())
        });
}

/// Build and present an alert dialog in one step.
pub fn ask(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    responses: &[Response],
    default: &str,
    close: &str,
    on_response: impl FnOnce(&str) + 'static,
) {
    let dialog = build(heading, body, responses, default, close);
    choose(&dialog, parent, on_response);
}
