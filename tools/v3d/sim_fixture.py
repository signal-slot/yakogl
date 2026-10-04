#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Write tests/fixtures/v3d_mesa_programs.txt: Mesa's V3D 4.2 code for the
REAL program pairs of the shader corpus, for the QPU interpreter's validation
(tests/v3d_sim_mesa.rs, src/shaderc/online/v3d/sim.rs).

"Real" is glsl2v3d.py's pairing rules 1 and 2 (the programs gl_test.c links,
and the <name>.vert + <name>.frag convention); rule 3's varying-cover
fallback is left out, so every pair is a program someone wrote. Plus
femtovg's (Slint's renderer): femtovg links every fragment shader with its
one vertex shader, femtovg/main.vert (the only .vert under
tools/shaders/femtovg/), and Mesa compiles all 29 such pairs. A pair is
kept when v3d_shc compiles it.

Per pair the fixture holds what the test needs to drive Mesa's code and to
line it up with the IR evaluator:
  - vs_path, fs_path: the source paths under tools/shaders (the test compiles these
    with our front end for eval.rs);
  - spirv_io: the SPIR-V Location of every user in/out variable of both
    stages, by name (Mesa's VARYING_SLOT_VAR0 + Location is the FS input
    slot; our IR names its varyings by the same source names);
  - fs, vs, cs: Mesa's code words and uniform stream, and the prog_data the
    test reads (input_slots and the flat/noperspective/centroid flags;
    vattr_sizes);
  - attributes, uniform_names: as v3d_shc prints them.

The format is line-based, so the Rust test parses it without a JSON crate
(the library has no dev-dependencies). `#` lines are the header (build and
counts, pinned by the test); then per pair:

  real <vs_path> <fs_path>                             (every real pair, Mesa's or not)
  pair <vs_path> <fs_path>
  io <vs|fs> <in|out> <name> <location>
  attr <name> <location> <record>[,<record>...]      (- for an unread slot)
  uniform <frag|vert> <name> <slot=N|binding=N> <vec4s> [<state>]
  <fs|vs|cs> meta <key>=<value> ...                   (flags as hex)
  fs input <slot> <component>                          (in LDVARY order)
  <fs|vs|cs> vattr <n>,<n>,...
  <fs|vs|cs> code <hex> <hex> ...
  <fs|vs|cs> uniforms <contents>:<hex> ...
  end

Usage: sim_fixture.py [--out FILE]
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, "..", ".."))
DEFAULT_OUT = os.path.join(REPO, "tests", "fixtures", "v3d_mesa_programs.txt")
sys.path.insert(0, HERE)
import glsl2v3d as g  # noqa: E402

VPM_SIZE = 65536
FEMTOVG_VS = "femtovg/main.vert"
FS_KEYS = ("threads", "num_inputs", "input_slots", "flat_shade_flags", "noperspective_flags",
           "centroid_flags", "writes_z", "uses_discard", "uses_center_w")
VS_KEYS = ("threads", "vattr_sizes", "vpm_output_size", "uses_vid", "uses_iid")


