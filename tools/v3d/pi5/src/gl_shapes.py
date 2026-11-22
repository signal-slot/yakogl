#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draw the GL calls of one gl_test case through Mesa's GL driver, so the
driver builds and submits the V3D jobs the case makes (the GL side of
`tools/v3d/drm_shim_ref.sh run` for the tools/v3d/pi5/gl_*.clif references).

    tools/v3d/pi5/src/gl_shapes.py <libdir> <shape>

<libdir> holds the libEGL.so.1 and libGLESv2.so.2 to use. The program runs on
an EGL surfaceless display (EGL_PLATFORM_SURFACELESS_MESA) in an OpenGL ES 3.0
context, 64x64 like gl_test (TW x TH). It reads nothing back: on the drm-shim
no job ever completes, so every glReadPixels of gl_test is left out and each
glFinish of the case stays where the case has it (it ends a job). The draws
use gl_test's ES 1.00 pair (driver/tools/pvrlab/gl_test_shaders_es100.h:
gl_test_vs_src, gl_test_fs_src) with gl_test's 40-byte cuv vertex (pos4,
color4, uv2 = (1, 1)) at attributes 0, 1, 2, as cuv_attribs() sets them.

gl_test's own cases 5, 58, 59, 66 and 102 draw to the window surface, which
has a depth/stencil buffer. Here that surface is a framebuffer object: a 64x64
GL_RGBA texture and a GL_DEPTH24_STENCIL8 renderbuffer ("the window"). Case 67
builds its own framebuffer objects, as gl_test does.

Shapes (gl_test case, function in driver/tools/pvrlab/gl_test.c):

  scissor    5    case_scissor: an unscissored black clear and glFinish, then
                  scissor (0,0,32,32): a clear and a yellow full-screen quad.
  scisbox    58   case_clearscis: a blue clear and a red clear scissored to
                  (16,16,32,32); then blue, a green quad and the scissored red
                  clear after it; then blue and a clear with an empty (0-wide)
                  scissor box. A glFinish after each of the three.
  writemask  59   case_clearmask: a clear with glColorMask all off; a red
                  depth-tested (GL_LESS) quad, a depth clear with
                  glDepthMask(GL_FALSE), a green quad behind; a stencil clear to
                  0, a stencil clear to 0xff through glStencilMask(0x0f), a
                  draw testing EQUAL 0xff, glFinish, and a draw testing EQUAL
                  0x0f in the next job (it reads the stencil the earlier job
                  wrote).
  viewport   66   case_clearvp: a white clear at the full viewport, a blue
                  clear at a quarter viewport, a green clear scissored to the
                  bottom-left quarter. A glFinish after each.
  fbods      67   case_fbods frames 1 to 8: a colour-only FBO with the depth and
                  then the stencil test on; a GL_RGB colour target cleared with
                  alpha 0.25; two FBOs over one colour texture with a
                  GL_DEPTH_COMPONENT16 renderbuffer each (depth-tested draws);
                  a GL_STENCIL_INDEX8 FBO with a scissored stencil-only clear,
                  and the same with a draw ahead, in one job and across a
                  glFinish.
  colormask  102  case_maskclear: R+A masked clear; a G-only clear scissored to
                  the bottom-left 32x32; a B-only clear followed by a yellow
                  quad on the bottom-left quadrant with every channel on.
  mrt4       -    a clear of four GL_RGBA8 colour targets (glDrawBuffers 4).
  mrt8       -    the same with eight targets (V3D 7.1's maximum).

The P10 texture shapes (TEX_SHAPES) draw into the same window. tex2d, texfmt
and mipgen use gl_test's ES 1.00 UV pair (gl_test_uv_vs_src,
gl_test_texture_fs_src: texture2D(u_tex, v_uv), u_tex on unit 0) with its
24-byte puv vertex (pos4, uv2; puv_quad(): a full-screen quad, uv (0,0) at
NDC (-1,-1)) at attributes 0 and 1. Their textures hold fixed bytes
(tex_bytes()). Each of the four runs with tfu_spy.c ahead of the drm-shim
(with_tfu_spy()), which prints every TFU submission's registers on stdout.

  tex2d      -    a 64x64 GL_RGBA8 texture (glTexImage2D with data, level 0
                  only, wrap REPEAT) drawn after a black clear with NEAREST
                  min and mag filters, glFinish, and again with LINEAR: two
                  jobs.
  texfmt     -    the same NEAREST draw from a 64x64 GL_SRGB8_ALPHA8 texture
                  (Mesa's 7.1 Transfer func sRGB), glFinish, and from a 64x64
                  GL_RGB565 texture (GL_UNSIGNED_SHORT_5_6_5, Mesa's
                  B5G6R5_UNORM: R and B swapped against RGB565): two jobs.
  mipgen     -    glTexStorage2D of a 64x64 GL_RGBA8 texture with 7 levels,
                  level 0 by glTexSubImage2D, glGenerateMipmap (Mesa's TFU job:
                  not in the CLIF dump; its registers are tfu_spy.c's stdout
                  lines), then the draw with LINEAR_MIPMAP_LINEAR and LINEAR:
                  one job.
  cube       115  case_texcube: six 4x4 GL_RGBA faces (tools/v3d/tex_cube_case.h
                  tcc_make_faces: R a ramp along s, G along t, B 16 + 40 *
                  face), CLAMP_TO_EDGE, gl_test's texcube pair
                  (gl_test_texcube_vs_src, gl_test_texcube_fs_src) with
                  tcc_faces' u_major, u_sx, u_ty, a 32x32 viewport; per face,
                  NEAREST and then LINEAR: a clear to (0,0,0,0), the quad and a
                  glFinish, 12 jobs. An ES 3.0 context, so Mesa's core asks
                  for seamless cube filtering (v3d has no field for it).

The P13 draw shapes (DRAW_SHAPES) use the same window and gl_test's 40-byte
cuv vertex; each is one job (one glFinish), so that the packets between the
draws of a job show what a state change emits:

  blend      -    GL_BLEND on, a black clear, then in one job a quad with
                  glBlendFuncSeparate(SRC_ALPHA, ONE_MINUS_SRC_ALPHA, ONE, ZERO)
                  and GL_FUNC_ADD (colour alpha 0.5), and a quad with
                  glBlendFuncSeparate(CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR,
                  ONE, ONE), glBlendEquationSeparate(GL_FUNC_REVERSE_SUBTRACT,
                  GL_MAX) and glBlendColor(0.25, 0.5, 0.75, 0.5).
  depthfunc  -    depth cleared to 0.5, depth writes off; a quad at z -0.5
                  under GL_LESS, then a quad at z 0.75 under GL_GEQUAL.
  stencilops -    D24S8 window, stencil cleared to 0x10; a quad with
                  glStencilFunc(GL_LEQUAL, 0x20, 0x3f), glStencilOp(GL_INCR,
                  GL_DECR, GL_INVERT), glStencilMask(0x0f) and the depth test
                  (GL_LESS) on; a quad with front ops (REPLACE, KEEP, INCR_WRAP)
                  and back ops (ZERO, DECR_WRAP, INVERT) through the Separate
                  entry points, func NOTEQUAL 0x05 mask 0xf0, write mask 0xff.
  indexed    -    glDrawElements of the quad's six vertices as GL_TRIANGLES with
                  GL_UNSIGNED_BYTE, GL_UNSIGNED_SHORT (at a byte offset) and
                  GL_UNSIGNED_INT indices, and a GL_TRIANGLE_STRIP of u16
                  indices with an index restart (GL_PRIMITIVE_RESTART_FIXED_INDEX,
                  0xffff).
  instanced  -    glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 3): a_pos per vertex,
                  a_color and a_off (vec2) with glVertexAttribDivisor 1 from a
                  second buffer (a shrunken quad moved by a_off).
  prims      -    GL_POINTS (4 vertices, gl_PointSize 8.0 in the vertex shader),
                  GL_LINES (4 vertices, glLineWidth(1.0)) and GL_TRIANGLE_STRIP
                  (4 vertices) from one 12-vertex buffer.

