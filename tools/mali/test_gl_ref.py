#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Tests of gl_ref.py's `--uniform-values` values (struct members among
them: the names and value indices each member gets) and `--instances`/
`--divisors` layout and of pan_ref_diff.py's `--instanced` extraction that
need no Mesa build.

    python3 -I tools/mali/test_gl_ref.py

The instanced layout is gl_test case 141's (es3instdiv): `pos` per vertex
from a buffer of six vec4s, `offs` and `icol` interleaved in a buffer of
two 24-byte records with divisor 2. The trace snippet is the shape of the
pandecode lines of tools/mali/testdata/instdiv_141_es3instdiv.expect.json's
run (Mesa 26.2.1 on drm-shim).
"""
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gl_ref  # noqa: E402
import pan_ref_diff  # noqa: E402

INST_VERT = os.path.join(HERE, "testdata", "instdiv_141_es3instdiv.vert")
INST_FRAG = os.path.join(HERE, "testdata", "instdiv_141_es3instdiv.frag")

TRACE = """\
      Entry 1 @7ffffffce7d0:
        Address: 0x7ffffffce160
          Attribute @7ffffffce160:
            Type: Attribute
            Attribute type: 1D
            Offset enable: true
            Format (v7): RGBA32F RGBA
            Table: 2
            Frequency: Vertex
            Divisor R: 0
            Divisor E: 0
            Offset: 0
            Buffer index: 0x0
            Stride: 0x10
            Attribute stride: 16
            Packet stride: 0
            Divisor D: 0x0
          Attribute @7ffffffce180:
            Type: Attribute
            Attribute type: 1D POT Divisor
            Offset enable: false
            Format (v7): RG32F RGB1
            Table: 2
            Frequency: Instance
            Divisor R: 1
            Divisor E: 0
            Offset: 0
            Buffer index: 0x1
            Stride: 0x18
            Attribute stride: 24
            Packet stride: 0
            Divisor D: 0x0
      Entry 2 @7ffffffce7e0:
          Buffer @7ffffffce1c0:
            Type: Buffer
            Buffer type: Simple
            Size: 0x60
            Address: 0x7fffffffe6000
            Stride: 0x0
          Buffer @7ffffffce1e0:
            Type: Buffer
            Buffer type: Simple
            Size: 0x30
            Address: 0x7fffffffdd000
            Stride: 0x0
      Entry 3 @7ffffffce7f0:
Position FAU @7ffffffce000:
  Index count: 6
  Instance count: 4
  Vertex offset: 0
  Instance offset: 0
"""


class UniformValues(unittest.TestCase):
    def test_values_by_type(self):
        self.assertEqual(gl_ref.uniform_components(0, "vec2"), (True, [16.0, 17.0]))
        self.assertEqual(gl_ref.uniform_components(1, "ivec3"), (False, [32, 33, 34]))
        # a bool is true where its component is even: both values in a bvecN
        self.assertEqual(gl_ref.uniform_components(2, "bool"), (False, [1]))
        self.assertEqual(gl_ref.uniform_components(2, "bvec4"), (False, [1, 0, 1, 0]))

    def test_a_type_without_values_is_named(self):
        self.assertIsNone(gl_ref.uniform_components(0, "Light"))


STRUCT_FRAG = """\
varying mediump float v_vtxOut;

struct subType
{
	mediump float s0;
	mediump vec2 s1[2];
};
struct structType
{
	mediump int m0;
	bvec4 m1;
	mediump float m2[3];
	subType m3;
};
uniform mediump vec2 u_before;
uniform structType u_var;
uniform structType u_arr[2];
uniform sampler2D u_tex;
uniform mediump float u_after;

