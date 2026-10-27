/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_hello: Z1 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md). Before
 * main() the board glue has reset the GIC EOI mode, armed WDT0 and printed
 * the arming (nanopc_t6.c). This prints the build, counts T6_HELLO_SECONDS
 * on the system timer (each second a k_sleep: it needs the timer interrupt,
 * which a GIC left in Linux's EOI mode delivers once only), feeding the
 * watchdog, and resets the board back into Armbian.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>

#include <nanopc_t6.h>

#define T6_HELLO_SECONDS 10

BUILD_ASSERT(T6_HELLO_SECONDS + 5 < CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S,
	     "the run ends well inside the watchdog's lifetime");

int main(void)
{
	int64_t start = k_uptime_get();

	printk("t6_hello: build %s\n", T6_HELLO_BUILD);
	printk("t6_hello: %s, console uart2 at %u baud, image at 0x%lx\n", CONFIG_BOARD,
	       (unsigned int)DT_PROP(DT_CHOSEN(zephyr_console), current_speed),
	       (unsigned long)CONFIG_SRAM_BASE_ADDRESS);
	printk("t6_hello: WDT0 period %u ms, fed until %u ms of uptime\n", t6_watchdog_period_ms(),
	       t6_watchdog_deadline_ms());
	for (int i = 1; i <= T6_HELLO_SECONDS; i++) {
		k_sleep(K_SECONDS(1));
		t6_watchdog_feed();
		printk("t6_hello: %d s (uptime %lld ms)\n", i, (long long)k_uptime_get());
	}
	printk("t6_hello: PASS: counted %d s in %lld ms\n", T6_HELLO_SECONDS,
	       (long long)(k_uptime_get() - start));
	t6_reset_to_linux("t6_hello done");
}
