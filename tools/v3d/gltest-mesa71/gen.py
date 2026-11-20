#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""gen.py: Mesa's offline V3D 7.1 programs for gl_test cases 5, 58, 59, 66, 67, 102.

    python3 -I tools/v3d/gltest-mesa71/gen.py --shc V3D_SHC --record71 BIN \\
        --mesa-src TREE --out-dir DIR --work DIR
    python3 -I tools/v3d/gltest-mesa71/gen.py --recheck tools/v3d/gltest-mesa71 \\
        --out-dir DIR

--recheck needs no compiler: each committed header is written again from its
committed JSON into --out-dir (recheck() below), for the caller to compare.

Run by tools/v3d/check71.d/gltest_mesa71.sh (README.md here); not meant to be
run by hand, the script builds the two binaries it needs.

For each program in PROGRAMS: the GLSL ES 1.00 sources are translated and
assembled to SPIR-V as tools/v3d/glsl2v3d.py does (its translate(), then
`glslangValidator -G`), and Mesa's v3d compiler runs on the pair through
v3d_shc at the Pi 5's identity (`--device 71 --rev 10 --compat-rev 2
--vpm-size 65536`: 7.1.10.2, tools/v3d/pi5/v3d_ident.txt). Written into
--out-dir:

  <name>.json    v3d_shc's stdout as it is: the QPU words of the fragment,
                 vertex and coordinate shaders, their uniform streams and
                 prog_data (tools/v3d/README.md describes the format)
  <name>.disasm  v3d_shc's stderr (Mesa's statistics and disassembly at 7.1),
                 the SPIR-V paths replaced by <vs.spv> and <fs.spv>
  <name>.h       the same as C (code words, uniform streams with their
                 QUNIFORM_* contents) plus the shader state record Mesa emits
                 for it: record71 (Mesa's v3d_compute_vpm_config and its 7.1
                 draw-index packer, v3dx_draw.c:477-546) gives the field values
                 and the packed 32 bytes with every address 0; and the vertex
                 attribute records' read counts (v3dx_draw.c:802-847, GFXH-930)

The control program (CONTROL) is compiled the same way into --work only; the
caller compares it with tools/v3d/tri-shaders71, which was read off the board's
own Mesa dump. Stdlib only, besides glsl2v3d's imports.
"""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
sys.path.insert(0, os.path.join(REPO, "tools", "v3d"))
import glsl2v3d as g  # noqa: E402

BACKEND = os.path.join(REPO, "src", "v3d", "backend.rs")
SHC_ARGS = ["--device", "71", "--rev", "10", "--compat-rev", "2", "--vpm-size", "65536"]

# name -> (vs, fs, what draws with it). A source is ("file", path under
# tools/shaders) or ("backend", the Rust constant in src/v3d/backend.rs).
PROGRAMS = {
    "color_uv": (("file", "gl_test_es100/color_uv.vert"), ("file", "gl_test_es100/color_uv.frag"),
                 "gl_test_vs_src + gl_test_fs_src\n"
                 " * (driver/tools/pvrlab/gl_test_shaders_es100.h), the one program cases 5, 58,\n"
                 " * 59, 67 and 102 link; case 66 links none"),
    "clear": (("backend", "CLEAR_VS"), ("backend", "CLEAR_FS"),
              "the back end's clear quad (src/v3d/backend.rs CLEAR_VS + CLEAR_FS), drawn for\n"
              " * a clear the TLB cannot do: scissored (5, 58), write-masked (59, 102)"),
}
CONTROL = ("passthrough_interp_color",
           ("file", "gl_test_es100/passthrough.vert"), ("file", "gl_test_es100/interp_color.frag"))

FS_ARGS = ["threads", "single_seg", "writes_z", "disable_ez", "uses_center_w", "uses_pid",
           "lock_scoreboard_on_first_thrsw", "uses_implicit_point_line_varyings", "num_inputs"]
VS_ARGS = ["threads", "single_seg", "vpm_input_size", "vpm_output_size", "vcm_cache_size",
           "uses_vid", "uses_iid"]


def die(msg):
    sys.exit("gen.py: " + msg)


def backend_source(const):
    """The NUL-terminated byte string `pub const NAME: &[u8] = b"...";`."""
    text = open(BACKEND).read()
    m = re.search(r'pub const %s: &\[u8\] =\s*b"((?:[^"\\]|\\.)*)";' % re.escape(const), text)
    if not m:
        die("%s: no `pub const %s: &[u8] = b\"...\"`" % (BACKEND, const))
    s = m.group(1).encode("ascii").decode("unicode_escape")
    if not s.endswith("\0"):
        die("%s is not NUL-terminated" % const)
    return s[:-1]


