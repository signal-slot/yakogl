// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The online compile cache keys on the PowerVR's runtime switches
//! (`pvr_native_3d`, `pvr_native_array`): the same source compiled with a
//! switch off is not handed back after it is turned on, nor the reverse.
//! The record's `native_cube` bits (mask 4: 3D, mask 2: 2D array) say which code a
//! program has.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_native_switch_cache
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::c_char;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

const FS_3D: &str = "#version 300 es
precision highp float;
uniform highp sampler3D s;
in vec3 uv;
out vec4 o;
void main() { o = texture(s, uv); }
";
const FS_ARRAY: &str = "#version 300 es
precision highp float;
uniform highp sampler2DArray s;
in vec3 uv;
out vec4 o;
void main() { o = texture(s, uv); }
";

fn compile(src: &str) -> ShcShader {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let f = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(SHC_STAGE_FRAGMENT, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "compile refused");
    out
}

/// The record's native bit with the switch off, then on, then off again
/// (the second off must come from the cache and still be the off code).
fn check(src: &str, set: extern "C" fn(core::ffi::c_int), bit: u32) {
    yakogl::shaderc::online::backend::forget_compiled();
    set(0);
    let off = compile(src).native_cube as u32 & bit;
    set(1);
    let on = compile(src).native_cube as u32 & bit;
    set(0);
    let off_again = compile(src).native_cube as u32 & bit;
    assert_eq!(off, 0, "switch off: not native");
    assert_eq!(on, bit, "switch on after an off compile: native (not the cached off record)");
    assert_eq!(off_again, 0, "switch off again: the off record");
}

/// One test: each check resets the compiler, and a binary that resets holds one.
#[test]
fn the_cache_key_covers_the_native_3d_and_array_switches() {
    check(FS_3D, yakogl::shaderc::pvr_set_native_3d, 4);
    check(FS_ARRAY, yakogl::shaderc::pvr_set_native_array, 2);
    yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
    yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
}
