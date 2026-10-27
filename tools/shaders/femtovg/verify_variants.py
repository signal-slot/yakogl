#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Check tools/shaders/femtovg/ against the femtovg 0.26.0 no_std fork's program assembly.

Usage: python3 -I verify_variants.py [<femtovg src/renderer/opengl dir>]

Without an argument the body of main-fs.glsl is recovered from the committed
fill_color.frag (everything after the SELECT_SHADER define block) and the
whole corpus is regenerated from it with the variant rules of the fork's
MainProgram::new / OpenGl::new_from_function_cstr (see PROVENANCE.md). That
proves the files agree with each other and with the rules, not with upstream.
With a directory (an upstream femtovg-0.26.0 or the patched fork) main-fs.glsl
and main-vs.glsl are read from it, and the comparison is against upstream
text; for an UNpatched upstream the NO_DISCARD hunk is absent, so only the
patched fork matches the nodiscard files.

Exit status 0: every expected file exists, is byte for byte what the rules give,
and no other file is in the directory (besides this script and PROVENANCE.md).
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
V, DEFS = "#version 100", "#define EDGE_AA 1"
# ShaderType::to_u8() -> corpus name; index 7 is None in both the fork and upstream.
PROGS = [(0, "fill_gradient"), (1, "fill_image"), (2, "stencil"), (3, "fill_image_gradient"),
         (4, "filter_image"), (5, "fill_color"), (6, "texture_copy_unclipped"),
         (8, "fill_gradient_conic"), (9, "fill_image_gradient_conic")]
# generate_shader_program_variants(): with a glyph texture only these exist
GLYPH = {0, 1, 3, 5, 8, 9}
# main_programs_no_scissor: the same set
NOSCISSOR = GLYPH
SCISSOR = "float scissor = scissorMask(fpos);"


def rd(p):
    with open(p, newline="") as f:
        return f.read().replace("\r\n", "\n")


def variants(fs, vs):
    """name -> text, as MainProgram::new assembles them (glyph, no_discard, no_scissor)."""
    out = {"main.vert": f"{V}\n{DEFS}\n{vs}"}

    def prog(n, glyph, nd, ns):
        sel = (f"#define SELECT_SHADER {n}\n" + ("#define ENABLE_GLYPH_TEXTURE" if glyph else "")
               + ("\n#define NO_DISCARD" if nd else "") + ("\n#define NO_SCISSOR" if ns else ""))
        body = fs.replace(SCISSOR, "float scissor = 1.0;") if ns else fs
        return f"{V}\n{DEFS}\n{sel}\n{body}"

    for n, name in PROGS:
        for nd in (False, True):
            if n == 6 and nd:
                continue  # TextureCopyUnclipped is built with no_discard = false
            sfx = ".nodiscard" if nd else ""
            out[f"{name}{sfx}.frag"] = prog(n, False, nd, False)
            if n in GLYPH:
                out[f"{name}.glyph{sfx}.frag"] = prog(n, True, nd, False)
            if nd and n in NOSCISSOR:
                out[f"{name}.nodiscard.noscissor.frag"] = prog(n, False, True, True)
    return out


def main():
    if len(sys.argv) > 1:
        d = sys.argv[1]
        fs, vs = rd(os.path.join(d, "main-fs.glsl")), rd(os.path.join(d, "main-vs.glsl"))
        src = d
    else:
        fc = rd(os.path.join(HERE, "fill_color.frag"))
        head = f"{V}\n{DEFS}\n#define SELECT_SHADER 5\n\n"
        assert fc.startswith(head), "fill_color.frag does not start with the expected defines"
        fs = fc[len(head):]
        mv = rd(os.path.join(HERE, "main.vert"))
        assert mv.startswith(f"{V}\n{DEFS}\n")
        vs = mv[len(f"{V}\n{DEFS}\n"):]
        src = "recovered from fill_color.frag / main.vert"
    want = variants(fs, vs)
    bad = 0
    for name, text in sorted(want.items()):
        p = os.path.join(HERE, name)
        if not os.path.exists(p):
            print("MISSING", name)
            bad += 1
        elif rd(p) != text:
            print("DIFFERS", name)
            bad += 1
    extra = sorted(set(os.listdir(HERE)) - set(want) - {"verify_variants.py", "PROVENANCE.md"})
    for e in extra:
        print("EXTRA", e)
        bad += 1
    print(f"{len(want)} variants expected ({src}); {bad} problem(s)")
    return 1 if bad else 0


sys.exit(main())
