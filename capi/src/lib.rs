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
#![no_std]

// Linked for its `#[no_mangle]` exports.
use yakogl as _;
// The gl_test runner's (gl_test_set_list, gl_test_runner, ...), for an
// archive a gl_test image links.
#[cfg(feature = "gl-test-runner")]
use yakogl_gltest_runner as _;

const _: () = assert!(!yakogl::HOST_TEST, "libyakogl.a must not be built with the yakogl host-test feature");

/// Every panic is fatal (panic = "abort"): log it through the OSAL's log hook
/// and call the platform's `abort()`.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    yakogl::glog!("yakogl: panic: {}", info);
    extern "C" {
        fn abort() -> !;
    }
    // SAFETY: abort() has no preconditions.
    unsafe { abort() }
}
