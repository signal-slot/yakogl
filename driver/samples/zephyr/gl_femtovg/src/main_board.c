/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_femtovg on the board layer (yakogl_board.h): the same FemtoVG frame as
 * main.c (fvg_demo's fvg_draw_frame through the Rust staticlib), for the
 * boards whose board file turns CONFIG_YAKOGL_BOARD on. Today that is the
 * Verdin iMX8M Plus (GC7000); the AM62P keeps main.c and its pvr_sample_*
 * glue until the sample moves to the board layer as a whole
 * (driver/docs/one-method.md, section 5, step 4).
 *
 * The run follows slint/src/main.c:
 *
 *   board_window(W, H)                the GPU carve-out on the i.MX8MP
 *   EGL up (RGBA8888 + stencil 8, ES 2)
 *   fvg_draw_frame, eglSwapBuffers, board_show()
 *   four glReadPixels probes, judged against fvg_demo's scene
 *   board_finish(): the verdict line and the APP record
 *
 * The probes are main.c's four points. Their expected colours come from
 * fvg_demo/src/lib.rs: the clear rgb(24,24,32), the rounded rect
 * rgb(80,160,240), and the vertical gradient from rgb(240,80,120) at the
 * top of its rect to rgb(250,210,90) at the bottom, mixed at the pixel
 * centre. GRAD_TOL is v3d_gl_femtovg's: femtovg dithers gradients by up to
 * half an 8-bit step.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include <yakogl_board.h>
#include <yakogl_record.h>

#define WIN_W CONFIG_GL_FEMTOVG_WINDOW_WIDTH
#define WIN_H CONFIG_GL_FEMTOVG_WINDOW_HEIGHT

/* board_window() decides where the window is; a fixed address is main.c's. */
BUILD_ASSERT(CONFIG_GL_FEMTOVG_WINDOW_PHYS == 0,
	     "GL_FEMTOVG_WINDOW_PHYS is not used on the board layer: board_window() places the window");

#define GRAD_TOL 2

/* Where a run that died stopped (kind PROGRESS, YK_RECORD_WHERE_STAGE | n). */
enum stage {
	STAGE_WINDOW = 1,
	STAGE_EGL,
	STAGE_DRAW,
	STAGE_SWAPPED,
	STAGE_CHECKED,
};

/* Rust entry point (fvg_zephyr staticlib, re-exported from fvg_demo). Draws one
 * FemtoVG frame through the currently-current GL ES 2.0 context. */
int fvg_draw_frame(const void *(*get_proc)(const char *name), uint32_t width, uint32_t height);

/* FemtoVG/glow resolve every GL entry point they need through this hook. */
static const void *fvg_get_proc(const char *name)
{
	return (const void *)eglGetProcAddress(name);
}

static struct board_window s_win;
static EGLDisplay s_dpy;
static EGLSurface s_surf;

/* Each EGL call is checked on its own: eglGetError reports the LAST call. */
static int egl_fail(const char *call)
{
	printk("gl_femtovg: %s failed: EGL error 0x%x\n", call, eglGetError());
	return -EIO;
}

static int egl_up(void)
{
	/* femtovg fills and strokes through the stencil (as in slint) */
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window nw;
	EGLConfig cfg;
	EGLContext ctx;
	EGLint n = 0;

	nw = (struct gles_native_window){
		.phys = s_win.phys, .width = s_win.w, .height = s_win.h,
		.stride_bytes = s_win.stride, .format = s_win.format,
	};
	s_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_dpy, NULL, NULL)) {
		return egl_fail("eglInitialize");
	}
	if (!eglChooseConfig(s_dpy, cfg_attribs, &cfg, 1, &n)) {
		return egl_fail("eglChooseConfig");
	}
	if (n != 1) {
		printk("gl_femtovg: eglChooseConfig: %d configs for RGBA8888 + stencil 8\n", n);
		return -ENOENT;
	}
	s_surf = eglCreateWindowSurface(s_dpy, cfg, &nw, NULL);
	if (s_surf == EGL_NO_SURFACE) {
		return egl_fail("eglCreateWindowSurface");
	}
	ctx = eglCreateContext(s_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (ctx == EGL_NO_CONTEXT) {
		return egl_fail("eglCreateContext");
	}
	if (!eglMakeCurrent(s_dpy, s_surf, s_surf, ctx)) {
		return egl_fail("eglMakeCurrent");
	}
	printk("gl_femtovg: GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	return 0;
}

static int draw(void)
{
	int ret;
	GLenum err;

	board_progress(YK_RECORD_WHERE_STAGE | STAGE_DRAW);
	glViewport(0, 0, WIN_W, WIN_H);
	ret = fvg_draw_frame(fvg_get_proc, WIN_W, WIN_H);
	if (ret != 0) {
		printk("gl_femtovg: fvg_draw_frame failed: %d (GL error 0x%x)\n", ret, glGetError());
		return -EIO;
	}
	if (!eglSwapBuffers(s_dpy, s_surf)) {
		return egl_fail("eglSwapBuffers");
	}
	err = glGetError();
	if (err != GL_NO_ERROR) {
		printk("gl_femtovg: GL error 0x%x during the frame\n", err);
		return -EIO;
	}
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_SWAPPED);
	ret = board_show(&s_win);
	if (ret != 0) {
		/* the check reads the window, not the screen: go on */
		printk("gl_femtovg: board_show = %d (the window is not on the screen)\n", ret);
	}
	return 0;
}

