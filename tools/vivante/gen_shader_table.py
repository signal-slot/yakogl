#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""gen_shader_table.py - the offline shader table for the Vivante GC7000 backend.

Runs glsl2vivante.py's translation + etna_shc over every shader under
yakogl/tools/shaders and writes src/shaderc/vivante_blobs.rs: the same table
type the PowerVR offline path uses (ShcBlob, looked up by the SHA-256 of the
GLSL source text), with the GC7000 instruction words and the register wiring
(ShcViv) in place of USC and its PowerVR metadata.

Two differences from the PowerVR table (src/shaderc/blobs.rs, written by
`yakogl-tools glsl2usc`):
  - no blend variants: blending and colour masks are PE state on this GPU, so
    a fragment shader has one binary (blend_key 0), and a second one only if
    it reads gl_FrontFacing, which is compiled in here (see below);
  - the input list is the whole tools/shaders tree rather than a hand-kept
    order, because there is no variant matrix to keep deterministic.

It also writes src/vivante/clear_shader.rs: the two internal shaders the
backend draws its clear quad with (tools/shaders/internal/clear.*), as the
same ShcShader the table holds, so the backend programs them through one
code path.

This is the Rust port of the C generator this branch syncs from (upstream
tools/vivante/gen_shader_table.py, which wrote vivante_shader_blobs.c and
src/vivante/gc_clear_shader.h). The reflection half is unchanged; only the
emitters below the "Rust emitters" banner differ, and they follow the
conventions of the generated PowerVR table, src/shaderc/blobs.rs.

Usage:
  ETNA_SHC=<mesa>/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc \\
      python3 tools/vivante/gen_shader_table.py
