// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `usc_exec` runs the USC global memory load `ld` and the data fence `wdf`
//! over a host memory map (`Inputs::mem`): hand-built groups read a
//! 16384-byte uniform block at its first and last dword, one dword and a
//! vec4 at a time, from an address held in shared registers and from one
//! computed at run time; a read of the loaded register before its `wdf`, and
//! a read outside the map, are errors the interpreter reports. The plan is
//! `docs/notes/pvr-ubo-memory-load.md` (§1 the encoding, §4 the wait).
//! HOST-ONLY.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test usc_exec_ld
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::srcs;

mod usc_exec;

/* The groups, pinned byte for byte.
 *
 * `ld` (pco `group_map(O_LD)`, pco_map.py:3301-3316): a MAIN header with
 * `oporg` be and no write port, the backend-op word, the address pair in
 * s0 (`srcseladd` s0), the destination in s3, the ISS byte. Bytes 3..6 are
 * the backend-op words of the note's §1 table (`ld drc0, 1, s0` =
 * f1 04 00, `ld drc0, 4, s0` = f1 10 00, cache mode normal). The note gives
 * only that word: the header, the source sections, the ISS byte and the
 * pad are derived from isa_tables.rs (the repo's encoder over those tables,
 * `assemble::close`), and pco's offline corpus holds no `ld` to compare
 * them with. `ld-encode` checks the whole group against PCO_SHC. */
/// `ld drc0, 1, sh8:sh9` into t2.
const LD1_SH8_T2: [u8; 12] = [0x66, 0xa0, 0x00, 0xf1, 0x04, 0x00, 0x88, 0x08, 0x00, 0x42, 0x00, 0xff];
/// `ld drc0, 4, sh8:sh9` into t2..t5.
const LD4_SH8_T2: [u8; 12] = [0x66, 0xa0, 0x00, 0xf1, 0x10, 0x00, 0x88, 0x08, 0x00, 0x42, 0x00, 0xff];
/// `ld drc0, 1, t0:t1` into t2.
const LD1_T0_T2: [u8; 10] = [0x65, 0xa0, 0x00, 0xf1, 0x04, 0x00, 0x40, 0x42, 0x00, 0xff];
/// `ld drc0, 4, t0:t1` into t2..t5.
const LD4_T0_T2: [u8; 10] = [0x65, 0xa0, 0x00, 0xf1, 0x10, 0x00, 0x40, 0x42, 0x00, 0xff];
/// `wdf` on DRC 0: a control group, `ctrlop` wdf, `miscctl` 0 (the note's
/// §4). The same bytes as pco's own `wdf` in the offline corpus.
const WDF: [u8; 4] = [0x02, 0x80, 0x6a, 0xff];
/* The run-time address and the outputs, from isa_tables.rs through the
 * repo's encoder (`assemble::build_group` for the moves, `close` for the
 * imadd32): no `ld` in them, so nothing of the note to pin. */
/// `mbyp t1, sh1` (the address's high dword).
const T1_SH1: [u8; 10] = [0x35, 0x82, 0x00, 0x87, 0x81, 0x08, 0x00, 0x00, 0x00, 0x41];
/// `mbyp t4, sh2` (the index).
const T4_SH2: [u8; 10] = [0x35, 0x82, 0x00, 0x87, 0x82, 0x08, 0x00, 0x00, 0x00, 0x44];
/// `mbyp t5, sh3` (the stride in bytes).
const T5_SH3: [u8; 10] = [0x35, 0x82, 0x00, 0x87, 0x83, 0x08, 0x00, 0x00, 0x00, 0x45];
/// `mbyp t6, sh0` (the address's low dword).
const T6_SH0: [u8; 10] = [0x35, 0x82, 0x00, 0x87, 0x80, 0x08, 0x00, 0x00, 0x00, 0x46];
/// `imadd32 t0, t4, t5, t6`: the low dword of base + index * stride. No
/// carry into the high dword: the tests keep the block inside one 4 GiB
/// window (the compiler's `add64_32` carries; this model has no such op).
const T0_IMADD: [u8; 12] = [0x36, 0x82, 0x00, 0xe2, 0xc4, 0x65, 0x00, 0x46, 0x00, 0x00, 0x40, 0xff];
/// `mbyp pixout{k}, t{2+k}` with `olchk`, and with `end` on the last.
const PIX: [[u8; 8]; 4] = [
    [0x34, 0x8a, 0x00, 0x87, 0x42, 0x00, 0x00, 0x20],
    [0x34, 0x8a, 0x00, 0x87, 0x43, 0x00, 0x00, 0x21],
    [0x34, 0x8a, 0x00, 0x87, 0x44, 0x00, 0x00, 0x22],
    [0x34, 0x8a, 0x80, 0x87, 0x45, 0x00, 0x00, 0x23],
];
/// `mbyp pixout0, t2` with `olchk` and `end`.
const PIX0_END: [u8; 8] = [0x34, 0x8a, 0x80, 0x87, 0x42, 0x00, 0x00, 0x20];

