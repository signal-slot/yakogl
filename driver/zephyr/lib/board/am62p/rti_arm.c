/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * AM62P: make sure the MAIN RTI0 watchdog is running, arming it when no
 * earlier stage did, and feed it for a bounded time
 * (CONFIG_YAKOGL_AM62P_RTI_SELF_ARM).
 *
 * Why: an image entered from U-Boot with "go" (docs/standalone-boot.md, "What
 * the image does on a go entry") finds the RTI stopped, because this U-Boot has
 * no watchdog driver. The SoC-layer feeder
 * (CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT) only feeds a running RTI, so such
 * an image had no way back from a hang. An image entered through kexec finds
 * the RTI running (Linux started it); then this code only feeds it, as the
 * SoC-layer feeder would. The two are exclusive (Kconfig): the SoC-layer feeder reads
 * the RTI at POST_KERNEL 0, before any TISCI power-on, which on a U-Boot entry
 * may be a read of a module that is off.
 *
 * Order, at POST_KERNEL CONFIG_YAKOGL_AM62P_RTI_SELF_ARM_INIT_PRIORITY
 * (after TISCI):
 *   1. TISCI device 125 (MAIN RTI0) on, unless it already is. This is done
 *      here rather than in the board's tisci_power_on list: the list is the
 *      board DTS's (each sample would have to override it), and here
 *      it is tied to the option and ordered before the first register read;
 *   2. if DWDCTRL says enabled: feed only (kexec entry);
 *   3. otherwise set the functional clock's parent to 2 (the 32768 Hz
 *      parent Linux uses), read the rate back through TISCI, refuse a rate
 *      the board did not report or one that cannot hold 60 s, program the
 *      preload from that rate, a 50 % window and the NMI reaction (the ESM
 *      turns it into the SoC reset: the path every kexec rc=6 run took; it
 *      is also the reaction Linux's driver programs), enable, read all three
 *      back;
 *   4. log "rti_arm: armed ..." (the netboot runner refuses an image without
 *      this literal) and feed from a timer until the bound, then stop and
 *      let the RTI reset the SoC.
 * Any refusal in 1-3 logs "rti_arm: FAILED ..." and resets the SoC at once:
 * an image that could not arm its watchdog must not run.
 *
 * Register offsets and values: the TI K3 RTI as Zephyr's
 * drivers/watchdog/wdt_ti_rti.c (Apache-2.0) defines them. The feeding rule
 * (feed only when the down counter is in the open half) is the SoC-layer
 * feeder's (soc/ti/k3/am62px/soc_a53_wdt.c).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <zephyr/init.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/reboot.h>
#include <zephyr/logging/log.h>
#include <zephyr/drivers/firmware/tisci/tisci.h>

#include "rti_arm.h"
#include "board_internal.h"

LOG_MODULE_REGISTER(rti_arm, LOG_LEVEL_INF);

#define RTI_NODE   DT_NODELABEL(main_rti0)
#define TISCI_NODE DT_NODELABEL(dmsc)

/* MAIN RTI0 in TISCI ids: the U-Boot and Linux device trees give it
 * power-domains = <&k3_pds 125 ...>, clocks = <&k3_clks 125 0> and
 * assigned-clock-parents = <&k3_clks 125 2>. */
#define RTI_TISCI_DEV    125
#define RTI_TISCI_CLK    0
#define RTI_TISCI_PARENT 2

#define RTI_DWDCTRL     0x90
#define RTI_DWDPRLD     0x94
#define RTI_WDSTATUS    0x98
#define RTI_WDKEY       0x9c
#define RTI_DWDCNTR     0xa0
#define RTI_WWDRXNCTRL  0xa4
#define RTI_WWDSIZECTRL 0xa8

#define RTI_DWDCTRL_ENABLED 0xa98559da
#define RTI_WDSTATUS_CLEAR  0x3f
#define RTI_WWDRX_NMI       0xa
#define RTI_WWDSIZE_50P     0x50
#define RTI_WDKEY_SEQ0      0xe51a
#define RTI_WDKEY_SEQ1      0xa35c

#define RTI_TIMEOUT_SECONDS 60

BUILD_ASSERT(CONFIG_YAKOGL_AM62P_RTI_SELF_ARM_INIT_PRIORITY > CONFIG_TISCI_INIT_PRIORITY,
	     "the RTI is armed through TISCI, so after the TISCI driver");

static mm_reg_t s_rti;
static uint32_t s_feeds;
static int64_t s_deadline;

static void rti_feed_tick(struct k_timer *t)
{
	uint32_t reload = (sys_read32(s_rti + RTI_DWDPRLD) + 1) << RTI_ARM_PRELOAD_SHIFT;
	uint32_t cntr = sys_read32(s_rti + RTI_DWDCNTR);

	/* A hung system must not be kept alive: after the bound the RTI is
	 * left alone and resets the SoC. */
	if (k_uptime_get() > s_deadline) {
		LOG_WRN("rti_arm: feeding bound of %u s over after %u feeds; the RTI resets the SoC",
			CONFIG_YAKOGL_AM62P_WDT_SECONDS, s_feeds);
		k_timer_stop(t);
		return;
	}
	/* The board layer's "still making progress" (board_watchdog_feed). */
	if (!board_wdog_alive()) {
		LOG_WRN("rti_arm: the sample stopped saying it is alive; the RTI resets the SoC");
		k_timer_stop(t);
		return;
	}
	/* The open window is the last 50 % of the period; a key written in
	 * the closed window resets the SoC at once, so stay clear of the edge. */
	if (cntr < reload * 45 / 100) {
		sys_write32(RTI_WDKEY_SEQ0, s_rti + RTI_WDKEY);
		sys_write32(RTI_WDKEY_SEQ1, s_rti + RTI_WDKEY);
		s_feeds++;
	}
}

K_TIMER_DEFINE(rti_feed_timer, rti_feed_tick, NULL);

#ifdef CONFIG_YAKOGL_AM62P_RTI_PROOF
/* The proof image's backstop: the RTI should have reset the SoC about 60 s
 * after it was armed. If the image is still alive after
 * CONFIG_YAKOGL_AM62P_RTI_PROOF_BACKSTOP_SECONDS, the RTI path does not
 * work: say so in a line of its own and reset by software, so the proof
 * needs no one at the power switch unless the CPU itself hangs as well. */
static void rti_proof_backstop(struct k_work *work)
{
	ARG_UNUSED(work);
	LOG_ERR("rti_arm: proof FAILED: no RTI reset within %u s of arming (counter 0x%x): "
		"software reset",
		CONFIG_YAKOGL_AM62P_RTI_PROOF_BACKSTOP_SECONDS, sys_read32(s_rti + RTI_DWDCNTR));
	k_busy_wait(20000);
	sys_reboot(SYS_REBOOT_COLD);
}

static K_WORK_DELAYABLE_DEFINE(rti_proof_work, rti_proof_backstop);
#endif

static void start_feeding(void)
{
#ifdef CONFIG_YAKOGL_AM62P_RTI_PROOF
	LOG_WRN("rti_arm: proof: not feeding; the RTI must reset the SoC within 60 s "
		"(software backstop at %u s)",
		CONFIG_YAKOGL_AM62P_RTI_PROOF_BACKSTOP_SECONDS);
	k_work_schedule(&rti_proof_work, K_SECONDS(CONFIG_YAKOGL_AM62P_RTI_PROOF_BACKSTOP_SECONDS));
#else
	s_deadline = k_uptime_get() + (int64_t)CONFIG_YAKOGL_AM62P_WDT_SECONDS * MSEC_PER_SEC;
	k_timer_start(&rti_feed_timer, K_MSEC(100), K_MSEC(100));
#endif
}

static FUNC_NORETURN void refuse(const char *what, long long value)
{
	LOG_ERR("rti_arm: FAILED (%s: %lld): resetting the SoC now", what, value);
	/* let the minimal log reach the UART before the reset */
	k_busy_wait(20000);
	sys_reboot(SYS_REBOOT_COLD);
	CODE_UNREACHABLE;
}

static int rti_arm_init(void)
{
	const struct device *tisci = DEVICE_DT_GET(TISCI_NODE);
	bool requested = false;
	bool on = false;
	uint64_t rate = 0;
	int32_t preload;
	int rc;

	if (!device_is_ready(tisci)) {
		refuse("TISCI not ready", 0);
	}
	/* 1. the module on (a request is NACKed while Linux still holds it
	 * exclusively, and none is needed then) */
	rc = tisci_cmd_dev_is_on(tisci, RTI_TISCI_DEV, &requested, &on);
	if (rc != 0) {
		refuse("TISCI state of device 125", rc);
	}
	if (!on) {
		rc = tisci_cmd_get_device(tisci, RTI_TISCI_DEV);
		if (rc != 0) {
			refuse("TISCI device 125 on", rc);
		}
	}
	device_map(&s_rti, DT_REG_ADDR(RTI_NODE), DT_REG_SIZE(RTI_NODE), K_MEM_CACHE_NONE);

	/* 2. a running RTI (kexec entry): feed it, change nothing */
	if (sys_read32(s_rti + RTI_DWDCTRL) == RTI_DWDCTRL_ENABLED) {
		LOG_INF("rti_arm: RTI already running (preload 0x%x, counter 0x%x): feeding it for %u s",
			sys_read32(s_rti + RTI_DWDPRLD), sys_read32(s_rti + RTI_DWDCNTR),
			CONFIG_YAKOGL_AM62P_WDT_SECONDS);
		printk(BOARD_WDOG_ARMED_LINE);
		start_feeding();
		return 0;
	}

	/* 3. arm it from the clock rate the firmware reports */
	rc = tisci_cmd_clk_set_parent(tisci, RTI_TISCI_DEV, RTI_TISCI_CLK, RTI_TISCI_PARENT);
	if (rc != 0) {
		/* not a refusal by itself: the rate read below decides */
		LOG_WRN("rti_arm: setting clock parent %u failed (%d)", RTI_TISCI_PARENT, rc);
	}
	rc = tisci_cmd_clk_get_freq(tisci, RTI_TISCI_DEV, RTI_TISCI_CLK, &rate);
	if (rc != 0) {
		refuse("TISCI rate of clock 125:0", rc);
	}
	if (!rti_arm_rate_known(rate)) {
		refuse("RTI clock rate is not one the board reported, Hz", (long long)rate);
	}
	preload = rti_arm_preload(rate, RTI_TIMEOUT_SECONDS);
	if (preload < 0) {
		refuse("60 s does not fit the 12-bit preload at this rate, Hz", (long long)rate);
	}
	sys_write32(RTI_WDSTATUS_CLEAR, s_rti + RTI_WDSTATUS);
	sys_write32(RTI_WWDRX_NMI, s_rti + RTI_WWDRXNCTRL);
	sys_write32((uint32_t)preload, s_rti + RTI_DWDPRLD);
	sys_write32(RTI_WWDSIZE_50P, s_rti + RTI_WWDSIZECTRL);
	sys_write32(RTI_DWDCTRL_ENABLED, s_rti + RTI_DWDCTRL);

	uint32_t ctrl = sys_read32(s_rti + RTI_DWDCTRL);
	uint32_t prld = sys_read32(s_rti + RTI_DWDPRLD);
	uint32_t size = sys_read32(s_rti + RTI_WWDSIZECTRL);

	if (ctrl != RTI_DWDCTRL_ENABLED) {
		refuse("DWDCTRL read back", ctrl);
	}
	if (prld != (uint32_t)preload) {
		refuse("DWDPRLD read back", prld);
	}
	if (size != RTI_WWDSIZE_50P) {
		refuse("WWDSIZECTRL read back", size);
	}
	/* 4. the marker the netboot runner looks for, then the bounded feeding */
	LOG_INF("rti_arm: armed preload 0x%x rate %llu Hz: expiry %llu ms, fed for %u s", prld,
		(unsigned long long)rate, (unsigned long long)rti_arm_expiry_ms(prld, rate),
		IS_ENABLED(CONFIG_YAKOGL_AM62P_RTI_PROOF) ? 0 : CONFIG_YAKOGL_AM62P_WDT_SECONDS);
	printk(BOARD_WDOG_ARMED_LINE);
	start_feeding();
	return 0;
}

SYS_INIT(rti_arm_init, POST_KERNEL, CONFIG_YAKOGL_AM62P_RTI_SELF_ARM_INIT_PRIORITY);
