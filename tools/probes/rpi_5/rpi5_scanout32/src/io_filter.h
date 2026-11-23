/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * io_filter.h: the one rule of what rpi5_scanout32 may write to the HVS.
 *
 * Only list memory at word >= HVS6S_KEPT (the firmware's first words stay
 * untouched) and channel 0's LPTRS register. Every other HVS offset
 * (DISPCTRL, the channel CTRL/STAT/DL registers, other channels' LPTRS,
 * list words below HVS6S_KEPT) is refused. Pure, header-only; included by
 * main.c's io_write32 and by io_filter_selftest.sh. The includer defines
 * HVS_SIZE and CHAN first (main.c does; the selftest does).
 */
#ifndef RPI5_SCANOUT32_IO_FILTER_H
#define RPI5_SCANOUT32_IO_FILTER_H

#include <stdbool.h>
#include <stdint.h>

#include "hvs6_dlist_swap.h"

static inline bool scanout32_write_ok(uint32_t off)
{
	return off == hvs6s_lptrs_reg(CHAN) ||
	       (off >= hvs6s_word_off(HVS6S_KEPT) && off < HVS_SIZE);
}

#endif /* RPI5_SCANOUT32_IO_FILTER_H */
