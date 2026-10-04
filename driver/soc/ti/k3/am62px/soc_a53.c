/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * TI AM62P Cortex-A53 SoC hooks. Zephyr's common K3 Cortex-A code only
 * unlocks the control-module partitions; this adds the CPU-interface state
 * a previous operating system may leave behind when Zephyr is entered
 * through kexec from Linux instead of from U-Boot.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/arch/arm64/lib_helpers.h>
#include <zephyr/arch/arm64/cpu.h>
#include "ctrl_partitions.h"

#ifdef CONFIG_SOC_AM62PX_A53_BOOT_MARKER
/* One raw character on the console UART per early boot stage, before the
 * console driver exists: 'H' from the early hook, 'K' from the first
 * PRE_KERNEL_1 init. A boot that shows neither never reached Zephyr's C
 * code (or the UART is dead); one that shows both but no banner hung in a
 * driver init. The UART keeps the previous boot stage's line settings.
 */
#define BOOT_MARKER_UART DT_CHOSEN(zephyr_console)

static void boot_marker(char c)
{
	mm_reg_t base;

	device_map(&base, DT_REG_ADDR(BOOT_MARKER_UART), DT_REG_SIZE(BOOT_MARKER_UART),
		   K_MEM_CACHE_NONE);
	/* 8250: LSR (reg 5, shift 2) bit 5 = THR empty; THR at reg 0 */
	for (int i = 0; i < 100000 && !(sys_read32(base + 0x14) & BIT(5)); i++) {
	}
	sys_write32((uint32_t)c, base);
}

static int boot_marker_prekernel(void)
{
	boot_marker('K');
	return 0;
}
SYS_INIT(boot_marker_prekernel, PRE_KERNEL_1, 0);
#else
static inline void boot_marker(char c)
{
	ARG_UNUSED(c);
}
#endif

#define ICC_CTLR_EL1_EOIMODE BIT(1)

/* Linux runs the GICv3 CPU interface in split EOI/deactivate mode
 * (ICC_CTLR_EL1.EOImode = 1) and does not restore it before jumping to a
 * new image. Zephyr's GIC driver never writes ICC_CTLR_EL1 and only ends an
 * interrupt with ICC_EOIR1, so in that mode every interrupt stays active
 * after its first delivery: the system timer ticks once and the kernel
 * hangs in the first timed wait. Put the interface back into the reset
 * mode (EOI deactivates). Only possible once the system register interface
 * is enabled; after a plain reset it already is in the reset mode.
 */
static void gic_cpu_interface_reset_mode(void)
{
	if (read_sysreg(ICC_SRE_EL1) & BIT(0)) {
		uint64_t ctlr = read_sysreg(ICC_CTLR_EL1);

		if (ctlr & ICC_CTLR_EL1_EOIMODE) {
			write_sysreg(ctlr & ~ICC_CTLR_EL1_EOIMODE, ICC_CTLR_EL1);
			barrier_isync_fence_full();
		}
	}
}

void soc_early_init_hook(void)
{
	k3_unlock_all_ctrl_partitions();
	gic_cpu_interface_reset_mode();
	boot_marker('H');
}

void soc_per_core_init_hook(void)
{
	gic_cpu_interface_reset_mode();
}
