/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_hello: a no-GPU image for the Raspberry Pi 5 (item H9 of
 * docs/notes/pre-study-rpi5.md), built and not run. It prints the build, the
 * board, the console's speed and the RAM the image was linked into (from the
 * overlay's sram0), then counts three seconds on the system timer.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>

#define RPI5_HELLO_SECONDS 3

int main(void)
{
	int64_t start = k_uptime_get();

	printk("rpi5_hello: build %s\n", RPI5_HELLO_BUILD);
	printk("rpi5_hello: %s, console at %u baud, image RAM 0x%lx + 0x%lx\n", CONFIG_BOARD,
	       (unsigned int)DT_PROP(DT_CHOSEN(zephyr_console), current_speed),
	       (unsigned long)CONFIG_SRAM_BASE_ADDRESS, (unsigned long)CONFIG_SRAM_SIZE * 1024UL);
	for (int i = 1; i <= RPI5_HELLO_SECONDS; i++) {
		k_sleep(K_SECONDS(1));
		printk("rpi5_hello: %d s (uptime %lld ms)\n", i, (long long)k_uptime_get());
	}
	printk("rpi5_hello: counted %d s in %lld ms\n", RPI5_HELLO_SECONDS,
	       (long long)(k_uptime_get() - start));
	return 0;
}
