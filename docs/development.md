# Development

See [`design.md`](design.md) for the product design and [`usage.md`](usage.md)
for user-facing behaviour.

## Requirements

- Rust 1.92 or newer, installed with rustup. Debian trixie’s `apt` Rust is
  1.85; the GTK bindings and edition-2024 let-chains need a newer toolchain.
- GTK 4.18+, libadwaita 1.7+ and GtkSourceView 5.16+ development packages.
  Ubuntu 24.04 is too old.
- On Debian 13:

```sh
sudo apt install build-essential pkg-config \
  libgtk-4-dev libadwaita-1-dev libgtksourceview-5-dev
```

## Architecture

```
src/
  core/
    appearance.rs       validated, app-written content font preferences
    themes.rs           owned metadata, user descriptors, discovery and saved choice
    diff.rs             line splitting, hunk computation, word spans,
                        hunk application, accept-all invariant, diff deadline
    document.rs         live path, saved text and character count, mtime,
                        preferred line ending, size/NUL-refusing reads,
                        save-size limit, rename
    snapshot_store.rs   canonical path mapping, atomic writes, scan,
                        link (move/copy) and baseline move
    undo.rs             bounded in-memory undo stack (100 entries or 64 MiB)
    textstats.rs        status-bar numbers: byte and character counts, line count,
                        line-ending style, trailing-whitespace and non-empty
                        blank-line flags, cursor line/column/byte position,
                        selection size, thousands separators
    elapsed.rs          last-edit status text and its refresh schedule
    tabs.rs             no-tab policy helpers: tab detection and conversion
    textlines.rs        GTK-compatible line breaks, splitting and byte offsets
    eol.rs              line-ending conversion (LF / CRLF, including lone CR/U+2029)
    indent.rs           indentation editing: increase including empty lines,
                        strict decrease, double and halve
    whitespace.rs       leading/trailing ASCII space/tab detection, non-empty
                        blank-line detection, the next wrapped trailing or
                        blank-line run, leading/trailing trim plans, and
                        blank-line remove/strip plans
    inline.rs           byte-exact word, line-ending and fallback change units,
                        with the same diff deadline as `diff.rs`
    insert.rs           preparing a file's text for insertion: size limits,
                        BOM, line-ending matching
    pending.rs          PendingAction: actions that wait for a save, including
                        OpenPath(PathBuf); ExternalCheck and AfterOpen: what
                        follows an external-change check or an aborted open
    recent.rs           bounded recent-document paths, line state and lazy
                        existence pruning
    relink.rs           ranking of baselines that could be linked
    rename.rs           plan a rename or move (name or path); move a file
                        with its baseline
    session.rs          Diff Mode session: focus, inspection, accept, accept all, undo
    fsutil.rs           atomic replace with a rename-only fallback, bounded
                        UTF-8 reads, regular-file/NUL/size refusal, path helpers
  ui/
    window.rs           main window: shared state, widget construction, status
                        updates and the registry that owns every open window
    window/
      clipboard.rs      header Copy Selected Text button and focused-pane tracking
      editing.rs        insert filter for tabs and document line endings, Tab
                        key, indentation editing, trailing-whitespace find and trim,
                        EOL conversion, file insertion, word wrap, read-only state
      file_actions.rs   new file (including Shift-click to a new window) and
                        open (chooser or recent path first, post-chooser save
                        prompt), save (with external-change overwrite guard),
                        save as, rename or move, Convert Tabs prompt,
                        external-change handling, save-then-continue and the
                        Quit continuation after a save prompt
      diff_actions.rs   Diff Mode wiring: applies session results to the
                        snapshot store and the UI, Diff Mode keys, Create
                        Baseline and Confirm
      menus.rs          menu models (including the Open menu), shared menu
                        actions, accelerators and Quit
      flags.rs          DocumentFlags (dirty, read-only, close, external-change,
                        inserted line endings) and scoped loading guards
      flash.rs          FlashState: the header button's "No Changes" flash
      focus.rs          restore content focus after menus and dialogs close
      search.rs         search bar, Find, Replace and Go to Line
      status.rs         StatusState (measured text, last edit, refresh timer),
                        status bar refresh methods and Alt dot tracking
      theme_keys.rs     hidden Alt+T / Alt+Shift+T theme cycling
      tests.rs          display-dependent GTK window smoke scenarios
    diff_page.rs        persistent full/inspection view pairs, tags, independent
                        scrolling and anchor placement, toolbar and pane headers
    diff_toolbar.rs     GTK-free toolbar state derived from the review session
    gutter_selection.rs whole-line selection from left line-number gutters
    app_dialog.rs       Manage App: one dialog with a tab per page
    app_dialog/
      cleanup_dialog.rs shared scaffolding of those tabs (page, rows, confirm flow)
      snapshots_tab.rs  Baseline Snapshots tab: orphan review, deletion and linking
      appearance_tab.rs theme picker and content font controls
      font_controls.rs  staged font editing, global apply and persistence
    about_dialog.rs     About dialog
    dialogs.rs          shared AlertDialog builder and response helpers
    status_bar.rs       status bar widget for the editor and the diff panes
    toast.rs            shared toast construction
    theme_catalog.rs    startup-only catalog, isolated scheme loading and diagnostics
    theme_palette.rs    fixed/XML review colours, scheme-derived dirty tint and previews
    theme.rs            applies the current theme at runtime: light/dark
                        appearance, style schemes, stylesheet, content font
                        size, change listeners for diff highlight colors
assets/
  notediff-dark.xml     embedded built-in GtkSourceView style schemes
  notediff-light.xml
  notediff.css          application stylesheet (placeholders filled by theme.rs)
```

