/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * __wrap___start: linked with -Wl,--wrap=__start, so the image header's
 * "b __start" (zephyr/arch/arm64/core/header.S) lands here first. Saves
 * x0..x3, CurrentEL, its own address and MPIDR_EL1 into rpi5_boot_record
 * (.noinit, so Zephyr's BSS clearing does not erase it), then branches to
 * __real___start with x0..x30, sp, DAIF and NZCV as found. The only state not
 * kept is TPIDR_EL0, used as the one scratch holder and left 0 (its value at
 * entry is UNKNOWN, Zephyr sets it up later). Runs with the MMU and caches in
 * whatever state the firmware left; the record is cleaned+invalidated to the
 * point of coherency so Zephyr's cached reads see it.
 *
 * Only the CPU with MPIDR affinity 0 saves (the Pi 5 boot core); a CPU started
 * later through smp.c's &__start passes through without touching the record.
 * Keep the stores below in step with struct rpi5_boot_state (rpi5_boot.h).
 */
#include <zephyr/toolchain.h>

_ASM_FILE_PROLOGUE

	.section .noinit, "aw", @nobits
	.balign 64
	.globl	rpi5_boot_record
	.type	rpi5_boot_record, %object
rpi5_boot_record:
	.skip	64
	.size	rpi5_boot_record, . - rpi5_boot_record

	.section .text.rpi5_boot_x0, "ax", %progbits
	.globl	__wrap___start
	.type	__wrap___start, %function
__wrap___start:
	msr	tpidr_el0, x16		/* x16 is the scratch; keep its value */
	mrs	x16, mpidr_el1
	ubfx	x16, x16, #0, #24
	cbnz	x16, 1f			/* not the boot core: leave the record */

	adrp	x16, rpi5_boot_record
	add	x16, x16, :lo12:rpi5_boot_record
	stp	x0, x1, [x16, #8]	/* x0..x3 first: they are free after */
	stp	x2, x3, [x16, #24]
	mrs	x0, CurrentEL
	adr	x1, __wrap___start
	mrs	x2, mpidr_el1
	str	x0, [x16, #40]
	str	x1, [x16, #48]
	str	x2, [x16, #56]
	movz	x3, #0x5052		/* "RPI5BOOT", little endian */
	movk	x3, #0x3549, lsl #16
	movk	x3, #0x4f42, lsl #32
	movk	x3, #0x544f, lsl #48
	str	x3, [x16, #0]
	dc	civac, x16
	dsb	sy
	ldp	x0, x1, [x16, #8]
	ldp	x2, x3, [x16, #24]
1:
	mrs	x16, tpidr_el0
	msr	tpidr_el0, xzr
	b	__real___start
	.size	__wrap___start, . - __wrap___start

	.text
	.globl	rpi5_boot_state
	.type	rpi5_boot_state, %function
rpi5_boot_state:
	adrp	x0, rpi5_boot_record
	add	x0, x0, :lo12:rpi5_boot_record
	ret
	.size	rpi5_boot_state, . - rpi5_boot_state
