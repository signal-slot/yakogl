/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test case 2 (driver/tools/pvrlab/gl_test.c, case_texture), its "two
 * texture units" part on its own, for a Mesa reference
 * (tools/mali/mesa-board-ref.sh): the ES 1.00 shaders the Mali build uses
 * (gl_test_shaders_es100.h gl_test_uv_vs_src / gl_test_texture2_fs_src), the
 * 4x4 quadrant texture on unit 0, a 1x1 yellow texture on unit 1, one draw
 * (u_tex0 = 0, u_tex1 = 1), then the same draw with the sampler uniforms
 * swapped (u_tex0 = 1, u_tex1 = 0). Checks 1-4 are the case's.
 *
 *   cc -O1 -Wall -Werror -o tex2 tex2.c -lEGL -lGLESv2
 *   ./tex2 [--libdir <mesa lib dir>]
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

#define NAME "tex2"
#define TW 64u
#define TH 64u

static const char vs_src[] =
	"#version 100\n"
	"/* UV-only VS. ES 1.00 twin of gl_test_uv_vs_src. */\n"
	"attribute vec4 a_pos;\n"
	"attribute vec2 a_uv;\n"
	"varying vec2 v_uv;\n"
	"void main() {\n"
	"    gl_Position = a_pos;\n"
	"    v_uv = a_uv;\n"
	"}\n";

static const char fs_src[] =
	"#version 100\n"
	"/* Two samplers: the unit-0 texture weighted 0.5 plus the unit-1 texture\n"
	" * weighted 0.25. ES 1.00 twin of gl_test_texture2_fs_src. */\n"
	"precision mediump float;\n"
	"varying vec2 v_uv;\n"
	"uniform sampler2D u_tex0;\n"
	"uniform sampler2D u_tex1;\n"
	"void main() {\n"
	"    gl_FragColor = vec4(texture2D(u_tex0, v_uv).rgb * 0.5 + texture2D(u_tex1, v_uv).rgb * 0.25, 1.0);\n"
	"}\n";

struct puv_vertex { float pos[4]; float uv[2]; };

static int check_no, first_bad = -1;
static uint32_t first_got, first_want;
static uint32_t rows[TW * TH], px[TW * TH];

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

/* The case's make_quadrant_texture(4): red | green at t = 0, blue | white at
 * t = 1, NEAREST, clamp to edge. */
static GLuint quadrant_texture(void)
{
	static uint8_t texels[4 * 4 * 4];
	GLuint tex = 0;

	for (uint32_t y = 0; y < 4; y++)
		for (uint32_t x = 0; x < 4; x++) {
			uint8_t *p = &texels[(y * 4 + x) * 4];
			bool right = x >= 2, top = y >= 2;

			p[0] = (!top && !right) || (top && right) ? 255 : 0;
			p[1] = (!top && right) || (top && right) ? 255 : 0;
			p[2] = top ? 255 : 0;
			p[3] = 255;
		}
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	return tex;
}

int main(int argc, char **argv)
{
	static const EGLint cfg_attribs[] = {
		EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE,
	};
	static const EGLint pb_attribs[] = { EGL_WIDTH, TW, EGL_HEIGHT, TH, EGL_NONE };
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static const float c[6][4] = {
		{ -1, -1, 0, 0 }, { 1, -1, 1, 0 }, { 1, 1, 1, 1 },
		{ -1, -1, 0, 0 }, { 1, 1, 1, 1 }, { -1, 1, 0, 1 },
	};
	static const uint8_t yellow[4] = { 255, 255, 0, 255 };
	struct puv_vertex q[6];
	EGLDisplay dpy;
	EGLConfig cfg;
	EGLint n = 0;
	GLuint vs, fs, prog, vbo, quad, yel = 0;
	GLint linked = 0, l0, l1;
	bool ok = true;

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
	vs = compile(GL_VERTEX_SHADER, vs_src);
	fs = compile(GL_FRAGMENT_SHADER, fs_src);
	if (!vs || !fs)
		return 1;
	prog = glCreateProgram();
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	/* yakogl assigns them in declaration order; Mesa is told the same */
	glBindAttribLocation(prog, 0, "a_pos");
	glBindAttribLocation(prog, 1, "a_uv");
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &linked);
	if (!linked)
		return 1;
	glUseProgram(prog);
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
	glViewport(0, 0, TW, TH);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);

	glActiveTexture(GL_TEXTURE0);
	quad = quadrant_texture();
	glActiveTexture(GL_TEXTURE1);
	glGenTextures(1, &yel);
	glBindTexture(GL_TEXTURE_2D, yel);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, yellow);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glActiveTexture(GL_TEXTURE0);
	l0 = glGetUniformLocation(prog, "u_tex0");
	l1 = glGetUniformLocation(prog, "u_tex1");
	if (l0 < 0 || l1 < 0)
		return 1;

	glUniform1i(l0, 0);
	glUniform1i(l1, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish(); /* frame 1 */
	read_topdown();
	ok &= expect_px("2 units: red*0.5 + yellow*0.25", 16, 48, 0xff0040bf, 2);
	ok &= expect_px("2 units: blue*0.5 + yellow*0.25", 16, 16, 0xff804040, 2);

	glUniform1i(l0, 1);
	glUniform1i(l1, 0);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glFinish(); /* frame 2 */
	read_topdown();
	ok &= expect_px("swapped: yellow*0.5 + red*0.25", 16, 48, 0xff0080bf, 2);
	ok &= expect_px("swapped: yellow*0.5 + blue*0.25", 16, 16, 0xff408080, 2);
	ok &= glGetError() == GL_NO_ERROR;

	glDeleteTextures(1, &quad);
	glDeleteTextures(1, &yel);
	glDeleteProgram(prog);
	glDeleteBuffers(1, &vbo);
	eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroySurface(dpy, pb);
	eglDestroyContext(dpy, ctx);
	eglTerminate(dpy);
	if (ok)
		printf("verdict " NAME ": PASS\n");
	else
		printf("verdict " NAME ": FAIL, first bad check %d got 0x%08x want 0x%08x\n",
		       first_bad, first_got, first_want);
	return ok ? 0 : 1;
}
