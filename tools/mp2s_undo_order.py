#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Does a dry-run transcript print every undo before its change?

    tools/board-stm32mp2-standalone.sh --dry-run <verb> ... 2>&1 | tools/mp2s_undo_order.py [FILE]

Reads the transcript of tools/board-stm32mp2-standalone.sh --dry-run (stdin
or FILE) and checks docs/notes/stm32mp257f-dk-standalone.md, sections 5 and 7:
each change to the board (a `DRY board$` line that makes or removes a file
under /boot, or runs fw_setenv) must be preceded by the undo line of that
change:

  mkdir of the stage directory  <- "UNDO: ... standalone.sh uninstall"
  rm -rf of the stage directory <- "UNDO: ... standalone.sh install"
  default.bin written, or the environment written with mp2s_left=
                                <- "UNDO, from Linux ...: ... nodefault"
  nodefault's fw_setenv / rm of default.bin
                                <- "nothing to undo: nodefault only removes"

Reads (fw_printenv, ls) are not changes. A change of a kind this checker does
not know fails: a new change needs its undo named here first.

Exit 0 ok, 1 an undo missing or late, 2 usage or an empty transcript.
"""
import re
import sys

UNDO = [
    ("install", re.compile(r"\bUNDO: \S*board-stm32mp2-standalone\.sh uninstall\b")),
    ("uninstall", re.compile(r"\bUNDO: \S*board-stm32mp2-standalone\.sh install\b")),
    ("default", re.compile(r"\bUNDO, from Linux\b.*standalone\.sh nodefault\b")),
    ("nodefault", re.compile(r"\bnothing to undo: nodefault only removes\b")),
]
BOARD = re.compile(r"^DRY board\$ (.*)$")
WRITER = re.compile(r"(?:^|[\s;&|])(mkdir|tee|mv|rm|fw_setenv)\b")


def kind(cmd):
    """The undo a change needs; None for a read, 'unknown' for a change not known."""
    code = cmd.split("   #", 1)[0]
    words = WRITER.findall(code)
    if not words:
        return None
    if "mkdir" in words:
        return "install"
    if "rm" in words and "-rf" in code:
        return "uninstall"
    if "fw_setenv" in words:
        return "default" if "mp2s_left=" in cmd else "nodefault"
    if "rm" in words and "default.bin" in code:
        return "nodefault"
    if "tee" in words or "mv" in words:
        return "default" if "default.bin" in code else "unknown"
    return "unknown"


def check(lines):
    seen = set()
    bad = []
    changes = 0
    for n, line in enumerate(lines, 1):
        line = line.rstrip("\n")
        for name, rx in UNDO:
            if rx.search(line):
                seen.add(name)
        m = BOARD.match(line)
        if not m:
            continue
        k = kind(m.group(1))
        if k is None:
            continue
        changes += 1
        if k == "unknown":
            bad.append((n, "a change with no known undo: " + m.group(1)))
        elif k not in seen:
            bad.append((n, "change before its undo (%s): %s" % (k, m.group(1))))
    return changes, bad


def main(argv):
    if len(argv) > 2 or (len(argv) == 2 and argv[1].startswith("-")):
        sys.stderr.write(__doc__.split("\n\n")[0] + "\n")
        return 2
    if len(argv) == 2:
        with open(argv[1], encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
    else:
        lines = sys.stdin.readlines()
    if not any(BOARD.match(x) for x in lines):
        sys.stderr.write("mp2s_undo_order: no 'DRY board$' line in the transcript\n")
        return 2
    changes, bad = check(lines)
    for n, why in bad:
        print("line %d: %s" % (n, why))
    if bad:
        return 1
    print("undo order ok: %d change(s), each after its undo" % changes)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
