/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin iMX95 (i.MX 95, Cortex-A55) board glue (CONFIG_YAKOGL_VERDIN_IMX95):
 *
 *   reset     verdin_imx95_reset(): PSCI SYSTEM_RESET over SMC to TF-A, the
 *             way every route of docs/standalone-boot.md gives the board
 *             back (U-Boot, then Linux); if it returns, WDOG3 resets the
 *             board
 *   lifetime  a timer that takes that reset CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S
 *             after boot, whatever the sample does
 *   watchdog  WDOG3, armed at PRE_KERNEL_1 and fed up to the lifetime
 *             (verdin_imx95_wdog.c)
 *   record    before the reset, the board layer's last record into the BBM
 *             GPRs if it is still pending (verdin_imx95_record.c)
 *
 * Nothing here is i.MX 95 specific beyond the names: PSCI is TF-A's. Built,
 * not run on a board yet.
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <stdint.h>

#include "verdin_imx95.h"

/* PSCI 0.2+ SYSTEM_RESET (SMC32 function id). */
#define PSCI_SYSTEM_RESET 0x84000009u

static int64_t psci_call(uint32_t fn)
{
	register uint64_t x0 __asm__("x0") = fn;

	/* SMCCC: x0 in and out; x1-x3 may come back changed; x4-x17 are
	 * preserved from SMCCC 1.1 on, and nothing here needs them across. */
	__asm__ volatile("smc #0" : "+r"(x0) : : "x1", "x2", "x3", "memory");
	return (int64_t)x0;
}

/* The record sink's (verdin_imx95_record.c); nothing without it. */
__weak void verdin_imx95_record_flush(void)
{
}

FUNC_NORETURN void verdin_imx95_reset(const char *why)
{
	int64_t rc;

	verdin_imx95_record_flush();
	printk("verdin_imx95: %s: PSCI SYSTEM_RESET at %lld ms\n", why, (long long)k_uptime_get());
	rc = psci_call(PSCI_SYSTEM_RESET);
	/* Interrupts off from here: the feeder stops, and WDOG3 bites. */
	(void)arch_irq_lock();
	if (verdin_imx95_wdog_timeout_ms() != 0u) {
		printk("verdin_imx95: PSCI SYSTEM_RESET returned %lld: halted; WDOG3 resets the "
		       "board within %u ms\n",
		       (long long)rc, verdin_imx95_wdog_timeout_ms());
	} else {
		printk("verdin_imx95: PSCI SYSTEM_RESET returned %lld: halted with WDOG3 not "
		       "armed; only a power cycle ends this\n",
		       (long long)rc);
	}
	for (;;) {
		__asm__ volatile("wfi");
	}
}

static void lifetime_expired(struct k_timer *t)
{
	ARG_UNUSED(t);
	/* A timer handler cannot block; the reset is a single SMC. */
	verdin_imx95_reset("lifetime over");
}

static K_TIMER_DEFINE(lifetime, lifetime_expired, NULL);

static int verdin_imx95_init(void)
{
	k_timer_start(&lifetime, K_SECONDS(CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S), K_NO_WAIT);
	return 0;
}
SYS_INIT(verdin_imx95_init, POST_KERNEL, 99);
