/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_triangle - the smallest YakoGL application: EGL window surface over a
 * pixel buffer this program owns, one Gouraud triangle, read back through
 * glReadPixels. The same source draws on the LVDS panel when the window's
 * physical address is the scan-out buffer.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/crc.h>
#include <zephyr/logging/log.h>
#include <zephyr/cache.h>
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <gles_offline.h>
#include <string.h>
#include "sample_glue.h"

LOG_MODULE_REGISTER(gl_triangle, LOG_LEVEL_INF);

#define FB_W CONFIG_GL_TRIANGLE_WINDOW_WIDTH
#define FB_H CONFIG_GL_TRIANGLE_WINDOW_HEIGHT

#if CONFIG_GL_TRIANGLE_WINDOW_PHYS == 0
/* The window: a tightly packed buffer in the image. Zephyr's kernel memory
 * is identity mapped on this target, so its address is its physical address.
 * The GPU (and the display) read and write it directly, so it is not zeroed
 * by the boot code (__noinit) and the CPU cleans its cache lines before use.
 * XRGB8888 when a display scans it out, RGBA8888 for the read-back only case. */
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
#define FB_PHYS   ((uintptr_t)CONFIG_GL_TRIANGLE_WINDOW_PHYS)
#define FB_STRIDE (CONFIG_GL_TRIANGLE_WINDOW_STRIDE ? CONFIG_GL_TRIANGLE_WINDOW_STRIDE : FB_W * 4)
#define FB_FORMAT GLES_NATIVE_FORMAT_XRGB8888
#endif
/* the window is drawn y-up when the display scans it bottom row first */
#define WIN_FORMAT (FB_FORMAT | pvr_sample_window_flags())

/* The offline shader table only accepts the GLSL it was built from, byte for
 * byte; the library hands the text out by name (passthrough.vert: position +
 * colour through, interp_color.frag: the interpolated colour). */
static GLuint build_program(void)
{
	GLuint vs = glCreateShader(GL_VERTEX_SHADER), fs = glCreateShader(GL_FRAGMENT_SHADER);
	GLuint prog = glCreateProgram();
	const char *v = gles_offline_shader_source("passthrough.vert");
	const char *f = gles_offline_shader_source("interp_color.frag");
	GLint ok = 0;

	if (!v || !f) {
		LOG_ERR("shader sources missing from the offline table");
		return 0;
	}

	glShaderSource(vs, 1, &v, NULL);
	glCompileShader(vs);
	glShaderSource(fs, 1, &f, NULL);
	glCompileShader(fs);
	glAttachShader(prog, vs);
	glAttachShader(prog, fs);
	glLinkProgram(prog);
	glGetProgramiv(prog, GL_LINK_STATUS, &ok);
	if (!ok) {
		char log[128];

		glGetProgramInfoLog(prog, sizeof(log), NULL, log);
		LOG_ERR("link failed: %s", log);
		return 0;
	}
	return prog;
}

#if CONFIG_GL_TRIANGLE_WINDOW_PHYS == 0
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
	static const float verts[3][8] = {
		/* x, y, z, w,  r, g, b, a */
		{  0.0f,  0.8f, 0.0f, 1.0f, 1.0f, 0.0f, 0.0f, 1.0f },
		{ -0.8f, -0.8f, 0.0f, 1.0f, 0.0f, 1.0f, 0.0f, 1.0f },
		{  0.8f, -0.8f, 0.0f, 1.0f, 0.0f, 0.0f, 1.0f, 1.0f },
	};
	struct gles_native_window win = {
		.phys = FB_PHYS, .width = FB_W, .height = FB_H,
		.stride_bytes = FB_STRIDE, .format = WIN_FORMAT,
	};
	EGLDisplay dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	EGLConfig cfg;
	EGLSurface surf;
	EGLContext ctx;
	EGLint n = 0;
	GLuint prog, vbo;
	uint8_t px[4] = { 0, 0, 0, 0 };

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

	prog = build_program();
	if (!prog) {
		return 1;
	}
	glUseProgram(prog);
	glGenBuffers(1, &vbo);
	glBindBuffer(GL_ARRAY_BUFFER, vbo);
	glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);
	glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE, 32, (const void *)0);
	glVertexAttribPointer(1, 4, GL_FLOAT, GL_FALSE, 32, (const void *)16);
	glEnableVertexAttribArray(0);
	glEnableVertexAttribArray(1);

	glViewport(0, 0, FB_W, FB_H);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClear(GL_COLOR_BUFFER_BIT);
	glDrawArrays(GL_TRIANGLES, 0, 3);
	eglSwapBuffers(dpy, surf);

	/* The centre is (W/2, H/2) in GL rows, the definition the i.MX8MP's
	 * gc_gl_triangle uses; this read (W/2, H/3) until 2026-09-23, so the two
	 * boards' "centre" were different pixels. */
	glReadPixels(FB_W / 2, FB_H / 2, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	LOG_INF("triangle drawn; centre pixel r=%u g=%u b=%u a=%u (GL error 0x%x)",
		px[0], px[1], px[2], px[3], glGetError());
	{
		/* The shared PICTURE record (kind 4), the same words gc_gl_triangle
		 * emits: w1 the centre pixel as glReadPixels returns it read as a
		 * little-endian word, w2 CRC-32/IEEE over the RGBA of the column
		 * samples at y = i*H/24 for i = 3..21 (inside the triangle, so
		 * independent of the clear colour), w3 W<<16 | H. */
		uint8_t col[19 * 4];
		uint32_t w1 = px[0] | px[1] << 8 | px[2] << 16 | (uint32_t)px[3] << 24;

		for (int i = 3; i <= 21; i++) {
			glReadPixels(FB_W / 2, (i * FB_H) / 24, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE,
				     &col[(i - 3) * 4]);
		}
		LOG_INF("gl_triangle: record %08x %08x %08x %08x", 0x594b0400u, w1,
			crc32_ieee(col, sizeof(col)), (uint32_t)(FB_W << 16 | FB_H));
	}
	/* One column of samples, top to bottom: shows which rows the GPU wrote. */
	for (int y = 0; y < FB_H; y += FB_H / 24) {
		glReadPixels(FB_W / 2, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
		LOG_INF("  row %3d: %02x%02x%02x", y, px[0], px[1], px[2]);
	}
	return 0;
}

int main(void)
{
	int rc;

	LOG_INF("gl_triangle: %ux%u window at 0x%lx", FB_W, FB_H, (unsigned long)FB_PHYS);
	pvr_sample_log_boot_time("main()");
#if CONFIG_GL_TRIANGLE_WINDOW_PHYS == 0
	/* Start from a CPU-drawn pattern (dark blue, a white bar at the top) so
	 * the display path can be judged even when the GPU draw fails; the GPU
	 * overwrites it. */
	fill_pattern();
	sys_cache_data_flush_and_invd_range(s_fb, sizeof(s_fb));
#endif
	rc = draw();
	LOG_INF("gl_triangle: %s", rc ? "FAILED" : "done");
	/* Shown even after a failed draw: the pattern tells the display path apart. */
	if (pvr_sample_show_window(FB_PHYS, FB_W, FB_H, FB_STRIDE, WIN_FORMAT) && rc == 0) {
		rc = -EIO;
	}
	pvr_sample_log_boot_time("the first frame");
	return pvr_sample_finish(rc);
}