def spirv(src, stage, work, stem):
    """Translate (glsl2v3d.translate) and assemble one source; the .spv path."""
    kind, what = src
    ext = ".vert" if stage == "vs" else ".frag"
    if kind == "file":
        base, rel = g.SHADERS, what
    else:
        base, rel = work, stem + "_src" + ext
        with open(os.path.join(work, rel), "w") as f:
            f.write(backend_source(what))
    text = g.translate(rel, base)
    tp = os.path.join(work, stem + ext)
    with open(tp, "w") as f:
        f.write(text)
    spv = tp + ".spv"
    r = subprocess.run([g.GLSLANG, "-G", tp, "-o", spv], capture_output=True, text=True)
    if r.returncode:
        die("glslang refused %s:\n%s%s" % (what, r.stdout, r.stderr))
    return spv


def compile_program(shc, name, vs, fs, work):
    """(json text, json dict, disassembly text) from v3d_shc."""
    vspv = spirv(vs, "vs", work, name + "_vs")
    fspv = spirv(fs, "fs", work, name + "_fs")
    r = subprocess.run([shc, vspv, fspv] + SHC_ARGS, capture_output=True, text=True)
    err = r.stderr.replace(vspv, "<vs.spv>").replace(fspv, "<fs.spv>")
    if r.returncode:
        die("v3d_shc refused %s (exit %d):\n%s" % (name, r.returncode, err))
    js = json.loads(r.stdout)
    gpu = js["gpu"]
    if (gpu["ver"], gpu.get("rev"), gpu.get("compat_rev"), gpu["vpm_size"]) != (71, 10, 2, 65536):
        die("%s was compiled for %r, not 7.1.10.2 with a 64 KiB VPM" % (name, gpu))
    return r.stdout, js, err


def flag(v):
    if isinstance(v, bool):
        return int(v)
    if isinstance(v, int) and v >= 0:
        return v
    die("prog_data value %r is not a count" % (v,))


def record(record71, js):
    """record71's `name value` lines as a dict (ints), in its order."""
    args = [str(flag(js["fs"][k])) for k in FS_ARGS]
    args += [str(flag(js["vs"][k])) for k in VS_ARGS]
    args += [str(flag(js["cs"][k])) for k in VS_ARGS]
    r = subprocess.run([record71] + args, capture_output=True, text=True)
    if r.returncode:
        die("record71 failed (exit %d): %s" % (r.returncode, r.stderr))
    out = {}
    for line in r.stdout.splitlines():
        k, v = line.split()
        out[k] = int(v, 0)
    return out


def attr_reads(js):
    """Per vertex attribute record i: (name, location, values read by the CS,
    by the VS), v3dx_draw.c:802-847 with one element per attribute record (the
    state tracker's vertex elements for the program's inputs), so the
    GFXH-930/GFXH-1602 dummy reads are applied as Mesa applies them."""
    names = {}
    for a in js["attributes"]:
        for rec in a["records"]:
            if rec is not None:
                names[rec] = (a["name"], a["location"])
    n = len(names)
    if sorted(names) != list(range(n)):
        die("attribute records %r are not 0..%d" % (sorted(names), n - 1))
    cs, vs = js["cs"], js["vs"]
    cs_uses_builtins = cs["uses_iid"] or cs["uses_biid"] or cs["uses_vid"]
    loaded = False
    out = []
    for i in range(n):
        csv = cs["vattr_sizes"][i]
        if csv:
            loaded = True
        if i == 0 and cs_uses_builtins and not loaded:
            csv, loaded = 1, True
        elif i == n - 1 and not loaded:
            csv, loaded = 1, True
        out.append((names[i][0], names[i][1], csv, vs["vattr_sizes"][i]))
    return out


def quniform_names(mesa_src):
    """Mesa's enum quniform_contents, by value."""
    text = open(os.path.join(mesa_src, "src", "broadcom", "compiler", "v3d_compiler.h")).read()
    m = re.search(r"enum quniform_contents \{(.*?)\n\};", text, re.S)
    if not m:
        die("no enum quniform_contents in Mesa's v3d_compiler.h")
    body = re.sub(r"/\*.*?\*/", "", m.group(1), flags=re.S)
    names = [x.strip() for x in body.split(",") if x.strip()]
    if any("=" in x for x in names):
        die("enum quniform_contents has an explicit value; this reader counts")
    return names


