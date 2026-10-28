/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test case 42 es3array (driver/tools/pvrlab/gl_test.c, case_es3array) on
 * its own, for a Mesa reference (tools/mali/mesa-board-ref.sh): an 8x8x3
 * RGBA8 2D array texture, es300/array.frag (gl_test_es300_array_fs_src:
 * texture(), textureSize() and texelFetch() of one sampler2DArray) drawn at
 * layers 0, 1, 2; then layer 1 as an FBO colour attachment
 * (glFramebufferTextureLayer) cleared blue; then layers 0, 1, 2 again. The
 * draws, checks and their order are the case's (checks 1-6).
 *
 *   cc -O1 -Wall -Werror -o es3array es3array.c -lEGL -lGLESv2
 *   ./es3array [--libdir <mesa lib dir>]
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

#define NAME "es3array"
#define TW 64u
#define TH 64u

static const char vs_src[] =
	"#version 300 es\n"
	"layout(location = 0) in vec2 pos;\n"
	"out vec2 v_xy;\n"
	"void main()\n"
	"{\n"
	"    v_xy = pos;\n"
	"    gl_Position = vec4(pos, 0.0, 1.0);\n"
	"}\n";

static const char fs_src[] =
	"#version 300 es\n"
	"precision mediump float;\n"
	"precision mediump sampler2DArray;\n"
	"uniform sampler2DArray layers;\n"
	"uniform float layer;\n"
	"in vec2 v_xy;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    vec2 uv = v_xy * 0.5 + 0.5;\n"
	"    ivec3 sz = textureSize(layers, 0);\n"
	"    vec4 t = texture(layers, vec3(uv, layer));\n"
	"    vec4 f = texelFetch(layers, ivec3(int(uv.x * float(sz.x)), int(uv.y * float(sz.y)), int(layer)), 0);\n"
	"    fragColor = vec4(t.rgb, f.a);\n"
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
	static uint8_t texels[3][8 * 8 * 4];
	GLuint prog, tex = 0, fbo = 0;
	GLint layer_loc;
	bool ok = true;

	prog = link_program(vs_src, fs_src);
	if (!prog)
		return -1;
	for (int l = 0; l < 3; l++)
		fill_texels(texels[l], 64, (uint8_t)(l * 80), (uint8_t)(255 - l * 80), (uint8_t)(l * 40), (uint8_t)(100 + l * 50));
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
	glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA8, 8, 8, 3, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
	glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
	if (!gl_ok("array upload"))
		return -1;
	glUseProgram(prog);
	layer_loc = glGetUniformLocation(prog, "layer");
	glUniform1i(glGetUniformLocation(prog, "layers"), 0);
	puv_quad_setup();
	glViewport(0, 0, TW, TH);
	for (int l = 0; l < 3; l++) {
		char what[64];

		glUniform1f(layer_loc, (float)l);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		if (!gl_ok("array draw"))
			return -1;
		read_topdown();
		snprintf(what, sizeof(what), "layer %d (rgb texture, a texelFetch)", l);
		ok &= expect_px(what, 32, 32, rgba_u32((uint8_t)(l * 80), (uint8_t)(255 - l * 80), (uint8_t)(l * 40), (uint8_t)(100 + l * 50)), 2);
	}
	/* render into layer 1 (blue), then sample layers 0..2 again */
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, 1);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) {
		printf(NAME ": array layer FBO incomplete\n");
		return -1;
	}
	glViewport(0, 0, 8, 8);
	glClearColor(0.0f, 0.0f, 1.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glViewport(0, 0, TW, TH);
	for (int l = 0; l < 3; l++) {
		char what[64];
		uint32_t want = l == 1 ? rgba_u32(0, 0, 255, 255)
				     : rgba_u32((uint8_t)(l * 80), (uint8_t)(255 - l * 80), (uint8_t)(l * 40), (uint8_t)(100 + l * 50));

		glUniform1f(layer_loc, (float)l);
		glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
		glClear(GL_COLOR_BUFFER_BIT);
		glDrawArrays(GL_TRIANGLES, 0, 6);
		glFinish();
		read_topdown();
		snprintf(what, sizeof(what), "after FBO: layer %d", l);
		ok &= expect_px(what, 32, 32, want, 2);
	}
	glDeleteFramebuffers(1, &fbo);
	glDeleteTextures(1, &tex);
	return ok && gl_ok("es3array teardown") ? 0 : -1;
}
