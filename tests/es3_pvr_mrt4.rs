// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 step 2 on the PowerVR BXS-4-64 (plan #325 item mrt-compiler): the
//! online compiler writes up to four colour outputs, each packed for its own
//! target and placed where `pvr::mrt_layout::layout` puts it -- the target's
//! output registers (one dword for the 8-bit, R16F, R32F and 32-bit integer
//! classes, an even pair for RGBA16F and RGBA16I/UI, four for RGBA32I/UI), or,
//! for the first target that does not fit and every one after it, a tile
//! buffer the end-of-tile program loads back (`eot3_tb`, `eot4_tb`,
//! `eot4_tb2`, `eot4_tb3`). An integer target beside a float one included.
//!
//! What is checked:
//! * the compiler's copy of the layout walk (`chain::target_layout`) is
//!   `mrt_layout::layout` for every list of up to four targets;
//! * the tile-buffer store groups (`add64_32`, `savmsk.icm`, the `csel` of
//!   the two masks, `st.tiled`) are Mesa 26.2.1 pco's bytes, read from
//!   `pco_shc` for three RGBA16F outputs, where the third spills
//!   (`PCO_SHC_OUT=f16x4`, the groups at 0x106..0x16e of its program);
//! * programs of 2, 3 and 4 outputs (vec4, ivec4 and uvec4, an integer
//!   target beside a float one, highp vec4 into RGBA16F): each register
//!   target's dwords in its registers, run through the host byte model
//!   (tests/usc_exec) where it has no tile-buffer store, and each spilled
//!   target's dwords stored by `st.tiled` from its packed temporaries to the
//!   address `add64_32` makes of its buffer's shared pair and the pixel's
//!   `tiled_st_comp` offset, the record naming the buffers
//!   (`num_tile_buffers`, `tile_buffer_sh`) after everything else the program
//!   reads from the shared file;
//! * a written gl_FragDepth and a discard beside several targets.
//!
//! HOST-ONLY: no picture; the back end's side (PBE sources, end-of-tile
//! program per layout) is items mrt-backend and mrt-frameload.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_mrt4
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::pvr::mrt_layout::{layout, LayoutError, Place};
use yakogl::shaderc::online::powervr::assemble::{
    build_add64_32, build_group, build_st_tiled, tiled_st_comp, CSEL_GZ, SAVMSK_ICM,
};
use yakogl::shaderc::online::powervr::chain::{
    compile_fragment_for, compile_fragment_targets, target_layout, TargetPack, TargetSlot,
};
use yakogl::shaderc::online::powervr::decode::{decode_group, Group};
use yakogl::shaderc::online::powervr::encode::encode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::{dsts, srcs, Operand};
use yakogl::shaderc::online::powervr::sel::{Bank, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc};
use yakogl::shaderc::online::powervr::shapes::build_opt;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{ShcIntRt, SHC_RT_F16, SHC_RT_F16X4, SHC_RT_F32, SHC_RT_RGBA8};

mod usc_exec;

/* `regbank` (isa_tables.rs ENUMS) */
const SPECIAL: u32 = 0;
const TEMP: u32 = 1;
const SHARED: u32 = 4;

fn op(bank: u32, index: u32) -> Operand {
    Operand { bank, index }
}

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug)]
struct Rec {
    num_outputs: u8,
    num_tile_buffers: u8,
    tile_buffer_sh: u16,
    uniform_dwords: u16,
    writes_depth: u8,
    image: Vec<u8>,
}

/// One fragment shader through the chain, for the per-target list `targets`
/// (draw buffer i written from location i), or under the variant `key`.
fn compile(src: &str, variant: Result<&[TargetPack], u32>) -> Result<Rec, String> {
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
        Ok(Rec {
            num_outputs: rec.num_outputs,
            num_tile_buffers: rec.num_tile_buffers,
            tile_buffer_sh: rec.tile_buffer_sh,
            uniform_dwords: rec.uniform_dwords,
            writes_depth: rec.writes_depth,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
    })
    .unwrap()
}

fn compile_targets(src: &str, targets: &[TargetPack]) -> Rec {
    compile(src, Ok(targets)).unwrap_or_else(|e| panic!("{targets:?}: {e}"))
}

/// The program's groups, in order.
fn groups(image: &[u8]) -> Vec<Group> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        off += g.total;
        out.push(g);
    }
    out
}

/// The operation names of a group.
fn names(g: &Group) -> Vec<&'static str> {
    g.instrs().filter_map(|(_, ph, r)| ops_for(r, ph).next().map(|o| o.name)).collect()
}

