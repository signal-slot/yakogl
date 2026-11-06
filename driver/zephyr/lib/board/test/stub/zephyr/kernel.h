/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host stub: the timer and uptime calls rpi_wdog.c makes. A timer fires only
 * when the test calls its handler. */
#ifndef STUB_ZEPHYR_KERNEL_H
#define STUB_ZEPHYR_KERNEL_H
#include <stdint.h>
#include <stdbool.h>
#include <zephyr/toolchain.h>
struct k_timer {
	void (*expiry)(struct k_timer *t);
	bool running;
	int64_t first_ms, period_ms;
};
#define K_MSEC(ms) ((int64_t)(ms))
#define K_TIMER_DEFINE(name, expiry_fn, stop_fn) \
	static struct k_timer name = { .expiry = (expiry_fn) }
static inline void k_timer_start(struct k_timer *t, int64_t first, int64_t period)
{
	t->running = true;
	t->first_ms = first;
	t->period_ms = period;
}
static inline void k_timer_stop(struct k_timer *t)
{
	t->running = false;
}
int64_t k_uptime_get(void);
#endif
