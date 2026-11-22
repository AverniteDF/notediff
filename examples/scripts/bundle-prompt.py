#!/usr/bin/env python3
# --- notediff-script ---
# name = Bundle LLM Prompt
# description = Build a Markdown LLM prompt from files and project folders
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
# [field.prompt]
# label = Prompt Text
# type = multiline
# arg = --prompt
# placeholder = Optional inline instructions (otherwise uses a top-level prompt.txt)
#
# [field.prompt_file]
# label = Prompt File
# type = file
# arg = --prompt-file
# placeholder = Optional path to a prompt text file (mutually exclusive with Prompt Text)
#
# [field.exclude]
# label = Exclude Patterns
# type = list
# arg = --exclude
# placeholder = Glob patterns to skip, one per line or comma-separated (e.g. *.test.js, docs)
#
# [field.max_file_size]
# label = Max File Size
# type = text
# arg = --max-file-size
# default = 1M
#
# [field.max_total_size]
# label = Max Total Size
# type = text
# arg = --max-total-size
# default = 8M
#
# [field.max_files]
# label = Max Files
# type = int
# arg = --max-files
# default = 1500
# min = 1
# max = 100000
#
# [field.verbose]
# label = Verbose details on stderr
# type = bool
# arg = --verbose
# default = false
# --- end-notediff-script ---
"""
bundle-prompt.py - build an LLM prompt from files and folders.

Command-line adaptation of the "LLM Prompt Builder" web app. Point it at
files and/or project folders and it produces a single Markdown prompt:

    <contents of prompt.txt, if present>

    ### Attachments
    **The user has attached N files:**
    - `path/one.py`
    - `path/two.md`
    **Here is the content of each attached file:**
    `path/one.py`:
    ```python
    ...
    ```

The prompt goes to stdout (status information goes to stderr, so pipes are
clean), or to a file (-o) and/or the clipboard (-c).

Requires only Python 3.8+ (standard library).
"""

import argparse
import base64
import fnmatch
import os
import re
import stat
import subprocess
import sys
import unicodedata

__version__ = "1.0.0"

# ---------------------------------------------------------------------------
# Limits (identical to the web app; overridable on the command line)
# ---------------------------------------------------------------------------
MAX_FILE_SIZE = 1 * 1024 * 1024    # per file
MAX_TOTAL_SIZE = 8 * 1024 * 1024   # sum of all inlined file content
MAX_FILES = 1500                   # stop scanning beyond this
BINARY_SNIFF_BYTES = 8192

# ---------------------------------------------------------------------------
# Filters
# ---------------------------------------------------------------------------
# Directories that are never descended into (dependencies, build output,
# caches, IDE state)
IGNORED_DIRS = frozenset([
    '.git', '.svn', '.hg', 'node_modules', 'bower_components',
    'dist', 'build', 'target', 'vendor', 'obj',
    'venv', '.venv', '__pycache__', '.pytest_cache', '.mypy_cache', '.ruff_cache', '.tox', '.nox',
    'coverage', '.nyc_output',
    '.idea', '.vscode', '.vs',
    '.next', '.nuxt', '.svelte-kit', '.angular', '.cache', '.parcel-cache', '.turbo',
    '.gradle', '.terraform', '.serverless',
])

# Files that are noise
IGNORED_FILES = frozenset(['.ds_store', 'thumbs.db', 'desktop.ini'])

# Files likely to contain credentials - excluded from the output entirely
SECRET_NAMES = frozenset([
    'id_rsa', 'id_dsa', 'id_ecdsa', 'id_ed25519',
    '.npmrc', '.netrc', '.pypirc', '.htpasswd', '.envrc', '.secrets',
    'credentials.json', 'secrets.json', 'secrets.yaml', 'secrets.yml', 'service-account.json',
])
SECRET_EXTENSIONS = frozenset(['pem', 'key', 'p12', 'pfx', 'jks', 'keystore', 'ppk'])
_ENV_FILE_RE = re.compile(r'\.env(\..+)?')  # .env, .env.local - not .envrc/.environment

