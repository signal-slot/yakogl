#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Group a dEQP-GLES3 not-pass list into issue drafts (markdown on stdout).

  gles3-triage.py NOTPASS.tsv [--host docs/es3/host-gles3-tip.tsv]
                  [--gpu vivante] [--table tools/cts/imx8mp-gles3-tree.txt]
                  [--depth 4] [--examples 5]

NOTPASS.tsv: one case per line, "<outcome>\\t<case>[\\t<message>[\\t<family>]]" (the
shape of docs/es3/am62p-gles3-notpass.tsv, with an optional 4th column, the
cause-family id such as 'RC1-int-mix' or '#1285-watchdog'), or
"[<record>:]<outcome> <case>" (the shape of docs/es3/pi4-gles3-fails.tsv).
Lines starting with '#' are headers. Rows with three columns parse as before.

Cause families:
  --families RULES --apply OUT.tsv   write a copy of the input (headers kept)
        with the 4th column filled. A row that already has a family keeps it;
        a row no rule matches gets 'unassigned'. RULES
        (tools/cts/am62p-gles3-families.txt): non-comment lines
        "<family-id>\\t<comma-separated case globs>[\\t<message regex>]"; the
        first line whose glob matches the case (and whose regex, re.search,
        matches the message) wins. In RULES a line is a comment when it starts
        with '#' not followed by a digit (so '#1285-watchdog' is a family id).
  --check   exit 1 and list the rows with an empty outcome or an empty or
        'unassigned' family (a missing 4th column counts as empty). Otherwise
        print "family<TAB>count" per family, most rows first, then
        "total<TAB>N" (N = the non-comment rows). The table is what the
        family counts of docs/es3-cts-failures.md are compared with.
  When the input has a family column and --depth is not given, the markdown
  groups by family (with each family's count) instead of by dEQP path.

A group is the case's dEQP path under "dEQP-GLES3." cut at --depth levels
(default 4, e.g. functional.texture.filtering.2d).

Host cross-reference: docs/es3/host-gles3-tip.tsv has one row per
(gpu, group) with pass/fail/notsupported/crash/other counts, only for the
two-level groups ("functional.texture"), not per case. Each case takes the
verdict of its host group: "host-also-fails" when the host has fails/crashes
and no pass (likely GL core / shader compiler, not GPU-specific),
"board-only" when the host has passes and no fail/crash/other, "host-mixed"
otherwise, "host-notsupported" / "host-unknown" when it has no data. The
host's render verdicts are meaningless (host-only run): only the non-render
groups are a firm guide.

Table: each case is mapped to the first entry (line, counting from 0, of the
non-comment lines) whose comma-separated glob filters match it, shown as eNN.
"""
import argparse
import collections
import fnmatch
import re
import sys

PREFIX = "dEQP-GLES3."
LINE_RE = re.compile(r"^(?:(\S+?):)?([A-Za-z]+)\s+(dEQP-\S+)")


def parse_notpass(path):
    rows = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            if "\t" in line:
                p = line.split("\t")
                rows.append((p[0].strip(), p[1].strip() if len(p) > 1 else "",
                             p[2].strip() if len(p) > 2 else "",
                             p[3].strip() if len(p) > 3 else ""))
            else:
                m = LINE_RE.match(line)
                if not m:
                    sys.exit(f"{path}: unrecognised row: {line[:80]}")
                rows.append((m.group(2), m.group(3), "", ""))
    return rows


def has_family_column(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if line.strip() and not line.startswith("#") and line.count("\t") >= 3:
                return True
    return False


def parse_families(path):
    """[(family, [globs], regex or None)] in file order."""
    rules = []
    with open(path, encoding="utf-8") as f:
        for n, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            if not line.strip() or (line.startswith("#") and not re.match(r"#\d", line)):
                continue
            p = line.split("\t")
            if len(p) < 2 or not p[0].strip() or not p[1].strip():
                sys.exit(f"{path}:{n}: need <family>TAB<globs>[TAB<regex>]")
            rx = re.compile(p[2]) if len(p) > 2 and p[2] else None
            rules.append((p[0].strip(), [g.strip() for g in p[1].split(",") if g.strip()], rx))
    return rules


def family_of(rules, case, msg):
    for fam, globs, rx in rules:
        if any(fnmatch.fnmatchcase(case, g) for g in globs) and (rx is None or rx.search(msg)):
            return fam
    return "unassigned"


def apply_families(src, rules, out):
    with open(src, encoding="utf-8", errors="replace") as f, \
            open(out, "w", encoding="utf-8") as o:
        for raw in f:
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                o.write(line + "\n")
                continue
            if "\t" in line:
                p = line.split("\t")
            else:
                m = LINE_RE.match(line)
                if not m:
                    sys.exit(f"{src}: unrecognised row: {line[:80]}")
                p = [m.group(2), m.group(3)]
            while len(p) < 4:
                p.append("")
            if not p[3].strip():
                p[3] = family_of(rules, p[1].strip(), p[2].strip())
            o.write("\t".join(p[:4]) + "\n")


def check(rows):
    bad = [r for r in rows if not r[0] or not r[3] or r[3] == "unassigned"]
    if bad:
        print(f"check: {len(bad)} of {len(rows)} rows lack an outcome or a family:",
              file=sys.stderr)
        for o, c, _m, fam in bad:
            print(f"  [{o or '<empty outcome>'}] [{fam or '<empty family>'}] {c}", file=sys.stderr)
        return 1
    cnt = collections.Counter(r[3] for r in rows)
    for fam, n in sorted(cnt.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"{fam}\t{n}")
    print(f"total\t{len(rows)}")
    return 0


def parse_host(path, gpu):
    """group -> (pass, fail, notsupported, crash, other) for one gpu."""
    out = {}
    cols = None
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            p = line.split("\t")
            if p[0] == "gpu":
                cols = p
                continue
            if cols is None or p[0] != gpu:
                continue
            d = dict(zip(cols, p))
            out[d["group"]] = tuple(int(d[k]) for k in
                                    ("pass", "fail", "notsupported", "crash", "other"))
    if not out:
        sys.exit(f"{path}: no rows for gpu '{gpu}'")
    return out


def host_verdict(counts):
    if counts is None:
        return "host-unknown"
    p, f, _ns, c, o = counts
    if f + c > 0 and p == 0:
        return "host-also-fails"
    if p > 0 and f + c + o == 0:
        return "board-only"
    if p == 0 and f + c + o == 0:
        return "host-notsupported"
    return "host-mixed"


def parse_table(path):
    ents = []
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.strip()
            if line and not line.startswith("#"):
                ents.append([g for g in line.split(",") if g])
    return ents


def find_entry(ents, case):
    for i, pats in enumerate(ents):
        if any(fnmatch.fnmatchcase(case, g) for g in pats):
            return i
    return None


def host_group(host, rel):
    """Longest host group that is a dotted prefix of rel."""
    parts = rel.split(".")
    for n in range(len(parts), 0, -1):
        g = ".".join(parts[:n])
        if g in host:
            return host[g]
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("notpass")
    ap.add_argument("--host")
    ap.add_argument("--gpu", default="vivante", help="host gpu column (default vivante)")
    ap.add_argument("--table")
    ap.add_argument("--depth", type=int, default=None)
    ap.add_argument("--families", help="family rule file (am62p-gles3-families.txt)")
    ap.add_argument("--apply", metavar="OUT.tsv", help="with --families: write the 4-column copy")
    ap.add_argument("--check", action="store_true", help="every row has an outcome and a family")
    ap.add_argument("--examples", type=int, default=5)
    a = ap.parse_args()
    if a.depth is not None and a.depth < 1:
        sys.exit("--depth must be >= 1")
    if bool(a.apply) != bool(a.families):
        sys.exit("--families and --apply go together")
    if a.apply:
        apply_families(a.notpass, parse_families(a.families), a.apply)
        return 0

    rows = parse_notpass(a.notpass)
    if a.check:
        return check(rows)
    by_family = a.depth is None and has_family_column(a.notpass)
    if a.depth is None:
        a.depth = 4
    host = parse_host(a.host, a.gpu) if a.host else None
    ents = parse_table(a.table) if a.table else None

    groups = collections.OrderedDict()
    for outcome, case, msg, fam in rows:
        rel = case[len(PREFIX):] if case.startswith(PREFIX) else case
        key = (fam or "unassigned") if by_family else ".".join(rel.split(".")[:a.depth])
        g = groups.setdefault(key, {"cases": [], "out": collections.Counter(),
                                    "ent": collections.Counter(),
                                    "host": collections.Counter(),
                                    "msg": collections.Counter()})
        g["cases"].append(case)
        g["out"][outcome] += 1
        if msg and msg != outcome:
            g["msg"][msg[:100]] += 1
        if ents is not None:
            e = find_entry(ents, case)
            g["ent"]["e%02d" % e if e is not None else "none"] += 1
        if host is not None:
            g["host"][host_verdict(host_group(host, rel))] += 1

    order = sorted(groups, key=lambda k: (-len(groups[k]["cases"]), k))
    print(f"# dEQP-GLES3 not-pass triage: {len(rows)} cases in {len(groups)} groups "
          f"({'by family' if by_family else 'depth %d' % a.depth})\n")
    print(f"Input: `{a.notpass}`" + (f"; host: `{a.host}` (gpu {a.gpu})" if host else "")
          + (f"; table: `{a.table}`" if ents is not None else "") + "\n")
    for k in order:
        g = groups[k]
        n = len(g["cases"])
        mix = ", ".join(f"{o} {c}" for o, c in g["out"].most_common())
        dom = g["out"].most_common(1)[0][0]
        print(f"## {k}\n")
        print(f"- cases: {n}")
        if by_family:
            print(f"- family: {k} ({n})")
        print(f"- verdicts: {mix}")
        if ents is not None:
            print("- table entries: " + ", ".join(f"{e} ({c})" for e, c in sorted(g["ent"].items())))
        if host is not None:
            h = g["host"]
            print(f"- host-also-fails: {h['host-also-fails']}")
            print(f"- board-only: {h['board-only']}")
            rest = ", ".join(f"{v} {c}" for v, c in sorted(h.items())
                             if v not in ("host-also-fails", "board-only"))
            if rest:
                print(f"- other host: {rest}")
        if g["msg"]:
            print("- messages: " + "; ".join(f"`{m}` ({c})" for m, c in g["msg"].most_common(3)))
        print("- examples:")
        for c in sorted(g["cases"])[:a.examples]:
            print(f"  - `{c}`")
        scope = "GL core / shader compiler" if host is not None and \
            g["host"]["host-also-fails"] * 2 > n else "GPU"
        print(f"- suggested title: `dEQP-GLES3 {k}: {n} {dom} cases ({scope})`\n")


if __name__ == "__main__":
    sys.exit(main() or 0)
