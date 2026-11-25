/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The board layer on the Raspberry Pi 5 (BCM2712), for the images that are
 * on the common part of it (the glmark2 image): the part of yakogl_board.h
 * that has no screen yet.
 *   window/show  refused with -ENODEV: the Pi 5's mailbox framebuffer
 *                (rpi5/board_rpi5.c, YAKOGL_BOARD_RPI5) is not proven on
 *                the board, so the board layer gives no window yet
 *   record       the yk-record line on the console UART (common/record.c)
 *   watchdog     the PM watchdog (rpi4/rpi_wdog.c, the SoC a parameter),
 *                fed until YAKOGL_BOARD_LIFETIME_S
 *   reset        the feeding stops (board_wdog_release); the PM watchdog
 *                resets the SoC within its 10 s period and tryboot returns
 *                to Linux
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <string.h>

#include "yakogl_board.h"
#include "board_internal.h"

int board_window(uint32_t w, uint32_t h, struct board_window *out)
{
	ARG_UNUSED(w);
	ARG_UNUSED(h);
	if (out != NULL) {
		memset(out, 0, sizeof(*out));
	}
	printk("board: rpi5 has no window yet (no screen in the board layer)\n");
	return -ENODEV;
}

int board_show(const struct board_window *win)
{
	ARG_UNUSED(win);
	return -ENODEV;
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
	printk("board: feeding stopped; the PM watchdog resets the SoC\n");
	board_wdog_release();
	for (;;) {
		k_sleep(K_FOREVER);
	}
}
