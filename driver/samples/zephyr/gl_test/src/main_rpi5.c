/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_gl_test: the GL suite on the Raspberry Pi 5's V3D 7.1.
 *
 * The Pi 4's main_rpi4.c without the screen: the console is the board's UART
 * and nothing is drawn (the Pi 5's mailbox framebuffer is not proven on the
 * board, driver/docs/rpi5-display.md). driver/tools/pvrlab/gl_test.c runs
 * through YakoGL's GL core, its in-process compiler and its V3D 7.1 back end
 * on the OSAL src/osal/zephyr_v3d71.c; the V3D clock switch is the board
 * layer's (driver/zephyr/lib/board/rpi5/rpi5_v3d.c, the platform of
 * EGL_DEFAULT_DISPLAY). The PM watchdog is the board layer's too
 * (YAKOGL_RPI5_WDOG).
 *
 * The lines tools/board.sh pi5 reads (END_RE_DEFAULT ends the session on the
 * last of them):
 *   v3d_gl_test: build <describe> ...
 *   board: PM watchdog resets the SoC ~N s after boot (...)
 *   v3d_gl_test: case <n> <PASS|FAIL|no such case|SKIP|TIMEOUT>
 *   v3d_gl_test: <p> of <c> cases passed (<ms> ms)
 *   v3d_gl_test RECORD end, build <describe>
 * With CONFIG_V3D_GL_TEST_RECORD the verdicts are in the record at the end of
 * the run, or at CONFIG_V3D_GL_TEST_RECORD_AT_S if it has not ended.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>

#include "osal_internal.h" /* osal_zephyr_v3d_default_platform, osal_v3d_diag_print */
#include "rpi_mbox.h"
#include "rpi_wdog.h"

#ifndef V3D_BUILD
#define V3D_BUILD "unknown"
#endif

#define MAX_CASES 64

#ifndef CONFIG_V3D_GL_TEST_RECORD_AT_S
#define CONFIG_V3D_GL_TEST_RECORD_AT_S 0
#endif

/* gl_test.c's entry points */
int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce, int panel);
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);

/* gl_test.c's V3D arm starts the OSAL with this: the board layer's platform,
 * the one EGL_DEFAULT_DISPLAY gets (NULL when the mailbox is not mapped; the
 * OSAL then refuses its init and says why). */
void *gl_test_v3d_osal_arg(void)
{
	return (void *)osal_zephyr_v3d_default_platform();
}

static atomic_t record_taken;
static void verdicts(void); /* below */
static int64_t run_ms;
static int run_rc;
static bool run_done;

static void take_record(const char *ending)
{
	if (!atomic_cas(&record_taken, 0, 1))
		return;
	printk("v3d_gl_test RECORD (the UART), build %s, %lld s after boot: %s\n", V3D_BUILD,
	       (long long)(k_uptime_get() / 1000), ending);
	printk("--- the OSAL ---\n");
	osal_v3d_diag_print();
	printk("--- the verdicts ---\n");
	verdicts();
	printk("v3d_gl_test RECORD end, build %s\n", V3D_BUILD);
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
	if (IS_ENABLED(CONFIG_V3D_GL_TEST_RECORD)) {
		k_work_schedule(&deadline_work, K_SECONDS(CONFIG_V3D_GL_TEST_RECORD_AT_S));
		printk("v3d_gl_test: build %s (the record at the end or at %d s)\n", V3D_BUILD,
		       CONFIG_V3D_GL_TEST_RECORD_AT_S);
	} else {
		printk("v3d_gl_test: build %s\n", V3D_BUILD);
	}
	rpi_wdog_announce();

	/* the V3D at its highest clock, as v3d_gl_triangle */
	int ret = rpi_mbox_init();
	uint32_t hz = 0, max_hz = 0;
	if (ret == 0 && rpi_clock_max_rate(RPI_FW_CLK_V3D, &max_hz) == 0 && max_hz &&
	    rpi_clock_rate(RPI_FW_CLK_V3D, &hz) == 0 && hz != max_hz) {
		ret = rpi_clock_set_rate(RPI_FW_CLK_V3D, max_hz);
		printk("V3D clock %u -> %u Hz (%d)\n", hz, max_hz, ret);
	}

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

	if (!run_done)
		printk("v3d_gl_test: the run has not ended; %u case(s) reported so far\n", results);
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
