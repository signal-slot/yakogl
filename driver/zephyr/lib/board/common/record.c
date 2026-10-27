/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The result record's writer on every board (yakogl_board.h). The format and
 * the rules (the sequence number, breadcrumbs dropped once a result was
 * stored) are driver/rust/src/board/record.rs, reached through yd_record_*.
 * This file keeps the image's one writer, hands each record to the board's
 * sink (board_sink_store) and prints it as the yk-record console line.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <yakogl_drivers.h>
#include "yakogl_board.h"
#include "yakogl_record.h"
#include "board_internal.h"

static struct yd_record_writer s_writer;

/*
 * Interrupts are locked around the writer and the sink, not a spinlock: the
 * fatal handler stores a record too, and it may have interrupted a store.
 * The line is printed after the lock, through printk, so that no log level
 * or log backend can drop it.
 */
static void emit(int store, const uint32_t w[4], unsigned int key)
{
	if (store) {
		board_sink_store(w);
	}
	irq_unlock(key);
	if (store) {
		printk(YK_RECORD_LINE_FMT, w[0], w[1], w[2], w[3]);
	}
}

void board_result(unsigned int kind, uint32_t w1, uint32_t w2, uint32_t w3)
{
	uint32_t w[4];
	unsigned int key = irq_lock();

	emit(yd_record_result(&s_writer, kind, w1, w2, w3, w), w, key);
}

void board_result_tag(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2, uint32_t w3)
{
	uint32_t w[4];
	unsigned int key = irq_lock();

	emit(yd_record_result_tag(&s_writer, kind, tag, w1, w2, w3, w), w, key);
}

void board_progress(uint32_t where)
{
	uint32_t w[4];
	unsigned int key = irq_lock();

	emit(yd_record_progress(&s_writer, where, k_uptime_get_32(), w), w, key);
}

void board_record_breadcrumb(unsigned int kind, uint8_t tag, uint32_t w1, uint32_t w2,
			     uint32_t w3)
{
	uint32_t w[4];
	unsigned int key = irq_lock();

	emit(yd_record_breadcrumb(&s_writer, kind, tag, w1, w2, w3, w), w, key);
}

bool board_record_have_result(void)
{
	return s_writer.have_result != 0u;
}