# Lockfiles are listed but their content is replaced with a placeholder
LOCKFILES = frozenset([
    'package-lock.json', 'npm-shrinkwrap.json', 'yarn.lock', 'pnpm-lock.yaml', 'bun.lockb',
    'cargo.lock', 'poetry.lock', 'pipfile.lock', 'uv.lock', 'pdm.lock',
    'composer.lock', 'gemfile.lock', 'go.sum', 'packages.lock.json', 'flake.lock', 'mix.lock',
])

# Known binary extensions - listed but content is replaced with a placeholder
BINARY_EXTENSIONS = frozenset([
    'exe', 'dll', 'so', 'dylib', 'bin', 'o', 'a', 'lib', 'class', 'jar', 'war', 'wasm', 'pdb', 'node',
    'pyc', 'pyo', 'db', 'sqlite', 'sqlite3',
    'zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', 'zst', '7z', 'rar', 'iso', 'dmg', 'pkg', 'deb', 'rpm', 'msi', 'apk', 'ipa',
    'png', 'jpg', 'jpeg', 'gif', 'bmp', 'ico', 'webp', 'tif', 'tiff', 'heic', 'psd', 'ai',
    'mp3', 'wav', 'ogg', 'flac', 'm4a', 'mp4', 'mov', 'avi', 'mkv', 'webm',
    'pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx',
    'woff', 'woff2', 'ttf', 'otf', 'eot',
])

# ---------------------------------------------------------------------------
# Language tags for code fences
# ---------------------------------------------------------------------------
EXTENSION_MAP = {
    'html': 'html', 'htm': 'html', 'xhtml': 'html', 'vue': 'vue', 'svelte': 'svelte',
    'css': 'css', 'scss': 'scss', 'sass': 'sass', 'less': 'less',
    'js': 'javascript', 'mjs': 'javascript', 'cjs': 'javascript', 'jsx': 'jsx',
    'ts': 'typescript', 'mts': 'typescript', 'cts': 'typescript', 'tsx': 'tsx',
    'json': 'json', 'jsonc': 'jsonc', 'json5': 'json5',
    'md': 'markdown', 'markdown': 'markdown', 'rst': 'rst', 'txt': 'text', 'log': 'text',
    'yaml': 'yaml', 'yml': 'yaml', 'toml': 'toml', 'ini': 'ini', 'cfg': 'ini', 'conf': 'ini', 'properties': 'properties',
    'xml': 'xml', 'svg': 'xml', 'xsl': 'xml', 'xsd': 'xml', 'plist': 'xml', 'csproj': 'xml', 'sln': 'text',
    'py': 'python', 'pyi': 'python', 'rb': 'ruby', 'php': 'php', 'pl': 'perl', 'pm': 'perl', 'lua': 'lua',
    'c': 'c', 'h': 'c', 'cpp': 'cpp', 'cc': 'cpp', 'cxx': 'cpp', 'hpp': 'cpp', 'hh': 'cpp', 'hxx': 'cpp',
    'cs': 'csharp', 'java': 'java', 'kt': 'kotlin', 'kts': 'kotlin', 'scala': 'scala', 'groovy': 'groovy',
    'go': 'go', 'rs': 'rust', 'swift': 'swift', 'm': 'objectivec', 'mm': 'objectivec', 'dart': 'dart',
    'zig': 'zig', 'hs': 'haskell', 'ex': 'elixir', 'exs': 'elixir', 'erl': 'erlang', 'clj': 'clojure', 'cljs': 'clojure',
    'r': 'r', 'jl': 'julia', 'tex': 'latex',
    'sh': 'bash', 'bash': 'bash', 'zsh': 'bash', 'fish': 'fish', 'bat': 'batch', 'cmd': 'batch',
    'ps1': 'powershell', 'psm1': 'powershell', 'vbs': 'vbscript', 'vb': 'vb',
    'sql': 'sql', 'graphql': 'graphql', 'gql': 'graphql', 'proto': 'protobuf',
    'tf': 'hcl', 'hcl': 'hcl', 'cmake': 'cmake', 'mk': 'makefile', 'gradle': 'gradle',
    'csv': 'csv', 'tsv': 'tsv', 'diff': 'diff', 'patch': 'diff', 'asm': 'asm', 's': 'asm',
    'hbs': 'handlebars', 'ejs': 'ejs', 'pug': 'pug', 'jinja': 'jinja', 'j2': 'jinja', 'twig': 'twig', 'erb': 'erb',
}
FILENAME_MAP = {
    'dockerfile': 'dockerfile', 'makefile': 'makefile', 'gnumakefile': 'makefile', 'cmakelists.txt': 'cmake',
    'jenkinsfile': 'groovy', 'vagrantfile': 'ruby', 'gemfile': 'ruby', 'rakefile': 'ruby', 'podfile': 'ruby',
    'procfile': 'text', '.bashrc': 'bash', '.zshrc': 'bash', '.bash_profile': 'bash', '.profile': 'bash',
    '.gitignore': 'gitignore', '.dockerignore': 'gitignore', '.gitattributes': 'gitattributes',
    '.editorconfig': 'ini', '.babelrc': 'json', '.eslintrc': 'json', '.prettierrc': 'json',
}


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------
def format_bytes(n):
    if n < 1024:
        return "%d B" % n
    if n < 1024 * 1024:
        return "%.1f KB" % (n / 1024)
    return "%.2f MB" % (n / (1024 * 1024))


