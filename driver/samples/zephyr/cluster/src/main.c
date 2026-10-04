/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * cluster: signal-slot/agl-slint-cluster's instrument cluster, the same
 * source on the Raspberry Pi 4 (V3D, 1920x720 panel), the Verdin AM62P
 * (PowerVR, 1024x600) and the Verdin iMX8M Plus (GC7000, 1280x800)
 * (driver/docs/one-method.md, section 5, step 5). The board is chosen by the
 * Zephyr board only; what differs per board is behind yakogl_board.h and in
 * boards/.
 *
 * The window is the panel's own size (CONFIG_CLUSTER_PANEL_WIDTH x _HEIGHT,
 * set in boards/<board>.conf); the cluster scales itself to it
 * (app-window.slint fit-width), so it looks the same on the three panels.
 * The UI and its sim are Rust (rust/, the cluster's own ui/ through
 * ui/zephyr-cluster.slint), the check and the records too (check/, through
 * rust/src/board.rs). This file is the thin part:
 *
 *   board_window(W, 2 H)                 two halves, each an EGL surface
 *   EGL up (RGBA8888 + stencil, ES 2)
 *   frame 1 (Slint clock 0), 400 ticks,  the checked frame, into half 0, as
 *   frame 2 (1000 ms), frame 3 (3000)    v3d_gl_cluster and host/ref draw it
 *   eglSwapBuffers, eglWaitGL
 *   cluster_board_check()                probes for this size, sentinel,
 *                                        alpha, guard rows, glReadPixels
 *   board_show(half 0)
 *   CONFIG_YAKOGL_BOARD_ANIMATE_S of the drive cycle, unpaced, pipelined
 *   board_result: FINGERPRINT, PERF, APP (with YK_RECORD_APP_SHOWN_BY_COPY
 *   when board_show copied), then PROBES (the verdict)
 *   board_finish()
 *
 * The animation runs whenever the checked frame was drawn and checked, also
 * after a failed check (the fps is measured either way; the verdict keeps
 * the check's failure).
 *
 * The animation keeps one frame in flight, as v3d_gl_cluster's demo: frame
 * N+1 is drawn (recorded) into the half that frame N does not use; then N's
 * fence is waited for (a poll of EGL_SYNC_STATUS, which submits nothing),
 * N is shown (board_show: the DSS or LCDIF flip on the Verdins, the pan on
 * the Pi 4, whose window is a view into its two-screen framebuffer
 * (CONFIG_YAKOGL_BOARD_SCANOUT_VIEW): no copy on any board, and the APP
 * record's YK_RECORD_APP_SHOWN_BY_COPY says so), and only then is N+1 swapped (submitted). So a
 * half is complete when it is shown, and the half on screen is not
 * rendered into until it was flipped away from. The flip takes effect at the
 * display's next frame, so N+1's render may start while the display still
 * scans the half it writes (v3d_gl_cluster: "unmeasured"; a third half would
 * remove it).
 *
 * The records are stored after the animation: a run that dies in it must
 * not leave a PASS as its last record. PROBES is stored last because the
 * last record of a run is its verdict; its status is the whole run's.
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

#define WIN_W CONFIG_CLUSTER_PANEL_WIDTH
#define WIN_H CONFIG_CLUSTER_PANEL_HEIGHT
BUILD_ASSERT(WIN_W > 0 && WIN_H > 0, "boards/<board>.conf sets CONFIG_CLUSTER_PANEL_WIDTH and _HEIGHT");
/* The half the checked frame is drawn into, and the check reads; its guard
 * rows are the rows after it in memory, so it is the first half. */
#define CHECK_HALF 0
BUILD_ASSERT(CHECK_HALF == 0, "the check's guard rows are the rows after half 0");
/* The bound of one fence wait: longer is a failure that names the frame. */
#define FENCE_WAIT_MAX_US 2000000u
#define REPORT_MS 10000

/* Where a run that died stopped (kind PROGRESS, YK_RECORD_WHERE_STAGE | n);
 * the last one reached also goes into APP's w1. */
enum stage {
	STAGE_WINDOW = 1,
	STAGE_EGL,
	STAGE_FIRST_FRAMES,
	STAGE_SWAPPED,
	STAGE_CHECKED,
	STAGE_ANIMATING,
	STAGE_ANIMATED,
};

/* ---- what the Rust archive needs from C ---------------------------------- */

/* The UI heap (Slint, femtovg, the glyph textures): 48 MiB, a sys_heap over a
 * static arena; its high-water mark is printed at the end. */
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

/* Slint's animation clock, set by this file: the checked frame is the same
 * on every run and on the host reference. */
static uint64_t s_ui_clock_ms;

uint64_t slint_zephyr_uptime_ms(void)
{
	return s_ui_clock_ms;
}

/* rust/src/lib.rs */
typedef const void *(*cluster_get_proc_fn)(const char *name);
int slint_cluster_draw_frame(cluster_get_proc_fn get_proc, uint32_t width, uint32_t height);
int slint_cluster_advance(uint32_t n);
int slint_cluster_speed_x10(void);

/* rust/src/board.rs. The sequence and the window's fill values are the
 * check crate's (check/src/lib.rs), the one definition host/ref uses too. */
struct cluster_sequence {
	uint32_t frame_ticks; /* telemetry periods between frames 1 and 2 */
	uint32_t frame_ms[3]; /* Slint's clock at frames 1, 2 and 3 */
	uint32_t tick_ms;     /* the telemetry period */
	uint32_t guard_rows, sentinel, guard;
};
struct cluster_check_result {
	uint32_t status; /* YK_RECORD_STATUS_PASS / _FAIL: the check alone */
	uint32_t probes[3];
	uint32_t fingerprint[3];
};
struct cluster_frame_times {
	uint32_t draw_us, wait_us, show_us, swap_us, frame_us;
};
struct cluster_run_records {
	uint32_t app[3];
	uint32_t perf[3];
};
BUILD_ASSERT(sizeof(struct cluster_sequence) == 32, "rust/src/board.rs Sequence");
BUILD_ASSERT(sizeof(struct cluster_check_result) == 28, "rust/src/board.rs CheckResult");
BUILD_ASSERT(sizeof(struct cluster_frame_times) == 20, "rust/src/board.rs CFrameTimes");
BUILD_ASSERT(sizeof(struct cluster_run_records) == 24, "rust/src/board.rs RunRecords");
void cluster_board_sequence(struct cluster_sequence *out);
int cluster_board_check(const uint32_t *cpu, uint32_t w, uint32_t h, uint32_t guard_rows, uint32_t format,
			struct cluster_check_result *out);
int cluster_board_frame(const struct cluster_frame_times *t);
void cluster_board_report(uint32_t t_ms, uint32_t interval_ms);
void cluster_board_finish(uint32_t w, uint32_t h, uint8_t stage, uint32_t status, bool shown_by_copy,
			  uint32_t elapsed_ms, struct cluster_run_records *out);

/* ---- where the checked frames' time goes ---------------------------------
 *
 * Frame 1 took 10.2 s on the AM62P and 0.49 s on the Pi 4 (2026-10-03). To
 * say what runs in it from the board's own clock, not by guess, the GL calls
 * that can be slow at a first frame are timed while the checked frames are
 * drawn (s_gl_timing): shader compile and link, texture uploads, buffer
 * uploads and draws. The rest of slint_cluster_draw_frame is Slint and
 * FemtoVG on the CPU (item tree, layout, glyph and path work) and the GL
 * calls not listed here. FemtoVG resolves its GL entry points once, at the
 * first frame, so the wrappers stay in place in the animation; they time
 * nothing there. */
enum gl_timed { GT_COMPILE, GT_LINK, GT_TEXIMAGE, GT_TEXSUB, GT_BUFFER, GT_DRAW, GT_N };
static const char *const gt_name[GT_N] = { "glCompileShader", "glLinkProgram", "glTexImage2D",
					   "glTexSubImage2D", "glBufferData", "glDraw*" };
static bool s_gl_timing;
static uint32_t s_gt_calls[GT_N];
static uint64_t s_gt_cycles[GT_N], s_gt_bytes[GT_N];

#define GT_TIMED(i, bytes, call)                                                   \
	do {                                                                       \
		if (!s_gl_timing) {                                                \
			call;                                                      \
			break;                                                     \
		}                                                                  \
		const uint64_t c0_ = k_cycle_get_64();                             \
		call;                                                              \
		s_gt_cycles[i] += k_cycle_get_64() - c0_;                          \
		s_gt_calls[i]++;                                                   \
		s_gt_bytes[i] += (bytes);                                          \
	} while (0)

static void GL_APIENTRY t_CompileShader(GLuint sh)
{
	GT_TIMED(GT_COMPILE, 0, glCompileShader(sh));
}
static void GL_APIENTRY t_LinkProgram(GLuint p)
{
	GT_TIMED(GT_LINK, 0, glLinkProgram(p));
}
/* bytes as RGBA (4 per texel): FemtoVG's images and glyph textures are */
static void GL_APIENTRY t_TexImage2D(GLenum t, GLint l, GLint ifmt, GLsizei w, GLsizei h, GLint b, GLenum f,
				     GLenum ty, const void *px)
{
	GT_TIMED(GT_TEXIMAGE, px ? (uint64_t)w * (uint64_t)h * 4u : 0u,
		 glTexImage2D(t, l, ifmt, w, h, b, f, ty, px));
}
static void GL_APIENTRY t_TexSubImage2D(GLenum t, GLint l, GLint x, GLint y, GLsizei w, GLsizei h, GLenum f,
					GLenum ty, const void *px)
{
	GT_TIMED(GT_TEXSUB, (uint64_t)w * (uint64_t)h * 4u, glTexSubImage2D(t, l, x, y, w, h, f, ty, px));
}
static void GL_APIENTRY t_BufferData(GLenum t, GLsizeiptr n, const void *d, GLenum u)
{
	GT_TIMED(GT_BUFFER, (uint64_t)n, glBufferData(t, n, d, u));
}
static void GL_APIENTRY t_DrawArrays(GLenum m, GLint first, GLsizei n)
{
	GT_TIMED(GT_DRAW, 0, glDrawArrays(m, first, n));
}
static void GL_APIENTRY t_DrawElements(GLenum m, GLsizei n, GLenum t, const void *i)
{
	GT_TIMED(GT_DRAW, 0, glDrawElements(m, n, t, i));
}

static const struct {
	const char *name;
	const void *fn;
} s_timed[] = {
	{ "glCompileShader", (const void *)t_CompileShader }, { "glLinkProgram", (const void *)t_LinkProgram },
	{ "glTexImage2D", (const void *)t_TexImage2D },       { "glTexSubImage2D", (const void *)t_TexSubImage2D },
	{ "glBufferData", (const void *)t_BufferData },       { "glDrawArrays", (const void *)t_DrawArrays },
	{ "glDrawElements", (const void *)t_DrawElements },
};

static const void *get_proc(const char *name)
{
	for (size_t i = 0; i < ARRAY_SIZE(s_timed); i++) {
		if (strcmp(name, s_timed[i].name) == 0) {
			return s_timed[i].fn;
		}
	}
	return (const void *)eglGetProcAddress(name);
}

/* One line per checked frame: its wall time, then each timed GL call kind
 * (calls, ms, KiB), and what is left (Slint, FemtoVG, untimed GL); then a
 * new count. */
static void gl_timing_line(unsigned int frame, uint64_t frame_us)
{
	uint64_t gl_us = 0;

	printk("cluster: frame %u: %llu ms:", frame, (unsigned long long)(frame_us / 1000u));
	for (int i = 0; i < GT_N; i++) {
		const uint64_t us = k_cyc_to_us_floor64(s_gt_cycles[i]);

		gl_us += us;
		printk(" %s %u/%llu ms/%llu KiB", gt_name[i], s_gt_calls[i], (unsigned long long)(us / 1000u),
		       (unsigned long long)(s_gt_bytes[i] / 1024u));
		s_gt_calls[i] = 0;
		s_gt_cycles[i] = 0;
		s_gt_bytes[i] = 0;
	}
	printk("; rest (Slint, FemtoVG, untimed GL) %llu ms\n",
	       (unsigned long long)((frame_us > gl_us ? frame_us - gl_us : 0u) / 1000u));
}

/* ---- the run ------------------------------------------------------------- */

static struct board_window s_half[2]; /* the window's halves, as EGL and the screen see them */
static EGLDisplay s_dpy;
static EGLContext s_ctx;
static EGLSurface s_surf[2];
static enum stage s_stage;
static struct cluster_sequence s_seq;
static struct cluster_check_result s_check;
static bool s_checked;        /* the checked frame was drawn and checked */
static uint32_t s_elapsed_ms; /* the animation's length, as measured */

static void stage(enum stage s)
{
	s_stage = s;
	board_progress(YK_RECORD_WHERE_STAGE | s);
}

static uint32_t us_since(uint32_t c0)
{
	return (uint32_t)k_cyc_to_us_floor64(k_cycle_get_32() - c0);
}

/* Each EGL call is checked on its own: eglGetError reports the LAST call. */
static int egl_fail(const char *call)
{
	printk("cluster: %s failed: EGL error 0x%x\n", call, eglGetError());
	return -EIO;
}

static int window(void)
{
	struct board_window full;
	const uint32_t rows = 2u * WIN_H;
	int ret = board_window(WIN_W, rows, &full);

	if (ret) {
		printk("cluster: board_window(%u, %u) = %d\n", WIN_W, rows, ret);
		return ret;
	}
	if (full.stride != WIN_W * 4u) {
		/* the check (check/, Frame) walks packed rows: a view's wider pitch
		 * would be read wrongly, so it is refused here, by name */
		printk("cluster: board_window gave stride %u, not %u: the check reads packed rows only\n",
		       full.stride, WIN_W * 4u);
		return -EINVAL;
	}
	/* Half 0 starts with the sentinel, half 1 with the guard value: half 1's
	 * first guard_rows rows are half 0's guard rows for the check (no rows after
	 * half 1: on the Pi 4 the window is the 2-screen framebuffer, the
	 * firmware's measured grant). Written through the CPU's view, Normal
	 * non-cacheable on every board (yakogl_board.h). */
	uint32_t *px = full.cpu;
	const size_t half_px = (size_t)WIN_W * WIN_H, all_px = (size_t)WIN_W * rows;

	for (size_t i = 0; i < all_px; i++) {
		px[i] = i < half_px ? s_seq.sentinel : s_seq.guard;
	}
	for (int k = 0; k < 2; k++) {
		const size_t off = (size_t)k * WIN_H * full.stride;

		s_half[k] = full;
		s_half[k].h = WIN_H;
		s_half[k].phys = full.phys + off;
		s_half[k].cpu = (uint8_t *)full.cpu + off;
	}
	printk("cluster: window %ux%u, two halves at phys 0x%lx and 0x%lx, format 0x%x, stride %u\n", WIN_W,
	       WIN_H, (unsigned long)s_half[0].phys, (unsigned long)s_half[1].phys, full.format, full.stride);
	return 0;
}

static int egl_up(void)
{
	/* femtovg fills concave paths and strokes through the stencil */
	static const EGLint cfg_attribs[] = {
		EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
		EGL_STENCIL_SIZE, 8, EGL_SURFACE_TYPE, EGL_WINDOW_BIT,
		EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE,
	};
	static const EGLint ctx_attribs[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
	static struct gles_native_window nw[2];
	EGLConfig cfg;
	EGLint n = 0;

	s_dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
	if (!eglInitialize(s_dpy, NULL, NULL)) {
		return egl_fail("eglInitialize");
	}
	if (!eglChooseConfig(s_dpy, cfg_attribs, &cfg, 1, &n)) {
		return egl_fail("eglChooseConfig");
	}
	if (n != 1) {
		printk("cluster: eglChooseConfig: %d configs for RGBA8888 + stencil 8\n", n);
		return -ENOENT;
	}
	for (int k = 0; k < 2; k++) {
		nw[k] = (struct gles_native_window){
			.phys = s_half[k].phys, .width = s_half[k].w, .height = s_half[k].h,
			.stride_bytes = s_half[k].stride, .format = s_half[k].format,
		};
		s_surf[k] = eglCreateWindowSurface(s_dpy, cfg, &nw[k], NULL);
		if (s_surf[k] == EGL_NO_SURFACE) {
			return egl_fail("eglCreateWindowSurface");
		}
	}
	s_ctx = eglCreateContext(s_dpy, cfg, EGL_NO_CONTEXT, ctx_attribs);
	if (s_ctx == EGL_NO_CONTEXT) {
		return egl_fail("eglCreateContext");
	}
	if (!eglMakeCurrent(s_dpy, s_surf[CHECK_HALF], s_surf[CHECK_HALF], s_ctx)) {
		return egl_fail("eglMakeCurrent");
	}
	printk("cluster: GL_RENDERER %s, GL_VERSION %s\n", (const char *)glGetString(GL_RENDERER),
	       (const char *)glGetString(GL_VERSION));
	return 0;
}

/* A GL error during a frame is a failure (a refused draw logs its reason). */
static int gl_clean(const char *when)
{
	GLenum err = glGetError();

	if (err != GL_NO_ERROR) {
		printk("cluster: GL error 0x%x %s\n", err, when);
		return -EIO;
	}
	return 0;
}

static void show(int half)
{
	static bool reported;
	int ret = board_show(&s_half[half]);

	/* the check reads the window, not the screen: go on, say it once */
	if (ret && !reported) {
		printk("cluster: board_show = %d (the window is not on the screen)\n", ret);
		reported = true;
	}
}

static int checked_frame(void)
{
	int ret = 0;

	stage(STAGE_FIRST_FRAMES);
	for (unsigned int f = 0; f < 3 && ret == 0; f++) {
		if (f == 1) {
			ret = slint_cluster_advance(s_seq.frame_ticks);
			printk("cluster: advance %u ticks: rc %d, speed x10 %d (host reference: 672)\n",
			       s_seq.frame_ticks, ret, slint_cluster_speed_x10());
			if (ret) {
				return -EIO;
			}
		}
		s_ui_clock_ms = s_seq.frame_ms[f];
		int64_t t0 = k_uptime_get();
		const uint64_t c0 = k_cycle_get_64();

		s_gl_timing = true;
		ret = slint_cluster_draw_frame(get_proc, WIN_W, WIN_H);
		s_gl_timing = false;
		printk("cluster: frame %u (Slint clock %llu ms): rc %d, %lld ms\n", f + 1,
		       (unsigned long long)s_ui_clock_ms, ret, (long long)(k_uptime_get() - t0));
		gl_timing_line(f + 1, k_cyc_to_us_floor64(k_cycle_get_64() - c0));
	}
	if (ret) {
		return -EIO;
	}
	const int64_t t_swap = k_uptime_get();

	if (!eglSwapBuffers(s_dpy, s_surf[CHECK_HALF])) {
		return egl_fail("eglSwapBuffers");
	}
	/* The swap submits the frame and may return while it still renders
	 * (src/gl/egl.rs: a swap waits for the previous frame only); the check
	 * reads memory, so wait for it. */
	if (!eglWaitGL()) {
		return egl_fail("eglWaitGL");
	}
	printk("cluster: checked frame swap + wait: %lld ms\n", (long long)(k_uptime_get() - t_swap));
	ret = gl_clean("during the checked frame");
	if (ret) {
		return ret;
	}
	stage(STAGE_SWAPPED);
	ret = cluster_board_check(s_half[CHECK_HALF].cpu, WIN_W, WIN_H, s_seq.guard_rows, s_half[CHECK_HALF].format,
				  &s_check);
	s_checked = ret >= 0;
	show(CHECK_HALF);
	stage(STAGE_CHECKED);
	return ret == 0 ? 0 : -EIO;
}

/* The frame in flight: its half and its fence. */
struct pending {
	bool live;
	int half;
	EGLSync fence;
};

/* Wait for the frame in flight, then show it: 0, or -ETIMEDOUT / -EIO. */
static int complete(struct pending *p, uint32_t *wait_us, uint32_t *show_us, int *shown)
{
	EGLAttrib status = 0;
	uint32_t c0 = k_cycle_get_32(), polls = 0;
	int ret = 0;

	for (;;) {
		if (!eglGetSyncAttrib(s_dpy, p->fence, EGL_SYNC_STATUS, &status)) {
			ret = egl_fail("eglGetSyncAttrib");
			break;
		}
		if (status == EGL_SIGNALED) {
			break;
		}
		if (us_since(c0) > FENCE_WAIT_MAX_US) {
			printk("cluster: the frame in flight (half %d) not done after %u polls in %u us\n",
			       p->half, polls, us_since(c0));
			ret = -ETIMEDOUT;
			break;
		}
		polls++;
		k_busy_wait(20);
	}
	*wait_us = us_since(c0);
	if (!eglDestroySync(s_dpy, p->fence)) {
		printk("cluster: eglDestroySync failed: EGL error 0x%x\n", eglGetError());
	}
	p->live = false;
	if (ret) {
		return ret;
	}
	c0 = k_cycle_get_32();
	show(p->half);
	*show_us = us_since(c0);
	*shown = p->half;
	return 0;
}

static int animate(void)
{
	const int64_t bound_ms = (int64_t)CONFIG_YAKOGL_BOARD_ANIMATE_S * 1000;
	const int64_t t0 = k_uptime_get();
	uint64_t ticks_done = 0;
	int64_t t, next_report = REPORT_MS, last_report = 0;
	struct pending pend = { 0 };
	int shown = CHECK_HALF, ret = 0;
	uint32_t drawn = 0, counted = 0;

	stage(STAGE_ANIMATING);
	while ((t = k_uptime_get() - t0) < bound_ms) {
		const uint32_t c_start = k_cycle_get_32();
		const uint64_t due = (uint64_t)t / s_seq.tick_ms;
		struct cluster_frame_times ft = { 0 };
		const bool had_pending = pend.live;
		const int half = (pend.live ? pend.half : shown) ^ 1;

		if (due > ticks_done) {
			slint_cluster_advance((uint32_t)(due - ticks_done));
			ticks_done = due;
		}
		s_ui_clock_ms = s_seq.frame_ms[2] + (uint64_t)t; /* on from the checked frame */
		if (!eglMakeCurrent(s_dpy, s_surf[half], s_surf[half], s_ctx)) {
			ret = egl_fail("eglMakeCurrent");
			break;
		}
		uint32_t c = k_cycle_get_32();

		ret = slint_cluster_draw_frame(get_proc, WIN_W, WIN_H);
		ft.draw_us = us_since(c);
		if (ret) {
			printk("cluster: animation frame %u: slint_cluster_draw_frame rc %d\n", drawn + 1, ret);
			ret = -EIO;
			break;
		}
		if (pend.live) {
			ret = complete(&pend, &ft.wait_us, &ft.show_us, &shown);
			if (ret) {
				break;
			}
		}
		c = k_cycle_get_32();
		if (!eglSwapBuffers(s_dpy, s_surf[half])) {
			ret = egl_fail("eglSwapBuffers");
			break;
		}
		ft.swap_us = us_since(c);
		ret = gl_clean("during an animation frame");
		if (ret) {
			break;
		}
		pend.fence = eglCreateSync(s_dpy, EGL_SYNC_FENCE, NULL);
		if (pend.fence == EGL_NO_SYNC) {
			ret = egl_fail("eglCreateSync");
			break;
		}
		pend.live = true;
		pend.half = half;
		drawn++;
		ft.frame_us = us_since(c_start);
		/* an iteration that presented the frame before it is counted */
		if (had_pending) {
			counted++;
			if (cluster_board_frame(&ft)) {
				printk("cluster: animation frame %u took %u us, over the budget\n", drawn,
				       ft.frame_us);
				ret = -ETIMEDOUT;
				break;
			}
		}
		board_watchdog_feed();
		t = k_uptime_get() - t0;
		if (t >= next_report) {
			cluster_board_report((uint32_t)t, (uint32_t)(t - last_report));
			last_report = t;
			next_report += REPORT_MS;
		}
	}
	s_elapsed_ms = (uint32_t)(k_uptime_get() - t0);
	/* the last frame in flight is presented, outside the counts */
	if (pend.live) {
		uint32_t w = 0, s = 0;
		int drain = complete(&pend, &w, &s, &shown);

		ret = ret ? ret : drain;
	}
	/* an animation that presented nothing measured nothing: not a pass */
	if (ret == 0 && bound_ms > 0 && counted == 0) {
		printk("cluster: animation: no frame presented in %u ms\n", s_elapsed_ms);
		ret = -ENODATA;
	}
	stage(STAGE_ANIMATED);
	return ret;
}

int main(void)
{
	struct cluster_run_records rec = { 0 };
	struct sys_memory_stats hs;
	int rc;

	sys_heap_init(&s_ui_heap, s_ui_heap_mem, sizeof(s_ui_heap_mem));
	cluster_board_sequence(&s_seq);
	printk("cluster: agl-slint-cluster at %ux%u, then %d s of animation\n", WIN_W, WIN_H,
	       CONFIG_YAKOGL_BOARD_ANIMATE_S);
	stage(STAGE_WINDOW);
	rc = window();
	if (rc == 0) {
		stage(STAGE_EGL);
		rc = egl_up();
	}
	if (rc == 0) {
		rc = checked_frame();
	}
	if (s_checked) {
		int arc = animate();

		rc = rc ? rc : arc;
	}
	if (sys_heap_runtime_stats_get(&s_ui_heap, &hs) == 0) {
		printk("cluster: UI heap %zu bytes now, %zu at most (high-water), of %u\n", hs.allocated_bytes,
		       hs.max_allocated_bytes, UI_HEAP_BYTES);
	}
	if (s_checked) {
		const uint32_t status = rc == 0 ? YK_RECORD_STATUS_PASS : YK_RECORD_STATUS_FAIL;

		cluster_board_finish(WIN_W, WIN_H, (uint8_t)s_stage, status, board_show_copies() != 0u, s_elapsed_ms,
				     &rec);
		board_result(YK_RECORD_KIND_FINGERPRINT, s_check.fingerprint[0], s_check.fingerprint[1],
			     s_check.fingerprint[2]);
		board_result_tag(YK_RECORD_KIND_PERF, 0, rec.perf[0], rec.perf[1], rec.perf[2]);
		board_result(YK_RECORD_KIND_APP, rec.app[0], rec.app[1], rec.app[2]);
		board_result_tag(YK_RECORD_KIND_PROBES, (uint8_t)status, s_check.probes[0], s_check.probes[1],
				 s_check.probes[2]);
	}
	board_finish("cluster", rc);
}
