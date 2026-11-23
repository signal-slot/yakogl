#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Self-test of corpus_diff.py, without Mesa, the CTS or the network.

    python3 -I tools/mali/test_corpus_diff.py

The pairing of single shaders and their generated partners, the dEQP and
glmark2 inputs, the verdicts and the summary on synthetic rows, then the
whole tool as a command on four committed programs of tools/mali/testdata
with their committed references as the cache (--no-mesa): one equal, one
refused, one against another program's reference (differ), one with no
reference. The last test builds and runs tests/mali_corpus_diff.rs (cargo).
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import corpus_diff as cd  # noqa: E402

TOOL = os.path.join(HERE, "corpus_diff.py")
TESTDATA = os.path.join(HERE, "testdata")


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


class Partners(unittest.TestCase):
    def test_version(self):
        self.assertEqual(cd.glsl_version("void main(){}"), "100")
        self.assertEqual(cd.glsl_version("// x\n#version 300 es\nvoid main(){}"), "300 es")
        self.assertEqual(cd.glsl_version("/* #version 450 */\n#version 100\n"), "100")
        self.assertEqual(cd.glsl_version("#version 450\n"), "450")

    def test_fragment_partner_writes_each_varying_from_its_own_attribute(self):
        fs = ("precision mediump float;\nvarying vec2 v_uv;\nvarying lowp vec4 v_c, v_d[2];\n"
              "// varying vec3 gone;\nvoid main() { gl_FragColor = v_c + v_d[1] + vec4(v_uv, 0.0, 1.0); }\n")
        vs = cd.partner_for(fs, "frag")
        self.assertTrue(vs.startswith("#version 100\n"))
        self.assertIn("varying vec2 v_uv;", vs)
        self.assertIn("varying lowp vec4 v_c;", vs)
        self.assertIn("varying lowp vec4 v_d[2];", vs)
        self.assertNotIn("gone", vs)
        for line in ("v_uv = yk_a0.xy;", "v_c = yk_a1;", "v_d[0] = yk_a2;", "v_d[1] = yk_a3;",
                     "gl_Position = yk_pos;"):
            self.assertIn(line, vs)

    def test_vertex_partner_reads_every_varying(self):
        vs = ("#version 300 es\nin vec4 a_pos;\nflat out highp int v_i;\nout mat2 v_m;\n"
              "void main() { v_i = 1; v_m = mat2(1.0); gl_Position = a_pos; }\n")
        fs = cd.partner_for(vs, "vert")
        self.assertTrue(fs.startswith("#version 300 es\n"))
        self.assertIn("flat in highp int v_i;", fs)
        self.assertIn("in mat2 v_m;", fs)
        self.assertIn("c += vec4(float(v_i));", fs)
        self.assertIn("c += vec4(v_m[0], v_m[1]);", fs)
        self.assertIn("yk_color = c;", fs)
        self.assertNotIn("a_pos", fs)

    def test_a_type_without_a_rule_has_no_partner(self):
        self.assertIsNone(cd.partner_for("struct S { float x; };\nvarying S v;\nvoid main(){}\n", "frag"))


class Gather(unittest.TestCase):
    def test_shaders_pairing(self):
        with tempfile.TemporaryDirectory() as td:
            vs = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n"
            fs = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n"
            write(os.path.join(td, "a", "x.vert"), vs)
            write(os.path.join(td, "a", "x.frag"), fs)
            write(os.path.join(td, "a", "y.frag"), fs.replace("= v;", "= v * 2.0;"))
            write(os.path.join(td, "a", "z.vert"), vs.replace("= p;", "= p * 2.0;", 1))
            write(os.path.join(td, "b", "only.vert"), vs.replace("= p;", "= -p;", 1))
            write(os.path.join(td, "b", "f1.frag"), fs)
            write(os.path.join(td, "b", "f2.frag"), "#version 300 es\nprecision mediump float;\nin vec4 v;\n"
                  "out vec4 c;\nvoid main() { c = v; }\n")
            write(os.path.join(td, "b", "desk.frag"), "#version 450\nvoid main() {}\n")
            c = cd.Corpus()
            cd.gather_shaders(c, td)
            names = sorted(r["name"].replace(td + os.sep, "") for r in c.rows.values())
            self.assertEqual(names, [
                "a/x.vert + a/x.frag",            # the stem pair
                "a/y.frag + partner",            # two vertex shaders: no partner from the directory
                "a/z.vert + partner",
                "b/f2.frag + partner",           # another version than the only vertex shader
                "b/only.vert + b/f1.frag",       # the directory's only vertex shader
            ])
            self.assertEqual(dict(c.skipped), {"shaders: desktop GLSL (#version 450)": 1})
            self.assertEqual({r["group"] for r in c.rows.values()}, {"a", "b"})

    def test_deqp_group_is_the_group_of_most_cases(self):
        with tempfile.TemporaryDirectory() as td:
            vs, fs = "void main() { gl_Position = vec4(0.0); }\n", "void main() {}\n"
            key = cd.program_key(vs, fs)
            write(os.path.join(td, "programs", key + ".vert"), vs)
            write(os.path.join(td, "programs", key + ".frag"), fs)
            cases = ["dEQP-GLES2.functional.shaders.loops.a", "dEQP-GLES2.functional.shaders.random.b",
                     "dEQP-GLES2.functional.shaders.random.c"]
            write(os.path.join(td, "programs.tsv"), "program\tcase\n" + "".join("%s\t%s\n" % (key, x) for x in cases))
            c = cd.Corpus()
            cd.gather_deqp(c, td)
            self.assertEqual(c.rows, {("deqp", key): {"group": "shaders.random", "uses": 3, "name": cases[0]}})

    def test_glmark2_tables_are_read(self):
        entries, programs = cd.glmark2_programs()
        self.assertEqual(len(entries), 33)
        self.assertEqual(entries[0], "build:use-vbo=false")
        self.assertEqual(programs[0], ["ace2+4748"])
        self.assertEqual(len(programs[20]), 9)


