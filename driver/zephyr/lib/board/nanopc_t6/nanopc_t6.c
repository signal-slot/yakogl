/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The NanoPC-T6 board glue, the Zephyr side (CONFIG_YAKOGL_NANOPC_T6;
 * driver/docs/nanopc-t6-mali.md, "Z1"). The register logic is driver/rust:
 * ip::dw_wdt and soc::rk3588 behind yd_dw_wdt_arm() / yd_dw_wdt_feed(), and
 * aarch64 behind yd_gic_active_reset() / yd_gic_cpuif_reset() /
 * yd_psci_system_reset(), and the
 * feeder's decision behind yd_t6_feed_verdict() (board::nanopc_t6), and the
 * board's own pins and supply behind yd_t6_usb_vbus_on() / yd_t6_hdmi0_pins()
 * (board::nanopc_t6_pins). This file
 * is registration only: the init hooks, the mappings, a timer, the prints.
 *
 * Order (the T6:<tag> lines are the early banner, board::nanopc_t6_early;
 * a tag missing from the UART says where the image stopped):
 *   reset.S's plat_prep_c hook, the image's first code (no stack yet):
 *     yd_t6_early_entry, at EL2 with the MMU or a cache on (U-Boot's `go`),
 *     turns them off and cleans zephyr,sram to the point of coherency first
 *     (T6:G; board::nanopc_t6_go; after kexec they are off and it does
 *     nothing). Then, MMU off, it prints T6:A with the EL found, points VBAR_EL2
 *     at a reporter (T6:X for any later exception taken to EL2), and with
 *     CONFIG_YAKOGL_NANOPC_T6_VHE_EXIT writes HCR_EL2 = RW only before
 *     Zephyr's z_arm64_el2_init reads it (Linux's VHE kexec leaves E2H set;
 *     driver/rust/src/aarch64.rs).
 *   z_arm64_el1_plat_init, the end of z_arm64_el1_init (EL1, MMU off): T6:A1,
 *     T6:A2, WDT0 armed (yd_t6_early_arm on the physical addresses; T6:A3
 *     with its result). A WDT0 that is already running is re-TOPped and
 *     restarted, never stopped. reset.S calls z_arm64_el1_init (which calls
 *     this hook last) before it returns into z_prep_c, z_prep_c calls
 *     z_cstart, z_cstart runs EARLY to PRE_KERNEL_2 and starts the thread
 *     that runs the rest (zephyr af17c0c8 arch/arm64/core/reset.S:248-257,
 *     reset.c:315, prep_c.c:34-50, kernel/init.c:541-578 and 274-318): so
 *     WDT0 is armed before every init hook of every level, by the code's
 *     order and not by the linker's.
 *     A SYS_INIT hook cannot promise that: priority 0 is the lowest there is
 *     (include/zephyr/init.h:44), and hooks of one level and priority run in
 *     link order (the linker script sorts the sections by name only,
 *     include/zephyr/linker/linker-defs.h:70-73, and they share one name);
 *     with CONFIG_POSIX_API four POSIX hooks at PRE_KERNEL_1 0 ran before
 *     the arming when it was one. check_early_hooks.cmake, run after every
 *     link, fails the build unless this hook is ours and calls the arming,
 *     and unless the code it reaches passes the MMU-off checks (below).
 *     The arming results go to s_arm and s_arm_rc, which are __noinit: the
 *     BSS that z_prep_c zeroes afterwards does not hold them.
 *   The pre-arm window, before WDT0 is armed by this image: reset.S up to
 *   the hook above, z_arm64_el2_init and z_arm64_el1_init, and the T6:A
 *   code: register writes and uart2, no wait on a device, no memory but the
 *   stack. A fault in it halts the CPU for good (T6:X at EL2), and nothing
 *   of the image's own covers that. Linux's dw_wdt does: nanopc-t6.sh --go
 *   (by default; --no-linux-wdt turns it off) opens /dev/watchdog0 in the
 *   jump's own ssh command, so WDT0 is running with Linux's period (44.7 s
 *   from the open)
 *   when the image starts, and the arming re-arms it ("was running").
 *   EARLY: every interrupt left Active at the GIC distributor and at this
 *     CPU's redistributor deactivated (an Active interrupt is not signalled
 *     again; Linux clears them at its start, Zephyr's init does not), then
 *     the GICv3 CPU interface back to EOImode 0 (Linux left it in split
 *     mode; with it, the system timer would tick once), its active priorities
 *     (ICC_AP1Rn_EL1) and ICC_BPR1_EL1 to 0 (an interrupt the crashing CPU
 *     had not ended would keep the running priority raised, and the timer
 *     would not tick at all). Before Zephyr's GICv3 init at PRE_KERNEL_1,
 *     which writes none of them (driver/rust/src/aarch64.rs). The MMU is on
 *     here (z_prep_c ran z_arm64_mm_init before z_cstart) and the GIC node's
 *     reg banks are mapped flat as device memory by arch/arm64/core/mmu.c, as
 *     Zephyr's own GICv3 driver needs: the devicetree's physical addresses
 *     are used as they are. Interrupts stay masked until the first thread
 *     runs (reset.S masks DAIF; only SError is unmasked before z_cstart).
 *   PRE_KERNEL_1 99 (after the early console at 60): map WDT0 and the CRU
 *     for the feeder; T6:B through the console, the marker and what the
 *     arming found, T6:C once WDT0 is armed; if it refused, say why and
 *     reset the SoC through PSCI at once: an image without its watchdog must
 *     not run. If PSCI returns, arm once more with TOP 15 and wait for it; if
 *     that refuses too, the board stays halted until its power is cut.
 *   PRE_KERNEL_2 0: what the board's start sets up itself, so that every
 *     route (U-Boot's `go`, kexec, the adopt of Linux's picture) ends in the
 *     state Linux leaves (board::nanopc_t6_pins; driver/docs/
 *     nanopc-t6-display.md, "What the start sets up itself"): the USB
 *     Type-A ports' 5 V switch on (yd_t6_usb_vbus_on, first: a sink powered
 *     from those ports takes seconds to come up), then the HDMI0 pins'
 *     functions (yd_t6_hdmi0_pins: without them the controller sees no
 *     hot-plug). After Linux the same words are there and nothing changes.
 *     One line with every word as found and as left. Not a Kconfig choice:
 *     Linux leaves both on, and an image that left them off after a reset
 *     would differ by its route.
 *   POST_KERNEL 0: a 1 s timer feeds WDT0 while yd_t6_feed_verdict() says
 *     so: before CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S of uptime, and while the
 *     sample's t6_watchdog_feed() calls come at most
 *     CONFIG_YAKOGL_NANOPC_T6_ALIVE_S apart. After that WDT0 resets the SoC
 *     within one period, also when the image hangs or a fatal error halts
 *     it (the timer stops with it). In an image with the board layer
 *     (CONFIG_YAKOGL_BOARD) t6_watchdog_feed() is board_watchdog_feed(), and
 *     the timer asks the layer's board_wdog_alive() as every board's feeder
 *     does: one "still making progress" for the sample and the OSAL's waits.
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
#include <zephyr/linker/section_tags.h>