def format_count(n):
    if n >= 1000:
        return "%.*fk" % (0 if n >= 10000 else 1, n / 1000)
    return str(n)


def get_file_extension(filename):
    last_dot = filename.rfind('.')
    if last_dot <= 0:
        return ''
    return filename[last_dot + 1:].lower()


def language_for(filename):
    lower = filename.lower()
    if lower in FILENAME_MAP:
        return FILENAME_MAP[lower]
    return EXTENSION_MAP.get(get_file_extension(lower), '')


def normalize_newlines(text):
    return re.sub(r'\r\n?', '\n', text)


def get_fence_level(content):
    """Smallest backtick fence that cannot be closed prematurely by the content."""
    level = 3
    while '`' * level in content:
        level += 1
    return level


def inline_code(text):
    """Inline code span that survives backticks inside the text."""
    runs = re.findall(r'`+', text)
    longest = max((len(r) for r in runs), default=0)
    if longest == 0:
        return '`%s`' % text
    f = '`' * (longest + 1)
    return '%s %s %s' % (f, text, f)


def placeholder(message):
    return '```\n(%s)\n```' % message


def utf16_len(s):
    """Length in UTF-16 code units, matching JavaScript's String.length."""
    return len(s.encode('utf-16-le', errors='surrogatepass')) // 2


def parse_size(text):
    """Parse sizes like 500000, 512K, 2M, 1.5M."""
    m = re.fullmatch(r'\s*([0-9]*\.?[0-9]+)\s*([kKmMgG]?)[bB]?\s*', text)
    if not m:
        raise argparse.ArgumentTypeError("invalid size %r (examples: 500000, 512K, 2M)" % text)
    mult = {'': 1, 'k': 1024, 'm': 1024 ** 2, 'g': 1024 ** 3}[m.group(2).lower()]
    return int(float(m.group(1)) * mult)


# ---------------------------------------------------------------------------
# Filtering
# ---------------------------------------------------------------------------
def dir_skip_reason(name):
    return 'ignored directory' if name.lower() in IGNORED_DIRS else None


def file_skip_reason(name):
    lower = name.lower()
    if lower in IGNORED_FILES:
        return 'ignored file'
    if (_ENV_FILE_RE.fullmatch(lower) or lower in SECRET_NAMES
            or get_file_extension(lower) in SECRET_EXTENSIONS):
        return 'may contain secrets'
    return None


# ---------------------------------------------------------------------------
# Collection
# ---------------------------------------------------------------------------
class Entry(object):
    __slots__ = ('fs_path', 'rel', 'name', 'size')

    def __init__(self, fs_path, rel, name, size):
        self.fs_path = fs_path   # real location on disk
        self.rel = rel           # display path (root-name prefixed, later prefix-stripped)
        self.name = name
        self.size = size


class Collector(object):
    def __init__(self, max_files, excludes, skip_real_paths):
        self.max_files = max_files
        self.excludes = excludes
        self.skip_real_paths = skip_real_paths  # e.g. the -o output file
        self.files = []
        self.skipped = []   # (path, reason)
        self.notes = []     # (path, reason)
        self.truncated = False

    def is_excluded(self, name, rel_in_root):
        for pat in self.excludes:
            if fnmatch.fnmatchcase(name, pat) or fnmatch.fnmatchcase(rel_in_root, pat):
                return True
        return False


