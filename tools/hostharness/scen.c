/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * scen.c - harness program: GL/EGL call streams gl_test.c does not make.
 *
 * usage: scen <scenario> [args]   (HH_SHADERS = shader source directory)
 *   state N SPECIAL   float edge cases (SPECIAL=1: NaN/Inf too) through every
 *                     float-taking state call, a query sweep and a draw each step
 *   gets              every pname 0..0xffff through glGet*v / glIsEnabled, the
 *                     string / precision / texparam / internalformat /
 *                     renderbuffer / attachment / vertex-attrib / buffer /
 *                     program / shader queries, hints, pixel store
 *   egl               config / choose-config / surface / context / sync sweep,
 *                     bad arguments, pool exhaustion, terminate + reinit
 *   progs DIR ALL NOSTATE   link sweep over DIR (ALL=1: every vs x fs pair);
 *                     each linked program: every uniform set + read back
 *                     (arrays, over-long counts, mismatched setters), attribute
 *                     formats, divisors, all primitive modes, index types
 *   tex N SEED        texture uploads: 26 format/type pairs, UNPACK_*, storage,
 *                     mipmaps, ETC1/ETC2/EAC, CopyTex*, delete in flight
 *   buf N SEED        buffers, VAOs, MapBufferRange / Flush / Unmap, IPointer
 *   fbo N SEED        FBOs, renderbuffers, MSAA, blit, invalidate, readpixels
 *   stress N BIG      N draws (frame splits, arena fill with BIG client verts)
 *   ctx2              two contexts sharing objects, a third one, destroy current
 *   inst0             instanced attribute enabled without a pointer
 *   win               window surfaces (RGBA/XRGB/RGB565/bad stride) and native
 *                     pixmap EGL images as texture and renderbuffer
 * The scenarios follow the review harness rev-fidelity used to compare the
 * Rust port with C 0bbdb8a (reimplemented here).
 */
#define GL_GLEXT_PROTOTYPES 1
#define EGL_EGLEXT_PROTOTYPES 1
#include <dirent.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <GLES3/gl3.h>
#include <GLES2/gl2ext.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include "gles_osal.h"
#include "osal_internal.h"
#include "pvr/pvr_backend.h"
#include "hh.h"

static const char *SH;
static EGLDisplay dpy;
static EGLConfig cfg;
static EGLSurface pb;
static EGLContext ctx;
#define W 64
#define H 64

static uint32_t rs = 0x12345678;
static uint32_t rnd(void) { rs ^= rs << 13; rs ^= rs >> 17; rs ^= rs << 5; return rs; }
static float rndf(void) { return (float)(int32_t)(rnd() % 20001 - 10000) / 10000.0f; }

static char *slurp(const char *dir, const char *name)
{
	char p[1024];
	FILE *f;
	long n;
	char *b;

	snprintf(p, sizeof p, "%s/%s/%s", SH, dir, name);
	f = fopen(p, "rb");
	if (!f) { printf("HH-ERROR cannot open %s\n", p); exit(2); }
	fseek(f, 0, SEEK_END); n = ftell(f); rewind(f);
	b = malloc(n + 1);
	if (fread(b, 1, n, f) != (size_t)n) exit(2);
	b[n] = 0; fclose(f);
	return b;
}

/* drain the GL error queue (the wrapper logs every non-zero value) */
static void E(const char *w)
{
	int n = 0;

	(void)w;
	while (glGetError() && n++ < 8)
		;
}

static GLuint compile(GLenum type, const char *src)
{
	GLuint s = glCreateShader(type);
	GLint ok = 0, ll = 0;
	char log[512] = "";

	glShaderSource(s, 1, &src, NULL);
	glCompileShader(s);
	glGetShaderiv(s, GL_COMPILE_STATUS, &ok);
	glGetShaderiv(s, GL_INFO_LOG_LENGTH, &ll);
	if (!ok) { glGetShaderInfoLog(s, sizeof log, NULL, log); glDeleteShader(s); return 0; }
	return s;
}

static GLuint linkp(GLuint vs, GLuint fs)
{
	GLuint p = glCreateProgram();
	GLint ok = 0;
	char log[512] = "";

	glAttachShader(p, vs); glAttachShader(p, fs); glLinkProgram(p);
	glGetProgramiv(p, GL_LINK_STATUS, &ok);
	if (!ok) { glGetProgramInfoLog(p, sizeof log, NULL, log); glDeleteProgram(p); return 0; }
	return p;
}

static GLuint prog(const char *d1, const char *v, const char *d2, const char *f)
{
	char *a = slurp(d1, v), *b = slurp(d2, f);
	GLuint vs = compile(GL_VERTEX_SHADER, a), fs = compile(GL_FRAGMENT_SHADER, b), p = 0;

	if (vs && fs) p = linkp(vs, fs);
	free(a); free(b);
	return p;
}

static void backend_up(void)
{
	struct osal_zephyr_init oi = { 0 };

	if (g_osal->init(&oi)) exit(3);
	if (pvr_backend_init()) { printf("backend init failed\n"); exit(3); }
}

static void gl_up(int samples, int es)
{
	EGLint n = 0;
	EGLint ca[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_SAMPLES, samples, EGL_NONE };
	EGLint pa[] = { EGL_WIDTH, W, EGL_HEIGHT, H, EGL_NONE };
	EGLint xa[] = { EGL_CONTEXT_CLIENT_VERSION, es, EGL_NONE };

	backend_up();
	dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	eglInitialize(dpy, NULL, NULL);
	eglChooseConfig(dpy, ca, &cfg, 1, &n);
	pb = eglCreatePbufferSurface(dpy, cfg, pa);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, xa);
	eglMakeCurrent(dpy, pb, pb, ctx);
	eglGetError();
}

static const float edge[] = { 0.f, 1.f, -1.f, 0.5f, 2.f, 1e9f, -1e9f, 0.49999997f, 0.50000006f, 1e-30f, -0.f, 0.999999f,
			      1.0000001f, 3e9f, 5e9f, -3e9f, 1e20f, 1e38f };
/* HH_INRANGE=1 keeps every float in a range where a float->int conversion is
 * defined in C (the x86 artefacts of out-of-range casts then cannot occur) */
static int inrange;
static float ef(void)
{
	uint32_t r = rnd();

	if (r % 4 == 0) {
		float e = edge[(r >> 8) % (sizeof edge / sizeof edge[0])];
		return inrange && fabsf(e) > 1000.f ? 3.f : e;
	}
	if (r % 4 == 1) return (float)(rnd() % 100001) / 100000.f;
	if (r % 4 == 2) return (float)(rnd() % 256) / 255.f;
	return rndf() * 4.f;
}
static float special(unsigned i) { static const float s[] = { NAN, INFINITY, -INFINITY }; return s[i % 3]; }

/* ---------------------------------------------------------------- programs */
static GLuint tex2d[8];
static void mk_textures(void)
{
	uint32_t px[16 * 16];

	glGenTextures(8, tex2d);
	for (int i = 0; i < 8; i++) {
		for (int k = 0; k < 256; k++) px[k] = rnd();
		glActiveTexture(GL_TEXTURE0 + i); glBindTexture(GL_TEXTURE_2D, tex2d[i]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 16 >> (i & 1), 16, 0, GL_RGBA, GL_UNSIGNED_BYTE, px);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, (i & 2) ? GL_NEAREST : GL_LINEAR);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, (i & 4) ? GL_NEAREST : GL_LINEAR);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, (i % 3 == 0) ? GL_REPEAT : (i % 3 == 1) ? GL_CLAMP_TO_EDGE : GL_MIRRORED_REPEAT);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, (i % 3 == 1) ? GL_REPEAT : GL_CLAMP_TO_EDGE);
		if (i == 5) { glGenerateMipmap(GL_TEXTURE_2D); glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR); }
	}
	glActiveTexture(GL_TEXTURE0);
	E("mk_textures");
}

static int is_sampler(GLenum t)
{
	return t == GL_SAMPLER_2D || t == GL_SAMPLER_CUBE || t == GL_SAMPLER_3D || t == GL_SAMPLER_2D_ARRAY ||
	       t == GL_SAMPLER_2D_SHADOW || t == GL_SAMPLER_2D_ARRAY_SHADOW || t == GL_SAMPLER_CUBE_SHADOW || t == 0x8D66;
}