/// A group's sources, s0..s5.
fn group_srcs(g: &Group) -> [Option<Operand>; 6] {
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

fn group_dsts(g: &Group) -> [Option<Operand>; 2] {
    g.dst.as_ref().map(dsts).unwrap_or([None; 2])
}

/// The pixel output registers written, in order: (register, source temp).
fn pixout_writes(image: &[u8]) -> Vec<(u32, u32)> {
    groups(image)
        .iter()
        .filter(|g| names(g) == ["mbyp"])
        .filter_map(|g| {
            let d = group_dsts(g)[0]?;
            let s = group_srcs(g)[0]?;
            (d.bank == SPECIAL && (32..36).contains(&d.index) && s.bank == TEMP).then(|| (d.index - 32, s.index))
        })
        .collect()
}

/// One tile-buffer store: the stored temp, the buffer's shared pair and the
/// `tiled_st_comp` offset its address was made from.
#[derive(Debug, PartialEq, Eq)]
struct Store {
    data: u32,
    sh: u32,
    comp: u32,
}

/// The program's tile-buffer stores, each traced back to the `add64_32`
/// that made its address and the move that put the buffer's high dword in
/// a temporary; and that a `wdf` follows each store.
fn tile_stores(image: &[u8]) -> Vec<Store> {
    let gs = groups(image);
    let mut out = Vec::new();
    for (i, g) in gs.iter().enumerate() {
        if names(g) != ["st.tiled"] {
            continue;
        }
        let s = group_srcs(g);
        let (data, addr) = (s[0].expect("data in s0"), s[3].expect("address in s3"));
        assert!(s[4].is_some(), "a sample mask in s4");
        assert_eq!(names(&gs[i + 1]), ["wdf"], "a wdf drains each store");
        /* the add64_32 writing the address pair, and its operands */
        let add = gs[..i]
            .iter()
            .rev()
            .find(|g| names(g) == ["add64_32"] && group_dsts(g)[0] == Some(addr))
            .expect("the address's add64_32");
        assert_eq!(group_dsts(add)[1], Some(op(addr.bank, addr.index + 1)), "the pair's high dword");
        let a = group_srcs(add);
        let (lo, hi, off) = (a[0].unwrap(), a[1].unwrap(), a[2].unwrap());
        assert_eq!((lo.bank, off.bank), (SHARED, SPECIAL));
        let mov = gs[..i]
            .iter()
            .rev()
            .find(|g| names(g) == ["mbyp"] && group_dsts(g)[0] == Some(hi))
            .expect("the high dword's move");
        assert_eq!(group_srcs(mov)[0], Some(op(SHARED, lo.index + 1)), "the buffer's high dword");
        assert_eq!(data.bank, TEMP);
        out.push(Store { data: data.index, sh: lo.index, comp: off.index });
    }
    out
}

/// The colour packs, in program order: (destination, first source), the
/// colour's temporaries (not the mask's `pck.cov` of a constant).
fn packs(image: &[u8]) -> Vec<(u32, u32)> {
    groups(image)
        .iter()
        .filter(|g| names(g) == ["pck"])
        .filter_map(|g| {
            let s = group_srcs(g)[2].filter(|s| s.bank == TEMP)?;
            Some((group_dsts(g)[0]?.index, s.index))
        })
        .collect()
}

fn ends_with_nop(image: &[u8]) -> bool {
    groups(image).last().is_some_and(|g| names(g) == ["nop"])
}

/// `outs`: (location, GLSL type, value) per output.
fn fragment(outs: &[(u32, &str, &str)]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (loc, ty, _) in outs {
        s += &format!("layout(location = {loc}) out highp {ty} o{loc};\n");
    }
    s += "void main() {\n";
    for (loc, ty, v) in outs {
        s += &format!("  o{loc} = {ty}({v});\n");
    }
    s + "}\n"
}

const RGBA8: TargetPack = TargetPack::Class(SHC_RT_RGBA8);
const RGBA16F: TargetPack = TargetPack::Class(SHC_RT_F16X4);
const R16F: TargetPack = TargetPack::Class(SHC_RT_F16);
const R32F: TargetPack = TargetPack::Class(SHC_RT_F32);
const fn int(channels: u8, bits: u8, signed: bool) -> TargetPack {
    TargetPack::Int(ShcIntRt { channels, bits, signed })
}
const R32I: TargetPack = int(1, 32, true);
const R32UI: TargetPack = int(1, 32, false);
const RGBA8UI: TargetPack = int(4, 8, false);
const RGBA16I: TargetPack = int(4, 16, true);
const RGBA16UI: TargetPack = int(4, 16, false);
const RGBA32UI: TargetPack = int(4, 32, false);

/// The register starts and tile buffers `target_layout` gives, by target.
fn places(t: &[TargetPack]) -> Vec<TargetSlot> {
    let l = target_layout(t).unwrap_or_else(|| panic!("{t:?} has a layout"));
    l.place[..t.len()].iter().map(|p| p.0).collect()
}

fn reg(first: u8) -> TargetSlot {
    TargetSlot::Register { first }
}
fn tb(index: u8) -> TargetSlot {
    TargetSlot::TileBuffer { index }
}

/// The compiler's walk is `mrt_layout::layout`, for every list of one to
/// four targets of every class and integer layout the compiler packs.
#[test]
fn target_layout_is_mrt_layout_for_every_list() {
    let kinds =
        [RGBA8, RGBA16F, R16F, R32F, R32I, int(2, 16, false), RGBA16UI, int(2, 32, true), int(3, 32, false), RGBA32UI];
    let mut checked = 0;
    for n in 1..=4u32 {
        for code in 0..(kinds.len() as u32).pow(n) {
            let t: Vec<TargetPack> =
                (0..n).map(|i| kinds[(code / (kinds.len() as u32).pow(i)) as usize % kinds.len()]).collect();
            let d: Vec<u8> = t.iter().map(|t| t.dwords() as u8).collect();
            match (target_layout(&t), layout(&d)) {
                (Some(c), Ok(m)) => {
                    assert_eq!((c.targets, c.output_regs, c.tile_buffers), (m.targets, m.output_regs, m.tile_buffers));
                    for (i, (&p, &dw)) in m.place.iter().zip(&d).enumerate() {
                        let want = match p {
                            Place::Register { first } => reg(first),
                            Place::TileBuffer { index } => tb(index),
                        };
                        assert_eq!(c.place[i], (want, dw), "{t:?} target {i}");
                    }
                }
                (None, Err(LayoutError::NoProgram { .. })) => {}
                (c, m) => panic!("{t:?}: compiler {c:?}, mrt_layout {m:?}"),
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 10 + 100 + 1000 + 10000);
    assert_eq!(target_layout(&[]), None);
    assert_eq!(target_layout(&[RGBA8; 5]), None);
}

fn bytes_of(g: &Group) -> Vec<u8> {
    let mut out = vec![0u8; g.total];
    assert_eq!(encode_group(g, &mut out, 0), Ok(g.total));
    out
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// The groups of pco's first tile-buffer store, byte for byte: Mesa 26.2.1's
/// `pco_shc fs` of three `vec4` outputs under `PCO_SHC_OUT=f16x4` (the third
/// spills to tile buffer 0, its address at sh0..1), offsets 0xfc..0x16e:
///
/// ```text
/// 0x0fc mbyp     t6 = sh1                          35820087810800000046
/// 0x106 add64_32 t0:t1 = {sh0, t6} + sr58          378e00e88066803a0800c08041ff
/// 0x114 savmsk   t6 = vm                           44a2002000000046
/// 0x11c savmsk   t7 = icm                          44a2002100000047
/// 0x124 pck.cov  t8 = cov(1.0)                     5792009c108040a00030002c48ff
/// 0x132 shift    t8 = t8 >> 1                      55d240010280a8808148
/// 0x13c tstz     t8 = t8 == 0                      99c200d33cf0a09c1e87c8cf8011002048ff
/// 0x14e csel     t6 = t8 > 0 ? t6 : t7             78d200d03cf2b08787c6e88010471146
/// 0x15e st.tiled t2 to t0:t1, mask t6              76a800f264880442c0a600ff
/// 0x16a wdf                                        02806aff
/// ```
#[test]
fn tile_store_groups_are_pcos() {
    use yakogl::shaderc::online::powervr::shapes::{PCK_COV, SAVMSK, TSTZ, USHR};
    let t = |i| op(TEMP, i);
    let mov = build_group(&SelGroup {
        op: SelOp::Copy,
        ty: OpType::F32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: 6 }),
        srcs: [Some(SelSrc::Fixed { bank: Bank::Shared, reg: 1, mods: Mods::default() }), None, None],
    })
    .unwrap();
    assert_eq!(bytes_of(&mov), hex("35820087810800000046"));
    assert_eq!(tiled_st_comp(0), 58);
    assert_eq!(tiled_st_comp(1), 59);
    let add = build_add64_32(t(0), op(SHARED, 0), t(6), op(SPECIAL, 58)).unwrap();
    assert_eq!(bytes_of(&add), hex("378e00e88066803a0800c08041ff"));
    assert_eq!(bytes_of(&build_opt(&SAVMSK, &[None; 3], Some(t(6))).unwrap()), hex("44a2002000000046"));
    assert_eq!(bytes_of(&build_opt(&SAVMSK_ICM, &[None; 3], Some(t(7))).unwrap()), hex("44a2002100000047"));
    let cov = build_opt(&PCK_COV, &[Some(op(SPECIAL, 64)), None, None], Some(t(8))).unwrap();
    assert_eq!(bytes_of(&cov), hex("5792009c108040a00030002c48ff"));
    let shr = build_opt(&USHR, &[Some(t(8)), Some(op(SPECIAL, 1)), None], Some(t(8))).unwrap();
    assert_eq!(bytes_of(&shr), hex("55d240010280a8808148"));
    let tstz = build_opt(&TSTZ, &[Some(t(8)), None, None], Some(t(8))).unwrap();
    assert_eq!(bytes_of(&tstz), hex("99c200d33cf0a09c1e87c8cf8011002048ff"));
    let csel = build_opt(&CSEL_GZ, &[Some(t(8)), Some(t(6)), Some(t(7))], Some(t(6))).unwrap();
    assert_eq!(bytes_of(&csel), hex("78d200d03cf2b08787c6e88010471146"));
    let st = build_st_tiled(t(2), t(0), t(6)).unwrap();
    assert_eq!(bytes_of(&st), hex("76a800f264880442c0a600ff"));
    /* pco's second store: t5 to t3:t4 (sr59), mask t0 */
    assert_eq!(bytes_of(&build_st_tiled(t(5), t(3), t(0)).unwrap()), hex("76a800f264880445c3a000ff"));
    let add2 = build_add64_32(t(3), op(SHARED, 0), t(0), op(SPECIAL, 59)).unwrap();
    assert_eq!(bytes_of(&add2), hex("378e00e88060803b0800c08344ff"));
}

/// Two outputs: two vec4 into RGBA8 (key 0) at registers 0, 1; and an
/// integer target beside a float one, RGBA8UI + RGBA8, the uvec4's bytes
/// packed as they are.
#[test]
fn two_outputs_land_in_their_registers() {
    let src = fragment(&[(0, "vec4", "0.25"), (1, "vec4", "0.5")]);
    let rec = compile(&src, Err(0)).unwrap();
    assert_eq!(rec.num_outputs, 2);
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1]);
    assert!(tile_stores(&rec.image).is_empty());

    let src = fragment(&[(0, "uvec4", "1u, 2u, 3u, 4u"), (1, "vec4", "1.0, 0.0, 0.0, 1.0")]);
    let t = [RGBA8UI, RGBA8];
    assert_eq!(places(&t), [reg(0), reg(1)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (2, 0));
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1]);
    let out = usc_exec::run_binary(&rec.image, &usc_exec::sentinel_inputs(1.0e30), true).expect("runs");
    assert_eq!(out.get(&0), Some(&0x0403_0201), "RGBA8UI bytes: {out:x?}");
    assert_eq!(out.get(&1), Some(&0xff00_00ff), "RGBA8 unorm: {out:x?}");
}

