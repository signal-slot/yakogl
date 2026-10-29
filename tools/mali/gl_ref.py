#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draw once with one GLSL ES 1.00 program through a Mesa GL driver, so the
driver compiles it as an application's draw would (the reference arm of
tools/mali/drm_shim_ref.sh).

    tools/mali/gl_ref.py <libdir> vs.vert fs.frag [--prim triangles|lines|points]
        [--size WxH] [--viewport X,Y,W,H] [--depth-range N,F] [--uniform-values]
        [--second-frame] [--depth-stencil] [--second-clear depth|stencil]
        [--blend-func SRC,DST] [--blend-color R,G,B,A] [--mrt N] [--levels N]
        [--samples 4]

<libdir> holds the libEGL.so.1 and libGLESv2.so.2 to use. The program runs on
an EGL surfaceless display (EGL_PLATFORM_SURFACELESS_MESA) into a WxH RGBA8
framebuffer object (64x64 by default; the viewport is the whole framebuffer
and the depth range 0,1 unless given: a run that sets them makes the
viewport scale and offset words of the trace's FAU table distinct), binds the attributes to locations in declaration order
(as glsl2pan.py numbers them), gives each a buffer of zeros, clears and draws
six vertices, then glFlush()es: the driver submits the job, and a pandecode
trace (PAN_MESA_DEBUG=trace) holds the programs. It reads nothing back, as it
runs on drm-shim, where no job ever completes. Prints the renderer and
glGetError, and exits non-zero on any EGL or GL failure.

`--uniform-values` sets every float uniform the two stages declare, so the
trace's FAU table names each pushed uniform word by its value: uniform k of
the declaration list (the vertex shader's, then the fragment shader's not
already declared, `uniforms()`) gets component c (column-major for a matrix)
= 16 * (k + 1) + c, an integer fp16 holds exactly. Without it every uniform
is 0 and a uniform word cannot be told from a zero pad.

`--second-frame` draws again after the glFlush, without a clear, and flushes:
the second frame starts from the first one's image, which Mesa reloads
with its preload shader (pan_fb_preload.c), so the trace holds that
program too.

`--samples 4` draws into a WxH pbuffer of a 4x RGB8 config (EGL_SAMPLES 4,
gl_test's 4x config) instead of the framebuffer object: Mesa keeps the
surface's four samples in memory. With `--second-frame` the second frame is
gl_test case 214's split 4x frame (tools/cts/log/2026-10-28-splitref): its
pre-frame draw reloads each sample with Mesa's per-sample preload program,
which the trace then holds. Not with `--mrt` or `--depth-stencil`.

`--depth-stencil` attaches a D24S8 renderbuffer as depth and stencil,
clears both with the colour, and draws with the depth test (LESS, writes
on) and the stencil test (ALWAYS, REPLACE): with `--second-frame` the
second frame also reloads depth and stencil, Mesa's Z/S preload (pre
frame 1). `--second-clear depth` (or `stencil`) clears that plane first
in the second frame, so Mesa preloads only the other one.

`--blend-func SRC,DST` turns GL_BLEND on for the draw with glBlendFunc of
the two GL factor names (without the `GL_` prefix, e.g.
`CONSTANT_COLOR,ONE_MINUS_CONSTANT_COLOR`), and `--blend-color` sets
glBlendColor (0,0,0,0 by default). A blend the Mali's fixed-function unit
cannot run (two different constants read, MIN/MAX, SRC_ALPHA_SATURATE) is
drawn by the driver with a blend shader, which the trace disassembles after
the draw's blend descriptor ("Blend shader 0 @<va>", then its Shader block).

`--mrt N` makes the context an ES 3.0 one (the shaders are `#version 300 es`,
their inputs `layout(location)`; nothing is bound by name) and the framebuffer
N RGBA8 colour attachments, all enabled with glDrawBuffers, so the program's
outputs `layout(location = i)` each end in a BLEND of render target i and the
trace holds N render target and blend descriptors (gl_test cases 55 and 106).

Each sampler2D or samplerCube uniform (with or without `--uniform-values`)
reads its own texture unit, the n-th sampler of the declaration list unit n:
a 2x2 RGBA8 texture, nearest, clamped to the edge, or for a samplerCube gl_test
case 113's cube (16x16 RGBA8, five levels on each of the six faces,
GL_NEAREST_MIPMAP_NEAREST, clamped to the edge), so the trace holds the cube
texture descriptor and its planes too. A sampler keeps its place in the
numbering of the float uniforms.

