/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The STM32MP257F-DK's Cortex-A35 board layer, its core
 * (CONFIG_YAKOGL_STM32MP2; docs/notes/pre-study-stm32mp257f-dk.md, sections
 * 4 and 8): the early banner, the hand-over record and the end of a run
 * through PSCI SYSTEM_RESET, with a bounded lifetime. The board's firmware
 * as read under ST's Linux (#1281): TF-A v2.10 (BL31 answers PSCI 1.1 over
 * SMC), OP-TEE 4.0, ST's U-Boot 2023.10 starting Linux with booti through
 * extlinux; a GIC-400 at 0x4ac00000; System RAM 0x84000000-0xfa7fffff.
 *
 * Order (the MP2:<tag> lines are the early banner; a tag missing from the
 * console says where the image stopped):
 *   __wrap___start, the image's first code (the image header's branch,
 *     linked with --wrap=__start; caches and MMU as the previous stage left
 *     them, no stack): stm32mp2_entry.S fills mp2_handover and prints MP2:A
 *     with the EL found.
 *   z_arm64_el1_plat_init, the end of z_arm64_el1_init (MMU off): MP2:A1,
 *     the EL2-to-EL1 step is done.
 *   PRE_KERNEL_1 0: the watchdog's arm point (mp2_wdog_arm): this hook
 *     only takes the system counter. The OP-TEE watchdog (arm,smc-wdt) is
 *     stm32mp2_wdog.c's, with its own hooks at the same three points, in
 *     an image that has that file (CONFIG_YAKOGL_STM32MP2_WDOG); it calls
 *     mp2_note_wdog_armed() for the report.
 *   PRE_KERNEL_1 99 (after the console at 60): MP2:B, the hand-over record
 *     line, PSCI_VERSION, the lifetime (mp2_wdog_report: the watchdog's
 *     report point).
 *   POST_KERNEL 0: the lifetime timer starts (mp2_wdog_feed_start: the
 *     watchdog's feeder point). At CONFIG_YAKOGL_STM32MP2_LIFETIME_S of
 *     uptime it resets the board through PSCI, whatever the sample does.
 *   A fatal error (k_sys_fatal_error_handler): printed, then PSCI reset.
 *   mp2_reset_to_linux(): the end of a run, PSCI SYSTEM_RESET.
 * What this core does not cover: a hang before the system timer runs or
 * with interrupts locked, and a PSCI that returns. Only a watchdog covers
 * those; the board-read study found the OP-TEE watchdog (32 s) already
 * running when U-Boot hands over, which the watchdog item is to confirm.
 *
 * The hand-over record line (one line, key=value pairs after the tag):
 *   mp2-handover: v=1 el=2 sctlr=0x... m=0 c=0 i=1 x0=0x... x1=0x... x2=0x...
 *     x3=0x... pc=0xa0000000 cntfrq=... mpidr=0x80000000 psci=0x10001
 * sctlr is SCTLR_EL2 (el=2) or SCTLR_EL1 as found; m, c and i are its M, C
 * and I bits. x0-x3 are the entry values, printed
 * and never asserted. At U-Boot's `go` (ST r3.1) the image starts at EL2
 * with SCTLR M/C/I = 0, x0 = 0 and x1 non-null; x2 and x3 are
 * indeterminate. TF-A r3.1 BL31 leaves SPI 183 in group 1, non-secure
 * (plat/st/stm32mp2/bl31_plat_setup.c:151, stm32mp_gic_init, GICv2 driver). pc is where the entry code ran (the load
 * address, if the image was not moved). cntfrq is decimal Hz, the rest
 * hex. Without the entry code's record the line is
 * "mp2-handover: v=1 missing".
 */
#include <stddef.h>
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/devicetree.h>
#include <zephyr/fatal.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>

#include "stm32mp2.h"

#define USART2_NODE DT_NODELABEL(usart2)
#define USART2_PHYS 0x400e0000u /* stm32mp2_entry.S MP2_USART2_PHYS */
#define USART_ISR   0x1cu
#define USART_TDR   0x28u
#define USART_TXE   BIT(7)
#define TXE_POLL_LIMIT 0x100000u

BUILD_ASSERT(DT_REG_ADDR(USART2_NODE) == USART2_PHYS,
	     "the early banner writes USART2 at 0x400e0000");
BUILD_ASSERT(DT_SAME_NODE(DT_CHOSEN(zephyr_console), USART2_NODE),
	     "MP2:B goes through the console, USART2 like the raw lines");

/* stm32mp2_entry.S's HO_* offsets. */
BUILD_ASSERT(offsetof(struct mp2_handover, seen) == 0 &&
	     offsetof(struct mp2_handover, x) == 8 &&
	     offsetof(struct mp2_handover, el) == 40 &&
	     offsetof(struct mp2_handover, sctlr) == 48 &&
	     offsetof(struct mp2_handover, mpidr) == 56 &&
	     offsetof(struct mp2_handover, cntfrq) == 64 &&
	     offsetof(struct mp2_handover, pc) == 72 && sizeof(struct mp2_handover) == 80,
	     "struct mp2_handover is the layout stm32mp2_entry.S stores");

/* SMCCC function ids (PSCI 1.1, DEN0022): 32-bit calls. */
#define PSCI_VERSION      0x84000000u
#define PSCI_SYSTEM_RESET 0x84000009u

#define SCTLR_M BIT(0)
#define SCTLR_C BIT(2)
#define SCTLR_I BIT(12)

/*
 * Written by the image's first code. A non-zero initial value keeps it in
 * .data: z_prep_c clears .bss after that code has run, and the image's own
 * load gives every boot a fresh, unfilled record (a .noinit one could be a
 * stale one from an earlier image). check_early_hooks.cmake fails the build
 * if it lands anywhere else. Two 64-byte lines, cleaned to the point of
 * coherency by the entry code.
 */
struct mp2_handover mp2_handover __aligned(64) = {
	.seen = MP2_HANDOVER_EMPTY,
};

static uint64_t s_cntpct_pre_kernel;
static int32_t s_psci_version = -1;
static uint32_t s_wdog_armed_s;

void mp2_note_wdog_armed(uint32_t timeout_s)
{
	s_wdog_armed_s = timeout_s;
}

const struct mp2_handover *mp2_handover_get(void)
{
	return &mp2_handover;
}

uint32_t mp2_lifetime_ms(void)
{
	return CONFIG_YAKOGL_STM32MP2_LIFETIME_S * MSEC_PER_SEC;
}

int32_t mp2_psci_version(void)
{
	return s_psci_version;
}

/* An SMCCC call to BL31: x0-x17 may change (SMCCC 1.1 and later keep
 * x4-x17, older ones do not promise it). */
static int64_t mp2_smc(uint64_t fn, uint64_t a1, uint64_t a2, uint64_t a3)
{
	register uint64_t x0 __asm__("x0") = fn;
	register uint64_t x1 __asm__("x1") = a1;
	register uint64_t x2 __asm__("x2") = a2;
	register uint64_t x3 __asm__("x3") = a3;

	__asm__ volatile("smc #0"
			 : "+r"(x0), "+r"(x1), "+r"(x2), "+r"(x3)
			 :
			 : "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14",
			   "x15", "x16", "x17", "memory");
	return (int64_t)x0;
}

static FUNC_NORETURN void halt(void)
{
	(void)arch_irq_lock();
	for (;;) {
		__asm__ volatile("wfi");
	}
}

FUNC_NORETURN void mp2_reset_to_linux(const char *why)
{
	int32_t rc;

	printk("mp2: %s: PSCI SYSTEM_RESET at %lld ms\n", why, (long long)k_uptime_get());
	rc = (int32_t)mp2_smc(PSCI_SYSTEM_RESET, 0, 0, 0);
	printk("mp2: PSCI SYSTEM_RESET returned %d: halted\n", rc);
	halt();
}

/* Weak in Zephyr's kernel/fatal.c (which halts): give the board back. */
void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
	ARG_UNUSED(esf);
	printk("mp2: fatal error %u\n", reason);
	mp2_reset_to_linux("fatal error");
}

