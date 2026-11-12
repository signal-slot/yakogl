// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Regression guard for the select.rs `StoreOutput` fix (89c06d85, recorded
//! in docs/notes/pvr-int-output-alu.md): integer colour outputs on the
//! PowerVR online back end in the shapes tests/es3_int_out_alu_pvr.rs,
//! tests/es3_int_out_copy_pvr.rs and tests/es3_int_vec_out_alu_pvr.rs do not
//! cover -- the output assigned in both branches of an if/else on a varying,
//! assigned in one branch after a default, accumulated in a constant-bound
//! `for` loop, returned from a helper function, computed with `& | ^ << >> ~`
//! and `%`, and from uniform-plus-varying arithmetic. `int` and `uint`
//! scalars, one `ivec2` and one `uvec4`.
//!
//! Each program must build under the 32-bit integer render target variant
//! and the dword in each pixel output must equal the GLSL result for two or
//! more inputs (a negative `int` among them; `%` is only given operands GLSL
//! ES 3.00 defines it for). The built USC bytes run through the host byte
//! model (tests/usc_exec `run_binary`). HOST-ONLY: no GPU arithmetic.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_out_cf_pvr
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

/// (components, signed) of `int`, `uint`, `ivecN`, `uvecN`.
fn shape(ty: &str) -> (usize, bool) {
    let n = match ty.as_bytes()[ty.len() - 1] {
        c @ b'2'..=b'4' => (c - b'0') as usize,
        _ => 1,
    };
    (n, ty.starts_with('i'))
}

/// The 32-bit integer variant `ty` is stored to: R32I/R32UI scalar,
/// RG32I/RG32UI for two components, RGBA32I/RGBA32UI for four.
fn rt32_key(ty: &str) -> u32 {
    let (n, signed) = shape(ty);
    shc_blend_key_int_rt(ShcIntRt { channels: if n == 1 || n == 2 { n as u8 } else { 4 }, bits: 32, signed })
}

/// A fragment shader of one flat varying `v` and one uniform `u` of type
/// `ty`, an output `o_out` of type `ty` at location 0, and `pre` (helper
/// functions) before `main`.
fn fragment(ty: &str, pre: &str, body: &str) -> String {
    let decls = format!("flat in highp {ty} v;\nuniform highp {ty} u;\nlayout(location = 0) out highp {ty} o_out;\n");
    format!("#version 300 es\n{decls}{pre}\nvoid main() {{ {body} }}\n")
}

/// `src` built under the 32-bit variant of `ty`, run with the varying's
/// components `v` (component `c` fills coefficients 4+4c..8+4c) and every
/// uniform dword `u`: the dwords in pixel outputs 0 to `ty`'s width.
fn run(src: &str, ty: &str, v: &[u32], u: u32) -> Vec<u32> {
    let (n, _) = shape(ty);
    let rec = compile_for(src, rt32_key(ty)).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs as usize, if n <= 2 { n } else { 4 }, "{src}: one dword a channel");
    let pix = pixel_outputs(&rec.image);
    for k in 0..n as u32 {
        assert!(pix.contains(&(32 + k)), "{src}: pixel output {k} written: {pix:?}");
    }
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.shared.fill(u);
    for (c, &x) in v.iter().enumerate() {
        inp.coeff[4 + 4 * c..8 + 4 * c].fill(x);
    }
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    (0..n as u32)
        .map(|k| *out.get(&k).unwrap_or_else(|| panic!("{src}: nothing in pixel output {k}: {out:?}")))
        .collect()
}

/// Scalar `int` program against a Rust model, over `xs` (uniform `u`).
fn int_case(pre: &str, body: &str, xs: &[i32], u: i32, model: impl Fn(i32, i32) -> i32) {
    let src = fragment("int", pre, body);
    for &x in xs {
        let got = run(&src, "int", &[x as u32], u as u32)[0] as i32;
        assert_eq!(got, model(x, u), "{body}: v = {x}, u = {u}");
    }
}

/// Scalar `uint` program against a Rust model.
fn uint_case(pre: &str, body: &str, xs: &[u32], u: u32, model: impl Fn(u32, u32) -> u32) {
    let src = fragment("uint", pre, body);
    for &x in xs {
        let got = run(&src, "uint", &[x], u)[0];
        assert_eq!(got, model(x, u), "{body}: v = {x}, u = {u}");
    }
}

const INTS: &[i32] = &[7, -5, 0, 1000, -123_456, i32::MAX, i32::MIN];
const NONNEG: &[i32] = &[7, 0, 1000, 123_456, i32::MAX];
const UINTS: &[u32] = &[7, 0, 1, 65_535, 0x8000_0000, u32::MAX];

// ---- int ----

#[test]
fn int_if_else_both_branches() {
    int_case("", "if (v > 0) { o_out = v * 2; } else { o_out = v - 3; }", &[7, -5, 0, 1000, -123_456], 0, |v, _| {
        if v > 0 {
            v * 2
        } else {
            v - 3
        }
    });
}

