# Changelog

All notable changes to notediff are documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses semantic
versioning.

## [Unreleased]

### Added

- The header Open button now opens a menu with **Browse…**, up to ten recently
  opened file paths and **Clear Recent** with a right-aligned trash icon. Each
  recent entry shows a bold file name above a smaller, dimmed directory path
  ending in `/`, with home paths shortened to `~/...`; recent rows have no
  tooltips. With no recent files the button opens
  the chooser directly. Missing paths are pruned lazily when the menu is shown,
  so startup does no recent-list filesystem scan.

### Changed

- Startup no longer rewrites the installed scheme files when their content is
  already up to date, avoiding a few fsyncs on every launch.
- New File now shows the same **Save changes?** prompt as Open File, with
  **Open New Window**, **Discard Changes** and **Save** and no Cancel button.
  **Open New Window** opens an empty window and leaves the current document
  untouched; Esc still aborts the pending action.
- Every **Save changes?** prompt, including Close Window and Quit, now reads
  **The document has unsaved changes.**

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

[Unreleased]: https://github.com/AverniteDF/notediff/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/AverniteDF/notediff/releases/tag/v1.0.0
