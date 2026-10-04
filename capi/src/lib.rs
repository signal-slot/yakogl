// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `libyakogl.a`: the YakoGL C ABI as a static library.
//!
//! The library crate `yakogl` carries every `#[no_mangle]` entry point; this
//! crate only links it into a staticlib and owns the one thing a staticlib
//! needs that a library must not have: the panic handler. Keeping the handler
//! here lets a Rust application depend on `yakogl` directly and provide its
//! own (a Rust image has exactly one).
//!
//! The Rust runtime symbols the staticlib defines (the panic entry point,
//! compiler_builtins' `memcpy`, `sin`, ...) are made local afterwards by
//! `capi/assemble.cmake`, so the archive links next to other Rust staticlibs
//! and leaves the platform's libc/libm in charge.
//!
//! One build is different: `shaderc-remote` (the host build that compiles its
//! shaders over TCP) links std into `yakogl`, and std already owns the panic
//! handler and the unwinder. This crate then defines neither -- two panic
//! handlers in one image is a link error -- and is plain `std` itself.
#![cfg_attr(not(feature = "shaderc-remote"), no_std)]

// Linked for its `#[no_mangle]` exports.
use yakogl as _;

const _: () = assert!(!yakogl::HOST_TEST, "libyakogl.a must not be built with the yakogl host-test feature");

/// Every panic is fatal (panic = "abort"): log it through the OSAL's log hook
/// and call the platform's `abort()`. std brings its own, so a
/// `shaderc-remote` build leaves this out.
#[cfg(not(feature = "shaderc-remote"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    yakogl::glog!("yakogl: panic: {}", info);
    extern "C" {
        fn abort() -> !;
    }
    // SAFETY: abort() has no preconditions.
    unsafe { abort() }
}