"""
import os
import re
import sys
import json
import hashlib
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import es100 as u            # noqa: E402  (GLSL type table, ES 1.00 helpers)
import glsl2vivante as v     # noqa: E402  (the translation + etna_shc driver)

SHADERS = os.path.join(HERE, "..", "shaders")

# Directories under tools/shaders/ whose shaders belong in the SHIPPED table,
# plus the one that is deliberately not a table row.
#
# An allowlist and not a denylist, because the failure it prevents is silent
# in both directions. The walk in main() takes every .vert and .frag it finds,
# so a directory added for some other purpose becomes rows in a table that
# ships in a product image, and nobody reviewing that directory would think to
# look here. cts-runner's wedge probes are exactly that: 13 shaders written to
# HANG A GPU (wedge_probe/steps/s08..s36.vert, a_many_temps,
# b_many_instructions, d60/d70_temps, consume_v_color) which a regeneration
# would have shipped. Found 2026-09-22 while the rebuild was being prepared,
# before any of them reached a table.
#
# So a new directory FAILS rather than being guessed either way: add it here
# to ship it, and the failure says so.
TABLE_DIRS = {
    "",                 # tools/shaders itself
    "es100",
    "es300",
    "femtovg",
    "gl_test_es100",
    "qt",
    "qt3d",
}
# Compiled into the backend rather than the table: the clear shaders.
NOT_TABLE_DIRS = {"internal"}


def tool_identity():
    """Who compiled this table: the etna_shc binary and glslang's version.

    `etna_shc` is built from a Mesa tree and has no version string, so its
    identity is the digest of the binary. `glslangValidator` is a release and
    prints one.

    This exists because a regeneration on this host produced 339 rows and
    changed 308 of the 324 that already existed, and nothing in the table said
    which toolchain had built the committed one -- so "the reference" and "what
    my machine produces" were indistinguishable by reading the file.
    """
    ident = {}
    try:
        with open(v.ETNA_SHC, "rb") as f:
            ident["etna_shc"] = hashlib.sha256(f.read()).hexdigest()
    except OSError as e:
        ident["etna_shc"] = "unreadable (%s)" % e.__class__.__name__
    try:
        r = subprocess.run([v.GLSLANG, "--version"], capture_output=True, text=True)
        first = (r.stdout or r.stderr).strip().splitlines()
        ident["glslang"] = first[0][:80] if first else "no version line"
    except OSError as e:
        ident["glslang"] = "unreadable (%s)" % e.__class__.__name__
    return ident


# One row of the emitted table, for comparing a rebuild against the committed
# file row by row. The generator writes this shape, so parsing it back is
# reading its own output rather than guessing at someone else's.
#
# The pattern consumes no newline on either side. An earlier version anchored
# on `\n    ShcBlob {` and ended with `},\n`, which ate the newline the NEXT
# row needed to match -- so it found every other row, reported "changed []"
# for a table whose rows had changed, and the refusal below would never have
# fired. Caught by the sensitivity test rather than by reading it.
ROW_RE = re.compile(r"    ShcBlob \{\n(.*?)\n    \},", re.S)
ROW_NAME_RE = re.compile(r'name: "(.*?)\\0"')


# `static CODE_<n>: [u32; N] = [ ... ];` and the reference to it inside a row.
CODE_DEF_RE = re.compile(r"static (CODE_\d+|[A-Z_]*CODE_[A-Z0-9_]+): \[u32; \d+\] = \[(.*?)\];", re.S)
CODE_REF_RE = re.compile(r"(CODE_\d+|[A-Z_]*CODE_[A-Z0-9_]+)\.as_ptr\(\)")


def rows_that_change(old_text, new_text):
    """(reproduced, changed, added, removed) between two emitted tables.

    Keyed by the row's name, so a reordering is not counted as a change and a
    row that moved is compared with itself.

    **A row's `CODE_<n>` symbol is resolved to the instruction words it names
    before comparing.** The index is positional: adding one source near the
    front renumbers every constant after it, so comparing the row text alone
    reported `color_uv.vert` as CHANGED when its only difference was
    `CODE_1` becoming `CODE_2` and its 4 words were identical. That read as
    "0 of 324 reproduced" -- a toolchain verdict drawn from the emitter's
    numbering. Measured 2026-09-22, and the third instrument of mine that day
    to name what it could see and treat the rest as one thing.
    """
    def rows(text):
        code = {m.group(1): " ".join(m.group(2).split()) for m in CODE_DEF_RE.finditer(text)}
        out = {}
        for m in ROW_RE.finditer(text):
            body = m.group(1)
            n = ROW_NAME_RE.search(body)
            if not n:
                continue
            body = CODE_REF_RE.sub(lambda r: "CODE[" + code.get(r.group(1), "?") + "]", body)
            out[n.group(1)] = body
        return out

    old, new = rows(old_text), rows(new_text)
    same = sorted(k for k in old if k in new and old[k] == new[k])
    changed = sorted(k for k in old if k in new and old[k] != new[k])
    return same, changed, sorted(set(new) - set(old)), sorted(set(old) - set(new))


def stray_dirs(found):
    """Directories holding shaders that this generator was never told about.

    `found` maps a directory relative to tools/shaders ("" for the root) to
    the shader files in it. Separate from main() so that the check has a
    self-test: a scan that cannot fail says nothing when it passes.
    """
    return sorted(set(found) - TABLE_DIRS - NOT_TABLE_DIRS)


def _self_test():
    """The allowlist catches a stray directory and spares the known ones."""
    allowed = {"": ["a.vert"], "es100": ["b.frag"], "internal": ["clear.vert"]}
    assert stray_dirs(allowed) == [], "a known directory was flagged"

    with_probe = dict(allowed)
    with_probe["wedge_probe"] = ["a_many_temps.vert"]
    with_probe["wedge_probe/steps"] = ["s08.vert"]
    assert stray_dirs(with_probe) == ["wedge_probe", "wedge_probe/steps"], \
        "the shape that would have shipped 13 GPU-hanging shaders was not flagged"

    # And the real tree, which is the only check that stays true as the tree
    # changes: whatever is on disk today must be accounted for.
    on_disk = {}
    for d, _, files in os.walk(SHADERS):
        for f in files:
            if f.endswith((".vert", ".frag")):
                rel = os.path.relpath(d, SHADERS)
                on_disk.setdefault("" if rel == "." else rel, []).append(f)
    assert on_disk, "no shaders found under tools/shaders; this check would pass vacuously"
    real = stray_dirs(on_disk)
    if real:
        raise SystemExit("gen_shader_table --self-test: unaccounted directory(s) on disk: %s\n"
                         "The generator would refuse a rebuild. Add them to TABLE_DIRS or "
                         "NOT_TABLE_DIRS, or move test material out of tools/shaders/."
                         % ", ".join(real))
    print("gen_shader_table: allowlist self-test ok (%d directories accounted for)" % len(on_disk))

    # The row comparison the rebuild refusal rests on. Synthetic first, so the
    # check is shown to FAIL on a changed row -- an earlier pattern here found
    # every other row and reported "changed []" for a table whose rows had
    # changed, which would have made the refusal unreachable.
    a = ('\n    ShcBlob {\n        name: "a.vert\\0",\n        code: [1, 2],\n    },'
         '\n    ShcBlob {\n        name: "b.frag\\0",\n        code: [3],\n    },\n')
    b = ('\n    ShcBlob {\n        name: "a.vert\\0",\n        code: [1, 2],\n    },'
         '\n    ShcBlob {\n        name: "b.frag\\0",\n        code: [9],\n    },'
         '\n    ShcBlob {\n        name: "c.vert\\0",\n        code: [7],\n    },\n')
    same, changed, added, removed = rows_that_change(a, b)
    assert (same, changed, added, removed) == (["a.vert"], ["b.frag"], ["c.vert"], []), \
        "the row comparison does not see a changed row: %r" % ((same, changed, added, removed),)
    assert rows_that_change(b, a)[3] == ["c.vert"], "a removed row is not seen"

    # And on the real table, because a parser that stops matching the emitted
    # shape would report "0 rows, nothing changed" and pass.
    if os.path.exists(OUT_TABLE):
        with open(OUT_TABLE) as f:
            committed = f.read()
        same, changed, added, removed = rows_that_change(committed, committed)
        assert len(same) > 100, \
            "only %d rows parsed from the committed table; the emitted shape has moved" % len(same)
        assert not (changed or added or removed), "the committed table is not identical to itself"
        print("gen_shader_table: %d committed rows parse; row comparison ok" % len(same))

    ident = tool_identity()
    print("gen_shader_table: this host's tools: etna_shc %s, %s"
          % (ident["etna_shc"][:16], ident["glslang"]))

# gl_FrontFacing: this GPU hands the fragment shader a raw 0.0/1.0 whose sense
# depends on glFrontFace, so Mesa compiles one binary per winding and picks the
# comparison with a shader key (etnaviv_nir.c, `ine` against `ieq`). We do the
# same in the offline table: a fragment shader that reads gl_FrontFacing gets a
# second entry under the same sha256, compiled from the source with the sense
# negated, and the GL core asks for it when the effective winding is reversed.
# Blending is PE state on this GPU, so the key space is otherwise unused here
# and 1 collides with nothing.
READS_FRONT_FACING = "gl_FrontFacing"
KEY_FRONT_FLIPPED = 1
OUT_TABLE = os.path.join(HERE, "..", "..", "src", "shaderc", "vivante_blobs.rs")
OUT_CLEAR = os.path.join(HERE, "..", "..", "src", "vivante", "clear_shader.rs")

SHC_UNIFORM_NO_BLOCK = 0xffff
SHC_VIV_NO_REG = 0xff
SHC_VIV_MAX_IO = 16
MAX_ATTRIBS = 16

# Mesa slot bases (src/compiler/shader_enums.h).
VERT_ATTRIB_GENERIC0 = 15
VARYING_SLOT_POS = 0
VARYING_SLOT_PSIZ = 6
VARYING_SLOT_VAR0 = 32

# nir's enum glsl_interp_mode -> the hardware's VARYING_INTERPOLATION_MODE_*
# (what etna_link_shader writes into GL_HALTI5_SHADER_ATTRIBUTES).
INTERP_SMOOTH, INTERP_NONPERSPECTIVE, INTERP_FLAT = 0, 1, 2
NIR_INTERP = {0: INTERP_SMOOTH, 1: INTERP_SMOOTH, 2: INTERP_FLAT, 3: INTERP_NONPERSPECTIVE}

# enum shc_viv_const == Mesa's enum etna_uniform_contents, so the tool's
# numbers go into the table unchanged. Only these are expected here.
CONST_UNUSED, CONST_LITERAL, CONST_UNIFORM = 0, 1, 2
# Mesa's ETNA_UNIFORM_UBO0_ADDR == SHC_VIV_CONST_UBO_ADDR (src/shaderc/mod.rs)
CONST_UBO_ADDR = 11
CONST_NAMES = {0: "UNUSED", 1: "LITERAL", 2: "UNIFORM", 3: "TEXRECT_SCALE_X",
               4: "TEXRECT_SCALE_Y", 5: "TEXTURE_WIDTH", 6: "TEXTURE_HEIGHT",
               7: "TEXTURE_DEPTH", 8: "SAMPLER_LOD_MIN", 9: "SAMPLER_LOD_MAX",
               10: "SAMPLER_LOD_BIAS", 11: "UBO_ADDR"}
# Contents whose data field is a sampler binding, not a uniform index.
CONST_SAMPLER_QUERY = {3, 4, 5, 6, 7, 8, 9, 10}


class Fail(Exception):
    pass


# ---- reflection out of the translated GLSL ---------------------------------

DECL_IN = re.compile(r"layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*in\s+(\w+)\s+(\w+)\s*;")
DECL_OUT = re.compile(r"layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*out\s+(\w+)\s+(\w+)\s*;")
DECL_UNIFORM = re.compile(
    r"layout\s*\(\s*location\s*=\s*(\d+)\s*\)\s*uniform\s+(\w+)\s+(\w+)\s*(?:\[\s*(\w+)\s*\])?\s*;")
DECL_SAMPLER = re.compile(
    r"layout\s*\(\s*binding\s*=\s*(\d+)\s*\)\s*uniform\s+(sampler\w+)\s+(\w+)\s*;")


DECL_STRUCT = re.compile(r"struct\s+(\w+)\s*\{(.*?)\}\s*;", re.S)
STRUCT_MEMBER = re.compile(r"(\w+)\s+(\w+)\s*(?:\[\s*(\w+)\s*\])?\s*;")


def struct_types(translated):
    """Struct declarations, as {name: [(type, member, array size text)]}. Qt's
    scene-graph shaders declare one uniform of a struct type; GL reports its
    members individually ("_25.qt_Matrix"), and so does the GC7000 constant
    file, one vec4 slot at a time."""
    out = {}
    for m in DECL_STRUCT.finditer(translated):
        out[m.group(1)] = [(mm.group(1), mm.group(2), mm.group(3))
                           for mm in STRUCT_MEMBER.finditer(m.group(2))]
    return out


def block_uniforms(translated):
    """Every non-sampler uniform of the translated shader, in declaration order,
    at its std430 dword offset. That is the layout the GL core packs a
    glUniform* value with (src/gl/program.rs), so it is the layout the constant
    register file is mapped from. A uniform of a struct type expands into one
    entry per member, in the order the members take their vec4 slots."""
    defines = u._scan_defines(translated)
    structs = struct_types(translated)
    out, off = [], 0

    def add(name, typ, arr_text, slot_off):
        nonlocal off
        if typ not in u.GLSL_TYPES:
            raise Fail("unsupported uniform type '%s'" % typ)
        size, align, gl_type = u.GLSL_TYPES[typ]
        n = u._resolve_size(arr_text, defines) if arr_text else 1
        stride = (size + align - 1) // align * align if arr_text else size
        off = (off + align - 1) // align * align
        out.append({"name": name, "type": typ, "dword_off": off, "dwords": stride * n,
                    "gl_type": gl_type, "array_len": n, "binding": 0, "inner": 1,
                    "size": size, "align": align, "stride": stride,
                    "slot_off": slot_off})
        off += stride * n
        return elem_slots(typ) * n

    for m in DECL_UNIFORM.finditer(translated):
        typ, name, arr = m.group(2), m.group(3), m.group(4)
        if typ in structs:
            if arr:
                raise Fail("uniform '%s': arrays of structs are not reflected" % name)
            slot_off = 0
            for mtyp, mname, marr in structs[typ]:
                if mtyp in structs:
                    raise Fail("uniform '%s': nested structs are not reflected" % name)
                slot_off += add("%s.%s" % (name, mname), mtyp, marr, slot_off)
            continue
        add(name, typ, arr, 0)
    return out, off


def block_samplers(translated, js):
    """The samplers the compiled shader actually reads, each at the sampler
    register the compiler put it in.

    Two separate facts, from two places. The declaration gives the type and
    the GLSL binding; which samplers survived compilation only the compiler
    knows, and etna_shc reports that by listing a survivor in `uniform_names`
    with a "unit" key (a block uniform has a "slot" instead).

    Where each survivor ended up is the part with a trap in it, and the trap
    has been walked into twice.

    etna_shc's "unit" looks like the register and is not: the tool reports 0
    for every sampler it lists, so texture2.frag, which samples both of its
    declared samplers, comes back with u_tex0 and u_tex1 both claiming unit 0.

    The dense renumbering looks like the answer and is not either. It is true
    of `nir_lower_samplers` in general, and compact.frag -- which declares
    bindings 0 and 1 and samples only binding 1 -- is exactly the shader it
    should apply to. **This toolchain does not do it.** Read the instruction:

        texture.frag   (binding 0, sampled)   0x07811018
        compact.frag   (binding 1, sampled)   0x0f811018
        texture2.frag  (both sampled)         0x07821018 then 0x0f811018

    Bit 27 of the first word is the sampler index, and texture2.frag pins it
    down: its u_tex0 load has the bit clear and its u_tex1 load has it set.
    compact.frag's single load has it **set**. The surviving sampler still
    reads register 1, the number it was declared with, and a driver that puts
    its descriptor in slot 0 leaves the shader sampling an empty slot. On the
    GC7000UL that comes back black, which is what gl_test case 24 read.

    So the binding is simply the declared one. The only thing the compiler
    changes is *which* samplers are there: taking the declarations wholesale
    -- the code before all this -- named samplers the shader had dropped, so
    glGetUniformLocation answered for a uniform that is not in the binary."""
    types, declared = {}, {}
    for m in DECL_SAMPLER.finditer(translated):
        if m.group(2) not in u.SAMPLER_TYPES:
            raise Fail("unsupported sampler type '%s'" % m.group(2))
        types[m.group(3)] = u.SAMPLER_TYPES[m.group(2)]
        declared[m.group(3)] = int(m.group(1))
    out = []
    for un in js["uniform_names"]:
        if "unit" not in un:
            continue                   # a block uniform: it has a vec4 slot
        if un["name"] not in types:
            raise Fail("sampler '%s' has no declaration" % un["name"])
        out.append({"name": un["name"], "dword_off": SHC_UNIFORM_NO_BLOCK, "dwords": 1,
                    "gl_type": types[un["name"]], "array_len": 1,
                    "binding": declared[un["name"]], "inner": 1})
    out.sort(key=lambda s: s["binding"])
    return out


def elem_slots(typ):
    """vec4 slots one element of this type occupies, the way the GL state
    tracker counts them (glsl_count_vec4_slots): one per matrix column,
    one for anything else."""
    return {"mat2": 2, "mat3": 3, "mat4": 4}.get(typ, 1)


def uniform_index_to_block_dword(var, index):
    """Map a constant-file reference (Mesa's uniform index: vec4 slot * 4 +
    component) to the dword offset the GL core stores that component at."""
    cols = elem_slots(var["type"])
    rel = index - var["slot"] * 4
    elem, c = divmod(rel, cols * 4)
    if elem >= var["array_len"]:
        raise Fail("uniform '%s': constant file reads element %d of %d"
                   % (var["name"], elem, var["array_len"]))
    if cols > 1:                       # matrix: one column per vec4 slot
        col, row = divmod(c, 4)
        intra = col * var["align"] + row
    else:
        intra = c
    return var["dword_off"] + elem * var["stride"] + intra


# ---- one shader ------------------------------------------------------------

def compile_one(path, text=None):
    stage = u.stage_of(path)
    if text is None:
        text = open(path).read()
    version = u.es_version(text)
    translated = (v.translate_es100_gl(text, stage) if version == 100 else
                  v.translate_es300_gl(text, stage) if version in (300, 310) else
                  v.translate_450_gl(text))
    js, err = v.compile_one(path, text)
    if err:
        raise Fail(err)
    return stage, text, translated, js


def io_list(entries, names_by_slot, what):
    out = []
    for e in entries:
        if len(out) >= SHC_VIV_MAX_IO:
            raise Fail("more than %d %s" % (SHC_VIV_MAX_IO, what))
        if not 0 <= e["reg"] <= 0xfe:
            raise Fail("%s register %d out of range" % (what, e["reg"]))
        out.append({"reg": e["reg"], "slot": e["slot"] & 0xff,
                    "components": e["num_components"] if "num_components" in e else e["components"],
                    "interp": NIR_INTERP.get(e.get("interp", 0), INTERP_SMOOTH),
                    "name": names_by_slot.get(e["slot"], "")})
    return out


def reflect(path, text=None, sha_text=None):
    """Reflect one shader. `text` compiles something other than the file (a
    variant); `sha_text` is what the table is keyed by, which for a variant is
    still the original source, because that is what the application hands
    glShaderSource and what the lookup hashes."""
    stage, text, translated, js = compile_one(path, text)
    vs = stage == "vs"

    # names, by the slot the compiler reports
    in_names, out_names = {}, {}
    for m in DECL_IN.finditer(translated):
        loc, name = int(m.group(1)), m.group(3)
        in_names[(VERT_ATTRIB_GENERIC0 if vs else VARYING_SLOT_VAR0) + loc] = name
        if vs:
            in_names.setdefault(loc, name)   # sources the compiler numbers rawly
    for m in DECL_OUT.finditer(translated):
        out_names[VARYING_SLOT_VAR0 + int(m.group(1))] = m.group(3)
    in_types = {int(m.group(1)): m.group(2) for m in DECL_IN.finditer(translated)}

    sh = {"name": os.path.relpath(path, SHADERS), "stage": stage,
          "code": [int(w, 16) for w in js["code"]], "temps": js["num_temps"],
          "uses_discard": 1 if js["uses_discard"] else 0,
          # The hardware's gl_FrontFacing is 0.0/1.0 with no sense of its own,
          # so a shader that reads it needs one binary per winding and the GL
          # core has to know to ask for the second. See FRONT_FACING below.
          "reads_front_facing": 1 if READS_FRONT_FACING in text else 0,
          "key": 0}

    # the GC7000 halves
    viv = {
        "in": io_list(js["inputs"], in_names, "shader inputs"),
        "out": io_list(js["outputs"], out_names, "shader outputs"),
        "vs_pos_out_reg": js["vs_pos_out_reg"],
        "vs_pointsize_out_reg": js["vs_pointsize_out_reg"],
        "vs_id_in_reg": js["vs_id_in_reg"],
        "ps_color_out_reg": js["ps_color_out_reg"],
        "ps_depth_out_reg": js["ps_depth_out_reg"],
        "vs_load_balancing": int(js["vs_load_balancing"], 16),
        "input_count_unk8": js["input_count_unk8"],
    }
    if not vs:
        # the compiler places fragment varyings in registers 1..N; the backend
        # counts on that (the varying index is the register minus one)
        for i, e in enumerate(viv["in"]):
            if e["reg"] != i + 1:
                raise Fail("fragment input %d is in register %d, expected %d"
                           % (i, e["reg"], i + 1))

    # attributes (vertex only): the GL core's glGetAttribLocation reflection
    attribs = []
    if vs:
        for e in viv["in"]:
            # the compiler reports gl_vert_attrib for a GL shader; some sources
            # come through with the raw layout location instead, so fall back
            # to it and let the declaration below confirm which it was
            loc = e["slot"] - VERT_ATTRIB_GENERIC0
            if loc < 0 or loc not in in_types:
                loc = e["slot"]
            if loc >= MAX_ATTRIBS:
                raise Fail("vertex input slot %d is not a generic attribute" % e["slot"])
            typ = in_types.get(loc)
            if typ is None or typ not in u.GLSL_TYPES:
                raise Fail("attribute at location %d has no declaration" % loc)
            attribs.append({"location": loc, "components": e["components"],
                            "vi_reg": e["reg"], "format": 0, "name": e["name"],
                            "gl_type": u.GLSL_TYPES[typ][2]})

    # uniforms: the std430 block the GL core writes, plus the samplers
    blk, block_dw = block_uniforms(translated)
    samplers = block_samplers(translated, js)
    for un in js["uniform_names"]:
        if "slot" not in un:
            continue                   # a sampler: it has a texture unit, not a slot
        members = [b for b in blk
                   if b["name"] == un["name"] or b["name"].startswith(un["name"] + ".")]
        want = 0
        for b in members:
            b["slot"] = un["slot"] + b["slot_off"]
            want += elem_slots(b["type"]) * b["array_len"]
        if members and un["vec4s"] != want:
            raise Fail("uniform '%s': tool reports %d vec4 slots, the declaration needs %d"
                       % (un["name"], un["vec4s"], want))
    by_slot = sorted((b for b in blk if "slot" in b), key=lambda b: b["slot"])

    # the constant register file, with every uniform reference rewritten from
    # the tool's vec4-slot index space into the block the GL core writes
    contents, data = [], []
    for i, c in enumerate(js["uniforms"]):
        kind, raw = c["contents"], int(c["data"], 16)
        if kind == CONST_UNIFORM:
            var = None
            for b in by_slot:
                if b["slot"] * 4 <= raw < (b["slot"] + elem_slots(b["type"]) * b["array_len"]) * 4:
                    var = b
                    break
            if var is None:
                raise Fail("constant register %d reads uniform index %d, "
                           "which no declared uniform covers" % (i, raw))
            raw = uniform_index_to_block_dword(var, raw)
        elif kind == CONST_UBO_ADDR:
            raw = 0  # the backend writes the address of the stage's block copy
        elif kind not in (CONST_UNUSED, CONST_LITERAL) and kind not in CONST_SAMPLER_QUERY:
            raise Fail("constant register %d has unsupported contents %s"
                       % (i, CONST_NAMES.get(kind, kind)))
        contents.append(kind)
        data.append(raw)
    if CONST_UBO_ADDR in contents:
        # A run-time index is a LOAD from the stage's uniform block in memory
        # (etna_shc lowers uniforms to UBO 0, as the GL state tracker does),
        # at Mesa's offsets: the array's vec4 slot * 16 + index * 16 per
        # slot. The backend copies the GL core's block there
        # (src/vivante/backend.rs, reads_ubo), so every array must sit in
        # that block exactly where Mesa's offsets point.
        for b in by_slot:
            if b["array_len"] <= 1:
                continue
            per = elem_slots(b["type"])
            if b["dword_off"] != b["slot"] * 4 or b["stride"] != per * 4 or (per > 1 and b["align"] != 4):
                raise Fail("uniform array '%s' reads by a run-time index from block dword %d "
                           "(stride %d) where Mesa's offsets point at %d (stride %d)"
                           % (b["name"], b["dword_off"], b["stride"], b["slot"] * 4, per * 4))

    sh["attribs"] = attribs
    sh["uniforms"] = blk + samplers
    sh["uniform_dwords"] = block_dw
    sh["const_contents"] = contents
    sh["const_data"] = data
    sh["viv"] = viv
    sh["sha256"] = hashlib.sha256((sha_text if sha_text is not None else text).encode()).digest()
    sh["text"] = text
    return sh


# ---- Rust emitters ---------------------------------------------------------
#
# The shape of src/shaderc/blobs.rs, the generated PowerVR table: `static`
# arrays per shader, one ShcBlob per table row with every field written out,
# names through `cname()`, source texts as a `concat!` of one line each with a
# trailing NUL. Fields the C table left to its zero initialiser are written
# here as that zero, so the two tables hold the same bytes.

# The field widths of the C ABI, so a name that would be truncated is a
# failure here rather than a silently shortened name at run time.
SHC_UNIFORM_NAME_MAX = 64
SHC_NAME_MAX = 32


def rs_name(s, field_max, what):
    """`cname("...")`, checked against the array it is copied into: cname()
    keeps field_max - 1 bytes and a NUL, as a C string initialiser does."""
    if len(s) > field_max - 1:
        raise Fail("%s name '%s' does not fit %d bytes plus a NUL"
                   % (what, s, field_max - 1))
    return 'cname("%s")' % s


def rs_words(name, words, typ="u32", per_line=8, fmt="0x%08x"):
    out = ["static %s: [%s; %d] = [" % (name, typ, len(words))]
    for i in range(0, len(words), per_line):
        out.append("    " + " ".join((fmt + ",") % w for w in words[i:i + per_line]))
    out.append("];")
    return "\n".join(out)


def rs_string(text, ind):
    """The GLSL text as a `concat!` of one literal per line, NUL-terminated:
    what `gles_offline_shader_source` hands an application back."""
    out = []
    for line in text.splitlines(True):
        esc = line.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")
        out.append('%s"%s",' % (ind, esc))
    out.append('%s"\\0",' % ind)
    return "\n".join(out)


def rs_io(e):
    return ('ShcVivIo { reg: %d, slot: %d, components: %d, interp: %d, name: %s }'
            % (e["reg"], e["slot"], e["components"], e["interp"],
               rs_name(e["name"], SHC_NAME_MAX, "register")))


def rs_uniform(un):
    return ('ShcUniform { name: %s, dword_off: %d, dwords: %d, gl_type: 0x%04x, '
            'array_len: %d, binding: %d, inner: %d }'
            % (rs_name(un["name"], SHC_UNIFORM_NAME_MAX, "uniform"), un["dword_off"],
               un["dwords"], un["gl_type"], un["array_len"], un["binding"], un["inner"]))


def rs_attrib(a):
    return ('ShcAttrib { location: %d, components: %d, vi_reg: %d, format: %d, '
            'name: %s, gl_type: 0x%04x }'
            % (a["location"], a["components"], a["vi_reg"], a["format"],
               rs_name(a["name"], SHC_NAME_MAX, "attribute"), a["gl_type"]))


def reg(x):
    return SHC_VIV_NO_REG if x is None or x < 0 else x


def rs_padded(items, render, pad_fn, zero, ind):
    """A fixed-size ShcShader/ShcViv array: `[X::ZERO; N]` when the shader has
    none, else the generated entries through the padding helper."""
    if not items:
        return zero
    out = ["%s([" % pad_fn]
    for it in items:
        out.append("%s    %s," % (ind, render(it)))
    out.append("%s])" % ind)
    return "\n".join(out)


def emit_shader(out, sh, tag, ind):
    """The `static` arrays of one shader, then its ShcShader body as a string
    whose fields are indented by `ind`."""
    up = tag.upper()
    out.append(rs_words("CODE_%s" % up, sh["code"]))
    if sh["const_contents"]:
        out.append(rs_words("CC_%s" % up, sh["const_contents"], "u8", 16, "%u"))
        out.append(rs_words("CD_%s" % up, sh["const_data"]))
    if sh["uniforms"]:
        out.append("static UNI_%s: [ShcUniform; %d] = [" % (up, len(sh["uniforms"])))
        for un in sh["uniforms"]:
            out.append("    %s," % rs_uniform(un))
        out.append("];")

    viv = sh["viv"]
    b = []
    b.append("%susc: CODE_%s.as_ptr()," % (ind, up))
    b.append("%susc_dw: %d," % (ind, len(sh["code"])))
    b.append("%stemps: %d," % (ind, sh["temps"]))
    # PowerVR reflection: this GPU's backend reads the register wiring in
    # `viv` instead, and the C table leaves these zero.
    b.append("%svaryings: [ShcVarying::ZERO; 16]," % ind)
    b.append("%snum_varyings: 0," % ind)
    b.append("%suses_w: 0," % ind)
    b.append("%suses_discard: %d," % (ind, sh["uses_discard"]))
    b.append("%sreads_front_facing: %d," % (ind, sh["reads_front_facing"]))
    b.append("%smeta_off: 0," % ind)
    b.append("%sfs_front_face_off1: 0," % ind)
    b.append("%snum_outputs: 0," % ind)
    b.append("%snum_tile_buffers: 0," % ind)
    b.append("%stile_buffer_sh: 0," % ind)
    b.append("%sattribs: %s," % (ind, rs_padded(sh["attribs"], rs_attrib, "pad_attribs",
                                                "[ShcAttrib::ZERO; 16]", ind)))
    b.append("%snum_attribs: %d," % (ind, len(sh["attribs"])))
    b.append("%svs_output_size: 0," % ind)
    b.append("%svs_psize_off: 0," % ind)
    b.append("%svs_vertex_id_reg1: 0," % ind)
    b.append("%svs_instance_id_reg1: 0," % ind)
    b.append("%suniform_dwords: %d," % (ind, sh["uniform_dwords"]))
    b.append("%suniform_sh_dwords: 0," % ind)
    if sh["uniforms"]:
        b.append("%suniforms: UNI_%s.as_ptr()," % (ind, up))
    else:
        b.append("%suniforms: core::ptr::null()," % ind)
    b.append("%snum_uniforms: %d," % (ind, len(sh["uniforms"])))
    b.append("%sblend_key: 0," % ind)
    b.append("%suses_fbfetch: 0," % ind)
    # This GPU has no blend-constant shader variant: glBlendColor goes into
    # PE_ALPHA_BLEND_COLOR as hardware state, so no shader here reserves four
    # dwords of its uniform block for it. Saying so is not optional. Left at
    # 0, the field does not read as "none" -- it reads as "the blend constants
    # are at dword 0", and the GL core then writes glBlendColor over the first
    # four dwords of every fragment uniform block on every draw. That is what
    # made a fragment uniform never reach its shader while a vertex one always
    # did.
    b.append("%sblend_consts_off: SHC_NO_BLEND_CONSTS," % ind)
    # PowerVR fields: no GC7000 program iterates a point sprite, a Z plane
    # or carries a gl_FragCoord y pair.
    b.append("%suses_pntc: 0," % ind)
    b.append("%suses_z: 0," % ind)
    b.append("%sfragcoord_ytf_off1: 0," % ind)
    b.append("%snative_cube: 0," % ind)
    # the fields f7b8a221 (the vertex attribute layout decided at link) added
    # to ShcShader, with the values it gave every committed row
    b.append("%slayout_at_link: 0," % ind)
    b.append("%sdeclared: core::ptr::null()," % ind)
    b.append("%snum_declared: 0," % ind)
    b.append("%sviv: ShcViv {" % ind)
    vi = ind + "    "
    b.append("%sin_: %s," % (vi, rs_padded(viv["in"], rs_io, "pad_io",
                                           "[ShcVivIo::ZERO; SHC_VIV_MAX_IO]", vi)))
    b.append("%snum_in: %d," % (vi, len(viv["in"])))
    b.append("%sout: %s," % (vi, rs_padded(viv["out"], rs_io, "pad_io",
                                           "[ShcVivIo::ZERO; SHC_VIV_MAX_IO]", vi)))
    b.append("%snum_out: %d," % (vi, len(viv["out"])))
    # link() fills every entry the backend reads (one per fragment input), so
    # the table's value is never used. The C table leaves it zero-initialised;
    # keep that, so the two tables hold the same bytes.
    b.append("%svs_reg_of_in: [0; SHC_VIV_MAX_IO]," % vi)
    b.append("%svs_pos_out_reg: %d," % (vi, reg(viv["vs_pos_out_reg"])))
    b.append("%svs_pointsize_out_reg: %d," % (vi, reg(viv["vs_pointsize_out_reg"])))
    b.append("%svs_id_in_reg: %d," % (vi, reg(viv["vs_id_in_reg"])))
    b.append("%sps_color_out_reg: %d," % (vi, reg(viv["ps_color_out_reg"])))
    b.append("%sps_depth_out_reg: %d," % (vi, reg(viv["ps_depth_out_reg"])))
    b.append("%svs_load_balancing: 0x%08x," % (vi, viv["vs_load_balancing"]))
    b.append("%sinput_count_unk8: %d," % (vi, viv["input_count_unk8"]))
    if sh["const_contents"]:
        b.append("%sconst_contents: CC_%s.as_ptr()," % (vi, up))
        b.append("%sconst_data: CD_%s.as_ptr()," % (vi, up))
    else:
        b.append("%sconst_contents: core::ptr::null()," % vi)
        b.append("%sconst_data: core::ptr::null()," % vi)
    b.append("%sconst_count: %d," % (vi, len(sh["const_contents"])))
    b.append("%s}," % ind)
    b.append("%spriv_: core::ptr::null()," % ind)
    return "\n".join(b)


TABLE_HEADER = """// SPDX-License-Identifier: Apache-2.0
//
// vivante_blobs.rs - GENERATED by tools/vivante/gen_shader_table.py. DO NOT EDIT.
// Re-run the generator (see tools/vivante/README.md).
//
// The offline shader table of a Vivante GC7000 (i.MX8MP) build: one entry per
// shader under yakogl/tools/shaders, holding the GC7000 instruction words and
// the register wiring (ShcViv), found by the SHA-256 of the GLSL text an
// application passes to glShaderSource. Exactly one GPU's table is compiled
// into a build; blobs.rs is the PowerVR one.
//
// Blending and colour masks are PE state on this GPU, not shader variants, so
// every fragment shader appears once with blend_key 0. A shader that reads
// gl_FrontFacing appears a second time under the same sha256 with blend_key
// SHC_VIV_KEY_FRONT_FLIPPED: the hardware value carries no sense, so the
// comparison is compiled in and there is one binary per winding.
//"""

