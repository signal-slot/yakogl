#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""Run the GL calls of one dEQP-GLES2 fbo.render case on a Mesa GL driver,
so a pandecode trace (tools/mali/pan_zs_ref.sh) holds the frames Mesa makes
for it.

    zs_seq.py <libdir> <case>     run it
    zs_seq.py --list              the cases, one key a line

<libdir> holds the libEGL.so.1 and libGLESv2.so.2 to use; the context is an
OpenGL ES 2.0 one on an EGL surfaceless display, so every frame is of a
framebuffer object. A case's key is its dEQP name under
dEQP-GLES2.functional.fbo.render. with `.` as `__`. The GL calls are
transcribed from VK-GL-CTS modules/gles2/functional/es2fFboRenderTest.cpp:
SharedDepthbufferTest, RecreateBuffersTest<GL_DEPTH_BUFFER_BIT> and
<GL_STENCIL_BUFFER_BIT> (rebind_ / no_rebind_), StencilClearsTest and the
Framebuffer helper (colour, then depth, then stencil attached; the
framebuffer unbound at the end; checkCompleteness binds it, asks
glCheckFramebufferStatus and ends the case as NotSupported on
GL_FRAMEBUFFER_UNSUPPORTED, which this program does too: it prints the
status and stops). A quad is sglr::drawQuad's: attribute a_position (vec4,
the corners' z p0.z, mid, mid, p1.z) and a_coord, glDrawElements of six
GL_UNSIGNED_SHORT indices 0,2,1,2,1,3 from client memory. The case's
textures (dEQP's metaballs and quads patterns) have the case's size and
format but hold a plain ramp: no descriptor this reference keeps holds a
texel. Where the case reads the framebuffer back (glReadPixels, or draws a
colour texture to the screen) this program glFlush()es instead: on drm-shim
no job completes, so nothing could be read, and the flush submits every
frame Mesa holds, as the read-back would. Each step prints a `zs_seq:` line
with the GL calls it made, so a frame of the trace is told by the calls
before it. Exits non-zero on any EGL or GL failure.
"""
import ctypes as C
import os
import sys

EGL_PLATFORM_SURFACELESS_MESA, EGL_OPENGL_ES_API = 0x31DD, 0x30A0
EGL_CONTEXT_CLIENT_VERSION, EGL_NONE = 0x3098, 0x3038
GL_VERTEX_SHADER, GL_FRAGMENT_SHADER = 0x8B31, 0x8B30
GL_COMPILE_STATUS, GL_LINK_STATUS = 0x8B81, 0x8B82
GL_TEXTURE_2D, GL_TEXTURE0, GL_RGB, GL_RGBA, GL_UNSIGNED_BYTE = 0x0DE1, 0x84C0, 0x1907, 0x1908, 0x1401
GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER, GL_LINEAR = 0x2801, 0x2800, 0x2601
GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE = 0x2802, 0x2803, 0x812F
GL_FRAMEBUFFER, GL_RENDERBUFFER, GL_FRAMEBUFFER_COMPLETE = 0x8D40, 0x8D41, 0x8CD5
GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT = 0x8CE0, 0x8D00, 0x8D20
GL_RGB565, GL_DEPTH_COMPONENT16, GL_STENCIL_INDEX8 = 0x8D62, 0x81A5, 0x8D48
GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT, GL_STENCIL_BUFFER_BIT = 0x4000, 0x0100, 0x0400
GL_DEPTH_TEST, GL_STENCIL_TEST, GL_SCISSOR_TEST, GL_EQUAL = 0x0B71, 0x0B90, 0x0C11, 0x0202
GL_TRIANGLES, GL_UNSIGNED_SHORT, GL_FLOAT, GL_RENDERER = 0x0004, 0x1403, 0x1406, 0x1F01
NAMES = {GL_RGB565: "GL_RGB565", GL_DEPTH_COMPONENT16: "GL_DEPTH_COMPONENT16",
         GL_STENCIL_INDEX8: "GL_STENCIL_INDEX8", GL_RGBA: "GL_RGBA",
         GL_DEPTH_BUFFER_BIT: "GL_DEPTH_BUFFER_BIT", GL_STENCIL_BUFFER_BIT: "GL_STENCIL_BUFFER_BIT",
         GL_DEPTH_ATTACHMENT: "GL_DEPTH_ATTACHMENT", GL_STENCIL_ATTACHMENT: "GL_STENCIL_ATTACHMENT",
         0x8CDD: "GL_FRAMEBUFFER_UNSUPPORTED", GL_FRAMEBUFFER_COMPLETE: "GL_FRAMEBUFFER_COMPLETE"}
ALL = "GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT"

# SingleTex2DShader and FlatColorShader (es2fFboRenderTest.cpp)
TEX_VS = b"""attribute highp vec4 a_position;
attribute mediump vec2 a_coord;
varying mediump vec2 v_coord;
void main (void)
{
    gl_Position = a_position;
    v_coord = a_coord;
}
"""
TEX_FS = b"""uniform sampler2D u_sampler0;
varying mediump vec2 v_coord;
void main (void)
{
    gl_FragColor = texture2D(u_sampler0, v_coord);
}
"""
FLAT_VS = b"""attribute highp vec4 a_position;
void main (void)
{
    gl_Position = a_position;
}
"""
FLAT_FS = b"""uniform mediump vec4 u_color;
void main (void)
{
    gl_FragColor = u_color;
}
"""


class Unsupported(Exception):
    """dEQP's checkCompleteness: the case ends NotSupported here."""


class Gl:
    def __init__(self, libdir):
        self.egl = C.CDLL(os.path.join(libdir, "libEGL.so.1"))
        self.gl = C.CDLL(os.path.join(libdir, "libGLESv2.so.2"))
        for name, args in (("glClearColor", [C.c_float] * 4),
                           ("glUniform4f", [C.c_int] + [C.c_float] * 4)):
            getattr(self.gl, name).argtypes = args
        self.gl.glGetString.restype = C.c_char_p
        self.gl.glGetUniformLocation.argtypes = [C.c_uint, C.c_char_p]
        self.gl.glGetAttribLocation.argtypes = [C.c_uint, C.c_char_p]

    def __getattr__(self, name):
        return getattr(self.gl, name)

    def context(self):
        egl = self.egl
        egl.eglGetProcAddress.restype = C.c_void_p
        egl.eglGetProcAddress.argtypes = [C.c_char_p]
        gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(
            egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
        dpy = gpd(EGL_PLATFORM_SURFACELESS_MESA, None, None)
        egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
        if not dpy or not egl.eglInitialize(dpy, None, None):
            raise SystemExit("zs_seq: eglInitialize failed")
        egl.eglBindAPI(EGL_OPENGL_ES_API)
        egl.eglCreateContext.restype = C.c_void_p
        egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
        ctx = egl.eglCreateContext(dpy, None, None, (C.c_int * 3)(EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE))
        egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
        if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
            raise SystemExit("zs_seq: no GLES 2 context")
        print("zs_seq: renderer %s" % self.gl.glGetString(GL_RENDERER).decode())


def step(text):
    print("zs_seq: " + text)


def program(gl, vs, fs):
    prog = gl.glCreateProgram()
    for kind, src in ((GL_VERTEX_SHADER, vs), (GL_FRAGMENT_SHADER, fs)):
        sh = gl.glCreateShader(kind)
        p = C.c_char_p(src)
        gl.glShaderSource(sh, 1, C.byref(p), None)
        gl.glCompileShader(sh)
        ok = C.c_int()
        gl.glGetShaderiv(sh, GL_COMPILE_STATUS, C.byref(ok))
        if not ok.value:
            raise SystemExit("zs_seq: a shader does not compile")
        gl.glAttachShader(prog, sh)
    gl.glLinkProgram(prog)
    ok = C.c_int()
    gl.glGetProgramiv(prog, GL_LINK_STATUS, C.byref(ok))
    if not ok.value:
        raise SystemExit("zs_seq: link failed")
    return prog


def texture(gl, fmt, size):
    t = C.c_uint()
    gl.glGenTextures(1, C.byref(t))
    gl.glBindTexture(GL_TEXTURE_2D, t)
    n = 3 if fmt == GL_RGB else 4
    texels = (C.c_ubyte * (n * size * size))(*[i & 0xff for i in range(n * size * size)])
    gl.glTexImage2D(GL_TEXTURE_2D, 0, fmt, size, size, 0, fmt, GL_UNSIGNED_BYTE, texels)
    for pname, value in ((GL_TEXTURE_MIN_FILTER, GL_LINEAR), (GL_TEXTURE_MAG_FILTER, GL_LINEAR),
                         (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE), (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)):
        gl.glTexParameteri(GL_TEXTURE_2D, pname, value)
    step("texture %d: %dx%d %s, GL_LINEAR, GL_CLAMP_TO_EDGE" % (t.value, size, size,
                                                               "GL_RGB" if fmt == GL_RGB else "GL_RGBA"))
    return t.value


def renderbuffer(gl, fmt, w, h, name=0):
    if not name:
        rb = C.c_uint()
        gl.glGenRenderbuffers(1, C.byref(rb))
        name = rb.value
    gl.glBindRenderbuffer(GL_RENDERBUFFER, name)
    gl.glRenderbufferStorage(GL_RENDERBUFFER, fmt, w, h)
    return name


class Fbo:
    """es2fFboRenderTest.cpp's Framebuffer."""

    def __init__(self, gl, colour, depth, stencil, w=128, h=128):
        f = C.c_uint()
        gl.glGenFramebuffers(1, C.byref(f))
        self.fbo = f.value
        gl.glBindFramebuffer(GL_FRAMEBUFFER, self.fbo)
        if colour == GL_RGBA:
            self.colour = texture(gl, GL_RGBA, w)
            gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, self.colour, 0)
            what = "texture %d (GL_RGBA %dx%d)" % (self.colour, w, h)
        else:
            self.colour = renderbuffer(gl, colour, w, h)
            gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, self.colour)
            what = "renderbuffer %d (%s %dx%d)" % (self.colour, NAMES[colour], w, h)
        parts = ["GL_COLOR_ATTACHMENT0 " + what]
        self.depth = self.stencil = 0
        if depth:
            self.depth = renderbuffer(gl, GL_DEPTH_COMPONENT16, w, h)
            gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, self.depth)
            parts.append("GL_DEPTH_ATTACHMENT renderbuffer %d (GL_DEPTH_COMPONENT16 %dx%d)" % (self.depth, w, h))
        if stencil:
            self.stencil = renderbuffer(gl, GL_STENCIL_INDEX8, w, h)
            gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, self.stencil)
            parts.append("GL_STENCIL_ATTACHMENT renderbuffer %d (GL_STENCIL_INDEX8 %dx%d)" % (self.stencil, w, h))
        gl.glBindFramebuffer(GL_FRAMEBUFFER, 0)
        step("framebuffer %d: %s; glBindFramebuffer(0)" % (self.fbo, ", ".join(parts)))

    def complete(self, gl):
        gl.glBindFramebuffer(GL_FRAMEBUFFER, self.fbo)
        status = gl.glCheckFramebufferStatus(GL_FRAMEBUFFER)
        step("glBindFramebuffer(%d) glCheckFramebufferStatus = 0x%x %s" % (
            self.fbo, status, NAMES.get(status, "")))
        if status == 0x8CDD:
            raise Unsupported()
        if status != GL_FRAMEBUFFER_COMPLETE:
            raise SystemExit("zs_seq: framebuffer %d incomplete" % self.fbo)
        gl.glBindFramebuffer(GL_FRAMEBUFFER, 0)
        step("glBindFramebuffer(0)")


