# Writing Scripts for notediff

`notediff`'s **Run Script…** (`F6`) feature lets you run local executable
scripts from a dynamically generated GUI form and open their `stdout` directly
in the current editor if it is empty and untitled, or in a new window otherwise.

This document specifies everything needed to write, install, and debug your own
scripts.

---

## 1. Script Location & Discovery

`notediff` discovers scripts in:

```text
$XDG_DATA_HOME/notediff/scripts/
# normally ~/.local/share/notediff/scripts/
```

- The folder is scanned every time the **Run Script** dialog opens, so adding,
  editing, or removing scripts takes effect immediately without restarting
  `notediff`.
- Regular files and symlinks to regular files are listed. Subdirectories and
  dot-files (names starting with `.`) are ignored.
- Scripts are sorted case-insensitively by their display `name`, then by file
  name.
- You can also run a script stored anywhere on disk using **Browse…** in the
  dialog header.

### Executability & Shebang

Scripts are executed directly by the OS (never passed through a shell). Every
script must:

1. Have execute permission (`chmod +x my-script.py`).
2. Start with a valid shebang line (for example `#!/usr/bin/env python3`,
   `#!/usr/bin/env bash`, `#!/usr/bin/env node`, or `#!/usr/bin/env lua`).

A non-executable file in the scripts folder still appears in the sidebar with a
`(not executable)` notice and a disabled **Run** button so the cause is clear.

---

## 2. Metadata Comment Block

A script may include a metadata block within its first **64 KiB** (valid UTF-8)
describing its title, defaults, and GUI input fields. Scripts without a
metadata block are still runnable — they display their file name stem, define
no input fields, default to `sandbox = read-only` and `output_ext = txt`, and
receive the standard `NOTEDIFF_*` context environment variables.

### Syntax

The block is delimited by `--- notediff-script ---` and
`--- end-notediff-script ---` inside comments using `#`, `//`, or `--`:

```python
#!/usr/bin/env python3
# --- notediff-script ---
# name = My Tool
# description = Short summary shown above the form.
# sandbox = read-only
# output_ext = md
#
# [field.target]
# label = Input File
# type = file
# required = true
# positional = true
# help = Choose a UTF-8 text file
# --- end-notediff-script ---
```

Rules:

- The opening marker line determines the comment prefix (`#`, `//`, or `--`).
  Every line inside the block must begin with that same prefix (optional
  leading whitespace is allowed).
- Blank lines and `#`-comment lines inside the block are ignored.
- Entries use `key = value`. Surrounding whitespace is trimmed; values are
  unquoted plain text and may not contain control characters.
- Unknown keys, duplicate keys, duplicate field IDs, and unclosed blocks
  produce a line-numbered diagnostic in the dialog and disable **Run**.

### Top-Level Header Keys

Header keys appear before the first `[field.<id>]` section. All are optional:

| Key | Default | Description |
| --- | --- | --- |
| `name` | File name stem | Display name shown in the sidebar and form header |
| `description` | *(empty)* | Short description shown under the script title |
| `sandbox` | `read-only` | Initial **Safety** mode: `read-only`, `writable-dir`, or `none` (user overrides and field drafts are remembered across sessions in `$XDG_DATA_HOME/notediff/script-state` and can be cleared per script with **Reset**) |
| `output_ext` | `txt` | Extension for the timestamped output window title (`<stem>-YYYY-MM-DD-HHMMSS.<ext>`) and **Save As** initial name. ASCII alphanumeric, up to 16 characters (leading `.` stripped) |

---

## 3. Input Fields (`[field.<id>]`)

Each `[field.<id>]` section defines one input control, rendered in declaration
order.

- `<id>` must be 1–64 ASCII characters, start with a letter or `_`, and contain
  only letters, digits, `_`, or `-`. IDs must be unique within a script even
  after uppercasing and replacing `-` with `_` (for example, `my-file` and
  `MY_FILE` cannot both be used).

### Field Types

| `type` | Widget | Value Passed | Extra Keys |
| --- | --- | --- | --- |
| `text` | Single-line text entry | Trimmed string | `default`, `placeholder` |
| `multiline` | Word-wrapped multi-line text box | Trimmed string | `default`, `placeholder` |
| `int` | Integer spin button | Base-10 integer string | `default`, `min`, `max` |
| `bool` | Checkbox | Flag only when checked (`1` / `0` in env) | `default` (`true`/`false`/`1`/`0`) |
| `choice` | Dropdown menu | Selected option string | `options` (required), `default` |
| `slider` | Stepped horizontal scale with tick marks and live value readout | Selected option string | `options` (required), `default` |
| `file` | Text entry + **Browse…** file picker | Trimmed path string | `default`, `placeholder` |
| `dir` | Text entry + **Browse…** folder picker | Trimmed path string | `default`, `placeholder` |
| `paths` | Multi-path list with **+ File…**, **+ Folder…**, and **+ Current File** / **+ Current Folder** (when a saved file is open) | List of absolute paths | *(none)* |
| `list` | Multi-line text box split on newlines and commas | List of trimmed non-empty strings | `default`, `placeholder` |

