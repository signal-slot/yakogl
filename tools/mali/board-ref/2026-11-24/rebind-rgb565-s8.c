// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Mesa's reference for #11329 (docs/es2-cts-failures.md M.1 row 14): the
// GL sequence of dEQP-GLES2.functional.fbo.render.recreate_colorbuffer.
// rebind_rbo_rgb565_stencil_index8 (VK-GL-CTS es2fFboRenderTest.cpp,
// RecreateBuffersTest<GL_COLOR_BUFFER_BIT>::render with m_rebind true),
// drawn through the board's Mesa (tools/mali/mesa-board-ref.sh's flags)
// into a 128x128 FBO.
//
//   ./rebind-rgb565-s8          RGB565 renderbuffer + STENCIL_INDEX8 renderbuffer
//                               (rebind_rbo_rgb565_stencil_index8)
//   ./rebind-rgb565-s8 tex      GL_RGB/UNSIGNED_BYTE 2D texture + DEPTH_COMPONENT16
//                               + STENCIL_INDEX8 renderbuffers
//                               (rebind_tex2d_rgb_depth_component16_stencil_index8)
//   ./rebind-rgb565-s8 texs8    GL_RGB/UNSIGNED_BYTE 2D texture + STENCIL_INDEX8
//                               (rebind_tex2d_rgb_stencil_index8; the board's Mesa
//                               answers "tex" with FRAMEBUFFER_UNSUPPORTED)
//
// The sequence, as dEQP issues it: bind the FBO, viewport 128x128, clear
// colour (1, 0, 0, 1), depth and stencil, enable the depth test, draw a
// textured quad at z 0 (the "quads" texture). Scissor (32, 32, 64, 64),
// clear stencil to 1, scissor off. With the FBO still bound (rebind): delete
// the colour buffer (which detaches it), bind the same name again, give it
// the same format and size (a texture also gets MIN_FILTER LINEAR) and
// attach it. Clear colour only, to (0, 0, 1, 0), stencil test EQUAL 0 0xff,
// draw a textured quad from z 1 to z -1 (the "metaballs" texture), stencil
// and depth tests off, read back. No glFinish anywhere: the frames Mesa
// submits are its own.
//
// The two textures are 64x64 RGB/UNSIGNED_BYTE of one colour each (dEQP's
// patterns do not change a descriptor): quads (255, 255, 0), metaballs
// (0, 255, 255). Without a depth buffer the expected read-back is the second
// clear colour inside the scissor box (the recreated buffer was cleared to
// (0, 0, 1, 0) and stencil 1 keeps the second draw out; the RGB formats read
// alpha 255) and the metaballs colour outside; the verdict is PASS when
// every pixel is that. With a depth buffer it prints the pixels but gives no
// verdict.
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define W 128
#define H 128

static GLuint sh(GLenum t, const char *s)
{
    GLuint o = glCreateShader(t);
    glShaderSource(o, 1, &s, NULL);
    glCompileShader(o);
    GLint ok;
    glGetShaderiv(o, GL_COMPILE_STATUS, &ok);
    if (!ok) { fprintf(stderr, "compile failed\n"); exit(1); }
    return o;
}

static void flat_tex(GLuint name, unsigned char r, unsigned char g, unsigned char b)
{
    static unsigned char px[64 * 64 * 3];
    for (int i = 0; i < 64 * 64; i++) { px[i * 3] = r; px[i * 3 + 1] = g; px[i * 3 + 2] = b; }
    glBindTexture(GL_TEXTURE_2D, name);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, 64, 64, 0, GL_RGB, GL_UNSIGNED_BYTE, px);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
}