/* Weak in Zephyr's arch/arm64/core/reset.c; called at the end of
 * z_arm64_el1_init at EL1, MMU off (device accesses), stack set. */
void z_arm64_el1_plat_init(void);
void z_arm64_el1_plat_init(void)
{
	static const char a1[] = "MP2:A1\r\n";
	volatile uint32_t *isr = (volatile uint32_t *)(uintptr_t)(USART2_PHYS + USART_ISR);
	volatile uint32_t *tdr = (volatile uint32_t *)(uintptr_t)(USART2_PHYS + USART_TDR);

	for (size_t i = 0; i < sizeof(a1) - 1; i++) {
		for (uint32_t n = 0; n < TXE_POLL_LIMIT && !(*isr & USART_TXE); n++) {
		}
		*tdr = (uint8_t)a1[i];
	}
}

/* The watchdog's arm point (the first PRE_KERNEL_1 hook): the OP-TEE
 * watchdog is armed here once the layer has it. */
static int mp2_wdog_arm(void)
{
	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(s_cntpct_pre_kernel));
	return 0;
}
SYS_INIT(mp2_wdog_arm, PRE_KERNEL_1, 0);

static void print_handover(void)
{
	const struct mp2_handover *h = &mp2_handover;

	if (h->seen != MP2_HANDOVER_SEEN) {
		printk("mp2-handover: v=1 missing\n");
		return;
	}
	/* One printk: the line stays whole on the console. */
	printk("mp2-handover: v=1 el=%u sctlr=0x%llx m=%u c=%u i=%u x0=0x%llx x1=0x%llx "
	       "x2=0x%llx x3=0x%llx pc=0x%llx cntfrq=%llu mpidr=0x%llx psci=0x%x\n",
	       (unsigned int)((h->el >> 2) & 3), (unsigned long long)h->sctlr,
	       (h->sctlr & SCTLR_M) ? 1u : 0u, (h->sctlr & SCTLR_C) ? 1u : 0u,
	       (h->sctlr & SCTLR_I) ? 1u : 0u, (unsigned long long)h->x[0],
	       (unsigned long long)h->x[1], (unsigned long long)h->x[2],
	       (unsigned long long)h->x[3], (unsigned long long)h->pc,
	       (unsigned long long)h->cntfrq, (unsigned long long)h->mpidr,
	       (unsigned int)s_psci_version);
}

