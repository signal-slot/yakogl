/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_femtovg - draws one FemtoVG frame through YakoGL + EGL. The Rust
 * staticlib (fvg_zephyr, wrapping Agent A's no_std fvg_demo over femtovg/glow)
 * exposes fvg_draw_frame(); we bring up an EGL window surface over a scan-out
 * buffer, make it current, hand FemtoVG a get-proc-address hook, then show the
 * result on the LVDS panel (pvr_sample_show_on_dss).
 */
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <zephyr/cache.h>
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <stdint.h>
#include "sample_glue.h"

LOG_MODULE_REGISTER(gl_femtovg, LOG_LEVEL_INF);

#define FB_W CONFIG_GL_FEMTOVG_WINDOW_WIDTH
#define FB_H CONFIG_GL_FEMTOVG_WINDOW_HEIGHT

#if CONFIG_GL_FEMTOVG_WINDOW_PHYS == 0
/* The window: a tightly packed buffer in the image, identity-mapped so its
 * address is its physical address. The GPU and the display read/write it
 * directly, so it is not zeroed by the boot code (__noinit) and the CPU cleans
 * its cache lines before use. XRGB8888 when a display scans it out. */
static uint8_t s_fb[FB_W * FB_H * 4] __noinit __aligned(4096);
#define FB_PHYS   ((uintptr_t)s_fb)
#define FB_STRIDE (FB_W * 4)
#ifdef CONFIG_PVR_SAMPLE_SHOW_ON_DSS
#define FB_FORMAT GLES_NATIVE_FORMAT_XRGB8888
#else
#define FB_FORMAT GLES_NATIVE_FORMAT_RGBA8888
#endif
#else
/* A scan-out buffer (XRGB8888 rows) that the display keeps showing. */
#define FB_PHYS   ((uintptr_t)CONFIG_GL_FEMTOVG_WINDOW_PHYS)
#define FB_STRIDE (CONFIG_GL_FEMTOVG_WINDOW_STRIDE ? CONFIG_GL_FEMTOVG_WINDOW_STRIDE : FB_W * 4)
#define FB_FORMAT GLES_NATIVE_FORMAT_XRGB8888
#endif
/* the window is drawn y-up when the display scans it bottom row first */
#define WIN_FORMAT (FB_FORMAT | pvr_sample_window_flags())

/* Rust entry point (fvg_zephyr staticlib, re-exported from fvg_demo). Draws one
 * FemtoVG frame through the currently-current GL ES 2.0 context. */
int fvg_draw_frame(const void *(*get_proc)(const char *name),
		   uint32_t width, uint32_t height);

/* FemtoVG/glow resolve every GL entry point they need through this hook. */
static const void *fvg_get_proc(const char *name)
{
	return (const void *)eglGetProcAddress(name);
}

#if CONFIG_GL_FEMTOVG_WINDOW_PHYS == 0
static void fill_pattern(void)
{
	uint32_t *px = (uint32_t *)s_fb;

	for (int y = 0; y < FB_H; y++) {
		uint32_t c = y < FB_H / 12 ? 0x00ffffffu : 0x00102040u;

		for (int x = 0; x < FB_W; x++) {
			px[y * FB_W + x] = c;
		}
	}
}
#endif

static int draw(void)
{
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_DEPTH_SIZE, 16, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	struct gles_native_window win = {
		.phys = FB_PHYS, .width = FB_W, .height = FB_H,
		.stride_bytes = FB_STRIDE, .format = WIN_FORMAT,
	};
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLSurface surf;
	EGLContext ctx;
	EGLint n = 0;
	int rc;

	if (!eglInitialize(dpy, NULL, NULL) || !eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1) {
		LOG_ERR("EGL init failed: 0x%x", eglGetError());
		return 1;
	}
	surf = eglCreateWindowSurface(dpy, cfg, &win, NULL);
	ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (surf == EGL_NO_SURFACE || ctx == EGL_NO_CONTEXT || !eglMakeCurrent(dpy, surf, surf, ctx)) {
		LOG_ERR("EGL surface/context failed: 0x%x", eglGetError());
		return 1;
	}

	glViewport(0, 0, FB_W, FB_H);
	rc = fvg_draw_frame(fvg_get_proc, FB_W, FB_H);
	if (rc != 0) {
		LOG_ERR("fvg_draw_frame failed: %d (GL error 0x%x)", rc, glGetError());
		return 1;
	}
	eglSwapBuffers(dpy, surf);
	LOG_INF("FemtoVG frame drawn (GL error 0x%x)", glGetError());

	/* Ground-truth read-back: sample known points so the render is verifiable
	 * without a camera. GL origin is bottom-left, so y is flipped from the
	 * fvg_demo layout. Expected (fvg_demo.rs): bg rgb(24,24,32);
	 * blue rounded rect rgb(80,160,240) around screen (26%,25%);
	 * gradient rect top pink rgb(240,80,120) .. bottom yellow rgb(250,210,90)
	 * around screen x 73%. */
	{
		struct { const char *what; int x, y; } pts[] = {
			{ "bg",        FB_W * 1 / 2,  FB_H * 1 / 2 },
			{ "blue-rect", FB_W * 26 / 100, FB_H - 1 - FB_H * 25 / 100 },
			{ "grad-top",  FB_W * 73 / 100, FB_H - 1 - FB_H * 13 / 100 },
			{ "grad-bot",  FB_W * 73 / 100, FB_H - 1 - FB_H * 37 / 100 },
		};
		for (unsigned i = 0; i < sizeof(pts) / sizeof(pts[0]); i++) {
			uint8_t px[4] = { 0, 0, 0, 0 };

			glReadPixels(pts[i].x, pts[i].y, 1, 1, GL_RGBA,
				     GL_UNSIGNED_BYTE, px);
			LOG_INF("readback %-9s (%4d,%4d) = R=%3u G=%3u B=%3u A=%3u",
				pts[i].what, pts[i].x, pts[i].y,
				px[0], px[1], px[2], px[3]);
		}
	}

	return 0;
}

int main(void)
{
	int rc;

	LOG_INF("gl_femtovg: %ux%u window at 0x%lx", FB_W, FB_H, (unsigned long)FB_PHYS);
#if CONFIG_GL_FEMTOVG_WINDOW_PHYS == 0
	/* A CPU-drawn pattern first, so the display path can be judged even when
	 * the GPU draw fails; FemtoVG overwrites it. */
	fill_pattern();
	sys_cache_data_flush_and_invd_range(s_fb, sizeof(s_fb));
#endif
	rc = draw();
	LOG_INF("gl_femtovg: %s", rc ? "FAILED" : "done");
	if (pvr_sample_show_window(FB_PHYS, FB_W, FB_H, FB_STRIDE, WIN_FORMAT) && rc == 0) {
		rc = -EIO;
	}
	return pvr_sample_finish(rc);
}
