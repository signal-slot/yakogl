// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR BXS-4-64 (BVNC 36.53.104.796, the TI AM62P's GPU).
//!
//! The firmware start and stop sequences and the firmware image loader are the
//! TI/Img Rogue DDK 25.2's own (`rgxstartstop.c`, `rgxfwimageutils.c`,
//! vendored unmodified in `third_party/ti-img-rogue-ddk-25.2`, Dual MIT/GPLv2,
//! used under MIT). The DDK wrote them to be built outside the DDK on top of
//! an abstraction layer, `rgxlayer.h`, whose implementation belongs to the
//! system that uses them. [`rgxlayer`] is ours.
//!
//! The rest of the kernel side is ours too, written against the vendored DDK
//! types (docs/source-form.md step 7, driver/docs/pvr-kernel-rust.md): the
//! context ([`km`]), the carveout's allocators ([`mem`]), the GPU page tables
//! ([`mmu`]), the command rings ([`ccb`]), the parameter manager's freelists
//! ([`freelist`]), the firmware-interface structures built before the
//! firmware starts ([`fwinit`]), the render service's firmware side
//! ([`render`]), and the layout numbers they use, pinned against the DDK
//! headers ([`fwif`]).

pub mod ccb;
pub mod freelist;
pub mod fwif;
pub mod fwinit;
pub mod kick_times;
pub mod km;
pub mod mem;
pub mod mmu;
pub mod pool;
pub mod render;
pub mod rgxlayer;
