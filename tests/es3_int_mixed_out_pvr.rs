// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC1 on the PowerVR online back end: a fragment shader drawn into an
//! integer colour attachment beside attachments of another class, compiled
//! under the mixed key (`yakogl::target_key::shc_blend_key_mixed`,
//! docs/notes/pvr-int-mixed-targets.md section 2.2) through the same entry
//! a draw takes (`chain::compile_fragment_for`), which decodes the key into
//! the per-target list and packs each output for its own target.
//!
//! What is checked: the record carries the mixed key, does not read the
//! pixel, and spans the output registers the end-of-tile emit reads
//! (`num_outputs`); each output is written once, at the first register
//! `shc_mixed_out_layout` gives its target; and the built USC bytes, run
//! through the host byte model (tests/usc_exec `run_binary`), leave the
//! expected value in each register. The mixes: RGBA8 + R32UI, the modf
//! executor's two integer attachments of different layouts (highp float as
//! `floatBitsToUint` into R32UI, the whole part into R32I), a three-target
//! mix RGBA8 + RG16UI + R32F, and R32I + RGBA16F (the pair starts on an
//! even register). HOST-ONLY: no GPU arithmetic and no GL read-back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_int_mixed_out_pvr
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, shc_blend_key_rt, ShcIntRt, SHC_RT_F16X4, SHC_RT_F32, SHC_RT_RGBA8};
use yakogl::target_key::{shc_blend_key_mixed, shc_mixed_out_layout};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    blend_key: u32,
    uses_fbfetch: u8,
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
            blend_key: rec.blend_key,
            uses_fbfetch: rec.uses_fbfetch,
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

/// The output registers (0-based) a program writes, sorted: the
/// destinations of the moves into the bank the last operation writes (the
/// program ends on its last pixel output write); pixel output n is special
/// register 32 + n.
fn out_regs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    let mut r: Vec<u32> = ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i - 32)).collect();
    r.sort_unstable();
    r
}

const R32UI: ShcIntRt = ShcIntRt { channels: 1, bits: 32, signed: false };
const R32I: ShcIntRt = ShcIntRt { channels: 1, bits: 32, signed: true };
const RG16UI: ShcIntRt = ShcIntRt { channels: 2, bits: 16, signed: false };

/// RGBA8 of (0.25, 0.5, 0.75, 1.0), one byte a channel, red lowest.
const PACKED_F: u32 = 0xFFBF_8040;

/// `src` under the mixed key of `targets` (per-target keys, draw buffer i
/// written from location i), run once with every varying coefficient
/// `bits` (a flat varying is its value whichever plane it is read from).
/// Checks the record and the registers written against the key's layout and
/// returns the output registers' values.
fn run(src: &str, targets: &[u32], bits: u32) -> std::collections::BTreeMap<u32, u32> {
    let key = shc_blend_key_mixed(targets).unwrap_or_else(|| panic!("{targets:x?} is a mix"));
    let (start, span) = shc_mixed_out_layout(targets).expect("fits");
    let rec = compile_for(src, key).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(rec.blend_key, key, "{src}: the record carries the mixed key");
    assert_eq!(rec.uses_fbfetch, 0, "{src}: no target reads the pixel");
    assert_eq!(rec.num_outputs, span, "{src}: the registers the targets span");
    let mut want: Vec<u32> = Vec::new();
    for (i, &t) in targets.iter().enumerate() {
        let d = yakogl::target_key::shc_target_key_dwords(t).unwrap();
        want.extend((0..d).map(|k| u32::from(start[i]) + k));
    }
    assert_eq!(out_regs(&rec.image), want, "{src}: each target's dwords once, at its registers");
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.coeff.fill(bits);
    usc_exec::run_binary(&rec.image, &inp, true).unwrap_or_else(|e| panic!("{src}: byte model: {e}"))
}

