/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * slint: the one Slint sample, the same source on the Raspberry Pi 4 (V3D),
 * the Verdin AM62P (PowerVR) and the Verdin iMX8M Plus (GC7000)
 * (driver/docs/one-method.md, section 5, step 1). The board is chosen by
 * the Zephyr board only; everything that differs per board is behind
 * yakogl_board.h.
 *
 * This file is the thin part: the window from the board layer, EGL/GL
 * through YakoGL, the swaps, the screen and the records. The frames, the
 * vehicle and the check are Rust (rust/src/lib.rs, check/src/lib.rs):
 *
 *   board_window(1024, 600 + guard rows)  sentinel window, guard rows
 *   EGL up (RGBA8888 + stencil, ES 2)     the window's 1024x600 only
 *   slint_sample_first_frame()            demo_vehicle at t = 0, 2 frames
 *   eglSwapBuffers, board_show()
 *   slint_sample_check()                  the one check (5 points at
 *                                         tolerance 1, sentinel, alpha,
 *                                         guard rows, glReadPixels)
 *   CONFIG_YAKOGL_BOARD_ANIMATE_S of demo_vehicle frames, each shown
 *   board_result(FINGERPRINT), board_result_tag(PROBES)
 *   board_finish()
 *
 * The records are stored after the animation, not after the check: a run
 * that dies in the animation must not leave a PASS as its last record, and
 * board_progress() is dropped once a result was stored. PROBES is stored
 * last because the last record of a run is its verdict; FINGERPRINT is a
 * fingerprint, not a verdict. PROBES' status is the whole run's (the check
 * and the animation).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_heap.h>
#include <errno.h>
#include <stdint.h>
#include <string.h>

#include <EGL/egl.h>
#include <GLES2/gl2.h>

#include <yakogl_board.h>
#include <yakogl_record.h>
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
/* SafeUI route 3 on the Verdin iMX8MP direct image: the M7 shows the frames
 * and publishes the vehicle state (docs/notes/imx8mp-direct-boot.md) */
#include <board_imx8mp_route3.h>
#endif

/* The window (Kconfig SLINT_WINDOW_*): the AM62P's 1024x600 by default,
 * which the other panels also fit; a board sets its panel's size in its conf. */
#define WIN_W CONFIG_SLINT_WINDOW_WIDTH
#define WIN_H CONFIG_SLINT_WINDOW_HEIGHT
/* check/src/lib.rs: GUARD_ROWS, SENTINEL */
#define GUARD_ROWS 8u
#define SENTINEL   0x00000001u
/* The guard rows as they were before the first frame: read, never painted
 * (in a view they are on the screen, CONFIG_YAKOGL_BOARD_SCANOUT_VIEW). */
static uint32_t s_guard_before[GUARD_ROWS * WIN_W];

/* Where a run that died stopped (kind PROGRESS, YK_RECORD_WHERE_STAGE | n). */
enum stage {
	STAGE_WINDOW = 1,
	STAGE_EGL,
	STAGE_FIRST_FRAME,
	STAGE_SWAPPED,
	STAGE_CHECKED,
	STAGE_ANIMATING,
	STAGE_ANIMATED,
};

/* ---- what the Rust archive needs from C ---------------------------------- */

/* The UI heap (Slint, femtovg, the glyph textures): gl_slint's 48 MiB, a
 * sys_heap over a static arena. */
#define UI_HEAP_BYTES (48u * 1024u * 1024u)
static uint8_t s_ui_heap_mem[UI_HEAP_BYTES] __noinit __aligned(64);
static struct sys_heap s_ui_heap;

void *slint_heap_alloc(size_t alignment, size_t size)
{
	return sys_heap_aligned_alloc(&s_ui_heap, alignment, size);
}

void slint_heap_free(void *ptr)
{
	sys_heap_free(&s_ui_heap, ptr);
}

/* rust/src/lib.rs */
typedef const void *(*slint_get_proc_fn)(const char *name);
struct slint_sample_result {
	uint32_t status; /* YK_RECORD_STATUS_PASS / _FAIL */
	uint32_t probes[3];
	uint32_t fingerprint[3];
};
BUILD_ASSERT(sizeof(struct slint_sample_result) == 28, "rust/src/lib.rs SampleResult");
const char *slint_sample_build_inputs(void);
int slint_sample_first_frame(slint_get_proc_fn get_proc, uint32_t w, uint32_t h);
int slint_sample_animate_frame(slint_get_proc_fn get_proc, uint32_t w, uint32_t h, uint32_t t_ms,
			       uint32_t frame);
