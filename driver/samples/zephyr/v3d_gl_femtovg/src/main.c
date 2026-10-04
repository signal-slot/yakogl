/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_gl_femtovg: the Raspberry Pi 4 sibling of gl_femtovg -- one FemtoVG
 * frame (fvg_demo's fvg_draw_frame, through gl_femtovg's Rust staticlib),
 * drawn by YakoGL's GL core, its in-process compiler and its V3D backend,
 * with every pixel of the window read back and judged against the scene.
 *
 * The board support is v3d_gl_triangle's: the firmware framebuffer is the
 * console (top half) and the EGL window (bottom half, 1280x360); the
 * watchdog returns the board to Linux.
 *
 * Two steps, each with its own verdict:
 *
 * 1. The programs. femtovg compiles all 30 of its programs when the canvas
 *    is created (renderer/opengl.rs, new_from_context), whether a frame uses
 *    them or not, and fvg_draw_frame then returns 1 with no reason. So this
 *    sample first builds the same 30 programs from femtovg's own shader text
 *    (fvg_programs.h) and prints each program that
 *    does not link with its info log, a histogram keyed on the whole log,
 *    and the program femtovg stops at: the first in its order that fails.
 *
 * 2. The frame. fvg_draw_frame draws fvg_demo's scene (the constants below
 *    mirror fvg_demo/src/lib.rs): a clear to rgb(24,24,32), a solid rounded
 *    rect, a vertical linear gradient rect and a stroked polyline. Every
 *    pixel of the window is put in one class by its position (canvas
 *    coordinates: y down, which is the buffer's row order; pixel centres):
 *
 *      rrect    at least FILL_MARGIN inside the rounded rect: rgb(80,160,240)
 *      grad     at least FILL_MARGIN inside the gradient rect: the colour
 *               mixed at the pixel centre, within GRAD_TOL (femtovg dithers
 *               gradients by +-0.5 of an 8-bit step, main-fs.glsl)
 *      stroke   within STROKE_CORE of a segment's centre line and at least
 *               STROKE_END_GAP from its ends: rgb(120,240,160)
 *      bg       clear of every shape by the margins below: rgb(24,24,32)
 *      edge     everything else (anti-aliased fringes, joins): counted only
 *
 *    and alpha is 255 in every checked class. The row above the window must
 *    keep the value the CPU wrote, and four glReadPixels reads are compared
 *    with memory.
 *
 * Every step prints its raw result; the verdict line names what failed.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <errno.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include "osal_internal.h" /* struct osal_zephyr_v3d_init */
#include "rpi_mbox.h"
#include "fbcon.h"
#include "rpi_wdog.h"
#include "fvg_programs.h" /* step 1 */

#ifndef V3D_BUILD
#define V3D_BUILD "unknown"
#endif

#define FB_W 1280
#define FB_H 720

#define SENTINEL 0xffff00ffu /* magenta, written by the CPU beforehand */
#define GUARD    0xff0000ffu /* red, the row above the window */

/* ---- the scene: fvg_demo/src/lib.rs, fvg_draw_frame ----------------------- */

#define RGBA8(r, g, b) (0xff000000u | (uint32_t)(b) << 16 | (uint32_t)(g) << 8 | (uint32_t)(r))
#define FVG_BG     RGBA8(24, 24, 32)
#define FVG_RRECT  RGBA8(80, 160, 240)
#define FVG_STROKE RGBA8(120, 240, 160)

/* The console hook before fbcon's (the UART) and fbcon's own. fbcon_attach()
 * replaces the printk hook, so without this the UART gets nothing after it;
 * with it every line goes to both, and the host's UART log has what the screen
 * has (v3d_deqp does the same). */
static printk_hook_fn_t uart_out, fbcon_out;
/* The UART is polled: every character a timed step prints waits for it
 * (86.8 us at 115200 8N1). The time is added up here, so each timed step
 * also reports itself without it (uart_us). */
static uint64_t uart_cycles;

static int console_tee(int c)
{
	if (uart_out) {
		uint32_t t0 = k_cycle_get_32();
		uart_out(c);
		uart_cycles += k_cycle_get_32() - t0;
	}
	return fbcon_out ? fbcon_out(c) : c;
}

static int64_t uart_us(void)
{
	return (int64_t)k_cyc_to_us_floor64(uart_cycles);
}

static const float grad_top[3] = { 240, 80, 120 }, grad_bot[3] = { 250, 210, 90 };
#define FVG_RRECT_RADIUS 16.0f
#define FVG_STROKE_WIDTH 6.0f
/* the polyline's points, as fractions of the window */
static const float stroke_pts[5][2] = {
	{ 0.10f, 0.75f }, { 0.30f, 0.55f }, { 0.50f, 0.80f }, { 0.70f, 0.55f }, { 0.90f, 0.75f },
};

/* femtovg's fringe is 1 px (fringe_width = 1 / dpi): the anti-aliased band of
 * a fill is within 0.5 px of the path, of a stroke within 0.5 px of its outer
 * edge; the margins below keep every checked pixel out of it with room. */
#define FILL_MARGIN    2.0f
#define STROKE_CORE    1.5f  /* strokeMask is 1 up to 2.5 px off the centre line */
#define STROKE_END_GAP 12.0f /* joins (miter) and butt caps stay out of the class */
#define BG_STROKE_GAP  (FVG_STROKE_WIDTH / 2 + 3.0f)
#define BG_JOIN_GAP    24.0f /* a miter tip at these angles is ~3.1 px; generous */
#define EXACT_TOL      1u
#define GRAD_TOL       2u

/* Why the run failed; NULL after a pass. */
static const char *verdict_what = "the run did not reach the check";
static char verdict_buf[200];

static int clk_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	int ret = rpi_clock_state(RPI_FW_CLK_V3D, on);
	printk("  fw clock 5 %s: %d%s\n", on ? "on" : "off", ret, ret ? " (not fatal)" : "");
	return 0; /* as v3d_triangle: which clock the toggle needs is a board question */
}