#include <yakogl_drivers.h>

#include "board_internal.h"
#include "nanopc_t6.h"
#ifdef CONFIG_YAKOGL_BOARD
#include "yakogl_board.h"
#endif

#define WDT0_NODE DT_NODELABEL(wdt0)
#define WDT0_PHYS 0xfeaf0000u /* Linux rk3588-base.dtsi:2463 */
#define CRU_PHYS  0xfd7c0000u /* Linux rk3588-base.dtsi:744 */
#define CRU_MAP   0x1000u     /* CLKGATE_CON(15) is CRU + 0x83c */
#define IOC_PHYS   0xfd5f0000u /* Linux rk3588-base.dtsi:729-731 */
#define IOC_MAP    0x10000u
#define GPIO4_PHYS 0xfec50000u /* Linux rk3588-base.dtsi:3308-3310 */
#define GPIO4_MAP  0x1000u
#define UART2_NODE DT_NODELABEL(uart2)
#define UART2_PHYS 0xfeb50000u /* board::nanopc_t6_early::UART2_PHYS */
/* The GICv3 (Zephyr's rk3588.dtsi): reg 0 the distributor, reg 1 the one
 * redistributor region. */
#define GIC_NODE DT_NODELABEL(gic)

BUILD_ASSERT(DT_NODE_HAS_STATUS(WDT0_NODE, okay), "nanopc_t6.dts names wdt0");
BUILD_ASSERT(DT_REG_ADDR(WDT0_NODE) == WDT0_PHYS, "WDT0 is at 0xfeaf0000");
BUILD_ASSERT(DT_PROP(WDT0_NODE, clock_frequency) == 24000000,
	     "yd_dw_wdt_arm counts the period on xin24m");
