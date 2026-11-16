#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of corpus_forward.py on temporary git repositories.

    python3 -I tools/mali/test_corpus_forward.py

Each test runs the tool as a command, as the gate does.
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "corpus_forward.py")
CORPUS = "tools/mali/corpus-verdicts.txt"


class Repo(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        self.git("init", "-q")

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *a):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t",
                        "-c", "commit.gpgsign=false", *a], cwd=self.root, check=True,
                       capture_output=True)

    def commit(self, *lines):
        p = os.path.join(self.root, CORPUS)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write("".join(l + "\n" for l in lines))
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "c")

    def base(self, *lines):
        self.commit(*lines)
        self.git("tag", "base")

    def run_tool(self, *args):
        r = subprocess.run([sys.executable, "-I", TOOL, *args], capture_output=True,
                           text=True, cwd=self.root)
        return r.returncode, r.stdout + r.stderr


class Forward(Repo):
    def test_ok_lost(self):
        self.base("# c", "a.frag: ok", "b.frag: ok")
        self.commit("a.frag: ok")
        rc, out = self.run_tool("base")
        self.assertEqual(rc, 1, out)
        self.assertIn("b.frag: ok -> (removed)", out)
        self.assertIn("1 regressions", out)

    def test_ok_to_refusal(self):
        self.base("a.frag: ok")
        self.commit("a.frag: not lowered")
        rc, out = self.run_tool("base")
        self.assertEqual(rc, 1, out)
        self.assertIn("a.frag: ok -> not lowered", out)

    def test_refusal_to_ok_and_text_change(self):
        self.base("a.frag: x", "b.frag: y")
        self.commit("a.frag: ok", "b.frag: z")
        rc, out = self.run_tool("--verbose", "base")
        self.assertEqual(rc, 0, out)
        self.assertIn("info", out)
        self.assertIn("0 ok -> 1 ok, 0 regressions", out)

    def test_path_added(self):
        self.base("a.frag: ok")
        self.commit("a.frag: ok", "b.frag: nope")
        rc, out = self.run_tool("base")
        self.assertEqual(rc, 0, out)
        self.assertIn("1 commits, 1 ok -> 1 ok, 0 regressions", out)

    def test_intermediate_regression_reported(self):
        self.base("a.frag: ok")
        self.commit("a.frag: broken")
        self.commit("a.frag: ok")
        rc, out = self.run_tool("base")
        self.assertEqual(rc, 1, out)
        self.assertIn("a.frag: ok -> broken", out)
        self.assertIn("2 commits, 1 ok -> 1 ok, 1 regressions", out)

    def test_worktree_and_head_rev(self):
        self.base("a.frag: ok")
        with open(os.path.join(self.root, CORPUS), "w") as f:
            f.write("a.frag: bad\n")
        rc, out = self.run_tool("base")
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.run_tool("base", "HEAD")[0], 0)

    def test_malformed_line(self):
        self.base("a.frag: ok")
        self.commit("a.frag ok")
        rc, out = self.run_tool("base")
        self.assertEqual(rc, 2, out)
        self.assertIn("malformed", out)


class Compare(Repo):
    """--compare: the forward-only rule as one step read from files (the entry
    point `corpus-verdicts.sh --check` uses). The pinned file is the default
    tools/mali/corpus-verdicts.txt unless named; the candidate is a file or -."""

    def write(self, name, *lines):
        p = os.path.join(self.root, name)
        if os.path.dirname(p):
            os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write("".join(l + "\n" for l in lines))

    def run_stdin(self, *args):
        r = subprocess.run([sys.executable, "-I", TOOL, *args], input=self.cand,
                           capture_output=True, text=True, cwd=self.root)
        return r.returncode, r.stdout + r.stderr

    def test_forward_ok_new_path_text_change(self):
        self.write(CORPUS, "# c", "a.frag: x", "c.frag: old text")
        self.write("cand.txt", "a.frag: ok", "b.frag: ok", "c.frag: new text")
        rc, out = self.run_tool("--compare", "cand.txt", "--verbose")
        self.assertEqual(rc, 0, out)
        self.assertIn("info check a.frag: x -> ok", out)
        self.assertIn("info check b.frag: (new) -> ok", out)
        self.assertIn("info check c.frag: old text -> new text", out)
        self.assertIn("compare, 0 ok -> 2 ok, 0 regressions", out)

    def test_ok_to_refusal_and_removed(self):
        self.write(CORPUS, "a.frag: ok", "b.frag: ok")
        self.write("cand.txt", "a.frag: not lowered")
        rc, out = self.run_tool("--compare", "cand.txt")
        self.assertEqual(rc, 1, out)
        self.assertIn("check a.frag: ok -> not lowered", out)
        self.assertIn("check b.frag: ok -> (removed)", out)
        self.assertIn("compare, 2 ok -> 0 ok, 2 regressions", out)

    def test_pinned_refusal_may_disappear(self):
        # The rule of the commit walk, unchanged: only an ok path may not
        # regress; a pinned refusal missing from the candidate is allowed.
        self.write(CORPUS, "a.frag: x")
        self.write("cand.txt")
        rc, out = self.run_tool("--compare", "cand.txt")
        self.assertEqual(rc, 0, out)

    def test_stdin_candidate_and_named_pinned(self):
        self.write("pinned.txt", "a.frag: ok")
        self.cand = "a.frag: nope\n"
        rc, out = self.run_stdin("--compare", "-", "pinned.txt")
        self.assertEqual(rc, 1, out)
        self.assertIn("check a.frag: ok -> nope", out)

    def test_malformed_candidate(self):
        self.write(CORPUS, "a.frag: ok")
        self.write("cand.txt", "a.frag ok")
        rc, out = self.run_tool("--compare", "cand.txt")
        self.assertEqual(rc, 2, out)
        self.assertIn("malformed", out)


if __name__ == "__main__":
    unittest.main()
