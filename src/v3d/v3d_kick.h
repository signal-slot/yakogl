/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_kick.h - what the V3D backend hands gles_osal.h's submit().
 *
 * The V3D has no firmware and no ring: a job is a binning control list (BCL,
 * thread CT0) and a render control list (RCL, thread CT1) in GPU memory, with
 * a tile allocation buffer and a tile state array for the binner. The OSAL
 * starts the binner, waits for it, starts the renderer and waits for that
 * (driver/src/v3d/v3d_hw.c: v3d_submit_bin, v3d_wait(FLDONE),
 * v3d_submit_render, v3d_wait(FRDONE)), and returns a fence that stands for
 * the job's completion. Everything that makes the job runnable belongs to the
 * OSAL: the cache invalidation before each list, the write-back after the
 * render the CPU will read, zeroing the tile state array, the binner's
 * overflow memory, and a reset after a hang.
 *
 * The backend is Rust (src/v3d/kick.rs is this struct's mirror); the OSALs
 * are C and include this. `yakogl-tools abi-layout` pins the two together.
 * Addresses are V3D MMU (GPU virtual) addresses.
 */
#ifndef V3D_KICK_H
#define V3D_KICK_H

#include <stdint.h>

/* First word of the descriptor, so an OSAL built for another GPU rejects it
 * instead of reading the wrong struct ("V3DK"). */
#define V3D_KICK_MAGIC 0x4b443356u

struct v3d_kick {
	uint32_t magic;           /* V3D_KICK_MAGIC */
	uint32_t bcl_start, bcl_end;
	uint32_t rcl_start, rcl_end;
	uint32_t tile_alloc, tile_alloc_size;
	uint32_t tile_state, tile_state_size;
	uint32_t pad;
};

#endif
