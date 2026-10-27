#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Run pan_shc on one GL program written in GLSL ES 1.00.

    tools/mali/glsl2pan.py vs.vert fs.frag [--gpu-id HEX] [--gpu-variant N]
                           [--prim triangles|lines|points] [--keep DIR]

pan_shc (tools/mali/README-pan_shc.md) reads OpenGL SPIR-V, as v3d_shc does.
glslangValidator emits OpenGL SPIR-V (-G) only for desktop GLSL or for GLSL
ES 3.10 and later, so each stage is rewritten first, the way
tools/vivante/glsl2vivante.py rewrites it for etna_shc and v3d_shc: attributes
and varyings become `in`/`out` with locations in declaration order, samplers
get bindings, other uniforms get explicit locations. Three things differ from
glsl2vivante, and all three are about keeping what Mesa's GL driver sees for the
ES 1.00 source:

  - The target is `#version 310 es`, not `#version 460`. A desktop profile
    keeps the precision qualifiers as text only, so glslang emits no
    RelaxedPrecision (checked with glslang 16.4.0: `precision mediump float`
    in a 460 FS gives no decoration, in a 310 es FS one per mediump value),
    and spirv_to_nir would see every FS value as highp. Panfrost lowers
    mediump varyings and outputs to 16 bits (bifrost_nir.c:912-916,
    nir_lower_mediump_io), so the precision changes the program.
  - gl_FragColor and gl_FragData[0] become `mediump vec4` outputs: GLSL ES
    1.00 section 7.2 declares both mediump.
  - gl_Position is not moved to the front of main() (glsl2vivante does that
    for the PowerVR tool's register order, es100._hoist_gl_position).

Attribute locations are the declaration order, which is what a program gets
when the application binds them in that order (glBindAttribLocation); pan_shc
prints the location next to each attribute's record.

stdout and stderr are pan_shc's. glslangValidator is GLSLANG (default
glslangValidator on PATH), pan_shc is PAN_SHC.
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "vivante"))
import es100 as u  # noqa: E402  (the ES 1.00 statement splitter glsl2vivante uses)

GLSLANG = os.environ.get("GLSLANG", "glslangValidator")


def translate_es100(text, stage):
    """GLSL ES 1.00 -> GLSL ES 3.10 with explicit locations (see the module text)."""
    src = u.strip_comments(text)
    defines = u._scan_defines(text)
    decl = re.compile(r"^\s*(invariant\s+)?(attribute|varying|uniform)\s+"
                      r"((?:highp|mediump|lowp)\s+)?(\w+)\s+(.*?)\s*;\s*$", re.S)
    out = []
    attrib_loc = varying_loc = binding = uniform_loc = 0
    for top, kind, chunk in u.es100_statements(src):
        if kind == "pp":
            if re.match(r"\s*#\s*(version|extension)\b", chunk):
                continue
            out.append(chunk)
            continue
        m = decl.match(chunk) if (top and kind == "stmt") else None
        if not m:
            out.append(chunk)
            continue
        invariant, storage, prec, typ, declarators = m.groups()
        prec = prec or ""
        for d in [x.strip() for x in declarators.split(",")]:
            dm = re.match(r"(\w+)\s*(\[\s*([\w\s+]+?)\s*\])?$", d)
            if not dm:
                raise SystemExit("glsl2pan: cannot parse declarator '%s'" % d)
            name, array = dm.group(1), dm.group(2) or ""
            count = u._resolve_size(dm.group(3), defines) if dm.group(3) else 1
            if storage == "attribute":
                out.append("layout(location = %d) in %s%s %s;\n" % (attrib_loc, prec, typ, name))
                attrib_loc += u.ES100_LOCATIONS[typ] * count
            elif storage == "varying":
                direction = "out" if stage == "vs" else "in"
                inv = "invariant " if (invariant and stage == "vs") else ""
                out.append("%slayout(location = %d) %s %s%s %s%s;\n"
                           % (inv, varying_loc, direction, prec, typ, name, array))
                varying_loc += u.ES100_LOCATIONS[typ] * count
            elif typ in u.SAMPLER_TYPES:
                out.append("layout(binding = %d) uniform %s%s %s%s;\n"
                           % (binding, prec, typ, name, array))
                binding += count
            else:
                out.append("layout(location = %d) uniform %s%s %s%s;\n"
                           % (uniform_loc, prec, typ, name, array))
                uniform_loc += u.ES100_LOCATIONS.get(typ, 1) * count
    header = ["#version 310 es\n"]
    body = "".join(out)
    if stage == "fs" and re.search(r"\bgl_FragColor\b", body):
        header.append("layout(location = 0) out mediump vec4 yk_FragColor;\n"
                      "#define gl_FragColor yk_FragColor\n")
    if stage == "fs" and re.search(r"\bgl_FragData\b", body):
        if re.search(r"\bgl_FragData\s*\[\s*[1-9]", body):
            raise SystemExit("glsl2pan: gl_FragData[n] with n > 0 is not ES 2.0")
        body = re.sub(r"\bgl_FragData\s*\[\s*0\s*\]", "yk_FragData0", body)
        header.append("layout(location = 0) out mediump vec4 yk_FragData0;\n")
    header.append("#define texture2D texture\n#define texture2DProj textureProj\n"
                  "#define texture2DLod textureLod\n#define texture2DProjLod textureProjLod\n"
                  "#define textureCube texture\n#define textureCubeLod textureLod\n")
    return "".join(header) + body


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("vs")
    ap.add_argument("fs")
    ap.add_argument("--gpu-id")
    ap.add_argument("--gpu-variant")
    ap.add_argument("--prim")
    ap.add_argument("--keep", help="write the 3.10 sources and the SPIR-V into this directory")
    a = ap.parse_args()
    pan_shc = os.environ.get("PAN_SHC")
    if not pan_shc:
        raise SystemExit("glsl2pan: set PAN_SHC to the pan_shc binary (tools/mali/build_pan_shc.sh)")
    with tempfile.TemporaryDirectory() as td:
        d = a.keep or td
        os.makedirs(d, exist_ok=True)
        spv = []
        for path, stage, ext in ((a.vs, "vs", ".vert"), (a.fs, "fs", ".frag")):
            text = open(path).read()
            if u.es_version(text) != 100:
                raise SystemExit("glsl2pan: %s is not GLSL ES 1.00" % path)
            src = os.path.join(d, stage + ext)
            out = os.path.join(d, stage + ".spv")
            with open(src, "w") as f:
                f.write(translate_es100(text, stage))
            r = subprocess.run([GLSLANG, "-G", src, "-o", out], capture_output=True, text=True)
            if r.returncode != 0:
                sys.stderr.write(r.stdout + r.stderr)
                raise SystemExit("glsl2pan: glslang failed on %s (rewritten as %s)" % (path, src))
            spv.append(out)
        cmd = [pan_shc] + spv
        for opt in ("gpu_id", "gpu_variant", "prim"):
            v = getattr(a, opt)
            if v is not None:
                cmd += ["--" + opt.replace("_", "-"), v]
        sys.exit(subprocess.run(cmd).returncode)


if __name__ == "__main__":
    main()
