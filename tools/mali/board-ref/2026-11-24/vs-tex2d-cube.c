// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Mesa's reference for #11308 (docs/es2-cts-failures.md M.1 row 17, the
// uniform_api ...sampler2D_samplerCube_vertex cases): a vertex shader that
// reads a sampler2D and a samplerCube, drawn through the board's Mesa
// (tools/mali/mesa-board-ref.sh's flags) into a 64x64 RGBA8 FBO.
//
//   ./vs-tex2d-cube
//
// u_tex2d (unit 1) is a 4x4 RGBA8 2D texture of (64, 0, 0, 255) and
// u_cube (unit 0) a 4x4 RGBA8 cube of (0, 128, 0, 255) on every face, both
// NEAREST / CLAMP_TO_EDGE. The units are crossed (2D on 1, cube on 0) as the
// uniform_api cases set them. Two triangles covering the target take their
// colour from the vertex stage: texture2D(u_tex2d, (0.5, 0.5)) +
// textureCube(u_cube, (0, 0, 1)) + (0, 0, 0.5, 0), so every pixel must be
// (64, 128, 128, 255) +-1. Prints the pixel count that differs and the
// FNV-1a of the read-back bytes; the verdict is PASS when none differs.
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define W 64
#define H 64

static GLuint sh(GLenum t, const char *s)
{
    GLuint o = glCreateShader(t);
    glShaderSource(o, 1, &s, NULL);
    glCompileShader(o);
    GLint ok;
    glGetShaderiv(o, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[1024];
        glGetShaderInfoLog(o, sizeof log, NULL, log);
        fprintf(stderr, "compile failed: %s\n", log);
        exit(1);
    }
    return o;
}

static void params(GLenum target)
{
    glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
}

static int near(int a, int b)
{
    return a - b <= 1 && b - a <= 1;
}

int main(void)
{
    PFNEGLGETPLATFORMDISPLAYEXTPROC gpd =
        (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    EGLDisplay d = gpd(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, NULL);
    if (!eglInitialize(d, NULL, NULL)) { fprintf(stderr, "eglInitialize\n"); return 1; }
    eglBindAPI(EGL_OPENGL_ES_API);
    const EGLint cfg_attrs[] = {
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE,
    };
    EGLConfig cfg;
    EGLint n = 0;
    if (!eglChooseConfig(d, cfg_attrs, &cfg, 1, &n) || n < 1) { fprintf(stderr, "no pbuffer config\n"); return 1; }
    const EGLint pb_attrs[] = { EGL_WIDTH, 16, EGL_HEIGHT, 16, EGL_NONE };
    EGLSurface pb = eglCreatePbufferSurface(d, cfg, pb_attrs);
    const EGLint ca[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
    EGLContext c = eglCreateContext(d, cfg, EGL_NO_CONTEXT, ca);
    if (pb == EGL_NO_SURFACE || !eglMakeCurrent(d, pb, pb, c)) { fprintf(stderr, "pbuffer/makecurrent\n"); return 1; }
    printf("renderer: %s\nversion: %s\nmode: vertex sampler2D (unit 1) + samplerCube (unit 0)\n",
           glGetString(GL_RENDERER), glGetString(GL_VERSION));
    GLint vtu = 0;
    glGetIntegerv(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, &vtu);
    printf("max vertex texture image units: %d\n", vtu);

    GLuint fbt, fb;
    glGenTextures(1, &fbt);
    glBindTexture(GL_TEXTURE_2D, fbt);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, W, H, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
    glGenFramebuffers(1, &fb);
    glBindFramebuffer(GL_FRAMEBUFFER, fb);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, fbt, 0);
    printf("status: 0x%04x\n", glCheckFramebufferStatus(GL_FRAMEBUFFER));

    static unsigned char red[16 * 4], green[16 * 4];
    for (int i = 0; i < 16; i++) {
        red[i * 4] = 64; red[i * 4 + 3] = 255;
        green[i * 4 + 1] = 128; green[i * 4 + 3] = 255;
    }
    GLuint t2, tc;
    glActiveTexture(GL_TEXTURE1);
    glGenTextures(1, &t2);
    glBindTexture(GL_TEXTURE_2D, t2);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, red);
    params(GL_TEXTURE_2D);
    glActiveTexture(GL_TEXTURE0);
    glGenTextures(1, &tc);
    glBindTexture(GL_TEXTURE_CUBE_MAP, tc);
    for (int f = 0; f < 6; f++)
        glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, 0, GL_RGBA, 4, 4, 0, GL_RGBA,
                     GL_UNSIGNED_BYTE, green);
    params(GL_TEXTURE_CUBE_MAP);

    GLuint p = glCreateProgram();
    glAttachShader(p, sh(GL_VERTEX_SHADER,
        "attribute vec2 a_pos;\nuniform sampler2D u_tex2d;\nuniform samplerCube u_cube;\n"
        "varying vec4 v_col;\n"
        "void main() {\n"
        "  v_col = texture2D(u_tex2d, vec2(0.5)) + textureCube(u_cube, vec3(0.0, 0.0, 1.0))\n"
        "        + vec4(0.0, 0.0, 0.5, 0.0);\n"
        "  gl_Position = vec4(a_pos, 0.0, 1.0);\n}\n"));
    glAttachShader(p, sh(GL_FRAGMENT_SHADER,
        "precision mediump float;\nvarying vec4 v_col;\nvoid main() { gl_FragColor = v_col; }\n"));
    glBindAttribLocation(p, 0, "a_pos");
    glLinkProgram(p);
    GLint ok;
    glGetProgramiv(p, GL_LINK_STATUS, &ok);
    if (!ok) { fprintf(stderr, "link failed\n"); return 1; }
    glUseProgram(p);
    glUniform1i(glGetUniformLocation(p, "u_tex2d"), 1);
    glUniform1i(glGetUniformLocation(p, "u_cube"), 0);

    const float v[] = { -1, -1, 1, -1, -1, 1, 1, 1 };
    glViewport(0, 0, W, H);
    glClearColor(0, 0, 0, 0);
    glClear(GL_COLOR_BUFFER_BIT);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, v);
    glEnableVertexAttribArray(0);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);

    static unsigned char px[W * H * 4];
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, px);
    printf("glGetError: 0x%04x\n", glGetError());
    unsigned h = 2166136261u;
    int bad = 0;
    for (unsigned i = 0; i < sizeof px; i++) h = (h ^ px[i]) * 16777619u;
    for (int i = 0; i < W * H; i++) {
        const unsigned char *q = px + i * 4;
        if (!near(q[0], 64) || !near(q[1], 128) || !near(q[2], 128) || q[3] != 255) bad++;
    }
    printf("pixel (32,32): %u %u %u %u\n", px[(32 * W + 32) * 4], px[(32 * W + 32) * 4 + 1],
           px[(32 * W + 32) * 4 + 2], px[(32 * W + 32) * 4 + 3]);
    printf("pixels differing: %d\nfnv1a: %08x\nverdict: %s\n", bad, h, bad ? "FAIL" : "PASS");
    return 0;
}
