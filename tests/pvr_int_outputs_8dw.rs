// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: a fragment shader with two RGBA32UI outputs, eight dwords,
//! writes both, so dEQP-GLES3's `builtin_functions.common.modf` vec3/vec4
//! cases (the shader executor's two RGBA32UI attachments) are drawn and
//! carry both results (issue #313).
//!
//! A BXS-4-64 pixel has four 32-bit output registers. Under the integer
//! variant of RGBA32UI (`shc_blend_key_int_rt`) the program writes output 0
//! to the four registers and stores output 1's four dwords to tile buffer 0
//! (`st.tiled`, the pixel's `tiled_st_comp` offsets 58..61); the end-of-tile
//! program `eot2_tb1` emits attachment 1 from that buffer
//! (`pvr::mrt_layout::layout`). What is checked, for constants, for a flat
//! integer varying and for dEQP's modf executor at three precisions:
//!
//! - the record: four output registers and one tile buffer, the layout's
//!   key, and the back end draws two RGBA32UI attachments with it
//!   (`int_targets_drawn`) and with no other record;
//! - the values of BOTH outputs, through the host byte model
//!   (tests/usc_exec). The model has no tile-buffer store, so the program
//!   is run twice: once without its stores (the registers: output 0), and
//!   once with each `st.tiled` of dword k turned into a write of pixel
//!   output register k and its own register writes left out (output 1).
//!   Each must equal what the same expression gives as a program's only
//!   RGBA32UI output, run through the model with the same inputs.
//!
//! HOST-ONLY: no GPU runs and no pixel is read back. The draw through the
//! GL core and the end-of-tile emit are tests/pvr_int_spill.rs's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_int_outputs_8dw
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use std::collections::BTreeMap;

use yakogl::backend::{GlesDraw, GlesDrawTarget, GLES_SURF_RGBA32UI};
use yakogl::pvr::backend::int_targets_drawn;
use yakogl::pvr::mrt_layout::{layout, EotProgram};
use yakogl::shaderc::online::powervr::assemble::{build_pixout_write_to, tiled_st_comp};
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::powervr::decode::{decode_group, Group};
use yakogl::shaderc::online::powervr::encode::encode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::{dsts, srcs};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt, ShcShader};

mod usc_exec;

/* `regbank` (isa_tables.rs ENUMS) */
const SPECIAL: u32 = 0;
const TEMP: u32 = 1;
/* PIXOUT0 is special register 32 */
const PIXOUT0: u32 = 32;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    num_tile_buffers: u8,
    blend_key: u32,
    image: Vec<u8>,
}

/// The integer variant of RGBA32UI.
fn rgba32ui_key() -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels: 4, bits: 32, signed: false })
}

/// One fragment shader through the chain under the RGBA32UI variant.
fn compile(src: &str) -> Rec {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, rgba32ui_key()).map_err(|e| format!("{e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok::<_, String>(Rec {
            num_outputs: rec.num_outputs,
            num_tile_buffers: rec.num_tile_buffers,
            blend_key: rec.blend_key,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
    })
    .unwrap()
    .unwrap_or_else(|e| panic!("compile: {e}\n{src}"))
}

/// The program's groups with their byte offsets, in order.
fn groups(image: &[u8]) -> Vec<(usize, Group)> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        let t = g.total;
        out.push((off, g));
        off += t;
    }
    out
}

fn names(g: &Group) -> Vec<&'static str> {
    g.instrs().filter_map(|(_, ph, r)| ops_for(r, ph).next().map(|o| o.name)).collect()
}

/// A group's first source (s0..s5) of each slot, merged.
fn group_srcs(g: &Group) -> [Option<yakogl::shaderc::online::powervr::operands::Operand>; 6] {
    let mut out = [None; 6];
    for r in [g.lower.as_ref(), g.upper.as_ref()].into_iter().flatten() {
        for (o, s) in out.iter_mut().zip(srcs(r)) {
            if s.is_some() {
                *o = s;
            }
        }
    }
    out
}

fn first_dst(g: &Group) -> Option<yakogl::shaderc::online::powervr::operands::Operand> {
    g.dst.as_ref().and_then(|d| dsts(d)[0])
}

/// A move into pixel output register 0..3.
fn is_pixout_write(g: &Group) -> bool {
    names(g) == ["mbyp"] && first_dst(g).is_some_and(|d| d.bank == SPECIAL && (PIXOUT0..PIXOUT0 + 4).contains(&d.index))
}

