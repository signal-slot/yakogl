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


if __name__ == "__main__":
    unittest.main()