#[test]
fn int_default_then_one_branch() {
    int_case("", "o_out = 5; if (v < 0) { o_out = v + 1; }", &[7, -5, 0, 1000, -123_456], 0, |v, _| {
        if v < 0 {
            v + 1
        } else {
            5
        }
    });
}

#[test]
fn int_for_loop_accumulates() {
    int_case(
        "",
        "int s = 0; for (int i = 0; i < 4; i++) { s += v + i; } o_out = s;",
        &[7, -5, 0, 1000, -123_456],
        0,
        |v, _| (0..4).fold(0i32, |s, i| s + v + i),
    );
}

#[test]
fn int_function_return() {
    int_case(
        "int f(int a) { if (a < 0) { return -a; } return a * 3 + 1; }",
        "o_out = f(v);",
        &[7, -5, 0, 1000, -123_456],
        0,
        |v, _| if v < 0 { -v } else { v * 3 + 1 },
    );
}

#[test]
fn int_bitwise() {
    int_case("", "o_out = ((v & 0xff) | (v << 3)) ^ (v >> 2) ^ ~v;", INTS, 0, |v, _| {
        ((v & 0xff) | v.wrapping_shl(3)) ^ (v >> 2) ^ !v
    });
}

#[test]
fn int_remainder() {
    int_case("", "o_out = v % 7 + v % 1000;", NONNEG, 0, |v, _| v % 7 + v % 1000);
}

#[test]
fn int_uniform_plus_varying() {
    int_case("", "o_out = u * v + u;", INTS, -3, |v, u| u.wrapping_mul(v).wrapping_add(u));
    int_case("", "o_out = u * v + u;", &[-5, 11], 9, |v, u| u.wrapping_mul(v).wrapping_add(u));
}

// ---- uint ----

#[test]
fn uint_if_else_both_branches() {
    uint_case("", "if (v > 100u) { o_out = v * 2u; } else { o_out = v + 3u; }", UINTS, 0, |v, _| {
        if v > 100 {
            v.wrapping_mul(2)
        } else {
            v + 3
        }
    });
}

#[test]
fn uint_default_then_one_branch() {
    uint_case("", "o_out = 5u; if (v > 3u) { o_out = v - 1u; }", UINTS, 0, |v, _| if v > 3 { v - 1 } else { 5 });
}

#[test]
fn uint_for_loop_accumulates() {
    uint_case("", "uint s = 0u; for (int i = 0; i < 4; i++) { s += v + uint(i); } o_out = s;", UINTS, 0, |v, _| {
        (0..4u32).fold(0u32, |s, i| s.wrapping_add(v).wrapping_add(i))
    });
}

#[test]
fn uint_function_return() {
    uint_case(
        "uint f(uint a) { if (a > 100u) { return a >> 1; } return a * 3u + 1u; }",
        "o_out = f(v);",
        UINTS,
        0,
        |v, _| if v > 100 { v >> 1 } else { v.wrapping_mul(3).wrapping_add(1) },
    );
}

#[test]
fn uint_bitwise() {
    uint_case("", "o_out = ((v & 0xffu) | (v << 3)) ^ (v >> 2) ^ ~v;", UINTS, 0, |v, _| {
        ((v & 0xff) | (v << 3)) ^ (v >> 2) ^ !v
    });
}

#[test]
fn uint_remainder() {
    uint_case("", "o_out = v % 7u + v % 1000u;", UINTS, 0, |v, _| v % 7 + v % 1000);
}

#[test]
fn uint_uniform_plus_varying() {
    uint_case("", "o_out = u * v + u;", UINTS, 0x9e37_79b9, |v, u| u.wrapping_mul(v).wrapping_add(u));
}

// ---- vectors ----

#[test]
fn ivec2_if_else_and_bitwise() {
    let body = "if (v.x > v.y) { o_out = ivec2(v.x - v.y, v.x ^ v.y); } else { o_out = ivec2((v.y << 2) | 1, ~v.x); }";
    let src = fragment("ivec2", "", body);
    for x in [[41i32, -5], [-1, 7], [0, 0], [-100, -200]] {
        let got = run(&src, "ivec2", &x.map(|c| c as u32), 0);
        let want = if x[0] > x[1] { [x[0] - x[1], x[0] ^ x[1]] } else { [(x[1] << 2) | 1, !x[0]] };
        assert_eq!(got, want.map(|c| c as u32), "ivec2 {x:?}");
    }
}

#[test]
fn uvec4_loop_and_bitwise() {
    let body = "uvec4 s = uvec4(0u); for (int i = 0; i < 3; i++) { s = (s << 1) ^ (v + uvec4(uint(i))); } \
                o_out = s | (v >> 4);";
    let src = fragment("uvec4", "", body);
    for x in [[41u32, 0, u32::MAX, 0x8000_0000], [1, 2, 3, 4], [0x7fff_ffff, 9, 0, 65535]] {
        let got = run(&src, "uvec4", &x, 0);
        let want: Vec<u32> =
            x.iter().map(|&v| (0..3u32).fold(0u32, |s, i| (s << 1) ^ v.wrapping_add(i)) | (v >> 4)).collect();
        assert_eq!(got, want, "uvec4 {x:?}");
    }
}
