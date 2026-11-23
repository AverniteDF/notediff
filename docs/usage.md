# Using notediff

## Using the editor

**ChromeOS/Baguette focus:** Linux apps launched from the ChromeOS launcher may
open in the foreground without keyboard focus on Baguette-based ChromeOS. This
is a platform-wide issue; click the window to focus it. See the development
notes for details and workarounds.

- Open files with **Ctrl+O**, the header-bar button or **Open…** in the File
  menu. **Ctrl+O** and **Open…** show the chooser directly. The header-bar
  button always opens a menu: its first row has a path field and a **Browse…**
  button, followed by up to ten recently opened file paths, newest first. Type
  an absolute path or `~/...`, or a path relative to the current document's
  folder, and press Enter to open it (or **Shift+Enter** to open it in a new
  window); **Browse…** opens the chooser (**Shift-click** opens the chosen file
  in a new window). A directory
  or device opens the chooser at that location (opening the chosen file in a
  new window when **Shift+Enter** was used). A missing or unreadable path
  flashes the field red for a second and keeps the menu open so you can try
  again; content errors (invalid UTF-8, NUL byte, too large) show a dialog.
  This happens before any unsaved-changes prompt. With the path field empty,
  Right moves focus to **Browse…**; Left returns to the field. The
  button's **Open File (Ctrl+O)** tooltip is hidden while this menu is showing.
  Each recent entry shows the file name in bold on
  the first line and its directory path in smaller, dimmed text below, ending
  with a slash. Directories under your home folder are shortened to `~/...`; the
  stored paths stay absolute. Each entry has a right-aligned hide icon (an eye
  with a slash); clicking it removes just that entry immediately, without
  confirmation. While **Shift** is held, a small new-window icon appears beside
  each recent file name, and clicking an entry opens it in a new window without
  disturbing the current document.
  Choosing the rest of the entry behaves like Open File.
  The chooser is shown first; a chosen file replaces the document in the
  current window. If the document has unsaved changes, the **Save changes?**
  prompt says **The document has unsaved changes.** and offers
  **Open New Window**, **Discard Changes** and **Save**; there is no Cancel
  button, but Esc aborts the pending open. **Open New Window** leaves the
  current document untouched. This action is unavailable in Diff Mode; use
  **New Window** or finish the review first. A file that is already open in
  another window is not opened a second time: that window is raised and a toast
  says **File is already open in this window**. The requesting window also
  shows **File is already open in another window**. This check happens
  before any save prompt, so redirecting never replaces or discards the current
  document. It applies to the chooser, recent entries, **Open New Window**, and
  command-line or file-manager launches. A command-line path or a file-manager
  launch otherwise opens a new window for each file. A newly opened or reloaded
  document starts with the cursor and scroll position at the top.
  Opening the file already loaded in the current window reloads it from disk.
  A dirty document first asks **Reload from Disk?** with the body
  **The document has unsaved changes.** and offers **Cancel** or
  **Reload from Disk**. A clean document reloads when the disk
  text differs. When the disk text is identical, nothing is replaced and a
  toast says **Document is identical with its file on disk**, preserving the
  cursor, scroll position and undo history. Reloading follows the usual
  Convert Tabs and read-only rules.
  Recent paths are kept in `$XDG_DATA_HOME/notediff/recent` (normally
  `~/.local/share/notediff/recent`). A missing file is removed only when the
  Open menu is used, so opening the editor does not scan the list at startup.
  Paths on a temporarily unavailable shared volume are kept; renaming or
  moving a file or using Save As updates the list to its new path. Only
  absolute UTF-8 paths without control characters are remembered.
- The system file chooser's **Open files read-only** checkbox belongs to the
  desktop portal and only restricts file access for sandboxed apps. It does not
  put notediff into Read-Only mode and does not stop changes from being saved;
  use the status-bar **Read Only** toggle to lock a document instead.
- **Copy Selected Text** (copy icon immediately right of Open File) copies the
  editor selection, or the selection in the last-focused visible Diff Mode or
  inspection pane, through the clipboard API. It omits display-only diff markers
  and works for read-only text too, keeping the selection. A toast says
  **Copied 3 characters to clipboard**, or **Copied 45 characters (6 lines) to
  clipboard** for multiple selected lines. Counts use Unicode characters, not
  bytes; a selection ending at the start of a line does not count that line.
  Singular counts read **Copied 1 character to clipboard**. With no selection
  in the editor outside Diff Mode it selects the whole document first, so the
  copied range is visible, and then copies it; an empty document says **No
  selected text to copy!** (the clipboard is left unchanged). A Diff Mode or
  inspection pane with no selection says the same warning without selecting
  anything. After content reaches the clipboard the copy icon briefly becomes a
  checkmark for about one second; this covers the button, Ctrl+C, the
  right-click Copy, and the title menu's **Copy file name** / **Copy full
  path**. A warning leaves the icon unchanged. Review highlighting alone is not
  a text selection. In the editor native Ctrl+C is unchanged and shows no
  toast; Diff Mode panes intercept the copy signal only to leave out
  display-only markers.
- Start a new document with **Ctrl+N**, the header-bar button beside Open, or
  **New File** in the menu. It replaces the document in the current window; an
  unsaved document first shows the same **Save changes?** prompt with
  **Open New Window**, **Discard Changes** and **Save**. Choosing
  **Open New Window** opens an empty window and keeps this document; Esc
  leaves everything as it is. The file and its baseline on disk are untouched.
  Shift-clicking the header New button opens a new window instead. New File is
  unavailable in Diff Mode.
- Open another empty window with **Ctrl+Shift+N** or **New Window** in the File
  menu. The current window and document are left alone. New Window remains
  available in Diff Mode, so another file can be opened without disturbing the
  review.
