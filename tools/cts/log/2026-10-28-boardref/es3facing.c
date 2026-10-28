/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test case 49 es3facing (driver/tools/pvrlab/gl_test.c, case_es3facing)
 * on its own, for a Mesa reference (tools/mali/mesa-board-ref.sh):
 * es300/facing.frag (green where gl_FrontFacing, red elsewhere) on the
 * counter-clockwise full-screen quad, glFrontFace CCW then CW into the
 * window (pbuffer); then the same into an 8x8 texture FBO (no y flip),
 * sampled back x0.25 into the window by es300/hdrsample.frag. The draws,
 * checks and their order are the case's (checks 1-4).
 *
 *   cc -O1 -Wall -Werror -o es3facing es3facing.c -lEGL -lGLESv2
 *   ./es3facing [--libdir <mesa lib dir>]
 */
#define _GNU_SOURCE
#include <EGL/egl.h>
#include <GLES3/gl3.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define NAME "es3facing"
#define TW 64u
#define TH 64u

static const char vs_src[] =
	"#version 300 es\n"
	"layout(location = 0) in vec4 pos;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    // the vertex index names the colour: 0 red, 1 green, 2 blue, 3 white ...\n"
	"    int i = gl_VertexID % 4;\n"
	"    v_col = vec4(i == 0 || i == 3 ? 1.0 : 0.0, i == 1 || i == 3 ? 1.0 : 0.0, i == 2 || i == 3 ? 1.0 : 0.0, 1.0);\n"
	"    gl_Position = pos;\n"
	"}\n";

static const char fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    fragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0);\n"
	"}\n";

static const char cube_vs_src[] =
	"#version 300 es\n"
	"layout(location = 0) in vec2 pos;\n"
	"out vec2 v_xy;\n"
	"void main()\n"
	"{\n"
	"    v_xy = pos;\n"
	"    gl_Position = vec4(pos, 0.0, 1.0);\n"
	"}\n";

static const char hdrsample_fs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"uniform sampler2D hdr;\n"
	"in vec2 v_xy;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    // a quarter of the HDR texel: values above 1.0 stay visible in an 8-bit target\n"
	"    fragColor = texture(hdr, v_xy * 0.5 + 0.5) * 0.25;\n"
	"}\n";

struct puv_vertex { float pos[4]; float uv[2]; };

static int check_no, first_bad = -1;
static uint32_t first_got, first_want;
static uint32_t rows[TW * TH], px[TW * TH];

__attribute__((unused)) static uint32_t rgba_u32(uint8_t r, uint8_t g, uint8_t b, uint8_t a)
{
	return ((uint32_t)a << 24) | ((uint32_t)b << 16) | ((uint32_t)g << 8) | r;
}

__attribute__((unused)) static void fill_texels(uint8_t *dst, uint32_t n, uint8_t r, uint8_t g, uint8_t b, uint8_t a)
{
	for (uint32_t i = 0; i < n; i++) {
		dst[i * 4 + 0] = r; dst[i * 4 + 1] = g; dst[i * 4 + 2] = b; dst[i * 4 + 3] = a;
	}
}

static int chan_diff(uint32_t a, uint32_t b)
{
	int d = 0;

	for (int i = 0; i < 32; i += 8) {
		int x = (int)((a >> i) & 0xff) - (int)((b >> i) & 0xff);

		if (x < 0)
			x = -x;
		if (x > d)
			d = x;
	}
	return d;
}

static bool expect_px(const char *what, uint32_t x, uint32_t y, uint32_t want, int tol)
{
	uint32_t got = px[y * TW + x];
	bool ok = chan_diff(got, want) <= tol;

	check_no++;
	printf(NAME ":   %-40s (%2u,%2u) = 0x%08x  want 0x%08x  %s  [check %d]\n",
	       what, x, y, got, want, ok ? "ok" : "MISMATCH", check_no);
	if (!ok && first_bad < 0) {
		first_bad = check_no;
		first_got = got;
		first_want = want;
	}
	return ok;
}

static bool gl_ok(const char *where)
{
	GLenum e = glGetError();

	if (e != GL_NO_ERROR)
		printf(NAME ": GL error 0x%x after %s\n", (unsigned)e, where);
	return e == GL_NO_ERROR;
}

static void read_topdown(void)
{
	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, rows);
	for (uint32_t y = 0; y < TH; y++)
		memcpy(&px[y * TW], &rows[(TH - 1 - y) * TW], TW * 4);
}

static GLuint compile(GLenum type, const char *src)
{
	GLuint s = glCreateShader(type);
	GLint ok = 0;
	char log[256] = "";

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	if (!ok) {
		glGetShaderInfoLog(s, sizeof(log), NULL, log);
		printf(NAME ": shader compile failed: %s\n", log);
		return 0;
	}
	return s;
}

