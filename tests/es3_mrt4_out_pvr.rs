// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 on the PowerVR BXS-4-64: where the online compiler puts a
//! four-output fragment shader's colours, against `pvr::mrt_layout`
//! (`register_layout`: four 32-bit output registers a pixel, one for an
//! RGBA8-class target, an even pair for a 64-bit one, no back-fill, a
//! target past the fourth register refused by name).
//!
//! The compiler is given the variant a draw of that layout compiles under
//! (key 0 for RGBA8 attachments; the per-target list, `TargetPack`, for
//! attachments of different classes) and the bytes run through the host
//! byte model (tests/usc_exec). Pixel output n is special register 32 + n,
//! so a shader that writes output register r writes 32 + r. The cases: four
//! RGBA8 outputs (o0-o3), a shader that writes only location 3, RGBA8 x2 +
//! RGBA16F (a pair), four targets of different classes, and a layout that
//! does not fit (refused by the layout and by the compiler, never packed
//! past register 3). HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_out_pvr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{register_layout, LayoutError};
use yakogl::shaderc::online::powervr::chain::{compile_fragment_for, compile_fragment_targets, TargetPack};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{ShcIntRt, SHC_RT_F16X4, SHC_RT_F32, SHC_RT_RGBA8};

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
    compile_with(src, Err(key))
}

/// One fragment shader through the chain for the per-target list `targets`
/// (draw buffer i written from location i, each packed for its own target).
fn compile_targets(src: &str, targets: &[TargetPack]) -> Result<Rec, String> {
    compile_with(src, Ok(targets))
}

fn compile_with(src: &str, variant: Result<&[TargetPack], u32>) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = match variant {
            Ok(t) => compile_fragment_targets(&l.module, &l.reflection, a, t),
            Err(key) => compile_fragment_for(&l.module, &l.reflection, a, key),
        }
        .map_err(|e| format!("chain: {e:?}"))?;
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
/// 2..4). One target key names one class for every output, so the variant
/// that packs only the third output as halves is the per-target list.
#[test]
fn rgba8_x2_and_rgba16f_pair_land_in_0_1_2() {
    let src = fragment(&[(0, VEC4), (1, VEC4), (2, VEC4)]);
    let want = want_starts(&[1, 1, 2]);
    assert_eq!(want, [0, 1, 2]);
    let t = [TargetPack::Class(SHC_RT_RGBA8), TargetPack::Class(SHC_RT_RGBA8), TargetPack::Class(SHC_RT_F16X4)];
    let rec = compile_targets(&src, &t).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rec.num_outputs, 4, "two RGBA8 dwords and a pair");
    assert_eq!(out_regs(&rec.image), [0, 1, 2, 3], "o2's halves in the pair 2, 3");
    let out = usc_exec::run_binary(&rec.image, &usc_exec::sentinel_inputs(1.0e30), true).expect("runs");
    /* o2 = vec4(0.3): four equal halves, two to a dword */
    let h = u32::from(half_bits(0.3));
    assert_eq!(out.get(&2), Some(&(h | h << 16)), "{out:x?}");
    assert_eq!(out.get(&3), Some(&(h | h << 16)), "{out:x?}");
}

/// Four targets of different classes, one dword each (RGBA8, R32UI, R32F,
/// RGBA8): layout [1,1,1,1], o0-o3 in registers 0..3.
#[test]
fn four_targets_of_different_classes_land_in_0_to_3() {
    let src = "#version 300 es\nprecision highp float;\n\
               layout(location = 0) out vec4 o0;\nlayout(location = 1) out highp uint o1;\n\
               layout(location = 2) out float o2;\nlayout(location = 3) out vec4 o3;\n\
               void main() { o0 = vec4(0.1); o1 = 7u; o2 = 0.5; o3 = vec4(0.4); }\n";
    let r32ui = ShcIntRt { channels: 1, bits: 32, signed: false };
    let t = [
        TargetPack::Class(SHC_RT_RGBA8),
        TargetPack::Int(r32ui),
        TargetPack::Class(SHC_RT_F32),
        TargetPack::Class(SHC_RT_RGBA8),
    ];
    let want = want_starts(&[1, 1, 1, 1]);
    let rec = compile_targets(src, &t).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rec.num_outputs, 4);
    assert_eq!(out_regs(&rec.image), want);
    let out = usc_exec::run_binary(&rec.image, &usc_exec::sentinel_inputs(1.0e30), true).expect("runs");
    assert_eq!(out.get(&1), Some(&7), "R32UI as it is: {out:x?}");
    assert_eq!(out.get(&2), Some(&0.5f32.to_bits()), "R32F as it is: {out:x?}");
    assert_ne!(out.get(&0), out.get(&3), "o0 and o3 their own colours: {out:x?}");
}

/// IEEE half of a value exact enough for the test's constants (normal, no
/// rounding past the tie).
fn half_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = (b >> 16) & 0x8000;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = b & 0x7f_ffff;
    let mut h = (exp as u32) << 10 | man >> 13;
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && h & 1 != 0) {
        h += 1;
    }
    (sign | h) as u16
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
