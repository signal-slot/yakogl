/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * WDOG1 armed by the image itself, then fed for a bounded time
 * (CONFIG_GC_BOARD_WDOG_SELF_ARM; docs/standalone-boot.md, "What the image
 * does on a go entry"; the bring-up record is
 * docs/notes/imx8mp-netboot.md, "Watchdog").
 *
 * Why: an image that U-Boot starts with "go" cannot rely on any earlier stage
 * having armed WDOG1, and gc_sample.c only services the one it finds. The
 * register logic -- the CCM WDOG root and gate, the WCR value for every state
 * WDOG1 can be found in, the read-back and its refusals -- is driver/rust
 * ip::imx_wdog behind yd_imx_wdog_arm(); this file is the Zephyr side.
 *
 * Order:
 *   PRE_KERNEL_1 1 (after gc_sample.c's and gc_result.c's mappings at 0):
 *     map WDOG1 and the CCM, yd_imx_wdog_arm(60 s, WDOG_B on), then either
 *     log "gc_wdog_arm: armed ..." and leave a kind WDOG tag 1 breadcrumb,
 *     or log "gc_wdog_arm: FAILED ...", store tag 2 and reset the SoC
 *     through PSCI at once: an image that could not arm its watchdog must
 *     not run;
 *   POST_KERNEL 0: a 1 s timer feeds through gc_sample_service_wdog() until
 *     CONFIG_GC_BOARD_WDOG_SECONDS of uptime, then stops, and WDOG1 resets the
 *     SoC within 60 s. Every other feeder in gc_board (the hold, the fatal
 *     handler, GC_SAMPLE_WDOG_TIMER_FEED) goes through the same gate,
 *     gc_wdog_arm_may_feed(), so none of them can outlive the bound.
 *
 * CONFIG_GC_BOARD_WDOG_PROOF: armed, never fed, record tag 3, and a one-shot
 * timer as a software backstop (tag 4, PSCI reset) in case WDOG1 does not
 * reset the SoC.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/kernel/mm.h>

#include <yakogl_drivers.h>

#include "gc_result.h"
#include "board_internal.h"
#include "gc_sample.h"
#include "gc_wdog_arm.h"

#define WDOG1_PHYS      0x30280000u /* hal_nxp WDOG1_BASE */
#define CCM_PHYS        0x30380000u /* hal_nxp CCM_BASE */
#define CCM_SIZE        0x10000u    /* TARGET_ROOT[141] ends at 0xc6ff */
#define WDOG_TIMEOUT_MS 60000u

BUILD_ASSERT(CONFIG_GC_SAMPLE_WDOG_BASE == WDOG1_PHYS,
	     "gc_sample.c services the watchdog this file arms");

static bool s_armed;
static struct yd_imx_wdog_arm s_arm;
static uint32_t s_armed_at_ms;
static volatile uint32_t s_feeds; /* feed_tick's feeds; read by gc_result_stage() */

uint32_t gc_wdog_arm_feeds(void)
{
	return s_feeds;
}

bool gc_wdog_arm_may_feed(void)
{
	if (!s_armed) {
		return true;
	}
	if (IS_ENABLED(CONFIG_GC_BOARD_WDOG_PROOF)) {
		return false;
	}
	/* The board layer's "still making progress" (board_watchdog_feed). */
	return (IS_ENABLED(CONFIG_GC_BOARD_WDOG_UNBOUNDED) ||
		k_uptime_get() < (int64_t)CONFIG_GC_BOARD_WDOG_SECONDS * MSEC_PER_SEC) &&
	       board_wdog_alive();
}

static uint32_t rec_w1(void)
{
	return (uint32_t)s_arm.wcr_found | (uint32_t)s_arm.wcr_now << 16;
}

static uint32_t rec_w2(void)
{
	return (s_arm.flags & 0xffu) | (s_arm.refusal & 0xffu) << 8 |
	       (uint32_t)s_arm.wmcr_found << 16;
}

/* PSCI SYSTEM_RESET straight through SMC (SMCCC 0x84000009), as the fatal
 * handler does: at PRE_KERNEL_1 no reboot driver can be assumed. */
static FUNC_NORETURN void psci_system_reset(void)
{
	register unsigned long x0 __asm__("x0") = 0x84000009UL;

	__asm__ volatile("smc #0" : "+r"(x0) : : "x1", "x2", "x3", "memory");
	for (;;) {
	}
}

