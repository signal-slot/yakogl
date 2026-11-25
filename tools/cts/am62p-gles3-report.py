#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Per-row table of an AM62P dEQP-GLES3 tree record, and the #1285 outcome section.

    tools/cts/am62p-gles3-report.py [--table T] [--mustpass M] [--note-section ROWS] RECORD_DIR...

A record directory is what `tools/cts/board-stage.sh am62p gles3` has `tools/board.sh am62p
tree` write: records/am62p/deqp-gles3-<sha8>[-n3d<v>][-idiv<v>]/ with

    tree.txt    one line per row: <label> TAB <image> TAB <selector> TAB <filter>; the label
                is img<N>-<sha8> (row index N-1 of tools/cts/am62p-gles3-tree.txt)
    <YYYYMMDD-HHMM>-<label>-<sha8|probe>/   one run directory per row run: console.txt (the
                redacted UART console), meta.txt and INDEX.txt, whose "verdict: " line is the
                board.sh verdict line (verdict=, resets=, back_after=<s>s)

Rows are numbered from 0, as TREE_ROWS and board-stage.sh name them (rows 22-38 are the
texture.filtering.3d formats and sizes images of #1285). A row run twice (in one record or in
two) is reported from its latest run directory (the YYYYMMDD-HHMM stamp; a tie goes to the
later RECORD_DIR on the command line). A row no record ran is "not run".

The build of a record is the <sha8> (with its -n3d/-idiv suffix) of the directory name, else
the img<N>-<tag> tag of its tree.txt labels. Records of different builds are an error (exit 1):
one report is one build. No build at all is an error too (exit 2).

stdout: a markdown table, one line per row of the table T: index, label (or the row's first
filter when no run has a label), build, verdict, resets, back_after in s, cases begun (the
image's own summary; "N+" when the image printed none: N result lines were seen and the
image stopped), then over the row's mustpass cases (the row's filters matched against M,
deqp-uart-tally.py's rule): Pass / Fail / NotSupported / QualityWarning (CompatibilityWarning
counted here) / other (any other status) / no result. A last line sums the columns.