### Field Keys

| Key | Required | Description |
| --- | --- | --- |
| `type` | Yes | One of the ten field types above |
| `label` | No | Human-readable label (defaults to `<id>`) |
| `default` | No | Initial widget value. For `int`, must parse as an integer; for `bool`, `true`/`false`/`1`/`0`; for `choice` and `slider`, must match an entry in `options` (otherwise defaults to the first option) |
| `placeholder` | No | Hint text for `text`, `multiline`, `file`, `dir`, and `list` |
| `help` | No | Dimmed caption rendered below the control |
| `required` | No | `true` or `false` (default `false`). When `true`, **Run** stays disabled until the field has a non-empty value |
| `arg` | No | CLI flag to pass this field (for example `--prompt`, `-x`, `--verbose`, or `--format=`). Mutually exclusive with `positional = true` |
| `positional` | No | `true` or `false` (default `false`). Appends value(s) as positional CLI arguments after all flags. Mutually exclusive with `arg` |
| `options` | For `choice` & `slider` | Discrete choices separated by bare pipe characters (see example below), or by commas if no pipe is present. Duplicate entries are ignored |
| `min` / `max` | No (`int`) | Optional integer lower/upper bounds (`min` must be `<= max`) |

Example `options` lines (use bare `|` characters, or `,` if no `|` is present):

```ini
options = low | medium | high
options = 2, 4, 8
```

---

## 4. Parameter Passing

When the user clicks **Run**, `notediff` passes field values through CLI
arguments (according to `arg` / `positional`) **and** through environment
variables.

### CLI Arguments

1. All `arg` flags are emitted first, in field declaration order.
2. All `positional = true` fields are emitted afterward, in field declaration
   order.
3. Empty optional values are omitted so your script's own defaults and mutually
   exclusive CLI options work naturally.
4. Formatting per field type:
   - **`text`, `multiline`, `int`, `choice`, `slider`, `file`, `dir`:**
     - If `arg` ends with `=`, emits one argument `<arg><value>` (e.g.,
       `--format=markdown`).
     - Otherwise emits two arguments `[<arg>, <value>]` (e.g.,
       `--top`, `15`).
     - If `positional = true`, emits `[<value>]`.
   - **`bool`:**
     - When checked: emits `[<arg>]` (or `["true"]` if `positional = true`).
     - When unchecked: emits nothing.
   - **`list`, `paths`:**
     - Emits `[<arg>, <item>]` (or `<arg><item>` if `arg` ends with `=`) once
       for each non-empty item, or `[<item>]` per item when `positional = true`.
   - **Neither `arg` nor `positional = true`:**
     - No CLI arguments are emitted; read the value from its environment
       variable instead.

### Environment Variables & Working Directory

Every invocation receives:

| Variable | Value |
| --- | --- |
| `NOTEDIFF_INPUT_<UPPER_ID>` | Value of field `<id>` (`-` replaced by `_`, uppercased). `1` or `0` for `bool`; newline-joined (`\n`) for `list` and `paths`; empty string if unset |
| `NOTEDIFF_CURRENT_FILE` | Canonical absolute path of the invoking window's open file (omitted for untitled windows) |
| `NOTEDIFF_CURRENT_DIR` | Parent directory of `NOTEDIFF_CURRENT_FILE` (omitted for untitled windows) |
| `NOTEDIFF_SANDBOX` | Active **Safety** mode: `read-only`, `writable-dir`, or `none` |
| `NOTEDIFF_WRITABLE_DIR` | Path of the allowed writable directory (set only when `NOTEDIFF_SANDBOX=writable-dir`) |

If the invoking window has a saved file, the script's working directory (`cwd`)
is set to `NOTEDIFF_CURRENT_DIR`. `stdin` is always connected to `/dev/null`.

---

## 5. Safety & Sandboxing

When `bwrap` (bubblewrap) is installed and user/mount namespaces are permitted
by the host, the **Safety Setting** selector in the dialog footer offers three modes:

- **Read-Only Sandbox** (`sandbox = read-only`):
  ```text
  bwrap --die-with-parent --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp -- <script> [args...]
  ```
  The host filesystem is mounted read-only, with private ephemeral `/tmp` for
  temporary writes. This hides existing host files under `/tmp` from the script.
- **Allow Writing to Folder…** (`sandbox = writable-dir`):
  Same as above, plus `--bind <dir> <dir>` to permit direct writes inside the
  user-selected folder as well.
- **No Sandbox** (`sandbox = none`):
  Executes `<script> [args...]` directly with normal user permissions.