class Quad:
    """sglr::drawQuad with a program of a_position (and a_coord)."""

    INDICES = (C.c_ushort * 6)(0, 2, 1, 2, 1, 3)
    COORDS = (C.c_float * 8)(0, 0, 0, 1, 1, 0, 1, 1)

    def __init__(self, gl):
        self.gl = gl
        self.keep = []

    def __call__(self, prog, p0, p1, what):
        gl = self.gl
        mid = 0.5 * (p0[2] + p1[2])
        pos = (C.c_float * 16)(p0[0], p0[1], p0[2], 1, p0[0], p1[1], mid, 1,
                               p1[0], p0[1], mid, 1, p1[0], p1[1], p1[2], 1)
        self.keep.append(pos)
        gl.glUseProgram(prog)
        loc = gl.glGetAttribLocation(prog, b"a_position")
        gl.glVertexAttribPointer(loc, 4, GL_FLOAT, 0, 0, pos)
        gl.glEnableVertexAttribArray(loc)
        coord = gl.glGetAttribLocation(prog, b"a_coord")
        if coord >= 0:
            gl.glVertexAttribPointer(coord, 2, GL_FLOAT, 0, 0, self.COORDS)
            gl.glEnableVertexAttribArray(coord)
        gl.glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, self.INDICES)
        gl.glDisableVertexAttribArray(loc)
        if coord >= 0:
            gl.glDisableVertexAttribArray(coord)
        step("drawQuad (%s, z %g..%g): glDrawElements(GL_TRIANGLES, 6)" % (what, p0[2], p1[2]))


