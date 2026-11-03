#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of gl_ref.py's `--instances`/`--divisors` layout and of
pan_ref_diff.py's `--instanced` extraction that need no Mesa build.

    python3 -I tools/mali/test_gl_ref.py

The instanced layout is gl_test case 141's (es3instdiv): `pos` per vertex
from a buffer of six vec4s, `offs` and `icol` interleaved in a buffer of
two 24-byte records with divisor 2. The trace snippet is the shape of the
pandecode lines of tools/mali/testdata/instdiv_141_es3instdiv.expect.json's
run (Mesa 26.2.1 on drm-shim).
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gl_ref  # noqa: E402
import pan_ref_diff  # noqa: E402

INST_VERT = os.path.join(HERE, "testdata", "instdiv_141_es3instdiv.vert")
INST_FRAG = os.path.join(HERE, "testdata", "instdiv_141_es3instdiv.frag")

TRACE = """\
      Entry 1 @7ffffffce7d0:
        Address: 0x7ffffffce160
          Attribute @7ffffffce160:
            Type: Attribute
            Attribute type: 1D
            Offset enable: true
            Format (v7): RGBA32F RGBA
            Table: 2
            Frequency: Vertex
            Divisor R: 0
            Divisor E: 0
            Offset: 0
            Buffer index: 0x0
            Stride: 0x10
            Attribute stride: 16
            Packet stride: 0
            Divisor D: 0x0
          Attribute @7ffffffce180:
            Type: Attribute
            Attribute type: 1D POT Divisor
            Offset enable: false
            Format (v7): RG32F RGB1
            Table: 2
            Frequency: Instance
            Divisor R: 1
            Divisor E: 0
            Offset: 0
            Buffer index: 0x1
            Stride: 0x18
            Attribute stride: 24
            Packet stride: 0
            Divisor D: 0x0
      Entry 2 @7ffffffce7e0:
          Buffer @7ffffffce1c0:
            Type: Buffer
            Buffer type: Simple
            Size: 0x60
            Address: 0x7fffffffe6000
            Stride: 0x0
          Buffer @7ffffffce1e0:
            Type: Buffer
            Buffer type: Simple
            Size: 0x30
            Address: 0x7fffffffdd000
            Stride: 0x0
      Entry 3 @7ffffffce7f0:
Position FAU @7ffffffce000:
  Index count: 6
  Instance count: 4
  Vertex offset: 0
  Instance offset: 0
"""


class InstanceLayout(unittest.TestCase):
    def test_case_141(self):
        sizes = gl_ref.attribute_sizes_es3(open(INST_VERT).read())
        self.assertEqual(sizes, {"pos": 4, "offs": 2, "icol": 4})
        self.assertEqual(gl_ref.instance_layout([4, 2, 4], [0, 2, 2], 4),
                         (16, 24, 2, [(0, 0), (1, 0), (1, 8)]))

    def test_records_follow_the_smallest_divisor(self):
        self.assertEqual(gl_ref.instance_layout([4, 1, 3], [0, 3, 1], 5)[1:3], (16, 5))
        self.assertEqual(gl_ref.instance_layout([4, 1], [0, 3], 5)[2], 2)
        self.assertEqual(gl_ref.instance_layout([4, 2], [0, 0], 4), (24, 0, 0, [(0, 0), (0, 16)]))

    def test_bad_divisors_are_refused_before_any_library(self):
        for opts in (["--divisors", "0,2"], ["--divisors", "0,2,2,2"], ["--divisors", "0,-1,2"],
                     ["--instances", "0"]):
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "gl_ref.py"), "/nonexistent",
                                INST_VERT, INST_FRAG, *opts], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2, (opts, r.stderr))


class InstancedExtract(unittest.TestCase):
    def test_divisor_fields_and_draw(self):
        with tempfile.NamedTemporaryFile("w", suffix=".trace") as f:
            f.write(TRACE)
            f.flush()
            plain = pan_ref_diff.read_attributes(f.name)
            inst = pan_ref_diff.read_attributes(f.name, True)
        self.assertNotIn("draw", plain)
        self.assertEqual(sorted(plain["attributes"][0]), ["buffer", "format", "frequency", "offset", "stride"])
        a = inst["attributes"][1]
        self.assertEqual((a["type"], a["offset_enable"], a["divisor_r"], a["divisor_e"], a["divisor_d"]),
                         ("1D POT Divisor", False, 1, 0, 0))
        self.assertEqual(inst["attributes"][0]["type"], "1D")
        self.assertEqual(inst["attribute_buffers"][1]["size"], 0x30)
        self.assertEqual(inst["draw"], {"index_count": 6, "instance_count": 4, "instance_offset": 0})


if __name__ == "__main__":
    unittest.main(verbosity=2)
