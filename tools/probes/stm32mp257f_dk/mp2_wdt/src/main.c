/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_wdt: the bounded proof of the opt-in OP-TEE watchdog refresh on the
 * STM32MP257F-DK's A35 (README.md; driver/rust/src/soc/stm32mp2/
 * smc_wdt_refresh.rs through mp2_wdtr.h). Before main() the board layer has
 * printed the early banner and the hand-over record (stm32mp2.c); this image
 * does not have the layer's own watchdog (CONFIG_YAKOGL_STM32MP2_WDOG off).
 *
 * It prints its banner, calls INIT (then SET_TIMEOUT, ENABLE 1, PET), pets
 * every CONFIG_MP2_WDT_PET_S for CONFIG_MP2_WDT_REFRESH_S, printing the
 * time left after each pet, then stops petting: a loop with a fixed count,
 * so the petting ends by construction. From there it only reads the time
 * left; OP-TEE panics and the IWDG resets the board about
 * CONFIG_MP2_WDT_TIMEOUT_S after the last pet. The layer's lifetime
 * (PSCI SYSTEM_RESET) is later still and catches a watchdog that does not
 * fire: no step strands the board.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>

#include <stm32mp2.h>

#include "mp2_wdtr.h"

#define TIMEOUT_S CONFIG_MP2_WDT_TIMEOUT_S
#define PET_S     CONFIG_MP2_WDT_PET_S
#define REFRESH_S CONFIG_MP2_WDT_REFRESH_S
#define PETS      (REFRESH_S / PET_S)

BUILD_ASSERT(!IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "one client of the watchdog: the layer's stm32mp2_wdog.c is off in this image");
BUILD_ASSERT(TIMEOUT_S >= MP2_WDTR_MIN_TIMEOUT_S, "OP-TEE's early interrupt needs 6 s or more");
BUILD_ASSERT(PET_S + MP2_WDTR_EARLY_S < TIMEOUT_S,
	     "each pet comes before OP-TEE's early interrupt");
BUILD_ASSERT(PETS >= 1 && REFRESH_S % PET_S == 0, "REFRESH_S is a whole number of pets");
BUILD_ASSERT(REFRESH_S + TIMEOUT_S + 30 <= CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the layer's lifetime reset comes well after the watchdog's");

static int64_t s_start_ms;

static long long t_s(void)
{
	return (long long)((k_uptime_get() - s_start_ms) / MSEC_PER_SEC);
}

/* "left=<s>", or why GET_TIMELEFT gave none. */
static void print_left(const char *what)
{
	struct yd_mp2_wdtr r;
	int rc = yd_mp2_wdtr_time_left(&r);

	if (rc == 0) {
		printk("mp2_wdt: %s t=%lld left=%llu\n", what, t_s(), (unsigned long long)r.left_s);
	} else {
		printk("mp2_wdt: %s t=%lld left=? (rc %d code %d)\n", what, t_s(), rc, (int)r.code);
	}
}

int main(void)
{
	struct yd_mp2_wdtr r;
	int rc;
	int i;

	printk("mp2_wdt: build %s\n", MP2_WDT_BUILD);
	printk("mp2_wdt: %s, console usart2 at %u baud, image at 0x%lx, timer %u Hz\n", CONFIG_BOARD,
	       (unsigned int)DT_PROP(DT_CHOSEN(zephyr_console), current_speed),
	       (unsigned long)CONFIG_SRAM_BASE_ADDRESS, (unsigned int)sys_clock_hw_cycles_per_sec());
	printk("mp2_wdt: PSCI version 0x%x, reset at %u ms of uptime at the latest\n",
	       (unsigned int)mp2_psci_version(), mp2_lifetime_ms());
	printk("mp2_wdt: fid 0x%08x, timeout %u s, PET every %u s for %u s, then stop petting\n",
	       MP2_WDTR_FUNC_ID_DK, TIMEOUT_S, PET_S, REFRESH_S);

	s_start_ms = k_uptime_get();
	rc = yd_mp2_wdtr_init(MP2_WDTR_FUNC_ID_DK, TIMEOUT_S, &r);
	if (rc != 0) {
		printk("mp2_wdt: FAIL: init rc %d, command %u answered %d, bounds %llu..%llu s\n", rc,
		       (unsigned int)r.cmd, (int)r.code, (unsigned long long)r.min_s,
		       (unsigned long long)r.max_s);
		mp2_reset_to_linux("mp2_wdt init refused");
	}
	printk("mp2_wdt: init ok: bounds %llu..%llu s, timeout %u s\n", (unsigned long long)r.min_s,
	       (unsigned long long)r.max_s, TIMEOUT_S);
	print_left("init");

	for (i = 1; i <= PETS; i++) {
		k_sleep(K_SECONDS(PET_S));
		rc = yd_mp2_wdtr_pet(&r);
		if (rc != 0) {
			printk("mp2_wdt: pet t=%lld refused: rc %d code %d\n", t_s(), rc, (int)r.code);
			break;
		}
		print_left("pet");
	}

	printk("mp2_wdt: stopped petting at t=%lld after %d pets: OP-TEE panics in about %u s, "
	       "the IWDG resets in about %u s\n",
	       t_s(), i - 1, TIMEOUT_S - MP2_WDTR_EARLY_S, TIMEOUT_S);
	for (;;) {
		k_sleep(K_SECONDS(PET_S));
		print_left("unpetted");
	}
}
