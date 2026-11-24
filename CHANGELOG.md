# Changelog

All notable changes to notediff are documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses semantic
versioning.

## [Unreleased]

### Added

- `install.sh` now bundles example presets from `examples/presets/` (including
  **Create a notediff Script** and **Greenfield Project Development** for
  `prompt-packer.py`) and seeds any missing bundled presets into `script-state`
  without overwriting existing presets of the same name.

## [1.5.0] - 2026-11-23

### Added

- **Run Script** (`F6`) now remembers the last-selected script, per-script
  parameter drafts, overridden sandbox mode, and chosen writable folder
  across restarts (stored in `$XDG_DATA_HOME/notediff/script-state`, saved on
  script switch, dialog close, and **Run**). A **Presets** menu beside the
  script title lets you save, overwrite, load, and delete named per-script
  presets or **Reset to Defaults**. **No Sandbox** still requires confirmation
  before every run.
- The bundled `prompt-packer.py` example gains an optional **Prompt Text**
  field (and `--prompt`), so the packed Markdown opens with your instructions
  instead of `(Insert your prompt here)`.

### Changed

- The About dialog now reads "A plain-text editor with document baseline
  comparison. Review changes without needing a repository."
- Moved the **Run** button in **Run Script** into the bottom footer bar,
  right-aligned alongside the sandbox controls; the footer no longer shows a
  separate **Safety Setting:** label.
- `install.sh` now also installs missing example themes into
  `$XDG_DATA_HOME/notediff/themes/`, installs `docs/scripts.md`, and seeds a
  **Create a notediff Script** example preset for `prompt-packer.py` on a fresh
  install.

## [1.4.0] - 2026-11-23

### Added

- Press **F4** to open **Manage App…**.
- A blocked edit in the read-only editor briefly opens the mode popover with
  a yellow outline around **Read Only**, without moving keyboard focus.
- **Find in Files…** (**Ctrl+Shift+F**, also in the main menu) searches UTF-8
  files in the selected folder and, by default, its subfolders. Uncheck
  **Recursive** in the filters row to search only the selected folder.
  The search supports **Match Case** (`Aa`), **Whole Word** (`W`), optional file and
  exclude globs, session-persisted inputs, collapsible per-file result groups
  (collapsed by default) with match counts, live in-memory searching of dirty
  open buffers, and cancellable background execution. Each matching line
  appears once, with a bounded preview around the highlighted match. Clicking
  a bold line number cycles matches. Clicking the preview opens or raises the
  target and jumps to the match if it is not in Diff Mode.
- **Run Script…** (**F6**, also in its own section of the main menu) discovers
  executable scripts in `$XDG_DATA_HOME/notediff/scripts/` (or any script
  chosen with **Browse…**), includes a clickable `ℹ️` header button to open the
  embedded script authoring guide (`docs/scripts.md`), renders a parameter form from an embedded
  `--- notediff-script ---` comment block, optionally runs scripts inside a
  `bwrap` read-only or single-writable-folder sandbox, and opens `stdout` in
  the current window when empty and untitled or in a new editor window
  otherwise, with a timestamped unsaved name (read-only if the output contains
  tab characters). Shipped with
  `examples/scripts/prompt-packer.py`, `examples/scripts/folder-search.py`, and
  `examples/scripts/file-stats.py` (installed automatically by `install.sh` when
  not already present).

### Changed

- Added a little more space between the checkbox and its label in **Find in
  Files** and **Run Script**.
- **Run Script** requires explicit confirmation before running without
  bubblewrap, including when sandboxing is unavailable.
- **Accept All** uses the exact live text without rebuilding it from a new diff.

### Fixed

- **Find in Files** checks that a clicked result still matches at its original
  position. If not, it shows **Aborted stale search** and asks you to search
  again instead of selecting unrelated text.
- **Rename or Move** never replaces an existing live destination, and a move
  across filesystems no longer hides a previously unnoticed external edit.
