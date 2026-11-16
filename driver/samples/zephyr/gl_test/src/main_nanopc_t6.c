/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test on the NanoPC-T6's Mali-G610, kexec'd out of Armbian
 * (driver/docs/nanopc-t6-mali.md, "Z1"; driver/tools/kexec-zephyr/nanopc-t6.sh).
 *
 * driver/tools/pvrlab/gl_test.c runs through YakoGL's GL core, its in-process
 * compiler and its Mali backend, on the Zephyr OSAL src/osal/zephyr_mali.c
 * over our own kernel side (board::nanopc_t6_osal: Z1 to Z3, then CSG 0 /
 * CS 0, each kick one job on its ring). The suite renders into its own
 * surfaces and reads them back; the board has no display on Zephyr.
 *
 * What this file adds to the suite is the board's part only:
 *
 * - the CSF firmware is the board glue's (driver/zephyr/lib/board/nanopc_t6/
 *   nanopc_t6_mali.c, in every Mali image): it embeds
 *   driver/src/fw/mali_csffw_blob.h and is the OSAL's default platform, which
 *   gl_test.c's Mali arm takes with init(NULL), as eglInitialize does;
 * - the watchdog: the board glue arms WDT0 before main() and feeds it until
 *   CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S; the OSAL feeds it in every delay, and
 *   every case's end feeds it here, so a case stuck on the CPU for longer than
 *   CONFIG_YAKOGL_NANOPC_T6_ALIVE_S ends in WDT0's reset, not in a hang;
 * - the record: the build first, the runner's lines as on every board
 *   ("gl_test: case N <name>: PASS/FAIL/SKIP/TIMEOUT", "gl_test: all: ..."),
 *   the OSAL's close line (osal_mali_diag_print), "gl_test: OK|FAILED (rc=N)"
 *   as the AM62P's image ends, then the board glue's PSCI reset back into
 *   Armbian (t6_reset_to_linux).
 */
#include <stdlib.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <nanopc_t6.h>
#include "osal_internal.h" /* osal_mali_diag_print */

#ifndef T6_GL_TEST_BUILD
#define T6_GL_TEST_BUILD "unknown"
#endif

#define MAX_CASES 64

/* gl_test.c's entry points */
int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce, int panel);
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);

/* gl_test.c's weak hook, called by the runner after each case: the case
 * ended, so the run is still making progress. */
void gl_test_case_result(int c, int rc)
{
	ARG_UNUSED(c);
	ARG_UNUSED(rc);
	t6_watchdog_feed();
}

/* CONFIG_T6_GL_TEST_CASES, "1,2,3": the case numbers, or -1 when it is not
 * such a list (as main_am62p.c's parse_list). */
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

static int run(void)
{
	int gl_case = CONFIG_T6_GL_TEST_CASE;

	if (CONFIG_T6_GL_TEST_CASES[0] != '\0') {
		static int list[MAX_CASES];
		int n = parse_list(CONFIG_T6_GL_TEST_CASES, list, MAX_CASES);
		unsigned stored = 0;

		if (n <= 0) {
			printk("gl_test: FAIL (CONFIG_T6_GL_TEST_CASES \"%s\" is not a list of cases)\n",
			       CONFIG_T6_GL_TEST_CASES);
			return -1;
		}
		gl_case = gl_test_set_list(list, (unsigned)n, &stored);
		printk("gl_test: the list %s (%u cases) on one context\n", CONFIG_T6_GL_TEST_CASES,
		       stored);
	}
	return gl_test_run(gl_case, 0, 0, 0, 0, 0);
}

int main(void)
{
	int rc;

	printk("t6_gl_test: build %s\n", T6_GL_TEST_BUILD);
	printk("t6_gl_test: WDT0 period %u ms, fed until %u ms of uptime, alive window %u s\n",
	       t6_watchdog_period_ms(), t6_watchdog_deadline_ms(),
	       (unsigned int)CONFIG_YAKOGL_NANOPC_T6_ALIVE_S);
	t6_watchdog_feed();
	rc = run();
	/* the OSAL's close record: kicks, carve-out pages, frees, restarts, the
	 * tiler heap, and the sched refusal that stopped it, if one did */
	osal_mali_diag_print();
	printk("gl_test: %s (rc=%d)\n", rc ? "FAILED" : "OK", rc);
	t6_reset_to_linux("gl_test done");
}
