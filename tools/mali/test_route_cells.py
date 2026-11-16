#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of route_cells.py on synthetic route.sh rows.

    python3 -I tools/mali/test_route_cells.py

Each test runs the tool as a command, as a reviewer does.
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "route_cells.py")


def rows(*r):
    return "".join("%s\t%s\t%s\t%s\n" % x for x in r)


class RouteCells(unittest.TestCase):
    def run_tool(self, main, tip):
        with tempfile.TemporaryDirectory() as d:
            a, b = os.path.join(d, "main.tsv"), os.path.join(d, "tip.tsv")
            for p, text in ((a, main), (b, tip)):
                with open(p, "w") as f:
                    f.write(text)
            r = subprocess.run([sys.executable, "-I", TOOL, a, b], capture_output=True, text=True)
        return r.returncode, r.stdout + r.stderr

    def test_no_move_passes(self):
        m = rows(("a", "EQUAL", "1", "-"), ("b", "REFUSED", "-", "fs (-95): x"), ("c", "DIFFER", "3", "vs: y"))
        rc, out = self.run_tool(m, m)
        self.assertEqual(rc, 0, out)
        self.assertIn("EQUAL -> same words", out)
        self.assertIn("REFUSED -> REFUSED", out)
        self.assertIn("rule 1: 0", out)
        self.assertIn("rule 2: 0", out)
        self.assertIn("compiled, not Mesa's: main 1, tip 1", out)

    def test_equal_to_refused_breaks_rule_1(self):
        rc, out = self.run_tool(rows(("a", "EQUAL", "1", "-")), rows(("a", "REFUSED", "-", "link (-95): z")))
        self.assertEqual(rc, 1, out)
        self.assertIn("rule 1: 1", out)
        self.assertIn("rule 1\ta: EQUAL -> REFUSED: link (-95): z", out)

    def test_equal_to_differ_breaks_rule_1(self):
        rc, out = self.run_tool(rows(("a", "EQUAL", "1", "-")), rows(("a", "DIFFER", "2", "vs: w")))
        self.assertEqual(rc, 1, out)
        self.assertIn("EQUAL -> DIFFER (other words)", out)
        self.assertIn("rule 1: 1", out)

    def test_refused_to_differ_breaks_rule_2(self):
        rc, out = self.run_tool(rows(("a", "REFUSED", "-", "vs (-95): q")), rows(("a", "DIFFER", "2", "fs: w")))
        self.assertEqual(rc, 1, out)
        self.assertIn("rule 2: 1", out)
        self.assertIn("compiled, not Mesa's: main 0, tip 1", out)

    def test_gains_and_fixes_pass(self):
        m = rows(("a", "REFUSED", "-", "fs (-95): q"), ("b", "DIFFER", "2", "vs: w"), ("c", "DIFFER", "4", "vs: w"))
        t = rows(("a", "EQUAL", "1", "-"), ("b", "EQUAL", "3", "-"), ("c", "REFUSED", "-", "link (-95): r"))
        rc, out = self.run_tool(m, t)
        self.assertEqual(rc, 0, out)
        self.assertIn("REFUSED -> EQUAL", out)
        self.assertIn("DIFFER -> EQUAL (other words)", out)
        self.assertIn("DIFFER -> REFUSED", out)
        self.assertIn("compiled, not Mesa's: main 2, tip 0", out)

    def test_a_moved_program_without_a_reference_fails(self):
        rc, out = self.run_tool(rows(("a", "REFUSED", "-", "fs (-95): q")), rows(("a", "NOREF", "1", "-")))
        self.assertEqual(rc, 1, out)
        self.assertIn("needs a reference: 1", out)
        rc, out = self.run_tool(rows(("a", "NOREF", "1", "-")), rows(("a", "NOREF", "1", "-")))
        self.assertEqual(rc, 0, out)
        self.assertIn("NOREF -> same words", out)

    def test_a_row_in_one_file_only_fails(self):
        rc, out = self.run_tool(rows(("a", "EQUAL", "1", "-"), ("b", "EQUAL", "2", "-")), rows(("a", "EQUAL", "1", "-")))
        self.assertEqual(rc, 1, out)
        self.assertIn("in one file only: 1", out)

    def test_a_malformed_row_is_exit_2(self):
        rc, out = self.run_tool("a\tSAME\t1\t-\n", rows(("a", "EQUAL", "1", "-")))
        self.assertEqual(rc, 2, out)
        self.assertIn("not a route row", out)
        rc, out = self.run_tool(rows(("a", "EQUAL", "1", "-"), ("a", "EQUAL", "1", "-")), rows(("a", "EQUAL", "1", "-")))
        self.assertEqual(rc, 2, out)
        self.assertIn("a twice", out)


if __name__ == "__main__":
    unittest.main()
