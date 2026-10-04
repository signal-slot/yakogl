/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* WDOG1 armed by the image itself (CONFIG_GC_BOARD_WDOG_SELF_ARM); see
 * gc_wdog_arm.c. */
#ifndef GC_WDOG_ARM_H
#define GC_WDOG_ARM_H

#include <stdbool.h>
#include <stdint.h>

/* Whether WDOG1 may be serviced now: always before the arming; after it,
 * only while the uptime is inside CONFIG_GC_BOARD_WDOG_SECONDS, and never in
 * a proof image. gc_sample_service_wdog() asks this before every service. */
bool gc_wdog_arm_may_feed(void);

/* The proof image's end of the sample (gc_sample_finish): rewrite the proof
 * record and wait for WDOG1 (or the backstop) to reset the SoC. */
void gc_wdog_arm_proof_hold(void);

/* How many times the 1 s feeder timer has fed WDOG1 (0 in a proof image,
 * which never feeds). The kind STAGE breadcrumb's w3 (gc_result.h). */
uint32_t gc_wdog_arm_feeds(void);

#endif /* GC_WDOG_ARM_H */
