/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_display: U3, the Raspberry Pi 5's display path from a kernel= image
 * (driver/docs/rpi5-display.md). This is the skeleton: the watchdog, the
 * banner and a step table the display steps are added to.
 *
 * The first action is the watchdog's: the board layer (rpi_wdog.c,
 * CONFIG_YAKOGL_RPI5_WDOG) arms the BCM2712 PM watchdog in the first
 * PRE_KERNEL_1 hook, before main() runs, and stops feeding it after
 * CONFIG_YAKOGL_RPI5_WDOG_HOLD_S; the reset brings Linux back through tryboot.
 * main() never feeds it.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include "verdict.h"

#define PERIOD_S 10 /* WDOG_PERIOD_S of rpi_wdog.c: one arm */

struct step {
	const char *name;
	int (*run)(void); /* 0: ok */
};

static int step_skel(void)
{
	VERDICT("U3 skel", "ok");
	return 0;
}

static const struct step steps[] = {
	{ "skel", step_skel },
};

int main(void)
{
	printk("rpi5_display: build %s\n", RPI5_DISPLAY_BUILD);
	printk("rpi5_display: %s, watchdog armed at boot, fed for %d s, period %d s\n",
	       CONFIG_BOARD, CONFIG_YAKOGL_RPI5_WDOG_HOLD_S, PERIOD_S);

	for (size_t i = 0; i < ARRAY_SIZE(steps); i++) {
		if (steps[i].run()) {
			printk("rpi5_display: step %s failed\n", steps[i].name);
			break;
		}
	}

	/* Stop; the watchdog resets the board. */
	for (;;) {
		k_sleep(K_SECONDS(10));
	}
	return 0;
}
