// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A float `vec2` or `vec3` colour output whose value is an ALU result, on
//! the PowerVR online back end (`o = v.xy * 2.0 + 1.0`). Selection read all
//! four lanes of a colour output from its value, and a narrower ALU result
//! writes only its own lanes, so the program read registers nothing writes
//! and was refused ("a register read that nothing writes"). The lanes past
//! the output's declared width now repeat lane 0 (select.rs `StoreOutput`);
//! this file asserts nothing about them.
//!
//! What is checked, per shape: the PowerVR build succeeds, and the built USC
//! bytes, run through the host byte model (tests/usc_exec `run_binary`),
//! leave the expected value in each declared lane of pixel output 0 (the
//! default variant packs the colour as RGBA8 unorm, R in the low byte, so
//! a lane is compared as a byte, within one count, and a result outside
//! [0, 1] saturates; under the RGBA16F variant, `SHC_RT_F16X4`, the lanes
//! are halves, R in the low half of dword 0, B in the low half of dword 1,
//! and a value outside [0, 1] is kept). The shapes are `vec2` and `vec3` of
//! a sum, a product and a conversion (`vec2(ivec2)`), a `vec4` ALU output as
//! the control, and a `vec2` copy of a varying. HOST-ONLY: no GPU arithmetic
//! and no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_float_vec_out_alu_pvr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_rt, SHC_BLEND_KEY_REPLACE, SHC_RT_F16X4};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One fragment shader through the chain under the variant `key`: the USC
/// image, or the refusal.
fn compile(src: &str, key: u32) -> Result<Vec<u8>, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(code.iter().flat_map(|w| w.to_le_bytes()).collect())
    })
    .unwrap()
}

/// A fragment shader of one varying `v` of type `vty` and one float colour
/// output `o` of type `ty` at location 0.
fn fragment(vty: &str, ty: &str, body: &str) -> String {
    let flat = if vty.starts_with("ivec") { "flat " } else { "" };
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{flat}in {vty} v;\n\
         layout(location = 0) out {ty} o;\nvoid main() {{ {body} }}\n"
    )
}

/// `src` built under the variant `key` and run with the varying's component
/// `c` iterating to `comps[c]` (bits): the pixel output dwords. A
/// varying's coefficients start at 4, after the four of W, which is 1.0; a
/// flat varying's value is its plane's C coefficient, 2 on (fragment.rs
/// `flat_group`).
fn run_key(src: &str, key: u32, comps: &[u32]) -> std::collections::BTreeMap<u32, u32> {
    let img = compile(src, key).unwrap_or_else(|e| panic!("{src}: {e}"));
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff[0] = 1.0f32.to_bits();
    let at = if src.contains("flat in") { 6 } else { 4 };
    for (c, w) in comps.iter().enumerate() {
        inp.coeff[at + 4 * c] = *w;
    }
    usc_exec::run_binary(&img, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"))
}

/// `run_key` under the default (RGBA8) variant: the first `n` bytes of pixel
/// output 0 (R first).
fn run(src: &str, comps: &[u32], n: usize) -> Vec<u8> {
    let out = run_key(src, 0, comps);
    let word = *out.get(&0).unwrap_or_else(|| panic!("{src}: nothing in pixel output 0: {out:?}"));
    word.to_le_bytes()[..n].to_vec()
}

/// `src` under the RGBA16F variant: each declared lane exactly `want` (every
/// value chosen is a half).
fn check_f16(src: &str, comps: &[u32], want: &[f32]) {
    let out = run_key(src, SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F16X4), comps);
    let got: Vec<f32> = (0..want.len())
        .map(|i| {
            let w = *out.get(&(i as u32 / 2)).unwrap_or_else(|| panic!("{src}: no pixel output {}: {out:?}", i / 2));
            usc_exec::f16_to_f32((w >> (16 * (i % 2))) as u16)
        })
        .collect();
    assert_eq!(got, want, "{src}: lanes under RGBA16F");
}