/// Three outputs, an ivec4 into RGBA16I (a pair), a uvec4 into RGBA8UI and
/// a vec4 into RGBA8: registers 0..2, 2, 3.
#[test]
fn three_outputs_int_pair_beside_float() {
    let src =
        fragment(&[(0, "ivec4", "-1, 2, -3, 4"), (1, "uvec4", "5u, 6u, 7u, 8u"), (2, "vec4", "0.0, 1.0, 0.0, 1.0")]);
    let t = [RGBA16I, RGBA8UI, RGBA8];
    assert_eq!(places(&t), [reg(0), reg(2), reg(3)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 0));
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
    let out = usc_exec::run_binary(&rec.image, &usc_exec::sentinel_inputs(1.0e30), true).expect("runs");
    assert_eq!(out.get(&0), Some(&0x0002_ffff), "RGBA16I red, green: {out:x?}");
    assert_eq!(out.get(&1), Some(&0x0004_fffd), "RGBA16I blue, alpha: {out:x?}");
    assert_eq!(out.get(&2), Some(&0x0807_0605), "RGBA8UI: {out:x?}");
    assert_eq!(out.get(&3), Some(&0xff00_ff00), "RGBA8: {out:x?}");
}

/// Four one-dword outputs of four classes, two of them integer: R32I, RGBA8,
/// R32UI, R16F at registers 0..3.
#[test]
fn four_outputs_int_beside_float_in_registers() {
    let src = fragment(&[(0, "int", "-7"), (1, "vec4", "1.0"), (2, "uint", "9u"), (3, "float", "0.5")]);
    let t = [R32I, RGBA8, R32UI, R16F];
    assert_eq!(places(&t), [reg(0), reg(1), reg(2), reg(3)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 0));
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert!(!ends_with_nop(&rec.image), "the last pixel write ends the program");
    let out = usc_exec::run_binary(&rec.image, &usc_exec::sentinel_inputs(1.0e30), true).expect("runs");
    assert_eq!(out.get(&0), Some(&(-7i32 as u32)), "{out:x?}");
    assert_eq!(out.get(&1), Some(&0xffff_ffff), "{out:x?}");
    assert_eq!(out.get(&2), Some(&9), "{out:x?}");
    assert_eq!(out.get(&3).map(|v| v & 0xffff), Some(0x3800), "R16F 0.5: {out:x?}");
}

