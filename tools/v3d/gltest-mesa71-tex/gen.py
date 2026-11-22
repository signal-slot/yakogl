#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""gen.py: Mesa's offline V3D 7.1 programs for gl_test's texture, mipmap and cube cases.

    python3 -I tools/v3d/gltest-mesa71-tex/gen.py --shc V3D_SHC --record71 BIN \\
        --mesa-src TREE --out-dir DIR --work DIR
    python3 -I tools/v3d/gltest-mesa71-tex/gen.py --recheck tools/v3d/gltest-mesa71-tex \\
        --out-dir DIR

A thin driver over tools/v3d/gltest-mesa71/gen.py (loaded, not copied: its
compile_program, record, attr_reads, header and recheck are used as they are)
with this directory's PROGRAMS. The only change to the output is the two
lines of the header comment that name the generator and the check script.
Run by tools/v3d/check71.d/gltest_mesa71_tex.sh; the programs and the cases
that link them are in README.md here.
"""
import argparse
import importlib.util
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ORIG = os.path.join(HERE, "..", "gltest-mesa71", "gen.py")
spec = importlib.util.spec_from_file_location("gltest_mesa71_gen", ORIG)
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)

PROGRAMS = {
    "texture": (("file", "gl_test_es100/uv.vert"), ("file", "gl_test_es100/texture.frag"),
                "gl_test_uv_vs_src + gl_test_texture_fs_src\n"
                " * (driver/tools/pvrlab/gl_test_shaders_es100.h): case 2 (case_texture, the\n"
                " * 2D texture filled by glTexImage2D and the glGenerateMipmap chain)"),
    "texture2": (("file", "gl_test_es100/uv.vert"), ("file", "gl_test_es100/texture2.frag"),
                 "gl_test_uv_vs_src + gl_test_texture2_fs_src\n"
                 " * (driver/tools/pvrlab/gl_test_shaders_es100.h): case 2 (case_texture, its\n"
                 " * two-sampler draw)"),
    "cube": (("file", "es300/cube.vert"), ("file", "es300/cube.frag"),
             "gl_test_es300_cube_vs_src + gl_test_es300_cube_fs_src\n"
             " * (driver/tools/pvrlab/gl_test_shaders_es100.h, #version 300 es): case 41\n"
             " * (case_es3cube, the first GL_TEXTURE_CUBE_MAP sample)"),
}
g.PROGRAMS = PROGRAMS

_header = g.header


def header(name, why, srcs, js, rec, attrs, qnames):
    """The original header, with a sampler (a uniform_names entry with a
    binding and no slot) written as its own comment line."""
    samplers = {}
    fake = dict(js)
    fake["uniform_names"] = []
    for u in js["uniform_names"]:
        if "slot" in u:
            fake["uniform_names"].append(u)
        else:
            samplers[u["name"]] = u
            fake["uniform_names"].append(dict(u, slot=-1, vec4s=0))
    text = _header(name, why, srcs, fake, rec, attrs, qnames)
    for n, u in samplers.items():
        text = text.replace("/*   %s: %s stage storage, slot -1, 0 vec4 */" % (n, u["stage"]),
                            "/*   %s: sampler, %s stage, binding %d (no uniform words) */"
                            % (n, u["stage"], u["binding"]))
    return (text
            .replace("tools/v3d/gltest-mesa71/gen.py", "tools/v3d/gltest-mesa71-tex/gen.py")
            .replace("tools/v3d/check71.d/gltest_mesa71.sh", "tools/v3d/check71.d/gltest_mesa71_tex.sh"))


g.header = header


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--recheck", metavar="COMMITTED")
    ap.add_argument("--shc")
    ap.add_argument("--record71")
    ap.add_argument("--mesa-src")
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--work")
    a = ap.parse_args()
    os.makedirs(a.out_dir, exist_ok=True)
    if a.recheck:
        sys.exit(1 if g.recheck(a.recheck, a.out_dir) else 0)
    if not (a.shc and a.record71 and a.mesa_src and a.work):
        ap.error("--shc, --record71, --mesa-src and --work are needed without --recheck")
    os.makedirs(a.work, exist_ok=True)
    qnames = g.quniform_names(a.mesa_src)
    for name, (vs, fs, why) in PROGRAMS.items():
        text, js, dis = g.compile_program(a.shc, name, vs, fs, a.work)
        rec = g.record(a.record71, js)
        with open(os.path.join(a.out_dir, name + ".json"), "w") as f:
            f.write(text)
        with open(os.path.join(a.out_dir, name + ".disasm"), "w") as f:
            f.write(dis)
        with open(os.path.join(a.out_dir, name + ".h"), "w") as f:
            f.write(header(name, why, (g.describe(vs), g.describe(fs)), js, rec, g.attr_reads(js), qnames))
        print("gen.py: %s: fs %d, vs %d, cs %d instructions, record flags 0x%08x"
              % (name, len(js["fs"]["code"]), len(js["vs"]["code"]), len(js["cs"]["code"]), rec["word0"]))


if __name__ == "__main__":
    main()
