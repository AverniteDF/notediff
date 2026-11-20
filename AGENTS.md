# Agent Notes

`notediff` is a Linux desktop editor for UTF-8 plaintext and Markdown with a
baseline-comparison Diff Mode. It is built with Rust 2024, GTK 4.18+,
libadwaita 1.7+ and GtkSourceView 5.16+.

Read these first:

- `README.md` for the project overview and user-facing quick start.
- `docs/design.md` for architecture and invariants.
- `docs/usage.md` for exact user-visible behaviour and strings.
- `docs/development.md` for the module map and test/CI notes.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo +1.92.0 check --locked
cargo test --locked gtk_window_smoke -- --ignored
```

The smoke test needs a display. CI runs it under Xvfb with a DBus session. On a
ChromeOS/Sommelier host whose Wayland bridge drops the display, force X11:

```sh
GDK_BACKEND=x11 NO_AT_BRIDGE=1 dbus-run-session -- \
  cargo test --locked gtk_window_smoke -- --ignored
```

## Invariants

- The live file is the source of truth. A baseline is disposable and never the
  only copy of a document.
- Accepting every hunk, in any order, must leave the baseline byte-identical to
  the live text. The property test in `core/diff.rs` protects this.
- Documents are plain text. No language or syntax highlighting is attached to
  editor or diff buffers.
- No tab characters in editable documents. Tabs in opened files trigger the
  Convert Tabs prompt; pasted/dropped tabs become spaces.
- Atomic writes must never touch the destination when the temporary write fails.
  Only a rejected rename may fall back to a direct write.
- The 16 MiB limit applies to open, reload, save and baselines.
- Diff Mode locks document-replacing file actions: New File, Open File, Save As
  and Rename or Move are disabled. New Window remains available.
- A canonical document path is open in at most one window. Opening it from
  another window raises the existing window; opening it in its own window
  reloads it from disk after confirming unsaved changes.
- User-visible labels and messages are documented in `docs/usage.md`. Update the
  docs in the same change as behavioural code.
- User-visible behaviour also gets one bullet under `## [Unreleased]` in
  `CHANGELOG.md`, in the same change. The release procedure only renames, dates
  and verifies those entries; internal refactors, tests and doc reshuffles need
  no entry.
- Compatibility/migration code is out of scope: this is a greenfield 1.x
  project. Prefer removing obsolete code to keeping compatibility paths.

## Layout

The `core/` modules are GTK-free and unit-tested headlessly. The `ui/` modules
own GTK widgets and callbacks. `ui/window.rs` owns the window registry and
delegates editing, file actions, Diff Mode, menus, search and status logic to
its submodules. See `docs/development.md` for the full map.

## Release checks

- Follow `docs/releasing.md` for the complete release SOP, including `gh`
  authentication, tag creation and GitHub release publishing.
- `cargo fmt --check`, clippy, unit tests, the smoke test and the 1.92 check are
  green.
- Version and release date agree in `Cargo.toml`, `CHANGELOG.md` and
  `src/ui/about_dialog.rs`.
- The application ID is `io.github.AverniteDF.Notediff` in `src/main.rs` and the
  desktop-file/icon names match it.
- Distribution builds use the size profile in `docs/development.md` → Production
  builds (`strip`, fat LTO, one codegen unit); day-to-day builds stay on the
  default thin LTO.
- `target/` must not be committed.
