#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of nir_refs.py that need no Mesa build.

    python3 tools/mali/test_nir_refs.py

The cut and the normaliser on fbload_fbload's dumps as Mesa printed them
(testdata/nir_refs/fbload_fbload.vs.stdout and .fs.stdout, the print_vs and
the print_fs stream up to just after the first nir_opt_find_array_copies;
testdata/fbload_fbload.stderr, the prints at selection) against copies
normalised by hand (testdata/nir_refs/fbload_fbload.*.nir), and the
committed fbload_fbload.nir.sha against their SHA-256. Then the rules one by
one on small dumps and streams written here; no shim is drawn.
"""
import hashlib
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "testdata")
sys.path.insert(0, HERE)
import nir_refs as n  # noqa: E402


def read(*path):
    with open(os.path.join(DATA, *path)) as f:
        return f.read()


class Fbload(unittest.TestCase):
    def points(self):
        return n.points_of(read("nir_refs", "fbload_fbload.vs.stdout"),
                           read("nir_refs", "fbload_fbload.fs.stdout"), read("fbload_fbload.stderr"))

    def test_both_points_equal_the_hand_copies(self):
        got = self.points()
        self.assertEqual([(v, p) for v, p, _, _, _ in got], n.ORDER)
        for v, p, _, text, _ in got:
            with self.subTest(variant=v, point=p):
                self.assertEqual(text, read("nir_refs", "fbload_fbload.%s.%s.nir" % (v, p)))

    def test_passes_before_the_ssa_point(self):
        got = {(v, p): passes for v, p, _, _, passes in self.points()}
        # the vertex shader has no local: vars_to_ssa and the removal of
        # dead variables print nothing
        self.assertEqual(got[("vs", "ssa")],
                         ["nir_inline_functions", "nir_split_var_copies", "nir_lower_var_copies"])
        self.assertEqual(got[("fs", "ssa")],
                         ["nir_lower_variable_initializers", "nir_inline_functions",
                          "nir_lower_global_vars_to_local", "nir_lower_vars_to_ssa",
                          "nir_remove_dead_variables"])
        self.assertIsNone(got[("fs", "select")])

    def test_committed_sha_file(self):
        self.assertEqual(n.sha_text(self.points()), read("fbload_fbload.nir.sha"))
        line = "vs ssa : %s : nir_inline_functions,nir_split_var_copies,nir_lower_var_copies\n" % \
            hashlib.sha256(read("nir_refs", "fbload_fbload.vs.ssa.nir").encode()).hexdigest()
        self.assertEqual(read("fbload_fbload.nir.sha").split("\n")[0] + "\n", line)


LOOP = """\
shader: MESA_SHADER_FRAGMENT
name: GLSL1
inputs: 1
decl_var shader_out INTERP_MODE_NONE none highp vec4 o (FRAG_RESULT_DATA0.xyzw, 0, 0)
decl_function main () (entrypoint)

impl main {
    decl_var  INTERP_MODE_NONE none highp float t
    block b0:   // preds:
    32     %3 = load_const (0x00000000)
    32     %7 = deref_var &a (function_temp float[2])
    32     %8 = deref_array &(*%7)[%3] (function_temp float)  // &a[%3]
    32x4   %5 = @load_input (%3 (0x0)) (base=0, range=1, component=0, dest_type=float32)  // v_in
                // succs: b1
    loop {
        block b1:   // preds: b0 b4
        32     %9 = phi b0: %3 (0x0), b4: %12
        32    %10 = ige %9, %3 (0x0)
                    // succs: b2 b3
        if %10 {
            block b2:  // preds: b1, succs: b5
            break
        } else {
            block b3:  // preds: b1, succs: b4
        }
        block b4:   // preds: b3
        32    %12 = fadd %9, %5.x // exact
                    // succs: b1
    }
    block b5:  // preds: b2, succs: b6
    block b6:
}