If `bwrap` is unavailable or blocked, **No Sandbox** is the only available
mode. Selecting a script that defaults to `none` or running without bubblewrap
does not bypass confirmation: choose **Run Without Sandbox** in the **Run
without a sandbox?** prompt to proceed.

**Only run scripts you trust.** These modes restrict direct filesystem writes,
not reads, the inherited environment, the network, or desktop session services.
They are not an isolation boundary for untrusted scripts.

---

## 6. Output, Errors & Limits

- **Asynchronous & cancellable:** Scripts run on a background thread. Clicking
  **Cancel** (or closing the dialog) terminates the script process group.
  A background process that holds an output pipe open does not delay
  completion indefinitely; keep output-producing jobs in the foreground.
- **`stdout` → editor window (on exit status `0`):**
  - Must be valid UTF-8, contain no NUL bytes, and not exceed **16 MiB**
    (matching `notediff`'s document limit). Exceeding 16 MiB stops the script
    and reports an error.
  - Output above **1 MiB** opens with a warning toast that Diff Mode may be
    slow.
  - If the invoking window is empty and untitled (outside Diff Mode), it is
    reused in place; otherwise a new editor window is opened. The output opens
    editable with a timestamped unsaved title
    (`<stem>-YYYY-MM-DD-HHMMSS.<ext>`), pre-filled in **Save As**. Closing a
    non-empty output window without saving prompts **Save changes?** like any
    other unsaved document.
  - If `stdout` contains tab characters, the window opens **Read-Only (`RO`)**
    to uphold `notediff`'s no-tab policy without mutating output; unlocking
    **Read Only** triggers the normal **Convert Tabs** prompt.
- **`stderr`:**
  - Captured up to **256 KiB** (further bytes are drained and truncated with
    `… (stderr truncated)` so the child never blocks on a full pipe).
  - On exit status `0`, the **first non-empty line** of `stderr` (up to 140
     characters) is shown as an info toast in the output window unless a
     large-output or tab warning takes precedence. Emit a concise one-line
     summary on `stderr` if you want status feedback in the UI.
  - On any non-zero exit status, the **Run Script** dialog stays open and shows
    an error dialog containing the exit code and captured `stderr` so the user
    can adjust inputs and retry.

---

## 7. Complete Examples

### Minimal Bash script (environment variables only)

```bash
#!/usr/bin/env bash
# --- notediff-script ---
# name = Sort Unique Lines
# description = Sort and deduplicate lines from a chosen text file.
# sandbox = read-only
# output_ext = txt
#
# [field.target]
# label = Input File
# type = file
# required = true
#
# [field.reverse]
# label = Reverse order
# type = bool
# default = false
# --- end-notediff-script ---
set -euo pipefail

args=(-u)
if [[ "${NOTEDIFF_INPUT_REVERSE:-0}" == "1" ]]; then
  args+=(-r)
fi

sort "${args[@]}" -- "$NOTEDIFF_INPUT_TARGET"
echo "Sorted $(basename "$NOTEDIFF_INPUT_TARGET")" >&2
```

### Node.js / JavaScript script (`//` comment prefix)

```javascript
#!/usr/bin/env node
// --- notediff-script ---
// name = Pretty-Print JSON
// description = Format a JSON file with configurable indentation.
// sandbox = read-only
// output_ext = json
//
// [field.file]
// label = JSON File
// type = file
// required = true
// positional = true
//
// [field.indent]
// label = Spaces per indent
// type = slider
// arg = --indent
// options = 2 | 4
// default = 2
// --- end-notediff-script ---

const fs = require("fs");
const args = process.argv.slice(2);
const indentIdx = args.indexOf("--indent");
const spaces = indentIdx >= 0 ? Number(args[indentIdx + 1]) : 2;
const filePath = args[args.length - 1];

const data = JSON.parse(fs.readFileSync(filePath, "utf8"));
process.stdout.write(JSON.stringify(data, null, spaces) + "\n");
console.error(`Formatted ${filePath} (${spaces}-space indent)`);
```

### Shipped Reference Scripts

See the ready-to-use scripts in `examples/scripts/`:

- [`examples/scripts/file-stats.py`](../examples/scripts/file-stats.py) — a
  compact Python template demonstrating `file`, `int`, `bool`, and `choice`
  fields with `argparse`.
- [`examples/scripts/folder-search.py`](../examples/scripts/folder-search.py) —
  a recursive folder search report demonstrating `NOTEDIFF_INPUT_*` and
  `NOTEDIFF_CURRENT_DIR` environment variables with `text`, `dir`, `bool`,
  `slider`, `int`, `list`, and `choice` fields.
- [`examples/scripts/prompt-packer.py`](../examples/scripts/prompt-packer.py) —
  a full-featured script demonstrating `paths`, `list`, `slider`, and `int`
  fields.