TABLE_PRELUDE = """#![allow(clippy::all)]
#![cfg_attr(rustfmt, rustfmt_skip)]

#[allow(unused_imports)]
use super::{
    pad_attribs, pad_io, ShcAttrib, ShcBlob, ShcShader, ShcSource, ShcUniform, ShcVarying, ShcViv,
    ShcVivIo, SHC_NO_BLEND_CONSTS, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX, SHC_VIV_MAX_IO,
};
#[allow(unused_imports)]
use crate::util::cname;

/// Pad a generated register list to the fixed array of [`ShcViv`], the way
/// `pad_varyings`/`pad_attribs` do for [`ShcShader`].
"""

CLEAR_HEADER = """// SPDX-License-Identifier: Apache-2.0
//
// clear_shader.rs - GENERATED by tools/vivante/gen_shader_table.py. DO NOT EDIT.
// Re-run the generator (see tools/vivante/README.md).
//
// The two internal shaders the Vivante backend draws its clear quad with
// (tools/shaders/internal/clear.vert and clear.frag). They are ordinary
// ShcShader objects so the backend programs them through the same code path
// as an application's shaders: the vertex shader passes the quad position
// through, the fragment shader writes one uniform vec4.

#![allow(clippy::all)]
#![cfg_attr(rustfmt, rustfmt_skip)]

#[allow(unused_imports)]
use crate::shaderc::pad_io;
#[allow(unused_imports)]
use crate::shaderc::{
    pad_attribs, pad_io, ShcAttrib, ShcShader, ShcUniform, ShcVarying, ShcViv, ShcVivIo,
    SHC_NO_BLEND_CONSTS, SHC_VIV_MAX_IO,
};
#[allow(unused_imports)]
use crate::util::cname;

/// One of the two shaders below. [`ShcShader`] holds raw pointers -- into the
/// `static` arrays of this module -- so it is not `Sync` on its own, exactly
/// as the offline table's `ShcBlob` is not; `shaderc` marks that one `Sync`
/// for the same reason this marks these.
#[repr(transparent)]
pub struct ClearShader(pub ShcShader);

// SAFETY: the pointers point at the immutable statics of this module.
unsafe impl Sync for ClearShader {}
"""