- Save with **Ctrl+S**, Save As with **Ctrl+Shift+S**. Save As is unavailable
  while Diff Mode is showing, as are New File, Open File and Rename or Move;
  finish or close the review first. When a file that has a baseline is saved
  under a new path, notediff offers to copy the baseline to the new file
  (**Copy Baseline**); the original keeps its own. If the chosen path is
  already open in another window, Save As is refused with **Could not save**
  and `That file is already open in another window.` and neither document
  changes; saving onto the current window's own path remains allowed.
- Rename or move the open file with **F2** or **Rename or Move…** in the File
  menu. Enter a new name to rename in place, or a path to move it (absolute,
  relative to the file's folder, or starting with `~/`; a folder keeps the file
  name). The destination folder must exist and nothing is overwritten. The file
  is moved on disk and its baseline moves with it, so Review Changes keeps
  working. Unsaved edits are untouched. Unavailable in Diff Mode. For an
  untitled document, this opens Save As instead.
- In the Rename or Move dialog, **...** (to the right of the input field) opens a file browser (starting in the
  file's folder) to pick the new folder and name; the chosen path is filled into
  the field and nothing happens until you press **Rename**.
- **Insert Path** (right-click menu) has **File…** and **Directory…**.
  **File…** inserts the absolute path of a chosen file; **Directory…** inserts
  the absolute path of a chosen folder. The path is inserted at the cursor or
  replaces the current selection, stays selected, and the whole edit is one
  undo step. Nothing is read, so file size, contents and folder permissions do
  not matter. Both choices are unavailable for read-only documents and in Diff
  Mode.
- **Add Text from File…** (right-click menu) inserts the whole contents of a
  chosen file at the cursor or replaces the current selection. The inserted text
  stays selected, and the whole edit is one undo step. Tabs become spaces, a
  leading BOM is dropped, and line endings are converted to match a document
  that uses one style. Files over 1 MiB ask for confirmation. Files over 16 MiB,
  folders and non-UTF-8 files are refused. An empty file inserts nothing. This
  action is unavailable for read-only documents and in Diff Mode.
- Find with **Ctrl+F**; if a short single-line selection exists in the editor,
  it pre-populates the Find field (selected so typing immediately replaces it).
  All matches are highlighted live while the bar is open, with a match counter
  (`3 of 12` or `No matches`) beside the field. Incremental search starts near
  the cursor rather than jumping back to line 1. Press **Enter** or **F3** (or
  the down arrow button) for the next match, and **Shift+Enter** or
  **Shift+F3** (or the up arrow button) for the previous match; **F3** and
  **Shift+F3** keep working even after the Find bar is closed. Toggle **Aa**
  (Match Case) or **W** (Whole Word) in the bar. **Esc** closes
  the Find/Replace bar from anywhere in the window, unless a dialog or menu is
  open, clears match highlights, and returns focus to the editor.
  Ordinary typing never opens Find. Cancelling Open File restores editor focus.
  Focus also returns after the unsaved-changes prompt closes, after the main
  menu or the Open menu is dismissed, or after closing Manage App… → Baseline Snapshots. In Diff Mode it returns to a
  visible pane instead; Find and newly opened dialogs retain their own focus.
- **Replace…** (**Ctrl+H**, main menu, or the expand arrow at the left of the
  Find bar) adds a replacement field to the Find bar; pressing **Ctrl+H** while
  Find is already open with a query focuses the Replace field directly.
  **Replace** (**Enter** in the Replace field) changes the selected match and
  finds the next; if the selection is not a match, it only finds the next match.
  **Replace All** (**Ctrl+Enter** in the Replace field) changes every
  non-overlapping match in the document and reports the count. Each operation
  is one undo step. An empty replacement deletes matches; an empty search or
  zero matches disables Replace and Replace All. Matching is literal (no regex),
  case-insensitive unless **Aa** (Match Case) is active, respects **W** (Whole
  Word), and wraps at the ends of the document. Replacement tabs become
  spaces. Replace is disabled for read-only documents and in Diff Mode; Find
  remains available for read-only documents and is disabled in Diff Mode.
- **Find in Files…** (**Ctrl+Shift+F**, main menu) opens a dialog to search
  recursively across UTF-8 text files in a folder (defaulting to the current
  document's folder, or your home directory for untitled windows). Input field
  values and toggles are remembered when the dialog is closed and reopened
  during the session; a short single-line selection in the editor pre-populates
  the query. Toggle **Aa** (Match Case) or **W** (Whole Word), and optionally
  narrow results with comma-separated **Files** (for example `*.rs, *.md`) and
  **Exclude** globs. Common dependency/build folders (`.git`, `node_modules`,
  `target`, `dist`, `build`, `venv`, `__pycache__`) and binary or oversized
  (>16 MiB) files are skipped automatically; up to 1,000 matches are shown,
  grouped into collapsible per-file sections (collapsed by default) with each
  file's match count beside its name. For files currently open with unsaved
  changes in any notediff window, the live in-memory buffer is searched instead
  of the older on-disk copy (marked `unsaved` beside the match count) so line
  numbers and selections always match what is on screen. Searches run
  asynchronously and can be stopped with **Cancel**. Clicking a match raises the window that already owns
  that file or opens it in a new window, and jumps directly to the matching
  line and selection; the dialog stays open unless the match targets the
  searching window's own open document. Each result preview pre-scrolls to
  center the matched text, and long lines can be dragged left or right to pan
  horizontally (keeping the line number pinned, and loading the full
  untruncated line lazily on first drag). Available in Diff Mode as well.
- Click or drag the line-number gutter to select whole lines. The same works
  in either full Diff Mode pane.
- Go to Line with **Ctrl+G** (out-of-range numbers are clamped). It is
  unavailable in Diff Mode.
- Toggle **Word Wrap** with **Alt+Z** or in the main menu. It starts checked in each new window.
  When off, long editor lines scroll horizontally. The setting lasts for the
  life of the window, works on read-only documents, and is disabled in Diff Mode.
- **Edit Indentation** (right-click menu) changes the leading spaces of the
  lines the selection touches, or of the whole document when nothing is
  selected. Only ASCII spaces count as indentation. Each operation is one undo
  step, keeps line breaks untouched, and is unavailable for read-only documents
  and in Diff Mode.
  - **Increase** (**Ctrl+]**) adds one space to every target line, including
    truly empty ones.
  - **Decrease** (**Ctrl+[**) removes one space from every target line only
    when every target line has at least one leading space;
    otherwise it changes nothing and shows no message. The empty line after a
    file's terminating line break is ignored, but other empty lines still
    participate.
  - **Double** (**Ctrl+Shift+]**) doubles the leading spaces and **Halve**
    (**Ctrl+Shift+[**) halves them. Double does nothing (with a toast) if every
    target line has no indentation; Halve does the same, and also refuses if any
    target line has an odd number of leading spaces, naming the first such line.
- **Trim Whitespace** (right-click menu) has four operations on the lines the
  selection touches, or on the whole document when nothing is selected. A blank
  line is empty or contains only spaces and tabs. Each item is greyed out when
  it would do nothing; the submenu heading is greyed out when all four are
  unavailable. Each operation is one undo step, reports the affected count in a
  success toast, and is unavailable for read-only documents and in Diff Mode.
  - **Leading** removes the leading ASCII spaces and tabs from each target line.
  - **Trailing** removes the trailing ASCII spaces and tabs from each target
    line.
  - **Blank Lines (strip)** keeps each blank target line but removes all ASCII
    spaces and tabs from it, leaving the line break.
  - **Blank Lines (remove)** deletes every blank target line.
  Example toasts include `5 leading whitespace characters were removed`,
  `3 blank lines were removed`, and
  `3 blank lines were stripped of 5 whitespace characters`.
- **EOL Conversion** (click the status-bar line-ending field) converts every line ending in the document to
  Windows (CRLF) or Unix (LF), including mixed endings. It is one undo step and
  leaves the document with unsaved changes. It is unavailable in Diff Mode and
  for read-only documents.
- A status bar under the editing area shows
  `bytes: <n>  chars: <n>  lines: <n> | Ln: <line>  Col: <col>  Pos: <byte pos>`
  on the left, and the line-ending style (Unix (LF), Windows (CRLF) or Mixed),
  `UTF-8` and the mode (`INS`, `OVR` after pressing Insert, or `RO` for a
  read-only file) on the right. Numbers use thousands
  separators (for example `bytes: 12,507`). `bytes` is the UTF-8 size of the
  current text, including unsaved edits, and `chars` is its character count.
  Ln, Col and Pos are 1-based; Col counts characters and Pos
  counts bytes. While text is selected, `Sel:` replaces `Pos:` and shows the
  number of bytes selected and the number of lines the selection touches
  (for example `Sel: 1,234 bytes, 4 lines`); a selection that ends at the very
  start of a line does not count that line. When the whole document is
  selected, `Sel:` reads `All` instead of the counts.
  A green `●` left of the line-ending field means the document has no trailing
  whitespace; it turns yellow when any line ends in spaces or tabs, and red when
  any line contains only spaces or tabs (a non-empty blank line). Red takes
  precedence, but a non-empty blank line is also trailing whitespace, so
  holding Alt while the dot is red shows the yellow state. The yellow tooltip
  is **Find trailing whitespace**; the red tooltip is **Find non-empty blank
  line**. Clicking a yellow dot selects the next trailing-whitespace
  run; clicking a red dot selects the whole content of the next non-empty blank
  line. Both scroll the result onscreen, focus the editor and wrap at the end of
  the document; repeated clicks advance through the document. Clicking a green
  dot does nothing. Both finds work in read-only documents and do nothing in
  Diff Mode.
  Clicking the line-ending field
  opens the Windows (CRLF) / Unix (LF) conversion choices, clicking the mode
  field (`INS`, `OVR` or `RO`) opens a checkable **Read Only** choice (unlocking a file that
  contains tabs asks the Convert Tabs question first; **Yes** converts them to spaces and
  marks the document dirty, and the conversion is not an undo step, so Ctrl+Z cannot bring
  the tabs back—reload the file to return to the tabbed text). After the cursor
  fields, `31 seconds ago` shows how long ago your last edit was, in
  a single unit (seconds, minutes or hours), with **just now** shown during the
  first second instead of `0 seconds ago`. It appears after the first edit
  and resets when a file is opened or reloaded.
- notediff has a **no tab characters** policy. Opening a file that contains
  tabs shows a **Convert Tabs** prompt with **Cancel**, **Yes** (red), and
  **No (Read Only)** (blue, the default for Enter). **Yes** replaces each tab with one
  space (the document opens with unsaved changes), **No (Read Only)** opens the
  file unchanged with editing disabled (`RO` in the status bar), and **Cancel**
  aborts the open (also with Esc). The same prompt appears when **Reload from Disk** finds tabs
  in a changed file (Cancel there keeps the current text, and the
  external-change prompt does not return until the file changes again). If a
  file gains tabs between that check and the actual reload, it opens read-only
  instead. Tabs in pasted or
  dropped text are converted to spaces automatically. The **Tab** key inserts
  one space (in overwrite mode it overwrites the next character with a space
  instead), or indents every selected line by one space; **Shift+Tab** removes
  one leading space from the selected lines (or the current line when nothing
  is selected).
- **About notediff** (menu) shows everything at once: the version (a blue
  button; click it to copy the version number), a short description, the
  release date, the copyright line and the no-warranty / MIT License notice.
- **Plain-text rendering.** Every file is shown as plain text. No language is
  attached to the editor or Diff Mode buffers, so Markdown headings, emphasis,
  code spans and list markers appear literally, and a `.rs`, `.md` or
  extensionless file is styled the same way. GtkSourceView still provides undo,
  line numbers, current-line highlighting and bracket matching. Find uses the
  buffer's normal selection colours. Theme schemes supply the text, selection,
  gutter, current-line and review colours.
- A document with unsaved changes shows a slightly raised `●` at 80% size
  followed by two spaces before its name in the title, and only the header bar
  gets a subtle
  scheme-derived accent tint until it is saved or reverted.
  A read-only document also shows **(Read-Only)** after its name. Hovering the
  title shows the full path; the tooltip is hidden while the right-click menu
  is showing. Right-clicking it offers **Copy file name** and
  **Copy full path**; untitled documents have no tooltip and both actions are
  disabled. Closing the menu with Esc, a click outside, or either copy action
  returns focus to the editor. Both copy actions briefly flash the copy
  button's checkmark and show no toast.
- Closing a window with unsaved text prompts to save, discard or cancel. For an
  untitled document the prompt's save action reads **Save As** and opens the file
  chooser. A new
  untitled document that is empty, including one that was typed into and then
  cleared out again, closes without a prompt. A saved file whose text again
  matches what is on disk (for example after undoing every edit) is not
  considered modified either.
- notediff starts with a dark, low-glare theme. Choose Light or an installed user theme in
  **Manage App…**, **Appearance** tab; it applies to every window at
  once and is remembered. Editor and diff text (and their line numbers) are
  slightly larger than the rest of the interface and use JetBrains Mono when it is
  installed, falling back to the system monospace font (see Appearance settings).
- On window focus, and before entering Diff Mode, notediff compares the file's
  modification time with the last read and offers to reload or keep the
  in-memory draft. Reloading follows the no-tab policy described above.
  The focus check also works in Diff Mode. Reloading returns to the editor
  and discards unconfirmed review changes, without changing the saved baseline.
  The dialog requires **Reload from Disk** or **Keep Current Text**; Esc and
  other dismissal attempts do not close it. Keeping the current text leaves the
  review untouched and continues any pending action, but does not acknowledge
  the disk change, so the warning can return on the next check. Saving with
  **Ctrl+S** also checks the modification time and asks before overwriting a
  file that changed on disk.
- Quit with **Ctrl+Q** or **Quit** in the main menu. Every window is handled at
  once through the normal close flow: a clean window with no accepted review
  changes closes immediately, while a dirty document gets its save prompt and a
  window with accepted review changes is asked before they are discarded. Each
  affected window shows its own prompt without waiting for the others.
  Cancelling a prompt leaves that window open; windows that already closed stay
  closed, and the application keeps running until every window closes.

Files are read and written as UTF-8. CRLF, LF and mixed endings are preserved
exactly. Files containing a NUL byte are refused. Files above 1 MiB show a
warning at open and reload because diffing runs in-process. Files above 16 MiB
are refused at open and reload, and a buffer that grows above 16 MiB cannot be
saved until text is removed. New line breaks, including Enter, follow the
document's dominant ending so a CRLF file stays CRLF.

## Review Changes

Press **F5** or click **Review Changes (F5)** in the header bar.
Diff Mode compares the baseline with the current editor text, so unsaved edits
can be reviewed without writing them first. The left pane is the live document
(read-only) and its header shows the document's path, shortened to `~/...` when
it is under your home folder; hover for the full path. The right pane is
the baseline and its header reads `Baseline`. Each header has a right-aligned
local timestamp in `YYYY-MM-DD HH:MM` form. The live side shows the last
in-memory edit, or the file's last-save time before any edit; its tooltip says
**Last edit** after an edit, or **Last saved** before one. The baseline side
shows the saved snapshot's modification time, with the tooltip **Last saved**.
Accepts and undos in memory do not change the baseline timestamp.
The Confirm button shows how many
changes have been accepted since the last Confirm, for example `Confirm (3 changes)`.
Each whole-hunk accept or inline accept counts as one change; Accept All counts
the pending whole hunks as changes and remains one undo step.
Each individual accept adds exactly one; Undo
restores the previous count. The Confirm count is independent of the number
of pending hunks, which can change when re-diffing regroups them. A replacement
split around an unchanged blank line is shown as one hunk when both sides
contain changed text; otherwise blank-separated changes remain separate.
For an untitled document the header button reads **Save As** and opens the
file chooser, while `F5` does nothing until the document has been saved once.

| Key | Action |
| --- | --- |
| `Up` / `Down` | Previous / next hunk in full diff, or individual change in inspection (no wrap) |
| `Right` | Accept the selected hunk or inline change in memory, re-diff, advance |
| `Left` | Undo the last accept in memory |
| `Enter` | Inspect the focused hunk or return to the full diff |
| `Esc` | Return to the full diff from inspection. In the full diff, leave Diff Mode, confirming first when changes have been accepted |

These are unmodified shortcuts. With Shift, Ctrl, Alt, Super or Meta held, they
do not navigate, accept, undo or change inspection.

Accept All is available from the full Diff Mode toolbar. Accepting and undoing only
change the in-memory baseline; nothing is written to disk until **Confirm**.
When **Esc** would discard accepted changes, the prompt is **Discard accepted
changes?** with **Cancel** and **Discard and End Review**. It does not change
the live document or the saved baseline.
**Confirm** saves the live document first if it has unsaved edits, then writes
the accepted baseline in one go. Cancelling that save prompt keeps Diff Mode
open and the baseline untouched. **Create Baseline** also needs a saved
document.

The same actions are available as buttons in the Diff Mode toolbar. The leave
button on the right reads **Close** in the full diff or **Back** in inspection
while no uncommitted changes have been accepted, and
**Confirm (1 change)** or **Confirm (2 changes)**, and so on, once at least one has;
**Confirm** turns green when every hunk has
been accepted. **Confirm** commits the accepted baseline to disk. If differences
remain, it stays in Diff Mode, resets the accepted count to zero and clears
accept-Undo history while preserving the current hunk and scroll position.
Green **Confirm** commits and returns to the editor. **Close** leaves without
writing anything; **Back** returns from inspection to the full diff. In the full diff, **Esc** asks to discard accepted changes before returning to the
editor; with nothing accepted it returns immediately. **Left** is the undo key.
Closing the window asks the same before discarding uncommitted in-memory hunks,
without touching the last confirmed baseline; a dirty live document still gets
the usual save prompt first. Quit prompts every affected window at once.
New File and Open File are disabled while Diff Mode is showing, so accepted
changes cannot be discarded by opening another file; their header buttons also
lose their shortcut tooltips. Use **New Window** or finish the review first.

Line-level changes are highlighted as background tags and word-level changes
within changed lines get an additional highlight. Adjacent word differences
separated only by whitespace form one continuous highlight, like the review
unit that Accept applies; unchanged words and punctuation separate highlights. Normal hunks use dark amber,
with brighter amber for inline differences; selected hunks use dark red, with
brighter red for inline differences. Dark themes use white text; light themes
use black text on lighter amber/red backgrounds. Highlights are opaque, without
blending against the editor background. Blank lines inside a hunk show a faded,
display-only pilcrow (`¶`) on the word-level background, so an empty change stays
visible; the marker is never copied or accepted. A change in the final newline
also marks the display-only line after it on the side that changed; a pure
insertion or deletion at the end of the file does not mark the unchanged side.
In inspection, blank-line pilcrows use the focused-hunk background unless the
selected change covers them.
When the focused hunk is a pure
insertion or deletion, the empty pane marks the exact boundary with the same
3px strong-red bar used by inspection, so that pane still shows where the
change belongs; the bar scrolls with the text on both axes. In the full diff,
the focused hunk's line numbers use the stronger current-line colour (and bold
unless the theme explicitly sets `bold="false"`); the theme's `auto`/percentage
gutter opacity applies only to normal line numbers, so focused rows are not
dimmed twice. An empty side emphasises its boundary row.
Both panes scroll independently in both directions. Mouse wheel, scrollbar,
touchpad and Page Up/Page Down scrolling affect only the pane being used.
Moving to a hunk scrolls each pane to its own anchor and resets horizontal
scrolling to zero. The full diff leaves lines unwrapped, and the two panes are
always the same width.

Each pane has its own status bar showing `bytes`, `chars`, `lines`, the line-ending
style and `UTF-8` for that pane's text (live document on the left, in-memory
baseline on the right). The bars update after every accept and undo. The
cursor and insert-mode fields are omitted because the panes are read-only.