int slint_sample_check(const uint32_t *cpu, uint32_t w, uint32_t h, uint32_t stride, uint32_t guard_rows,
		       const uint32_t *guard_before, uint32_t format, struct slint_sample_result *out);
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
void slint_sample_route3(bool on);
int slint_sample_animate_frame_m7(slint_get_proc_fn get_proc, uint32_t w, uint32_t h, uint32_t t_ms,
				  uint32_t frame, const struct cluster_vehicle_state *v);

/* The frame's vehicle in route 3: the M7's newest (the last one read when a
 * read races its write; zeros before its first), or NULL for the demo drive
 * when the M7 is absent (the picture then moves without the safe core's
 * elements, which is how the panel says the M7 did not come up). */
static const struct cluster_vehicle_state *route3_vehicle(void)
{
	static struct cluster_vehicle_state last;
	static bool have;
	struct cluster_vehicle_state v;
	int64_t t0 = k_uptime_get();

	/* Until the M7's first publish, wait for it (bounded by the time the
	 * board layer gives the M7 to come up): a frame drawn from no data
	 * would put zeros under the M7's own values for a frame or two
	 * (2026-10-22, the first frames wrong). */
	while (!have && board_route3_state() != BOARD_ROUTE3_ABSENT &&
	       k_uptime_get() - t0 < CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_M7_WAIT_MS) {
		if (board_route3_vehicle(&v)) {
			last = v;
			have = true;
			printk("slint: route 3: the M7's vehicle state after %lld ms\n",
			       (long long)(k_uptime_get() - t0));
			break;
		}
		board_watchdog_feed();
		k_sleep(K_MSEC(1));
	}
	if (board_route3_state() == BOARD_ROUTE3_ABSENT)
		return NULL;
	if (board_route3_vehicle(&v)) {
		last = v;
		have = true;
	}
	return &last;
}
#endif

static const void *get_proc(const char *name)
{
	return (const void *)eglGetProcAddress(name);
}

/* ---- the run ------------------------------------------------------------- */

/* The windows (Kconfig SLINT_WINDOW_BUFFERS): one on a board whose board
 * layer copies or pans; two or three where the screen flips to a window by
 * address and in place (the i.MX8MP at the panel's size), so that the frame
 * being drawn is never the one scanned. s_win and s_surf are the current
 * ones; next_window() advances before each animated frame. */
#define WINDOWS CONFIG_SLINT_WINDOW_BUFFERS
static struct board_window s_wins[WINDOWS];
static EGLSurface s_surfs[WINDOWS];
static unsigned int s_cur;
static struct board_window s_win; /* the window as EGL and the screen see it */
static EGLDisplay s_dpy;
static EGLSurface s_surf;
static EGLContext s_ctx;
static struct slint_sample_result s_result;
static bool s_checked;

/* Each EGL call is checked on its own: eglGetError reports the LAST call. */
static int egl_fail(const char *call)
{
	printk("slint: %s failed: EGL error 0x%x\n", call, eglGetError());
	return -EIO;
}

