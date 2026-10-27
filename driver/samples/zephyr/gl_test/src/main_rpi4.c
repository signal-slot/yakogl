/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_gl_test: the GL suite on the Raspberry Pi 4's V3D.
 *
 * driver/tools/pvrlab/gl_test.c is the suite the AM62P and the i.MX8MP are
 * held to, case by case, every case checked by reading pixels back. This
 * sample runs it through YakoGL's GL core, its in-process compiler and its
 * V3D backend, on the Zephyr OSAL src/osal/zephyr_v3d.c. The suite renders
 * into its own 64x64 surface; the HDMI framebuffer is only the console.
 *
 * The console scrolls, so the verdicts are repeated at the end: one line per
 * case (PASS, FAIL, SKIP, or "no such case"), a count, and the build. A FAIL
 * names itself above: the suite prints the check that failed, and the backend
 * prints the reason for every draw it refused ("yakogl/v3d: draw refused:").
 * CONFIG_V3D_GL_TEST_CASE runs one case alone, so its lines are the only ones
 * on the screen.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

#include "osal_internal.h" /* struct osal_zephyr_v3d_init */
#include "rpi_mbox.h"
#include "fbcon.h"
#include "rpi_wdog.h"

#ifndef V3D_BUILD
#define V3D_BUILD "unknown"
#endif

#define FB_W 1280
#define FB_H 720
#define MAX_CASES 64

/* gl_test.c's entry points */
int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce, int panel);
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);

static int clk_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	int ret = rpi_clock_state(RPI_FW_CLK_V3D, on);
	printk("  fw clock 5 %s: %d%s\n", on ? "on" : "off", ret, ret ? " (not fatal)" : "");
	return 0; /* as v3d_gl_triangle */
}

static struct osal_zephyr_v3d_init plat = { .clock_set = clk_set };

/* gl_test.c's V3D arm starts the OSAL with this */
void *gl_test_v3d_osal_arg(void)
{
	return &plat;
}

/* ---- the frozen record (CONFIG_V3D_GL_TEST_RECORD) -------------------------
 *
 * The console is drawn into the HDMI framebuffer and scrolls; the board's
 * watchdog ends the image, and the screen is all anyone reads. So every printk
 * line is also kept in a ring, and at the end of the run -- or at
 * CONFIG_V3D_GL_TEST_RECORD_AT_S after boot, if the run has not ended -- the
 * screen is cleared and one record is printed and frozen: the last lines of
 * the run (the case's read-backs, the stall lines), the OSAL's job record
 * (osal_v3d_diag_print), and the verdicts. Nothing prints to the screen after
 * it; the watchdog holds CONFIG_V3D_GL_TEST_HOLD_S. First used on the
 * textured-case hang and the step C failures (2026-09-24). */
#ifndef CONFIG_V3D_GL_TEST_RECORD_LINES
#define CONFIG_V3D_GL_TEST_RECORD_LINES 0
#define CONFIG_V3D_GL_TEST_RECORD_AT_S 0
#endif
#define RING_LINES 30 /* the most CONFIG_V3D_GL_TEST_RECORD_LINES allows */
#define RING_COLS  158
#define REPLAY     CONFIG_V3D_GL_TEST_RECORD_LINES

static char ring[RING_LINES][RING_COLS + 1];
static unsigned ring_next, ring_count, ring_col;
static printk_hook_fn_t screen_out;
/* The console hook from before the first fbcon_attach (the UART). fbcon
 * replaces the printk hook, so without this the UART got nothing after the
 * banner (129K on 9efd1dce, 2026-09-25: the fault dumps scrolled off the
 * screen record and never reached the host's UART log). Every character
 * goes to it too, the frozen record's time included (v3d_deqp and slint's
 * 19644131 do the same). */
static printk_hook_fn_t uart_out;
static atomic_t record_taken;
static volatile bool frozen, recording;
static struct rpi_fb fb;
static bool have_screen;

