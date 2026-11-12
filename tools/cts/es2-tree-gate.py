#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Stage 1's five fields of a whole dEQP-GLES2 tree record, as one verdict.

    tools/cts/es2-tree-gate.py [--accepted FILE] [--mustpass FILE] [--table FILE] RECORD_DIR

RECORD_DIR is a tree record as tools/cts/board-stage.sh <board> deqp writes it
(records/<board>/deqp-<sha8>/): tree.txt (one row per image or table entry:
name, image, selector, filter), one run directory per row
(<YYYYMMDD>-<HHMM>-<name>-<sha8>/ with console.txt, or console.ts), and, for
the i.MX8MP's selector form after `tools/board.sh imx8mp tally`, results.tsv.
The i.MX8MP (imx8mp-full-tree.txt, one image, rows picked by a selector) and
the Raspberry Pi 4 (pi4-full-tree.txt, one image per row) are read the same
way: from the consoles' "deqp: <Status> <case>" lines through
tools/cts/deqp-uart-tally.py (scan, read_log; not parsed again here), and from
results.tsv where the consoles of a row are absent.

It prints

    Fail N                  cases that ended Fail
    NoResult N              mustpass cases (--mustpass, default
                            tools/cts/gles2-mustpass.txt) without a result line,
                            or the images' own "without" count if larger
    crashes N               Crash, Timeout, InternalError, ResourceError cases,
                            and runs dEQP ended with die() ("deqp: fatal:")
    NotSupported N
    NotSupported names: all in es2-accepted.txt
                        (or: not in es2-accepted.txt: <names>)

