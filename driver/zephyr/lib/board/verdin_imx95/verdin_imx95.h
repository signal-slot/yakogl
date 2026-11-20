/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * verdin_imx95.h -- the Verdin iMX95 board glue (verdin_imx95.c), in every
 * image built for the board (CONFIG_YAKOGL_VERDIN_IMX95).
 */
#ifndef YAKOGL_VERDIN_IMX95_H
#define YAKOGL_VERDIN_IMX95_H

#include <zephyr/toolchain.h>

/* The end of a run: `why` is printed, then PSCI SYSTEM_RESET (TF-A's BL31,
 * which asks the System Manager for the reset). If it returns, the error is
 * printed and the core halts: no watchdog of the SoC is armed by this image
 * (verdin_imx95_wdog_stub). Any context (the lifetime timer calls it from
 * its handler). Not run on a board yet. */
FUNC_NORETURN void verdin_imx95_reset(const char *why);

/* STUB: the i.MX 95's watchdog is not driven by this image. Prints one line
 * that says so at boot; returns -ENOTSUP. */
int verdin_imx95_wdog_stub(void);

#endif /* YAKOGL_VERDIN_IMX95_H */