static GLuint link_program(const char *vsrc, const char *fsrc)
{
	GLuint vs = compile(GL_VERTEX_SHADER, vsrc), fs = compile(GL_FRAGMENT_SHADER, fsrc), prog;
	GLint linked = 0;

	if (!vs || !fs)
		return 0;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &linked);
	return linked ? prog : 0;
}

/* gl_test's puv_quad_setup: a full-screen quad (two triangles), vec4 position
 * at location 0 and uv (0,0) at NDC (-1,-1) at location 1. */
__attribute__((unused)) static void puv_quad_setup(void)
{
	static const float c[6][4] = {
		{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
		{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
	};
	struct puv_vertex q[6];
	GLuint vbo = 0;

	for (int i = 0; i < 6; i++) {
		q[i].pos[0] = c[i][0]; q[i].pos[1] = c[i][1]; q[i].pos[2] = 0.0f; q[i].pos[3] = 1.0f;
		q[i].uv[0] = c[i][2]; q[i].uv[1] = c[i][3];
	}
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(q), q, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)0);
	glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, sizeof(struct puv_vertex), (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
}

static int run_case(void);

int main(int argc, char **argv)
{
	static const EGLint cfg_attribs[] = {
		EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE,
	};
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE };
	EGLDisplay dpy;
	EGLConfig cfg;
	EGLint n = 0;
	int rc;

	/* --libdir <dir>: run against the Mesa in <dir> (the board's 26.2.1
	 * build is reached through LD_LIBRARY_PATH only; mesa-board-ref.sh
	 * passes arguments, not an environment), so set it and start again.
	 * The disk shader cache is off too: a cache hit gives Mesa 26.2.1 a
	 * different binary from a fresh compile (2026-10-28-splitref, section 6),
	 * and two runs are compared word by word. */
	if (argc >= 3 && !strcmp(argv[1], "--libdir")) {
		const char *cur = getenv("LD_LIBRARY_PATH");

		if (!cur || strcmp(cur, argv[2])) {
			setenv("LD_LIBRARY_PATH", argv[2], 1);
			setenv("MESA_SHADER_CACHE_DISABLE", "true", 1);
			execv("/proc/self/exe", argv);
			perror(NAME ": execv");
			return 1;
		}
	}
	dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1)
		return 1;
	EGLSurface pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (pb == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, pb, pb, ctx)) {
		printf(NAME ": surface/context failed (0x%x)\n", eglGetError());
		return 1;
	}
	printf(NAME ": %s / %s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION));
	glViewport(0, 0, TW, TH);
	rc = run_case();
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, pb);
	eglDestroyContext(dpy, ctx);
	eglTerminate(dpy);
	if (rc < 0) {
		printf("verdict " NAME ": FAIL rc %d\n", rc);
		return 1;
	}
	if (first_bad < 0)
		printf("verdict " NAME ": PASS\n");
	else
		printf("verdict " NAME ": FAIL, first bad check %d got 0x%08x want 0x%08x\n",
		       first_bad, first_got, first_want);
	return first_bad < 0 ? 0 : 1;
}

/* ---- the case ------------------------------------------------------------ */
static int run_case(void)
{
	GLuint prog, tex = 0, fbo = 0, sprog;
	bool ok = true;

	prog = link_program(vs_src, fs_src);
	sprog = link_program(cube_vs_src, hdrsample_fs_src);
	if (!prog || !sprog)
		return -1;
	glUseProgram(prog);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	for (int cw = 0; cw < 2; cw++) {
		glFrontFace(cw ? GL_CW : GL_CCW);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!gl_ok("facing draw"))
			return -1;
		read_topdown();
		ok &= expect_px(cw ? "window, front CW: back (red)" : "window, front CCW: front (green)",
				32, 32, cw ? rgba_u32(255, 0, 0, 255) : rgba_u32(0, 255, 0, 255), 2);
	}
	/* an FBO (no y flip), sampled back x0.25 */
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	for (int cw = 0; cw < 2; cw++) {
		glBindFramebuffer(GL_FRAMEBUFFER, fbo);
		glViewport(0, 0, 8, 8);
		glUseProgram(prog);
		puv_quad_setup();
		glFrontFace(cw ? GL_CW : GL_CCW);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glViewport(0, 0, TW, TH);
		glFrontFace(GL_CCW);
		glUseProgram(sprog);
		glUniform1i(glGetUniformLocation(sprog, "hdr"), 0);
		puv_quad_setup();
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		ok &= expect_px(cw ? "fbo, front CW: back (red x0.25)" : "fbo, front CCW: front (green x0.25)",
				32, 32, cw ? rgba_u32(64, 0, 0, 64) : rgba_u32(0, 64, 0, 64), 3);
	}
	glFrontFace(GL_CCW);
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok && gl_ok("es3facing teardown") ? 0 : -1;
}
