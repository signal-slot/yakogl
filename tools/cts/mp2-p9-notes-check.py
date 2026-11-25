#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check the DK's P9 notes: the plan's acceptance lines (default) or docs-p9's.

    tools/cts/mp2-p9-notes-check.py [--strict] [--only c2|c3|c4] [--bringup FILE]
        [--p9 FILE] [--prestudy FILE] [--shard-logs GLOB] [--p9-diff FILE]
        [--p9-diff-opt OPT]...
    tools/cts/mp2-p9-notes-check.py --docs-p9 [--only c1|c2|c3] [...]

ACCEPTANCE MODE (default; plan #405, p9-report checks itself with it). One
line per criterion "cN: ok", "cN: pending: <why>" or "cN: failed: <what>",
then "mp2-p9-notes-accept: K ok, F failed, P pending" (last line). A
criterion whose notes are absent (nothing recorded yet) is pending, not a
failure; --strict turns every pending into a failure. Exit 0 when F is 0 (and
P is 0 with --strict), 1 otherwise, 2 for an unreadable note or a bad option.

  c2  the P9 note and the bring-up note each have a "mp2-gltest-diff: new
      <digits>" line per gl_test shard, and each such line (or the line right
      after it) gives a time against the lifetime: "<N> s of <M> s" (also
      "/", "against", "within"). Pending while no line in either
      note has a time yet. Once one has, a line with no time is a failure, and
      so is a note with no line.
  c3  the "mp2-p9-diff: shared S, new N, fixed F, missing M" line quoted in
      the P9 note equals the last line of
      `python3 -I tools/cts/mp2-p9-diff.py --report-only <shard logs>` (logs:
      --shard-logs, default tools/cts/log/*-mp2-gles2-*/s*/shard-*.log), and
      either M is 0 or each missing case that run lists is named on a note
      line that also says "cause". Pending when no shard log exists or the
      note quotes no such line.
  c4  the "| Q11" row of the pre-study note says "Decided", names option A or
      B ("option A|B", or "Decided: A|B") and gives a record name
      records/stm32mp257f-dk/<YYYYMMDD-HHMM>-<name>-<sha8>. Pending when the
      row has no "Decided" yet (an "Answered ..." or open row).

DOCS-P9 MODE (--docs-p9; what docs-p9 ran before it landed the note edits).
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
import glob
import os
import re
import subprocess
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


NUM = r"\d+(?:\.\d+)?"
TIME_VS_LIFETIME = re.compile(
    NUM + r"\s*s\b[^|\n]*?(?:\bof\b|/|\bagainst\b|\bwithin\b)[^|\n]*?" + NUM + r"\s*s\b", re.I)
P9_SUMMARY = re.compile(r"mp2-p9-diff: shared \d+, new \d+, fixed \d+, missing \d+")
Q11_ROW = re.compile(r"^\s*\|\s*Q11\b")
DECIDED = re.compile(r"\bDecided\b")
OPTION = re.compile(r"\boption\s+[AB]\b|\bDecided\W{0,3}(?:option\s+)?[AB]\b")
Q11_RECORD = re.compile(
    r"records/stm32mp257f-dk/\d{8}-\d{4}-[a-z0-9][a-z0-9_.-]*-[0-9a-f]{8}\b")


def gltest_lines(lines):
    """[(line, timed)] for every 'mp2-gltest-diff: new <digits>' line."""
    out = []
    for i, l in enumerate(lines):
        if GLTEST_LINE.search(l):
            window = l + "\n" + (lines[i + 1] if i + 1 < len(lines) else "")
            out.append((l.strip(), bool(TIME_VS_LIFETIME.search(window))))
    return out


def accept_c2(bringup, p9, names):
    """(state, text) with state ok|pending|failed."""
    per = [(n, gltest_lines(lines)) for n, lines in zip(names, (p9, bringup))]
    if not any(timed for _, r in per for _, timed in r):
        return "pending", "no timed 'mp2-gltest-diff: new' line in %s yet" % " or ".join(names)
    for n, r in per:
        if not r:
            return "failed", "no 'mp2-gltest-diff: new' line in %s" % n
        for line, timed in r:
            if not timed:
                return "failed", ("%s: a 'mp2-gltest-diff: new' line without a time "
                                  "against the lifetime ('N s of M s'): %s" % (n, line[:100]))
    return "ok", ""


