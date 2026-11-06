/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The V3D 7.1 clear-only frame of rpi5_clear: the binning list, the render
 * list and the generic tile list of docs/notes/v3d71-clear-frame.md section 1,
 * packet by packet, for one RGBA8888 render target stored to a raster buffer.
 *
 * The 7.1 packers of H4 (driver/src/v3d/generated/v3d_packet_v71_pack.h) are
 * not in the tree yet, so the probe writes its packets itself: each field is
 * put at the bit position Mesa's generated 7.1 packer uses (gen_pack_header.py
 * on third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml, version 71).
 * tools/v3d/check71.d/clear71.sh compares the bytes with Mesa's own packers.
 *
 * No OS calls: the caller owns the memory and the GPU addresses.
 */
#ifndef FRAME71_H
#define FRAME71_H

#include <stdbool.h>
#include <stdint.h>

#include "v3d_frame.h" /* struct v3d_tiling, v3d_frame_tiling_gen */

/* A list being written: `cap` bytes at CPU `p`, GPU address `gpu`. A packet
 * that does not fit is not written and clears `ok`. */
struct cl71 {
	uint8_t *p;
	uint32_t gpu, len, cap;
	bool ok;
};

void cl71_init(struct cl71 *cl, void *buf, uint32_t gpu, uint32_t cap);

/* The GPU address one past the last byte written. */
static inline uint32_t cl71_end(const struct cl71 *cl)
{
	return cl->gpu + cl->len;
}

struct frame71 {
	uint32_t width, height;    /* pixels */
	uint32_t clear_rgba8;      /* R in bits 0..7, A in 24..31 */
	uint32_t target;           /* GPU address of the raster RGBA8888 buffer */
	uint32_t stride;           /* its row stride in bytes */
	uint32_t tile_alloc;       /* GPU address, 64-byte aligned */
};

/* The tile size and counts of the frame: v3d_frame_tiling_gen at 71 for one
 * 32 bpp render target (4 bytes a pixel), no multisampling. */
void frame71_tiling(const struct frame71 *f, struct v3d_tiling *t);

/*
 * Write the three lists. The generic tile list goes into `tl` (Mesa keeps it
 * in the job's indirect buffer); the render list branches to it. Returns
 * false, and nothing may be submitted, when a list overflowed or a value does
 * not fit its field (a zero or too large size, a misaligned tile allocation).
 */
bool frame71_build_clear(const struct frame71 *f, struct cl71 *bcl, struct cl71 *rcl, struct cl71 *tl);

#endif
