#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Compare two CLIF dumps packet by packet.

Reads the text Mesa's V3D_DEBUG=clif writes (src/broadcom/clif/clif_dump.c and
the cle packet printer v3d_print_group), e.g. tools/v3d/shim71/*.clif. Every
`@format <kind>` section that holds packets (a control list, a shader record)
is a list, named <kind><ordinal of that kind>; `@add_bin`/`@add_render` job
submissions are packets of a list `submit`, their argument lines being the
fields arg0, arg1, ... Binary and blank sections are not compared.

Within a list the packets are aligned by name (difflib), then the fields of
paired packets are compared as text. A difference is printed as
    <list> #<n> <PACKET>.<field>: <a> != <b>
and a packet present on one side only as
    <list> #<n> <PACKET>: present != absent   (or absent != present)
(n counts from 0 in the dump that has the packet).

--ignore PACKET.field (repeatable; fnmatch wildcards, `*` in either part) and
--ignore-addresses (every field whose value is a buffer address: `[name+off]`
or `[null]`) leave fields out. Exit 0 equal, 1 different, 2 unreadable input.
The last line is `clif_cmp: N packets, D differences` (N: packets of A).
"""
import argparse
import difflib
import fnmatch
import re
import sys

PREFIX = re.compile(r"^MESA: info: ?")
PACKET = re.compile(r"^[A-Z][A-Z0-9_]*$")
FIELD = re.compile(r"^\s+/\*\s*(.*?):\s*\*/\s*(.*)$")
FORMAT = re.compile(r"^@format\s+(\S+)")
SUBMIT = re.compile(r"^@(add_bin|add_render)\b")


class Packet:
    def __init__(self, name):
        self.name = name
        self.fields = []  # (field, value)


def parse(path):
    """Return [(list name, [Packet])] in file order."""
    lists = []
    kinds = {}
    cur = None
    pkt = None
    sub = None
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = PREFIX.sub("", raw.rstrip("\n"))
            m = FORMAT.match(line)
            if m or line.startswith("@buffer"):
                cur = pkt = None
                if m and m.group(1) not in ("blank", "binary"):
                    k = m.group(1)
                    n = kinds.get(k, 0)
                    kinds[k] = n + 1
                    cur = []
                    lists.append((k + str(n), cur))
                continue
            m = SUBMIT.match(line)
            if m:
                if sub is None:
                    sub = []
                    lists.append(("submit", sub))
                cur = sub
                pkt = Packet(m.group(1).upper())
                cur.append(pkt)
                continue
            if cur is None or not line.strip():
                continue
            if cur is sub:
                if pkt is not None:
                    pkt.fields.append(("arg%d" % len(pkt.fields), line.strip()))
            elif PACKET.match(line):
                pkt = Packet(line)
                cur.append(pkt)
            elif pkt is not None:
                m = FIELD.match(line)
                if m:
                    pkt.fields.append((m.group(1), m.group(2)))
    return lists


def is_address(v):
    return v is not None and v.startswith("[")


def align(xs, ys):
    """Aligned index pairs (i, j); None for the side without the element."""
    sm = difflib.SequenceMatcher(None, xs, ys, autojunk=False)
    for op, i1, i2, j1, j2 in sm.get_opcodes():
        if op == "equal":
            for k in range(i2 - i1):
                yield i1 + k, j1 + k
        else:
            for i in range(i1, i2):
                yield i, None
            for j in range(j1, j2):
                yield None, j


def main():
    ap = argparse.ArgumentParser(description="compare two CLIF dumps packet by packet")
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--ignore", action="append", default=[], metavar="PACKET.field",
                    help="leave this field out (repeatable, * wildcards)")
    ap.add_argument("--ignore-addresses", action="store_true",
                    help="leave out every field whose value is a buffer address")
    args = ap.parse_args()
    pats = []
    for p in args.ignore:
        if "." not in p:
            ap.error("--ignore wants PACKET.field, got %r" % p)
        pats.append(tuple(p.split(".", 1)))

    def ignored(pk, fl, va, vb):
        if args.ignore_addresses and (is_address(va) or is_address(vb)):
            return True
        return any(fnmatch.fnmatchcase(pk, a) and fnmatch.fnmatchcase(fl, b) for a, b in pats)

    try:
        la, lb = parse(args.a), parse(args.b)
    except OSError as e:
        print("clif_cmp: %s" % e)
        return 2
    diffs = []
    npk = sum(len(p) for _, p in la)
    names_a = [n for n, _ in la]
    names_b = [n for n, _ in lb]
    for i, j in align(names_a, names_b):
        if j is None:
            diffs.append("%s: list only in A" % names_a[i])
            continue
        if i is None:
            diffs.append("%s: list only in B" % names_b[j])
            continue
        name, pa = la[i]
        pb = lb[j][1]
        for x, y in align([p.name for p in pa], [p.name for p in pb]):
            if y is None:
                diffs.append("%s #%d %s: present != absent" % (name, x, pa[x].name))
                continue
            if x is None:
                diffs.append("%s #%d %s: absent != present" % (name, y, pb[y].name))
                continue
            p, q = pa[x], pb[y]
            fa, fb = dict(p.fields), dict(q.fields)
            for fl in [f for f, _ in p.fields] + [f for f, _ in q.fields if f not in fa]:
                va, vb = fa.get(fl), fb.get(fl)
                if va == vb or ignored(p.name, fl, va, vb):
                    continue
                diffs.append("%s #%d %s.%s: %s != %s" % (
                    name, x, p.name, fl,
                    "<none>" if va is None else va, "<none>" if vb is None else vb))
    for d in diffs:
        print(d)
    print("clif_cmp: %d packets, %d differences" % (npk, len(diffs)))
    return 1 if diffs else 0


if __name__ == "__main__":
    sys.exit(main())
