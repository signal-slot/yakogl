// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-drivers`: the display and SoC drivers original to this project
//! (docs/source-form.md, group E), written from reference manuals and
//! datasheets, with register names and bit positions from NXP's BSD-3 hal_nxp
//! headers ([`soc::imx8mp::regs`], generated).
//!
//! The crate is the register logic only. Zephyr sees C: each driver has a
//! shim in `driver/drivers/<class>/` that registers a device
//! (`DEVICE_DT_INST_DEFINE`, the class's API table), maps the register ranges,
//! fills a `#[repr(C)]` configuration from the devicetree and calls the
//! `extern "C"` entry points of [`ffi`]. Kernel objects stay on the C side.
//! The layout of every shared struct is pinned on both sides
//! (`include/yakogl_drivers.h` `_Static_assert`s, [`ffi`] `const` asserts).
//!
//! Modules:
//! - [`ip`]: drivers for an IP block, independent of the SoC that carries it.
//! - [`soc`]: SoC glue (clock roots, gates) and the generated register maps.
//! - [`gpu`]: the OS layer under vendored GPU code: the PowerVR DDK's
//!   `rgxlayer.h` ([`gpu::pvr::rgxlayer`], feature `pvr`); and the
//!   Vivante GC7000 kernel side ([`gpu::vivante`], feature `vivante`).
//! - [`mmio`]: register access; [`env`]: the services the OS lends.
//!
//! `no_std`, no allocator, no floating point (the target is softfloat and the
//! code runs at PRE_KERNEL_1 and in ISRs), no statics: all state lives in the
//! device's `data` blob, so every instance is independent.
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]
// The same allow as the library's src/lib.rs: clippy from 2026-10 flags index
// loops over parallel per-render-target arrays.
#![allow(clippy::needless_range_loop)]

pub mod board;
pub mod env;
pub mod ffi;
pub mod gpu;
pub mod ip;
pub mod mmio;
pub mod soc;
pub mod video;

#[cfg(test)]
pub mod model;
#[cfg(test)]
pub mod model_am62p;
#[cfg(test)]
pub mod model_hdmi;

/// errno numbers, one table per C library ([`errno_numbers`]).
pub mod errno_numbers;

/// The errno values the entry points return (negated): Zephyr's numbering
/// ([`errno_numbers::ZEPHYR`]), because the crate's C callers are built
/// against Zephyr's libc. A Linux build that links the crate translates in
/// its own shim (tools/pvr/km/record_c.c pins ETIMEDOUT to 116).
pub mod errno {
    use crate::errno_numbers::ZEPHYR as N;

    pub const EIO: i32 = N.eio;
    pub const EAGAIN: i32 = N.eagain;
    pub const ENOMEM: i32 = N.enomem;
    pub const EFAULT: i32 = N.efault;
    pub const ENOSPC: i32 = N.enospc;
    pub const ERANGE: i32 = N.erange;
    pub const ENODEV: i32 = N.enodev;
    pub const EBUSY: i32 = N.ebusy;
    pub const EINVAL: i32 = N.einval;
    pub const ENODATA: i32 = N.enodata;
    pub const EMSGSIZE: i32 = N.emsgsize;
    pub const ETIMEDOUT: i32 = N.etimedout;
    /// The PowerVR render service stopped taking kicks
    /// (`gpu::pvr::freelist::stop`): a kick it cannot complete was found.
    pub const ESHUTDOWN: i32 = N.eshutdown;
    pub const ENOTSUP: i32 = N.enotsup;
    /// A staged bring-up stopped on purpose (STOP_AFTER).
    pub const ECANCELED: i32 = N.ecanceled;
}

/// The panic handler of the staticlib: hand the location to the C side, which
/// logs it, flushes the log (`LOG_PANIC()`) and stops the kernel
/// (`k_panic()`). Nothing in the crate panics on purpose -- errors are
/// returned as negative errno -- so this is for a broken invariant only.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe extern "C" {
        fn yd_panic(file: *const u8, file_len: usize, line: u32) -> !;
    }
    let (file, line) = match info.location() {
        Some(l) => (l.file(), l.line()),
        None => ("?", 0),
    };
    // SAFETY: the C side reads `file_len` bytes from `file` and never returns.
    unsafe { yd_panic(file.as_ptr(), file.len(), line) }
}
