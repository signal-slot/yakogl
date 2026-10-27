/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * latch_model.c: the i.MX8MP board layer's show and VSYNC waits on the host.
 *
 * board_imx8mp.c is compiled unchanged against shim/ (Zephyr's kernel calls on
 * POSIX threads, gc_render_alloc on the C heap) and against the simulated
 * LCDIF below: imx8mp_display_set_fb() returns at the next multiple of the
 * period T, as the real one returns when the LCDIF clears SHADOW_LOAD_EN at
 * its next VSYNC. A frame of each application is played in the order its
 * sample calls EGL and the board (the draw is a sleep of D), and the run
 * prints how many panel periods a frame took. That is the number the
 * swap-interval wiring must not double (docs/notes/imx8mp-frame-floor.md,
 * "Waits per swap").
 *
 *   latch_model <app> <osal> [frames]
 *     app   panel    Slint, SINGLE_BUFFER, the panel's size: swap, then show
 *                    (scanned in place)
 *           view     the same with a 1024x600 window; scanned in place when
 *                    built with CONFIG_YAKOGL_BOARD_SCANOUT_VIEW, else copied
 *                    (the SV and SO rows)
 *           cluster  two panel-sized halves, PIPELINED: draw N+1, show N,
 *                    swap N+1 (the pattern of the cluster sample, removed
 *                    2026-10-19; GLES_NATIVE_WINDOW_PIPELINED)
 *           glmark2  800x600, EGL_BACK_BUFFER, swap interval 0: the swap
 *                    shows the frame before (EGL's three buffers), then waits
 *                    0 periods
 *           fifo     the same with swap interval 1 (glmark2 --swap-mode fifo,
 *                    and any BACK window at EGL's default interval)
 *     osal  unwired  src/osal/zephyr_gc.c without .wait_vblank (main before
 *                    this change): EGL's interval loop stops at -ENOTSUP
 *           wired    .wait_vblank calls yakogl_zephyr_wait_vblank
 *
 * Output, one line starting "app=": app, osal, buffers, view, periods/frame
 * (wall time over frames x T), latches/frame (set_fb calls), and in-place
 * draws: cluster frames submitted while the LCDIF still scanned the half
 * they write (the cluster's invariant; expected 0).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "yakogl_board.h"
#include "gc_render.h"
#include "imx8mp_display.h"

#define PANEL_W 1280u
#define PANEL_H 800u

static int64_t s_period_us = 4000; /* T */
static int64_t s_draw_us = 1200;   /* D: the frame's CPU and GPU time */

/* ---- the simulated LCDIF ---------------------------------------------- */
static pthread_mutex_t s_lcdif = PTHREAD_MUTEX_INITIALIZER;
static uintptr_t s_scanned; /* what the LCDIF reads now */
static unsigned long s_latches;

const struct device model_gpu_device;

static void sleep_until_us(int64_t t)
{
	struct timespec ts = { .tv_sec = t / 1000000, .tv_nsec = (t % 1000000) * 1000 };

	while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &ts, NULL) == EINTR) {
	}
}

void imx8mp_display_geometry(uint32_t *width, uint32_t *height)
{
	*width = PANEL_W;
	*height = PANEL_H;
}

int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch)
{
	ARG_UNUSED(pitch);
	pthread_mutex_lock(&s_lcdif);
	s_scanned = fb_phys;
	pthread_mutex_unlock(&s_lcdif);
	return 0;
}

/* the address is taken at the next VSYNC, and the call returns then */
int imx8mp_display_set_fb(uintptr_t fb_phys)
{
	int64_t now = model_now_us();

	sleep_until_us((now / s_period_us + 1) * s_period_us);
	pthread_mutex_lock(&s_lcdif);
	s_scanned = fb_phys;
	s_latches++;
	pthread_mutex_unlock(&s_lcdif);
	return 0;
}

static uintptr_t scanned(void)
{
	uintptr_t p;

	pthread_mutex_lock(&s_lcdif);
	p = s_scanned;
	pthread_mutex_unlock(&s_lcdif);
	return p;
}

/* ---- the rest of what board_imx8mp.c links against --------------------- */
int gc_render_init(const struct device *dev)
{
	ARG_UNUSED(dev);
	return 0;
}

int gc_render_alloc(size_t size, size_t align, struct gc_bo *out)
{
	void *p = aligned_alloc(align, (size + align - 1u) / align * align);

	if (p == NULL) {
		return -ENOMEM;
	}
	memset(p, 0, size);
	*out = (struct gc_bo){ .cpu = p, .phys = (uintptr_t)p, .size = size };
	return 0;
}

void board_show_copied(uint32_t w, uint32_t h, const char *why)
{
	ARG_UNUSED(w);
	ARG_UNUSED(h);
	ARG_UNUSED(why);
}

/* ---- the OSAL and EGL's swap ------------------------------------------- */
int yakogl_zephyr_wait_vblank(int64_t timeout_us); /* board_imx8mp.c */
static bool s_wired;

/* osal::wait_vblank: the op of src/osal/zephyr_gc.c, or none */
static int osal_wait_vblank(int64_t timeout_us)
{
	return s_wired ? yakogl_zephyr_wait_vblank(timeout_us) : -ENOTSUP;
}

/* the tail of eglSwapBuffers (src/gl/egl.rs): the swap-interval loop */
static void swap_interval_loop(int interval)
{
	for (int i = 0; i < interval; i++) {
		if (osal_wait_vblank(100000) == -ENOTSUP) {
			break;
		}
	}
}

static void draw(void)
{
	sleep_until_us(model_now_us() + s_draw_us);
}

static void must_show(const struct board_window *w)
{
	int rc = board_show(w);

	if (rc != 0) {
		fprintf(stderr, "latch_model: board_show = %d\n", rc);
		exit(1);
	}
}

