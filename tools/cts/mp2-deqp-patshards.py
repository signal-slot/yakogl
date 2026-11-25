#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Cut the DK's dEQP-GLES2 list into pattern shards that fit the case slot.

    python3 -I tools/cts/mp2-deqp-patshards.py --out DIR [--budget-s 450]
        [--case-ms 150] [--group-ms 'texture.*=750' ...]
        [--case-bytes 300] [--group-bytes 'texture.*=1300' ...]
        [--console-bytes 1048576] [--slot 16384]
        [--list mp2-deqp-gles2.txt] [--exclude mp2-deqp-gles2-exclude.txt]
        [--mustpass gles2-mustpass.txt]

The DK dEQP image takes its filter from a 16 KiB case slot
(tools/cts/mp2-deqp-slot.py, driver/samples/zephyr/deqp/src/main.cpp): a
--deqp-case value of ','-separated patterns, at most --slot minus 1 bytes
(the slot needs a NUL after the filter). A flat list of 3,000 names is
266 KB, so a shard is written as prefix patterns instead ("a.b.c.*", dEQP
matches '*' against any run of characters).

The case set is the one tools/cts/mp2-deqp-list-check.sh counts: the cases of
gles2-mustpass.txt that a line of mp2-deqp-gles2.txt covers (the line is the
case or a "prefix*" glob) and no line of mp2-deqp-gles2-exclude.txt (name or
glob, a TAB and a reason) covers. A prefix pattern is used for a name
component only when every mustpass case below it is in the set, so the
excludes are honoured by construction; otherwise the pattern recurses into
the children, down to single case names. Units are taken in mustpass order
and packed greedily into a shard until the next one would pass the time
budget (--budget-s, at --case-ms a case, the first matching --group-ms
PATTERN=MS overriding it; PATTERN is an fnmatch pattern on the case name, or
matched after any '.', so 'texture.*' means every case below a "texture"
component), the console budget (--console-bytes, default 1 MiB, at
--case-bytes a case, --group-bytes PATTERN=BYTES the same way) or the slot.
A unit that does not fit a shard alone is split into its children; a single
case that does not fit alone is a refusal (exit 1, nothing written).

For the DK sweep the texture groups overran the 600 s lifetime at about 800
cases and filled the 1 MiB console, so use roughly
  --group-ms 'texture.*=750' --group-bytes 'texture.*=1300'.

Output in --out: filter-NNNN.txt (the filter, one line), excluded.txt (the
exclude names), manifest.tsv (index, cases, est_s, est_bytes, filter_bytes,
file). The run is deterministic. It prints the case count, the shard count,
the largest filter ("of SLOT-1") and the extra cases the patterns run:
mustpass cases a pattern matches that are not in the set, counted against
gles2-mustpass.txt only, not against the image's full dEQP case tree (the
prefix rule makes it 0 there; the image's tree can hold names mustpass lacks).
Exit 0 done, 1 refusal or bad input, 2 usage.
"""
import argparse
import fnmatch
import os
import sys


def read_lines(path):
    with open(path, encoding="utf-8") as f:
        for raw in f:
            s = raw.rstrip("\n")
            if s.strip() and not s.lstrip().startswith("#"):
                yield s


def covers(pat, name):
    """mp2-deqp-list-check.sh's under(): the name itself or a '*'-ended prefix."""
    if pat == name:
        return True
    return pat.endswith("*") and name.startswith(pat[:-1])


def expand(mustpass, listf, exclf):
    """The ordered mustpass list, the case set and the exclude names."""
    must, seen = [], set()
    for ln in read_lines(mustpass):
        ln = ln.strip()
        if ln not in seen:
            seen.add(ln)
            must.append(ln)
    groups = [ln.strip() for ln in read_lines(listf)]
    excl = [ln.split("\t")[0].strip() for ln in read_lines(exclf)]
    cases = [c for c in must
             if any(covers(g, c) for g in groups) and not any(covers(x, c) for x in excl)]
    return must, cases, excl


class Node:
    __slots__ = ("name", "kids", "total", "inc", "is_case")

    def __init__(self, name):
        self.name = name
        self.kids = {}
        self.total = 0
        self.inc = 0
        self.is_case = False


def build(must, inc):
    root = Node("")
    for c in must:
        node = root
        node.total += 1
        node.inc += c in inc
        parts = c.split(".")
        for i, p in enumerate(parts):
            node = node.kids.setdefault(p, Node(".".join(parts[: i + 1])))
            node.total += 1
            node.inc += c in inc
        node.is_case = True
    return root


def lookup(rules, name, default):
    for pat, val in rules:
        if fnmatch.fnmatchcase(name, pat) or fnmatch.fnmatchcase(name, "*." + pat):
            return val
    return default


def case_names(node, inc):
    if node.is_case and node.name in inc:
        yield node.name
    for kid in node.kids.values():
        yield from case_names(kid, inc)


def units(node, inc):
    """Patterns for the node's included cases in mustpass order: (text, names, node)."""
    if node.inc == 0:
        return
    if node.inc == node.total and not node.is_case and node.kids:
        yield (node.name + ".*", list(case_names(node, inc)), node)
        return
    if node.is_case and node.name in inc:
        yield (node.name, [node.name], None)
    for kid in node.kids.values():
        yield from units(kid, inc)


