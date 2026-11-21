/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * vi95_probe.h -- what the Verdin iMX95 probes share (vi95_probe.c): the
 * banner, a raw SCMI message, and the end of a run.
 *
 * Every line a probe prints starts with its name and ": " (VI95_PROBE_NAME,
 * the directory's name), so a run's record is grep "^imx95_<probe>: ". Each
 * probe ends with one verdict line, "<probe>: PASS ..." or
 * "<probe>: FAIL <why>", then the PSCI reset (verdin_imx95_reset). The
 * exception is imx95_wdog, whose pass is WDOG3's reset itself.
 */
#ifndef VI95_PROBE_H
#define VI95_PROBE_H

#include <stddef.h>
#include <stdint.h>
#include <zephyr/toolchain.h>

#define VI95_P(fmt, ...) printk(VI95_PROBE_NAME ": " fmt "\n", ##__VA_ARGS__)

/* The build line and WDOG3's arming as the board glue ran it at
 * PRE_KERNEL_1 (CS and TOVAL as found and as left, whether U-Boot left it
 * running). Returns the arming's rc: a probe whose WDOG3 is not armed
 * never gets here (the glue resets at once), so 0. */
int vi95_probe_banner(void);

/* One SCMI command over the board's raw channel (verdin_imx95_scmi):
 * protocol `proto`, message `msg`, payload `w[0..n)`; the reply's return
 * values (after the status) into `ret[0..cap)`. Returns the number of
 * return values (>= 0) with *status the SCMI status (0 SUCCESS), or a
 * negative errno of the transport (*status then 0). */
int vi95_scmi(uint8_t proto, uint8_t msg, const uint32_t *w, size_t n, uint32_t *ret, size_t cap,
	      int32_t *status);

/* An SCMI status's DEN0056 name ("SUCCESS", "DENIED", ...). */
const char *vi95_scmi_status(int32_t status);

/* The verdict was printed: the PSCI reset (Linux comes back through U-Boot). */
FUNC_NORETURN void vi95_probe_end(void);

#endif /* VI95_PROBE_H */
