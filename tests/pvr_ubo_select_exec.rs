// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR online compiler's uniform-block member read, as the passes
//! below the selector build it, runs in `usc_exec` over a 16384-byte host
//! buffer of distinct words and reads the dwords it names. Plan #378 item
//! `ubo-select`; the design is `docs/notes/pvr-ubo-memory-load.md` §2-§4.
//!
//! Each case is `ubo_rows::MemberLoads::load` -- `add64_32` of the shared
//! address pair and the byte offset, `ld`, `wdf` -- put through `prepare`,
//! `index_high_shared`, the allocator, `verify::check_ld_waits` and the
//! builder (`assemble::build_group`) exactly as `chain` runs a program,
//! followed by moves of the loaded lanes into t2.. and the pixel-output
//! moves of `tests/usc_exec_ld.rs`. Covered: a 16384-byte block's first
//! dword (byte 0) and last (byte 16380) at a constant offset, the last vec4
//! in one 4-dword burst, a run-time byte offset from a shared register, a
//! block that straddles a 4 GiB boundary (the `add64_32` carry), and the
//! pair past sh255 read through `idx0`. No selector routes a read here yet
//! (`ubo-chain` does), and nothing has run on hardware: the model is this
//! repository's reading of the note. HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ubo_select_exec
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::powervr::alloc::{allocate, Boundary, Limits};
use yakogl::shaderc::online::powervr::assemble::build_group;
use yakogl::shaderc::online::powervr::encode::encode_group;
use yakogl::shaderc::online::powervr::prepare::{index_high_shared, prepare_counted};
use yakogl::shaderc::online::powervr::sel::{Bank, Mods, OpType, SelDst, SelGroup, SelOp, SelSrc, VReg};
use yakogl::shaderc::online::powervr::select::Selected;
use yakogl::shaderc::online::powervr::ubo_rows::{MemberLoads, MemberOffset};
use yakogl::shaderc::online::powervr::verify::check_ld_waits;

mod usc_exec;

/// `mbyp pixout{k}, t{2+k}`, `olchk`, `end` on the last of `n`
/// (tests/usc_exec_ld.rs).
const PIX: [[u8; 8]; 4] = [
    [0x34, 0x8a, 0x00, 0x87, 0x42, 0x00, 0x00, 0x20],
    [0x34, 0x8a, 0x00, 0x87, 0x43, 0x00, 0x00, 0x21],
    [0x34, 0x8a, 0x00, 0x87, 0x44, 0x00, 0x00, 0x22],
    [0x34, 0x8a, 0x80, 0x87, 0x45, 0x00, 0x00, 0x23],
];
const PIX0_END: [u8; 8] = [0x34, 0x8a, 0x80, 0x87, 0x42, 0x00, 0x00, 0x20];

/// The block's device address with room below 2^32 for its whole length.
const BASE: u64 = 0x1_2340_0000;
/// The block straddling a 4 GiB boundary: 4096 bytes below it, 12288 above.
const CARRY_BASE: u64 = 0x1_ffff_f000;
/// 16384 bytes: `MAX_UNIFORM_BLOCK_SIZE` under ES 3.0.
const BLOCK_DW: u32 = 4096;
/// The shared register a run-time byte offset is read from.
const OFFSET_SH: u16 = 2;

fn block_word(k: u32) -> u32 {
    0xb10c_0000 ^ k.wrapping_mul(0x9e37_79b9)
}

/// `pair` holds the block address; `OFFSET_SH` the run-time byte offset.
fn inputs(base: u64, pair: u16, offset: u32) -> usc_exec::Inputs {
    let mut inp = usc_exec::sentinel_inputs(-7.0);
    inp.mem = vec![(base, (0..BLOCK_DW).map(block_word).collect())];
    inp.shared[usize::from(pair)] = base as u32;
    inp.shared[usize::from(pair) + 1] = (base >> 32) as u32;
    inp.shared[usize::from(OFFSET_SH)] = offset;
    inp
}

