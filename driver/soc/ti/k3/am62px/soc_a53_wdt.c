/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Feed the main-domain RTI watchdog a previous boot stage left running.
 *
 * When Zephyr is entered through kexec from Linux, the K3 RTI watchdog at
 * main_rti0 keeps running (60 s period, 50 % window, started by the boot
 * loader or Linux) and cannot be stopped. The RTI only accepts a service in
 * its open window (down counter below half the preload); a service in the
 * closed window resets the SoC at once. So this samples the counter and
 * feeds only when it is below 45 % of the preload. Zephyr's wdt_ti_rti
 * driver has no counter read and would reconfigure a locked watchdog, hence
 * the direct register access. A watchdog that is not running is left alone.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/logging/log.h>
#include "soc_a53_wdt.h"

LOG_MODULE_REGISTER(rti_feed, CONFIG_SOC_LOG_LEVEL);

#define RTI_NODE DT_NODELABEL(main_rti0)

#define RTI_DWDCTRL         0x90
#define RTI_DWDPRLD         0x94
#define RTI_WDKEY           0x9c
#define RTI_DWDCNTR         0xa0
#define RTI_WDKEY_SEQ0      0xe51a
#define RTI_WDKEY_SEQ1      0xa35c
#define RTI_DWDCTRL_ENABLED 0xa98559da
#define RTI_PRELOAD_SHIFT   13

static mm_reg_t s_rti;
static uint32_t s_feeds;
static int64_t s_deadline;
static volatile bool s_released;   /* set from a fatal-error handler: no locks */

static void rti_feed_tick(struct k_timer *t)
{
	uint32_t reload = (sys_read32(s_rti + RTI_DWDPRLD) + 1) << RTI_PRELOAD_SHIFT;
	uint32_t cntr = sys_read32(s_rti + RTI_DWDCNTR);

	/* A hung system must not be kept alive forever: after the configured
	 * lifetime the watchdog is left alone and resets the SoC. */
	if (s_released) {
		k_timer_stop(t);
		return;
	}
	if (s_deadline && k_uptime_get() > s_deadline) {
		LOG_WRN("RTI feeder lifetime over after %u feeds; letting the watchdog reset",
			s_feeds);
		k_timer_stop(t);
		return;
	}
	/* open window: the last 50 % of the period; stay clear of its edge */
	if (cntr < reload * 45 / 100) {
		sys_write32(RTI_WDKEY_SEQ0, s_rti + RTI_WDKEY);
		sys_write32(RTI_WDKEY_SEQ1, s_rti + RTI_WDKEY);
		s_feeds++;
	}
}

K_TIMER_DEFINE(rti_feed_timer, rti_feed_tick, NULL);

void soc_am62px_inherited_wdt_release(void)
{
	/* Only a flag: the caller may be an exception handler on one CPU while
	 * the timer fires on another; the next tick stops the timer itself. */
	s_released = true;
	LOG_WRN("RTI feeder released after %u feeds; the watchdog resets the SoC", s_feeds);
}

static int rti_feed_init(void)
{
	device_map(&s_rti, DT_REG_ADDR(RTI_NODE), DT_REG_SIZE(RTI_NODE), K_MEM_CACHE_NONE);
	if (sys_read32(s_rti + RTI_DWDCTRL) != RTI_DWDCTRL_ENABLED) {
		LOG_INF("RTI watchdog not running, nothing to feed");
		return 0;
	}
	LOG_INF("RTI watchdog running (preload 0x%x, counter 0x%x): feeding it",
		sys_read32(s_rti + RTI_DWDPRLD), sys_read32(s_rti + RTI_DWDCNTR));
	if (CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT_SECONDS > 0) {
		s_deadline = k_uptime_get() +
			     (int64_t)CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT_SECONDS * MSEC_PER_SEC;
	}
	k_timer_start(&rti_feed_timer, K_MSEC(100), K_MSEC(100));
	return 0;
}

/* Right after the kernel is up, before any driver init that may take long. */
SYS_INIT(rti_feed_init, POST_KERNEL, 0);
