/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_product: the shape of a product image for the STM32MP257F-DK's
 * standalone default (README.md; docs/notes/stm32mp257f-dk-standalone.md
 * section 6). Before main() the board layer has printed the early banner
 * and the hand-over record (stm32mp2.c); this image does not have the
 * layer's own watchdog (CONFIG_YAKOGL_STM32MP2_WDOG off).
 *
 * It prints its banner, calls INIT (then SET_TIMEOUT 32, ENABLE 1, PET)
 * through driver/rust/src/soc/stm32mp2/smc_wdt_refresh.rs (mp2_wdtr.h) and
 * resets through PSCI if that is refused. Then the main loop pets every
 * CONFIG_MP2_PRODUCT_PET_S, but only while the heartbeat (a counter a
 * k_timer advances every CONFIG_MP2_PRODUCT_HEARTBEAT_MS) moved since the
 * previous PET: a stalled timer or a stuck main loop stops the petting, and
 * OP-TEE and the IWDG reset the board about 32 s after the last PET. One
 * line a minute says it is alive.
 *
 * CONFIG_MP2_PRODUCT_LIFETIME_S (120 s by default) is the image's own PSCI
 * SYSTEM_RESET, so a counted default of it comes back to Linux. 0 (a real
 * product) means none: only `forever` with BOARD_PERSON_PRESENT=1 may carry
 * that build. The layer's lifetime (CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
 * 3600 s in prj.conf) has no off switch and stays the backstop.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/atomic.h>
#include <zephyr/sys/printk.h>

#include <stm32mp2.h>

#include "mp2_wdtr.h"

/* The product watchdog marker: the runner's `default <image> forever`
 * refuses an image without these bytes (tools/mp2s_image_check.py
 * --require-marker). It is printed once INIT succeeded. */
#define MP2_PRODUCT_WDT_MARKER "mp2_wdtr: product refresh armed"

#define TIMEOUT_S    32u /* the DK's timeout-sec, the one Linux uses */
#define PET_S        CONFIG_MP2_PRODUCT_PET_S
#define HEARTBEAT_MS CONFIG_MP2_PRODUCT_HEARTBEAT_MS
#define LIFETIME_S   CONFIG_MP2_PRODUCT_LIFETIME_S
#define LINE_S       60

BUILD_ASSERT(!IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "one client of the watchdog: the layer's stm32mp2_wdog.c is off in this image");
BUILD_ASSERT(PET_S + MP2_WDTR_EARLY_S < TIMEOUT_S,
	     "each pet comes before OP-TEE's early interrupt");
BUILD_ASSERT(HEARTBEAT_MS < PET_S * MSEC_PER_SEC,
	     "the heartbeat advances at least once between two pets");
BUILD_ASSERT(LIFETIME_S == 0 || LIFETIME_S < CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the image's own lifetime comes before the layer's");

static atomic_t s_heartbeat;
static int64_t s_start_ms;

static void heartbeat(struct k_timer *t)
{
	ARG_UNUSED(t);
	atomic_inc(&s_heartbeat);
}
static K_TIMER_DEFINE(s_heartbeat_timer, heartbeat, NULL);

static void lifetime_over(struct k_timer *t)
{
	ARG_UNUSED(t);
	mp2_reset_to_linux("mp2_product lifetime over");
}
static K_TIMER_DEFINE(s_lifetime, lifetime_over, NULL);

static long long t_s(void)
{
	return (long long)((k_uptime_get() - s_start_ms) / MSEC_PER_SEC);
}

int main(void)
{
	struct yd_mp2_wdtr r;
	atomic_val_t last;
	long long next_line = LINE_S;
	unsigned int pets = 0;
	int rc;

	printk("mp2_product: build %s\n", MP2_PRODUCT_BUILD);
	printk("mp2_product: %s, console usart2 at %u baud, image at 0x%lx, timer %u Hz\n",
	       CONFIG_BOARD, (unsigned int)DT_PROP(DT_CHOSEN(zephyr_console), current_speed),
	       (unsigned long)CONFIG_SRAM_BASE_ADDRESS,
	       (unsigned int)sys_clock_hw_cycles_per_sec());
	if (LIFETIME_S != 0) {
		/* Uptime from boot, as the layer's own lifetime. */
		k_timer_start(&s_lifetime, K_MSEC(MAX(LIFETIME_S * MSEC_PER_SEC - k_uptime_get(), 0)),
			      K_NO_WAIT);
		printk("mp2_product: PSCI version 0x%x, lifetime %u s (PSCI SYSTEM_RESET), "
		       "layer's at %u ms\n",
		       (unsigned int)mp2_psci_version(), LIFETIME_S, mp2_lifetime_ms());
	} else {
		printk("mp2_product: PSCI version 0x%x, no lifetime of its own (product build), "
		       "layer's at %u ms\n",
		       (unsigned int)mp2_psci_version(), mp2_lifetime_ms());
	}
	printk("mp2_product: fid 0x%08x, timeout %u s, PET every %u s while the heartbeat "
	       "(%u ms) advances\n",
	       MP2_WDTR_FUNC_ID_DK, TIMEOUT_S, PET_S, HEARTBEAT_MS);

	s_start_ms = k_uptime_get();
	rc = yd_mp2_wdtr_init(MP2_WDTR_FUNC_ID_DK, TIMEOUT_S, &r);
	if (rc != 0) {
		printk("mp2_product: FAIL: init rc %d, command %u answered %d, bounds %llu..%llu s\n",
		       rc, (unsigned int)r.cmd, (int)r.code, (unsigned long long)r.min_s,
		       (unsigned long long)r.max_s);
		mp2_reset_to_linux("mp2_product init refused");
	}
	printk("%s: bounds %llu..%llu s, timeout %u s\n", MP2_PRODUCT_WDT_MARKER,
	       (unsigned long long)r.min_s, (unsigned long long)r.max_s, TIMEOUT_S);

	last = atomic_get(&s_heartbeat);
	k_timer_start(&s_heartbeat_timer, K_MSEC(HEARTBEAT_MS), K_MSEC(HEARTBEAT_MS));

	for (;;) {
		atomic_val_t now;

		k_sleep(K_SECONDS(PET_S));
		now = atomic_get(&s_heartbeat);
		if (now == last) {
			/* Unhealthy: no PET; OP-TEE panics and the IWDG resets
			 * about TIMEOUT_S after the last one. */
			printk("mp2_product: heartbeat stalled at %ld t=%lld, not petting\n",
			       (long)now, t_s());
			continue;
		}
		last = now;
		rc = yd_mp2_wdtr_pet(&r);
		if (rc != 0) {
			printk("mp2_product: pet t=%lld refused: rc %d code %d\n", t_s(), rc,
			       (int)r.code);
			continue;
		}
		pets++;
		if (t_s() >= next_line) {
			rc = yd_mp2_wdtr_time_left(&r);
			if (rc == 0) {
				printk("mp2_product: alive t=%lld heartbeat=%ld pets=%u left=%llu\n",
				       t_s(), (long)now, pets, (unsigned long long)r.left_s);
			} else {
				printk("mp2_product: alive t=%lld heartbeat=%ld pets=%u left=? "
				       "(rc %d code %d)\n",
				       t_s(), (long)now, pets, rc, (int)r.code);
			}
			next_line += LINE_S;
		}
	}
}
