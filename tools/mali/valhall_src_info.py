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

bir.rs `src_info`, `nr_srcs`, `nr_staging_srcs` and `is_signed` are compared
with this list by the Mali test
`the_source_table_equals_mesa_26_2_1_valhall_c` (tests.rs). Regenerate it
when the Mesa version moves; the script fails (exit 1, no output) on an
entry it cannot read.
"""

import re
import sys

FLAGS = ["absneg", "swizzle", "notted", "widen", "lanes", "halfswizzle", "lane", "combine"]
SIZES = {"VA_SIZE_8": 8, "VA_SIZE_16": 16, "VA_SIZE_32": 32, "VA_SIZE_64": 64}


def fail(msg):
    sys.stderr.write("valhall_src_info: %s\n" % msg)
    sys.exit(1)


def entries(text):
    head = "valhall_opcodes[BI_NUM_OPCODES] = {"
    if head not in text:
        fail("no valhall_opcodes[] table")
    body = text.split(head, 1)[1]
    for m in re.finditer(r"\[BI_OPCODE_(\w+)\] = \{\n(.*?)\n    \},\n", body, re.S):
        yield m.group(1), m.group(2)


def source(fields):
    for k, v in fields.items():
        if k in FLAGS and v not in ("0", "1"):
            fail("source field .%s = %s" % (k, v))
        if k not in FLAGS and k != "size":
            fail("unknown source field .%s" % k)
    flags = [k for k in FLAGS if fields.get(k) == "1"]
    size = fields.get("size", "VA_SIZE_8")
    if size not in SIZES:
        fail("source size %s" % size)
    return "%s/%d" % (",".join(flags) or "-", SIZES[size])


def line(name, ent):
    m = re.search(r"\.srcs = \{\n(.*?)\n        \},\n", ent, re.S)
    if not m and ".srcs = {\n        }," not in ent:
        fail("%s: no .srcs" % name)
    srcs = []
    if m:
        for s in re.findall(r"\{\n(.*?)\n            \}", m.group(1), re.S):
            srcs.append(source(dict(re.findall(r"\.(\w+) = (\w+),", s))))
    g = dict(re.findall(r"^        \.(\w+) = (\w+),$", ent, re.M))
    for k in ("is_signed", "nr_staging_srcs", "nr_srcs"):
        if k not in g:
            fail("%s: no .%s" % (name, k))
    if int(g["nr_staging_srcs"]) + int(g["nr_srcs"]) != len(srcs):
        fail("%s: %d sources, not nr_staging_srcs + nr_srcs" % (name, len(srcs)))
    return " ".join([name, g["is_signed"], g["nr_staging_srcs"], g["nr_srcs"]] + srcs)


def main(argv):
    if len(argv) != 1:
        fail("usage: valhall_src_info.py <valhall.c>")
    with open(argv[0]) as f:
        text = f.read()
    out = [line(n, e) for n, e in entries(text)]
    if not out:
        fail("no entry")
    sys.stdout.write("".join(l.rstrip() + "\n" for l in out))


if __name__ == "__main__":
    main(sys.argv[1:])
