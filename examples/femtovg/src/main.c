/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * femtovg: one femtovg frame, the same source on the Raspberry Pi 4 (V3D),
 * the Verdin AM62P (PowerVR) and the Verdin iMX8M Plus (GC7000). The board
 * is chosen by the Zephyr board only; everything that differs per board is
 * behind yakogl_board.h.
 *
 * femtovg (a 2D vector renderer, the one Slint draws with) draws a clear, a
 * rounded rect, a gradient rect and a stroked polyline through the public
 * OpenGL ES 2.0 API on YakoGL, into a 1024x600 window. This file is the
 * thin part: the window, EGL, the program survey, the screen and the
 * records. What is drawn and what is expected are the check crate's
 * (check/src/lib.rs, through rust/src/lib.rs), one definition for both:
 *
 *   femtovg_scene()                      window size, fill words
 *   board_window(1024, 600 + guard rows) sentinel window, guard rows
 *   EGL up (RGBA8888 + stencil 8, ES 2)  the window's 1024x600 only
 *   programs_run()                       femtovg's 30 programs, each
 *                                        failure named (programs.h)
 *   femtovg_draw(), eglSwapBuffers, board_show()
 *   femtovg_check()                      every pixel classified, guard
 *                                        rows, glReadPixels against
 *                                        memory, and PICTURE's words
 *   board_result(PICTURE)                the frame's fingerprint
 *   board_result(APP, fail)              only after a failed check
 *   board_finish()
 *
 * The records are the triangle's (examples/triangle/src/main.c): the last
 * record of a run is its verdict. A checked frame stores PICTURE. After a
 * pass that is the last record. After a failed check an APP record follows
 * it: w1 = the check's failure code << 8 | 2 (fail), the codes being
 * check/src/lib.rs `Failure`; w2 = 1 frame presented; w3 = the GPU resets.
 * A run that never reached the check stores no PICTURE, and board_finish()
 * stores its APP fail record.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <stdint.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include <yakogl_board.h>
#include <yakogl_record.h>

#include "programs.h"

/* Where a run that died stopped (kind PROGRESS, YK_RECORD_WHERE_STAGE | n). */
enum stage {
	STAGE_WINDOW = 1,
	STAGE_EGL,
	STAGE_PROGRAMS,
	STAGE_DRAW,
	STAGE_SWAPPED,
	STAGE_CHECKED,
};

/* ---- rust/src/lib.rs ------------------------------------------------------ */

struct femtovg_scene {
	uint32_t w, h, guard_rows, sentinel, guard;
};
BUILD_ASSERT(sizeof(struct femtovg_scene) == 20, "rust/src/lib.rs Scene");

struct femtovg_result {
	uint32_t status;  /* YK_RECORD_STATUS_PASS / _FAIL */
	uint32_t failure; /* check/src/lib.rs Failure, 0 for a pass */
	uint32_t picture[3];
};
BUILD_ASSERT(sizeof(struct femtovg_result) == 20, "rust/src/lib.rs CheckResult");

typedef void (*femtovg_read_pixels_fn)(GLint x, GLint y, GLsizei w, GLsizei h, GLenum format,
				       GLenum type, void *pixels);
typedef const void *(*femtovg_get_proc_fn)(const char *name);
void femtovg_scene(struct femtovg_scene *out);
int femtovg_draw(femtovg_get_proc_fn get_proc, uint32_t w, uint32_t h);
int femtovg_check(const uint32_t *cpu, uint32_t w, uint32_t h, uint32_t guard_rows,
		  uint32_t format, femtovg_read_pixels_fn read_pixels, struct femtovg_result *out);

/* ---- the run -------------------------------------------------------------- */

static struct femtovg_scene s_scene;
static struct board_window s_win; /* the window as EGL and the screen see it */
static EGLDisplay s_dpy;
static EGLSurface s_surf;
static struct programs_survey s_survey;
static struct femtovg_result s_result;
static bool s_checked;

/* femtovg and glow resolve every GL entry point through this. */
static const void *get_proc(const char *name)
{
	return (const void *)eglGetProcAddress(name);
}

/* Each EGL call is checked on its own: eglGetError reports the LAST call. */
static int egl_fail(const char *call)
{
	printk("femtovg: %s failed: EGL error 0x%x\n", call, eglGetError());
	return -EIO;
}

static int window(void)
{
	const uint32_t w = s_scene.w, h = s_scene.h, guard_rows = s_scene.guard_rows;
	struct board_window full;
	int ret = board_window(w, h + guard_rows, &full);

	if (ret) {
		printk("femtovg: board_window(%u, %u) = %d\n", w, h + guard_rows, ret);
		return ret;
	}
	if (full.stride != w * 4u) {
		printk("femtovg: board_window gave stride %u, not %u\n", full.stride, w * 4u);
		return -EINVAL;
	}
	/* The window starts with the sentinel, the guard rows after it with
	 * the guard word, written through the CPU's view (Normal non-cacheable
	 * on every board, yakogl_board.h: no cache maintenance). */
	uint32_t *px = full.cpu;
	size_t win_px = (size_t)w * h, all_px = (size_t)w * (h + guard_rows);

	for (size_t i = 0; i < all_px; i++)
		px[i] = i < win_px ? s_scene.sentinel : s_scene.guard;
	s_win = full;
	s_win.h = h; /* EGL and the screen get the window, not the guard rows */
	printk("femtovg: window %ux%u at phys 0x%lx, format 0x%x, %u guard rows\n", s_win.w,
	       s_win.h, (unsigned long)s_win.phys, s_win.format, guard_rows);
	return 0;
}

