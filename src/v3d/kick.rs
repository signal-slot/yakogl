// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the V3D backend hands `gles_osal.h`'s `submit()` (`v3d_kick.h`, the C
//! view the OSALs include).
//!
//! A job is a binning control list and a render control list with the
//! binner's tile allocation buffer and tile state array. The OSAL runs the
//! two lists one after the other, waits for each, and owns everything that
//! makes the job runnable: the cache invalidation before each list, the
//! write-back after a render the CPU will read, zeroing the tile state array,
//! the binner's overflow memory, and the reset after a hang. The backend
//! writes only the lists.

/// First word of the descriptor, so an OSAL built for another GPU rejects it
/// instead of reading the wrong struct ("V3DK").
pub const V3D_KICK_MAGIC: u32 = 0x4b44_3356;

/// `struct v3d_kick`: the submit descriptor. Addresses are V3D MMU (GPU
/// virtual) addresses.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dKick {
    /// [`V3D_KICK_MAGIC`].
    pub magic: u32,
    pub bcl_start: u32,
    pub bcl_end: u32,
    pub rcl_start: u32,
    pub rcl_end: u32,
    pub tile_alloc: u32,
    pub tile_alloc_size: u32,
    pub tile_state: u32,
    pub tile_state_size: u32,
    pub pad: u32,
}
