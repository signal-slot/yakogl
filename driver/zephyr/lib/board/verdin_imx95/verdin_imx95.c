/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin iMX95 (i.MX 95, Cortex-A55) board glue (CONFIG_YAKOGL_VERDIN_IMX95):
 *
 *   reset     verdin_imx95_reset(): PSCI SYSTEM_RESET over SMC to TF-A, the
 *             way every route of docs/standalone-boot.md gives the board
 *             back (U-Boot, then Linux)
 *   lifetime  a timer that takes that reset CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S
 *             after boot, whatever the sample does
 *   watchdog  STUB (verdin_imx95_wdog_stub): none armed. The module's
 *             Linux enables WDOG3 with its external reset output
 *             (imx95-verdin.dtsi), and Toradex's U-Boot is built with a
 *             watchdog (verdin-imx95_defconfig: CONFIG_WDT,
 *             CONFIG_ULP_WATCHDOG, 30 s); whether that one still runs after
 *             "go" is not known, and this image does not feed it.
 *
 * Nothing here is i.MX 95 specific beyond the names: PSCI is TF-A's. Built,
 * not run on a board yet.
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
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

FUNC_NORETURN void verdin_imx95_reset(const char *why)
{
	int64_t rc;

	printk("verdin_imx95: %s: PSCI SYSTEM_RESET at %lld ms\n", why, (long long)k_uptime_get());
	rc = psci_call(PSCI_SYSTEM_RESET);
	printk("verdin_imx95: PSCI SYSTEM_RESET returned %lld: halted (no watchdog armed)\n",
	       (long long)rc);
	(void)arch_irq_lock();
	for (;;) {
		__asm__ volatile("wfi");
	}
}

int verdin_imx95_wdog_stub(void)
{
	printk("verdin_imx95: STUB: no watchdog armed (the i.MX 95 WDOG is not driven "
	       "by this image); the lifetime timer resets through PSCI at %d s\n",
	       CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S);
	return -ENOTSUP;
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
	(void)verdin_imx95_wdog_stub();
	k_timer_start(&lifetime, K_SECONDS(CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S), K_NO_WAIT);
	return 0;
}
SYS_INIT(verdin_imx95_init, POST_KERNEL, 99);
