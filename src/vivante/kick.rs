// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the Vivante backend hands `gles_osal.h`'s `submit()`.
//!
//! The GC7000 has no firmware: a submit is a command stream in GPU memory
//! that the front end fetches. The OSAL runs that stream -- by pointing the
//! front end at it, or by linking it into a ring it owns -- and returns a
//! fence that stands for its completion.
//!
//! The backend writes only its own commands. Everything that makes the stream
//! runnable and observable belongs to the OSAL, which is the side that knows
//! the hardware:
//!
//!   - the EVENT command that raises the completion interrupt. The GC7000 has
//!     30 events and recycles them, so a raw event id cannot be chosen by a
//!     caller that does not know which are in flight. The backend must NOT
//!     emit an EVENT, an END or a LINK.
//!   - the cache flushes and the pipeline stalls the epilogue needs before
//!     the fence is allowed to fire.
//!   - [`GC_KICK_TAIL_BYTES`] of scratch at the end of the stream's
//!     allocation.
//!
//! This is the whole GPU-specific part of the OSAL contract for this backend,
//! so it lives in its own module: the OSAL includes `gc_kick.h`, the C view of
//! this, not the backend.

/// First word of the descriptor, so an OSAL built for another GPU rejects it
/// instead of reading the wrong struct ("GCKK").
pub const GC_KICK_MAGIC: u32 = 0x474b_4b43;

/// Scratch the OSAL may write immediately after the stream, so the backend
/// stops this far short of the end of its command buffer.
///
/// An OSAL that links the stream into a ring rather than copying it needs to
/// write a LINK back to the ring in the eight bytes at `stream_bytes`; that is
/// what the Zephyr driver's render service does (`gc_render.h`,
/// `GC_CMDBUF_TAIL`). Reserving the room unconditionally costs two words per
/// frame and keeps the backend free of a per-OSAL rule.
pub const GC_KICK_TAIL_BYTES: u32 = 8;

/// `struct gc_kick`: the submit descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GcKick {
    /// [`GC_KICK_MAGIC`].
    pub magic: u32,
    /// Length of the command stream, a multiple of 8.
    pub stream_bytes: u32,
    /// GPU address of the first command word.
    pub stream_gpu: u64,
}

impl GcKick {
    pub const ZERO: GcKick = GcKick { magic: 0, stream_bytes: 0, stream_gpu: 0 };
}