static struct osal_zephyr_v3d_init plat = { .clock_set = clk_set };

/* ---- step 2: the frame and its check ------------------------------------- */

/* fvg_draw_frame (fvg_demo, re-exported by gl_femtovg/rust): one frame through
 * the current context; 0 on success, 1 when the renderer (its programs) could
 * not be created, 2 when the canvas could not. */
int fvg_draw_frame(const void *(*get_proc)(const char *name), uint32_t width, uint32_t height);

static const void *fvg_get_proc(const char *name)
{
	return (const void *)eglGetProcAddress(name);
}

/* femtovg's sdroundrect (main-fs.glsl) for a rect at (x, y): negative inside */
static float sd_rrect(float px, float py, float x, float y, float w, float h, float r)
{
	float cx = x + w * 0.5f, cy = y + h * 0.5f;
	float dx = fabsf(px - cx) - (w * 0.5f - r), dy = fabsf(py - cy) - (h * 0.5f - r);
	float ox = dx > 0 ? dx : 0, oy = dy > 0 ? dy : 0;
	float in = dx > dy ? dx : dy;
	return (in < 0 ? in : 0) + sqrtf(ox * ox + oy * oy) - r;
}

/* distance from p to segment ab, and where along it (0..len) the foot is */
static float seg_dist(float px, float py, float ax, float ay, float bx, float by, float *along,
		      float *len)
{
	float dx = bx - ax, dy = by - ay;
	float l2 = dx * dx + dy * dy;
	float t = ((px - ax) * dx + (py - ay) * dy) / l2;
	t = t < 0 ? 0 : t > 1 ? 1 : t;
	float fx = ax + t * dx - px, fy = ay + t * dy - py;
	*len = sqrtf(l2);
	*along = t * *len;
	return sqrtf(fx * fx + fy * fy);
}

static uint32_t max_diff(uint32_t a, uint32_t b)
{
	uint32_t m = 0;
	for (int c = 0; c < 4; c++) {
		int d = (int)((a >> (8 * c)) & 0xff) - (int)((b >> (8 * c)) & 0xff);
		uint32_t ad = (uint32_t)(d < 0 ? -d : d);
		if (ad > m)
			m = ad;
	}
	return m;
}

