/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dEQP on a Mali board: the board's part of the image. Two boards:
 *
 * - the NanoPC-T6's Mali-G610, kexec'd out of Armbian
 *   (driver/tools/kexec-zephyr/nanopc-t6.sh). The rest is the board glue
 *   every nanopc_t6 image has (driver/zephyr/lib/board/nanopc_t6): WDT0
 *   armed before main() and fed until CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S,
 *   the CSF firmware handed to the Mali OSAL (nanopc_t6_mali.c), and
 *   t6_reset_to_linux();
 * - the Verdin iMX95's Mali-G310, however the run script starts it (U-Boot's
 *   `go` on this board; the boot route is not the image's). The rest is the
 *   board glue and the board layer every verdin_imx95 image has
 *   (driver/zephyr/lib/board/verdin_imx95): WDOG3 armed at PRE_KERNEL_1 and
 *   fed until CONFIG_YAKOGL_BOARD_LIFETIME_S while the alive window holds,
 *   the CSF firmware and the GPU's SCMI power-on (verdin_imx95_mali.c), and
 *   board_finish(): the yk-record line and PSCI SYSTEM_RESET.
 *
 * dEQP renders into its pbuffer; neither image brings a display up, the
 * record is the console UART.
 *
 * - The build, before the first case (the other boards print theirs from
 *   sample_glue.c or v3d_board.c; this image has neither).
 * - The watchdog: deqp_log.c calls osal_zephyr_mali_board_feed() when a case
 *   begins and when it ends, and the OSAL calls it in every delay, so a case
 *   stuck on the CPU for longer than the board's alive window
 *   (CONFIG_YAKOGL_NANOPC_T6_ALIVE_S, CONFIG_YAKOGL_BOARD_ALIVE_S) ends in
 *   the watchdog's reset, not in a hang.
 * - The end, which the deqp main calls after its summary: the OSAL's close
 *   line, "deqp: OK|FAILED (rc=N)" as the gl_test image ends, then the
 *   board's end: on the T6 pvr_sample_finish() and the PSCI reset back into
 *   Armbian; on the Verdin iMX95 mali_deqp_finish() and board_finish() (the
 *   board layer's sample_glue.c has a pvr_sample_finish of its own, which
 *   holds and returns, so this one has another name there).
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include "osal_internal.h" /* osal_mali_diag_print */
#include "sample_glue.h"   /* pvr_sample_finish */

#if defined(CONFIG_YAKOGL_NANOPC_T6)
#include <nanopc_t6.h>
#ifndef T6_DEQP_BUILD
#define T6_DEQP_BUILD "unknown"
#endif
#define MALI_DEQP_TAG   "t6_deqp"
#define MALI_DEQP_BUILD T6_DEQP_BUILD
#elif defined(CONFIG_YAKOGL_BOARD_VERDIN_IMX95)
#include <yakogl_board.h>
#ifndef VI95_DEQP_BUILD
#define VI95_DEQP_BUILD "unknown"
#endif
#define MALI_DEQP_TAG   "vi95_deqp"
#define MALI_DEQP_BUILD VI95_DEQP_BUILD
FUNC_NORETURN void mali_deqp_finish(int rc); /* src/main.cpp calls it */
#else
#error "deqp: src/mali_board.c knows the NanoPC-T6 and the Verdin iMX95 (with the board layer)"
#endif

static int mali_deqp_report(void)
{
	printk(MALI_DEQP_TAG ": build %s\n", MALI_DEQP_BUILD);
	return 0;
}
SYS_INIT(mali_deqp_report, APPLICATION, 1);

#if defined(CONFIG_YAKOGL_NANOPC_T6)
int pvr_sample_finish(int rc)
#else
FUNC_NORETURN void mali_deqp_finish(int rc)
#endif
{
	/* the OSAL's close record: kicks, carve-out pages, frees, and the sched
	 * refusal that stopped it, if one did */
	osal_mali_diag_print();
	printk("deqp: %s (rc=%d)\n", rc ? "FAILED" : "OK", rc);
#if defined(CONFIG_YAKOGL_NANOPC_T6)
	t6_reset_to_linux("deqp done");
#else
	/* "deqp: PASS|FAIL", the yk-record line, then PSCI SYSTEM_RESET
	 * (board_verdin_imx95.c, board_reset) */
	board_finish("deqp", rc);
#endif
}