### Inspect a hunk

Focus a hunk, then press **Enter** or click **Inspect** just left of **Accept**.
Each pane shows only its side of that hunk, wraps long lines and scrolls
independently. Insertions and deletions leave one pane empty. Its header keeps
the same document path or `Baseline` label and timestamp in either view,
without an empty-side suffix. The pane statistics describe the excerpts.

Inspection reviews **individual changes within paired lines**, rather than
accepting the entire line. Pure inserted/deleted blocks remain atomic.
Changes separated only by whitespace are reviewed as one phrase-level change;
an unchanged word or punctuation still separates them.
**Up** and **Down** (or the arrow buttons) select the
previous/next change, continuing into adjacent pending hunks when necessary.
Unchanged context has the normal editor background. The selected change is
highlighted with the stronger red inline background; other differences use the
selected-hunk red background. Both use white text in dark themes and black text
on lighter reds in light themes, without underlines. Review highlights change
colour only and add no bold formatting. Full Diff Mode
keeps its amber inline backgrounds. The whole hunk is not highlighted.
The **Accept** button or **Right** accepts only the selection. For example,
`quick → fast` and `jumps → sleeps` in one sentence can be accepted independently.

Replacements are atomic (not separate delete/insert accepts). Whitespace and
punctuation changes are reviewable too. Line-ending changes are separate.
A zero-width side is marked by a
strong-red caret at the exact insertion/deletion boundary; the adjacent text is
not itself replaced or styled. A wholly inserted or
deleted block is one atomic change: its
entire non-empty side uses the stronger focused-red background, and Right
accepts the whole block in one undo
step. Up/Down moves to another change, not between lines of that block. Unequal
multiline replacements remain atomic: notediff does not guess which lines
should be paired.
Use full Diff Mode to accept a whole hunk at once.