BUILD_ASSERT(!IS_ENABLED(CONFIG_WDT_DW), "WDT0 belongs to this file, not to Zephyr's wdt_dw");
BUILD_ASSERT(DT_REG_ADDR(UART2_NODE) == UART2_PHYS && DT_PROP(UART2_NODE, reg_shift) == 2,
	     "the early banner writes uart2's 32-bit registers at 0xfeb50000");
BUILD_ASSERT(DT_NODE_HAS_COMPAT(GIC_NODE, arm_gic_v3) && DT_NUM_REGS(GIC_NODE) == 2 &&
		     DT_PROP_OR(GIC_NODE, redistributor_regions, 1) == 1,
	     "yd_gic_active_reset is given the distributor and one redistributor region");
BUILD_ASSERT(DT_SAME_NODE(DT_CHOSEN(zephyr_console), UART2_NODE),
	     "T6:B goes through the console, uart2 like the raw lines");
BUILD_ASSERT(CONFIG_YAKOGL_NANOPC_T6_ALIVE_S < CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S,
	     "the alive window is inside the lifetime");
#ifdef CONFIG_YAKOGL_BOARD
BUILD_ASSERT(CONFIG_YAKOGL_NANOPC_T6_LIFETIME_S == CONFIG_YAKOGL_BOARD_LIFETIME_S &&
		     CONFIG_YAKOGL_NANOPC_T6_ALIVE_S == CONFIG_YAKOGL_BOARD_ALIVE_S,
	     "a board-layer image sets the watchdog with CONFIG_YAKOGL_BOARD_LIFETIME_S and _ALIVE_S");
#endif
BUILD_ASSERT(CONFIG_MP_MAX_NUM_CPUS == 1,
	     "z_arm64_el1_plat_init runs on every CPU that boots; WDT0 is armed there once");
/* k_mem_map's (and so device_map's) free virtual range runs from the
 * image's end to KERNEL_VM_BASE + KERNEL_VM_SIZE; the image may fill
 * zephyr,sram, so the virtual space must reach past it, or the mappings land
 * inside the image (nanopc_t6_defconfig). */
BUILD_ASSERT(CONFIG_KERNEL_VM_BASE == CONFIG_SRAM_BASE_ADDRESS &&
		     (uint64_t)CONFIG_SRAM_SIZE * 1024u < (uint64_t)CONFIG_KERNEL_VM_SIZE,
	     "CONFIG_KERNEL_VM_SIZE must cover zephyr,sram and leave room for device_map");

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

static mm_reg_t s_wdt;
static mm_reg_t s_cru;
/* Written by the arming below with the MMU off, before z_prep_c zeroes the
 * BSS: so not in the BSS. */
