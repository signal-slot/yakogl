// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vector forms of tests/es3_int_out_alu_pvr.rs: an `ivec2`..`ivec4` or
//! `uvec2`..`uvec4` colour output whose value is an ALU result, on the
//! PowerVR online back end -- the shape of dEQP-GLES3's `fragment_out.*`
//! and `shaders.operator.*` integer cases (`o_out = v + ivec4(1)`,
//! `o_out = ivec4(f)`, `o_out = v * 3u`). The scalar form was refused with
//! "a register read that nothing writes" until select.rs `StoreOutput`
//! repeated lane 0 past the output's declared width (89c06d85); the vector
//! forms write every lane they declare.
//!
//! What is checked is the integer that lands in each pixel output of the
//! integer render target variant (`shc_blend_key_int_rt`): RG32I / RG32UI
//! for a two-component output, RGBA32I / RGBA32UI for three and four (GL
//! has no renderable RGB32 integer format; the fourth channel of a
//! three-component output is undefined and not checked). The built USC
//! bytes run through the host byte model (tests/usc_exec `run_binary`).
//! HOST-ONLY: no GPU arithmetic, and no GL read-back -- no host row renders
//! and reads back an integer target.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_vec_out_alu_pvr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    image: Vec<u8>,
}

/// One fragment shader through the chain under the variant `key`, or the
/// refusal.
fn compile_for(src: &str, key: u32) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(Rec { num_outputs: rec.num_outputs, image: code.iter().flat_map(|w| w.to_le_bytes()).collect() })
    })
    .unwrap()
}

/// The program's operations, in order: (name, the first destination's bank
/// and index).
fn ops(image: &[u8]) -> Vec<(&'static str, Option<(u32, u32)>)> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let Some(op) = ops_for(r, ph).next() else { continue };
            let d0 = g.dst.as_ref().and_then(|d| dsts(d)[0]).map(|o| (o.bank, o.index));
            out.push((op.name, d0));
        }
        off += g.total;
    }
    out
}

/// The pixel output registers a program writes, in order: the destinations
/// of the moves into the bank the last operation writes (the program ends
/// on its last pixel output write).
fn pixel_outputs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i)).collect()
}

/// The components of a GLSL integer vector type: `ivec3` is (3, signed).
fn shape(ty: &str) -> (usize, bool) {
    let n = ty.as_bytes()[ty.len() - 1] - b'0';
    (n as usize, ty.starts_with('i'))
}

/// The 32-bit integer variant a `ty` output is stored to: RG32I/RG32UI for
/// two components, RGBA32I/RGBA32UI for three and four.
fn rt32_key(ty: &str) -> u32 {
    let (n, signed) = shape(ty);
    shc_blend_key_int_rt(ShcIntRt { channels: if n == 2 { 2 } else { 4 }, bits: 32, signed })
}

/// A fragment shader of one flat varying `v` of type `vty` and one integer
/// colour output `o_out` of type `ty` at location 0.
fn fragment(vty: &str, ty: &str, body: &str) -> String {
    format!(
        "#version 300 es\nflat in highp {vty} v;\nlayout(location = 0) out highp {ty} o_out;\n\
         void main() {{ {body} }}\n"
    )
}

/// `src` under the 32-bit variant of `ty`, run once with component `c` of
/// the one varying `v[c]`: the dwords in pixel outputs 0 to `ty`'s width.
/// The varyings start at coefficient 4, after the W plane (as
/// tests/es2_pvr_ms4_constancy.rs lays them out); component `c` is every
/// coefficient of its plane (a flat varying is its value whichever one it
/// is read from), and everything else stays the sentinel.
fn run(src: &str, ty: &str, v: &[u32]) -> Vec<u32> {
    let (n, _) = shape(ty);
    let dwords = if n == 2 { 2 } else { 4 };
    let rec = compile_for(src, rt32_key(ty)).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs as usize, dwords, "{src}: one dword a channel");
    /* pixel output k is special register 32 + k (usc_exec: PIXOUT0) */
    let pix = pixel_outputs(&rec.image);
    for k in 0..n as u32 {
        assert!(pix.contains(&(32 + k)), "{src}: pixel output {k} written: {pix:?}");
    }
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    for (c, &x) in v.iter().enumerate() {
        inp.coeff[4 + 4 * c..8 + 4 * c].fill(x);
    }
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    (0..n as u32)
        .map(|k| *out.get(&k).unwrap_or_else(|| panic!("{src}: nothing in pixel output {k}: {out:?}")))
        .collect()
}

/// The input vectors the integer forms run with: distinct per component,
/// with signs, zero and the wrap points.
fn int_inputs(n: usize) -> Vec<Vec<i32>> {
    [[41, -5, 0, i32::MAX], [-1, 7, i32::MIN, 1000], [0, 0, -2, 3]].iter().map(|v| v[..n].to_vec()).collect()
}

