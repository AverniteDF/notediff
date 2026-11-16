# Changelog

All notable changes to notediff are documented here. The project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and uses semantic
versioning.

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

[1.0.0]: https://github.com/AverniteDF/notediff/releases/tag/v1.0.0