/// The register a spilled output's dwords are packed into, from the pack
/// whose first source is the output's own move block.
fn assert_spill(rec: &Rec, want: &[(u32, u32)]) {
    let stores = tile_stores(&rec.image);
    let got: Vec<(u32, u32)> = stores.iter().map(|s| (s.sh, s.comp)).collect();
    assert_eq!(got, want, "(shared pair, tiled_st_comp) per stored dword: {stores:?}");
    assert!(ends_with_nop(&rec.image), "a nop ends a program whose last group is a store's wdf");
    /* no pixel write ends the program */
    let gs = groups(&rec.image);
    assert!(gs[..gs.len() - 1].iter().all(|g| g.hdr.get("end") != Some(1)));
}

/// Three highp vec4 outputs into RGBA16F (the Quick 3D G-buffer): the
/// third does not fit the four registers and is stored to tile buffer 0
/// (`eot3_tb`), its two halves-dwords to the pixel's offsets 0 and 1.
#[test]
fn three_rgba16f_spill_the_third_to_tile_buffer_0() {
    let src = fragment(&[(0, "vec4", "0.25"), (1, "vec4", "0.5"), (2, "vec4", "1.0, 0.25, 0.5, 0.75")]);
    let t = [RGBA16F; 3];
    assert_eq!(places(&t), [reg(0), reg(2), tb(0)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 1));
    assert_eq!(rec.tile_buffer_sh, 0, "nothing else in the shared file: the pair at sh0");
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_spill(&rec, &[(0, 58), (0, 59)]);
    /* the stored dwords are the third output's two packs */
    let p = packs(&rec.image);
    assert_eq!(p.len(), 6, "two packs an output: {p:?}");
    let stored: Vec<u32> = tile_stores(&rec.image).iter().map(|s| s.data).collect();
    assert_eq!(stored, [p[4].0, p[5].0]);
    let written: Vec<u32> = pixout_writes(&rec.image).iter().map(|w| w.1).collect();
    assert_eq!(written, [p[0].0, p[1].0, p[2].0, p[3].0]);
}

