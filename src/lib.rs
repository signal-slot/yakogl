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
#![cfg_attr(any(not(feature = "host-test"), target_os = "none"), no_std)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::too_many_arguments)]
// Clippy from 2026-10 widens needless_range_loop to loops whose index also
// names the slot (handle allocators, parallel arrays); an iterator there reads
// worse than the index.
#![allow(clippy::needless_range_loop)]

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

#[cfg(all(test, not(feature = "host-test")))]
compile_error!(
    "unit tests need the host-test feature: cargo test --no-default-features --features host-test,gpu-powervr"
);

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
///  - GL_OES_texture_npot, GL_OES_depth_texture and
///    GL_OES_packed_depth_stencil are listed in GL_EXTENSIONS (ES 3.0 has
///    them in core). With the first, a non-power-of-two texture has a mip
///    chain and any wrap mode, as in ES 3.0, rather than ES 2.0 3.7.10's
///    "clamped to the edge and filtered without mipmaps"
///    (`gl::texture::NPOT_RESTRICTED`).
pub const REPORT_ES2: bool = cfg!(feature = "report-es2");

#[macro_use]
pub mod util;
#[macro_use]
pub mod osal;
mod abi_layout;
pub mod backend;
/// Where a draw's CPU time goes, step by step (the GL core and the GC7000).
pub mod draw_steps;
/// The depth/stencil contents every backend keeps between kicks, and when
/// a buffer may be let go.
pub mod ds_retention;
pub mod gl;
/// The NanoPC-T6's Mali-G610 backend (only in a `gpu-mali` build).
pub mod mali;
/// The PowerVR BXS-4-64 backend (only in a `gpu-powervr` build).
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
#[cfg(all(feature = "no-shader-table", not(feature = "shaderc-online"), not(feature = "precompiled-shaders")))]
compile_error!(
    "feature \"no-shader-table\" needs a back end that needs no Mesa table: enable \"shaderc-online\" or \"precompiled-shaders\""
);

/* A build has a shader back end: the in-process compiler, the precompiled
 * shaders alone (`precompiled-shaders` without the compiler), or the Mesa
 * reference as its table (`mesa-reference-table`, measurement images only;
 * build.rs sets `mesa_reference`), or more than one in the host tests. */
#[cfg(all(not(mesa_reference), not(feature = "shaderc-online"), not(feature = "precompiled-shaders")))]
compile_error!(
    "no shader back end: enable \"shaderc-online\" or \"precompiled-shaders\" (or, for a measurement image only, \"mesa-reference-table\")"
);
#[cfg(all(feature = "mesa-reference-table", feature = "no-shader-table"))]
compile_error!("features \"mesa-reference-table\" and \"no-shader-table\" contradict each other");

/* Exactly one GPU per build: one of the four features, never none and never
 * two. */
#[cfg(not(any(
    all(feature = "gpu-powervr", not(feature = "gpu-vivante"), not(feature = "gpu-v3d"), not(feature = "gpu-mali")),
    all(not(feature = "gpu-powervr"), feature = "gpu-vivante", not(feature = "gpu-v3d"), not(feature = "gpu-mali")),
    all(not(feature = "gpu-powervr"), not(feature = "gpu-vivante"), feature = "gpu-v3d", not(feature = "gpu-mali")),
    all(not(feature = "gpu-powervr"), not(feature = "gpu-vivante"), not(feature = "gpu-v3d"), feature = "gpu-mali"),
)))]
compile_error!(
    "enable exactly one GPU feature: \"gpu-powervr\" (AM62P), \"gpu-vivante\" (i.MX8MP), \"gpu-v3d\" (Raspberry Pi 4) or \"gpu-mali\" (NanoPC-T6)"
);

/* The V3D's only shader back end is the in-process compiler (no offline
 * table: docs/online-shaderc/v3d-backend.md), so a `gpu-v3d` build without
 * it, or with a table, is refused here by name
 * rather than built into a library that cannot compile a shader. */
#[cfg(all(feature = "gpu-v3d", not(feature = "shaderc-online")))]
compile_error!(
    "feature \"gpu-v3d\" needs \"shaderc-online\": the in-process compiler is the V3D's only shader back end"
);
#[cfg(all(feature = "gpu-v3d", mesa_reference))]
compile_error!(
    "feature \"gpu-v3d\" takes no table (\"no-shader-table\" in the host tests, no \"mesa-reference-table\"): there is no Mesa reference for the V3D"
);

/* The Mali-G610's only shader back end is the in-process compiler as well
 * (driver/docs/nanopc-t6-mali.md, "Shader compiler": our own Valhall back
 * end in src/shaderc/online/mali/, no table), refused here by name in the
 * same two ways. */
#[cfg(all(feature = "gpu-mali", not(feature = "shaderc-online")))]
compile_error!(
    "feature \"gpu-mali\" needs \"shaderc-online\": the in-process compiler is the Mali's only shader back end"
);
#[cfg(all(feature = "gpu-mali", mesa_reference))]
compile_error!(
    "feature \"gpu-mali\" takes no table (\"no-shader-table\" in the host tests, no \"mesa-reference-table\"): there is no Mesa reference table for the Mali"
);