/// The read of `burst` dwords at `off` from the pair at `pair`, through the
/// compiler's passes, then the output moves: the program's bytes.
fn program(pair: u16, off: MemberOffset, burst: u8) -> Vec<u8> {
    let loads = MemberLoads { on: true, pair };
    let mut groups = loads.load(off, burst, VReg(10), VReg(20)).expect("the option is on").to_vec();
    for k in 0..u16::from(burst) {
        groups.push(SelGroup {
            op: SelOp::Copy,
            ty: OpType::F32,
            dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: 2 + k }),
            srcs: [Some(SelSrc::Virtual { v: VReg(20 + u32::from(k)), mods: Mods::default() }), None, None],
        });
    }
    let mut buf = vec![0u8; 1 << 20];
    let a = Arena::new(&mut buf);
    let sel = Selected {
        groups: a.alloc_copy(&groups).unwrap(),
        vregs: 32,
        outputs: &[],
        inputs: &[],
        discard: None,
        alpha_to_one: None,
    };
    let (sel, _) = prepare_counted(&sel, &a).expect("prepares");
    let sel = index_high_shared(&sel, &a).expect("indexes");
    let limits = Limits { temps: 64, shared: 2048, vtx_in: 32, coeff: 32 };
    let al = allocate(sel.groups, limits, [], Boundary::NONE, &a).expect("allocates");
    check_ld_waits(al.groups).expect("each ld waits");
    let mut out = Vec::new();
    for g in al.groups {
        let b = build_group(g).unwrap_or_else(|e| panic!("{g:?}: {e:?}"));
        let at = out.len();
        out.resize(at + b.total, 0);
        assert_eq!(encode_group(&b, &mut out, at), Ok(b.total));
    }
    match burst {
        1 => out.extend_from_slice(&PIX0_END),
        4 => out.extend_from_slice(&PIX.concat()),
        _ => unreachable!(),
    }
    out
}

fn run(image: &[u8], inp: &usc_exec::Inputs) -> Vec<u32> {
    usc_exec::run_binary(image, inp, true).unwrap_or_else(|e| panic!("{e}")).values().copied().collect()
}

/// A constant offset, burst 1: the block's first and last dword, with and
/// without the carry into the address's high word.
#[test]
fn constant_offset_first_and_last_dword() {
    for k in [0, 1, 1023, 1024, BLOCK_DW - 1] {
        let image = program(8, MemberOffset::Dword(k), 1);
        for base in [BASE, CARRY_BASE] {
            assert_eq!(run(&image, &inputs(base, 8, 0)), [block_word(k)], "base {base:#x}, dword {k}");
        }
    }
}

/// A constant offset, burst 4: the first and the last vec4 (dwords
/// 4092..4095, byte 16368).
#[test]
fn constant_offset_first_and_last_vec4() {
    for k in [0, 4, BLOCK_DW - 4] {
        let image = program(8, MemberOffset::Dword(k), 4);
        for base in [BASE, CARRY_BASE] {
            let want: Vec<u32> = (k..k + 4).map(block_word).collect();
            assert_eq!(run(&image, &inputs(base, 8, 0)), want, "base {base:#x}, vec4 at dword {k}");
        }
    }
}

/// A run-time byte offset (here read from a shared register; the selector
/// computes it from the clamped index): one program reads byte 0 and byte
/// 16380, and a vec4 at byte 16368.
#[test]
fn run_time_offset_first_and_last() {
    let off = MemberOffset::Bytes(SelSrc::Fixed { bank: Bank::Shared, reg: OFFSET_SH, mods: Mods::default() });
    let one = program(8, off, 1);
    let four = program(8, off, 4);
    for base in [BASE, CARRY_BASE] {
        assert_eq!(run(&one, &inputs(base, 8, 0)), [block_word(0)]);
        assert_eq!(run(&one, &inputs(base, 8, 4 * (BLOCK_DW - 1))), [block_word(BLOCK_DW - 1)]);
        let want: Vec<u32> = (BLOCK_DW - 4..BLOCK_DW).map(block_word).collect();
        assert_eq!(run(&four, &inputs(base, 8, 4 * (BLOCK_DW - 4))), want);
    }
}

/// The address pair past sh255, read through `idx0` (both halves, one
/// rewrite each): the last dword still comes back.
#[test]
fn a_pair_past_sh255() {
    let image = program(300, MemberOffset::Dword(BLOCK_DW - 1), 1);
    for base in [BASE, CARRY_BASE] {
        assert_eq!(run(&image, &inputs(base, 300, 0)), [block_word(BLOCK_DW - 1)], "base {base:#x}");
    }
}

/// The model's own control: the same read with the pair holding another
/// address misses the block and is refused, so a pass above is the address
/// the program computed and not a lucky default.
#[test]
fn a_wrong_address_misses_the_block() {
    let image = program(8, MemberOffset::Dword(0), 1);
    let mut inp = inputs(BASE, 8, 0);
    inp.shared[8] = (BASE as u32).wrapping_add(4 * BLOCK_DW);
    assert!(usc_exec::run_binary(&image, &inp, true).is_err());
}
