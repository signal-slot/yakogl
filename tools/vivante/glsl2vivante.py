#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""GLSL ES 1.00 or 3.00 -> GLSL 4.60 (OpenGL SPIR-V) -> etna_shc, for every shader
under yakogl/tools/shaders. Prints one line per shader: ok (instructions, temps)
or the failure. The 4.60 rewrite mirrors glsl2usc's, with GL semantics instead
of Vulkan's: attributes and varyings get locations, samplers get bindings, other
uniforms get explicit locations (what glslangValidator -G requires)."""
import os, re, subprocess, sys, json, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import es100 as u   # noqa: E402  (the ES 1.00 parser pieces of the former glsl2usc.py)

ETNA_SHC = os.environ.get("ETNA_SHC", os.path.expanduser(
    "~/pvr-lab/mesa-build/mesa-26.2.1/build-etnaviv/src/gallium/drivers/etnaviv/etna_shc"))
GLSLANG = os.environ.get("GLSLANG", "glslangValidator")


def translate_es100_gl(text, stage):
    src = u.strip_comments(text)
    defines = u._scan_defines(text)
    decl = re.compile(r"^\s*(invariant\s+)?(attribute|varying|uniform)\s+"
                      r"(?:(?:highp|mediump|lowp)\s+)?(\w+)\s+(.*?)\s*;\s*$", re.S)
    out = []
    attrib_loc = varying_loc = binding = uniform_loc = 0
    attrib_names = []
    uses_fragcolor = re.search(r"\bgl_FragColor\b", src) is not None
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
        invariant, storage, typ, declarators = m.group(1), m.group(2), m.group(3), m.group(4)
        for d in [x.strip() for x in declarators.split(",")]:
            dm = re.match(r"(\w+)\s*(\[\s*(\w+)\s*\])?$", d)
            if not dm:
                raise SystemExit("cannot parse declarator '%s'" % d)
            name, array = dm.group(1), dm.group(2) or ""
            count = u._resolve_size(dm.group(3), defines) if dm.group(3) else 1
            if storage == "attribute":
                out.append("layout(location = %d) in %s %s;\n" % (attrib_loc, typ, name))
                attrib_loc += u.ES100_LOCATIONS[typ] * count
                attrib_names.append(name)
            elif storage == "varying":
                direction = "out" if stage == "vs" else "in"
                inv = "invariant " if (invariant and stage == "vs") else ""
                out.append("%slayout(location = %d) %s %s %s%s;\n"
                           % (inv, varying_loc, direction, typ, name, array))
                varying_loc += u.ES100_LOCATIONS[typ] * count
            elif typ in u.SAMPLER_TYPES:
                out.append("layout(binding = %d) uniform %s %s;\n" % (binding, typ, name))
                binding += 1
            else:
                out.append("layout(location = %d) uniform %s %s%s;\n" % (uniform_loc, typ, name, array))
                uniform_loc += u.ES100_LOCATIONS.get(typ, 1) * count
    header = ["#version 460\n"]
    if stage == "fs" and uses_fragcolor:
        header.append("layout(location = 0) out vec4 yk_FragColor;\n#define gl_FragColor yk_FragColor\n")
    header.append("#define texture2D texture\n#define texture2DProj textureProj\n"
                  "#define texture2DLod textureLod\n#define texture2DProjLod textureProjLod\n")
    body = "".join(out)
    if stage == "vs":
        body = u._hoist_gl_position(body, attrib_names)
    return "".join(header) + body


def translate_es300_gl(text, stage):
    """GLSL ES 3.00 -> GLSL 4.60 (OpenGL SPIR-V).

    These sources are already written the modern way -- `in` and `out` rather
    than attribute and varying, `texture()` rather than `texture2D()`, and
    often with a layout() of their own -- so the rewrite is small: raise the
    version to one glslang will emit OpenGL SPIR-V for, and give a location or
    a binding to every interface declaration that has not got one, which is
    what that SPIR-V flavour insists on.

    Locations need not agree between the two stages: the link matches varyings
    by name, which is what GLSL ES means by a varying. Within a stage they do
    have to be unique, so the ones written by hand are read first and stepped
    over.
    """
    src = u.strip_comments(text)
    defines = u._scan_defines(text)
    decl = re.compile(r"^\s*(?:(flat|smooth|noperspective|centroid)\s+)*"
                      r"(in|out|uniform)\s+"
                      r"(?:(?:highp|mediump|lowp)\s+)?(\w+)\s+(.*?)\s*;\s*$", re.S)
    taken = {"in": set(), "out": set(), "uniform": set(), "binding": set()}
    for m in re.finditer(r"layout\s*\(([^)]*)\)\s*(?:(?:flat|smooth|noperspective|centroid)\s+)*"
                         r"(in|out|uniform)\b", src):
        what, storage = m.group(1), m.group(2)
        loc = re.search(r"location\s*=\s*(\d+)", what)
        bind = re.search(r"binding\s*=\s*(\d+)", what)
        if loc:
            taken[storage].add(int(loc.group(1)))
        if bind:
            taken["binding"].add(int(bind.group(1)))

    def take(kind, n):
        """The first run of n free slots of this kind."""
        i = 0
        while any((i + k) in taken[kind] for k in range(n)):
            i += 1
        for k in range(n):
            taken[kind].add(i + k)
        return i

    out = []
    for top, kind, chunk in u.es100_statements(src):
        if kind == "pp":
            if re.match(r"\s*#\s*(version|extension)\b", chunk):
                continue
            out.append(chunk)
            continue
        m = decl.match(chunk) if (top and kind == "stmt") else None
        if not m or re.match(r"\s*layout\s*\(", chunk):
            out.append(chunk)
            continue
        qualifier, storage, typ, declarators = m.group(1), m.group(2), m.group(3), m.group(4)
        pre = (qualifier + " ") if qualifier else ""
        for d in [x.strip() for x in declarators.split(",")]:
            dm = re.match(r"(\w+)\s*(\[\s*(\w+)\s*\])?$", d)
            if not dm:
                raise SystemExit("cannot parse declarator '%s'" % d)
            name, array = dm.group(1), dm.group(2) or ""
            count = u._resolve_size(dm.group(3), defines) if dm.group(3) else 1
            if storage == "uniform" and typ in u.SAMPLER_TYPES:
                out.append("layout(binding = %d) uniform %s %s;\n"
                           % (take("binding", 1), typ, name))
            else:
                slots = u.ES100_LOCATIONS.get(typ, 1) * count
                out.append("%slayout(location = %d) %s %s %s%s;\n"
                           % (pre, take(storage, slots), storage, typ, name, array))
    return "#version 460\n" + "".join(out)


def translate_450_gl(text):
    """The GLSL 450 sources are written for the PowerVR tool (Vulkan SPIR-V): the
    push-constant block becomes plain located uniforms, sampler set/binding pairs
    become bindings."""
    def block(m):
        loc, out = 0, []
        for d in re.findall(r"(\w+)\s+(\w+)\s*;", m.group(1)):
            out.append("layout(location = %d) uniform %s %s;" % (loc, d[0], d[1]))
            loc += u.ES100_LOCATIONS.get(d[0], 1)
        return "\n".join(out)
    text = re.sub(r"layout\s*\(\s*push_constant\s*\)\s*uniform\s+\w+\s*\{(.*?)\}\s*;", block, text, flags=re.S)
    return re.sub(r"layout\s*\(\s*set\s*=\s*\d+\s*,\s*binding", "layout(binding", text)


def compile_one(path, text=None):
    """Compile one shader. `text` overrides the file's contents, which is how
    a variant is built: the caller rewrites the source and the path is then
    only used to decide the stage."""
    stage = u.stage_of(path)
    if text is None:
        text = open(path).read()
    version = u.es_version(text)
    if version == 100:
        text = translate_es100_gl(text, stage)
    elif version in (300, 310):
        text = translate_es300_gl(text, stage)
    else:
        text = translate_450_gl(text)
    with tempfile.TemporaryDirectory() as td:
        ext = ".vert" if stage == "vs" else ".frag"
        src = os.path.join(td, "s" + ext)
        spv = os.path.join(td, "s.spv")
        open(src, "w").write(text)
        r = subprocess.run([GLSLANG, "-G", src, "-o", spv], capture_output=True, text=True)
        if r.returncode != 0:
            return None, "glslang: " + r.stdout.strip().splitlines()[-1][:120]
        r = subprocess.run([ETNA_SHC, "vert" if stage == "vs" else "frag", spv],
                           capture_output=True, text=True)
        if r.returncode != 0:
            return None, "etna_shc: " + (r.stderr.strip().splitlines() or ["?"])[-1][:120]
        js = json.loads(r.stdout[:r.stdout.index("\n}\n") + 3])
        return js, None


def main():
    root = os.path.join(HERE, "..", "shaders")
    paths = []
    for d, _, fs in os.walk(root):
        for f in sorted(fs):
            if f.endswith((".vert", ".frag")) and not f.endswith(".nodiscard.frag"):
                paths.append(os.path.join(d, f))
    ok = 0
    for p in sorted(paths):
        js, err = compile_one(p)
        rel = os.path.relpath(p, root)
        if js:
            ok += 1
            print("ok   %-48s %3u instr, %2u temps, %u uniforms" % (rel, js["code_size"] // 4,
                  js["num_temps"], len(js["uniforms"])))
        else:
            print("FAIL %-48s %s" % (rel, err))
    print("%d/%d compiled" % (ok, len(paths)))


if __name__ == "__main__":
    main()
