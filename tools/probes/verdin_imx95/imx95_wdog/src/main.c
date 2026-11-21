/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx95_wdog: does WDOG3 reset the Verdin iMX95 (README.md)? The board glue
 * armed it at PRE_KERNEL_1 (driver/zephyr/lib/board/verdin_imx95/
 * verdin_imx95_wdog.c) and, with CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED=n,
 * never refreshes it. main() prints what U-Boot left (the banner's wdog
 * line), a line every 10 s, and the line that says the reset is due. The
 * pass is the System Manager's reset of the board (FAULT_WDOG3,
 * sys_reset) and U-Boot's banner after it. If the board is still running
 * 30 s after the timeout, FAIL and the PSCI reset.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include "verdin_imx95.h"
#include "vi95_probe.h"

#define GRACE_S 30u

int main(void)
{
	uint32_t timeout_ms = verdin_imx95_wdog_timeout_ms();
	uint32_t until_s = timeout_ms / 1000u + GRACE_S;

	vi95_probe_banner();
	BUILD_ASSERT(!IS_ENABLED(CONFIG_YAKOGL_VERDIN_IMX95_WDOG_FEED),
		     "imx95_wdog: WDOG3 must not be fed");
	BUILD_ASSERT(CONFIG_YAKOGL_VERDIN_IMX95_LIFETIME_S > 60 + GRACE_S,
		     "the lifetime's PSCI reset comes after the FAIL line");
	VI95_P("not fed: reset expected within %u ms of the arming (uptime now %lld ms)",
	       timeout_ms, (long long)k_uptime_get());
	for (uint32_t s = 10; s <= until_s; s += 10) {
		k_sleep(K_TIMEOUT_ABS_MS((int64_t)s * 1000));
		VI95_P("alive at %u s", s);
	}
	VI95_P("FAIL no reset: still running %u s after WDOG3's %u ms timeout", GRACE_S,
	       timeout_ms);
	vi95_probe_end();
	return 0;
}
