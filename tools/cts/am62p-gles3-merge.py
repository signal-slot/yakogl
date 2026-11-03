#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Merge the slices of an AM62P dEQP-GLES3 whole-tree run into the not-pass list.

    tools/cts/am62p-gles3-merge.py [--mustpass FILE] [--against OLD.tsv] [--depth N] SLICE_DIR... > new.tsv

The AM62P tree (tools/cts/am62p-gles3-tree.txt, README section 2e) runs in
board sessions; each commits one slice directory

    tools/cts/log/<YYYY-MM-DD>-am62p-gles3-<sha8>-rows<AA>-<BB>/
        README.md   "commit: <sha8>" and one verdict line per row, see check-tree-log.py
        cases.tsv   one line per case dEQP reported:  <case> TAB <status> [TAB <first message line>]
                    status: Pass, Fail, NotSupported, QualityWarning (CompatibilityWarning
                    counts as QualityWarning; any other non-Pass word, e.g. Crash,
                    InternalError, ResourceError, counts as Fail). Cases outside the
                    mustpass list are ignored; a case twice with different statuses
                    (in one slice or two) is an error.
        (totals)    the README may carry the session's totals; this script recounts.

The commit is the <sha8> of the directory name; README.md's "commit:" line must
say the same. Every slice must name the same commit.

stdout: the not-pass TSV in the format of docs/es3/am62p-gles3-notpass.tsv (header
comments, then "<outcome> TAB <case> TAB <first message line>" in mustpass order;
outcome Fail, NotSupported, QualityWarning, or NoResult for a mustpass case with
no status, whose message is empty). stderr: the totals (Pass, Fail, NotSupported,
QualityWarning, no result; they sum to the mustpass size, 42,810) and, with
--against OLD.tsv, the delta of not-pass cases per group (the first --depth
dot-separated parts of the case name, default 5). Exit 1 on any refusal.
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SLICE = re.compile(r"^\d{4}-\d{2}-\d{2}-am62p-gles3-([0-9a-f]{8})-rows(\d\d)-(\d\d)$")
OUTCOMES = ("Pass", "Fail", "NotSupported", "QualityWarning", "NoResult")


def die(msg):
    sys.stderr.write("am62p-gles3-merge: %s\n" % msg)
    sys.exit(1)


def norm(status):
    if status in ("Pass", "Fail", "NotSupported", "QualityWarning"):
        return status
    if status == "CompatibilityWarning":
        return "QualityWarning"
    return "Fail"


def slice_commit(d):
    m = SLICE.match(os.path.basename(os.path.normpath(d)))
    if not m:
        die("%s: the directory name is not <date>-am62p-gles3-<sha8>-rowsAA-BB" % d)
    readme = os.path.join(d, "README.md")
    if not os.path.isfile(readme):
        die("%s: no README.md" % d)
    with open(readme, encoding="utf-8", errors="replace") as f:
        said = re.findall(r"^commit: *([0-9a-f]+)\s*$", f.read(), re.M)
    if said != [m.group(1)]:
        die("%s: README.md 'commit:' line is %s, the directory name says %s" % (d, said or "missing", m.group(1)))
    return m.group(1)


def read_cases(d, into):
    path = os.path.join(d, "cases.tsv")
    if not os.path.isfile(path):
        die("%s: no cases.tsv" % d)
    with open(path, encoding="utf-8", errors="replace") as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 2 or len(parts) > 3 or not parts[0] or not parts[1]:
                die("%s:%d: not <case> TAB <status> [TAB <message>]: %r" % (path, n, line))
            case, status = parts[0], parts[1]
            msg = parts[2] if len(parts) == 3 else ""
            old = into.get(case)
            if old is not None and old[0] != status:
                die("%s:%d: %s is %s here and %s in %s" % (path, n, case, status, old[0], old[2]))
            into[case] = (status, msg, d)


def read_tsv(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            p = line.rstrip("\n").split("\t")
            out[p[1]] = p[0]
    return out


def group_of(case, depth):
    return ".".join(case.split(".")[:depth])


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("slices", nargs="+", metavar="SLICE_DIR")
    ap.add_argument("--mustpass", default=os.path.join(HERE, "gles3-mustpass.txt"))
    ap.add_argument("--against", metavar="OLD.tsv")
    ap.add_argument("--depth", type=int, default=5)
    a = ap.parse_args()

    commits = {slice_commit(d) for d in a.slices}
    if len(commits) != 1:
        die("the slices name different commits: %s" % ", ".join(sorted(commits)))
    commit = commits.pop()

    seen = {}
    for d in a.slices:
        read_cases(d, seen)
    with open(a.mustpass, encoding="utf-8") as f:
        mustpass = [l.strip() for l in f if l.strip() and not l.startswith("#")]

    count = dict.fromkeys(OUTCOMES, 0)
    rows = []
    for case in mustpass:
        s = seen.get(case)
        if s is None:
            out, msg = "NoResult", ""
        else:
            out = norm(s[0])
            msg = s[1] or (s[0] if out == "Fail" else "")
        count[out] += 1
        if out != "Pass":
            rows.append((out, case, msg))
    total = sum(count.values())
    if total != len(mustpass):
        die("internal: totals %d != mustpass %d" % (total, len(mustpass)))

    w = sys.stdout.write
    w("# SPDX-License-Identifier: Apache-2.0\n# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n")
    w("# dEQP-GLES3 on the TI AM62P (Verdin, PowerVR), build %s: every mustpass case\n" % commit)
    w("# (tools/cts/gles3-mustpass.txt) whose result is not Pass. One case per line, tab separated:\n")
    w("# <outcome> <case> <first message line>. NoResult: the case never ran\n")
    w("# (no status in any slice).\n")
    for r in rows:
        w("%s\t%s\t%s\n" % r)

    e = sys.stderr.write
    e("slices: %d, commit %s, cases with a status: %d (in the mustpass list: %d)\n" % (
        len(a.slices), commit, len(seen), sum(1 for c in mustpass if c in seen)))
    e("Pass %d  Fail %d  NotSupported %d  QualityWarning %d  no result %d  = %d (mustpass %d)\n" % (
        count["Pass"], count["Fail"], count["NotSupported"], count["QualityWarning"],
        count["NoResult"], total, len(mustpass)))

    if a.against:
        old = read_tsv(a.against)
        new = {c: o for o, c, _ in rows}
        groups = {}
        for c, o in old.items():
            groups.setdefault(group_of(c, a.depth), {}).setdefault(o, [0, 0])[0] += 1
        for c, o in new.items():
            groups.setdefault(group_of(c, a.depth), {}).setdefault(o, [0, 0])[1] += 1
        e("delta against %s (not-pass cases per group, old -> new):\n" % a.against)
        for g in sorted(groups):
            parts = ["%s %d -> %d" % (o, v[0], v[1]) for o, v in sorted(groups[g].items()) if v[0] != v[1]]
            if parts:
                e("  %-70s %s\n" % (g, ", ".join(parts)))
        e("not pass before %d, now %d; no longer not-pass %d, newly not-pass %d, outcome changed %d\n" % (
            len(old), len(new), len(set(old) - set(new)), len(set(new) - set(old)),
            sum(1 for c in new if c in old and old[c] != new[c])))


if __name__ == "__main__":
    main()
