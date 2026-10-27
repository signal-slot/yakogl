#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of sweep_passsets.py on synthetic records and git repositories.

    python3 -I tools/mali/test_sweep_passsets.py

Each test runs the tool as a command, as the gate does.
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "sweep_passsets.py")


def tool(*args, cwd=None):
    r = subprocess.run([sys.executable, "-I", TOOL, *args], capture_output=True, text=True,
                       cwd=cwd)
    return r.returncode, r.stdout + r.stderr


def sweep_log(verdicts, summary=True, run=256):
    """verdicts: {case: 'PASS'|'REFUSED'|...}; run: the cases the run covered."""
    lines = ["run-local.sh: start x", "panthor-gltest: %d case(s), one process each" % run]
    for n, v in sorted(verdicts.items()):
        lines.append("panthor-gltest: verdict case %d name%d: %s rc 0" % (n, n, v))
    if summary:
        counts = {}
        for v in verdicts.values():
            counts[v] = counts.get(v, 0) + 1
        lines.append("panthor-gltest: %d case(s) run: " % run +
                     ", ".join("%s %d" % (k, counts[k]) for k in sorted(counts)))
    return "\n".join(lines) + "\n"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def verdicts(passing, total=6):
    return {n: ("PASS" if n in passing else "REFUSED") for n in range(total)}


class SetsMode(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def record(self, name, runs, sub="ours"):
        for i, passing in enumerate(runs):
            write(os.path.join(self.root, name, sub, "0%d-panthor-gltest--sweep-%d.log" % (i, i)),
                  sweep_log(verdicts(passing)))

    def test_growing_sets_pass(self):
        self.record("2026-10-19-a", [{1}, {1}])
        self.record("2026-10-20-b", [{1, 2}, {1, 2}])
        rc, out = tool(self.root)
        self.assertEqual(rc, 0, out)
        self.assertIn("2026-10-19-a", out)
        self.assertIn("1 PASS", out)
        self.assertIn("2 PASS", out)

    def test_shrink_names_the_cases(self):
        self.record("2026-10-19-a", [{1, 3, 4}, {1, 3, 4}])
        self.record("2026-10-20-b", [{1, 2}, {1, 2}])
        rc, out = tool(self.root)
        self.assertEqual(rc, 1, out)
        self.assertIn("SHRINK 2026-10-19-a", out)
        self.assertIn("lacks: 3 4", out)

    def test_record_set_is_both_runs(self):
        # case 3 passes in one run only: not in the record's set, so no shrink.
        self.record("2026-10-19-a", [{1, 3}, {1}])
        self.record("2026-10-20-b", [{1}, {1}])
        rc, out = tool(self.root)
        self.assertEqual(rc, 0, out)

    def test_baselines_partial_and_single_case_logs_are_skipped(self):
        self.record("2026-10-19-a", [{1}, {1}])
        self.record("2026-10-20-b", [{1, 2}, {1, 2}])
        # the tree before the change: has case 5, must not count
        self.record("2026-10-19-a", [{1, 5}], sub="base")
        write(os.path.join(self.root, "2026-10-19-a", "ours", "09-killed-sweep.log"),
              sweep_log(verdicts({1, 5}), summary=False))
        write(os.path.join(self.root, "2026-10-19-a", "ours", "10-one.log"),
              "panthor-gltest: verdict case 5 x: PASS \npanthor-gltest: 1 case(s) run: PASS 1\n")
        rc, out = tool(self.root)
        self.assertEqual(rc, 0, out)
        self.assertIn("(2 sweep log(s))", out)

    def test_multi_case_set_runs_are_not_sweeps(self):
        # a 4-case --set run without the record's PASS case 1 must not empty its set
        self.record("2026-10-19-a", [{1, 2}, {1, 2}])
        self.record("2026-10-20-b", [{1, 2, 3}, {1, 2, 3}])
        part = {n: "REFUSED" for n in range(4)}
        write(os.path.join(self.root, "2026-10-19-a", "03-set.log"), sweep_log(part, run=4))
        rc, out = tool(self.root)
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"2026-10-19-a\s+2 PASS \(2 sweep log\(s\)\)")

    def test_summary_mismatch_is_an_error(self):
        self.record("2026-10-19-a", [{1}, {1}])
        p = os.path.join(self.root, "2026-10-19-a", "ours", "00-panthor-gltest--sweep-0.log")
        with open(p) as f:
            text = f.read().replace("PASS 1", "PASS 2")
        write(p, text)
        rc, out = tool(self.root)
        self.assertEqual(rc, 2, out)
        self.assertIn("ERROR", out)

    def test_directory_without_sweep_is_ignored(self):
        self.record("2026-10-19-a", [{1}, {1}])
        write(os.path.join(self.root, "2026-10-18-notes", "README.md"), "nothing\n")
        rc, out = tool(self.root)
        self.assertEqual(rc, 0, out)
        self.assertNotIn("notes", out)


