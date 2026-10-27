// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Broadcom V3D 4.2 backend of the Raspberry Pi 4, the sibling of
//! [`crate::vivante`] and [`crate::pvr`].
//!
//! `gl/draw.rs` resolves the current GL context into one
//! [`crate::backend::GlesDraw`] and calls the backend; this one records the
//! draw into a job built by the frame builder in `driver/src/v3d/` (through
//! FFI) and submits the job through the OSAL. The design is
//! `docs/online-shaderc/v3d-backend.md`, section "M6: the GL core on the V3D".
#![cfg(feature = "gpu-v3d")]
//!
//! | module | what it does |
//! | --- | --- |
//! | [`ffi`] | the job builder's C types and functions, pinned to its headers |
//! | [`state`] | the GL core's draw in gallium's terms (st/mesa's part), and the refusals |
//! | [`backend`] | jobs, their memory, the draw, the table |
//! | [`kick`] | the descriptor the OSAL's `submit()` takes |

pub mod backend;
pub mod ffi;
pub mod kick;
pub mod state;
pub mod texture;
