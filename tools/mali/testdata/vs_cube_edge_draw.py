#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
"""One draw of a vs_cube_edge_ref.sh case on Mesa's drm-shim (issue #7489).

    vs_cube_edge_draw.py <lib dir> <testdata dir> <case>
    vs_cube_edge_draw.py --list

Makes a GLES 2 context (surfaceless EGL), builds the case's program from
<testdata dir>/<prog>.vert and .frag, makes the case's cube map at unit 0
(every face and level, zero texels: the texels change no descriptor), sets
its sampler state and uniforms, draws one triangle into a 256x256 RGBA8
framebuffer object and flushes. The trace's last draw is the case's.

Prints `vs_cube_edge_draw: GL_VERSION <string>` (whether the context is the
ES 3.2 one Mesa gives a GLES 2 request, or ES 2.0 under
MESA_GLES_VERSION_OVERRIDE=2.0) and one line per attribute and uniform.

The cases (CASES below), transcribed from VK-GL-CTS 93bca01861b0 (the
CTS_COMMIT of tools/cts/mali-deqp-build.sh):

- vs_cube_edge.*: texture.vertex.cube.filtering.linear_mipmap_linear_
  {linear,nearest}_repeat and .wrap.repeat_repeat (VertexCubeTextureCase,
  es2fVertexTextureTests.cpp): a GL_RGB / GL_UNSIGNED_BYTE cube of 32x32
  with 6 levels, min GL_LINEAR_MIPMAP_LINEAR, the case's mag filter,
  GL_REPEAT on s and t, u_lod of the first cell (log2(32 / 28)). `edge`
  draws three directions on the +X face within 0.01 of its edge with +Y
  (the cells at a face's border of dEQP's 63x63 grid), `centre` three near
  the face's centre: the program and descriptors do not depend on the
  direction (vs_cube_edge_ref.sh checks it).
- cube_lod_vs.*: shaders.texture_functions.vertex.texturecubelod
  (es2fShaderTextureFunctionTests.cpp, texCubeMipmap): a GL_RGBA /
  GL_UNSIGNED_BYTE cube of 256x256 with 9 levels, min
  GL_LINEAR_MIPMAP_NEAREST, mag GL_LINEAR, GL_REPEAT on s and t; a_in0
  from (-1, -1, 1.01) to (1, 1, 1.01) (the +Z face to its four edges),
  a_in1 the level of detail from -1 to 9.
- cube_lod_fs.*: the same map, sampler and inputs, the look-up in the
  fragment stage. dEQP-GLES2 has no fragment texturecubelod
  (invalid.texturecubelod_in_fragment checks that GLSL ES 1.00 refuses
  it), and Mesa 26.2.1's panfrost does not expose GL_EXT_shader_texture_lod
  ("extension unsupported in fragment shader"), so this pair is GLSL ES
  3.00 (textureLod on a samplerCube) and runs in the ES 3 context only.

`--contexts <case>` prints the contexts vs_cube_edge_ref.sh draws the case
in; `--title <case>` its one-line description.
"""
import ctypes as C
import sys

GL = {"NEAREST": 0x2600, "LINEAR": 0x2601, "LINEAR_MIPMAP_NEAREST": 0x2701,
      "LINEAR_MIPMAP_LINEAR": 0x2703, "CLAMP_TO_EDGE": 0x812F, "REPEAT": 0x2901,
      "RGB": 0x1907, "RGBA": 0x1908}

# u_lod of the vertex cube's first cell: log2(32 / 28)
VCUBE_LOD = 0.19264507
# directions on the +X face (major axis x = 1): t = -y, s = -z
EDGE = [1.0, 0.995, -0.5, 1.0, 0.995, 0.5, 1.0, 0.9999, 0.0]
CENTRE = [1.0, 0.0, 0.0, 1.0, 0.1, 0.0, 1.0, 0.0, 0.1]
# texturecubelod: the +Z face (z = 1.01) out to its edges, levels -1 .. 9
LOD_COORD = [-1.0, -1.0, 1.01, 1.0, -1.0, 1.01, 1.0, 1.0, 1.01]
LOD_LEVEL = [-1.0, 4.0, 9.0]
POS2 = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]
POS4 = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0]


def vcube(mag, coord):
    return {"prog": "vs_cube_edge", "contexts": ("es3", "es2"), "size": 32, "levels": 6, "format": "RGB",
            "min": "LINEAR_MIPMAP_LINEAR", "mag": mag, "wrap": ("REPEAT", "REPEAT"),
            "attribs": [("a_position", 2, POS2), ("a_texCoord", 3, coord)],
            "uniforms": [("u_texture", "i", [0]), ("u_lod", "f", [VCUBE_LOD])]}


def cubelod(prog, contexts=("es3", "es2")):
    return {"prog": prog, "contexts": contexts, "size": 256, "levels": 9, "format": "RGBA",
            "min": "LINEAR_MIPMAP_NEAREST", "mag": "LINEAR", "wrap": ("REPEAT", "REPEAT"),
            "attribs": [("a_position", 4, POS4), ("a_in0", 3, LOD_COORD), ("a_in1", 1, LOD_LEVEL)],
            "uniforms": [("u_sampler", "i", [0])]}