void main (void)
{
    gl_FragColor = vec4(u_var.m2[1] + u_arr[1].m3.s1[0].x + u_before.x + u_after);
}
"""


class StructUniforms(unittest.TestCase):
    def test_struct_definitions(self):
        self.assertEqual(gl_ref.struct_types(STRUCT_FRAG), {
            "subType": [("s0", "float", None), ("s1", "vec2", 2)],
            "structType": [("m0", "int", None), ("m1", "bvec4", None), ("m2", "float", 3),
                           ("m3", "subType", None)],
        })

    def test_members_by_dEQP_names(self):
        structs = gl_ref.struct_types(STRUCT_FRAG)
        self.assertEqual(gl_ref.uniform_leaves("u_var", "structType", structs), [
            ("u_var.m0", "int"), ("u_var.m1", "bvec4"), ("u_var.m2[0]", "float"), ("u_var.m2[1]", "float"),
            ("u_var.m2[2]", "float"), ("u_var.m3.s0", "float"), ("u_var.m3.s1[0]", "vec2"),
            ("u_var.m3.s1[1]", "vec2")])
        arr = gl_ref.uniform_leaves("u_arr[2]", "structType", structs)
        self.assertEqual(len(arr), 16)
        self.assertEqual(arr[0], ("u_arr[0].m0", "int"))
        self.assertEqual(arr[8], ("u_arr[1].m0", "int"))
        self.assertEqual(arr[-1], ("u_arr[1].m3.s1[1]", "vec2"))
        # not a struct: unchanged, a plain array declarator included
        self.assertEqual(gl_ref.uniform_leaves("u_v[3]", "vec4", structs), [("u_v[3]", "vec4")])

    def test_a_define_sizes_a_struct_array(self):
        text = "#define N 2\nstruct S { mediump float a[N]; };\nuniform S s[N];\n"
        structs = gl_ref.struct_types(text)
        self.assertEqual(structs, {"S": [("a", "float", 2)]})
        self.assertEqual([n for n, _ in gl_ref.uniform_leaves("s[N]", "S", structs, {"N": 2})],
                         ["s[0].a[0]", "s[0].a[1]", "s[1].a[0]", "s[1].a[1]"])

    def test_struct_values_take_the_next_indices(self):
        got = gl_ref.uniform_settings("void main(){}", STRUCT_FRAG)
        names = [g[0] for g in got]
        self.assertEqual(names[0], "u_before")
        self.assertEqual(got[0][2:], (True, [16.0, 17.0]))
        # u_var.m0 is index 1, an int; u_var.m1 index 2, a bvec4
        self.assertEqual(got[1], ("u_var.m0", "int", False, [32]))
        self.assertEqual(got[2], ("u_var.m1", "bvec4", False, [1, 0, 1, 0]))
        self.assertEqual(got[4], ("u_var.m2[1]", "float", True, [80.0]))
        self.assertEqual(got[9], ("u_arr[0].m0", "int", False, [160]))
        # 1 + 8 + 16 members, then the sampler keeps index 25: u_after is 26
        self.assertEqual(got[-1], ("u_after", "float", True, [432.0]))
        self.assertEqual(len(got), 1 + 8 + 16 + 1)

    def test_without_a_struct_the_numbering_is_unchanged(self):
        text = "uniform vec2 a;\nuniform sampler2D t;\nuniform ivec2 b;\nuniform vec4 c[3];\n"
        got = gl_ref.uniform_settings(text, "uniform bool d;\n")
        self.assertEqual(got, [("a", "vec2", True, [16.0, 17.0]), ("b", "ivec2", False, [48, 49]),
                               ("c[3]", "vec4", True, [64.0, 65.0, 66.0, 67.0]), ("d", "bool", False, [1])])

    def test_a_sampler_member_is_refused(self):
        text = "struct S { mediump float a; sampler2D t; };\nuniform S s;\n"
        with self.assertRaises(SystemExit) as e:
            gl_ref.uniform_settings(text, "")
        self.assertIn("sampler2D uniform (s.t)", str(e.exception))

    def test_the_fclane2_references_set_members(self):
        # deqp_fclane2_struct_nested_float.frag, as drawn for its reference
        pair = os.path.join(HERE, "testdata", "deqp_fclane2_struct_nested_float")
        got = gl_ref.uniform_settings(open(pair + ".vert").read(), open(pair + ".frag").read())
        self.assertEqual([g[0] for g in got], [
            "u_var.m0", "u_var.m1.ms0", "u_var.m1.ms1[0]", "u_var.m1.ms1[1]", "u_var.m1.ms2[0].mss0",
            "u_var.m1.ms2[0].mss1", "u_var.m1.ms2[1].mss0", "u_var.m1.ms2[1].mss1", "u_var.m2"])


class InstanceLayout(unittest.TestCase):
    def test_case_141(self):
        sizes = gl_ref.attribute_sizes_es3(open(INST_VERT).read())
        self.assertEqual(sizes, {"pos": 4, "offs": 2, "icol": 4})
        self.assertEqual(gl_ref.instance_layout([4, 2, 4], [0, 2, 2], 4),
                         (16, 24, 2, [(0, 0), (1, 0), (1, 8)]))

    def test_records_follow_the_smallest_divisor(self):
        self.assertEqual(gl_ref.instance_layout([4, 1, 3], [0, 3, 1], 5)[1:3], (16, 5))
        self.assertEqual(gl_ref.instance_layout([4, 1], [0, 3], 5)[2], 2)
        self.assertEqual(gl_ref.instance_layout([4, 2], [0, 0], 4), (24, 0, 0, [(0, 0), (0, 16)]))

    def test_bad_divisors_are_refused_before_any_library(self):
        for opts in (["--divisors", "0,2"], ["--divisors", "0,2,2,2"], ["--divisors", "0,-1,2"],
                     ["--instances", "0"]):
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "gl_ref.py"), "/nonexistent",
                                INST_VERT, INST_FRAG, *opts], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2, (opts, r.stderr))


class AttribBindings(unittest.TestCase):
    """`--attrib-bindings`: explicit glBindAttribLocation, aliasing included."""

    def test_parse(self):
        self.assertEqual(gl_ref.parse_attrib_bindings("a_0=15,a_16=15,b=0"), [("a_0", 15), ("a_16", 15), ("b", 0)])
        for bad in ("a_0", "a_0=", "a_0=x", "1a=2", "a=1,,b=2", "a=-1", ""):
            with self.assertRaises(ValueError, msg=bad):
                gl_ref.parse_attrib_bindings(bad)

    def test_without_bindings_the_numbering_is_declaration_order(self):
        names = ["p", "m", "c"]
        self.assertEqual(gl_ref.attribute_locations(names, {"p": 1, "m": 1, "c": 1}), [0, 1, 2])
        self.assertEqual(gl_ref.attribute_locations(names, {"p": 1, "m": 3, "c": 1}), [0, 1, 4])

    def test_aliased_locations_are_kept(self):
        names = ["a_0", "a_2", "a_1", "a_3"]
        w = {n: 1 for n in names}
        self.assertEqual(gl_ref.attribute_locations(names, w, [("a_0", 1), ("a_1", 1), ("a_2", 0), ("a_3", 0)]),
                         [1, 0, 1, 0])

    def test_unlisted_attributes_skip_bound_locations(self):
        names = ["pos", "a", "b"]
        w = {"pos": 1, "a": 2, "b": 1}
        # `a` covers 1-2 (bound), so `pos` takes 0 and `b` the next free one, 3
        self.assertEqual(gl_ref.attribute_locations(names, w, [("a", 1)]), [0, 1, 3])
        # a matrix that does not fit before a bound location moves past it
        self.assertEqual(gl_ref.attribute_locations(["m", "x"], {"m": 2, "x": 1}, [("x", 1)]), [2, 1])

    def test_a_name_the_shader_lacks_is_refused(self):
        with self.assertRaises(ValueError):
            gl_ref.attribute_locations(["a"], {"a": 1}, [("b", 0)])

    def test_bad_option_is_refused_before_any_library(self):
        for opts in (["--attrib-bindings", "a_0"], ["--attrib-bindings", "a=1,b=x"],
                     ["--attrib-bindings", "a=1", "--divisors", "0,2,2"]):
            r = subprocess.run([sys.executable, "-I", os.path.join(HERE, "gl_ref.py"), "/nonexistent",
                                INST_VERT, INST_FRAG, *opts], capture_output=True, text=True)
            self.assertEqual(r.returncode, 2, (opts, r.stderr))


class InstancedExtract(unittest.TestCase):
    def test_divisor_fields_and_draw(self):
        with tempfile.NamedTemporaryFile("w", suffix=".trace") as f:
            f.write(TRACE)
            f.flush()
            plain = pan_ref_diff.read_attributes(f.name)
            inst = pan_ref_diff.read_attributes(f.name, True)
        self.assertNotIn("draw", plain)
        self.assertEqual(sorted(plain["attributes"][0]), ["buffer", "format", "frequency", "offset", "stride"])
        a = inst["attributes"][1]
        self.assertEqual((a["type"], a["offset_enable"], a["divisor_r"], a["divisor_e"], a["divisor_d"]),
                         ("1D POT Divisor", False, 1, 0, 0))
        self.assertEqual(inst["attributes"][0]["type"], "1D")
        self.assertEqual(inst["attribute_buffers"][1]["size"], 0x30)
        self.assertEqual(inst["draw"], {"index_count": 6, "instance_count": 4, "instance_offset": 0})


if __name__ == "__main__":
    unittest.main(verbosity=2)
