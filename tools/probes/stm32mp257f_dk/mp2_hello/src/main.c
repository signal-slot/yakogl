/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_hello: the first image on the STM32MP257F-DK's A35 (README.md).
 * Before main() the board layer has printed the early banner and the
 * hand-over record (stm32mp2.c). This prints the build, counts
 * MP2_HELLO_SECONDS on the system timer (each second a k_sleep: the arch
 * timer's interrupt through the GIC-400) and resets the board back into
 * ST's Linux through PSCI. With CONFIG_MP2_HELLO_HANG it prints one line
 * and spins with interrupts locked instead (the OP-TEE watchdog must reset).
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>

#include <stm32mp2.h>

#define MP2_HELLO_SECONDS 5

BUILD_ASSERT(MP2_HELLO_SECONDS + 5 < CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the run ends well inside the layer's lifetime");

int main(void)
{
	int64_t start = k_uptime_get();

	printk("mp2_hello: build %s\n", MP2_HELLO_BUILD);
	printk("mp2_hello: %s, console usart2 at %u baud, image at 0x%lx, timer %u Hz\n",
	       CONFIG_BOARD, (unsigned int)DT_PROP(DT_CHOSEN(zephyr_console), current_speed),
	       (unsigned long)CONFIG_SRAM_BASE_ADDRESS, (unsigned int)sys_clock_hw_cycles_per_sec());
	printk("mp2_hello: PSCI version 0x%x, reset at %u ms of uptime at the latest\n",
	       (unsigned int)mp2_psci_version(), mp2_lifetime_ms());
#ifdef CONFIG_MP2_HELLO_HANG
	printk("mp2_hello: hanging on purpose with interrupts locked, the OP-TEE watchdog must reset\n");
	(void)irq_lock();
	for (;;) {
		/* no timer tick, no lifetime reset, no PSCI: the watchdog alone */
		__asm__ volatile("" ::: "memory");
	}
#endif
	for (int i = 1; i <= MP2_HELLO_SECONDS; i++) {
		k_sleep(K_SECONDS(1));
		printk("mp2_hello: %d s (uptime %lld ms)\n", i, (long long)k_uptime_get());
	}
	printk("mp2_hello: PASS: counted %d s in %lld ms\n", MP2_HELLO_SECONDS,
	       (long long)(k_uptime_get() - start));
	mp2_reset_to_linux("mp2_hello done");
}
