/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * image.S - the stand-in image of the mp2-standalone QEMU harness (README.md
 * in this directory). An arm64 Image header (magic "ARM\x64" at byte 56,
 * image_size at byte 16 = IMGSZ), then code that says on QEMU virt's PL011
 * whether it runs at its link address and returns to U-Boot's "go" with 0, so
 * a case can read the console and the environment after the jump. A Zephyr
 * image of the DK never returns; this one does on purpose.
 */
#ifndef IMGSZ
#define IMGSZ 0x2000
#endif
#define LINK_ADDR 0xa0000000	/* the DK's load address of the one-shot scripts */
#define UART_DR 0x09000000	/* QEMU virt's PL011 data register */

	.text
	.globl _start
_start:
.Lstart:
	b	entry			/* code0 */
	.word	0			/* code1 */
	.quad	0			/* text_offset */
	.quad	IMGSZ			/* image_size */
	.quad	0			/* flags */
	.quad	0, 0, 0			/* res2..res4 */
	.ascii	"ARM\x64"		/* magic, byte 56 */
	.word	0			/* res5 */

entry:
	adr	x4, .Lstart		/* local: no relocation in the raw binary */
	ldr	x5, =LINK_ADDR
	adr	x1, msg_at
	cmp	x4, x5
	b.eq	1f
	adr	x1, msg_away
1:	ldr	x2, =UART_DR
2:	ldrb	w3, [x1], #1
	cbz	w3, 3f
	str	w3, [x2]
	b	2b
3:	mov	x0, #0
	ret

msg_at:
	.asciz	"mp2q: image ran at its link address 0xa0000000\r\n"
msg_away:
	.asciz	"mp2q: image ran away from its link address\r\n"
	.balign	8
	.ltorg
