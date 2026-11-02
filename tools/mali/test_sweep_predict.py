#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of sweep_predict.py on small synthetic records in temporary
directories:  python3 -I tools/mali/test_sweep_predict.py"""
import contextlib
import io
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import sweep_predict as sp  # noqa: E402

P = "panthor-gltest: verdict case "
REF = "REFUSED compile: Mali(Valhall: a second texture look-up is not lowered yet)"
SWEEP = [
    "# header",
    P + "0 triangle: PASS",
    P + "1 uniform: PASS ",
    P + "2 texture: " + REF,
    P + "3 depth: " + REF,
    P + "4 multi: " + REF,
    P + "5 gen: " + REF,
    P + "6 bad: FAIL rc -1, first bad check 5",
    P + "7 ?: NO-CASE ",
    "panthor-gltest: 8 case(s) run: PASS 2",
]
GL_TEST = """\
static int case_tex(void)
{
	return link_program(gl_test_uv_vs_src, gl_test_texture2_fs_src);
}
static int case_two(int which)
{
	return link_program(gl_test_a_vs_src, gl_test_b_fs_src);
}
static int case_gen(void)
{
	snprintf(buf, sizeof buf, "void main() {}");
	return 0;
}
static int run_case(int c)
{
	switch (c) {
	case 2: rc = case_tex(); break;
	case 3: rc = case_two(1); break;
	case 4: rc = case_two(2); break;
	case 5: rc = case_gen(); break;
	}
}
"""
GEN = """\
ENTRIES = [('gl_test_uv_vs_src', 'uv.vert'), ('gl_test_texture2_fs_src', 'texture2.frag'),
           ('gl_test_a_vs_src', 'es100/a.vert'),
           ('gl_test_b_fs_src', 'es100/b.frag', 'a note')]
