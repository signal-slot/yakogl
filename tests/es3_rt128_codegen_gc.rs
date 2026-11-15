// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the 128-bit colour-output split runs in the lowering
//! chain when keyed (item gc-rt128-codegen,
//! docs/notes/gc-128bit-render-targets.md).
//!
//! - with the key empty (every compile today) a `vec4` / `uvec4` output
//!   compiles to the same words as under a key naming outputs the shader does
//!   not write, which leaves the module untouched;
//! - with output 0 keyed the compile goes through the split: the companion
//!   output is `FragColor(4)`, past the four colour registers selection has
//!   (`MAX_COLOUR_OUTPUTS`), and the refusal is selection's, by name (the
//!   entry item lifts it);
//! - the key is put back after the compile, a refusal included.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_rt128_codegen_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::wide128_rt::{compile_rt_planes, set_compile_rt_planes, RtPlanes};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT};

/// The key is process-wide and `forget_compiled` resets the arena the code
/// lives in.
static COMPILING: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    g
}

fn text(log: &[core::ffi::c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// The code words of `src` as a fragment shader under `key`, or the log.
fn compile(src: &str, key: RtPlanes) -> Result<Vec<u32>, String> {
    forget_compiled();
    let src = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    let old = set_compile_rt_planes(key);
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(SHC_STAGE_FRAGMENT, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    assert_eq!(set_compile_rt_planes(old), key, "the key is still the one set");
    if rc == 0 {
        Ok(sh.usc_words().to_vec())
    } else {
        Err(text(&log))
    }
}

fn fs(ty: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nin vec2 uv;\nuniform highp {ty} u;\nout highp {ty} o;\nvoid main() {{ {body} }}\n"
    )
}

#[test]
fn an_empty_key_is_the_module_as_before() {
    let _g = serial();
    for (ty, body) in [("vec4", "o = u + vec4(uv, 0.0, 1.0);"), ("uvec4", "o = u + uvec4(1u);")] {
        let src = fs(ty, body);
        let plain = compile(&src, RtPlanes::NONE).expect("compiles");
        assert!(!plain.is_empty());
        let other = compile(&src, RtPlanes::NONE.with(2).with(3)).expect("compiles");
        assert_eq!(plain, other, "{ty}");
        assert_eq!(plain, compile(&src, RtPlanes::NONE).expect("compiles"), "{ty}: deterministic");
    }
}

#[test]
fn a_keyed_output_goes_through_the_split() {
    let _g = serial();
    for ty in ["vec4", "uvec4"] {
        let err = compile(&fs(ty, "o = u;"), RtPlanes::NONE.with(0)).expect_err("the companion has no register yet");
        assert!(err.contains("a colour output past the fourth draw buffer"), "{ty}: {err}");
    }
}

#[test]
fn the_key_is_put_back() {
    let _g = serial();
    assert_eq!(compile_rt_planes(), RtPlanes::NONE);
    let _ = compile(&fs("vec4", "o = u;"), RtPlanes::NONE.with(0));
    assert_eq!(compile_rt_planes(), RtPlanes::NONE, "a refusal included");
    let _ = compile(&fs("vec4", "o = u;"), RtPlanes::NONE);
    assert_eq!(compile_rt_planes(), RtPlanes::NONE);
}
