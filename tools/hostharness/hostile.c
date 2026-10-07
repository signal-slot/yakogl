/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hostile.c - harness program: hostile call sequences (from the rev-panics
 * and rev-soundness reviews) that hit known wrap-around, ownership and
 * pointer-alignment problems. One sequence per process (a panic or a
 * sanitizer report ends it).
 *
 * usage: hostile <name>        hostile list   (prints the names)
 */
#define GL_GLEXT_PROTOTYPES 1
#define EGL_EGLEXT_PROTOTYPES 1
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include <stdint.h>
#include <GLES3/gl3.h>
#include <GLES2/gl2ext.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include "gles_osal.h"
#include "osal_internal.h"
#include "gl_internal.h"
#include "backend.h"
#if !defined(YAKOGL_GPU_V3D)
#include "pvr/pvr_backend.h"
#endif
#include "hh.h"

static const char *SH;
static struct gl_context *hctx;

static char *slurp(const char *name)
{
	char p[1024];
	FILE *f;
	long n;
	char *b;

	snprintf(p, sizeof p, "%s/%s", SH, name);
	f = fopen(p, "rb");
	if (!f) { printf("HH-ERROR cannot open %s\n", p); exit(2); }
	fseek(f, 0, SEEK_END); n = ftell(f); rewind(f);
	b = malloc(n + 1);
	if (fread(b, 1, n, f) != (size_t)n) exit(2);
	b[n] = 0; fclose(f);
	return b;
}

static GLuint program(const char *v, const char *f)
{
	char *vs = slurp(v), *fs = slurp(f);
	const char *a = vs, *b = fs;
	GLuint s1 = glCreateShader(GL_VERTEX_SHADER), s2 = glCreateShader(GL_FRAGMENT_SHADER), p = glCreateProgram();

	glShaderSource(s1, 1, &a, NULL); glCompileShader(s1);
	glShaderSource(s2, 1, &b, NULL); glCompileShader(s2);
	glAttachShader(p, s1); glAttachShader(p, s2); glLinkProgram(p);
	free(vs); free(fs);
	return p;
}

/* the unit tests' TestGl: platform + backend + one 16x16 context */
static void up(void)
{
	struct osal_zephyr_init oi = { 0 };

#if defined(YAKOGL_GPU_V3D)
	/* the V3D build's backend, through the table (src/backend.h) */
	if (g_osal->init(&oi) || yakogl_backend_init()) { printf("HH-ERROR bring-up failed\n"); exit(3); }
#else
	if (g_osal->init(&oi) || pvr_backend_init()) { printf("HH-ERROR bring-up failed\n"); exit(3); }
#endif
	hctx = gl_context_create(16, 16);
	if (!hctx) { printf("HH-ERROR gl_context_create failed\n"); exit(3); }
}

static GLuint prog(void)
{
	GLuint p = program("passthrough.vert", "uniform_color.frag");

	glUseProgram(p);
	return p;
}

static void E(void) { while (glGetError()) ; }

/* ---- range checks done in wrapping arithmetic ---- */

static void vbo_negative_offset(void)
{
	GLuint b;
	uint8_t data[64];

	up(); prog();
	memset(data, 0x11, sizeof data);
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 64, data, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 2, GL_SHORT, 0, 8, (void *)(intptr_t)-8);
	glEnableVertexAttribArray(0);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
}

static void elements_setup(GLuint *b)
{
	static float v[64];

	up(); prog();
	glGenBuffers(2, b);
	glBindBuffer(GL_ARRAY_BUFFER, b[0]);
	glBufferData(GL_ARRAY_BUFFER, 256, v, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, NULL); glEnableVertexAttribArray(0);
	glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, NULL); glEnableVertexAttribArray(1);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, b[1]);
}

static void elements_negative_u16(void)
{
	GLuint b[2];
	uint16_t ix[] = { 0, 1, 2, 0, 1, 2 };

	elements_setup(b);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, 12, ix, GL_STATIC_DRAW);
	glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, (void *)(intptr_t)-2);
	glFinish();
}

static void elements_negative_u8(void)
{
	GLuint b[2];
	uint8_t ix[16] = { 0 };

	elements_setup(b);
	glBufferData(GL_ELEMENT_ARRAY_BUFFER, 16, ix, GL_STATIC_DRAW);
	glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, (void *)(intptr_t)-2);
	glFinish();
}

