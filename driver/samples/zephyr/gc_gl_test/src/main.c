/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_gl_test - the GL test suite on the i.MX8MP's Vivante GC7000UL.
 *
 * tools/pvrlab/gl_test.c is the on-silicon suite the PowerVR side is held to:
 * forty-odd cases, every one of them checked by reading pixels back, covering
 * uniforms, textures, depth and stencil, cull, scissor, FBOs, blending and
 * colour masks, every primitive mode, client arrays, VAOs, EGLImage and the
 * stencil-then-cover fills FemtoVG draws with. The same file compiles here:
 * it is GL-level and portable, and only the few lines that start the backend
 * know which GPU this is.
 *
 * Running it is the point. A triangle proves the pipe; this proves the
 * backend, and it does it case by case, so a failure names itself instead of
 * arriving as "the UI is black".
 *
 * Reading a run, on a board with no console: the result record in the SNVS
 * (report_result: the case, or the set's passed count and first failure),
 * which survives the SoC reset. The image resets the SoC right after the
 * record is written (gc_sample_finish_recorded), so a one-case run costs its
 * boot and the case, not a hold. Only the diagnostic knobs whose answer has
 * no record field (REPORT_COUNTERS, DEPTH_VALUE, DEPTH_PROBE,
 * REPORT_CHECK_COLOUR) still put a code in the return time
 * (gc_sample_hold_code: 70 s + CONFIG_GC_SAMPLE_HOLD_CODE_STEP s * code).
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include <gc_render.h>

#if defined(CONFIG_YAKOGL_VIVANTE_DEPTH_GUARD)
/* For the depth canary only; the probe lives in the backend, which is where
 * the depth allocation's shape is known. */
#include "backend.h"
#endif

#include "gc_sample.h"
#include "gc_result.h"
#include "osal_internal.h" /* osal_zephyr_diag_print */
#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
#include "imx8mp_display.h"
#endif

LOG_MODULE_REGISTER(gc_gl_test, LOG_LEVEL_INF);

/*
 * The suite takes longer than the watchdog Linux left armed, so it cannot be
 * fed from the work: a thread of its own does it, and past a deadline it
 * gives up on the suite and resets the SoC rather than leaving the board to
 * the watchdog's two silent minutes. Its deadline is bounded, which is the
 * lab rule -- feeding that watchdog forever once cost a day of board time.
 *
 * The first run of this sample came back in 132 s, which is WDOG1's 124..128
 * plus the boot: the suite had been cut off part way through and said
 * nothing about how far it got.
 */
/*
 * The case the suite was in when a fence first failed to complete, or -1.
 *
 * A kick that never completes costs two seconds and, because gc_isr frees a
 * hardware event only when its interrupt arrives, leaks its event for good.
 * Thirty of those and every later submit pays a second as well. That reads
 * from the outside as "the suite got slow", which is how it has been read so
 * far. Naming the case it started in turns a hang back into a hang.
 */
static volatile int s_hang_case = -1;

/* The select only exists when the counters are asked for; this function is
 * compiled either way. */
#ifndef CONFIG_GC_GL_TEST_COUNTER_SELECT
#define CONFIG_GC_GL_TEST_COUNTER_SELECT 0
#endif

/*
 * What the run spent its waiting on, as six bits: bit 5 says at least one
 * submit found all thirty hardware events in flight (a 1 s wait), and the low
 * five are how many 2 s fence waits gave up, saturating at 31.
 *
 * The regression set does not finish inside its deadline while every case on
 * its own finishes in seconds, so something accumulates across the cases. The
 * two waits long enough to account for that are the only ones in the path,
 * and this is the only way to tell them apart on a board with no console.
 */
static unsigned int counter_code(void)
{
	uint32_t fence_timeouts = 0, event_busy = 0, wait_ms = 0;

	gc_render_counters(&fence_timeouts, &event_busy, &wait_ms);
	LOG_INF("gc_gl_test: %u fence timeouts, %u event starvations, %u ms waiting",
		fence_timeouts, event_busy, wait_ms);

	if (CONFIG_GC_GL_TEST_COUNTER_SELECT == 1) {
		/* Seconds spent in the two waits, which is the question the
		 * flags only stand in for. 63 means "63 or more", which is
		 * where a regression set lands: use select 2 to see past it. */
		return MIN(wait_ms / 1000u, 63u);
	}
	if (CONFIG_GC_GL_TEST_COUNTER_SELECT == 3) {
		/* The case a kick first failed to complete in; 63 when none
		 * did, because a case number of 0 is a real answer. */
		return s_hang_case < 0 || s_hang_case > 62 ? 63u
						           : (unsigned int)s_hang_case;
	}
	if (CONFIG_GC_GL_TEST_COUNTER_SELECT == 2) {
		/* The same in units of eight seconds, so the scale reaches the
		 * 400 s a set is given instead of saturating a sixteenth of
		 * the way in. One code step is 8 s; 63 means 504 s or more. */
		return MIN(wait_ms / 8000u, 63u);
	}
	return (event_busy ? 32u : 0u) | MIN(fence_timeouts, 31u);
}

static volatile bool s_suite_done;
static volatile unsigned int s_stage;   /* 0 before the suite, 1 inside, 2 after */


/* Which case the suite is in, set by run_case() in gl_test.c. */
extern volatile int gl_test_current_case;

#define SUPERVISOR_TICK_MS 250
#define SUPERVISOR_STACK   2048

static void supervisor(void *a, void *b, void *c)
{
	const int64_t deadline_ms = (int64_t)CONFIG_GC_GL_TEST_DEADLINE_SECONDS * 1000;

	ARG_UNUSED(a);
	ARG_UNUSED(b);
	ARG_UNUSED(c);
	while (true) {
		gc_sample_service_wdog();
		if (s_hang_case < 0) {
			uint32_t fence_timeouts = 0;

			gc_render_counters(&fence_timeouts, NULL, NULL);
			if (fence_timeouts) {
				s_hang_case = gl_test_current_case;
				LOG_ERR("gc_gl_test: a kick did not complete in case %d",
					s_hang_case);
			}
		}
		if (!s_suite_done && k_uptime_get() > deadline_ms) {
			int c = gl_test_current_case;

			LOG_ERR("gc_gl_test: the suite did not finish in %d s (stage %u, "
				"case %d); resetting", CONFIG_GC_GL_TEST_DEADLINE_SECONDS,
				s_stage, c);
			gc_sample_verdict("gc_gl_test", false, "the suite did not finish in time");
			/* A sweep keeps its own record: the cases it finished,
			 * and by the count in w0, which case it hung in. */
			if (!IS_ENABLED(CONFIG_GC_GL_TEST_SWEEP)) {
				gc_result_case(c < 0 ? 0xffffu : (unsigned int)c, -1,
					       GC_RESULT_CASE_UNFINISHED, s_stage, 0);
			}
			/* The record names the case (UNFINISHED); the counters
			 * have no record field, so they keep the coded hold. */
			if (IS_ENABLED(CONFIG_GC_GL_TEST_REPORT_COUNTERS)) {
				gc_sample_hold_code(counter_code());
			}
			gc_sample_finish_recorded();
		}
		k_sleep(K_MSEC(SUPERVISOR_TICK_MS));
	}
}

K_THREAD_DEFINE(gc_gl_test_supervisor, SUPERVISOR_STACK, supervisor, NULL, NULL, NULL,
		K_PRIO_COOP(1), 0, 0);

int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce,
		unsigned int term0, int panel);

