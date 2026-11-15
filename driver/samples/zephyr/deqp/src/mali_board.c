/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dEQP on the NanoPC-T6's Mali-G610, kexec'd out of Armbian
 * (driver/tools/kexec-zephyr/nanopc-t6.sh): the board's part of the image.
 *
 * The rest is the board glue every nanopc_t6 image has
 * (driver/zephyr/lib/board/nanopc_t6): WDT0 armed before main() and fed
 * until CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S, the CSF firmware handed to the
 * Mali OSAL (nanopc_t6_mali.c), and t6_reset_to_linux(). dEQP renders into
 * its pbuffer; the board has no display on Zephyr, the record is uart2.
 *
 * - The build, before the first case (the other boards print theirs from
 *   sample_glue.c or v3d_board.c; this image has neither).
 * - The watchdog: deqp_log.c calls t6_watchdog_feed() when a case begins and
 *   when it ends, and the OSAL feeds it in every delay, so a case stuck on
 *   the CPU for longer than CONFIG_YAKOGL_NANOPC_T6_ALIVE_S ends in WDT0's
 *   reset, not in a hang.
 * - The end (pvr_sample_finish, which the deqp main calls after its summary):
 *   the OSAL's close line, "deqp: OK|FAILED (rc=N)" as the gl_test image
 *   ends, then the PSCI reset back into Armbian.
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <nanopc_t6.h>
#include "osal_internal.h" /* osal_mali_diag_print */
#include "sample_glue.h"   /* pvr_sample_finish */

#ifndef T6_DEQP_BUILD
#define T6_DEQP_BUILD "unknown"
#endif

static int t6_deqp_report(void)
{
	printk("t6_deqp: build %s\n", T6_DEQP_BUILD);
	return 0;
}
SYS_INIT(t6_deqp_report, APPLICATION, 1);

int pvr_sample_finish(int rc)
{
	/* the OSAL's close record: kicks, carve-out pages, frees, and the sched
	 * refusal that stopped it, if one did */
	osal_mali_diag_print();
	printk("deqp: %s (rc=%d)\n", rc ? "FAILED" : "OK", rc);
	t6_reset_to_linux("deqp done");
}