/* The watchdog's report point (after the console). */
static int mp2_wdog_report(void)
{
	s_psci_version = (int32_t)mp2_smc(PSCI_VERSION, 0, 0, 0);
	printk("MP2:B console up (USART2, polled)\n");
	print_handover();
	printk("mp2: system counter at PRE_KERNEL_1: %llu ticks\n",
	       (unsigned long long)s_cntpct_pre_kernel);
	if (s_wdog_armed_s != 0) {
		printk("mp2: OP-TEE watchdog armed by this image (%u s); PSCI SYSTEM_RESET at "
		       "%u s of uptime at the latest\n",
		       s_wdog_armed_s, CONFIG_YAKOGL_STM32MP2_LIFETIME_S);
	} else {
		printk("mp2: no watchdog armed by this image; PSCI SYSTEM_RESET at %u s of "
		       "uptime at the latest\n",
		       CONFIG_YAKOGL_STM32MP2_LIFETIME_S);
	}
	return 0;
}
SYS_INIT(mp2_wdog_report, PRE_KERNEL_1, 99);

static void lifetime_over(struct k_timer *t)
{
	ARG_UNUSED(t);
	mp2_reset_to_linux("lifetime over");
}
static K_TIMER_DEFINE(s_lifetime, lifetime_over, NULL);

/* The watchdog's feeder point (the first POST_KERNEL hook): for now the
 * lifetime only. Armed in uptime from boot (the system timer starts at 0). */
static int mp2_wdog_feed_start(void)
{
	int64_t left = (int64_t)mp2_lifetime_ms() - k_uptime_get();

	k_timer_start(&s_lifetime, K_MSEC(MAX(left, 0)), K_NO_WAIT);
	return 0;
}
SYS_INIT(mp2_wdog_feed_start, POST_KERNEL, 0);
