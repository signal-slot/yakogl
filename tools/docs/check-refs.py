#!/usr/bin/env python3 -I
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check every `path:line` reference in Markdown files.

usage: python3 -I tools/docs/check-refs.py FILE.md [FILE.md ...]

A reference is a file path (it has an extension) followed by ':N' or ':N-M',
as in `src/gl/state.rs:40-173`. The file must exist (relative to the
repository root, else to the Markdown file's directory) and every line number
must be within the file's length. A bare ':N' with no path in front, as in
"(:57)", is not a reference and is not checked. Exit status 1 when any
reference fails. Standard library only.
"""
import re
import sys
from pathlib import Path

REF = re.compile(r"(?<![\w./-])((?:[\w.-]+/)*[\w-]+\.[A-Za-z]\w*):(\d+)(?:-(\d+))?(?!\w)")


def find_root(start):
    for p in [start, *start.parents]:
        if (p / ".git").exists():
            return p
    return start


def nlines(path):
    with open(path, "rb") as f:
        return sum(1 for _ in f)


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    bad = total = 0
    for name in argv:
        md = Path(name).resolve()
        root = find_root(md.parent)
        cache = {}
        for lineno, text in enumerate(md.read_text(encoding="utf-8").splitlines(), 1):
            for m in REF.finditer(text):
                path, a, b = m.group(1), int(m.group(2)), m.group(3)
                total += 1
                nums = [a] + ([int(b)] if b else [])
                target = next((c for c in (root / path, md.parent / path) if c.is_file()), None)
                err = None
                if target is None:
                    err = "no such file"
                else:
                    n = cache.setdefault(target, nlines(target))
                    if min(nums) < 1 or (b and nums[1] < nums[0]) or max(nums) > n:
                        err = "line out of range (file has %d lines)" % n
                if err:
                    bad += 1
                    print("%s:%d: %s: %s" % (name, lineno, m.group(0), err))
    print("check-refs: %d references, %d bad" % (total, bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