The `core` modules, including the Diff Mode session logic, have no GTK
dependency and are unit-tested headlessly. The
primary invariant — accepting every hunk, in any order, leaves the baseline
byte-identical to the live text — is covered by a deterministic property test
with random texts, CRLF, LF, lone CR and U+2029 endings.

Diffing and inline refinement share a one-second time budget. A timed-out diff
is still valid; it may be less minimal, but every hunk range remains usable.
`apply_all_hunks` builds its result in one pass rather than re-diffing per hunk.

## Tests

```sh
cargo test
```

One ignored UI smoke test runs the editing, file-action and Diff Mode
scenarios on the same GTK thread with a temporary snapshot store. It covers
tab handling, read-only mode, Find/Replace, EOL conversion, all four indentation
operations, the trailing-whitespace and non-empty-blank-line dot states
(including the Alt override and both finds), leading/trailing and blank-line
trim, Word Wrap,
the Insert Path submenu and Add Text from File insertion with selection,
baseline moves, links and copies, and the Diff
Mode accept, undo and confirm flow. It exercises inline inspection keys, selection
highlights, partial-line review, line endings, wrapped selection scrolling, pure
insertions and deletions, and long-document scroll placement after repeated
and rapid Accept/Undo operations. It also covers the external-change overwrite
prompt on save, simultaneous multi-window Quit across dirty, accepted-review
and clean windows, Esc and window-close review-discard confirmations, the Open
File save prompt after the chooser (including Open New Window and no Cancel
button),
recent-menu ordering, lazy pruning, open-recent and per-entry removal, the
document title tooltip and copy menu (including tooltip suppression while the
menu is open and focus restoration after dismissing it or choosing either copy
action), the Diff Mode path and timestamp headers,
Shift-click New, disabled New/Open actions and hidden header tooltips in Diff
Mode, duplicate-path redirection to the owning window, same-window reload
confirmation and identical-content detection, the Open menu's tooltip
suppression, CRLF
Enter handling, Add Text from File line-ending preservation, and that every editor and
diff buffer remains plain text. It checks that closing a window asks before
discarding unconfirmed hunks and releases its `AppWindow` state. Run it in a graphical session. Theme coverage
uses isolated directories and includes descriptor
errors, missing/invalid XML and parents, deterministic duplicates, inherited styles,
an example theme as a user bundle, saved-ID fallback without rewriting, and existing/future
editor plus all four diff buffers. Ordinary unit tests cover descriptor parsing,
discovery, colour fallback, contrast and stylesheet substitution, regular-file
read rejection, atomic-write fallback policy and permissions, save-size limits,
and the undo byte budget.

```sh
cargo test gtk_window_smoke -- --ignored
```

On a ChromeOS/Sommelier host whose Wayland bridge drops the display, force X11:
`GDK_BACKEND=x11 NO_AT_BRIDGE=1 dbus-run-session -- cargo test --locked gtk_window_smoke -- --ignored`.

