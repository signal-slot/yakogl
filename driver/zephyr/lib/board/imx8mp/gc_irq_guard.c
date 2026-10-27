/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_irq_guard - an interrupt nobody registered must not end the run.
 *
 * On the i.MX8MP an image booted with kexec out of Linux takes, at a random
 * moment in its first minute, an interrupt that no driver in the image
 * connected (measured 2026-09-20 through the reset timing: reason
 * K_ERR_SPURIOUS_IRQ, the line still active in the GIC). Zephyr's answer to
 * that is a fatal error and a halt, which on this board means two minutes
 * of silence and a watchdog reset. This guard connects every line that is
 * still on the spurious handler, so that such an interrupt is counted,
 * disabled and reported on the console instead.
 *
 * It runs BEFORE any driver, not after -- PRE_KERNEL_1 at priority 50, just
 * after the GIC. The reason is at the bottom of this file, next to the
 * SYS_INIT that says so. This sentence used to read "after all drivers
 * initialised", which is the opposite of what the code does and was copied
 * out of here into gc_common's README before anyone checked it.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/irq.h>
#include <zephyr/sw_isr_table.h>
#include <zephyr/sys/printk.h>
#include <zephyr/dt-bindings/interrupt-controller/arm-gic.h>
#include "gc_irq_guard.h"

extern void z_irq_spurious(const void *unused);

static struct gc_irq_guard_hit hits[GC_IRQ_GUARD_MAX_HITS];
static unsigned int nhits;

static void guard_isr(const void *arg)
{
	unsigned int irq = (unsigned int)(uintptr_t)arg;

	irq_disable(irq);
	if (nhits < GC_IRQ_GUARD_MAX_HITS) {
		hits[nhits].irq = irq;
		hits[nhits].uptime_ms = k_uptime_get_32();
		nhits++;
	}
	/* No printk here: on arm64 with FPU sharing, printk's argument
	 * spilling needs the FPU, and the FPU is off in interrupt context
	 * (arch/arm64/core/fpu.c). gc_irq_guard_report() prints the hits. */
}

void gc_irq_guard_report(void)
{
	for (unsigned int i = 0; i < nhits; i++) {
		printk("irq guard: unexpected interrupt %u at %u ms, disabled\n",
		       hits[i].irq, hits[i].uptime_ms);
	}
}

unsigned int gc_irq_guard_hits(const struct gc_irq_guard_hit **out)
{
	*out = hits;
	return nhits;
}

static int irq_guard_init(void)
{
	unsigned int guarded = 0;

	for (unsigned int irq = 0; irq < CONFIG_NUM_IRQS; irq++) {
		if (_sw_isr_table[irq].isr != z_irq_spurious) {
			continue;   /* a driver owns it */
		}
		if (irq_connect_dynamic(irq, IRQ_DEFAULT_PRIORITY, guard_isr,
					(const void *)(uintptr_t)irq, 0) >= 0) {
			guarded++;
		}
	}
	printk("irq guard: %u lines covered\n", guarded);
	return 0;
}

/* The static ISR table is complete at link time, so this can run before any
 * driver: interrupts are enabled when the first thread starts, and a stray
 * interrupt during POST_KERNEL driver init was fatal too (seen once on
 * gc_gl_triangle, a fatal within its first second). It must still run after
 * the GIC driver (PRE_KERNEL_1, CONFIG_INTC_INIT_PRIORITY): connecting a
 * line sets its priority, and for an SGI or PPI (INTID < 32) that goes
 * through this CPU's redistributor, whose address the GIC driver records
 * at init. Before that the address is 0 and the write is a translation
 * fault at 0x10400 (measured 2026-09-21: every image faulted in
 * arm_gic_irq_set_priority before main, whatever else it carried). */
BUILD_ASSERT(CONFIG_INTC_INIT_PRIORITY < 50, "the guard must run after the GIC init");
SYS_INIT(irq_guard_init, PRE_KERNEL_1, 50);
