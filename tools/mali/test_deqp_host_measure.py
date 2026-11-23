# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Self-test of deqp-host-measure.sh's crash resume, without the CTS.

DEQP_RUNNER is a fake deqp-gles2 that writes a .qpa block per case of the
case list and dies (exit 139) inside the case named by FAKE_CRASH. The test
runs the script's `run` and `extract` stages over 7 cases of
tools/cts/gles2-mustpass.txt and checks that every case after the crashing
one is measured and the crashing one has its own 'crash' row. Also
programs.tsv and programs/ (corpus_diff.py's input, named by its hash) and
--cases (a case list in place of the slices').
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "deqp-host-measure.sh")
MUSTPASS = os.path.join(HERE, "..", "cts", "gles2-mustpass.txt")
TEXT = ("online shaderc: the code generator refused this fragment shader: "
        "Mali(Valhall: an integer or boolean value is not lowered yet)")

RUNNER = r'''#!/bin/bash
list=$1 log=$2
while read -r c; do
	[ -n "$c" ] || continue
	echo "#beginTestCaseResult $c" >> "$log"
	if [ "$c" = "$FAKE_CRASH" ]; then exit 139; fi
	echo '<TestCaseResult><Result StatusCode="Fail">x</Result>' >> "$log"
	echo "<ShaderProgram LinkStatus=\"false\"><InfoLog></InfoLog><VertexShader CompileStatus=\"true\"><ShaderSource>void main() { gl_Position = vec4(0.0); }</ShaderSource><InfoLog></InfoLog></VertexShader><FragmentShader CompileStatus=\"false\"><ShaderSource>void main() {}</ShaderSource><InfoLog>$FAKE_TEXT</InfoLog></FragmentShader></ShaderProgram>" >> "$log"
	echo '#endTestCaseResult' >> "$log"
done < "$list"
'''


class CrashResume(unittest.TestCase):
    def run_script(self, td, crash, chunk, caselist=None):
        with open(MUSTPASS) as f:
            cases = [l.strip() for l in f if l.strip()][:7]
        sl = os.path.join(td, "slice")
        os.mkdir(sl)
        with open(os.path.join(sl, "refusals.tsv"), "w") as f:
            f.write("case\ttext\n")
            f.writelines("%s\t%s\n" % (c, TEXT) for c in cases)
        with open(os.path.join(sl, "results.tsv"), "w") as f:
            f.write("case\tstatus\n")
            f.writelines("%s\tFail\n" % c for c in cases)
        runner = os.path.join(td, "runner.sh")
        with open(runner, "w") as f:
            f.write(RUNNER)
        os.chmod(runner, 0o755)
        out = os.path.join(td, "out")
        env = dict(os.environ, DEQP_RUNNER=runner, FAKE_TEXT=TEXT,
                   FAKE_CRASH=cases[crash] if crash is not None else "")
        extra = ["--cases", caselist] if caselist else []
        for stage in ("run", "extract"):
            r = subprocess.run(["bash", SCRIPT, "--out", out, "--slice", sl, "--jobs", "1",
                                "--chunk", str(chunk), "--stage", stage] + extra,
                               env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        rows = {}
        with open(os.path.join(out, "measure.tsv")) as f:
            next(f)
            for l in f:
                p = l.rstrip("\n").split("\t")
                rows[p[0]] = p
        with open(os.path.join(out, "summary.md")) as f:
            return cases, rows, f.read()

    def test_crash_mid_chunk(self):
        for chunk in (100, 3):
            with tempfile.TemporaryDirectory() as td:
                cases, rows, summary = self.run_script(td, 3, chunk)
                self.assertEqual(len(rows), 7)
                self.assertEqual(rows[cases[3]][3], "crash")
                self.assertEqual(rows[cases[3]][7], "exit=139")
                for i, c in enumerate(cases):
                    if i != 3:
                        self.assertEqual(rows[c][3], "Fail", (chunk, c))
                        self.assertEqual(rows[c][5], "intval", (chunk, c))
                self.assertIn("### Crash", summary)
                self.assertIn("`%s`" % cases[3], summary)

    def test_crash_first_and_last(self):
        for crash in (0, 6):
            with tempfile.TemporaryDirectory() as td:
                cases, rows, _ = self.run_script(td, crash, 100)
                self.assertEqual([c for c in cases if rows[c][3] == "crash"], [cases[crash]])
                self.assertEqual(sum(1 for c in cases if rows[c][5] == "intval"), 6)

    def test_no_crash(self):
        with tempfile.TemporaryDirectory() as td:
            cases, rows, summary = self.run_script(td, None, 100)
            self.assertTrue(all(rows[c][3] == "Fail" for c in cases))
            self.assertNotIn("### Crash", summary)


    def test_programs(self):
        """programs.tsv: one row per case with a program, one file pair per
        distinct text, named by the hash corpus_diff.py computes."""
        with tempfile.TemporaryDirectory() as td:
            cases, rows, _ = self.run_script(td, 3, 100)
            out = os.path.join(td, "out")
            with open(os.path.join(out, "programs.tsv")) as f:
                lines = f.read().splitlines()
            self.assertEqual(lines[0], "program\tcase")
            keys = {l.split("\t")[0] for l in lines[1:]}
            self.assertEqual([l.split("\t")[1] for l in lines[1:]], [c for i, c in enumerate(cases) if i != 3])
            self.assertEqual(len(keys), 1)
            key = keys.pop()
            with open(os.path.join(out, "programs", key + ".vert")) as f:
                vs = f.read()
            with open(os.path.join(out, "programs", key + ".frag")) as f:
                fs = f.read()
            self.assertEqual(fs, "void main() {}\n")
            sys.path.insert(0, HERE)
            import corpus_diff
            self.assertEqual(corpus_diff.program_key(vs, fs), key)

    def test_case_list(self):
        """--cases runs the list's cases, not the slices' refused ones."""
        with tempfile.TemporaryDirectory() as td:
            with open(MUSTPASS) as f:
                want = [l.strip() for l in f if l.strip()][2:5]
            lst = os.path.join(td, "list.txt")
            with open(lst, "w") as f:
                f.write("# a comment\n" + "\n".join(want) + "\n\n")
            _, rows, _ = self.run_script(td, None, 100, lst)
            self.assertEqual(sorted(rows), sorted(want))


if __name__ == "__main__":
    unittest.main()
