/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_boot_probe - how far does a kexec'd image get on a board with no
 * console? Reset the SoC the moment a chosen stage is reached; the time
 * until Linux is back says whether the stage was reached at all.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/reboot.h>
#include "gc_sample.h"

/* PSCI SYSTEM_RESET straight through SMC: usable before any driver exists.
 * SMCCC function id 0x84000009 (PSCI 0.2, ARM DEN 0022). */
static void reset_now(void)
{
	register unsigned long x0 __asm__("x0") = 0x84000009UL;

	__asm__ volatile("smc #0" : "+r"(x0) : : "x1", "x2", "x3", "memory");
	for (;;) {
	}
}

/*
 * A fatal error is reported the only way this board can: as a delay before
 * the reset, on top of the ~3 s an immediate reset takes to bring Linux back.
 *   FATAL_CODE=0: delay = 40 * reason + 5 * kind, kind 1 data abort,
 *                 2 instruction abort, 3 SError, 0 anything else
 *   FATAL_CODE=1: delay = 2 * ESR_EL1.EC (0..126)
 *   FATAL_CODE=2: delay = the active GIC interrupt id (see the handler)
 *   FATAL_CODE=3: delay = 75 + FAR_EL1 bits 23:18 (I2C2 40, GPIO4 8, IOMUXC 12, CCM/GPC 14)
 * The watchdog's 128 s means no fatal at all.
 */
#include <zephyr/arch/arm64/exception.h>
#include <zephyr/fatal.h>

void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
	/* ESR/FAR are read first: Zephyr's own fatal print has run already, and
	 * on a build with FPU sharing it overwrote ESR with an FPU trap; build
	 * the probe with CONFIG_FPU_SHARING=n to read the real values. */
	uint64_t esr = read_sysreg(ESR_EL1);
	uint64_t far = read_sysreg(FAR_EL1);
	unsigned int ec = (esr >> 26) & 0x3f;
	unsigned int delay;

	switch (CONFIG_GC_BOOT_PROBE_FATAL_CODE) {
	case 1:  /* exception class: 3 s per unit (data abort 0x25 -> 111 s) */
		delay = 3u * ec;
		break;
	case 3:  /* FAR bits 25:20: I2C2 0x30a..... -> 30 s, GPIO 0x302..... -> 14 s,
		  * IOMUXC/CCM/GPC 0x303..... -> 16 s, DDR 0x6.. -> 10 + 2 * (0..63) */
		delay = 10u + 2u * (unsigned int)((far >> 20) & 0x3f);
		break;
	case 4:  /* ELR: 8 KiB steps from the image base, 10 + index */
		delay = 10u + (unsigned int)(((esf->elr - 0x60000000ULL) >> 13) & 0x7f);
		break;
	default: /* reason: 40 s per unit */
		delay = 40u * reason;
		break;
	}
	for (unsigned int s = 0; s < delay; s++) {
		gc_sample_service_wdog();
		k_busy_wait(1000000);
	}
	reset_now();
}

#if CONFIG_GC_BOOT_PROBE_STAGE == 0
void soc_early_init_hook(void)
{
	reset_now();
}
#else
void soc_early_init_hook(void)
{
}
#if CONFIG_GC_BOOT_PROBE_STAGE == 1
static int stage1(void) { reset_now(); return 0; }
SYS_INIT(stage1, PRE_KERNEL_1, 0);
#elif CONFIG_GC_BOOT_PROBE_STAGE == 2
static int stage2(void) { reset_now(); return 0; }
SYS_INIT(stage2, POST_KERNEL, 0);
#elif CONFIG_GC_BOOT_PROBE_STAGE == 5
static int stage5(void) { reset_now(); return 0; }
SYS_INIT(stage5, POST_KERNEL, 45);   /* right after the GPIO drivers (40) */
#elif CONFIG_GC_BOOT_PROBE_STAGE == 6
static int stage6(void) { reset_now(); return 0; }
SYS_INIT(stage6, POST_KERNEL, 99);   /* after every POST_KERNEL driver */
#endif
#endif

int main(void)
{
#ifdef CONFIG_GC_BOOT_PROBE_WDOG
	/* The samples' watchdog service: device_map of WDOG1 and the two
	 * WSR writes. A hang here shows as the watchdog's two minutes. */
	gc_sample_service_wdog();
#endif
#if CONFIG_GC_BOOT_PROBE_STAGE == 4
	k_sleep(K_SECONDS(5));
#endif
#if CONFIG_GC_BOOT_PROBE_SLEEPS > 0
	/* Several short sleeps: each needs its own timer interrupt. */
	for (int i = 0; i < CONFIG_GC_BOOT_PROBE_SLEEPS; i++) {
		k_sleep(K_SECONDS(1));
	}
#endif
#if CONFIG_GC_BOOT_PROBE_FEED_SECONDS > 0
	/* Does the service actually hold the watchdog off? Feed it every
	 * second for longer than its 128 s and then reset: Linux back at
	 * FEED_SECONDS + boot means yes, at ~133 s means no. */
	for (int i = 0; i < CONFIG_GC_BOOT_PROBE_FEED_SECONDS; i++) {
		gc_sample_service_wdog();
		k_sleep(K_SECONDS(1));
	}
#endif
#ifdef CONFIG_GC_BOOT_PROBE_PRINTK
	printk("gc_boot_probe: printk with the panel console compiled in\n");
#endif
#ifdef CONFIG_GC_BOOT_PROBE_SYS_REBOOT
	sys_reboot(SYS_REBOOT_COLD);   /* the samples' way back: the PSCI driver */
#endif
	reset_now();
	return 0;
}
