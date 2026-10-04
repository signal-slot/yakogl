// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! YakoGL: a portable, source-compatible OpenGL ES 2.0 (+ the ES 3.0 entry
//! points Qt's RHI uses) and EGL library for GPUs that have no operating
//! system driver.
//!
//! The C ABI is the product: applications compile against the Khronos headers
//! in `include/` and link `libyakogl.a`. Everything in this crate that a C
//! caller can see (`gl*`, `egl*`, `g_osal`, the structs in `include/`) is
//! `#[repr(C)]` / `extern "C"` and matches those headers.
//!
//! Layering (DESIGN.md):
//!   gl/       API entry points, context + object model, the draw resolver
//!   backend   the GPU-agnostic backend contract (`GlesDraw`, `YakoglBackend`)
//!   pvr/      the PowerVR BXS backend and its command generators
//!   shaderc/  the shader back end (offline table keyed by GLSL SHA-256)
//!   osal/     the OS abstraction (Zephyr, bare metal, Linux host)
//!
//! Threading: like the C library it replaces, YakoGL is single-threaded by
//! contract (one thread issues GL/EGL calls). Global state lives in
//! [`util::Global`] cells; see its safety note.
//!
//! Linking (README.md, "Linking"): this crate is an rlib without a panic
//! handler. C applications link `libyakogl.a`, which the `yakogl-capi` crate
//! (capi/) builds around it together with the panic handler and the C parts;
//! a Rust application depends on this crate, provides the one panic handler of
//! its image, and has the build system link the C parts (the OSAL, the
//! driver's command generators).
// Bare-metal targets stay no_std even with host-test, so that its
// compile_error below is what the build reports.
#![cfg_attr(any(not(any(feature = "host-test", feature = "std")), target_os = "none"), no_std)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]

// The shared structs are pinned at their LP64 layout (src/abi_layout.rs and
// the C headers, the driver structs mirrored in src/pvr/ffi.rs): on any other
// data model those pins would not apply.
#[cfg(not(target_pointer_width = "64"))]
compile_error!("yakogl supports 64-bit (LP64) targets only: the C ABI layouts are pinned for LP64");

#[cfg(any(
    all(feature = "osal-linux", feature = "osal-zephyr"),
    all(feature = "osal-linux", feature = "osal-baremetal"),
    all(feature = "osal-zephyr", feature = "osal-baremetal"),
))]
compile_error!(
    "select at most one of the features osal-linux, osal-zephyr, osal-baremetal \
     (depend on yakogl with default-features = false)"
);

#[cfg(all(feature = "host-test", target_os = "none"))]
compile_error!("the host-test feature is for host unit tests only (it links std and starts without a GPU backend)");

#[cfg(all(feature = "shaderc-remote", target_os = "none"))]
compile_error!(
    "the shaderc-remote feature needs an operating system (TCP sockets): a bare-metal or Zephyr \
     build uses the offline shader table"
);

#[cfg(all(test, not(feature = "host-test")))]
compile_error!("unit tests need the host-test feature: cargo test --no-default-features --features host-test");

/// True in the host-test build (std linked, null `g_backend`). The C archive
/// crate (capi/) refuses to build against it.
pub const HOST_TEST: bool = cfg!(feature = "host-test");

/// True when this build reports OpenGL ES 2.0 rather than ES 3.0 (the cargo
/// feature `report-es2`, which CMake `-DYAKOGL_REPORT_ES2=ON`, Kconfig
/// `CONFIG_YAKOGL_REPORT_ES2` and `make REPORT_ES2=1` turn on; those three
/// default to on, so a C or Zephyr build reports ES 2.0 unless it asks for ES
/// 3.0 with `OFF` / `n` / `0`. The cargo feature alone defaults to off).
///
/// The entry points are the same either way: what changes is the version this
/// library claims and the few rules ES 3.0 dropped from ES 2.0. Claiming ES
/// 3.0 while behaving as ES 2.0 is not a middle ground -- a client reads
/// GL_MAJOR_VERSION and believes it -- so the two go together:
///
///  - `glGetString(GL_VERSION)` / `GL_SHADING_LANGUAGE_VERSION` name ES 2.0
///    and GLSL ES 1.00;
///  - the ES 3.0-only queries GL_MAJOR_VERSION, GL_MINOR_VERSION and
///    GL_NUM_EXTENSIONS are GL_INVALID_ENUM;
///  - a non-power-of-two texture has no mip chain and samples as incomplete
///    unless it is clamped to the edge and filtered without mipmaps (ES 2.0
///    3.7.10, which needs GL_OES_texture_npot and this library has not got it).
pub const REPORT_ES2: bool = cfg!(feature = "report-es2");

#[macro_use]
pub mod util;
#[macro_use]
pub mod osal;
mod abi_layout;
pub mod backend;
/// The depth/stencil contents every backend keeps between kicks, and when
/// a buffer may be let go.
pub mod ds_retention;
pub mod gl;
/// The PowerVR BXS-4-64 backend (not in a `gpu-vivante` or `gpu-v3d` build).
pub mod pvr;
pub mod shaderc;
/// The Raspberry Pi 4's V3D 4.2 backend (only in a `gpu-v3d` build).
pub mod v3d;
/// The i.MX8MP's Vivante GC7000 backend (only in a `gpu-vivante` build).
pub mod vivante;

#[cfg(test)]
pub mod testutil;

/// What the test suite has to keep about itself, checked by reading the
/// crate's own source (`testutil` holds the guard those rules are about).
#[cfg(test)]
mod test_rules;

/* `no-shader-table` removes the fallback, so a build that takes it must have a
 * back end needing none. Saying so here turns a confusing pile of missing-item
 * errors into one sentence. */
#[cfg(all(feature = "no-shader-table", not(feature = "shaderc-online")))]
compile_error!("feature \"no-shader-table\" needs a back end that needs no table: enable \"shaderc-online\"");

/* One GPU per build. The V3D's only shader back end is the in-process
 * compiler (no offline table, no remote path: docs/online-shaderc/v3d-backend.md),
 * so a `gpu-v3d` build without it, or with a table or a server, is refused here
 * by name rather than built into a library that cannot compile a shader or
 * that compiles for the wrong GPU. */
#[cfg(all(feature = "gpu-v3d", feature = "gpu-vivante"))]
compile_error!("features \"gpu-v3d\" and \"gpu-vivante\" select two GPUs: a build has exactly one");
#[cfg(all(feature = "gpu-v3d", not(feature = "shaderc-online")))]
compile_error!(
    "feature \"gpu-v3d\" needs \"shaderc-online\": the in-process compiler is the V3D's only shader back end"
);
#[cfg(all(feature = "gpu-v3d", not(feature = "no-shader-table")))]
compile_error!("feature \"gpu-v3d\" needs \"no-shader-table\": there is no offline table for the V3D");
#[cfg(all(feature = "gpu-v3d", feature = "shaderc-remote"))]
compile_error!("feature \"gpu-v3d\" excludes \"shaderc-remote\": the shader server has no V3D path");
