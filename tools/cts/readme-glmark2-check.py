#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""readme-glmark2-check.py - the README's glmark2 table names one main commit.

README.md's "glmark2-es2 at 800x600" table gives a score and a commit per
board. A score is only comparable across boards when all three were measured
at the same main commit, and only checkable when tools/cts/RESULTS.md holds a
glmark2 row for each board at that commit. This checks exactly that:

  * every board (AM62P, Raspberry Pi 4, i.MX8MP) has a row in the table;
  * the rows name one commit (short hashes match by prefix);
  * the commit is an ancestor of --ref (git merge-base --is-ancestor; skipped
    with a warning when git or the ref is unavailable, and with --ref none);
  * RESULTS.md's glmark2 table has a row for each board citing that commit;
  * with --scenes (off by default), each of those rows also gives a number of
    ms a frame for every named scene and cites a record directory.

--scenes terrain,shadow,refract reads a scene figure as "<scene> <n> ms" (the
row text, markdown bold ignored). Both row styles match: glmark2-ms.py --cells
prints "terrain 178.86 ms (6 FPS)", and RESULTS.md rows have "terrain 250 ms
(4 FPS)". A record directory is records/<board>/<YYYYMMDD-HHMM>-<name>-<sha8>/
with <board> pi4, imx8mp or am62p and <sha8> the common commit's first 8
characters (by prefix).

The live README fails this by design until its boards are re-measured at one
commit, so the gate runs the check on fixtures only (cts-selftest.sh).

Usage:
  python3 tools/cts/readme-glmark2-check.py [--readme F] [--results F] [--ref R]
                                                [--scenes terrain,shadow,refract]
Exit status: 0 passes, 1 fails, 2 a file or table could not be parsed.
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))

# normalised first cell (lowercase, letters and digits only) -> board
BOARDS = {
    "am62p": "AM62P",
    "raspberrypi4": "Raspberry Pi 4",
    "pi4": "Raspberry Pi 4",
    "imx8mp": "i.MX8MP",
}
RECORD_DIR = {"AM62P": "am62p", "Raspberry Pi 4": "pi4", "i.MX8MP": "imx8mp"}
WANT = ["AM62P", "Raspberry Pi 4", "i.MX8MP"]
HASH = re.compile(r"`([0-9a-f]{7,40})`")


def norm(cell):
    return re.sub(r"[^a-z0-9]", "", cell.lower())


def table_rows(path, header_re):
    """Rows (lists of cells) of the first table whose header matches header_re."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as e:
        print("error: %s: %s" % (path, e.strerror))
        sys.exit(2)
    start = None
    for i, ln in enumerate(lines):
        if ln.startswith("|") and re.search(header_re, ln, re.I):
            start = i
            break
    if start is None:
        print("error: %s: no table with a header matching %r" % (path, header_re))
        sys.exit(2)
    rows = []
    for ln in lines[start + 1:]:
        if not ln.startswith("|"):
            break
        cells = [c.strip() for c in ln.strip().strip("|").split("|")]
        if cells and not re.fullmatch(r"[-: ]+", cells[0]):
            rows.append(cells)
    return rows


def board_hashes(rows):
    out = {}
    for cells in rows:
        b = BOARDS.get(norm(cells[0]))
        if b:
            out[b] = HASH.findall(" ".join(cells[1:]))
    return out


def board_text(rows):
    out = {}
    for cells in rows:
        b = BOARDS.get(norm(cells[0]))
        if b:
            out[b] = " ".join(cells[1:]).replace("*", "")
    return out


def same(a, b):
    return a.startswith(b) or b.startswith(a)


def scene_fails(board, text, scenes, commit):
    fails = []
    for sc in scenes:
        m = re.search(r"\b%s\b\s+(\S+)\s+ms\b" % re.escape(sc), text, re.I)
        if not m:
            fails.append("RESULTS: the %s glmark2 row has no '%s <n> ms' figure" % (board, sc))
        elif not re.fullmatch(r"\d+(?:\.\d+)?", m.group(1)):
            fails.append("RESULTS: the %s glmark2 row has a non-numeric %s figure '%s'" % (
                board, sc, m.group(1)))
    recs = re.findall(r"records/%s/\d{8}-\d{4}-[A-Za-z0-9._-]+-([0-9a-f]{8})/?(?![\w-])" %
                      RECORD_DIR[board], text)
    if not recs:
        fails.append("RESULTS: the %s glmark2 row cites no record directory "
                     "records/%s/<YYYYMMDD-HHMM>-<name>-<sha8>/" % (board, RECORD_DIR[board]))
    elif not any(same(commit, r) for r in recs):
        fails.append("RESULTS: the %s glmark2 row's record directories (%s) are not at `%s`" % (
            board, ", ".join(recs), commit))
    return fails


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--readme", default=os.path.join(ROOT, "README.md"))
    ap.add_argument("--results", default=os.path.join(HERE, "RESULTS.md"))
    ap.add_argument("--ref", default="origin/main",
                    help="the commit must be its ancestor ('none' skips)")
    ap.add_argument("--scenes", default="",
                    help="comma list of scenes (e.g. terrain,shadow,refract): each "
                    "board's RESULTS row also needs '<scene> <n> ms' and a record dir")
    a = ap.parse_args()
    scenes = [x.strip() for x in a.scenes.split(",") if x.strip()]

    readme = board_hashes(table_rows(a.readme, r"\|\s*Board\s*\|.*Score"))
    rrows = table_rows(a.results, r"\|\s*Board\s*\|.*glmark2")
    results = board_hashes(rrows)
    rtext = board_text(rrows)
    fails = []

    cited = {}
    for b in WANT:
        if b not in readme:
            fails.append("README: no glmark2 row for %s" % b)
        elif not readme[b]:
            fails.append("README: the %s row names no commit in backticks" % b)
        else:
            cited[b] = readme[b][0]
    commits = []
    for c in cited.values():
        if not any(same(c, k) for k in commits):
            commits.append(c)
    if len(commits) > 1:
        fails.append("README: the boards name %d commits, want one: %s" % (
            len(commits), ", ".join("%s `%s`" % (b, c) for b, c in cited.items())))

    if commits:
        commit = commits[0]
        if a.ref.lower() != "none":
            for c in commits:
                try:
                    r = subprocess.run(["git", "-C", ROOT, "merge-base",
                                        "--is-ancestor", c, a.ref],
                                       capture_output=True, text=True)
                except OSError:
                    print("warning: ancestor check skipped: git unavailable")
                    break
                if r.returncode == 1:
                    fails.append("`%s` is not an ancestor of %s" % (c, a.ref))
                elif r.returncode != 0:
                    print("warning: ancestor check skipped for `%s`: %s" % (
                        c, r.stderr.strip() or "git failed"))
        for b in WANT:
            if b not in results:
                fails.append("RESULTS: no glmark2 row for %s" % b)
            elif not any(same(commit, h) for h in results[b]):
                fails.append("RESULTS: the %s glmark2 row does not cite `%s`" % (b, commit))
            elif scenes:
                fails += scene_fails(b, rtext[b], scenes, commit)

    for f in fails:
        print("FAIL: " + f)
    if fails:
        return 1
    print("ok: %s at `%s`, RESULTS.md has a row for each" % (", ".join(WANT), commits[0]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