fn uint_inputs(n: usize) -> Vec<Vec<u32>> {
    [[41, 0, u32::MAX, 0x8000_0000], [1, 2, 3, 4], [0x7fff_ffff, 9, 0, 65535]].iter().map(|v| v[..n].to_vec()).collect()
}

/// `o_out = v + ivecN(1)`: the per-component sum, wrapping at 32 bits.
fn int_add(ty: &str) {
    let (n, _) = shape(ty);
    let src = fragment(ty, ty, &format!("o_out = v + {ty}(1);"));
    for x in int_inputs(n) {
        let got: Vec<i32> =
            run(&src, ty, &x.iter().map(|&c| c as u32).collect::<Vec<_>>()).iter().map(|&w| w as i32).collect();
        let want: Vec<i32> = x.iter().map(|c| c.wrapping_add(1)).collect();
        assert_eq!(got, want, "{ty}: {x:?} + 1");
    }
}

/// `o_out = v + uvecN(1u)` and `v * 3u`: unsigned wrap per component.
fn uint_arith(ty: &str) {
    let (n, _) = shape(ty);
    let add = fragment(ty, ty, &format!("o_out = v + {ty}(1u);"));
    let mul = fragment(ty, ty, "o_out = v * 3u;");
    for x in uint_inputs(n) {
        let want: Vec<u32> = x.iter().map(|c| c.wrapping_add(1)).collect();
        assert_eq!(run(&add, ty, &x), want, "{ty}: {x:?} + 1u");
        let want: Vec<u32> = x.iter().map(|c| c.wrapping_mul(3)).collect();
        assert_eq!(run(&mul, ty, &x), want, "{ty}: {x:?} * 3u");
    }
}

/// `o_out = ivecN(f)` / `uvecN(f)` from a flat `vecN`: truncation toward
/// zero per component.
fn from_float(ty: &str) {
    let (n, signed) = shape(ty);
    let vty = format!("vec{n}");
    let src = fragment(&vty, ty, &format!("o_out = {ty}(v);"));
    let cases: &[[f32; 4]] = if signed {
        &[[2.75, -2.75, 0.5, -100.0], [1.0e6, -0.5, 7.0, -3.9]]
    } else {
        &[[3.9, 0.0, 65536.5, 1.0], [12.25, 100.0, 0.75, 4096.0]]
    };
    for x in cases {
        let x = &x[..n];
        let got = run(&src, ty, &x.iter().map(|c| c.to_bits()).collect::<Vec<_>>());
        let want: Vec<u32> = x.iter().map(|&c| if signed { c as i32 as u32 } else { c as u32 }).collect();
        assert_eq!(got, want, "{ty}({x:?})");
    }
}

/// `o_out = v`: the control, a copy of the input.
fn copy(ty: &str) {
    let (n, _) = shape(ty);
    let src = fragment(ty, ty, "o_out = v;");
    let x: Vec<u32> = [0xffff_fff9u32, 12, 0x8000_0000, 5][..n].to_vec();
    assert_eq!(run(&src, ty, &x), x, "{ty}: copy");
}

#[test]
fn ivec2_add_lands_in_the_output() {
    int_add("ivec2");
}

#[test]
fn ivec3_add_lands_in_the_output() {
    int_add("ivec3");
}

#[test]
fn ivec4_add_lands_in_the_output() {
    int_add("ivec4");
}

#[test]
fn uvec2_arithmetic_lands_in_the_output() {
    uint_arith("uvec2");
}

#[test]
fn uvec3_arithmetic_lands_in_the_output() {
    uint_arith("uvec3");
}

#[test]
fn uvec4_arithmetic_lands_in_the_output() {
    uint_arith("uvec4");
}

#[test]
fn ivec2_from_float_lands_in_the_output() {
    from_float("ivec2");
}

#[test]
fn ivec3_from_float_lands_in_the_output() {
    from_float("ivec3");
}

#[test]
fn ivec4_from_float_lands_in_the_output() {
    from_float("ivec4");
}

#[test]
fn uvec2_from_float_lands_in_the_output() {
    from_float("uvec2");
}

#[test]
fn uvec3_from_float_lands_in_the_output() {
    from_float("uvec3");
}

#[test]
fn uvec4_from_float_lands_in_the_output() {
    from_float("uvec4");
}

#[test]
fn ivec2_copy_lands_in_the_output() {
    copy("ivec2");
}

#[test]
fn ivec3_copy_lands_in_the_output() {
    copy("ivec3");
}

#[test]
fn ivec4_copy_lands_in_the_output() {
    copy("ivec4");
}

#[test]
fn uvec2_copy_lands_in_the_output() {
    copy("uvec2");
}

#[test]
fn uvec3_copy_lands_in_the_output() {
    copy("uvec3");
}

#[test]
fn uvec4_copy_lands_in_the_output() {
    copy("uvec4");
}
