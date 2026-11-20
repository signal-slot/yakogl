/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16: the read-only register dump (docs/notes/rpi5-display.md
 * section 4; driver/docs/rpi5-display.md). Reads only. Every read prints a
 * line, a block that did not map included (its words read 0xffffffff, which
 * the line says). Offsets are scanout_alive.h's, bases pv_hdmi.h's.
 */
#ifndef RPI5_SCANOUT16_DUMP_H
#define RPI5_SCANOUT16_DUMP_H

#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <stdint.h>

#include "pv_hdmi.h"
#include "scanout_alive.h"

#define DUMP_HVS_BASE 0x107c580000ull /* rpi:bcm2712.dtsi:612-617 */
#define DUMP_HVS_SIZE 0x1a000u
#define DUMP_ALL_ONES 0xffffffffu

static inline mm_reg_t dump_map(const char *name, uint64_t pa, uint32_t size)
{
	mm_reg_t va = 0;

	printk("rpi5_scanout16: mapping %s at 0x%llx, %u bytes\n", name, (unsigned long long)pa,
	       size);
	device_map(&va, (uintptr_t)pa, size, K_MEM_CACHE_NONE);
	if (va == 0) {
		printk("rpi5_scanout16: %s NOT mapped; its words below read 0x%08x\n", name,
		       DUMP_ALL_ONES);
	}
	return va;
}

static inline uint32_t dump_rd(mm_reg_t base, uint32_t off)
{
	return base ? sys_read32(base + off) : DUMP_ALL_ONES;
}

static inline void dump_valve(const char *name, mm_reg_t pv)
{
	uint32_t c = dump_rd(pv, SA_PV_CONTROL);
	uint32_t v = dump_rd(pv, SA_PV_V_CONTROL);

	printk("rpi5_scanout16: %s PV_CONTROL=0x%08x EN=%u PV_V_CONTROL=0x%08x VIDEN=%u\n", name, c,
	       (unsigned)(c & SA_PV_CONTROL_EN), v, (unsigned)(v & SA_PV_VCONTROL_VIDEN));
}

/* The whole dump; returns 0 when the HVS mapped and VERSION is C0 or D0. */
static inline int dump_all(void)
{
	mm_reg_t hvs = dump_map("hvs", DUMP_HVS_BASE, DUMP_HVS_SIZE);
	mm_reg_t pv0 = dump_map("pixelvalve0", PV0_BASE, PV0_SIZE);
	mm_reg_t pv1 = dump_map("pixelvalve1", PV1_BASE, PV1_SIZE);
	uint32_t ver = dump_rd(hvs, SA_HVS_VERSION);
	uint32_t step = ver & SA_HVS_VERSION_MASK;
	int d0 = step == SA_HVS_VERSION_D0;

	printk("rpi5_scanout16: HVS VERSION=0x%08x (%s)\n", ver,
	       d0 ? "D0" : step == SA_HVS_VERSION_C0 ? "C0" : "unknown stepping");
	for (unsigned int ch = 0; ch < 3; ch++) {
		uint32_t c0 = dump_rd(hvs, sa_reg_ctrl0(d0, ch));
		uint32_t st = dump_rd(hvs, sa_reg_status(d0, ch));
		uint32_t dl = dump_rd(hvs, sa_reg_dl(d0, ch));

		printk("rpi5_scanout16: chan%u CTRL0=0x%08x ENB=%u STATUS=0x%08x FRCNT=%u MODE=%u "
		       "DL=0x%08x LACT=%u\n",
		       ch, c0, (unsigned)(c0 >> 31), st,
		       (unsigned)((st >> SA_STATUS_FRCNT_SHIFT) & SA_STATUS_FRCNT_MASK),
		       (unsigned)((st >> SA_STATUS_MODE_SHIFT) & SA_STATUS_MODE_MASK), dl,
		       (unsigned)(dl & SA_DL_LACT_MASK));
	}
	dump_valve("pixelvalve0", pv0);
	dump_valve("pixelvalve1", pv1);
	return (!hvs || (step != SA_HVS_VERSION_C0 && step != SA_HVS_VERSION_D0)) ? 1 : 0;
}

#endif /* RPI5_SCANOUT16_DUMP_H */