static __noinit struct yd_dw_wdt_arm s_arm;
static __noinit int s_arm_rc;

/*
 * The arming (nanopc_t6.c, "Order"). Weak in Zephyr's arch/arm64/core/
 * reset.c; called at the end of z_arm64_el1_init, MMU off: every access is
 * to Device memory, where the dc zva of a memset or an unaligned access
 * faults. yd_t6_early_arm writes its record one field at a time, and
 * check_early_hooks.cmake fails the build if anything this function reaches
 * calls memset, memcpy or memmove, branches through a register (br, blr and
 * their pointer-authentication forms), or loads or stores at an immediate
 * offset that is not a multiple of the access size. It cannot check that a
 * base register holds an aligned address: that rests on the code (the record
 * is a 4-byte aligned struct, the stack 16-byte aligned by reset.S).
 */
void z_arm64_el1_plat_init(void);
void z_arm64_el1_plat_init(void)
{
	s_arm_rc = yd_t6_early_arm(WDT0_PHYS, CRU_PHYS, CONFIG_YAKOGL_NANOPC_T6_WDT_TOP, &s_arm);
}
static struct yd_gic_cpuif_reset s_gic;
static struct yd_gic_active_reset s_gic_active;
static volatile int64_t s_alive_ms = -1;
static volatile bool s_released;
static volatile uint32_t s_feeds;

