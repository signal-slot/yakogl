#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# es100_ref71.sh - Mesa's V3D 7.1 output for every es100 program, as a table.
#
# Usage: tools/v3d/es100_ref71.sh [--out FILE]
#   V3D_SHC  the v3d_shc binary (tools/v3d/README.md, H2: Mesa 26.2.1's build-v3d
#            or drm_shim_ref.sh build); needed. Without it the script exits 3
#            and says `gap: V3D_SHC`.
#   --out    where to write (default tools/v3d/es100_counts_71.tsv).
#
# The program set is every .vert/.frag under tools/shaders/es100, the es100
# directory tests/v3d_online_corpus.rs walks as part of tools/shaders (one row
# per source, sorted by path). v3d_shc takes a program, a VS and an FS, and the VS
# is keyed by the FS's inputs; a source alone is not a program. Each source is
# therefore paired with a stub of the other stage made here: a stub VS writes every
# varying the FS declares from a uniform vec4 (not a constant, which Mesa's
# nir_opt_varyings would remove), a stub FS reads every varying the VS declares
# and adds them into gl_FragColor. Only the source's own stage is recorded.
# Sources are translated and assembled as tools/v3d/glsl2v3d.py does
# (glsl2vivante's translate_es100_gl, glslangValidator -G), then run at
# `v3d_shc --device 71` (rev 7, compat_rev 0: what Mesa's drm-shim reports).
#
# Columns: path, stage (vs|fs), compiled (yes|no), Mesa's reason ("ok" when
# compiled, else the failing step and its message), instructions, threads and
# uniform count of the stage (the length of v3d_shc's `uniforms`; "-" when not
# compiled). Needs glslangValidator and python3.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
OUT="$HERE/es100_counts_71.tsv"
while [ $# -gt 0 ]; do
  case "$1" in
  --out) OUT="$2"; shift 2 ;;
  *) sed -n '5,10p' "$0" >&2; exit 2 ;;
  esac
done

if [ -z "${V3D_SHC-}" ] || [ ! -x "${V3D_SHC-}" ]; then
  echo "gap: V3D_SHC (set it to Mesa 26.2.1's v3d_shc, tools/v3d/README.md H2)" >&2
  exit 3
fi

exec python3 -I - "$REPO" "$OUT" <<'PY'
import json, os, re, subprocess, sys, tempfile

repo, out_path = sys.argv[1], sys.argv[2]
sys.path.insert(0, os.path.join(repo, "tools", "v3d"))
sys.path.insert(0, os.path.join(repo, "tools", "vivante"))
import glsl2v3d as g  # noqa: E402

shc = os.environ["V3D_SHC"]
root = os.path.join(g.SHADERS, "es100")
paths = sorted(os.path.relpath(os.path.join(d, f), g.SHADERS)
               for d, _, fs in os.walk(root) for f in fs if f.endswith((".vert", ".frag")))
work = tempfile.mkdtemp(prefix="es100_ref71-", dir=os.environ.get("TMPDIR"))

VARYING = re.compile(r"\bvarying\s+(?:(?:highp|mediump|lowp)\s+)?(float|vec[234]|mat[234])\s+([^;]+);")
NCOMP = {"float": 1, "vec2": 2, "vec3": 3, "vec4": 4, "mat2": 4, "mat3": 9, "mat4": 16}


def varyings(text):
    """(type, name, array size or 0) of the source's varying declarations."""
    out = []
    for m in VARYING.finditer(g.u.strip_comments(text)):
        for d in m.group(2).split(","):
            n = re.match(r"\s*(\w+)\s*(?:\[\s*(\d+)\s*\])?\s*$", d)
            if not n:
                sys.exit("es100_ref71: cannot read varying `%s`" % d.strip())
            out.append((m.group(1), n.group(1), int(n.group(2) or 0)))
    return out


def value(ty, src):
    """An expression of type `ty` taken from the uniform vec4 `src`."""
    if ty == "float":
        return src + ".x"
    if ty.startswith("vec"):
        return "%s(%s)" % (ty, src) if ty == "vec4" else "%s.%s" % (src, "xyzw"[:int(ty[3])])
    n = int(ty[3])
    return "%s(%s)" % (ty, ", ".join([src + ".x"] * (n * n)))


def stub_vs(vs_list):
    s = ["#version 100", "precision highp float;", "attribute vec4 a_pos;", "uniform vec4 u_stub;"]
    body = ["    gl_Position = a_pos;"]
    for ty, name, arr in vs_list:
        s.append("varying %s %s%s;" % (ty, name, "[%d]" % arr if arr else ""))
        for i in range(arr or 1):
            body.append("    %s%s = %s;" % (name, "[%d]" % i if arr else "", value(ty, "u_stub")))
    return "\n".join(s + ["void main()", "{"] + body + ["}", ""])