static void set_uniforms(GLuint p, int pi)
{
	GLint nu = 0, maxl = 0;

	glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &nu);
	glGetProgramiv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH, &maxl);
	for (GLint i = 0; i < nu + 1; i++) {
		char name[256] = "?";
		GLsizei len = -1;
		GLint size = -1;
		GLenum type = 0;
		static float fv[16 * 64];
		static GLint iv[4 * 64];
		static GLuint uv[4 * 64];

		glGetActiveUniform(p, i, (i % 5 == 4) ? 6 : (GLsizei)sizeof name, &len, &size, &type, name);
		GLint loc = glGetUniformLocation(p, name);
		if (i >= nu) { E("past-end uniform"); break; }
		if (i % 5 == 4) { glGetActiveUniform(p, i, sizeof name, &len, &size, &type, name); loc = glGetUniformLocation(p, name); }
		if (size > 1) {
			char n2[300];
			char *br = strchr(name, '[');

			if (br) *br = 0;
			for (int k = 0; k <= size; k += (size > 4 ? size / 3 : 1)) { snprintf(n2, sizeof n2, "%s[%d]", name, k); glGetUniformLocation(p, n2); }
			snprintf(n2, sizeof n2, "%s[%d]", name, size); glGetUniformLocation(p, n2);
			glGetUniformLocation(p, name);
		}
		int cnt = size > 64 ? 64 : size;
		if (cnt < 1) cnt = 1;
		int over = (pi + i) % 3 == 0 ? 2 : 0;   /* sometimes pass count beyond the array */
		if (cnt + over > 64) over = 0;
		for (int k = 0; k < 16 * 64; k++) fv[k] = (k % 7 == 3) ? ef() : rndf();
		for (int k = 0; k < 4 * 64; k++) { iv[k] = (GLint)(rnd() % 1000) - 500; uv[k] = rnd(); }
		switch (type) {
		case GL_FLOAT: glUniform1fv(loc, cnt + over, fv); break;
		case GL_FLOAT_VEC2: glUniform2fv(loc, cnt + over, fv); break;
		case GL_FLOAT_VEC3: glUniform3fv(loc, cnt + over, fv); break;
		case GL_FLOAT_VEC4: glUniform4fv(loc, cnt + over, fv); break;
		case GL_FLOAT_MAT2: glUniformMatrix2fv(loc, cnt + over, GL_FALSE, fv); break;
		case GL_FLOAT_MAT3: glUniformMatrix3fv(loc, cnt + over, GL_FALSE, fv); break;
		case GL_FLOAT_MAT4: glUniformMatrix4fv(loc, cnt + over, GL_FALSE, fv); break;
		case GL_INT: case GL_BOOL: glUniform1iv(loc, cnt + over, iv); break;
		case GL_INT_VEC2: case GL_BOOL_VEC2: glUniform2iv(loc, cnt + over, iv); break;
		case GL_INT_VEC3: case GL_BOOL_VEC3: glUniform3iv(loc, cnt + over, iv); break;
		case GL_INT_VEC4: case GL_BOOL_VEC4: glUniform4iv(loc, cnt + over, iv); break;
		case GL_UNSIGNED_INT: glUniform1uiv(loc, cnt + over, uv); break;
		case GL_UNSIGNED_INT_VEC2: glUniform2uiv(loc, cnt + over, uv); break;
		case GL_UNSIGNED_INT_VEC3: glUniform3uiv(loc, cnt + over, uv); break;
		case GL_UNSIGNED_INT_VEC4: glUniform4uiv(loc, cnt + over, uv); break;
		default:
			if (is_sampler(type)) { for (int k = 0; k < cnt; k++) iv[k] = (i + k + pi) % 8; glUniform1iv(loc, cnt, iv); }
			else hh_step("unhandled uniform type 0x%x", type);
		}
		E("uniform set");
		/* mismatched setters: error codes must agree */
		if ((pi + i) % 4 == 1) {
			glUniform1f(loc, 1.5f); glUniform1i(loc, 3); glUniform1ui(loc, 3); glUniform4f(loc, 1, 2, 3, 4);
			glUniform2i(loc, 1, 2); glUniformMatrix4fv(loc, 1, GL_TRUE, fv); glUniformMatrix3fv(loc, 1, GL_FALSE, fv);
			glUniform3ui(loc, 1, 2, 3); glUniform1fv(loc, -1, fv); glUniform1fv(loc, 0, fv);
			E("mismatch");
		}
		if (size > 1) {
			char n2[300];
			GLint l2;

			snprintf(n2, sizeof n2, "%s[%d]", name, size - 1);
			l2 = glGetUniformLocation(p, n2);
			if (type == GL_FLOAT_VEC4) glUniform4fv(l2, 3, fv + 8);
			else if (type == GL_FLOAT_MAT4) glUniformMatrix4fv(l2, 2, GL_FALSE, fv + 3);
			else if (type == GL_FLOAT) glUniform1fv(l2, 2, fv + 1);
			E("tail set");
		}
		{
			/* big buffers: the read-back length is what is compared */
			static float g[16 * 64];
			static GLint gi[16 * 64];
			static GLuint gu[16 * 64];

			glGetUniformfv(p, loc, g); glGetUniformiv(p, loc, gi); (void)gu;
			E("get");
		}
	}
	glUniform1f(-1, 1.f); glUniform1f(9999, 1.f); glUniform4fv(-2, 1, (float[4]){ 0 });
	E("bad loc");
}

static GLuint vbo, ibo8, ibo16, ibo32;
static void set_attribs(GLuint p, int pi)
{
	GLint na = 0;

	glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &na);
	for (int l = 0; l < 16; l++) { glDisableVertexAttribArray(l); glVertexAttribDivisor(l, 0); }
	for (GLint i = 0; i < na; i++) {
		char name[128] = "?";
		GLsizei len = 0;
		GLint size = 0;
		GLenum type = 0;

		glGetActiveAttrib(p, i, sizeof name, &len, &size, &type, name);
		GLint loc = glGetAttribLocation(p, name);
		if (loc < 0) continue;
		int mode = (pi + i) % 7;
		glBindBuffer(GL_ARRAY_BUFFER, vbo);
		if (i == 0 || mode < 3) { glVertexAttribPointer(loc, 4, GL_FLOAT, GL_FALSE, 64, (void *)(uintptr_t)(16 * (i % 4))); glEnableVertexAttribArray(loc); }
		else if (mode == 3) { glVertexAttribPointer(loc, 3, GL_UNSIGNED_BYTE, GL_TRUE, 64, (void *)(uintptr_t)3); glEnableVertexAttribArray(loc); }
		else if (mode == 4) { glVertexAttribPointer(loc, 2, GL_SHORT, (pi & 1) ? GL_TRUE : GL_FALSE, 0, (void *)(uintptr_t)2); glEnableVertexAttribArray(loc); }
		else if (mode == 5) { glVertexAttrib4f(loc, ef(), ef(), rndf(), 1.f); }
		else { glVertexAttribPointer(loc, 4, GL_FLOAT, GL_FALSE, 64, (void *)(uintptr_t)16); glEnableVertexAttribArray(loc); glVertexAttribDivisor(loc, 1 + pi % 2); }
		E("attrib");
	}
}

static void vary_state(void)
{
	static const GLenum bf[] = { GL_ZERO, GL_ONE, GL_SRC_COLOR, GL_ONE_MINUS_SRC_COLOR, GL_DST_COLOR, GL_ONE_MINUS_DST_COLOR,
				     GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA, GL_CONSTANT_COLOR,
				     GL_ONE_MINUS_CONSTANT_COLOR, GL_CONSTANT_ALPHA, GL_ONE_MINUS_CONSTANT_ALPHA, GL_SRC_ALPHA_SATURATE };
	static const GLenum be[] = { GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT, GL_MIN, GL_MAX };
	static const GLenum cf[] = { GL_NEVER, GL_LESS, GL_EQUAL, GL_LEQUAL, GL_GREATER, GL_NOTEQUAL, GL_GEQUAL, GL_ALWAYS };
	static const GLenum so[] = { GL_KEEP, GL_ZERO, GL_REPLACE, GL_INCR, GL_INCR_WRAP, GL_DECR, GL_DECR_WRAP, GL_INVERT };
	uint32_t r = rnd();

	if (r & 1) glEnable(GL_BLEND); else glDisable(GL_BLEND);
	glBlendFuncSeparate(bf[rnd() % 15], bf[rnd() % 15], bf[rnd() % 15], bf[rnd() % 15]);
	glBlendEquationSeparate(be[rnd() % 5], be[rnd() % 5]);
	glBlendColor(ef(), ef(), ef(), ef());
	if (r & 2) glEnable(GL_DEPTH_TEST); else glDisable(GL_DEPTH_TEST);
	glDepthFunc(cf[rnd() % 8]); glDepthMask((r >> 2) & 1);
	if (r & 8) glEnable(GL_STENCIL_TEST); else glDisable(GL_STENCIL_TEST);
	glStencilFuncSeparate(GL_FRONT, cf[rnd() % 8], (GLint)(rnd() % 600) - 100, rnd());
	glStencilFuncSeparate(GL_BACK, cf[rnd() % 8], (GLint)(rnd() % 600) - 100, rnd());
	glStencilOpSeparate(GL_FRONT, so[rnd() % 8], so[rnd() % 8], so[rnd() % 8]);
	glStencilOpSeparate(GL_BACK, so[rnd() % 8], so[rnd() % 8], so[rnd() % 8]);
	glStencilMaskSeparate(GL_FRONT, rnd()); glStencilMaskSeparate(GL_BACK, rnd() & 0xff);
	if (r & 16) glEnable(GL_CULL_FACE); else glDisable(GL_CULL_FACE);
	glCullFace((r & 32) ? GL_FRONT : (r & 64) ? GL_BACK : GL_FRONT_AND_BACK); glFrontFace((r & 128) ? GL_CW : GL_CCW);
	if (r & 256) glEnable(GL_POLYGON_OFFSET_FILL); else glDisable(GL_POLYGON_OFFSET_FILL);
	glPolygonOffset(ef() * 8.f, ef() * 8.f);
	if (r & 512) glEnable(GL_SCISSOR_TEST); else glDisable(GL_SCISSOR_TEST);
	glScissor((GLint)(rnd() % 100) - 20, (GLint)(rnd() % 100) - 20, rnd() % 90, rnd() % 90);
	glViewport((GLint)(rnd() % 100) - 30, (GLint)(rnd() % 100) - 30, 1 + rnd() % 120, 1 + rnd() % 120);
	glDepthRangef(ef(), ef());
	glColorMask((r >> 10) & 1, (r >> 11) & 1, (r >> 12) & 1, (r >> 13) & 1);
	if ((r >> 14) & 3) glColorMask(1, 1, 1, 1);
	glLineWidth(1.f + (float)(rnd() % 50) / 10.f);
	if (r & (1 << 16)) glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE); else glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
	if (r & (1 << 17)) glEnable(GL_SAMPLE_COVERAGE); else glDisable(GL_SAMPLE_COVERAGE);
	glSampleCoverage(ef(), (r >> 18) & 1);
	if (r & (1 << 19)) glEnable(GL_DITHER); else glDisable(GL_DITHER);
	if (r & (1 << 20)) glEnable(GL_RASTERIZER_DISCARD); else glDisable(GL_RASTERIZER_DISCARD);
	if ((r >> 21) & 7) glDisable(GL_RASTERIZER_DISCARD);
	E("vary_state");
}

static void draw_some(int k)
{
	static const GLenum modes[] = { GL_TRIANGLES, GL_TRIANGLE_STRIP, GL_TRIANGLE_FAN, GL_POINTS, GL_LINES, GL_LINE_STRIP, GL_LINE_LOOP };
	static const uint16_t ci[] = { 0, 1, 2, 2, 1, 3, 4, 5 };
	GLenum m = modes[k % 7];

	switch ((k / 7) % 8) {
	case 0: glDrawArrays(m, k % 3, 6 + k % 5); break;
	case 1: glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo16); glDrawElements(m, 7, GL_UNSIGNED_SHORT, (void *)(uintptr_t)(2 * (k % 4))); break;
	case 2: glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo8); glDrawElements(m, 9, GL_UNSIGNED_BYTE, (void *)(uintptr_t)(k % 5)); break;
	case 3: glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo32); glDrawElements(m, 8, GL_UNSIGNED_INT, (void *)(uintptr_t)(4 * (k % 3))); break;
	case 4: glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0); glDrawElements(m, 8, GL_UNSIGNED_SHORT, ci); break;
	case 5: glDrawArraysInstanced(m, 0, 6, 1 + k % 4); break;
	case 6: glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo16); glDrawElementsInstanced(m, 6, GL_UNSIGNED_SHORT, 0, 1 + k % 3); break;
	case 7: glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo16); glDrawRangeElements(m, 0, 5, 6, GL_UNSIGNED_SHORT, 0); break;
	}
	E("draw");
}

