/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The image's first code on the STM32MP257F-DK's A35 (stm32mp2.c, "Order").
 *
 * __wrap___start: linked with -Wl,--wrap=__start (CMakeLists.txt), so the
 * image header's "b __start" (zephyr arch/arm64/core/header.S) lands here
 * before any of Zephyr's code. Zephyr's own first hook comes too late for
 * x0: reset.S's __reset_prep_c uses x0 as a scratch (`switch_el x0`,
 * reset.S:45) before it calls z_arm64_el2_plat_prep_c. The same technique
 * as tools/probes/rpi_5/common/boot/boot_x0.S.
 *
 * It:
 *   - stores what the previous stage handed over into mp2_handover (a
 *     line-aligned record in .data, which z_prep_c's BSS clear does not
 *     touch): x0-x3, CurrentEL, SCTLR_EL2 or SCTLR_EL1 by the EL, MPIDR_EL1,
 *     CNTFRQ_EL0 and the address it runs at; then cleans and invalidates the
 *     record's lines to the point of coherency, so that the C side reads it
 *     after the MMU and the D-cache are on, whatever the caches' state here;
 *   - prints "MP2:A EL<n>" on USART2, polled, at its physical address, on
 *     the line setup TF-A and U-Boot left (115200 8N1);
 *   - branches to __real___start with x0-x3 as found.
 * Clobbers x9-x17 only, which neither booti nor go define at entry and
 * which Zephyr's __start does not read. Only the first entry fills the
 * record: a core started later (PSCI CPU_ON on &__start, which the wrap
 * redirects as well) finds it filled and passes through.
 *
 * The record's layout is struct mp2_handover (stm32mp2.h); the offsets
 * below are checked against it by BUILD_ASSERTs in stm32mp2.c.
 */
#include <zephyr/toolchain.h>

#define MP2_USART2_PHYS 0x400e0000 /* stm32mp2.c BUILD_ASSERTs the dts */
#define USART_ISR       0x1c
#define USART_TDR       0x28
#define USART_ISR_TXE   7
/* About 87 us a character at 115200: a USART nobody enabled never raises
 * TXE, and the banner must not hang the boot on it. */
#define TXE_POLL_LIMIT  0x100000

#define HO_SEEN    0
#define HO_X0      8
#define HO_X1      16
#define HO_X2      24
#define HO_X3      32
#define HO_EL      40
#define HO_SCTLR   48
#define HO_MPIDR   56
#define HO_CNTFRQ  64
#define HO_PC      72

/* "MP2SEEN\0", little-endian (MP2_HANDOVER_SEEN in stm32mp2.h). */
#define HO_SEEN_W0 0x504d
#define HO_SEEN_W1 0x5332
#define HO_SEEN_W2 0x4545
#define HO_SEEN_W3 0x004e

/* One character in w12 to USART2 (x10), bounded wait on TXE; x13, x14. */
.macro mp2_putc
	mov	x13, #TXE_POLL_LIMIT
1:	ldr	w14, [x10, #USART_ISR]
	tbnz	w14, #USART_ISR_TXE, 2f
	subs	x13, x13, #1
	b.ne	1b
2:	str	w12, [x10, #USART_TDR]
.endm

/* The NUL-terminated string at x11 to USART2 (x10); x11-x14. */
.macro mp2_puts
3:	ldrb	w12, [x11], #1
	cbz	w12, 4f
	mp2_putc
	b	3b
4:
.endm

	.section .text.mp2_wrap_start, "ax", %progbits
	.global __wrap___start
	.type __wrap___start, %function
__wrap___start:
	adrp	x16, mp2_handover
	add	x16, x16, :lo12:mp2_handover
	movz	x17, #HO_SEEN_W0
	movk	x17, #HO_SEEN_W1, lsl #16
	movk	x17, #HO_SEEN_W2, lsl #32
	movk	x17, #HO_SEEN_W3, lsl #48
	ldr	x9, [x16, #HO_SEEN]
	cmp	x9, x17
	b.eq	9f			/* filled by an earlier entry */

	stp	x0, x1, [x16, #HO_X0]
	stp	x2, x3, [x16, #HO_X2]
	mrs	x11, CurrentEL
	str	x11, [x16, #HO_EL]
	ubfx	x15, x11, #2, #2
	cmp	x15, #2
	b.ne	5f
	mrs	x12, sctlr_el2
	b	6f
5:	mrs	x12, sctlr_el1
6:	str	x12, [x16, #HO_SCTLR]
	mrs	x13, mpidr_el1
	str	x13, [x16, #HO_MPIDR]
	mrs	x13, cntfrq_el0
	str	x13, [x16, #HO_CNTFRQ]
	adr	x13, __wrap___start
	str	x13, [x16, #HO_PC]
	str	x17, [x16, #HO_SEEN]
	/* The record spans two 64-byte lines (80 bytes, 64-byte aligned). */
	dsb	sy
	dc	civac, x16
	add	x13, x16, #64
	dc	civac, x13
	dsb	sy

	/* The banner: "\r\nMP2:A EL<n>\r\n", n = CurrentEL[3:2] (x15). */
	movz	x10, #(MP2_USART2_PHYS & 0xffff)
	movk	x10, #(MP2_USART2_PHYS >> 16), lsl #16
	adr	x11, mp2_banner_a
	mp2_puts
	add	w12, w15, #'0'
	mp2_putc
	adr	x11, mp2_banner_eol
	mp2_puts
9:
	b	__real___start
	.size __wrap___start, . - __wrap___start

mp2_banner_a:
	.asciz "\r\nMP2:A EL"
mp2_banner_eol:
	.asciz "\r\n"
	.balign 4