CASES = {
    "vs_cube_edge.lml_linear_repeat.edge": vcube("LINEAR", EDGE),
    "vs_cube_edge.lml_linear_repeat.centre": vcube("LINEAR", CENTRE),
    "vs_cube_edge.lml_nearest_repeat.edge": vcube("NEAREST", EDGE),
    "cube_lod_vs.lmn_linear_repeat": cubelod("cube_lod_vs"),
    "cube_lod_fs.lmn_linear_repeat": cubelod("cube_lod_fs", ("es3",)),
}


def title(name):
    c = CASES[name]
    return "%s: cube %dx%d, %d levels, %s; min %s, mag %s, wrap s %s, wrap t %s" % (
        name, c["size"], c["size"], c["levels"], c["format"], c["min"], c["mag"], c["wrap"][0], c["wrap"][1])


def main():
    if sys.argv[1:] == ["--list"]:
        print("\n".join(CASES))
        return
    if sys.argv[1:2] == ["--title"]:
        print(title(sys.argv[2]))
        return
    if sys.argv[1:2] == ["--contexts"]:
        print(" ".join(CASES[sys.argv[2]]["contexts"]))
        return
    libdir, data, name = sys.argv[1:]
    case = CASES[name]
    egl = C.CDLL(libdir + "/libEGL.so.1")
    gl = C.CDLL(libdir + "/libGLESv2.so.2")
    egl.eglGetProcAddress.restype = C.c_void_p
    egl.eglGetProcAddress.argtypes = [C.c_char_p]
    gpd = C.CFUNCTYPE(C.c_void_p, C.c_uint, C.c_void_p, C.c_void_p)(
        egl.eglGetProcAddress(b"eglGetPlatformDisplayEXT"))
    dpy = gpd(0x31DD, None, None)  # EGL_PLATFORM_SURFACELESS_MESA
    egl.eglInitialize.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p]
    if not dpy or not egl.eglInitialize(dpy, None, None):
        raise SystemExit("draw: eglInitialize failed")
    egl.eglBindAPI(0x30A0)
    egl.eglCreateContext.restype = C.c_void_p
    egl.eglCreateContext.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    # dEQP-GLES2 asks for a GLES 2 context; Mesa makes its ES 3.x one unless
    # MESA_GLES_VERSION_OVERRIDE=2.0 (vs_cube_edge_ref.sh's es2 context)
    ctx = egl.eglCreateContext(dpy, None, None, (C.c_int * 3)(0x3098, 2, 0x3038))
    egl.eglMakeCurrent.argtypes = [C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p]
    if not ctx or not egl.eglMakeCurrent(dpy, None, None, ctx):
        raise SystemExit("draw: no GLES 2 context")
    gl.glGetString.restype = C.c_char_p
    print("vs_cube_edge_draw: GL_VERSION %s" % gl.glGetString(0x1F02).decode())
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
            log = C.create_string_buffer(4096)
            gl.glGetShaderInfoLog(sh, 4096, None, log)
            raise SystemExit("draw: %s does not compile: %s" % (path, log.value.decode()))
        return sh

    prog = gl.glCreateProgram()
    gl.glAttachShader(prog, shader(0x8B31, "%s/%s.vert" % (data, case["prog"])))
    gl.glAttachShader(prog, shader(0x8B30, "%s/%s.frag" % (data, case["prog"])))
    for loc, (aname, _, _) in enumerate(case["attribs"]):
        gl.glBindAttribLocation(prog, loc, aname.encode())
    gl.glLinkProgram(prog)
    ok = C.c_int()
    gl.glGetProgramiv(prog, 0x8B82, C.byref(ok))
    if not ok.value:
        raise SystemExit("draw: link failed")
    gl.glUseProgram(prog)

    size, levels, fmt = case["size"], case["levels"], GL[case["format"]]
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
    for pname, value in ((0x2802, case["wrap"][0]), (0x2803, case["wrap"][1]),
                         (0x2801, case["min"]), (0x2800, case["mag"])):
        gl.glTexParameteri(0x8513, pname, GL[value])

    gl.glGetUniformLocation.argtypes = [C.c_uint, C.c_char_p]
    for uname, kind, vals in case["uniforms"]:
        loc = gl.glGetUniformLocation(prog, uname.encode())
        if loc < 0:
            raise SystemExit("draw: no uniform %s" % uname)
        if kind == "i":
            gl.glUniform1i(loc, vals[0])
        else:
            gl.glUniform1fv(loc, 1, (C.c_float * 1)(*vals))
        print("vs_cube_edge_draw: uniform %s = %s" % (uname, " ".join(repr(v) for v in vals)))
    keep = []
    for loc, (aname, comps, vals) in enumerate(case["attribs"]):
        arr = (C.c_float * len(vals))(*vals)
        keep.append(arr)
        gl.glVertexAttribPointer(loc, comps, 0x1406, 0, 0, arr)
        gl.glEnableVertexAttribArray(loc)
        print("vs_cube_edge_draw: attribute %d %s = %s" % (loc, aname, " ".join(repr(v) for v in vals)))
    # a surfaceless context starts with a 0x0 viewport, which culls the draw
    gl.glViewport(0, 0, 256, 256)
    gl.glClearColor(C.c_float(0.125), C.c_float(0.25), C.c_float(0.5), C.c_float(1.0))
    gl.glClear(0x4000)
    gl.glDrawArrays(0x0004, 0, 3)
    gl.glFlush()
    err = gl.glGetError()
    if err:
        raise SystemExit("draw: glGetError 0x%x" % err)


if __name__ == "__main__":
    main()
