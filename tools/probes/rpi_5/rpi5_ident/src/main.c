/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_ident: D2 part 1 (docs/notes/pre-study-rpi5.md, section 6). Reads the
 * V3D 7.1 hub identity (HUB_IDENT0..3), the core identity (CORE_IDENT0..2)
 * and MMU_DEBUG_INFO with no power or clock setup, a line each, to compare
 * with Linux's debugfs v3d_ident (D3).
 *
 * The first action is the watchdog's: the board layer (rpi_wdog.c,
 * CONFIG_YAKOGL_RPI5_WDOG) arms the BCM2712 PM watchdog in the first
 * PRE_KERNEL_1 hook, before main() runs, and stops feeding it after
 * CONFIG_YAKOGL_RPI5_WDOG_HOLD_S; the reset brings Linux back through tryboot.
 *
 * The hub and the core lie above 4 GiB (BCM2712_V3D_HUB_BASE,
 * BCM2712_V3D_CORE0_BASE in v3d_regs.h), outside Zephyr's rpi_5 MMU map, so
 * each is device_map()ped (device memory, uncached) before it is read. Each
 * line is printed right after its read: if a read faults (the V3D unpowered or
 * unclocked), the last line on the console names the register before it, and
 * the watchdog resets the board with no value line for the faulting one.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>

#include "v3d_regs.h"

#define HUB_SIZE  0x4000
#define CORE_SIZE 0x6000

int main(void)
{
	mm_reg_t hub, core;
	uint32_t hub_ident[4], core_ident[3], mmu;

	printk("rpi5_ident: build %s\n", RPI5_IDENT_BUILD);
	printk("rpi5_ident: %s, V3D 7.1, no power or clock setup, watchdog armed at boot\n", CONFIG_BOARD);

	device_map(&hub, BCM2712_V3D_HUB_BASE, HUB_SIZE, K_MEM_CACHE_NONE);
	device_map(&core, BCM2712_V3D_CORE0_BASE, CORE_SIZE, K_MEM_CACHE_NONE);
	printk("rpi5_ident: hub and core0 mapped, reading\n");

	for (int i = 0; i < 4; i++) {
		hub_ident[i] = sys_read32(hub + V3D_HUB_CTL_IDENT0 + 4 * i);
		printk("rpi5_ident: HUB_IDENT%d 0x%08x\n", i, hub_ident[i]);
	}
	for (int i = 0; i < 3; i++) {
		core_ident[i] = sys_read32(core + V3D_CTL_0_IDENT0 + 4 * i);
		printk("rpi5_ident: CORE_IDENT%d 0x%08x\n", i, core_ident[i]);
	}
	mmu = sys_read32(hub + V3D_MMU0_DEBUG_INFO);
	printk("rpi5_ident: MMU_DEBUG_INFO 0x%08x\n", mmu);

	/* The decode is a convenience; the raw words above are the record
	 * (tools/v3d/ident71.py decodes them in full). */
	printk("rpi5_ident: V3D %u.%u, %u core(s), %u QPUs, VPM %u bytes, PA %u / VA %u bits, MMU %s\n",
	       V3D_HUB_CTL_IDENT1_TVER_GET(hub_ident[1]), V3D_HUB_CTL_IDENT1_REV_GET(hub_ident[1]),
	       V3D_HUB_CTL_IDENT1_NCORES_GET(hub_ident[1]),
	       V3D_CTL_0_IDENT1_NSLC_GET(core_ident[1]) * V3D_CTL_0_IDENT1_QUPS_GET(core_ident[1]),
	       V3D_CTL_0_IDENT1_VPM_SIZE_GET(core_ident[1]) * 8192u, 30u + V3D_MMU0_DEBUG_INFO_PA_WIDTH_GET(mmu),
	       30u + V3D_MMU0_DEBUG_INFO_VA_WIDTH_GET(mmu),
	       (hub_ident[2] & V3D_HUB_CTL_IDENT2_WITH_MMU_SET) ? "yes" : "no");
	printk("rpi5_ident: done\n");

	/* The watchdog's feeding stops at HOLD_S; its reset ends the run. */
	for (;;) {
		k_sleep(K_SECONDS(10));
	}
	return 0;
}