/// The tile-buffer stores: (group index, stored temporary, the dword of the
/// pixel's tile-buffer entry it goes to). The dword is read from the
/// `add64_32` that made the store's address: its offset is the special
/// register `tiled_st_comp(k)`.
fn stores(gs: &[(usize, Group)]) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    for (i, (_, g)) in gs.iter().enumerate() {
        if names(g) != ["st.tiled"] {
            continue;
        }
        let s = group_srcs(g);
        let (data, addr) = (s[0].expect("data in s0"), s[3].expect("address in s3"));
        assert_eq!(data.bank, TEMP, "a store from a temporary");
        let add = gs[..i]
            .iter()
            .rev()
            .map(|(_, g)| g)
            .find(|g| names(g) == ["add64_32"] && first_dst(g) == Some(addr))
            .expect("the address's add64_32");
        let off = group_srcs(add)[2].expect("the offset");
        assert_eq!(off.bank, SPECIAL);
        let k = (0..8).find(|&k| tiled_st_comp(k) as u32 == off.index).expect("a tiled_st_comp offset");
        out.push((i, data.index, k as u32));
    }
    out
}

/// The program's store block: from the first group after the last pixel
/// output write (the two sample masks `savmsk` reads, their `csel`, then a
/// buffer address `add64_32`, an `st.tiled` and its `wdf` a dword) to the
/// end. Nothing in it but the four stores' data comes from before it, and
/// nothing after it but the `nop` that ends the program.
fn store_block(gs: &[(usize, Group)]) -> usize {
    let last_write = gs.iter().rposition(|(_, g)| is_pixout_write(g)).expect("a pixel output write");
    let first = last_write + 1;
    assert_eq!(names(&gs[first].1), ["savmsk"], "the store block starts with the sample mask");
    for (_, g) in &gs[first..] {
        let n = names(g);
        assert!(!n.iter().any(|n| n.starts_with("br")), "no branch in the store block: {n:?}");
        assert!(!is_pixout_write(g), "no pixel output write in the store block");
    }
    first
}

/// The program's groups before `cut`, the pixel output writes among them
/// left out when `keep_writes` is false, then `append`: an image the byte
/// model runs. No branch is at or after the first pixel output write, so
/// leaving the writes out moves no branch's way.
fn prefix(image: &[u8], gs: &[(usize, Group)], cut: usize, keep_writes: bool, append: &[Group]) -> Vec<u8> {
    let first_write = gs.iter().position(|(_, g)| is_pixout_write(g)).expect("a pixel output write");
    assert!(
        gs[first_write..].iter().all(|(_, g)| !names(g).iter().any(|n| n.starts_with("br"))),
        "no branch after the first pixel output write"
    );
    let mut out = Vec::new();
    for (off, g) in &gs[..cut] {
        if keep_writes || !is_pixout_write(g) {
            out.extend_from_slice(&image[*off..*off + g.total]);
        }
    }
    for g in append {
        let mut buf = vec![0u8; g.total];
        encode_group(g, &mut buf, 0).expect("encodes");
        out.extend_from_slice(&buf);
    }
    out
}

/// Output 0 (the registers) and output 1 (the tile buffer) of a program of
/// two RGBA32UI outputs, run through the byte model on `inp`.
fn both_outputs(rec: &Rec, inp: &usc_exec::Inputs) -> ([u32; 4], [u32; 4]) {
    let gs = groups(&rec.image);
    let st = stores(&gs);
    assert_eq!(st.iter().map(|s| s.2).collect::<Vec<_>>(), [0, 1, 2, 3], "output 1's four dwords, in order");
    let writes: Vec<u32> =
        gs.iter().filter(|(_, g)| is_pixout_write(g)).map(|(_, g)| first_dst(g).unwrap().index - PIXOUT0).collect();
    assert_eq!(writes, [0, 1, 2, 3], "output 0 in the four registers");
    let cut = store_block(&gs);
    assert!(st.iter().all(|s| s.0 > cut), "the stores are in the store block");
    let tail: Vec<&str> = gs[cut..].iter().flat_map(|(_, g)| names(g)).collect();
    assert_eq!(tail.last(), Some(&"nop"), "the stores' nop ends the program");
    /* the store data is not written inside the block */
    for (_, g) in &gs[cut..] {
        if let Some(d) = first_dst(g) {
            assert!(!(d.bank == TEMP && st.iter().any(|s| s.1 == d.index)), "stored temp {} rewritten", d.index);
        }
    }
    let collect = |out: BTreeMap<u32, u32>, what: &str| -> [u32; 4] {
        assert_eq!(out.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3], "{what}: {out:x?}");
        [out[&0], out[&1], out[&2], out[&3]]
    };
    /* registers: the program up to its store block */
    let a = prefix(&rec.image, &gs, cut, true, &[]);
    let regs = usc_exec::run_binary(&a, inp, true).unwrap_or_else(|e| panic!("registers run: {e}"));
    /* tile buffer: the register writes left out, and each store's data
     * written to the pixel output register of its dword */
    let moves: Vec<Group> = st
        .iter()
        .map(|&(_, data, k)| build_pixout_write_to(k as u16, data as u16, false).expect("a pixel output write"))
        .collect();
    let b = prefix(&rec.image, &gs, cut, false, &moves);
    let tb = usc_exec::run_binary(&b, inp, true).unwrap_or_else(|e| panic!("tile buffer run: {e}"));
    (collect(regs, "registers"), collect(tb, "tile buffer"))
}

