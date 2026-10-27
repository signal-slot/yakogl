/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * What a kexec from Linux leaves in the boot CPU's GIC CPU interface.
 *
 * Linux runs the GICv3 CPU interface in split EOI/deactivate mode
 * (ICC_CTLR_EL1.EOImode = 1) and does not restore it before jumping to a new
 * image. Zephyr's GIC driver never writes ICC_CTLR_EL1 and ends an interrupt
 * with ICC_EOIR1 only, so in that mode every interrupt stays active after its
 * first delivery: the system timer ticks once and the kernel hangs in its
 * first timed wait. Put the interface back into the reset mode on the boot
 * CPU; the secondary CPUs come out of PSCI CPU_ON in reset state.
 *
 * This was the SafeUI Verdin board extension's board_a53.c, and it moved here
 * because every image kexec'd onto an i.MX8MP needs it, whichever board file
 * describes the carrier (the Verdin, the Armadillo-X2). It runs at the EARLY
 * init level: before the board and SoC early hooks, and before PRE_KERNEL_1,
 * where the GIC driver and the system timer come up. The register sequence
 * is the one board_a53.c had, unchanged.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/arch/arm64/lib_helpers.h>
#include <zephyr/sys/barrier.h>

#define ICC_CTLR_EL1_EOIMODE BIT(1)

static int gc_board_gic_eoi_reset(void)
{
	if (read_sysreg(ICC_SRE_EL1) & BIT(0)) {
		uint64_t ctlr = read_sysreg(ICC_CTLR_EL1);

		if (ctlr & ICC_CTLR_EL1_EOIMODE) {
			write_sysreg(ctlr & ~ICC_CTLR_EL1_EOIMODE, ICC_CTLR_EL1);
			barrier_isync_fence_full();
		}
	}
	return 0;
}
SYS_INIT(gc_board_gic_eoi_reset, EARLY, 0);
