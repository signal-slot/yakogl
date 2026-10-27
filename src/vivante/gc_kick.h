/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_kick.h - what the Vivante backend hands gles_osal.h's submit().
 *
 * The GC7000 has no firmware: a submit is a command stream in GPU memory that
 * the front end fetches. The OSAL runs that stream -- by pointing the front end
 * at it, or by linking it into a ring it owns -- and returns a fence that
 * stands for its completion.
 *
 * The backend writes only its own commands. Everything that makes the stream
 * runnable and observable belongs to the OSAL, which is the side that knows the
 * hardware:
 *
 *   - the EVENT command that raises the completion interrupt. The GC7000 has
 *     30 events and recycles them, so a raw event id cannot be chosen by a
 *     caller that does not know which are in flight. The backend must NOT emit
 *     an EVENT, an END or a LINK.
 *   - the cache flushes and the pipeline stalls the epilogue needs before the
 *     fence is allowed to fire.
 *   - GC_KICK_TAIL_BYTES of scratch at the end of the stream's allocation
 *     (below).
 *
 * This is the whole GPU-specific part of the OSAL contract for this backend,
 * so it lives in its own header: the OSAL includes this, not the backend.
 *
 * The backend is Rust here (src/vivante/kick.rs is this struct's mirror); the
 * OSALs are C and include this. `yakogl-tools abi-layout` pins the two
 * together, so drift on either side fails the build.
 */
#ifndef GC_KICK_H
#define GC_KICK_H

#include <stdint.h>

/* First word of the descriptor, so an OSAL built for another GPU rejects it
 * instead of reading the wrong struct ("GCKK"). */
#define GC_KICK_MAGIC 0x474b4b43u

/*
 * Scratch the OSAL may write immediately after the stream, so the backend
 * stops this far short of the end of its command buffer.
 *
 * An OSAL that links the stream into a ring rather than copying it needs to
 * write a LINK back to the ring in the eight bytes at `stream_bytes`; that is
 * what the Zephyr driver's render service does (gc_render.h, GC_CMDBUF_TAIL).
 * Reserving the room unconditionally costs two words per frame and keeps the
 * backend free of a per-OSAL rule.
 */
#define GC_KICK_TAIL_BYTES 8u

struct gc_kick {
	uint32_t magic;        /* GC_KICK_MAGIC */
	uint32_t stream_bytes; /* length of the command stream, a multiple of 8 */
	uint64_t stream_gpu;   /* GPU address of the first command word */
};

#endif /* GC_KICK_H */
