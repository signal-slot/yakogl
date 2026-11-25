// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A Mali link whose stages place a varying at different offsets (#11309,
//! the link half; docs/es2-cts-failures.md M.1 row 15).
//!
//! The 8ff01055 tree failed 10 dEQP-GLES2 `shaders.random.*.fragment.*` cases
//! at glLinkProgram with `a Mali link whose stages place a varying at
//! different offsets is not lowered yet ('c')` (`MALI_LAYOUTS_DIFFER`,
//! src/shaderc/online/mali/link.rs, raised in `link_shaders_mali` when
//! `pair` returns `Pairing::Differ` and no `VaryingOrder` can be made, or the
//! compile for it is not available). This file keeps the smallest pair that
//! meets it: the vertex shader writes `v_a` and `v_b` (two vec4), the
//! fragment shader reads only `v_b`. The vertex layout puts `v_b` at offset
//! 16 behind the unread `v_a`; the fragment shader's own layout puts it at 0,
//! and the link cannot rebuild the fragment shader for the vertex order
//! (the "vertex layout with an output the fragment shader does not read
//! before one it does" case of `MALI_LAYOUTS_DIFFER`'s documentation: Mesa's
//! link removes `v_a`, the vertex shader would have to be built for the
//! program). The reverse-declaration pair of gl_test case 209 `varyorder`
//! links on the host today (the rebuild by `VaryingOrder`).
//!
//! * `current_state_link_is_refused_at_different_offsets` passes today and
//!   asserts the refusal text. Flip it when #11309's link fix lands (it
//!   will fail then: remove it, and the `#[ignore]` of the next test).
//! * `target_link_succeeds_and_stages_agree_on_offsets` is `#[ignore]`d: the
//!   link succeeds and every varying the fragment record lists sits at the
//!   offset of the vertex record's slot of the same name. Plan #400 removes
//!   the `#[ignore]` with the fix (`live_varying_loads` in link.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_link_varying_offsets
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::online::mali::{MaliShader, Section};
use yakogl::shaderc::online::SHC_ONLINE;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

const LOG: usize = 4096;

/// What `MALI_LAYOUTS_DIFFER` says (link.rs), with the first varying that
/// differs in the fragment shader's order.
const REFUSAL: &str = "a Mali link whose stages place a varying at different offsets is not lowered yet ('v_b')";

const VS: &str = "#version 100
attribute vec4 a_a;
attribute vec4 a_b;
attribute vec4 a_pos;
varying highp vec4 v_a;
varying highp vec4 v_b;
void main()
{
    v_a = a_a;
    v_b = a_b;
    gl_Position = a_pos;
}
";

/// Reads only `v_b`, which the vertex layout places after the unread `v_a`.
const FS: &str = "#version 100
varying highp vec4 v_b;
void main()
{
    gl_FragColor = v_b;
}
";

/// The compiler's process-wide state takes one pair at a time.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static COMPILING: Mutex<()> = Mutex::new(());
    COMPILING.lock().unwrap_or_else(|p| p.into_inner())
}

fn log_text(log: &[c_char]) -> String {
    let s: String = log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
    s.trim().replace('\n', " / ")
}

fn compile(stage: u32, src: &str) -> ShcShader {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; LOG];
    let z = format!("{src}\0");
    let f = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { f(stage, z.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    assert_eq!(rc, 0, "the shader does not compile: {}\n{src}", log_text(&log));
    out
}

/// Compile both stages and link them: (rc, log, vertex, fragment); a
/// successful link may have replaced the fragment shader.
fn link() -> (i32, String, ShcShader, ShcShader) {
    let _one = one_at_a_time();
    yakogl::shaderc::online::backend::forget_compiled();
    let vs = compile(SHC_STAGE_VERTEX, VS);
    let mut fs = compile(SHC_STAGE_FRAGMENT, FS);
    let mut log = [0 as c_char; LOG];
    let f = SHC_ONLINE.link.expect("the online back end links");
    // SAFETY: two distinct valid shaders and a log of the size announced.
    let rc = unsafe { f(&vs, &mut fs, log.as_mut_ptr(), log.len()) };
    (rc, log_text(&log), vs, fs)
}

#[test]
fn current_state_link_is_refused_at_different_offsets() {
    let (rc, log, _vs, _fs) = link();
    assert_ne!(rc, 0, "the link holds: #11309's fix has landed; flip this test and un-ignore the target");
    assert!(log.contains(REFUSAL), "expected a log containing {REFUSAL:?}, got: {log}");
}

#[test]
#[ignore = "#11309 link: MALI_LAYOUTS_DIFFER is refused today (live_varying_loads fix pending, plan #400)"]
fn target_link_succeeds_and_stages_agree_on_offsets() {
    let (rc, log, vs, fs) = link();
    assert_eq!(rc, 0, "the link is refused: {log}");
    let (vr, fr) = (MaliShader::of(&vs).expect("a Mali record"), MaliShader::of(&fs).expect("a Mali record"));
    let generic = |r: &MaliShader<'_>, loc: u8| {
        r.varyings.iter().find(|s| s.section == Section::Generic && s.varying == Some(loc) && s.mask != 0).copied()
    };
    let name = |b: &[c_char]| -> Vec<u8> { b.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect() };
    let outs = &vs.varyings[..vs.num_varyings as usize];
    let ins = &fs.varyings[..fs.num_varyings as usize];
    assert_eq!(ins.len(), 1, "the fragment shader reads v_b");
    for v in ins {
        let o = outs.iter().find(|o| name(&o.name) == name(&v.name)).expect("the vertex shader writes it");
        let n = String::from_utf8_lossy(&name(&v.name)).into_owned();
        let a = generic(&vr, o.location).unwrap_or_else(|| panic!("{n}: no vertex slot"));
        let b = generic(&fr, v.location).unwrap_or_else(|| panic!("{n}: no fragment slot"));
        assert_eq!(b.offset, a.offset, "{n}: fragment offset vs vertex offset");
        assert_eq!(b.mask, a.mask, "{n}: components");
    }
}
