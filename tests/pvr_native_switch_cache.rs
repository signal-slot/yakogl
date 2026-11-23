// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The online compile cache keys on the PowerVR's native 3D and native
//! 2D-array switches (`pvr_set_native_3d`, `pvr_set_native_array`).
//!
//! The instruction selection produces different code for a `sampler3D` /
//! `sampler2DArray` look-up depending on the switch, but the cache used to key
//! on (sha, stage, blend key) only: a program compiled with a switch off came
//! back from the cache after the switch was turned on, and the reverse.
//! Pinned here: the same text under the other switch state is a new compile
//! (a different record, the compiler-run counter moves), and under the same
//! state a second compile is a cache hit (same record, counter unchanged).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_native_switch_cache
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]
use core::ffi::c_char;
use yakogl::shaderc::online::store::shc_online_compiler_runs;
use yakogl::shaderc::{
    native_3d_enabled, native_array_enabled, pvr_set_native_3d, pvr_set_native_array, ShcShader, SHC_STAGE_FRAGMENT,
};

const FS_3D: &str = "#version 300 es
precision highp float;
precision highp sampler3D;
uniform sampler3D s;
in vec3 c;
out vec4 o;
void main() { o = texture(s, c); }
";

const FS_ARRAY: &str = "#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform sampler2DArray s;
in vec3 c;
out vec4 o;
void main() { o = texture(s, c); }
";

/// Compile and keep the holder: the record stays live, so its address cannot be reused.
fn compile(src: &str) -> ShcShader {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc =
        unsafe { compile(SHC_STAGE_FRAGMENT, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    let msg: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    assert_eq!(rc, 0, "compiles: {msg}");
    out
}

/// Restores both switches (and the precompiled table) when dropped.
struct Restore(bool, bool);
impl Drop for Restore {
    fn drop(&mut self) {
        pvr_set_native_3d(self.0 as _);
        pvr_set_native_array(self.1 as _);
        yakogl::shaderc::precompiled::shc_precompiled_set_enabled(1);
    }
}

/// The switches, the cache and the counter are process globals: one test at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn check(src: &str, set: extern "C" fn(core::ffi::c_int)) {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = Restore(native_3d_enabled(), native_array_enabled());
    yakogl::shaderc::precompiled::shc_precompiled_set_enabled(0);
    yakogl::shaderc::online::backend::forget_compiled();
    set(0);
    let r0 = shc_online_compiler_runs();
    let off = compile(src);
    assert_eq!(shc_online_compiler_runs(), r0.wrapping_add(1), "first compile runs the compiler");
    let off2 = compile(src);
    assert_eq!(shc_online_compiler_runs(), r0.wrapping_add(1), "same switch: a cache hit");
    assert_eq!(off.priv_, off2.priv_, "same switch: the same record");
    set(1);
    let on = compile(src);
    assert_eq!(shc_online_compiler_runs(), r0.wrapping_add(2), "switch flipped: a new compile, not a stale hit");
    assert_ne!(off.priv_, on.priv_, "switch flipped: a different record");
    assert_ne!(off.usc_words(), on.usc_words(), "switch flipped: different code");
    let on2 = compile(src);
    assert_eq!(shc_online_compiler_runs(), r0.wrapping_add(2), "flipped switch unchanged: a cache hit");
    assert_eq!(on.priv_, on2.priv_);
    set(0);
    let back = compile(src);
    assert_eq!(shc_online_compiler_runs(), r0.wrapping_add(2), "back to off: the first record again");
    assert_eq!(back.priv_, off.priv_);
}

#[test]
fn native_3d_switch_is_part_of_the_cache_key() {
    check(FS_3D, pvr_set_native_3d);
}

#[test]
fn native_array_switch_is_part_of_the_cache_key() {
    check(FS_ARRAY, pvr_set_native_array);
}