/* The two fringe classes check the anti-aliased edges, which the others
 * leave out: a pixel centre 0.2-0.8 of the way through a 1-px fringe must
 * be drawn partly, strictly between the background and the shape's colour
 * (within FRINGE_TOL of that range, and more than FRINGE_TOL from the
 * background). The stroke's fringe is drawn by femtovg's second stroke pass
 * (stencil EQUAL 0 after a first pass that discards and increments the
 * stencil); the rounded rect's by a plain convex fill, the control. The
 * first Slint frame on the board lost a stroke fringe (the track point). */
enum cls { C_RRECT, C_GRAD, C_STROKE, C_BG, C_SFRINGE, C_RFRINGE, C_EDGE, C_N };
static const char *const cls_name[C_N] = { "rrect", "grad", "stroke", "bg", "sfringe", "rfringe", "edge" };
#define FRINGE_TOL 2u

struct result {
	uint32_t total, n[C_N], bad[C_N], worst[C_N];
	bool have_first;
	enum cls first_cls;
	uint32_t first_x, first_y, first_got, first_want;
};

static void check_window(const uint8_t *win, uint32_t pitch, uint32_t w, uint32_t h,
			 struct result *r)
{
	const float W = (float)w, H = (float)h;
	/* fvg_draw_frame's geometry */
	const float rx = W * 0.08f, ry = H * 0.10f, rw = W * 0.36f, rh = H * 0.30f;
	const float gx = W * 0.54f, gy = H * 0.10f, gw = W * 0.38f, gh = H * 0.30f;
	float sp[5][2];
	for (int i = 0; i < 5; i++) {
		sp[i][0] = stroke_pts[i][0] * W;
		sp[i][1] = stroke_pts[i][1] * H;
	}

	memset(r, 0, sizeof(*r));
	for (uint32_t row = 0; row < h; row++) {
		const uint32_t *p = (const uint32_t *)(win + (size_t)row * pitch);
		float y = (float)row + 0.5f;
		for (uint32_t col = 0; col < w; col++) {
			float x = (float)col + 0.5f;
			uint32_t got = p[col], want = 0, tol = EXACT_TOL;
			enum cls c = C_EDGE;

			float d_rrect = sd_rrect(x, y, rx, ry, rw, rh, FVG_RRECT_RADIUS);
			float gdx = fmaxf(gx - x, x - (gx + gw)), gdy = fmaxf(gy - y, y - (gy + gh));
			float d_grad = fmaxf(gdx, gdy); /* box distance, negative inside */
			float d_line = 1e30f, d_join = 1e30f;
			bool core = false, sfringe = false;
			for (int s = 0; s < 4; s++) {
				float along, len;
				float d = seg_dist(x, y, sp[s][0], sp[s][1], sp[s + 1][0], sp[s + 1][1],
						   &along, &len);
				if (d < d_line)
					d_line = d;
				if (d <= STROKE_CORE && along >= STROKE_END_GAP &&
				    along <= len - STROKE_END_GAP)
					core = true;
				/* femtovg: alpha = 3.5 - d over the outer pixel of a width-6
				 * stroke with a 1-px fringe; 2.7..3.3 is alpha 0.8..0.2 */
				if (d >= 2.7f && d <= 3.3f && along >= STROKE_END_GAP &&
				    along <= len - STROKE_END_GAP)
					sfringe = true;
			}
			for (int i = 0; i < 5; i++) {
				float jx = x - sp[i][0], jy = y - sp[i][1];
				float d = sqrtf(jx * jx + jy * jy);
				if (d < d_join)
					d_join = d;
			}

			if (d_rrect <= -FILL_MARGIN && d_line > BG_STROKE_GAP) {
				c = C_RRECT;
				want = FVG_RRECT;
			} else if (d_grad <= -FILL_MARGIN && d_line > BG_STROKE_GAP) {
				float t = (y - gy) / gh;
				t = t < 0 ? 0 : t > 1 ? 1 : t;
				want = 0xff000000u;
				for (int k = 0; k < 3; k++) {
					float v = grad_top[k] + t * (grad_bot[k] - grad_top[k]);
					want |= (uint32_t)(v + 0.5f) << (8 * k);
				}
				c = C_GRAD;
				tol = GRAD_TOL;
			} else if (core && d_rrect > FILL_MARGIN && d_grad > FILL_MARGIN) {
				c = C_STROKE;
				want = FVG_STROKE;
			} else if (d_rrect >= FILL_MARGIN && d_grad >= FILL_MARGIN &&
				   d_line >= BG_STROKE_GAP && d_join >= BG_JOIN_GAP) {
				c = C_BG;
				want = FVG_BG;
			} else if (sfringe && d_line >= 2.7f && d_join >= STROKE_END_GAP &&
				   d_rrect > FILL_MARGIN && d_grad > FILL_MARGIN) {
				c = C_SFRINGE;
				want = FVG_STROKE;
			} else if (fabsf(d_rrect) <= 0.3f && d_line > BG_STROKE_GAP) {
				/* the fill's fringe is centred on the path: alpha 0.2..0.8 */
				c = C_RFRINGE;
				want = FVG_RRECT;
			}

			r->total++;
			r->n[c]++;
			if (c == C_EDGE)
				continue;
			if (c == C_SFRINGE || c == C_RFRINGE) {
				/* drawn partly: inside [bg, colour] per channel, not bg */
				bool inside = (got >> 24) == 0xff;
				for (int k = 0; k < 3; k++) {
					int g = (int)((got >> (8 * k)) & 0xff), b = (int)((FVG_BG >> (8 * k)) & 0xff),
					    f = (int)((want >> (8 * k)) & 0xff);
					int lo = (b < f ? b : f) - (int)FRINGE_TOL, hi = (b > f ? b : f) + (int)FRINGE_TOL;
					inside &= g >= lo && g <= hi;
				}
				uint32_t from_bg = max_diff(got, FVG_BG);
				if (from_bg > r->worst[c])
					r->worst[c] = from_bg; /* here: the largest distance from bg */
				if (!inside || from_bg <= FRINGE_TOL) {
					r->bad[c]++;
					if (!r->have_first) {
						r->have_first = true;
						r->first_cls = c;
						r->first_x = col;
						r->first_y = row;
						r->first_got = got;
						r->first_want = want;
					}
				}
				continue;
			}
			uint32_t md = max_diff(got, want);
			if (md > r->worst[c])
				r->worst[c] = md;
			if (md > tol || (got >> 24) != 0xff) {
				r->bad[c]++;
				if (!r->have_first) {
					r->have_first = true;
					r->first_cls = c;
					r->first_x = col;
					r->first_y = row;
					r->first_got = got;
					r->first_want = want;
				}
			}
		}
	}
}

