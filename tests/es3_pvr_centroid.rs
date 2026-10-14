// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `centroid in` fragment inputs on the PowerVR back end. Without
//! multisampling the centroid is the pixel centre, so a program that declares
//! an input `centroid` must build the same fragment program and the same
//! varying layout as its twin without the qualifier.
//!
//! The sources are modelled on the dEQP linkage `varying.interpolation.centroid*`
//! and `qualification_order` cases (VK-GL-CTS, Apache-2.0, Copyright (c) 2014
//! The Android Open Source Project and The Khronos Group Inc.).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_centroid
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// The program's layout: the fragment program words, per stage each varying's
/// (location, components, vertex-output offset), the vertex output size and
/// the W flag.
type Layout = (Vec<u32>, Vec<(u8, u8, u8)>, Vec<(u8, u8, u8)>, u8, u8);

fn build(vs: &str, fs: &str) -> Result<Layout, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(format!("link: {}", text_of(&log)));
    }
    let var = |s: &ShcShader| -> Vec<(u8, u8, u8)> {
        s.varyings[..s.num_varyings as usize].iter().map(|v| (v.location, v.components, v.vtxout_off)).collect()
    };
    // SAFETY: `usc` points at `usc_dw` words of the compiled program.
    let words = unsafe { std::slice::from_raw_parts(fso.usc, fso.usc_dw as usize) }.to_vec();
    Ok((words, var(&vso), var(&fso), vso.vs_output_size, fso.uses_w))
}

const VS: &str = "#version 300 es
in highp vec4 a_pos;
in highp vec4 a_c;
QUAL out highp vec4 v_a;
QUAL out highp vec2 v_b;
QUAL2 out highp float v_c;
void main() { gl_Position = a_pos; v_a = a_c; v_b = a_c.xy; v_c = a_c.z; }
";
const FS: &str = "#version 300 es
QUAL in highp vec4 v_a;
QUAL in highp vec2 v_b;
QUAL2 in highp float v_c;
layout(location = 0) out highp vec4 o;
void main() { o = v_a + vec4(v_b, v_c, 1.0); }
";

/// Both stages with `QUAL` and `QUAL2` replaced.
fn program(q: &str, q2: &str) -> (String, String) {
    let f = |s: &str| s.replace("QUAL2", q2).replace("QUAL", q);
    (f(VS), f(FS))
}

fn assert_twin(q: &str, q2: &str, twin_q: &str, twin_q2: &str) {
    let (vs, fs) = program(q, q2);
    let (tvs, tfs) = program(twin_q, twin_q2);
    let got = build(&vs, &fs).unwrap_or_else(|e| panic!("`{q}`/`{q2}` does not build: {e}"));
    let twin = build(&tvs, &tfs).unwrap_or_else(|e| panic!("twin `{twin_q}`/`{twin_q2}` does not build: {e}"));
    assert_eq!(got, twin, "`{q}`/`{q2}` lays out differently from `{twin_q}`/`{twin_q2}`");
}

#[test]
fn centroid_smooth_is_the_smooth_layout() {
    assert_twin("centroid", "centroid", "smooth", "smooth");
    assert_twin("smooth centroid", "smooth centroid", "smooth", "smooth");
    assert_twin("centroid", "centroid", "", "");
}

#[test]
fn centroid_flat_is_the_flat_layout() {
    assert_twin("flat centroid", "flat centroid", "flat", "flat");
}

#[test]
fn mixed_orders_keep_the_layout_of_their_twins() {
    // Flat varyings go after the smooth ones, centroid or not.
    assert_twin("centroid", "flat", "smooth", "flat");
    assert_twin("flat", "centroid", "flat", "smooth");
    assert_twin("smooth", "flat centroid", "smooth", "flat");
}

#[test]
fn centroid_input_beside_frag_coord_builds() {
    let vs = "#version 300 es
in highp vec4 a_pos; centroid out highp vec2 v;
void main() { gl_Position = a_pos; v = a_pos.xy; }
";
    let fs = "#version 300 es
centroid in highp vec2 v; layout(location = 0) out highp vec4 o;
void main() { o = vec4(v, gl_FragCoord.z, 1.0); }
";
    let with = |q: &str| build(&vs.replace("centroid", q), &fs.replace("centroid", q)).expect("builds");
    assert_eq!(with("centroid"), with("smooth"));
}
