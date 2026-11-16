/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * bus_addr.h: BCM2712 (Raspberry Pi 5) bus/DMA address translation for
 * display buffers. Header-only C11, static inline, no Zephyr dependency.
 *
 * Every route of the Pi 5 display (adopt the firmware's scanout, a mailbox
 * framebuffer, an HVS6 display list of our own) turns between the address
 * the display hardware or the firmware uses ("bus") and the ARM physical
 * address Zephyr maps ("arm"). On the Pi 4 the mailbox answer is a VC bus
 * address carrying the 0xc0000000 alias, which the board layer masks. On the
 * BCM2712 there is no alias to mask (see BCM2712_BUS_TO_ARM_OFFSET).
 *
 * Source keys (the trees of driver/docs/rpi5-v3d71-facts.md, "Sources and
 * rules"; only facts and file:line are cited, no code is copied):
 *   rpi:     raspberrypi/linux, rpi-6.12.y
 *   zephyr:  the Zephyr tree the workspace pins
 *   facts:   driver/docs/rpi5-v3d71-facts.md
 * Citations marked (facts) were taken from that file (sections 10 and 11,
 * "The BCM2712 property mailbox"); they were not re-read in a tree for this
 * header. The scaler6 paths of rpi:vc4_hvs.c / vc4_plane.c could not be
 * re-read here either, so nothing about the display list's pointer words
 * is asserted below: that stays a U3 parameter.
 *
 * Constants the sources do not settle are macros the includer may define
 * before this header; each carries a TODO naming the board check (U3 of
 * docs/notes/pre-study-rpi5.md) that decides it.
 */
#ifndef YAKOGL_RPI5_SCANOUT_BUS_ADDR_H
#define YAKOGL_RPI5_SCANOUT_BUS_ADDR_H

#include <stdbool.h>
#include <stdint.h>

/*
 * Offset added to a bus address to get the ARM physical address: 0, the
 * identity. Source (facts, citing rpi:bcm2712.dtsi:189-193,440 and
 * rpi:bcm2712-rpi.dtsi:14): the `soc` node has no dma-ranges, the `firmware`
 * node has an empty `dma-ranges;`, and the `axi` bus maps the first 64 GiB
 * one to one. The Pi 4's `soc` has dma-ranges <0xc0000000 0x0 0x00000000
 * 0x40000000> (rpi:arch/arm/boot/dts/broadcom/bcm2711.dtsi:45); the BCM2712
 * has no such alias.
 */
#define BCM2712_BUS_TO_ARM_OFFSET 0ull

/*
 * Width of an address this path accepts: 40 bits. Source: the `rpi_5`
 * Zephyr defconfig sets the MMU's virtual and physical address spaces to 40
 * bits (facts, citing zephyr:boards/raspberrypi/rpi_5/
 * rpi_5_bcm2712_defconfig:3-4). That is the ARM side; the number of high
 * bits the HVS6 display list carries is not read here.
 * TODO(U3): confirm the display list's high-address width (rpi:vc4_hvs.c /
 * vc4_plane.c scaler6 pointer words against the firmware's scanout in the
 * Linux reference dump, board check "linux_ref"): a firmware list whose
 * high word has bits set above bit 39 would show it. Until then 40 bits.
 */
#ifndef BCM2712_DISPLAY_ADDR_BITS
#define BCM2712_DISPLAY_ADDR_BITS 40u
#endif
#define BCM2712_DISPLAY_ADDR_LIMIT (1ull << BCM2712_DISPLAY_ADDR_BITS)

/*
 * The split point: low word = bits 31:0 (a 32-bit register or list word).
 * Source for the 32-bit word: the mailbox message buffer address must fit
 * 32 bits (facts, citing rpi:drivers/firmware/raspberrypi.c:53). That is the
 * mailbox's word, used here as the natural low-word width; where the
 * display list puts the high bits is TODO(U3) as above.
 */
#define BCM2712_ADDR_LO_BITS 32u

/*
 * Start and size of the DRAM Zephyr can map for a display buffer. Not
 * settled by the sources: the Zephyr SoC file places the image at sram0
 * 0x200000 (zephyr:dts/arm64/broadcom/bcm2712.dtsi:40-44, facts) and the
 * probe overlays make it 32 MiB, which says what the image owns, not what
 * DRAM exists. Defaults: 0 and 1 GiB, the low part every Pi 5 has.
 * TODO(U3): the probe's console line that prints the memory node and
 * reserved-memory of the DTB a kernel= image receives (board check
 * "fwfb-dt") decides both; then override these macros.
 */
#ifndef BCM2712_DRAM_BASE
#define BCM2712_DRAM_BASE 0x00000000ull
#endif
#ifndef BCM2712_DRAM_SIZE
#define BCM2712_DRAM_SIZE 0x40000000ull
#endif

/*
 * The peripheral window `soc@107c000000`: child address a is at
 * 0x10_0000_0000 + a, size 0x8000_0000. Source: facts, citing
 * rpi:bcm2712.dtsi:189,191. An address in it is a register, never a
 * framebuffer.
 */
#define BCM2712_SOC_BASE 0x1000000000ull
#define BCM2712_SOC_SIZE 0x0080000000ull

/* Failure return: -ERANGE of Zephyr's errno.h. */
#define BCM2712_BUS_ERANGE (-34)

/* True when [addr, addr + len) lies inside [base, base + size). Overflow safe. */
static inline bool bcm2712_range_inside(uint64_t addr, uint64_t len, uint64_t base, uint64_t size)
{
	if (addr < base || addr - base >= size) {
		return false;
	}
	return len <= size - (addr - base);
}

/* An address the display path can express at all: below 2^40. */
static inline bool bcm2712_addr_is_display(uint64_t addr)
{
	return addr < BCM2712_DISPLAY_ADDR_LIMIT;
}

/* True when [addr, addr + len) lies in the DRAM Zephyr can map. */
static inline bool bcm2712_addr_in_dram(uint64_t addr, uint64_t len)
{
	return bcm2712_range_inside(addr, len, BCM2712_DRAM_BASE, BCM2712_DRAM_SIZE);
}

/* True when addr is in the SoC peripheral window (a register, not memory). */
static inline bool bcm2712_addr_in_soc(uint64_t addr)
{
	return bcm2712_range_inside(addr, 1, BCM2712_SOC_BASE, BCM2712_SOC_SIZE);
}

/* bus -> ARM physical; rejects an address above the display width. */
static inline int bcm2712_bus_to_arm(uint64_t bus, uint64_t *arm)
{
	uint64_t a;

	if (!bcm2712_addr_is_display(bus)) {
		return BCM2712_BUS_ERANGE;
	}
	a = bus + BCM2712_BUS_TO_ARM_OFFSET;
	if (!bcm2712_addr_is_display(a)) {
		return BCM2712_BUS_ERANGE;
	}
	*arm = a;
	return 0;
}

/* ARM physical -> bus, the inverse of bcm2712_bus_to_arm(). */
static inline int bcm2712_arm_to_bus(uint64_t arm, uint64_t *bus)
{
	uint64_t a;

	if (!bcm2712_addr_is_display(arm)) {
		return BCM2712_BUS_ERANGE;
	}
	a = arm - BCM2712_BUS_TO_ARM_OFFSET;
	if (a > arm || !bcm2712_addr_is_display(a)) { /* a > arm: wrapped */
		return BCM2712_BUS_ERANGE;
	}
	*bus = a;
	return 0;
}

/*
 * A firmware or mailbox bus address of a framebuffer Zephyr maps: translate,
 * then require [arm, arm + len) inside DRAM. A Pi 4 style answer
 * (0xc0000000 alias) is refused because it lies outside the default DRAM.
 */
static inline int bcm2712_fb_bus_to_arm(uint64_t bus, uint64_t len, uint64_t *arm)
{
	uint64_t a;
	int rc = bcm2712_bus_to_arm(bus, &a);

	if (rc != 0) {
		return rc;
	}
	if (!bcm2712_addr_in_dram(a, len)) {
		return BCM2712_BUS_ERANGE;
	}
	*arm = a;
	return 0;
}

/*
 * Split an address into a low word (bits 31:0) and a high word (the bits
 * above, at most BCM2712_DISPLAY_ADDR_BITS - 32 of them). An address wider
 * than the display is refused. Where the high word sits inside the HVS6
 * pointer words is not decided here (TODO(U3) above).
 */
static inline int bcm2712_dlist_split(uint64_t addr, uint32_t *lo, uint32_t *hi)
{
	if (!bcm2712_addr_is_display(addr)) {
		return BCM2712_BUS_ERANGE;
	}
	*lo = (uint32_t)(addr & 0xffffffffull);
	*hi = (uint32_t)(addr >> BCM2712_ADDR_LO_BITS);
	return 0;
}

/* Join the two words again; a high word with bits above the width is refused. */
static inline int bcm2712_dlist_join(uint32_t lo, uint32_t hi, uint64_t *addr)
{
	uint64_t a = ((uint64_t)hi << BCM2712_ADDR_LO_BITS) | lo;

	if (!bcm2712_addr_is_display(a)) {
		return BCM2712_BUS_ERANGE;
	}
	*addr = a;
	return 0;
}

/* True when the address fits the 32-bit word of the mailbox buffer. */
static inline bool bcm2712_addr_fits_mailbox(uint64_t addr)
{
	return addr <= 0xffffffffull;
}

#endif /* YAKOGL_RPI5_SCANOUT_BUS_ADDR_H */