static void mk_buffers(void)
{
	static float v[64 * 64];
	uint8_t i8[64];
	uint16_t i16[64];
	uint32_t i32[64];

	for (int i = 0; i < 64 * 64; i++) v[i] = rndf();
	for (int i = 0; i < 64; i++) { i8[i] = rnd() % 32; i16[i] = rnd() % 32; i32[i] = rnd() % 32; }
	glGenBuffers(1, &vbo); glBindBuffer(GL_ARRAY_BUFFER, vbo); glBufferData(GL_ARRAY_BUFFER, sizeof v, v, GL_STATIC_DRAW);
	glGenBuffers(1, &ibo8); glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo8); glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof i8, i8, GL_STATIC_DRAW);
	glGenBuffers(1, &ibo16); glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo16); glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof i16, i16, GL_STATIC_DRAW);
	glGenBuffers(1, &ibo32); glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo32); glBufferData(GL_ELEMENT_ARRAY_BUFFER, sizeof i32, i32, GL_STATIC_DRAW);
	E("mk_buffers");
}

static int cmpstr(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }
static int list(const char *dir, const char *suffix, char **out, int max)
{
	char p[1024];
	DIR *d;
	struct dirent *e;
	int n = 0;

	snprintf(p, sizeof p, "%s/%s", SH, dir);
	d = opendir(p);
	if (!d) { printf("HH-ERROR no shader directory %s\n", p); return 0; }
	while ((e = readdir(d)) && n < max) {
		size_t l = strlen(e->d_name), s = strlen(suffix);
		if (l > s && !strcmp(e->d_name + l - s, suffix)) out[n++] = strdup(e->d_name);
	}
	closedir(d);
	qsort(out, n, sizeof *out, cmpstr);
	return n;
}

static void sc_progs(const char *dir, int allpairs, int nostate)
{
	static char *vs[512], *fs[512];
	static GLuint vsh[512], fsh[512];
	int nv = list(dir, ".vert", vs, 512), nf = list(dir, ".frag", fs, 512), pi = 0;

	mk_textures(); mk_buffers();
	for (int i = 0; i < nv; i++) { char *s = slurp(dir, vs[i]); hh_step("vs %s", vs[i]); vsh[i] = compile(GL_VERTEX_SHADER, s); free(s); }
	for (int i = 0; i < nf; i++) { char *s = slurp(dir, fs[i]); hh_step("fs %s", fs[i]); fsh[i] = compile(GL_FRAGMENT_SHADER, s); free(s); }
	for (int i = 0; i < nv; i++) {
		int drawn = 0;

		if (!vsh[i]) continue;
		for (int j = 0; j < nf; j++) {
			if (!fsh[j]) continue;
			if (!allpairs) { size_t l = strcspn(vs[i], "."); if (strncmp(vs[i], fs[j], l) || fs[j][l] != '.') continue; }
			hh_step("link %s + %s", vs[i], fs[j]);
			GLuint p = linkp(vsh[i], fsh[j]);
			if (!p) continue;
			if (drawn < 2) {
				hh_step("prog %s + %s", vs[i], fs[j]);
				glUseProgram(p); set_uniforms(p, pi); set_attribs(p, pi);
				for (int k = 0; k < 3; k++) { if (!nostate) vary_state(); draw_some(pi * 3 + k); }
				drawn++; pi++;
				if (pi % 4 == 0) { glFlush(); E("flush"); }
				if (pi % 6 == 0) eglSwapBuffers(dpy, pb);
				if (pi % 10 == 0) { glClearColor(ef(), ef(), ef(), ef()); glClearDepthf(ef()); glClearStencil(rnd()); glClear(((rnd() % 8) << 8) & (GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT)); E("clear"); }
				if (pi % 15 == 0) { uint32_t px[4]; glReadPixels(0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px); E("readpixels"); }
			}
			glUseProgram(0); glDeleteProgram(p);
		}
	}
	glFinish(); hh_memdump("end-progs");
}

static void sc_state(int n, int withspecial)
{
	GLuint p = prog(".", "passthrough.vert", ".", "uniform_color.frag");
	GLint lc;

	mk_buffers(); glUseProgram(p); lc = glGetUniformLocation(p, "u_color"); set_attribs(p, 0);
	for (int i = 0; i < n; i++) {
		float a = ef(), b = ef(), c = ef(), d = ef();

		if (withspecial && i % 5 == 0) a = special(i / 5);
		if (withspecial && i % 7 == 0) c = special(i / 7);
		hh_step("state %d", i);
		switch (i % 8) {
		case 0: glClearColor(a, b, c, d); glClear(GL_COLOR_BUFFER_BIT); break;
		case 1: glClearDepthf(a); glClearStencil((GLint)rnd()); glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT); break;
		case 2: glEnable(GL_BLEND); glBlendFunc(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_ALPHA); glBlendColor(a, b, c, d); break;
		case 3: glDepthRangef(a, b); break;
		case 4: glEnable(GL_POLYGON_OFFSET_FILL); glPolygonOffset(a * 16.f, c * 16.f); break;
		case 5: glViewport((GLint)(a * 50.f), (GLint)(b * 50.f), (GLsizei)(1 + fabsf(c) * 100.f > 4096.f ? 4096 : 1 + fabsf(c) * 100.f), rnd() % 200); break;
		case 6: glLineWidth(fminf(fabsf(a) * 8.f, inrange ? 64.f : 300.f) + 0.01f); glSampleCoverage(c, 1); glEnable(GL_SAMPLE_COVERAGE); break;
		case 7: glUniform4f(lc, a, b, c, d); glEnable(GL_SCISSOR_TEST); glScissor((GLint)(a * 40.f), (GLint)(b * 40.f), rnd() % 70, rnd() % 70); glClearColor(d, c, b, a); glClear(GL_COLOR_BUFFER_BIT); glDisable(GL_SCISSOR_TEST); break;
		}
		E("state");
		{
			GLfloat g[16];
			GLint gi[16];
			GLboolean gb[16];
			static const GLenum pn[] = { GL_COLOR_CLEAR_VALUE, GL_DEPTH_CLEAR_VALUE, GL_BLEND_COLOR, GL_DEPTH_RANGE,
						     GL_POLYGON_OFFSET_FACTOR, GL_VIEWPORT, GL_LINE_WIDTH, GL_SCISSOR_BOX,
						     GL_POLYGON_OFFSET_UNITS, GL_SAMPLE_COVERAGE_VALUE, GL_STENCIL_CLEAR_VALUE };

			for (unsigned q = 0; q < sizeof pn / sizeof pn[0]; q++) {
				glGetFloatv(pn[q], g); glGetBooleanv(pn[q], gb);
				/* integer queries of out-of-range float state are an x86
				 * artefact of the C build (cvttss2si): HH_INRANGE skips them */
				if (!inrange || !withspecial) glGetIntegerv(pn[q], gi);
			}
			E("gets");
		}
		draw_some(i); glFinish();
	}
}

