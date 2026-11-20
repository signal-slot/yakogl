/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Verdin iMX95's A55 (yakogl_board.h;
 * CONFIG_YAKOGL_BOARD_VERDIN_IMX95), so that the examples build for the
 * board (examples/zephyr.cmake):
 *   window   none yet: board_window() refuses (-ENOTSUP) with a line
 *   screen   none yet: the i.MX 95 display path (DPU, LVDS/MIPI DSI on the
 *            Mallow's connectors) has no Zephyr driver of ours
 *   record   the yk-record line on the console UART (common/record.c)
 *   watchdog none: STUB (verdin_imx95.c, verdin_imx95_wdog_stub)
 *   reset    verdin_imx95_reset(): PSCI SYSTEM_RESET
 * Built, not run on a board yet.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <string.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "verdin_imx95.h"

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	memset(out, 0, sizeof(*out));
	printk("verdin_imx95: no display yet: board_window(%u x %u) refused\n", w, h);
	return -ENOTSUP;
}

int board_show(const struct board_window *win)
{
	ARG_UNUSED(win);
	board_first_frame();
	return -ENOTSUP;
}

int board_selector(void)
{
	return -1;
}

void board_sink_store(const uint32_t w[4])
{
	ARG_UNUSED(w); /* the UART line is the sink */
}

FUNC_NORETURN void board_reset(void)
{
	verdin_imx95_reset("board: run over");
}
