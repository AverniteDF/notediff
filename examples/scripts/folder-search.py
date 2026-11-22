#!/usr/bin/env python3
# --- notediff-script ---
# name = Folder Search Report
# description = Search UTF-8 text files in a folder and open a Markdown report.
# sandbox = read-only
# output_ext = md
#
# [field.search_query]
# label = Search Query
# type = text
# required = true
#
# [field.folder]
# label = Folder
# type = dir
# help = Leave blank to search the current document's folder.
#
# [field.match_case]
# label = Match Case
# type = bool
# default = false
#
# [field.regular_expression]
# label = Regular Expression
# type = bool
# default = false
#
# [field.context_lines]
# label = Context Lines
# type = slider
# options = 0 | 1 | 2 | 3 | 5
# default = 1
#
# [field.max_matches]
# label = Max Matches
# type = int
# default = 200
# min = 1
# max = 5000
#
# [field.exclude_patterns]
# label = Exclude Patterns
# type = list
# placeholder = *.log, build
# help = Match file or folder names. Separate patterns with commas or newlines.
#
# [field.output_format]
# label = Output Format
# type = choice
# options = grouped | table
# default = grouped
# --- end-notediff-script ---

"""Run from notediff or set NOTEDIFF_INPUT_* variables and run directly.

For example:
    NOTEDIFF_INPUT_SEARCH_QUERY=TODO NOTEDIFF_INPUT_FOLDER=. ./folder-search.py
"""

import os
import re
import stat
import sys
from collections import deque
from fnmatch import fnmatchcase
from pathlib import Path


EXCLUDED_NAMES = {".git", "node_modules", "target", "__pycache__"}
BINARY_CONTROLS = re.compile(r"[\x00-\x08\x0e-\x1f\x7f]")
MARKDOWN_SPECIAL = re.compile(r"([\\`*_{}\[\]<>#!&|~])")
MAX_REPORT_BYTES = 16 * 1024 * 1024


def field(name, default=""):
    return os.environ.get("NOTEDIFF_INPUT_" + name) or default


def search_paths(folder, excludes):
    def excluded(name):
        return name in EXCLUDED_NAMES or any(
            fnmatchcase(name, pattern) for pattern in excludes
        )

    def on_error(error):
        if error.filename == str(folder):
            raise error

    for directory, subdirs, filenames in os.walk(folder, onerror=on_error):
        subdirs[:] = sorted(name for name in subdirs if not excluded(name))
        for name in sorted(filenames):
            if not excluded(name):
                yield Path(directory) / name


def search_file(path, pattern, context, limit):
    """Keep only matches and their context, and validate the whole file."""
    matches = set()
    lines = {}
    previous = deque(maxlen=context)
    context_end = 0
    try:
        if not stat.S_ISREG(path.lstat().st_mode):
            return set(), {}
        with path.open(encoding="utf-8-sig") as source:
            for number, raw_line in enumerate(source, 1):
                if BINARY_CONTROLS.search(raw_line):
                    return set(), {}
                if len(matches) == limit and number > context_end:
                    # Finish checking for binary or malformed UTF-8 data.
                    continue
                line = raw_line.rstrip("\n")
                if len(matches) < limit and pattern.search(line):
                    matches.add(number)
                    lines.update(previous)
                    context_end = number + context
                if number <= context_end:
                    lines[number] = line
                previous.append((number, line))
    except (OSError, UnicodeError):
        return set(), {}
    return matches, lines


def display_text(value):
    value = value.encode("utf-8", errors="backslashreplace").decode("utf-8")
    value = value.expandtabs(4)
    return re.sub(r"[\x00-\x1f\x7f]", lambda m: repr(m[0])[1:-1], value)


def markdown_text(value):
    return MARKDOWN_SPECIAL.sub(r"\\\1", display_text(value))


