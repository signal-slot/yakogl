#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Builds the SYNTHETIC record directories of glmark2-one-commit-issues-selftest.sh.

    build.py OUTDIR

Every figure below is invented (fixture values, not a measurement). Scene names and
reference pictures come from tools/cts/glmark2-pictures.txt, so the fixtures follow the
table; a scene listed in MISMATCH gets a picture far from the reference. Layout written:
OUTDIR/<case>/records/<board>/common-<sha8>/ (GM, GT run directories, meta.txt, INDEX.txt,
console.txt; the i.MX8MP has glmark2-pages.txt instead of a console, as common-row.py reads).
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TABLE = os.path.join(HERE, "..", "..", "glmark2-pictures.txt")
SCORE = {"am62p": 501, "pi4": 222, "imx8mp": 77}
SLOW = {"am62p": ("80.000", "10.000", "35.000"), "pi4": ("33.000", "9.000", "20.000"),
        "imx8mp": ("250.000", "100.000", "200.000")}


def ref(board):
    out = {}
    for line in open(TABLE, encoding="utf-8"):
        p = [x.strip() for x in line.rstrip("\n").split(" | ", 2)]
        if len(p) == 3 and p[0] == board:
            out[p[1]] = p[2]
    return out


def make(root, board, sha8, mismatch=(), x2=False):
    base = os.path.join(root, "records", board, "common-" + sha8)
    gm = os.path.join(base, "20261201-1000-GM-" + sha8)
    gt = os.path.join(base, "20261201-1020-GT-" + sha8)
    os.makedirs(gm)
    os.makedirs(gt)
    table = ref(board)
    lines = []
    for info in list(table)[:33]:
        sc = info.split("]")[0].lstrip("[")
        ms = dict(zip(("terrain", "shadow", "refract"), SLOW[board])).get(sc, "10.000")
        fps = int(round(1000 / float(ms)))
        pic = table[info]
        if sc in mismatch:
            pic = re.sub(r"mean \d+ \d+ \d+", "mean 250 250 250", pic)
            pic = re.sub(r"probes c [0-9a-f]{8}", "probes c fafafaff", pic)
        lines.append("glmark2-mem: entry 0 before setup: x")
        lines.append("glmark2-mem: entry 0 end: x")
        lines.append("%s: FPS: %d FrameTime: %s ms" % (info, fps, ms))
        lines.append("glmark2-picture: %s: %s" % (info, pic))
    lines.append("glmark2 Score: %d" % SCORE[board])
    lines.append("glmark2: score %d: 33 scenes ran, 0 failed, 0 unsupported" % SCORE[board])
    body = "\n".join(lines) + "\n"
    v = "board.sh %s verdict=done rc=0 result=x record=a,b resets=0" % board
    for d in (gm, gt):
        open(os.path.join(d, "meta.txt"), "w").write("name=x\n")
        open(os.path.join(d, "INDEX.txt"), "w").write("verdict: %s\nGL errors raised 0\n" % v)
    if board == "imx8mp":
        pages = "glmark2-pages: fixture\n" + "\n".join(l for l in lines if "glmark2-mem" not in l) + "\n"
        open(os.path.join(base, "glmark2-pages.txt"), "w").write(pages)
        open(os.path.join(gt, "INDEX.txt"), "a").write(
            "  CASE (1) seq 1: set status 0 (set passed) passed 48 of 48 run; skipped 2, first skip case 3; "
            "failing 0: first case none, second case none\n")
    else:
        open(os.path.join(gm, "console.txt"), "w").write(body)
        open(os.path.join(gt, "console.txt"), "w").write(
            "gl_test: all: 0 of 50 cases failed (50 passed; 0 timed out; 0 skipped)\n")
    if x2:
        open(os.path.join(base, "measured-on"), "w").write("measured-on: armadillo-x2\nnote: fixture\n")


def case(out, name, shas, mismatch=None, x2=False):
    root = os.path.join(out, name)
    for b in ("am62p", "pi4", "imx8mp"):
        make(root, b, shas[b], (mismatch or {}).get(b, ()), x2 and b == "imx8mp")


def main():
    out = sys.argv[1]
    same = {b: "0badf00d" for b in SCORE}
    case(out, "plain", same)
    case(out, "x2", same, x2=True)
    case(out, "mismatch", same, mismatch={"pi4": ("shadow",)})
    case(out, "skew", dict(same, pi4="0badf00e"))


main()
