/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Shared part of the samples' host replay programs (replay.h). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include "replay.h"

extern unsigned int yakogl_draws_refused(void);

static EGLDisplay s_dpy;
static EGLSurface s_surf;
static void *s_win_buf;
static struct gles_native_window s_nw;

static unsigned s_frames, s_failed, s_glerrors, s_unresolved;
static unsigned s_refused_total, s_refused_mark;
static unsigned long s_draws, s_draws_mark;

/* Draws are counted through the glDraw* the archive receives. The wrappers
 * call the library's function, so the draw itself is unchanged. */
typedef void (*draw_arrays_fn)(GLenum, GLint, GLsizei);
typedef void (*draw_elements_fn)(GLenum, GLsizei, GLenum, const void *);
static draw_arrays_fn real_draw_arrays;
static draw_elements_fn real_draw_elements;

static void wrap_draw_arrays(GLenum mode, GLint first, GLsizei count)
{
	s_draws++;
	real_draw_arrays(mode, first, count);
}

static void wrap_draw_elements(GLenum mode, GLsizei count, GLenum type, const void *idx)
{
	s_draws++;
	real_draw_elements(mode, count, type, idx);
}

/* The compiled-shader store. A host build has a built-in 512 KiB region; an
 * image lends CONFIG_YAKOGL_SHADERC_KEEP_BYTES (zephyr/Kconfig, default 1 MiB,
 * src/osal/zephyr_shaderc_scratch.c), which is what the boards run with.
 * REPLAY_KEEP_BYTES overrides it (0: the built-in region). */
extern int shc_online_lend_keep(unsigned char *buf, size_t len);

static int lend_keep(void)
{
	const char *e = getenv("REPLAY_KEEP_BYTES");
	size_t len = e && *e ? (size_t)strtoul(e, NULL, 0) : 1048576;
	void *buf;

	if (len == 0)
		return 0;
	buf = aligned_alloc(4096, (len + 4095) & ~(size_t)4095);
	if (!buf || shc_online_lend_keep(buf, len)) {
		fprintf(stderr, "replay: lending %zu bytes of shader store failed\n", len);
		return -1;
	}
	printf("replay: shader store %zu bytes (the image's CONFIG_YAKOGL_SHADERC_KEEP_BYTES)\n", len);
	return 0;
}

int replay_egl_up(uint32_t w, uint32_t h)
{
	if (lend_keep())
		return -1;
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	EGLConfig cfg;
	EGLint n = 0;
	size_t stride = (size_t)w * 4u;

	s_win_buf = aligned_alloc(4096, (stride * h + 4095) & ~(size_t)4095);
	if (!s_win_buf) {
		fprintf(stderr, "replay: no memory for the %ux%u window\n", w, h);
		return -1;
	}
	memset(s_win_buf, 0, stride * h);
	s_nw = (struct gles_native_window){ .phys = (uintptr_t)s_win_buf, .width = w, .height = h,
					    .stride_bytes = stride, .format = GLES_NATIVE_FORMAT_XRGB8888 };
	s_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_dpy, NULL, NULL)) {
		fprintf(stderr, "replay: eglInitialize failed: 0x%x\n", eglGetError());
		return -1;
	}
	if (!eglChooseConfig(s_dpy, cfg_attribs, &cfg, 1, &n) || n != 1) {
		fprintf(stderr, "replay: eglChooseConfig: %d configs for RGBA8888 + stencil 8\n", n);
		return -1;
	}
	s_surf = eglCreateWindowSurface(s_dpy, cfg, &s_nw, NULL);
	EGLContext ctx = eglCreateContext(s_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (s_surf == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(s_dpy, s_surf, s_surf, ctx)) {
		fprintf(stderr, "replay: window surface, context or make-current failed: 0x%x\n", eglGetError());
		return -1;
	}
	printf("replay: GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	real_draw_arrays = (draw_arrays_fn)(void *)eglGetProcAddress("glDrawArrays");
	real_draw_elements = (draw_elements_fn)(void *)eglGetProcAddress("glDrawElements");
	if (!real_draw_arrays || !real_draw_elements) {
		fprintf(stderr, "replay: eglGetProcAddress has no glDrawArrays/glDrawElements\n");
		return -1;
	}
	s_refused_mark = yakogl_draws_refused();
	return 0;
}

const void *replay_get_proc(const char *name)
{
	const void *fn;

	if (!strcmp(name, "glDrawArrays"))
		return (const void *)wrap_draw_arrays;
	if (!strcmp(name, "glDrawElements"))
		return (const void *)wrap_draw_elements;
	fn = (const void *)eglGetProcAddress(name);
	if (!fn) {
		/* glow resolves every name at context creation and panics when it
		 * calls an unresolved one; the name is reported either way */
		printf("replay: unresolved %s\n", name);
		s_unresolved++;
	}
	return fn;
}

void replay_frame_end(const char *label, int rc)
{
	unsigned refused;
	GLenum err;

	if (!eglSwapBuffers(s_dpy, s_surf)) {
		printf("replay: eglSwapBuffers failed: 0x%x\n", eglGetError());
		rc = rc ? rc : -1;
	}
	err = glGetError();
	refused = yakogl_draws_refused() - s_refused_mark;
	s_refused_mark += refused;
	s_refused_total += refused;
	s_frames++;
	s_failed += rc != 0;
	s_glerrors += err != GL_NO_ERROR;
	printf("replay: frame %u %s: rc %d glerror 0x%x refused %u draws %lu\n", s_frames, label, rc, err, refused,
	       s_draws - s_draws_mark);
	s_draws_mark = s_draws;
}

int replay_finish(void)
{
	printf("replay: summary frames=%u failed=%u glerrors=%u refused=%u unresolved=%u\n", s_frames, s_failed,
	       s_glerrors, s_refused_total, s_unresolved);
	/* unresolved names are informational: glow asks for every GL name,
	 * desktop ones included, at context creation, and panics only on a
	 * call to one, so a run that finishes called none */
	return (s_failed || s_glerrors || s_refused_total || s_frames == 0) ? 1 : 0;
}
