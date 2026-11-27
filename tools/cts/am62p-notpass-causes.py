#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Sort each non-Pass line of an AM62P dEQP-GLES3 not-pass TSV into a cause.

usage: python3 -I tools/cts/am62p-notpass-causes.py [--strict] [--list] TSV

The TSV is docs/es3/am62p-gles3-notpass.tsv (lines '#...' are header; the rest
are '<outcome>\\t<case>\\t<first message line>'). Every line is tried against
the RULES table below, in order; the first rule that matches takes the line.
The output is a per-cause count with the document that states the cause, then
the lines no rule took ('unattributed'). Exit 0 always; with --strict exit 1
if any line is unattributed. --list also prints each cause's cases.

A cause whose status is 'open' is one the document names but does not explain
(cause unknown, or no host verdict yet); the line is attributed to that
documented row, not explained.
"""
import re
import sys

# ---------------------------------------------------------------------------
# THE TABLE. One row per rule, first match wins, so the specific rows come
# before the general ones. Fields:
#   id      cause id (K1..K6 are texture-failure-map.md section 2.2.1)
#   status  outcomes the rule takes: a set of Fail/NotSupported/QualityWarning/
#           NoResult, or None for any
#   name    regex (re.search) on the case name without the prefix
#           'dEQP-GLES3.functional.'
#   msg     regex (re.search) on the message column, or None
#   state   'known' (the document gives the cause) or 'open'
#   ref     document and section (docs/es3/ unless a path is given)
#   text    one line of what the cause is
# ---------------------------------------------------------------------------
F, NS, QW, NR = "Fail", "NotSupported", "QualityWarning", "NoResult"
SPEC = r"^texture\.specification\."
INT = r"(?:(?:r|rg|rgb|rgba)(?:8|16|32)u?i|rgb10_a2ui)"

RULES = [
    # --- NoResult: the run never reported (the 3D linear_mipmap_linear images)
    ("NR-3D-LML", {NR}, r"^texture\.filtering\.3d\..*_linear_mipmap_linear$", None, "known",
     "docs/notes/pvr-3d-mipmap-hang.md section 8; texture-failure-map.md section 2.1 (filtering.3d)",
     "3D linear_mipmap_linear: no result (issue #1285, level-0 slice grid, slow check)"),
    ("NR-OTHER", {NR}, r"", None, "open", "docs/notes/pvr-3d-mipmap-hang.md", "no result, outside the 3D mip images"),

    # --- texture.specification: K1..K6 (texture-failure-map.md 2.2.1)
    ("K6", {F}, SPEC + r"texstorage3d\.size\.2d_array_64x32x3_7_levels$", None, "known",
     "texture-failure-map.md 2.2.1 K6", "stacked 2D array: smallest levels under one row per layer"),
    ("K1b", {F}, SPEC + r"random_teximage2d\.cube_", None, "known",
     "texture-failure-map.md 2.2.1 K1b", "integer cube sampler refused by the compiler (POT sides)"),
    ("K1", {F}, SPEC + r"(?:basic_teximage2d|basic_texsubimage2d|teximage2d_pbo|texsubimage2d_pbo)\." + INT + r"_cube$",
     None, "known", "texture-failure-map.md 2.2.1 K1", "integer cube sampler refused by the compiler"),
    ("K1", {F}, SPEC + r"(?:teximage3d_pbo|texsubimage3d_pbo)\." + INT + r"_(?:2d_array|3d)$",
     None, "known", "texture-failure-map.md 2.2.1 K1", "integer 3D / 2D-array sampler refused by the compiler"),
    ("K2", {F}, SPEC + r"(?:(?:basic_teximage2d|basic_texsubimage2d|teximage2d_pbo|texsubimage2d_pbo)\.rgba32f_(?:2d|cube)"
     r"|(?:teximage3d_pbo|texsubimage3d_pbo)\.rgba32f_(?:3d|2d_array)"
     r"|texstorage2d\.format\.rgba32f_2d"
     r"|(?:teximage3d|texsubimage2d|texsubimage3d)_unpack_params\.rgba32f_complex)$",
     None, "known", "texture-failure-map.md 2.2.1 K2 (fixed by ecd9743e)", "RGBA32F stored as halves: values above 65504 became infinity"),
    ("K3", {F}, SPEC + r"texstorage2d\.(?:format\..*_cube|size\.cube_57x57_[26]_levels)$", None, "known",
     "texture-failure-map.md 2.2.1 K3", "NPOT cube keeps only its base level (src/gl change)"),
    ("K4", {F}, SPEC + r"(?:basic_teximage3d\..*_2d_array|texstorage3d\.format\..*_2d_array"
     r"|texstorage3d\.size\.2d_array_57x63x5_2_levels|texsubimage3d_depth\..*)$", None, "known",
     "texture-failure-map.md 2.2.1 K4", "NPOT 2D array keeps only its base level (src/gl change)"),
    ("K5", {F}, SPEC + r"(?:basic_teximage3d\..*_3d|basic_texsubimage3d\..*_3d|texstorage3d\.format\..*_3d"
     r"|texstorage3d\.size\.3d_.*)$", None, "known",
     "texture-failure-map.md 2.2.1 K5", "mipmapped 3D samples level 0 of its slice grid (native 3D)"),

    # --- fbo (fbo-fix-designs.md section 0, sections 1-8)
    ("FBO-5", {F}, r"^fbo\.color\.tex3d\.(?:r|rg)(?:8|16|32)u?i$", None, "known",
     "fbo-fix-designs.md 5", "3D slice of an R/RG integer format"),
    ("FBO-1", {F}, r"^fbo\.(?:color\.(?:tex2d|tex2darray|texcube|tex3d)\.|invalidate\.format\.|"
     r"render\.(?:recreate_color|resize)\.tex2d_)(?:r|rg)(?:8|16|32)u?i(?:_|$)", None, "known",
     "fbo-fix-designs.md 1", "R/RG integer colour textures and renderbuffers stored as RGBA (fbo-rint)"),
    ("FBO-2", {F}, r"^fbo\.msaa\.[24]_samples\.", None, "known",
     "fbo-fix-designs.md 2", "4x MSAA depth/stencil resolve blit refused (fbo-msaablit)"),
    ("FBO-3", {F}, r"^fbo\.color\.texcube\.", None, "known",
     "fbo-fix-designs.md 3", "cube faces rendered through an FBO, sampled by a native-cube program"),
    ("FBO-4", {F}, r"^fbo\.color\.tex3d\.", None, "known",
     "fbo-fix-designs.md 4", "3D slices rendered through an FBO, sampled through the slice grid"),
    ("FBO-6", {F}, r"^fbo\.(?:blit\.default_framebuffer\.r8_|color\.blend\.(?:r8|rg8)_|"
     r"render\.shared_colorbuffer(?:_clear)?\.)", None, "known",
     "fbo-fix-designs.md 6", "R8 / RG8 / RG16F targets keep lanes the format lacks"),
    ("FBO-7", {F}, r"^fbo\.invalidate\.(?:sub|whole)\.unbind_blit_msaa_", None, "known",
     "fbo-fix-designs.md 7", "4x blit that carries a depth plane (joins fbo-msaablit)"),
    ("FBO-8", {F}, r"^fbo\.depth\.depth_(?:test|write)_clamp\.", None, "known",
     "fbo-fix-designs.md 8", "per-fragment gl_FragDepth (joins the shaders.fragdepth cause)"),

    # --- fragment_out (int-rt-gc.md section 6)
    ("FO-B", {F}, r"^fragment_out\.array\.(?:int|uint)\.", None, "known",
     "int-rt-gc.md 6.1, 6.2", "integer MRT clusters B1 (48) and B2 (24)"),
    ("FO-RANDOM", {F}, r"^fragment_out\.random\.", None, "known",
     "int-rt-gc.md 6.3-6.6", "random draw-buffer mixes: clusters C, D1-D3, E1, E3"),
    ("FO-ARRAY-FLOAT", {F}, r"^fragment_out\.array\.float\.", None, "open",
     "int-rt-gc.md 6.7 (not named in the cluster table)", "float array draw buffers"),

    # --- state_query, uniform_api, qualification_order (backend-caps.md 11)
    ("S-A", {F}, r"^(?:implementation_limits\.max_(?:color_attachments|draw_buffers)|"
     r"state_query\.integers\.max_(?:color_attachments|draw_buffers)_)", None, "known",
     "backend-caps.md 11.4", "the PowerVR answers 3 draw buffers / colour attachments"),
    ("S-B", {F}, r"max_vertex_output_components", None, "known",
     "backend-caps.md 11.5", "GL_MAX_VERTEX_OUTPUT_COMPONENTS is 60 on the PowerVR"),
    ("S-C", {F}, r"^state_query\.rbo\.renderbuffer_component_size_color", None, "known",
     "backend-caps.md 11.6", "GL_RGB10_A2 renderbuffer held as RGBA8"),
    ("S-D", {F}, r"^uniform_api\.random\.83$", None, "known",
     "backend-caps.md 11.7", "vertex shader did not fit the shader image (fixed a0ee07e3)"),
    ("S-E", {F}, r"^shaders\.qualification_order\.variables\.valid", None, "open",
     "backend-caps.md 11.8", "centroid scalar varyings, image wrong; cause open"),
    ("S-F", {NS}, r"^(?:state_query\.indexed\.|negative_api\.fragment\..*i$|draw_buffers_indexed\.)", None, "known",
     "backend-caps.md 11.9", "GL_EXT_draw_buffers_indexed (and advanced blend) not claimed; not ES 3.0"),

    # --- shaders (shader-failure-map.md section 2, int-rt-gc.md)
    ("SH-MODF", {F}, r"^shaders\.builtin_functions\.common\.modf", None, "open",
     "shader-failure-map.md 2 (builtin_functions); int-rt-gc.md 6.1; docs/notes/pvr-mrt4-budget.md",
     "modf writes two outputs (more than 4 output dwords)"),
    ("SH-PREC-MAT", {F, QW}, r"^shaders\.builtin_functions\.precision\.(?:matrixcompmult|outerproduct|transpose)", None, "open",
     "shader-failure-map.md 2 (builtin_functions)", "matrix precision functions: results outside dEQP's bound"),
    ("SH-PREC", {F, QW}, r"^shaders\.builtin_functions\.precision\.", None, "open",
     "shader-failure-map.md 2 (builtin_functions)", "precision cases outside the bound (sign, tanh, ...)"),
    ("SH-DERIV", {F}, r"^shaders\.derivate\.", None, "known",
     "shader-failure-map.md 2 (derivate)", "RGBA32UI / 4-sample surface of the derivative texture cases"),
    ("SH-FRAGDEPTH", {F}, r"^shaders\.fragdepth\.", None, "open",
     "shader-failure-map.md 2 (fragdepth); fbo-fix-designs.md 8", "gl_FragDepth written after control flow"),
    ("SH-TEXFN", {F}, r"^shaders\.texture_functions\.", None, "known",
     "shader-failure-map.md 2 (texture_functions); texture-failure-map.md", "2D-array / 3D / textureSize look-ups"),

    # --- texture.* outside specification (texture-failure-map.md 2.1)
    ("T-SHADOW", {F, QW}, r"^texture\.shadow\.", None, "known",
     "texture-failure-map.md 2.1 (shadow)", "shadow look-ups: compare, stacked 2D array, low-quality result"),
    ("T-FILTER", {F}, r"^texture\.filtering\.(?:2d|cube|2d_array|3d)\.", None, "known",
     "texture-failure-map.md 2.1 (filtering); docs/notes/pvr-3d-mipmap-hang.md", "2D array stacking; 3D slice grid; NPOT levels"),
    ("T-FORMAT", {F}, r"^texture\.format\.", None, "known",
     "texture-failure-map.md 2.1 (format.sized)", "integer / sized formats sampled or uploaded wrongly"),
    ("T-UNITS", {F}, r"^texture\.units\.", None, "open",
     "texture-failure-map.md 2.1 (units)", "mixed-type texture units; format list shared with sized formats"),
    ("T-VERTEX", {F}, r"^texture\.vertex\.", None, "open",
     "texture-failure-map.md 2.1 (vertex)", "texture fetch from the vertex shader"),
    ("T-MIPMAP", {F}, r"^texture\.mipmap\.", None, "known",
     "texture-failure-map.md 2.1 (mipmap.3d)", "3D mip selection, bias on 3D, NPOT levels"),
    ("T-SWIZZLE", {F}, r"^texture\.swizzle\.", None, "open",
     "texture-failure-map.md 2.1 (swizzle)", "sampler swizzle state; not format-bound"),

    # --- vertex_arrays
    ("VA", {F}, r"^vertex_arrays\.single_attribute\.", None, "open",
     "shader-failure-map.md 2 (output_types, int2_10_10_10)", "vertex attribute formats"),

    # --- rasterization quality warnings
    ("QW-RAST", {QW}, r"^rasterization\.", None, "open",
     "shader-failure-map.md 2", "projected interpolation / multisampled wide lines"),

    # --- NotSupported (texture-failure-map.md 3, backend-caps.md 3-4, srgb.md)
    ("NS-ASTC", {NS}, r"^texture\.(?:compressed\.astc|wrap\.astc_)", None, "known",
     "texture-failure-map.md 3", "ASTC: not an ES 3.0 requirement"),
    ("NS-ETC", {NS}, r"^texture\.(?:wrap\.(?:etc2|eac)|format\.compressed)", None, "known",
     "texture-failure-map.md 3; pvr-formats.md 2.4", "ETC2/EAC compressed formats"),
    ("NS-SRGB-R8RG8", {NS}, None, r"GL_EXT_texture_sRGB_(?:R8|RG8)", "known",
     "srgb.md (EXT_texture_sRGB_R8/RG8 not claimed)", "extension not claimed"),
    ("NS-FLOAT-RT", {NS}, None, r"requires GL_EXT_color_buffer_float|^Format not supported$|Unsupported attachment format", "known",
     "backend-caps.md 4 (EXT_color_buffer_float); int-rt-gc.md 3", "float colour targets need EXT_color_buffer_float"),
    ("NS-MSAA8", {NS}, r"^(?:fbo\.msaa\.8_samples|multisample\.fbo_8_samples)", None, "known",
     "backend-caps.md 3 (MAX_SAMPLES 4)", "8 samples exceed MAX_SAMPLES 4"),
    ("NS-MSAA-RGBA16F", {NS}, r"^fbo\.msaa\.[24]_samples\.rgba16f$", None, "known",
     "docs/notes/es3-msaa-renderbuffers.md (d), row fbo.msaa.{2,4}_samples colour", "float multisample colour renderbuffer: stays NotSupported"),
    ("NS-MSAA-DEFAULT-FB", {NS}, r"^multisample\.default_framebuffer\.", None, "known",
     "docs/notes/es3-msaa-renderbuffers.md (d), row multisample.default_framebuffer", "no 4x EGL config on the dEQP image (\"No multisample buffers\")"),
    ("NS-DEPTH-STENCIL", {NS}, None, r"Separate depth and stencil", "open",
     "fbo-fix-designs.md 2, 7", "separate depth and stencil attachments"),
    ("NS-PBO-FLOAT", {NS}, r"^pbo\.renderbuffer\.", None, "known",
     "backend-caps.md 4 (EXT_color_buffer_float)", "float renderbuffer targets"),
]


def compile_rules():
    out = []
    for r in RULES:
        cid, st, name, msg, state, ref, text = r
        out.append((cid, st, re.compile(name) if name is not None else None,
                    re.compile(msg) if msg else None, state, ref, text))
    return out


def read_tsv(path):
    lines = []
    with open(path, encoding="utf-8") as f:
        for raw in f:
            raw = raw.rstrip("\n")
            if not raw or raw.startswith("#"):
                continue
            parts = raw.split("\t")
            if len(parts) < 2:
                continue
            lines.append((parts[0], parts[1], parts[2] if len(parts) > 2 else ""))
    return lines


def attribute(lines):
    rules = compile_rules()
    prefix = "dEQP-GLES3.functional."
    by_cause = {}
    unattr = []
    for outcome, case, msg in lines:
        short = case[len(prefix):] if case.startswith(prefix) else case
        for cid, st, nre, mre, state, ref, text in rules:
            if st is not None and outcome not in st:
                continue
            if nre is not None and not nre.search(short):
                continue
            if mre is not None and not mre.search(msg):
                continue
            by_cause.setdefault(cid, []).append((outcome, case))
            break
        else:
            unattr.append((outcome, case, msg))
    return by_cause, unattr


def main(argv):
    strict = "--strict" in argv
    listing = "--list" in argv
    args = [a for a in argv if not a.startswith("--")]
    if len(args) != 1:
        sys.stderr.write(__doc__)
        return 2
    lines = read_tsv(args[0])
    by_cause, unattr = attribute(lines)
    meta = {}
    for cid, st, nre, mre, state, ref, text in compile_rules():
        meta.setdefault(cid, (state, ref, text))
    print("cause\tcases\tstate\tdocument\twhat")
    order = [c for c in meta if c in by_cause]
    for cid in order:
        state, ref, text = meta[cid]
        print("%s\t%d\t%s\t%s\t%s" % (cid, len(by_cause[cid]), state, ref, text))
        if listing:
            for o, c in by_cause[cid]:
                print("\t%s\t%s" % (o, c))
    known = sum(len(v) for c, v in by_cause.items() if meta[c][0] == "known")
    opened = sum(len(v) for c, v in by_cause.items() if meta[c][0] == "open")
    print("total\t%d\tknown\t%d\topen\t%d" % (len(lines), known, opened))
    print("unattributed\t%d" % len(unattr))
    for o, c, m in unattr:
        print("\t%s\t%s\t%s" % (o, c, m))
    if strict and unattr:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