// sglr::drawQuad: corners p0 and p1, the other two at the mean z.
static void quad(float z0, float z1)
{
    const float zm = (z0 + z1) / 2;
    const float pos[] = { -1, -1, z0, 1, -1, 1, zm, 1, 1, -1, zm, 1, 1, 1, z1, 1 };
    const float uv[] = { 0, 0, 0, 1, 1, 0, 1, 1 };
    const GLushort idx[] = { 0, 1, 2, 2, 1, 3 };
    glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 0, pos);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 0, uv);
    glEnableVertexAttribArray(1);
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, idx);
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
    int texs8 = argc > 1 && !strcmp(argv[1], "texs8");
    int tex = texs8 || (argc > 1 && !strcmp(argv[1], "tex"));
    int dep = tex && !texs8;
    printf("renderer: %s\nversion: %s\nmode: %s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION),
           texs8 ? "rebind tex2d rgb + stencil_index8"
           : tex ? "rebind tex2d rgb + depth_component16 + stencil_index8"
                 : "rebind rbo rgb565 + stencil_index8");

    GLuint prog = glCreateProgram();
    glAttachShader(prog, sh(GL_VERTEX_SHADER,
        "attribute highp vec4 a_position;\nattribute mediump vec2 a_coord;\n"
        "varying mediump vec2 v_coord;\n"
        "void main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n"));
    glAttachShader(prog, sh(GL_FRAGMENT_SHADER,
        "uniform sampler2D u_sampler0;\nvarying mediump vec2 v_coord;\n"
        "void main (void)\n{\n    gl_FragColor = texture2D(u_sampler0, v_coord);\n}\n"));
    glBindAttribLocation(prog, 0, "a_position");
    glBindAttribLocation(prog, 1, "a_coord");
    glLinkProgram(prog);

    const GLuint metaballs = 1, quads = 2;
    flat_tex(quads, 255, 255, 0);
    flat_tex(metaballs, 0, 255, 255);

    // Framebuffer(ctx, config, 128, 128), as dEQP builds it.
    GLuint fb, col, depth = 0, stencil;
    glGenFramebuffers(1, &fb);
    glBindFramebuffer(GL_FRAMEBUFFER, fb);
    if (tex) {
        glGenTextures(1, &col);
        glBindTexture(GL_TEXTURE_2D, col);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, W, H, 0, GL_RGB, GL_UNSIGNED_BYTE, NULL);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col, 0);
    } else {
        glGenRenderbuffers(1, &col);
        glBindRenderbuffer(GL_RENDERBUFFER, col);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB565, W, H);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, col);
    }
    if (dep) {
        glGenRenderbuffers(1, &depth);
        glBindRenderbuffer(GL_RENDERBUFFER, depth);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, W, H);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
    }
    glGenRenderbuffers(1, &stencil);
    glBindRenderbuffer(GL_RENDERBUFFER, stencil);
    glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, W, H);
    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, stencil);
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
    glBindFramebuffer(GL_FRAMEBUFFER, fb); // checkCompleteness
    GLenum st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
    printf("status before: 0x%04x\n", st);
    if (st != GL_FRAMEBUFFER_COMPLETE) { printf("verdict: UNSUPPORTED\n"); return 0; }

    glUseProgram(prog);
    glUniform1i(glGetUniformLocation(prog, "u_sampler0"), 0);

    // Draw scene.
    glBindFramebuffer(GL_FRAMEBUFFER, fb);
    glViewport(0, 0, W, H);
    glClearColor(1, 0, 0, 1);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    glEnable(GL_DEPTH_TEST);
    glBindTexture(GL_TEXTURE_2D, quads);
    quad(0, 0);
    glEnable(GL_SCISSOR_TEST);
    glScissor(W / 4, H / 4, W / 2, H / 2);
    glClearStencil(1);
    glClear(GL_STENCIL_BUFFER_BIT);
    glDisable(GL_SCISSOR_TEST);

    // Recreate the colour buffer under the same name, the FBO still bound.
    if (tex) {
        glDeleteTextures(1, &col);
        glBindTexture(GL_TEXTURE_2D, col);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB, W, H, 0, GL_RGB, GL_UNSIGNED_BYTE, NULL);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, col, 0);
    } else {
        glDeleteRenderbuffers(1, &col);
        glBindRenderbuffer(GL_RENDERBUFFER, col);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB565, W, H);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, col);
    }
    printf("status after: 0x%04x\n", glCheckFramebufferStatus(GL_FRAMEBUFFER));

    glClearColor(0, 0, 1, 0);
    glClearStencil(0);
    glClear(GL_COLOR_BUFFER_BIT);
    glEnable(GL_STENCIL_TEST);
    glStencilFunc(GL_EQUAL, 0, 0xff);
    glBindTexture(GL_TEXTURE_2D, metaballs);
    quad(1, -1);
    glDisable(GL_STENCIL_TEST);
    glDisable(GL_DEPTH_TEST);

    static unsigned char px[W * H * 4];
    glReadPixels(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, px);
    printf("glGetError: 0x%04x\n", glGetError());
    unsigned h = 2166136261u;
    for (unsigned i = 0; i < sizeof px; i++) h = (h ^ px[i]) * 16777619u;
    const unsigned char *in = px + (64 * W + 64) * 4, *out = px + (8 * W + 8) * 4;
    printf("inside (64,64): %u %u %u %u\noutside (8,8): %u %u %u %u\n", in[0], in[1], in[2], in[3], out[0],
           out[1], out[2], out[3]);
    printf("fnv1a: %08x\n", h);
    if (dep) return 0;
    int bad = 0;
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            const unsigned char *q = px + (y * W + x) * 4;
            int box = x >= W / 4 && x < 3 * W / 4 && y >= H / 4 && y < 3 * H / 4;
            if (q[0] != 0 || q[1] != (box ? 0 : 255) || q[2] != 255 || q[3] != 255) bad++;
        }
    printf("pixels differing: %d\nverdict: %s\n", bad, bad ? "FAIL" : "PASS");
    return 0;
}