static void map_buffer_range_wrap(void)
{
	GLuint b;
	void *p;

	up();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 64, NULL, GL_STATIC_DRAW);
	p = glMapBufferRange(GL_ARRAY_BUFFER, PTRDIFF_MAX, 2, GL_MAP_WRITE_BIT);
	hh_step("mapped %s", p ? "NON-NULL (wrapped range accepted)" : "NULL");
	if (p) glUnmapBuffer(GL_ARRAY_BUFFER);
}

static void flush_mapped_wrap(void)
{
	GLuint b;

	up();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 64, NULL, GL_STATIC_DRAW);
	glMapBufferRange(GL_ARRAY_BUFFER, 0, 64, GL_MAP_WRITE_BIT | GL_MAP_FLUSH_EXPLICIT_BIT);
	glFlushMappedBufferRange(GL_ARRAY_BUFFER, PTRDIFF_MAX, 2);
	glUnmapBuffer(GL_ARRAY_BUFFER);
}

static void buffer_subdata_wrap(void)
{
	GLuint b;
	uint8_t d[8] = { 1, 2, 3, 4, 5, 6, 7, 8 };

	up();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 64, NULL, GL_STATIC_DRAW);
	glBufferSubData(GL_ARRAY_BUFFER, PTRDIFF_MAX - 2, 8, d);
	glBufferSubData(GL_ARRAY_BUFFER, 60, PTRDIFF_MAX, d);
	glFinish();
}

/* ---- application pointers that are not aligned for their type ---- */

static void *unaligned(void)
{
	static uint8_t buf[64];

	return ((uintptr_t)buf % 4 == 0) ? buf + 1 : buf;
}

static void unaligned_uniform4fv(void)
{
	GLuint p;

	up(); p = prog();
	glUniform4fv(glGetUniformLocation(p, "u_color"), 1, unaligned());
	E();
}

static void unaligned_get_integerv(void) { up(); glGetIntegerv(GL_VIEWPORT, unaligned()); }
static void unaligned_gen_textures(void) { up(); glGenTextures(2, unaligned()); }
static void unaligned_vertex_attrib4fv(void) { up(); glVertexAttrib4fv(0, unaligned()); }
static void unaligned_get_uniformfv(void)
{
	GLuint p;

	up(); p = prog();
	glGetUniformfv(p, glGetUniformLocation(p, "u_color"), unaligned());
}

/* ---- attachments of the wrong storage ---- */

static void compressed_texture_as_fbo_color(void)
{
	GLuint t, fb;
	static uint8_t blocks[16 * 16 * 8];
	static uint8_t px[64 * 64 * 4];

	up(); prog();
	glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
	glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 64, 64, 0, sizeof blocks, blocks);
	glGenFramebuffers(1, &fb); glBindFramebuffer(GL_FRAMEBUFFER, fb);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE) {
		hh_step("ETC2 texture accepted as colour attachment: read back 64x64");
		glReadPixels(0, 0, 64, 64, GL_RGBA, GL_UNSIGNED_BYTE, px);
	}
}

static void rgb565_external_readpixels(void)
{
	/* exactly the 4x4 RGB565 image: an over-read is a sanitizer report */
	uint8_t *backing = calloc(1, 32);
	struct osal_gpu_mem mem = { backing, 0x8010000000ull, 0, 32, (void *)1 };
	GLuint t, fb;
	uint8_t px[64];

	hh_region_add(backing, 32, "ext565");
	up();
	glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
	gl_texture_attach_external(&mem, 4, 4, GLES_SURF_RGB565, NULL);
	glGenFramebuffers(1, &fb); glBindFramebuffer(GL_FRAMEBUFFER, fb);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
	glCheckFramebufferStatus(GL_FRAMEBUFFER);
	glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteTextures(1, &t);
}

static void tex_sub_image_odd_width_external(void)
{
	/* exactly the image: an over-write is a sanitizer report */
	uint8_t *backing = malloc(3 * 2 * 4);
	struct osal_gpu_mem mem = { backing, 0x8010000000ull, 0, 3 * 2 * 4, (void *)1 };
	uint8_t px[3 * 2 * 4];
	GLuint t;

	hh_region_add(backing, 24, "ext3x2");
	memset(px, 0x11, sizeof px);
	up();
	glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
	gl_texture_attach_external(&mem, 3, 2, GLES_SURF_RGBA8888, NULL);
	glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 3, 2, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glDeleteTextures(1, &t);
}

