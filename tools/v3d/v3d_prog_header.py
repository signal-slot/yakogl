#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Turn one v3d_shc JSON (tools/v3d/README.md) into a C header that
driver/src/v3d/v3d_frame.h's v3d_frame_build_triangle() can take.

    v3d_prog_header.py prog.json out.h --name tri [--assume fs.uses_pid=false ...]

The header holds the QPU code of the three programs (FS, VS, CS) as uint64_t
arrays, their uniform streams as struct v3d_uniform arrays, and one function
per stage that fills struct v3d_fs_prog / v3d_vs_prog with the code's GPU
address, which only the caller knows.

Every prog_data field the frame builder reads must be present in the JSON. A
field that is absent is an error, not a default: the only way past it is an
explicit --assume stage.key=value, and every assumption is written into the
header as <NAME>_ASSUMED so the program that uses it can print it. A value
the frame builder cannot use (a spill, a points draw, a varying flag beyond
word 0) is refused here, before any C is written.
"""
import argparse
import hashlib
import json
import sys

# The fields v3d_frame_build_triangle reads, per stage (v3d_frame.h).
FS_KEYS = ["threads", "single_seg", "spill_size", "num_inputs", "flat_shade_flags",
           "noperspective_flags", "centroid_flags", "writes_z", "disable_ez",
           "uses_center_w", "lock_scoreboard_on_first_thrsw",
           "uses_implicit_point_line_varyings", "uses_pid", "force_per_sample_msaa",
           "tmu_dirty_rcl", "uniforms", "code"]
VS_KEYS = ["threads", "single_seg", "spill_size", "vpm_input_size", "vpm_output_size",
           "separate_segments", "vcm_cache_size", "uses_vid", "uses_iid", "uses_biid",
           "writes_psiz", "tmu_dirty_rcl", "vattr_sizes", "uniforms", "code"]
# quniform_contents values v3d_frame.c writes (v3d_frame.h V3D_QUNIFORM_*)
SUPPORTED_CONTENTS = {0: "CONSTANT", 1: "UNIFORM", 2: "VIEWPORT_X_SCALE", 3: "VIEWPORT_Y_SCALE",
                      4: "VIEWPORT_Z_OFFSET", 5: "VIEWPORT_Z_SCALE",
                      # V3D-private: gl_FragCoord.y's window transform
                      # (v3d_frame.h V3D_QUNIFORM_FB_WPOS_Y_*, from f.wpos)
                      0x10000: "FB_WPOS_Y_SCALE", 0x10001: "FB_WPOS_Y_OFFSET"}


def die(msg):
    sys.exit("v3d_prog_header: " + msg)


def parse_assume(items):
    out = {}
    for it in items:
        if "=" not in it or "." not in it.split("=", 1)[0]:
            die("--assume wants stage.key=value, got %r" % it)
        k, v = it.split("=", 1)
        try:
            out[k] = json.loads(v)
        except json.JSONDecodeError:
            die("--assume %s: %r is not a JSON value" % (k, v))
    return out


def get(prog, stage, key, assume, used):
    name = stage + "." + key
    if key in prog[stage]:
        if name in assume:
            die("%s is in the JSON (%r); an --assume for it would be ignored, drop it"
                % (name, prog[stage][key]))
        return prog[stage][key]
    if name in assume:
        used.append("%s=%s" % (name, json.dumps(assume[name])))
        return assume[name]
    die("%s is absent from the JSON; it is not defaulted. Pass --assume %s=<value> "
        "only with the evidence for that value" % (name, name))


def cbool(v, name):
    if not isinstance(v, bool):
        die("%s must be a bool, got %r" % (name, v))
    return "true" if v else "false"


def cuint(v, name):
    if isinstance(v, str):
        v = int(v, 0)
    if not isinstance(v, int) or isinstance(v, bool) or v < 0 or v >= 1 << 32:
        die("%s must be a 32-bit unsigned value, got %r" % (name, v))
    return v


def uniform_places(prog):
    """Where each uniform sits in its stage's storage, in 32-bit words, as
    (stage, name, word, words) with stage "vert" or "frag". QUNIFORM_UNIFORM's
    data is a word index into that storage (v3d_frame.h). The two compilers
    describe it differently, and each record says which it is:

      YakoGL's (chain.rs): "word" and "words", the IR's dword offset and size.
      Mesa's (v3d_shc.c): "slot" and "vec4s", a vec4 slot; its streams read
      word slot * 4 + component (tests/v3d_sim_mesa.rs env(), stream()).

    A sampler ("binding") or a state uniform ("state") has no value the sample
    can supply, so it is refused here."""
    un = prog.get("uniform_names")
    if un is None:
        die("uniform_names is absent")
    out = []
    for i, e in enumerate(un):
        where = "uniform_names[%d] (%s)" % (i, e.get("name"))
        if e.get("stage") not in ("vert", "frag"):
            die("%s: stage %r is not vert or frag" % (where, e.get("stage")))
        if "binding" in e or "state" in e:
            die("%s: a sampler or state uniform; the frame builder supplies neither" % where)
        if "word" in e and "slot" not in e:
            word, words = cuint(e["word"], where + ".word"), cuint(e["words"], where + ".words")
        elif "slot" in e and "word" not in e:
            word, words = 4 * cuint(e["slot"], where + ".slot"), 4 * cuint(e["vec4s"], where + ".vec4s")
        else:
            die("%s: wants exactly one of word (YakoGL) and slot (Mesa)" % where)
        if not e["name"].isidentifier():
            die("%s: the name is not a C identifier" % where)
        out.append((e["stage"], e["name"], word, words))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("json")
    ap.add_argument("out")
    ap.add_argument("--name", required=True, help="C identifier prefix")
    ap.add_argument("--assume", action="append", default=[])
    a = ap.parse_args()
    if not a.name.isidentifier():
        die("--name must be a C identifier")
    with open(a.json, "rb") as fp:
        raw = fp.read()
    prog = json.loads(raw)
    digest = hashlib.sha256(raw).hexdigest()
    assume = parse_assume(a.assume)
    used = []
    N = a.name
    U = N.upper()

    gpu = prog.get("gpu")
    if not gpu or gpu.get("ver") != 42:
        die("gpu.ver is not 42")
    # Which compiler made the code. YakoGL's own writes "yakogl-online"
    # (src/shaderc/online/v3d/chain.rs); tools/v3d/v3d_shc writes no key at
    # all, and the header says so rather than filling in a name.
    compiler = gpu.get("compiler")
    if compiler is None:
        compiler_text = "absent from the JSON (v3d_shc, Mesa, writes no gpu.compiler)"
    elif compiler == "yakogl-online":
        compiler_text = compiler
    else:
        die("gpu.compiler %r is not one this script knows" % (compiler,))
    if gpu.get("prim") != "triangles":
        die("gpu.prim must be present and \"triangles\" (the FS key is per primitive)")
    if gpu.get("per_vertex_point_size") is not False:
        die("gpu.per_vertex_point_size must be present and false for a triangle draw")
    for s in ("fs", "vs", "cs"):
        if s not in prog:
            die("stage %s is absent" % s)

    fs = {k: get(prog, "fs", k, assume, used) for k in FS_KEYS}
    vs = {k: get(prog, "vs", k, assume, used) for k in VS_KEYS}
    cs = {k: get(prog, "cs", k, assume, used) for k in VS_KEYS}
    for k in assume:
        if not any(u.startswith(k + "=") for u in used):
            die("--assume %s names no field the frame builder reads" % k)

    # Read so that an absent key is an error, then checked against what M5
    # writes (Mesa 26.2.1 gallium v3d): the record's point-size bit is false
    # for a triangle draw (v3dx_draw.c:556-558), sample-rate shading is only
    # set with MSAA, which M5 has none of (:567-568), and tmu_dirty_rcl (a
    # TMU write: an image or SSBO store) makes Mesa ask the kernel for a TMU
    # cache flush after the job (v3d_job.c:694) and flush on a memory barrier
    # (v3d_context.c:109-112); M5 does not implement either, so it refuses.
    cbool(fs["force_per_sample_msaa"], "fs.force_per_sample_msaa")
    for s, d in (("vs", vs), ("cs", cs)):
        if cbool(d["writes_psiz"], s + ".writes_psiz") == "true":
            die("%s.writes_psiz is true: a triangle draw has no point size in the VPM" % s)
    for s, d in (("fs", fs), ("vs", vs), ("cs", cs)):
        if cbool(d["tmu_dirty_rcl"], s + ".tmu_dirty_rcl") == "true":
            die("%s.tmu_dirty_rcl is true: M5 does not handle a shader that writes "
                "through the TMU" % s)
    for s, d in (("fs", fs), ("vs", vs), ("cs", cs)):
        if cuint(d["spill_size"], s + ".spill_size"):
            die("%s spills (spill_size %s): the spill uniforms are not written" %
                (s, d["spill_size"]))
        if not d["code"]:
            die("%s has no code" % s)
        for i, u in enumerate(d["uniforms"]):
            c = u["contents"]
            if c not in SUPPORTED_CONTENTS:
                die("%s uniform %d: quniform_contents %d is not written by the frame "
                    "builder" % (s, i, c))
    # Every QUNIFORM_UNIFORM word a stream reads lies in a uniform of that
    # stage's storage (the VS and the CS read the vertex shader's), so the
    # program that fills the storage by name fills every word read.
    places = uniform_places(prog)
    storage_words = {}
    for st in ("vert", "frag"):
        storage_words[st] = max([w + n for (s2, _, w, n) in places if s2 == st], default=0)
    for s, d, st in (("fs", fs, "frag"), ("vs", vs, "vert"), ("cs", cs, "vert")):
        for i, u in enumerate(d["uniforms"]):
            if u["contents"] != 1:
                continue
            x = cuint(u["data"], "%s uniform %d data" % (s, i))
            if not any(s2 == st and w <= x < w + n for (s2, _, w, n) in places):
                die("%s uniform %d reads %s storage word %d, which no uniform_names "
                    "entry covers" % (s, i, st, x))
    uses_storage = any(u["contents"] == 1 for d in (fs, vs, cs) for u in d["uniforms"])
    if cuint(fs["num_inputs"], "fs.num_inputs") > 24:
        die("fs.num_inputs > 24: the JSON prints word 0 of the varying flags only")
    for k in ("flat_shade_flags", "noperspective_flags", "centroid_flags"):
        if cuint(fs[k], "fs." + k):
            die("fs.%s is not zero: M5 writes the ZERO_ALL_* packets only" % k)
    for s, d in (("vs", vs), ("cs", cs)):
        if len(d["vattr_sizes"]) != 16:
            die("%s.vattr_sizes has %d entries, expected 16" % (s, len(d["vattr_sizes"])))

    attrs = prog.get("attributes")
    if attrs is None:
        die("attributes is absent")
    rec_used = set()
    for at in attrs:
        if len(at["records"]) != 1:
            die("attribute %s spans %d slots; M5 feeds single-slot attributes only"
                % (at["name"], len(at["records"])))
        r = at["records"][0]
        if r is not None:
            rec_used.add(r)
    nrec = max(rec_used) + 1 if rec_used else 0
    if rec_used != set(range(nrec)):
        die("attribute records %s are not 0..n-1" % sorted(rec_used))

    o = []
    w = o.append
    w("/* SPDX-License-Identifier: Apache-2.0 */")
    w("/* Generated by tools/v3d/v3d_prog_header.py from %s" % a.json.split("/")[-1])
    w(" * (sha256 %s). Do not edit. */" % digest)
    w("#ifndef %s_PROG_H" % U)
    w("#define %s_PROG_H" % U)
    w("")
    w("#include <stddef.h>")
    w("#include <stdint.h>")
    w('#include "v3d_frame.h"')
    w("")
    w("#define %s_JSON_SHA256 \"%s\"" % (U, digest))
    w("/* gpu.compiler: which compiler made this code. */")
    w("#define %s_COMPILER \"%s\"" % (U, compiler_text))
    w("/* Values that were absent from the JSON and given with --assume. */")
    w("#define %s_ASSUMED \"%s\"" % (U, ", ".join(used)))
    w("#define %s_VPM_SIZE %d" % (U, cuint(gpu["vpm_size"], "gpu.vpm_size")))
    w("#define %s_VPM_SIZE_SOURCE \"%s\"" % (U, gpu["vpm_size_source"]))
    w("#define %s_NUM_ATTRS %d" % (U, nrec))
    w("/* QUNIFORM_UNIFORM: whether any stream reads uniform storage, the words")
    w(" * each stage's storage spans, and where each uniform sits in it. */")
    w("#define %s_USES_STORAGE %d" % (U, 1 if uses_storage else 0))
    w("#define %s_VS_STORAGE_WORDS %d" % (U, storage_words["vert"]))
    w("#define %s_FS_STORAGE_WORDS %d" % (U, storage_words["frag"]))
    w("#define %s_NUM_UNIFORM_PLACES %d" % (U, len(places)))
    w("struct %s_uniform_place {" % N)
    w("\tint fs; /* 1: the fragment shader's storage, 0: the vertex shader's */")
    w("\tconst char *name;")
    w("\tuint32_t word, words;")
    w("};")
    if places:
        w("static const struct %s_uniform_place %s_uniform_places[%d] = {" % (N, N, len(places)))
        for (st, name, word, words) in places:
            w("\t{ %d, \"%s\", %d, %d }," % (1 if st == "frag" else 0, name, word, words))
        w("};")
    for at in attrs:
        if at["records"][0] is None:
            w("/* attribute %s (location %d) is not read */" % (at["name"], at["location"]))
            continue
        w("#define %s_ATTR_%s_RECORD %d /* location %d */" %
          (U, at["name"].upper(), at["records"][0], at["location"]))
    w("")
    for s, d in (("fs", fs), ("vs", vs), ("cs", cs)):
        w("static const uint64_t %s_%s_code[%d] __attribute__((aligned(8))) = {" %
          (N, s, len(d["code"])))
        for c in d["code"]:
            w("\t0x%016xull," % int(c, 16))
        w("};")
        if d["uniforms"]:
            w("static const struct v3d_uniform %s_%s_uniforms[%d] = {" %
              (N, s, len(d["uniforms"])))
            for u in d["uniforms"]:
                w("\t{ %d, 0x%08xu }, /* %s */" % (u["contents"], cuint(u["data"], "data"),
                                                  SUPPORTED_CONTENTS[u["contents"]]))
            w("};")
        w("")

    def uni(s, d):
        if d["uniforms"]:
            return "%s_%s_uniforms" % (N, s), len(d["uniforms"])
        return "NULL", 0

    un, nu = uni("fs", fs)
    w("static inline void %s_fs_prog(struct v3d_fs_prog *p, uint32_t code)" % N)
    w("{")
    w("\t*p = (struct v3d_fs_prog){")
    w("\t\t.code = code,")
    w("\t\t.threads = %d," % cuint(fs["threads"], "fs.threads"))
    w("\t\t.single_seg = %s," % cbool(fs["single_seg"], "fs.single_seg"))
    w("\t\t.num_inputs = %d," % cuint(fs["num_inputs"], "fs.num_inputs"))
    for k in ("flat_shade_flags", "noperspective_flags", "centroid_flags"):
        w("\t\t.%s = 0x%08xu," % (k, cuint(fs[k], "fs." + k)))
    for k in ("writes_z", "disable_ez", "uses_center_w", "lock_scoreboard_on_first_thrsw",
              "uses_implicit_point_line_varyings", "uses_pid"):
        w("\t\t.%s = %s," % (k, cbool(fs[k], "fs." + k)))
    w("\t\t.uniforms = %s," % un)
    w("\t\t.num_uniforms = %d," % nu)
    w("\t};")
    w("}")
    w("")
    for s, d in (("vs", vs), ("cs", cs)):
        un, nu = uni(s, d)
        w("static inline void %s_%s_prog(struct v3d_vs_prog *p, uint32_t code)" % (N, s))
        w("{")
        w("\t*p = (struct v3d_vs_prog){")
        w("\t\t.code = code,")
        w("\t\t.threads = %d," % cuint(d["threads"], s + ".threads"))
        w("\t\t.single_seg = %s," % cbool(d["single_seg"], s + ".single_seg"))
        for k in ("vpm_input_size", "vpm_output_size", "vcm_cache_size"):
            w("\t\t.%s = %d," % (k, cuint(d[k], s + "." + k)))
        for k in ("separate_segments", "uses_vid", "uses_iid", "uses_biid"):
            w("\t\t.%s = %s," % (k, cbool(d[k], s + "." + k)))
        w("\t\t.vattr_sizes = { %s }," % ", ".join(str(cuint(v, s + ".vattr_sizes"))
                                                   for v in d["vattr_sizes"]))
        w("\t\t.uniforms = %s," % un)
        w("\t\t.num_uniforms = %d," % nu)
        w("\t};")
        w("}")
        w("")
    w("#endif")
    with open(a.out, "w") as fp:
        fp.write("\n".join(o) + "\n")


if __name__ == "__main__":
    main()
