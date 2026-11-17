// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `usc_exec` runs the PowerVR `load_ubo` sequences of
//! `docs/notes/pvr-ubo-memory-load.md` §2, built with the real encoder, over
//! a 16384-byte host buffer of distinct words: the shared register pair
//! holds the block's device address, `add64_32` adds the byte offset, `ld`
//! reads (burst 1 and burst 4), `wdf` waits.
//!
//! - constant offset: `add64_32 t0,t1 = sh8,sh9 + 4*dword_off`, `ld`, `wdf`,
//!   at dword 0 and 4095 (burst 1) and the last vec4, dwords 4092..4095
//!   (burst 4);
//! - run-time offset: the index clamped to the array (`imax`, `imin`),
//!   scaled to bytes (`imul`), `add64_32`, `ld`, `wdf`; an index past the end
//!   (and below zero) reads the first or the last element;
//! - the carry: a block whose low address word wraps inside the block, so the
//!   `add64_32` carries into the high word.
//!
//! `add64_32` has no builder in `assemble.rs` yet (`ubo-select` adds it);
//! the test makes the group from pco's own `add64_32` of the end-of-tile
//! programs (`eot2_tb` in `tools/pco_shc/usc_blobs.txt`): the header, ISS
//! and `s3` stay pco's, the operand sections are rebuilt with the repo's
//! encoder, and with pco's operands the result is pco's bytes (checked).
//! The output moves are the groups of `tests/usc_exec_ld.rs`. Nothing here
//! has run on hardware: the model is this repository's reading of the note.
//! HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test usc_exec_ubo_seq
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::assemble::{build_group, build_imm_move, build_ld, close, Ld, Parts};
use yakogl::shaderc::online::powervr::decode::{decode_group, Group};
use yakogl::shaderc::online::powervr::encode::encode_group;
use yakogl::shaderc::online::powervr::fragment::wait_data;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::legalise::{choose_dst_variant, choose_src_variant, fill_dsts, fill_srcs};
use yakogl::shaderc::online::powervr::operands::Operand;
use yakogl::shaderc::online::powervr::prepare::const_reg;
use yakogl::shaderc::online::powervr::sel::{Bank, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc};
use yakogl::shaderc::online::powervr::shapes::{build, IMAX, IMIN, IMUL};

mod usc_exec;

const SPECIAL: u32 = 0;
const TEMP: u32 = 1;
const SHARED: u32 = 4;

fn op(bank: u32, index: u32) -> Operand {
    Operand { bank, index }
}

fn bytes_of(g: &Group) -> Vec<u8> {
    let mut out = vec![0u8; g.total];
    assert_eq!(encode_group(g, &mut out, 0), Ok(g.total));
    out
}

/// pco's `add64_32` group of `eot2_tb`, the template the builder below
/// takes its header, ISS and `s3` from.
fn pco_add64() -> (Group, Vec<u8>) {
    let txt = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/pco_shc/usc_blobs.txt"))
        .expect("tools/pco_shc/usc_blobs.txt");
    let line = txt.lines().find(|l| l.starts_with("eot2_tb |")).expect("eot2_tb");
    let words = line.split("words ").nth(1).and_then(|w| w.split(" |").next()).expect("a words column");
    let bytes: Vec<u8> = words
        .split_whitespace()
        .flat_map(|w| u32::from_str_radix(w.trim_start_matches("0x"), 16).expect("a hex word").to_le_bytes())
        .collect();
    let mut off = 0;
    while off < bytes.len() {
        let g = decode_group(&bytes, off).unwrap_or_else(|e| panic!("eot2_tb @{off}: {e:?}"));
        let first = g.instrs().next().map(|(_, ph, r)| ops_for(r, ph).next().map_or("?", |o| o.name));
        if first == Some("add64_32") {
            let raw = bytes[off..off + g.total].to_vec();
            return (g, raw);
        }
        off += g.total;
    }
    panic!("no add64_32 in eot2_tb");
}

