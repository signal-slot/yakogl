#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of pan_ref_diff.py and ref_cases.py that need no Mesa build.

    python3 tools/mali/test_pan_ref_diff.py

The data is committed: tools/mali/testdata/pan_shc-out/ holds what pan_shc
printed for tri and tri_varying (a recorded run, stdout and stderr),
testdata/*.expect.json the shim driver's programs (Mesa 26.2.1) and
testdata/board-25.0.7/ the board's traces (Mesa 25.0.7) with their
expect.json, testdata/shim-25.0.7/ the traces of the 25.0.7 shim driver. Each test runs the tool as a command, as the gate does.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "testdata")
BOARD = os.path.join(DATA, "board-25.0.7")
OUT = os.path.join(DATA, "pan_shc-out")
sys.path.insert(0, HERE)
import pan_ref_diff  # noqa: E402


def load(path):
    with open(path) as f:
        return json.load(f)


def tool(*args):
    r = subprocess.run([sys.executable, os.path.join(HERE, "pan_ref_diff.py"), *args],
                       capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


def shc(program):
    return os.path.join(OUT, program + ".json"), os.path.join(OUT, program + ".disasm")


class PanRefDiff(unittest.TestCase):
    def mutated(self, program, edit):
        """A copy of testdata/<program>.expect.json after edit(dict); returns its path."""
        d = load(os.path.join(DATA, program + ".expect.json"))
        edit(d)
        fd, path = tempfile.mkstemp(suffix=".expect.json")
        with os.fdopen(fd, "w") as f:
            json.dump(d, f)
        self.addCleanup(os.unlink, path)
        return path

    def test_expect_equal(self):
        for program, n in (("tri", 2), ("tri_varying", 3)):
            rc, out = tool("diff", *shc(program), "--expect",
                           os.path.join(DATA, program + ".expect.json"))
            self.assertEqual(rc, 0, out)
            self.assertIn("%d region(s) compared, 0 differ or missing" % n, out)
            self.assertEqual(out.count("byte-equal"), n, out)
            self.assertEqual(out.count(" facts: equal"), n, out)

    def test_mutated_expect_fails(self):
        def word(d):
            r = d["regions"]["vs.position"]
            r["words"][0] = "%016x" % (int(r["words"][0], 16) ^ 1)

        def fau_constant(d):  # the word that pan_shc promoted to the constant 4
            d["regions"]["vs.position"]["fau"][5] = "00000005"

        def regs(d):
            d["regions"]["fs.fragment"]["work_reg_count"] = 64

        def preload(d):
            d["regions"]["vs.position"]["preload"] = "0x0000000000000000"

        def fau_sysval(d):  # a driver-filled word is state, not code: not compared
            d["regions"]["vs.position"]["fau"][0] = "deadbeef"

        for edit, what in ((word, "instruction words differ"), (fau_constant, "fau word 5"),
                           (regs, "work_reg_count"), (preload, "preload")):
            rc, out = tool("diff", *shc("tri"), "--expect", self.mutated("tri", edit))
            self.assertEqual(rc, 1, edit.__name__ + "\n" + out)
            self.assertIn(what if what != "instruction words differ" else "instructions differ",
                          out, edit.__name__)
        rc, out = tool("diff", *shc("tri"), "--expect", self.mutated("tri", fau_sysval))
        self.assertEqual(rc, 0, "a sysval word is not compared\n" + out)

    def test_secondary_offset(self):
        def edit(d):
            d["secondary_offset"] = 128

        rc, out = tool("diff", *shc("tri_varying"), "--expect", self.mutated("tri_varying", edit))
        self.assertEqual(rc, 1, out)
        self.assertIn("secondary_offset differs: pan_shc 256, reference 128", out)

    def test_region_pan_shc_has_and_the_file_lacks(self):
        def edit(d):
            del d["regions"]["vs.varying"]

        rc, out = tool("diff", *shc("tri_varying"), "--expect", self.mutated("tri_varying", edit))
        self.assertEqual(rc, 1, out)
        self.assertIn("vs.varying: the reference has no shader at VA (none)", out)

    def test_region_the_file_has_and_pan_shc_lacks(self):
        rc, out = tool("diff", *shc("tri"), "--expect", os.path.join(DATA, "tri_varying.expect.json"))
        self.assertEqual(rc, 1, out)
        self.assertIn("vs.varying: pan_shc has no such region", out)

    def test_draw_is_the_first_draw(self):
        # The board's tri trace has a "Varying shader" only in its SECOND draw
        # (Mesa's own blit). Read over the whole file, pan_ref_diff paired the
        # app's tri with a varying program of the blit and reported a
        # missing vs.varying.
        tri = pan_ref_diff.read_draw(os.path.join(BOARD, "tri.pandecode"))
        self.assertEqual(sorted(tri), ["Fragment", "Position"])
        trivar = pan_ref_diff.read_draw(os.path.join(BOARD, "tri_varying.pandecode"))
        self.assertEqual(sorted(trivar), ["Fragment", "Position", "Varying"])
        self.assertEqual(tri["Position"]["va"], 0x7fffffffc000)
        rc, out = tool("diff", *shc("tri"), os.path.join(BOARD, "tri.pandecode"), "--draw")
        self.assertEqual(rc, 1, out)  # 25.0.7 is not 26.2.1: the words differ
        self.assertNotIn("has no shader", out)
        self.assertIn("2 region(s) compared", out)

    def test_extract_reproduces_the_committed_files(self):
        for program in ("tri", "tri_varying"):
            rc, out = tool("extract", os.path.join(BOARD, program + ".pandecode"), "--draw")
            self.assertEqual(rc, 0, out)
            self.assertEqual(json.loads(out), load(os.path.join(BOARD, program + ".expect.json")))

    def test_diff_ref(self):
        for program, n in (("tri", 2), ("tri_varying", 3)):
            board = os.path.join(BOARD, program + ".expect.json")
            rc, out = tool("diff-ref", board, os.path.join(BOARD, program + ".pandecode"))
            self.assertEqual(rc, 0, out)
            self.assertIn("%d region(s) compared, 0 differ or missing" % n, out)
            rc, out = tool("diff-ref", board, os.path.join(DATA, program + ".expect.json"))
            self.assertEqual(rc, 1, "25.0.7 against 26.2.1\n" + out)
            self.assertIn("%d region(s) compared, " % n, out)
            self.assertNotIn(", 0 differ or missing", out)

    def test_board_equals_the_25_0_7_shim(self):
        # shim-25.0.7/*.trace: drm_shim_ref.sh run on a Mesa 25.0.7 build, same source
        for program, n in (("tri", 2), ("tri_varying", 3)):
            rc, out = tool("diff-ref", os.path.join(BOARD, program + ".pandecode"),
                           os.path.join(DATA, "shim-25.0.7", program + ".trace"))
            self.assertEqual(rc, 0, out)
            self.assertIn("%d region(s) compared, 0 differ or missing" % n, out)
            self.assertEqual(out.count("byte-equal"), n, out)

    def test_ref_cases_lint(self):
        r = subprocess.run([sys.executable, os.path.join(HERE, "ref_cases.py"), "--lint"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("75 case(s), 214 region(s)", r.stdout)

    def test_ref_cases_lint_refuses_a_bare_differs(self):
        # `differs:<cause>` alone matched any difference (it was read by prefix)
        with tempfile.NamedTemporaryFile("w", suffix=".txt") as f:
            f.write("mat4 ctl_mat4.vert tri.frag triangles vs.position=differs:mat-order\n")
            f.flush()
            r = subprocess.run([sys.executable, os.path.join(HERE, "ref_cases.py"), "--lint", f.name],
                               capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("neither `equal` nor", r.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