/// RGBA8 + R32UI, the integer at location 1 and at location 0: the colour
/// packed as bytes, the integer as it is.
#[test]
fn rgba8_and_r32ui_hold_their_values() {
    let rgba8 = shc_blend_key_rt(SHC_RT_RGBA8);
    let r32ui = shc_blend_key_int_rt(R32UI);
    for int_loc in [1u32, 0] {
        let f_loc = 1 - int_loc;
        let src = format!(
            "#version 300 es\nprecision highp float;\nflat in highp uint v;\n\
             layout(location = {f_loc}) out vec4 o_f;\n\
             layout(location = {int_loc}) out highp uint o_i;\n\
             void main() {{ o_f = vec4(0.25, 0.5, 0.75, 1.0); o_i = v * 3u; }}\n"
        );
        let targets = if int_loc == 1 { [rgba8, r32ui] } else { [r32ui, rgba8] };
        let out = run(&src, &targets, 0x0102_0304);
        assert_eq!(out.get(&f_loc), Some(&PACKED_F), "{src}: RGBA8 bytes: {out:x?}");
        assert_eq!(out.get(&int_loc), Some(&0x0306_090C), "{src}: R32UI as it is: {out:x?}");
    }
}

/// dEQP's executor for `builtin_functions.common.modf`: the highp float
/// result written as its bits to an R32UI attachment, the whole part (an
/// int) to an R32I one. Two integer attachments of different layouts.
#[test]
fn modf_executor_outputs_hold_their_values() {
    let src = "#version 300 es\nprecision highp float;\nflat in highp float v;\n\
               layout(location = 0) out highp uint o_fract;\n\
               layout(location = 1) out highp int o_whole;\n\
               void main() { float w; float f = modf(v, w); \
               o_fract = floatBitsToUint(f); o_whole = int(w); }\n";
    let targets = [shc_blend_key_int_rt(R32UI), shc_blend_key_int_rt(R32I)];
    let out = run(src, &targets, (-2.75f32).to_bits());
    assert_eq!(out.get(&0), Some(&(-0.75f32).to_bits()), "fract's bits: {out:x?}");
    assert_eq!(out.get(&1), Some(&(-2i32 as u32)), "the whole part: {out:x?}");
}

/// Three targets of three classes: RGBA8, RG16UI (two channels in one
/// dword, red low) and R32F, in registers 0, 1, 2.
#[test]
fn rgba8_rg16ui_r32f_hold_their_values() {
    let src = "#version 300 es\nprecision highp float;\nflat in highp uint v;\n\
               layout(location = 0) out vec4 o0;\n\
               layout(location = 1) out highp uvec2 o1;\n\
               layout(location = 2) out highp float o2;\n\
               void main() { o0 = vec4(0.25, 0.5, 0.75, 1.0); o1 = uvec2(v, v + 0x1000u); o2 = 0.5; }\n";
    let targets = [shc_blend_key_rt(SHC_RT_RGBA8), shc_blend_key_int_rt(RG16UI), shc_blend_key_rt(SHC_RT_F32)];
    assert_eq!(shc_mixed_out_layout(&targets), Some(([0, 1, 2], 3)));
    let out = run(src, &targets, 0x234);
    assert_eq!(out.get(&0), Some(&PACKED_F), "RGBA8 bytes: {out:x?}");
    assert_eq!(out.get(&1), Some(&0x1234_0234), "RG16UI, red low: {out:x?}");
    assert_eq!(out.get(&2), Some(&0.5f32.to_bits()), "R32F as it is: {out:x?}");
}

/// R32I + RGBA16F: the pair starts on an even register, so register 1 is
/// skipped and the halves land in 2 and 3.
#[test]
fn r32i_and_rgba16f_pair_skip_a_register() {
    let src = "#version 300 es\nprecision highp float;\nflat in highp int v;\n\
               layout(location = 0) out highp int o0;\n\
               layout(location = 1) out vec4 o1;\n\
               void main() { o0 = v - 50; o1 = vec4(0.5, 1.0, 2.0, -1.0); }\n";
    let targets = [shc_blend_key_int_rt(R32I), shc_blend_key_rt(SHC_RT_F16X4)];
    assert_eq!(shc_mixed_out_layout(&targets), Some(([0, 2, 0], 4)));
    let out = run(src, &targets, 8);
    assert_eq!(out.get(&0), Some(&(-42i32 as u32)), "R32I as it is: {out:x?}");
    /* halves of 0.5, 1.0, 2.0, -1.0: 0x3800, 0x3C00, 0x4000, 0xBC00 */
    assert_eq!(out.get(&2), Some(&0x3C00_3800), "red and green halves: {out:x?}");
    assert_eq!(out.get(&3), Some(&0xBC00_4000), "blue and alpha halves: {out:x?}");
}