After acceptance, inspection follows the next pending change by live-text
position when it lies in the hunk being inspected. Re-diff regrouping uses the
original hunk's live-text byte span, so a split hunk still counts as one
region. If the accepted change was the region's last pending change, inspection
advances to the next pending hunk; if there is none, it falls back to the last
remaining change. The excerpts scroll independently to keep the selected change
visible, including within long wrapped paragraphs. **Left** undoes the last accept and restores the exact
selection. **Esc** and **Enter**
return to the full diff at the selected hunk. With no uncommitted accepts, the
right-aligned button reads **Back**, with tooltip **Return to full view (Enter)**.
It returns without accepting, undoing or committing anything; there is no
separate Back button beside Accept. There is no information row below
the toolbar: keyboard hints, the In sync message and change descriptions are
omitted so the panes have more space. Short button tooltips show the controls.
Accepting the final change returns to the full, in-sync diff automatically.
Undo after that automatic exit restores a hunk selection in the full diff.
Switching back into an unchanged inspection remembers its inline selection.

Full diff and inspection use separate, persistent pane pairs. The full-document
buffers, layout and viewport stay intact during inspection, so returning to
the same hunk requires no text replacement or scroll restoration. Accept/Undo
incrementally update the retained full diff; navigation positions it at the
newly selected hunk before switching back. There is no fade or opacity mask.

