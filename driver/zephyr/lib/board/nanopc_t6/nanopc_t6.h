/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * nanopc_t6.h -- what a sample on the NanoPC-T6 calls (nanopc_t6.c).
 *
 * WDT0 is armed before main() and fed from a timer until
 * CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S of uptime at the latest. A sample says
 * it is still making progress with t6_watchdog_feed() and ends with
 * t6_reset_to_linux(), which resets the board back into Armbian.
 */
#ifndef YAKOGL_NANOPC_T6_H
#define YAKOGL_NANOPC_T6_H

#include <stdint.h>
#include <zephyr/toolchain.h>

/* "Still making progress." Once called, the feeding continues only while the
 * calls come at most CONFIG_YAKOGL_NANOPC_T6_ALIVE_S apart. Any context. */
void t6_watchdog_feed(void);

/* Uptime in ms after which nothing feeds WDT0. */
uint32_t t6_watchdog_deadline_ms(void);

/* WDT0's period in ms (rounded down), 0 if it is not armed. */
uint32_t t6_watchdog_period_ms(void);

/* The end of a run: the feeding stops, `why` is printed, then PSCI
 * SYSTEM_RESET. If the firmware returns, WDT0 resets the board within one
 * period. With CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF it waits for WDT0 instead.
 * Thread context. */
FUNC_NORETURN void t6_reset_to_linux(const char *why);

#endif /* YAKOGL_NANOPC_T6_H */