The vertex data is fixed in this file so that the jobs' bytes are the same on
every run. Prints the renderer and glGetError; exits non-zero on any EGL or GL
failure.
"""
import ctypes as C
import os
import struct
import subprocess
import sys

EGL_PLATFORM_SURFACELESS_MESA = 0x31DD
EGL_OPENGL_ES_API = 0x30A0
EGL_CONTEXT_CLIENT_VERSION = 0x3098
EGL_NONE = 0x3038
GL_VERTEX_SHADER, GL_FRAGMENT_SHADER = 0x8B31, 0x8B30
GL_COMPILE_STATUS, GL_LINK_STATUS = 0x8B81, 0x8B82
GL_TEXTURE_2D, GL_UNSIGNED_BYTE, GL_FLOAT = 0x0DE1, 0x1401, 0x1406
GL_RGB, GL_RGBA, GL_RGBA8 = 0x1907, 0x1908, 0x8058
GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER, GL_NEAREST = 0x2801, 0x2800, 0x2600
GL_FRAMEBUFFER, GL_RENDERBUFFER, GL_FRAMEBUFFER_COMPLETE = 0x8D40, 0x8D41, 0x8CD5
GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT = 0x8CE0, 0x8D00
GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT = 0x8D20, 0x821A
GL_DEPTH24_STENCIL8, GL_DEPTH_COMPONENT16, GL_STENCIL_INDEX8 = 0x88F0, 0x81A5, 0x8D48
GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT = 0x4000, 0x0100, 0x0400
GL_SCISSOR_TEST, GL_DEPTH_TEST, GL_STENCIL_TEST, GL_BLEND = 0x0C11, 0x0B71, 0x0B90, 0x0BE2
GL_LESS, GL_EQUAL, GL_KEEP = 0x0201, 0x0202, 0x1E00
GL_ARRAY_BUFFER, GL_STATIC_DRAW = 0x8892, 0x88E4
GL_TRIANGLES, GL_RENDERER = 0x0004, 0x1F01
GL_LINEAR, GL_LINEAR_MIPMAP_LINEAR = 0x2601, 0x2703
GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE = 0x2802, 0x2803, 0x812F
GL_SRGB8_ALPHA8, GL_RGB565, GL_UNSIGNED_SHORT_5_6_5 = 0x8C43, 0x8D62, 0x8363
GL_TEXTURE_CUBE_MAP, GL_TEXTURE_CUBE_MAP_POSITIVE_X = 0x8513, 0x8515
GL_TEXTURE0, GL_UNPACK_ALIGNMENT = 0x84C0, 0x0CF5
GL_ZERO, GL_ONE = 0, 1
GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA = 0x0302, 0x0303
GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR = 0x8001, 0x8002
GL_FUNC_ADD, GL_FUNC_REVERSE_SUBTRACT, GL_MAX = 0x8006, 0x800B, 0x8008
GL_LEQUAL, GL_GEQUAL, GL_NOTEQUAL = 0x0203, 0x0206, 0x0205
GL_REPLACE, GL_INCR, GL_DECR, GL_INVERT = 0x1E01, 0x1E02, 0x1E03, 0x150A
GL_INCR_WRAP, GL_DECR_WRAP = 0x8507, 0x8508
GL_FRONT, GL_BACK = 0x0404, 0x0405
GL_ELEMENT_ARRAY_BUFFER, GL_PRIMITIVE_RESTART_FIXED_INDEX = 0x8893, 0x8D69
GL_UNSIGNED_SHORT, GL_UNSIGNED_INT = 0x1403, 0x1405
GL_POINTS, GL_LINES, GL_TRIANGLE_STRIP = 0x0000, 0x0001, 0x0005
T, F = 1, 0
TW = TH = 64

# gl_test_shaders_es100.h: gl_test_vs_src and gl_test_fs_src, comments left out.
VS = b"""#version 100
attribute vec4 a_pos;
attribute vec4 a_color;
attribute vec2 a_uv;
varying vec4 v_color;
varying vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
    v_uv = a_uv;
}
"""
FS = b"""#version 100
precision mediump float;
varying vec4 v_color;
varying vec2 v_uv;
void main() { gl_FragColor = v_color * vec4(v_uv, 1.0, 1.0); }
"""
# instanced: the cuv VS with a per-instance offset (a_off at attribute 2), the
# quad shrunk to a quarter of the viewport.
INST_VS = b"""#version 100
attribute vec4 a_pos;
attribute vec4 a_color;
attribute vec2 a_off;
varying vec4 v_color;
varying vec2 v_uv;
void main() {
    gl_Position = vec4(a_pos.xy * 0.25 + a_off, a_pos.zw);
    v_color = a_color;
    v_uv = vec2(1.0, 1.0);
}
"""
# prims: gl_test_vs_src with gl_PointSize.
POINT_VS = b"""#version 100
attribute vec4 a_pos;
attribute vec4 a_color;
attribute vec2 a_uv;
varying vec4 v_color;
varying vec2 v_uv;
void main() {
    gl_Position = a_pos;
    gl_PointSize = 8.0;
    v_color = a_color;
    v_uv = a_uv;
}
"""
# gl_test_uv_vs_src and gl_test_texture_fs_src (gl_test_shaders_es100.h),
# comments left out: tex2d, texfmt, mipgen.
UV_VS = b"""#version 100
attribute vec4 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    gl_Position = a_pos;
    v_uv = a_uv;
}
"""
TEX_FS = b"""#version 100
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() { gl_FragColor = texture2D(u_tex, v_uv); }
"""
# gl_test_texcube_vs_src and gl_test_texcube_fs_src: cube (case 115).
CUBE_VS = b"""#version 100
attribute vec4 a_pos;
uniform vec3 u_major;
uniform vec3 u_sx;
uniform vec3 u_ty;
varying vec3 v_dir;
void main() {
    v_dir = u_major + a_pos.x * u_sx + a_pos.y * u_ty;
    gl_Position = a_pos;
}
"""
CUBE_FS = b"""#version 100
precision mediump float;
uniform samplerCube u_s;
varying vec3 v_dir;
void main() {
    gl_FragColor = textureCube(u_s, v_dir);
}
"""
# tools/v3d/tex_cube_case.h tcc_faces: (major, sx, ty) per face, GL's order
CUBE_FACES = (
    ((1, 0, 0), (0, 0, -1), (0, -1, 0)),
    ((-1, 0, 0), (0, 0, 1), (0, -1, 0)),
    ((0, 1, 0), (1, 0, 0), (0, 0, 1)),
    ((0, -1, 0), (1, 0, 0), (0, 0, -1)),
    ((0, 0, 1), (1, 0, 0), (0, -1, 0)),
    ((0, 0, -1), (-1, 0, 0), (0, -1, 0)),
)
FULL = ((-1, -1), (1, -1), (1, 1), (-1, -1), (1, 1), (-1, 1))
LOWER_LEFT = ((-1, -1), (0, -1), (0, 0), (-1, -1), (0, 0), (-1, 0))

SHAPES = ("scissor", "scisbox", "writemask", "viewport", "fbods", "colormask", "mrt4", "mrt8")
TEX_SHAPES = ("tex2d", "texfmt", "mipgen", "cube")
DRAW_SHAPES = ("blend", "depthfunc", "stencilops", "indexed", "instanced", "prims")


class Gl:
    """The GLES entry points, called by name; every float argument is wrapped."""

    def __init__(self, lib):
        self.lib = lib

    def __getattr__(self, name):
        fn = getattr(self.lib, name)

        def call(*args):
            return fn(*[C.c_float(a) if isinstance(a, float) else a for a in args])
        return call


def puv_quad(uv=True):
    """gl_test's puv_quad(): pos (x, y, 0, 1) and uv, (0,0) at NDC (-1,-1);
    uv (0,0) everywhere when uv is False (case 115 reads only a_pos)."""
    out = []
    for x, y in FULL:
        u, v = ((x + 1) / 2.0, (y + 1) / 2.0) if uv else (0.0, 0.0)
        out += [float(x), float(y), 0.0, 1.0, u, v]
    return out


def tex_bytes(w, h, bpp):
    """Fixed texel bytes of a w x h image, bpp bytes a texel: byte k of texel
    (i, j) is (4 * i + 16 * j + 64 * k) & 0xff."""
    return bytes((4 * i + 16 * j + 64 * k) & 0xff
                 for j in range(h) for i in range(w) for k in range(bpp))


def quad(corners, z, rgb, alpha=1.0):
    """cuv_set() for each corner: pos (x, y, z, 1), colour (rgb, alpha), uv (1, 1)."""
    out = []
    for x, y in corners:
        out += [float(x), float(y), z, 1.0, rgb[0], rgb[1], rgb[2], alpha, 1.0, 1.0]
    return out


class Case:
    def __init__(self, gl):
        self.gl = gl
        self.vbo = None

    def program(self, vs=VS, fs=FS, attrs=(b"a_pos", b"a_color", b"a_uv")):
        gl = self.gl

        def shader(kind, src):
            sh = gl.glCreateShader(kind)
            p = C.c_char_p(src)
            gl.glShaderSource(sh, 1, C.byref(p), None)
            gl.glCompileShader(sh)
            ok = C.c_int()
            gl.glGetShaderiv(sh, GL_COMPILE_STATUS, C.byref(ok))
            if not ok.value:
                log = C.create_string_buffer(4096)
                gl.glGetShaderInfoLog(sh, 4096, None, log)
                raise SystemExit("gl_shapes: shader does not compile: %s" % log.value.decode())
            return sh

        prog = gl.glCreateProgram()
        gl.glAttachShader(prog, shader(GL_VERTEX_SHADER, vs))
        gl.glAttachShader(prog, shader(GL_FRAGMENT_SHADER, fs))
        for i, name in enumerate(attrs):
            gl.glBindAttribLocation(prog, i, name)
        gl.glLinkProgram(prog)
        ok = C.c_int()
        gl.glGetProgramiv(prog, GL_LINK_STATUS, C.byref(ok))
        if not ok.value:
            raise SystemExit("gl_shapes: link failed")
        gl.glUseProgram(prog)
        return prog

    def upload(self, floats):
        """upload_vbo() and cuv_attribs(): a GL_ARRAY_BUFFER of 40-byte vertices."""
        gl = self.gl
        if self.vbo is None:
            vbo = C.c_uint()
            gl.glGenBuffers(1, C.byref(vbo))
            self.vbo = vbo.value
        gl.glBindBuffer(GL_ARRAY_BUFFER, self.vbo)
        data = (C.c_float * len(floats))(*floats)
        gl.glBufferData(GL_ARRAY_BUFFER, C.c_ssize_t(C.sizeof(data)), data, GL_STATIC_DRAW)
        for i, (n, off) in enumerate(((4, 0), (4, 16), (2, 32))):
            gl.glVertexAttribPointer(i, n, GL_FLOAT, F, 40, C.c_void_p(off))
            gl.glEnableVertexAttribArray(i)

    def texture(self, fmt):
        gl = self.gl
        tex = C.c_uint()
        gl.glGenTextures(1, C.byref(tex))
        gl.glBindTexture(GL_TEXTURE_2D, tex)
        gl.glTexImage2D(GL_TEXTURE_2D, 0, fmt, TW, TH, 0, fmt, GL_UNSIGNED_BYTE, None)
        gl.glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST)
        gl.glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST)
        return tex.value

    def renderbuffer(self, fmt):
        gl = self.gl
        rb = C.c_uint()
        gl.glGenRenderbuffers(1, C.byref(rb))
        gl.glBindRenderbuffer(GL_RENDERBUFFER, rb)
        gl.glRenderbufferStorage(GL_RENDERBUFFER, fmt, TW, TH)
        return rb.value

    def fbo(self, colours, attach=()):
        """A bound framebuffer object: colour textures at COLOR_ATTACHMENT0..,
        and (attachment point, renderbuffer) pairs."""
        gl = self.gl
        fb = C.c_uint()
        gl.glGenFramebuffers(1, C.byref(fb))
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fb)
        for i, tex in enumerate(colours):
            gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_TEXTURE_2D, tex, 0)
        for point, rb in attach:
            gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb)
        self.complete()
        return fb.value

    def complete(self):
        status = self.gl.glCheckFramebufferStatus(GL_FRAMEBUFFER)
        if status != GL_FRAMEBUFFER_COMPLETE:
            raise SystemExit("gl_shapes: framebuffer status 0x%x" % status)

    def window(self):
        """gl_test's window surface: RGBA8 colour and a D24S8 buffer."""
        self.fbo([self.texture(GL_RGBA)],
                 [(GL_DEPTH_STENCIL_ATTACHMENT, self.renderbuffer(GL_DEPTH24_STENCIL8))])
        self.gl.glViewport(0, 0, TW, TH)

    # ---- the shapes ---------------------------------------------------------

    def scissor(self):
        gl = self.gl
        self.window()
        self.program()
        self.upload(quad(FULL, 0.0, (1.0, 1.0, 0.0)))
        gl.glDisable(GL_SCISSOR_TEST)
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glEnable(GL_SCISSOR_TEST)
        gl.glScissor(0, 0, 32, 32)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glDisable(GL_SCISSOR_TEST)

    def scisbox(self):
        gl = self.gl
        self.window()
        self.program()
        self.upload(quad(FULL, 0.0, (0.0, 1.0, 0.0)))
        for step in range(3):
            gl.glClearColor(0.0, 0.0, 1.0, 1.0)
            gl.glClear(GL_COLOR_BUFFER_BIT)
            if step == 1:
                gl.glDrawArrays(GL_TRIANGLES, 0, 6)
            gl.glEnable(GL_SCISSOR_TEST)
            gl.glScissor(16, 16, 0 if step == 2 else 32, 32)
            gl.glClearColor(1.0, 0.0, 0.0, 1.0)
            gl.glClear(GL_COLOR_BUFFER_BIT)
            gl.glFinish()
            gl.glDisable(GL_SCISSOR_TEST)

    def writemask(self):
        gl = self.gl
        self.window()
        self.program()
        # colour: every channel masked off
        self.upload(quad(FULL, 0.0, (0.0, 1.0, 0.0)))
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glColorMask(F, F, F, F)
        gl.glClearColor(1.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glColorMask(T, T, T, T)
        gl.glFinish()
        # depth: a masked depth clear between two depth-tested draws
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClearDepthf(1.0)
        gl.glDepthMask(T)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        gl.glEnable(GL_DEPTH_TEST)
        gl.glDepthFunc(GL_LESS)
        self.upload(quad(FULL, 0.0, (1.0, 0.0, 0.0)))
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glDepthMask(F)
        gl.glClearDepthf(1.0)
        gl.glClear(GL_DEPTH_BUFFER_BIT)
        gl.glDepthMask(T)
        self.upload(quad(FULL, 0.5, (0.0, 1.0, 0.0)))
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glDisable(GL_DEPTH_TEST)
        # stencil: a clear through a 0x0f write mask, read in this job and the next
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glStencilMask(0xff)
        gl.glClearStencil(0x00)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
        gl.glStencilMask(0x0f)
        gl.glClearStencil(0xff)
        gl.glClear(GL_STENCIL_BUFFER_BIT)
        gl.glStencilMask(0xff)
        gl.glEnable(GL_STENCIL_TEST)
        gl.glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP)
        self.upload(quad(FULL, 0.0, (0.0, 1.0, 0.0)))
        gl.glStencilFunc(GL_EQUAL, 0xff, 0xff)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glStencilFunc(GL_EQUAL, 0x0f, 0xff)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glDisable(GL_STENCIL_TEST)

    def viewport(self):
        gl = self.gl
        self.window()
        gl.glDisable(GL_SCISSOR_TEST)
        gl.glColorMask(T, T, T, T)
        gl.glClearColor(1.0, 1.0, 1.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glViewport(0, 0, TW // 2, TH // 2)
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glViewport(0, 0, TW, TH)
        gl.glEnable(GL_SCISSOR_TEST)
        gl.glScissor(0, 0, TW // 2, TH // 2)
        gl.glClearColor(0.0, 1.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glDisable(GL_SCISSOR_TEST)

    def fbods(self):
        gl = self.gl
        self.program()
        # 0..5 red at z -0.5 (near), 6..11 green at z +0.5 (far)
        self.upload(quad(FULL, -0.5, (1.0, 0.0, 0.0)) + quad(FULL, 0.5, (0.0, 1.0, 0.0)))
        gl.glViewport(0, 0, TW, TH)
        # frame 1: colour only, the depth test on
        tex = self.texture(GL_RGBA)
        fbo_a = self.fbo([tex])
        gl.glEnable(GL_DEPTH_TEST)
        gl.glDepthFunc(GL_LESS)
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glDrawArrays(GL_TRIANGLES, 6, 6)
        gl.glFinish()
        # frame 2: the stencil test on, no stencil buffer
        gl.glEnable(GL_STENCIL_TEST)
        gl.glStencilFunc(GL_EQUAL, 1, 0xff)
        gl.glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP)
        gl.glClearStencil(0)
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glDisable(GL_STENCIL_TEST)
        # frame 3: a GL_RGB colour target, cleared with alpha 0.25
        self.fbo([self.texture(GL_RGB)])
        gl.glDisable(GL_DEPTH_TEST)
        gl.glClearColor(1.0, 0.0, 0.0, 0.25)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        # frames 4 and 5: two FBOs over one colour texture, a depth buffer each
        depth_a = self.renderbuffer(GL_DEPTH_COMPONENT16)
        depth_b = self.renderbuffer(GL_DEPTH_COMPONENT16)
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo_a)
        gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth_a)
        fbo_b = self.fbo([tex], [(GL_DEPTH_ATTACHMENT, depth_b)])
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo_a)
        gl.glEnable(GL_DEPTH_TEST)
        gl.glDepthFunc(GL_LESS)
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClearDepthf(1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo_b)
        gl.glDrawArrays(GL_TRIANGLES, 6, 6)
        gl.glFinish()
        gl.glDisable(GL_DEPTH_TEST)
        # frame 6: a scissored stencil-only clear on a STENCIL_INDEX8 FBO
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo_a)
        gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0)
        sten = self.renderbuffer(GL_STENCIL_INDEX8)
        gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, sten)
        self.complete()
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClearStencil(0)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
        gl.glEnable(GL_SCISSOR_TEST)
        gl.glScissor(16, 16, 32, 32)
        gl.glClearStencil(1)
        gl.glClear(GL_STENCIL_BUFFER_BIT)
        gl.glDisable(GL_SCISSOR_TEST)
        gl.glEnable(GL_STENCIL_TEST)
        gl.glStencilFunc(GL_EQUAL, 0, 0xff)
        gl.glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()
        gl.glDisable(GL_STENCIL_TEST)
        # frames 7 and 8: the same with a draw ahead, in one job and across a glFinish
        for step in range(2):
            gl.glClearColor(0.0, 0.0, 1.0, 1.0)
            gl.glClearStencil(0)
            gl.glClear(GL_COLOR_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
            gl.glDrawArrays(GL_TRIANGLES, 6, 6)
            gl.glEnable(GL_SCISSOR_TEST)
            gl.glScissor(16, 16, 32, 32)
            gl.glClearStencil(1)
            gl.glClear(GL_STENCIL_BUFFER_BIT)
            gl.glDisable(GL_SCISSOR_TEST)
            gl.glClearStencil(0)
            if step == 1:
                gl.glFinish()
            gl.glEnable(GL_STENCIL_TEST)
            gl.glStencilFunc(GL_EQUAL, 0, 0xff)
            gl.glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP)
            gl.glDrawArrays(GL_TRIANGLES, 0, 6)
            gl.glFinish()
            gl.glDisable(GL_STENCIL_TEST)
        gl.glBindFramebuffer(GL_FRAMEBUFFER, 0)

    def colormask(self):
        gl = self.gl
        self.window()
        gl.glDisable(GL_SCISSOR_TEST)
        gl.glDisable(GL_BLEND)
        # 1. full screen, R and A
        gl.glColorMask(T, T, T, T)
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glColorMask(T, F, F, T)
        gl.glClearColor(1.0, 1.0, 1.0, 0.5)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glColorMask(T, T, T, T)
        # 2. scissored, G only
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glEnable(GL_SCISSOR_TEST)
        gl.glScissor(0, 0, 32, 32)
        gl.glColorMask(F, T, F, F)
        gl.glClearColor(0.0, 1.0, 0.0, 0.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glDisable(GL_SCISSOR_TEST)
        gl.glColorMask(T, T, T, T)
        # 3. fused into a draw, B only
        self.program()
        self.upload(quad(LOWER_LEFT, 0.0, (1.0, 1.0, 0.0)))
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()
        gl.glColorMask(F, F, T, F)
        gl.glClearColor(0.0, 0.0, 1.0, 0.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glColorMask(T, T, T, T)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()

    def mrt(self, n):
        gl = self.gl
        self.fbo([self.texture(GL_RGBA) for _ in range(n)])
        bufs = (C.c_uint * n)(*[GL_COLOR_ATTACHMENT0 + i for i in range(n)])
        gl.glDrawBuffers(n, bufs)
        gl.glViewport(0, 0, TW, TH)
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glFinish()

    def mrt4(self):
        self.mrt(4)

    def mrt8(self):
        self.mrt(8)

    # ---- the P10 texture shapes -----------------------------------------------

    def puv(self, prog_args, uv=True):
        """the window, a program, unit 0 active, and puv_quad() at attribute 0
        (pos4) and, when uv, 1 (uv2), 24 bytes a vertex."""
        gl = self.gl
        self.window()
        prog = self.program(*prog_args)
        gl.glActiveTexture(GL_TEXTURE0)
        floats = puv_quad(uv)
        vbo = C.c_uint()
        gl.glGenBuffers(1, C.byref(vbo))
        gl.glBindBuffer(GL_ARRAY_BUFFER, vbo.value)
        data = (C.c_float * len(floats))(*floats)
        gl.glBufferData(GL_ARRAY_BUFFER, C.c_ssize_t(C.sizeof(data)), data, GL_STATIC_DRAW)
        for i, (n, off) in enumerate(((4, 0), (2, 16))[:2 if uv else 1]):
            gl.glVertexAttribPointer(i, n, GL_FLOAT, F, 24, C.c_void_p(off))
            gl.glEnableVertexAttribArray(i)
        return prog

    def uv_program(self):
        prog = self.puv((UV_VS, TEX_FS, (b"a_pos", b"a_uv")))
        self.gl.glUniform1i(self.gl.glGetUniformLocation(prog, b"u_tex"), 0)

    def new_texture(self, target):
        gl = self.gl
        tex = C.c_uint()
        gl.glGenTextures(1, C.byref(tex))
        gl.glBindTexture(target, tex.value)
        gl.glPixelStorei(GL_UNPACK_ALIGNMENT, 4)
        return tex.value

    def filters(self, target, min_filter, mag_filter):
        self.gl.glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min_filter)
        self.gl.glTexParameteri(target, GL_TEXTURE_MAG_FILTER, mag_filter)

    def image2d(self, ifmt, fmt, typ, bpp):
        """a bound 64x64 2D texture of tex_bytes(), level 0 only, NEAREST."""
        self.new_texture(GL_TEXTURE_2D)
        self.gl.glTexImage2D(GL_TEXTURE_2D, 0, ifmt, TW, TH, 0, fmt, typ, tex_bytes(TW, TH, bpp))
        self.filters(GL_TEXTURE_2D, GL_NEAREST, GL_NEAREST)

    def draw_tex(self):
        gl = self.gl
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glFinish()

    def tex2d(self):
        self.uv_program()
        self.image2d(GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE, 4)
        self.draw_tex()
        self.filters(GL_TEXTURE_2D, GL_LINEAR, GL_LINEAR)
        self.draw_tex()

    def texfmt(self):
        self.uv_program()
        self.image2d(GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE, 4)
        self.draw_tex()
        self.image2d(GL_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, 2)
        self.draw_tex()

    def mipgen(self):
        gl = self.gl
        self.uv_program()
        self.new_texture(GL_TEXTURE_2D)
        gl.glTexStorage2D(GL_TEXTURE_2D, 7, GL_RGBA8, TW, TH)
        gl.glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE,
                           tex_bytes(TW, TH, 4))
        gl.glGenerateMipmap(GL_TEXTURE_2D)
        self.filters(GL_TEXTURE_2D, GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR)
        self.draw_tex()

    def cube(self):
        gl = self.gl
        prog = self.puv((CUBE_VS, CUBE_FS, (b"a_pos",)), uv=False)
        gl.glUniform1i(gl.glGetUniformLocation(prog, b"u_s"), 0)
        loc = [gl.glGetUniformLocation(prog, n) for n in (b"u_major", b"u_sx", b"u_ty")]
        self.new_texture(GL_TEXTURE_CUBE_MAP)
        ramp = (0, 85, 170, 255)
        for f in range(6):
            texels = bytes(c for j in range(4) for i in range(4)
                           for c in (ramp[i], ramp[j], 16 + 40 * f, 255))
            gl.glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, GL_RGBA, 4, 4, 0,
                            GL_RGBA, GL_UNSIGNED_BYTE, texels)
        gl.glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE)
        gl.glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)
        gl.glViewport(0, 0, 32, 32)
        for i in range(12):
            for at, v in zip(loc, CUBE_FACES[i // 2]):
                gl.glUniform3f(at, float(v[0]), float(v[1]), float(v[2]))
            filt = GL_LINEAR if i & 1 else GL_NEAREST
            self.filters(GL_TEXTURE_CUBE_MAP, filt, filt)
            gl.glClearColor(0.0, 0.0, 0.0, 0.0)
            gl.glClear(GL_COLOR_BUFFER_BIT)
            gl.glDrawArrays(GL_TRIANGLES, 0, 6)
            gl.glFinish()
        gl.glViewport(0, 0, TW, TH)

    # ---- the P13 draw shapes ----------------------------------------------------

    def blend(self):
        gl = self.gl
        self.window()
        self.program()
        self.upload(quad(FULL, 0.0, (1.0, 0.5, 0.0), 0.5) + quad(FULL, 0.0, (0.0, 1.0, 1.0), 0.5))
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glEnable(GL_BLEND)
        gl.glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ZERO)
        gl.glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_ADD)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glBlendColor(0.25, 0.5, 0.75, 0.5)
        gl.glBlendFuncSeparate(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR, GL_ONE, GL_ONE)
        gl.glBlendEquationSeparate(GL_FUNC_REVERSE_SUBTRACT, GL_MAX)
        gl.glDrawArrays(GL_TRIANGLES, 6, 6)
        gl.glFinish()
        gl.glDisable(GL_BLEND)

    def depthfunc(self):
        gl = self.gl
        self.window()
        self.program()
        self.upload(quad(FULL, -0.5, (1.0, 0.0, 0.0)) + quad(FULL, 0.75, (0.0, 1.0, 0.0)))
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClearDepthf(0.5)
        gl.glDepthMask(T)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        gl.glEnable(GL_DEPTH_TEST)
        gl.glDepthMask(F)
        gl.glDepthFunc(GL_LESS)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glDepthFunc(GL_GEQUAL)
        gl.glDrawArrays(GL_TRIANGLES, 6, 6)
        gl.glFinish()
        gl.glDisable(GL_DEPTH_TEST)
        gl.glDepthMask(T)

    def stencilops(self):
        gl = self.gl
        self.window()
        self.program()
        self.upload(quad(FULL, 0.0, (1.0, 0.0, 0.0)) + quad(FULL, 0.25, (0.0, 1.0, 0.0)))
        gl.glClearColor(0.0, 0.0, 1.0, 1.0)
        gl.glClearDepthf(1.0)
        gl.glStencilMask(0xff)
        gl.glClearStencil(0x10)
        gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
        gl.glEnable(GL_STENCIL_TEST)
        gl.glEnable(GL_DEPTH_TEST)
        gl.glDepthFunc(GL_LESS)
        gl.glStencilFunc(GL_LEQUAL, 0x20, 0x3f)
        gl.glStencilOp(GL_INCR, GL_DECR, GL_INVERT)
        gl.glStencilMask(0x0f)
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)
        gl.glStencilMask(0xff)
        gl.glStencilFuncSeparate(GL_FRONT, GL_NOTEQUAL, 0x05, 0xf0)
        gl.glStencilFuncSeparate(GL_BACK, GL_NOTEQUAL, 0x05, 0xf0)
        gl.glStencilOpSeparate(GL_FRONT, GL_REPLACE, GL_KEEP, GL_INCR_WRAP)
        gl.glStencilOpSeparate(GL_BACK, GL_ZERO, GL_DECR_WRAP, GL_INVERT)
        gl.glDrawArrays(GL_TRIANGLES, 6, 6)
        gl.glFinish()
        gl.glDisable(GL_STENCIL_TEST)
        gl.glDisable(GL_DEPTH_TEST)

    def indexed(self):
        gl = self.gl
        self.window()
        self.program()
        self.upload(quad(FULL, 0.0, (1.0, 1.0, 0.0)))
        ibo = C.c_uint()
        gl.glGenBuffers(1, C.byref(ibo))
        gl.glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo.value)

        def indices(fmt, values):
            data = struct.pack("<%d%s" % (len(values), fmt), *values)
            gl.glBufferData(GL_ELEMENT_ARRAY_BUFFER, C.c_ssize_t(len(data)), data, GL_STATIC_DRAW)

        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        indices("B", (0, 1, 2, 3, 4, 5))
        gl.glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_BYTE, C.c_void_p(0))
        indices("H", (5, 5, 0, 1, 2, 3, 4, 5))
        gl.glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, C.c_void_p(4))
        indices("I", (0, 1, 2, 3, 4, 5))
        gl.glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_INT, C.c_void_p(0))
        gl.glEnable(GL_PRIMITIVE_RESTART_FIXED_INDEX)
        indices("H", (0, 1, 2, 0xffff, 3, 4, 5))
        gl.glDrawElements(GL_TRIANGLE_STRIP, 7, GL_UNSIGNED_SHORT, C.c_void_p(0))
        gl.glFinish()
        gl.glDisable(GL_PRIMITIVE_RESTART_FIXED_INDEX)

    def instanced(self):
        gl = self.gl
        self.window()
        self.program(INST_VS, FS, (b"a_pos", b"a_color", b"a_off"))
        # per vertex: a_pos at attribute 0, a 16-byte vertex
        floats = []
        for x, y in FULL:
            floats += [float(x), float(y), 0.0, 1.0]
        vbo = C.c_uint()
        gl.glGenBuffers(1, C.byref(vbo))
        gl.glBindBuffer(GL_ARRAY_BUFFER, vbo.value)
        data = (C.c_float * len(floats))(*floats)
        gl.glBufferData(GL_ARRAY_BUFFER, C.c_ssize_t(C.sizeof(data)), data, GL_STATIC_DRAW)
        gl.glVertexAttribPointer(0, 4, GL_FLOAT, F, 16, C.c_void_p(0))
        gl.glEnableVertexAttribArray(0)
        # per instance (divisor 1): colour4 and offset2, 24 bytes
        inst = [1.0, 0.0, 0.0, 1.0, -0.5, -0.5,
                0.0, 1.0, 0.0, 1.0, 0.5, -0.5,
                0.0, 0.0, 1.0, 1.0, 0.0, 0.5]
        ibo = C.c_uint()
        gl.glGenBuffers(1, C.byref(ibo))
        gl.glBindBuffer(GL_ARRAY_BUFFER, ibo.value)
        data = (C.c_float * len(inst))(*inst)
        gl.glBufferData(GL_ARRAY_BUFFER, C.c_ssize_t(C.sizeof(data)), data, GL_STATIC_DRAW)
        for i, (n, off) in ((1, (4, 0)), (2, (2, 16))):
            gl.glVertexAttribPointer(i, n, GL_FLOAT, F, 24, C.c_void_p(off))
            gl.glEnableVertexAttribArray(i)
            gl.glVertexAttribDivisor(i, 1)
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 3)
        gl.glFinish()

    def prims(self):
        gl = self.gl
        self.window()
        self.program(POINT_VS, FS)
        pts = ((-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5))
        lines = ((-0.9, -0.9), (0.9, -0.9), (0.9, 0.9), (-0.9, 0.9))
        strip = ((-1, -1), (1, -1), (-1, 1), (1, 1))
        self.upload(quad(pts, 0.0, (1.0, 1.0, 1.0)) + quad(lines, 0.0, (1.0, 1.0, 0.0))
                    + quad(strip, 0.0, (0.0, 1.0, 1.0)))
        gl.glClearColor(0.0, 0.0, 0.0, 1.0)
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glDrawArrays(GL_POINTS, 0, 4)
        gl.glLineWidth(1.0)
        gl.glDrawArrays(GL_LINES, 4, 4)
        gl.glDrawArrays(GL_TRIANGLE_STRIP, 8, 4)
        gl.glFinish()


