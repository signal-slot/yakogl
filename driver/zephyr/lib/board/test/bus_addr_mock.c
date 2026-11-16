/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * bus_addr_mock: driver/zephyr/lib/board/rpi5_scanout/bus_addr.h on the
 * host. Checked: the identity translation both ways and their round trips;
 * the edges of the 40-bit width, of DRAM and of the SoC window; rejected
 * addresses (too wide, a Pi 4 alias, a buffer running past DRAM, overflow);
 * the dlist low/high split and join, including a rejected high word.
 *
 * Prints "bus_addr_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdio.h>

#include "../rpi5_scanout/bus_addr.h"

static int checks, failed;

#define CHECK(cond)                                                                \
	do {                                                                       \
		checks++;                                                          \
		if (!(cond)) {                                                     \
			failed++;                                                  \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond);     \
		}                                                                  \
	} while (0)

#define LIMIT (1ull << 40)

static void test_translate(void)
{
	static const uint64_t v[] = {0, 1, 0x200000, 0x3fffffff, 0x40000000, 0xc0000000ull,
				     0xffffffffull, 0x100000000ull, 0x107c013880ull, LIMIT - 1};
	uint64_t a, b;
	unsigned int i;

	for (i = 0; i < sizeof(v) / sizeof(v[0]); i++) {
		a = b = 0xdeadull;
		CHECK(bcm2712_bus_to_arm(v[i], &a) == 0);
		CHECK(a == v[i] + BCM2712_BUS_TO_ARM_OFFSET);
		CHECK(bcm2712_arm_to_bus(a, &b) == 0);
		CHECK(b == v[i]);
		/* The BCM2712 has no alias: nothing is masked off. */
		CHECK(a == v[i]);
	}
	/* Rejected: results stay untouched. */
	a = 0x1234;
	CHECK(bcm2712_bus_to_arm(LIMIT, &a) == BCM2712_BUS_ERANGE && a == 0x1234);
	CHECK(bcm2712_bus_to_arm(~0ull, &a) == BCM2712_BUS_ERANGE && a == 0x1234);
	CHECK(bcm2712_arm_to_bus(LIMIT, &a) == BCM2712_BUS_ERANGE && a == 0x1234);
	CHECK(bcm2712_arm_to_bus(~0ull, &a) == BCM2712_BUS_ERANGE && a == 0x1234);
}

static void test_ranges(void)
{
	const uint64_t base = BCM2712_DRAM_BASE, size = BCM2712_DRAM_SIZE;

	CHECK(bcm2712_addr_is_display(0));
	CHECK(bcm2712_addr_is_display(LIMIT - 1));
	CHECK(!bcm2712_addr_is_display(LIMIT));

	/* DRAM edges. */
	CHECK(bcm2712_addr_in_dram(base, 1));
	CHECK(bcm2712_addr_in_dram(base, size));
	CHECK(bcm2712_addr_in_dram(base + size - 1, 1));
	CHECK(bcm2712_addr_in_dram(base + size - 1, 0));
	CHECK(!bcm2712_addr_in_dram(base, size + 1));
	CHECK(!bcm2712_addr_in_dram(base + size, 0));
	CHECK(!bcm2712_addr_in_dram(base + size - 1, 2));
	CHECK(!bcm2712_addr_in_dram(base + size, 1));
	CHECK(!bcm2712_addr_in_dram(base, ~0ull));
	CHECK(!bcm2712_addr_in_dram(~0ull, 2)); /* addr + len would wrap */
	if (base > 0) {
		CHECK(!bcm2712_addr_in_dram(base - 1, 1));
	}

	/* SoC window edges: the mailbox block is in, the byte below and above out. */
	CHECK(bcm2712_addr_in_soc(BCM2712_SOC_BASE));
	CHECK(bcm2712_addr_in_soc(0x107c013880ull));
	CHECK(bcm2712_addr_in_soc(BCM2712_SOC_BASE + BCM2712_SOC_SIZE - 1));
	CHECK(!bcm2712_addr_in_soc(BCM2712_SOC_BASE - 1));
	CHECK(!bcm2712_addr_in_soc(BCM2712_SOC_BASE + BCM2712_SOC_SIZE));
	CHECK(!bcm2712_addr_in_soc(0x200000));
	/* The window and DRAM do not overlap. */
	CHECK(!bcm2712_addr_in_dram(BCM2712_SOC_BASE, 1));

	/* Mailbox word: 32 bits. */
	CHECK(bcm2712_addr_fits_mailbox(0xffffffffull));
	CHECK(!bcm2712_addr_fits_mailbox(0x100000000ull));
}