static void odd_width_pbuffer_readpixels(void)
{
	EGLDisplay d;
	EGLConfig c;
	EGLint n, pa[] = { EGL_WIDTH, 61, EGL_HEIGHT, 35, EGL_NONE }, xa[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLSurface s;
	EGLContext x;
	uint8_t *px = malloc(61 * 35 * 4);

	up();
	d = eglGetDisplay(EGL_DEFAULT_DISPLAY); eglInitialize(d, NULL, NULL);
	eglChooseConfig(d, (EGLint[]){ EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_NONE }, &c, 1, &n);
	s = eglCreatePbufferSurface(d, c, pa); x = eglCreateContext(d, c, EGL_NO_CONTEXT, xa);
	eglMakeCurrent(d, s, s, x);
	glClearColor(1, 0, 0, 1); glClear(GL_COLOR_BUFFER_BIT);
	glReadPixels(0, 0, 61, 35, GL_RGBA, GL_UNSIGNED_BYTE, px);
	eglTerminate(d);
}

/* ---- sizes that overflow ---- */

static void context_create_overflow(void)
{
	struct gl_context *c;

	up();
	c = gl_context_create(0x80000000u, 0x80000000u);
	hh_step("context %s", c ? "created" : "refused");
	if (c) gl_context_destroy(c);
}

static void uniform_location_fuzz(void)
{
	static const char *pairs[2][2] = { { "mvp.vert", "uniform_color.frag" }, { "passthrough.vert", "uniform_color.frag" } };
	static float vals[4096], out[4096];
	static int ivals[4096];
	static const int extra[] = { INT_MIN, INT_MAX, 0x7fffff00, 0xff00, 0xffff, 0x10000, -256, -255 };
	static const int cnts[] = { 1, 2, 255, 256, 1024, INT_MAX, -1, 0, INT_MIN };

	up();
	for (int k = 0; k < 4096; k++) { vals[k] = 1.0f; ivals[k] = 7; }
	for (int pi = 0; pi < 2; pi++) {
		GLuint p = program(pairs[pi][0], pairs[pi][1]);

		glUseProgram(p);
		for (int li = -3; li < 600 + 8; li++) {
			int l = li < 600 ? li : extra[li - 600];

			for (unsigned ci = 0; ci < sizeof cnts / sizeof cnts[0]; ci++) {
				int cnt = cnts[ci], mc = cnt < 200 ? cnt : 200;

				if (cnt > 1000) continue;
				glUniform1fv(l, cnt, vals); glUniform2fv(l, cnt, vals); glUniform3fv(l, cnt, vals); glUniform4fv(l, cnt, vals);
				glUniform1iv(l, cnt, ivals); glUniform4iv(l, cnt, ivals);
				glUniformMatrix2fv(l, mc, 1, vals); glUniformMatrix3fv(l, mc, 1, vals); glUniformMatrix4fv(l, mc, 1, vals);
				glUniformMatrix3fv(l, mc, 0, vals); glUniformMatrix4fv(l, mc, 0, vals);
			}
			glUniform1i(l, INT_MAX); glUniform1i(l, -1); glUniform1i(l, 9);
			glUniform4f(l, 1, 2, 3, 4);
			glGetUniformfv(p, l, out);
			E();
		}
	}
}

static void get_uniform_array_into_small_buffer(void)
{
	GLuint p;
	GLint n = 0;

	up();
	p = program("mvp.vert", "uniform_color.frag");
	glUseProgram(p);
	glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &n);
	for (GLint i = 0; i < n; i++) {
		char name[128];
		GLint size = 0;
		GLenum type = 0;
		float *one;

		glGetActiveUniform(p, i, sizeof name, NULL, &size, &type, name);
		/* a buffer of exactly one element of the uniform's type */
		one = malloc(type == GL_FLOAT_MAT4 ? 64 : type == GL_FLOAT_MAT3 ? 36 : 16);
		glGetUniformfv(p, glGetUniformLocation(p, name), one);
		free(one);
	}
}

