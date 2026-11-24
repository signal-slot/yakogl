#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check that the DK notes carry the P9 result lines (roadmap P9).

    tools/cts/mp2-p9-notes-check.py [--bringup FILE] [--p9 FILE]
                                    [--prestudy FILE] [--only c1|c2|c3]

One line per criterion, in order ("cN: ok" or "cN: missing: <what>"), then
the summary "mp2-p9-notes-check: K ok, M missing" (last line). Exit 0 when
M is 0, 1 when a criterion is missing, 2 when a note cannot be read or an
option is wrong. --only c1|c2|c3 checks, prints and counts one criterion
alone. A tree whose DK runs have not landed reports missing items: that is
normal, and the point — docs-p9 runs this before it lands the note edits.
The self-test is tools/cts/mp2_p9_notes_check_selftest.sh.

The three notes are the defaults below. What each criterion wants:

  c1  the "Board verdicts" section of the bring-up note holds the whole
      gl_test regression-set result: a line with the last line of
      tools/cts/mp2-gltest-diff.py ("mp2-gltest-diff: new N, flaky F,
      expected E, missing M"; matched as "mp2-gltest-diff: new <digits>"),
      and the section names the origin/main commit the images were built
      from as "built from <sha>" (7 to 40 hex digits, backticks optional;
      the last such mention in the section counts). The section starts at a
      heading line "#..# Board verdicts" and runs to the next heading of
      level 1 or 2.

  c2  the P9 note holds the dEQP-GLES2 sweep result: the last line of
      tools/cts/mp2-p9-diff.py ("mp2-p9-diff: shared S, new N, fixed F,
      missing M"; matched as "mp2-p9-diff: shared <digits>") with M = 0,
      and it carries a "built from <sha>" of its own naming the same commit
      as c1's (equal, or one a prefix of the other).

  c3  both the P9 note and the pre-study note have a line containing "Q11"
      that names both a board record ("<YYYYMMDD-HHMM>-<name>-<sha8>", the
      runner's record directory name) and a measured line width ("N px",
      "width N" or "N wide").
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
NOTES = os.path.join(ROOT, "docs", "notes")

BOARD_HEAD = re.compile(r"^\s*#{1,2}\s+Board verdicts\b")
SECTION_END = re.compile(r"^\s*#{1,2}\s")
GLTEST_LINE = re.compile(r"mp2-gltest-diff: new \d")
P9_LINE = re.compile(r"mp2-p9-diff: shared \d")
MISSING_NUM = re.compile(r"missing (\d+)(?!\d)")
# the build commit convention: "built from `abc1234`" (lower-case in prose,
# case does not matter); the 7..40 hex digits are a commit sha
BUILT = re.compile(r"built from `?([0-9a-f]{7,40})", re.I)
# a board record directory name: 20261123-0451-gc_triangle-250f61d4
RECORD = re.compile(r"\b\d{8}-\d{4}-[a-z0-9][a-z0-9_.-]*-[0-9a-f]{8}\b")
# a measured line width: "3 px", "3 px wide", "width 3", "width: 3.0", "3 wide"
WIDTH = re.compile(r"\d+(?:\.\d+)?\s*(?:px|pixels)\b|\bwidth\s*[:=]?\s*\d+(?:\.\d+)?"
                   r"|\b\d+(?:\.\d+)?\s+wide\b", re.I)


def read_lines(path):
    with open(path, errors="replace") as fh:
        return fh.read().splitlines()


def board_verdicts(lines):
    """(found, its lines) of the note's "Board verdicts" section."""
    out = []
    inside = False
    for l in lines:
        if not inside:
            inside = bool(BOARD_HEAD.match(l))
            continue
        if SECTION_END.match(l):
            break
        out.append(l)
    return inside, out


def built_sha(lines):
    """The sha of the last "built from <sha>" mention, or None."""
    sha = None
    for l in lines:
        ms = list(BUILT.finditer(l))
        if ms:
            sha = ms[-1].group(1).lower()
    return sha


def same_commit(a, b):
    return a is not None and b is not None and (a == b or a.startswith(b) or b.startswith(a))


def q11_named(lines):
    return any("Q11" in l and RECORD.search(l) and WIDTH.search(l) for l in lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bringup", default=os.path.join(NOTES, "stm32mp257f-dk-bringup.md"))
    ap.add_argument("--p9", default=os.path.join(NOTES, "stm32mp257f-dk-p9.md"))
    ap.add_argument("--prestudy", default=os.path.join(NOTES, "pre-study-stm32mp257f-dk.md"))
    ap.add_argument("--only", choices=("c1", "c2", "c3"))
    a = ap.parse_args()
    try:
        bringup = read_lines(a.bringup)
        p9 = read_lines(a.p9)
        prestudy = read_lines(a.prestudy)
    except OSError as e:
        print("mp2-p9-notes-check: %s" % e, file=sys.stderr)
        return 2

    b = os.path.basename
    section_found, section = board_verdicts(bringup)
    section_sha = built_sha(section)

    # c1: the gl_test result line and the build commit in Board verdicts
    if not section_found:
        c1 = "missing: no 'Board verdicts' section in %s" % b(a.bringup)
    elif not any(GLTEST_LINE.search(l) for l in section):
        c1 = ("missing: no 'mp2-gltest-diff: new ...' line in the "
              "'Board verdicts' section of %s" % b(a.bringup))
    elif section_sha is None:
        c1 = ("missing: no 'built from <sha>' commit in the 'Board verdicts' "
              "section of %s" % b(a.bringup))
    else:
        c1 = "ok"

    # c2: the dEQP result line (missing 0) and the same build commit
    line = next((l for l in reversed(p9) if P9_LINE.search(l)), None)
    p9_sha = built_sha(p9)
    miss = MISSING_NUM.findall(line)[-1] if line and MISSING_NUM.findall(line) else None
    if line is None:
        c2 = "missing: no 'mp2-p9-diff: shared ...' result line in %s" % b(a.p9)
    elif miss != "0":
        c2 = ("missing: the 'mp2-p9-diff' line in %s has missing %s, not 0"
              % (b(a.p9), miss if miss is not None else "?"))
    elif p9_sha is None:
        c2 = "missing: no 'built from <sha>' commit in %s" % b(a.p9)
    elif section_sha is None:
        c2 = ("missing: no 'built from <sha>' commit in the 'Board verdicts' "
              "section of %s to compare with" % b(a.bringup))
    elif not same_commit(p9_sha, section_sha):
        c2 = ("missing: %s names built from %s but the gl_test section names %s"
              % (b(a.p9), p9_sha, section_sha))
    else:
        c2 = "ok"

    # c3: a Q11 line with a board record and a measured width, in both notes
    bad = [b(p) for p, lines in ((a.p9, p9), (a.prestudy, prestudy)) if not q11_named(lines)]
    if bad:
        c3 = ("missing: no Q11 line naming a board record and a measured line "
              "width in %s" % ", ".join(bad))
    else:
        c3 = "ok"

    k = m = 0
    for cid, msg in (("c1", c1), ("c2", c2), ("c3", c3)):
        if a.only and cid != a.only:
            continue
        print("%s: %s" % (cid, msg))
        if msg == "ok":
            k += 1
        else:
            m += 1
    print("mp2-p9-notes-check: %d ok, %d missing" % (k, m))
    return 1 if m else 0


if __name__ == "__main__":
    sys.exit(main())