/* The gradient of fvg_demo's rect (b) at canvas row y: its rect starts at
 * 10% of the height and is 30% of it tall; the colour is mixed at the pixel
 * centre. */
static uint8_t grad_mix(uint8_t top, uint8_t bottom, int y)
{
	float t = ((float)y + 0.5f - WIN_H * 0.10f) / (WIN_H * 0.30f);

	t = t < 0.0f ? 0.0f : (t > 1.0f ? 1.0f : t);
	return (uint8_t)((float)top + ((float)bottom - (float)top) * t + 0.5f);
}

/* Main.c's four points, now judged. x, y are canvas coordinates (y down);
 * glReadPixels counts rows from the bottom. Returns how many points are off. */
static int check(void)
{
	struct probe {
		const char *what;
		int x, y;
		uint8_t rgb[3];
	} pts[] = {
		{ "bg", WIN_W * 1 / 2, WIN_H * 1 / 2, { 24, 24, 32 } },
		{ "blue-rect", WIN_W * 26 / 100, WIN_H * 25 / 100, { 80, 160, 240 } },
		{ "grad-top", WIN_W * 73 / 100, WIN_H * 13 / 100, { 0 } },
		{ "grad-bot", WIN_W * 73 / 100, WIN_H * 37 / 100, { 0 } },
	};
	static const uint8_t grad_top[3] = { 240, 80, 120 }, grad_bot[3] = { 250, 210, 90 };
	int bad = 0;

	for (int c = 0; c < 3; c++) {
		pts[2].rgb[c] = grad_mix(grad_top[c], grad_bot[c], pts[2].y);
		pts[3].rgb[c] = grad_mix(grad_top[c], grad_bot[c], pts[3].y);
	}
	for (unsigned int i = 0; i < ARRAY_SIZE(pts); i++) {
		uint8_t px[4] = { 0, 0, 0, 0 };
		bool ok;

		glReadPixels(pts[i].x, WIN_H - 1 - pts[i].y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		ok = px[3] == 255;
		for (int c = 0; c < 3; c++) {
			ok = ok && abs((int)px[c] - (int)pts[i].rgb[c]) <= GRAD_TOL;
		}
		printk("gl_femtovg: readback %-9s (%4d,%4d) = R=%3u G=%3u B=%3u A=%3u, "
		       "expected %3u %3u %3u 255: %s\n",
		       pts[i].what, pts[i].x, pts[i].y, px[0], px[1], px[2], px[3], pts[i].rgb[0],
		       pts[i].rgb[1], pts[i].rgb[2], ok ? "ok" : "OFF");
		bad += ok ? 0 : 1;
	}
	return bad;
}

int main(void)
{
	int rc;

	printk("gl_femtovg: one FemtoVG frame at %ux%u on the board layer\n", WIN_W, WIN_H);
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_WINDOW);
	rc = board_window(WIN_W, WIN_H, &s_win);
	if (rc != 0) {
		printk("gl_femtovg: board_window(%u, %u) = %d\n", WIN_W, WIN_H, rc);
	} else {
		printk("gl_femtovg: window %ux%u at phys 0x%lx, stride %u, format 0x%x\n", s_win.w,
		       s_win.h, (unsigned long)s_win.phys, s_win.stride, s_win.format);
		board_progress(YK_RECORD_WHERE_STAGE | STAGE_EGL);
		rc = egl_up();
	}
	if (rc == 0) {
		rc = draw();
	}
	if (rc == 0) {
		int bad = check();

		board_progress(YK_RECORD_WHERE_STAGE | STAGE_CHECKED);
		if (bad != 0) {
			printk("gl_femtovg: %d of 4 readback points off\n", bad);
			rc = -EIO;
		}
	}
	board_finish("gl_femtovg", rc);
}
