/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test case 206 es3levels (driver/tools/pvrlab/gl_test.c, case_es3levels)
 * on its own, for a Mesa reference (tools/mali/mesa-board-ref.sh): a 16x16x4
 * 3D texture and a 16x16x2 2D array, levels 0-2 one colour each,
 * GL_TEXTURE_MAX_LEVEL 2, NEAREST_MIPMAP_NEAREST; es300/vol.frag and
 * es300/array.frag draw them into 16x16, 8x8 and 4x4 viewports (implicit LOD
 * 0, 1, 2). The draws, checks and their order are the case's (checks 1-6,
 * the non-PowerVR expectations).
 *
 *   cc -O1 -Wall -Werror -o es3levels es3levels.c -lEGL -lGLESv2
 *   ./es3levels [--libdir <mesa lib dir>]
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

#define NAME "es3levels"
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

static const char vol_fs_src[] =
	"#version 300 es\n"
	"precision highp float;\n"
	"precision highp int;\n"
	"uniform highp sampler3D vol;\n"
	"uniform float zz;\n"
	"in vec2 v_xy;\n"
	"layout(location = 0) out vec4 fragColor;\n"
	"void main()\n"
	"{\n"
	"    vec3 p = vec3(v_xy * 0.5 + 0.5, zz);\n"
	"    ivec3 sz = textureSize(vol, 0);\n"
	"    float a = textureLod(vol, p, 0.0).r;\n"
	"    vec4 f = texelFetch(vol, ivec3(0, 0, int(zz * float(sz.z))), 0);\n"
	"    fragColor = vec4(texture(vol, p).r, a, f.b, float(sz.z) / 255.0);\n"
	"}\n";

static const char array_fs_src[] =
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
	enum { SIDE = 16, DEPTH = 4, LAYERS = 2, LEVELS = 3 };
	/* level l's colour, RGBA */
	static const uint8_t col[LEVELS][4] = {
		{ 0x20, 0xe0, 0x60, 0xff }, { 0x90, 0x40, 0xc0, 0xff }, { 0xf0, 0x80, 0x10, 0xff },
	};
	static uint8_t texels[SIDE * SIDE * DEPTH * 4];
	static const GLenum targets[2] = { GL_TEXTURE_3D, GL_TEXTURE_2D_ARRAY };
	GLuint prog[2], tex[2] = { 0, 0 };
	bool ok = true;

	prog[0] = link_program(vs_src, vol_fs_src);
	prog[1] = link_program(vs_src, array_fs_src);
	if (!prog[0] || !prog[1])
		return -1;
	glGenTextures(2, tex);
	for (int i = 0; i < 2; i++) {
		glBindTexture(targets[i], tex[i]);
		for (int l = 0; l < LEVELS; l++) {
			/* a 3D level halves its depth, an array keeps its layers */
			const int side = SIDE >> l, depth = i == 0 ? (DEPTH >> l > 0 ? DEPTH >> l : 1) : LAYERS;

			fill_texels(texels, (uint32_t)(side * side * depth), col[l][0], col[l][1], col[l][2], col[l][3]);
			glTexImage3D(targets[i], l, GL_RGBA8, side, side, depth, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels);
		}
		glTexParameteri(targets[i], GL_TEXTURE_MAX_LEVEL, LEVELS - 1);
		glTexParameteri(targets[i], GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
		glTexParameteri(targets[i], GL_TEXTURE_MAG_FILTER, GL_NEAREST);
		glTexParameteri(targets[i], GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
		glTexParameteri(targets[i], GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
		glTexParameteri(targets[i], GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
	}
	if (!gl_ok("es3levels upload"))
		return -1;
	puv_quad_setup();
	glClearColor(0.0f, 0.0f, 0.0f, 0.0f);
	for (int i = 0; i < 2; i++) {
		glUseProgram(prog[i]);
		if (i == 0) {
			glUniform1i(glGetUniformLocation(prog[i], "vol"), 0);
			glUniform1f(glGetUniformLocation(prog[i], "zz"), 0.125f);
		} else {
			glUniform1i(glGetUniformLocation(prog[i], "layers"), 0);
			glUniform1f(glGetUniformLocation(prog[i], "layer"), 0.0f);
		}
		/* the case binds each target's texture to unit 0 at upload; both stay bound */
		for (int l = 0; l < LEVELS; l++) {
			const uint32_t vp = (uint32_t)SIDE >> l;
			char what[64];
			uint32_t want;

			glClear(GL_COLOR_BUFFER_BIT);   /* the whole window: glClear ignores the viewport */
			glViewport(0, 0, (GLsizei)vp, (GLsizei)vp);
			glDrawArrays(GL_TRIANGLES, 0, 6);
			glFinish();
			if (!gl_ok("es3levels draw"))
				return -1;
			read_topdown();
			if (i == 0) {
				snprintf(what, sizeof(what), "3D L%d", l);
				want = rgba_u32(col[l][0], col[0][0], col[0][2], DEPTH);
			} else {
				snprintf(what, sizeof(what), "array L%d", l);
				want = rgba_u32(col[l][0], col[l][1], col[l][2], col[0][3]);
			}
			/* the viewport's centre pixel, top-down */
			ok &= expect_px(what, vp / 2, TH - 1 - vp / 2, want, 2);
		}
	}
	glViewport(0, 0, TW, TH);
	glDeleteTextures(2, tex);
	return ok && gl_ok("es3levels teardown") ? 0 : -1;
}