def stage_const(stage):
    return "SHC_STAGE_VERTEX" if stage == "vs" else "SHC_STAGE_FRAGMENT"


def emit_table(shaders):
    """The whole of src/shaderc/vivante_blobs.rs, as text."""
    out = [TABLE_HEADER]
    # Which toolchain built these bytes, in the same shape the PowerVR
    # generator records (`// toolchain: pco_shc=... glslang=...`). The
    # committed table predates this line and therefore carries none, which is
    # itself the fact worth having: an unlabelled table is one whose origin
    # nobody can check.
    ident = tool_identity()
    out.append("// toolchain: etna_shc=%s glslang=%s" % (ident["etna_shc"], ident["glslang"]))
    out.append("// Shaders in this table:")
    for sh in shaders:
        out.append("//   %-20s %-2s  sha256=%s"
                   % (sh["name"], sh["stage"], sh["sha256"].hex()))
    out.append(TABLE_PRELUDE)

    bodies = []
    for i, sh in enumerate(shaders):
        bodies.append(emit_shader(out, sh, str(i), " " * 12))
        out.append("")
    out.append("pub static SHC_BLOB_TABLE: [ShcBlob; %d] = [" % len(shaders))
    for sh, body in zip(shaders, bodies):
        out.append("    ShcBlob {")
        out.append("        sha256: [%s],"
                   % ", ".join("0x%02x" % b for b in sh["sha256"]))
        out.append('        name: "%s\\0",' % sh["name"])
        out.append("        stage: %s," % stage_const(sh["stage"]))
        out.append("        blend_key: %d," % sh["key"])
        out.append("        shader: ShcShader {")
        out.append(body)
        out.append("        },")
        out.append("    },")
    out.append("];")
    out.append("")
    out.append("pub static SHC_SOURCE_TABLE: [ShcSource; %d] = [" % len(shaders))
    for sh in shaders:
        out.append("    ShcSource {")
        out.append('        name: "%s",' % os.path.basename(sh["name"]))
        out.append("        text: concat!(")
        out.append(rs_string(sh["text"], " " * 12))
        out.append("        ),")
        out.append("    },")
    out.append("];")
    return "\n".join(out) + "\n"