The right-aligned **Confirm** retains its save behavior. A partial Confirm
preserves inspection and the selected change, then becomes **Back**. After
returning to full diff it becomes **Close**.
The **Close** button has no tooltip; **Confirm** retains its concise save tooltip.
Use **Space** to activate a focused toolbar button.

### Create Baseline

The header button reflects the state of the open file:

- **Untitled document:** the button reads **Save As**, is enabled on a blue
  (suggested-action) background, and opens the Save As dialog. It has no
  tooltip. Once saved, the button becomes **Create Baseline**.
- **Saved file with no baseline yet:** the button reads **Create Baseline**.
  Clicking it writes the saved text to the snapshot store
  and nothing else: you stay in the editor, and the button becomes **Review
  Changes (F5)**, disabled while the text matches the new baseline. A document
  with unsaved edits gets the usual Cancel / Save prompt
  first. `F5` only shows a short notice until a baseline exists.
- **Saved file with a baseline:** the button reads **Review Changes (F5)**. It
  is enabled while the editor text differs from the baseline and disabled when
  the two match or while Diff Mode is showing. Its `Compare with baseline`
  tooltip is hidden in Diff Mode. `F5` still flashes **No Changes** when there
  is nothing to review.

The button refreshes when a file is opened or saved, after a baseline is
created, after the file is renamed, when the window regains focus, when
Manage App… → Baseline Snapshots closes, and when an edit settles and changes whether the
text matches the baseline.

