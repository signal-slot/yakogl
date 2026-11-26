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
//! | [`clear_mrt`] | the clear quad's fragment source for several colour attachments |
//! | [`kick`] | the descriptor the OSAL's `submit()` takes |
//!
//! A `gpu-v3d71` build (the Raspberry Pi 5's V3D 7.1, which implies
//! `gpu-v3d`) adds the 7.1 table `table71` (`yakogl_backend_v3d71`, the
//! build's `g_backend`) and the pure 7.1 modules its glue (`glue71`) maps the
//! GL core's draws onto (docs/notes/v3d71-table71.md). Each of them is also
//! host-tested on its own through `#[path]` (tests/v3d71_*.rs).
//!
//! The pure 7.1 modules are in the library only in a `host-test` build: they
//! build their results in `Vec`s and `String`s (and `prim71` names `std`),
//! and the product library is `no_std` with no allocator (src/lib.rs). Until
//! they have a `no_std` form, a product `gpu-v3d71` build carries the table
//! alone, which refuses every draw by name (`table71::REF_NO_RECORDING`).

pub mod backend;
pub mod clear_mrt;
pub mod ffi;
pub mod kick;
pub mod state;
pub mod texture;

#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod admit71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
#[allow(clippy::if_same_then_else)]
pub mod attr71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod clearplan71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod draw71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod ffi71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod fill71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod format71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod glue71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod ident71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod limits71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod prim71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod record71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
#[allow(clippy::eq_op)]
pub mod rt71;
#[cfg(feature = "gpu-v3d71")]
pub mod table71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod uniform71;
#[cfg(all(feature = "gpu-v3d71", feature = "host-test"))]
pub mod zskeep71;