ES100_TWINS = {
    'uv.vert': 'gl_test_es100/uv.vert',
    'texture2.frag': 'gl_test_es100/texture2.frag',
}
"""


def corpus(*lines):
    return "# generated\n" + "".join(l + "\n" for l in lines)


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.rec = os.path.join(self.tmp.name, "rec")
        os.mkdir(self.rec)
        self.write("rec/sweep-1.verdicts.txt", "\n".join(SWEEP) + "\n")
        self.write("gl_test.c", GL_TEST)
        self.write("gen.py", GEN)

    def write(self, name, text):
        with open(os.path.join(self.tmp.name, name), "w") as f:
            f.write(text)

    def run_tool(self, corpus_text, *extra):
        self.write("corpus.txt", corpus_text)
        argv = [self.rec, "--corpus", os.path.join(self.tmp.name, "corpus.txt"),
                "--gl-test", os.path.join(self.tmp.name, "gl_test.c"),
                "--gen", os.path.join(self.tmp.name, "gen.py"), *extra]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(sp.main(argv), 0)
        return out.getvalue().splitlines()


class Predict(Fixture):
    ALL_OK = corpus(
        "tools/shaders/gl_test_es100/uv.vert: ok",
        "tools/shaders/gl_test_es100/texture2.frag: ok",
        "tools/shaders/es100/a.vert: ok",
        "tools/shaders/es100/b.frag: ok",
    )

    def test_pass_and_other_verdicts_are_kept_byte_for_byte(self):
        out = self.run_tool(self.ALL_OK)
        self.assertEqual(out[0], P + "0 triangle: PASS")
        self.assertEqual(out[1], P + "1 uniform: PASS ")
        self.assertEqual(out[6], P + "6 bad: FAIL rc -1, first bad check 5")
        self.assertEqual(out[7], P + "7 ?: NO-CASE ")

    def test_refused_with_every_shader_ok_is_a_candidate(self):
        out = self.run_tool(self.ALL_OK)
        self.assertEqual(out[2], P + "2 texture: PASS-CANDIDATE")
        # one function serves two cases: both are judged
        self.assertEqual(out[3], P + "3 depth: PASS-CANDIDATE")
        self.assertEqual(out[4], P + "4 multi: PASS-CANDIDATE")

    def test_a_refused_shader_keeps_the_case_refused_with_its_text(self):
        out = self.run_tool(corpus(
            "tools/shaders/gl_test_es100/uv.vert: ok",
            "tools/shaders/gl_test_es100/texture2.frag: Valhall: a built-in input is not lowered yet",
            "tools/shaders/es100/a.vert: ok",
            "tools/shaders/es100/b.frag: ok"))
        self.assertEqual(out[2], P + "2 texture: " + REF.split(" ", 1)[0] + REF[len("REFUSED"):])
        self.assertIn("# case 2 stays REFUSED: tools/shaders/gl_test_es100/texture2.frag: "
                      "Valhall: a built-in input is not lowered yet", out)
        self.assertEqual(out[3], P + "3 depth: PASS-CANDIDATE")

    def test_desktop_source_is_looked_up_by_its_es100_twin(self):
        out = self.run_tool(corpus(
            "tools/shaders/uv.vert: front end: Rejected",
            "tools/shaders/texture2.frag: front end: Rejected",
            "tools/shaders/gl_test_es100/uv.vert: ok",
            "tools/shaders/gl_test_es100/texture2.frag: ok",
            "tools/shaders/es100/a.vert: ok",
            "tools/shaders/es100/b.frag: ok"))
        self.assertEqual(out[2], P + "2 texture: PASS-CANDIDATE")

    def test_source_missing_from_the_corpus_or_generated_stays_refused(self):
        out = self.run_tool(corpus("tools/shaders/gl_test_es100/uv.vert: ok"))
        self.assertTrue(out[2].endswith(REF))
        self.assertTrue(out[5].endswith(REF))
        self.assertTrue(any(l.startswith("# case 2 stays REFUSED: tools/shaders/gl_test_es100/texture2.frag is not in the corpus")
                            for l in out))
        self.assertTrue(any(l.startswith("# case 5 stays REFUSED: no shader source symbol") for l in out))

    def test_summary_counts(self):
        out = self.run_tool(self.ALL_OK)
        self.assertIn("# 8 case(s) predicted from sweep-1.verdicts.txt: "
                      "FAIL 1, NO-CASE 1, PASS 2, PASS-CANDIDATE 3, REFUSED 1", out)

    def test_cases_option_limits_the_output(self):
        out = self.run_tool(self.ALL_OK, "--cases", "2,6")
        self.assertEqual([l for l in out if not l.startswith("#")],
                         [P + "2 texture: PASS-CANDIDATE", P + "6 bad: FAIL rc -1, first bad check 5"])


class Sweeps(Fixture):
    def test_the_highest_numbered_sweep_is_the_last_and_disagreement_is_listed(self):
        self.write("rec/sweep-2.verdicts.txt", "\n".join(SWEEP).replace(
            P + "1 uniform: PASS ", P + "1 uniform: FAIL rc -1") + "\n")
        out = self.run_tool(corpus())
        self.assertIn(P + "1 uniform: FAIL rc -1", out)
        self.assertIn("# case 1 differs between sweeps: PASS then FAIL", out)
        self.assertTrue(any("from sweep-2.verdicts.txt" in l for l in out))

    def test_sweep_10_sorts_after_sweep_2(self):
        self.write("rec/sweep-2.verdicts.txt", "\n".join(SWEEP) + "\n")
        self.write("rec/sweep-10.verdicts.txt", P + "0 triangle: FAIL x\n")
        out = self.run_tool(corpus())
        self.assertEqual(out[0], P + "0 triangle: FAIL x")

    def test_a_record_without_sweeps_is_an_error(self):
        os.remove(os.path.join(self.rec, "sweep-1.verdicts.txt"))
        with self.assertRaises(SystemExit):
            sp.read_sweeps(self.rec)

    def test_bad_cases_option_is_an_error(self):
        with self.assertRaises(SystemExit):
            self.run_tool(corpus(), "--cases", "x")


class Generated(unittest.TestCase):
    """Cases 149 and 166 build their GLSL with snprintf in gl_test.c."""
    FILES = {"gl_test_vsunits_fs_src": "gltest/vsunits_fs.frag", "gl_test_fshp_vs_src": "gltest/fshp_vs.vert"}
    CORP = {"tools/shaders/gltest/vsunits_fs.frag": "ok", "tools/shaders/gltest/fshp_vs.vert": "ok"}
    OK = {n: "ok" for texts, _ in sp.GENERATED.values() for n in texts}

    def judge(self, num, gen, symbols=None):
        return sp.judge(num, symbols or {}, self.FILES, self.CORP, gen)

    def test_checked_in_sources_are_what_the_generators_make(self):
        for texts, _ in sp.GENERATED.values():
            for name, text in texts.items():
                with open(os.path.join(sp.TESTDATA, name)) as f:
                    self.assertEqual(f.read(), text, name)

    def test_generators_follow_gl_test_c(self):
        with open(os.path.join(sp.ROOT, "driver/tools/pvrlab/gl_test.c")) as f:
            c = f.read()
        for needle in (
                '"uniform sampler2D u_s%d;\\n"', '"    r += u_w[%d].%c * texture2DLod(u_s%d, c, 0.0);\\n"',
                '"    r += u_w[%d].%c * texture2D(u_s%d, c);\\n"', '"    r += u_p[%d].%c * u_p[%d];\\n"',
                '"    vec4 r = u_p[%d].%c * vec4(1.0, 0.0, 1.0, 1.0);\\n"',
                "#define SHP_SEL0 56", "#define SHP_MAX_VEC4 70", "#define SHP_SEL_CONST 9",
                "shp_idx[] = { 0, 32, 60, 63, 64, 65, 66, 67, 69 }", "#define FSHP_UNITS 16",
                "#define VSU_UNITS 16"):
            self.assertIn(needle, c)

    def test_every_generated_source_has_a_verdict(self):
        self.assertEqual(set(sp.gen_verdicts()), set(self.OK))

    def test_generated_text_shapes(self):
        self.assertEqual(sp.vsunits_vs(1).count("texture2DLod"), 1)
        self.assertIn("r += u_w[3].w * texture2DLod(u_s15, c, 0.0);\n", sp.vsunits_vs(16))
        self.assertIn("uniform vec4 u_p[70];\n", sp.shp_frag(70))
        self.assertNotIn(" * u_p[64];", sp.shp_frag(64))
        self.assertIn(" * u_p[69];", sp.shp_frag(70))

    def test_case_with_no_symbol_but_generated_sources_is_judged(self):
        self.assertEqual(self.judge(166, self.OK), (True, ""))

    def test_a_refused_generated_source_keeps_the_case_refused(self):
        gen = dict(self.OK, **{"sweepgen_149_vs.vert": "Valhall: x is not lowered yet"})
        ok, why = self.judge(149, gen)
        self.assertFalse(ok)
        self.assertEqual(why, "tools/mali/testdata/sweepgen_149_vs.vert: Valhall: x is not lowered yet")

    def test_a_generated_source_without_a_verdict_keeps_the_case_refused(self):
        ok, why = self.judge(166, {})
        self.assertFalse(ok)
        self.assertIn("sweepgen_166_a.frag is not in sweepgen-verdicts.txt", why)

    def test_the_static_shader_of_the_pair_is_still_judged(self):
        sp_corp = dict(self.CORP, **{"tools/shaders/gltest/fshp_vs.vert": "Valhall: y"})
        ok, why = sp.judge(166, {}, self.FILES, sp_corp, self.OK)
        self.assertFalse(ok)
        self.assertEqual(why, "tools/shaders/gltest/fshp_vs.vert: Valhall: y")

    def test_the_tip_record_no_longer_says_built_at_run_time(self):
        rec = os.path.join(sp.ROOT, "tools/cts/log/2026-10-28-final")
        if not os.path.isdir(rec):
            self.skipTest("no tools/cts/log/2026-10-28-final")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(sp.main([rec]), 0)
        self.assertNotIn("built at run time", out.getvalue())


class Parsers(unittest.TestCase):
    def test_line_format_keeps_head_and_spacing(self):
        m = sp.LINE.match(P + "12 name: REFUSED compile: x")
        self.assertEqual((m.group("num"), m.group("verdict"), m.group("rest")), ("12", "REFUSED", " compile: x"))
        self.assertIsNone(sp.LINE.match("panthor-gltest: 8 case(s) run: PASS 2"))


if __name__ == "__main__":
    unittest.main()