/// The four dwords a program of one RGBA32UI output writes on `inp`.
fn one_output(src: &str, inp: &usc_exec::Inputs) -> [u32; 4] {
    let rec = compile(src);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 0), "one RGBA32UI output in the registers");
    let out = usc_exec::run_binary(&rec.image, inp, true).unwrap_or_else(|e| panic!("one output: {e}\n{src}"));
    assert_eq!(out.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3], "{out:x?}");
    [out[&0], out[&1], out[&2], out[&3]]
}

/// The draw dEQP's executor makes: two RGBA32UI attachments with `fs`.
fn two_rgba32ui(fs: &ShcShader) -> GlesDraw {
    let t = GlesDrawTarget { gpu: 0x10_0000, w: 64, h: 1, format: GLES_SURF_RGBA32UI, out_shift: 0 };
    let mut d =
        GlesDraw { target_gpu: t.gpu, target_w: 64, target_h: 1, target_format: GLES_SURF_RGBA32UI, ..GlesDraw::ZERO };
    d.targets[0] = t;
    d.targets[1] = GlesDrawTarget { gpu: t.gpu + 0x1_0000, ..t };
    d.num_targets = 2;
    d.fs = fs;
    d
}

/// The record says r0-r3 and tile buffer 0 under the RGBA32UI key, and the
/// back end draws the two attachments with it, `eot2_tb1` emitting them.
fn assert_drawn(rec: &Rec, what: &str) {
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 1), "{what}: r0-r3 and tile buffer 0");
    assert_eq!(rec.blend_key, rgba32ui_key(), "{what}: the variant's key");
    let l = layout(&[4, 4]).expect("two RGBA32UI targets lay out");
    assert_eq!((l.output_regs, l.tile_buffers, l.program), (4, 1, EotProgram::Eot2Tb1));
    let fs = ShcShader {
        blend_key: rec.blend_key,
        num_outputs: rec.num_outputs,
        num_tile_buffers: rec.num_tile_buffers,
        ..ShcShader::ZERO
    };
    assert_eq!(int_targets_drawn(&two_rgba32ui(&fs), Some(&fs)), Ok(()), "{what}: drawn");
    let eight = ShcShader { num_outputs: 8, num_tile_buffers: 0, ..fs };
    assert!(int_targets_drawn(&two_rgba32ui(&eight), Some(&eight)).is_err(), "{what}: never eight registers");
}

/// Two outputs of distinct constants: each lands where its attachment is
/// emitted from, every dword its own.
#[test]
fn two_constant_rgba32ui_outputs_write_both() {
    let src = "#version 300 es\n\
               layout(location = 0) out highp uvec4 o0;\n\
               layout(location = 1) out highp uvec4 o1;\n\
               void main() {\n\
               \to0 = uvec4(1u, 2u, 0x80000003u, 4u);\n\
               \to1 = uvec4(0x50607080u, 6u, 0xdeadbeefu, 0xffffffffu);\n\
               }\n";
    let rec = compile(src);
    assert_drawn(&rec, "constants");
    let (r, t) = both_outputs(&rec, &usc_exec::sentinel_inputs(1.0e30));
    assert_eq!(r, [1, 2, 0x8000_0003, 4], "output 0");
    assert_eq!(t, [0x5060_7080, 6, 0xdead_beef, 0xffff_ffff], "output 1");
}