static void must_window(uint32_t w, uint32_t h, struct board_window *out)
{
	int rc = board_window(w, h, out);

	if (rc != 0) {
		fprintf(stderr, "latch_model: board_window(%ux%u) = %d\n", w, h, rc);
		exit(1);
	}
}

/* ---- the applications -------------------------------------------------- */
struct app {
	const char *name;
	int interval;
	void (*setup)(void);
	void (*frame)(void);
};

static struct board_window s_win, s_half[2];
static unsigned long s_inplace_draws;

/* Slint (examples/slint/src/main.c): EGL_SINGLE_BUFFER, the
 * sample shows its window after eglSwapBuffers; interval 1 */
static void slint_panel_setup(void)
{
	must_window(PANEL_W, PANEL_H, &s_win);
}
static void slint_view_setup(void)
{
	must_window(1024u, 600u, &s_win);
}
static void slint_frame(void)
{
	draw();
	swap_interval_loop(1);
	must_show(&s_win);
}

/* the cluster: one window of two panel-sized halves, each its own surface */
static int s_cl_half, s_cl_pending = -1;
static void cluster_setup(void)
{
	struct board_window w;

	must_window(PANEL_W, 2u * PANEL_H, &w);
	for (int k = 0; k < 2; k++) {
		s_half[k] = w;
		s_half[k].h = PANEL_H;
		s_half[k].phys = w.phys + (size_t)k * PANEL_H * w.stride;
		s_half[k].cpu = (uint8_t *)w.cpu + (size_t)k * PANEL_H * w.stride;
	}
}
static void cluster_frame(void)
{
	draw(); /* recorded only: the GPU runs it at the swap */
	if (s_cl_pending >= 0) {
		must_show(&s_half[s_cl_pending]);
	}
	/* the swap submits the frame: the half it writes must be off the panel */
	if (scanned() == s_half[s_cl_half].phys) {
		s_inplace_draws++;
	}
	swap_interval_loop(1);
	s_cl_pending = s_cl_half;
	s_cl_half ^= 1;
}

/* glmark2: EGL_BACK_BUFFER, the swap shows the frame before from EGL's own
 * buffers (src/gl/egl.rs swap_window_buffers) and the sample shows nothing */
#define EGL_BUFS 3
static struct board_window s_egl[EGL_BUFS];
static int s_egl_cur, s_egl_pending = -1;
static void glmark2_setup(void)
{
	must_window(800u, 600u, &s_egl[0]);
	for (int k = 1; k < EGL_BUFS; k++) {
		struct gc_bo bo;

		if (gc_render_alloc((size_t)800u * 4u * 600u, 64, &bo) != 0) {
			exit(1);
		}
		s_egl[k] = s_egl[0];
		s_egl[k].phys = (uintptr_t)bo.phys;
		s_egl[k].cpu = bo.cpu;
	}
}
static void egl_back_frame(int interval)
{
	draw();
	if (s_egl_pending >= 0) {
		must_show(&s_egl[s_egl_pending]);
	}
	s_egl_pending = s_egl_cur;
	s_egl_cur = (s_egl_cur + 1) % EGL_BUFS;
	swap_interval_loop(interval);
}
static void glmark2_frame(void)
{
	egl_back_frame(0);
}
static void fifo_frame(void)
{
	egl_back_frame(1);
}

static const struct app s_apps[] = {
	{ "panel", 1, slint_panel_setup, slint_frame },
	{ "view", 1, slint_view_setup, slint_frame },
	{ "cluster", 1, cluster_setup, cluster_frame },
	{ "glmark2", 0, glmark2_setup, glmark2_frame },
	{ "fifo", 1, glmark2_setup, fifo_frame },
};

int main(int argc, char **argv)
{
	const struct app *a = NULL;
	int frames = 60;
	int64_t t0;
	unsigned long l0;

	if (argc < 3) {
		fprintf(stderr, "usage: latch_model <panel|view|cluster|glmark2|fifo> <unwired|wired> [frames]\n");
		return 2;
	}
	for (size_t i = 0; i < ARRAY_SIZE(s_apps); i++) {
		if (strcmp(argv[1], s_apps[i].name) == 0) {
			a = &s_apps[i];
		}
	}
	if (a == NULL || (strcmp(argv[2], "wired") != 0 && strcmp(argv[2], "unwired") != 0)) {
		fprintf(stderr, "latch_model: unknown app or osal\n");
		return 2;
	}
	s_wired = strcmp(argv[2], "wired") == 0;
	if (argc > 3) {
		frames = atoi(argv[3]);
	}
	a->setup();
	/* warm up: the first show turns the panel on, the next ones fill the
	 * screen buffers */
	for (int f = 0; f < 6; f++) {
		a->frame();
	}
	s_inplace_draws = 0;
	pthread_mutex_lock(&s_lcdif);
	l0 = s_latches;
	pthread_mutex_unlock(&s_lcdif);
	t0 = model_now_us();
	for (int f = 0; f < frames; f++) {
		a->frame();
	}
	const double periods = (double)(model_now_us() - t0) / (double)s_period_us / frames;

	pthread_mutex_lock(&s_lcdif);
	const double latches = (double)(s_latches - l0) / frames;

	pthread_mutex_unlock(&s_lcdif);
	printf("app=%s osal=%s buffers=%d view=%d interval=%d periods/frame=%.2f latches/frame=%.2f inplace-draws=%lu\n",
	       a->name, argv[2], CONFIG_YAKOGL_BOARD_IMX8MP_SCREEN_BUFFERS,
	       IS_ENABLED(CONFIG_YAKOGL_BOARD_SCANOUT_VIEW), a->interval, periods, latches,
	       s_inplace_draws);
	return 0;
}
