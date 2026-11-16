//! Shared toast construction.
//!
//! Every toast starts with a category emoji and two spaces, then the message.
//! Titles are shown as plain text, so file names and error strings containing
//! markup characters are displayed literally.

/// The category of a toast, which selects its leading emoji.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Warning,
    Error,
    Info,
}

impl ToastKind {
    pub fn emoji(self) -> &'static str {
        match self {
            ToastKind::Success => "✅",
            ToastKind::Warning => "⚠️",
            ToastKind::Error => "❌",
            ToastKind::Info => "ℹ️",
        }
    }
}

/// The full toast title: `<emoji><space><space><message>`.
pub fn title(kind: ToastKind, message: &str) -> String {
    format!("{}  {message}", kind.emoji())
}

/// Build a toast for `kind` and `message`. Markup is disabled so arbitrary
/// text is never interpreted as Pango markup.
pub fn new(kind: ToastKind, message: &str) -> adw::Toast {
    let toast = adw::Toast::new(&title(kind, message));
    toast.set_use_markup(false);
    toast
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_prefixes_the_message_with_two_spaces() {
        for kind in [
            ToastKind::Success,
            ToastKind::Warning,
            ToastKind::Error,
            ToastKind::Info,
        ] {
            assert_eq!(title(kind, "Saved"), format!("{}  Saved", kind.emoji()));
        }
    }

    #[test]
    fn every_kind_has_a_distinct_emoji() {
        let emojis = [
            ToastKind::Success.emoji(),
            ToastKind::Warning.emoji(),
            ToastKind::Error.emoji(),
            ToastKind::Info.emoji(),
        ];
        for (index, emoji) in emojis.iter().enumerate() {
            assert!(!emoji.is_empty());
            assert!(!emojis[..index].contains(emoji));
        }
    }
}