static void shader_source_and_logs(void)
{
	const char *srcs[] = { "void main(){}", NULL, "" };
	int lens[] = { INT_MIN, 5, 0 };
	char buf[8];
	GLsizei n = 0;
	GLint sz = 0;
	GLenum ty = 0;
	GLuint s, p;
	char *lng = malloc(5001);

	up();
	s = glCreateShader(GL_VERTEX_SHADER);
	glShaderSource(s, 3, srcs, lens);
	glShaderSource(s, 0, srcs, lens);
	glShaderSource(s, INT_MIN, srcs, lens);
	glCompileShader(s);
	glGetShaderInfoLog(s, INT_MAX, &n, NULL);
	glGetShaderInfoLog(s, 1, &n, buf);
	glGetShaderInfoLog(s, 0, &n, buf);
	glGetShaderInfoLog(s, INT_MIN, &n, buf);
	glGetShaderSource(s, 1, &n, buf);
	p = glCreateProgram();
	glAttachShader(p, s);
	glLinkProgram(p);
	glGetProgramInfoLog(p, 1, &n, buf);
	glGetProgramInfoLog(p, 8, &n, buf);
	glGetActiveUniform(p, UINT_MAX, 8, &n, &sz, &ty, buf);
	glGetActiveAttrib(p, UINT_MAX, 8, &n, &sz, &ty, buf);
	glGetActiveUniform(p, 0, 0, &n, &sz, &ty, buf);
	memset(lng, 0xC3, 5000);
	lng[5000] = 0;
	glShaderSource(s, 1, (const char *const *)&lng, NULL);
	glCompileShader(s);
	E();
}

static void texture_size_extremes(void)
{
	static const int wh[][2] = { { INT_MAX, INT_MAX }, { INT_MAX, 1 }, { 1, INT_MAX }, { 65536, 65536 }, { 0x40000000, 4 }, { INT_MIN, 1 } };
	static const int lvls[] = { INT_MIN, -1, 12, 13, INT_MAX };
	static const int xy[][2] = { { INT_MIN, INT_MIN }, { INT_MAX, INT_MAX }, { -8, -8 }, { INT_MIN + 1, 0 }, { INT_MAX - 4, 0 } };
	static uint8_t px[8 * 8 * 16], out[16 * 16 * 4];
	GLuint t, rb;

	up();
	glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
	for (unsigned i = 0; i < sizeof wh / sizeof wh[0]; i++) {
		hh_fail_alloc_in(1);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, wh[i][0], wh[i][1], 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		hh_fail_alloc_in(1);
		glTexStorage2D(GL_TEXTURE_2D, INT_MAX, GL_RGBA8, wh[i][0], wh[i][1]);
		E();
	}
	hh_fail_alloc_in(0);
	for (unsigned i = 0; i < sizeof lvls / sizeof lvls[0]; i++) {
		glTexImage2D(GL_TEXTURE_2D, lvls[i], GL_RGBA, 1, 1, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glTexSubImage2D(GL_TEXTURE_2D, lvls[i], 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glCopyTexSubImage2D(GL_TEXTURE_2D, lvls[i], 0, 0, 0, 0, 1, 1);
	}
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glTexSubImage2D(GL_TEXTURE_2D, 0, INT_MAX, INT_MAX, INT_MAX, INT_MAX, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 7, 7, 1, 1, GL_RGBA, GL_FLOAT, px);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, 0x1234, 0x5678, px);
	glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, INT_MIN, INT_MIN, 16, 16, 0);
	glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, INT_MAX, INT_MAX, 16, 16, 0);
	glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, INT_MAX - 3, -5, 8, 8);
	glGenerateMipmap(GL_TEXTURE_2D);
	for (unsigned i = 0; i < sizeof xy / sizeof xy[0]; i++)
		glReadPixels(xy[i][0], xy[i][1], 16, 16, GL_RGBA, GL_UNSIGNED_BYTE, out);
	glReadPixels(-0x7ffffff0, 0, INT_MAX, 1, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glGenRenderbuffers(1, &rb); glBindRenderbuffer(GL_RENDERBUFFER, rb);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, INT_MAX, INT_MAX);
	glRenderbufferStorageMultisample(GL_RENDERBUFFER, INT_MAX, GL_RGBA4, 4097, 1);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA4, 4096, 4096);
	glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_COMPRESSED_RGB8_ETC2, 4096, 4096, 0, INT_MIN, NULL);
	glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, INT_MAX & ~3, 0, 8, 4, GL_COMPRESSED_RGB8_ETC2, 16, px);
	E();
}