static void sc_gets(void)
{
	GLuint p = prog(".", "uv.vert", ".", "texture.frag");
	GLuint fb, rb;

	glUseProgram(p); mk_textures(); mk_buffers();
	glGenFramebuffers(1, &fb); glGenRenderbuffers(1, &rb);
	for (unsigned pn = 0; pn < 0x10000; pn++) {
		GLint gi[32];
		GLfloat gf[32];
		GLboolean gb[32];
		GLenum e1, e2, e3, e4;

		glGetIntegerv(pn, gi); e1 = glGetError(); glGetFloatv(pn, gf); e2 = glGetError();
		glGetBooleanv(pn, gb); e3 = glGetError(); glIsEnabled(pn); e4 = glGetError();
		(void)e1; (void)e2; (void)e3; (void)e4;
	}
	for (unsigned cap = 0; cap < 0x10000; cap++) {
		glEnable(cap);
		if (!glGetError()) { glIsEnabled(cap); glDisable(cap); glIsEnabled(cap); glGetError(); }
	}
	for (unsigned s = 0x1F00; s < 0x1F04; s++) glGetString(s);
	glGetString(GL_SHADING_LANGUAGE_VERSION); glGetString(0x1234); glGetError();
	{ GLint ne = 0; glGetIntegerv(GL_NUM_EXTENSIONS, &ne); for (int i = 0; i <= ne; i++) glGetStringi(GL_EXTENSIONS, i); E("stringi"); }
	for (unsigned st = GL_FRAGMENT_SHADER; st <= GL_VERTEX_SHADER; st++)
		for (unsigned pt = 0x8DF0; pt <= 0x8DF6; pt++) { GLint r[2] = { 9, 9 }, pr = 9; glGetShaderPrecisionFormat(st, pt, r, &pr); glGetError(); }
	static const GLenum targets[] = { GL_TEXTURE_2D, GL_TEXTURE_CUBE_MAP, GL_TEXTURE_3D, GL_TEXTURE_2D_ARRAY, 0x8D65 };
	for (unsigned t = 0; t < 5; t++)
		for (unsigned pn = 0x2800; pn < 0x2804; pn++) { GLint v = -9; GLfloat f = -9; glGetTexParameteriv(targets[t], pn, &v); glGetError(); glGetTexParameterfv(targets[t], pn, &f); glGetError(); }
	for (unsigned pn = 0x813A; pn <= 0x813D; pn++) { GLint v = -9; glGetTexParameteriv(GL_TEXTURE_2D, pn, &v); glGetError(); }
	for (unsigned pn = 0x884A; pn <= 0x884F; pn++) { GLint v = -9; glGetTexParameteriv(GL_TEXTURE_2D, pn, &v); glGetError(); }
	for (unsigned pn = 0x2800; pn < 0x2804; pn++)
		for (unsigned v = 0x2600; v < 0x2710; v++) { glTexParameteri(GL_TEXTURE_2D, pn, v); glGetError(); }
	for (unsigned pn = 0x2802; pn < 0x2804; pn++) {
		static const GLenum w[] = { GL_REPEAT, GL_CLAMP_TO_EDGE, GL_MIRRORED_REPEAT, 0x812D, 0x2900 };
		for (int k = 0; k < 5; k++) { glTexParameterf(GL_TEXTURE_2D, pn, (float)w[k]); glGetError(); }
	}
	static const GLenum ifs[] = { GL_RGBA8, GL_RGB8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT24,
				      GL_DEPTH24_STENCIL8, GL_STENCIL_INDEX8, GL_R8, GL_RG8, GL_RGBA16F, GL_RGBA32F, GL_R16F,
				      GL_DEPTH_COMPONENT32F, GL_DEPTH32F_STENCIL8, GL_SRGB8_ALPHA8, GL_RGB10_A2, GL_RGBA, GL_RGB, 0x1234 };
	for (unsigned k = 0; k < sizeof ifs / sizeof ifs[0]; k++) {
		GLint v[8] = { -9, -9, -9, -9, -9, -9, -9, -9 };

		hh_step("internalformat 0x%x", ifs[k]);
		glGetInternalformativ(GL_RENDERBUFFER, ifs[k], GL_NUM_SAMPLE_COUNTS, 1, v); glGetError();
		glGetInternalformativ(GL_RENDERBUFFER, ifs[k], GL_SAMPLES, 7, v + 1); glGetError();
		for (int smp = 0; smp <= 8; smp += 2) {
			static const GLenum atn[] = { GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_COLOR_ATTACHMENT0 };

			glBindRenderbuffer(GL_RENDERBUFFER, rb); glRenderbufferStorageMultisample(GL_RENDERBUFFER, smp, ifs[k], 33, 17); glGetError();
			for (unsigned pn = 0x8D42; pn <= 0x8D44; pn++) { GLint q = -9; glGetRenderbufferParameteriv(GL_RENDERBUFFER, pn, &q); }
			for (unsigned pn = 0x8D50; pn <= 0x8D55; pn++) { GLint q = -9; glGetRenderbufferParameteriv(GL_RENDERBUFFER, pn, &q); }
			{ GLint q = -9; glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &q); glGetError(); }
			glBindFramebuffer(GL_FRAMEBUFFER, fb);
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb); glCheckFramebufferStatus(GL_FRAMEBUFFER);
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, 0);
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb); glCheckFramebufferStatus(GL_FRAMEBUFFER);
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, 0);
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb); glCheckFramebufferStatus(GL_FRAMEBUFFER);
			for (unsigned at = 0; at < 3; at++)
				for (unsigned pn = 0x8CD0; pn <= 0x8CD4; pn++) { GLint q = -9; glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, atn[at], pn, &q); glGetError(); }
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, 0);
			glBindFramebuffer(GL_FRAMEBUFFER, 0); glGetError();
		}
	}
	for (int i = 0; i < 17; i++) {
		GLint v[4] = { -9, -9, -9, -9 };
		GLfloat f[4] = { -9, -9, -9, -9 };
		void *ptr = (void *)1;

		for (unsigned pn = 0x8622; pn <= 0x8626; pn++) glGetVertexAttribiv(i, pn, v);
		glGetVertexAttribiv(i, GL_VERTEX_ATTRIB_ARRAY_NORMALIZED, v); glGetVertexAttribiv(i, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, v);
		glGetVertexAttribiv(i, GL_VERTEX_ATTRIB_ARRAY_DIVISOR, v); glGetVertexAttribiv(i, GL_VERTEX_ATTRIB_ARRAY_INTEGER, v);
		glGetVertexAttribfv(i, GL_CURRENT_VERTEX_ATTRIB, f);
		glGetVertexAttribPointerv(i, GL_VERTEX_ATTRIB_ARRAY_POINTER, &ptr); glGetError();
	}
	{
		static const GLenum bp[] = { GL_BUFFER_SIZE, GL_BUFFER_USAGE, GL_BUFFER_MAPPED, GL_BUFFER_ACCESS_FLAGS, GL_BUFFER_MAP_LENGTH, GL_BUFFER_MAP_OFFSET, 0x1234 };
		static const GLenum bt[] = { GL_ARRAY_BUFFER, GL_ELEMENT_ARRAY_BUFFER, GL_UNIFORM_BUFFER, GL_PIXEL_UNPACK_BUFFER, GL_COPY_READ_BUFFER, GL_TRANSFORM_FEEDBACK_BUFFER, 0x1234 };
		for (int t = 0; t < 7; t++)
			for (int q = 0; q < 7; q++) { GLint v = -9; glGetBufferParameteriv(bt[t], bp[q], &v); glGetError(); }
	}
	{
		static const GLenum pp[] = { GL_DELETE_STATUS, GL_LINK_STATUS, GL_VALIDATE_STATUS, GL_INFO_LOG_LENGTH, GL_ATTACHED_SHADERS, GL_ACTIVE_ATTRIBUTES,
					     GL_ACTIVE_ATTRIBUTE_MAX_LENGTH, GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH, GL_ACTIVE_UNIFORM_BLOCKS,
					     GL_TRANSFORM_FEEDBACK_VARYINGS, GL_PROGRAM_BINARY_LENGTH, 0x1234 };
		static const GLenum sp[] = { GL_SHADER_TYPE, GL_DELETE_STATUS, GL_COMPILE_STATUS, GL_INFO_LOG_LENGTH, GL_SHADER_SOURCE_LENGTH, 0x1234 };
		GLuint sh[4] = { 0 };
		GLsizei c = -1, l = -1;
		char src[64];

		for (int q = 0; q < 13; q++) { GLint v = -9; glGetProgramiv(p, pp[q], &v); glGetError(); }
		glValidateProgram(p); { GLint v = -9; glGetProgramiv(p, GL_VALIDATE_STATUS, &v); }
		glGetAttachedShaders(p, 4, &c, sh);
		for (int q = 0; q < 6; q++) { GLint v = -9; glGetShaderiv(sh[0], sp[q], &v); glGetError(); }
		glGetShaderSource(sh[0], sizeof src, &l, src); glGetShaderSource(sh[0], 1, &l, src); glGetShaderSource(sh[0], 0, &l, src); glGetError();
		glGetProgramInfoLog(p, sizeof src, &l, src); glGetShaderInfoLog(sh[0], sizeof src, &l, src);
		glIsShader(sh[0]); glIsShader(p); glIsProgram(p); glIsProgram(sh[0]);
		glDetachShader(p, sh[1]); glGetAttachedShaders(p, 4, &c, sh); glAttachShader(p, sh[1]); glGetError();
		glBindAttribLocation(p, 3, "a_uv"); glLinkProgram(p); glGetAttribLocation(p, "a_uv"); glGetError();
		glReleaseShaderCompiler(); glShaderBinary(1, sh, 0x1234, src, 4); glGetError();
	}
	{
		static const GLenum ht[] = { GL_GENERATE_MIPMAP_HINT, GL_FRAGMENT_SHADER_DERIVATIVE_HINT, 0x1234 }, hm[] = { GL_FASTEST, GL_NICEST, GL_DONT_CARE, 0 };
		for (int a = 0; a < 3; a++) for (int b = 0; b < 4; b++) { glHint(ht[a], hm[b]); glGetError(); }
	}
	for (unsigned pn = 0x0CF0; pn <= 0x0D05; pn++) for (int v = -1; v <= 9; v++) { glPixelStorei(pn, v); glGetError(); }
	for (unsigned pn = 0x806D; pn <= 0x806E; pn++) { glPixelStorei(pn, 3); glGetError(); }
	/* back to the defaults (the sweep above left arbitrary unpack state) */
	for (unsigned pn = 0x0CF0; pn <= 0x0CF5; pn++) glPixelStorei(pn, 0);
	glPixelStorei(GL_UNPACK_SKIP_IMAGES, 0); glPixelStorei(GL_UNPACK_IMAGE_HEIGHT, 0);
	for (unsigned pn = 0x0D00; pn <= 0x0D04; pn++) glPixelStorei(pn, 0);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 4); glPixelStorei(GL_PACK_ALIGNMENT, 4);
	glGetError();
	/* the remaining small entry points: each once with sane and bad args */
	glIsBuffer(vbo); glIsBuffer(9999); glIsTexture(tex2d[0]); glIsTexture(9999); glIsFramebuffer(fb); glIsRenderbuffer(rb);
	glIsRenderbuffer(9999); glIsFramebuffer(9999); glIsEnabled(GL_BLEND); glIsVertexArrayOES(0);
	/* ES 3.0 texture objects the library takes (3D / array paths) */
	{
		static uint8_t v3[4 * 4 * 3 * 4];
		GLuint t3;
		GLfloat pf[4] = { 1, 0, 0, 1 };
		GLint pi[4] = { GL_NEAREST, 0, 0, 0 };

		glGenTextures(1, &t3);
		glBindTexture(GL_TEXTURE_2D_ARRAY, t3);
		glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_RGBA8, 4, 4, 3, 0, GL_RGBA, GL_UNSIGNED_BYTE, v3); glGetError();
		glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 1, 1, 1, 2, 2, 1, GL_RGBA, GL_UNSIGNED_BYTE, v3); glGetError();
		glTexStorage3D(GL_TEXTURE_3D, 2, GL_RGBA8, 4, 4, 4); glGetError();
		glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, 0, GL_COMPRESSED_RGB8_ETC2, 4, 4, 3, 0, 24, v3); glGetError();
		glTexParameterfv(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_LOD, pf); glTexParameteriv(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MAG_FILTER, pi);
		glTexParameterfv(GL_TEXTURE_2D, 0x1004 /* GL_TEXTURE_BORDER_COLOR */, pf); glGetError();
		glBindFramebuffer(GL_FRAMEBUFFER, fb);
		glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, t3, 0, 1); glCheckFramebufferStatus(GL_FRAMEBUFFER); glGetError();
		glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, 0, 0, 0);
		glBindFramebuffer(GL_FRAMEBUFFER, 0);
		glDeleteTextures(1, &t3);
		glBindTexture(GL_TEXTURE_2D, tex2d[0]);
	}
	/* scalar uniform setters on a sampler */
	{
		GLint l = glGetUniformLocation(p, "u_tex");
		glUniform2i(l, 1, 2); glUniform3i(l, 1, 2, 3); glUniform4i(l, 1, 2, 3, 4); glUniform1i(l, 0);
		glUniform2f(l, 1, 2); glUniform3f(l, 1, 2, 3); glUniform2ui(l, 1, 2); glUniform4ui(l, 1, 2, 3, 4); glUniform3ui(l, 1, 2, 3);
		glUniform1ui(l, 1); glUniform2iv(l, 1, (GLint[2]){ 1, 2 }); glUniform3iv(l, 1, (GLint[3]){ 1, 2, 3 }); glGetError();
		glUniform1uiv(l, 1, (GLuint[1]){ 1 }); glUniform2uiv(l, 1, (GLuint[2]){ 1, 2 });
		glUniform3uiv(l, 1, (GLuint[3]){ 1, 2, 3 }); glUniform4uiv(l, 1, (GLuint[4]){ 1, 2, 3, 4 }); glGetError();
	}
	glStencilFunc(GL_LEQUAL, 3, 0xff); glStencilOp(GL_KEEP, GL_INCR, GL_DECR); glStencilMask(0x0f);
	glBlendEquation(GL_FUNC_SUBTRACT); glBlendEquation(0x1234); glBlendFunc(GL_ONE, 0x1234); glGetError();
	glFinish(); glFlush();
	glDeleteFramebuffers(1, &fb); glDeleteRenderbuffers(1, &rb); glDeleteBuffers(1, &vbo); glDeleteTextures(8, tex2d);
	E("end");
}