--note-section ROWS (e.g. 22-38, or 22,24-26) prints after the table a markdown section for
docs/notes/pvr-3d-mipmap-hang.md: the build commit (expanded to 40 hex when this checkout
knows it), one line per row with its 3D case(s), verdict and resets, and a closing sentence
that says whether every row ended verdict=done with resets 0. Exit 0 on a printed report,
also when rows are not clean (the report says so); exit 1 on a build mismatch.
"""
import argparse
import glob
import importlib.util
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RECORD = re.compile(r"^deqp-gles3-([0-9a-f]{8}(?:-n3d\d+)?(?:-idiv\d+)?)$")
RUNDIR = re.compile(r"^(\d{8}-\d{4})-(.+)-(?:[0-9a-f]{8}|probe)$")
LABEL = re.compile(r"^img(\d+)-(\S+)$")
SHORT = {"Pass": "Pass", "Fail": "Fail", "NotSupported": "NotSupported",
         "QualityWarning": "QualityWarning", "CompatibilityWarning": "QualityWarning"}


def load(name):
    """A sibling tool as a module (imported for its parsers, not modified)."""
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), os.path.join(HERE, name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


tally = load("deqp-uart-tally")
treelog = load("check-tree-log")


def die(msg, rc=1):
    sys.stderr.write("am62p-gles3-report: %s\n" % msg)
    sys.exit(rc)


def parse_rows(spec, n):
    out = set()
    for part in spec.split(","):
        m = re.fullmatch(r"(\d+)(?:-(\d+))?", part.strip())
        if not m:
            die("--note-section: %r is not N or N-M" % part, 2)
        a, b = int(m.group(1)), int(m.group(2) or m.group(1))
        if a > b or b >= n:
            die("--note-section: rows %s outside 0..%d" % (part, n - 1), 2)
        out.update(range(a, b + 1))
    return sorted(out)


def record_build(d):
    """The build tag of a record directory: its name's, else its tree.txt labels' tag."""
    m = RECORD.match(os.path.basename(os.path.normpath(d)))
    if m:
        return m.group(1)
    tags = set()
    try:
        with open(os.path.join(d, "tree.txt"), encoding="utf-8", errors="replace") as f:
            for line in f:
                lm = LABEL.match(line.split("\t", 1)[0])
                if lm:
                    tags.add(lm.group(2))
    except OSError:
        pass
    if len(tags) == 1:
        return tags.pop()
    if len(tags) > 1:
        die("%s: tree.txt labels name several builds: %s" % (d, ", ".join(sorted(tags))))
    return None


def read_runs(d):
    """{label: (stamp, run dir)} of a record, the latest run of each label."""
    runs = {}
    for r in sorted(glob.glob(os.path.join(d, "*", ""))):
        m = RUNDIR.match(os.path.basename(os.path.normpath(r)))
        if not m or not (os.path.isfile(os.path.join(r, "INDEX.txt")) or os.path.isfile(os.path.join(r, "console.txt"))):
            continue
        stamp, label = m.group(1), m.group(2)
        if label not in runs or stamp >= runs[label][0]:
            runs[label] = (stamp, r.rstrip("/"))
    return runs


def verdict_of(run):
    """The board.sh verdict line of a run directory (INDEX.txt), or None."""
    try:
        with open(os.path.join(run, "INDEX.txt"), encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("verdict: "):
                    return line[len("verdict: "):].strip()
    except OSError:
        pass
    return None


def row_index(label):
    m = LABEL.match(label)
    return int(m.group(1)) - 1 if m else None


def expand(c):
    sha = c[:8]
    try:
        r = subprocess.run(["git", "-C", HERE, "rev-parse", "--verify", "--quiet", sha + "^{commit}"],
                           capture_output=True, text=True)
        if r.returncode == 0 and re.fullmatch(r"[0-9a-f]{40}", r.stdout.strip()):
            return r.stdout.strip() + c[8:]
    except OSError:
        pass
    return c


def row_report(i, filters, mustpass, run, label):
    """One row's figures (a dict)."""
    rx = tally.filter_regex(filters)
    cases = [c for c in mustpass if rx and rx.match(c)]
    row = {"index": i, "label": label or filters[0], "filters": filters, "cases": cases,
           "verdict": "not run", "resets": "-", "back": "-", "begun": "-",
           "Pass": 0, "Fail": 0, "NotSupported": 0, "QualityWarning": 0, "other": 0, "none": len(cases),
           "statuses": {}}
    if run is None:
        return row
    line = verdict_of(run)
    if line is not None:
        row["verdict"] = treelog.field(line, "verdict")
        row["resets"] = treelog.field(line, "resets")
        ba = treelog.field(line, "back_after")
        row["back"] = ba[:-1] if ba.endswith("s") else ba
    else:
        row["verdict"] = "no verdict line"
    try:
        with open(os.path.join(run, "console.txt"), encoding="utf-8", errors="replace") as f:
            r = tally.scan(f.read())
    except OSError:
        r = None
    if r is None:
        return row
    got = r["got"]
    row["begun"] = str(r["summary"][0]) if r["summary"] else "%d+" % len(got)
    row["none"] = 0
    for c in cases:
        s = got.get(c)
        if s is None:
            row["none"] += 1
        else:
            row[SHORT.get(s, "other")] += 1
        row["statuses"][c] = s
    return row


COLS = ("Pass", "Fail", "NotSupported", "QualityWarning", "other", "none")