ES 3 mode: when either shader's first line is `#version 300 es`, the context
is an OpenGL ES 3.0 one (EGL_CONTEXT_CLIENT_VERSION 3), the attributes are
the vertex shader's `in` declarations (declaration order, bound the same way),
and two more sampler types get textures, each at its unit as above:
`sampler3D` a 4x4x5 RGBA8 volume (gl_test case 48, GL_LINEAR, clamped on
r too) and `sampler2DArray` an 8x8x3 RGBA8 array (case 46, GL_NEAREST).
`--levels N` (N > 1) gives both a chain of N levels at 16x16 (3D depth 4,
halving; the array 2 layers) with GL_NEAREST_MIPMAP_NEAREST (case 206). A
program without that first line takes the ES 2.0 path above unchanged.
`usampler2D` / `isampler2D` get a 2x2 RGBA8UI / RGBA8I texture (nearest), and
`--uniform-values` sets `int` / `ivecN` uniforms with glUniform*iv (the same
16*(k+1)+c values, as integers).
"""
import ctypes as C
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "vivante"))
import es100 as u  # noqa: E402

EGL_PLATFORM_SURFACELESS_MESA = 0x31DD
EGL_DEFAULT_DISPLAY = None
EGL_OPENGL_ES_API = 0x30A0
EGL_CONTEXT_CLIENT_VERSION = 0x3098
EGL_NONE = 0x3038
EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES = 0x3033, 0x0001, 0x3031
EGL_RED_SIZE, EGL_GREEN_SIZE, EGL_BLUE_SIZE, EGL_WIDTH, EGL_HEIGHT = 0x3024, 0x3023, 0x3022, 0x3057, 0x3056
GL_VERTEX_SHADER, GL_FRAGMENT_SHADER = 0x8B31, 0x8B30
GL_COMPILE_STATUS, GL_LINK_STATUS, GL_INFO_LOG_LENGTH = 0x8B81, 0x8B82, 0x8B84
GL_TEXTURE_2D, GL_RGBA, GL_UNSIGNED_BYTE = 0x0DE1, 0x1908, 0x1401
GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_COMPLETE = 0x8D40, 0x8CE0, 0x8CD5
GL_COLOR_BUFFER_BIT, GL_FLOAT, GL_RENDERER = 0x4000, 0x1406, 0x1F01
GL_TEXTURE0, GL_NEAREST, GL_CLAMP_TO_EDGE = 0x84C0, 0x2600, 0x812F
GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES = 0x8D41, 0x88F0
GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT = 0x8D00, 0x8D20
GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT = 0x0100, 0x0400
GL_DEPTH_TEST, GL_STENCIL_TEST, GL_LESS, GL_ALWAYS, GL_KEEP, GL_REPLACE = 0x0B71, 0x0B90, 0x0201, 0x0207, 0x1E00, 0x1E01
GL_BLEND = 0x0BE2
BLEND_FACTORS = {
    "ZERO": 0, "ONE": 1, "SRC_COLOR": 0x0300, "ONE_MINUS_SRC_COLOR": 0x0301,
    "SRC_ALPHA": 0x0302, "ONE_MINUS_SRC_ALPHA": 0x0303, "DST_ALPHA": 0x0304,
    "ONE_MINUS_DST_ALPHA": 0x0305, "DST_COLOR": 0x0306, "ONE_MINUS_DST_COLOR": 0x0307,
    "SRC_ALPHA_SATURATE": 0x0308, "CONSTANT_COLOR": 0x8001, "ONE_MINUS_CONSTANT_COLOR": 0x8002,
    "CONSTANT_ALPHA": 0x8003, "ONE_MINUS_CONSTANT_ALPHA": 0x8004,
}
GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER, GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T = 0x2801, 0x2800, 0x2802, 0x2803
GL_TEXTURE_CUBE_MAP, GL_TEXTURE_CUBE_MAP_POSITIVE_X, GL_NEAREST_MIPMAP_NEAREST = 0x8513, 0x8515, 0x2700
GL_TEXTURE_3D, GL_TEXTURE_2D_ARRAY, GL_RGBA8, GL_LINEAR = 0x806F, 0x8C1A, 0x8058, 0x2601
GL_TEXTURE_WRAP_R, GL_TEXTURE_MAX_LEVEL = 0x8072, 0x813D
SAMPLERS = ("sampler2D", "samplerCube", "sampler3D", "sampler2DArray", "usampler2D", "isampler2D")
# integer samplers: (internal format, format, type) of a 2x2 texture
INT_SAMPLERS = {"usampler2D": (0x8D7C, 0x8D99, 0x1401), "isampler2D": (0x8D8E, 0x8D99, 0x1400)}
PRIMS = {"triangles": 0x0004, "lines": 0x0001, "points": 0x0000}


def attributes(text):
    """The attribute names in declaration order (ES 1.00 `attribute` lines)."""
    names = []
    decl = re.compile(r"^\s*attribute\s+(?:(?:highp|mediump|lowp)\s+)?\w+\s+(.*?)\s*;\s*$", re.S)
    for top, kind, chunk in u.es100_statements(u.strip_comments(text)):
        m = decl.match(chunk) if (top and kind == "stmt") else None
        if m:
            names += [d.strip() for d in m.group(1).split(",")]
    return names


def is_es3(*texts):
    """True when a shader's first non-blank line is `#version 300 es`."""
    for text in texts:
        for line in text.splitlines():
            if line.strip():
                if re.match(r"^\s*#\s*version\s+300\s+es\b", line):
                    return True
                break
    return False