/// Four targets with a 64-bit one among them: RGBA8, RGBA8, RGBA16F, RGBA8
/// -- the fourth spills (`eot4_tb`); RGBA16F, RGBA16F, RGBA8, RGBA16UI -- the
/// last two spill (`eot4_tb2`), the integer pair's two dwords to buffer 1;
/// RGBA32UI first -- the other three spill (`eot4_tb3`), a float beside it.
#[test]
fn four_targets_spill_per_mrt_layout() {
    let src = fragment(&[(0, "vec4", "0.1"), (1, "vec4", "0.2"), (2, "vec4", "0.3"), (3, "vec4", "0.4")]);
    let t = [RGBA8, RGBA8, RGBA16F, RGBA8];
    assert_eq!(places(&t), [reg(0), reg(1), reg(2), tb(0)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 1));
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_spill(&rec, &[(0, 58)]);

    let src = fragment(&[(0, "vec4", "0.1"), (1, "vec4", "0.2"), (2, "vec4", "0.3"), (3, "uvec4", "1u, 2u, 3u, 4u")]);
    let t = [RGBA16F, RGBA16F, RGBA8, RGBA16UI];
    assert_eq!(places(&t), [reg(0), reg(2), tb(0), tb(1)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 2));
    assert_spill(&rec, &[(0, 58), (2, 58), (2, 59)]);

    let src = fragment(&[(0, "uvec4", "1u, 2u, 3u, 4u"), (1, "float", "0.5"), (2, "vec4", "0.3"), (3, "int", "-2")]);
    let t = [RGBA32UI, R32F, RGBA8, R32I];
    assert_eq!(places(&t), [reg(0), tb(0), tb(1), tb(2)]);
    let rec = compile_targets(&src, &t);
    assert_eq!((rec.num_outputs, rec.num_tile_buffers), (4, 3));
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_spill(&rec, &[(0, 58), (2, 58), (4, 58)]);
}

