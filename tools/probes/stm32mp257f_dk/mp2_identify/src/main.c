/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_identify: the STM32MP257F-DK's return-trial image (README.md). Before
 * main() the board layer has printed the early banner and the hand-over
 * record (stm32mp2.c) and armed OP-TEE's watchdog (stm32mp2_wdog.c: the
 * "gc_wdog_arm: armed" line; a refusal has reset the board already). This
 * prints the build, the hand-over again in short, the watchdog's state and
 * time left, lets the feeder run MP2_IDENTIFY_SECONDS, and resets the board
 * back into ST's Linux through PSCI. It touches nothing but the console,
 * the system timer and the SMCs to TF-A and OP-TEE: no GPU, no SCMI, no RCC.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>

#include <stm32mp2.h>
#include <stm32mp2_wdog.h>

#define MP2_IDENTIFY_SECONDS 3

BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "the return-trial image arms OP-TEE's watchdog at boot");
BUILD_ASSERT(MP2_IDENTIFY_SECONDS + 5 < CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the run ends well inside the layer's lifetime");

static void print_time_left(const char *when)
{
	uint64_t left = 0;
	int32_t rc = mp2_smcwdt_time_left(MP2_SMCWDT_FUNC_ID, &left);

	if (rc == 0) {
		printk("mp2_identify: watchdog time left %s: %llu s\n", when,
		       (unsigned long long)left);
	} else {
		printk("mp2_identify: watchdog time left %s: GET_TIMELEFT answered %d\n", when,
		       (int)rc);
	}
}

int main(void)
{
	const struct mp2_handover *h = mp2_handover_get();
	const struct mp2_wdog_state *w = mp2_wdog_state_get();
	int64_t start = k_uptime_get();

	printk("mp2_identify: build %s\n", MP2_IDENTIFY_BUILD);
	printk("mp2_identify: %s, console usart2 at %u baud, image at 0x%lx, timer %u Hz\n",
	       CONFIG_BOARD, (unsigned int)DT_PROP(DT_CHOSEN(zephyr_console), current_speed),
	       (unsigned long)CONFIG_SRAM_BASE_ADDRESS, (unsigned int)sys_clock_hw_cycles_per_sec());
	if (h->seen == MP2_HANDOVER_SEEN) {
		printk("mp2_identify: handed over at EL%u, pc 0x%llx, x0 0x%llx, sctlr 0x%llx, "
		       "cntfrq %llu Hz, mpidr 0x%llx, PSCI 0x%x\n",
		       (unsigned int)((h->el >> 2) & 3), (unsigned long long)h->pc,
		       (unsigned long long)h->x[0], (unsigned long long)h->sctlr,
		       (unsigned long long)h->cntfrq, (unsigned long long)h->mpidr,
		       (unsigned int)mp2_psci_version());
	} else {
		printk("mp2_identify: no hand-over record (the entry code did not run)\n");
	}
	printk("mp2_identify: watchdog fid 0x%08x armed rc %d, timeout %u s, feeder %s, "
	       "reset at %u ms of uptime at the latest\n",
	       (unsigned int)MP2_SMCWDT_FUNC_ID, (int)w->arm_rc, (unsigned int)w->timeout_s,
	       w->feeding ? "running" : "stopped", mp2_lifetime_ms());
	print_time_left("at main");
	for (int i = 1; i <= MP2_IDENTIFY_SECONDS; i++) {
		k_sleep(K_SECONDS(1));
		printk("mp2_identify: %d s (uptime %lld ms), %u PETs, %u refused\n", i,
		       (long long)k_uptime_get(), (unsigned int)w->feeds,
		       (unsigned int)w->pet_failures);
	}
	print_time_left("after the wait");
	if (w->arm_rc == 0 && w->feeds >= MP2_IDENTIFY_SECONDS - 1 && w->pet_failures == 0) {
		printk("mp2_identify: PASS: watchdog armed (%u s), %u PETs in %lld ms\n",
		       (unsigned int)w->timeout_s, (unsigned int)w->feeds,
		       (long long)(k_uptime_get() - start));
	} else {
		printk("mp2_identify: FAIL: watchdog rc %d, %u PETs, %u refused (first %d)\n",
		       (int)w->arm_rc, (unsigned int)w->feeds, (unsigned int)w->pet_failures,
		       (int)w->first_pet_rc);
	}
	mp2_reset_to_linux("mp2_identify done");
}