/// `add64_32 dst.lo, dst.hi = lo, hi, add` (pco's `.s`, no carry-in): the
/// 64-bit value `hi:lo` plus the 32-bit `add`.
fn add64_32(lo: Operand, hi: Operand, add: Operand, dst: [Operand; 2]) -> Vec<u8> {
    let (t, _) = pco_add64();
    let ops = [Some(lo), Some(hi), Some(add), None, None, None];
    let mut lower = yakogl::shaderc::online::powervr::isa::Region::new(
        yakogl::shaderc::online::powervr::isa::BitSetId::Src,
        choose_src_variant(&ops, 0).expect("a source form"),
    );
    fill_srcs(&mut lower, &ops, 0).expect("the sources fit");
    let dops = [Some(dst[0]), Some(dst[1])];
    let mut d = yakogl::shaderc::online::powervr::isa::Region::new(
        yakogl::shaderc::online::powervr::isa::BitSetId::Dst,
        choose_dst_variant(&dops).expect("a destination form"),
    );
    fill_dsts(&mut d, &dops).expect("the destinations fit");
    let parts = Parts {
        alutype: t.alutype,
        instrs: t.instrs,
        lower: Some(lower),
        upper: t.upper,
        iss: t.iss,
        dst: Some(d),
        align_pad: 0,
    };
    bytes_of(&close(&parts, t.hdr).expect("a closed group"))
}

/// `mbyp t{dst}, sh{src}`: the move that puts the pair's high half in a
/// temporary. `add64_32`'s `s1` has no shared-register form in the ISA
/// tables (`choose_src_variant` refuses it; `s0` and `s2` take one), and pco
/// legalises such a source with a move too.
fn copy_shared(src: u32, dst: u32) -> Vec<u8> {
    let g = SelGroup {
        op: SelOp::Copy,
        ty: OpType::U32,
        dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dst as u16 }),
        srcs: [Some(SelSrc::Fixed { bank: Bank::Shared, reg: src as u16, mods: Mods::default() }), None, None],
    };
    bytes_of(&build_group(&g).expect("mbyp"))
}

/// `add64_32 t0, t1 = sh{PAIR}, sh{PAIR+1}, add`, the high half through t4.
fn pair_plus(seq: &mut Vec<Vec<u8>>, add: Operand, dst: [Operand; 2]) {
    seq.push(copy_shared(PAIR + 1, 4));
    seq.push(add64_32(op(SHARED, PAIR), op(TEMP, 4), add, dst));
}

fn ld(burst: u8, dst: u32, addr: u32) -> Vec<u8> {
    bytes_of(&build_ld(&Ld::imm(op(TEMP, dst), op(TEMP, addr), burst)).expect("ld"))
}

fn wdf() -> Vec<u8> {
    bytes_of(&wait_data().expect("wdf"))
}

/// `mbyp pixout{k}, t{2+k}`, `olchk`, `end` on the last of `n`
/// (tests/usc_exec_ld.rs).
const PIX: [[u8; 8]; 4] = [
    [0x34, 0x8a, 0x00, 0x87, 0x42, 0x00, 0x00, 0x20],
    [0x34, 0x8a, 0x00, 0x87, 0x43, 0x00, 0x00, 0x21],
    [0x34, 0x8a, 0x00, 0x87, 0x44, 0x00, 0x00, 0x22],
    [0x34, 0x8a, 0x80, 0x87, 0x45, 0x00, 0x00, 0x23],
];
const PIX0_END: [u8; 8] = [0x34, 0x8a, 0x80, 0x87, 0x42, 0x00, 0x00, 0x20];

fn outputs(n: usize) -> Vec<u8> {
    match n {
        1 => PIX0_END.to_vec(),
        4 => PIX.concat(),
        _ => unreachable!(),
    }
}

/// The block's device address with room below 2^32 for its whole length.
const BASE: u64 = 0x1_2340_0000;
/// The block straddling a 4 GiB boundary: 4096 bytes below it, 12288 above.
const CARRY_BASE: u64 = 0x1_ffff_f000;
/// 16384 bytes: `MAX_UNIFORM_BLOCK_SIZE` under ES 3.0.
const BLOCK_DW: u32 = 4096;
/// The address pair's shared registers, the index's shared register.
const PAIR: u32 = 8;
const INDEX: u32 = 2;

fn block_word(k: u32) -> u32 {
    0xb10c_0000 ^ k.wrapping_mul(0x9e37_79b9)
}