/// `run`, each declared lane within one count of `want`.
fn check(src: &str, comps: &[u32], want: &[u8]) {
    let got = run(src, comps, want.len());
    assert!(got.iter().zip(want).all(|(g, w)| g.abs_diff(*w) <= 1), "{src}: lanes {got:?}, want {want:?}");
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// `o = v.xy * 2.0 + 1.0` in a `vec2`: the sum of a product.
#[test]
fn vec2_multiply_add_lands_in_both_lanes() {
    let src = fragment("vec4", "vec2", "o = v.xy * 2.0 + 1.0;");
    check(&src, &bits(&[-0.4, -0.1, 0.0, 0.0]), &[51, 204]);
    check(&src, &bits(&[3.0, -3.0, 0.0, 0.0]), &[255, 0]);
}

/// `o = v.xyz * 2.0 + 1.0` in a `vec3`.
#[test]
fn vec3_multiply_add_lands_in_three_lanes() {
    let src = fragment("vec4", "vec3", "o = v.xyz * 2.0 + 1.0;");
    check(&src, &bits(&[-0.4, -0.3, -0.2, 0.0]), &[51, 102, 153]);
    check(&src, &bits(&[3.0, -3.0, 0.0, 0.0]), &[255, 0, 255]);
}

/// A sum of different components, so the lanes are not one value broadcast:
/// `vec2(v.x + v.y, v.z - v.w)` and the `vec3` equivalent.
#[test]
fn mixed_lane_sums_keep_their_lanes_apart() {
    let v2 = fragment("vec4", "vec2", "o = vec2(v.x + v.y, v.z - v.w);");
    check(&v2, &bits(&[0.1, 0.3, 0.9, 0.1]), &[102, 204]);
    let v3 = fragment("vec4", "vec3", "o = vec3(v.x + v.y, v.z - v.w, v.x * v.w);");
    check(&v3, &bits(&[0.1, 0.3, 0.9, 0.5]), &[102, 102, 13]);
}

/// A conversion from an integer vector: `vec2(ivec2)` and `vec3(ivec3)` of a
/// flat varying.
#[test]
fn vec_from_ivec_conversion_lands_in_the_lanes() {
    let v2 = fragment("ivec4", "vec2", "o = vec2(v.xy);");
    check(&v2, &[1, 0, 0, 0], &[255, 0]);
    check(&v2, &[(-3i32) as u32, 7, 0, 0], &[0, 255]);
    let v3 = fragment("ivec4", "vec3", "o = vec3(v.xyz);");
    check(&v3, &[5, (-1i32) as u32, 1, 0], &[255, 0, 255]);
}

/// The control: a `vec4` ALU output writes all four lanes.
#[test]
fn vec4_multiply_add_lands_in_four_lanes() {
    let src = fragment("vec4", "vec4", "o = v * 2.0 + 1.0;");
    check(&src, &bits(&[-0.4, -0.3, -0.2, -0.5]), &[51, 102, 153, 0]);
}

/// The shape that always built: a `vec2` that is a copy of a varying.
#[test]
fn vec2_copy_lands_in_both_lanes() {
    let src = fragment("vec4", "vec2", "o = v.xy;");
    check(&src, &bits(&[0.2, 0.8, 0.0, 0.0]), &[51, 204]);
}

/// The same shapes under the RGBA16F variant, where the lanes keep values
/// outside [0, 1]: each declared lane is the float the arithmetic makes.
#[test]
fn f16_target_keeps_the_float_values() {
    let v2 = fragment("vec4", "vec2", "o = v.xy * 2.0 + 1.0;");
    check_f16(&v2, &bits(&[1.5, -3.25, 0.0, 0.0]), &[4.0, -5.5]);
    let v3 = fragment("vec4", "vec3", "o = v.xyz * 2.0 + 1.0;");
    check_f16(&v3, &bits(&[1.5, -3.25, 10.0, 0.0]), &[4.0, -5.5, 21.0]);
    let v3i = fragment("ivec4", "vec3", "o = vec3(v.xyz);");
    check_f16(&v3i, &[(-3i32) as u32, 7, 100, 0], &[-3.0, 7.0, 100.0]);
    let v4 = fragment("vec4", "vec4", "o = v * 2.0 + 1.0;");
    check_f16(&v4, &bits(&[1.5, -3.25, 10.0, -1.0]), &[4.0, -5.5, 21.0, -1.0]);
    let copy = fragment("vec4", "vec2", "o = v.xy;");
    check_f16(&copy, &bits(&[2.5, -0.75, 0.0, 0.0]), &[2.5, -0.75]);
}