def print_table(rows, build):
    print("| row | label | build | verdict | resets | back_after s | begun | Pass | Fail | NotSupported | "
          "QualityWarning | other | no result |")
    print("|---:|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for r in rows:
        print("| %d | %s | %s | %s | %s | %s | %s | %s |" % (
            r["index"], r["label"], build if r["verdict"] != "not run" else "-", r["verdict"], r["resets"],
            r["back"], r["begun"], " | ".join(str(r[c]) for c in COLS)))
    ran = [r for r in rows if r["verdict"] != "not run"]
    tot = {c: sum(r[c] for r in rows) for c in COLS}
    print("| all | %d of %d rows run | %s | %d done | %d | | | %s |" % (
        len(ran), len(rows), build, sum(1 for r in rows if r["verdict"] == "done"),
        sum(int(r["resets"]) for r in ran if r["resets"].isdigit()), " | ".join(str(tot[c]) for c in COLS)))
    print()
    print("mustpass %d: Pass %d, Fail %d, NotSupported %d, QualityWarning %d, other %d, no result %d." % (
        sum(len(r["cases"]) for r in rows), *(tot[c] for c in COLS)))


def case_text(r):
    """The 3D case(s) of a row: its one mustpass case, else its filters."""
    names = [c[len("dEQP-GLES3.functional."):] if c.startswith("dEQP-GLES3.functional.") else c
             for c in (r["cases"] if len(r["cases"]) == 1 else r["filters"])]
    if len(r["cases"]) == 1:
        return "`%s`" % names[0]
    shown = ", ".join("`%s`" % n for n in names[:3])
    more = " and %d more filters" % (len(names) - 3) if len(names) > 3 else ""
    return "%s%s (%d mustpass cases)" % (shown, more, len(r["cases"]))


def print_note(rows, sel, build, dirs, table):
    full = expand(build)
    picked = [rows[i] for i in sel]
    root = os.path.dirname(os.path.dirname(HERE))
    tab = os.path.relpath(os.path.abspath(table), root)
    tab = os.path.basename(table) if tab.startswith("..") else tab
    print("## Outcome on the board, build %s" % full[:8])
    print()
    print("Build: commit `%s` (records %s). Rows %s of `%s`, numbered from 0:" % (
        full, ", ".join("`%s/`" % os.path.basename(os.path.normpath(d)) for d in dirs), fmt_rows(sel), tab))
    print()
    for r in picked:
        stat = ""
        if len(r["cases"]) == 1 and r["verdict"] != "not run":
            stat = ", %s" % (r["statuses"].get(r["cases"][0]) or "no result")
        elif r["verdict"] != "not run":
            stat = ", %d Pass / %d Fail / %d NotSupported / %d QualityWarning / %d other / %d no result" % tuple(
                r[c] for c in COLS)
        back = ", back after %s s" % r["back"] if r["back"] not in ("-", "?") else ""
        lab = " (%s)" % r["label"] if r["verdict"] != "not run" else ""
        print("- row %d%s: %s, verdict=%s, resets=%s%s%s." % (
            r["index"], lab, case_text(r), r["verdict"], r["resets"], stat, back))
    print()
    clean = [r for r in picked if r["verdict"] == "done" and r["resets"] == "0"]
    if len(clean) == len(picked):
        print("Every one of the %d rows ended verdict=done with resets 0." % len(picked))
    else:
        bad = [r for r in picked if r not in clean]
        print("Not every row ended clean: %d of %d rows ended verdict=done with resets 0; rows %s did not (%s)." % (
            len(clean), len(picked), fmt_rows([r["index"] for r in bad]),
            "; ".join("row %d verdict=%s resets=%s" % (r["index"], r["verdict"], r["resets"]) for r in bad)))


def fmt_rows(idx):
    out, run = [], []
    for i in sorted(idx):
        if run and i == run[-1] + 1:
            run.append(i)
            continue
        if run:
            out.append("%d-%d" % (run[0], run[-1]) if len(run) > 1 else str(run[0]))
        run = [i]
    if run:
        out.append("%d-%d" % (run[0], run[-1]) if len(run) > 1 else str(run[0]))
    return ", ".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("dirs", nargs="+", metavar="RECORD_DIR")
    ap.add_argument("--table", default=os.path.join(HERE, "am62p-gles3-tree.txt"))
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"))
    ap.add_argument("--note-section", metavar="ROWS")
    a = ap.parse_args()

    for d in a.dirs:
        if not os.path.isdir(d):
            die("%s: not a directory" % d, 2)
    builds = {}
    for d in a.dirs:
        b = record_build(d)
        if b is None:
            die("%s: names no build (not deqp-gles3-<sha8>, no img<N>-<sha8> labels in tree.txt)" % d, 2)
        builds.setdefault(b, []).append(d)
    if len(builds) != 1:
        die("the records name different builds: %s" % "; ".join(
            "%s (%s)" % (b, ", ".join(ds)) for b, ds in sorted(builds.items())))
    build = next(iter(builds))

    table = treelog.table_rows(a.table)
    with open(a.mustpass, encoding="utf-8") as f:
        mustpass = [l.strip() for l in f if l.strip() and not l.startswith("#")]

    latest = {}  # row index -> (stamp, order, run dir, label)
    for order, d in enumerate(a.dirs):
        for label, (stamp, run) in read_runs(d).items():
            i = row_index(label)
            if i is None:
                i = treelog.row_of(table, label)
            if i is None or i >= len(table):
                sys.stderr.write("am62p-gles3-report: %s: run %s is no row of %s, ignored\n" % (d, run, a.table))
                continue
            key = (stamp, order)
            if i not in latest or key >= latest[i][:2]:
                latest[i] = (stamp, order, run, label)
    rows = []
    for i, (_, filters) in enumerate(table):
        hit = latest.get(i)
        rows.append(row_report(i, filters, mustpass, hit[2] if hit else None, hit[3] if hit else None))

    print_table(rows, build)
    if a.note_section:
        print()
        print_note(rows, parse_rows(a.note_section, len(rows)), build, a.dirs, a.table)
    return 0


if __name__ == "__main__":
    sys.exit(main())
