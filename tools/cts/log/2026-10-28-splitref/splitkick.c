/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test case 214 splitkick (driver/tools/pvrlab/gl_test.c) on its own, for
 * a Mesa reference (tools/mali/mesa-board-ref.sh): the same 4x pbuffer, the
 * same shaders (gl_test_shaders_es100.h), the same draws, the same glFinish
 * between the two kicks and the same checks, numbered as the case numbers
 * them (1, 2: GL errors after each kick; 3-6: the pixels).
 *
 *   cc -O1 -Wall -Werror -o splitkick splitkick.c -lEGL -lGLESv2
 *   ./splitkick [--libdir <mesa lib dir>]
 */
#define _GNU_SOURCE
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define TW 64u
#define TH 64u

static const char vs_src[] =
	"#version 100\n"
	"/* Two-varying VS: colour + uv. ES 1.00 twin of gl_test_vs_src, the pair most\n"
	" * gl_test cases draw with. Order is the twin's: a_pos 0, a_color 1, a_uv 2. */\n"
	"attribute vec4 a_pos;\n"
	"attribute vec4 a_color;\n"
	"attribute vec2 a_uv;\n"
	"varying vec4 v_color;\n"
	"varying vec2 v_uv;\n"
	"void main() {\n"
	"    gl_Position = a_pos;\n"
	"    v_color = a_color;\n"
	"    v_uv = a_uv;\n"
	"}\n";

static const char fs_src[] =
	"#version 100\n"
	"/* Two-varying FS: colour modulated by uv. ES 1.00 twin of gl_test_fs_src. */\n"
	"precision mediump float;\n"
	"varying vec4 v_color;\n"
	"varying vec2 v_uv;\n"
	"void main() { gl_FragColor = v_color * vec4(v_uv, 1.0, 1.0); }\n";

struct cuv_vertex { float pos[4]; float color[4]; float uv[2]; };

static int check_no, first_bad = -1;
static uint32_t first_got, first_want;
static uint32_t rows[TW * TH], px[TW * TH];

static bool check(bool ok, uint32_t got, uint32_t want)
{
	check_no++;
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
		printf("splitkick: GL error 0x%x after %s\n", (unsigned)e, where);
	return e == GL_NO_ERROR;
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

	printf("splitkick:   %-40s (%2u,%2u) = 0x%08x  want 0x%08x  %s  [check %d]\n",
	       what, x, y, got, want, ok ? "ok" : "MISMATCH", check_no + 1);
	return check(ok, got, want);
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
		printf("splitkick: shader compile failed: %s\n", log);
		return 0;
	}
	return s;
}

static void cuv_set(struct cuv_vertex *v, float x, float y)
{
	v->pos[0] = x; v->pos[1] = y; v->pos[2] = 0.0f; v->pos[3] = 1.0f;
	v->color[0] = v->color[1] = v->color[2] = v->color[3] = 1.0f;
	v->uv[0] = v->uv[1] = 1.0f;
}

int main(int argc, char **argv)
{
	static const EGLint cfg4[] = {
		EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, 4, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8,
		EGL_NONE,
	};
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLDisplay dpy;
	EGLConfig cfg;
	EGLint n = 0, samples = 0;
	struct cuv_vertex v[3];
	GLuint vs, fs, prog, vbo;
	GLint linked = 0;
	bool ok = true;

	/* --libdir <dir>: run against the Mesa in <dir> (the board's 26.2.1
	 * build sits beside the system 25.0.7 and is reached through
	 * LD_LIBRARY_PATH only; mesa-board-ref.sh passes arguments, not an
	 * environment), so set it and start again before libEGL is used. */
	if (argc >= 3 && !strcmp(argv[1], "--libdir")) {
		const char *cur = getenv("LD_LIBRARY_PATH");

		if (!cur || strcmp(cur, argv[2])) {
			setenv("LD_LIBRARY_PATH", argv[2], 1);
			execv("/proc/self/exe", argv);
			perror("splitkick: execv");
			return 1;
		}
	}
	dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(dpy, NULL, NULL))
		return 1;
	if (!eglChooseConfig(dpy, cfg4, &cfg, 1, &n) || n != 1) {
		printf("splitkick: SKIP (no 4x EGL config offered)\n");
		return 3;
	}
	eglGetConfigAttrib(dpy, cfg, EGL_SAMPLES, &samples);
	EGLSurface pb = eglCreatePbufferSurface(dpy, cfg, pb_attribs);
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (pb == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, pb, pb, ctx)) {
		printf("splitkick: surface/context failed (0x%x)\n", eglGetError());
		return 1;
	}
	printf("splitkick: %s / %s, EGL_SAMPLES %d\n", glGetString(GL_RENDERER), glGetString(GL_VERSION), samples);
	vs = compile(GL_VERTEX_SHADER, vs_src);
	fs = compile(GL_FRAGMENT_SHADER, fs_src);
	if (!vs || !fs)
		return 1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	/* yakogl assigns them in declaration order; Mesa is told the same */
	glBindAttribLocation(prog, 0, "a_pos");
	glBindAttribLocation(prog, 1, "a_color");
	glBindAttribLocation(prog, 2, "a_uv");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &linked);
	if (!linked)
		return 1;
	glUseProgram(prog);
	cuv_set(&v[0], -1.0f, -1.0f);
	cuv_set(&v[1], 1.0f, -1.0f);
	cuv_set(&v[2], 1.0f, 1.0f);
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(v), v, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)16);
	glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, sizeof(struct cuv_vertex), (const void *)32);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);
	glEnableVertexAttribArray(2);
	glViewport(0, 0, TW, TH);
	glDisable(GL_BLEND);
	glDisable(GL_DEPTH_TEST);
	glDisable(GL_STENCIL_TEST);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);

	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish(); /* kick 1 */
	ok &= check(gl_ok("splitkick kick 1"), 0, 0);

	glDrawArrays(GL_TRIANGLES, 0, 3); /* kick 2: no clear, reloads kick 1's samples */
	glFinish();
	ok &= check(gl_ok("splitkick kick 2"), 0, 0);
	glReadPixels(0, 0, TW, TH, GL_RGBA, GL_UNSIGNED_BYTE, rows);
	for (uint32_t y = 0; y < TH; y++)
		memcpy(&px[y * TW], &rows[(TH - 1 - y) * TW], TW * 4);
	ok &= expect_px("x4 split: inside", 40, TH - 1 - 20, 0xffffffffu, 0);
	ok &= expect_px("x4 split: outside", 20, TH - 1 - 40, 0xff000000u, 0);
	ok &= expect_px("x4 split: diagonal half covered", 32, TH - 1 - 32, 0xff808080u, 2);
	ok &= expect_px("x4 split: diagonal half covered (40,40)", 40, TH - 1 - 40, 0xff808080u, 2);

	glUseProgram(0);
	glDeleteProgram(prog);
	glDeleteBuffers(1, &vbo);
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, pb);
	eglDestroyContext(dpy, ctx);
	eglTerminate(dpy);
	if (ok)
		printf("verdict splitkick: PASS\n");
	else
		printf("verdict splitkick: FAIL, first bad check %d got 0x%08x want 0x%08x\n",
		       first_bad, first_got, first_want);
	return ok ? 0 : 1;
}