def add_file(col, fs_path, rel, rel_in_root, size):
    """Returns False once the file cap has been hit."""
    if len(col.files) >= col.max_files:
        col.truncated = True
        return False
    name = os.path.basename(fs_path)
    reason = file_skip_reason(name)
    if not reason and col.is_excluded(name, rel_in_root):
        reason = 'excluded by --exclude'
    if not reason and col.skip_real_paths and os.path.realpath(fs_path) in col.skip_real_paths:
        reason = 'this is the output file'
    if reason:
        col.skipped.append((rel, reason))
    else:
        col.files.append(Entry(fs_path, rel, name, size))
    return True


def walk_dir(col, dirpath, rel, rel_in_root):
    if col.truncated:
        return
    try:
        names = sorted(os.listdir(dirpath))
    except OSError as err:
        col.skipped.append((rel + '/', 'could not be listed (%s)' % type(err).__name__))
        return

    for name in names:
        if col.truncated:
            return
        full = os.path.join(dirpath, name)
        child_rel = rel + '/' + name
        child_in_root = (rel_in_root + '/' + name) if rel_in_root else name

        try:
            lst = os.lstat(full)
        except OSError as err:
            col.skipped.append((child_rel, 'could not be read (%s)' % type(err).__name__))
            continue

        mode = lst.st_mode
        if stat.S_ISLNK(mode):
            try:
                st = os.stat(full)           # follow the link
            except OSError:
                col.skipped.append((child_rel, 'broken symlink'))
                continue
            if stat.S_ISDIR(st.st_mode):
                col.skipped.append((child_rel + '/', 'symlink to directory (not followed)'))
                continue
            mode = st.st_mode
            size = st.st_size
        else:
            size = lst.st_size

        if stat.S_ISDIR(mode):
            reason = dir_skip_reason(name)
            if not reason and col.is_excluded(name, child_in_root):
                reason = 'excluded by --exclude'
            if reason:
                col.skipped.append((child_rel + '/', reason))
                continue
            walk_dir(col, full, child_rel, child_in_root)
        elif stat.S_ISREG(mode):
            add_file(col, full, child_rel, child_in_root, size)
        else:
            col.skipped.append((child_rel, 'not a regular file'))


def collect(paths, col):
    for arg in paths:
        if not os.path.lexists(arg):
            raise UsageError("no such file or directory: %s" % arg)
        full = os.path.abspath(arg)
        name = os.path.basename(full) or 'root'
        if os.path.isdir(full):
            # A folder named explicitly on the command line is always entered,
            # even if its name is on the ignore list (only descendants are filtered).
            walk_dir(col, full, name, '')
        elif os.path.isfile(full):
            add_file(col, full, name, name, os.stat(full).st_size)
        else:
            raise UsageError("not a regular file or directory: %s" % arg)
        if col.truncated:
            break


# ---------------------------------------------------------------------------
# Content handling
# ---------------------------------------------------------------------------
def has_utf16_bom(b):
    return len(b) >= 2 and ((b[0] == 0xFF and b[1] == 0xFE) or (b[0] == 0xFE and b[1] == 0xFF))


def looks_binary(b):
    if has_utf16_bom(b):
        return False
    sample = b[:BINARY_SNIFF_BYTES]
    n = len(sample)
    if n == 0:
        return False
    suspicious = 0
    for byte in sample:
        if byte == 0:
            return True
        # Control characters other than tab, LF, VT, FF, CR, ESC
        if byte < 0x20 and byte not in (0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x1B):
            suspicious += 1
    return suspicious / n > 0.10


def decode_bytes(b):
    """Returns (text, encoding-name)."""
    if len(b) >= 2:
        if b[0] == 0xFF and b[1] == 0xFE:
            return b[2:].decode('utf-16-le', errors='replace'), 'utf-16le'
        if b[0] == 0xFE and b[1] == 0xFF:
            return b[2:].decode('utf-16-be', errors='replace'), 'utf-16be'
    try:
        return b.decode('utf-8-sig'), 'utf-8'   # strict; also strips a UTF-8 BOM
    except UnicodeDecodeError:
        pass
    try:
        return b.decode('cp1252'), 'windows-1252'
    except UnicodeDecodeError:                  # bytes undefined in cp1252
        return b.decode('latin-1'), 'windows-1252'