def plan(must, cases, a):
    inc = set(cases)
    root = build(must, inc)
    room = a.slot - 1  # filter[length] must be the NUL
    limit_ms = a.budget_s * 1000

    def cost(names):
        return (sum(lookup(a.group_ms, n, a.case_ms) for n in names),
                sum(lookup(a.group_bytes, n, a.case_bytes) for n in names))

    def fit(unit):
        """A unit that does not fit a shard alone becomes its children's units."""
        pat, names, node = unit
        ms, by = cost(names)
        if len(pat) <= room and ms <= limit_ms and by <= a.console_bytes:
            return [unit]
        if node is None or not node.kids:
            why = []
            if len(pat) > room:
                why.append("filter %d bytes > slot room %d" % (len(pat), room))
            if ms > limit_ms:
                why.append("%g ms > budget %g ms" % (ms, limit_ms))
            if by > a.console_bytes:
                why.append("%g bytes > console budget %d" % (by, a.console_bytes))
            raise ValueError("%s does not fit a shard alone: %s" % (pat, ", ".join(why)))
        out = []
        for kid in node.kids.values():
            for u in units(kid, inc):
                out += fit(u)
        return out

    flat = []
    for kid in root.kids.values():
        for u in units(kid, inc):
            flat += fit(u)

    shards, cur, cur_ms, cur_by, cur_len = [], [], 0, 0, 0
    for pat, names, _ in flat:
        ms, by = cost(names)
        add_len = len(pat) + (1 if cur else 0)
        if cur and (cur_ms + ms > limit_ms or cur_by + by > a.console_bytes
                    or cur_len + add_len > room):
            shards.append((cur, cur_ms, cur_by, cur_len))
            cur, cur_ms, cur_by, cur_len = [], 0, 0, 0
            add_len = len(pat)
        cur.append((pat, names))
        cur_ms += ms
        cur_by += by
        cur_len += add_len
    if cur:
        shards.append((cur, cur_ms, cur_by, cur_len))
    return shards


def check(must, cases, shards):
    """Every case covered exactly once; the extra mustpass cases the patterns run."""
    inc = set(cases)
    got = {}
    extra = 0
    for items, _, _, _ in shards:
        exact = {p for p, _ in items if not p.endswith("*")}
        pre = {p[:-1] for p, _ in items if p.endswith("*")}
        for c in must:
            if c in exact or any(c[: i + 1] in pre for i, ch in enumerate(c) if ch == "."):
                if c in inc:
                    got[c] = got.get(c, 0) + 1
                else:
                    extra += 1
    for c in cases:
        if got.get(c, 0) != 1:
            raise ValueError("internal: %s is covered %d times" % (c, got.get(c, 0)))
    return extra


def rule(s):
    p, sep, v = s.rpartition("=")
    try:
        if not sep or not p:
            raise ValueError
        return p, float(v)
    except ValueError:
        raise argparse.ArgumentTypeError("not PATTERN=NUMBER: %r" % s)


def main(argv):
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--list", default=os.path.join(here, "mp2-deqp-gles2.txt"))
    ap.add_argument("--exclude", default=os.path.join(here, "mp2-deqp-gles2-exclude.txt"))
    ap.add_argument("--mustpass", default=os.path.join(here, "gles2-mustpass.txt"))
    ap.add_argument("--budget-s", type=float, default=450)
    ap.add_argument("--case-ms", type=float, default=150)
    ap.add_argument("--group-ms", type=rule, action="append", default=[])
    ap.add_argument("--case-bytes", type=float, default=300)
    ap.add_argument("--group-bytes", type=rule, action="append", default=[])
    ap.add_argument("--console-bytes", type=int, default=1 << 20)
    ap.add_argument("--slot", type=int, default=16384)
    a = ap.parse_args(argv)
    if a.budget_s <= 0 or a.case_ms < 0 or a.case_bytes < 0 or a.console_bytes <= 0 or a.slot < 2:
        ap.error("budget, console and slot must be positive, costs not negative")
    try:
        must, cases, excl = expand(a.mustpass, a.list, a.exclude)
        shards = plan(must, cases, a)
        extra = check(must, cases, shards)
    except (OSError, ValueError) as e:
        print("mp2-deqp-patshards: %s" % e, file=sys.stderr)
        return 1
    os.makedirs(a.out, exist_ok=True)
    rows = ["index\tcases\test_s\test_bytes\tfilter_bytes\tfile"]
    for i, (items, ms, by, ln) in enumerate(shards):
        fn = "filter-%04d.txt" % i
        with open(os.path.join(a.out, fn), "w", encoding="ascii", newline="\n") as f:
            f.write(",".join(p for p, _ in items) + "\n")
        rows.append("%d\t%d\t%.1f\t%d\t%d\t%s"
                    % (i, sum(len(n) for _, n in items), ms / 1000.0, by, ln, fn))
    with open(os.path.join(a.out, "manifest.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(rows) + "\n")
    with open(os.path.join(a.out, "excluded.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("".join(x + "\n" for x in excl))
    biggest = max((s[3] for s in shards), default=0)
    print("mp2-deqp-patshards: %d cases, %d shards, largest filter %d bytes of %d, "
          "%d extra cases (against gles2-mustpass.txt)"
          % (len(cases), len(shards), biggest, a.slot - 1, extra))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
