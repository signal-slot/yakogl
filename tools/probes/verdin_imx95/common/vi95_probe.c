/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * What the Verdin iMX95 probes share (vi95_probe.h). The board glue
 * (driver/zephyr/lib/board/verdin_imx95) has armed WDOG3 at PRE_KERNEL_1
 * before any of this runs, and ends every run with the PSCI reset.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <string.h>

#include <yakogl_drivers.h>

#include "verdin_imx95.h"
#include "verdin_imx95_scmi.h"
#include "vi95_probe.h"

int vi95_probe_banner(void)
{
	int rc;
	const struct yd_wdog32_arm *a = verdin_imx95_wdog_arm(&rc);

	VI95_P("build %s", VI95_PROBE_BUILD);
	/* "wdog" is the key a script reads: found as U-Boot left it, then
	 * as armed. */
	VI95_P("wdog found CS 0x%08x TOVAL 0x%08x (%s); armed rc %d CS 0x%08x TOVAL 0x%08x "
	       "timeout %u ms",
	       a->cs_found, a->toval_found,
	       (a->flags & YD_WDOG32_WAS_RUNNING) ? "running" : "stopped", rc, a->cs_now,
	       a->toval_now, a->timeout_ms);
	return rc;
}

int vi95_scmi(uint8_t proto, uint8_t msg, const uint32_t *w, size_t n, uint32_t *ret, size_t cap,
	      int32_t *status)
{
	uint32_t tx[1 + 8];
	uint32_t rx[YD_IMX95_SCMI_MAX_MSG / 4];
	int len;
	size_t k;

	*status = 0;
	if (n > 8) {
		return -EINVAL;
	}
	/* DEN0056: message id 7:0, type 9:8 (0, a command), protocol 17:10,
	 * token 27:18 (0: one message in flight, as the Rust side's) */
	tx[0] = (uint32_t)msg | (uint32_t)proto << 10;
	if (n != 0) {
		memcpy(&tx[1], w, n * 4);
	}
	len = verdin_imx95_scmi((const uint8_t *)tx, 4 * (1 + n), (uint8_t *)rx, sizeof(rx));
	if (len < 0) {
		return len;
	}
	if (len < 8) {
		return -EBADMSG;
	}
	*status = (int32_t)rx[1];
	k = (size_t)(len - 8) / 4;
	if (k > cap) {
		k = cap;
	}
	memcpy(ret, &rx[2], k * 4);
	return (int)k;
}

const char *vi95_scmi_status(int32_t status)
{
	static const char *const names[] = {
		"SUCCESS",      "NOT_SUPPORTED", "INVALID_PARAMETERS", "DENIED",
		"NOT_FOUND",    "OUT_OF_RANGE",  "BUSY",               "COMMS_ERROR",
		"GENERIC_ERROR", "HARDWARE_ERROR", "PROTOCOL_ERROR",
	};

	return status <= 0 && -status < (int32_t)ARRAY_SIZE(names) ? names[-status] : "unknown";
}

FUNC_NORETURN void vi95_probe_end(void)
{
	verdin_imx95_reset(VI95_PROBE_NAME " done");
}
