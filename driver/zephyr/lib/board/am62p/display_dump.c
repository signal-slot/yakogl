/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin AM62P display path's registers on the console, in the format of
 * driver/docs/am62p-display-dump-linux.txt ("ADDRESS WIDTH VALUE"), so a run
 * can be diffed against the dump taken while Linux drove the panel. The read
 * list is driver/docs/am62p-display-cold.md, "Board plan (a)": every address
 * is safe to read (no read-to-clear, no FIFO pop); the DSI SEND / RD_INIT /
 * WRDAT / FIFO_RST / RDDAT and *_CLR registers are skipped as there.
 *
 * CONFIG_YAKOGL_SAMPLE_DISPLAY_DUMP; called by sample_glue.c with the counters.
 * Reads only: nothing here changes the display.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/printk.h>
#include <zephyr/devicetree.h>
#include <yakogl_drv/sn65dsi8x.h>
#include "sample_glue.h"

struct range {
	uint32_t phys;
	uint16_t first, last; /* offsets, inclusive, 4-byte steps */
};

/* SPRUJ83D 14.8.9.1 (DSS), 14.8.9.2 (DSI, wrapper), 14.8.6.3 (D-PHY). */
static const struct range ranges[] = {
	{0x30220000, 0x000, 0x0a0}, /* DSS1 common */
	{0x30222000, 0x020, 0x03c}, /* VIDL1 */
	{0x30222000, 0x1fc, 0x230},
	{0x30226000, 0x020, 0x03c}, /* VID */
	{0x30226000, 0x1fc, 0x230},
	{0x30228000, 0x000, 0x028}, /* OVR2 */
	{0x3022b000, 0x000, 0x058}, /* VP2 */
	{0x30270000, 0x000, 0x010}, /* DSI wrapper */
	{0x30500000, 0x000, 0x07c}, /* DSI controller */
	{0x30500000, 0x084, 0x088},
	{0x30500000, 0x0a4, 0x0a8},
	{0x30500000, 0x0b0, 0x0f8},
	{0x30500000, 0x100, 0x114},
	{0x30500000, 0x130, 0x148},
	{0x30500000, 0x170, 0x188},
	{0x30500000, 0x1a0, 0x1ac},
	{0x30500000, 0x1fc, 0x1fc},
	{0x301c0000, 0x000, 0x014}, /* D-PHY TX */
	{0x301c0000, 0x020, 0x020},
	{0x301c0000, 0x040, 0x040},
	{0x301c0000, 0x04c, 0x050},
	{0x301c0000, 0x154, 0x154},
	{0x301c0000, 0x26c, 0x26c},
	{0x301c0000, 0x36c, 0x36c},
	{0x301c0000, 0x46c, 0x46c},
	{0x301c0000, 0x56c, 0x56c},
	{0x301c0000, 0xb00, 0xb04},
	{0x301c0000, 0x0f8, 0x0fc},
	{0x301c0000, 0xf00, 0xf14},
	{0x301c0000, 0xff8, 0xffc},
};

static mm_reg_t map_block(uint32_t phys)
{
	static uint32_t mapped_phys[8];
	static mm_reg_t mapped_va[8];
	static int n;

	for (int i = 0; i < n; i++) {
		if (mapped_phys[i] == phys) {
			return mapped_va[i];
		}
	}
	if (n == ARRAY_SIZE(mapped_phys)) {
		return 0;
	}
#ifdef CONFIG_MMU
	device_map(&mapped_va[n], phys, 0x1000, K_MEM_CACHE_NONE);
#else
	mapped_va[n] = phys;
#endif
	mapped_phys[n] = phys;
	return mapped_va[n++];
}

void pvr_sample_display_dump(const char *when)
{
	printk("display dump %s begin\n", when);
	for (size_t i = 0; i < ARRAY_SIZE(ranges); i++) {
		mm_reg_t va = map_block(ranges[i].phys);

		if (va == 0) {
			printk("# %08x: not mapped\n", ranges[i].phys);
			continue;
		}
		for (uint32_t off = ranges[i].first; off <= ranges[i].last; off += 4) {
			printk("0x%08x 32 0x%08x\n", ranges[i].phys + off, sys_read32(va + off));
		}
	}
#if DT_NODE_HAS_STATUS(DT_NODELABEL(panel), okay)
	{
		const struct device *bridge = DEVICE_DT_GET(DT_NODELABEL(panel));
		int st = sn65dsi8x_irq_stat(bridge);

		printk("# sn65dsi8x IRQ_STAT %d (0x%02x), health 0x%x\n", st, st < 0 ? 0 : st,
		       sn65dsi8x_health(bridge));
	}
#endif
	printk("display dump %s end\n", when);
}
