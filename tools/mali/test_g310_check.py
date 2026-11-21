#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of g310_check.py that need no Mesa build.

    python3 tools/mali/test_g310_check.py

The option-set selection, the summary and diff-ref parsing, and the choice of
references are checked with stand-in extracts; no shim is drawn.
"""
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import g310_check as g  # noqa: E402


def ref(words="aa", fau="bb", trace="x.trace"):
    return {"trace": trace, "secondary_offset": 8,
            "regions": {"fs.fragment": {"va": "0x1", "words": [words], "fau": [fau],
                                        "work_reg_count": 4, "preload": "0x0"}}}


class PickOptionSet(unittest.TestCase):
    def pick(self, results, committed):
        calls = []

        def extract_for(opts):
            calls.append(opts)
            return results.get(tuple(opts))
        return g.pick_option_set(extract_for, committed), calls

    def test_first_match_wins_and_stops(self):
        want = ref()
        got, calls = self.pick({("--uniform-values",): ref(trace="other")}, want)
        self.assertEqual(got, ["--uniform-values"])
        self.assertEqual(calls, [["--uniform-values"]])

    def test_later_set(self):
        want = ref()
        res = {("--uniform-values",): ref(words="zz"), (): ref(),
               ("--prim", "points"): ref()}
        got, calls = self.pick(res, want)
        self.assertEqual(got, [])
        self.assertEqual(len(calls), 2)

    def test_points_sets_tried_in_order(self):
        res = {("--prim", "points", "--uniform-values"): ref()}
        got, _ = self.pick(res, ref())
        self.assertEqual(got, ["--prim", "points", "--uniform-values"])

    def test_failed_draw_is_skipped_not_fatal(self):
        got, calls = self.pick({(): ref()}, ref())
        self.assertEqual(got, [])
        self.assertEqual(len(calls), 2)

    def test_none_matches(self):
        got, calls = self.pick({}, ref())
        self.assertIsNone(got)
        self.assertEqual(calls, g.OPTION_SETS)


class Diff(unittest.TestCase):
    def test_identical_ignores_trace(self):
        self.assertEqual(g.diff_fields(ref(trace="a"), ref(trace="b")), [])

    def test_fields_named(self):
        self.assertEqual(g.diff_fields(ref(), ref(words="cc", fau="dd")),
                         ["fs.fragment.words", "fs.fragment.fau"])

    def test_region_and_secondary(self):
        a, b = ref(), ref()
        b["regions"]["vs.position"] = a["regions"]["fs.fragment"]
        b["secondary_offset"] = 16
        self.assertEqual(g.diff_fields(a, b), ["vs.position(region)", "secondary_offset"])


class Parsing(unittest.TestCase):
    def test_diffref(self):
        text = "fs.fragment vs 0x1: byte-equal\npan_ref_diff: 2 region(s) compared, 1 differ or missing\n"
        self.assertEqual(g.parse_diffref(text), (2, 1))
        self.assertIsNone(g.parse_diffref("nothing"))

    def test_summary_round_trip(self):
        results = {"a": ("IDENTICAL", ""), "b": ("IDENTICAL", ""), "c": ("DIFFERS", "x"),
                   "d": ("NO-OPTION-SET", "no pair")}
        line = g.format_summary(results)
        self.assertEqual(line, "2 identical, 1 differ, 1 skipped")
        self.assertEqual(g.parse_summary("noise\n" + line + "\n"), (2, 1, 1))
        self.assertIsNone(g.parse_summary("2 identical, 1 differ"))

    def test_lines(self):
        self.assertEqual(g.format_line("n", "IDENTICAL", "--uniform-values"), "IDENTICAL n")
        self.assertEqual(g.format_line("n", "DIFFERS", "a b"), "DIFFERS n a b")
        self.assertEqual(g.format_line("n", "NO-OPTION-SET", ""), "NO-OPTION-SET n")


class Select(unittest.TestCase):
    def test_default_is_every_deqp_reference(self):
        refs = g.select([])
        self.assertTrue(refs)
        self.assertTrue(all(os.path.basename(r).startswith("deqp_") for r in refs))

    def test_name_and_glob(self):
        one = g.select(["deqp_arith_constexpr_ball_fs_highp"])
        self.assertEqual([os.path.basename(r) for r in one],
                         ["deqp_arith_constexpr_ball_fs_highp.expect.json"])
        self.assertGreater(len(g.select(["deqp_arith_constexpr_*"])), 1)

    def test_since_keeps_changed_only(self):
        changed = ["tools/mali/testdata/deqp_arith_constexpr_ball_fs_highp.expect.json"]
        one = g.select([], "REV", changed)
        self.assertEqual([os.path.basename(r) for r in one],
                         ["deqp_arith_constexpr_ball_fs_highp.expect.json"])
        self.assertEqual(g.select([], "REV", []), [])

    def test_args(self):
        self.assertEqual(g.parse_args(["--since", "r", "x"]), ("r", None, None, ["x"]))
        self.assertIsNone(g.parse_args(["--since"]))


if __name__ == "__main__":
    unittest.main()