def header(name, why, srcs, js, rec, attrs, qnames):
    up, lo = "MESA71_" + name.upper(), "mesa71_" + name
    o = ["/* SPDX-License-Identifier: Apache-2.0 */",
         "/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */",
         "/*",
         " * GENERATED by tools/v3d/gltest-mesa71/gen.py from %s.json (Mesa 26.2.1's v3d" % name,
         " * compiler through v3d_shc at V3D 7.1.10.2:",
         " * `%s`); do not edit." % " ".join(SHC_ARGS),
         " * tools/v3d/check71.d/gltest_mesa71.sh regenerates and compares it.",
         " *",
         " * Program: %s." % why,
         " * Sources: VS %s," % srcs[0],
         " * FS %s." % srcs[1],
         " *",
         " * The shader state record is the BCM2712 D0 layout (GL Shader State Record",
         " * Draw Index), as Mesa's emit_shader_state_record() fills it",
         " * (v3dx_draw.c:477-546) for a triangle draw with no geometry shader, packed",
         " * by Mesa's 7.1 packer with the three code and three uniform addresses 0",
         " * (the draw's own addresses go into those fields).",
         " */",
         "#ifndef %s_H" % up,
         "#define %s_H" % up,
         "",
         "#include <stdint.h>",
         ""]
    used = sorted({u["contents"] for s in ("fs", "vs", "cs") for u in js[s]["uniforms"]})
    o.append("/* uniform contents: Mesa's enum quniform_contents (v3d_compiler.h) */")
    for c in used:
        if c not in (qnames if isinstance(qnames, dict) else range(len(qnames))):
            die("%s: uniform contents %d is not in Mesa's enum" % (name, c))
        o.append("#define %s_%s %du" % (up, qnames[c], c))
    o.append("")
    for st, label in (("fs", "FS"), ("vs", "VS"), ("cs", "CS")):
        s = js[st]
        code, unis = s["code"], s["uniforms"]
        o.append("/* %s: %d instructions, %d uniform words, %d threads */"
                 % (label, len(code), len(unis), s["threads"]))
        o.append("#define %s_%s_CODE_COUNT %du" % (up, label, len(code)))
        o.append("static const uint64_t %s_%s_code[%d] = {" % (lo, st, len(code)))
        o += ["\t%sull," % w for w in code]
        o.append("};")
        o.append("#define %s_%s_UNIFORM_COUNT %du" % (up, label, len(unis)))
        if unis:
            o.append("static const uint32_t %s_%s_uniform[%d] = {" % (lo, st, len(unis)))
            o += ["\t%su, /* %s */" % (u["data"], qnames[u["contents"]]) for u in unis]
            o.append("};")
            o.append("static const uint8_t %s_%s_uniform_contents[%d] = {" % (lo, st, len(unis)))
            o.append("\t" + " ".join("%d," % u["contents"] for u in unis))
            o.append("};")
        o.append("")
    if js["uniform_names"]:
        o.append("/* GL uniforms: a QUNIFORM_UNIFORM word's data is slot * 4 + component */")
        for u in js["uniform_names"]:
            o.append("/*   %s: %s stage storage, slot %d, %d vec4 */"
                     % (u["name"], u["stage"], u["slot"], u["vec4s"]))
        o.append("")
    fs = js["fs"]
    o.append("/* FS inputs (VARYING_SLOT_*, component), the order the VS writes them */")
    o.append("#define %s_FS_NUM_INPUTS %du" % (up, fs["num_inputs"]))
    o.append("/* %s */" % (", ".join("%d.%d" % (x["slot"], x["component"]) for x in fs["input_slots"])
                           or "none"))
    for k in ("flat_shade_flags", "noperspective_flags", "centroid_flags"):
        o.append("#define %s_FS_%s %su" % (up, k.upper(), fs[k]))
    o.append("")
    o.append("/* vpm_config from Mesa's v3d_compute_vpm_config: bin (CS), render (VS) */")
    for k in ("vpm_bin_As", "vpm_bin_Ve", "vpm_bin_Vc", "vpm_As", "vpm_Ve", "vpm_Vc"):
        o.append("#define %s_%s %du" % (up, k.upper(), rec[k]))
    o.append("")
    o.append("/* shader state record (D0 layout), field by field */")
    for k, v in rec.items():
        if k.startswith("vpm_") or k.startswith("word"):
            continue
        o.append("#define %s_REC_%s %du" % (up, k.upper(), v))
    o.append("/* the packed 32 bytes, little-endian words, addresses 0 */")
    o.append("static const uint32_t %s_rec[8] = {" % lo)
    o.append("\t" + " ".join("0x%08xu," % rec["word%d" % i] for i in range(8)))
    o.append("};")
    o.append("/* the 24-bit flag word: the first dword without byte 3, the varyings count")
    o.append(" * (tools/v3d/tri-shaders71's REC_FLAGS is the same word) */")
    o.append("#define %s_REC_FLAGS 0x%08xu" % (up, rec["word0"] & 0xffffff))
    o.append("")
    o.append("/* vertex attribute records (one per enabled array, in this order): values")
    o.append(" * read by the CS and by the VS, GFXH-930 applied; stride, type and address")
    o.append(" * are the draw's */")
    o.append("#define %s_ATTR_COUNT %du" % (up, len(attrs)))
    o.append("struct %s_attr { const char *name; uint8_t location, cs_values, vs_values; };" % lo)
    o.append("static const struct %s_attr %s_attr[%d] = {" % (lo, lo, len(attrs)))
    for nm, loc, csv, vsv in attrs:
        o.append('\t{ "%s", %d, %d, %d },' % (nm, loc, csv, vsv))
    o.append("};")
    o.append("")
    o.append("#endif /* %s_H */" % up)
    return "\n".join(o) + "\n"