- Saving refuses an unavailable current path instead of silently recreating it.
  **Save As** to the current path also checks for external changes.
- Baselines are written privately without a non-atomic fallback, and Link and
  Copy cannot replace a baseline that appears during the operation.
- **Run Script** no longer gets stuck on inherited output pipes and rejects
  fields that would overwrite each other's environment variable.
- **Find in Files** checks for cancellation within long lines and bounds the
  number of hits it collects per line and the size of each preview.
- Closing **Find in Files** or **Run Script** releases the dialog's state.
- Non-UTF-8 command-line arguments produce an error instead of a panic.

## [1.3.0] - 2026-11-22

### Added

- **Find and Replace** now pre-populates from a short single-line selection,
  highlights all matches live with a match counter (`3 of 12` / `No matches`),
  anchors incremental search at the cursor instead of jumping to line 1, adds
  **Whole Word** (`W`), linked Previous/Next buttons, **Shift+Enter** / **F3**
  / **Shift+F3** navigation (working even with the bar closed), an in-bar
  Replace toggle, and **Ctrl+Enter** for Replace All.
- Holding **Shift** in the header **Open** menu now shows a small new-window
  icon beside each recent file name; **Shift-clicking** a recent entry or
  **Browse…**, or pressing **Shift+Enter** in the path field, opens the file in
  a new window without disturbing the current document.

### Changed

- **Quit** now handles every window at once: clean windows close immediately
  and each dirty window or window with accepted review changes shows its own
  prompt, instead of waiting for the previous window.

### Fixed

- Undoing (**Ctrl+Z**) after **EOL Conversion** now restores the original line
  endings and preferred line-ending style instead of re-converting them back.
- Hiding the last recent file in the **Open** menu now keeps the menu open with
  the path field focused instead of closing it.
- **Rename or Move** now refuses a destination path that is already open in
  another window, matching **Save As**.
- Resizing the window or changing the content font while inspecting a Diff Mode
  hunk now updates empty-side carets immediately.
- Fixed GObject reference-cycle leaks in the **Appearance** font controls and
  the **Open** menu, and improved focus restoration after context-menu edits.
- Hardened file I/O and theme loading: snapshots under ejected ChromeOS
  removable drives (`/mnt/chromeos/removable/...`) are now treated as
  unavailable rather than orphaned, user `scheme.xml` files are bounded to
  1 MiB (ignoring symlinks and commented-out line-number styles), and
  cross-filesystem moves sync the destination before removing the source.

## [1.2.0] - 2026-11-21

### Added

- The editor status bar now shows a trailing-whitespace dot: green with the
  tooltip **No trailing whitespace**, yellow with **Find trailing whitespace**.
  Clicking a yellow dot selects the next trailing-whitespace run and wraps at
  the end of the document.
- The status dot turns red when a line contains only spaces or tabs. Clicking it
  selects the next non-empty blank line; holding Alt shows the yellow
  trailing-whitespace state and clicking then selects the next trailing run.
