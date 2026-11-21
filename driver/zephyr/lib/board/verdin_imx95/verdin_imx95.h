/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * verdin_imx95.h -- the Verdin iMX95 board glue (verdin_imx95.c), in every
 * image built for the board (CONFIG_YAKOGL_VERDIN_IMX95).
 */
#ifndef YAKOGL_VERDIN_IMX95_H
#define YAKOGL_VERDIN_IMX95_H

#include <stdint.h>
#include <zephyr/toolchain.h>

/* The end of a run: `why` is printed, then PSCI SYSTEM_RESET (TF-A's BL31,
 * which asks the System Manager for the reset). If it returns, the error is
 * printed and the core halts with interrupts off, so nothing feeds WDOG3 and
 * it resets the board (verdin_imx95_wdog.c). Any context (the lifetime timer
 * calls it from its handler). Not run on a board yet. */
FUNC_NORETURN void verdin_imx95_reset(const char *why);

/* WDOG3's timeout as armed at boot (verdin_imx95_wdog.c), 0 if the arming
 * refused (the image then resets at once). Any context. */
uint32_t verdin_imx95_wdog_timeout_ms(void);

/* The board layer's last record into the BBM GPRs if it is still pending
 * (verdin_imx95_record.c, CONFIG_YAKOGL_VERDIN_IMX95_RECORD_BBM); without the
 * sink a no-op. verdin_imx95_reset calls it first. Any context. */
void verdin_imx95_record_flush(void);

struct yd_wdog32_arm;
/* WDOG3's arming as it ran at PRE_KERNEL_1 (yakogl_drivers.h struct
 * yd_wdog32_arm: CS and TOVAL as found and as left, whether it was
 * running, the refusal) and its return code (0 armed). For the probes'
 * record; any context after POST_KERNEL. */
const struct yd_wdog32_arm *verdin_imx95_wdog_arm(int *rc);

#endif /* YAKOGL_VERDIN_IMX95_H */
