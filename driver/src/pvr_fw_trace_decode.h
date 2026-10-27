/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_fw_trace_decode.h - decoder for the PowerVR firmware's trace ring.
 *
 * The DDK firmware writes one ring per thread (RGXFWIF_TRACEBUF.sTraceBuf[],
 * RGXFW_THREAD_NUM of them) as a sequence of entries: a string-format ID
 * (RGXFW_LOG_VALIDID), a 64-bit timestamp in two dwords, and
 * RGXFW_SF_PARAMNUM(id) parameter dwords. The IDs and their format strings are
 * the DDK's (third_party/ti-img-rogue-ddk-25.2/include/rgx_fwif_sf.h).
 */

#ifndef PVR_FW_TRACE_DECODE_H
#define PVR_FW_TRACE_DECODE_H

#include <stdint.h>

#include "pvr_ddk.h"

#ifdef __cplusplus
extern "C" {
#endif

/**
 * pvr_fwt_emit_fn - sink for one decoded trace line.
 * @ctx:  opaque caller context.
 * @line: NUL-terminated decoded line, WITHOUT a trailing newline.
 *
 * If NULL is passed to pvr_fw_trace_dump(), lines go to printf("%s\n", line).
 */
typedef void (*pvr_fwt_emit_fn)(void *ctx, const char *line);

/**
 * pvr_fw_trace_dump() - decode and emit every thread's trace ring.
 * @tb:          CPU pointer to the RGXFWIF_TRACEBUF the firmware updates (the
 *               one RGXFWIF_SYSINIT.sTraceBufCtl points at).
 * @ring_cpu:    array[@num_threads] of CPU pointers to each thread's ring (the
 *               firmware only knows the rings by their firmware addresses).
 * @num_threads: threads to walk, at most RGXFW_THREAD_NUM.
 * @emit, @ctx:  line sink.
 *
 * Each ring is RGXFWIF_TRACEBUF.ui32TraceBufSizeInDWords long. The walk
 * starts at the thread's ui32TracePointer (the oldest entry once the ring has
 * wrapped) and emits one line per entry: "T<t>-<GROUP>> [<ts>] <message>". An
 * ID not in the table is emitted raw; RGXFW_SF_MAIN_ASSERT_FAILED prints the
 * firmware's assert record and ends that thread's walk.
 *
 * The caller must CPU-invalidate @tb and every ring first if they live in a
 * cached mapping (the firmware wrote them).
 */
void pvr_fw_trace_dump(const RGXFWIF_TRACEBUF *tb, uint32_t *const *ring_cpu,
		       uint32_t num_threads, pvr_fwt_emit_fn emit, void *ctx);

/**
 * pvr_fw_trace_dump_one() - decode and emit one thread's ring.
 * @space:       the thread's RGXFWIF_TRACEBUF_SPACE (trace pointer, assert record).
 * @ring_cpu:    CPU pointer to that thread's ring.
 * @ring_dwords: the ring's length in dwords.
 * @thread:      thread index, for the "T<t>" line prefix.
 * @emit, @ctx:  line sink.
 *
 * Returns the number of entries emitted.
 */
uint32_t pvr_fw_trace_dump_one(const RGXFWIF_TRACEBUF_SPACE *space,
			       const uint32_t *ring_cpu, uint32_t ring_dwords,
			       uint32_t thread, pvr_fwt_emit_fn emit, void *ctx);

#ifdef __cplusplus
}
#endif

#endif /* PVR_FW_TRACE_DECODE_H */
