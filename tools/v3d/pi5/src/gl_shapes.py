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

The vertex data is fixed in this file so that the jobs' bytes are the same on
every run. Prints the renderer and glGetError; exits non-zero on any EGL or GL
failure.
"""
import ctypes as C
import os
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
FULL = ((-1, -1), (1, -1), (1, 1), (-1, -1), (1, 1), (-1, 1))
LOWER_LEFT = ((-1, -1), (0, -1), (0, 0), (-1, -1), (0, 0), (-1, 0))

SHAPES = ("scissor", "scisbox", "writemask", "viewport", "fbods", "colormask", "mrt4", "mrt8")


class Gl:
    """The GLES entry points, called by name; every float argument is wrapped."""

    def __init__(self, lib):
        self.lib = lib

    def __getattr__(self, name):
        fn = getattr(self.lib, name)

        def call(*args):
            return fn(*[C.c_float(a) if isinstance(a, float) else a for a in args])
        return call


def quad(corners, z, rgb):
    """cuv_set() for each corner: pos (x, y, z, 1), colour (rgb, 1), uv (1, 1)."""
    out = []
    for x, y in corners:
        out += [float(x), float(y), z, 1.0, rgb[0], rgb[1], rgb[2], 1.0, 1.0, 1.0]
    return out


class Case:
    def __init__(self, gl):
        self.gl = gl
        self.vbo = None

    def program(self):
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
        gl.glAttachShader(prog, shader(GL_VERTEX_SHADER, VS))
        gl.glAttachShader(prog, shader(GL_FRAGMENT_SHADER, FS))
        for i, name in enumerate((b"a_pos", b"a_color", b"a_uv")):
            gl.glBindAttribLocation(prog, i, name)
        gl.glLinkProgram(prog)
        ok = C.c_int()
        gl.glGetProgramiv(prog, GL_LINK_STATUS, C.byref(ok))
        if not ok.value:
            raise SystemExit("gl_shapes: link failed")
        gl.glUseProgram(prog)

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


def main():
    args = sys.argv[1:]
    if len(args) != 2 or args[1] not in SHAPES:
        print(__doc__, file=sys.stderr)
        return 2
    libdir, shape = args
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