def emit_clear(clear):
    """The whole of src/vivante/clear_shader.rs, as text."""
    out = [CLEAR_HEADER]
    for base, tag in (("clear.vert", "vs"), ("clear.frag", "ps")):
        body = emit_shader(out, clear[base], "clear_" + tag, " " * 4)
        out.append("pub static GC_CLEAR_%s: ClearShader = ClearShader(ShcShader {" % tag.upper())
        out.append(body)
        out.append("});")
        out.append("")
    return "\n".join(out)


def append_rows(old_text, shaders):
    """`old_text` (the committed table) with `shaders` added after its rows.

    Every existing row, symbol and source entry stays byte for byte: the new
    rows' arrays are numbered after the last existing one, the two tables
    grow by len(shaders), and the header lists the new shaders under a line
    naming the toolchain that compiled them. This is how a shader is added
    without a NEW REFERENCE (see main()): a host whose etna_shc cannot
    reproduce the committed rows can still compile rows of its own.
    """
    blob_part = old_text.split("pub static SHC_BLOB_TABLE", 1)[1].split("pub static SHC_SOURCE_TABLE", 1)[0]
    n_old = len(ROW_NAME_RE.findall(blob_part))
    blob_hdr = "pub static SHC_BLOB_TABLE: [ShcBlob; %d] = [" % n_old
    src_hdr = "pub static SHC_SOURCE_TABLE: [ShcSource; %d] = [" % n_old
    if old_text.count(blob_hdr) != 1 or old_text.count(src_hdr) != 1:
        raise SystemExit("gen_shader_table --append: the committed table is not one this can extend")
    defs, bodies = [], []
    for i, sh in enumerate(shaders):
        bodies.append(emit_shader(defs, sh, str(n_old + i), " " * 12))
        defs.append("")
    rows, sources = [], []
    for sh, body in zip(shaders, bodies):
        rows += ["    ShcBlob {",
                 "        sha256: [%s]," % ", ".join("0x%02x" % b for b in sh["sha256"]),
                 '        name: "%s\\0",' % sh["name"],
                 "        stage: %s," % stage_const(sh["stage"]),
                 "        blend_key: %d," % sh["key"],
                 "        shader: ShcShader {", body, "        },", "    },"]
        sources += ["    ShcSource {", '        name: "%s",' % os.path.basename(sh["name"]),
                    "        text: concat!(", rs_string(sh["text"], " " * 12), "        ),", "    },"]
    ident = tool_identity()
    listing = ["// Appended rows (--append), compiled by etna_shc=%s glslang=%s:"
               % (ident["etna_shc"], ident["glslang"])]
    listing += ["//   %-20s %-2s  sha256=%s" % (sh["name"], sh["stage"], sh["sha256"].hex()) for sh in shaders]
    # after the header's comment block (the committed table's prelude may be
    # an older generator's, so it is not looked for)
    lines = old_text.split("\n")
    end = next(i for i, line in enumerate(lines) if not line.startswith("//"))
    text = "\n".join(lines[:end] + listing + lines[end:])
    blob_at = text.index(blob_hdr)
    text = text[:blob_at] + "\n".join(defs) + "\n" + text[blob_at:]
    blob_at = text.index(blob_hdr)
    blob_end = text.index("\n];\n", blob_at)
    text = text[:blob_end] + "\n" + "\n".join(rows) + text[blob_end:]
    src_at = text.index(src_hdr)
    src_end = text.index("\n];\n", src_at)
    text = text[:src_end] + "\n" + "\n".join(sources) + text[src_end:]
    n_new = n_old + len(shaders)
    text = text.replace(blob_hdr, "pub static SHC_BLOB_TABLE: [ShcBlob; %d] = [" % n_new)
    text = text.replace(src_hdr, "pub static SHC_SOURCE_TABLE: [ShcSource; %d] = [" % n_new)
    return text