def attributes_es3(text):
    """The `in` names of a GLSL ES 3.00 vertex shader in declaration order."""
    names = []
    decl = re.compile(r"^\s*(?:layout\s*\([^)]*\)\s*)?in\s+(?:(?:highp|mediump|lowp)\s+)?\w+\s+(.*?)\s*;\s*$", re.S)
    for top, kind, chunk in u.es100_statements(u.strip_comments(text)):
        m = decl.match(chunk) if (top and kind == "stmt") else None
        if m:
            names += [d.strip() for d in m.group(1).split(",")]
    return names


INT_UNIFORM_COMPONENTS = {"int": 1, "ivec2": 2, "ivec3": 3, "ivec4": 4}
UNIFORM_COMPONENTS = {"float": 1, "vec2": 2, "vec3": 3, "vec4": 4, "mat2": 4, "mat3": 9, "mat4": 16}


def uniforms(*texts):
    """The (name, type) of every uniform the texts declare, in declaration
    order, each name once (a uniform both stages declare is one GL uniform)."""
    out = []
    decl = re.compile(r"^\s*uniform\s+(?:(?:highp|mediump|lowp)\s+)?(\w+)\s+(.*?)\s*;\s*$", re.S)
    for text in texts:
        for top, kind, chunk in u.es100_statements(u.strip_comments(text)):
            m = decl.match(chunk) if (top and kind == "stmt") else None
            if m:
                for d in m.group(2).split(","):
                    if d.strip() not in [n for n, _ in out]:
                        out.append((d.strip(), m.group(1)))
    return out


def uniform_value(k, c):
    """Component c of uniform k under --uniform-values."""
    return float(16 * (k + 1) + c)


def option(args, name, default, parse):
    """The value of `--name V` in args (removed from it), parsed, or default."""
    if name not in args:
        return default
    i = args.index(name)
    value = parse(args[i + 1])
    del args[i:i + 2]
    return value