def accept_c3(p9, logs, diff_tool, diff_opts, p9name):
    quoted = [m.group(0) for l in p9 for m in P9_SUMMARY.finditer(l)]
    if not logs:
        return "pending", "no dEQP shard log under the shard-log glob yet"
    if not quoted:
        return "pending", "no 'mp2-p9-diff: shared ...' line quoted in %s yet" % p9name
    cmd = [sys.executable, "-I", diff_tool, "--report-only"] + diff_opts + logs
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, errors="replace")
    except OSError as e:
        return "failed", "cannot run mp2-p9-diff.py: %s" % e
    out = r.stdout.splitlines()
    if r.returncode != 0 or not out:
        return "failed", "mp2-p9-diff.py exited %d: %s" % (r.returncode, r.stderr.strip()[:100])
    last = out[-1].strip()
    if not P9_SUMMARY.fullmatch(last):
        return "failed", "unexpected last line of mp2-p9-diff.py: %s" % last[:100]
    if quoted[-1] != last:
        return "failed", "%s quotes '%s' but the shard logs give '%s'" % (p9name, quoted[-1], last)
    miss = int(MISSING_NUM.findall(last)[-1])
    if miss == 0:
        return "ok", ""
    names, inside = [], False
    for l in out:
        if l.startswith("== "):
            inside = "missing from the DK run" in l
        elif inside and l.strip() and not l.startswith("mp2-p9-diff:"):
            names.append(l.strip())
    bare = [n for n in names if not any(n in l and re.search("cause", l, re.I) for l in p9)]
    if bare:
        return "failed", ("missing %d: %d case(s) without a line naming it and its cause, first %s"
                          % (miss, len(bare), bare[0]))
    return "ok", ""


def accept_c4(prestudy, name):
    rows = [l for l in prestudy if Q11_ROW.match(l)]
    if not rows:
        return "failed", "no '| Q11' row in %s" % name
    row = rows[0]
    if not DECIDED.search(row):
        return "pending", "the Q11 row of %s does not say 'Decided' yet" % name
    if not OPTION.search(row):
        return "failed", "the Q11 row says Decided but names neither option A nor B"
    if not Q11_RECORD.search(row):
        return "failed", "the Q11 row names no records/stm32mp257f-dk/<YYYYMMDD-HHMM>-<name>-<sha8> record"
    return "ok", ""


def accept_main(a, bringup, p9, prestudy):
    b = os.path.basename
    logs = sorted(glob.glob(a.shard_logs))
    res = {
        "c2": accept_c2(bringup, p9, (b(a.p9), b(a.bringup))),
        "c3": accept_c3(p9, logs, a.p9_diff, a.p9_diff_opt, b(a.p9)),
        "c4": accept_c4(prestudy, b(a.prestudy)),
    }
    k = f = p = 0
    for cid in ("c2", "c3", "c4"):
        if a.only and cid != a.only:
            continue
        state, text = res[cid]
        if state == "pending" and a.strict:
            state, text = "failed", "pending (--strict): " + text
        print("%s: %s" % (cid, "ok" if state == "ok" else "%s: %s" % (state, text)))
        k += state == "ok"
        f += state == "failed"
        p += state == "pending"
    print("mp2-p9-notes-accept: %d ok, %d failed, %d pending" % (k, f, p))
    return 1 if f else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bringup", default=os.path.join(NOTES, "stm32mp257f-dk-bringup.md"))
    ap.add_argument("--p9", default=os.path.join(NOTES, "stm32mp257f-dk-p9.md"))
    ap.add_argument("--prestudy", default=os.path.join(NOTES, "pre-study-stm32mp257f-dk.md"))
    ap.add_argument("--only", choices=("c1", "c2", "c3", "c4"))
    ap.add_argument("--docs-p9", action="store_true", help="the docs-p9 checks c1..c3")
    ap.add_argument("--strict", action="store_true", help="pending counts as failed")
    ap.add_argument("--shard-logs", default=os.path.join(
        ROOT, "tools", "cts", "log", "*-mp2-gles2-*", "s*", "shard-*.log"))
    ap.add_argument("--p9-diff", default=os.path.join(HERE, "mp2-p9-diff.py"))
    ap.add_argument("--p9-diff-opt", action="append", default=[],
                    help="extra option for mp2-p9-diff.py (repeatable)")
    a = ap.parse_args()
    if (a.only == "c1" and not a.docs_p9) or (a.only == "c4" and a.docs_p9):
        print("mp2-p9-notes-check: --only %s does not exist in this mode" % a.only,
              file=sys.stderr)
        return 2
    try:
        bringup = read_lines(a.bringup)
        p9 = read_lines(a.p9)
        prestudy = read_lines(a.prestudy)
    except OSError as e:
        print("mp2-p9-notes-check: %s" % e, file=sys.stderr)
        return 2

    if not a.docs_p9:
        return accept_main(a, bringup, p9, prestudy)

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
