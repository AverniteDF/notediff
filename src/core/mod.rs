//! Pure, GTK-free application core.
//!
//! Everything in this module is deliberately free of toolkit types so that
//! document handling, snapshot mapping, hunk application and undo can be
//! exercised by `cargo test` without a display server.

pub mod appearance;
pub mod diff;
pub mod document;
pub mod elapsed;
pub mod eol;
pub mod fsutil;
pub mod indent;
pub mod inline;
pub mod insert;
pub mod pending;
pub mod recent;
pub mod relink;
pub mod rename;
pub mod scripts;
pub mod session;
pub mod snapshot_store;
pub mod tabs;
pub mod textlines;
pub mod textstats;
pub mod themes;
pub mod undo;
pub mod whitespace;

pub const LARGE_FILE_WARNING_BYTES: u64 = 1024 * 1024;

/// Files larger than this are refused at open and reload, and a document above
/// this size cannot be saved or written as a baseline. GtkTextBuffer copies
/// the whole text for many operations, so a larger document makes the editor
/// unresponsive. This matches the hard limit for Add Text from File.
pub const MAX_OPEN_BYTES: u64 = 16 * 1024 * 1024;
