// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The identity registers of the GPUs the host tests model: the offset from
//! the GPU's base and the value, one table per GPU. A mock takes a table
//! and runs the same sequences over it; nothing in the kernel side reads
//! these tables, it reads the registers ([`super::ident`]).

use super::{
    GPU_AS_PRESENT, GPU_ID, GPU_L2_FEATURES, GPU_L2_PRESENT, GPU_MEM_FEATURES, GPU_MMU_FEATURES, GPU_SHADER_PRESENT,
    GPU_TILER_FEATURES, GPU_TILER_PRESENT,
};

/// The RK3588's Mali-G610 as the NanoPC-T6 reads it: panthor's probe line
/// under Armbian (2026-10-18/19, super's module doc) and `GPU_ID` read with
/// the GPU active.
pub const G610: [(usize, u32); 9] = [
    (GPU_ID, super::G610_GPU_ID),
    (GPU_L2_FEATURES, 0x0712_0306),
    (GPU_TILER_FEATURES, 0x809),
    (GPU_MEM_FEATURES, 0x301),
    (GPU_MMU_FEATURES, 0x2830),
    (GPU_AS_PRESENT, 0xff),
    (GPU_SHADER_PRESENT, 0x50005),
    (GPU_TILER_PRESENT, 1),
    (GPU_L2_PRESENT, 1),
];

/// The i.MX 95's Mali-G310.
///
/// Sourced: `GPU_ID`'s arch major 10, arch minor 12 and product major 4
/// (bits 31:28, 27:24, 19:16), Mesa 26.2.1's model table,
/// src/panfrost/model/pan_model.c:91-100 (`PAN_PROD_ID(10, 12, 4)`, "G310v1"
/// to "G310v5"; the variant is `GPU_CORE_FEATURES & 0xff`,
/// src/panfrost/lib/kmod/panthor_kmod.c:162, which this kernel side does not
/// read). panthor names product 4 of arch 10 "Mali-G310" (panthor_hw.c
/// `get_gpu_model_name`).
///
/// Everything else is in [`G310_ASSUMED`]: no public source this repository
/// could read on 2026-11-20 gave an i.MX 95 probe line (no panthor
/// "Mali-G310 id ... Features: ... shader_present=..." line from a list or
/// a bug report was found; the search could not reach the web from the
/// host that wrote this). The values there are chosen to differ from the
/// G610's where the code must follow the register (shader cores, the
/// `GPU_ID` low half) and to equal them where nothing better is known.
pub const G310: [(usize, u32); 9] = [
    (GPU_ID, G310_GPU_ID),
    (GPU_L2_FEATURES, G310_ASSUMED.l2_features),
    (GPU_TILER_FEATURES, G310_ASSUMED.tiler_features),
    (GPU_MEM_FEATURES, G310_ASSUMED.mem_features),
    (GPU_MMU_FEATURES, G310_ASSUMED.mmu_features),
    (GPU_AS_PRESENT, G310_ASSUMED.as_present),
    (GPU_SHADER_PRESENT, G310_ASSUMED.shader_present),
    (GPU_TILER_PRESENT, G310_ASSUMED.tiler_present),
    (GPU_L2_PRESENT, G310_ASSUMED.l2_present),
];

/// The G310's `GPU_ID`: arch 10.12, product 4 (Mesa, sourced); arch
/// revision 0 and version r0p0 status 0 assumed ([`G310_ASSUMED`]).
pub const G310_GPU_ID: u32 = 10 << 28 | 12 << 24 | G310_ASSUMED.arch_rev << 20 | 4 << 16;

/// The G310 values no public source gives, in one place. Each is replaced on
/// the first day on the i.MX 95 by one register read: panthor's probe
/// lines under the NXP BSP or mainline (`dmesg | grep panthor`: "Mali-G310
/// id 0x.... major .. minor .. status ..", "Features: L2:.. Tiler:.. Mem:..
/// MMU:.. AS:..", "shader_present=.. l2_present=.. tiler_present=..") or our
/// own [`super::ident`] dump of the same registers.
pub struct Assumed {
    /// `GPU_ID` bits 23:20 (bits 15:0, the version, are taken as 0).
    pub arch_rev: u32,
    /// `L2_FEATURES`: the G610's (a 64-byte line, which the tiler heap's
    /// context stride follows, heap.rs `ctx_stride`).
    pub l2_features: u32,
    /// `TILER_FEATURES`, `MEM_FEATURES`: the G610's.
    pub tiler_features: u32,
    pub mem_features: u32,
    /// `MMU_FEATURES`: the G610's 48 VA / 40 PA bits.
    pub mmu_features: u32,
    /// `AS_PRESENT`: eight address spaces, as the G610.
    pub as_present: u32,
    /// `SHADER_PRESENT`: two cores, 0x3 (not the G610's 0x50005, so a mask
    /// copied from the T6 shows).
    pub shader_present: u32,
    /// One tiler, one L2 (panthor powers one core group, panthor_gpu.c
    /// `panthor_gpu_l2_power_on`).
    pub tiler_present: u32,
    pub l2_present: u32,
}

/// See [`Assumed`].
pub const G310_ASSUMED: Assumed = Assumed {
    arch_rev: 0,
    l2_features: 0x0712_0306,
    tiler_features: 0x809,
    mem_features: 0x301,
    mmu_features: 0x2830,
    as_present: 0xff,
    shader_present: 0x3,
    tiler_present: 1,
    l2_present: 1,
};
