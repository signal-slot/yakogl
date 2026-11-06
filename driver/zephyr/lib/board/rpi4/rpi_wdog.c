/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The bounded lifetime of a Raspberry Pi 4 or 5 test image: the BCM2711 or
 * BCM2712 PM watchdog is armed at boot and fed from a kernel timer until a deadline, then
 * no longer, so the SoC resets RPI_WDOG_HOLD_S seconds after boot plus at most
 * one watchdog period -- also when the test hangs, faults or wedges the GPU,
 * because the feeding stops with it. Booted through the firmware's tryboot
 * (`reboot "0 tryboot"`), that reset brings the board back into Linux with
 * nobody at the power switch.
 *
 * Register facts from Linux v6.17 drivers/watchdog/bcm2835_wdt.c (cited, not
 * transcribed): PM_RSTC 0x1c, PM_WDOG 0x24 (:23-25), the password in every
 * write (:27), a 20-bit timeout in 1/65536 s ticks (:29, :43), and the RSTC
 * WRCFG field (bits 5:4) set to full reset (:30, :33).
 *
 * The SoC is a parameter (CONFIG_SOC_BCM2712 or the Pi 4's): the register block
 * is the same on both, only its address and the init order differ. BCM2712's
 * `pm: watchdog@7d200000` (brcm,bcm2712-pm, size 0x308;
 * driver/docs/rpi5-v3d71-facts.md, "The power domain and the reset: pm") sits
 * on the bus at 0x7d200000; the SoC node maps the bus 0x7c000000.. to ARM
 * 0x10_7c000000.. (the same mapping Zephyr's bcm2712.dtsi gives uart10:
 * serial@107d001000 for the bus 0x7d001000), so the block is at 0x10_7d200000.
 * The Pi 5 has nothing to wait for, and the board has no remote power: the
 * watchdog is armed in the first PRE_KERNEL_1 hook, before any driver of the
 * image can hang; the feeder timer and the console lines wait for POST_KERNEL
 * (the system timer and the console are not up before).
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/toolchain.h>
#include <stdbool.h>

#include "rpi_wdog.h"
#include "board_internal.h"

#ifdef CONFIG_SOC_BCM2712
#define PM_BASE            0x107d200000ull /* bus 0x7d200000, bcm2712-ds.dtsi:122-129 */
#else
#define PM_BASE            0xfe100000u /* bcm2711.dtsi:113 */
#endif
#define PM_RSTC            0x1c
#define PM_WDOG            0x24
#define PM_PASSWORD        0x5a000000u
#define PM_WDOG_TICKS_MAX  0x000fffffu /* about 16 s */
#define PM_RSTC_WRCFG_MASK 0x00000030u
#define PM_RSTC_FULL_RESET 0x00000020u

#define WDOG_PERIOD_S 10   /* one arm */
#define FEED_EVERY_MS 3000

static mm_reg_t pm;
static int64_t deadline_ms;

static void arm(uint32_t seconds)
{
	uint32_t ticks = seconds << 16;
	if (ticks > PM_WDOG_TICKS_MAX)
		ticks = PM_WDOG_TICKS_MAX;
	sys_write32(PM_PASSWORD | ticks, pm + PM_WDOG);
	uint32_t rstc = sys_read32(pm + PM_RSTC) & ~PM_RSTC_WRCFG_MASK;
	sys_write32(PM_PASSWORD | rstc | PM_RSTC_FULL_RESET, pm + PM_RSTC);
}

/* The image says whether it is still making progress. The default is
 * always: every image's watchdog then behaves as before the hook (fed until
 * RPI_WDOG_HOLD_S). An image that loops for a long time (v3d_gl_cluster's
 * demo) overrides it, so a stuck or slow step stops the feeding and the
 * reset follows within WDOG_PERIOD_S; the deadline stays the outer bound.
 * Called from the feeder timer's handler: it must only read state. */
__weak bool rpi_wdog_alive(void)
{
	/* The board layer's "still making progress" (board_watchdog_feed);
	 * always true without it. */
	return board_wdog_alive();
}

static void feed(struct k_timer *t)
{
	if (k_uptime_get() >= deadline_ms || !rpi_wdog_alive()) {
		k_timer_stop(t); /* no more feeding: the reset follows */
		return;
	}
	arm(WDOG_PERIOD_S);
}

K_TIMER_DEFINE(feeder, feed, NULL);

static void deadline_set(void)
{
#ifdef CONFIG_YAKOGL_BOARD
	/* The board layer's one bound replaces the sample's RPI_WDOG_HOLD_S. */
	deadline_ms = (int64_t)CONFIG_YAKOGL_BOARD_LIFETIME_S * 1000;
#else
	deadline_ms = (int64_t)RPI_WDOG_HOLD_S * 1000;
#endif
}

#ifdef CONFIG_SOC_BCM2712
/* Armed first, nothing but the register block mapped. The deadline counts from
 * boot (k_uptime_get() is 0 until the system timer starts). */
static int rpi_wdog_init(void)
{
	device_map(&pm, PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	deadline_set();
	arm(WDOG_PERIOD_S);
	return 0;
}

SYS_INIT(rpi_wdog_init, PRE_KERNEL_1, 0);

static int rpi_wdog_start(void)
{
	k_timer_start(&feeder, K_MSEC(FEED_EVERY_MS), K_MSEC(FEED_EVERY_MS));
	printk(BOARD_WDOG_ARMED_LINE);
	rpi_wdog_announce();
	return 0;
}

SYS_INIT(rpi_wdog_start, POST_KERNEL, 0);
#else
static int rpi_wdog_init(void)
{
	device_map(&pm, PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	deadline_set();
	arm(WDOG_PERIOD_S);
	k_timer_start(&feeder, K_MSEC(FEED_EVERY_MS), K_MSEC(FEED_EVERY_MS));
	printk(BOARD_WDOG_ARMED_LINE);
	return 0;
}

SYS_INIT(rpi_wdog_init, POST_KERNEL, 0);
#endif

void rpi_wdog_announce(void)
{
	printk("board: PM watchdog resets the SoC ~%d s after boot (tryboot -> Linux)\n",
	       RPI_WDOG_HOLD_S + WDOG_PERIOD_S);
}