def with_tfu_spy(libdir):
    """Run this program again with tfu_spy.c's ioctl() ahead of the drm-shim in
    LD_PRELOAD (built as <libdir>/gl_shapes_tfu_spy.so), so that every TFU
    submission's registers are printed; returns once that is in place."""
    if os.environ.get("GL_SHAPES_TFU_SPY"):
        return
    here = os.path.dirname(os.path.abspath(__file__))
    uapi = os.path.join(here, "..", "..", "..", "..", "third_party", "mesa-26.2.1",
                        "include", "drm-uapi")
    so = os.path.join(libdir, "gl_shapes_tfu_spy.so")
    subprocess.run(["cc", "-shared", "-fPIC", "-O2", "-I", os.path.normpath(uapi), "-o", so,
                    os.path.join(here, "tfu_spy.c"), "-ldl"], check=True)
    env = dict(os.environ, GL_SHAPES_TFU_SPY=so)
    env["LD_PRELOAD"] = ":".join(p for p in (so, os.environ.get("LD_PRELOAD")) if p)
    os.execve(sys.executable, [sys.executable, "-u", os.path.abspath(__file__)] + sys.argv[1:],
              env)


def main():
    args = sys.argv[1:]
    if len(args) != 2 or args[1] not in SHAPES + TEX_SHAPES + DRAW_SHAPES:
        print(__doc__, file=sys.stderr)
        return 2
    libdir, shape = args
    if shape in TEX_SHAPES:
        with_tfu_spy(libdir)
    egl = C.CDLL(os.path.join(libdir, "libEGL.so.1"))
    glib = C.CDLL(os.path.join(libdir, "libGLESv2.so.2"))
    egl.eglGetProcAddress.restype = C.c_void_p
    egl.eglGetProcAddress.argtypes = [C.c_char_p]
    gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(
        egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
    dpy = gpd(EGL_PLATFORM_SURFACELESS_MESA, None, None)
    egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
    if not dpy or not egl.eglInitialize(dpy, None, None):
        print("gl_shapes: eglInitialize failed", file=sys.stderr)
        return 1
    egl.eglBindAPI(EGL_OPENGL_ES_API)
    attrs = (C.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE)
    egl.eglCreateContext.restype = C.c_void_p
    egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    ctx = egl.eglCreateContext(dpy, None, None, attrs)
    egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
        print("gl_shapes: no GLES 3.0 context", file=sys.stderr)
        return 1
    glib.glGetString.restype = C.c_char_p
    print("gl_shapes: renderer %s" % glib.glGetString(GL_RENDERER).decode())
    gl = Gl(glib)
    getattr(Case(gl), shape)()
    err = glib.glGetError()
    print("gl_shapes: %s, framebuffer %dx%d, glGetError 0x%x" % (shape, TW, TH, err))
    return 0 if err == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
