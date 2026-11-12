#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of valhall_src_info.py that need no Mesa build.

    python3 -I tools/mali/test_valhall_src_info.py

The input is a small table written here in the layout of Mesa's generated
valhall.c (two entries, made up for the test). Each test runs the tool as a
command: the table as written gives its two lines; a table with a line the
tool does not recognise in full (a re-indented closing brace, a field
written without spaces, a size with an odd spacing) or with entries or
sources other than it declares makes it fail, naming the line.
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))

FLAGS = ["absneg", "swizzle", "notted", "widen", "lanes", "halfswizzle", "lane", "combine"]
TAIL = ["type_size = 32", "has_dest = 1", "is_signed = {signed}", "unit = VA_UNIT_FMA",
        "nr_srcs = {srcs}", "nr_staging_srcs = 0", "nr_staging_dests = 0", "clamp = 0",
        "saturate = 0", "rhadd = 0", "round_mode = 0", "condition = 0", "result_type = 0",
        "vecsize = 0", "register_format = 0", "slot = 0", "sr_count = 0",
        "sr_write_count = 0", "sr_control = 0"]


def source(set_flags, size):
    lines = ["            {"]
    lines += ["                .%s = %d," % (f, f in set_flags) for f in FLAGS]
    if size:
        lines.append("                .size = VA_SIZE_%d," % size)
    lines.append("            },")
    return lines


def entry(name, srcs, signed=0):
    lines = ["    [BI_OPCODE_%s] = {" % name, "        .exact = 0x1000ULL,", "        .srcs = {"]
    for s in srcs:
        lines += source(*s)
    lines.append("        },")
    lines += ["        .%s," % t.format(signed=signed, srcs=len(srcs)) for t in TAIL]
    lines.append("    },")
    return lines


def table():
    lines = ["#include \"valhall.h\"", "", "const uint32_t valhall_immediates[32] = {", "    0x0,", "};",
             "", "const struct va_opcode_info", "valhall_opcodes[BI_NUM_OPCODES] = {", ""]
    lines += entry("ONE_F32", [(["absneg", "swizzle"], 32), ([], None)])
    lines.append("")
    lines += entry("TWO_S32", [(["widen"], 16)], signed=1)
    lines += ["", "};", ""]
    return "\n".join(lines)


GOOD = ["ONE_F32 0 0 2 absneg,swizzle/32 -/8", "TWO_S32 1 0 1 widen/16"]


def run(text):
    with tempfile.NamedTemporaryFile("w", suffix=".c", delete=False) as f:
        f.write(text)
    try:
        r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "valhall_src_info.py"), f.name],
                           capture_output=True, text=True)
    finally:
        os.unlink(f.name)
    return r.returncode, r.stdout, r.stderr


class ValhallSrcInfo(unittest.TestCase):
    def assert_refused(self, text, line):
        rc, out, err = run(text)
        self.assertEqual((rc, out), (1, ""), err)
        self.assertIn(":%d:" % line, err)

    def line_of(self, text, needle, nth=0):
        hits = [i + 1 for i, l in enumerate(text.split("\n")) if l == needle]
        return hits[nth]

    def test_the_table_as_written(self):
        rc, out, err = run(table())
        self.assertEqual((rc, err), (0, ""))
        self.assertEqual(out.splitlines(), GOOD)

    def test_a_reindented_closing_brace_is_refused(self):
        t = table()
        n = self.line_of(t, "    },")
        lines = t.split("\n")
        lines[n - 1] = "  },"
        self.assert_refused("\n".join(lines), n)

    def test_a_field_without_spaces_is_refused(self):
        t = table()
        n = self.line_of(t, "                .absneg = 1,")
        self.assert_refused(t.replace("                .absneg = 1,", "                .absneg=1,", 1), n)

    def test_a_size_with_an_odd_spacing_is_refused(self):
        t = table()
        n = self.line_of(t, "                .size = VA_SIZE_32,")
        self.assert_refused(t.replace(".size = VA_SIZE_32,", ".size = VA_SIZE_32 ,", 1), n)

    def test_a_source_count_other_than_declared_is_refused(self):
        t = table().replace("        .nr_srcs = 2,", "        .nr_srcs = 1,")
        self.assert_refused(t, self.line_of(t, "    },"))

    def test_a_field_given_twice_is_refused(self):
        t = table().replace("        .rhadd = 0,", "        .rhadd = 0,\n        .rhadd = 0,", 1)
        self.assert_refused(t, self.line_of(t, "        .rhadd = 0,", 1))

    def test_an_entry_the_table_does_not_close_is_refused(self):
        t = table()
        t = t[: t.rindex("};")]
        self.assert_refused(t, len(t.split("\n")))


if __name__ == "__main__":
    unittest.main()