fn inputs(base: u64, index: u32) -> usc_exec::Inputs {
    let mut inp = usc_exec::sentinel_inputs(-7.0);
    inp.mem = vec![(base, (0..BLOCK_DW).map(block_word).collect())];
    inp.shared[PAIR as usize] = base as u32;
    inp.shared[PAIR as usize + 1] = (base >> 32) as u32;
    inp.shared[INDEX as usize] = index;
    inp
}

fn run(groups: &[Vec<u8>], inp: &usc_exec::Inputs) -> Result<Vec<u32>, String> {
    let out = usc_exec::run_binary(&groups.concat(), inp, true)?;
    Ok(out.values().copied().collect())
}

/// The operand holding `v`: a special constant register when the hardware
/// has one, else a `movi32` into `scratch` (the group is pushed on `seq`).
fn imm(seq: &mut Vec<Vec<u8>>, v: u32, scratch: u32) -> Operand {
    match const_reg(v) {
        Some(r) => op(SPECIAL, u32::from(r)),
        None => {
            seq.push(bytes_of(&build_imm_move(op(TEMP, scratch), v).expect("movi32")));
            op(TEMP, scratch)
        }
    }
}

/// The note's constant-offset sequence for `comps` dwords at `dword_off`.
fn constant_seq(dword_off: u32, comps: u8) -> Vec<Vec<u8>> {
    let mut seq = Vec::new();
    let off = imm(&mut seq, 4 * dword_off, 7);
    pair_plus(&mut seq, off, [op(TEMP, 0), op(TEMP, 1)]);
    seq.push(ld(comps, 2, 0));
    seq.push(wdf());
    seq.push(outputs(if comps == 1 { 1 } else { 4 }));
    seq
}

/// The note's run-time sequence over an array of `len` elements of `comps`
/// dwords each: clamp the index in sh2 to `[0, len)`, scale to bytes,
/// `add64_32`, `ld`, `wdf`.
fn runtime_seq(len: u32, comps: u8) -> Vec<Vec<u8>> {
    let mut seq = Vec::new();
    let zero = op(SPECIAL, 0);
    let g = |t: &_, a, b, d| bytes_of(&build(t, &[Some(a), Some(b), None], d).expect("a built group"));
    seq.push(g(&IMAX, op(SHARED, INDEX), zero, op(TEMP, 6)));
    let top = imm(&mut seq, len - 1, 7);
    seq.push(g(&IMIN, op(TEMP, 6), top, op(TEMP, 6)));
    let stride = imm(&mut seq, 4 * u32::from(comps), 7);
    seq.push(g(&IMUL, op(TEMP, 6), stride, op(TEMP, 7)));
    pair_plus(&mut seq, op(TEMP, 7), [op(TEMP, 0), op(TEMP, 1)]);
    seq.push(ld(comps, 2, 0));
    seq.push(wdf());
    seq.push(outputs(if comps == 1 { 1 } else { 4 }));
    seq
}

/// With pco's own operands the builder makes pco's `add64_32` group byte
/// for byte: the encoding is Mesa's, only the operands move.
#[test]
fn add64_32_is_pcos_group() {
    let (_, pco) = pco_add64();
    assert_eq!(add64_32(op(TEMP, 0), op(TEMP, 1), op(SPECIAL, 54), [op(TEMP, 1), op(TEMP, 2)]), pco);
    /* and it decodes as the one operation, on both write ports */
    let mine = add64_32(op(SHARED, PAIR), op(TEMP, 4), op(SPECIAL, 54), [op(TEMP, 0), op(TEMP, 1)]);
    let g = decode_group(&mine, 0).expect("decodes");
    let names: Vec<_> = g.instrs().map(|(_, ph, r)| ops_for(r, ph).next().map(|o| o.name)).collect();
    assert_eq!(names, [Some("add64_32")]);
    assert_eq!((g.hdr.get("w0p"), g.hdr.get("w1p")), (Some(1), Some(1)));
}