static int egl_up(void)
{
	/* EGL_SINGLE_BUFFER: this sample shows the window itself (board_show
	 * after the swap) and reads it back as memory, so the window is
	 * rendered in place and complete at every swap; the default,
	 * EGL_BACK_BUFFER, would have EGL show its own buffers (gles_osal.h,
	 * present). */
	static const EGLint single_buffer[] = { EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER, EGL_NONE };
	/* femtovg fills concave paths and strokes through the stencil */
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window nw[WINDOWS];
	EGLConfig cfg;
	EGLint n = 0;

	for (unsigned int i = 0; i < WINDOWS; i++) {
		nw[i] = (struct gles_native_window){
			.phys = s_wins[i].phys, .width = s_wins[i].w, .height = s_wins[i].h,
			.stride_bytes = s_wins[i].stride, .format = s_wins[i].format,
		};
	}
	/* every board: the Pi 4 board layer gives the V3D OSAL its clock
	 * switch through the OSAL's default-platform hook (db659a88) */
	s_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_dpy, NULL, NULL))
		return egl_fail("eglInitialize");
	if (!eglChooseConfig(s_dpy, cfg_attribs, &cfg, 1, &n))
		return egl_fail("eglChooseConfig");
	if (n != 1) {
		printk("slint: eglChooseConfig: %d configs for RGBA8888 + stencil 8\n", n);
		return -ENOENT;
	}
	for (unsigned int i = 0; i < WINDOWS; i++) {
		s_surfs[i] = eglCreateWindowSurface(s_dpy, cfg, &nw[i], single_buffer);
		if (s_surfs[i] == EGL_NO_SURFACE)
			return egl_fail("eglCreateWindowSurface");
	}
	s_surf = s_surfs[0];
	s_ctx = eglCreateContext(s_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (s_ctx == EGL_NO_CONTEXT)
		return egl_fail("eglCreateContext");
	if (!eglMakeCurrent(s_dpy, s_surf, s_surf, s_ctx))
		return egl_fail("eglMakeCurrent");
	printk("slint: GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	return 0;
}

/* eglSwapBuffers submits the frame and waits for it; a GL error during the
 * frame is a failure (a refused draw logs its reason). */
static int swap(void)
{
	if (!eglSwapBuffers(s_dpy, s_surf))
		return egl_fail("eglSwapBuffers");
	GLenum err = glGetError();
	if (err != GL_NO_ERROR) {
		printk("slint: GL error 0x%x during the frame\n", err);
		return -EIO;
	}
	return 0;
}

static int window_one(unsigned int i);

static int window(void)
{
	for (unsigned int i = 0; i < WINDOWS; i++) {
		int ret = window_one(i);

		if (ret)
			return ret;
	}
	s_cur = 0;
	s_win = s_wins[0];
	return 0;
}

/* The next window, before an animated frame: the GPU draws into it while
 * the screen still scans the one shown last (one window: a no-op). */
static int next_window(void)
{
	if (WINDOWS == 1)
		return 0;
	s_cur = (s_cur + 1) % WINDOWS;
	s_win = s_wins[s_cur];
	s_surf = s_surfs[s_cur];
	if (!eglMakeCurrent(s_dpy, s_surf, s_surf, s_ctx))
		return egl_fail("eglMakeCurrent (next window)");
	return 0;
}

static int window_one(unsigned int i)
{
	struct board_window full;
	int ret = board_window(WIN_W, WIN_H + GUARD_ROWS, &full);

	if (ret) {
		printk("slint: board_window(%u, %u) = %d\n", WIN_W, WIN_H + GUARD_ROWS, ret);
		return ret;
	}
	/* w * 4, or the scan-out buffer's pitch when the window is a view into
	 * it (CONFIG_YAKOGL_BOARD_SCANOUT_VIEW): handed to EGL as is */
	if (full.stride < WIN_W * 4u || full.stride % 4u != 0u) {
		printk("slint: board_window gave stride %u, not whole pixels of at least %u\n", full.stride,
		       WIN_W * 4u);
		return -EINVAL;
	}
	/* The window's rows start with the sentinel (near black, alpha 0:
	 * nothing a user sees in a view), WIN_W pixels a row at the stride; the
	 * guard rows after them are read into s_guard_before and never
	 * written, and a view's padding is the screen's and never touched.
	 * The snapshot is taken here: after board_window() returned, which is
	 * after every write the board layer makes to that memory (the i.MX8MP
	 * view's zeroing and the Pi 4 view's console move happen inside it; the
	 * AM62P and the copy paths write nothing there), and before egl_up()
	 * and the first GL frame. A console printk after this point goes beside
	 * the window on the Pi 4 view, never into these rows.
	 * Through the CPU's view (Normal non-cacheable on every board,
	 * yakogl_board.h: no cache maintenance). */
	for (uint32_t y = 0; y < WIN_H + GUARD_ROWS; y++) {
		uint32_t *row = (uint32_t *)((uint8_t *)full.cpu + (size_t)y * full.stride);

		if (y < WIN_H) {
			for (uint32_t x = 0; x < WIN_W; x++)
				row[x] = SENTINEL;
		} else if (i == 0) {
			/* the guard rows of the checked window (the first frame's) */
			memcpy(&s_guard_before[(y - WIN_H) * WIN_W], row, WIN_W * 4u);
		}
	}
	s_wins[i] = full;
	s_wins[i].h = WIN_H; /* EGL and the screen get the window, not the guard rows */
	printk("slint: window %u: %ux%u at phys 0x%lx, stride %u, format 0x%x, %u guard rows\n", i,
	       s_wins[i].w, s_wins[i].h, (unsigned long)s_wins[i].phys, s_wins[i].stride,
	       s_wins[i].format, GUARD_ROWS);
	return 0;
}

static void show(void)
{
	static bool reported;
	int ret = board_show(&s_win);

	/* the check reads the window, not the screen: go on, say it once */
	if (ret && !reported) {
		printk("slint: board_show = %d (the window is not on the screen)\n", ret);
		reported = true;
	}
}

static int first_frame_and_check(void)
{
	int ret;

	board_progress(YK_RECORD_WHERE_STAGE | STAGE_FIRST_FRAME);
	ret = slint_sample_first_frame(get_proc, WIN_W, WIN_H);
	if (ret)
		return ret;
	ret = swap();
	if (ret)
		return ret;
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_SWAPPED);
	show();
	/* The check's reference points (check/src/lib.rs REFERENCE) are pixel
	 * positions and colours of the 1024x600 design, and the check reads
	 * the whole window back through the CPU: on another window size the
	 * points land elsewhere, so the verdict would be about the wrong
	 * pixels, and the read-back of a 1280x800 window in non-cacheable
	 * memory held the picture still for about two seconds on the Verdin
	 * iMX8MP (2026-10-22). So the check runs only at the design size;
	 * elsewhere the frame is shown and the run goes on unchecked. */
	if (WIN_W == 1024u && WIN_H == 600u) {
		ret = slint_sample_check(s_win.cpu, WIN_W, WIN_H, s_win.stride / 4u, GUARD_ROWS,
					 s_guard_before, s_win.format, &s_result);
		s_checked = ret >= 0;
	} else {
		printk("slint: first-frame check skipped: the reference points are the 1024x600 "
		       "design's, this window is %ux%u\n", WIN_W, WIN_H);
		s_checked = false;
		ret = 0;
	}
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_CHECKED);
	return ret;
}