If the baseline already equals the live text, Review Changes briefly shows
**No Changes** on the button and the view is not entered. An in-sync diff
(after accepting everything) stays on screen as two identical panes with the
navigation and accept buttons disabled.

## Appearance settings

Open **Manage App… → Appearance** to choose a theme and edit the content font.
Theme changes apply immediately. Font fields are drafts until **Apply Font** is
clicked or Enter is pressed in either field; closing the dialog discards
unapplied font edits. Applied settings affect
all existing and future editor/diff views and their line numbers, leaving the
interface font alone. Each newly opened dialog starts from the current settings;
applying font fields replaces both font settings globally.

### Themes

The **Appearance** tab of **Manage App…** lists **notediff Dark** (default),
**notediff Light**, then installed user themes, each with a small gutter-and-text
sample. Click a row to change every open window immediately. The selected ID is
saved in `$XDG_DATA_HOME/notediff/theme` (normally
`~/.local/share/notediff/theme`).

#### User themes

Each immediate subdirectory of `$XDG_DATA_HOME/notediff/themes` is one bundle:

```
cola-n-cream/
  theme.conf
  scheme.xml
```

`theme.conf` is UTF-8, with four required keys, each appearing exactly once:

```ini
id = cola-n-cream
name = Cola & Cream
description = Warm cream theme with cola-brown text and caramel accents
mode = dark
```

Blank lines and whole-line `#` comments are allowed. Values are unquoted text;
unknown or repeated keys, empty values and control characters are errors.
`mode` is exactly `dark` or `light`. IDs start with an ASCII lowercase letter,
contain only lowercase letters, digits and hyphens, and have at most 64 bytes.
`dark`, `light` and the `notediff-` prefix are reserved. Names/descriptions display
as plain text. No palette values belong in the descriptor.

`scheme.xml` is a normal GtkSourceView 5 style scheme whose ID must be
`user-<descriptor-id>` (for this example, `user-cola-n-cream`). It supplies the
editor text, selection, cursor, current-line, gutter, bracket and review
colours. notediff renders documents as plain text, so syntax (`def:*` or
language-prefixed) styles are not needed. Default GtkSourceView search paths
remain available for parent schemes. Bundles cannot depend on other notediff
user bundles; each is loaded by its own manager. A bundle without a line-number
sentinel loads its exact `scheme.xml`; when a sentinel is present, notediff
writes a sanitized copy to `$XDG_DATA_HOME/notediff/styles/<scheme-id>.xml`
and loads that copy. The bundle's `scheme.xml` remains the source of truth.

The Cola & Cream example deliberately pairs `mode = dark` (dark application
chrome) with a light cream editor and explicit light review colours. `mode`
selects the libadwaita chrome and the fallback review palette; it does not
constrain the scheme's own colours.

The `line-numbers` foreground is optional. It may be omitted, blank, `auto`, or a
percentage such as `70%`:

```xml
<style name="line-numbers" foreground="70%" background="gutter-bg"/>
```

`auto`, a missing value and a blank value mean 50% opacity. notediff derives the
normal gutter colour from the `current-line-number` foreground at that opacity.
These sentinels are notediff extensions, not GtkSourceView colours; notediff
sanitizes them before loading the scheme. An explicit `line-numbers` colour
always wins, and a parent scheme's explicit colour counts as defined unless the
theme uses a sentinel. Invalid percentages fall back to `auto`.

`current-line-number` is bold by default, matching the editor's cursor line;
set `bold="false"` to disable it. The `auto`/percentage `line-numbers` opacity
affects normal line numbers only; focused Diff Mode numbers use the full
`current-line-number` colour.

The project includes four example bundles: **Cola & Cream**, **Forest Dusk**,
**Grape Bubblegum** and **Slate Haze**. Install all of them, or replace the loop
list with one name, from the project root:

```bash
themes="${XDG_DATA_HOME:-$HOME/.local/share}/notediff/themes"
for theme in cola-n-cream forest-dusk grape-bubblegum slate-haze; do
  mkdir -p "$themes/$theme"
  cp "examples/themes/$theme/theme.conf" "examples/themes/$theme/scheme.xml" "$themes/$theme/"
done
```

