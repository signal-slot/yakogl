/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_wdog: does the BCM2712 PM watchdog reset the Raspberry Pi 5? The
 * watchdog is armed by the board layer in the first PRE_KERNEL_1 hook and fed
 * from a timer for CONFIG_YAKOGL_RPI5_WDOG_HOLD_S seconds; this image prints
 * the build, a line per feed interval, and when the feeding has stopped. The
 * reset (tryboot then brings Linux back) follows within one watchdog period;
 * the console going quiet and the boot loader's log after it are the proof.
 * If the image is still alive well past that, it says so and spins: nothing
 * resets the board then, a person must power-cycle it.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#define HOLD_S       CONFIG_YAKOGL_RPI5_WDOG_HOLD_S
#define PERIOD_S     10 /* WDOG_PERIOD_S of rpi_wdog.c: one arm */
#define FEED_EVERY_S 3  /* FEED_EVERY_MS of rpi_wdog.c */
#define GRACE_S      15 /* alive this long past the period: the watchdog failed */

int main(void)
{
	printk("rpi5_wdog: build %s\n", RPI5_WDOG_BUILD);
	printk("rpi5_wdog: %s, watchdog armed at boot, fed for %d s\n", CONFIG_BOARD, HOLD_S);

	/* The feeder (rpi_wdog.c) stops at its first tick at or after HOLD_S. */
	while (k_uptime_get() < (int64_t)(HOLD_S + FEED_EVERY_S) * 1000) {
		printk("rpi5_wdog: feeding, uptime %lld ms\n", (long long)k_uptime_get());
		k_sleep(K_SECONDS(FEED_EVERY_S));
	}

	int64_t stopped = k_uptime_get();

	printk("rpi5_wdog: feeding stopped, reset expected within %d s\n", PERIOD_S);
	while (k_uptime_get() < stopped + (int64_t)(PERIOD_S + GRACE_S) * 1000) {
		k_sleep(K_SECONDS(1));
	}
	printk("rpi5_wdog: FAIL no reset\n");
	for (;;) {
		k_sleep(K_SECONDS(10));
	}
	return 0;
}