/// The block's device address: above 4 GiB, so a dropped high dword
/// reads outside the map, and with room below 2^32 in its low dword.
const BASE: u64 = 0x1_2340_0000;
/// 16384 bytes: `MAX_UNIFORM_BLOCK_SIZE` under ES 3.0.
const BLOCK_DW: u32 = 4096;

fn block_word(k: u32) -> u32 {
    0xb10c_0000 ^ k.wrapping_mul(0x9e37_79b9)
}

/// The inputs: the block in memory at [`BASE`], `addr` in sh8:sh9, and
/// `runtime` (base, index, byte stride) in sh0..sh3.
fn inputs(addr: u64, runtime: Option<(u64, u32, u32)>) -> usc_exec::Inputs {
    let mut inp = usc_exec::sentinel_inputs(-7.0);
    inp.mem = vec![(BASE, (0..BLOCK_DW).map(block_word).collect())];
    inp.shared[8] = addr as u32;
    inp.shared[9] = (addr >> 32) as u32;
    if let Some((base, index, stride)) = runtime {
        inp.shared[..4].copy_from_slice(&[base as u32, (base >> 32) as u32, index, stride]);
    }
    inp
}

fn program(groups: &[&[u8]]) -> Vec<u8> {
    groups.concat()
}

fn run(groups: &[&[u8]], inp: &usc_exec::Inputs) -> Result<Vec<u32>, String> {
    let out = usc_exec::run_binary(&program(groups), inp, true)?;
    Ok(out.values().copied().collect())
}

/// The pinned `ld` words decode as `ld` in its immediate-burst form, with
/// the burst, DRC 0, the address in s0 and the destination in s3 they say.
#[test]
fn ld_words_decode() {
    let regbank = |o: yakogl::shaderc::online::powervr::operands::Operand| {
        let e = yakogl::shaderc::online::powervr::isa_tables::ENUMS.iter().find(|e| e.name == "regbank").unwrap();
        (e.name_of(o.bank).unwrap(), o.index)
    };
    for (bytes, burst, addr) in [
        (&LD1_SH8_T2[..], 1, ("shared", 8)),
        (&LD4_SH8_T2[..], 4, ("shared", 8)),
        (&LD1_T0_T2[..], 1, ("temp", 0)),
        (&LD4_T0_T2[..], 4, ("temp", 0)),
    ] {
        let g = decode_group(bytes, 0).unwrap_or_else(|e| panic!("{bytes:02x?}: {e:?}"));
        assert_eq!(g.total, bytes.len());
        let ins: Vec<_> = g.instrs().collect();
        assert_eq!(ins.len(), 1);
        let (_, ph, r) = ins[0];
        assert_eq!(ops_for(r, ph).next().map(|o| o.name), Some("ld"));
        assert_eq!(r.variant().name, "ld_immbl");
        assert_eq!(r.get("burstlen"), Some(burst));
        assert_eq!(r.get("drc"), Some(0));
        assert_eq!(r.get("srcseladd"), Some(0));
        assert_eq!(r.get("cachemode_ld"), Some(0));
        assert_eq!((g.hdr.get("w0p"), g.hdr.get("w1p"), g.hdr.get("rpt")), (Some(0), Some(0), Some(1)));
        assert!(g.dst.is_none());
        let lo = g.lower.as_ref().map(srcs).unwrap();
        let up = g.upper.as_ref().map(srcs).unwrap();
        assert_eq!(regbank(lo[0].unwrap()), addr);
        assert_eq!(regbank(up[3].unwrap()), ("temp", 2));
        /* the note's §1 backend-op word */
        let word: &[u8] = if burst == 1 { &[0xf1, 0x04, 0x00] } else { &[0xf1, 0x10, 0x00] };
        assert_eq!(&bytes[3..6], word);
    }
    let g = decode_group(&WDF, 0).unwrap();
    let names: Vec<_> = g.instrs().map(|(_, ph, r)| ops_for(r, ph).next().map(|o| o.name)).collect();
    assert_eq!(names, [Some("wdf")]);
    assert_eq!(g.hdr.get("miscctl"), Some(0));
}