static void draw_extremes(void)
{
	static const struct { GLenum ty; int sz; } tys[] = { { GL_FLOAT, 4 }, { GL_SHORT, 2 }, { GL_UNSIGNED_BYTE, 1 }, { GL_FIXED, 3 } };
	static const int strides[] = { 0, 1, INT_MAX };
	static const uintptr_t offs[] = { 0, 255, 256, UINTPTR_MAX, UINTPTR_MAX - 7, (uintptr_t)1 << 40 };
	static const int fc[][2] = { { INT_MAX, INT_MAX }, { INT_MAX, 1 }, { 0, INT_MAX }, { 1, 3 } };
	static const uintptr_t eoffs[] = { 0, 254, 255, 256, UINTPTR_MAX, UINTPTR_MAX - 1, (uintptr_t)1 << 63 };
	static const GLenum ets[] = { GL_UNSIGNED_BYTE, GL_UNSIGNED_SHORT, GL_UNSIGNED_INT };
	static const int ecnt[] = { INT_MAX, 3, 128, 129 };
	static float v[64];
	GLuint b;

	up(); prog();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 256, v, GL_STATIC_DRAW);
	for (unsigned t = 0; t < 4; t++)
		for (unsigned s = 0; s < 3; s++)
			for (unsigned o = 0; o < 6; o++) {
				glVertexAttribPointer(0, tys[t].sz, tys[t].ty, 1, strides[s], (void *)offs[o]); glEnableVertexAttribArray(0);
				glVertexAttribPointer(1, tys[t].sz, tys[t].ty, 0, strides[s], (void *)offs[o]); glEnableVertexAttribArray(1);
				for (unsigned k = 0; k < 4; k++) {
					/* keep the converted path from really allocating 2^31 vertices */
					hh_fail_alloc_in(fc[k][1] == INT_MAX && tys[t].ty != GL_FLOAT ? 1 : 0);
					if (offs[o] == 0 || (offs[o] >= 255 && fc[k][1] < 100) || tys[t].ty == GL_FLOAT)
						glDrawArrays(GL_TRIANGLES, fc[k][0], fc[k][1]);
					E();
				}
			}
	hh_fail_alloc_in(0);
	glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, b);
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, NULL);
	glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, NULL);
	for (unsigned c = 0; c < 4; c++)
		for (unsigned o = 0; o < 7; o++)
			for (unsigned t = 0; t < 3; t++) {
				if (eoffs[o] > ((uintptr_t)1 << 62)) continue;
				if (ecnt[c] == INT_MAX) hh_fail_alloc_in(1);
				glDrawElements(GL_TRIANGLES, ecnt[c], ets[t], (void *)eoffs[o]);
				glDrawRangeElements(GL_TRIANGLES, 5, 1, ecnt[c], ets[t], (void *)eoffs[o]);
				hh_fail_alloc_in(0);
				E();
			}
	glFinish();
}

static void instanced_extremes(void)
{
	static const uintptr_t offs[] = { 0, 252, 253, UINTPTR_MAX - 3, UINTPTR_MAX };
	static const int strides[] = { 0, INT_MAX };
	static const unsigned divs[] = { 1, UINT_MAX };
	static float v[64];
	GLuint b;

	up(); prog();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 256, v, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, NULL); glEnableVertexAttribArray(0);
	for (unsigned o = 0; o < 5; o++)
		for (unsigned s = 0; s < 2; s++) {
			glVertexAttribPointer(1, 4, GL_FLOAT, 0, strides[s], (void *)offs[o]); glEnableVertexAttribArray(1);
			for (unsigned d = 0; d < 2; d++) {
				glVertexAttribDivisor(1, divs[d]);
				glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 5);
				glDrawArraysInstanced(GL_TRIANGLES, 0, 3, INT_MIN);
				E();
			}
		}
	glFinish();
}

static void instanced_attrib_without_pointer(void)
{
	static float v[64];
	GLuint b;

	up(); prog();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 256, v, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, NULL); glEnableVertexAttribArray(0);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glEnableVertexAttribArray(1); glVertexAttribDivisor(1, 1);
	glDrawArraysInstanced(GL_TRIANGLES, 0, 3, 2);
	glFinish();
}

static void over_65535_vertices(void)
{
	float *v = calloc(70000 * 4, sizeof(float));
	GLuint b;

	up(); prog();
	glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
	glBufferData(GL_ARRAY_BUFFER, 70000 * 16, v, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, NULL); glEnableVertexAttribArray(0);
	glDrawArrays(GL_TRIANGLES, 0, 69999);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, v);
	glDrawArrays(GL_TRIANGLES, 0, 69999);
	glFinish();
	free(v);
}

