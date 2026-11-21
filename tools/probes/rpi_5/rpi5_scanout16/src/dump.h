/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16: the read-only register dump of the build skeleton. It maps
 * the HVS and pixel valves 0 and 1 with device_map() and prints, one line per
 * step, the HVS VERSION word and its stepping (C0/D0), CTRL0, STATUS (FRCNT,
 * MODE) and DL (LACT) of channels 0..2, and PV_CONTROL.EN and
 * PV_V_CONTROL.VIDEN of each valve. A block that did not map still prints its
 * lines: a read of an unmapped block returns 0xffffffff. No register is
 * written.
 */
#ifndef RPI5_SCANOUT16_DUMP_H
#define RPI5_SCANOUT16_DUMP_H

#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>

#include <stdint.h>

#include "pv_hdmi.h"
#include "scanout_alive.h"

#define DUMP_HVS_BASE 0x107c580000ull /* rpi:arch/arm64/boot/dts/broadcom/bcm2712.dtsi:612-617 */
#define DUMP_HVS_SIZE 0x1a000u
#define DUMP_ALL_ONES 0xffffffffu
/* PV_V_CONTROL.VIDEN, bit 0: rpi:vc4_regs.h:180 (scanout_alive.h names the
 * same bit SA_PV_VCONTROL_VIDEN; the dump keeps its own). */
#define DUMP_PV_VIDEN (1u << 0)

static inline uint32_t dump_rd(mm_reg_t base, uint32_t size, uint32_t off)
{
	return base != 0 && off < size ? sys_read32(base + off) : DUMP_ALL_ONES;
}

static inline int dump_pv(unsigned int n, uint64_t pa, uint32_t size)
{
	mm_reg_t va = 0;
	uint32_t ctl, vctl;

	device_map(&va, (uintptr_t)pa, size, K_MEM_CACHE_NONE);
	ctl = dump_rd(va, size, SA_PV_CONTROL);
	vctl = dump_rd(va, size, SA_PV_V_CONTROL);
	printk("rpi5_scanout16: dump pv%u %s at 0x%llx: PV_CONTROL 0x%08x EN %u "
	       "PV_V_CONTROL 0x%08x VIDEN %u\n",
	       n, va != 0 ? "mapped" : "NOT mapped", (unsigned long long)pa, ctl,
	       (ctl & SA_PV_CONTROL_EN) != 0, vctl, (vctl & DUMP_PV_VIDEN) != 0);
	return va == 0 ? 1 : 0;
}

/* Returns the number of blocks (HVS, pixel valve 0, pixel valve 1) that did
 * not map: 0 when all did. */
static inline int dump_all(void)
{
	mm_reg_t hvs = 0;
	uint32_t version;
	int d0, rc;
	unsigned int ch;

	device_map(&hvs, (uintptr_t)DUMP_HVS_BASE, DUMP_HVS_SIZE, K_MEM_CACHE_NONE);
	rc = hvs == 0 ? 1 : 0;
	printk("rpi5_scanout16: dump hvs %s at 0x%llx, %u bytes\n",
	       hvs != 0 ? "mapped" : "NOT mapped", (unsigned long long)DUMP_HVS_BASE,
	       DUMP_HVS_SIZE);
	version = dump_rd(hvs, DUMP_HVS_SIZE, SA_HVS_VERSION);
	d0 = sa_is_d0(version);
	printk("rpi5_scanout16: dump HVS VERSION 0x%08x stepping %s\n", version,
	       version == DUMP_ALL_ONES ? "unreadable" :
	       (version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_D0 ? "D0" :
	       (version & SA_HVS_VERSION_MASK) == SA_HVS_VERSION_C0 ? "C0" : "unknown");
	for (ch = 0; ch < SA_NUM_CHANNELS; ch++) {
		uint32_t c = dump_rd(hvs, DUMP_HVS_SIZE, sa_reg_ctrl0(d0, ch));
		uint32_t s = dump_rd(hvs, DUMP_HVS_SIZE, sa_reg_status(d0, ch));
		uint32_t dl = dump_rd(hvs, DUMP_HVS_SIZE, sa_reg_dl(d0, ch));

		printk("rpi5_scanout16: dump ch%u CTRL0 0x%08x ENB %u STATUS 0x%08x "
		       "FRCNT %u MODE %u DL 0x%08x LACT %u\n",
		       ch, c, (c & SA_CTRL0_ENB) != 0, s,
		       (s >> SA_STATUS_FRCNT_SHIFT) & SA_STATUS_FRCNT_MASK,
		       (s >> SA_STATUS_MODE_SHIFT) & SA_STATUS_MODE_MASK, dl,
		       dl & SA_DL_LACT_MASK);
	}
	rc += dump_pv(0, PV0_BASE, PV0_SIZE);
	rc += dump_pv(1, PV1_BASE, PV1_SIZE);
	return rc;
}

#endif /* RPI5_SCANOUT16_DUMP_H */