def main_append(rels):
    """`--append <path under tools/shaders>...`: compile those shaders and add
    them to the committed table (append_rows), refusing any already in it."""
    with open(OUT_TABLE) as f:
        old_text = f.read()
    shaders = []
    for rel in rels:
        p = os.path.join(SHADERS, rel)
        try:
            sh = reflect(p)
        except (Fail, SystemExit) as e:
            raise SystemExit("gen_shader_table --append: %s did not compile: %s" % (rel, e))
        if sh["stage"] == "fs" and sh["reads_front_facing"]:
            raise SystemExit("gen_shader_table --append: %s reads gl_FrontFacing (two rows); "
                             "regenerate the table instead" % rel)
        if ('name: "%s\\0"' % sh["name"]) in old_text:
            raise SystemExit("gen_shader_table --append: %s is already in the table" % rel)
        shaders.append(sh)
    new_text = append_rows(old_text, shaders)
    same, changed, added, removed = rows_that_change(old_text, new_text)
    if changed or removed or len(added) != len(shaders):
        raise SystemExit("gen_shader_table --append: the splice changed existing rows "
                         "(%d changed, %d removed, %d added)" % (len(changed), len(removed), len(added)))
    with open(OUT_TABLE, "w") as f:
        f.write(new_text)
    print("kept %d committed rows byte for byte, appended %d" % (len(same), len(added)))
    return 0