static void blend_variant_oom(void)
{
	float verts[] = { -1, -1, 0, 1, 1, -1, 0, 1, 0, 1, 0, 1 };

	up(); prog();
	glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts); glEnableVertexAttribArray(0);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	glEnable(GL_BLEND); glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
	hh_fail_alloc_in(1);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
	hh_step("memory available again: this draw must be blended");
	hh_fail_alloc_in(0);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	glFinish();
}

static void alloc_failure_sweep(void)
{
	GLuint p;

	up();
	p = program("passthrough.vert", "uniform_color.frag");
	glUseProgram(p);
	for (unsigned n = 1; n < 14; n++) {
		hh_step("fail allocation %u from here", n);
		hh_fail_alloc_in(n);
		for (int round = 0; round < 2; round++) {
			GLuint t, fb, b;
			uint8_t px[8 * 8 * 4], out[8 * 8 * 4], ix[] = { 0, 1, 2 };
			int16_t v[64] = { 0 };

			memset(px, 0x7f, sizeof px);
			glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
			glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, px);
			glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px);
			glGenerateMipmap(GL_TEXTURE_2D);
			glGenFramebuffers(1, &fb); glBindFramebuffer(GL_FRAMEBUFFER, fb);
			glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
			glGenBuffers(1, &b); glBindBuffer(GL_ARRAY_BUFFER, b);
			glBufferData(GL_ARRAY_BUFFER, 128, v, GL_STATIC_DRAW);
			glBufferSubData(GL_ARRAY_BUFFER, 0, 128, v);
			glVertexAttribPointer(0, 2, GL_SHORT, 0, 0, NULL); glEnableVertexAttribArray(0);
			glEnable(GL_BLEND); glBlendFunc(GL_SRC_ALPHA, GL_ONE);
			glClear(GL_COLOR_BUFFER_BIT);
			glDrawArrays(GL_TRIANGLES, 0, 3);
			glBindBuffer(GL_ARRAY_BUFFER, 0);
			glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_BYTE, ix);
			glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, out);
			glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 8, 8, 0);
			glFinish();
			E();
			glBindFramebuffer(GL_FRAMEBUFFER, 0);
		}
	}
}

/* ---- EGL ---- */

static EGLDisplay egl_up(void)
{
	EGLDisplay d;

	up();
	d = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	eglInitialize(d, NULL, NULL);
	return d;
}

static EGLConfig cfg1(EGLDisplay d)
{
	EGLConfig c = NULL;
	EGLint n = 0;

	eglChooseConfig(d, (EGLint[]){ EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_NONE }, &c, 1, &n);
	return c;
}

static void egl_image_destroy_then_use_sibling(void)
{
	EGLDisplay d = egl_up();
	EGLConfig c = cfg1(d);
	EGLSurface s = eglCreatePbufferSurface(d, c, (EGLint[]){ EGL_WIDTH, 8, EGL_HEIGHT, 8, EGL_NONE });
	EGLContext x = eglCreateContext(d, c, NULL, (EGLint[]){ EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE });
	GLuint t[2];
	uint8_t px[8 * 8 * 4];
	EGLImage img;

	memset(px, 0x55, sizeof px);
	eglMakeCurrent(d, s, s, x);
	glGenTextures(2, t);
	glBindTexture(GL_TEXTURE_2D, t[0]);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, px);
	img = eglCreateImage(d, x, EGL_GL_TEXTURE_2D, (EGLClientBuffer)(uintptr_t)t[0], NULL);
	glBindTexture(GL_TEXTURE_2D, t[1]);
	glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img);
	eglDestroyImage(d, img);
	hh_step("image destroyed; both textures are still alive: upload into the sibling");
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glBindTexture(GL_TEXTURE_2D, t[0]);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glDeleteTextures(2, t);
	eglTerminate(d);
}

