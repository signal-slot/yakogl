/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test on Zephyr: runs tools/pvrlab/gl_test.c (the on-silicon YakoGL
 * test suite of the Linux harness) on the real GPU, then holds the picture
 * and resets like the other samples. Results go to the console as
 * "gl_test: case N <name>: PASS/FAIL" and "gl_test: all: N of 20 cases failed".
 */
#include <stdlib.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include "sample_glue.h"
#include "osal_internal.h" /* osal_zephyr_diag_print */

int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce,
		unsigned int term0, int panel);
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);

/* CONFIG_GL_TEST_CASES, "169,162,...": the case numbers, or -1 when it is
 * not such a list (as v3d_gl_test's parse_list). */
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

/* The panel modes draw into the same scan-out buffer the Linux harness uses
 * (tidss fb0, XRGB8888 1024x600, pitch 4096); on this board that memory is
 * outside Zephyr's own and the DSS application plane is pointed at it. */
#define PANEL_PHYS 0xE2900000UL
#define PANEL_W 1024
#define PANEL_H 600

#ifdef CONFIG_YAKOGL_GPU_POWERVR
#include "pvr_backend.h"   /* pvr_backend_set_option, PVR_BACKEND_OPT_DBIAS_INT */
void pvr_debug_set_masked_clear(int on);
#endif

int main(void)
{
#ifdef CONFIG_YAKOGL_GPU_POWERVR
	/* on by default (board-verified); n is the off arm of an A/B */
	pvr_debug_set_masked_clear(IS_ENABLED(CONFIG_GL_TEST_PVR_MASKED_CLEAR));
	printk("gl_test: pvr_debug_masked_clear=%d (clears honour glColorMask)\n",
	       IS_ENABLED(CONFIG_GL_TEST_PVR_MASKED_CLEAR));
	/* the A/B switch: -1 leaves the backend's default, and says so */
	if (CONFIG_GL_TEST_PVR_DBIAS_INT >= 0)
		pvr_backend_set_option(PVR_BACKEND_OPT_DBIAS_INT, (uint32_t)CONFIG_GL_TEST_PVR_DBIAS_INT);
	printk("gl_test: PVR_BACKEND_OPT_DBIAS_INT=%d (%s)\n", CONFIG_GL_TEST_PVR_DBIAS_INT,
	       CONFIG_GL_TEST_PVR_DBIAS_INT >= 0 ? "set" : "-1: untouched");
	/* on by default; CONFIG_GL_TEST_PVR_KEEP_ZS is the opt-out */
	pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, !IS_ENABLED(CONFIG_GL_TEST_PVR_KEEP_ZS));
	printk("gl_test: PVR_BACKEND_OPT_SWAP_DROPS_ZS=%d\n", !IS_ENABLED(CONFIG_GL_TEST_PVR_KEEP_ZS));
#endif
	int rc;

	if (CONFIG_GL_TEST_PANEL) {
		rc = pvr_sample_show_on_dss(PANEL_PHYS, PANEL_W, PANEL_H, PANEL_W * 4);
		if (rc) {
			return pvr_sample_finish(rc);
		}
	}
	int gl_case = CONFIG_GL_TEST_CASE;

	if (CONFIG_GL_TEST_CASES[0] != '\0') {
		static int list[64];
		int n = parse_list(CONFIG_GL_TEST_CASES, list, 64);
		unsigned stored = 0;

		if (n <= 0) {
			printk("gl_test: FAIL (CONFIG_GL_TEST_CASES \"%s\" is not a list of cases)\n",
			       CONFIG_GL_TEST_CASES);
			return pvr_sample_finish(-1);
		}
		gl_case = gl_test_set_list(list, (unsigned)n, &stored);
		printk("gl_test: the list %s (%u cases) on one context\n", CONFIG_GL_TEST_CASES, stored);
	}
	rc = gl_test_run(gl_case, IS_ENABLED(CONFIG_GL_TEST_PRETRANSFORM), 0,
			 CONFIG_GL_TEST_DEBUG, 0, 0, CONFIG_GL_TEST_PANEL);
	/* the OSAL's close record: the pending-free queue's counters */
	osal_zephyr_diag_print();
	printk("gl_test: %s (rc=%d)\n", rc ? "FAILED" : "OK", rc);
	return pvr_sample_finish(rc);
}