def report(query, folder, results, total, capped, limit, output_format):
    limit_note = f" (Max Matches limit of {limit} reached)" if capped else ""
    yield "# Folder Search Report\n\n"
    yield f"- **Query:** {markdown_text(query)}\n"
    yield f"- **Folder:** {markdown_text(str(folder))}\n"
    yield f"- **Total matches:** {total}{limit_note}\n\n"

    if not results:
        yield "No matches found.\n"
    elif output_format == "table":
        yield "| File | Line | Text |\n| --- | ---: | --- |\n"
        for path, matches, lines in results:
            filename = markdown_text(path)
            for number in sorted(matches):
                yield f"| {filename} | {number} | {markdown_text(lines[number])} |\n"
    else:
        for path, matches, lines in results:
            yield f"## {markdown_text(path)}\n\n```\n"
            previous_number = min(lines) - 1
            width = len(str(max(lines)))
            for number, line in sorted(lines.items()):
                if number > previous_number + 1:
                    yield "```\n\n```\n"
                marker = ">" if number in matches else " "
                yield f"{marker} {number:>{width}} | {display_text(line)}\n"
                previous_number = number
            yield "```\n\n"


def main():
    query = field("SEARCH_QUERY")
    if not query.strip() or "\n" in query or "\r" in query:
        raise ValueError("Enter a non-empty, single-line Search Query.")

    folder_value = field("FOLDER")
    if not folder_value.strip():
        folder_value = os.environ.get("NOTEDIFF_CURRENT_DIR", "")
    if not folder_value.strip():
        raise ValueError("Choose a Folder or run from a saved document.")
    folder = Path(os.path.abspath(os.path.expanduser(folder_value)))

    output_format = field("OUTPUT_FORMAT", "grouped")
    if output_format not in {"grouped", "table"}:
        raise ValueError("Output Format must be grouped or table.")

    limit_value = field("MAX_MATCHES", "200")
    try:
        limit = int(limit_value)
    except ValueError:
        limit = 0
    if not 1 <= limit <= 5000:
        raise ValueError("Max Matches must be an integer from 1 to 5000.")

    context_value = field("CONTEXT_LINES", "1")
    if context_value not in {"0", "1", "2", "3", "5"}:
        raise ValueError("Context Lines must be 0, 1, 2, 3, or 5.")
    context = int(context_value) if output_format == "grouped" else 0
    flags = 0 if field("MATCH_CASE") == "1" else re.IGNORECASE
    expression = query if field("REGULAR_EXPRESSION") == "1" else re.escape(query)
    try:
        pattern = re.compile(expression, flags)
    except (re.error, OverflowError, RecursionError) as error:
        raise ValueError(f"Invalid regular expression: {error}") from error

    excludes = field("EXCLUDE_PATTERNS").splitlines()
    results = []
    total = 0
    try:
        for path in search_paths(folder, excludes):
            matches, lines = search_file(path, pattern, context, limit - total)
            if matches:
                results.append((path.relative_to(folder).as_posix(), matches, lines))
                total += len(matches)
                if total == limit:
                    break
    except OSError as error:
        raise ValueError(f"Cannot read folder {str(folder)!r}: {error.strerror}") from error

    capped = total == limit
    output = bytearray()
    for part in report(query, folder, results, total, capped, limit, output_format):
        output.extend(part.encode("utf-8"))
        if len(output) > MAX_REPORT_BYTES:
            raise ValueError(
                "Report exceeds notediff's 16 MiB limit. Lower Max Matches or Context Lines."
            )
    sys.stdout.buffer.write(output)

    if total:
        matches_label = "match" if total == 1 else "matches"
        files_label = "file" if len(results) == 1 else "files"
        suffix = " (limit reached)" if capped else ""
        print(f"Found {total} {matches_label} in {len(results)} {files_label}{suffix}",
              file=sys.stderr)
    else:
        print("No matches found.", file=sys.stderr)


if __name__ == "__main__":
    try:
        main()
    except ValueError as error:
        print(display_text(str(error)), file=sys.stderr)
        sys.exit(1)
