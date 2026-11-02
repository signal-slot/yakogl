#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Regenerate the fixture slices next to this file (made-up data, not a board's).

    python3 tools/cts/testdata/am62p-gles3-merge/mkfixtures.py

selftest.py runs the committed files; this only documents how they were made.
"""
import os

os.chdir(os.path.dirname(os.path.abspath(__file__)))
P = "dEQP-GLES3.t."
T = "\t"


def w(path, text):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def v(r, verdict="done", resets="0"):
    return ("row %d: board.sh am62p fix-r%d rc=0 verdict=%s boot=uboot-mmc row boot_verdict=ok "
            "result=legacy:deqp_summary resets=%s record=absent back_after=40s\n" % (r, r, verdict, resets))


def sl(kind, sha, lo, hi, rows, extra=""):
    d = "%s/2026-11-01-am62p-gles3-%s-rows%02d-%02d" % (kind, sha, lo, hi)
    w(d + "/README.md", "# fixture slice (made-up data)\ncommit: %s\n%s%s" % (sha, "".join(rows), extra))
    return d


w("mustpass.txt", "".join(P + c + "\n" for c in ("a.x", "a.y", "b.x", "b.y", "c.x", "d.x")))
c12 = P + "a.x" + T + "Pass\n" + P + "a.y" + T + "Fail" + T + "Mismatch at pixel 3\n" + P + "zz" + T + "Pass\n"
c34 = (P + "b.x" + T + "NotSupported" + T + "Extension missing\n" + P + "b.y" + T + "Pass\n"
       + P + "c.x" + T + "Crash\n")
w(sl("good", "aaaaaaaa", 1, 2, [v(1), v(2)]) + "/cases.tsv", c12)
for kind, sha, extra, rows in (
        ("good", "aaaaaaaa", "", [v(3), v(4)]),
        ("mixed", "bbbbbbbb", "", [v(3), v(4)]),
        ("missing", "aaaaaaaa", "", [v(3)]),
        ("ip", "aaaaaaaa", "board console at 10.1.2.3 via /dev/ttyUSB0\n", [v(3), v(4)]),
        ("reset", "aaaaaaaa", "", [v(3), v(4, "done", "2")])):
    w(sl(kind, sha, 3, 4, rows, extra) + "/cases.tsv", c34)
w("old.tsv", "# made-up previous list\n"
  + "".join("%s%s%s%s%s\n" % (o, T, P + c, T, m) for o, c, m in
            (("Fail", "a.y", "Fail"), ("Fail", "b.y", "Fail"), ("NoResult", "d.x", ""))))
