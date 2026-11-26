#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Render the STM32MP257F-DK's per-group dEQP-GLES2 results.tsv from UART shard logs.

    tools/cts/mp2-deqp-results.py [--list FILE] [--exclude FILE] [--mustpass FILE] LOG... > results.tsv

The output has the i.MX8MP baseline's form (imx8mp-full-tree-29460f96.results.tsv):
the header '# index<TAB>filter<TAB>begun<TAB>passed<TAB>failed<TAB>notsup<TAB>other
<TAB>noresult<TAB>failing names (';'-separated)' and one row per entry of --list
(default mp2-deqp-gles2.txt, 83 groups), in the list's order.

Logs are read as deqp-uart-tally.py reads them: NULs, CRs and a leading time stamp
are dropped, and "deqp: <Status> <case>" lines count, the last status of a case
winning over every log. A case belongs to the first entry of the list with a
filter (comma-separated; "X.*" is the prefix "X.") that matches it. Names the
--exclude file covers (mp2-deqp-gles2-exclude.txt's syntax, as mp2-p9-diff.py reads
it) are left out of every column.

Columns, as the baseline's generator (tools/board.sh results_tsv) fills them:
  begun     cases with a result line in the group
  passed    Pass
  failed    Fail
  notsup    NotSupported
  other     every other status: QualityWarning and CompatibilityWarning (what the
            i.MX8MP's 19 are) and InternalError, ResourceError, Crash, Timeout
  noresult  the group's --mustpass cases (default gles2-mustpass.txt, minus the
            excluded) with no result line; 0 for a group with no result line at
            all (it did not run: begun 0, as an unrun slice)
  failing   ';'-separated names of every case counted in failed and other, sorted;
            '-' when none. Like the baseline's, the column also lists warnings
            (its other 19 are all warnings): mp2-p9-diff.py reads a name there as
            a failure on both boards alike.

Differences from the baseline that cannot be avoided: the baseline counts dEQP's
whole tree per entry (17,371 begun, more than the mustpass list), so begun here is
what the logs show, which for a complete sweep is the same set; a case that no
group matches (not in the list) is counted on stderr and not in any row; and a
UART log has result lines only, so a case that began but ended without a result
(a hang) is a noresult only when the group ran at all.
Exit 2 on a usage or input error.
"""
import argparse
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PASS, NOTSUP, FAIL = "Pass", "NotSupported", "Fail"


def load_tally():
    spec = importlib.util.spec_from_file_location(
        "deqp_uart_tally", os.path.join(HERE, "deqp-uart-tally.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def lines_of(path):
    with open(path) as fh:
        for l in fh:
            l = l.split("#", 1)[0].strip()
            if l:
                yield l


def exclude_patterns(path):
    out = []
    with open(path) as fh:
        for l in fh:
            l = l.rstrip("\n")
            if l.startswith("#") or not l.strip():
                continue
            out.append(l.split("\t", 1)[0].strip())
    return out


def covered(name, pats):
    return any(g == name or (g.endswith("*") and name.startswith(g[:-1])) for g in pats)


def matches(filt, name):
    return name.startswith(filt[:-1]) if filt.endswith("*") else name == filt


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--list", default=os.path.join(HERE, "mp2-deqp-gles2.txt"))
    ap.add_argument("--exclude", default=os.path.join(HERE, "mp2-deqp-gles2-exclude.txt"))
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles2-mustpass.txt"))
    ap.add_argument("logs", nargs="+", metavar="LOG")
    a = ap.parse_args()
    try:
        tally = load_tally()
        groups = [l.split(",") for l in lines_of(a.list)]
        excl = exclude_patterns(a.exclude)
        must = [n for n in lines_of(a.mustpass) if not covered(n, excl)]
        status = {}
        for p in a.logs:
            with open(p, errors="replace") as fh:
                status.update(tally.scan(fh.read())["got"])
    except OSError as e:
        print("mp2-deqp-results: %s" % e, file=sys.stderr)
        return 2

    def group_of(name):
        for i, fl in enumerate(groups):
            if any(matches(f, name) for f in fl):
                return i
        return None

    rows = [dict(b=0, p=0, f=0, n=0, o=0, names=[], must=0) for _ in groups]
    stray = 0
    for name, st in status.items():
        if covered(name, excl):
            continue
        i = group_of(name)
        if i is None:
            stray += 1
            continue
        r = rows[i]
        r["b"] += 1
        if st == PASS:
            r["p"] += 1
        elif st == NOTSUP:
            r["n"] += 1
        else:
            r["f" if st == FAIL else "o"] += 1
            r["names"].append(name)
    for name in must:
        i = group_of(name)
        if i is not None:
            rows[i]["must"] += 1
    for r in rows:
        r["nr"] = 0
    for name in must:
        i = group_of(name)
        if i is not None and rows[i]["b"] and name not in status:
            rows[i]["nr"] += 1
    if stray:
        print("mp2-deqp-results: %d case(s) match no group, not counted" % stray, file=sys.stderr)
    out = ["# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\t"
           "failing names (';'-separated)"]
    for i, fl in enumerate(groups):
        r = rows[i]
        out.append("%d\t%s\t%d\t%d\t%d\t%d\t%d\t%d\t%s" % (
            i, ",".join(fl), r["b"], r["p"], r["f"], r["n"], r["o"], r["nr"],
            ";".join(sorted(r["names"])) or "-"))
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