extern unsigned int gl_test_passed, gl_test_total;
extern int gl_test_first_fail, gl_test_first_fail_at;
extern volatile int gl_test_check_no, gl_test_first_bad_check;
extern const char *gl_test_skip;
extern volatile uint32_t gl_test_first_bad_got, gl_test_first_bad_want;
int gl_test_msaa_page(unsigned int tag, uint32_t *w);

/* A colour as six bits: the marker, then one bit per channel at half scale.
 * Enough to tell red from green from blue, which is what a texture or a mip
 * level failing usually comes down to. 32 black .. 39 white.
 *
 * gl_test's pixels are 0xAABBGGRR, the order read_topdown() leaves them in
 * and the order its expectations are written in (red is 0xff0000ff). */
static unsigned int colour_code(uint32_t px)
{
	return 32u | ((px & 0xffu) >= 128u ? 4u : 0u)
		   | (((px >> 8) & 0xffu) >= 128u ? 2u : 0u)
		   | (((px >> 16) & 0xffu) >= 128u ? 1u : 0u);
}

/*
 * The record Linux reads back from the SNVS after the reset (gc_result.h):
 * got and want exactly, where the hold time can carry six bits of one of them.
 * Written whatever the hold then reports, so it never depends on which
 * reporting option the image was built with.
 */
#ifdef CONFIG_GC_GL_TEST_SWEEP
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);