static void test_fb(void)
{
	uint64_t a = 0x77;
	const uint64_t base = BCM2712_DRAM_BASE, size = BCM2712_DRAM_SIZE;

	CHECK(bcm2712_fb_bus_to_arm(base + 0x2000000, 1920ull * 1080 * 4, &a) == 0);
	CHECK(a == base + 0x2000000);
	/* The last framebuffer that fits, then one byte more. */
	a = 0x77;
	CHECK(bcm2712_fb_bus_to_arm(base + size - 4096, 4096, &a) == 0 && a == base + size - 4096);
	a = 0x77;
	CHECK(bcm2712_fb_bus_to_arm(base + size - 4096, 4097, &a) == BCM2712_BUS_ERANGE && a == 0x77);
	/* A Pi 4 style 0xc0000000 answer is not a DRAM address here. */
	CHECK(bcm2712_fb_bus_to_arm(0xc0100000ull, 4096, &a) == BCM2712_BUS_ERANGE);
	/* A register address, a too wide address, a wrapping length. */
	CHECK(bcm2712_fb_bus_to_arm(0x107c013880ull, 4, &a) == BCM2712_BUS_ERANGE);
	CHECK(bcm2712_fb_bus_to_arm(LIMIT, 4, &a) == BCM2712_BUS_ERANGE);
	CHECK(bcm2712_fb_bus_to_arm(base, ~0ull, &a) == BCM2712_BUS_ERANGE);
	CHECK(a == 0x77);
}

static void test_dlist(void)
{
	static const uint64_t v[] = {0, 1, 0xffffffffull, 0x100000000ull, 0x123456789aull,
				     0x107c013880ull, 0x7fffffffffull, 0xff00000000ull, LIMIT - 1};
	uint32_t lo, hi;
	uint64_t a;
	unsigned int i;

	for (i = 0; i < sizeof(v) / sizeof(v[0]); i++) {
		lo = hi = 0xa5a5a5a5u;
		CHECK(bcm2712_dlist_split(v[i], &lo, &hi) == 0);
		CHECK(lo == (uint32_t)(v[i] & 0xffffffffu));
		CHECK(hi == (uint32_t)(v[i] >> 32));
		CHECK(hi <= 0xffu); /* 40 bits: the high word fits a byte */
		a = 0;
		CHECK(bcm2712_dlist_join(lo, hi, &a) == 0 && a == v[i]);
	}
	CHECK(bcm2712_dlist_split(0x123456789aull, &lo, &hi) == 0 && lo == 0x3456789au && hi == 0x12);

	/* Rejected: too wide on split, a high word beyond the width on join. */
	lo = hi = 0x11;
	CHECK(bcm2712_dlist_split(LIMIT, &lo, &hi) == BCM2712_BUS_ERANGE && lo == 0x11 && hi == 0x11);
	a = 0x55;
	CHECK(bcm2712_dlist_join(0, 0x100, &a) == BCM2712_BUS_ERANGE && a == 0x55);
	CHECK(bcm2712_dlist_join(0xffffffffu, 0xffffffffu, &a) == BCM2712_BUS_ERANGE && a == 0x55);
	CHECK(bcm2712_dlist_join(0xffffffffu, 0xff, &a) == 0 && a == LIMIT - 1);
}

int main(void)
{
	test_translate();
	test_ranges();
	test_fb();
	test_dlist();
	printf("bus_addr_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
