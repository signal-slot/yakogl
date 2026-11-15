// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 on the PowerVR BXS-4-64: where the online compiler puts a
//! four-output fragment shader's colours, against `pvr::mrt_layout`
//! (`register_layout`: four 32-bit output registers a pixel, one for an
//! RGBA8-class target, an even pair for a 64-bit one, no back-fill, a
//! target past the fourth register refused by name).
//!
//! The compiler is given the variant a draw of that layout compiles under
//! (key 0 for RGBA8 attachments) and the bytes run through the host byte
//! model (tests/usc_exec). Pixel output n is special register 32 + n, so a
//! shader that writes output register r writes 32 + r. The cases: four
//! RGBA8 outputs (o0-o3), a shader that writes only location 3, RGBA8 x2 +
//! RGBA16F (a pair), and a layout that does not fit (refused by the layout
//! and by the compiler, never packed past register 3). HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_out_pvr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{register_layout, LayoutError};
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_rt, SHC_RT_F16X4};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// The output registers (0-based) a program writes, in order: the
/// destinations of the moves into the bank the last operation writes (the
/// program ends on its last pixel output write); pixel output n is special
/// register 32 + n.
fn out_regs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i - 32)).collect()
}

/// `outs`: (location, GLSL type) per output, each written its own constant.
fn fragment(outs: &[(u32, &str)]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\n");
    for (loc, ty) in outs {
        s += &format!("layout(location = {loc}) out highp {ty} o{loc};\n");
    }
    s += "void main() {\n";
    for (loc, ty) in outs {
        let v = 0.1 * (*loc as f32 + 1.0);
        s += &format!("  o{loc} = {ty}({v:?});\n");
    }
    s + "}\n"
}

/// The PBE source positions `register_layout` gives targets of `dwords`.
fn want_starts(dwords: &[u8]) -> Vec<u32> {
    let l = register_layout(dwords).expect("fits");
    assert_eq!(l.tile_buffers, 0);
    (0..dwords.len()).map(|i| u32::from(l.place[i].source_pos())).collect()
}

const VEC4: &str = "vec4";

#[test]
fn four_rgba8_outputs_land_in_registers_0_to_3() {
    let src = fragment(&[(0, VEC4), (1, VEC4), (2, VEC4), (3, VEC4)]);
    let rec = compile_for(&src, 0).unwrap_or_else(|e| panic!("{e}"));
    let want = want_starts(&[1, 1, 1, 1]);
    assert_eq!(want, [0, 1, 2, 3]);
    assert_eq!(rec.num_outputs, 4, "the record's output count is the layout's output_regs");
    assert_eq!(out_regs(&rec.image), want, "each output once, in its register");
}

#[test]
fn each_output_carries_its_own_colour() {
    let src = fragment(&[(0, VEC4), (1, VEC4), (2, VEC4), (3, VEC4)]);
    let rec = compile_for(&src, 0).unwrap_or_else(|e| panic!("{e}"));
    let out = usc_exec::run_binary(&rec.image, &usc_exec::sentinel_inputs(1.0e30), true).expect("runs");
    let vals: Vec<u32> = (0..4).map(|n| *out.get(&n).unwrap_or_else(|| panic!("no output {n}: {out:?}"))).collect();
    for i in 0..4 {
        for j in i + 1..4 {
            assert_ne!(vals[i], vals[j], "registers {i} and {j} hold different colours: {vals:x?}");
        }
    }
}

#[test]
fn only_location_3_writes_register_3() {
    let src = fragment(&[(3, VEC4)]);
    let rec = compile_for(&src, 0).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rec.num_outputs, 4, "the record spans registers 0..=3");
    assert_eq!(out_regs(&rec.image), [3], "o3 alone, in register 3");
}

/// RGBA8 x2 + RGBA16F: layout [1,1,2] gives registers 0, 1, 2 (a pair at
/// 2..4); the variant is the F16X4 key of the pair target.
#[test]
#[ignore = "pvr-shout4: no variant packs one of several outputs as RGBA16F; the F16X4 key is read as a blend (blend.rs:85 via chain.rs:714)"]
fn rgba8_x2_and_rgba16f_pair_land_in_0_1_2() {
    let src = fragment(&[(0, VEC4), (1, VEC4), (2, VEC4)]);
    let want = want_starts(&[1, 1, 2]);
    assert_eq!(want, [0, 1, 2]);
    let rec = compile_for(&src, shc_blend_key_rt(SHC_RT_F16X4)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rec.num_outputs, 4, "two RGBA8 dwords and a pair");
    assert_eq!(&out_regs(&rec.image)[..3], &[0, 1, 2][..]);
}

/// What does hold today for [1,1,2]: under key 0 the three outputs are
/// written to registers 0, 1, 2 (each as RGBA8; the pair is not packed).
#[test]
fn three_outputs_under_key_0_start_at_0_1_2() {
    let src = fragment(&[(0, VEC4), (1, VEC4), (2, VEC4)]);
    let rec = compile_for(&src, 0).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(out_regs(&rec.image), want_starts(&[1, 1, 2]));
}

/// A layout that does not fit is refused by name, and the compiler does not
/// pack a fifth output into the four registers.
#[test]
fn a_layout_past_four_registers_is_refused() {
    assert_eq!(
        register_layout(&[1, 1, 1, 2]).map(|_| ()),
        Err(LayoutError::OutputRegs { target: 3, first: 4, dwords: 2 })
    );
    assert_eq!(
        register_layout(&[2, 2, 1, 1]).map(|_| ()),
        Err(LayoutError::OutputRegs { target: 2, first: 4, dwords: 1 })
    );
    let src = fragment(&[(0, VEC4), (1, VEC4), (2, VEC4), (3, VEC4), (4, VEC4)]);
    let e = compile_for(&src, 0).err().expect("five outputs are refused");
    eprintln!("refusal: {e}");
}