static void sc_egl(void)
{
	EGLConfig cs[64];
	EGLint n = 0, maj = -1, min = -1;

	backend_up();
	dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	eglInitialize(dpy, &maj, &min);
	for (unsigned s = 0x3053; s <= 0x3055; s++) eglQueryString(dpy, s);
	eglQueryString(dpy, EGL_CLIENT_APIS); eglQueryString(EGL_NO_DISPLAY, EGL_EXTENSIONS); eglGetError();
	eglGetConfigs(dpy, cs, 64, &n);
	eglGetConfigs(dpy, NULL, 0, &n);
	for (int i = 0; i < n; i++)
		for (unsigned a = 0x3020; a <= 0x3043; a++) { EGLint v = -9; eglGetConfigAttrib(dpy, cs[i], a, &v); eglGetError(); }
	static const EGLint attrs[] = { EGL_RED_SIZE, EGL_GREEN_SIZE, EGL_BLUE_SIZE, EGL_ALPHA_SIZE, EGL_DEPTH_SIZE, EGL_STENCIL_SIZE,
					EGL_SAMPLES, EGL_SAMPLE_BUFFERS, EGL_SURFACE_TYPE, EGL_RENDERABLE_TYPE, EGL_BUFFER_SIZE,
					EGL_CONFIG_ID, EGL_NATIVE_RENDERABLE, EGL_COLOR_BUFFER_TYPE, EGL_CONFORMANT, EGL_LEVEL,
					EGL_CONFIG_CAVEAT, EGL_TRANSPARENT_TYPE, EGL_MAX_SWAP_INTERVAL, EGL_MIN_SWAP_INTERVAL,
					EGL_NATIVE_VISUAL_ID, EGL_BIND_TO_TEXTURE_RGBA, EGL_MATCH_NATIVE_PIXMAP, 0x3099 };
	for (int it = 0; it < 3000; it++) {
		EGLint al[16], k = 0, na = rnd() % 4, m = -9;
		EGLConfig out[64];

		for (unsigned q = 0; q < (unsigned)na; q++) {
			uint32_t r;
			al[k++] = attrs[rnd() % 24];
			r = rnd();
			al[k++] = (r % 5 == 0) ? EGL_DONT_CARE : (r % 5 == 1) ? (EGLint)(rnd() % 0x80) : (EGLint)(rnd() % 10);
		}
		al[k] = EGL_NONE;
		memset(out, 0, sizeof out);
		eglChooseConfig(dpy, al, out, (it % 7 == 0) ? 2 : 64, &m);
		eglGetError();
	}
	for (int i = 0; i < n; i++) {
		EGLint pa[] = { EGL_WIDTH, 40 + i, EGL_HEIGHT, 24, EGL_NONE }, xa[] = { EGL_CONTEXT_CLIENT_VERSION, 2 + (i & 1), EGL_NONE };
		EGLSurface s = eglCreatePbufferSurface(dpy, cs[i], pa);
		EGLContext c = eglCreateContext(dpy, cs[i], EGL_NO_CONTEXT, xa);

		eglGetError();
		hh_step("config %d", i);
		if (s == EGL_NO_SURFACE || c == EGL_NO_CONTEXT) { if (s) eglDestroySurface(dpy, s); if (c) eglDestroyContext(dpy, c); continue; }
		eglMakeCurrent(dpy, s, s, c); eglGetError();
		for (unsigned a = 0x3020; a <= 0x30A0; a++) {
			EGLint v = -9;
			eglQuerySurface(dpy, s, a, &v); eglGetError();
			eglQueryContext(dpy, c, a, &v); eglGetError();
			eglSurfaceAttrib(dpy, s, a, 1); eglGetError();
		}
		glGetString(GL_VERSION); glGetString(GL_SHADING_LANGUAGE_VERSION);
		glClearColor(0.1f * i, 0.3f, 0.7f, 1.f); glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
		{ uint8_t px[16]; glReadPixels(3, 3, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px); }
		eglSwapInterval(dpy, i); eglGetError(); eglSwapBuffers(dpy, s); eglGetError();
		{
			EGLSync sy = eglCreateSync(dpy, EGL_SYNC_FENCE, NULL);
			EGLAttrib v = -9;

			eglGetError();
			if (sy) {
				eglClientWaitSync(dpy, sy, EGL_SYNC_FLUSH_COMMANDS_BIT, 1000);
				eglGetSyncAttrib(dpy, sy, EGL_SYNC_STATUS, &v); eglGetSyncAttrib(dpy, sy, EGL_SYNC_TYPE, &v);
				eglWaitSync(dpy, sy, 0); eglDestroySync(dpy, sy);
			}
			eglGetError();
		}
		eglWaitGL(); eglWaitNative(EGL_CORE_NATIVE_ENGINE); eglWaitClient(); eglGetError();
		eglDestroySurface(dpy, s); eglDestroyContext(dpy, c); eglGetError();
		glClear(GL_COLOR_BUFFER_BIT); glGetError();
		eglGetCurrentContext(); eglGetCurrentSurface(EGL_DRAW); eglGetCurrentSurface(EGL_READ); eglGetCurrentDisplay();
		eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT); eglGetError();
		eglMakeCurrent(dpy, s, s, c); eglGetError();
	}
	hh_step("bad arguments");
	{ EGLint pa[] = { EGL_WIDTH, -1, EGL_HEIGHT, 8, EGL_NONE }; eglCreatePbufferSurface(dpy, cs[0], pa); eglGetError(); }
	{ EGLint pa[] = { EGL_WIDTH, 0, EGL_HEIGHT, 0, EGL_NONE }; eglCreatePbufferSurface(dpy, cs[0], pa); eglGetError(); }
	{ EGLint pa[] = { EGL_WIDTH, 5000, EGL_HEIGHT, 5000, EGL_LARGEST_PBUFFER, 1, EGL_NONE }; EGLSurface s = eglCreatePbufferSurface(dpy, cs[0], pa); EGLint w = -1; if (s) eglQuerySurface(dpy, s, EGL_WIDTH, &w); eglGetError(); }
	{ EGLint pa[] = { 0x1234, 1, EGL_NONE }; eglCreatePbufferSurface(dpy, cs[0], pa); eglGetError(); }
	{ EGLint pa[] = { EGL_WIDTH, 8, EGL_HEIGHT, 8, EGL_TEXTURE_FORMAT, EGL_TEXTURE_RGBA, EGL_TEXTURE_TARGET, EGL_TEXTURE_2D, EGL_MIPMAP_TEXTURE, 1, EGL_NONE };
	  EGLSurface s = eglCreatePbufferSurface(dpy, cs[0], pa); eglGetError();
	  if (s) { eglBindTexImage(dpy, s, EGL_BACK_BUFFER); eglReleaseTexImage(dpy, s, EGL_BACK_BUFFER); eglGetError(); } }
	eglBindTexImage(dpy, (EGLSurface)0x77, EGL_BACK_BUFFER); eglGetError();
	eglCopyBuffers(dpy, pb, NULL); eglGetError();
	{ EGLint xa[] = { EGL_CONTEXT_CLIENT_VERSION, 1, EGL_NONE }; eglCreateContext(dpy, cs[0], EGL_NO_CONTEXT, xa); eglGetError(); }
	{ EGLint xa[] = { EGL_CONTEXT_CLIENT_VERSION, 4, EGL_NONE }; eglCreateContext(dpy, cs[0], EGL_NO_CONTEXT, xa); eglGetError(); }
	{ EGLint xa[] = { EGL_CONTEXT_MAJOR_VERSION, 3, EGL_CONTEXT_MINOR_VERSION, 1, EGL_NONE }; eglCreateContext(dpy, cs[0], EGL_NO_CONTEXT, xa); eglGetError(); }
	{ EGLint xa[] = { EGL_CONTEXT_MAJOR_VERSION, 3, 0x30FB, 1, 0x3100, 1, 0x31BD, 0x31BE, EGL_NONE }; eglCreateContext(dpy, cs[0], EGL_NO_CONTEXT, xa); eglGetError(); }
	eglCreateContext(dpy, cs[0], EGL_NO_CONTEXT, NULL); eglGetError();
	eglCreateContext(dpy, (EGLConfig)0x1234, EGL_NO_CONTEXT, NULL); eglGetError();
	eglCreateContext(dpy, cs[0], (EGLContext)0x1234, NULL); eglGetError();
	for (int i = 0; i < 40; i++) { EGLContext c = eglCreateContext(dpy, cs[0], EGL_NO_CONTEXT, NULL); if (!c) { eglGetError(); break; } }
	for (int i = 0; i < 40; i++) { EGLint pa[] = { EGL_WIDTH, 8, EGL_HEIGHT, 8, EGL_NONE }; EGLSurface s = eglCreatePbufferSurface(dpy, cs[0], pa); if (!s) { eglGetError(); break; } }
	eglCreateWindowSurface(dpy, cs[0], (EGLNativeWindowType)0, NULL); eglGetError();
	eglCreatePixmapSurface(dpy, cs[0], (EGLNativePixmapType)0, NULL); eglGetError();
	eglCreatePlatformWindowSurface(dpy, cs[0], NULL, NULL); eglGetError();
	eglCreatePlatformPixmapSurface(dpy, cs[0], NULL, NULL); eglGetError();
	eglCreatePbufferFromClientBuffer(dpy, EGL_OPENVG_IMAGE, NULL, cs[0], NULL); eglGetError();
	eglBindAPI(EGL_OPENGL_ES_API); eglBindAPI(EGL_OPENGL_API); eglBindAPI(EGL_OPENVG_API); eglQueryAPI(); eglGetError();
	eglBindAPI(EGL_OPENGL_ES_API);
	eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, NULL, NULL); eglGetError();
	eglGetDisplay((EGLNativeDisplayType)0x55); eglGetError();
	{
		static const char *names[] = { "glDrawArrays", "eglSwapBuffers", "glTexStorage2D", "glNoSuch", "glUniform1ui",
					       "glBindVertexArray", "glBindVertexArrayOES", "glMapBufferRange", "glFenceSync",
					       "glDrawBuffers", "eglCreateImageKHR", "glEGLImageTargetTexture2DOES", "" };
		for (int i = 0; i < 13; i++) eglGetProcAddress(names[i]);
	}
	eglTerminate(dpy); eglGetError();
	eglGetConfigs(dpy, cs, 64, &n); eglGetError();
	eglInitialize(dpy, NULL, NULL); eglReleaseThread();
	eglTerminate(dpy);
}

