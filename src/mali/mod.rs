// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Arm Mali-G610 backend of the NanoPC-T6 (RK3588; Valhall, arch 10),
//! the sibling of [`crate::v3d`], [`crate::vivante`] and [`crate::pvr`].
//!
//! The design is driver/docs/nanopc-t6-mali.md; milestone L2 (a GL draw
//! through this backend, our compiler and the panthor development OSAL)
//! was measured on the board on 2026-10-18.
//!
//! | module | what it does |
//! | --- | --- |
//! | [`genxml`] | the descriptors and CS instructions, packed or refused |
//! | [`arena`] | bump arenas over (CPU mapping, GPU VA) pairs |
//! | [`cs`] | the command-stream encoder and decoder |
//! | [`frame`] | one frame: its descriptors and its command stream |
//! | [`program`] | a Valhall record as the frame builder's meta |
//! | [`backend`] | the GL core's capability answers, the table, the submit through the OSAL |

#![cfg(feature = "gpu-mali")]

pub mod arena;
pub mod backend;
pub mod blend;
pub mod cs;
pub mod frame;
pub mod genxml;
pub mod program;

#[cfg(test)]
mod frame_blend_tests;
#[cfg(test)]
mod frame_cull_tests;
#[cfg(test)]
mod frame_discard_tests;
#[cfg(test)]
mod frame_flip_tests;
#[cfg(test)]
mod frame_formats_tests;
#[cfg(test)]
mod frame_indexed_tests;
#[cfg(test)]
mod frame_instance_tests;
#[cfg(test)]
mod frame_msaa_tests;
#[cfg(test)]
mod frame_points_tests;
#[cfg(test)]
mod frame_polyoffset_tests;
#[cfg(test)]
mod frame_preload_tests;
#[cfg(test)]
mod frame_prims_tests;
#[cfg(test)]
mod frame_query_tests;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod frame_tex_tests;
#[cfg(test)]
mod frame_ubo_tests;
#[cfg(test)]
mod frame_zs_tests;
#[cfg(test)]
mod trace;
