#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Print the per-source facts of Mesa's generated Valhall opcode table.

    tools/mali/valhall_src_info.py <valhall.c> > tools/mali/testdata/valhall_src_info.txt

<valhall.c> is the file Mesa's valhall.c.py writes into a built tree
(build-pan/src/panfrost/compiler/bifrost/valhall/valhall.c of the
$MESA_PAN_TREE that tools/mali/build_pan_shc.sh builds). The output is
facts, not code: one line per `valhall_opcodes[]` entry,

    <BI_OPCODE name> <is_signed> <nr_staging_srcs> <nr_srcs> <source>...

and one <source> per entry of `.srcs` (the staging sources that are read,
then the sources, valhall.c.py), written `<flags>/<size>`: the flags set to
1 among absneg swizzle notted widen lanes halfswizzle lane combine, joined
by commas (`-` for none), and the size in bits (8 when the entry has no
`.size`, as the zeroed field reads: VA_SIZE_8 is 0, valhall.h).

The table is read line by line in the layout valhall.c.py writes. Every
line between `valhall_opcodes[BI_NUM_OPCODES] = {` and its `};` must be one
of that layout's lines in full, in its place; each entry must give every
field once, each source every flag once, and its sources must be as many
as its `nr_staging_srcs + nr_srcs`; the entries must be as many as the
table's `[BI_OPCODE_` lines. Anything else fails (exit 1, no output) with
the line number. tools/mali/test_valhall_src_info.py tests that.

bir.rs `src_info`, `nr_srcs`, `nr_staging_srcs` and `is_signed` are compared
with this list by the Mali test
`the_source_table_equals_mesa_26_2_1_valhall_c` (tests.rs). Regenerate it
when the Mesa version moves.
"""

import re
import sys

FLAGS = ["absneg", "swizzle", "notted", "widen", "lanes", "halfswizzle", "lane", "combine"]
SIZES = {"VA_SIZE_8": 8, "VA_SIZE_16": 16, "VA_SIZE_32": 32, "VA_SIZE_64": 64}
# the entry's fields after `.srcs`, in valhall.c.py's order
FIELDS = ["type_size", "has_dest", "is_signed", "unit", "nr_srcs", "nr_staging_srcs",
          "nr_staging_dests", "clamp", "saturate", "rhadd", "round_mode", "condition",
          "result_type", "vecsize", "register_format", "slot", "sr_count", "sr_write_count",
          "sr_control"]

HEAD = "valhall_opcodes[BI_NUM_OPCODES] = {"
ENTRY = re.compile(r"^    \[BI_OPCODE_(\w+)\] = \{$")
EXACT = re.compile(r"^        \.exact = 0x[0-9a-f]+ULL,$")
FLAG = re.compile(r"^                \.(%s) = ([01]),$" % "|".join(FLAGS))
SIZE = re.compile(r"^                \.size = (VA_SIZE_(?:8|16|32|64)),$")
FIELD = re.compile(r"^        \.(%s) = (\w+),$" % "|".join(FIELDS))
NUMBER = re.compile(r"^[0-9]+$")


class ReadError(Exception):
    pass


def read(lines):
    """The facts' lines of the table in `lines` (each without its newline)."""
    start = [i for i, l in enumerate(lines) if l == HEAD]
    if len(start) != 1:
        raise ReadError(0, "%d lines `%s`, not one" % (len(start), HEAD))
    out = []
    declared = 0
    entry = None   # (name, sources, fields) of the entry being read
    src = None     # the flags and size of the source being read
    state = "table"
    for i in range(start[0] + 1, len(lines)):
        n, l = i + 1, lines[i]

        def bad(why="not a line of the table's layout"):
            raise ReadError(n, "%s: %r" % (why, l))

        if "[BI_OPCODE_" in l:
            declared += 1
        if state == "table":
            if l == "":
                continue
            if l == "};":
                if declared != len(out):
                    raise ReadError(n, "%d entries read, %d declared" % (len(out), declared))
                return out
            m = ENTRY.match(l)
            if not m:
                bad()
            entry, state = (m.group(1), [], {}), "exact"
        elif state == "exact":
            if not EXACT.match(l):
                bad()
            state = "srcs"
        elif state == "srcs":
            if l != "        .srcs = {":
                bad()
            state = "source"
        elif state == "source":
            if l == "            {":
                src, state = {}, "flag"
            elif l == "        },":
                state = "field"
            else:
                bad()
        elif state == "flag":
            m, s = FLAG.match(l), SIZE.match(l)
            if m or s:
                k, v = (m.group(1), m.group(2)) if m else ("size", s.group(1))
                if k in src:
                    bad("given twice")
                src[k] = v
            elif l == "            },":
                missing = [f for f in FLAGS if f not in src]
                if missing:
                    bad("no ." + ", .".join(missing) + " before")
                flags = [f for f in FLAGS if src[f] == "1"]
                entry[1].append("%s/%d" % (",".join(flags) or "-", SIZES[src.get("size", "VA_SIZE_8")]))
                state = "source"
            else:
                bad()
        elif state == "field":
            m = FIELD.match(l)
            if m:
                if m.group(1) in entry[2]:
                    bad("given twice")
                entry[2][m.group(1)] = m.group(2)
            elif l == "    },":
                name, srcs, g = entry
                missing = [f for f in FIELDS if f not in g]
                if missing:
                    bad("%s: no .%s" % (name, ", .".join(missing)))
                for k in ("is_signed", "nr_staging_srcs", "nr_srcs"):
                    if not NUMBER.match(g[k]):
                        bad("%s: .%s = %s" % (name, k, g[k]))
                if int(g["nr_staging_srcs"]) + int(g["nr_srcs"]) != len(srcs):
                    bad("%s: %d sources, not nr_staging_srcs + nr_srcs" % (name, len(srcs)))
                out.append(" ".join([name, g["is_signed"], g["nr_staging_srcs"], g["nr_srcs"]] + srcs))
                entry, state = None, "table"
            else:
                bad()
    raise ReadError(len(lines), "the table ends without `};`")


def main(argv):
    if len(argv) != 1:
        sys.stderr.write("usage: valhall_src_info.py <valhall.c>\n")
        sys.exit(1)
    with open(argv[0]) as f:
        lines = f.read().split("\n")
    try:
        out = read(lines)
    except ReadError as e:
        sys.stderr.write("valhall_src_info: %s:%d: %s\n" % (argv[0], e.args[0], e.args[1]))
        sys.exit(1)
    if not out:
        sys.stderr.write("valhall_src_info: %s: no entry\n" % argv[0])
        sys.exit(1)
    sys.stdout.write("".join(l + "\n" for l in out))


if __name__ == "__main__":
    main(sys.argv[1:])