def replace_rows(old_text, shaders):
    """`old_text` with the rows of `shaders` (already in it, by name)
    recompiled in place: each row keeps its position, its symbol number and
    its source entry; its `static` arrays and its ShcShader body are the new
    compile's. Every other row stays byte for byte, and the header lists the
    replaced shaders under a line naming the toolchain that compiled them.
    This is how a fix in the toolchain reaches the rows it changes on a host
    whose etna_shc cannot reproduce the committed table (see append_rows).
    """
    text = old_text
    for sh in shaders:
        rows = [m for m in ROW_RE.finditer(text)
                if ROW_NAME_RE.search(m.group(1)) and ROW_NAME_RE.search(m.group(1)).group(1) == sh["name"]]
        if len(rows) != 1:
            raise SystemExit("gen_shader_table --replace: %s has %d rows, not one" % (sh["name"], len(rows)))
        row = rows[0]
        tag = re.search(r"usc: CODE_(\d+)\.as_ptr\(\)", row.group(1))
        if not tag:
            raise SystemExit("gen_shader_table --replace: %s has no numbered CODE array" % sh["name"])
        tag = tag.group(1)
        defs = []
        body = emit_shader(defs, sh, tag, " " * 12)
        head, sep, _ = row.group(1).partition("\n        shader: ShcShader {\n")
        if not sep:
            raise SystemExit("gen_shader_table --replace: %s: the row has no ShcShader body" % sh["name"])
        new_row = "    ShcBlob {\n" + head + sep + body + "\n        },\n    },"
        text = text[:row.start()] + new_row + text[row.end():]
        # the old arrays of that number, then the new ones where CODE was
        sym = re.compile(r"static (?:CODE|CC|CD|UNI)_%s: \[[^\]]*\] = \[.*?\n\];\n\n?" % tag, re.S)
        olds = list(sym.finditer(text))
        if not olds or not olds[0].group(0).startswith("static CODE_%s:" % tag):
            raise SystemExit("gen_shader_table --replace: %s: CODE_%s not found" % (sh["name"], tag))
        at = olds[0].start()
        for m in reversed(olds):
            text = text[:m.start()] + text[m.end():]
        text = text[:at] + "\n".join(defs) + "\n\n" + text[at:]
    ident = tool_identity()
    listing = ["// Replaced rows (--replace), compiled by etna_shc=%s glslang=%s:"
               % (ident["etna_shc"], ident["glslang"])]
    listing += ["//   %-20s %-2s  sha256=%s" % (sh["name"], sh["stage"], sh["sha256"].hex()) for sh in shaders]
    lines = text.split("\n")
    end = next(i for i, line in enumerate(lines) if not line.startswith("//"))
    return "\n".join(lines[:end] + listing + lines[end:])