/// One dword at a constant address: the block's first and its last.
#[test]
fn constant_address_first_and_last_dword() {
    for k in [0, BLOCK_DW - 1] {
        let got = run(&[&LD1_SH8_T2, &WDF, &PIX0_END], &inputs(BASE + 4 * u64::from(k), None)).unwrap();
        assert_eq!(got, [block_word(k)], "dword {k}");
    }
}

/// A vec4 (burst 4) at a constant address: the block's first and last.
#[test]
fn constant_address_first_and_last_vec4() {
    for k in [0, BLOCK_DW - 4] {
        let got = run(&[&LD4_SH8_T2, &WDF, &PIX[0], &PIX[1], &PIX[2], &PIX[3]], &inputs(BASE + 4 * u64::from(k), None))
            .unwrap();
        assert_eq!(got, (k..k + 4).map(block_word).collect::<Vec<_>>(), "vec4 at dword {k}");
    }
}

/// The address computed at run time, base + index * stride, into the pair
/// t0:t1: one dword at index 0 and 4095, a vec4 at index 0 and 1023.
#[test]
fn runtime_address_first_and_last() {
    let addr: [&[u8]; 5] = [&T1_SH1, &T4_SH2, &T5_SH3, &T6_SH0, &T0_IMADD];
    for index in [0, BLOCK_DW - 1] {
        let inp = inputs(0, Some((BASE, index, 4)));
        let got = run(&[&addr[..], &[&LD1_T0_T2, &WDF, &PIX0_END]].concat(), &inp).unwrap();
        assert_eq!(got, [block_word(index)], "dword index {index}");
    }
    for index in [0, BLOCK_DW / 4 - 1] {
        let inp = inputs(0, Some((BASE, index, 16)));
        let got = run(&[&addr[..], &[&LD4_T0_T2, &WDF, &PIX[0], &PIX[1], &PIX[2], &PIX[3]]].concat(), &inp).unwrap();
        assert_eq!(got, (4 * index..4 * index + 4).map(block_word).collect::<Vec<_>>(), "vec4 index {index}");
    }
}

/// A read of the loaded register with no `wdf` before it is the error the
/// interpreter reports: a missing wait shows.
#[test]
fn read_before_wdf_is_rejected() {
    let e = run(&[&LD1_SH8_T2, &PIX0_END], &inputs(BASE, None)).unwrap_err();
    assert!(e.contains("before the wdf on drc0"), "{e}");
    /* every lane of a burst is pending, not only the first */
    let e = run(&[&LD4_SH8_T2, &PIX[1], &PIX[3]], &inputs(BASE, None)).unwrap_err();
    assert!(e.contains("temp 3"), "{e}");
}

/// A read past the block's end, below its start, or with the high dword
/// lost, falls outside the map and is an error, not a value.
#[test]
fn read_outside_the_map_is_rejected() {
    for addr in [BASE + 16384, BASE - 4, BASE & 0xffff_ffff] {
        let e = run(&[&LD1_SH8_T2, &WDF, &PIX0_END], &inputs(addr, None)).unwrap_err();
        assert!(e.contains("outside every region"), "{addr:#x}: {e}");
    }
    /* a burst that starts inside and runs past the end */
    let e = run(&[&LD4_SH8_T2, &WDF, &PIX[0], &PIX[1], &PIX[2], &PIX[3]], &inputs(BASE + 16376, None)).unwrap_err();
    assert!(e.contains("outside every region"), "{e}");
    /* and an address that is not 4-byte aligned */
    let e = run(&[&LD1_SH8_T2, &WDF, &PIX0_END], &inputs(BASE + 2, None)).unwrap_err();
    assert!(e.contains("not 4-byte aligned"), "{e}");
}
