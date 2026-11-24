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
 * calls come at most CONFIG_YAKOGL_NANOPC_T6_ALIVE_S apart. Any context.
 * With the board layer (CONFIG_YAKOGL_BOARD) it is board_watchdog_feed(). */
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

/* The feeder stopped before a reset call (the lifetime is over, or no
 * progress): the board layer draws its record screen over the picture
 * (board_nanopc_t6.c), which then stays until WDT0's reset. Nothing without
 * CONFIG_YAKOGL_BOARD_RECORD_SCREEN. Timer context. */
#ifdef CONFIG_YAKOGL_BOARD_RECORD_SCREEN
void t6_record_screen(void);
#else
static inline void t6_record_screen(void)
{
}
#endif

/* Print the record of a display adopt (yd_t6_display_adopt, or the first
 * yd_t6_screen_show) and its return value `rc`: what it found, what it did
 * up to its stop, then one line with the stop's name. Every line starts with
 * "<tag>: " (driver/tools/kexec-zephyr/t6-verdict.sh reads the t6_display
 * probe's). */
struct yd_t6_display_adopt;
void t6_display_print_adopt(const char *tag, int rc, const struct yd_t6_display_adopt *a);

/* The name of a stage of the cold display chain (YD_T6_DISPLAY_COLD_STAGE_*;
 * "none" for 0, "unknown" for a number that is no stage). */
const char *t6_display_cold_stage_name(uint32_t stage);

/* Print where the cold display chain ended (yd_t6_display_cold, or a first
 * yd_t6_screen_show that took the cold route): its return value `rc`, the
 * time `ms` its call took and the record `c`, with the stage's and the
 * error's names, as one line "<tag>: chain rc ..."
 * (driver/tools/kexec-zephyr/t6-verdict.sh reads it). */
struct yd_t6_display_cold;
void t6_display_print_cold(const char *tag, int rc, long long ms,
			   const struct yd_t6_display_cold *c);

/* The callbacks the display's C entries take (yakogl_drivers.h): a busy
 * wait of `us` microseconds, and the uptime in microseconds. */
void t6_delay_us(uint32_t us);
uint64_t t6_now_us(void);

/* The read-backs of driver/docs/nanopc-t6-display.md, section 5, over a
 * picture that is up: yd_t6_display_status with a frame check over
 * T6_DISPLAY_CHECK_WAIT_US and T6_DISPLAY_TIMED_FRAMES frames timed (about
 * 1.1 s at 60 Hz), the watchdog fed after it, then the values printed, every
 * line "<tag>: " (the HPD line is the proof of a connected display;
 * driver/tools/kexec-zephyr/t6-verdict.sh reads the lines of the display
 * probes and of the board layer at the end of a run). Returns
 * yd_t6_display_status's rc. Thread context. */
#define T6_DISPLAY_CHECK_WAIT_US 100000
#define T6_DISPLAY_TIMED_FRAMES  60
struct yd_t6_display_bases;
struct yd_t6_display_status;
int t6_display_read_back(const char *tag, const struct yd_t6_display_bases *b,
			 struct yd_t6_display_status *s);

#endif /* YAKOGL_NANOPC_T6_H */