static uint32_t s_sweep_w[3];
static uint32_t s_sweep_done;
static uint64_t s_sweep_pass;   /* bit i: case FIRST + i passed */
static int s_sweep_last = -1;   /* the last case run */
extern uint32_t gl_test_leaked_error_cases[4];

/* gl_test's walker calls this after each case (the weak default is a no-op). */
void gl_test_case_result(int c, int rc)
{
	int i = c - CONFIG_GC_GL_TEST_SWEEP_FIRST;
	uint32_t st = rc == 0 ? 1u : rc == -1 ? 2u : 3u;

	if (i < 0 || i >= GC_RESULT_SWEEP_CASES) {
		return;
	}
	s_sweep_w[i / 16] |= st << (2 * (i % 16));
	if (st == 1u) {
		s_sweep_pass |= 1ull << i;
	}
	s_sweep_last = c;
	s_sweep_done++;
	gc_result_write_tag(GC_RESULT_SWEEP, (uint8_t)MIN(s_sweep_done, 255u), s_sweep_w[0],
			    s_sweep_w[1], s_sweep_w[2]);
}

uint32_t yakogl_gpu_work_lost(void);

/* the counters record's leak fields: the first leaker's offset from first
 * (0xff none) << 8 | how many of the 48 leaked */
static uint32_t leak_fields(int first)
{
	uint32_t n = 0, at = 0xffu;

	for (int i = 0; i < GC_RESULT_SWEEP_CASES; i++) {
		int c = first + i;

		if (c >= 0 && c < 128 &&
		    (gl_test_leaked_error_cases[c / 32] & (1u << (c % 32)))) {
			if (at == 0xffu) {
				at = (uint32_t)i;
			}
			n++;
		}
	}
	return at << 8 | n;
}

/* CONFIG_GC_GL_TEST_SWEEP_LISTS entry k as case numbers inside the sweep's
 * range; 0 when there is no such entry */
static unsigned parse_list(int k, int *list, unsigned max)
{
	const char *p = CONFIG_GC_GL_TEST_SWEEP_LISTS;
	const int f = CONFIG_GC_GL_TEST_SWEEP_FIRST;
	unsigned n = 0;

	for (int i = 0; i < k && *p; p++) {
		if (*p == ';') {
			i++;
		}
	}
	while (*p && *p != ';' && n < max) {
		int a = 0, b;

		while (*p == ' ' || *p == ',') {
			p++;
		}
		if (*p < '0' || *p > '9') {
			break;
		}
		while (*p >= '0' && *p <= '9') {
			a = a * 10 + (*p++ - '0');
		}
		b = a;
		if (*p == '-') {
			p++;
			b = 0;
			while (*p >= '0' && *p <= '9') {
				b = b * 10 + (*p++ - '0');
			}
		}
		for (int c = a; c <= b && n < max; c++) {
			/* 7 is the regression set, not a case */
			if (c != 7 && c >= f && c < f + GC_RESULT_SWEEP_CASES) {
				list[n++] = c;
			}
		}
	}
	return n;
}
#endif

/*
 * Case 16's x4 probes without a console (gl_test.c, gl_test_msaa_page): the
 * selector GC_RESULT_PERF_SEL | tag, tag 0xf0..0xfd, runs case 16 alone and
 * ends with that page of its readings as a PERF record, w0's low byte = the
 * tag, instead of the case's CASE record. Without the selector the run ends
 * with its CASE record as before. The tag, or -1 when the selector is not
 * one (or the image does not run case 16).
 */
static int msaa_page_tag(void)
{
	int sel = gc_result_selector();

#ifdef CONFIG_GC_GL_TEST_SWEEP
	if (CONFIG_GC_GL_TEST_SWEEP_FIRST > 16 || CONFIG_GC_GL_TEST_SWEEP_FIRST + GC_RESULT_SWEEP_CASES <= 16) {
		return -1;
	}
#else
	if (CONFIG_GC_GL_TEST_CASE != 16) {
		return -1;
	}
#endif
	if (sel < (int)GC_RESULT_PERF_SEL + 0xf0 || sel > (int)GC_RESULT_PERF_SEL + 0xfd) {
		return -1;
	}
	return sel - (int)GC_RESULT_PERF_SEL;
}

