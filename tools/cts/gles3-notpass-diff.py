#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare two dEQP-GLES3 not-pass TSVs case by case, by family.

  gles3-notpass-diff.py OLD NEW [--mustpass LIST] [--cases]
                        [--rows SPEC --tree am62p-gles3-tree.txt]

OLD and NEW are paths, or 'git:<rev>:<path>' (read with git show). A TSV line
is <outcome> TAB <case> TAB <message> (docs/es3/am62p-gles3-notpass.tsv, written
by gles3-notpass.py); '#' lines are the header. A mustpass case missing from a
TSV counts as Pass. Prints the totals per outcome, a table per family (the
first three components after 'dEQP-GLES3.functional.') and, with --cases, the
case lists. fixed: not Pass before, Pass now. regressed: Pass before, not Pass
now. changed: not Pass in both with a different outcome (a changed message
alone is not a change). --rows/--tree limit the diff to those tree rows.
Stdlib only; the row matching is gles3-notpass.py's, imported.
Exit 0, or 2 for bad input.
"""
import argparse
import os
import subprocess
import sys
from importlib.util import module_from_spec, spec_from_file_location

HERE = os.path.dirname(os.path.abspath(__file__))
PREFIX = "dEQP-GLES3.functional."


def notpass():
    """gles3-notpass.py (the file name has a hyphen, so no plain import)."""
    spec = spec_from_file_location("gles3_notpass", os.path.join(HERE, "gles3-notpass.py"))
    mod = module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def read_text(src):
    if src.startswith("git:"):
        parts = src.split(":", 2)
        if len(parts) != 3 or not parts[1] or not parts[2]:
            raise ValueError("%r is not git:<rev>:<path>" % src)
        r = subprocess.run(["git", "show", "%s:%s" % (parts[1], parts[2])],
                           capture_output=True, text=True, cwd=HERE)
        if r.returncode:
            raise ValueError("git show %s:%s: %s" % (parts[1], parts[2], r.stderr.strip()))
        return r.stdout
    with open(src, encoding="utf-8") as f:
        return f.read()


def parse(text, outcomes, name):
    res = {}
    for n, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) < 2 or f[0] not in outcomes or f[0] == "Pass":
            raise ValueError("%s:%d: not an outcome, case, message line" % (name, n))
        res[f[1]] = f[0]
    return res


def family(case):
    rest = case[len(PREFIX):] if case.startswith(PREFIX) else case
    return ".".join(rest.split(".")[:3])


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"))
    ap.add_argument("--cases", action="store_true", help="list the cases too")
    ap.add_argument("--rows", metavar="SPEC")
    ap.add_argument("--tree", metavar="TREE")
    a = ap.parse_args()
    if bool(a.rows) != bool(a.tree):
        ap.error("--rows and --tree go together")
    np = notpass()
    try:
        outcomes = np.COUNTED
        must = np.read_mustpass(a.mustpass)
        old = parse(read_text(a.old), outcomes, a.old)
        new = parse(read_text(a.new), outcomes, a.new)
        stray = sorted((set(old) | set(new)) - set(must))
        if a.rows:
            owner, n = np.case_rows(read_text(a.tree), must)
            keep = set(np.parse_rows(a.rows, n))
            must = [c for c in must if owner.get(c) in keep]
        elif stray:
            raise ValueError("%d TSV case(s) not in the mustpass list, first %s"
                             % (len(stray), stray[0]))
    except (ValueError, OSError, np.NotpassError) as e:
        sys.stderr.write("gles3-notpass-diff: %s\n" % e)
        return 2

    tot = {"old": dict.fromkeys(outcomes, 0), "new": dict.fromkeys(outcomes, 0)}
    fams, lists = {}, {"fixed": [], "regressed": [], "changed": []}
    for c in must:
        o, n = old.get(c, "Pass"), new.get(c, "Pass")
        tot["old"][o] += 1
        tot["new"][n] += 1
        if o == n:
            continue
        kind = "fixed" if n == "Pass" else "regressed" if o == "Pass" else "changed"
        lists[kind].append((c, o, n))
        fams.setdefault(family(c), dict.fromkeys(lists, 0))[kind] += 1

    out = ["cases: %d%s" % (len(must), " (rows %s)" % a.rows if a.rows else ""),
           "%-15s %8s %8s %8s" % ("outcome", "old", "new", "delta")]
    for k in outcomes:
        out.append("%-15s %8d %8d %+8d" % (k, tot["old"][k], tot["new"][k],
                                           tot["new"][k] - tot["old"][k]))
    out += ["", "%-50s %6s %9s %8s" % ("family", "fixed", "regressed", "changed")]
    for fam in sorted(fams):
        r = fams[fam]
        out.append("%-50s %6d %9d %8d" % (fam, r["fixed"], r["regressed"], r["changed"]))
    out.append("%-50s %6d %9d %8d" % ("total", len(lists["fixed"]), len(lists["regressed"]),
                                      len(lists["changed"])))
    if a.cases:
        for kind in lists:
            out += ["", "%s (%d)" % (kind, len(lists[kind]))]
            out += ["%s\t%s -> %s" % t for t in lists[kind]]
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