def main():
    args = sys.argv[1:]
    try:
        prim = option(args, "--prim", "triangles", str)
        width, height = option(args, "--size", (64, 64), lambda v: tuple(int(x) for x in v.split("x")))
        viewport = option(args, "--viewport", (0, 0, width, height), lambda v: tuple(int(x) for x in v.split(",")))
        near, far = option(args, "--depth-range", (0.0, 1.0), lambda v: tuple(float(x) for x in v.split(",")))
        second_clear = option(args, "--second-clear", None, str)
        blend_func = option(args, "--blend-func", None, lambda v: tuple(BLEND_FACTORS[x] for x in v.split(",")))
        mrt = option(args, "--mrt", 0, int)
        levels = option(args, "--levels", 1, int)
        samples = option(args, "--samples", 1, int)
        blend_color = option(args, "--blend-color", (0.0, 0.0, 0.0, 0.0),
                             lambda v: tuple(float(x) for x in v.split(",")))
    except (ValueError, IndexError, KeyError):
        print(__doc__, file=sys.stderr)
        return 2
    set_uniforms = "--uniform-values" in args
    if set_uniforms:
        args.remove("--uniform-values")
    second = "--second-frame" in args
    if second:
        args.remove("--second-frame")
    zs = "--depth-stencil" in args
    if zs:
        args.remove("--depth-stencil")
    if second_clear not in (None, "depth", "stencil") or (second_clear and not (second and zs)):
        print(__doc__, file=sys.stderr)
        return 2
    if (len(args) != 3 or prim not in PRIMS or len(viewport) != 4
            or (blend_func is not None and len(blend_func) != 2) or len(blend_color) != 4
            or not 0 <= mrt <= 8 or samples not in (1, 4) or (samples > 1 and (mrt or zs))):
        print(__doc__, file=sys.stderr)
        return 2
    libdir, vs_path, fs_path = args
    es3 = bool(mrt) or is_es3(open(vs_path).read(), open(fs_path).read())
    if levels < 1 or (levels > 1 and not es3):
        print(__doc__, file=sys.stderr)
        return 2
    egl = C.CDLL(os.path.join(libdir, "libEGL.so.1"))
    gl = C.CDLL(os.path.join(libdir, "libGLESv2.so.2"))
    egl.eglGetProcAddress.restype = C.c_void_p
    egl.eglGetProcAddress.argtypes = [C.c_char_p]
    gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(
        egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
    dpy = gpd(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, None)
    egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
    if not dpy or not egl.eglInitialize(dpy, None, None):
        print("gl_ref: eglInitialize failed", file=sys.stderr)
        return 1
    egl.eglBindAPI(EGL_OPENGL_ES_API)
    attrs = (C.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 3 if es3 else 2, EGL_NONE)
    cfg, surface = C.c_void_p(), None
    if samples > 1:
        # gl_test's 4x config and a pbuffer of the framebuffer's size
        cfg_attrs = (C.c_int * 11)(EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, samples, EGL_RED_SIZE, 8,
                                   EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_NONE)
        n = C.c_int()
        egl.eglChooseConfig.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_int, C.c_void_p]
        if not egl.eglChooseConfig(dpy, cfg_attrs, C.byref(cfg), 1, C.byref(n)) or n.value != 1:
            print("gl_ref: no %dx pbuffer config" % samples, file=sys.stderr)
            return 1
        egl.eglCreatePbufferSurface.restype = C.c_void_p
        egl.eglCreatePbufferSurface.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
        surface = egl.eglCreatePbufferSurface(dpy, cfg, (C.c_int * 5)(EGL_WIDTH, width, EGL_HEIGHT, height, EGL_NONE))
        if not surface:
            print("gl_ref: no %dx%d pbuffer" % (width, height), file=sys.stderr)
            return 1
    egl.eglCreateContext.restype = C.c_void_p
    egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    ctx = egl.eglCreateContext(dpy, cfg, None, attrs)
    egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    if not ctx or not egl.eglMakeCurrent(dpy, surface, surface, ctx):
        print("gl_ref: no GLES %d context" % (3 if es3 else 2), file=sys.stderr)
        return 1
    gl.glGetString.restype = C.c_char_p
    print("gl_ref: renderer %s" % gl.glGetString(GL_RENDERER).decode())
    if samples > 1:
        print("gl_ref: a %dx%d pbuffer of %d samples" % (width, height, samples))

    tex, fbo = C.c_uint(), C.c_uint()
    if samples == 1:
        # a 4x run draws into the pbuffer, the default framebuffer
        gl.glGenTextures(1, C.byref(tex))
        gl.glBindTexture(GL_TEXTURE_2D, tex)
        gl.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, width, height, 0, GL_RGBA, GL_UNSIGNED_BYTE, None)
        gl.glGenFramebuffers(1, C.byref(fbo))
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo)
        gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0)
    if mrt:
        # attachment 0 is the texture above; the others are alike
        for i in range(1, mrt):
            ti = C.c_uint()
            gl.glGenTextures(1, C.byref(ti))
            gl.glBindTexture(GL_TEXTURE_2D, ti)
            gl.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, width, height, 0, GL_RGBA, GL_UNSIGNED_BYTE, None)
            gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_TEXTURE_2D, ti, 0)
        bufs = (C.c_uint * mrt)(*[GL_COLOR_ATTACHMENT0 + i for i in range(mrt)])
        gl.glDrawBuffers(mrt, bufs)
    if zs:
        rb = C.c_uint()
        gl.glGenRenderbuffers(1, C.byref(rb))
        gl.glBindRenderbuffer(GL_RENDERBUFFER, rb)
        gl.glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, width, height)
        gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb)
        gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb)
    if gl.glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE:
        print("gl_ref: framebuffer incomplete", file=sys.stderr)
        return 1

    def shader(kind, path):
        src = open(path, "rb").read()
        sh = gl.glCreateShader(kind)
        p = C.c_char_p(src)
        gl.glShaderSource(sh, 1, C.byref(p), None)
        gl.glCompileShader(sh)
        ok = C.c_int()
        gl.glGetShaderiv(sh, GL_COMPILE_STATUS, C.byref(ok))
        if not ok.value:
            log = C.create_string_buffer(4096)
            gl.glGetShaderInfoLog(sh, 4096, None, log)
            raise SystemExit("gl_ref: %s does not compile: %s" % (path, log.value.decode()))
        return sh

    prog = gl.glCreateProgram()
    gl.glAttachShader(prog, shader(GL_VERTEX_SHADER, vs_path))
    gl.glAttachShader(prog, shader(GL_FRAGMENT_SHADER, fs_path))
    names = (attributes_es3 if es3 else attributes)(open(vs_path).read())
    for i, n in enumerate(names):
        gl.glBindAttribLocation(prog, i, n.encode())
    gl.glLinkProgram(prog)
    ok = C.c_int()
    gl.glGetProgramiv(prog, GL_LINK_STATUS, C.byref(ok))
    if not ok.value:
        log = C.create_string_buffer(4096)
        gl.glGetProgramInfoLog(prog, 4096, None, log)
        raise SystemExit("gl_ref: link failed: %s" % log.value.decode())
    gl.glUseProgram(prog)
    gl.glGetUniformLocation.argtypes = [C.c_uint, C.c_char_p]
    samplers = [(name, ty) for name, ty in uniforms(open(vs_path).read(), open(fs_path).read()) if ty in SAMPLERS]
    for unit, (name, ty) in enumerate(samplers):
        t = C.c_uint()
        gl.glActiveTexture(GL_TEXTURE0 + unit)
        gl.glGenTextures(1, C.byref(t))
        if ty == "samplerCube":
            # gl_test case 113's cube: 16x16 RGBA8, five levels, every face
            # (the trace's texture descriptor and planes are the cube's)
            target = GL_TEXTURE_CUBE_MAP
            gl.glBindTexture(target, t)
            for level in range(5):
                n = 16 >> level
                texels = (C.c_ubyte * (4 * n * n))(*([255, 0, 0, 255] * (n * n)))
                for face in range(6):
                    gl.glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, level, GL_RGBA, n, n, 0, GL_RGBA,
                                    GL_UNSIGNED_BYTE, texels)
            min_filter = GL_NEAREST_MIPMAP_NEAREST
            what = "a 16x16 RGBA8 cube map of 5 levels"
        elif ty in ("sampler3D", "sampler2DArray"):
            # gl_test case 48's volume (4x4x5, GL_LINEAR), case 46's array (8x8x3,
            # GL_NEAREST) or, with --levels, case 206's chains (16x16, three levels)
            vol = ty == "sampler3D"
            target = GL_TEXTURE_3D if vol else GL_TEXTURE_2D_ARRAY
            gl.glBindTexture(target, t)
            for level in range(levels):
                if levels > 1:
                    side, depth = 16 >> level, (max(4 >> level, 1) if vol else 2)
                else:
                    side, depth = (4, 5) if vol else (8, 3)
                texels = (C.c_ubyte * (4 * side * side * depth))(*([255, 0, 0, 255] * (side * side * depth)))
                gl.glTexImage3D(target, level, GL_RGBA8, side, side, depth, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels)
            if levels > 1:
                gl.glTexParameteri(target, GL_TEXTURE_MAX_LEVEL, levels - 1)
                min_filter = GL_NEAREST_MIPMAP_NEAREST
            else:
                min_filter = GL_LINEAR if vol else GL_NEAREST
            mag_filter = GL_LINEAR if vol and levels == 1 else GL_NEAREST
            gl.glTexParameteri(target, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE)
            what = "a %s of %d level%s" % ("4x4x5 RGBA8 volume" if vol and levels == 1 else
                                            "8x8x3 RGBA8 array" if levels == 1 else
                                            "16x16 RGBA8 " + ("volume" if vol else "array"),
                                            levels, "" if levels == 1 else "s")
        elif ty in INT_SAMPLERS:
            # an RGBA8UI / RGBA8I 2x2 texture (nearest: integer formats allow no other filter)
            target = GL_TEXTURE_2D
            gl.glBindTexture(target, t)
            ifmt, fmt, gty = INT_SAMPLERS[ty]
            texels = (C.c_ubyte * 16)(*([1, 2, 3, 4] * 4))
            gl.glTexImage2D(target, 0, ifmt, 2, 2, 0, fmt, gty, texels)
            min_filter = GL_NEAREST
            what = "a 2x2 %s texture" % ("RGBA8UI" if ty == "usampler2D" else "RGBA8I")
        else:
            target = GL_TEXTURE_2D
            gl.glBindTexture(target, t)
            texels = (C.c_ubyte * 16)(*([255, 0, 0, 255] * 4))
            gl.glTexImage2D(target, 0, GL_RGBA, 2, 2, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels)
            min_filter = GL_NEAREST
            what = "a 2x2 RGBA8 texture"
        if ty not in ("sampler3D", "sampler2DArray"):
            mag_filter = GL_NEAREST
        for pname, value in ((GL_TEXTURE_MIN_FILTER, min_filter), (GL_TEXTURE_MAG_FILTER, mag_filter),
                             (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE), (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)):
            gl.glTexParameteri(target, pname, value)
        gl.glUniform1i(gl.glGetUniformLocation(prog, name.encode()), unit)
        print("gl_ref: %s %s at unit %d: %s" % (ty, name, unit, what))
    gl.glActiveTexture(GL_TEXTURE0)
    if set_uniforms:
        for k, (name, ty) in enumerate(uniforms(open(vs_path).read(), open(fs_path).read())):
            if ty in SAMPLERS:
                continue
            n = UNIFORM_COMPONENTS.get(ty)
            if n is None and ty in INT_UNIFORM_COMPONENTS:
                n = INT_UNIFORM_COMPONENTS[ty]
                loc = gl.glGetUniformLocation(prog, name.encode())
                vals = (C.c_int * n)(*[int(uniform_value(k, c)) for c in range(n)])
                getattr(gl, "glUniform%div" % n)(loc, 1, vals)
                print("gl_ref: uniform %s %s at %d = %s" % (ty, name, loc, " ".join("%d" % v for v in vals)))
                continue
            if n is None:
                raise SystemExit("gl_ref: --uniform-values does not set a %s uniform (%s)" % (ty, name))
            loc = gl.glGetUniformLocation(prog, name.encode())
            vals = (C.c_float * n)(*[uniform_value(k, c) for c in range(n)])
            if ty.startswith("mat"):
                getattr(gl, "glUniformMatrix%dfv" % {4: 2, 9: 3, 16: 4}[n])(loc, 1, 0, vals)
            else:
                getattr(gl, "glUniform%dfv" % n)(loc, 1, vals)
            print("gl_ref: uniform %s %s at %d = %s" % (ty, name, loc, " ".join("%g" % v for v in vals)))
    zeros = (C.c_float * (6 * 16))()
    for i in range(len(names)):
        gl.glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, zeros)
        gl.glEnableVertexAttribArray(i)
    gl.glViewport(*viewport)
    gl.glDepthRangef(C.c_float(near), C.c_float(far))
    gl.glClearColor(C.c_float(0), C.c_float(0), C.c_float(1), C.c_float(1))
    if zs:
        gl.glEnable(GL_DEPTH_TEST)
        gl.glDepthFunc(GL_LESS)
        gl.glEnable(GL_STENCIL_TEST)
        gl.glStencilFunc(GL_ALWAYS, 1, 0xFF)
        gl.glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE)
    gl.glClear(GL_COLOR_BUFFER_BIT | (GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT if zs else 0))
    if blend_func is not None:
        gl.glEnable(GL_BLEND)
        gl.glBlendColor(*[C.c_float(c) for c in blend_color])
        gl.glBlendFunc(*blend_func)
    gl.glDrawArrays(PRIMS[prim], 0, 6)
    gl.glFlush()
    if second:
        if second_clear:
            gl.glClear(GL_DEPTH_BUFFER_BIT if second_clear == "depth" else GL_STENCIL_BUFFER_BIT)
        gl.glDrawArrays(PRIMS[prim], 0, 6)
        gl.glFlush()
    err = gl.glGetError()
    print("gl_ref: attributes %s, framebuffer %dx%d, viewport %s, depth range %g,%g, glGetError 0x%x"
          % (" ".join(names) or "(none)", width, height, ",".join(map(str, viewport)), near, far, err))
    return 0 if err == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
