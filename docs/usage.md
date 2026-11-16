# Using notediff

## Using the editor

**ChromeOS/Baguette focus:** Linux apps launched from the ChromeOS launcher may
open in the foreground without keyboard focus on Baguette-based ChromeOS. This
is a platform-wide issue; click the window to focus it. See the development
notes for details and workarounds.

- Open files with **Ctrl+O**, the header-bar button or **Open…** in the File
  menu. The chooser appears first; a chosen file replaces the document in the
  current window. If the current document has unsaved changes, the save prompt
  then offers **Open in New Window**, **Discard Changes** and **Save**; there is
  no second Cancel button, but Esc aborts the pending open. **Open in New Window**
  leaves the current document untouched. This action is unavailable in Diff Mode;
  use **New Window** or finish the review first. A command-line path or a
  file-manager launch opens a new window for each file.
- **Copy Selected Text** (copy icon immediately right of Open File) copies the
  editor selection, or the selection in the last-focused visible Diff Mode or
  inspection pane, through the clipboard API. It omits display-only diff markers
  and works for read-only text too, keeping the selection. A toast says
  **Copied 3 characters to clipboard**, or **Copied 45 characters (6 lines) to
  clipboard** for multiple selected lines. Counts use Unicode characters, not
  bytes; a selection ending at the start of a line does not count that line.
  Singular counts read **Copied 1 character to clipboard**. With no selection it
  says **No selected text to copy!** (the clipboard is left unchanged). Review
  highlighting alone is not a text selection. In the editor native Ctrl+C is
  unchanged and shows no toast; Diff Mode panes intercept the copy signal only
  to leave out display-only markers.
- Start a new document with **Ctrl+N**, the header-bar button beside Open, or
  **New File** in the menu. It replaces the document in the current window; an
  unsaved document first offers **Save**, **Discard Changes** or **Cancel**.
  The file and its baseline on disk are untouched. New File is unavailable in
  Diff Mode.
- Open another empty window with **Ctrl+Shift+N** or **New Window** in the File
  menu. The current window and document are left alone. New Window remains
  available in Diff Mode, so another file can be opened without disturbing the
  review.
- Save with **Ctrl+S**, Save As with **Ctrl+Shift+S**. Save As is unavailable
  while Diff Mode is showing, as are New File, Open File and Rename or Move;
  finish or close the review first. When a file that has a baseline is saved
  under a new path, notediff offers to copy the baseline to the new file
  (**Copy Baseline**); the original keeps its own.
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
- **Insert Text from File…** (right-click menu) inserts the whole contents of a
  chosen file at the cursor or replaces the current selection. The inserted text
  stays selected, and the whole edit is one undo step. Tabs become spaces, a
  leading BOM is dropped, and line endings are converted to match a document
  that uses one style. Files over 1 MiB ask for confirmation. Files over 16 MiB,
  folders and non-UTF-8 files are refused. An empty file inserts nothing. This
  action is unavailable for read-only documents and in Diff Mode.
- Find with **Ctrl+F**; press Enter to jump to the next match.
  Ordinary typing never opens Find. Cancelling Open File restores editor focus.
  Focus also returns after cancelling the unsaved-changes prompt, dismissing
  the main menu, or closing Manage App… → Baseline Snapshots. In Diff Mode it returns to a
  visible pane instead; Find and newly opened dialogs retain their own focus.
- **Replace…** (**Ctrl+H**, main menu) adds a replacement field to the Find bar.
  **Replace** changes the selected match and finds the next; if the selection
  is not a match, it only finds the next match. **Replace All** changes every
  non-overlapping match in the document and reports the count. Each operation
  is one undo step. An empty replacement deletes matches; an empty search does
  nothing. Matching is literal (no regex), case-insensitive unless **Match Case**
  is checked, and Find wraps at the end of the document. Replacement tabs become
  spaces. Replace is disabled for read-only documents and in Diff Mode; Find
  remains available for read-only documents and is disabled in Diff Mode.
- Click or drag the line-number gutter to select whole lines. The same works
  in either full Diff Mode pane.
- Go to Line with **Ctrl+G** (out-of-range numbers are clamped). It is
  unavailable in Diff Mode.
- Toggle **Word Wrap** with **Alt+Z** or in the main menu. It starts checked in each new window.
  When off, long editor lines scroll horizontally. The setting lasts for the
  life of the window, works on read-only documents, and is disabled in Diff Mode.
- **Scale Indentation** (right-click menu) has **Double** and **Halve**. They
  scale the leading spaces of the lines the selection touches, or of the whole
  document when nothing is selected. Only ASCII spaces count, and blank or
  whitespace-only lines count literally. Double does nothing (with a toast) if
  every target line has no indentation. Halve does the same, and also refuses
  if any target line has an odd number of leading spaces, naming the first such
  line. One undo step; unavailable for read-only documents.
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
  start of a line does not count that line. Clicking the line-ending field
  opens the Windows (CRLF) / Unix (LF) conversion choices, clicking the mode
  field (`INS`, `OVR` or `RO`) opens a checkable **Read Only** choice (unlocking a file that
  contains tabs asks the Convert Tabs question first). After the cursor
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
  initial release date, the copyright line and the no-warranty / MIT License
  notice.
- **Plain-text rendering.** Every file is shown as plain text. No language is
  attached to the editor or Diff Mode buffers, so Markdown headings, emphasis,
  code spans and list markers appear literally, and a `.rs`, `.md` or
  extensionless file is styled the same way. GtkSourceView still provides undo,
  line numbers, current-line highlighting and bracket matching. Find uses the
  buffer's normal selection colours. Theme schemes supply the text, selection,
  gutter, current-line and review colours.
- A document with unsaved changes shows a dot before its name in the title, and
  only the header bar gets a subtle scheme-derived accent tint until it is saved or reverted.
  A read-only document also shows **(Read-Only)** after its name.
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
- Quit with **Ctrl+Q** or **Quit** in the main menu. Every window closes through
  the normal close flow, so a dirty document still gets its save prompt and
  accepted review changes are confirmed before being discarded. When several
  windows need attention, their prompts appear one at a time; cancelling any
  prompt aborts the quit.

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
(read-only) and its header shows the canonical file path; the right pane is the
baseline and its header reads `Baseline`. The Confirm button shows how many
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

Accept All is available from the full Diff Mode toolbar. Accepting and undoing only
change the in-memory baseline; nothing is written to disk until **Confirm**.
When **Esc** would discard accepted changes, the prompt is **Discard accepted
changes?** with **Cancel** and **Discard and Exit Review**. It does not change
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
the usual save prompt first. Quit asks once per window. New File and Open File
are disabled while Diff Mode is showing, so accepted changes cannot be discarded
by opening another file; their header buttons also lose their shortcut
tooltips. Use **New Window** or finish the review first.

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
also marks the display-only line after it.
When the focused hunk is a pure
insertion or deletion, the empty pane marks the exact boundary with the same
3px strong-red bar used by inspection, so that pane still shows where the
change belongs; the bar scrolls with the text on both axes. In the full diff,
the focused hunk's line numbers use the stronger current-line colour (and bold
when the theme specifies it); an empty side emphasises its boundary row.
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
independently. Insertions and deletions leave one pane empty. Its header says
the same thing in either view: `Live document (<path>)` or `Baseline`, without
an empty-side suffix. The pane statistics describe the excerpts.

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
clicked; closing the dialog discards unapplied font edits. Applied settings affect
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
by OS filename ordering. Symlink directories and loose files are ignored. For
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

Missing or unreadable state uses defaults; malformed values are ignored per field.
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