def main_replace(rels):
    """`--replace <path under tools/shaders>...`: recompile those rows of the
    committed table in place (replace_rows), refusing a shader that is not
    in it, and refusing the result if any OTHER row changed."""
    with open(OUT_TABLE) as f:
        old_text = f.read()
    shaders = []
    for rel in rels:
        try:
            sh = reflect(os.path.join(SHADERS, rel))
        except (Fail, SystemExit) as e:
            raise SystemExit("gen_shader_table --replace: %s did not compile: %s" % (rel, e))
        if sh["stage"] == "fs" and sh["reads_front_facing"]:
            raise SystemExit("gen_shader_table --replace: %s reads gl_FrontFacing (two rows); "
                             "regenerate the table instead" % rel)
        if ('name: "%s\\0"' % sh["name"]) not in old_text:
            raise SystemExit("gen_shader_table --replace: %s is not in the table" % rel)
        shaders.append(sh)
    new_text = replace_rows(old_text, shaders)
    same, changed, added, removed = rows_that_change(old_text, new_text)
    want = sorted(sh["name"] for sh in shaders)
    if added or removed or sorted(changed) != want:
        raise SystemExit("gen_shader_table --replace: the splice changed other rows "
                         "(changed %s, added %s, removed %s; asked for %s)" % (changed, added, removed, want))
    with open(OUT_TABLE, "w") as f:
        f.write(new_text)
    print("kept %d committed rows byte for byte, replaced %d: %s" % (len(same), len(changed), ", ".join(changed)))
    return 0


def main():
    if "--append" in sys.argv[1:]:
        return main_append(sys.argv[sys.argv.index("--append") + 1:])
    if "--replace" in sys.argv[1:]:
        return main_replace(sys.argv[sys.argv.index("--replace") + 1:])
    for need, where in (("GLSL_TYPES", u), ("SAMPLER_TYPES", u), ("stage_of", u),
                        ("es_version", u), ("_scan_defines", u), ("_resolve_size", u),
                        ("translate_es100_gl", v), ("translate_es300_gl", v),
                        ("translate_450_gl", v), ("compile_one", v)):
        if not hasattr(where, need):
            raise SystemExit("gen_shader_table: %s.%s is missing; this generator needs the "
                             "upstream version of that module" % (where.__name__, need))

    paths = []
    found_dirs = {}
    for d, _, files in os.walk(SHADERS):
        for f in sorted(files):
            if f.endswith((".vert", ".frag")):
                paths.append(os.path.join(d, f))
                rel = os.path.relpath(d, SHADERS)
                rel = "" if rel == "." else rel
                found_dirs.setdefault(rel, []).append(f)
    paths.sort()

    # A directory this generator has never been told about is a failure, not a
    # guess. See TABLE_DIRS: everything it walks becomes rows in a table that
    # ships, and the 13 wedge probes are what happens when it does not ask.
    stray = stray_dirs(found_dirs)
    if stray:
        lines = ["gen_shader_table: %d directory(s) under tools/shaders/ that this "
                 "generator has no instruction about:" % len(stray)]
        for s in stray:
            names = found_dirs[s]
            lines.append("    %-24s %d shader(s), e.g. %s" % (s + "/", len(names), names[0]))
        lines.append("")
        lines.append("Everything walked here becomes a row in the SHIPPED table. Add the")
        lines.append("directory to TABLE_DIRS to ship its shaders, or to NOT_TABLE_DIRS if")
        lines.append("the backend compiles them itself. If it holds test material, it does")
        lines.append("not belong under tools/shaders/ at all.")
        raise SystemExit("\n".join(lines))
    internal = [p for p in paths if os.path.basename(os.path.dirname(p)) == "internal"]
    paths = [p for p in paths if p not in internal]

    shaders, failed = [], []
    for p in paths:
        try:
            sh = reflect(p)
            shaders.append(sh)
        except (Fail, SystemExit) as e:
            # glsl2vivante's ES 1.00 parser exits on a declarator it cannot
            # read (es100/light_amount_*.vert: `u_lights[0 + 16]`); that is a
            # shader left out, as README.md says, not the end of the table.
            failed.append((os.path.relpath(p, SHADERS), str(e)))
            continue
        # A fragment shader that reads gl_FrontFacing gets its second binary,
        # compiled from the source with the sense negated and keyed under the
        # ORIGINAL source's sha256, so the runtime finds it from the shader the
        # application actually compiled.
        if sh["stage"] == "fs" and sh["reads_front_facing"]:
            text = open(p).read()
            try:
                var = reflect(p, text.replace(READS_FRONT_FACING,
                                              "(!%s)" % READS_FRONT_FACING),
                              sha_text=text)
            except Fail as e:
                failed.append((os.path.relpath(p, SHADERS),
                               "front-facing variant: %s" % e))
                continue
            var["key"] = KEY_FRONT_FLIPPED
            var["name"] = sh["name"] + " @front_cw"
            shaders.append(var)
    for rel, why in failed:
        print("FAIL %-52s %s" % (rel, why))

    # THE COMMITTED TABLE IS THE REFERENCE.
    #
    # Regenerating on this host produces 339 rows and changes 308 of the 324
    # that already exist -- a different etna_shc and a different glslang, not
    # a different compiler on our side. Overwriting silently would replace the
    # reference every downstream number is measured against (the twin-draw
    # partition, the chain score, "the reference wedges at temps 40") without
    # anybody choosing to.
    #
    # So a rebuild that changes an existing row REFUSES, reports how many rows
    # it did reproduce, and says what to pass to mean it.
    new_text = emit_table(shaders)
    if os.path.exists(OUT_TABLE) and "--new-reference" not in sys.argv[1:]:
        with open(OUT_TABLE) as f:
            old_text = f.read()
        same, changed, added, removed = rows_that_change(old_text, new_text)
        total = len(same) + len(changed) + len(removed)
        if changed or removed:
            ident = tool_identity()
            print("reproduced %d of %d committed rows byte for byte" % (len(same), total))
            print("  changed %d, removed %d, added %d" % (len(changed), len(removed), len(added)))
            for n in changed[:5]:
                print("    changed: %s" % n)
            for n in removed[:5]:
                print("    removed: %s" % n)
            print("  this host's tools: etna_shc %s" % ident["etna_shc"][:16])
            print("                     %s" % ident["glslang"])
            raise SystemExit(
                "gen_shader_table: REFUSING to overwrite %s.\n"
                "A rebuild that changes committed rows is a NEW REFERENCE, not a\n"
                "regeneration: every number measured against the old table has to be\n"
                "re-derived in the same commit. Pass --new-reference to mean that, and\n"
                "record the tool identities above in the commit message."
                % os.path.relpath(OUT_TABLE, os.path.join(HERE, "..", "..")))
        print("reproduced %d of %d committed rows byte for byte%s"
              % (len(same), total, "" if not added else ", %d new" % len(added)))

    with open(OUT_TABLE, "w") as f:
        f.write(new_text)

    clear = {}
    for p in internal:
        base = os.path.basename(p)
        if base in ("clear.vert", "clear.frag"):
            clear[base] = reflect(p)
    if len(clear) == 2:
        with open(OUT_CLEAR, "w") as f:
            f.write(emit_clear(clear))
    else:
        print("note: tools/shaders/internal/clear.{vert,frag} missing; %s not written"
              % os.path.relpath(OUT_CLEAR, os.path.join(HERE, "..", "..")))

    print("%d/%d shaders in the table%s" % (len(shaders), len(shaders) + len(failed),
                                            "" if not failed else " (%d failed)" % len(failed)))
    return 1 if failed else 0


if __name__ == "__main__":
    if "--self-test" in sys.argv[1:]:
        _self_test()
        sys.exit(0)
    sys.exit(main())
