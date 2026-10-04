/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Shared shape of the i.MX8MP GPU bring-up samples (gc_identify, gc_fence,
 * gc_clear, gc_triangle).
 *
 * Every one of them runs from a kexec out of Torizon, so Linux is gone and
 * nothing can reset the board remotely. The lab rule is therefore: service
 * the watchdog Linux left armed only for a bounded time, then reset the SoC
 * through PSCI so the boot loader brings Linux back. Feeding the watchdog
 * forever once cost a day of board time.
 *
 * A sample must call gc_sample_finish() on EVERY path, including its failures,
 * so a stall can never leave the board without the way back.
 */
#ifndef GC_SAMPLE_H
#define GC_SAMPLE_H

#include <stdbool.h>

/*
 * How many times the timer feed has serviced the watchdog, or 0 when
 * CONFIG_GC_SAMPLE_WDOG_TIMER_FEED is off. One feed a second, so this is also
 * how long the timer has been running -- which says whether the system tick
 * is alive, and a report can carry it.
 */
#ifdef CONFIG_GC_SAMPLE_WDOG_TIMER_FEED
uint32_t gc_sample_wdog_feeds(void);
#else
static inline uint32_t gc_sample_wdog_feeds(void) { return 0; }
#endif

/* A word of the caller's choosing, stored alongside the fault syndrome: use
 * it to say where the run had got to when it died. */
void gc_sample_fault_note(uint32_t note);

/* Record that the run reached `mark`, now, in memory the SoC reset does not
 * clear. Unlike gc_sample_fault_note() this is written as it happens, so it
 * survives a run that hangs without ever faulting -- the case the fault
 * record cannot see. Needs CONFIG_GC_SAMPLE_FAULT_RECORD; a no-op without it. */
void gc_sample_progress(uint32_t mark);

/* Service the inherited watchdog once. Harmless when it is not armed. */
void gc_sample_service_wdog(void);

/* The GIC interrupt id this CPU is handling, 0xffff when none is active. */
unsigned int gc_sample_active_intid(void);

/* Print the one-line verdict the console is grepped for:
 *   "<name>: PASS" or "<name>: FAIL <reason>".
 * Returns `pass`, so it can be used in a return statement. */
bool gc_sample_verdict(const char *name, bool pass, const char *reason);

/* Hold for CONFIG_GC_SAMPLE_HOLD_SECONDS, servicing the watchdog, then reset
 * the SoC. Does not return. */
void gc_sample_finish(void);

/* The end of a sample whose verdict is already in the result record (SNVS):
 * reset the SoC at once. The record is the readout, so the return time does
 * not have to carry the verdict. Only a code the sample claimed
 * (gc_sample_hold_code) or a GPU reset count, which no record field carries,
 * still gets its coded hold. Does not return. */
void gc_sample_finish_recorded(void);

/* Replace the PASS/FAIL hold with 70 s + CONFIG_GC_SAMPLE_HOLD_CODE_STEP s *
 * code (code 0..63), for a sample that has six bits of state to report
 * through the return time and no record field for them. */
void gc_sample_hold_code(unsigned int code);

/* Hold codes 56..63 mean "the GPU was reset N times", N = code - 55 clamped at
 * 8. The driver recovers a wedged GPU by resetting it, which costs a frame --
 * on a cluster that is a safety-relevant event, so it has to be visible in the
 * verdict rather than inferred from a kernel log the panel channel cannot show.
 *
 * A sample does not have to call anything: gc_sample_verdict() reads the
 * driver's counter itself through a weak gc_render_resets(), so a run that
 * recovered a wedge CANNOT report a clean PASS even if the sample's own checks
 * all passed. That is the whole point -- the failure mode this guards against
 * is a recovered hang being read as success. */
#define GC_SAMPLE_HOLD_GPU_RESET_BASE 56u
#define GC_SAMPLE_HOLD_GPU_RESET_MAX  8u

/* WDOG1's control register as it was when the image started (WT in bits
 * 15:8, WDE bit 2): what Linux or U-Boot left running underneath us. */
uint16_t gc_sample_wdog_control(void);

#endif /* GC_SAMPLE_H */