/// The layouts no end-of-tile program was captured for are refused by name
/// (an RGBA32UI target and one after it, two targets).
#[test]
fn a_spill_with_no_end_of_tile_program_is_refused() {
    let src = fragment(&[(0, "uvec4", "1u"), (1, "float", "0.5")]);
    let e = compile(&src, Ok(&[RGBA32UI, R32F])).expect_err("refused");
    assert!(e.contains("no end-of-tile program"), "{e}");
}

/// The buffers' address pairs follow everything the program reads from the
/// shared file: here a uniform block.
#[test]
fn tile_buffer_pairs_follow_the_uniforms() {
    let src = "#version 300 es\nprecision highp float;\nuniform vec4 u;\n\
               layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
               layout(location = 2) out vec4 o2;\n\
               void main() { o0 = u; o1 = u * 2.0; o2 = u + 1.0; }\n";
    let rec = compile_targets(src, &[RGBA16F; 3]);
    assert_eq!(rec.num_tile_buffers, 1);
    assert!(rec.uniform_dwords >= 4);
    assert_eq!(rec.tile_buffer_sh, rec.uniform_dwords, "the pair straight after the block");
    let sh = u32::from(rec.tile_buffer_sh);
    assert_spill(&rec, &[(sh, 58), (sh, 59)]);
}

/// A written gl_FragDepth beside several targets: fed back (`depthf`, then
/// the `wdf`) before the first pixel write, and the record says so; with a
/// discard, under the kill test.
#[test]
fn depth_and_discard_beside_several_targets() {
    let src = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
               layout(location = 0) out vec4 o0;\nlayout(location = 1) out highp uint o1;\n\
               void main() { o0 = v; o1 = 3u; gl_FragDepth = v.z; }\n";
    let rec = compile_targets(src, &[RGBA8, R32UI]);
    assert_eq!(rec.writes_depth, 1);
    let gs = groups(&rec.image);
    let first_write = gs
        .iter()
        .position(|g| names(g) == ["mbyp"] && group_dsts(g)[0].is_some_and(|d| d.bank == SPECIAL && d.index == 32))
        .expect("a pixel write");
    let depthf = gs.iter().position(|g| names(g).contains(&"depthf")).expect("a depthf");
    assert!(depthf < first_write, "the depth before the colours");
    assert_eq!(names(&gs[depthf + 1]), ["wdf"]);
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1]);

    let src = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
               layout(location = 0) out vec4 o0;\nlayout(location = 1) out highp uint o1;\n\
               layout(location = 2) out vec4 o2;\n\
               void main() { if (v.x < 0.5) discard; o0 = v; o1 = 3u; o2 = v; gl_FragDepth = v.z; }\n";
    let rec = compile_targets(src, &[RGBA16F, R32UI, RGBA16F]);
    assert_eq!((rec.writes_depth, rec.num_tile_buffers), (1, 1));
    let gs = groups(&rec.image);
    let kill = gs.iter().position(|g| names(g).contains(&"tst") && group_dsts(g)[0].is_none()).expect("a kill test");
    let depthf = gs.iter().position(|g| names(g).contains(&"depthf")).expect("a depthf");
    let alphaf = gs.iter().position(|g| names(g).contains(&"alphaf")).expect("an alphaf");
    assert!(kill < alphaf && alphaf < depthf, "tstz, alphaf, depthf");
    assert_eq!(pixout_writes(&rec.image).iter().map(|w| w.0).collect::<Vec<_>>(), [0, 1, 2]);
    /* the control word a discard reads is sh0: the pair follows it */
    assert_eq!(rec.tile_buffer_sh, 1);
    assert_spill(&rec, &[(1, 58), (1, 59)]);
    assert!(depthf < groups(&rec.image).iter().position(|g| names(g) == ["st.tiled"]).unwrap());
}
