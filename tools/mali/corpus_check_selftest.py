#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Self-test of corpus_check.py's pure parts: no build, no Mesa, no dEQP.

    python3 -I tools/mali/corpus_check_selftest.py

The .qpa reading, the refusal and difference classes, the stage rows of a
measure, the rows file, the summary and the diff (its exit code on a
regression) on synthetic input.
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import corpus_check as cc  # noqa: E402

TABLE = {
    "Valhall: an integer or boolean value is not lowered yet": "IntValue",
    "Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet": "ColourForm",
}
INT = "Valhall: an integer or boolean value is not lowered yet"

QPA = """#beginTestCaseResult dEQP-GLES2.functional.shaders.operator.a
<TestCaseResult>
<ShaderProgram LinkStatus="OK"><InfoLog></InfoLog>
<VertexShader CompileStatus="OK"><ShaderSource>void main() { gl_Position = vec4(0.0); }</ShaderSource><InfoLog></InfoLog></VertexShader>
<FragmentShader CompileStatus="OK"><ShaderSource>void main() { gl_FragColor = vec4(1.0 &lt; 2.0); }</ShaderSource><InfoLog></InfoLog></FragmentShader>
</ShaderProgram>
<Result StatusCode="Pass">x</Result>
</TestCaseResult>
#endTestCaseResult
#beginTestCaseResult dEQP-GLES2.functional.shader_api.b
<TestCaseResult>
<Section Name="Vertex Shader Info"><KernelSource>void main() { }</KernelSource><Section Name="Info Log"><Text></Text></Section></Section>
<Result StatusCode="Pass">x</Result>
</TestCaseResult>
#endTestCaseResult
#beginTestCaseResult dEQP-GLES2.functional.texture.c
<TestCaseResult><Result StatusCode="Pass">x</Result></TestCaseResult>
#endTestCaseResult
"""


def row(key, stage, verdict, group="shaders.operator", text="-", cls="-", program="p1", source=None):
    return dict(key=key, group=group, stage=stage, ours="refused" if verdict == "REFUSED" else "compiled",
                ours_text=text, mesa="ok", verdict=verdict, detail="-", program=program,
                source=source or program + stage, **{"class": cls})