/// The sum and its carry, read straight off the two write ports: the low
/// word wraps into the high word, an offset that does not wrap leaves it.
#[test]
fn add64_32_carries_into_the_high_word() {
    let cases: [(u64, u32); 4] = [
        (0x1_ffff_fff0, 0x20),  /* wraps: lo 0x10, hi 2 */
        (0x1_ffff_fff0, 0x0f),  /* the last dword below the wrap: lo 0xffff_ffff */
        (0x1_ffff_fff0, 0x10),  /* exactly to 2^32: lo 0, hi 2 */
        (0x7_0000_0000, 16380), /* no carry */
    ];
    for (base, add) in cases {
        let mut seq = Vec::new();
        let off = imm(&mut seq, add, 7);
        pair_plus(&mut seq, off, [op(TEMP, 2), op(TEMP, 3)]);
        pair_plus(&mut seq, op(SPECIAL, 0), [op(TEMP, 4), op(TEMP, 5)]);
        seq.push(outputs(4));
        let got = run(&seq, &inputs(base, 0)).unwrap();
        let sum = base + u64::from(add);
        assert_eq!(got, [sum as u32, (sum >> 32) as u32, base as u32, (base >> 32) as u32], "{base:#x} + {add}");
    }
}

/// Constant offset, burst 1: the block's first and last dword, with and
/// without the carry.
#[test]
fn constant_offset_first_and_last_dword() {
    for base in [BASE, CARRY_BASE] {
        for k in [0, 1, 1023, 1024, BLOCK_DW - 1] {
            let got = run(&constant_seq(k, 1), &inputs(base, 0)).unwrap();
            assert_eq!(got, [block_word(k)], "base {base:#x}, dword {k}");
        }
    }
}

/// Constant offset, burst 4: the first and the last vec4, dwords 4092..4095.
#[test]
fn constant_offset_first_and_last_vec4() {
    for base in [BASE, CARRY_BASE] {
        for k in [0, 4, BLOCK_DW - 4] {
            let got = run(&constant_seq(k, 4), &inputs(base, 0)).unwrap();
            assert_eq!(got, (k..k + 4).map(block_word).collect::<Vec<_>>(), "base {base:#x}, vec4 at dword {k}");
        }
    }
}

/// Run-time offset over a 4096-dword array (burst 1): in range at both
/// ends, and an index past the end, far past it, or negative clamps.
#[test]
fn runtime_offset_dwords_clamp_to_the_array() {
    for base in [BASE, CARRY_BASE] {
        for (index, want) in [
            (0u32, 0),
            (1, 1),
            (2048, 2048),
            (BLOCK_DW - 1, BLOCK_DW - 1),
            (BLOCK_DW, BLOCK_DW - 1),
            (BLOCK_DW + 1, BLOCK_DW - 1),
            (1_000_000, BLOCK_DW - 1),
            (u32::MAX, 0), /* -1 as an int: below the array, the first element */
        ] {
            let got = run(&runtime_seq(BLOCK_DW, 1), &inputs(base, index)).unwrap();
            assert_eq!(got, [block_word(want)], "base {base:#x}, index {index}");
        }
    }
}

/// Run-time offset over a 1024-element vec4 array (burst 4, stride 16
/// bytes): the first and the last vec4, and an index past the end.
#[test]
fn runtime_offset_vec4_clamp_to_the_array() {
    for base in [BASE, CARRY_BASE] {
        for (index, want) in [(0u32, 0), (1, 1), (1023, 1023), (1024, 1023), (5000, 1023), (u32::MAX, 0)] {
            let got = run(&runtime_seq(BLOCK_DW / 4, 4), &inputs(base, index)).unwrap();
            assert_eq!(
                got,
                (4 * want..4 * want + 4).map(block_word).collect::<Vec<_>>(),
                "base {base:#x}, index {index}"
            );
        }
    }
}

/// Without the clamp the same sequence reads outside the block: the error
/// the model reports is what the clamp is for.
#[test]
fn unclamped_runtime_offset_leaves_the_block() {
    let mut seq = Vec::new();
    let stride = op(SPECIAL, 4);
    seq.push(bytes_of(&build(&IMUL, &[Some(op(SHARED, INDEX)), Some(stride), None], op(TEMP, 7)).unwrap()));
    pair_plus(&mut seq, op(TEMP, 7), [op(TEMP, 0), op(TEMP, 1)]);
    seq.push(ld(1, 2, 0));
    seq.push(wdf());
    seq.push(outputs(1));
    let e = run(&seq, &inputs(BASE, BLOCK_DW)).unwrap_err();
    assert!(e.contains("outside every region"), "{e}");
    assert_eq!(run(&seq, &inputs(BASE, BLOCK_DW - 1)).unwrap(), [block_word(BLOCK_DW - 1)]);
}