nir_opt_dce
"""

LOOP_NORMALISED = """\
decl_function main () (entrypoint)
impl main {
block b0: // preds:
32 %0 = load_const (0x00000000)
32 %1 = deref_var &t0 (function_temp float[2])
32 %2 = deref_array &(*%1)[%0] (function_temp float)
32x4 %3 = @load_input (%0 (0x0)) (base=0, range=1, component=0, dest_type=float32)
// succs: b1
loop {
block b1: // preds: b0 b4
32 %4 = phi b0: %0 (0x0), b4: %6
32 %5 = ige %4, %0 (0x0)
// succs: b2 b3
if %5 {
block b2: // preds: b1, succs: b5
break
} else {
block b3: // preds: b1, succs: b4
}
block b4: // preds: b3
32 %6 = fadd %4, %3.x // exact
// succs: b1
}
block b5: // preds: b2, succs: b6
block b6:
}
"""


def dump(text):
    """The one dump in a text."""
    dumps = n.cut_dumps(text.split("\n"))
    assert len(dumps) == 1, dumps
    return dumps[0][1]


class Normalise(unittest.TestCase):
    def test_loop_phi_comments_and_nesting(self):
        # a phi uses %12 before its definition; the `}` of the if and of the
        # loop do not end the function; fp_math_ctrl's `// exact` is kept
        self.assertEqual(n.normalise(dump(LOOP)), LOOP_NORMALISED)

    def test_names_per_function_and_divergence(self):
        text = ("shader: MESA_SHADER_VERTEX\ndecl_function f ()\n\nimpl f {\n"
                "    block b0:  // preds:\n    div 32  %4 = @load_vertex_id\n    block b1:\n}\n\n"
                "decl_function main () (entrypoint)\n\nimpl main {\n"
                "    block b0:  // preds:\n    con 32x2  %9 = load_const (0x0, 0x1)\n    block b1:\n}\n")
        self.assertEqual(n.normalise(dump(text)),
                         "decl_function f ()\nimpl f {\nblock b0: // preds:\ndiv 32 %0 = @load_vertex_id\n"
                         "block b1:\n}\ndecl_function main () (entrypoint)\nimpl main {\nblock b0: // preds:\n"
                         "con 32x2 %0 = load_const (0x0, 0x1)\nblock b1:\n}\n")

    def test_temporaries_renamed_in_order_of_first_use(self):
        text = ("shader: MESA_SHADER_FRAGMENT\ndecl_function main () (entrypoint)\n\nimpl main {\n"
                "    decl_var  INTERP_MODE_NONE none highp vec4 vec_ctor\n"
                "    block b0:  // preds:\n"
                "    32     %4 = deref_var &vec_ctor (function_temp highp vec4)\n"
                "    32     %5 = deref_var &const_temp#7 (function_temp highp float[2])\n"
                "    32     %6 = deref_var &vec_ctor (function_temp highp vec4)\n"
                "    32     %7 = deref_var &#3 (shader_temp highp float)\n"
                "    32     %8 = deref_var &u_v (uniform highp vec4)\n"
                "    32     %9 = deref_var &o (shader_out highp vec4)\n"
                "    32    %10 = deref_array &(*%5)[0] (function_temp highp float)  // &const_temp#7[0]\n"
                "    block b1:\n}\n")
        self.assertEqual(n.normalise(dump(text)),
                         "decl_function main () (entrypoint)\nimpl main {\nblock b0: // preds:\n"
                         "32 %0 = deref_var &t0 (function_temp highp vec4)\n"
                         "32 %1 = deref_var &t1 (function_temp highp float[2])\n"
                         "32 %2 = deref_var &t0 (function_temp highp vec4)\n"
                         "32 %3 = deref_var &t2 (shader_temp highp float)\n"
                         "32 %4 = deref_var &u_v (uniform highp vec4)\n"
                         "32 %5 = deref_var &o (shader_out highp vec4)\n"
                         "32 %6 = deref_array &(*%1)[0] (function_temp highp float)\n"
                         "block b1:\n}\n")

    def test_undefined_name_is_an_error(self):
        text = LOOP.replace("b4: %12", "b4: %13")
        with self.assertRaisesRegex(n.DumpError, "%13 used but not defined"):
            n.normalise(dump(text))

    def test_name_defined_twice_is_an_error(self):
        text = LOOP.replace("%10 = ige", "%9 = ige")
        with self.assertRaisesRegex(n.DumpError, "%9 defined twice"):
            n.normalise(dump(text))


class Cut(unittest.TestCase):
    def test_dumps_and_pass_names(self):
        text = ("gl_ref: renderer x\nglsl_to_nir\n" + LOOP + "nir_lower_io\nnir_lower_io (finished)\n" +
                LOOP.replace("FRAGMENT", "VERTEX").replace("nir_opt_dce\n", "") +
                "decl_function helper ()\n\nblock0 {\n}\n")
        got = n.events(text.split("\n"))
        self.assertEqual([v if k == "pass" else v[0] for k, v in got],
                         ["glsl_to_nir", "MESA_SHADER_FRAGMENT", "nir_opt_dce", "nir_lower_io", "nir_lower_io",
                          "MESA_SHADER_VERTEX"])
        dumps = n.cut_dumps(text.split("\n"))
        # a function without a body ends at its decl_function line; the BIR after it is not NIR
        self.assertEqual(dumps[1][1][-1], "decl_function helper ()")
        self.assertEqual(dumps[0][1][-1], "")

    def test_function_with_no_end(self):
        text = "\n".join(LOOP.split("\n")[:12]) + "\nshader: MESA_SHADER_VERTEX\n"
        with self.assertRaisesRegex(n.DumpError, "a function with no end"):
            n.cut_dumps(text.split("\n"))

    def test_ssa_point(self):
        fs = LOOP.replace("nir_opt_dce\n", "")
        stream = ("glsl_to_nir\n" + fs + "nir_lower_returns\nnir_inline_functions\n" +
                  fs.replace("%12", "%13") + "nir_lower_vars_to_ssa\n" + fs.replace("%12", "%14") +
                  "nir_remove_dead_variables\nnir_opt_find_array_copies\n" + fs.replace("%12", "%15"))
        got, passes = n.ssa_dump(stream.split("\n"), "fs")
        # the last dump before the name, from nir_lower_vars_to_ssa; the
        # dump after the name is not the point
        self.assertTrue(any("%14 = fadd" in line for line in got))
        self.assertEqual(passes, ["nir_inline_functions", "nir_lower_vars_to_ssa"])

    def test_ssa_point_model_errors(self):
        fs = LOOP.replace("nir_opt_dce\n", "")
        ok = "glsl_to_nir\n" + fs + "nir_lower_vars_to_ssa\nnir_remove_dead_variables\nnir_opt_find_array_copies\n"
        self.assertEqual(n.ssa_dump(ok.split("\n"), "fs")[1], [])
        # another pass between the two and the point
        wrong = ok.replace("nir_remove_dead_variables\n", "nir_remove_dead_variables\nnir_opt_dce\n")
        with self.assertRaisesRegex(n.DumpError, "nir_remove_dead_variables, nir_opt_dce before"):
            n.ssa_dump(wrong.split("\n"), "fs")
        # the two in the other order
        wrong = ok.replace("nir_lower_vars_to_ssa\nnir_remove_dead_variables",
                           "nir_remove_dead_variables\nnir_lower_vars_to_ssa")
        with self.assertRaisesRegex(n.DumpError, "not nir_lower_vars_to_ssa"):
            n.ssa_dump(wrong.split("\n"), "fs")
        with self.assertRaisesRegex(n.DumpError, "no nir_opt_find_array_copies in the fs stream"):
            n.ssa_dump(ok.replace("nir_opt_find_array_copies", "nir_opt_dce").split("\n"), "fs")
        with self.assertRaisesRegex(n.DumpError, "0 glsl_to_nir names"):
            n.ssa_dump(ok.replace("glsl_to_nir", "nir_lower_returns").split("\n"), "fs")
        # the stream of one stage holds that stage only
        with self.assertRaisesRegex(n.DumpError, "a MESA_SHADER_FRAGMENT dump in the vs stream"):
            n.ssa_dump(ok.split("\n"), "vs")

    def test_ssa_point_drops_functions_but_the_entry_point(self):
        # gl_mesa_tmp_<hash> printed by the last pass before the point, then
        # removed by nir_remove_non_entrypoints, which prints nothing
        fs = LOOP.replace("nir_opt_dce\n", "")
        extra = fs.replace("decl_function main () (entrypoint)\n",
                           "decl_function gl_mesa_tmp_0f ()\n\nimpl gl_mesa_tmp_0f {\n"
                           "    block b0:  // preds:, succs: b1\n    block b1:\n}\n\n"
                           "decl_function main () (entrypoint)\n")
        tail = "nir_lower_vars_to_ssa\nnir_remove_dead_variables\nnir_opt_find_array_copies\n"
        vs = ("glsl_to_nir\n" + fs + "nir_inline_functions\n" + fs + tail).replace("FRAGMENT", "VERTEX")
        stderr = (fs.replace("FRAGMENT", "VERTEX") + fs).split("\n")
        got = {}
        for name, last in (("plain", fs), ("extra", extra)):
            stream = "glsl_to_nir\n" + fs + "nir_inline_functions\n" + last + tail
            got[name] = n.points_of(vs, stream, "\n".join(stderr))
        self.assertEqual([p[3] for p in got["extra"]], [p[3] for p in got["plain"]])
        self.assertEqual(n.cut_dumps(extra.split("\n"))[0][1][4], "decl_function gl_mesa_tmp_0f ()")
        # the select point keeps every function
        self.assertIn("decl_function gl_mesa_tmp_0f ()", n.normalise(dump(extra)))
        b0 = "    block b0:   // preds:\n"
        called = extra.replace(b0, b0 + "    call gl_mesa_tmp_0f ()\n")
        with self.assertRaisesRegex(n.DumpError, "a call at the ssa point: call gl_mesa_tmp_0f"):
            n.points_of(vs, "glsl_to_nir\n" + fs + "nir_inline_functions\n" + called + tail, "\n".join(stderr))

    def test_sha_line_forms(self):
        points = [("vs", "ssa", [], "a\n", ["p1", "p1", "p2"]), ("vs.position", "select", [], "b\n", None),
                  ("fs", "ssa", [], "c\n", [])]
        h = [hashlib.sha256(t.encode()).hexdigest() for t in ("a\n", "b\n", "c\n")]
        self.assertEqual(n.sha_text(points), "vs ssa : %s : p1,p1,p2\nvs.position select : %s\nfs ssa : %s : -\n"
                         % tuple(h))

    def test_select_counts(self):
        vs = LOOP.replace("FRAGMENT", "VERTEX")
        self.assertEqual(sorted(n.select_dumps((vs + LOOP).split("\n"))), ["fs", "vs.position"])
        self.assertEqual(sorted(n.select_dumps((vs + vs + LOOP).split("\n"))),
                         ["fs", "vs.position", "vs.varying"])
        with self.assertRaisesRegex(n.DumpError, "3 vertex and 1 fragment"):
            n.select_dumps((vs * 3 + LOOP).split("\n"))
        with self.assertRaisesRegex(n.DumpError, "1 vertex and 2 fragment"):
            n.select_dumps((vs + LOOP * 2).split("\n"))


def ref(words, sec=None, varying=False):
    r = {"trace": "x.trace", "secondary_offset": sec,
         "regions": {"vs.position": {"words": words, "fau": ["0"]}, "fs.fragment": {"words": ["aa"]}}}
    if varying:
        r["regions"]["vs.varying"] = {"words": ["bb"]}
    return r


class Draw(unittest.TestCase):
    def test_words_equal(self):
        self.assertTrue(n.words_equal(ref(["01"]), ref(["01"])))
        # the FAU and the trace name are not the program
        other = ref(["01"])
        other["regions"]["vs.position"]["fau"] = ["1"]
        other["trace"] = "y.trace"
        self.assertTrue(n.words_equal(other, ref(["01"])))
        self.assertFalse(n.words_equal(ref(["02"]), ref(["01"])))
        self.assertFalse(n.words_equal(ref(["01"], sec=8), ref(["01"])))
        self.assertFalse(n.words_equal(ref(["01"], varying=True), ref(["01"])))

    def test_bindings(self):
        with tempfile.TemporaryDirectory() as d:
            base = os.path.join(d, "p")
            self.assertEqual(n.bindings(base), [])
            with open(base + ".bind", "w") as f:
                f.write("# a comment\na_0 1\n\na_1 1  # aliased\n")
            self.assertEqual(n.bindings(base), ["--attrib-bindings", "a_0=1,a_1=1"])
            with open(base + ".bind", "w") as f:
                f.write("a_0\n")
            with self.assertRaises(n.DumpError):
                n.bindings(base)

    def test_args(self):
        vals, names = n.parse_args(["--jobs", "3", "--texts", "/t", "deqp_*"])
        self.assertEqual((vals["--jobs"], vals["--texts"], names), ("3", "/t", ["deqp_*"]))
        self.assertIsNone(n.parse_args(["--jobs", "0"]))
        self.assertIsNone(n.parse_args(["--jobs"]))
        self.assertIsNone(n.parse_args(["--unknown"]))
        # a --scratch that is no directory is one usage error, not one ERROR per pair
        self.assertEqual(n.main(["--scratch", os.path.join(DATA, "no-such-dir"), "fbload_fbload"]), 2)

    def test_select_names(self):
        self.assertEqual(n.select(["fbload_fbload"]), ["fbload_fbload"])
        names = n.select([])
        self.assertIn("fbload_fbload", names)
        # every name is a committed pair
        for name in names[:20]:
            self.assertTrue(os.path.isfile(os.path.join(DATA, name + ".frag")))


if __name__ == "__main__":
    unittest.main()
