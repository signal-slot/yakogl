#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Draw a clear, or a clear and one triangle, through Mesa's GL driver, so the
driver builds and submits one V3D job (the GL side of tools/v3d/drm_shim_ref.sh).

    tools/v3d/gl_ref.py <libdir> clear|triangle [--size WxH]

<libdir> holds the libEGL.so.1 and libGLESv2.so.2 to use. The program runs on an
EGL surfaceless display (EGL_PLATFORM_SURFACELESS_MESA, an OpenGL ES 2.0
context) into a WxH RGBA8 framebuffer object (64x64 by default) and then
glFlush()es, so that the driver submits the job. It reads nothing back, as it
runs on the drm-shim, where no job ever completes.

  clear     glClearColor(0, 0, 1, 1) and glClear(GL_COLOR_BUFFER_BIT). Mesa makes
            this a job with a clear in its render list and an empty binning list
            (v3d_tlb_clear).
  triangle  the same clear, then one Gouraud triangle (a passthrough vertex
            shader and an interpolated-colour fragment shader, the pair of
            tools/shaders/passthrough.vert and interp_color.frag in GLSL ES
            1.00) with fixed vertices and colours, glDrawArrays(GL_TRIANGLES).

The vertex data is fixed in this file so that the job's bytes are the same on
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
GL_TEXTURE_2D, GL_RGBA, GL_UNSIGNED_BYTE = 0x0DE1, 0x1908, 0x1401
GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_FRAMEBUFFER_COMPLETE = 0x8D40, 0x8CE0, 0x8CD5
GL_COLOR_BUFFER_BIT, GL_FLOAT, GL_RENDERER, GL_TRIANGLES = 0x4000, 0x1406, 0x1F01, 0x0004

VS = b"""attribute vec4 a_pos;
attribute vec4 a_color;
varying vec4 v_color;
void main() {
    gl_Position = a_pos;
    v_color = a_color;
}
"""
FS = b"""precision mediump float;
varying vec4 v_color;
void main() { gl_FragColor = v_color; }
"""
# clip-space positions and colours of the three vertices (vec4 each)
POSITIONS = (-0.75, -0.75, 0.0, 1.0, 0.75, -0.75, 0.0, 1.0, 0.0, 0.75, 0.0, 1.0)
COLORS = (1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0)


def main():
    args = sys.argv[1:]
    size = (64, 64)
    if "--size" in args:
        i = args.index("--size")
        try:
            size = tuple(int(x) for x in args[i + 1].split("x"))
        except (ValueError, IndexError):
            size = ()
        del args[i:i + 2]
    if len(args) != 2 or args[1] not in ("clear", "triangle") or len(size) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    libdir, case = args
    width, height = size[0], size[1]
    egl = C.CDLL(os.path.join(libdir, "libEGL.so.1"))
    gl = C.CDLL(os.path.join(libdir, "libGLESv2.so.2"))
    egl.eglGetProcAddress.restype = C.c_void_p
    egl.eglGetProcAddress.argtypes = [C.c_char_p]
    gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(
        egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
    dpy = gpd(EGL_PLATFORM_SURFACELESS_MESA, None, None)
    egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
    if not dpy or not egl.eglInitialize(dpy, None, None):
        print("gl_ref: eglInitialize failed", file=sys.stderr)
        return 1
    egl.eglBindAPI(EGL_OPENGL_ES_API)
    attrs = (C.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE)
    egl.eglCreateContext.restype = C.c_void_p
    egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    ctx = egl.eglCreateContext(dpy, None, None, attrs)
    egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
        print("gl_ref: no GLES 2 context", file=sys.stderr)
        return 1
    gl.glGetString.restype = C.c_char_p
    print("gl_ref: renderer %s" % gl.glGetString(GL_RENDERER).decode())

    tex, fbo = C.c_uint(), C.c_uint()
    gl.glGenTextures(1, C.byref(tex))
    gl.glBindTexture(GL_TEXTURE_2D, tex)
    gl.glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, width, height, 0, GL_RGBA, GL_UNSIGNED_BYTE, None)
    gl.glGenFramebuffers(1, C.byref(fbo))
    gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo)
    gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0)
    if gl.glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE:
        print("gl_ref: framebuffer incomplete", file=sys.stderr)
        return 1
    gl.glViewport(0, 0, width, height)
    gl.glClearColor(C.c_float(0), C.c_float(0), C.c_float(1), C.c_float(1))
    gl.glClear(GL_COLOR_BUFFER_BIT)

    if case == "triangle":
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
                raise SystemExit("gl_ref: shader does not compile: %s" % log.value.decode())
            return sh

        prog = gl.glCreateProgram()
        gl.glAttachShader(prog, shader(GL_VERTEX_SHADER, VS))
        gl.glAttachShader(prog, shader(GL_FRAGMENT_SHADER, FS))
        gl.glBindAttribLocation(prog, 0, b"a_pos")
        gl.glBindAttribLocation(prog, 1, b"a_color")
        gl.glLinkProgram(prog)
        ok = C.c_int()
        gl.glGetProgramiv(prog, GL_LINK_STATUS, C.byref(ok))
        if not ok.value:
            log = C.create_string_buffer(4096)
            gl.glGetProgramInfoLog(prog, 4096, None, log)
            raise SystemExit("gl_ref: link failed: %s" % log.value.decode())
        gl.glUseProgram(prog)
        pos = (C.c_float * 12)(*POSITIONS)
        col = (C.c_float * 12)(*COLORS)
        gl.glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, pos)
        gl.glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, col)
        gl.glEnableVertexAttribArray(0)
        gl.glEnableVertexAttribArray(1)
        gl.glDrawArrays(GL_TRIANGLES, 0, 3)
    gl.glFlush()
    err = gl.glGetError()
    print("gl_ref: %s, framebuffer %dx%d, glGetError 0x%x" % (case, width, height, err))
    return 0 if err == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