class LogLayout(unittest.TestCase):
    """Records laid out like tools/cts/log/<date>-<item>/."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def record(self, name, runs, total=6):
        for i, v in enumerate(runs):
            write(os.path.join(self.root, name, "sweep-%d.verdicts.txt" % (i + 1)),
                  sweep_log(v if isinstance(v, dict) else verdicts(v, total)))
        write(os.path.join(self.root, name, "README.md"), "notes\n")
        write(os.path.join(self.root, name, "predictions.txt"), "case 1: PASS\n")
        # a case run that is not a sweep
        write(os.path.join(self.root, name, "case-5.log"),
              "panthor-gltest: verdict case 5 x: FAIL rc -1\npanthor-gltest: 1 case(s) run: FAIL 1\n")

    def test_verdicts_txt_records(self):
        self.record("2026-10-27-a", [{1}, {1}])
        self.record("2026-10-28-b", [{1, 2}, {1, 2}])
        rc, out = tool(self.root)
        self.assertEqual(rc, 0, out)
        self.assertIn("(2 sweep log(s))", out)

    def test_require_pass(self):
        self.record("2026-10-27-a", [{1}, {1}])
        self.record("2026-10-28-b", [{1, 2, 3}, {1, 2, 3}])
        rc, out = tool("--require", "1,2,3", self.root)
        self.assertEqual(rc, 0, out)

    def test_require_missing_case_names_log_and_line(self):
        self.record("2026-10-27-a", [{1}, {1}])
        self.record("2026-10-28-b", [{1, 2, 3}, {1, 2}])
        rc, out = tool("--require", "2,3", self.root)
        self.assertEqual(rc, 1, out)
        self.assertIn("REQUIRED case 3: not PASS in", out)
        self.assertIn("sweep-2.verdicts.txt: panthor-gltest: verdict case 3 name3: REFUSED rc 0", out)
        self.assertNotIn("REQUIRED case 2", out)

    def test_require_absent_case(self):
        self.record("2026-10-28-b", [{1}, {1}])
        rc, out = tool("--require", "99", self.root)
        self.assertEqual(rc, 1, out)
        self.assertIn("REQUIRED case 99: no verdict line in", out)

    def test_delta_exact_strings(self):
        self.record("2026-10-27-a", [{1}, {1}])
        self.record("2026-10-28-b", [{1, 2}, {1, 2}])
        rc, out = tool("--delta", self.root)
        self.assertEqual(rc, 0, out)
        self.assertIn("case 2: panthor-gltest: verdict case 2 name2: REFUSED rc 0 -> "
                      "panthor-gltest: verdict case 2 name2: PASS rc 0", out)
        self.assertNotIn("case 1:", out)
        self.assertIn("1 case(s) differ", out)

    def test_delta_with_one_record(self):
        self.record("2026-10-27-a", [{1}, {1}])
        rc, out = tool("--delta", self.root)
        self.assertEqual(rc, 0, out)
        self.assertIn("--delta needs two records", out)


class CorpusMode(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = self.tmp.name
        self.git("init", "-q")
        self.git("config", "user.email", "t@example.org")
        self.git("config", "user.name", "t")
        self.rev = {}

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True)

    def commit(self, tag, lines):
        write(os.path.join(self.repo, "tools/mali/corpus-verdicts.txt"),
              "# header\n" + "".join(l + "\n" for l in lines))
        self.git("add", "-A")
        self.git("commit", "-q", "-m", tag)
        self.git("tag", tag)

    def run_corpus(self, old, new):
        # the tool is copied next to the repository's tools/mali so git sees the repo
        dst = os.path.join(self.repo, "tools/mali/sweep_passsets.py")
        with open(TOOL) as f:
            write(dst, f.read())
        r = subprocess.run([sys.executable, "-I", dst, "corpus", old, new],
                           capture_output=True, text=True, cwd=self.repo)
        return r.returncode, r.stdout + r.stderr

    def test_forward_only(self):
        self.commit("a", ["x.frag: ok", "y.frag: Valhall: refused"])
        self.commit("b", ["x.frag: ok", "y.frag: ok", "z.frag: ok"])
        rc, out = self.run_corpus("a", "b")
        self.assertEqual(rc, 0, out)
        self.assertIn("all 1 ok line(s)", out)

    def test_ok_became_refusal(self):
        self.commit("a", ["x.frag: ok", "y.frag: ok"])
        self.commit("b", ["x.frag: Valhall: refused", "y.frag: ok"])
        rc, out = self.run_corpus("a", "b")
        self.assertEqual(rc, 1, out)
        self.assertIn("LOST x.frag: ok -> Valhall: refused", out)

    def test_ok_disappeared(self):
        self.commit("a", ["x.frag: ok", "y.frag: ok"])
        self.commit("b", ["y.frag: ok"])
        rc, out = self.run_corpus("a", "b")
        self.assertEqual(rc, 1, out)
        self.assertIn("LOST x.frag: ok -> (gone)", out)

    def test_unknown_revision(self):
        self.commit("a", ["x.frag: ok"])
        rc, out = self.run_corpus("a", "nope")
        self.assertNotEqual(rc, 0, out)


if __name__ == "__main__":
    unittest.main()