static void sc_tex(int iters)
{
	static const struct { GLenum ifmt, fmt, type; } F[] = {
		{ GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE }, { GL_RGB, GL_RGB, GL_UNSIGNED_BYTE }, { GL_LUMINANCE, GL_LUMINANCE, GL_UNSIGNED_BYTE },
		{ GL_ALPHA, GL_ALPHA, GL_UNSIGNED_BYTE }, { GL_LUMINANCE_ALPHA, GL_LUMINANCE_ALPHA, GL_UNSIGNED_BYTE }, { GL_RGB, GL_RGB, GL_UNSIGNED_SHORT_5_6_5 },
		{ GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4 }, { GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1 }, { GL_R8, GL_RED, GL_UNSIGNED_BYTE },
		{ GL_RG8, GL_RG, GL_UNSIGNED_BYTE }, { GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE }, { GL_RGB8, GL_RGB, GL_UNSIGNED_BYTE },
		{ GL_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5 }, { GL_RGBA4, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4 }, { GL_RGB5_A1, GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1 },
		{ GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT }, { GL_RGBA32F, GL_RGBA, GL_FLOAT }, { GL_R16F, GL_RED, GL_HALF_FLOAT },
		{ GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE }, { GL_RGBA, GL_RGB, GL_UNSIGNED_BYTE }, { GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT },
		{ GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT }, { GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8 },
		{ GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV }, { 0x80E1, 0x80E1, GL_UNSIGNED_BYTE }, { GL_R8, GL_LUMINANCE, GL_UNSIGNED_BYTE },
	};
	static uint8_t buf[1 << 20];
	GLuint t[8];
	GLuint p = prog(".", "uv.vert", ".", "texture.frag");

	mk_buffers(); glUseProgram(p); set_attribs(p, 0); glUniform1i(glGetUniformLocation(p, "u_tex"), 0);
	glGenTextures(8, t);
	for (int it = 0; it < iters; it++) {
		unsigned f = rnd() % (sizeof F / sizeof F[0]);
		int w = 1 + rnd() % 37, h = 1 + rnd() % 29, lvl = (rnd() % 4 == 0) ? (int)(rnd() % 4) : 0;

		if (rnd() % 8 == 0) { w = 1 << (rnd() % 7); h = 1 << (rnd() % 7); }
		for (unsigned k = 0; k < sizeof buf; k += 4) { uint32_t r = rnd(); memcpy(buf + k, &r, 4); }
		int al = 1 << (rnd() % 4), rl = (rnd() % 3 == 0) ? w + (int)(rnd() % 9) : 0, sp = (rnd() % 4 == 0) ? (int)(rnd() % 5) : 0, sr = (rnd() % 4 == 0) ? (int)(rnd() % 5) : 0;
		glPixelStorei(GL_UNPACK_ALIGNMENT, al); glPixelStorei(GL_UNPACK_ROW_LENGTH, rl); glPixelStorei(GL_UNPACK_SKIP_PIXELS, sp); glPixelStorei(GL_UNPACK_SKIP_ROWS, sr);
		glBindTexture(GL_TEXTURE_2D, t[it % 8]);
		hh_step("tex it=%d f=%u %dx%d lvl=%d", it, f, w, h, lvl);
		switch (rnd() % 6) {
		case 0: case 1: glTexImage2D(GL_TEXTURE_2D, lvl, F[f].ifmt, w, h, 0, F[f].fmt, F[f].type, (rnd() % 9 == 0) ? NULL : buf); E("teximage"); break;
		case 2:
			glTexImage2D(GL_TEXTURE_2D, 0, F[f].ifmt, w + 8, h + 8, 0, F[f].fmt, F[f].type, NULL); E("teximage0");
			glTexSubImage2D(GL_TEXTURE_2D, 0, rnd() % 9, rnd() % 9, w, h, F[f].fmt, F[f].type, buf); E("texsub");
			glTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, w + 8, h, F[f].fmt, F[f].type, buf); E("texsub-oob");
			glTexSubImage2D(GL_TEXTURE_2D, 0, -1, 0, 1, 1, F[f].fmt, F[f].type, buf); E("texsub-neg");
			glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, F[(f + 1) % 26].fmt, F[(f + 1) % 26].type, buf); E("texsub-fmt");
			break;
		case 3: {
			GLuint nt;
			glGenTextures(1, &nt); glBindTexture(GL_TEXTURE_2D, nt); glTexStorage2D(GL_TEXTURE_2D, 1 + rnd() % 5, F[f].ifmt, w, h); E("texstorage");
			glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, w, h, F[f].fmt, F[f].type, buf); E("storage-sub");
			glTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, w / 2 ? w / 2 : 1, h / 2 ? h / 2 : 1, F[f].fmt, F[f].type, buf); E("storage-sub1");
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, buf); E("immutable-teximage");
			glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 4, 4); E("storage-twice");
			glDrawArrays(GL_TRIANGLES, 0, 3); E("draw"); if (rnd() % 2) glFlush();
			glDeleteTextures(1, &nt); glBindTexture(GL_TEXTURE_2D, t[it % 8]);
			break;
		}
		case 4:
			glTexImage2D(GL_TEXTURE_2D, 0, F[f].ifmt, w, h, 0, F[f].fmt, F[f].type, buf); E("teximage"); glGenerateMipmap(GL_TEXTURE_2D); E("genmip");
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST + rnd() % 4);
			glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, rnd() % 6); glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_BASE_LEVEL, rnd() % 2); E("mipparams");
			break;
		case 5: {
			static const GLenum cf[] = { 0x8D64, GL_COMPRESSED_RGB8_ETC2, GL_COMPRESSED_SRGB8_ETC2, GL_COMPRESSED_RGBA8_ETC2_EAC,
						     GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
						     GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, GL_COMPRESSED_R11_EAC, GL_COMPRESSED_SIGNED_R11_EAC,
						     GL_COMPRESSED_RG11_EAC, GL_COMPRESSED_SIGNED_RG11_EAC, 0x83F0 };
			GLenum c = cf[rnd() % 12];
			int bw = (w + 3) / 4, bh = (h + 3) / 4;
			int bs = (c == GL_COMPRESSED_RGBA8_ETC2_EAC || c == GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC || c == GL_COMPRESSED_RG11_EAC || c == GL_COMPRESSED_SIGNED_RG11_EAC) ? 16 : 8;
			glCompressedTexImage2D(GL_TEXTURE_2D, lvl, c, w, h, 0, bw * bh * bs, buf); E("comp");
			glCompressedTexImage2D(GL_TEXTURE_2D, 0, c, w, h, 0, bw * bh * bs - 1, buf); E("comp-badsize");
			glCompressedTexSubImage2D(GL_TEXTURE_2D, lvl, 0, 0, w, h, c, bw * bh * bs, buf + 64); E("compsub");
			if (w > 8 && h > 8) {
				glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 4, 4, c, bs, buf + 128); E("compsub-blk");
				glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 2, 4, 4, 4, c, bs, buf + 128); E("compsub-misaligned");
			}
			break;
		}
		}
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, (rnd() % 2) ? GL_REPEAT : GL_CLAMP_TO_EDGE);
		glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST + rnd() % 2); E("params");
		glDrawArrays(GL_TRIANGLES, 0, 3); E("draw");
		switch (rnd() % 6) {
		case 0: glFlush(); break;
		case 1: glFinish(); break;
		case 2: eglSwapBuffers(dpy, pb); break;
		case 3: glCopyTexImage2D(GL_TEXTURE_2D, 0, (rnd() % 2) ? GL_RGBA : GL_RGB, rnd() % 40, rnd() % 40, 1 + rnd() % 40, 1 + rnd() % 40, 0); E("copyteximage"); break;
		case 4: glCopyTexSubImage2D(GL_TEXTURE_2D, 0, rnd() % 4, rnd() % 4, rnd() % 50, rnd() % 50, 1 + rnd() % 20, 1 + rnd() % 20); E("copytexsub"); break;
		default: break;
		}
		if (rnd() % 10 == 0) { glDeleteTextures(1, &t[it % 8]); glGenTextures(1, &t[it % 8]); E("regen"); }
	}
	glFinish(); hh_memdump("end-tex");
}