def shared_depthbuffer(gl, quad, stencil):
    """SharedDepthbufferTest::render, an RGB565 renderbuffer config."""
    w = h = 128
    tex, flat = program(gl, TEX_VS, TEX_FS), program(gl, FLAT_VS, FLAT_FS)
    gl.glUseProgram(tex)
    gl.glUniform1i(gl.glGetUniformLocation(tex, b"u_sampler0"), 0)
    gl.glUseProgram(flat)
    gl.glUniform4f(gl.glGetUniformLocation(flat, b"u_color"), 0.0, 1.0, 0.0, 1.0)
    step("programs: SingleTex2DShader (u_sampler0 = 0), FlatColorShader (u_color = 0,1,0,1)")
    metaballs, quads = texture(gl, GL_RGB, 64), texture(gl, GL_RGB, 64)
    gl.glViewport(0, 0, w, h)
    step("glViewport(0, 0, %d, %d)" % (w, h))
    a = Fbo(gl, GL_RGB565, True, stencil)
    a.complete(gl)
    b = Fbo(gl, GL_RGB565, False, stencil)
    gl.glBindFramebuffer(GL_FRAMEBUFFER, b.fbo)
    gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, a.depth)
    step("glBindFramebuffer(%d) (B) glFramebufferRenderbuffer(GL_DEPTH_ATTACHMENT, %d) (A's depth)"
         % (b.fbo, a.depth))
    gl.glClearColor(1.0, 0.0, 0.0, 1.0)
    gl.glClearStencil(1)
    gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
    step("glClearColor(1, 0, 0, 1) glClearStencil(1) glClear(%s)" % ALL)
    gl.glEnable(GL_DEPTH_TEST)
    gl.glBindFramebuffer(GL_FRAMEBUFFER, a.fbo)
    gl.glBindTexture(GL_TEXTURE_2D, quads)
    gl.glClearColor(0.0, 0.0, 0.0, 1.0)
    gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
    step("glEnable(GL_DEPTH_TEST) glBindFramebuffer(%d) (A) glBindTexture(%d) glClearColor(0, 0, 0, 1) "
         "glClear(%s)" % (a.fbo, quads, ALL))
    quad(tex, (-1, -1, 0), (1, 1, 0), "SingleTex2DShader")
    gl.glBindFramebuffer(GL_FRAMEBUFFER, b.fbo)
    gl.glBindTexture(GL_TEXTURE_2D, metaballs)
    step("glBindFramebuffer(%d) (B) glBindTexture(%d)" % (b.fbo, metaballs))
    quad(tex, (-1, -1, -1), (1, 1, 1), "SingleTex2DShader")
    gl.glDisable(GL_DEPTH_TEST)
    step("glDisable(GL_DEPTH_TEST)")
    if stencil:
        gl.glEnable(GL_SCISSOR_TEST)
        gl.glScissor(10, 10, 12, 25)
        gl.glClearStencil(0)
        gl.glClear(GL_STENCIL_BUFFER_BIT)
        gl.glDisable(GL_SCISSOR_TEST)
        step("glEnable(GL_SCISSOR_TEST) glScissor(10, 10, 12, 25) glClearStencil(0) "
             "glClear(GL_STENCIL_BUFFER_BIT) glDisable(GL_SCISSOR_TEST)")
        gl.glEnable(GL_STENCIL_TEST)
        gl.glStencilFunc(GL_EQUAL, 0, 0xff)
        step("glEnable(GL_STENCIL_TEST) glStencilFunc(GL_EQUAL, 0, 0xff)")
        quad(flat, (-1, -1, 0), (1, 1, 0), "FlatColorShader")
        gl.glDisable(GL_STENCIL_TEST)
        step("glDisable(GL_STENCIL_TEST)")


