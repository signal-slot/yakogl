// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The identity registers of the GPUs the host tests model: the offset from
//! the GPU's base and the value, one table per GPU. A mock takes a table
//! and runs the same sequences over it; nothing in the kernel side reads
//! these tables, it reads the registers ([`super::ident`]).

use super::{
    GPU_AS_PRESENT, GPU_CORE_FEATURES, GPU_ID, GPU_L2_FEATURES, GPU_L2_PRESENT, GPU_MEM_FEATURES, GPU_MMU_FEATURES,
    GPU_SHADER_PRESENT, GPU_TILER_FEATURES, GPU_TILER_PRESENT,
};

/// The RK3588's Mali-G610 as the NanoPC-T6 reads it: panthor's probe line
/// under Armbian (2026-10-18/19, super's module doc) and `GPU_ID` read with
/// the GPU active. `CORE_FEATURES` was not in that line: its variant byte is
/// 0 because Mesa 26.2.1 knows the RK3588's GPU as the G610, whose one row
/// is variant 0 (src/panfrost/model/pan_model.c:88, matched exactly by
/// `pan_get_model`, :139-149); the upper bits are assumed 0.
pub const G610: [(usize, u32); 10] = [
    (GPU_ID, super::G610_GPU_ID),
    (GPU_L2_FEATURES, 0x0712_0306),
    (GPU_CORE_FEATURES, 0),
    (GPU_TILER_FEATURES, 0x809),
    (GPU_MEM_FEATURES, 0x301),
    (GPU_MMU_FEATURES, 0x2830),
    (GPU_AS_PRESENT, 0xff),
    (GPU_SHADER_PRESENT, 0x50005),
    (GPU_TILER_PRESENT, 1),
    (GPU_L2_PRESENT, 1),
];

/// The i.MX 95's Mali-G310. Its sources, in docs/notes/mali-gpu-model.md:
///
/// - `GPU_ID` 0xac74_0001: panthor's probe line on an i.MX 95 in the
///   review of "drm/panthor: Add i.MX95 support" (dri-devel and
///   linux-arm-kernel, February 2025,
///   <https://lists.freedesktop.org/archives/dri-devel/2025-February/493478.html>
///   and its replies): "mali-g310 id 0xac74 major 0x0 minor 0x0 status 0x1",
///   where id is `GPU_ID >> 16` (panthor_hw.c:101-104 at v6.18 prints the same
///   fields). 0xac74 is arch 10.12 revision 7, product 4, which Mesa 26.2.1's
///   model table names G310 (src/panfrost/model/pan_model.c:91-95,
///   `PAN_PROD_ID(10, 12, 4)`), and panthor too (panthor_hw.c:25-26).
/// - `L2_FEATURES` 0x0711_0306, `TILER_FEATURES` 0x809, `MEM_FEATURES` 0x1:
///   the "Features: L2:0x7110306 Tiler:0x809 Mem:0x1" of a boot log in the
///   same thread.
///
/// Everything else is in [`G310_ASSUMED`]: the thread's log, as it could be
/// read on 2026-11-20 (through a search engine's quotation; the archives
/// themselves could not be fetched from the host that wrote this), gives no
/// `MMU`, `AS` or `shader_present` value.
pub const G310: [(usize, u32); 10] = [
    (GPU_ID, G310_GPU_ID),
    (GPU_L2_FEATURES, 0x0711_0306),
    (GPU_CORE_FEATURES, G310_ASSUMED.core_features),
    (GPU_TILER_FEATURES, 0x809),
    (GPU_MEM_FEATURES, 0x1),
    (GPU_MMU_FEATURES, G310_ASSUMED.mmu_features),
    (GPU_AS_PRESENT, G310_ASSUMED.as_present),
    (GPU_SHADER_PRESENT, G310_ASSUMED.shader_present),
    (GPU_TILER_PRESENT, G310_ASSUMED.tiler_present),
    (GPU_L2_PRESENT, G310_ASSUMED.l2_present),
];

/// The G310's `GPU_ID` (see [`G310`]): arch 10.12 r7, product 4, r0p0
/// status 1.
pub const G310_GPU_ID: u32 = 0xac74_0001;

/// The G310 values no public source gives, in one place. Each is replaced on
/// the first day on the i.MX 95 by one register read: panthor's probe
/// lines under the NXP BSP or mainline (`dmesg | grep panthor`: "Features:
/// L2:.. Tiler:.. Mem:.. MMU:.. AS:..", "shader_present=.. l2_present=..
/// tiler_present=..") or our own [`super::ident`] dump of the same registers
/// (`MMU_FEATURES` 0x0014, `AS_PRESENT` 0x0018, `SHADER_PRESENT` 0x0100,
/// `TILER_PRESENT` 0x0110, `L2_PRESENT` 0x0120, `CORE_FEATURES` 0x0008).
pub struct Assumed {
    /// `CORE_FEATURES`: variant 0, the G310v1 (Mesa's smaller tile buffer).
    /// Which of the five G310 variants the i.MX 95 has is not known until
    /// the board's register is read; the backend takes it from the GPU
    /// (src/mali/model.rs), so this value decides nothing but the mock's.
    pub core_features: u32,
    /// `MMU_FEATURES`: the G610's 48 VA / 40 PA bits.
    pub mmu_features: u32,
    /// `AS_PRESENT`: eight address spaces, as the G610.
    pub as_present: u32,
    /// `SHADER_PRESENT`: two cores, 0x3 (not the G610's 0x50005, so a mask
    /// copied from the T6 shows).
    pub shader_present: u32,
    /// One tiler, one L2 (`MEM_FEATURES` 0x1 above says one L2 slice;
    /// panthor powers one core group, panthor_gpu.c:229-247).
    pub tiler_present: u32,
    pub l2_present: u32,
}

/// See [`Assumed`].
pub const G310_ASSUMED: Assumed = Assumed {
    core_features: 0,
    mmu_features: 0x2830,
    as_present: 0xff,
    shader_present: 0x3,
    tiler_present: 1,
    l2_present: 1,
};