/* glReadPixels at GL (x, y) against the window memory at row h - 1 - y. */
static bool read_matches(const uint8_t *win, uint32_t pitch, uint32_t h, GLint x, GLint y)
{
	uint8_t px[4] = { 0 };
	glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
	uint32_t via_gl = (uint32_t)px[0] | (uint32_t)px[1] << 8 | (uint32_t)px[2] << 16 |
			  (uint32_t)px[3] << 24;
	uint32_t mem = ((const uint32_t *)(win + (size_t)(h - 1 - (uint32_t)y) * pitch))[x];
	printk("  glReadPixels(%4d,%3d) = 0x%08x, memory 0x%08x%s\n", x, y, via_gl, mem,
	       via_gl == mem ? "" : "  DIFFERENT");
	return via_gl == mem;
}

/* ---- the run --------------------------------------------------------------- */

static struct fvg_survey sv;

static bool egl_up(uint32_t window_phys, uint32_t w, uint32_t h, uint32_t pitch, EGLDisplay *dpy_out,
		   EGLSurface *surf_out)
{
	/* femtovg fills concave paths and strokes through the stencil */
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window win;
	win = (struct gles_native_window){
		.phys = window_phys, .width = w, .height = h, .stride_bytes = pitch,
		.format = GLES_NATIVE_FORMAT_RGBA8888,
	};
	EGLDisplay dpy = eglGetDisplay((EGLNativeDisplayType)&plat);
	EGLConfig cfg;
	EGLint n = 0;

	if (!eglInitialize(dpy, NULL, NULL)) {
		printk("  eglInitialize: 0x%x\n", eglGetError());
		verdict_what = "EGL did not initialise (the V3D, the OSAL or the backend)";
		return false;
	}
	if (!eglChooseConfig(dpy, cfg_attribs, &cfg, 1, &n) || n != 1) {
		printk("  eglChooseConfig: 0x%x\n", eglGetError());
		verdict_what = "no EGL config matches an RGBA8888 window with stencil";
		return false;
	}
	/* each call checked on its own: eglGetError reports the LAST call only */
	EGLSurface surf = eglCreateWindowSurface(dpy, cfg, &win, NULL);
	if (surf == EGL_NO_SURFACE) {
		printk("  eglCreateWindowSurface failed: EGL error 0x%x\n", eglGetError());
		verdict_what = "the EGL window surface was not created";
		return false;
	}
	EGLContext ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (ctx == EGL_NO_CONTEXT) {
		printk("  eglCreateContext failed: EGL error 0x%x\n", eglGetError());
		verdict_what = "the EGL context was not created";
		return false;
	}
	if (!eglMakeCurrent(dpy, surf, surf, ctx)) {
		printk("  eglMakeCurrent failed: EGL error 0x%x\n", eglGetError());
		verdict_what = "the EGL context could not be made current";
		return false;
	}
	printk("  GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	*dpy_out = dpy;
	*surf_out = surf;
	return true;
}

static void run(void)
{
	struct rpi_fb fb;
	printk("v3d_gl_femtovg: start (the UART only until the screen console is up)\n");
	int ret = rpi_mbox_init();
	if (ret == 0)
		ret = rpi_fb_alloc(FB_W, FB_H, &fb);
	if (ret)
		return; /* no screen: nothing can be reported */
	uint32_t half = fb.height / 2;
	uart_out = __printk_get_hook();
	fbcon_attach(fb.cpu, fb.width, fb.pitch, half, 1);
	fbcon_out = __printk_get_hook();
	__printk_hook_install(console_tee);
	printk("v3d_gl_femtovg: build %s\n", V3D_BUILD);
#ifdef V3D_NO_RPI_WDOG
	printk("board: NO watchdog (a QEMU build, V3D_RPI_WDOG=OFF)\n");
#else
	rpi_wdog_announce();
#endif
	printk("fb %ux%u pitch %u phys 0x%08x\n", fb.width, fb.height, fb.pitch, fb.phys);

	uint32_t hz = 0, max_hz = 0;
	if (rpi_clock_max_rate(RPI_FW_CLK_V3D, &max_hz) == 0 && max_hz &&
	    rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz != max_hz) {
		ret = rpi_clock_set_rate(RPI_FW_CLK_V3D, max_hz);
		printk("V3D clock %u -> %u Hz (%d)\n", hz, max_hz, ret);
	}

	uint32_t w = fb.width, h = fb.height - half;
	uint8_t *win = fb.cpu + (size_t)half * fb.pitch;
	for (uint32_t y = 0; y < h; y++) {
		uint32_t *p = (uint32_t *)(win + (size_t)y * fb.pitch);
		for (uint32_t x = 0; x < w; x++)
			p[x] = SENTINEL;
	}
	uint32_t *guard = (uint32_t *)(fb.cpu + (size_t)(half - 1) * fb.pitch);
	for (uint32_t x = 0; x < fb.width; x++)
		guard[x] = GUARD;

	EGLDisplay dpy;
	EGLSurface surf;
	if (!egl_up(fb.phys + half * fb.pitch, w, h, fb.pitch, &dpy, &surf))
		return;

	/* step 1 */
	printk("progress: EGL up, survey start\n");
	int64_t u0 = uart_us();
	fvg_survey_run(&sv);
	int64_t survey_uart_us = uart_us() - u0;
	fvg_survey_print(&sv);

	/* step 2: femtovg sets its own viewport (set_size) */
	int64_t t0 = k_uptime_get();
	u0 = uart_us();
	printk("progress: frame start\n");
	int rc = fvg_draw_frame(fvg_get_proc, w, h);
	printk("progress: frame drawn (rc %d)\n", rc);
	GLenum err_draw = glGetError();
	if (rc != 0) {
		fvg_keep_print("after fvg_draw_frame failed (its canvas already dropped)");
		printk("fvg_draw_frame: %d (GL error 0x%x)\n", rc, err_draw);
		snprintf(verdict_buf, sizeof(verdict_buf), "fvg_draw_frame %d: %s", rc,
			 rc == 1 && sv.failed ? "femtovg's programs do not all link (above)" :
			 rc == 1 ? "the renderer did not start, yet every program linked here" :
				   "the canvas could not be created");
		verdict_what = verdict_buf;
		return;
	}
	/* fvg_demo's canvas and renderer are locals of fvg_draw_frame: its 30
	 * programs are already deleted here, so this is what stays resident (the
	 * backend's clear program); the peak includes the canvas. */
	fvg_keep_print("after fvg_draw_frame (its canvas already dropped)");
	/* submits the job and waits for it */
	printk("progress: swap start\n");
	eglSwapBuffers(dpy, surf);
	GLenum err = glGetError();
	printk("progress: swap done\n");
	int64_t frame_ms = k_uptime_get() - t0, frame_uart_us = uart_us() - u0;
	printk("frame: %lld ms, GL error 0x%x after the draw, 0x%x after the swap\n",
	       (long long)frame_ms, err_draw, err);
	printk("without the UART: survey %lld ms, frame %lld ms (the UART took %lld, %lld ms)\n",
	       (long long)(sv.ms - survey_uart_us / 1000), (long long)(frame_ms - frame_uart_us / 1000),
	       (long long)(survey_uart_us / 1000), (long long)(frame_uart_us / 1000));
	if (err_draw != GL_NO_ERROR || err != GL_NO_ERROR) {
		verdict_what = "the GL reported an error during the frame (a refused draw logs its reason above)";
		return;
	}

	struct result r;
	check_window(win, fb.pitch, w, h, &r);
	uint32_t guard_bad = 0;
	for (uint32_t x = 0; x < fb.width; x++)
		guard_bad += guard[x] != GUARD;
	bool reads = read_matches(win, fb.pitch, h, 1, 1) &&
		     read_matches(win, fb.pitch, h, (GLint)(w * 26 / 100), (GLint)(h * 75 / 100)) &&
		     read_matches(win, fb.pitch, h, (GLint)(w * 73 / 100), (GLint)(h * 75 / 100)) &&
		     read_matches(win, fb.pitch, h, (GLint)(w * 40 / 100), (GLint)(h * 35 / 100));

	printk("window %u px:", r.total);
	for (int c = 0; c < C_N; c++)
		printk(" %s %u", cls_name[c], r.n[c]);
	printk("\n");
	for (int c = 0; c < C_SFRINGE; c++)
		printk("  %-6s wrong %u (largest channel diff %u)\n", cls_name[c], r.bad[c], r.worst[c]);
	for (int c = C_SFRINGE; c < C_EDGE; c++)
		printk("  %-7s not drawn partly %u of %u (largest distance from bg %u)\n", cls_name[c], r.bad[c],
		       r.n[c], r.worst[c]);
	if (r.have_first)
		printk("  first wrong (%s) at %u,%u: 0x%08x, expected 0x%08x\n", cls_name[r.first_cls],
		       r.first_x, r.first_y, r.first_got, r.first_want);
	printk("  guard row %u of %u changed\n", guard_bad, fb.width);

	uint32_t sum = 0;
	bool every_class = true;
	for (int c = 0; c < C_N; c++) {
		sum += r.n[c];
		every_class &= r.n[c] > 0;
	}
	verdict_what = sum != r.total || r.total != w * h || !every_class ? "the pixel classes do not add up" :
		       r.bad[C_BG]     ? "background not the clear colour" :
		       r.bad[C_RRECT]  ? "rounded rect colour" :
		       r.bad[C_GRAD]   ? "gradient colour" :
		       r.bad[C_STROKE] ? "stroke colour" :
		       r.bad[C_SFRINGE] ? "the stroke's anti-aliased fringe is not drawn partly" :
		       r.bad[C_RFRINGE] ? "the rounded rect's anti-aliased fringe is not drawn partly" :
		       guard_bad       ? "the guard row changed" :
		       !reads          ? "glReadPixels differs from memory" :
					 NULL;
}

int main(void)
{
	run();
	/* The console scrolls: repeat the build, the program count and the
	 * verdict together so one photo carries all three. */
	printk("v3d_gl_femtovg: build %s\n", V3D_BUILD);
	if (sv.built + sv.failed)
		fvg_survey_print(&sv);
	printk("v3d_gl_femtovg: %s (%s)\n", verdict_what ? "FAIL" : "PASS",
	       verdict_what ? verdict_what : "a FemtoVG frame through YakoGL, every pixel read back");
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