class CorpusCheck(unittest.TestCase):
    def test_qpa_programs_and_lone_shaders(self):
        cases = cc.read_qpa(QPA)
        self.assertEqual(list(cases), ["dEQP-GLES2.functional.shaders.operator.a",
                                       "dEQP-GLES2.functional.shader_api.b", "dEQP-GLES2.functional.texture.c"])
        (vs, fs), = cases["dEQP-GLES2.functional.shaders.operator.a"]
        self.assertIn("gl_Position", vs)
        self.assertIn("1.0 < 2.0", fs)
        self.assertEqual(cases["dEQP-GLES2.functional.shader_api.b"], [("void main() { }", "")])
        self.assertEqual(cases["dEQP-GLES2.functional.texture.c"], [])
        self.assertEqual(cc.deqp_group("dEQP-GLES2.functional.shaders.operator.a"), "shaders.operator")

    def test_refusal_classes(self):
        log = "fs (-95): online shaderc: the code generator refused this fragment shader: Mali(%s)" % INT
        self.assertEqual(cc.short_text(log), INT)
        self.assertEqual(cc.class_of_refusal(INT, TABLE), "type domain")
        irindex = "Valhall back end defect: an invariant of the pipeline did not hold (IrIndex at x)"
        self.assertEqual(cc.class_of_refusal(irindex, TABLE), "type domain, through a defect")
        self.assertEqual(cc.class_of_refusal("link: varying v not written", TABLE), "not the code generator")
        self.assertEqual(cc.class_of_refusal("panic: x", TABLE), "defect")

    def test_message_table_reads_mod_rs(self):
        t = cc.message_table(os.path.join(cc.REPO, "src/shaderc/online/mali/mod.rs"))
        self.assertEqual(t.get(INT), "IntValue")
        self.assertGreater(len(t), 40)
        # a variant main adds before CLASSES names it is counted apart, not lost
        self.assertEqual(cc.class_of_refusal("Valhall: a new gap", {"Valhall: a new gap": "NewGap"}),
                         "unclassed:NewGap")

    def test_difference_classes(self):
        for d, c in (("vs.position: 5 words ours, 6 reference", "words"),
                     ("fs.fragment: FAU words ours [1], reference Some([2])", "fau"),
                     ("fs.fragment: work_reg_count ours 32, reference 16", "work_reg_count"),
                     ("vs.position: preload ours 0x1, reference 0x0", "preload"),
                     ("secondary_offset ours 128, reference Some(64)", "varyings"),
                     ("fs: 24 words with padding, expected 16", "padding")):
            self.assertEqual(cc.class_of_difference(d), c, d)

    def test_stage_rows(self):
        m = ["DIFFER", "abc", "fs: x", "EQUAL", "-", "DIFFER", "fs.fragment: FAU words ours [1], reference [2]"]
        rs = cc.stage_rows("k#0", "g", m, "ok", "p" * 64, TABLE)
        self.assertEqual([(r["stage"], r["verdict"], r["class"]) for r in rs],
                         [("vs", "EQUAL", "-"), ("fs", "DIFFER", "fau")])
        self.assertEqual((rs[0]["program"], rs[0]["source"]), ("p" * 16, "-"))
        rs = cc.stage_rows("k#0", "g", m, "ok", "p" * 64, TABLE, sources=("a" * 64, "b" * 64))
        self.assertEqual([r["source"] for r in rs], ["a" * 16, "b" * 16])
        log = "fs (-95): online shaderc: the code generator refused this fragment shader: Mali(%s)" % INT
        m = ["REFUSED", "-", log, "BLOCKED", "-", "REFUSED", log]
        rs = cc.stage_rows("k#0", "g", m, "ok", "q" * 64, TABLE)
        self.assertEqual([(r["verdict"], r["ours"], r["class"], r["detail"]) for r in rs],
                         [("BLOCKED", "compiled", "-", "-"), ("REFUSED", "refused", "type domain", "IntValue")])
        # the front end refuses and Mesa does too: a negative case, REJECTED
        m = ["REFUSED", "-", "fs (-1): 0:3: error: x", "BLOCKED", "-", "REFUSED", "fs (-1): 0:3: error: x"]
        rs = cc.stage_rows("k#0", "g", m, "fail", "q" * 64, TABLE)
        self.assertEqual([r["verdict"] for r in rs], ["BLOCKED", "REJECTED"])
        rs = cc.stage_rows("k#0", "g", m, "ok", "q" * 64, TABLE)
        self.assertEqual([(r["verdict"], r["class"]) for r in rs],
                         [("BLOCKED", "-"), ("REFUSED", "not the code generator")])
        # a lone vertex shader: one row; compiled, it is SINGLE
        m = ["EQUAL", "d", "-", "EQUAL", "-", "EQUAL", "-"]
        rs = cc.stage_rows("k#1", "g", m, "-", "r" * 64, TABLE, lone="vs")
        self.assertEqual([(r["stage"], r["verdict"]) for r in rs], [("vs", "SINGLE")])

    def test_struct_uniforms_are_seen(self):
        self.assertTrue(cc.has_struct_uniform("struct S { float a; }; uniform S s;"))
        self.assertTrue(cc.has_struct_uniform("uniform mediump Light l[2];"))
        self.assertFalse(cc.has_struct_uniform("uniform highp vec4 v; uniform sampler2D t; // uniform S s;"))

    def test_rows_file_and_summary(self):
        rows = [row("dEQP-GLES2.functional.shaders.operator.a#0", "vs", "EQUAL"),
                row("dEQP-GLES2.functional.shaders.operator.a#0", "fs", "DIFFER", cls="fau"),
                row("dEQP-GLES2.functional.shaders.operator.b#0", "vs", "REFUSED", text=INT, cls="type domain",
                    program="p2"),
                row("dEQP-GLES2.functional.shaders.operator.b#0", "fs", "BLOCKED", program="p2"),
                row("testdata:tri", "vs", "EQUAL", group="testdata"),
                row("testdata:tri", "fs", "EQUAL", group="testdata")]
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "rows.tsv")
            cc.write_rows(p, [cc.ROWS_HEADER + " tree=x", "# cols"], rows)
            head, back = cc.read_rows(p)
        self.assertEqual(back, rows)
        cases = cc.case_outcomes(back)
        self.assertEqual({c: o for c, (o, _) in cases.items()},
                         {"dEQP-GLES2.functional.shaders.operator.a": "DIFFER",
                          "dEQP-GLES2.functional.shaders.operator.b": "REFUSED"})
        s = cc.summary(back, board={"dEQP-GLES2.functional.shaders.operator.b"})
        self.assertIn("2 cases: REFUSED 1, DIFFER 1. Compiled (every program): 1", s)
        self.assertIn("| type domain | 1 |", s)
        self.assertIn("| shaders.operator | 2 | 0 | 1 | 1 | 1 | 0 |", s)
        self.assertIn("### the board-refused cases", s)
        self.assertIn("1 cases: REFUSED 1.", s)

    def test_malformed_rows_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "rows.tsv")
            with open(p, "w") as f:
                f.write(cc.ROWS_HEADER + "\nk\tg\tvs\tcompiled\t-\tok\tMAYBE\t-\t-\tp\n")
            with self.assertRaises(ValueError):
                cc.read_rows(p)

    def test_diff_names_regressions_and_exits_1(self):
        old = [row("a#0", "vs", "REFUSED"), row("b#0", "vs", "REFUSED"), row("c#0", "vs", "EQUAL"),
               row("d#0", "vs", "EQUAL")]
        new = [row("a#0", "vs", "EQUAL"), row("b#0", "vs", "DIFFER"), row("c#0", "vs", "DIFFER"),
               row("d#0", "vs", "EQUAL")]
        text, reg = cc.diff(old, new)
        self.assertIn("refused -> equal 1, refused -> differ 1, equal -> differ 1 (regression)", text)
        self.assertEqual(len(reg), 1)
        self.assertTrue(reg[0].startswith("c#0 vs: EQUAL -> DIFFER"), reg)
        with tempfile.TemporaryDirectory() as d:
            a, b = os.path.join(d, "a.tsv"), os.path.join(d, "b.tsv")
            cc.write_rows(a, [cc.ROWS_HEADER], old)
            cc.write_rows(b, [cc.ROWS_HEADER], new)
            tool = os.path.join(HERE, "corpus_check.py")
            r = subprocess.run([sys.executable, "-I", tool, "diff", a, b], capture_output=True, text=True)
            self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
            self.assertIn("regression\tc#0 vs: EQUAL -> DIFFER", r.stdout)
            r = subprocess.run([sys.executable, "-I", tool, "diff", a, a], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            with open(b, "w") as f:
                f.write("junk\n")
            r = subprocess.run([sys.executable, "-I", tool, "diff", a, b], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2, r.stdout + r.stderr)

    def test_glmark2_table_is_read(self):
        progs = cc.glmark2_programs(os.path.join(cc.REPO, "tests/mali_glmark2_corpus.rs"))
        self.assertEqual(len(progs), 33)
        self.assertEqual(progs[0][2], ["ace2+4748"])
        self.assertEqual(sum(len(p[2]) for p in progs), 55)


if __name__ == "__main__":
    unittest.main(verbosity=1)
