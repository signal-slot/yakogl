/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Zephyr OSAL of the V3D 7.1 (BCM2712, Raspberry Pi 5): what it offers
 * beyond gles_osal.h's ops and zephyr_v3d.h's instruments. 7.1 images only
 * (zephyr_v3d71.c); nothing here is part of the 4.2 OSAL.
 */
#ifndef ZEPHYR_V3D71_H
#define ZEPHYR_V3D71_H

#include <stdbool.h>
#include <stdint.h>

#include "v3d_tfu71.h"

/* What one TFU job left in the registers, for the verdict. */
struct osal_v3d71_tfu_status {
	uint32_t cs_before, cs_after; /* TFU_CS (0x700) before the start and at the end */
	uint32_t hub_int_sts;         /* the hub status that ended the wait (or the last read) */
	uint32_t su;                  /* TFU_SU (0x704) after the wait */
	uint32_t us;                  /* the start to the end of the wait, polled in 1 us steps */
};

/*
 * Run one 7.1 TFU job (driver/src/v3d/v3d_tfu71.h) and wait for it: alone on
 * the GPU (the job in flight is finished first), the registers written in
 * Mesa's order (IIA, IIS, ICA, IUA, IOA, IOC, IOS, COEF0..3) and ICFG, with
 * IOC set, last. Done when the hub's TFUC status is up, or TFU_CS's
 * conversion count moved with the unit not busy. `timeout_us` bounds the
 * wait (0: the OSAL's stage bound, 4 s). Returns 0; -ETIMEDOUT when it did
 * not end; -EFAULT on an MMU fault; -EIO while the GPU is taken as hung;
 * -ENODEV before init or without a job. A fault or a timeout is logged and
 * the GPU reset (osal_v3d_recoveries); the job is not retried.
 * The caller owns the caches (the source written back before, the
 * destination invalidated after) and the GPU addresses (VA of the carve-out).
 * HUB_IDENT1 bit 17 is not consulted: the 7.1.10 reads 0 and runs it.
 *
 * The 4.2-shaped entries (osal_v3d_tfu, OSAL_QUERY_V3D_TFU_MIN_BYTES, _RUN,
 * _QUEUE) stay refused on this OSAL: their jobs have no IOC.
 */
int osal_v3d71_tfu(const struct v3d_tfu71_job *j, uint32_t timeout_us, struct osal_v3d71_tfu_status *st);

/* The TFU can be driven: init done and the GPU not taken as hung. */
bool osal_v3d71_tfu_present(void);

/* Jobs that ended, jobs that did not, and the sum of the wait times (us),
 * since boot. Any pointer may be NULL. */
void osal_v3d71_tfu_counts(uint32_t *ran, uint32_t *failed, uint32_t *us);

#endif
