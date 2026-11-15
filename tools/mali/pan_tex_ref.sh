#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pan_tex_ref.sh - Mesa 26.2.1's references for two dEQP-GLES2 groups on the
# Mali-G610: the vertex-shader cube look-ups of
# texture.vertex.cube.{filtering,wrap}.* (cause 7 of plan #373) and the cube
# look-up with a bias of texture.mipmap.cube.bias.linear_{linear,nearest}
# (cause 8). It rewrites tools/mali/testdata/pan_tex_ref/ (or <out dir>)
# byte for byte from the inputs below.
#
# Usage:
#   PAN_SHC=<pan_shc> tools/mali/pan_tex_ref.sh <drm_shim_ref work dir> [<out dir>]
#
# PAN_SHC defaults to $MESA_PAN_TREE/build-pan/src/panfrost/pan_shc/pan_shc
# (tools/mali/build_pan_shc.sh). <drm_shim_ref work dir> is a Mesa 26.2.1 build
# of tools/mali/drm_shim_ref.sh's build form (its inst/ holds the drm-shim,
# libEGL and libGLESv2).
#
# Inputs:
#   - the four shaders in the out dir, verbatim from VK-GL-CTS at
#     93bca01861b0, the CTS_COMMIT of tools/cts/mali-deqp-build.sh (the
#     board's deqp-gles2); the lines cited below are that commit's:
#       vcube.vert, vcube.frag: VertexCubeTextureCase::init,
#         modules/gles2/functional/es2fVertexTextureTests.cpp:858-875;
#       cubebias.vert, cubebias.frag: ProgramLibrary::getProgram,
#         modules/glshared/glsTextureTestUtil.cpp:96-117 for
#         PROGRAM_CUBE_FLOAT_BIAS (:390-391) at GLSL ES 1.00 and highp (the
#         TextureRenderer of TextureCubeMipmapCase,
#         es2fTextureMipmapTests.cpp:460-462, takes highp where the fragment
#         stage has it, as Mesa's does);
#   - the GL state of each case, from the same files (the CASES lists below):
#       vertex cube: a GL_RGB / GL_UNSIGNED_BYTE cube of 32x32 with 6 levels
#         (es2fVertexTextureTests.cpp:903-916), the case's min and mag filters
#         and wrap modes (:1130-1133, :1235-1251), u_lod of the first cell
#         (log2(32 / 28), :982-990);
#       cube bias: a GL_RGBA / GL_UNSIGNED_BYTE cube of 64x64 with 7 levels
#         (es2fTextureMipmapTests.cpp:481-483, :1196-1219), the case's min
#         filter, mag GL_NEAREST, GL_CLAMP_TO_EDGE (:567, :593-597), u_bias of
#         the first cell (1.0, :606, :634), u_colorScale 1 and u_colorBias 0
#         (the renderer's defaults).
#
# Outputs (in the out dir):
#   <prog>.pan_shc.json, <prog>.pan_shc.disasm: pan_shc on the pair
#     (glsl2pan.py; the build line reads "(not recorded)" for the checkout's
#     git describe, so the file does not depend on it): the FAU, uniform,
#     sysval and varying layout of the program;
#   <prog>.expect.json: the shim driver's programs of the first case's draw
#     (pan_ref_diff.py extract --draw: the words, the disassembly with each TEX
#     instruction, the registers, preload and FAU words). Every other case of
#     the group draws the same words; the script fails if one does not;
#   <prog>.pan_shc-vs-shim.txt: pan_ref_diff.py diff --expect of the two;
#   <prog>.<case>.desc: the sampler and texture descriptors of that case's
#     draw, as pandecode decodes them, per resource table (Position, Varying,
#     Fragment), with the table entry they sit at. The vertex cube reads the
#     Position and Varying tables' entries, the cube bias the Fragment
#     table's; each table also holds the other stage's unused sampler. The
#     GPU addresses are the shim's (the same on every run of one build;
#     compare fields, not addresses).
#
# The draws run each case in its own process on the drm-shim with the
# environment of drm_shim_ref.sh run (GPU_ID 0xa8670000, the board's revision
# is 5; the descriptors and words do not depend on it). The texture uploads
# are GPU blits with draws of their own (48 before a vertex cube draw, 54
# before a cube bias draw), so the case's draw is the trace's last one. PAN_TEX_REF_TRACES=<dir> keeps each case's whole trace there.
# python3 and glslangValidator as glsl2pan.py needs them.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
usage() { sed -n '12,18p' "$0" >&2; exit 2; }
[ $# -ge 1 ] && [ $# -le 2 ] || usage
WORK="$(cd "$1" && pwd)"
OUT="$(cd "${2:-$HERE/testdata/pan_tex_ref}" && pwd)"
PAN_SHC="${PAN_SHC:-${MESA_PAN_TREE:+$MESA_PAN_TREE/build-pan/src/panfrost/pan_shc/pan_shc}}"
[ -n "$PAN_SHC" ] && [ -x "$PAN_SHC" ] ||
  { echo "pan_tex_ref: set PAN_SHC (or MESA_PAN_TREE) to a pan_shc build" >&2; exit 1; }
export PAN_SHC
INST=$WORK/inst/lib64
[ -d "$INST" ] || INST=$WORK/inst/lib/$(gcc -dumpmachine)
[ -f "$INST/libpanfrost_noop_drm_shim.so" ] ||
  { echo "pan_tex_ref: $WORK has no drm_shim_ref.sh build" >&2; exit 1; }
for f in vcube.vert vcube.frag cubebias.vert cubebias.frag; do
  [ -f "$OUT/$f" ] || { echo "pan_tex_ref: $OUT has no $f" >&2; exit 1; }
done
TMP="$(mktemp -d "${TMPDIR:-/tmp}/pan_tex_ref.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

# One draw of a case: the program, the cube and its sampler state, as the CTS
# case sets them, then glFlush (the trace holds the draw's descriptors).
cat > "$TMP/draw.py" <<'EOF'
import ctypes as C
import sys

libdir, vs_path, fs_path, size, levels, fmt, minf, magf, wrap_s, wrap_t, uni = sys.argv[1:]
GL = {"NEAREST": 0x2600, "LINEAR": 0x2601, "NEAREST_MIPMAP_NEAREST": 0x2700,
      "LINEAR_MIPMAP_NEAREST": 0x2701, "NEAREST_MIPMAP_LINEAR": 0x2702,
      "LINEAR_MIPMAP_LINEAR": 0x2703, "CLAMP_TO_EDGE": 0x812F, "REPEAT": 0x2901,
      "MIRRORED_REPEAT": 0x8370, "RGB": 0x1907, "RGBA": 0x1908}
egl = C.CDLL(libdir + "/libEGL.so.1")
gl = C.CDLL(libdir + "/libGLESv2.so.2")
egl.eglGetProcAddress.restype = C.c_void_p
egl.eglGetProcAddress.argtypes = [C.c_char_p]
gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
dpy = gpd(0x31DD, None, None)
egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
if not dpy or not egl.eglInitialize(dpy, None, None):
    raise SystemExit("draw: eglInitialize failed")
egl.eglBindAPI(0x30A0)
egl.eglCreateContext.restype = C.c_void_p
egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
ctx = egl.eglCreateContext(dpy, None, None, (C.c_int * 3)(0x3098, 2, 0x3038))
egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
    raise SystemExit("draw: no GLES 2 context")
# an RGBA8 framebuffer object of 256x256 (the CTS draws into its window)
rt, fbo = C.c_uint(), C.c_uint()
gl.glGenTextures(1, C.byref(rt))
gl.glBindTexture(0x0DE1, rt)
gl.glTexImage2D(0x0DE1, 0, 0x1908, 256, 256, 0, 0x1908, 0x1401, None)
gl.glGenFramebuffers(1, C.byref(fbo))
gl.glBindFramebuffer(0x8D40, fbo)
gl.glFramebufferTexture2D(0x8D40, 0x8CE0, 0x0DE1, rt, 0)
if gl.glCheckFramebufferStatus(0x8D40) != 0x8CD5:
    raise SystemExit("draw: framebuffer incomplete")


def shader(kind, path):
    sh = gl.glCreateShader(kind)
    p = C.c_char_p(open(path, "rb").read())
    gl.glShaderSource(sh, 1, C.byref(p), None)
    gl.glCompileShader(sh)
    ok = C.c_int()
    gl.glGetShaderiv(sh, 0x8B81, C.byref(ok))
    if not ok.value:
        raise SystemExit("draw: %s does not compile" % path)
    return sh


prog = gl.glCreateProgram()
gl.glAttachShader(prog, shader(0x8B31, vs_path))
gl.glAttachShader(prog, shader(0x8B30, fs_path))
gl.glBindAttribLocation(prog, 0, b"a_position")
gl.glBindAttribLocation(prog, 1, b"a_texCoord")
gl.glLinkProgram(prog)
ok = C.c_int()
gl.glGetProgramiv(prog, 0x8B82, C.byref(ok))
if not ok.value:
    raise SystemExit("draw: link failed")
gl.glUseProgram(prog)
gl.glGetUniformLocation.argtypes = [C.c_uint, C.c_char_p]
# the cube at unit 0, every face and level (zeros: the texels change no
# descriptor), then the case's sampler state
size, levels, fmt = int(size), int(levels), GL[fmt]
tex = C.c_uint()
gl.glActiveTexture(0x84C0)
gl.glGenTextures(1, C.byref(tex))
gl.glBindTexture(0x8513, tex)
gl.glPixelStorei(0x0CF5, 1)
for level in range(levels):
    n = max(size >> level, 1)
    texels = (C.c_ubyte * (4 * n * n))()
    for face in range(6):
        gl.glTexImage2D(0x8515 + face, level, fmt, n, n, 0, fmt, 0x1401, texels)
for pname, value in ((0x2802, wrap_s), (0x2803, wrap_t), (0x2801, minf), (0x2800, magf)):
    gl.glTexParameteri(0x8513, pname, GL[value])
for item in uni.split(";"):
    name, vals = item.split("=")
    vals = [float(v) for v in vals.split(",")]
    loc = gl.glGetUniformLocation(prog, name.encode())
    if name in ("u_texture", "u_sampler"):
        gl.glUniform1i(loc, int(vals[0]))
    else:
        getattr(gl, "glUniform%dfv" % len(vals))(loc, 1, (C.c_float * len(vals))(*vals))
zeros = (C.c_float * (6 * 4))()
gl.glVertexAttribPointer(0, 2 if "vcube" in vs_path else 4, 0x1406, 0, 0, zeros)
gl.glVertexAttribPointer(1, 3, 0x1406, 0, 0, zeros)
gl.glEnableVertexAttribArray(0)
gl.glEnableVertexAttribArray(1)
# a surfaceless context starts with a 0x0 viewport, which culls the draw
gl.glViewport(0, 0, 256, 256)
gl.glClearColor(C.c_float(0.125), C.c_float(0.25), C.c_float(0.5), C.c_float(1.0))
gl.glClear(0x4000)
gl.glDrawArrays(0x0004, 0, 6)
gl.glFlush()
err = gl.glGetError()
if err:
    raise SystemExit("draw: glGetError 0x%x" % err)
EOF

# The sampler and texture descriptors of the trace's first draw, per
# resource table, without the host pointers pandecode prints elsewhere.
cat > "$TMP/desc.py" <<'EOF'
import re
import sys

trace, title = sys.argv[1:]
lines = open(trace).read().splitlines()
out = ["# " + title]
table = re.compile(r"^  (\w+) resources resource table @([0-9a-f]+)$")
entry = re.compile(r"^      Entry (\d+) @[0-9a-f]+:$")
seen, i = set(), 0
while i < len(lines):
    m = table.match(lines[i])
    if not m:
        i += 1
        continue
    if m.group(1) in seen:
        break  # the next draw or frame
    seen.add(m.group(1))
    out.append("%s resource table @%s" % (m.group(1), m.group(2)))
    i += 1
    while i < len(lines) and (lines[i].startswith("      ") or not lines[i].strip()):
        e = entry.match(lines[i])
        i += 1
        if not e:
            continue
        block = []
        while i < len(lines) and lines[i].startswith("        "):
            block.append(lines[i][6:])
            i += 1
        if any(re.match(r"^    (Sampler|Texture) @", b) for b in block):
            out.append("  Entry %s:" % e.group(1))
            out += [b for b in block if b.strip()]
if not seen:
    raise SystemExit("desc: no resource table in %s" % trace)
print("\n".join(out))
EOF

run_case() { # <prog> <case> <size> <levels> <format> <min> <mag> <wrap s> <wrap t> <uniforms>
  local prog=$1 name=$2
  shift 2
  rm -f "$TMP/$prog.trace"*
  LD_PRELOAD=$INST/libpanfrost_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
    PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
    PAN_MESA_DEBUG=trace PANDECODE_DUMP_FILE="$TMP/$prog.trace" \
    python3 -I "$TMP/draw.py" "$INST" "$OUT/$prog.vert" "$OUT/$prog.frag" "$@" \
    > "$TMP/draw.out" 2>&1 || { tail -5 "$TMP/draw.out" >&2; echo "pan_tex_ref: $prog $name: the draw failed" >&2; exit 1; }
  # The texture uploads are GPU blits (draws of their own) before the case's
  # draw, the last RUN_IDVS of the trace; keep the trace from there on.
  cat "$TMP/$prog.trace".* > "$TMP/$prog.full"
  awk '/ RUN_IDVS/ { n = NR } END { if (!n) exit 1; print n }' "$TMP/$prog.full" > "$TMP/line" ||
    { echo "pan_tex_ref: $prog $name: the trace has no draw" >&2; exit 1; }
  tail -n +"$(cat "$TMP/line")" "$TMP/$prog.full" > "$TMP/$prog.trace"
  [ -z "${PAN_TEX_REF_TRACES-}" ] || cp "$TMP/$prog.full" "$PAN_TEX_REF_TRACES/$prog.$name.trace"
  python3 -I "$TMP/desc.py" "$TMP/$prog.trace" \
    "$prog $name: cube ${1}x$1, $2 levels, $3; min $4, mag $5, wrap s $6, wrap t $7; $8" > "$OUT/$prog.$name.desc"
  python3 -I "$HERE/pan_ref_diff.py" extract "$TMP/$prog.trace" --draw > "$TMP/$prog.$name.expect.json"
  if [ -f "$OUT/$prog.expect.json.new" ]; then
    cmp -s "$OUT/$prog.expect.json.new" "$TMP/$prog.$name.expect.json" ||
      { echo "pan_tex_ref: $prog $name draws other words than the group's first case" >&2; exit 1; }
  else
    cp "$TMP/$prog.$name.expect.json" "$OUT/$prog.expect.json.new"
  fi
  echo "pan_tex_ref: $prog.$name.desc"
}

rm -f "$OUT"/*.desc "$OUT"/*.expect.json.new
for prog in vcube cubebias; do
  python3 -I "$HERE/glsl2pan.py" "$OUT/$prog.vert" "$OUT/$prog.frag" > "$TMP/$prog.json" 2> "$OUT/$prog.pan_shc.disasm"
  grep -q '^pan_shc: build .* mesa 26.2.1$' "$TMP/$prog.json" ||
    { echo "pan_tex_ref: $PAN_SHC is not a Mesa 26.2.1 pan_shc" >&2; exit 1; }
  sed '1s/^pan_shc: build .* mesa 26.2.1$/pan_shc: build (not recorded) mesa 26.2.1/' "$TMP/$prog.json" \
    > "$OUT/$prog.pan_shc.json"
done

# texture.vertex.cube.filtering.<min>_<mag>_<wrap> and .wrap.<s>_<t>
# (es2fVertexTextureTests.cpp:1186-1251)
WRAPS="clamp:CLAMP_TO_EDGE repeat:REPEAT mirror:MIRRORED_REPEAT"
VU="u_texture=0;u_lod=0.19264507"
for minf in nearest:NEAREST linear:LINEAR nearest_mipmap_nearest:NEAREST_MIPMAP_NEAREST \
    linear_mipmap_nearest:LINEAR_MIPMAP_NEAREST nearest_mipmap_linear:NEAREST_MIPMAP_LINEAR \
    linear_mipmap_linear:LINEAR_MIPMAP_LINEAR; do
  for magf in nearest:NEAREST linear:LINEAR; do
    for w in $WRAPS; do
      run_case vcube "filtering.${minf%%:*}_${magf%%:*}_${w%%:*}" 32 6 RGB \
        "${minf#*:}" "${magf#*:}" "${w#*:}" "${w#*:}" "$VU"
    done
  done
done
for ws in clamp:CLAMP_TO_EDGE mirror:MIRRORED_REPEAT; do
  for wt in $WRAPS; do
    run_case vcube "wrap.${ws%%:*}_${wt%%:*}" 32 6 RGB LINEAR_MIPMAP_LINEAR LINEAR "${ws#*:}" "${wt#*:}" "$VU"
  done
done
# texture.mipmap.cube.bias.linear_{linear,nearest} (es2fTextureMipmapTests.cpp:1082-1085, :1207-1219)
CU="u_sampler=0;u_bias=1.0;u_colorScale=1,1,1,1;u_colorBias=0,0,0,0"
run_case cubebias bias.linear_linear 64 7 RGBA LINEAR_MIPMAP_LINEAR NEAREST CLAMP_TO_EDGE CLAMP_TO_EDGE "$CU"
run_case cubebias bias.linear_nearest 64 7 RGBA LINEAR_MIPMAP_NEAREST NEAREST CLAMP_TO_EDGE CLAMP_TO_EDGE "$CU"

for prog in vcube cubebias; do
  mv "$OUT/$prog.expect.json.new" "$OUT/$prog.expect.json"
  python3 -I "$HERE/pan_ref_diff.py" diff "$OUT/$prog.pan_shc.json" "$OUT/$prog.pan_shc.disasm" \
    --expect "$OUT/$prog.expect.json" > "$OUT/$prog.pan_shc-vs-shim.txt" || true
  echo "pan_tex_ref: $prog.expect.json; pan_shc vs the shim: $(tail -1 "$OUT/$prog.pan_shc-vs-shim.txt")"
done