class Budget(object):
    def __init__(self, max_file_size, max_total_size):
        self.max_file_size = max_file_size
        self.max_total_size = max_total_size
        self.used = 0
        self.included = 0
        self.omitted = 0
        self.over_budget = 0


def render_attachment(item, budget, col):
    """Returns the fenced content (or a placeholder) for one attachment."""
    lower_name = item.name.lower()

    if lower_name in LOCKFILES:
        budget.omitted += 1
        return placeholder('Lockfile omitted for brevity.')
    if item.size > budget.max_file_size:
        budget.omitted += 1
        return placeholder('File exceeds the %s per-file limit [%s]. Omitted for brevity.'
                           % (format_bytes(budget.max_file_size), format_bytes(item.size)))
    if get_file_extension(lower_name) in BINARY_EXTENSIONS:
        budget.omitted += 1
        return placeholder('Binary file. Omitted for brevity.')
    if budget.used + item.size > budget.max_total_size:
        budget.omitted += 1
        budget.over_budget += 1
        return placeholder('Omitted: the %s total content limit was reached.'
                           % format_bytes(budget.max_total_size))

    try:
        with open(item.fs_path, 'rb') as fh:
            data = fh.read()
    except OSError as err:
        budget.omitted += 1
        col.notes.append((item.rel, 'could not be read (%s)' % type(err).__name__))
        return placeholder('File could not be read. Omitted.')

    if looks_binary(data):
        budget.omitted += 1
        return placeholder('This file appears to contain binary data. Omitted for brevity.')

    budget.used += len(data)
    budget.included += 1

    text, encoding = decode_bytes(data)
    if encoding != 'utf-8':
        col.notes.append((item.rel, 'decoded as %s' % encoding))

    # Only normalize line endings and drop trailing whitespace at end-of-file.
    # Leading whitespace and per-line trailing whitespace are preserved verbatim.
    content = normalize_newlines(text).rstrip() or '(blank file)'
    fence = '`' * get_fence_level(content)
    lang = language_for(item.name)
    return '%s%s\n%s\n%s' % (fence, lang, content, fence)


# ---------------------------------------------------------------------------
# Path handling
# ---------------------------------------------------------------------------
def strip_common_folder_prefix(items):
    if not items:
        return

    if len(items) == 1:
        items[0].rel = items[0].rel.split('/')[-1]
        return

    split_paths = [i.rel.split('/') for i in items]
    if any(len(p) < 2 for p in split_paths):
        return

    common = 0
    while True:
        if common >= len(split_paths[0]):
            break
        segment = split_paths[0][common]
        # Every path must keep at least its filename
        if any(len(p) <= common + 1 for p in split_paths):
            break
        if not all(p[common] == segment for p in split_paths):
            break
        common += 1

    if common > 0:
        for item, parts in zip(items, split_paths):
            item.rel = '/'.join(parts[common:])


def _fold(s):
    s = unicodedata.normalize('NFD', s)
    return ''.join(c for c in s if not unicodedata.combining(c)).casefold()


# Order of ASCII punctuation/symbols in the Unicode root collation (the order
# the browser's Intl.Collator uses): all of these sort before digits, which
# sort before letters.
_PUNCT_RANK = {c: i for i, c in enumerate("_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$")}


def _natural_key(segment):
    """Collation key approximating Intl.Collator(sensitivity='base', numeric=true):
    case- and accent-insensitive, digit runs compared by value,
    punctuation < digits < letters."""
    s = _fold(segment)
    key = []
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if '0' <= c <= '9':
            j = i
            while j < n and '0' <= s[j] <= '9':
                j += 1
            key.append((1, int(s[i:j]), ''))
            i = j
            continue
        if c in _PUNCT_RANK:
            key.append((0, _PUNCT_RANK[c], ''))
        elif c.isspace():
            key.append((0, -1, ''))
        elif unicodedata.category(c)[0] in 'PSZ':
            key.append((0, 100 + ord(c), ''))
        else:
            key.append((2, 0, c))
        i += 1
    return key