/// Both outputs from a flat integer varying, each a different function of
/// it: the two equal what each expression gives alone, on several inputs.
#[test]
fn two_rgba32ui_outputs_of_a_varying_write_both() {
    let head = "#version 300 es\nflat in highp uvec4 a;\n";
    let e0 = "a.wzyx + uvec4(1u, 2u, 3u, 4u)";
    let e1 = "a ^ uvec4(0xff00ff00u)";
    let two = format!(
        "{head}layout(location = 0) out highp uvec4 o0;\nlayout(location = 1) out highp uvec4 o1;\n\
         void main() {{ o0 = {e0}; o1 = {e1}; }}\n"
    );
    let alone = |e: &str| format!("{head}layout(location = 0) out highp uvec4 o0;\nvoid main() {{ o0 = {e}; }}\n");
    let rec = compile(&two);
    assert_drawn(&rec, "varying");
    for seed in 0..4 {
        let inp = usc_exec::Inputs::new(seed);
        let (r, t) = both_outputs(&rec, &inp);
        assert_eq!(r, one_output(&alone(e0), &inp), "seed {seed}: output 0");
        assert_eq!(t, one_output(&alone(e1), &inp), "seed {seed}: output 1");
        assert_ne!(r, t, "seed {seed}: two different results");
    }
}

/// dEQP's fragment executor for `modf` of `ty` at `prec`
/// (`glsShaderExecUtil.cpp`, VK-GL-CTS, Apache-2.0, Copyright (c) 2014 The
/// Android Open Source Project): both results as their bits, to outputs 0
/// and 1; with `only`, the one output alone at location 0.
fn modf_fragment(ty: &str, prec: &str, only: Option<u8>) -> String {
    let uty = if ty == "vec3" { "uvec3" } else { "uvec4" };
    let outs = match only {
        None => format!("layout(location = 0) out highp {uty} o_out0;\nlayout(location = 1) out highp {uty} o_out1;\n"),
        Some(k) => format!("layout(location = 0) out highp {uty} o_out{k};\n"),
    };
    let writes = match only {
        None => "\to_out0 = floatBitsToUint(out0);\n\to_out1 = floatBitsToUint(out1);\n".to_string(),
        Some(k) => format!("\to_out{k} = floatBitsToUint(out{k});\n"),
    };
    format!(
        "#version 300 es\n\
         in {prec} {ty} vtx_out_in0;\n\
         {outs}\
         void main (void)\n{{\n\
         \t{prec} {ty} in0 = vtx_out_in0;\n\
         \t{prec} {ty} out0;\n\
         \t{prec} {ty} out1;\n\
         \tout0 = modf(in0, out1);\n\
         {writes}\
         }}\n"
    )
}

/// `common.modf` vec3 and vec4 at every precision: the fraction in the
/// registers and the whole part in tile buffer 0, each what the executor
/// writes when it has that output alone, and the fraction plus the whole
/// part is the input.
#[test]
fn modf_vec3_and_vec4_write_fraction_and_whole_part() {
    for prec in ["lowp", "mediump", "highp"] {
        for ty in ["vec3", "vec4"] {
            let what = format!("{ty} {prec}");
            let rec = compile(&modf_fragment(ty, prec, None));
            assert_drawn(&rec, &what);
            for seed in 0..3 {
                let inp = usc_exec::Inputs::new(seed);
                let (r, t) = both_outputs(&rec, &inp);
                assert_eq!(r, one_output(&modf_fragment(ty, prec, Some(0)), &inp), "{what} seed {seed}: fraction");
                assert_eq!(t, one_output(&modf_fragment(ty, prec, Some(1)), &inp), "{what} seed {seed}: whole");
                let n = if ty == "vec3" { 3 } else { 4 };
                for c in 0..n {
                    let (f, w) = (f32::from_bits(r[c]), f32::from_bits(t[c]));
                    assert_eq!(w, w.trunc(), "{what} seed {seed} [{c}]: the whole part is whole");
                    assert!(f.abs() < 1.0, "{what} seed {seed} [{c}]: |fraction| < 1, {f}");
                    assert!(f == 0.0 || w == 0.0 || f.is_sign_negative() == w.is_sign_negative(), "{what} [{c}]");
                }
            }
        }
    }
}
