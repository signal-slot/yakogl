/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test case 141 es3instdiv (driver/tools/pvrlab/gl_test.c,
 * case_es3instdiv) on its own, for a Mesa reference
 * (tools/mali/mesa-board-ref.sh): case 47's program with
 * glVertexAttribDivisor 2 on the offset and the colour, 4 instances, as
 * arrays and as elements. The draws, checks and their order are the case's
 * (checks 1-8).
 *
 *   cc -O1 -Wall -Werror -o es3instdiv es3instdiv.c -lEGL -lGLESv2
 *   ./es3instdiv [--libdir <mesa lib dir>]
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

#define NAME "es3instdiv"
#define TW 64u
#define TH 64u

static const char vs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"layout(location = 0) in vec4 pos;\n"
	"layout(location = 1) in vec2 offs;\n"
	"layout(location = 2) in vec4 icol;\n"
	"out vec4 v_col;\n"
	"void main()\n"
	"{\n"
	"    gl_Position = vec4(pos.xy * 0.25 + offs, 0.0, 1.0);\n"
	"    /* the last instance proves gl_InstanceID: half its colour */\n"
	"    v_col = gl_InstanceID == 3 ? icol * 0.5 : icol;\n"
	"}\n";

static const char fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"in vec4 v_col;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    fragColor = v_col;\n"
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
	static const float quad6[6 * 4] = {
		-1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,
		-1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1,
	};
	static const float quad4[4 * 4] = { -1, -1, 0, 1,  1, -1, 0, 1,  1, 1, 0, 1,  -1, 1, 0, 1 };
	static const uint16_t idx[6] = { 0, 1, 2, 0, 2, 3 };
	/* per two instances: offset (2) + colour (4) */
	static const float inst[2 * 6] = {
		-0.5f, -0.5f,  1, 0, 0, 1,
		 0.5f, -0.5f,  0, 1, 0, 1,
	};
	GLuint prog, vb6 = 0, vb4 = 0, ib = 0, ivb = 0;
	bool ok = true;

	prog = link_program(vs_src, fs_src);
	if (!prog)
		return -1;
	glGenBuffers(1, &vb6);
	glBindBuffer(GL_ARRAY_BUFFER, vb6);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad6), quad6, GL_STATIC_DRAW);
	glGenBuffers(1, &vb4);
	glBindBuffer(GL_ARRAY_BUFFER, vb4);
	glBufferData(GL_ARRAY_BUFFER, sizeof(quad4), quad4, GL_STATIC_DRAW);
	glGenBuffers(1, &ib);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof(idx), idx, GL_STATIC_DRAW);
	glGenBuffers(1, &ivb);
	glBindBuffer(GL_ARRAY_BUFFER, ivb);
	glBufferData(GL_ARRAY_BUFFER, sizeof(inst), inst, GL_STATIC_DRAW);
	glUseProgram(prog);
	glViewport(0, 0, TW, TH);

	for (int pass = 0; pass < 2; pass++) {
		glBindBuffer(GL_ARRAY_BUFFER, pass == 0 ? vb6 : vb4);
		glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 16, (void *)0);
		glEnableVertexAttribArray(0);
		glBindBuffer(GL_ARRAY_BUFFER, ivb);
		glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 24, (void *)0);
		glVertexAttribPointer(2, 4, GL_FLOAT, GL_FALSE, 24, (void *)8);
		glEnableVertexAttribArray(1);
		glEnableVertexAttribArray(2);
		glVertexAttribDivisor(1, 2);
		glVertexAttribDivisor(2, 2);
		glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		if (pass == 0)
			glDrawArraysInstanced(GL_TRIANGLES, 0, 6, 4);
		else
			glDrawElementsInstanced(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, (void *)0, 4);
		glFinish();
		if (!gl_ok(pass == 0 ? "arrays instanced, divisor 2" : "elements instanced, divisor 2"))
			ok = false;
		read_topdown();
		ok &= expect_px(pass == 0 ? "arrays: instances 0-1 red" : "elements: instances 0-1 red", 16, 48,
				rgba_u32(255, 0, 0, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: instance 3 half green" : "elements: instance 3 half green", 48, 48,
				rgba_u32(0, 128, 0, 128), 3);
		ok &= expect_px(pass == 0 ? "arrays: top left = clear" : "elements: top left = clear", 16, 16,
				rgba_u32(0, 0, 0, 255), 2);
		ok &= expect_px(pass == 0 ? "arrays: top right = clear" : "elements: top right = clear", 48, 16,
				rgba_u32(0, 0, 0, 255), 2);
	}
	glVertexAttribDivisor(1, 0);
	glVertexAttribDivisor(2, 0);
	glDisableVertexAttribArray(1);
	glDisableVertexAttribArray(2);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
	glDeleteBuffers(1, &vb6);
	glDeleteBuffers(1, &vb4);
	glDeleteBuffers(1, &ib);
	glDeleteBuffers(1, &ivb);
	return ok && gl_ok("es3instdiv teardown") ? 0 : -1;
}