def spirv_locations(spv):
    """{"in": {name: location}, "out": {name: location}} of the user
    (non-BuiltIn) Input/Output variables."""
    r = subprocess.run([g.SPIRV_DIS, "--raw-id", spv], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit("sim_fixture: spirv-dis failed on %s: %s" % (spv, r.stderr))
    names, loc, builtin, sc = {}, {}, set(), {}
    for line in r.stdout.splitlines():
        m = re.match(r'\s*OpName %(\d+) "([^"]*)"', line)
        if m:
            names[m.group(1)] = m.group(2)
        m = re.match(r"\s*OpDecorate %(\d+) Location (\d+)", line)
        if m:
            loc[m.group(1)] = int(m.group(2))
        m = re.match(r"\s*OpDecorate %(\d+) BuiltIn", line)
        if m:
            builtin.add(m.group(1))
        m = re.match(r"\s*%(\d+) = OpVariable %\d+ (Input|Output)", line)
        if m:
            sc[m.group(1)] = m.group(2)
    out = {"in": {}, "out": {}}
    for vid, cls in sc.items():
        if vid in builtin or vid not in loc:
            continue
        out["in" if cls == "Input" else "out"][names.get(vid, "%" + vid)] = loc[vid]
    return out


def build_id():
    try:
        return subprocess.run(["git", "-C", REPO, "describe", "--always", "--dirty"],
                              capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=DEFAULT_OUT)
    args = ap.parse_args()
    if not os.path.isfile(g.V3D_SHC):
        raise SystemExit("sim_fixture: %s not found (set V3D_SHC)" % g.V3D_SHC)

    files = g.corpus()
    fset = set(files)
    pairs, unresolved = g.gl_test_pairs()
    if unresolved:
        raise SystemExit("sim_fixture: gl_test pairing could not resolve: %s" % unresolved)
    real = {(vs, fs) for vs, fs, _ in pairs + g.name_pairs(files) if vs in fset and fs in fset}
    # femtovg: main.vert with every fragment shader (see the module notes).
    if FEMTOVG_VS not in fset:
        raise SystemExit("sim_fixture: %s is not in the corpus" % FEMTOVG_VS)
    femtovg = {(FEMTOVG_VS, f) for f in files if f.startswith("femtovg/") and f.endswith(".frag")}
    if not femtovg:
        raise SystemExit("sim_fixture: no femtovg fragment shader in the corpus")
    real = sorted(real | femtovg)

    work = tempfile.mkdtemp(prefix="sim_fixture-", dir=os.environ.get("SIM_FIXTURE_TMP"))
    recs = {}
    out, failed = [], []
    for vs, fs in real:
        for f in (vs, fs):
            if f not in recs:
                recs[f] = g.prepare(f, work)
        rv, rf = recs[vs], recs[fs]
        if rv["spv"] is None or rf["spv"] is None:
            failed.append((vs, fs, "glslang: %s" % (rv["error"] or rf["error"]).splitlines()[0]))
            continue
        # The VPM size of the V3D 4.2 on the Raspberry Pi 4, measured on the
        # board 2026-09-24 (IDENT1.VPM_SIZE: 65536 bytes); v3d_shc otherwise
        # assumes 16384 and its JSON says so.
        r = subprocess.run([g.V3D_SHC, rv["spv"], rf["spv"], "--vpm-size", str(VPM_SIZE)],
                           capture_output=True, text=True)
        if r.returncode != 0:
            failed.append((vs, fs, "v3d_shc exit %d" % r.returncode))
            continue
        js = json.loads(r.stdout)
        rec = {"vs_path": vs, "fs_path": fs,
               "spirv_io": {"vs": spirv_locations(rv["spv"]), "fs": spirv_locations(rf["spv"])},
               "attributes": js["attributes"], "uniform_names": js["uniform_names"]}
        for st, keys in (("fs", FS_KEYS), ("vs", VS_KEYS), ("cs", VS_KEYS)):
            s = js[st]
            rec[st] = {k: s[k] for k in keys if k in s}
            rec[st]["code"] = s["code"]
            rec[st]["uniforms"] = [[u["contents"], u["data"]] for u in s["uniforms"]]
        out.append(rec)

    reasons = {}
    for vs, fs, why in failed:
        reasons.setdefault(why, []).append("%s + %s" % (vs, fs))
    for why, ps in sorted(reasons.items(), key=lambda kv: -len(kv[1])):
        print("not compiled: %d x %s (e.g. %s)" % (len(ps), why, ps[0]))
    lines = ["# V3D 4.2 Mesa programs for the QPU interpreter's validation; written by",
             "# tools/v3d/sim_fixture.py, read by tests/v3d_sim_mesa.rs.",
             "# build: %s" % build_id(),
             "# v3d_shc: %s" % os.path.basename(g.V3D_SHC),
             "# vpm size: %d" % VPM_SIZE,
             "# real pairs: %d" % len(real),
             "# compiled: %d" % len(out),
             "# not compiled: %d" % len(failed)]
    # Every real pair, compiled by Mesa or not: the test also runs OUR
    # compiler on them (tests/v3d_sim_mesa.rs, the three-way check).
    for vs, fs in real:
        lines.append("real %s %s" % (vs, fs))
    for r in out:
        lines.append("pair %s %s" % (r["vs_path"], r["fs_path"]))
        for st in ("vs", "fs"):
            for d in ("in", "out"):
                for name, loc in sorted(r["spirv_io"][st][d].items()):
                    lines.append("io %s %s %s %d" % (st, d, name, loc))
        for a in r["attributes"]:
            lines.append("attr %s %d %s" % (a["name"], a["location"],
                                            ",".join("-" if x is None else str(x) for x in a["records"])))
        for u in r["uniform_names"]:
            # A sampler has its binding (the texture index its code carries
            # in TMU config P0) instead of a slot; the slot is the stage's own
            # (driver_location per stage).
            where = ("slot=%d" % u["slot"]) if "slot" in u else ("binding=%d" % u["binding"])
            lines.append("uniform %s %s %s %d%s" % (u["stage"], u["name"], where, u["vec4s"],
                                                    (" " + u["state"]) if u.get("state") else ""))
        for st in ("fs", "vs", "cs"):
            s_ = r[st]
            meta = " ".join("%s=%s" % (k, s_[k]) for k in sorted(s_)
                            if k not in ("code", "uniforms", "input_slots", "vattr_sizes"))
            lines.append("%s meta %s" % (st, meta))
            for sl in s_.get("input_slots", []):
                lines.append("%s input %d %d" % (st, sl["slot"], sl["component"]))
            if "vattr_sizes" in s_:
                lines.append("%s vattr %s" % (st, ",".join(str(x) for x in s_["vattr_sizes"])))
            lines.append("%s code %s" % (st, " ".join(s_["code"])))
            lines.append("%s uniforms %s" % (st, " ".join("%d:%s" % (c, d) for c, d in s_["uniforms"])))
        lines.append("end")
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    tmp = args.out + ".tmp"
    with open(tmp, "w") as f:
        f.write("\n".join(lines) + "\n")
    if not any((r["vs_path"], r["fs_path"]) == g.KNOWN_POSITIVE for r in out):
        raise SystemExit("sim_fixture: FAIL: known positive %s + %s is not in %s" % (g.KNOWN_POSITIVE + (tmp,)))
    os.replace(tmp, args.out)
    print("sim_fixture: %d real pairs, %d compiled, %d not -> %s" % (len(real), len(out), len(failed), args.out))


if __name__ == "__main__":
    main()