def recreate(gl, quad, buffers, rebind, depth, stencil):
    """RecreateBuffersTest<buffers>::render, an RGB565 renderbuffer config."""
    w = h = 128
    tex = program(gl, TEX_VS, TEX_FS)
    gl.glUseProgram(tex)
    gl.glUniform1i(gl.glGetUniformLocation(tex, b"u_sampler0"), 0)
    step("program: SingleTex2DShader (u_sampler0 = 0)")
    metaballs, quads = texture(gl, GL_RGB, 64), texture(gl, GL_RGB, 64)
    fbo = Fbo(gl, GL_RGB565, depth, stencil)
    fbo.complete(gl)
    gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo.fbo)
    gl.glViewport(0, 0, w, h)
    gl.glClearColor(1.0, 0.0, 0.0, 1.0)
    gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
    gl.glEnable(GL_DEPTH_TEST)
    gl.glBindTexture(GL_TEXTURE_2D, quads)
    step("glBindFramebuffer(%d) glViewport(0, 0, %d, %d) glClearColor(1, 0, 0, 1) glClear(%s) "
         "glEnable(GL_DEPTH_TEST) glBindTexture(%d)" % (fbo.fbo, w, h, ALL, quads))
    quad(tex, (-1, -1, 0), (1, 1, 0), "SingleTex2DShader")
    if stencil:
        gl.glEnable(GL_SCISSOR_TEST)
        gl.glScissor(w // 4, h // 4, w // 2, h // 2)
        gl.glClearStencil(1)
        gl.glClear(GL_STENCIL_BUFFER_BIT)
        gl.glDisable(GL_SCISSOR_TEST)
        step("glEnable(GL_SCISSOR_TEST) glScissor(%d, %d, %d, %d) glClearStencil(1) "
             "glClear(GL_STENCIL_BUFFER_BIT) glDisable(GL_SCISSOR_TEST)" % (w // 4, h // 4, w // 2, h // 2))
    if not rebind:
        gl.glBindFramebuffer(GL_FRAMEBUFFER, 0)
        step("glBindFramebuffer(0)")
    attach, fmt, name = ((GL_DEPTH_ATTACHMENT, GL_DEPTH_COMPONENT16, fbo.depth)
                         if buffers == GL_DEPTH_BUFFER_BIT else
                         (GL_STENCIL_ATTACHMENT, GL_STENCIL_INDEX8, fbo.stencil))
    rb = C.c_uint(name)
    gl.glDeleteRenderbuffers(1, C.byref(rb))
    renderbuffer(gl, fmt, w, h, name)
    step("glDeleteRenderbuffers(%d) glBindRenderbuffer(%d) glRenderbufferStorage(%s, %d, %d)" % (
        name, name, NAMES[fmt], w, h))
    if rebind:
        gl.glFramebufferRenderbuffer(GL_FRAMEBUFFER, attach, GL_RENDERBUFFER, name)
        step("glFramebufferRenderbuffer(%s, %d) (the delete detached it from the bound framebuffer %d; "
             "the new storage is attached)" % (NAMES[attach], name, fbo.fbo))
    else:
        gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo.fbo)
        step("glBindFramebuffer(%d) (deleted while unbound, it keeps the old image as its %s, as the spec "
             "says (Mesa fbobject.c:2262-2284); the new storage under name %d is attached nowhere)"
             % (fbo.fbo, NAMES[attach], name))
    gl.glClearColor(0.0, 0.0, 1.0, 0.0)
    gl.glClearStencil(0)
    gl.glClear(buffers)
    step("glClearColor(0, 0, 1, 0) glClearStencil(0) glClear(%s)" % NAMES[buffers])
    if stencil:
        gl.glEnable(GL_STENCIL_TEST)
        gl.glStencilFunc(GL_EQUAL, 0, 0xff)
        step("glEnable(GL_STENCIL_TEST) glStencilFunc(GL_EQUAL, 0, 0xff)")
    gl.glBindTexture(GL_TEXTURE_2D, metaballs)
    step("glBindTexture(%d)" % metaballs)
    quad(tex, (-1, -1, 1), (1, 1, -1), "SingleTex2DShader")
    if stencil:
        gl.glDisable(GL_STENCIL_TEST)
    gl.glDisable(GL_DEPTH_TEST)
    step("%sglDisable(GL_DEPTH_TEST)" % ("glDisable(GL_STENCIL_TEST) " if stencil else ""))


def stencil_clears(gl, quad, colour, depth):
    """StencilClearsTest::render."""
    w = h = 128
    tex = program(gl, TEX_VS, TEX_FS)
    step("program: SingleTex2DShader")
    quads, metaballs = texture(gl, GL_RGBA, 128), texture(gl, GL_RGBA, 128)
    fbo = Fbo(gl, colour, depth, True)
    fbo.complete(gl)
    gl.glBindFramebuffer(GL_FRAMEBUFFER, fbo.fbo)
    gl.glViewport(0, 0, w, h)
    gl.glClearColor(0.0, 0.0, 0.0, 1.0)
    gl.glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)
    step("glBindFramebuffer(%d) glViewport(0, 0, %d, %d) glClearColor(0, 0, 0, 1) glClear(%s)" % (
        fbo.fbo, w, h, ALL))
    gl.glEnable(GL_SCISSOR_TEST)
    gl.glScissor(10, 16, 32, 120)
    gl.glClearStencil(1)
    gl.glClear(GL_STENCIL_BUFFER_BIT)
    step("glEnable(GL_SCISSOR_TEST) glScissor(10, 16, 32, 120) glClearStencil(1) glClear(GL_STENCIL_BUFFER_BIT)")
    gl.glScissor(16, 32, 100, 64)
    gl.glClearStencil(2)
    gl.glClear(GL_STENCIL_BUFFER_BIT)
    gl.glDisable(GL_SCISSOR_TEST)
    step("glScissor(16, 32, 100, 64) glClearStencil(2) glClear(GL_STENCIL_BUFFER_BIT) glDisable(GL_SCISSOR_TEST)")
    gl.glActiveTexture(GL_TEXTURE0)
    gl.glBindTexture(GL_TEXTURE_2D, quads)
    gl.glActiveTexture(GL_TEXTURE0 + 1)
    gl.glBindTexture(GL_TEXTURE_2D, metaballs)
    gl.glEnable(GL_STENCIL_TEST)
    gl.glStencilFunc(GL_EQUAL, 1, 0xffffffff)
    gl.glUseProgram(tex)
    gl.glUniform1i(gl.glGetUniformLocation(tex, b"u_sampler0"), 0)
    step("unit 0 texture %d, unit 1 texture %d; glEnable(GL_STENCIL_TEST) glStencilFunc(GL_EQUAL, 1, ~0) "
         "u_sampler0 = 0" % (quads, metaballs))
    quad(tex, (-1, -1, 0), (1, 1, 0), "SingleTex2DShader")
    gl.glStencilFunc(GL_EQUAL, 2, 0xffffffff)
    gl.glUniform1i(gl.glGetUniformLocation(tex, b"u_sampler0"), 1)
    step("glStencilFunc(GL_EQUAL, 2, ~0) u_sampler0 = 1")
    quad(tex, (-1, -1, 0), (1, 1, 0), "SingleTex2DShader")
    gl.glDisable(GL_STENCIL_TEST)
    step("glDisable(GL_STENCIL_TEST)")


def cases():
    """key -> (dEQP case name, run(gl, quad))."""
    out = {}

    def add(name, run):
        out[name.replace(".", "__")] = (name, run)
    for stencil in (True, False):
        add("shared_depthbuffer.rbo_rgb565_depth_component16" + ("_stencil_index8" if stencil else ""),
            lambda gl, q, s=stencil: shared_depthbuffer(gl, q, s))
    for group, buffers in (("recreate_depthbuffer", GL_DEPTH_BUFFER_BIT),
                           ("recreate_stencilbuffer", GL_STENCIL_BUFFER_BIT)):
        for rebind in (True, False):
            for both in (True, False):
                depth = both or buffers == GL_DEPTH_BUFFER_BIT
                stencil = both or buffers == GL_STENCIL_BUFFER_BIT
                cfg = "_".join(n for n, on in (("depth_component16", depth), ("stencil_index8", stencil)) if on)
                add("%s.%srbo_rgb565_%s" % (group, "rebind_" if rebind else "no_rebind_", cfg),
                    lambda gl, q, b=buffers, r=rebind, d=depth, s=stencil: recreate(gl, q, b, r, d, s))
    for colour, cname in ((GL_RGB565, "rbo_rgb565"), (GL_RGBA, "tex2d_rgba")):
        for depth in (True, False):
            add("stencil_clear.%s_%sstencil_index8" % (cname, "depth_component16_" if depth else ""),
                lambda gl, q, c=colour, d=depth: stencil_clears(gl, q, c, d))
    return out


def main():
    table = cases()
    if sys.argv[1:] == ["--list"]:
        print("\n".join(table))
        return 0
    if len(sys.argv) != 3 or sys.argv[2] not in table:
        print(__doc__, file=sys.stderr)
        return 2
    name, run = table[sys.argv[2]]
    print("zs_seq: dEQP-GLES2.functional.fbo.render.%s" % name)
    gl = Gl(sys.argv[1])
    gl.context()
    try:
        run(gl, Quad(gl))
        result = "drawn"
    except Unsupported:
        result = "NotSupported (dEQP's checkCompleteness): no frame"
    gl.glFlush()
    step("glFlush() (the case's read-back)")
    err = gl.glGetError()
    print("zs_seq: %s: %s, glGetError 0x%x" % (name, result, err))
    return 0 if err == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