def path_sort_key(rel):
    """Files before sub-directories at each level, then natural,
    case/accent-insensitive order. Names that compare equal keep their scan
    order (Python's sort is stable), as in the web app."""
    segs = rel.split('/')
    last = len(segs) - 1
    return [(0 if i == last else 1, _natural_key(seg)) for i, seg in enumerate(segs)]


# ---------------------------------------------------------------------------
# Main pipeline
# ---------------------------------------------------------------------------
class UsageError(Exception):
    pass


class ToolError(Exception):
    pass


def read_text_file(path):
    with open(path, 'rb') as fh:
        return decode_bytes(fh.read())[0]


def build_prompt(col, budget, explicit_prompt):
    entries = col.files

    # prompt.txt only counts if it sits at the top level of what was given
    # (either passed directly, or directly inside a given folder).
    prompt_entry = None
    if explicit_prompt is None:
        candidates = [e for e in entries
                      if e.name.lower() == 'prompt.txt' and len(e.rel.split('/')) <= 2]
        if len(candidates) > 1:
            raise ToolError("More than one top-level 'prompt.txt' was found.")
        prompt_entry = candidates[0] if candidates else None
    attachments = [e for e in entries if e is not prompt_entry]

    if not attachments:
        raise ToolError('No attachments found. Give files or folders (optionally alongside a prompt.txt).')

    strip_common_folder_prefix(attachments)
    attachments.sort(key=lambda e: path_sort_key(e.rel))

    prompt_text = '(Insert your prompt here)'
    if explicit_prompt is not None:
        prompt_text = normalize_newlines(explicit_prompt).strip() or prompt_text
    elif prompt_entry is not None:
        try:
            text = read_text_file(prompt_entry.fs_path)
        except OSError as err:
            raise ToolError("Could not read prompt.txt (%s)." % type(err).__name__)
        prompt_text = normalize_newlines(text).strip() or prompt_text

    n = len(attachments)
    lines = [
        prompt_text,
        '', '',
        '### Attachments',
        '',
        '**The user has attached %d file%s:**' % (n, '' if n == 1 else 's'),
    ]

    if n > 1:
        for item in attachments:
            lines.append('- ' + inline_code(item.rel))
        lines += ['', '**Here is the content of each attached file:**']

    for item in attachments:
        body = render_attachment(item, budget, col)
        lines += ['', inline_code(item.rel) + ':', body]

    return '\n'.join(lines), n


def report_rows(col, budget, max_files):
    """Returns (rows, n_warnings). The first n_warnings rows are the ones
    that always deserve the user's attention (truncation / budget exhausted)."""
    rows = []
    if col.truncated:
        rows.append(('', 'Scanning stopped after %d files - remaining files were not included.' % max_files))
    if budget is not None and budget.over_budget:
        rows.append(('', '%d file(s) omitted because the %s total content limit was reached.'
                     % (budget.over_budget, format_bytes(budget.max_total_size))))
    n_warnings = len(rows)
    rows += col.skipped
    rows += col.notes
    return rows, n_warnings


# ---------------------------------------------------------------------------
# Output: stderr messages, clipboard
# ---------------------------------------------------------------------------
class Console(object):
    COLORS = {'ok': '32', 'warn': '33', 'error': '31'}

    def __init__(self, quiet, color):
        self.quiet = quiet
        self.color = color

    def _paint(self, text, kind):
        if self.color and kind in self.COLORS:
            return '\033[%sm%s\033[0m' % (self.COLORS[kind], text)
        return text

    def status(self, text, kind=''):
        if not self.quiet:
            print(self._paint(text, kind), file=sys.stderr)

    def error(self, text):
        print(self._paint('Error: ' + text, 'error'), file=sys.stderr)

    def details(self, rows):
        print('Details (%d):' % len(rows), file=sys.stderr)
        for path, reason in rows:
            print('  - %s - %s' % (path, reason) if path else '  - %s' % reason, file=sys.stderr)


def _which(cmd):
    for d in os.environ.get('PATH', '').split(os.pathsep):
        p = os.path.join(d, cmd)
        if os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    return None


