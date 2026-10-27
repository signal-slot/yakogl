/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * board_internal.h -- what the common part of the board layer
 * (common/record.c, common/board.c) and each board's part give each other.
 * Not for samples: they include yakogl_board.h only.
 */
#ifndef YAKOGL_BOARD_INTERNAL_H
#define YAKOGL_BOARD_INTERNAL_H

#include <stdbool.h>
#include <stdint.h>
#include <zephyr/toolchain.h>

/* ---- each board gives these ---------------------------------------------- */

/* Keep one record in the board's sink (the i.MX8MP SNVS; nothing on the
 * AM62P and the Pi 4, whose sink is the console line record.c prints).
 * Called with interrupts locked, from any context. */
void board_sink_store(const uint32_t w[4]);

/* Reset the board after board_finish()'s hold. Does not return. */
FUNC_NORETURN void board_reset(void);

/* board_finish()'s last step before the hold, after the verdict record: a
 * board's own diagnostic record (the i.MX8MP's DISPLAY). `pass` is the
 * run's verdict. Optional: common/board.c has an empty weak one. */
void board_finish_record(bool pass);

/* ---- common/record.c gives these ----------------------------------------- */

/* A breadcrumb of any kind (`tag` in w0's low byte, the sequence untouched),
 * dropped once a result was stored: the i.MX8MP's STAGE and WDOG-armed
 * records. Any context. */
void board_record_breadcrumb(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2,
			     uint32_t w3);

/* True once a result record was stored this run. */
bool board_record_have_result(void);

/* ---- common/board.c gives these ------------------------------------------ */

/* Whether the sample still says it makes progress (board_watchdog_feed):
 * true until its first call, then while the calls come at most
 * CONFIG_YAKOGL_BOARD_ALIVE_S apart. Every board's feeder asks it; it only
 * reads, so a timer handler may call it. Always true without
 * CONFIG_YAKOGL_BOARD. */
#ifdef CONFIG_YAKOGL_BOARD
bool board_wdog_alive(void);
/* From now on board_wdog_alive() says no: the feeders stop and the watchdog
 * resets the board within one period (the Pi 4's board_reset). */
void board_wdog_release(void);
/* board_show() is about to copy a w x h window into a screen-sized buffer
 * (the fallback of yakogl_board.h): counted for board_show_copies(), and
 * the first one printed with `why`. Main only. */
void board_show_copied(uint32_t w, uint32_t h, const char *why);
#else
static inline bool board_wdog_alive(void)
{
	return true;
}
#endif

/* The literal every board prints once its watchdog is armed; the runner
 * (tools/board.sh) requires it in the image. */
#define BOARD_WDOG_ARMED_LINE "board: watchdog armed\n"

#endif /* YAKOGL_BOARD_INTERNAL_H */