static void report_result(int rc)
{
	int the_case = CONFIG_GC_GL_TEST_CASE;
	int page = msaa_page_tag();

	if (page >= 0) {
		uint32_t w[3];

		if (gl_test_msaa_page((unsigned int)page, w) == 0) {
			LOG_INF("gc_gl_test: case 16 probe page 0x%02x", page);
			gc_result_write_tag(GC_RESULT_PERF, (uint8_t)page, w[0], w[1], w[2]);
			return;
		}
		LOG_ERR("gc_gl_test: no case 16 probe page 0x%02x (the 4x half did not run)", page);
#ifdef CONFIG_GC_GL_TEST_SWEEP
		return;   /* the sweep's own record stays the verdict */
#endif
	}

#ifdef CONFIG_GC_GL_TEST_SWEEP
	/* The sweep's record is already there, rewritten after every case --
	 * unless a selector asked for one case alone, which ends with that
	 * case's check record, or for the leaks (gc_result.h). */
	{
		int only = gc_result_selector();
		const int f = CONFIG_GC_GL_TEST_SWEEP_FIRST;

		if (only >= GC_RESULT_SWEEP_SEL_LEAKS) {
			uint32_t timeouts = 0;

			gc_render_counters(&timeouts, NULL, NULL);
			gc_result_write_tag(GC_RESULT_SWEEP, GC_RESULT_SWEEP_TAG_LEAKS,
					    MIN(yakogl_gpu_work_lost(), 0xffffu) << 16 |
						    MIN(gc_render_resets(), 0xffu) << 8 |
						    MIN(timeouts, 0xffu),
					    (uint32_t)(s_sweep_pass >> 32) << 16 | leak_fields(f),
					    (uint32_t)s_sweep_pass);
			return;
		}
		if (only < 0 || only >= GC_RESULT_SWEEP_CASES || s_sweep_last < 0) {
			return;
		}
		the_case = s_sweep_last;
	}
#endif
	if (the_case == 7) {
		if (rc == 0) {
			gc_result_case(7, -1, GC_RESULT_CASE_PASS, gl_test_passed, gl_test_total);
		} else {
			gc_result_case(gl_test_first_fail < 0 ? 0xffffu
							      : (unsigned int)gl_test_first_fail,
				       -1, GC_RESULT_CASE_SET_FAIL, gl_test_passed, gl_test_total);
		}
	} else if (rc == 0) {
		gc_result_case(the_case, -1, GC_RESULT_CASE_PASS,
			       (uint32_t)gl_test_check_no, 0);
	} else if (gl_test_first_bad_check < 0) {
		gc_result_case(the_case, -1, GC_RESULT_CASE_NO_CHECK,
			       (uint32_t)gl_test_check_no, 0);
	} else {
		gc_result_case(the_case, gl_test_first_bad_check,
			       GC_RESULT_CASE_CHECK, gl_test_first_bad_got,
			       gl_test_first_bad_want);
	}
}

