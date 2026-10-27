/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#ifndef GC_IRQ_GUARD_H
#define GC_IRQ_GUARD_H

#include <stdint.h>

#define GC_IRQ_GUARD_MAX_HITS 8

struct gc_irq_guard_hit {
	unsigned int irq;        /* Zephyr irq number: the GIC INTID */
	uint32_t uptime_ms;
};

/* The unexpected interrupts seen so far, oldest first. */
unsigned int gc_irq_guard_hits(const struct gc_irq_guard_hit **out);

/* Print the hits so far (from thread context). */
void gc_irq_guard_report(void);

#endif /* GC_IRQ_GUARD_H */