The GitHub Actions workflow runs formatting, clippy with `-D warnings`, the
unit tests, the ignored smoke test under Xvfb plus a DBus session, and a
separate `cargo check` on Rust 1.92.

## Production builds

The committed release profile keeps `lto = "thin"` and leaves symbols in place,
which links quickly (about 20 s) and is the right default while developing. It
produces a binary of roughly 2.2 MB.

Builds for distribution should additionally strip the symbol table and let fat
LTO merge more code. Measured on Debian 13 on 2026-11-17:

| Profile | Binary | Build |
| --- | --- | --- |
| default release (`lto = "thin"`) | 2,218,216 B | ~20 s |
| `strip = true`, `lto = "fat"`, `codegen-units = 1` | 1,437,312 B (−35%) | ~2 min |
| the same plus `panic = "abort"` | 1,336,736 B (−40%) | ~2 min |

None of these settings change behaviour except `panic = "abort"`, which turns a
panic into an abort instead of unwinding through GTK callbacks; it also removes
the unwinding tables. Use it for distribution builds unless crash unwinding
matters to you.

Apply the profile overrides only when building for distribution, so normal
builds stay fast:

```sh
cargo build --release --config 'profile.release.lto="fat"' \
  --config 'profile.release.codegen-units=1' \
  --config 'profile.release.strip=true'

cargo install --locked --path . --config 'profile.release.lto="fat"' \
  --config 'profile.release.codegen-units=1' \
  --config 'profile.release.strip=true'
```

`cargo install` accepts the same overrides, so `install.sh` needs no change for
a smaller local install, and a tagged release may commit the settings to
`[profile.release]` instead.

For the full versioning, verification, tagging and GitHub publishing checklist,
see [Releasing](releasing.md).

Startup is dominated by GTK and the desktop's display bridge, not by the binary
size: a bare GTK window takes ~140 ms to appear on this ChromeOS/Sommelier
setup, and an empty notediff window ~300 ms. The scheme files under
`$XDG_DATA_HOME/notediff/styles` are only rewritten when their content changes,
so a launch does not fsync them again.

## Notes and limitations

- One baseline per file, no history. Replacing the live file outside notediff
  (rename, move, symlink retarget) breaks the association; either start a new
  baseline at any time or re-link the old one in **Manage App… → Baseline
  Snapshots**.
- Hunk rejection happens by leaving Diff Mode and editing the live text;
  rejection is not an in-Diff-Mode action.
- Files above 16 MiB are refused at open and reload, and a buffer that grows
  above that limit cannot be saved. The same limit applies to baselines, so the
  app cannot create a file it would refuse to reopen.
- Change notification relies on modification times, which is the portable
  approach for ChromeOS shared folders.
- Quit handles every window at once: clean windows close immediately, while
  dirty windows and windows with accepted review changes each show their prompt.
  Cancelling a prompt leaves that window open; windows already closed stay
  closed, and the application keeps running until every window closes.
- Diff Mode disables New File, Open File, Save As and Rename or Move. New Window
  remains available; finish or close the review to replace the current document.
- Sanitized copies of user theme XML are cached under
  `$XDG_DATA_HOME/notediff/styles` for the process lifetime and are not pruned
  when a bundle is removed.
- **ChromeOS/Baguette launch focus:** On ChromeOS builds using the Baguette
  (containerless) Linux environment, a Linux GUI app launched from the ChromeOS
  launcher can appear in the foreground without receiving keyboard focus. This
  affects notediff and other Linux apps (confirmed with GNOME Text Editor), so
  it is a host window-manager limitation rather than an app bug. Native Wayland
  cannot request activation through Sommelier: it advertises `gtk_shell1`
  version 1 and `gtk_surface1` version 1, where `present` is a no-op, and it
  does not advertise `xdg_activation_v1` (GTK's `request_focus` needs
  `gtk_surface1` version 3). The X11 bridge does not override the host's
  decision for a new guest window either. Clicking the window focuses it
  normally, with no loss of data. Check for a ChromeOS update; the known
  workaround is to disable `chrome://flags/#crostini-containerless` (Baguette)
  and reinstall the Linux environment after backing up files.