int main(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	int rc;

#ifdef CONFIG_SAFEUI_DISPLAY_IMX8MP
	/* Backlight on before anything else: on the normal kexec route Linux
	 * leaves the LCDIF scanning and the backlight off, so the panel
	 * console is drawing into a picture nobody can see. Nothing else about
	 * the display is touched -- this sample never shows a rendered frame,
	 * it only reads pixels back. */
	(void)imx8mp_display_on(0, 0);
#endif
#ifdef CONFIG_GC_GL_TEST_MSAA
	{
		extern void gc_debug_set_msaa(int on);

		/* the 4x resolve and the EGL 4x config, for the measurement */
		gc_debug_set_msaa(CONFIG_GC_GL_TEST_MSAA_BITS);
		LOG_INF("gc_gl_test: gc_debug_msaa=%d (4x MSAA and its EGL config on)",
			CONFIG_GC_GL_TEST_MSAA_BITS);
	}
#endif
#ifdef CONFIG_GC_GL_TEST_UNIIDX_NO_CLAMP
	{
		extern void shc_debug_set_uniidx(int bits);

		/* before the first glCompileShader: the compiler reads it there */
		shc_debug_set_uniidx(1);
		LOG_INF("gc_gl_test: shc_debug_uniidx=1 (no clamp before the offset multiply)");
	}
#endif
	gl_test_skip = CONFIG_GC_GL_TEST_SKIP;
	LOG_INF("gc_gl_test: case %d on the GC7000UL%s%s", CONFIG_GC_GL_TEST_CASE,
		CONFIG_GC_GL_TEST_SKIP[0] ? ", skipping " : "", CONFIG_GC_GL_TEST_SKIP);
	gc_sample_service_wdog();

	if (!device_is_ready(gpu)) {
		gc_sample_verdict("gc_gl_test", false, "the GPU device did not initialise");
		gc_result_case(CONFIG_GC_GL_TEST_CASE, -1, GC_RESULT_CASE_NO_DEVICE, 0, 0);
		gc_sample_finish_recorded();
		return 0;
	}
	/* gl_test_run() brings the OSAL and the backend up itself; the render
	 * service has to exist first because the OSAL forwards to it. */
	rc = gc_render_init(gpu);
	if (rc) {
		LOG_ERR("gc_render_init: %d", rc);
		gc_sample_verdict("gc_gl_test", false, "the render service did not come up");
		gc_result_case(CONFIG_GC_GL_TEST_CASE, -1, GC_RESULT_CASE_NO_DEVICE, 1, 0);
		gc_sample_finish_recorded();
		return 0;
	}

	{
		int64_t t0 = k_uptime_get();

		s_stage = 1;
#ifdef CONFIG_GC_GL_TEST_SWEEP
		{
			static int list[GC_RESULT_SWEEP_CASES];
			unsigned n = 0;

			/* A reader's selector (gc_result.h) picks one case of
			 * the sweep's range to run alone, in a fresh boot: the
			 * same image then tells a case's own failure from one a
			 * previous case left behind on the shared context. */
			int only = gc_result_selector();

			if (msaa_page_tag() >= 0) {
				/* a case 16 probe page: case 16 alone, as its own index */
				only = 16 - CONFIG_GC_GL_TEST_SWEEP_FIRST;
			}
			if (only >= GC_RESULT_SWEEP_SEL_LIST) {
				/* a list of the table, in its own order */
				n = parse_list(only - GC_RESULT_SWEEP_SEL_LIST, list,
					       GC_RESULT_SWEEP_CASES);
				LOG_INF("gc_gl_test: list %d, %u cases",
					only - GC_RESULT_SWEEP_SEL_LIST, n);
			}
			for (int i = 0; i < GC_RESULT_SWEEP_CASES &&
					only < GC_RESULT_SWEEP_SEL_LIST; i++) {
				int c = CONFIG_GC_GL_TEST_SWEEP_FIRST + i;

				if (only >= 0 && only < GC_RESULT_SWEEP_CASES && i != only) {
					continue;
				}
				if (c == 7) {
					/* the regression set, not a case */
					gl_test_case_result(c, -3);
					continue;
				}
				list[n++] = c;
			}
			rc = gl_test_run(gl_test_set_list(list, n, NULL), 0, 0, 0, 0, 0, 0);
		}
#else
		rc = gl_test_run(CONFIG_GC_GL_TEST_CASE, 0, 0, 0, 0, 0, 0);
#endif
		s_stage = 2;
		LOG_INF("gc_gl_test: the suite took %lld s",
			(long long)((k_uptime_get() - t0) / 1000));
		/* the OSAL's close record: the pending-free queue's counters */
		osal_zephyr_diag_print();
	}

	/*
	 * 0 by default: the record is the reading and nothing waits. A
	 * diagnostic whose answer is a coded hold sets it, so that the hold is
	 * the only thing that varies in the return time (Kconfig).
	 */
	while (k_uptime_get() < (int64_t)CONFIG_GC_GL_TEST_SETTLE_SECONDS * 1000) {
		k_sleep(K_MSEC(100));
	}
	s_suite_done = true;
	report_result(rc);
	if (CONFIG_GC_GL_TEST_CASE == 7) {
		LOG_INF("gc_gl_test: %u of %u cases passed; the first failure is case %d",
			gl_test_passed, gl_test_total, gl_test_first_fail);
	}

#if defined(CONFIG_YAKOGL_VIVANTE_DEPTH_GUARD)
	/*
	 * Did the pixel engine write outside its depth surface?
	 *
	 * The canary went down when the buffer was allocated, so everything
	 * the GPU has done to it since is visible. The hold is 70 s + 6 s per
	 * step, and the steps are chosen so that a clean run is unmistakable:
	 *
	 *   0        nothing changed at all -- no depth write happened, which
	 *            is itself a finding and not a pass
	 *   1        the write stayed inside the surface
	 *   2..62    it ran outside, by (code - 1) * 4 KiB, rounded up
	 *   63       no depth buffer to look at
	 */
	if (IS_ENABLED(CONFIG_GC_GL_TEST_DEPTH_VALUE)) {
		uint32_t word = 0;
		int uniform = gc_depth_modal_word(&word);
		unsigned int code;

		if (uniform < 0)
			code = 63u;   /* no depth buffer at all */
		else if (uniform == 0)
			/* Not uniform, so there is no single stored value to
			 * report and the candidate word means nothing. A
			 * distinct code, because reading "no majority" as a
			 * depth value is how the first two runs of this probe
			 * were ambiguous: both returned 0, which could equally
			 * have been a surface the clear never covered. 62 is
			 * depth 0.969..0.984, which no clear in the suite asks
			 * for. */
			code = 62u;
		else
			code = word >> 26;
		LOG_INF("gc_gl_test: the depth surface mostly holds 0x%08x "
			"(%s), depth %u of 16777215 (hold code %u)",
			word, uniform > 0 ? "a majority" : "no majority",
			word >> 8, code);
		gc_sample_hold_code(code);
	}

	if (IS_ENABLED(CONFIG_GC_GL_TEST_DEPTH_PROBE)) {
		int32_t first = 0, last = 0;
		uint32_t outside = 0;
		unsigned int code;

		int changed = gc_depth_canary_scan(&first, &last, &outside);

		if (changed < 0) {
			code = 63u;          /* no depth buffer at all */
		} else if (changed == 0) {
			code = 0u;           /* the GPU wrote none of it */
		} else if (outside == 0u) {
			code = 1u;
		} else {
			code = 1u + DIV_ROUND_UP(outside, 4096u);
			code = MIN(code, 62u);
		}
		LOG_INF("gc_gl_test: depth write %d..%d relative to the surface, "
			"%u bytes outside it (hold code %u)",
			first, last, outside, code);
		gc_sample_hold_code(code);
		gc_sample_verdict("gc_gl_test", outside == 0u,
				  outside ? "the depth write ran outside its surface" : NULL);
		gc_sample_finish_recorded();
		return 0;
	}
#endif

	/*
	 * What the run waited on, for any case and not just the set. This sat
	 * inside the CASE == 7 branch, which meant a single case could not be
	 * asked whether it hung a kick -- the one question worth asking of a
	 * case that the set slows down after.
	 */
	if (IS_ENABLED(CONFIG_GC_GL_TEST_REPORT_COUNTERS)) {
		gc_sample_hold_code(counter_code());
		gc_sample_verdict("gc_gl_test", rc == 0,
				  rc ? "at least one case failed" : NULL);
		gc_sample_finish_recorded();
		return 0;
	}


	/*
	 * One case, reported as the check inside it that failed first.
	 *
	 * The plain PASS/FAIL holds say whether a case works, which is one bit
	 * per board run, and a board run is minutes on a shared board. Case 2
	 * alone makes twenty-odd checks -- four texture sizes, a gradient,
	 * three mip levels, two texture units -- and knowing which of them
	 * gave way is the difference between a fix and a guess. So:
	 *
	 *   1       the case passed
	 *   2..62   the first failing check, as 1 + its number in the case
	 *   63      it failed, but no check said so: a GL error, a shader that
	 *           would not link, or a check numbered past 61
	 *
	 * The plain holds are left alone when this is off, so the readings
	 * already written down keep their meaning.
	 */
	if (CONFIG_GC_GL_TEST_CASE != 7 &&
	    IS_ENABLED(CONFIG_GC_GL_TEST_REPORT_CHECK)) {
		unsigned int code;

		if (rc == 0) {
			code = 1u;
		} else if (gl_test_first_bad_check < 0 || gl_test_first_bad_check > 61) {
			code = 63u;
		} else if (IS_ENABLED(CONFIG_GC_GL_TEST_REPORT_CHECK_COLOUR)) {
			/* The check number is already known by now; what it read
			 * is the next question, and one more run answers it. */
			code = colour_code(gl_test_first_bad_got);
		} else {
			code = (unsigned int)gl_test_first_bad_check + 1u;
		}
		LOG_INF("gc_gl_test: case %d made %d checks; the first bad one is %d, "
			"which read 0x%08x and wanted 0x%08x (hold code %u)",
			CONFIG_GC_GL_TEST_CASE, gl_test_check_no, gl_test_first_bad_check,
			gl_test_first_bad_got, gl_test_first_bad_want, code);
		gc_sample_hold_code(code);
	}

	gc_sample_verdict("gc_gl_test", rc == 0,
			  rc ? "at least one case failed" : NULL);
	if (rc != 0 && gc_render_ready()) {
		gc_render_stall_dump("gc_gl_test failed");
	}
	gc_sample_finish_recorded();
	return 0;
}
