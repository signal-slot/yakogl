/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_scanout16, skeleton: the build line, then the read-only register dump
 * (src/dump.h). No register is written. Later stages add adopt, fbcon, the
 * pattern and the hold here.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <stdbool.h>

#include "dump.h"
#include "rpi_wdog.h"

static volatile bool finished;

/* The feeding stops once the end line is out: the reset follows within one
 * watchdog period instead of at the end of the hold. */
bool rpi_wdog_alive(void)
{
	return !finished;
}

int main(void)
{
	int rc;

	printk("rpi5_scanout16: build %s\n", RPI5_SCANOUT16_BUILD);
	rc = dump_all();
	printk("rpi5_scanout16: dump rc=%d\n", rc);
	printk("rpi5_scanout16: skeleton done\n");
	finished = true;
	return 0;
}