**Restart notediff after installing, editing or removing bundles.** The catalog,
loaded schemes and palettes are retained for the process lifetime; there is no
live file monitor, Reload button or in-app installer. The Appearance picker and
theme cycling use the same order: Dark, Light, then bundle directory names sorted case-sensitively
by OS filename ordering. Symlink directories, symlinked `scheme.xml` files, `scheme.xml` files over 1 MiB
and loose files are ignored. For
repeated descriptor IDs, the first loadable bundle wins; invalid earlier bundles
do not reserve an ID. Invalid descriptors, missing/mismatched/unloadable XML,
unresolved parents, duplicates and unavailable saved choices appear in Appearance
and on standard error. A missing themes directory is normal. An unavailable saved
ID selects Dark without overwriting the saved preference until another selection.

Review highlights use four dedicated XML styles, with two standard palettes.
The built-in **notediff Dark** and **notediff Light** schemes include these styles
as a reference; user bundles may omit any or all of them and inherit the standard
palette for their declared mode. Dark-mode themes use white foreground
(`#ffffff`); light-mode themes use black foreground (`#000000`):

| XML style | Dark background | Light background |
| --- | --- | --- |
| `notediff:hunk` | `#664400` | `#fff0c2` |
| `notediff:word` | `#8a5a00` | `#ffd477` |
| `notediff:focused-hunk` | `#702020` | `#fad6d6` |
| `notediff:focused-word` | `#ad3030` | `#f39a9a` |

For example, add this inside a bundle's `scheme.xml` to override a review style:

```xml
<style name="notediff:hunk" foreground="#ffffff" background="#664400"/>
```

The dirty header uses a separate `notediff:accent` colour. The standard accents
are `#e0b36b` for dark themes and `#b36605` for light themes; the built-in
schemes show them as examples. User bundles may omit the style to use the
mode-standard accent, or override it:

```xml
<style name="notediff:accent" foreground="#e0b36b"/>
```

Both full diff and inspection use these exact, opaque foreground/background
pairs. They are never mixed with the editor background, selection colours or
other highlight layers. The foreground supplies the text colour inside review
highlights. Effective parent/use-style inheritance is supported. Missing styles
use the standards for the theme's declared dark/light mode, not its measured
editor background. Explicit XML overrides take precedence in either mode. Unset,
invalid or translucent fields use the matching mode-standard field.
A pair below 4.5:1 contrast falls back to its complete mode-standard
pair with a diagnostic, rather than changing its blend strength. Restart after
editing XML, as for other theme changes.

Theme previews show the gutter and a non-selected sample line. The gutter uses
the scheme's normal `line-numbers` colour, including its dimness; the sample text
uses the `text` colour with a readability fallback. The dirty-header
tint mixes `notediff:accent` with the header background at 35%. Normal editor
text is preserved, so an arbitrary user scheme can still contain poor colour
combinations (low base-text contrast is diagnosed). No automatic guarantee is
made for every theme style or the libadwaita header across all system settings.

### Fonts

The font family is a comma-separated list of unquoted names (for example
`Fira Code, monospace`); the first installed family is used. Names accept letters,
numbers, spaces, hyphens and underscores, must start with a letter or underscore,
and must not be empty. The list is limited to 256 UTF-8 bytes. Font size is a whole
number from 6 to 48 pt. Defaults are `JetBrains Mono, monospace` and 12 pt.

**Restore Defaults** at the bottom left immediately restores **notediff Dark**
and the default font family and size for all windows, and remembers both settings.
It also resets this dialog's font fields, discarding unapplied edits; font drafts
in other open Appearance dialogs are left alone. There is no confirmation, and
installed user themes are not removed. If a setting cannot be saved, it still
applies for this session and a warning is shown.

The app writes `$XDG_DATA_HOME/notediff/appearance` (normally
`~/.local/share/notediff/appearance`):

```
font_family = Fira Code, monospace
font_size_pt = 14
```

Missing, unreadable or oversized state uses defaults; malformed values are ignored
per field.
If saving fails, the font still applies for this session and a warning is shown.
Theme selection remains in the separate `theme` state file. Per-window zoom is
future work; the font size here is an application-wide preference.

## Snapshot storage

Baselines live under:

```
$XDG_DATA_HOME/notediff/snapshots/
# normally ~/.local/share/notediff/snapshots/
```

The store mirrors the canonical absolute path of each document. For example:

```
~/.local/share/notediff/snapshots/home/you/notes/todo.md
```

Each baseline is an exact byte-for-byte copy of the text, with no metadata or
header, and uses the same 16 MiB cap as documents. Symlinks are resolved, so a
link and its target share one baseline.
No baseline or other persistent artifact is written next to user documents.
Snapshot writes and live-file saves use a short-lived hidden temporary file and
a rename; it is normally removed immediately. A failure while writing the
temporary file leaves the original file untouched; only a mount that rejects the
rename itself falls back to a direct write.

**Manage App…** (menu), **Baseline Snapshots** tab, walks the store and lists:

- snapshots whose live file no longer exists (orphans), with a checkbox and
  last-modified date;
- snapshots whose live path is under an unavailable volume mount (for example
  an unshared `/mnt/chromeos/...` folder). A missing folder below a mounted
  volume is treated as an orphan instead.

Only checked orphans can be deleted, and deletion always requires confirmation.
Deleting an orphan discards only a baseline; live files are never touched.

### Re-linking baselines

If a file is renamed or moved outside notediff, its baseline is left behind as
an orphan and the file shows **Create Baseline**. Open the file, then open
**Manage App… → Baseline Snapshots**: a banner at the top shows the current document and
whether linking is available. It is not available for an untitled document, or
for a document that already has a baseline (linking never replaces one).

- **Link** (on each orphan row) *moves* the orphan's baseline to the current
  document. The old entry disappears.