void t6_watchdog_feed(void)
{
#ifdef CONFIG_YAKOGL_BOARD
	board_watchdog_feed();
#else
	s_alive_ms = k_uptime_get();
#endif
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

/* The name is older than the AP1Rn, BPR1 and Active clears; tools/check.sh
 * looks for it in the image. Linux's order: the distributor's and the
 * redistributor's Active state (gic_dist_init, gic_cpu_config), then the CPU
 * interface (gic_cpu_sys_reg_init; driver/rust/src/aarch64.rs). */
static int t6_gic_eoi_reset(void)
{
	yd_gic_active_reset(DT_REG_ADDR_BY_IDX(GIC_NODE, 0), DT_REG_ADDR_BY_IDX(GIC_NODE, 1),
			    DT_REG_SIZE_BY_IDX(GIC_NODE, 1),
			    DT_PROP_OR(GIC_NODE, redistributor_stride, 0), &s_gic_active);
	yd_gic_cpuif_reset(&s_gic);
	return 0;
}
SYS_INIT(t6_gic_eoi_reset, EARLY, 0);

/* Always printed, one line: a record says which interrupts the kernel before
 * us left Active, and that none is left after the clear. */
static void gic_active_report(void)
{
	const struct yd_gic_active_reset *a = &s_gic_active;

	if (a->flags & YD_GIC_RDIST_FOUND) {
		printk("t6: GIC active state: GICD_TYPER 0x%08x GICR_TYPER 0x%016llx "
		       "GICR_ISACTIVER0 0x%08x as found, ",
		       a->gicd_typer, (unsigned long long)a->gicr_typer, a->isactiver0);
	} else {
		printk("t6: GIC active state: GICD_TYPER 0x%08x, no redistributor with affinity "
		       "0x%08x in %u frame(s): SGIs and PPIs not scanned, ",
		       a->gicd_typer, a->aff, a->frames);
	}
	printk("%u INTID(s) active (", a->active);
	if (a->active == 0) {
		printk("none");
	} else {
		printk("INTIDs");
		for (uint32_t i = 0; i < MIN(a->active, (uint32_t)YD_GIC_ACTIVE_LISTED); i++) {
			printk(" %u", a->intids[i]);
		}
		if (a->active > YD_GIC_ACTIVE_LISTED) {
			printk(" ...");
		}
	}
	printk("), %u word(s) written ICACTIVER, %u still active\n", a->words_cleared,
	       a->still_active);
}

static int t6_wdt_report(void)
{
	/* For the feeder and the second arming; WDT0 already runs. */
	device_map(&s_wdt, WDT0_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&s_cru, CRU_PHYS, CRU_MAP, K_MEM_CACHE_NONE);
	printk("T6:B console up (uart2 through the ns16550 driver)\n");
	printk("%s\n", t6_marker);
	if (s_gic.flags & YD_GIC_EOIMODE_CLEARED) {
		printk("t6: GICv3 CPU interface: ICC_CTLR_EL1 0x%llx had EOImode 1 "
		       "(the kernel before us), now 0\n",
		       (unsigned long long)s_gic.ctlr_found);
	}
	/* Always printed: a record says whether the kernel before us left an
	 * active priority (a bit set in ICC_AP1R0_EL1). */
	if (s_gic.flags & YD_GIC_CPUIF_RESET) {
		printk("t6: GICv3 CPU interface: ICC_AP1R0_EL1 0x%08llx ICC_BPR1_EL1 0x%llx as found, "
		       "%u ICC_AP1Rn_EL1 and ICC_BPR1_EL1 now 0\n",
		       (unsigned long long)s_gic.ap1r0_found, (unsigned long long)s_gic.bpr1_found,
		       s_gic.ap1r_cleared);
	} else {
		printk("t6: GICv3 CPU interface: ICC_SRE_EL1 0x%llx, SRE clear: not touched\n",
		       (unsigned long long)s_gic.sre);
	}
	gic_active_report();
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

/* Each word a pins call wrote: its address, as found -> as left. */
static void pins_words(const struct yd_t6_pins *p)
{
	for (uint32_t i = 0; i < MIN(p->count, (uint32_t)YD_T6_PINS_WORDS); i++) {
		printk(" 0x%08x 0x%08x -> 0x%08x%s", p->phys[i], p->found[i], p->now[i],
		       (p->differ & BIT(i)) ? " (differs)" : "");
	}
}

/* The board's own pins and supply ("Order" above). The mappings stay: the
 * IOC and GPIO4 are mapped once per boot. */
static int t6_board_pins(void)
{
	struct yd_t6_pins vbus, hdmi;
	mm_reg_t ioc, gpio4;
	int vrc, hrc;

	device_map(&ioc, IOC_PHYS, IOC_MAP, K_MEM_CACHE_NONE);
	device_map(&gpio4, GPIO4_PHYS, GPIO4_MAP, K_MEM_CACHE_NONE);
	vrc = yd_t6_usb_vbus_on(ioc, gpio4, &vbus);
	hrc = yd_t6_hdmi0_pins(ioc, &hdmi);
	printk("t6: vbus5v0_usb rc %d, GPIO4_B0 level %u:", vrc, vbus.level);
	pins_words(&vbus);
	printk("; HDMI0 pins rc %d:", hrc);
	pins_words(&hdmi);
	printk("\n");
	return 0;
}
SYS_INIT(t6_board_pins, PRE_KERNEL_2, 0);

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

	/* With the board layer the alive time is the layer's (s_alive_ms stays
	 * -1 then); without it this is always true. */
	if (v == YD_T6_FEED_YES && !board_wdog_alive()) {
		v = YD_T6_FEED_NO_PROGRESS;
	}
	if (v != YD_T6_FEED_YES) {
		k_timer_stop(t);
		printk("t6: feeding stopped at %lld ms after %u feeds (%s): WDT0 resets the SoC "
		       "within %u ms\n",
		       (long long)now, s_feeds,
		       v < ARRAY_SIZE(feed_stop_why) && feed_stop_why[v] ? feed_stop_why[v] :
									  "unknown verdict",
		       s_arm.period_ms);
		/* after a reset call the record was shown and held already */
		if (v != YD_T6_FEED_RELEASED) {
			t6_record_screen();
		}
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
	/* Armed before uptime 0 (before z_prep_c, so before the system timer
	 * runs): the backstop fires after WDT0 would have. */
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
