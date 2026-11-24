// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Mesa's reference for #7489 (docs/es2-cts-failures.md M.1 rows 5 and 11):
// a vertex shader that samples a samplerCube near a face edge, drawn through
// the board's Mesa (tools/mali/mesa-board-ref.sh's flags) into a 64x64 RGBA8
// FBO.
//
//   ./vs-cube-edge         textureCube(u_tex, dir)            (implicit LOD 0)
//   ./vs-cube-edge lod     textureCubeLod(u_tex, dir, 1.5)
//
// The cube is 8x8 RGBA8 with its full mip chain (8, 4, 2, 1), each texel of
// face f, level l: R = x * 255 / (size - 1), G = y * 255 / (size - 1)
// (128 at size 1), B = 40 f + 8 l. GL_REPEAT on S and T,
// MIN LINEAR_MIPMAP_LINEAR, MAG LINEAR. 16x16 points (gl_PointSize 4) each
// take their colour from the vertex stage at the direction
// (1, 0.9 + 0.2 u, 2 v - 1), u, v in (0, 1): the +X face, crossing its edge
// with +Y at u = 0.5. Prints the centre pixel of every point (row v, column
// u) and the FNV-1a of the read-back bytes; there is no verdict, the pixels
// are the reference.
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define W 64
#define H 64
#define N 16
#define S 8

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

int main(int argc, char **argv)
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
    int lod = argc > 1 && !strcmp(argv[1], "lod");
    printf("renderer: %s\nversion: %s\nmode: %s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION),
           lod ? "vertex textureCubeLod 1.5, repeat, linear_mipmap_linear"
               : "vertex textureCube near the +X/+Y edge, repeat, linear_mipmap_linear");
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

    GLuint cube;
    glActiveTexture(GL_TEXTURE0);
    glGenTextures(1, &cube);
    glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
    static unsigned char tx[S * S * 4];
    for (int f = 0; f < 6; f++)
        for (int l = 0, s = S; s >= 1; l++, s /= 2) {
            for (int y = 0; y < s; y++)
                for (int x = 0; x < s; x++) {
                    unsigned char *q = tx + (y * s + x) * 4;
                    q[0] = s > 1 ? x * 255 / (s - 1) : 128;
                    q[1] = s > 1 ? y * 255 / (s - 1) : 128;
                    q[2] = 40 * f + 8 * l;
                    q[3] = 255;
                }
            glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, l, GL_RGBA, s, s, 0, GL_RGBA,
                         GL_UNSIGNED_BYTE, tx);
        }
    glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_REPEAT);
    glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_REPEAT);
    glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR);
    glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_LINEAR);

    const char *vs = lod
        ? "attribute vec2 a_uv;\nuniform samplerCube u_tex;\nvarying vec4 v_col;\n"
          "void main() {\n"
          "  vec3 dir = vec3(1.0, 0.9 + 0.2 * a_uv.x, 2.0 * a_uv.y - 1.0);\n"
          "  v_col = textureCubeLod(u_tex, dir, 1.5);\n"
          "  gl_Position = vec4(a_uv * 2.0 - 1.0, 0.0, 1.0);\n  gl_PointSize = 4.0;\n}\n"
        : "attribute vec2 a_uv;\nuniform samplerCube u_tex;\nvarying vec4 v_col;\n"
          "void main() {\n"
          "  vec3 dir = vec3(1.0, 0.9 + 0.2 * a_uv.x, 2.0 * a_uv.y - 1.0);\n"
          "  v_col = textureCube(u_tex, dir);\n"
          "  gl_Position = vec4(a_uv * 2.0 - 1.0, 0.0, 1.0);\n  gl_PointSize = 4.0;\n}\n";
    GLuint p = glCreateProgram();
    glAttachShader(p, sh(GL_VERTEX_SHADER, vs));
    glAttachShader(p, sh(GL_FRAGMENT_SHADER,
        "precision mediump float;\nvarying vec4 v_col;\nvoid main() { gl_FragColor = v_col; }\n"));
    glBindAttribLocation(p, 0, "a_uv");
    glLinkProgram(p);
    GLint ok;
    glGetProgramiv(p, GL_LINK_STATUS, &ok);
    if (!ok) { fprintf(stderr, "link failed\n"); return 1; }
    glUseProgram(p);
    glUniform1i(glGetUniformLocation(p, "u_tex"), 0);

    static float uv[N * N * 2];
    for (int j = 0; j < N; j++)
        for (int i = 0; i < N; i++) {
            uv[(j * N + i) * 2] = (i + 0.5f) / N;
            uv[(j * N + i) * 2 + 1] = (j + 0.5f) / N;
        }
    glViewport(0, 0, W, H);
    glClearColor(0, 0, 0, 0);
    glClear(GL_COLOR_BUFFER_BIT);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, uv);
    glEnableVertexAttribArray(0);
    glDrawArrays(GL_POINTS, 0, N * N);

    static unsigned char px[W * H * 4];
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, px);
    printf("glGetError: 0x%04x\n", glGetError());
    unsigned h = 2166136261u;
    for (unsigned i = 0; i < sizeof px; i++) h = (h ^ px[i]) * 16777619u;
    for (int j = 0; j < N; j++) {
        printf("v%02d:", j);
        for (int i = 0; i < N; i++) {
            const unsigned char *q = px + ((j * 4 + 2) * W + i * 4 + 2) * 4;
            printf(" %02x%02x%02x%02x", q[0], q[1], q[2], q[3]);
        }
        printf("\n");
    }
    printf("fnv1a: %08x\n", h);
    return 0;
}
