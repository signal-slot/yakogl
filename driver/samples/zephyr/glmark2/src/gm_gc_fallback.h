/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gm_gc_fallback.h: the GC7000 back end's fallback and wait counters, the
 * source of PERF page 4 (gm_perf_page.h). The library provides the strong
 * definition of yakogl_gc_fallback_stats (item fb-counters); the sample
 * defines a __weak one returning -ENOTSUP, so an image built before the
 * library counters exist reports "not available" (page 4 = 0xffff in every
 * value) and cannot be mistaken for zero.
 *
 * The contract: every field is a cumulative count since boot, never reset,
 * 64 bits wide (no wrap to handle). The page takes the difference between
 * the scene's first frame and the end of its last.
 */
#ifndef GM_GC_FALLBACK_H
#define GM_GC_FALLBACK_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

struct yakogl_gc_fallback_stats {
	/* glGenerateMipmap chains the GPU drew (once per call, however many
	 * levels the chain has) */
	uint64_t gpu_mip_chains;
	/* glGenerateMipmap calls that took the CPU filter instead */
	uint64_t cpu_mip_calls;
	/* times the CPU waited for the GPU outside the swap (the depth
	 * resolve's finish(), a tiled copy of a rendered texture, a readback
	 * of a render target; the swap's own wait is page 1) */
	uint64_t gpu_waits;
	/* bytes the CPU read or wrote in passes over the GPU carve-out
	 * (non-cacheable memory: tiled copies, CPU mipmaps, depth resolves) */
	uint64_t carveout_cpu_bytes;
};

/* Fills *out. 0 on success; -ENOTSUP when this build keeps no such counters
 * (the sample's weak definition); another negative errno on failure, which
 * the page treats like -ENOTSUP. */
int yakogl_gc_fallback_stats(struct yakogl_gc_fallback_stats *out);

#ifdef __cplusplus
}
#endif

#endif /* GM_GC_FALLBACK_H */
