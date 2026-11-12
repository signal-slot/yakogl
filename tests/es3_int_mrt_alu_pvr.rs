// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! An integer fragment output computed by arithmetic (`o_i = v + 1;`,
//! `o_i = int(v);`) written next to a float `vec4` output in one program
//! (multiple render targets), on the PowerVR online back end: the shape
//! that 89c06d85 (select.rs `StoreOutput`) made build for a single output
//! (tests/es3_int_out_alu_pvr.rs). Shapes: an `int` at location 1 and 0, a
//! `uint` at location 1 and 0, each beside a float output.
//!
//! Compiled under the default variant (key 0), the one a draw into a float
//! attachment next to an integer one would use (`chain::compile_fragment_for`
//! packs every output of an integer variant for one layout, so no variant
//! holds a float and an integer output). The built USC bytes run through the
//! host byte model (tests/usc_exec `run_binary`). HOST-ONLY: no GPU
//! arithmetic and no GL read-back.
//!
//! What holds today: every shape builds, writes pixel outputs 32 and 33
//! once each, and the float output is packed (RGBA8). What does not: the
//! integer output is packed like a colour too (unorm8, clamped to [0, 1]),
//! so its pixel output is 0, not the integer; that test is `#[ignore]`.
//! This is the known gap RC1 in docs/es3-cts-failures.md (mixed integer and
//! float colour attachments). The fix needs back-end work in
//! src/pvr/backend.rs (plan #325); when it lands, the byte-model assertions
//! (`PACKED_F`, pixel outputs [32, 33]) need updating.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_mrt_alu_pvr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    num_varyings: u8,
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
        Ok(Rec {
            num_outputs: rec.num_outputs,
            num_varyings: rec.num_varyings,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
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

/// A fragment shader with one flat varying `v` of type `vty`, a float
/// `vec4` output and an integer output `o_i` of type `ty`, the integer one
/// at location `int_loc` (0 or 1); `body` sets `o_i` from `v`.
fn fragment(vty: &str, ty: &str, int_loc: u32, body: &str) -> String {
    let float_loc = 1 - int_loc;
    format!(
        "#version 300 es\nprecision highp float;\nflat in highp {vty} v;\n\
         layout(location = {float_loc}) out vec4 o_f;\n\
         layout(location = {int_loc}) out highp {ty} o_i;\n\
         void main() {{ o_f = vec4(0.25, 0.5, 0.75, 1.0); {body} }}\n"
    )
}

/// RGBA8 of (0.25, 0.5, 0.75, 1.0), one byte a channel, red lowest.
const PACKED_F: u32 = 0xFFBF_8040;

/// `src` under the default variant, run once with the varying's every
/// coefficient `bits` (a flat varying is its value whichever plane it is
/// read from): the dwords in the float output's and the integer output's
/// pixel outputs.
fn run(src: &str, int_loc: u32, bits: u32) -> (u32, u32) {
    let rec = compile_for(src, 0).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.num_outputs, 2, "{src}: a dword an output");
    assert_eq!(rec.num_varyings, 1, "{src}: the one varying");
    /* pixel output 0 is special register 32 (usc_exec: PIXOUT0) */
    assert_eq!(pixel_outputs(&rec.image), [32, 33], "{src}: pixel outputs 0 and 1, once each");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff.fill(bits);
    let out = usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"));
    let at = |n: u32| *out.get(&n).unwrap_or_else(|| panic!("{src}: nothing in pixel output {n}: {out:?}"));
    (at(1 - int_loc), at(int_loc))
}

/// One shape: the varying's type, the integer output's type and location,
/// the body, the varying's bits and the integer the body computes.
type Shape = (&'static str, &'static str, u32, &'static str, u32, u32);

/// dEQP's `v + 1` and `int(f)`, and a `uint` product, at both locations.
const SHAPES: [Shape; 7] = [
    ("int", "int", 1, "o_i = v + 1;", 41, 42),
    ("float", "int", 1, "o_i = int(v);", 0x4030_0000, 2),
    ("uint", "uint", 1, "o_i = v * 3u;", 7, 21),
    ("float", "uint", 1, "o_i = uint(v);", 0x4079_999A, 3),
    ("int", "int", 0, "o_i = v + 1;", 41, 42),
    ("float", "int", 0, "o_i = int(v);", 0x4030_0000, 2),
    ("uint", "uint", 0, "o_i = v * 3u;", 7, 21),
];

/// Every shape builds, writes both pixel outputs once, and the float output
/// is the packed colour.
#[test]
fn int_alu_next_to_float_builds_and_float_lands() {
    for (vty, ty, loc, body, bits, _) in SHAPES {
        let src = fragment(vty, ty, loc, body);
        let (f, _) = run(&src, loc, bits);
        assert_eq!(f, PACKED_F, "{src}: the float output");
    }
}

/// The integer output holds the integer. NOT TODAY: the default variant
/// packs it as a colour, which is 0 for an integer under 2^23 read as a
/// float.
#[test]
#[ignore = "RC1: integer beside float output is packed as RGBA8 (reads 0); needs src/pvr/backend.rs"]
fn int_alu_next_to_float_lands() {
    for (vty, ty, loc, body, bits, want) in SHAPES {
        let src = fragment(vty, ty, loc, body);
        let (_, i) = run(&src, loc, bits);
        assert_eq!(i, want, "{src}: the integer output");
    }
}
