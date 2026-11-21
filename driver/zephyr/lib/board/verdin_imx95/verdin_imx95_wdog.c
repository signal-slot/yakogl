/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * WDOG3, the Verdin iMX95 A55's watchdog, armed by every image for the board
 * and fed for a bounded time (CONFIG_YAKOGL_VERDIN_IMX95; why this one and
 * what its timeout does: docs/notes/pre-study-imx95.md, "The watchdog").
 *
 * WDOG3 is the A55 non-secure agent's (Toradex's System Manager
 * configuration, imx-sm-toradex configs/verdin-imx95.cfg: "WDOG3 OWNER" in
 * the "A55 non-secure EENV" section), the one the module's Linux runs
 * (imx95-verdin.dtsi: &wdog3 with fsl,ext-reset-output), and its timeout is
 * the System Manager's FAULT_WDOG3 with reaction=sys_reset: the whole board
 * is reset through the PMIC (WDOG_ANY) and boots as from power-on, into
 * Linux. Arming it here does not depend on U-Boot's watchdog still running
 * after "go".
 *
 * The register logic (the unlock in whichever command width it is found in,
 * Linux's CS value with INT, TOVAL, the waits and the read-back) is
 * driver/rust ip::wdog32 behind yd_wdog32_arm(); this file is the Zephyr
 * side. Not Zephyr's wdt_mcux_wdog32 at &wdog3: it arms at POST_KERNEL only
 * (its setup sleeps), and it sets CS.INT only with a callback whose ISR on
 * the A55 clears the flag that the System Manager's WDOG3 interrupt is
 * (imx-sm devices/MIMX95/sm/dev_sm_handlers.c, WDOG3_IRQHandler). So the
 * node stays disabled and the driver out of the image (asserted below).
 *
 * Order:
 *   PRE_KERNEL_1 0: map WDOG3, interrupts off, yd_wdog32_arm(60 s, INT on).
 *     Nothing printed yet.
 *   POST_KERNEL 0: the line that says what the arming found and left and
 *     "board: watchdog armed" (tools/board.sh requires it), or, if the arming
 *     refused, what refused and the PSCI reset at once: an image that could
 *     not arm its watchdog must not run. Then a 1 s timer refreshes WDOG3
 *     while the uptime is under CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S and
 *     the board layer's alive window holds (board_wdog_alive: the samples'
 *     and the Mali OSAL's board_watchdog_feed()); after that WDOG3 resets
 *     the board within 60 s. A hang with interrupts locked stops the timer
 *     too.
 * Built, not run on a board yet.
 */
#include <zephyr/devicetree.h>
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <stdint.h>

#include <yakogl_drivers.h>

#include "board_internal.h"
#include "verdin_imx95.h"

#define WDOG3_PHYS 0x42490000u /* imx95.dtsi wdog3, the SoC dtsi's &wdog3 */
#define WDOG3_TIMEOUT_MS 60000u /* the Verdins' period (YAKOGL_BOARD_LIFETIME_S) */
/* Reads of CS for each of the two waits: RCS takes about 2.5 counter
 * periods (8 ms each, imx7ulp_wdt.c's post-RCS wait); a device read is
 * well over 10 ns, so 10^8 reads are more than a second. */
#define WDOG3_POLLS 100000000u

BUILD_ASSERT(DT_REG_ADDR(DT_NODELABEL(wdog3)) == WDOG3_PHYS, "WDOG3 is at 0x42490000");
BUILD_ASSERT(!DT_NODE_HAS_STATUS(DT_NODELABEL(wdog3), okay),
	     "WDOG3 belongs to verdin_imx95_wdog.c, not to Zephyr's wdt_mcux_wdog32");

static mm_reg_t s_wdog;
static struct yd_wdog32_arm s_arm;
static int s_arm_rc = -EAGAIN;
static uint64_t s_arm_ticks, s_cntfrq;
static volatile uint32_t s_feeds;

uint32_t verdin_imx95_wdog_timeout_ms(void)
{
	return s_arm_rc == 0 ? s_arm.timeout_ms : 0u;
}

const struct yd_wdog32_arm *verdin_imx95_wdog_arm(int *rc)
{
	if (rc != NULL) {
		*rc = s_arm_rc;
	}
	return &s_arm;
}

static int wdog_arm(void)
{
	uint64_t t0, t1;
	unsigned int key;

	device_map(&s_wdog, WDOG3_PHYS, 0x1000, K_MEM_CACHE_NONE);
	key = arch_irq_lock();
	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(t0));
	/* INT on: Linux's fsl,ext-reset-output, and the System Manager's
	 * FAULT_WDOG3 is WDOG3's interrupt. */
	s_arm_rc = yd_wdog32_arm(s_wdog, WDOG3_TIMEOUT_MS, 1u, WDOG3_POLLS, &s_arm);
	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(t1));
	arch_irq_unlock(key);
	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(s_cntfrq));
	s_arm_ticks = t1 - t0;
	return 0;
}
SYS_INIT(wdog_arm, PRE_KERNEL_1, 0);

static void feed_tick(struct k_timer *t)
{
	int64_t now = k_uptime_get();

	if (now >= (int64_t)CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S * MSEC_PER_SEC ||
	    !board_wdog_alive()) {
		k_timer_stop(t);
		printk("verdin_imx95: WDOG3 feeding stopped at %lld ms after %u feeds (%s): "
		       "it resets the board within %u ms\n",
		       (long long)now, s_feeds, board_wdog_alive() ? "lifetime over" : "no progress",
		       s_arm.timeout_ms);
		return;
	}
	yd_wdog32_refresh(s_wdog);
	s_feeds++;
}
static K_TIMER_DEFINE(s_feeder, feed_tick, NULL);

static int wdog_report(void)
{
	uint32_t us = s_cntfrq >= 1000000u ? (uint32_t)(s_arm_ticks / (s_cntfrq / 1000000u)) : 0u;

	if (s_arm_rc != 0) {
		printk("verdin_imx95: WDOG3 arming FAILED (rc %d, refusal %u): CS 0x%08x -> 0x%08x, "
		       "TOVAL 0x%x -> 0x%x, polls %u/%u in %u us\n",
		       s_arm_rc, s_arm.refusal, s_arm.cs_found, s_arm.cs_now, s_arm.toval_found,
		       s_arm.toval_now, s_arm.ulk_polls, s_arm.rcs_polls, us);
		verdin_imx95_reset("watchdog refused");
	}
	printk("verdin_imx95: WDOG3 armed: CS 0x%08x -> 0x%08x (%s), TOVAL 0x%x -> 0x%x, "
	       "timeout %u ms, polls %u/%u in %u us; fed until %u s of uptime\n",
	       s_arm.cs_found, s_arm.cs_now,
	       (s_arm.flags & YD_WDOG32_WAS_RUNNING) ? "was running" : "was stopped",
	       s_arm.toval_found, s_arm.toval_now, s_arm.timeout_ms, s_arm.ulk_polls,
	       s_arm.rcs_polls, us,
	       IS_ENABLED(CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED)
		       ? CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S
		       : 0);
	printk(BOARD_WDOG_ARMED_LINE);
	if (IS_ENABLED(CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED)) {
		k_timer_start(&s_feeder, K_SECONDS(1), K_SECONDS(1));
	} else {
		printk("verdin_imx95: WDOG3 is not fed (CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED=n): it "
		       "resets the board %u ms after the arming\n",
		       s_arm.timeout_ms);
	}
	return 0;
}
SYS_INIT(wdog_report, POST_KERNEL, 0);
