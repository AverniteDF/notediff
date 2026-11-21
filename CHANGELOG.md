# Changelog

All notable changes to notediff are documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses semantic
versioning.

## [Unreleased]

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
  **Trailing**, **Blank Lines (remove)** and **Blank Lines (strip)** for the
  selected lines or the whole document. Each item is greyed out when it would do
  nothing, reports the affected count, and is one undo step. The submenu heading
  is greyed out when all four are unavailable.
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

### Changed

- The dirty-document marker before the title is a `●` at 80% size, raised
  slightly, followed by two spaces.
- The Appearance tab explanation is shorter, and the user-theme restart note
  uses the same subdued style as the other explanatory text.
- The About dialog's version-copy toast now disappears after two seconds.
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

[Unreleased]: https://github.com/AverniteDF/notediff/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/AverniteDF/notediff/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/AverniteDF/notediff/releases/tag/v1.0.0