def stub_fs(fs_list):
    s = ["#version 100", "precision highp float;"]
    body = ["    vec4 c = vec4(0.0);"]
    for ty, name, arr in fs_list:
        s.append("varying %s %s%s;" % (ty, name, "[%d]" % arr if arr else ""))
        for i in range(arr or 1):
            e = name + ("[%d]" % i if arr else "")
            if ty.startswith("mat"):
                n = int(ty[3])
                e = " + ".join("%s[%d].x" % (e, k) for k in range(n))
                body.append("    c.x += %s;" % e)
            else:
                body.append("    c += vec4(%s);" % e if ty == "float" else
                            "    c += vec4(%s%s);" % (e, ", 0.0" * (4 - int(ty[3])) if ty != "vec4" else ""))
    return "\n".join(s + ["void main()", "{"] + body + ["    gl_FragColor = c;", "}", ""])


def assemble(name, text, stage):
    """Translate (as glsl2v3d does) and run glslang -G. (spv, None) or (None, why)."""
    ext = ".vert" if stage == "vs" else ".frag"
    src = os.path.join(work, name + ext)
    spv = os.path.join(work, name + ".spv")
    try:
        tr = g.gv.translate_es100_gl(text, stage)
    except SystemExit as e:
        return None, "translate: %s" % e
    open(src, "w").write(tr)
    r = subprocess.run([g.GLSLANG, "-G", src, "-o", spv], capture_output=True, text=True)
    if r.returncode:
        tag = "<%s>" % stage
        msg = (r.stdout + r.stderr).replace(src, tag)
        lines = [l.strip() for l in msg.splitlines() if l.strip() and l.strip() != tag]
        return None, "glslang: " + " / ".join(lines)
    return spv, None


def one(i, rel):
    stage = g.u.stage_of(rel)
    text = open(os.path.join(g.SHADERS, rel)).read()
    spv, why = assemble("s%d" % i, text, stage)
    if spv is None:
        return (rel, stage, "no", why, "-", "-", "-")
    # the translated source's varyings, read from the original text (es100 syntax)
    vl = varyings(text)
    pspv, pwhy = assemble("p%d" % i, stub_fs(vl) if stage == "vs" else stub_vs(vl),
                          "fs" if stage == "vs" else "vs")
    if pspv is None:
        sys.exit("es100_ref71: the stub partner of %s does not assemble: %s" % (rel, pwhy))
    vs, fs = (spv, pspv) if stage == "vs" else (pspv, spv)
    r = subprocess.run([shc, vs, fs, "--device", "71"], capture_output=True, text=True)
    if r.returncode:
        err = [l for l in r.stderr.splitlines() if l.strip() and not g.DISASM.match(l)]
        err = " / ".join(l.strip().replace(vs, "<vs.spv>").replace(fs, "<fs.spv>") for l in err)
        if r.returncode < 0:
            err = "v3d_shc killed by signal %d" % -r.returncode
        return (rel, stage, "no", err or "v3d_shc exit %d" % r.returncode, "-", "-", "-")
    js = json.loads(r.stdout)
    if js["gpu"]["ver"] != 71:
        sys.exit("es100_ref71: %s was compiled for ver %s" % (rel, js["gpu"]["ver"]))
    s = js[stage]
    return (rel, stage, "yes", "ok", str(len(s["code"])), str(s["threads"]), str(len(s["uniforms"])))


rows = [one(i, p) for i, p in enumerate(paths)]
n_yes = sum(r[2] == "yes" for r in rows)
with open(out_path, "w") as f:
    f.write("# Mesa 26.2.1 v3d_shc --device 71 (rev 7, compat_rev 0) over every .vert/.frag of\n")
    f.write("# tools/shaders/es100, one row per source (tools/v3d/es100_ref71.sh). The other\n")
    f.write("# stage of the program is a stub made by the script; its numbers are not here.\n")
    f.write("# programs: %d, compiled: %d, refused: %d\n" % (len(rows), n_yes, len(rows) - n_yes))
    f.write("# columns: path, stage, compiled, reason, instructions, threads, uniforms\n")
    for r in rows:
        f.write("\t".join(r).replace("\n", " / ") + "\n")
print("es100_ref71: %d programs, %d compiled at 7.1, %d refused -> %s"
      % (len(rows), n_yes, len(rows) - n_yes, out_path))
PY
