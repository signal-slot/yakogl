/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The NanoPC-T6 board glue, the Zephyr side (CONFIG_YAKOGL_NANOPC_T6;
 * driver/docs/nanopc-t6-mali.md, "Z1"). The register logic is driver/rust:
 * ip::dw_wdt and soc::rk3588 behind yd_dw_wdt_arm() / yd_dw_wdt_feed(), and
 * aarch64 behind yd_gic_eoi_mode_reset() / yd_psci_system_reset(), and the
 * feeder's decision behind yd_t6_feed_verdict() (board::nanopc_t6). This file
 * is registration only: the init hooks, the mappings, a timer, the prints.
 *
 * Order (the T6:<tag> lines are the early banner, board::nanopc_t6_early;
 * a tag missing from the UART says where the image stopped):
 *   reset.S's plat_prep_c hook, the image's first code (MMU off, no stack
 *     yet): yd_t6_early_entry prints T6:A with the EL found, points VBAR_EL2
 *     at a reporter (T6:X for any later exception taken to EL2), and with
 *     CONFIG_YAKOGL_NANOPC_T6_VHE_EXIT writes HCR_EL2 = RW only before
 *     Zephyr's z_arm64_el2_init reads it (Linux's VHE kexec leaves E2H set;
 *     driver/rust/src/aarch64.rs).
 *   z_arm64_el1_plat_init, the end of z_arm64_el1_init (MMU off): T6:A1.
 *   The pre-arm window, before WDT0 is armed by this image: reset.S (EL2
 *   to EL1), z_prep_c (BSS zero, data copy, z_arm64_mm_init: the MMU tables
 *   for the image and the board's regions, z_arm64_interrupt_init), z_cstart up to
 *   PRE_KERNEL_1 (the EARLY hook below, arch_kernel_init, the device
 *   states, the SoC and board early hooks), then t6_wdt_arm itself up to
 *   its arming write, including its two device_map() calls. t6_wdt_arm is
 *   the first PRE_KERNEL_1 hook (the gate row checks it in t6_hello's
 *   ELF). This is straight-line code with no wait on a device, and its
 *   length is not measured; but a fault in it halts the CPU for good: an
 *   exception, or the k_panic() in the arm64 MMU code when the
 *   CONFIG_MAX_XLAT_TABLES (8) tables run out, which device_map() can
 *   reach too. Nothing of the image's own covers that. Linux's dw_wdt
 *   does, opt-in: nanopc-t6.sh --go --linux-wdt opens /dev/watchdog0 in
 *   the jump's own ssh command, so WDT0 is running with Linux's period
 *   (44.7 s from the open) when the image starts, and t6_wdt_arm re-arms
 *   it ("was running").
 *   EARLY: the GICv3 CPU interface back to EOImode 0 (Linux left it in split
 *     mode; with it, the system timer would tick once).
 *   PRE_KERNEL_1 0: map uart2 (T6:A2), WDT0 and the CRU, arm WDT0
 *     (yd_dw_wdt_arm; T6:A3 with its result). A WDT0 that is already running
 *     is re-TOPped and restarted, never stopped.
 *   PRE_KERNEL_1 99 (after the early console at 60): T6:B through the
 *     console, the marker and what the arming found, T6:C once WDT0 is
 *     armed; if it refused, say why and reset the SoC through
 *     PSCI at once: an image without its watchdog must not run. If PSCI
 *     returns, arm once more with TOP 15 and wait for it; if that refuses
 *     too, the board stays halted until its power is cut.
 *   POST_KERNEL 0: a 1 s timer feeds WDT0 while yd_t6_feed_verdict() says
 *     so: before CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S of uptime, and while the
 *     sample's t6_watchdog_feed() calls come at most
 *     CONFIG_YAKOGL_NANOPC_T6_ALIVE_S apart. After that WDT0 resets the SoC within one period, also when the
 *     image hangs or a fatal error halts it (the timer stops with it).
 *
 * The marker (t6_marker, printed at boot) is what
 * driver/tools/kexec-zephyr/nanopc-t6.sh requires in an image before it
 * loads it: the literal says the watchdog is armed at boot and with which
 * bound.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <zephyr/kernel/mm.h>

#include <yakogl_drivers.h>

#include "board_internal.h"
#include "nanopc_t6.h"

#define WDT0_NODE DT_NODELABEL(wdt0)
#define WDT0_PHYS 0xfeaf0000u /* Linux rk3588-base.dtsi:2463 */
#define CRU_PHYS  0xfd7c0000u /* Linux rk3588-base.dtsi:744 */
#define CRU_MAP   0x1000u     /* CLKGATE_CON(15) is CRU + 0x83c */
#define UART2_NODE DT_NODELABEL(uart2)
#define UART2_PHYS 0xfeb50000u /* board::nanopc_t6_early::UART2_PHYS */

BUILD_ASSERT(DT_NODE_HAS_STATUS(WDT0_NODE, okay), "nanopc_t6.dts names wdt0");
BUILD_ASSERT(DT_REG_ADDR(WDT0_NODE) == WDT0_PHYS, "WDT0 is at 0xfeaf0000");
BUILD_ASSERT(DT_PROP(WDT0_NODE, clock_frequency) == 24000000,
	     "yd_dw_wdt_arm counts the period on xin24m");
BUILD_ASSERT(!IS_ENABLED(CONFIG_WDT_DW), "WDT0 belongs to this file, not to Zephyr's wdt_dw");
BUILD_ASSERT(DT_REG_ADDR(UART2_NODE) == UART2_PHYS && DT_PROP(UART2_NODE, reg_shift) == 2,
	     "the early banner writes uart2's 32-bit registers at 0xfeb50000");
BUILD_ASSERT(DT_SAME_NODE(DT_CHOSEN(zephyr_console), UART2_NODE),
	     "T6:B goes through the console, uart2 like the raw lines");
BUILD_ASSERT(CONFIG_YAKOGL_NANOPC_T6_ALIVE_S < CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S,
	     "the alive window is inside the lifetime");

#ifdef CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF
#define T6_PROOF "1"
#else
#define T6_PROOF "0"
#endif

/* The build-time marker: one line, the same in the image and on the UART. */
static const char t6_marker[] =
	"yakogl-wdt: nanopc_t6 WDT0 armed at boot; top " STRINGIFY(CONFIG_YAKOGL_NANOPC_T6_WDT_TOP)
	"; lifetime_s " STRINGIFY(CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S)
	"; alive_s " STRINGIFY(CONFIG_YAKOGL_NANOPC_T6_ALIVE_S)
	"; proof " T6_PROOF
	"; load " STRINGIFY(CONFIG_SRAM_BASE_ADDRESS);

#define PROOF_BACKSTOP_EXTRA_MS 10000u
/* The second arming after a refusal and a PSCI return: the longest period. */
#define T6_RETRY_TOP 15u

#ifdef CONFIG_YAKOGL_NANOPC_T6_VHE_EXIT
#define T6_EARLY_FLAGS YD_T6_EARLY_VHE_EXIT
#else
#define T6_EARLY_FLAGS 0
#endif

/*
 * The registration of the image's first code (yd_t6_early_entry, in the
 * drivers crate): Zephyr's reset.S calls z_arm64_el2_plat_prep_c (or the EL1
 * one, by the EL it was entered at) from __reset_prep_c before any C code;
 * both are weak there ("Stack is not yet available", "x23, x24 and x25 must
 * be preserved"; zephyr af17c0c8 arch/arm64/core/reset.S). These strong ones
 * pass the flags in x0. check_early_hooks.cmake, run after every link,
 * fails the build unless they won.
 */
__asm__(".pushsection .text.t6_plat_prep_c, \"ax\"\n"
	".global z_arm64_el2_plat_prep_c\n"
	".type z_arm64_el2_plat_prep_c, %function\n"
	"z_arm64_el2_plat_prep_c:\n"
	"	mov x0, #" STRINGIFY(T6_EARLY_FLAGS) "\n"
	"	b yd_t6_early_entry\n"
	".size z_arm64_el2_plat_prep_c, . - z_arm64_el2_plat_prep_c\n"
	".global z_arm64_el1_plat_prep_c\n"
	".type z_arm64_el1_plat_prep_c, %function\n"
	"z_arm64_el1_plat_prep_c:\n"
	"	mov x0, #" STRINGIFY(T6_EARLY_FLAGS) "\n"
	"	b yd_t6_early_entry\n"
	".size z_arm64_el1_plat_prep_c, . - z_arm64_el1_plat_prep_c\n"
	".popsection\n");

/* Weak in Zephyr's arch/arm64/core/reset.c; called at the end of
 * z_arm64_el1_init, MMU off. */
void z_arm64_el1_plat_init(void);
void z_arm64_el1_plat_init(void)
{
	yd_t6_early_el1();
}

static mm_reg_t s_uart;
static mm_reg_t s_wdt;
static mm_reg_t s_cru;
static struct yd_dw_wdt_arm s_arm;
static int s_arm_rc = -1;
static uint64_t s_icc_ctlr_found;
static volatile int64_t s_alive_ms = -1;
static volatile bool s_released;
static volatile uint32_t s_feeds;

void t6_watchdog_feed(void)
{
	s_alive_ms = k_uptime_get();
}

uint32_t t6_watchdog_deadline_ms(void)
{
	return IS_ENABLED(CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF)
		       ? 0
		       : CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S * MSEC_PER_SEC;
}

uint32_t t6_watchdog_period_ms(void)
{
	return s_arm_rc == 0 ? s_arm.period_ms : 0;
}

static FUNC_NORETURN void wait_for_wdt0(void)
{
	(void)arch_irq_lock();
	for (;;) {
		__asm__ volatile("wfi");
	}
}

static FUNC_NORETURN void psci_reset_or_wait(void)
{
	int rc = yd_psci_system_reset();

	printk("t6: PSCI SYSTEM_RESET returned %d: waiting for WDT0 (%u ms period)\n", rc,
	       t6_watchdog_period_ms());
	wait_for_wdt0();
}

/* The arming refused: PSCI first; if the firmware returns, WDT0 is the only
 * way out left, so arm it once more with the longest period. */
static FUNC_NORETURN void refused_reset(void)
{
	int rc = yd_psci_system_reset();

	printk("t6: PSCI SYSTEM_RESET returned %d: arming WDT0 again with TOP %u\n", rc,
	       T6_RETRY_TOP);
	s_arm_rc = yd_dw_wdt_arm(s_wdt, s_cru, T6_RETRY_TOP, &s_arm);
	if (s_arm_rc == 0) {
		printk("t6: WDT0 armed on the second try: CR 0x%08x TORR 0x%08x, period %u ms: "
		       "waiting for it\n",
		       s_arm.cr_now, s_arm.torr_now, s_arm.period_ms);
	} else {
		printk("t6: WDT0 arming FAILED again (rc %d, refusal %u, read back 0x%08x) and PSCI "
		       "returned: halted with no watchdog; only the power switch ends this\n",
		       s_arm_rc, s_arm.refusal, s_arm.cr_now);
	}
	wait_for_wdt0();
}

static int t6_gic_eoi_reset(void)
{
	s_icc_ctlr_found = yd_gic_eoi_mode_reset();
	return 0;
}
SYS_INIT(t6_gic_eoi_reset, EARLY, 0);

static int t6_wdt_arm(void)
{
	device_map(&s_uart, UART2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	yd_t6_banner_a2(s_uart);
	device_map(&s_wdt, WDT0_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&s_cru, CRU_PHYS, CRU_MAP, K_MEM_CACHE_NONE);
	s_arm_rc = yd_dw_wdt_arm(s_wdt, s_cru, CONFIG_YAKOGL_NANOPC_T6_WDT_TOP, &s_arm);
	yd_t6_banner_a3(s_uart, s_arm_rc, s_arm.cr_now, s_arm.period_ms);
	return 0;
}
SYS_INIT(t6_wdt_arm, PRE_KERNEL_1, 0);

static int t6_wdt_report(void)
{
	printk("T6:B console up (uart2 through the ns16550 driver)\n");
	printk("%s\n", t6_marker);
	if (s_icc_ctlr_found != 0) {
		printk("t6: GICv3 CPU interface: ICC_CTLR_EL1 0x%llx had EOImode 1 "
		       "(the kernel before us), now 0\n",
		       (unsigned long long)s_icc_ctlr_found);
	}
	if (s_arm_rc != 0) {
		printk("t6: WDT0 arming FAILED (rc %d, refusal %u): COMP_TYPE 0x%08x PARAM_1 0x%08x, "
		       "CR 0x%08x TORR 0x%08x as found, read back 0x%08x, flags 0x%x: "
		       "resetting the SoC now\n",
		       s_arm_rc, s_arm.refusal, s_arm.comp_type, s_arm.param1, s_arm.cr_found,
		       s_arm.torr_found, s_arm.cr_now, s_arm.flags);
		refused_reset();
	}
	printk("t6: WDT0 armed: CR 0x%08x -> 0x%08x (%s), TORR 0x%08x -> 0x%08x, CCVR 0x%08x, "
	       "PARAM_1 0x%08x, version 0x%08x, flags 0x%x: period %u ms (TOP %u), ",
	       s_arm.cr_found, s_arm.cr_now,
	       (s_arm.flags & YD_DW_WDT_WAS_RUNNING) ? "was running" : "was stopped",
	       s_arm.torr_found, s_arm.torr_now, s_arm.ccvr_now, s_arm.param1, s_arm.version,
	       s_arm.flags, s_arm.period_ms, CONFIG_YAKOGL_NANOPC_T6_WDT_TOP);
	if (IS_ENABLED(CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF)) {
		printk("never fed (proof)\n");
	} else {
		printk("fed until %u s of uptime, alive window %u s\n",
		       CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S, CONFIG_YAKOGL_NANOPC_T6_ALIVE_S);
	}
	printk("T6:C WDT0 armed, period %u ms\n", s_arm.period_ms);
	printk(BOARD_WDOG_ARMED_LINE);
	return 0;
}
SYS_INIT(t6_wdt_report, PRE_KERNEL_1, 99);

#ifdef CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF
static void proof_backstop(struct k_timer *t)
{
	ARG_UNUSED(t);
	printk("t6: proof FAILED: no WDT0 reset %u ms after arming (period %u ms): "
	       "PSCI SYSTEM_RESET\n",
	       s_arm.period_ms + PROOF_BACKSTOP_EXTRA_MS, s_arm.period_ms);
	psci_reset_or_wait();
}
static K_TIMER_DEFINE(s_backstop, proof_backstop, NULL);
#else
static const char *const feed_stop_why[] = {
	[YD_T6_FEED_RELEASED] = "released",
	[YD_T6_FEED_LIFETIME_OVER] = "lifetime over",
	[YD_T6_FEED_NO_PROGRESS] = "no progress",
};

static void feed_tick(struct k_timer *t)
{
	int64_t now = k_uptime_get();
	uint32_t v = yd_t6_feed_verdict(now, s_alive_ms, s_released, t6_watchdog_deadline_ms(),
					CONFIG_YAKOGL_NANOPC_T6_ALIVE_S * MSEC_PER_SEC);

	if (v != YD_T6_FEED_YES) {
		k_timer_stop(t);
		printk("t6: feeding stopped at %lld ms after %u feeds (%s): WDT0 resets the SoC "
		       "within %u ms\n",
		       (long long)now, s_feeds,
		       v < ARRAY_SIZE(feed_stop_why) && feed_stop_why[v] ? feed_stop_why[v] :
									  "unknown verdict",
		       s_arm.period_ms);
		return;
	}
	yd_dw_wdt_feed(s_wdt);
	s_feeds++;
}
static K_TIMER_DEFINE(s_feeder, feed_tick, NULL);
#endif

static int t6_wdt_feed_start(void)
{
#ifdef CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF
	/* Armed at uptime 0 (PRE_KERNEL_1, before the system timer runs). */
	k_timer_start(&s_backstop, K_MSEC(s_arm.period_ms + PROOF_BACKSTOP_EXTRA_MS), K_NO_WAIT);
#else
	k_timer_start(&s_feeder, K_SECONDS(1), K_SECONDS(1));
#endif
	return 0;
}
SYS_INIT(t6_wdt_feed_start, POST_KERNEL, 0);

FUNC_NORETURN void t6_reset_to_linux(const char *why)
{
	s_released = true;
	if (IS_ENABLED(CONFIG_YAKOGL_NANOPC_T6_WDT_PROOF)) {
		printk("t6: %s: proof: waiting for WDT0 (armed at boot, period %u ms, never fed)\n",
		       why, s_arm.period_ms);
		for (;;) {
			k_sleep(K_FOREVER);
		}
	}
	printk("t6: %s: PSCI SYSTEM_RESET at %lld ms (%u feeds)\n", why,
	       (long long)k_uptime_get(), s_feeds);
	psci_reset_or_wait();
}