static int egl_up(void)
{
	/* EGL_SINGLE_BUFFER: the window is shown itself (board_show after the
	 * swap) and read back as memory (examples/triangle/src/main.c). */
	static const EGLint single_buffer[] = { EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER, EGL_NONE };
	/* femtovg fills concave paths and strokes through the stencil */
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window nw;
	EGLConfig cfg;
	EGLint n = 0;

	nw = (struct gles_native_window){
		.phys = s_win.phys, .width = s_win.w, .height = s_win.h,
		.stride_bytes = s_win.stride, .format = s_win.format,
	};
	s_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_dpy, NULL, NULL))
		return egl_fail("eglInitialize");
	if (!eglChooseConfig(s_dpy, cfg_attribs, &cfg, 1, &n))
		return egl_fail("eglChooseConfig");
	if (n != 1) {
		printk("femtovg: eglChooseConfig: %d configs for RGBA8888 + stencil 8\n", n);
		return -ENOENT;
	}
	s_surf = eglCreateWindowSurface(s_dpy, cfg, &nw, single_buffer);
	if (s_surf == EGL_NO_SURFACE)
		return egl_fail("eglCreateWindowSurface");
	EGLContext ctx = eglCreateContext(s_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (ctx == EGL_NO_CONTEXT)
		return egl_fail("eglCreateContext");
	if (!eglMakeCurrent(s_dpy, s_surf, s_surf, ctx))
		return egl_fail("eglMakeCurrent");
	printk("femtovg: GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	return 0;
}

/* The one frame: femtovg's draw, eglSwapBuffers (submits and waits). */
static int draw(void)
{
	int64_t t0 = k_uptime_get();
	int rc = femtovg_draw(get_proc, s_win.w, s_win.h);
	GLenum err_draw = glGetError();

	if (rc != 0) {
		printk("femtovg: femtovg_draw = %d (GL error 0x%x): %s\n", rc, err_draw,
		       rc == 1 && s_survey.failed ? "femtovg's programs do not all link (above)" :
		       rc == 1 ? "the renderer did not start, yet every program linked here" :
				 "the canvas could not be created");
		return -EIO;
	}
	/* the canvas and its programs are gone again: what stays resident */
	programs_keep_print("after femtovg_draw (its canvas already dropped)");
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_DRAW);
	if (!eglSwapBuffers(s_dpy, s_surf))
		return egl_fail("eglSwapBuffers");
	GLenum err = glGetError();

	printk("femtovg: frame %lld ms\n", (long long)(k_uptime_get() - t0));
	if (err_draw != GL_NO_ERROR || err != GL_NO_ERROR) {
		/* a refused draw logs its reason above */
		printk("femtovg: GL error 0x%x after the draw, 0x%x after the swap\n", err_draw, err);
		return -EIO;
	}
	return 0;
}

static int run(void)
{
	int ret;

	board_progress(YK_RECORD_WHERE_STAGE | STAGE_WINDOW);
	ret = window();
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_EGL);
	ret = egl_up();
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_PROGRAMS);
	programs_run(&s_survey);
	programs_print(&s_survey);
	ret = draw();
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_SWAPPED);
	/* the check reads the window, not the screen: go on after an error */
	ret = board_show(&s_win);
	if (ret)
		printk("femtovg: board_show = %d (the window is not on the screen)\n", ret);
	/* the guard rows follow the window in memory: s_win.cpu covers both */
	ret = femtovg_check(s_win.cpu, s_win.w, s_win.h, s_scene.guard_rows, s_win.format,
			    glReadPixels, &s_result);
	s_checked = ret >= 0;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_CHECKED);
	if (ret < 0)
		return -EINVAL; /* not checked: the reason is printed */
	return ret == 0 ? 0 : -EBADMSG;
}

int main(void)
{
	int rc;

	femtovg_scene(&s_scene);
	printk("femtovg: one femtovg frame through YakoGL, %ux%u window\n", s_scene.w, s_scene.h);
	rc = run();
	if (s_checked) {
		board_result(YK_RECORD_KIND_PICTURE, s_result.picture[0], s_result.picture[1],
			     s_result.picture[2]);
		if (s_result.status != YK_RECORD_STATUS_PASS)
			board_result(YK_RECORD_KIND_APP,
				     s_result.failure << 8 | YK_RECORD_STATUS_FAIL, 1u,
				     board_gpu_resets());
	}
	board_finish("femtovg", rc);
}