VPM_KEYS = ("vpm_bin_As", "vpm_bin_Ve", "vpm_bin_Vc", "vpm_As", "vpm_Ve", "vpm_Vc")


def read_header(path, name):
    """From a committed header: (the record71 dict, in record71's order, and
    Mesa's quniform names by value), the values gen.py took from record71 and
    from Mesa's v3d_compiler.h."""
    up = "MESA71_" + name.upper()
    text = open(path).read()
    rec, qnames = {}, {}
    for m in re.finditer(r"^#define %s_(\w+) (0x[0-9a-f]+|\d+)u$" % up, text, re.M):
        k, v = m.group(1), int(m.group(2), 0)
        if k.startswith("QUNIFORM_"):
            qnames[v] = k
        elif k.startswith("VPM_"):
            key = [x for x in VPM_KEYS if x.upper() == k]
            if len(key) != 1:
                die("%s: unknown %s_%s" % (path, up, k))
            rec[key[0]] = v
        elif k.startswith("REC_") and k != "REC_FLAGS":
            rec[k[4:].lower()] = v
    m = re.search(r"mesa71_%s_rec\[8\] = \{\n\t(.*?)\n\};" % re.escape(name), text)
    if not m:
        die("%s: no mesa71_%s_rec[8]" % (path, name))
    words = re.findall(r"0x([0-9a-f]{8})u,", m.group(1))
    if len(words) != 8:
        die("%s: mesa71_%s_rec has %d words, not 8" % (path, name, len(words)))
    for i, w in enumerate(words):
        rec["word%d" % i] = int(w, 16)
    return rec, qnames


def derived(js, rec):
    """The record values record71 copies from the prog_data as they are
    (record71.c, v3dx_draw.c:477-546): a mismatch is a header that does not
    belong to its JSON. The values of v3d_compute_vpm_config (As, Ve) are
    checked against the VPM lines; Mesa computes them, not this."""
    fs, vs, cs = js["fs"], js["vs"], js["cs"]
    want = {
        "point_size_in_shaded_vertex_data": 0, "enable_clipping": 1,
        "fragment_shader_does_z_writes": flag(fs["writes_z"]),
        "turn_off_early_z_test": flag(fs["disable_ez"]),
        "fragment_shader_uses_real_pixel_centre_w_in_addition_to_centroid_w2": flag(fs["uses_center_w"]),
        "any_shader_reads_hardware_written_primitive_id": flag(fs["uses_pid"]),
        "insert_primitive_id_as_first_varying_to_fragment_shader": flag(fs["uses_pid"]),
        "do_scoreboard_wait_on_first_thread_switch": flag(fs["lock_scoreboard_on_first_thrsw"]),
        "disable_implicit_point_line_varyings": 1 - flag(fs["uses_implicit_point_line_varyings"]),
        "number_of_varyings_in_fragment_shader": flag(fs["num_inputs"]),
        "coordinate_shader_input_vpm_segment_size": flag(cs["vpm_input_size"]),
        "vertex_shader_input_vpm_segment_size": flag(vs["vpm_input_size"]),
        "coordinate_shader_output_vpm_segment_size": flag(cs["vpm_output_size"]),
        "vertex_shader_output_vpm_segment_size": flag(vs["vpm_output_size"]),
        "min_coord_shader_input_segments_required_in_play": rec.get("vpm_bin_As"),
        "min_vertex_shader_input_segments_required_in_play": rec.get("vpm_As"),
        "min_coord_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size":
            rec.get("vpm_bin_Ve"),
        "min_vertex_shader_output_segments_required_in_play_in_addition_to_vcm_cache_size":
            rec.get("vpm_Ve"),
        "coordinate_shader_4_way_threadable": int(cs["threads"] == 4),
        "vertex_shader_4_way_threadable": int(vs["threads"] == 4),
        "fragment_shader_4_way_threadable": int(fs["threads"] == 4),
        "coordinate_shader_start_in_final_thread_section": flag(cs["single_seg"]),
        "vertex_shader_start_in_final_thread_section": flag(vs["single_seg"]),
        "fragment_shader_start_in_final_thread_section": flag(fs["single_seg"]),
        "vertex_id_read_by_coordinate_shader": flag(cs["uses_vid"]),
        "instance_id_read_by_coordinate_shader": flag(cs["uses_iid"]),
        "vertex_id_read_by_vertex_shader": flag(vs["uses_vid"]),
        "instance_id_read_by_vertex_shader": flag(vs["uses_iid"]),
    }
    return len(want), ["%s: %r, the prog_data gives %r" % (k, rec.get(k), v)
                       for k, v in want.items() if rec.get(k) != v]


