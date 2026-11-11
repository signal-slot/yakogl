/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the STM32MP257F-DK's A35 (yakogl_board.h;
 * CONFIG_YAKOGL_BOARD_STM32MP2), for the samples that draw nothing on a
 * screen yet (gc_fence, gc_clear; plan #322, issues #7 and #1281):
 *   window   none yet: board_window() refuses (-ENOTSUP)
 *   screen   none yet: the display (LTDC, DSI, ADV7535, HDMI) is a later plan
 *   record   the yk-record line on USART2 (common/record.c)
 *   watchdog OP-TEE's arm,smc-wdt, armed at boot by stm32mp2_wdog.c
 *            (its "gc_wdog_arm: armed" line), and the layer's lifetime
 *            (CONFIG_YAKOGL_STM32MP2_LIFETIME_S)
 *   reset    mp2_reset_to_linux(): PSCI SYSTEM_RESET, back into ST's Linux
 */
#include <zephyr/kernel.h>
#include <errno.h>
#include <string.h>

#include "yakogl_board.h"
#include "board_internal.h"
#include "stm32mp2.h"

BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "a board-layer image on the DK runs under OP-TEE's watchdog");

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	ARG_UNUSED(w);
	ARG_UNUSED(h);
	memset(out, 0, sizeof(*out));
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
	mp2_reset_to_linux("board: run over");
}
