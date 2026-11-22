#!/usr/bin/env python3
# --- notediff-script ---
# name = File Word Statistics
# description = Summarize line, word, byte counts and top words for a text file
# sandbox = read-only
# output_ext = md
#
# [field.target]
# label = Target File
# type = file
# required = true
# positional = true
# help = Choose a UTF-8 text file to analyze
#
# [field.top]
# label = Top Words
# type = int
# arg = --top
# default = 15
# min = 1
# max = 200
#
# [field.case_sensitive]
# label = Case-sensitive word matching
# type = bool
# arg = --case-sensitive
# default = false
#
# [field.format]
# label = Output Format
# type = choice
# arg = --format
# options = markdown | plain
# default = markdown
# --- end-notediff-script ---

import argparse
from collections import Counter
import os
import re
import sys


def main():
    parser = argparse.ArgumentParser(description="Summarize basic statistics for a text file.")
    parser.add_argument("target", help="Text file to analyze")
    parser.add_argument("--top", type=int, default=15, help="Number of top words to show")
    parser.add_argument("--case-sensitive", action="store_true", help="Keep original word casing")
    parser.add_argument("--format", choices=["markdown", "plain"], default="markdown")
    args = parser.parse_args()

    try:
        with open(args.target, "r", encoding="utf-8") as fh:
            text = fh.read()
    except OSError as err:
        print(f"Could not read {args.target}: {err}", file=sys.stderr)
        return 1
    except UnicodeDecodeError:
        print(f"{args.target} is not valid UTF-8.", file=sys.stderr)
        return 1

    lines = text.splitlines()
    raw_words = re.findall(r"[A-Za-z0-9_']+", text)
    words = raw_words if args.case_sensitive else [w.lower() for w in raw_words]
    counts = Counter(words)
    top = counts.most_common(args.top)
    byte_count = len(text.encode("utf-8"))
    name = os.path.basename(args.target)

    if args.format == "markdown":
        print(f"# File Statistics: `{name}`\n")
        print(f"- **Path:** `{os.path.abspath(args.target)}`")
        print(f"- **Lines:** {len(lines):,}")
        print(f"- **Words:** {len(words):,} ({len(counts):,} unique)")
        print(f"- **Characters:** {len(text):,}")
        print(f"- **Bytes:** {byte_count:,}\n")
        print(f"## Top {len(top)} Words\n")
        print("| Rank | Word | Count |")
        print("| ---: | --- | ---: |")
        for rank, (word, count) in enumerate(top, 1):
            print(f"| {rank} | `{word}` | {count:,} |")
    else:
        print(f"File: {os.path.abspath(args.target)}")
        print(f"Lines: {len(lines):,}")
        print(f"Words: {len(words):,} ({len(counts):,} unique)")
        print(f"Characters: {len(text):,}")
        print(f"Bytes: {byte_count:,}\n")
        print(f"Top {len(top)} words:")
        for rank, (word, count) in enumerate(top, 1):
            print(f"  {rank:>3}. {word}: {count:,}")

    print(f"Analyzed {name}: {len(lines):,} lines, {len(words):,} words", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