static int tap(int c)
{
	if (uart_out)
		uart_out(c);
	if (frozen)
		return c;
	if (!recording) {
		if (c == '\n') {
			ring[ring_next][ring_col] = 0;
			ring_next = (ring_next + 1) % RING_LINES;
			ring_count += ring_count < RING_LINES;
			ring_col = 0;
		} else if (c != '\r' && ring_col < RING_COLS) {
			ring[ring_next][ring_col++] = (char)c;
		}
	}
	return screen_out ? screen_out(c) : c;
}

static void verdicts(void); /* below */
static int64_t run_ms;
static int run_rc;
static bool run_done;

static void take_record(const char *ending)
{
	if (!atomic_cas(&record_taken, 0, 1))
		return;
	if (!have_screen) {
		/* No screen (run()): the lines went to the UART as they were
		 * printed and nothing was kept, so the record is the OSAL's job
		 * record and the verdicts, there. */
		printk("v3d_gl_test RECORD (no screen: the UART), build %s, %lld s after boot: %s\n",
		       V3D_BUILD, (long long)(k_uptime_get() / 1000), ending);
		printk("--- the OSAL ---\n");
		osal_v3d_diag_print();
		printk("--- the verdicts ---\n");
		verdicts();
		printk("v3d_gl_test RECORD end, build %s\n", V3D_BUILD);
		return;
	}
	recording = true;
	fbcon_attach(fb.cpu, fb.width, fb.pitch, fb.height, 1); /* clears the screen */
	screen_out = __printk_get_hook();
	__printk_hook_install(tap);
	printk("v3d_gl_test RECORD, build %s, %lld s after boot: %s\n", V3D_BUILD,
	       (long long)(k_uptime_get() / 1000), ending);
	unsigned n = ring_count < REPLAY ? ring_count : REPLAY;
	printk("--- the last %u lines of the run ---\n", n);
	for (unsigned i = 0; i < n; i++)
		printk("%s\n", ring[(ring_next + RING_LINES - n + i) % RING_LINES]);
	printk("--- the OSAL ---\n");
	osal_v3d_diag_print();
	printk("--- the verdicts ---\n");
	verdicts();
	printk("v3d_gl_test RECORD end, build %s\n", V3D_BUILD);
	frozen = true;
}

static void deadline(struct k_work *w)
{
	ARG_UNUSED(w);
	take_record("INCOMPLETE (the run had not ended at the record deadline)");
}
static K_WORK_DELAYABLE_DEFINE(deadline_work, deadline);

/* ---- the verdicts, as the walker reports them ---------------------------- */

static int result_case[MAX_CASES], result_rc[MAX_CASES];
static unsigned results;

/* gl_test.c's weak hook: rc 0 pass, -1 fail, -2 no such case, -3 skipped,
 * -4 timed out (PowerVR only). */
void gl_test_case_result(int c, int rc)
{
	if (results < MAX_CASES) {
		result_case[results] = c;
		result_rc[results] = rc;
		results++;
	}
}

static const char *rc_name(int rc)
{
	switch (rc) {
	case 0: return "PASS";
	case -1: return "FAIL";
	case -2: return "no such case";
	case -3: return "SKIP";
	case -4: return "TIMEOUT";
	default: return "?";
	}
}

/* "5,58,59" -> {5, 58, 59}; -1 on anything that is not a list of numbers. */
static int parse_list(const char *s, int *out, unsigned max)
{
	unsigned n = 0;
	while (*s) {
		char *end;
		long v = strtol(s, &end, 10);
		if (end == s || v < 0 || n == max)
			return -1;
		out[n++] = (int)v;
		s = end;
		if (*s == ',')
			s++;
		else if (*s)
			return -1;
	}
	return (int)n;
}