and last "stage1: PASS" or "stage1: FAIL (<reasons>)". Stage 1 holds when Fail,
NoResult and crashes are 0, every row finished (a summary line, no lost lines,
no fatal), every NotSupported case is in the accepted list, and every entry of
that list is NotSupported (exactly the agreed set: 34 on 2026-11-12). Names of
NotSupported cases come from consoles only; a record without them cannot pass.
Exit status: 0 PASS, 1 FAIL, 2 the record is unreadable.
"""
import argparse
import fnmatch
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRASH = ("Crash", "Timeout", "InternalError", "ResourceError")
RUN_DIR = re.compile(r"^[0-9]{8}-[0-9]{4}-(.+)-([0-9a-f]{8}|probe)$")


class Unreadable(Exception):
    pass


def load_tally():
    spec = importlib.util.spec_from_file_location("deqp_uart_tally", os.path.join(HERE, "deqp-uart-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def names_of(path, what):
    """non-comment first words of a list file (es2-accepted.txt, gles2-mustpass.txt)"""
    try:
        with open(path, encoding="utf-8") as f:
            out = [l.split("#", 1)[0].strip() for l in f]
    except (OSError, UnicodeDecodeError) as e:
        raise Unreadable("cannot read %s %s: %s" % (what, path, e))
    return [l for l in out if l]


def tree_rows(rec):
    """[(name, selector)] of tree.txt, or None when the record has none"""
    p = os.path.join(rec, "tree.txt")
    if not os.path.isfile(p):
        return None
    rows = []
    with open(p, encoding="utf-8", errors="replace") as f:
        for l in f:
            c = l.rstrip("\n").split("\t")
            if c[0].strip() and not c[0].startswith("#"):
                rows.append((c[0], c[2] if len(c) > 2 else "-"))
    return rows


def console_of(d):
    for n in ("console.txt", "console.ts"):
        if os.path.isfile(os.path.join(d, n)):
            return os.path.join(d, n)
    return None


def run_dirs(rec, name=None):
    out = []
    for e in sorted(os.listdir(rec)):
        m = RUN_DIR.match(e)
        if m and os.path.isdir(os.path.join(rec, e)) and (name is None or m.group(1) == name):
            out.append(os.path.join(rec, e))
    return out


def tsv_rows(rec):
    """{selector: (begun, pass, fail, notsup, other, noresult)} of results.tsv ('-' is None), or None"""
    p = os.path.join(rec, "results.tsv")
    if not os.path.isfile(p):
        return None
    rows = {}
    with open(p, encoding="utf-8", errors="replace") as f:
        for l in f:
            if l.startswith("#") or not l.strip():
                continue
            c = l.rstrip("\n").split("\t")
            if len(c) < 8:
                raise Unreadable("results.tsv: a row with %d columns" % len(c))
            try:
                rows[c[0]] = None if c[2] == "-" else tuple(0 if x == "-" else int(x) for x in c[2:8])
            except ValueError:
                raise Unreadable("results.tsv: a count that is not a number in row %s" % c[0])
    return rows


def pick_run(tally, dirs):
    """(dir, console, verdict, scan) of a row's best run: the latest whose log is complete, else the latest"""
    best = None
    for d in dirs:
        c = console_of(d)
        if not c:
            continue
        with open(c, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        verdict = tally.read_log(text)[0]
        cur = (d, c, verdict, tally.scan(text))
        if verdict is None:
            best = cur
        elif best is None or best[2] is not None:
            best = cur
    return best


def gate(rec, accepted, mustpass, table_rows, tally):
    rows = tree_rows(rec)
    tsv = tsv_rows(rec)
    got, fatal_rows = {}, 0
    reasons, notes = [], []
    without = 0
    ns_blind = False  # NotSupported cases whose names no console gave
    tsv_ns = tsv_fail = tsv_other = tsv_nr = 0
    if rows is None:
        # no tree.txt: every run directory is a row
        dirs = run_dirs(rec)
        if not dirs and tsv is None:
            raise Unreadable("%s: no tree.txt, no run directories, no results.tsv" % rec)
        rows = [(RUN_DIR.match(os.path.basename(d)).group(1), "-") for d in dirs]
    if not rows:
        raise Unreadable("%s: tree.txt has no rows" % rec)
    if table_rows is not None and len(rows) != table_rows:
        reasons.append("%d rows in tree.txt, the table has %d" % (len(rows), table_rows))
    bad, nocon = [], []
    for name, sel in rows:
        run = pick_run(tally, run_dirs(rec, name))
        if run is None:
            t = tsv.get(sel) if tsv is not None else None
            if t is None:
                nocon.append(name)
                continue
            # a row with results.tsv numbers but no console: counts only, no names
            _, _, f, n, o, nr = t
            tsv_fail += f
            tsv_ns += n
            tsv_other += o
            tsv_nr += nr
            ns_blind |= n > 0
            notes.append("row %s: results.tsv only (no console)" % name)
            continue
        _, c, verdict, r = run
        got.update(r["got"])
        if r["summary"]:
            without += r["summary"][2]
        if verdict:
            bad.append("%s %s" % (name, verdict.split(":")[0]))
            if verdict.startswith("FATAL"):
                fatal_rows += 1
        elif r["loss"]:
            bad.append("%s INCOMPLETE" % name)
        if tsv is not None and tsv.get(sel) is not None and verdict is None:
            t = tsv[sel]
            mine = (sum(1 for s in r["got"].values() if s == "Fail"),
                    sum(1 for s in r["got"].values() if s == "NotSupported"))
            if (t[2], t[3]) != mine:
                reasons.append("row %s: results.tsv says Fail %d NotSupported %d, the console %d and %d"
                               % (name, t[2], t[3], mine[0], mine[1]))
    if nocon:
        reasons.append("%d row(s) without a console or results.tsv numbers: %s" % (len(nocon), ", ".join(nocon[:6])))
    if bad:
        reasons.append("row(s) not finished: " + ", ".join(bad[:6]))

    fails = sorted(c for c, s in got.items() if s == "Fail")
    crashes = sorted(c for c, s in got.items() if s in CRASH)
    nsn = sorted(c for c, s in got.items() if s == "NotSupported")
    n_fail = len(fails) + tsv_fail
    n_crash = len(crashes) + fatal_rows + tsv_other
    n_ns = len(nsn) + tsv_ns
    n_nr = without + tsv_nr
    if mustpass:
        n_nr = max(n_nr, len(set(mustpass) - set(got)) if got else len(mustpass))
    unacc = [c for c in nsn if not any(fnmatch.fnmatchcase(c, p) for p in accepted)]
    absent = [p for p in accepted if not any(fnmatch.fnmatchcase(c, p) for c in nsn)]

    info = ["record: %s rows (%s)" % (len(rows), "selector form with results.tsv" if tsv is not None else "console form")]
    info += notes
    out = list(info)
    out.append("Fail %d" % n_fail)
    out += ["  Fail: %s" % c for c in fails[:20]]
    out.append("NoResult %d" % n_nr)
    out.append("crashes %d" % n_crash)
    out += ["  %s: %s" % (got[c], c) for c in crashes[:20]]
    out.append("NotSupported %d" % n_ns)
    if unacc:
        out.append("NotSupported names: not in es2-accepted.txt: " + ", ".join(unacc))
    elif ns_blind:
        out.append("NotSupported names: not checkable (results.tsv numbers without a console)")
    else:
        out.append("NotSupported names: all in es2-accepted.txt")
    if n_fail:
        reasons.append("Fail %d" % n_fail)
    if n_nr:
        reasons.append("NoResult %d" % n_nr)
    if n_crash:
        reasons.append("crashes %d" % n_crash)
    if unacc:
        reasons.append("%d NotSupported not accepted" % len(unacc))
    if ns_blind:
        reasons.append("NotSupported names not checkable")
    if absent and not ns_blind and not nocon and not bad:
        reasons.append("%d accepted name(s) not NotSupported (%s)" % (len(absent), ", ".join(absent[:3])))
    out.append("stage1: PASS" if not reasons else "stage1: FAIL (%s)" % "; ".join(reasons))
    return out, not reasons


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("record")
    ap.add_argument("--accepted", default=os.path.join(HERE, "es2-accepted.txt"))
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles2-mustpass.txt"))
    ap.add_argument("--table", help="the board's full-tree table (imx8mp-full-tree.txt, pi4-full-tree.txt): "
                    "the record must have one row per non-comment line")
    a = ap.parse_args()
    try:
        if not os.path.isdir(a.record):
            raise Unreadable("%s is not a directory" % a.record)
        accepted = names_of(a.accepted, "--accepted")
        mustpass = names_of(a.mustpass, "--mustpass")
        table_rows = len(names_of(a.table, "--table")) if a.table else None
        out, ok = gate(a.record, accepted, mustpass, table_rows, load_tally())
    except (Unreadable, OSError) as e:
        print("es2-tree-gate.py: %s" % e, file=sys.stderr)
        return 2
    print("\n".join(out))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