static void sc_buf(int iters)
{
	GLuint p = prog(".", "passthrough.vert", ".", "uniform_color.frag"), b[6], va[3];
	static uint8_t data[1 << 16];
	GLint lc;

	glUseProgram(p); lc = glGetUniformLocation(p, "u_color"); mk_buffers(); glGenBuffers(6, b); glGenVertexArraysOES(3, va); E("gen");
	for (int v = 0; v < 4; v++) { glBindVertexArrayOES(v ? va[v - 1] : 0); glBindBuffer(GL_ARRAY_BUFFER, vbo); for (int l = 0; l < 3; l++) glVertexAttribPointer(l, 4, GL_FLOAT, 0, 64, 0); }
	glBindVertexArrayOES(0);
	for (int it = 0; it < iters; it++) {
		GLuint bo = b[rnd() % 6];
		GLenum tg = (rnd() % 3 == 0) ? GL_ELEMENT_ARRAY_BUFFER : GL_ARRAY_BUFFER;
		int sz = 16 + rnd() % 5000;

		for (int k = 0; k < sz + 64; k += 4) { float f = rndf(); memcpy(data + k, &f, 4); }
		if (tg == GL_ELEMENT_ARRAY_BUFFER) for (int k = 0; k < sz; k += 2) { uint16_t ix = rnd() % 8; memcpy(data + k, &ix, 2); }
		glBindVertexArrayOES(va[rnd() % 3] * (rnd() % 4 != 0)); E("bindvao");
		glBindBuffer(tg, bo);
		hh_step("buf it=%d tg=%x sz=%d", it, tg, sz);
		switch (rnd() % 8) {
		case 0: glBufferData(tg, sz, (rnd() % 5 == 0) ? NULL : data, GL_STATIC_DRAW + rnd() % 3 * 4); E("bufferdata"); break;
		case 1: glBufferSubData(tg, rnd() % 256, rnd() % 512, data); E("subdata"); break;
		case 2: {
			static const GLbitfield acc[] = { GL_MAP_WRITE_BIT, GL_MAP_WRITE_BIT | GL_MAP_INVALIDATE_BUFFER_BIT, GL_MAP_WRITE_BIT | GL_MAP_INVALIDATE_RANGE_BIT,
							  GL_MAP_READ_BIT, GL_MAP_READ_BIT | GL_MAP_WRITE_BIT, GL_MAP_WRITE_BIT | GL_MAP_FLUSH_EXPLICIT_BIT,
							  GL_MAP_WRITE_BIT | GL_MAP_UNSYNCHRONIZED_BIT, 0, GL_MAP_READ_BIT | GL_MAP_INVALIDATE_BUFFER_BIT, 0x100 };
			GLbitfield a = acc[rnd() % 10];
			int off = rnd() % 300, len = rnd() % 600;
			void *m = glMapBufferRange(tg, off, len, a);
			E("map");
			if (m) {
				GLint mapped = -1;

				glGetBufferParameteriv(tg, GL_BUFFER_MAPPED, &mapped);
				if (a & GL_MAP_WRITE_BIT) memcpy(m, data + 100, len);
				if (a & GL_MAP_FLUSH_EXPLICIT_BIT) { glFlushMappedBufferRange(tg, 0, len / 2); E("flushrange"); glFlushMappedBufferRange(tg, len, 8); E("flushrange-oob"); }
				if (rnd() % 4 == 0) { glDrawArrays(GL_TRIANGLES, 0, 3); E("draw-while-mapped"); glBufferSubData(tg, 0, 4, data); E("subdata-while-mapped"); glMapBufferRange(tg, 0, 4, GL_MAP_WRITE_BIT); E("remap"); }
				glUnmapBuffer(tg); E("unmap");
			}
			glUnmapBuffer(tg); E("unmap2");
			break;
		}
		case 3: {
			static const GLenum ty[] = { GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_INT, GL_UNSIGNED_INT, GL_FLOAT, GL_HALF_FLOAT, GL_FIXED, 0x1234 };
			int loc = rnd() % 3;
			GLenum t;

			glBindBuffer(GL_ARRAY_BUFFER, b[0]);
			t = ty[rnd() % 10];
			if (rnd() % 2) glVertexAttribIPointer(loc, 1 + rnd() % 4, t, rnd() % 40, (void *)(uintptr_t)(rnd() % 64));
			else glVertexAttribPointer(loc, 1 + rnd() % 5, t, rnd() % 2, (GLint)(rnd() % 40) - 2, (void *)(uintptr_t)(rnd() % 64));
			E("pointer");
			if (rnd() % 2) glEnableVertexAttribArray(loc); else glDisableVertexAttribArray(loc);
			glVertexAttribDivisor(loc, rnd() % 3); E("divisor");
			glVertexAttrib1f(loc, ef()); glVertexAttrib2f(loc, ef(), 1); glVertexAttrib3f(loc, 1, ef(), 2);
			glVertexAttrib1fv(loc, (GLfloat[1]){ 2 }); glVertexAttrib2fv(loc, (GLfloat[2]){ 2, 3 }); glVertexAttrib3fv(loc, (GLfloat[3]){ 2, 3, 4 });
			glVertexAttrib4fv(loc, (GLfloat[4]){ 2, 3, 4, 5 }); E("const attribs");
			break;
		}
		case 4: glDeleteBuffers(1, &bo); E("delete"); for (int k = 0; k < 6; k++) if (b[k] == bo) glGenBuffers(1, &b[k]); break;
		case 5: {
			GLuint v = va[rnd() % 3];
			glDeleteVertexArraysOES(1, &v);
			for (int k = 0; k < 3; k++)
				if (va[k] == v) { glGenVertexArraysOES(1, &va[k]); glBindVertexArrayOES(va[k]); glBindBuffer(GL_ARRAY_BUFFER, vbo); for (int l = 0; l < 3; l++) glVertexAttribPointer(l, 4, GL_FLOAT, 0, 64, 0); }
			glIsVertexArrayOES(v); E("delvao");
			break;
		}
		case 6: {
			static float cl[64];
			glBindBuffer(GL_ARRAY_BUFFER, 0);
			for (int k = 0; k < 64; k++) cl[k] = rndf();
			glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, cl); glEnableVertexAttribArray(0); glDrawArrays(GL_TRIANGLES, 0, 6); E("client draw");
			glBindBuffer(GL_ARRAY_BUFFER, vbo); glVertexAttribPointer(0, 4, GL_FLOAT, 0, 64, 0);
			break;
		}
		case 7:
			/* buffer targets the library does not keep */
			glBindBuffer(GL_COPY_READ_BUFFER, vbo); glBindBuffer(GL_UNIFORM_BUFFER, b[1]);
			glBufferData(GL_UNIFORM_BUFFER, 256, NULL, GL_DYNAMIC_DRAW);
			E("es3");
			break;
		}
		glUniform4f(lc, rndf(), rndf(), rndf(), 1.f);
		draw_some(it);
		if (rnd() % 5 == 0) glFlush();
		if (rnd() % 17 == 0) glFinish();
		hh_memdump("it");
	}
	glFinish();
}

static void sc_fbo(int iters)
{
	GLuint p = prog(".", "uv.vert", ".", "texture.frag"), fb[4], rb[4], tx[4];
	static uint32_t px[128 * 128];
	static const GLenum rf[] = { GL_RGBA8, GL_RGB565, GL_RGBA4, GL_RGB5_A1, GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT24, GL_DEPTH24_STENCIL8, GL_STENCIL_INDEX8, GL_RGB8 };

	mk_buffers(); glUseProgram(p); set_attribs(p, 0);
	glGenFramebuffers(4, fb); glGenRenderbuffers(4, rb); glGenTextures(4, tx);
	for (int it = 0; it < iters; it++) {
		int i = rnd() % 4, w = 1 + rnd() % 100, h = 1 + rnd() % 100;

		hh_step("fbo it=%d i=%d %dx%d", it, i, w, h);
		glBindFramebuffer(GL_FRAMEBUFFER, fb[i]);
		switch (rnd() % 8) {
		case 0:
			glBindTexture(GL_TEXTURE_2D, tx[i]);
			glTexImage2D(GL_TEXTURE_2D, 0, (rnd() % 3) ? GL_RGBA : GL_RGB, w, h, 0, (rnd() % 3) ? GL_RGBA : GL_RGB, GL_UNSIGNED_BYTE, NULL); E("teximg");
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tx[i], 0); E("fbtex");
			break;
		case 1: {
			static const GLenum at[] = { GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT, GL_COLOR_ATTACHMENT1 };
			GLenum f;

			glBindRenderbuffer(GL_RENDERBUFFER, rb[i]);
			f = rf[rnd() % 9];
			if (rnd() % 3 == 0) glRenderbufferStorageMultisample(GL_RENDERBUFFER, 4, f, w, h); else glRenderbufferStorage(GL_RENDERBUFFER, f, w, h);
			E("rbstorage");
			glFramebufferRenderbuffer(GL_FRAMEBUFFER, at[rnd() % 5], GL_RENDERBUFFER, rb[i] * (rnd() % 5 != 0)); E("fbrb");
			break;
		}
		case 2:
			glBindTexture(GL_TEXTURE_2D, tx[i]); glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 32, 32, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
			glGenerateMipmap(GL_TEXTURE_2D); glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tx[i], rnd() % 3); E("fbtex-level");
			break;
		case 3: {
			GLenum at[] = { GL_COLOR_ATTACHMENT0, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT };
			glInvalidateFramebuffer(GL_FRAMEBUFFER, 1 + rnd() % 3, at); E("invalidate");
			glInvalidateSubFramebuffer(GL_FRAMEBUFFER, 2, at, 0, 0, 8, 8); E("invalidatesub");
			break;
		}
		case 4:
			glBindFramebuffer(GL_READ_FRAMEBUFFER, fb[(i + 1) % 4]); glBindFramebuffer(GL_DRAW_FRAMEBUFFER, (rnd() % 2) ? fb[i] : 0);
			glBlitFramebuffer(0, 0, w, h, rnd() % 8, rnd() % 8, w, h, GL_COLOR_BUFFER_BIT | ((rnd() % 3 == 0) ? GL_DEPTH_BUFFER_BIT : 0), (rnd() % 2) ? GL_NEAREST : GL_LINEAR);
			E("blit"); glBindFramebuffer(GL_FRAMEBUFFER, fb[i]);
			break;
		case 5: {
			GLenum db[2] = { (rnd() % 2) ? GL_COLOR_ATTACHMENT0 : GL_NONE, GL_NONE };
			glDrawBuffers(1 + rnd() % 2, db); E("drawbuffers");
			glReadBuffer((rnd() % 2) ? GL_COLOR_ATTACHMENT0 : GL_BACK); E("readbuffer");
			break;
		}
		case 6: glDeleteTextures(1, &tx[i]); glGenTextures(1, &tx[i]); E("deltex-attached"); break;
		case 7:
			glDeleteRenderbuffers(1, &rb[i]); glGenRenderbuffers(1, &rb[i]);
			if (rnd() % 3 == 0) { glDeleteFramebuffers(1, &fb[i]); glGenFramebuffers(1, &fb[i]); }
			E("delrb");
			break;
		}
		glCheckFramebufferStatus(GL_FRAMEBUFFER);
		glViewport(0, 0, w, h); glClearColor(rndf(), rndf(), rndf(), 1); glClear(((rnd() % 8) << 8) & 0x4500); E("clear");
		glBindTexture(GL_TEXTURE_2D, tx[(i + 1) % 4]); glDrawArrays(GL_TRIANGLES, 0, 3); E("draw");
		if (rnd() % 3 == 0) {
			static const GLenum rfm[][2] = { { GL_RGBA, GL_UNSIGNED_BYTE }, { GL_RGB, GL_UNSIGNED_SHORT_5_6_5 }, { GL_RGB, GL_UNSIGNED_BYTE },
							 { 0x80E1, GL_UNSIGNED_BYTE }, { GL_RGBA, GL_FLOAT }, { GL_ALPHA, GL_UNSIGNED_BYTE } };
			int q = rnd() % 6;
			memset(px, 0x5a, sizeof px);
			glPixelStorei(GL_PACK_ALIGNMENT, 1 << (rnd() % 4));
			glReadPixels((GLint)(rnd() % 20) - 4, (GLint)(rnd() % 20) - 4, rnd() % 40, rnd() % 40, rfm[q][0], rfm[q][1], px); E("readpixels");
		}
		if (rnd() % 4 == 0) {
			glBindFramebuffer(GL_FRAMEBUFFER, 0); glBindTexture(GL_TEXTURE_2D, tx[i]); glDrawArrays(GL_TRIANGLES, 0, 3); E("draw-from-rt");
			if (rnd() % 2) eglSwapBuffers(dpy, pb);
		}
		if (rnd() % 9 == 0) glFinish();
	}
	glFinish(); hh_memdump("end-fbo");
}

