/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The record screen's ring (board_internal.h, board_record_screen_draw): a
 * printk hook in front of the console's keeps the last lines, and the board
 * draws them over the picture on its screen when the run ends. The rules
 * (which lines are kept, where they go on the screen) are the yakogl-drivers
 * crate's (board::record_screen); the font is the panel console's.
 *
 * What reaches the ring is what reaches printk: the image's own lines and,
 * with CONFIG_LOG_MODE_MINIMAL, Zephyr's log lines (a fatal error's
 * registers). Lines printed before the hook is in place are not in it.
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include <yakogl_drivers.h>

#include "board_internal.h"
#include "gc_font8x16.h"

BUILD_ASSERT(ARRAY_SIZE(gc_font8x16) == YD_RECORD_SCREEN_GLYPHS && GC_FONT_FIRST == 0x20 &&
		     GC_FONT_W == 8 && GC_FONT_H == YD_RECORD_SCREEN_FONT_H,
	     "the font is the one board::record_screen draws");

static struct yd_record_screen s_ring;
static printk_hook_fn_t s_prev;

/* printk has no lock of its own, and a timer handler prints too: one put at
 * a time, and none during a draw. */
static int ring_out(int c)
{
	unsigned int key = irq_lock();

	yd_record_screen_put(&s_ring, (uint32_t)c);
	irq_unlock(key);
	return s_prev ? s_prev(c) : c;
}

static int ring_init(void)
{
	/* After the UART console's own hook (PRE_KERNEL_1, priority 60): chain. */
	s_prev = __printk_get_hook();
	__printk_hook_install(ring_out);
	return 0;
}
SYS_INIT(ring_init, PRE_KERNEL_1, 90);

int board_record_screen_draw(void *px, uint32_t width, uint32_t height, uint32_t stride)
{
	char head[64];
	int n = snprintk(head, sizeof(head), "board: record screen, %lld s after boot",
			 (long long)(k_uptime_get() / 1000));
	unsigned int key = irq_lock();
	int rc = yd_record_screen_draw(&s_ring, px, width, height, stride, &gc_font8x16, head,
				       MIN((size_t)MAX(n, 0), sizeof(head) - 1u));

	irq_unlock(key);
	return rc;
}