static void egl_image_context_destroy(void)
{
	/* an external image bound as a texture; the context goes first */
	static uint8_t pix[16 * 16 * 4];
	struct gles_native_window pm = { (uint64_t)(uintptr_t)pix, 16, 16, 64, 1 };
	EGLDisplay d;
	EGLConfig c;
	EGLSurface s;
	EGLContext x;
	EGLImage img;
	GLuint t, rb;

	hh_region_add(pix, sizeof pix, "pixmap16");
	d = egl_up();
	c = cfg1(d);
	s = eglCreatePbufferSurface(d, c, (EGLint[]){ EGL_WIDTH, 8, EGL_HEIGHT, 8, EGL_NONE });
	x = eglCreateContext(d, c, NULL, (EGLint[]){ EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE });
	eglMakeCurrent(d, s, s, x);
	img = eglCreateImage(d, EGL_NO_CONTEXT, EGL_NATIVE_PIXMAP_KHR, (EGLClientBuffer)&pm, NULL);
	glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
	glEGLImageTargetTexture2DOES(GL_TEXTURE_2D, img);
	glGenRenderbuffers(1, &rb); glBindRenderbuffer(GL_RENDERBUFFER, rb);
	glEGLImageTargetRenderbufferStorageOES(GL_RENDERBUFFER, img);
	eglMakeCurrent(d, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
	eglDestroyContext(d, x);
	hh_step("context destroyed; now the image");
	eglDestroyImage(d, img);
	eglTerminate(d);
}

static void egl_pbuffer_extremes(void)
{
	static const int wh[][2] = { { INT_MAX, INT_MAX }, { INT_MIN, 1 }, { 4097, 1 }, { 0, 0 }, { 4096, 4096 } };
	EGLDisplay d = egl_up();
	EGLConfig c = cfg1(d);
	EGLSurface ss[6];
	EGLint v;

	for (unsigned i = 0; i < 5; i++) {
		hh_fail_alloc_in(1);
		eglCreatePbufferSurface(d, c, (EGLint[]){ EGL_WIDTH, wh[i][0], EGL_HEIGHT, wh[i][1], EGL_NONE });
		eglGetError();
	}
	hh_fail_alloc_in(0);
	for (int i = 0; i < 6; i++) ss[i] = eglCreatePbufferSurface(d, c, (EGLint[]){ EGL_WIDTH, 3, EGL_HEIGHT, 3, EGL_NONE });
	eglGetError();
	eglQuerySurface(d, (EGLSurface)0x1234, EGL_WIDTH, &v);
	eglQuerySurface(d, (EGLSurface)((uintptr_t)ss[0] + 1), EGL_WIDTH, &v);
	eglDestroySurface(d, ss[0]);
	eglDestroySurface(d, ss[0]);
	eglSwapBuffers(d, ss[1]);
	eglGetProcAddress("");
	eglGetProcAddress("glDrawArrays\x01");
	for (int i = 1; i < 6; i++) eglDestroySurface(d, ss[i]);
	eglGetError();
	eglTerminate(d);
}

static void egl_gl_after_terminate(void)
{
	EGLDisplay d = egl_up();
	EGLConfig c = cfg1(d);
	EGLSurface s = eglCreatePbufferSurface(d, c, (EGLint[]){ EGL_WIDTH, 5, EGL_HEIGHT, 5, EGL_NONE });
	EGLContext x = eglCreateContext(d, c, NULL, (EGLint[]){ EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE });
	uint8_t out[5 * 5 * 4];

	eglMakeCurrent(d, s, s, x);
	glClear(GL_COLOR_BUFFER_BIT);
	hh_step("destroy the surface the current context renders to, keep drawing");
	eglDestroySurface(d, s);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glReadPixels(0, 0, 5, 5, GL_RGBA, GL_UNSIGNED_BYTE, out);
	eglTerminate(d);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	glDrawArrays(GL_TRIANGLES, 0, 3);
	E();
}

static void egl_destroy_other_contexts_surface(void)
{
	EGLDisplay d = egl_up();
	EGLConfig c = cfg1(d);
	EGLint pa[] = { EGL_WIDTH, 4, EGL_HEIGHT, 4, EGL_NONE }, xa[] = { EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE };
	EGLSurface s1, s2;
	EGLContext c1, c2;

	gl_context_destroy(hctx);
	s1 = eglCreatePbufferSurface(d, c, pa);
	s2 = eglCreatePbufferSurface(d, c, pa);
	c1 = eglCreateContext(d, c, NULL, xa);
	c2 = eglCreateContext(d, c, NULL, xa);
	eglMakeCurrent(d, s1, s1, c1);
	glClear(GL_COLOR_BUFFER_BIT);
	eglMakeCurrent(d, s2, s2, c2);
	hh_step("destroy c1's surface while c2 is current");
	eglDestroySurface(d, s1);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	eglMakeCurrent(d, s2, s2, c1);
	glClear(GL_COLOR_BUFFER_BIT);
	glFinish();
	eglMakeCurrent(d, NULL, NULL, NULL);
	eglDestroyContext(d, c1);
	eglDestroyContext(d, c2);
	eglDestroySurface(d, s2);
	eglTerminate(d);
}

static void generate_mipmap_in_flight(void)
{
	static uint8_t px[32 * 32 * 4];
	GLuint t, fb;

	up(); prog();
	glGenTextures(1, &t); glBindTexture(GL_TEXTURE_2D, t);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 32, 32, 0, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glGenFramebuffers(1, &fb); glBindFramebuffer(GL_FRAMEBUFFER, fb);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
	glClearColor(0, 1, 0, 1);
	glClear(GL_COLOR_BUFFER_BIT);
	glFlush();   /* with HH_LAG the render is still in flight */
	glGenerateMipmap(GL_TEXTURE_2D);
	glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, px);
	glFinish();
}