- **Copy a baseline from another file** (a collapsed section, with a path
  filter) lists snapshots whose file still exists. **Copy** gives the current
  document its own copy of that baseline and leaves the original alone. This is
  useful after Save As, when you want to compare the new file against the old
  file's last baseline.

Both lists show the best matches first: same file name, then content similarity
to the current editor text, then most recently modified, with badges such as
"Same file name" and "87% similar". Link and Copy act immediately, without a
confirmation dialog; neither replaces an existing baseline. Live files are never
modified, and unsaved edits do not need to be
saved first. Renaming or moving with **Rename or Move** needs none of this,
because the baseline moves with the file, and Save As can copy the baseline
for you.

## Run Script

Press **F6** or choose **Run Script…** in the main menu to run an
executable script and open its output in a new notediff window. Because output
opens in a new window without modifying the current document, **Run Script…**
remains available while Diff Mode is open.

For the complete authoring specification (all field types, CLI/environment
parameter passing, sandboxing details and language examples), see
[Writing Scripts](scripts.md).

### Discovering and choosing scripts

notediff scans `$XDG_DATA_HOME/notediff/scripts/` (normally
`~/.local/share/notediff/scripts/`) each time the dialog opens, so newly added
or edited scripts appear immediately without restarting. Click **Browse…** in
the dialog header to pick any executable script elsewhere on disk.

The repository includes three example scripts under `examples/scripts/`:

- `prompt-packer.py` — packs selected files and project folders (with
  optional exclude globs and size limits) into a single Markdown prompt.
- `folder-search.py` — recursively searches UTF-8 text files in a folder for a
  literal or regex query and outputs a grouped or tabular Markdown report.
- `file-stats.py` — summarizes line, word, character and byte counts plus the
  top words of a text file; useful as a compact template for your own scripts.

`./install.sh` copies any missing example scripts into
`$XDG_DATA_HOME/notediff/scripts/` automatically (without overwriting existing
files). To copy or restore them manually from the project root:

```bash
scripts="${XDG_DATA_HOME:-$HOME/.local/share}/notediff/scripts"
mkdir -p "$scripts"
cp examples/scripts/prompt-packer.py examples/scripts/folder-search.py \
   examples/scripts/file-stats.py "$scripts/"
chmod +x "$scripts/prompt-packer.py" "$scripts/folder-search.py" "$scripts/file-stats.py"
```

### Script metadata comment block

A script may include a metadata block near the top (within the first 64 KiB)
delimited by `--- notediff-script ---` and `--- end-notediff-script ---` inside
`#`, `//` or `--` comments. Scripts without a block still run with no input
fields.

```python
#!/usr/bin/env python3
# --- notediff-script ---
# name = LLM Prompt Packer
# description = Pack files and project folders into a single Markdown prompt.
# sandbox = read-only
# output_ext = md
#
# [field.paths]
# label = Files and Folders
# type = paths
# required = true
# positional = true
# help = Select one or more files or folders to include
#
# [field.verbose]
# label = Verbose details on stderr
# type = bool
# arg = --verbose
# default = false
# --- end-notediff-script ---
```

Top-level header keys: `name`, `description`, `sandbox` (`read-only`,
`writable-dir` or `none`; default `read-only`) and `output_ext` (default `txt`).

Each `[field.<id>]` section defines one input widget in declaration order:

- `type` (required): `text`, `multiline`, `int`, `bool`, `choice`, `slider`
  (stepped scale over discrete `options`), `file`, `dir`, `paths` (multi-path
  picker with **+ File…**, **+ Folder…**, and quick **+ Current File** /
  **+ Current Folder** buttons when a file is open), or `list` (newline- or
  comma-separated values).
- `label`, `default`, `placeholder`, `help`, `required` (`true`/`false`).
- `arg` (CLI flag such as `--prompt`, `-x`, `--verbose`, or `--key=`) or
  `positional = true` (mutually exclusive). Flags are passed first, followed by
  positional values; empty optional values are omitted.
- `options` (required for `choice` and `slider`, separated by `|` or `,`), and
  `min` / `max` (integer bounds for `int`).

Every field value is also passed via `NOTEDIFF_INPUT_<UPPER_ID>` (with `-`
replaced by `_`; `1`/`0` for `bool`, newline-joined for `list`/`paths`),
alongside `NOTEDIFF_CURRENT_FILE`, `NOTEDIFF_CURRENT_DIR`, `NOTEDIFF_SANDBOX`,
and `NOTEDIFF_WRITABLE_DIR` (when applicable).

### Sandboxing

When `bwrap` (bubblewrap) is installed, the **Safety** selector in the footer
offers three modes:

- **Read-Only Sandbox** (default): mounts the host filesystem read-only with a
  private ephemeral `/tmp`, preventing scripts from modifying user files.
- **Allow Writing to Folder…**: keeps the rest of the filesystem read-only and
  grants write access only to the chosen folder.
- **No Sandbox**: runs the script directly with normal user permissions.

If `bwrap` is not installed, the dialog falls back to **No Sandbox**.

### Output and errors

- Scripts run asynchronously; click **Cancel** while running to stop the script.
- On exit status 0, `stdout` opens in the current window if it is empty and
  untitled, or in a new editor window otherwise, with a timestamped unsaved
  title (`<script>-YYYY-MM-DD-HHMMSS.<ext>`) pre-filled in **Save As**. Closing
  a non-empty output window without saving prompts **Save changes?** like any
  other unsaved document. If `stdout` contains tab characters, the window opens
  read-only under notediff's no-tab policy. Any `stderr` summary is shown as a
  toast in the output window.
- Output must be valid UTF-8, contain no NUL bytes, and stay within notediff's
  16 MiB document limit (output over 1 MiB shows a warning toast).
- On a non-zero exit status or validation error, the Run Script dialog stays
  open and displays an error dialog with the exit status and `stderr` output.
