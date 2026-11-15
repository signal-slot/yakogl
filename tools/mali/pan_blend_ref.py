#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Mesa 26.2.1's blend descriptor, its fixed-function/blend-shader choice and
its blend shader words for every blend state dEQP-GLES2.functional.
fragment_ops.blend draws, written to tools/mali/testdata/pan_blend_ref/.

    pan_blend_ref.py states [--out DIR]
        write DIR/states.tsv only (the state list; needs no Mesa).
    pan_blend_ref.py gen [--out DIR]
        draw every state on Mesa's drm-shim and write DIR/states.tsv,
        DIR/blend.tsv and DIR/shaders/<sha>.{words,txt}.
    pan_blend_ref.py --check
        gen into a scratch directory under $TMPDIR and compare it byte for
        byte with the committed tools/mali/testdata/pan_blend_ref/.
    pan_blend_ref.py draw <libdir> <states.tsv>
        (internal) the GL calls, run under the shim by gen.

DIR defaults to tools/mali/testdata/pan_blend_ref. `--shim WORK` before or
after the mode sets MESA_PAN_SHIM to WORK.

Inputs.
- The shim driver: MESA_PAN_SHIM names the work directory of
  `tools/mali/drm_shim_ref.sh build mesa-26.2.1.tar.xz <work dir>` (it holds
  inst/lib64 or inst/lib/<multiarch> with libEGL.so.1, libGLESv2.so.2 and
  libpanfrost_noop_drm_shim.so). It must not be MESA_PAN_TREE's directory:
  build_pan_shc.sh's tree has no EGL, and drm_shim_ref.sh build deletes the
  tree it extracts into. PAN_SHC is not used. The committed files were made
  with a second build of MESA_PAN_TREE's 26.2.1 source: `meson setup <build>
  $MESA_PAN_TREE` with drm_shim_ref.sh build's options and
  -Dprefix=<work dir>/inst, LLVM 21, then `ninja install -C <build>`.
- The environment of drm_shim_ref.sh run: PAN_GPU_ID=a867,
  MESA_LOADER_DRIVER_OVERRIDE=panthor, MESA_SHADER_CACHE_DISABLE=true,
  PAN_MESA_DEBUG=trace, PANDECODE_DUMP_FILE.
- The states, from VK-GL-CTS modules/gles2/functional/es2fBlendTests.cpp
  (BlendTests::init): blend colour (0.2, 0.4, 0.6, 0.8) everywhere;
  equation_src_func_dst_func (3 equations x 15 source x 14 destination
  factors, the same for rgb and alpha); rgb_func_alpha_func/src (FUNC_ADD,
  rgb and alpha source factor 15 x 15, destination ONE, ZERO, SRC_COLOR,
  DST_COLOR for both); rgb_func_alpha_func/dst (the same with source and
  destination swapped, SRC_ALPHA_SATURATE left out); rgb_equation_alpha_
  equation (3 x 3 equations, factors ONE/ONE). Each distinct state is kept
  once, in that order, with the first case that draws it. One more state,
  not in dEQP, puts glColorMask(1, 0, 1, 1) on the blend-shader state
  add constant_color/one_minus_constant_color.
- The target: a 64x64 single-sampled GL_RGBA/GL_UNSIGNED_BYTE texture
  framebuffer (the RGBA8 surface dEQP draws in; Mesa's blend key holds the
  format and sample count, not the surface kind). The fragment shader writes
  a mediump varying, as dEQP's blend program does.

Per state the draw is glClear (so no tile preload draw is made), the state,
one quad, glFlush. The trace holds one "Blend RT 0:" block per draw, then a
"Blend shader" block when the mode is Shader (genxml/decode.c,
pandecode_blend_descs).

Outputs (tab separated, one line per state, state ids from states.tsv):
- states.tsv: id, name, eqRGB srcRGB dstRGB eqA srcA dstA (GL enums in
  hex), colour mask (R G B A), first dEQP case.
- blend.tsv: id, name, choice (shader, fixed-function, opaque or off: the
  descriptor's Internal Mode), word0 and the equation word (word1) of the
  Blend descriptor packed from pandecode's fields with genxml/v10.xml's
  layout, the Blend Constant field, Num Comps/RT/Conversion of the
  fixed-function internal ("-" otherwise), the blend shader's file ("-" for
  none). The shader PC (an address) is not kept.
- shaders/<first 16 hex of sha256 of the .words>.words: one 64-bit word per
  line as the *.expect.json words (blend_case190_211_words.py), .txt:
  pandecode's bytes and disassembly lines.
"""
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_OUT = os.path.join(HERE, "testdata", "pan_blend_ref")

EQUATIONS = [("add", 0x8006), ("subtract", 0x800A), ("reverse_subtract", 0x800B)]
FUNCS = [
    ("zero", 0x0000), ("one", 0x0001), ("src_color", 0x0300), ("one_minus_src_color", 0x0301),
    ("dst_color", 0x0306), ("one_minus_dst_color", 0x0307), ("src_alpha", 0x0302),
    ("one_minus_src_alpha", 0x0303), ("dst_alpha", 0x0304), ("one_minus_dst_alpha", 0x0305),
    ("constant_color", 0x8001), ("one_minus_constant_color", 0x8002), ("constant_alpha", 0x8003),
    ("one_minus_constant_alpha", 0x8004), ("src_alpha_saturate", 0x0308),
]
NAME = {v: n for n, v in EQUATIONS + FUNCS}
ADD, ONE, ZERO, SRC_COLOR, DST_COLOR, SATURATE = 0x8006, 1, 0, 0x0300, 0x0306, 0x0308
BLEND_COLOR = (0.2, 0.4, 0.6, 0.8)
MASK_STATE = ((ADD, 0x8001, 0x8002, ADD, 0x8001, 0x8002), (1, 0, 1, 1))
GROUP = "dEQP-GLES2.functional.fragment_ops.blend."


def states():
    """[(eqs and factors, mask, first case)] in dEQP order, each once."""
    out, seen = [], set()

    def add(p, case, mask=(1, 1, 1, 1)):
        if (p, mask) not in seen:
            seen.add((p, mask))
            out.append((p, mask, case))

    for en, e in EQUATIONS:
        for sn, s in FUNCS:
            for dn, d in FUNCS:
                if d != SATURATE:
                    add((e, s, d, e, s, d), "%sequation_src_func_dst_func.%s_%s_%s" % (GROUP, en, sn, dn))
    for is_dst in (0, 1):
        for rn, r in FUNCS:
            for an, a in FUNCS:
                if is_dst and SATURATE in (r, a):
                    continue
                case = "%srgb_func_alpha_func.%s.%s_%s" % (GROUP, ("src", "dst")[is_dst], rn, an)
                for other in (ONE, ZERO, SRC_COLOR, DST_COLOR):
                    if is_dst:
                        add((ADD, other, r, ADD, other, a), case)
                    else:
                        add((ADD, r, other, ADD, a, other), case)
    for rn, r in EQUATIONS:
        for an, a in EQUATIONS:
            add((r, ONE, ONE, a, ONE, ONE), "%srgb_equation_alpha_equation.%s_%s" % (GROUP, rn, an))
    add(MASK_STATE[0], "-", MASK_STATE[1])
    return out


def state_name(p, mask):
    n = "%s_%s_%s__%s_%s_%s" % tuple(NAME[v] for v in p)
    if mask != (1, 1, 1, 1):
        n += "__mask_%d%d%d%d" % mask
    return n


def states_tsv():
    lines = []
    for i, (p, mask, case) in enumerate(states()):
        lines.append("%d\t%s\t%s\t%s\t%s\n" % (i, state_name(p, mask), " ".join("0x%04x" % v for v in p),
                                               " ".join(str(m) for m in mask), case))
    return "".join(lines)


def read_states(path):
    out = []
    for line in open(path):
        f = line.rstrip("\n").split("\t")
        out.append((tuple(int(v, 16) for v in f[2].split()), tuple(int(v) for v in f[3].split())))
    return out


# --- the draw, run under the shim -------------------------------------------

VS = (b"attribute highp vec4 a_position;\nattribute mediump vec4 a_color;\nvarying mediump vec4 v_color;\n"
      b"void main() { gl_Position = a_position; v_color = a_color; }\n")
FS = b"varying mediump vec4 v_color;\nvoid main() { gl_FragColor = v_color; }\n"


def draw(libdir, states_path):
    import ctypes as C
    egl = C.CDLL(os.path.join(libdir, "libEGL.so.1"))
    gl = C.CDLL(os.path.join(libdir, "libGLESv2.so.2"))
    egl.eglGetProcAddress.restype = C.c_void_p
    egl.eglGetProcAddress.argtypes = [C.c_char_p]
    gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
    dpy = gpd(0x31DD, None, None)  # EGL_PLATFORM_SURFACELESS_MESA
    egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
    if not dpy or not egl.eglInitialize(dpy, None, None):
        return 1
    egl.eglBindAPI(0x30A0)
    attrs = (C.c_int * 3)(0x3098, 2, 0x3038)
    egl.eglCreateContext.restype = C.c_void_p
    egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    ctx = egl.eglCreateContext(dpy, None, None, attrs)
    egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
        return 1
    gl.glGetString.restype = C.c_char_p
    print("pan_blend_ref: renderer %s" % gl.glGetString(0x1F01).decode())
    tex, fbo = C.c_uint(), C.c_uint()
    gl.glGenTextures(1, C.byref(tex))
    gl.glBindTexture(0x0DE1, tex)
    gl.glTexImage2D(0x0DE1, 0, 0x1908, 64, 64, 0, 0x1908, 0x1401, None)
    gl.glGenFramebuffers(1, C.byref(fbo))
    gl.glBindFramebuffer(0x8D40, fbo)
    gl.glFramebufferTexture2D(0x8D40, 0x8CE0, 0x0DE1, tex, 0)

    def shader(kind, src):
        sh = gl.glCreateShader(kind)
        p = C.c_char_p(src)
        gl.glShaderSource(sh, 1, C.byref(p), None)
        gl.glCompileShader(sh)
        return sh

    prog = gl.glCreateProgram()
    gl.glAttachShader(prog, shader(0x8B31, VS))
    gl.glAttachShader(prog, shader(0x8B30, FS))
    gl.glBindAttribLocation(prog, 0, b"a_position")
    gl.glBindAttribLocation(prog, 1, b"a_color")
    gl.glLinkProgram(prog)
    gl.glUseProgram(prog)
    gl.glViewport(0, 0, 64, 64)
    pos = (C.c_float * 12)(-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1)
    col = (C.c_float * 24)(*([0.25, 0.5, 0.75, 0.5] * 6))
    gl.glEnableVertexAttribArray(0)
    gl.glEnableVertexAttribArray(1)
    gl.glVertexAttribPointer(0, 2, 0x1406, 0, 0, pos)
    gl.glVertexAttribPointer(1, 4, 0x1406, 0, 0, col)
    f = C.c_float
    gl.glClearColor(f(0.5), f(0.5), f(0.5), f(0.5))
    gl.glEnable(0x0BE2)  # GL_BLEND
    gl.glBlendColor(*[f(c) for c in BLEND_COLOR])
    for p, mask in read_states(states_path):
        gl.glColorMask(1, 1, 1, 1)
        gl.glClear(0x4000)
        gl.glBlendEquationSeparate(p[0], p[3])
        gl.glBlendFuncSeparate(p[1], p[2], p[4], p[5])
        gl.glColorMask(*mask)
        gl.glDrawArrays(0x0004, 0, 6)
        gl.glFlush()
    err = gl.glGetError()
    print("pan_blend_ref: glGetError 0x%x" % err)
    return 1 if err else 0


# --- the trace --------------------------------------------------------------

# pandecode prints a 0 operand A or C as "XXX: INVALID"; a Shader-mode
# descriptor's equation word is 0 (Mesa sets only word0 and the internal).
OPERAND_A = {"XXX: INVALID": 0, "Zero": 1, "Src": 2, "Dest": 3}
OPERAND_B = {"Src Minus Dest": 0, "Src Plus Dest": 1, "Src": 2, "Dest": 3}
OPERAND_C = {"XXX: INVALID": 0, "Zero": 1, "Src": 2, "Dest": 3, "Src x 2": 4, "Src Alpha": 5, "Dest Alpha": 6,
             "Constant": 7}
BOOL = {"false": 0, "true": 1}
WORD_LINE = re.compile(r"^(?:[0-9a-f]{2} ){7}[0-9a-f]{2}    \S")


def field(block, path):
    """The value of a field of pandecode's indented dump, by its path."""
    stack = []  # (indent, key) of the enclosing groups
    for line in block:
        ind = len(line) - len(line.lstrip(" "))
        key, _, val = line.strip().partition(":")
        while stack and stack[-1][0] >= ind:
            stack.pop()
        if [k for _, k in stack] + [key] == list(path):
            return val.strip()
        stack.append((ind, key))
    raise ValueError("pan_blend_ref: no field %s" % "/".join(path))


def function_bits(block, which):
    g = lambda k: field(block, ["Equation", which, k])
    return (OPERAND_A[g("A")] | BOOL[g("Negate A")] << 3 | OPERAND_B[g("B")] << 4 | BOOL[g("Negate B")] << 7
            | OPERAND_C[g("C")] << 8 | BOOL[g("Invert C")] << 11)


def draws(trace_lines):
    """[(Blend RT 0 block, blend shader lines or None)] in trace order."""
    out, i, n = [], 0, len(trace_lines)
    while i < n:
        if trace_lines[i].strip() == "Blend RT 0:":
            ind = len(trace_lines[i]) - len(trace_lines[i].lstrip(" "))
            i += 1
            block = []
            while i < n and len(trace_lines[i]) - len(trace_lines[i].lstrip(" ")) > ind and trace_lines[i].strip():
                block.append(trace_lines[i])
                i += 1
            sh = None
            j = i
            while j < n and not trace_lines[j].strip():
                j += 1
            if j < n and trace_lines[j].startswith("Blend shader "):
                i = j
                i += 1
                assert trace_lines[i].startswith("Shader "), trace_lines[i]
                i += 1
                sh = []
                while i < n and WORD_LINE.match(trace_lines[i]):
                    sh.append(trace_lines[i])
                    i += 1
            out.append((block, sh))
        else:
            i += 1
    return out


def describe(block, sh, shaders):
    mode = field(block, ["Internal", "Mode"])
    choice = {"Shader": "shader", "Fixed-Function": "fixed-function", "Opaque": "opaque", "Off": "off"}[mode]
    const = int(field(block, ["Blend Constant"]))
    word0 = (BOOL[field(block, ["Load Destination"])] | BOOL[field(block, ["Alpha To One"])] << 8
             | BOOL[field(block, ["Enable"])] << 9 | BOOL[field(block, ["sRGB"])] << 10
             | BOOL[field(block, ["Round to FB precision"])] << 11 | const << 16)
    eq = function_bits(block, "RGB") | function_bits(block, "Alpha") << 12 | int(
        field(block, ["Equation", "Color Mask"])) << 28
    ff = "-"
    if mode != "Shader":
        ff = "%s/%s/%s/raw=%s" % (field(block, ["Internal", "Fixed-Function", "Num Comps"]),
                                  field(block, ["Internal", "Fixed-Function", "RT"]),
                                  field(block, ["Internal", "Fixed-Function", "Conversion", "Format (v7)"]),
                                  field(block, ["Internal", "Fixed-Function", "Conversion", "Raw"]))
    shfile = "-"
    if mode == "Shader":
        if not sh:
            raise ValueError("pan_blend_ref: a Shader-mode draw without a blend shader block")
        words = "".join("".join(reversed(line[:23].split(" "))) + "\n" for line in sh)
        shfile = "shaders/%s" % hashlib.sha256(words.encode()).hexdigest()[:16]
        shaders[shfile] = (words, "".join(line + "\n" for line in sh))
    elif sh:
        raise ValueError("pan_blend_ref: a blend shader block after a %s descriptor" % mode)
    return choice, "0x%08x" % word0, "0x%08x" % eq, const, ff, shfile


def shim_libdir():
    work = os.environ.get("MESA_PAN_SHIM")
    if not work:
        sys.exit("pan_blend_ref: set MESA_PAN_SHIM to the work dir of tools/mali/drm_shim_ref.sh build")
    inst = os.path.join(work, "inst", "lib64")
    if not os.path.isdir(inst):
        machine = subprocess.run(["gcc", "-dumpmachine"], capture_output=True, text=True).stdout.strip()
        inst = os.path.join(work, "inst", "lib", machine)
    if not os.path.isfile(os.path.join(inst, "libpanfrost_noop_drm_shim.so")):
        sys.exit("pan_blend_ref: %s has no drm-shim build" % work)
    return inst


def gen(out):
    os.makedirs(out, exist_ok=True)
    with open(os.path.join(out, "states.tsv"), "w") as f:
        f.write(states_tsv())
    st = states()
    lib = shim_libdir()
    scratch = tempfile.mkdtemp(prefix="pan_blend_ref.", dir=os.environ.get("TMPDIR"))
    try:
        env = dict(os.environ, LD_PRELOAD=os.path.join(lib, "libpanfrost_noop_drm_shim.so"), LD_LIBRARY_PATH=lib,
                   PAN_GPU_ID="a867", MESA_LOADER_DRIVER_OVERRIDE="panthor", MESA_SHADER_CACHE_DISABLE="true",
                   PAN_MESA_DEBUG="trace", PANDECODE_DUMP_FILE=os.path.join(scratch, "t"))
        env.pop("BIFROST_MESA_DEBUG", None)
        r = subprocess.run([sys.executable, "-I", "-u", os.path.abspath(__file__), "draw", lib,
                            os.path.join(out, "states.tsv")], env=env, capture_output=True, text=True)
        if r.returncode:
            sys.exit("pan_blend_ref: the draw failed\n" + r.stdout + r.stderr[-2000:])
        files = sorted((f for f in os.listdir(scratch) if f.startswith("t.")),
                       key=lambda f: (f.rsplit(".", 2)[-2], int(f.rsplit(".", 1)[-1])))
        lines = []
        for fn in files:
            lines += open(os.path.join(scratch, fn)).read().split("\n")
    finally:
        shutil.rmtree(scratch)
    found = draws(lines)
    if len(found) != len(st):
        sys.exit("pan_blend_ref: %d draws in the trace, %d states" % (len(found), len(st)))
    shaders, rows = {}, []
    for i, ((p, mask, _), (block, sh)) in enumerate(zip(st, found)):
        mask_bits = mask[0] | mask[1] << 1 | mask[2] << 2 | mask[3] << 3
        # a blend shader applies the mask itself; its descriptor's mask is 0
        if (field(block, ["Internal", "Mode"]) != "Shader"
                and int(field(block, ["Equation", "Color Mask"])) != mask_bits):
            sys.exit("pan_blend_ref: draw %d has colour mask %s, the state %d" %
                     (i, field(block, ["Equation", "Color Mask"]), mask_bits))
        rows.append("%d\t%s\t%s\t%s\t%s\t%d\t%s\t%s\n" % ((i, state_name(p, mask)) + describe(block, sh, shaders)))
    with open(os.path.join(out, "blend.tsv"), "w") as f:
        f.write("".join(rows))
    shdir = os.path.join(out, "shaders")
    if os.path.isdir(shdir):
        shutil.rmtree(shdir)
    os.makedirs(shdir)
    for name, (words, txt) in sorted(shaders.items()):
        with open(os.path.join(out, name + ".words"), "w") as f:
            f.write(words)
        with open(os.path.join(out, name + ".txt"), "w") as f:
            f.write(txt)
    print("pan_blend_ref: %d states, %d blend shaders in %s" % (len(rows), len(shaders), out))
    return 0


def tree(d):
    out = {}
    for root, _, files in os.walk(d):
        for fn in files:
            p = os.path.join(root, fn)
            out[os.path.relpath(p, d)] = open(p, "rb").read()
    return out


def check():
    scratch = tempfile.mkdtemp(prefix="pan_blend_ref.check.", dir=os.environ.get("TMPDIR"))
    try:
        gen(scratch)
        a, b = tree(DEFAULT_OUT), tree(scratch)
    finally:
        shutil.rmtree(scratch)
    bad = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
    for k in bad:
        print("pan_blend_ref --check: %s differs" % k, file=sys.stderr)
    print("pan_blend_ref --check: %s" % ("FAIL" if bad else "ok, %d files" % len(a)))
    return 1 if bad else 0


def main(argv):
    if "--shim" in argv[:-1]:
        i = argv.index("--shim")
        os.environ["MESA_PAN_SHIM"] = argv[i + 1]
        argv = argv[:i] + argv[i + 2:]
    if argv[:1] == ["--check"] and len(argv) == 1:
        return check()
    if len(argv) == 3 and argv[0] == "draw":
        return draw(argv[1], argv[2])
    if argv[:1] in (["states"], ["gen"]):
        out = DEFAULT_OUT
        if len(argv) == 3 and argv[1] == "--out":
            out = argv[2]
        elif len(argv) != 1:
            print(__doc__, file=sys.stderr)
            return 2
        if argv[0] == "gen":
            return gen(out)
        os.makedirs(out, exist_ok=True)
        with open(os.path.join(out, "states.tsv"), "w") as f:
            f.write(states_tsv())
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