static const struct { const char *name; void (*fn)(void); } T[] = {
	{ "vbo_negative_offset", vbo_negative_offset },
	{ "elements_negative_u16", elements_negative_u16 },
	{ "elements_negative_u8", elements_negative_u8 },
	{ "map_buffer_range_wrap", map_buffer_range_wrap },
	{ "flush_mapped_wrap", flush_mapped_wrap },
	{ "buffer_subdata_wrap", buffer_subdata_wrap },
	{ "unaligned_uniform4fv", unaligned_uniform4fv },
	{ "unaligned_get_integerv", unaligned_get_integerv },
	{ "unaligned_gen_textures", unaligned_gen_textures },
	{ "unaligned_vertex_attrib4fv", unaligned_vertex_attrib4fv },
	{ "unaligned_get_uniformfv", unaligned_get_uniformfv },
	{ "compressed_texture_as_fbo_color", compressed_texture_as_fbo_color },
	{ "rgb565_external_readpixels", rgb565_external_readpixels },
	{ "tex_sub_image_odd_width_external", tex_sub_image_odd_width_external },
	{ "odd_width_pbuffer_readpixels", odd_width_pbuffer_readpixels },
	{ "context_create_overflow", context_create_overflow },
	{ "uniform_location_fuzz", uniform_location_fuzz },
	{ "get_uniform_array_into_small_buffer", get_uniform_array_into_small_buffer },
	{ "shader_source_and_logs", shader_source_and_logs },
	{ "texture_size_extremes", texture_size_extremes },
	{ "draw_extremes", draw_extremes },
	{ "instanced_extremes", instanced_extremes },
	{ "instanced_attrib_without_pointer", instanced_attrib_without_pointer },
	{ "over_65535_vertices", over_65535_vertices },
	{ "blend_variant_oom", blend_variant_oom },
	{ "alloc_failure_sweep", alloc_failure_sweep },
	{ "egl_image_destroy_then_use_sibling", egl_image_destroy_then_use_sibling },
	{ "egl_image_context_destroy", egl_image_context_destroy },
	{ "egl_pbuffer_extremes", egl_pbuffer_extremes },
	{ "egl_gl_after_terminate", egl_gl_after_terminate },
	{ "egl_destroy_other_contexts_surface", egl_destroy_other_contexts_surface },
	{ "generate_mipmap_in_flight", generate_mipmap_in_flight },
};

int main(int argc, char **argv)
{
	const char *n = argc > 1 ? argv[1] : "list";

	if (!strcmp(n, "list")) {
		for (unsigned i = 0; i < sizeof T / sizeof T[0]; i++)
			puts(T[i].name);
		return 0;
	}
	hh_install();
	SH = getenv("HH_SHADERS");
	if (!SH) { printf("HH-ERROR HH_SHADERS is not set\n"); return 2; }
	for (unsigned i = 0; i < sizeof T / sizeof T[0]; i++)
		if (!strcmp(T[i].name, n)) {
			hh_step("hostile %s", n);
			T[i].fn();
			printf("hostile %s survived errors=%u\n", n, hh_errors());
			return 0;
		}
	printf("HH-ERROR unknown sequence %s\n", n);
	return 2;
}