class Table(unittest.TestCase):
    def table(self, td, measured, norefs=()):
        out, cache = os.path.join(td, "out"), os.path.join(td, "cache")
        os.makedirs(os.path.join(out, "pairs"))
        os.makedirs(cache)
        corpus = "corpus\tgroup\tprogram\tuses\tname\n"
        m = ""
        for i, (word, detail, has_ref) in enumerate(measured):
            key = "%016x" % i
            corpus += "deqp\tg%d\t%s\t%d\tcase%d\n" % (i % 2, key, i + 1, i)
            m += "%s\t%s\t%s\n" % (key, word, detail)
            if has_ref:
                write(os.path.join(out, "pairs", key + ".expect.json"), "{}")
            if i in norefs:
                write(os.path.join(cache, key + ".noref"), "gl_ref: link failed: error: x\n")
        write(os.path.join(out, "corpus.tsv"), corpus)
        write(os.path.join(out, "measure.tsv"), m)
        rows = cd.stage_table(out, cache)
        return out, rows

    def test_verdicts(self):
        with tempfile.TemporaryDirectory() as td:
            out, rows = self.table(td, [
                ("EQUAL", "-", True),
                ("DIFFER", "fs.fragment: 8 words ours, 7 reference; fs.fragment: FAU words ours [1], reference Some([2])", True),
                ("REFUSED", "Valhall: a loop in a vertex shader is not lowered yet", True),
                ("REFUSED", "front end: Rejected", False),
                ("NOREF", "-", False),
                ("ERROR", "the uniform is declared", True),
                ("REFUSED", "Valhall: an integer or boolean value is not lowered yet", False),
            ], norefs=(3, 4, 6))
            self.assertEqual([r[4] for r in rows], ["equal", "refused", "noref", "refused", "differ", "invalid", "error"])
            self.assertTrue(rows[3][6].endswith(" | Mesa: gl_ref: link failed: error: x"), rows[3])
            with open(os.path.join(out, "summary.md")) as f:
                s = f.read()
            self.assertIn("| deqp | 7 | 1 | 1 | 2 | 1 | 1 | 1 | 28 | 1 | 2 | 10 | 5 | 4 | 6 |", s)
            self.assertIn("- 1 (3): Valhall: a loop in a vertex shader is not lowered yet", s)
            self.assertIn("- fs.fragment: N words ours, N reference: 1", s)
            self.assertTrue(s.rstrip().endswith(
                "corpus_diff: 7 rows, 7 programs: equal 1, differ 1, refused 2, noref 1, invalid 1, error 1"))
            with open(os.path.join(out, "table.tsv")) as f:
                first = f.read()
            cd.stage_table(out, os.path.join(td, "cache"))
            with open(os.path.join(out, "table.tsv")) as f:
                self.assertEqual(f.read(), first, "two runs, one table")


class Command(unittest.TestCase):
    """The tool end to end on committed programs (builds the Rust test)."""

    def test_four_committed_programs(self):
        with tempfile.TemporaryDirectory() as td:
            pairs, cache, out = (os.path.join(td, x) for x in ("in", "cache", "out"))
            os.makedirs(pairs)
            os.makedirs(cache)
            progs = {"eq": ("tri_varying", "tri_varying"), "rf": ("fstex_cubeno_texcoord_allconst",) * 2,
                     "df": ("tri_varying", "tri"), "nr": ("tri_varying", None)}
            for name, (src, ref) in progs.items():
                vs = cd.read(os.path.join(TESTDATA, src + ".vert"))
                fs = cd.read(os.path.join(TESTDATA, src + ".frag"))
                if name == "nr":
                    fs += "// no reference for this text\n"
                elif name == "df":
                    fs += "// tri's reference for this text\n"
                write(os.path.join(pairs, name + ".vert"), vs)
                write(os.path.join(pairs, name + ".frag"), fs)
                if ref:
                    shutil.copyfile(os.path.join(TESTDATA, ref + ".expect.json"),
                                    os.path.join(cache, cd.program_key(vs, fs) + ".expect.json"))
            r = subprocess.run([sys.executable, "-I", TOOL, out, "--pairs", pairs, "--ref-cache", cache,
                                "--no-mesa", "--jobs", "2"], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertEqual(r.stdout.splitlines()[-1],
                             "corpus_diff: 4 rows, 4 programs: equal 1, differ 1, refused 1, noref 1, invalid 0, error 0")
            with open(os.path.join(out, "table.tsv")) as f:
                rows = {l.split("\t")[5]: l.rstrip("\n").split("\t") for l in f.readlines()[1:]}
            self.assertEqual({n: rows[n][4] for n in rows}, {"eq": "equal", "rf": "refused", "df": "differ", "nr": "noref"})
            self.assertIn("a texture coordinate other than", rows["rf"][6])
            self.assertTrue(rows["df"][6].startswith("vs.varying: "), rows["df"][6])


if __name__ == "__main__":
    unittest.main()
