# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The GLSL ES 1.00 parser pieces glsl2vivante.py shares with the PowerVR
shader tool. That tool is Rust now (tools/yakogl-tools); these functions are
copied verbatim from its Python original (upstream 7e76640 tools/glsl2usc.py)
so the Vivante tool keeps working unchanged."""
import re


def stage_of(path):
    if path.endswith(".vert"):
        return "vs"
    if path.endswith(".frag"):
        return "fs"
    raise SystemExit("glsl2usc: %s: expected a .vert or .frag file" % path)


# The GLSL types a uniform or an attribute can have, as (dwords, std430
# alignment in dwords, GL type enum). Verbatim from the former
# tools/glsl2usc.py, which this file took over the ES 1.00 pieces of; the
# Vivante table generator reads it for the reflection.
GLSL_TYPES = {
    "float": (1, 1, 0x1406), "vec2": (2, 2, 0x8B50), "vec3": (3, 4, 0x8B51), "vec4": (4, 4, 0x8B52),
    "int":   (1, 1, 0x1404), "ivec2": (2, 2, 0x8B53), "ivec3": (3, 4, 0x8B54), "ivec4": (4, 4, 0x8B55),
    "bool":  (1, 1, 0x8B56), "bvec2": (2, 2, 0x8B57), "bvec3": (3, 4, 0x8B58), "bvec4": (4, 4, 0x8B59),
    "mat2":  (4, 2, 0x8B5A), "mat3": (12, 4, 0x8B5B), "mat4": (16, 4, 0x8B5C),
    "uint":  (1, 1, 0x1405), "uvec2": (2, 2, 0x8DC6), "uvec3": (3, 4, 0x8DC7), "uvec4": (4, 4, 0x8DC8),
}

SAMPLER_TYPES = {"sampler2D": 0x8B5E, "samplerCube": 0x8B60, "sampler3D": 0x8B5F,
                 "sampler2DShadow": 0x8B62, "sampler2DArray": 0x8DC1,
                 "sampler2DArrayShadow": 0x8DC4, "samplerCubeShadow": 0x8DC5}


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", " ", text)


ES100_LOCATIONS = {"float": 1, "vec2": 1, "vec3": 1, "vec4": 1, "int": 1, "ivec2": 1,
                   "ivec3": 1, "ivec4": 1, "bool": 1, "bvec2": 1, "bvec3": 1, "bvec4": 1,
                   "uint": 1, "uvec2": 1, "uvec3": 1, "uvec4": 1,
                   "mat2": 2, "mat3": 3, "mat4": 4}


def es_version(text):
    """100 for GLSL ES 1.00 (or version-less legacy text), 300/310 for GLSL
    ES 3.x, None for anything the translator must leave alone."""
    t = strip_comments(text)
    m = re.search(r"^\s*#\s*version\s+(\d+)(\s+es)?", t, re.M)
    if m:
        if m.group(1) == "100":
            return 100
        if m.group(2) and m.group(1) in ("300", "310"):
            return int(m.group(1))
        return None
    return 100 if re.search(r"\b(attribute|varying|gl_FragColor|gl_FragData)\b", t) else None


def is_es100(text):
    return es_version(text) == 100


def es100_statements(text):
    """Split comment-stripped source into (is_toplevel, text) chunks at ';' and
    '}' boundaries outside braces; preprocessor lines are their own chunks."""
    out, buf, depth = [], "", 0
    i = 0
    while i < len(text):
        c = text[i]
        if c == "#" and buf.strip() == "":
            j = text.find("\n", i)
            j = len(text) if j < 0 else j + 1
            # A preprocessor directive must start a line: the preceding
            # statement chunk ends at its ';'/'}' without a newline, and the
            # whitespace now in `buf` (the separating newline) would be dropped.
            out.append((depth == 0, "pp", "\n" + text[i:j]))
            buf = ""
            i = j
            continue
        buf += c
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                out.append((True, "block", buf))
                buf = ""
        elif c == ";" and depth == 0:
            out.append((True, "stmt", buf))
            buf = ""
        i += 1
    if buf.strip():
        out.append((depth == 0, "stmt", buf))
    return out


def _scan_defines(text):
    """Object-like `#define NAME <int>` macros, for array sizes like
    `uniform vec4 frag[UNIFORMARRAY_SIZE];` (FemtoVG). Value must be a literal
    integer (optionally in parentheses)."""
    out = {}
    for mm in re.finditer(r"^\s*#\s*define\s+(\w+)\s+\(?\s*(\d+)\s*\)?\s*$", text, re.M):
        out[mm.group(1)] = int(mm.group(2))
    return out


def _resolve_size(tok, defines):
    """An array size token: a literal, or a #define'd integer macro."""
    if tok is None:
        return None
    tok = tok.strip()
    if tok.isdigit():
        return int(tok)
    if tok in defines:
        return defines[tok]
    if "+" in tok:
        # A constant sum such as dEQP's `u_lights[NUM_DIR + NUM_OMNI]`
        # (es100/light_amount_*.vert writes the operands out): each term is
        # itself a literal or a #define'd integer.
        return sum(_resolve_size(t, defines) for t in tok.split("+"))
    raise SystemExit("glsl2usc: array size '%s' is not an integer literal or a "
                     "#define'd integer" % tok)