def copy_to_clipboard(text):
    """Try the usual Linux clipboard tools, then the OSC 52 terminal escape.
    Returns a short description of the method used, or None on failure."""
    data = text.encode('utf-8')

    candidates = []
    if os.environ.get('WAYLAND_DISPLAY'):
        candidates.append(('wl-copy', ['wl-copy']))
    if os.environ.get('DISPLAY'):
        candidates.append(('xclip', ['xclip', '-selection', 'clipboard']))
        candidates.append(('xsel', ['xsel', '--clipboard', '--input']))
    # Environments with only one of the two variables set still get a fair try
    if not os.environ.get('WAYLAND_DISPLAY'):
        candidates.append(('wl-copy', ['wl-copy']))

    for label, cmd in candidates:
        if _which(cmd[0]) is None:
            continue
        try:
            # DEVNULL for stdout/stderr: these tools fork into the background to
            # serve the selection and would otherwise keep our pipes open.
            res = subprocess.run(cmd, input=data, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL, timeout=10)
            if res.returncode == 0:
                return label
        except (OSError, subprocess.SubprocessError):
            continue

    # Fallback: OSC 52 - asks the terminal emulator itself to set the clipboard.
    try:
        with open('/dev/tty', 'wb', buffering=0) as tty:
            tty.write(b'\033]52;c;' + base64.b64encode(data) + b'\a')
        return 'terminal escape sequence (OSC 52)'
    except OSError:
        return None


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------
EPILOG = """\
How it works:
  Files and folders are scanned recursively. Dependency/build/cache folders
  (node_modules, .git, dist, build, venv, ...) are skipped, as are files that
  look like they hold credentials (.env*, *.pem, *.key, id_rsa, ...). Lockfiles
  and binaries are listed but their contents are omitted. A top-level
  prompt.txt (passed directly, or directly inside a given folder) becomes the
  prompt text at the top of the output instead of being attached.

Examples:
  bundle-prompt.py -c ~/projects/myapp          # copy prompt to the clipboard
  bundle-prompt.py -o out.md src/ README.md     # write to a file
  bundle-prompt.py -p "Find the bug" main.py    # supply the prompt inline
  bundle-prompt.py . | wc -c                    # stdout is clean for piping
  bundle-prompt.py -x '*.test.js' -x docs .     # skip extra files/folders
"""


