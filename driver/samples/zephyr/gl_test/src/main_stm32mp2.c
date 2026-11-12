/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gl_test on the STM32MP257F-DK's A35: tools/pvrlab/gl_test.c on the DK's
 * Vivante GCNanoUltra31 (model 0x8000, revision 0x6205; #1281), through the
 * board layer (yakogl_board.h) as gc_fence and gc_clear are, not the
 * i.MX8MP's yakogl_gc_board.
 *
 * Before main() the board layer has powered the GPU for the driver
 * (stm32mp2/gc_power_mp2.c: SCMI over OP-TEE, the RCC; it refuses any part
 * but 0x8000/0x6205 with a PSCI reset) and armed OP-TEE's watchdog, which it
 * feeds until CONFIG_YAKOGL_STM32MP2_LIFETIME_S; there the layer resets the
 * board whatever the suite is doing. board_finish() ends the run with the
 * record and PSCI SYSTEM_RESET back into ST's Linux.
 *
 * What runs: CONFIG_GL_TEST_MP2_CASE (7, the regression set, by default),
 * or with CONFIG_GL_TEST_MP2_SET_HEAD only the first
 * CONFIG_GL_TEST_MP2_SET_HEAD_CASES cases of the regression set, in its
 * order (1: case 1, "uniform", alone), so a first board run is one small
 * case rather than fifty.
 *
 * The console has every case's "gl_test: case N <name>: PASS/FAIL" line and
 * "gl_test: all:"; the record (yk-record on USART2) is kind CASE, as the
 * i.MX8MP's SNVS record: one case w1 = case << 16 | first bad check << 8 |
 * status (CASE_PASS, CASE_CHECK, CASE_NO_CHECK; check 255 none), w2/w3 the
 * checks made and 0 on a pass, else what the bad check read and wanted; a
 * set or a head of more than one case the set summary (CASE_SET); no GPU
 * CASE_NO_DEVICE with w2 = 0 (the device) or 1 (gc_render_init).
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>

#include <yakogl_board.h>
#include <yakogl_record.h>

#include <gc.h>
#include <gc_render.h>

#include "osal_internal.h" /* osal_zephyr_diag_print */
#include "stm32mp2.h"      /* mp2_lifetime_ms */

#define NAME "gl_test"

int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce, int panel);
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);
unsigned gl_test_set_cases(const int **cases);

extern unsigned int gl_test_passed, gl_test_total, gl_test_skipped;
extern int gl_test_first_fail, gl_test_second_fail, gl_test_first_skip;
extern volatile int gl_test_check_no, gl_test_first_bad_check;
extern volatile uint32_t gl_test_first_bad_got, gl_test_first_bad_want;

/* kind CASE's w1 (driver/rust/src/board/record.rs, case_w1) */
static uint32_t case_w1(int gl_case, int bad_check, uint32_t status)
{
	uint32_t check = bad_check < 0 || bad_check > 254 ? 255u : (uint32_t)bad_check;

	return ((uint32_t)gl_case & 0xffffu) << 16 | check << 8 | (status & 0xffu);
}

/* a case number in a set summary's field, CASE_SET_NONE when there is none */
static uint32_t set_case(int c)
{
	return c < 0 ? YK_RECORD_CASE_SET_NONE : MIN((uint32_t)c, YK_RECORD_CASE_SET_NONE - 1u);
}

static void report(int gl_case, bool one, int rc)
{
	if (!one) {
		/* record.rs SetSummary::words: run = total minus skipped */
		uint32_t run = gl_test_total - MIN(gl_test_skipped, gl_test_total);

		board_result(YK_RECORD_KIND_CASE,
			     set_case(gl_test_first_fail) << 16 | MIN(gl_test_skipped, 0xffu) << 8 |
				     YK_RECORD_CASE_SET,
			     set_case(gl_test_second_fail) << 16 | MIN(gl_test_passed, 0xffffu),
			     set_case(gl_test_first_skip) << 16 | MIN(run, 0xffffu));
	} else if (rc == 0) {
		board_result(YK_RECORD_KIND_CASE, case_w1(gl_case, -1, YK_RECORD_CASE_PASS),
			     (uint32_t)gl_test_check_no, 0);
	} else if (gl_test_first_bad_check < 0) {
		board_result(YK_RECORD_KIND_CASE, case_w1(gl_case, -1, YK_RECORD_CASE_NO_CHECK),
			     (uint32_t)gl_test_check_no, 0);
	} else {
		board_result(YK_RECORD_KIND_CASE,
			     case_w1(gl_case, gl_test_first_bad_check, YK_RECORD_CASE_CHECK),
			     gl_test_first_bad_got, gl_test_first_bad_want);
	}
}

static int run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	int gl_case = CONFIG_GL_TEST_MP2_CASE;
	bool one = gl_case != 7;
	int rc;

#ifdef CONFIG_GL_TEST_MP2_SET_HEAD
	{
		const int *set = NULL;
		unsigned n = MIN(gl_test_set_cases(&set), (unsigned)CONFIG_GL_TEST_MP2_SET_HEAD_CASES);

		if (n == 1u) {
			/* one case alone, without the walk's list */
			gl_case = set[0];
			one = true;
		} else {
			unsigned stored = 0;

			gl_case = gl_test_set_list(set, n, &stored);
			n = stored;
			one = false;
		}
		printk(NAME ": the first %u case(s) of the regression set, from case %d\n", n, set[0]);
	}
#endif
	printk(NAME ": case %d on the STM32MP257F-DK's GPU (lifetime %u ms)\n", gl_case,
	       mp2_lifetime_ms());

	board_progress(YK_RECORD_WHERE_STAGE | YK_RECORD_STAGE_MAIN);
	if (!device_is_ready(gpu) || !gc_alive(gpu)) {
		printk(NAME ": FAIL: the GPU device did not initialise\n");
		board_result(YK_RECORD_KIND_CASE, case_w1(gl_case, -1, YK_RECORD_CASE_NO_DEVICE), 0, 0);
		return -ENODEV;
	}
	/* gl_test_run() brings the OSAL and the backend up itself; the render
	 * service has to exist first because the OSAL forwards to it. */
	rc = gc_render_init(gpu);
	if (rc) {
		printk(NAME ": FAIL: gc_render_init: %d\n", rc);
		board_result(YK_RECORD_KIND_CASE, case_w1(gl_case, -1, YK_RECORD_CASE_NO_DEVICE), 1, 0);
		return rc;
	}
	board_progress(YK_RECORD_WHERE_STAGE | YK_RECORD_STAGE_RENDER_INIT_DONE);

	{
		int64_t t0 = k_uptime_get();

		rc = gl_test_run(gl_case, 0, 0, 0, 0, 0);
		printk(NAME ": the suite took %lld ms\n", (long long)(k_uptime_get() - t0));
	}
	/* the OSAL's close record: the pending-free queue's counters */
	osal_zephyr_diag_print();
	report(gl_case, one, rc);
	if (rc != 0 && gc_render_ready()) {
		gc_render_stall_dump(NAME " failed");
	}
	return rc;
}

int main(void)
{
	board_finish(NAME, run());
}
