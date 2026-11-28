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

Stale rules (--stale RULES, default tools/cts/am62p-gles3-stale.txt, used when
it exists; --no-stale turns the class off): a row recorded before a merged fix
is reported as "stale: expected flip on rerun (<commit>)", not as an open
family. RULES lines, tab separated:
  <family-id> TAB <case globs> TAB <message regex or -> TAB <commit> [TAB <note>]
A row matches when its case glob and message regex (re.search) match. The row's
build comes from the NOTPASS header lines "# rows A-B: build <sha>, run <date>"
(row ranges of the tree, --tree, default tools/cts/am62p-gles3-tree.txt, counted
from 0). The row is stale when its build is older than the commit: both resolve
in git and the build is a proper ancestor of the commit, or (a sha git does not
know) the header's last run date is before the commit's date. A commit written
'pending:<item>' is a fix not on main yet: the rule never makes a row stale. A
row built at or after the commit stays open. The report prints stale and open
totals, and per group when it holds stale rows.

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
import os
import re
import subprocess
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


HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
DEFAULT_STALE = os.path.join(HERE, "am62p-gles3-stale.txt")
DEFAULT_TREE = os.path.join(HERE, "am62p-gles3-tree.txt")
BUILD_RE = re.compile(r"^#\s*rows\s+(\d+)(?:-(\d+))?:\s*build\s+([0-9a-fA-F]{7,40})"
                      r"(?:,\s*run\s+([0-9/-]+))?")


def parse_stale(path):
    """[(family, [globs], regex or None, commit)] in file order."""
    rules = []
    with open(path, encoding="utf-8") as f:
        for n, raw in enumerate(f, 1):
            line = raw.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            p = line.split("\t")
            if len(p) < 4 or not all(x.strip() for x in p[:4]):
                sys.exit(f"{path}:{n}: need <family>TAB<globs>TAB<regex or ->TAB<commit>")
            rx = None if p[2].strip() == "-" else re.compile(p[2])
            rules.append((p[0].strip(), [g.strip() for g in p[1].split(",") if g.strip()],
                          rx, p[3].strip()))
    return rules


def parse_builds(path):
    """[(first, last, build sha, run date or '')] from the header lines."""
    out = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            if not raw.startswith("#"):
                continue
            m = BUILD_RE.match(raw)
            if m:
                a = int(m.group(1))
                run = m.group(4) or ""
                if "/" in run:  # 2026-10-13/14 -> 2026-10-14
                    head, tail = run.split("/", 1)[0], run.split("/")[-1]
                    run = head[:len(head) - len(tail)] + tail
                out.append((a, int(m.group(2) or a), m.group(3), run))
    return out


def _git(*args):
    try:
        r = subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True)
    except OSError:
        return None
    return r.stdout.strip() if r.returncode == 0 else None


def _resolve(sha):
    return _git("rev-parse", "--verify", "--quiet", sha + "^{commit}")


def older_than(build, run, commit):
    """True when `build` (run on `run`) predates `commit`; False when unknown or pending."""
    if commit.startswith("pending:"):
        return False
    c = _resolve(commit)
    if c is None:
        return False
    b = _resolve(build)
    if b is not None:
        if b == c:
            return False
        r = subprocess.run(["git", "-C", ROOT, "merge-base", "--is-ancestor", b, c])
        return r.returncode == 0
    cd = _git("show", "-s", "--format=%cs", c)
    return bool(cd and run and run < cd)


def stale_of(rules, builds, ents, case, msg):
    """(family-id, commit) when a stale rule covers the row, else None."""
    if not rules or not builds or ents is None:
        return None
    e = find_entry(ents, case)
    if e is None:
        return None
    for a, b, sha, run in builds:
        if a <= e <= b:
            break
    else:
        return None
    for fam, globs, rx, commit in rules:
        if any(fnmatch.fnmatchcase(case, g) for g in globs) and (rx is None or rx.search(msg)) \
                and older_than(sha, run, commit):
            return fam, commit
    return None


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
    ap.add_argument("--stale", help="stale rule file (default tools/cts/am62p-gles3-stale.txt)")
    ap.add_argument("--no-stale", action="store_true", help="do not mark rows stale")
    ap.add_argument("--tree", help="tree file for the row->build map (default am62p-gles3-tree.txt)")
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

    srules = builds = tree = None
    spath = a.stale or (DEFAULT_STALE if os.path.exists(DEFAULT_STALE) else None)
    if not a.no_stale and spath:
        srules = parse_stale(spath)
        builds = parse_builds(a.notpass)
        tpath = a.tree or DEFAULT_TREE
        tree = parse_table(tpath) if os.path.exists(tpath) else None
        if not builds or tree is None:
            srules = None
    nstale = collections.Counter()

    groups = collections.OrderedDict()
    for outcome, case, msg, fam in rows:
        rel = case[len(PREFIX):] if case.startswith(PREFIX) else case
        key = (fam or "unassigned") if by_family else ".".join(rel.split(".")[:a.depth])
        g = groups.setdefault(key, {"stale": collections.Counter(), "cases": [], "out": collections.Counter(),
                                    "ent": collections.Counter(),
                                    "host": collections.Counter(),
                                    "msg": collections.Counter()})
        g["cases"].append(case)
        st = stale_of(srules, builds, tree, case, msg)
        if st:
            g["stale"][st] += 1
            nstale[st] += 1
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
    if srules is not None:
        ns = sum(nstale.values())
        print(f"Stale (a merged fix is newer than the row's build; expected to flip on rerun): "
              f"{ns}; open: {len(rows) - ns}; total: {len(rows)}\n")
        for (fam, commit), c in sorted(nstale.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"- stale {fam}: {c} (expected flip on rerun, {commit})")
        if nstale:
            print()
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
        if g["stale"]:
            ns = sum(g["stale"].values())
            print(f"- stale: {ns}, open: {n - ns}")
            for (fam, commit), c in sorted(g["stale"].items()):
                print(f"  - stale: expected flip on rerun ({commit}): {fam} {c}")
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
