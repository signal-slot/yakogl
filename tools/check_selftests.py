#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The tracked self-tests that no row of tools/check.sh runs.

usage: tools/check_selftests.py

A self-test is a tracked file whose name ends in `selftest.sh` or
`selftest.py` (third_party/ aside). The gate runs it when a runner names it:
tools/check.sh, or a self-test that runs others as its checks (RUNNERS). A
comment names nothing, and neither does scope_of_path() of tools/check.sh: it
maps a path to the rows a change of it reaches and runs none. A file is named
by the shortest end of its path that no other self-test shares: its file name,
or `pr-configs/selftest.sh` for one of several `selftest.sh`.

Prints the self-tests no runner names, one path per line, sorted.
tools/check.sh gives each of them a row, so a self-test is in the gate from the
commit that adds it. A self-test that runs others and is not in RUNNERS costs
a second run of those; it never leaves one out.
"""
import re
import subprocess
import sys
from pathlib import Path

GATE = "tools/check.sh"
RUNNERS = (
    GATE,
    "tools/cts/cts-selftest.sh",
    "driver/tools/kexec-zephyr/board_lock_selftest.sh",
)
SELFTEST = re.compile(r"selftest\.(sh|py)$")
# the gate's function that names paths without running them, to its closing brace
SCOPE_MAP = re.compile(r"^scope_of_path\(\) \{\n.*?^\}\n", re.M | re.S)
# what may stand next to a name without being part of a longer one
WORD = r"[A-Za-z0-9_.-]"


def selftests():
    out = subprocess.run(["git", "ls-files", "-z"], check=True, capture_output=True, text=True).stdout
    return sorted(p for p in out.split("\0") if SELFTEST.search(p) and not p.startswith("third_party/"))


def name_of(path, paths):
    """The shortest end of `path`, in whole components, that only `path` has."""
    parts = path.split("/")
    for n in range(1, len(parts)):
        end = "/".join(parts[-n:])
        if sum(p == end or p.endswith("/" + end) for p in paths) == 1:
            return end
    return path


def code(path):
    """What the runner at `path` can run: its lines that are not comments."""
    text = Path(path).read_text(errors="replace")
    if path == GATE:
        text, maps = SCOPE_MAP.subn("", text)
        if maps != 1:
            sys.exit(f"check_selftests.py: {GATE} has {maps} scope_of_path() functions, not 1")
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))


def unnamed():
    paths = selftests()
    runners = "\n".join(code(r) for r in RUNNERS)
    return [p for p in paths if not re.search(f"(?<!{WORD}){re.escape(name_of(p, paths))}(?!{WORD})", runners)]


if __name__ == "__main__":
    if len(sys.argv) != 1:
        sys.exit(__doc__)
    print("\n".join(unnamed()))
