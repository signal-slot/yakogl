// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR BXS-4-64 backend. Not in a `gpu-vivante` build: that one has
//! `vivante/` instead, and the shader ABI it compiles against is the GC7000's.
#![cfg(not(any(feature = "gpu-vivante", feature = "gpu-v3d")))]

pub mod backend;
pub mod blobs;
/// Mesa's csbgen packers (bindgen output, `yakogl-tools csbgen-bindings`;
/// build.rs compiles the C wrappers they link to).
#[rustfmt::skip]
#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unsafe_op_in_unsafe_fn,
    clippy::all,
    clippy::pedantic
)]
pub mod csbgen;
pub mod ffi;