def _hoist_gl_position(text, attrib_names):
    """Move the `gl_Position = ...;` statement to the front of main().

    pco_shc assigns vertex-input (vtxin) registers in the order attributes are
    first USED in main(), not in declaration/location order. On this GPU the
    clip-position attribute must land at vtxin0 -- a shader whose position
    attribute ends up at a non-zero vtxin does not rasterise (verified: FemtoVG
    main.vert does `ftcoord = tcoord;` before `gl_Position = ...vertex...`, so
    `tcoord` grabs vtxin0 and `vertex` is pushed to vtxin2 and the draw produces
    no fragments). Hoisting the gl_Position assignment makes its attributes the
    first used, so they take vtxin0.

    Only applied when a non-position attribute is used *before* gl_Position (so
    shaders that already compute position first are byte-for-byte unchanged), and
    only when the gl_Position expression does not depend on a local assigned
    earlier in main() (so the reorder is provably semantics-preserving). The SHA
    the runtime matches is taken from the ORIGINAL ES1.00 source, before this
    translation, so hoisting never changes shader lookup.
    """
    mm = re.search(r"\bvoid\s+main\s*\([^)]*\)\s*{", text)
    if not mm:
        return text
    body_start = mm.end()
    gm = re.search(r"[ \t]*gl_Position\s*=\s*[^;{}]*;[ \t]*\n?", text[body_start:])
    if not gm:
        return text
    stmt = gm.group(0)
    gp_start = body_start + gm.start()
    gp_end = body_start + gm.end()
    before = text[body_start:gp_start]
    if "gl_Position" in before:            # already first / multiple writes: leave it
        return text
    rhs = re.search(r"gl_Position\s*=\s*([^;{}]*);", stmt).group(1)

    def used(names, s):
        return {n for n in names if re.search(r"\b" + re.escape(n) + r"\b", s)}

    attrs_before = used(attrib_names, before)
    attrs_rhs = used(attrib_names, rhs)
    if not (attrs_before - attrs_rhs):     # no other attribute steals vtxin0: no-op
        return text
    # safety: gl_Position RHS must not reference a local assigned earlier in main()
    assigned_before = set(re.findall(r"\b([A-Za-z_]\w*)\s*=", before))
    rhs_ids = set(re.findall(r"\b([A-Za-z_]\w*)\b", rhs))
    if assigned_before & rhs_ids:
        return text
    return text[:body_start] + stmt + before + text[gp_end:]