static void sc_stress(int ndraws, int bigverts)
{
	GLuint p = prog(".", "passthrough.vert", ".", "uniform_color.frag"), p2 = prog(".", "uv.vert", ".", "texture.frag");
	GLint lc = glGetUniformLocation(p, "u_color");
	static float big[1 << 22];
	GLuint bb;

	mk_buffers(); mk_textures();
	for (int k = 0; k < (1 << 22); k++) big[k] = rndf();
	glGenBuffers(1, &bb);
	for (int i = 0; i < ndraws; i++) {
		if (i % 3 == 0) { glUseProgram(p); set_attribs(p, 0); glUniform4f(lc, rndf(), rndf(), rndf(), 1); }
		else if (i % 3 == 1) { glUseProgram(p2); set_attribs(p2, 0); glActiveTexture(GL_TEXTURE0); glBindTexture(GL_TEXTURE_2D, tex2d[i % 8]); }
		if (bigverts && i % 50 == 7) {
			glBindBuffer(GL_ARRAY_BUFFER, 0);
			glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, big); glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, big + 16);
			glDrawArrays(GL_TRIANGLES, 0, bigverts); E("bigdraw"); set_attribs(p, 0);
		} else if (i % 50 == 19) {
			glBindBuffer(GL_ARRAY_BUFFER, bb); glBufferData(GL_ARRAY_BUFFER, 4096 + i, big, GL_DYNAMIC_DRAW);
			glVertexAttribPointer(0, 4, GL_FLOAT, 0, 32, 0); glDrawArrays(GL_TRIANGLES, 0, 30); E("dyn");
			glBufferSubData(GL_ARRAY_BUFFER, 16, 64, big + 99); glDrawArrays(GL_TRIANGLES, 0, 30);
		} else if (i % 50 == 33) {
			GLuint t;
			glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, big);
			glUseProgram(p2); set_attribs(p2, 0); glDrawArrays(GL_TRIANGLES, 0, 6); glDeleteTextures(1, &t); E("tex-inflight-delete");
		} else {
			draw_some(i);
		}
		if (i % 97 == 96) {
			glScissor(i % 30, i % 20, 20, 20); glEnable(GL_SCISSOR_TEST); glClearColor(rndf(), 0, 0, 1);
			glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT); glDisable(GL_SCISSOR_TEST);
		}
		if (i % 1500 == 1499) eglSwapBuffers(dpy, pb);
		if (i % 4000 == 3999) { uint32_t px; glReadPixels(1, 1, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, &px); }
	}
	E("stress"); glFinish(); hh_memdump("end-stress");
}

static void sc_ctx2(void)
{
	EGLint pa[] = { EGL_WIDTH, 32, EGL_HEIGHT, 48, EGL_NONE }, xa[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLSurface s2 = eglCreatePbufferSurface(dpy, cfg, pa);
	EGLContext c2 = eglCreateContext(dpy, cfg, ctx, xa), c3 = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, xa);
	GLuint p = prog(".", "passthrough.vert", ".", "uniform_color.frag");

	eglGetError();
	mk_buffers(); mk_textures(); glUseProgram(p); set_attribs(p, 0); glUniform4f(glGetUniformLocation(p, "u_color"), .1f, .2f, .3f, 1);
	for (int r = 0; r < 12; r++) {
		EGLContext c = (r % 3 == 0) ? ctx : (r % 3 == 1) ? c2 : c3;
		EGLSurface s = (r % 2) ? s2 : pb;
		GLuint q;

		hh_step("round %d", r);
		eglMakeCurrent(dpy, s, s, c); eglGetError();
		glIsProgram(p); glIsBuffer(vbo); glIsTexture(tex2d[0]); glGetString(GL_VERSION);
		glUseProgram(p); E("use shared prog");
		q = prog(".", "uv.vert", ".", "texture.frag");
		glClearColor(.1f * r, .5f, .5f, 1); glClear(GL_COLOR_BUFFER_BIT);
		set_attribs(p, r); draw_some(r); glUseProgram(q); set_attribs(q, r); draw_some(r + 1); E("draws");
		{ uint8_t px[4 * 4 * 4]; glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, px); }
		if (r % 4 == 1) glFlush();
		if (r % 4 == 2) eglSwapBuffers(dpy, s);
		if (r == 7) eglDestroyContext(dpy, c3);
		if (r % 5 == 4) glDeleteProgram(q);
	}
	eglMakeCurrent(dpy, 0, 0, 0); glClear(GL_COLOR_BUFFER_BIT); glGetError();
	eglMakeCurrent(dpy, pb, s2, ctx); eglGetError(); glClear(GL_COLOR_BUFFER_BIT); glFinish();
	eglTerminate(dpy); hh_memdump("end-ctx2");
}

static void sc_inst0(void)
{
	GLuint p = prog(".", "passthrough.vert", ".", "uniform_color.frag");

	mk_buffers(); glUseProgram(p);
	glBindBuffer(GL_ARRAY_BUFFER, vbo); glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, 0); glEnableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glEnableVertexAttribArray(1); glVertexAttribDivisor(1, 1);   /* never given a pointer */
	glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2); E("inst0"); glFinish();
	hh_step("survived");
}

static void sc_win(void)
{
	static uint8_t fbmem[3][256 * 256 * 4];
	static uint8_t imgmem[64 * 64 * 4];
	EGLConfig cs[64];
	EGLint n = 0;

	hh_region_add(fbmem[0], sizeof fbmem[0], "window0");
	hh_region_add(fbmem[1], sizeof fbmem[1], "window1");
	hh_region_add(fbmem[2], sizeof fbmem[2], "window2");
	hh_region_add(imgmem, sizeof imgmem, "pixmap");
	backend_up();
	dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY); eglInitialize(dpy, NULL, NULL); eglGetConfigs(dpy, cs, 64, &n);
	for (int ci = 0; ci < n; ci++)
		for (unsigned fmt = 0; fmt <= 4; fmt++) {
			struct gles_native_window nw = { (uint64_t)(uintptr_t)fbmem[fmt % 3], 100 + 4 * ci, 60, (100 + 4 * ci) * (fmt == 3 ? 2 : 4), fmt };
			EGLint xa[] = { EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE };
			EGLSurface s;
			EGLContext c;
			GLuint p, t, rb, fb;

			if (fmt == 4) { nw.format = 1; nw.stride_bytes += 8; }
			hh_step("window cfg%d fmt%u", ci, fmt);
			s = eglCreateWindowSurface(dpy, cs[ci], &nw, NULL); eglGetError();
			if (!s) continue;
			c = eglCreateContext(dpy, cs[ci], EGL_NO_CONTEXT, xa); eglMakeCurrent(dpy, s, s, c); eglGetError();
			p = prog(".", "uv.vert", ".", "texture.frag"); mk_buffers(); glUseProgram(p); set_attribs(p, 0);
			struct gles_native_window pm = { (uint64_t)(uintptr_t)imgmem, 64, 64, 256, (fmt % 3) + 1 };
			if (pm.format == 3) pm.stride_bytes = 128;
			EGLImage im = eglCreateImage(dpy, EGL_NO_CONTEXT, EGL_NATIVE_PIXMAP_KHR, (EGLClientBuffer)&pm, NULL); eglGetError();
			glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t); glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, im); E("imgtex");
			for (int k = 0; k < 3; k++) {
				uint32_t px[64];
				glClearColor(.2f * k, .4f, .6f, 1); glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
				draw_some(k); draw_some(k + 3);
				memset(px, 0, sizeof px); glReadPixels(2, 2, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px); E("rp");
				eglSwapBuffers(dpy, s);
			}
			glGenRenderbuffers(1, &rb); glBindRenderbuffer(GL_RENDERBUFFER, rb); glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, im); E("imgrb");
			glGenFramebuffers(1, &fb); glBindFramebuffer(GL_FRAMEBUFFER, fb); glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
			glCheckFramebufferStatus(GL_FRAMEBUFFER); glViewport(0, 0, 64, 64); glClear(GL_COLOR_BUFFER_BIT); draw_some(1); glFinish();
			{ uint32_t px[64]; glReadPixels(1, 1, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px); }
			glBindFramebuffer(GL_FRAMEBUFFER, 0);
			/* the image is gone; the texture and the renderbuffer are deleted
			 * before it (destroying the image under a live sibling is the
			 * known use-after-free, exercised by the hostile program) */
			glDeleteFramebuffers(1, &fb); glDeleteRenderbuffers(1, &rb); glDeleteTextures(1, &t);
			eglDestroyImage(dpy, im); draw_some(2); glFinish(); E("after destroy image");
			eglMakeCurrent(dpy, 0, 0, 0); eglDestroySurface(dpy, s); eglDestroyContext(dpy, c); hh_memdump("win-end");
		}
	eglTerminate(dpy);
}

int main(int argc, char **argv)
{
	const char *sc = argc > 1 ? argv[1] : "state";

	hh_install();
	SH = getenv("HH_SHADERS");
	if (!SH) { printf("HH-ERROR HH_SHADERS is not set\n"); return 2; }
	inrange = getenv("HH_INRANGE") && atoi(getenv("HH_INRANGE"));
	pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, getenv("HH_DEBUG") ? atoi(getenv("HH_DEBUG")) : 0);
	if (getenv("HH_PRETRANSFORM")) pvr_backend_set_option(PVR_BACKEND_OPT_PRETRANSFORM, 1);
	if (getenv("HH_NOCOALESCE")) pvr_backend_set_option(PVR_BACKEND_OPT_NO_COALESCE, 1);
	if (argc > 3 && strcmp(sc, "progs") && strcmp(sc, "state") && strcmp(sc, "stress")) rs = (uint32_t)atoi(argv[3]);
	if (!strcmp(sc, "egl")) { sc_egl(); goto done; }
	if (!strcmp(sc, "win")) { sc_win(); goto done; }
	gl_up(getenv("HH_SAMPLES") ? atoi(getenv("HH_SAMPLES")) : 0, 3);
	if (!strcmp(sc, "progs")) sc_progs(argv[2], argc > 3 ? atoi(argv[3]) : 0, argc > 4 ? atoi(argv[4]) : 0);
	else if (!strcmp(sc, "state")) sc_state(argc > 2 ? atoi(argv[2]) : 400, argc > 3 ? atoi(argv[3]) : 0);
	else if (!strcmp(sc, "gets")) sc_gets();
	else if (!strcmp(sc, "tex")) sc_tex(atoi(argv[2]));
	else if (!strcmp(sc, "buf")) sc_buf(atoi(argv[2]));
	else if (!strcmp(sc, "fbo")) sc_fbo(atoi(argv[2]));
	else if (!strcmp(sc, "stress")) sc_stress(atoi(argv[2]), argc > 3 ? atoi(argv[3]) : 0);
	else if (!strcmp(sc, "ctx2")) sc_ctx2();
	else if (!strcmp(sc, "inst0")) sc_inst0();
	else { printf("HH-ERROR unknown scenario %s\n", sc); return 2; }
done:
	printf("scen %s done errors=%u\n", sc, hh_errors());
	return 0;
}