static int wdog_arm_init(void)
{
	mm_reg_t wdog;
	mm_reg_t ccm;
	int rc;

	device_map(&wdog, WDOG1_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&ccm, CCM_PHYS, CCM_SIZE, K_MEM_CACHE_NONE);
	/* WDOG_B on a timeout: the board's U-Boot and Linux device trees give
	 * WDOG1 fsl,ext-reset-output (the reset path every kexec run took). */
	rc = yd_imx_wdog_arm(wdog, ccm, WDOG_TIMEOUT_MS, 1u, &s_arm);
	s_armed_at_ms = k_uptime_get_32();
	if (rc != 0) {
		printk("gc_wdog_arm: FAILED (rc %d, refusal %u): WCR found 0x%04x read 0x%04x, "
		       "WMCR 0x%04x, flags 0x%x: resetting the SoC now\n",
		       rc, s_arm.refusal, s_arm.wcr_found, s_arm.wcr_now, s_arm.wmcr_found,
		       s_arm.flags);
		gc_result_write_tag(GC_RESULT_WDOG, GC_RESULT_WDOG_REFUSED, rec_w1(), rec_w2(),
				    s_armed_at_ms);
		psci_system_reset();
	}
	s_armed = true;
	printk("gc_wdog_arm: armed WCR 0x%04x -> 0x%04x (%s), WMCR 0x%04x, flags 0x%x: "
	       "expiry %u ms, fed for %u s\n",
	       s_arm.wcr_found, s_arm.wcr_now,
	       (s_arm.flags & YD_IMX_WDOG_WAS_RUNNING) ? "was running" : "was stopped",
	       s_arm.wmcr_found, s_arm.flags, s_arm.timeout_ms,
	       IS_ENABLED(CONFIG_GC_BOARD_WDOG_PROOF) ? 0 : CONFIG_GC_BOARD_WDOG_SECONDS);
	printk(BOARD_WDOG_ARMED_LINE);
	if (IS_ENABLED(CONFIG_GC_BOARD_WDOG_PROOF)) {
		gc_result_write_tag(GC_RESULT_WDOG, GC_RESULT_WDOG_PROOF, rec_w1(), rec_w2(),
				    s_armed_at_ms);
	} else {
		gc_result_breadcrumb(GC_RESULT_WDOG, GC_RESULT_WDOG_ARMED, rec_w1(), rec_w2(),
				     s_armed_at_ms);
	}
	return 0;
}
SYS_INIT(wdog_arm_init, PRE_KERNEL_1, 1);

#ifdef CONFIG_GC_BOARD_WDOG_PROOF
static void proof_backstop(struct k_timer *t)
{
	ARG_UNUSED(t);
	/* ISR context: the record, then the reset; nothing that can block. */
	printk("gc_wdog_arm: proof FAILED: no WDOG1 reset within %u s of arming: "
	       "software reset\n", CONFIG_GC_BOARD_WDOG_PROOF_BACKSTOP_SECONDS);
	gc_result_write_tag(GC_RESULT_WDOG, GC_RESULT_WDOG_PROOF_ALIVE, rec_w1(), rec_w2(),
			    k_uptime_get_32());
	psci_system_reset();
}
static K_TIMER_DEFINE(s_backstop, proof_backstop, NULL);

void gc_wdog_arm_proof_hold(void)
{
	/* The sample's own records replaced the proof record: put it back, so
	 * the reader finds what this run is about. */
	gc_result_write_tag(GC_RESULT_WDOG, GC_RESULT_WDOG_PROOF, rec_w1(), rec_w2(),
			    s_armed_at_ms);
	printk("gc_wdog_arm: proof: sample finished; waiting for WDOG1 (armed at %u ms)\n",
	       s_armed_at_ms);
	for (;;) {
		k_sleep(K_FOREVER);
	}
}
#else
void gc_wdog_arm_proof_hold(void)
{
}

static void feed_tick(struct k_timer *t)
{
	if (!gc_wdog_arm_may_feed()) {
		k_timer_stop(t);
		printk("gc_wdog_arm: feeding bound of %u s over; WDOG1 resets the SoC within "
		       "%u ms\n", CONFIG_GC_BOARD_WDOG_SECONDS, s_arm.timeout_ms);
		return;
	}
	gc_sample_service_wdog();
	s_feeds++;
}
static K_TIMER_DEFINE(s_feeder, feed_tick, NULL);
#endif

static int wdog_feed_start(void)
{
#ifdef CONFIG_GC_BOARD_WDOG_PROOF
	uint32_t now = k_uptime_get_32();
	uint32_t at = s_armed_at_ms + CONFIG_GC_BOARD_WDOG_PROOF_BACKSTOP_SECONDS * MSEC_PER_SEC;

	printk("gc_wdog_arm: proof: not feeding; WDOG1 must reset the SoC within %u ms of "
	       "arming (software backstop at %u s)\n",
	       s_arm.timeout_ms, CONFIG_GC_BOARD_WDOG_PROOF_BACKSTOP_SECONDS);
	k_timer_start(&s_backstop, K_MSEC(at > now ? at - now : 0), K_NO_WAIT);
#else
	k_timer_start(&s_feeder, K_SECONDS(1), K_SECONDS(1));
#endif
	return 0;
}
SYS_INIT(wdog_feed_start, POST_KERNEL, 0);