/* One "slint-frame: frames=N ms_per_frame=X.XX" line per FPS_WINDOW frames and
 * a final one for the rest. The time is k_uptime across the window's frames,
 * so it counts the whole frame (draw, swap, show); the first frame (the
 * checked one) is outside it. Integer math: printk has no floats. */
#define FPS_WINDOW 120u

static void fps_line(uint32_t frames, int64_t ms)
{
	if (!frames)
		return;
	uint32_t centi = (uint32_t)((ms * 100 + frames / 2) / frames);

	printk("slint-frame: frames=%u ms_per_frame=%u.%02u\n", frames, centi / 100u, centi % 100u);
}

static int animate(void)
{
	const int64_t bound_ms = (int64_t)CONFIG_YAKOGL_BOARD_ANIMATE_S * 1000;
	int64_t t0 = k_uptime_get(), now;
	uint32_t frame = 1, win_frames = 0;
	int64_t win_t0 = t0;
	int ret = 0;

	board_progress(YK_RECORD_WHERE_STAGE | STAGE_ANIMATING);
	while ((now = k_uptime_get() - t0) < bound_ms || IS_ENABLED(CONFIG_SLINT_ANIMATE_FOREVER)) {
		ret = next_window();
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
		if (ret == 0)
			ret = slint_sample_animate_frame_m7(get_proc, WIN_W, WIN_H, (uint32_t)now, frame,
							    route3_vehicle());
#else
		if (ret == 0)
			ret = slint_sample_animate_frame(get_proc, WIN_W, WIN_H, (uint32_t)now, frame);
#endif
		if (ret == 0)
			ret = swap();
		if (ret)
			break;
		show();
		board_watchdog_feed();
		frame++;
		if (++win_frames == FPS_WINDOW) {
			int64_t t = k_uptime_get();

			fps_line(win_frames, t - win_t0);
			win_t0 = t;
			win_frames = 0;
		}
	}
	fps_line(win_frames, k_uptime_get() - win_t0);
	now = k_uptime_get() - t0;
	printk("slint: animation: %u frames in %lld ms%s\n", frame - 1, (long long)now,
	       ret ? " (stopped by a failure)" : "");
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_ANIMATED);
	return ret;
}

int main(void)
{
	int rc;

	sys_heap_init(&s_ui_heap, s_ui_heap_mem, sizeof(s_ui_heap_mem));
#ifdef CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3
	slint_sample_route3(true);
	printk("slint: route 3: the M7 composites and shows the frames\n");
#endif
	printk("slint-build: %s\n", slint_sample_build_inputs());
	printk("slint: the cluster at %ux%u, then %d s of animation\n", WIN_W, WIN_H,
	       CONFIG_YAKOGL_BOARD_ANIMATE_S);
	board_progress(YK_RECORD_WHERE_STAGE | STAGE_WINDOW);
	rc = window();
	if (rc == 0) {
		board_progress(YK_RECORD_WHERE_STAGE | STAGE_EGL);
		rc = egl_up();
	}
	if (rc == 0)
		rc = first_frame_and_check();
	if (rc == 0)
		rc = animate();
	if (s_checked) {
		board_result(YK_RECORD_KIND_FINGERPRINT, s_result.fingerprint[0],
			     s_result.fingerprint[1], s_result.fingerprint[2]);
		board_result_tag(YK_RECORD_KIND_PROBES,
				 rc == 0 ? YK_RECORD_STATUS_PASS : YK_RECORD_STATUS_FAIL,
				 s_result.probes[0], s_result.probes[1], s_result.probes[2]);
	}
	board_finish("slint", rc);
}