def recheck(committed, out_dir):
    """--recheck: no compiler. Each committed header is written again into
    out_dir from its committed JSON, with the record71 values and quniform
    names read back from the header itself; the caller compares the bytes.
    So every line that comes from the JSON (code, uniform streams, FS inputs,
    attribute read counts with GFXH-930) and every comment is checked, and the
    record values that copy prog_data are checked against it. Returns the
    number of programs that failed."""
    failed = 0
    for name, (vs, fs, why) in PROGRAMS.items():
        js = json.load(open(os.path.join(committed, name + ".json")))
        gpu = js["gpu"]
        if (gpu["ver"], gpu.get("rev"), gpu.get("compat_rev"), gpu["vpm_size"]) != (71, 10, 2, 65536):
            print("FAIL: %s.json was compiled for %r, not 7.1.10.2 with a 64 KiB VPM" % (name, gpu))
            failed += 1
            continue
        rec, qnames = read_header(os.path.join(committed, name + ".h"), name)
        with open(os.path.join(out_dir, name + ".h"), "w") as f:
            f.write(header(name, why, (describe(vs), describe(fs)), js, rec, attr_reads(js), qnames))
        n, bad = derived(js, rec)
        for b in bad:
            print("FAIL: %s.h: %s" % (name, b))
        if bad:
            failed += 1
        else:
            print("gen.py: %s: header rewritten from %s.json, %d record values equal the prog_data"
                  % (name, name, n))
    return failed


def describe(src):
    kind, what = src
    return "tools/shaders/" + what if kind == "file" else "src/v3d/backend.rs " + what


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--recheck", metavar="COMMITTED")
    ap.add_argument("--shc")
    ap.add_argument("--record71")
    ap.add_argument("--mesa-src")
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--work")
    a = ap.parse_args()
    if a.recheck:
        os.makedirs(a.out_dir, exist_ok=True)
        sys.exit(1 if recheck(a.recheck, a.out_dir) else 0)
    if not (a.shc and a.record71 and a.mesa_src and a.work):
        ap.error("--shc, --record71, --mesa-src and --work are needed without --recheck")
    os.makedirs(a.out_dir, exist_ok=True)
    os.makedirs(a.work, exist_ok=True)
    qnames = quniform_names(a.mesa_src)
    for name, (vs, fs, why) in PROGRAMS.items():
        text, js, dis = compile_program(a.shc, name, vs, fs, a.work)
        rec = record(a.record71, js)
        with open(os.path.join(a.out_dir, name + ".json"), "w") as f:
            f.write(text)
        with open(os.path.join(a.out_dir, name + ".disasm"), "w") as f:
            f.write(dis)
        with open(os.path.join(a.out_dir, name + ".h"), "w") as f:
            f.write(header(name, why, (describe(vs), describe(fs)), js, rec, attr_reads(js), qnames))
        print("gen.py: %s: fs %d, vs %d, cs %d instructions, record flags 0x%08x"
              % (name, len(js["fs"]["code"]), len(js["vs"]["code"]), len(js["cs"]["code"]), rec["word0"]))
    name, vs, fs = CONTROL
    text, js, _ = compile_program(a.shc, name, vs, fs, a.work)
    rec = record(a.record71, js)
    with open(os.path.join(a.work, name + ".json"), "w") as f:
        f.write(text)
    with open(os.path.join(a.work, name + ".record"), "w") as f:
        f.write("".join("%s %d\n" % kv for kv in rec.items()))
    print("gen.py: control %s compiled into %s" % (name, a.work))


if __name__ == "__main__":
    main()