static void run(void)
{
	int ret = rpi_mbox_init();
	if (ret == 0)
		ret = rpi_fb_alloc(FB_W, FB_H, &fb);
	if (ret) {
		/* No framebuffer (the mailbox call fails on some Pi 4: -116): run
		 * the same list with the console on the UART alone, and the
		 * record (take_record) printed there. Nothing is drawn. */
		printk("v3d_gl_test: NO SCREEN (mailbox/framebuffer %d): headless, the UART is the only output\n",
		       ret);
	} else {
		/* once: the second attach (take_record) must not take tap for the UART */
		uart_out = __printk_get_hook();
		fbcon_attach(fb.cpu, fb.width, fb.pitch, fb.height, 1);
		have_screen = true;
		screen_out = __printk_get_hook();
		__printk_hook_install(tap);
	}
	if (IS_ENABLED(CONFIG_V3D_GL_TEST_RECORD)) {
		k_work_schedule(&deadline_work, K_SECONDS(CONFIG_V3D_GL_TEST_RECORD_AT_S));
		printk("v3d_gl_test: build %s (the record at the end or at %d s)\n", V3D_BUILD,
		       CONFIG_V3D_GL_TEST_RECORD_AT_S);
	} else {
		printk("v3d_gl_test: build %s\n", V3D_BUILD);
	}
#ifdef V3D_NO_RPI_WDOG
	printk("board: NO watchdog (a QEMU build, V3D_RPI_WDOG=OFF)\n");
#else
	rpi_wdog_announce();
#endif

	/* the V3D at its highest clock, as v3d_gl_triangle */
	uint32_t hz = 0, max_hz = 0;
	if (rpi_clock_max_rate(RPI_FW_CLK_V3D, &max_hz) == 0 && max_hz &&
	    rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz != max_hz) {
		ret = rpi_clock_set_rate(RPI_FW_CLK_V3D, max_hz);
		printk("V3D clock %u -> %u Hz (%d)\n", hz, max_hz, ret);
	}

#ifdef CONFIG_V3D_GL_TEST_MSAA
	{
		extern void v3d_debug_set_msaa(int on);

		/* the 4x jobs' EGL config, for the measurement (MSAA step 3) */
		v3d_debug_set_msaa(1);
		printk("v3d_gl_test: v3d_debug_msaa=1 (the 4x EGL config on)\n");
	}
#endif
	int gl_case = CONFIG_V3D_GL_TEST_CASE;
	if (gl_case < 0) {
		static int list[MAX_CASES];
		int n = parse_list(CONFIG_V3D_GL_TEST_CASES, list, MAX_CASES);
		if (n <= 0) {
			printk("v3d_gl_test: FAIL (CONFIG_V3D_GL_TEST_CASES \"%s\" is not a list of cases)\n",
			       CONFIG_V3D_GL_TEST_CASES);
			return;
		}
		unsigned stored = 0;
		gl_case = gl_test_set_list(list, (unsigned)n, &stored);
		printk("v3d_gl_test: the list %s (%u cases) on one context\n", CONFIG_V3D_GL_TEST_CASES,
		       stored);
	}
	int64_t t0 = k_uptime_get();
	int rc = gl_test_run(gl_case, 0, 0, 0, 0, 0);
	int64_t ms = k_uptime_get() - t0;

	/* a single case does not go through the walker's hook */
	if (CONFIG_V3D_GL_TEST_CASE >= 0 && results == 0)
		gl_test_case_result(CONFIG_V3D_GL_TEST_CASE, rc);

	run_ms = ms;
	run_rc = rc;
	run_done = true;
	if (!IS_ENABLED(CONFIG_V3D_GL_TEST_RECORD))
		verdicts(); /* else the record prints them */
}

static void verdicts(void)
{
	int64_t ms = run_ms;
	int rc = run_rc;
	if (!run_done) {
		printk("v3d_gl_test: the run has not ended; %u case(s) reported so far\n", results);
	}
	printk("\n");
	unsigned passed = 0, counted = 0;
	for (unsigned i = 0; i < results; i++) {
		printk("v3d_gl_test: case %d %s\n", result_case[i], rc_name(result_rc[i]));
		if (result_rc[i] == -3)
			continue; /* neither a pass nor a failure */
		counted++;
		passed += result_rc[i] == 0;
	}
	/* An empty run is not a pass: say so. */
	if (results == 0 && run_done)
		printk("v3d_gl_test: FAIL (no case ran; gl_test_run returned %d)\n", rc);
	else if (results != 0)
		printk("v3d_gl_test: %u of %u cases passed (%lld ms)\n", passed, counted, (long long)ms);
	printk("v3d_gl_test: build %s\n", V3D_BUILD);
}

int main(void)
{
	run();
	if (IS_ENABLED(CONFIG_V3D_GL_TEST_RECORD))
		take_record("DONE");
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
