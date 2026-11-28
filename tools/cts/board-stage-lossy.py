#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Report the done_lossy image rows of a board-stage run.

usage: tools/cts/board-stage-lossy.py FILE_OR_DIR

tools/board.sh ends every image run with a verdict line
    board.sh <board> <image> rc=<n> verdict=<v> ... [lost=<n>]
and board-stage.sh passes those lines on. FILE_OR_DIR is a log of such lines, or
a directory whose regular files (not recursive) are read as such logs. The last
verdict line of an image decides its row. One line is printed per done_lossy
image: "done_lossy <image> lost=<n>" (lost=? when the line carries none).

Exit 0: no done_lossy row. Exit 1: at least one. Exit 2: unreadable input, no
verdict line at all, or a verdict line with an empty verdict.
"""
import os
import re
import sys

VERDICT = re.compile(r"\bboard\.sh \S+ (\S+) .*?\bverdict=(\S*)")
LOST = re.compile(r"\blost=(\d+)\b")


def main(argv):
    if len(argv) != 2:
        sys.stderr.write(__doc__.split("\n\n")[0] + "\n")
        return 2
    path = argv[1]
    try:
        if os.path.isdir(path):
            files = sorted(os.path.join(path, e) for e in os.listdir(path)
                           if os.path.isfile(os.path.join(path, e)))
        else:
            files = [path]
        texts = []
        for f in files:
            with open(f, "rb") as fh:
                texts.append(fh.read().decode("utf-8", "replace"))
    except OSError as e:
        sys.stderr.write("board-stage-lossy: %s\n" % e)
        return 2
    rows = {}
    for text in texts:
        for line in text.splitlines():
            m = VERDICT.search(line)
            if not m:
                continue
            if not m.group(2):
                sys.stderr.write("board-stage-lossy: empty verdict in: %s\n" % line.strip())
                return 2
            lf = LOST.search(line)
            rows[m.group(1)] = (m.group(2), lf.group(1) if lf else "?")
    if not rows:
        sys.stderr.write("board-stage-lossy: no verdict line in %s\n" % path)
        return 2
    lossy = [(n, v[1]) for n, v in rows.items() if v[0] == "done_lossy"]
    for name, lost in sorted(lossy):
        print("done_lossy %s lost=%s" % (name, lost))
    return 1 if lossy else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