def build_parser():
    p = argparse.ArgumentParser(
        prog='bundle-prompt.py',
        description='Build an LLM prompt from files and project folders.',
        epilog=EPILOG,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument('paths', nargs='*', metavar='PATH', help='files and/or folders to include')

    out = p.add_argument_group('output')
    out.add_argument('-o', '--output', metavar='FILE',
                     help="write the prompt to FILE ('-' means stdout). "
                          "Without -o or -c the prompt goes to stdout")
    out.add_argument('-c', '--copy', action='store_true', help='copy the prompt to the clipboard')
    out.add_argument('-q', '--quiet', action='store_true', help='suppress the status summary on stderr')
    out.add_argument('-v', '--verbose', action='store_true',
                     help='list every skipped/omitted file and note on stderr')
    out.add_argument('--no-color', action='store_true', help='disable colored status output')

    pr = p.add_argument_group('prompt text (default: top-level prompt.txt, else a placeholder)')
    g = pr.add_mutually_exclusive_group()
    g.add_argument('-p', '--prompt', metavar='TEXT',
                   help='prompt text; a prompt.txt in the input is then attached like any other file')
    g.add_argument('-P', '--prompt-file', metavar='FILE',
                   help="read the prompt text from FILE ('-' means stdin)")

    flt = p.add_argument_group('filtering and limits')
    flt.add_argument('-x', '--exclude', action='append', default=[], metavar='GLOB',
                     help='skip files/folders whose name or path (relative to its PATH argument) '
                          'matches GLOB; repeatable')
    flt.add_argument('--max-file-size', type=parse_size, default=MAX_FILE_SIZE, metavar='SIZE',
                     help='per-file content limit (default: 1M)')
    flt.add_argument('--max-total-size', type=parse_size, default=MAX_TOTAL_SIZE, metavar='SIZE',
                     help='total content limit (default: 8M)')
    flt.add_argument('--max-files', type=int, default=MAX_FILES, metavar='N',
                     help='stop scanning after N files (default: %d)' % MAX_FILES)

    p.add_argument('--version', action='version', version='%(prog)s ' + __version__)
    return p


def run(args, console):
    # Explicit prompt text, if any
    explicit_prompt = None
    if args.prompt is not None:
        explicit_prompt = args.prompt
    elif args.prompt_file is not None:
        try:
            if args.prompt_file == '-':
                explicit_prompt = decode_bytes(sys.stdin.buffer.read())[0]
            else:
                explicit_prompt = read_text_file(args.prompt_file)
        except OSError as err:
            raise UsageError("cannot read prompt file %s (%s)" % (args.prompt_file, err.strerror or type(err).__name__))

    # Don't ingest our own output file if it lives inside a scanned folder
    skip_real = set()
    if args.output and args.output != '-':
        skip_real.add(os.path.realpath(args.output))

    excludes = [e.rstrip('/') for e in args.exclude if e.rstrip('/')]
    col = Collector(args.max_files, excludes, skip_real)

    collect(args.paths, col)

    if not col.files:
        raise ToolError('No eligible files found - everything was skipped (see -v for details).'
                        if col.skipped else 'No readable files found in the selection.')

    budget = Budget(args.max_file_size, args.max_total_size)
    output, n = build_prompt(col, budget, explicit_prompt)

    chars = utf16_len(output)
    tokens = (chars + 2) // 4   # round(chars / 4), halves rounded up
    summary = 'Processed %d attachment%s · %s characters (~%s tokens)' % (
        n, '' if n == 1 else 's', format(chars, ','), format_count(tokens))
    if budget.omitted:
        summary += ' · %d omitted' % budget.omitted
    if col.skipped:
        summary += ' · %d skipped' % len(col.skipped)
    warn = bool(budget.over_budget or col.truncated)

    rows, n_warnings = report_rows(col, budget, args.max_files)
    return output, summary, ('warn' if warn else 'ok'), rows, n_warnings


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    if not args.paths:
        parser.error('at least one file or folder is required')
    if args.max_files < 1:
        parser.error('--max-files must be at least 1')

    use_color = sys.stderr.isatty() and not args.no_color and 'NO_COLOR' not in os.environ
    console = Console(args.quiet, use_color)

    try:
        output, summary, kind, rows, n_warnings = run(args, console)
    except UsageError as err:
        parser.error(str(err))      # exits with status 2
    except ToolError as err:
        console.error(str(err))
        return 1
    except KeyboardInterrupt:
        console.error('interrupted')
        return 130

    # Deliver the result ------------------------------------------------------
    to_stdout = args.output == '-' or (not args.output and not args.copy)
    status_ok = True

    if args.output and args.output != '-':
        try:
            with open(args.output, 'w', encoding='utf-8', newline='\n') as fh:
                fh.write(output + '\n')
        except OSError as err:
            console.error('cannot write %s (%s)' % (args.output, err.strerror or type(err).__name__))
            return 1

    if args.copy:
        method = copy_to_clipboard(output)
        if method is None:
            console.error('failed to copy to clipboard. Install wl-clipboard '
                          '(sudo apt install wl-clipboard) or use -o FILE.')
            status_ok = False
        else:
            console.status('✓ Copied to clipboard via %s' % method, 'ok')
            if method.startswith('terminal') and len(output) > 100000:
                console.status('Note: terminals often ignore very large OSC 52 payloads; '
                               'paste to check, or install wl-clipboard.', 'warn')

    if to_stdout:
        try:
            sys.stdout.buffer.write((output + '\n').encode('utf-8'))
            sys.stdout.buffer.flush()
        except BrokenPipeError:
            # Downstream (e.g. `| head`) closed early; exit quietly.
            try:
                sys.stdout.close()
            except Exception:
                pass
            devnull = os.open(os.devnull, os.O_WRONLY)
            os.dup2(devnull, sys.stdout.fileno())

    # Status (stderr) ----------------------------------------------------------
    console.status(summary, kind)
    if args.verbose:
        if rows:
            console.details(rows)
    elif rows:
        # Always surface the important warnings; the rest is behind -v
        for _, reason in rows[:n_warnings]:
            console.status('Warning: ' + reason, 'warn')
        console.status('(%d detail%s - run with -v to list)' % (len(rows), '' if len(rows) == 1 else 's'))

    return 0 if status_ok else 1


if __name__ == '__main__':
    sys.exit(main())