- The editor context menu now has an **Edit Indentation** submenu at the
  bottom, below a separator, with **Increase** (Ctrl+]), **Decrease** (Ctrl+[),
  **Double** (Ctrl+Shift+]) and **Halve** (Ctrl+Shift+[).
- The editor context menu now has an **Insert Path** submenu with **File…** and
  **Directory…**, plus a top-level **Add Text from File…** item, which insert a
  path or a file's text like a paste without reading path targets.
- The editor context menu now has a **Trim Whitespace** submenu with **Leading**,
  **Trailing**, **Blank Lines (strip)** and **Blank Lines (remove)** for the
  selected lines or the whole document. Each item is greyed out when it would do
  nothing, reports the affected count, and is one undo step. The submenu heading
  is greyed out when all four are unavailable.
- The header Open button now opens a menu with a path field and a **Browse…**
  button. Typing a path and pressing Enter opens it; relative paths resolve
  from the current document's folder, and `~` expands to the home folder.
  Directories and devices open the chooser at that location. A missing or
  unreadable typed path flashes the field red for a second and keeps the menu
  open for another try; content errors still show a short open dialog, before
  any dirty document's save prompt. With an empty path field, Right moves to
  **Browse…** and Left returns to the field.
- Diff Mode pane headers show the live document's canonical path, shortened to
  `~/...` under the home directory with the full path in a tooltip, and
  right-aligned local timestamps for the last document edit (or save) and the
  saved baseline.

### Fixed

- **Ctrl+Shift+]** and **Ctrl+Shift+[** now trigger **Double** and **Halve** on
  platforms that report the shifted brace key values.
- Clicking or dragging the line-number gutter on a scrolled document now
  selects the lines under the pointer instead of lines from the top of the
  file.
- Save As now refuses a path that is already open in another window instead of
  letting both windows own the same file.
- Unlocking a read-only document that contains tabs no longer adds an undo step
  that does nothing and no longer shows a misleading **Tabs were converted to
  spaces** toast on Ctrl+Z.
- Moving the pointer no longer keeps changing the gutter selection after a drag
  is cancelled by the system.
- Diff Mode now ignores its review keys when Shift, Ctrl, Alt, Super or Meta is
  held, so Shift+Right and Ctrl+Up no longer accept a hunk or move the
  selection.
- A pure insertion or deletion at the end of a file no longer shows a spurious
  blank-line pilcrow on the side whose final newline did not change.
- Blank-line pilcrows in inspection now use the focused-hunk background instead
  of the editor background.
- Focused Diff Mode line numbers now match the editor's cursor-line number:
  derived gutter opacity is no longer applied a second time, and they use the
  same bold default.
- Fixed reference-cycle leaks in the About dialog, the status-bar whitespace dot
  and the Diff Mode line-number renderer.
- **uninstall.sh** now succeeds when no applications directory was ever created,
  and the installed launcher stays valid when the cargo binary path contains a
  backtick or dollar sign.
- Changing the content font now re-measures the Diff Mode line-number gutter, so
  larger line numbers no longer clip and smaller fonts no longer leave excess
  width.
- Changing the content font now resizes the Diff Mode caret on an empty hunk side
  to the new line height instead of keeping the previous height.
- Diff Mode carets now follow their text when the window width changes and a
  wrapped line reflows, instead of staying at the previous pixel position.
- **Review Changes** no longer stays enabled after an edit is undone back to the
  baseline before the pending text check runs.
- **Quit** now handles the window it was invoked from first, closing it when
  clean or prompting there, before moving on to other windows.
- **Go to Line** accepts numbers beyond the document and clamps them to the last
  line instead of reporting them as invalid.
- Cancelling **Save As** or an **Insert** chooser now returns keyboard focus to
  the editor.
- Copying a large Diff Mode selection no longer walks the selection one
  character at a time.
- The baseline-move failure message now points to **Manage App… → Baseline
  Snapshots** instead of a menu that does not exist.
- Opening or reloading a file now places the cursor and the scroll position at
  the beginning of the document.
- Pressing Enter in the **Appearance** font fields applies the font, like
  **Apply Font**.
- Escape now closes the search bar from any focus target in the window,
  including the editor and the replace field.
- A baseline that exists but cannot be read is now reported as an error instead
  of being treated as if the document had no baseline.
- A moved baseline whose old snapshot cannot be removed now reports that it was
  **moved**, not linked.
- Translucent `current-line-number` theme colours keep their own opacity in the
  derived gutter colour.
- State and theme descriptor files over 64 KiB are ignored instead of being read
  unbounded at startup.

### Changed

- The dirty-document marker before the title is a `●` at 80% size, raised
  slightly, followed by two spaces.
- The Appearance tab explanation is shorter, and the user-theme restart note
  uses the same subdued style as the other explanatory text.
- The About dialog's version-copy toast now disappears after two seconds.
- A non-regular file is now reported as **That is not a regular file.**
- Copying the file name or full path from the title menu no longer shows a
  success toast; the copy button's checkmark is the confirmation.
- The discard-accepted-changes dialog now uses one count-free message for every
  number of accepted changes, and its exit button reads **Discard and End
  Review**.

## [1.1.0] - 2026-11-18

### Added

- The header Open button now opens a menu with **Browse…** and up to ten
  recently opened file paths. Each entry shows a bold file name above a smaller,
  dimmed directory path ending in `/`, with home paths shortened to `~/...`, and
  has a right-aligned `view-conceal-symbolic` button that removes that entry
  immediately. Recent rows have no tooltips. With no recent files the button
  opens the chooser directly. Missing paths are pruned lazily when the menu is
  shown, so startup does no recent-list filesystem scan.
- Shift-clicking the header New button opens a new window. The shortcut is
  hidden from the tooltip and menus.
- The document title shows the full path in a tooltip and has a right-click menu
  with **Copy file name** and **Copy full path**.
- A canonical file path is now open in at most one window. Opening it from
  another window raises the existing window with **File is already open in this
  window**, and the requesting window shows **File is already open in another
  window**.
- Reopening the current window's file reloads it from disk. A dirty document
  first asks **Reload from Disk?** with **The document has unsaved changes.**
  before discarding edits. When the disk text is identical, the editor is left
  untouched and shows **Document is identical with its file on disk**, preserving
  the cursor, scroll position and undo history.

### Changed

- The Diff Mode pane headers now read **Document** and **Baseline**, without
  tooltips.
- Startup no longer rewrites the installed scheme files when their content is
  already up to date, avoiding a few fsyncs on every launch.
- New File now shows the same **Save changes?** prompt as Open File, with
  **Open New Window**, **Discard Changes** and **Save** and no Cancel button.
  **Open New Window** opens an empty window and leaves the current document
  untouched; Esc still aborts the pending action.
- Every **Save changes?** prompt, including Close Window and Quit, now reads
  **The document has unsaved changes.**
- The Open button and document title hide their tooltips while their menus are
  showing. Closing the title copy menu restores editor focus after Esc, an
  outside click, or either copy action.

## [1.0.0] - 2026-11-16

The first public release.

### Added

- A GTK 4 editor for UTF-8 plaintext and Markdown with no tab characters,
  application-wide themes and configurable content fonts.
- New File and New Window, with the usual save prompt before replacing a dirty
  document. Open File chooses the file first and can keep the current document by
  opening the selection in a new window.
- Diff Mode: compare the live file with one saved baseline, inspect whole hunks
  or individual word-level changes, and accept or undo them before confirming
  the baseline. New Window remains available while Diff Mode locks the other
  document-replacing file actions. Esc leaves the review, Left undoes, and
  closing or quitting asks before discarding accepted changes.
- Baseline snapshots stored under `$XDG_DATA_HOME/notediff/snapshots`, with
  moving, copying, re-linking and orphan cleanup.
- Rename or Move with the baseline following the file, and Save As with optional
  baseline carry-over.
- Built-in light and dark themes, user theme bundles, font settings, word wrap,
  Find/Replace, Go to Line, EOL conversion and indentation scaling.
- Whole-line selection by clicking or dragging the line-number gutter in the
  editor and either full Diff Mode pane.
- External-change detection, binary and oversized-file protection, and a clear
  16 MiB limit for opening, reloading, saving and baselines.
- An installer and uninstaller, desktop entry, application icon and GitHub
  Actions CI.

[Unreleased]: https://github.com/AverniteDF/notediff/compare/v1.5.0...HEAD
[1.5.0]: https://github.com/AverniteDF/notediff/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/AverniteDF/notediff/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/AverniteDF/notediff/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/AverniteDF/notediff/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/AverniteDF/notediff/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/AverniteDF/notediff/releases/tag/v1.0.0
