#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""The GL calls of gl_test cases 190 (blendonce) and 211 (msaablend), made
through Mesa's GL driver on the drm-shim, so the driver builds the blend
shaders those cases need (tools/mali/testdata/blend_case190.* and
blend_case211.*, docs/notes/mali-blend-shaders.md).

    blend_case190_211_ref.py <libdir> 190|211

<libdir> holds libEGL.so.1 and libGLESv2.so.2 (as tools/mali/gl_ref.py). Run
by tools/mali/testdata/blend_case190_211_ref.sh. The program is a pass-through
(a_pos, u_color), a 64x64 RGBA8 texture target (case 211: 4x through
EXT_multisampled_render_to_texture, as gl_test's pbuffer 4x is the same Mesa
sample count), the state of the cases and nothing read back (the shim runs no
job). 190 draws four quads in one frame; 211 draws the red triangle without
blending, then the blended full-screen quad.
"""
import ctypes as C
import os
import sys

GL_VERTEX_SHADER, GL_FRAGMENT_SHADER = 0x8B31, 0x8B30
GL_TEXTURE_2D, GL_RGBA, GL_UNSIGNED_BYTE = 0x0DE1, 0x1908, 0x1401
GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_COLOR_BUFFER_BIT = 0x8D40, 0x8CE0, 0x4000
GL_BLEND, GL_FLOAT, GL_TRIANGLES = 0x0BE2, 0x1406, 0x0004
ZERO, ONE, SRC_ALPHA, ONE_MINUS_SRC_ALPHA, ONE_MINUS_DST_ALPHA = 0, 1, 0x0302, 0x0303, 0x0305
CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR = 0x8001, 0x8002
FUNC_ADD, MIN = 0x8006, 0x8007

VS = b"attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n"
FS = b"precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }\n"


def main():
    if len(sys.argv) != 3 or sys.argv[2] not in ("190", "211"):
        print(__doc__, file=sys.stderr)
        return 2
    libdir, case = sys.argv[1], sys.argv[2]
    egl = C.CDLL(os.path.join(libdir, "libEGL.so.1"))
    gl = C.CDLL(os.path.join(libdir, "libGLESv2.so.2"))
    egl.eglGetProcAddress.restype = C.c_void_p
    egl.eglGetProcAddress.argtypes = [C.c_char_p]
    gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
    dpy = gpd(0x31DD, None, None)
    egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
    if not dpy or not egl.eglInitialize(dpy, None, None):
        return 1
    egl.eglBindAPI(0x30A0)
    attrs = (C.c_int * 3)(0x3098, 3, 0x3038)
    egl.eglCreateContext.restype = C.c_void_p
    egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    ctx = egl.eglCreateContext(dpy, None, None, attrs)
    egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
        return 1
    gl.glGetString.restype = C.c_char_p
    print("blend_case: renderer %s" % gl.glGetString(0x1F01).decode())
    tex, fbo = C.c_uint(), C.c_uint()
    gl.glGenTextures(1, C.byref(tex))
    gl.glBindTexture(GL_TEXTURE_2D, tex)
    gl.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, None)
    gl.glGenFramebuffers(1, C.byref(fbo))
    gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo)
    if case == "211":
        ms = C.CFUNCTYPE(None, C.c_uint, C.c_uint, C.c_uint, C.c_uint, C.c_int, C.c_int)(
            egl.eglGetProcAddress(b"glFramebufferTexture2DMultisampleEXT"))
        ms(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0, 4)
    else:
        gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0)

    def shader(kind, src):
        sh = gl.glCreateShader(kind)
        p = C.c_char_p(src)
        gl.glShaderSource(sh, 1, C.byref(p), None)
        gl.glCompileShader(sh)
        return sh

    prog = gl.glCreateProgram()
    gl.glAttachShader(prog, shader(GL_VERTEX_SHADER, VS))
    gl.glAttachShader(prog, shader(GL_FRAGMENT_SHADER, FS))
    gl.glBindAttribLocation(prog, 0, b"a_pos")
    gl.glLinkProgram(prog)
    gl.glUseProgram(prog)
    gl.glGetUniformLocation.argtypes = [C.c_uint, C.c_char_p]
    ucolor = gl.glGetUniformLocation(prog, b"u_color")
    gl.glViewport(0, 0, 64, 64)
    gl.glEnableVertexAttribArray(0)

    def quad(x0, y0, x1, y1, color):
        v = (C.c_float * 12)(x0, y0, x1, y0, x0, y1, x0, y1, x1, y0, x1, y1)
        gl.glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, v)
        gl.glUniform4f(ucolor, *[C.c_float(c) for c in color])
        gl.glDrawArrays(GL_TRIANGLES, 0, 6)

    f = C.c_float
    if case == "190":
        src = (0.75, 0.5, 0.25, 0.25)
        gl.glClearColor(f(0.25), f(0.5), f(0.75), f(0.5))
        gl.glClear(GL_COLOR_BUFFER_BIT)
        gl.glEnable(GL_BLEND)
        gl.glBlendEquation(FUNC_ADD)
        gl.glBlendFuncSeparate(SRC_ALPHA, ONE_MINUS_SRC_ALPHA, SRC_ALPHA, ONE_MINUS_SRC_ALPHA)
        quad(-1, -1, -0.5, 1, src)  # column 1: fixed function
        gl.glColorMask(0, 1, 1, 1)
        quad(-0.5, -1, 0, 1, src)  # column 2: fixed function, red masked
        gl.glColorMask(1, 1, 1, 1)
        gl.glBlendEquationSeparate(MIN, FUNC_ADD)
        quad(0, -1, 0.5, 1, src)  # column 3: MIN on the colour, a blend shader
        gl.glBlendEquation(FUNC_ADD)
        gl.glBlendFuncSeparate(ONE_MINUS_DST_ALPHA, ONE_MINUS_SRC_ALPHA, ONE, ZERO)
        quad(0.5, -1, 1, 1, src)  # column 4: 1-Ad / 1-As, a blend shader
    else:
        # the red triangle (no blend), then the blended green quad of alpha 0.5
        gl.glClearColor(f(0), f(0), f(0), f(1))
        gl.glClear(GL_COLOR_BUFFER_BIT)
        v = (C.c_float * 6)(-1, -1, 1, -1, 1, 1)
        gl.glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, v)
        gl.glUniform4f(ucolor, f(1), f(0), f(0), f(1))
        gl.glDrawArrays(GL_TRIANGLES, 0, 3)
        gl.glEnable(GL_BLEND)
        gl.glBlendColor(f(0.25), f(0.5), f(0.75), f(1))
        gl.glBlendFunc(CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR)
        quad(-1, -1, 1, 1, (0, 1, 0, 0.5))
    gl.glFlush()
    print("blend_case: case %s, glGetError 0x%x" % (case, gl.glGetError()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
