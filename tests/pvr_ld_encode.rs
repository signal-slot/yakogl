// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR online compiler builds, encodes, decodes and round-trips the
//! USC global memory load `ld` the way Mesa 26.2.1's pco encodes it:
//! `assemble::build_ld`, both burst-length forms (`ld`, form `ld_immbl`;
//! `ld.regbl`, form `ld_regbl`), the three cache modes, both data-return
//! channels, the header modifiers, and the address pair, burst register and
//! destination in source slots s0, s1 and s3 (an `idx0` destination
//! included). Plan #378 item `ld-encode`; the design is
//! `docs/notes/pvr-ubo-memory-load.md` §1. HOST-ONLY.
//!
//! What pco's output is compared with: the `ld.regbl` group of Mesa's own
//! end-of-tile programs for a spilled target (`eot2_tb`, `eot3_tb`,
//! `eot4_tb`, `eot4_tb2`, `eot4_tb3` in `tools/pco_shc/usc_blobs.txt`,
//! captured from pco by `capture_usc`; `trans_flush_tile_buffer`). The
//! builder makes that group byte for byte. `PCO_SHC` is not used: `pco_shc`
//! compiles no shader that reaches an `ld` (a uniform or storage block
//! asserts in `set_resource_used`, it gives no scratch range for
//! `load_scratch`, and a `buffer_reference` load faults), so the
//! immediate-burst words are pinned from Mesa's tables instead, with the
//! derivation beside them. pco's offline table holds no `ld` either.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ld_encode
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]
use yakogl::shaderc::online::powervr::assemble::{build_ld, BuildError, Ld, LdBurst, LdCache};
use yakogl::shaderc::online::powervr::decode::{decode_group, Group};
use yakogl::shaderc::online::powervr::encode::encode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::listing::group_text;
use yakogl::shaderc::online::powervr::operands::{srcs, Operand};

/* `regbank` (isa_tables.rs ENUMS) */
const SPECIAL: u32 = 0;
const TEMP: u32 = 1;
const SHARED: u32 = 4;
const IDX0: u32 = 6;

fn op(bank: u32, index: u32) -> Operand {
    Operand { bank, index }
}

fn bytes_of(g: &Group) -> Vec<u8> {
    let mut out = vec![0u8; g.total];
    assert_eq!(encode_group(g, &mut out, 0), Ok(g.total));
    out
}

fn built(ld: &Ld) -> Vec<u8> {
    bytes_of(&build_ld(ld).unwrap_or_else(|e| panic!("{ld:?}: {e:?}")))
}

/// The one operation of a decoded group and its form.
fn the_op(g: &Group) -> (&'static str, &'static str) {
    let ins: Vec<_> = g.instrs().collect();
    assert_eq!(ins.len(), 1);
    let (_, ph, r) = ins[0];
    (ops_for(r, ph).next().map_or("?", |o| o.name), r.variant().name)
}

/// The words of one program of `tools/pco_shc/usc_blobs.txt`, as bytes.
fn pco_program(name: &str) -> Vec<u8> {
    let txt = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/pco_shc/usc_blobs.txt"))
        .expect("tools/pco_shc/usc_blobs.txt");
    let line = txt.lines().find(|l| l.starts_with(&format!("{name} |"))).unwrap_or_else(|| panic!("{name}"));
    let words = line.split("words ").nth(1).and_then(|w| w.split(" |").next()).expect("a words column");
    words
        .split_whitespace()
        .flat_map(|w| u32::from_str_radix(w.trim_start_matches("0x"), 16).expect("a hex word").to_le_bytes())
        .collect()
}

/// pco's `ld.regbl` group in its tile-buffer flush (`trans_flush_tile_buffer`,
/// pco_trans_nir.c:726-773):
///
/// ```text
/// 56 a0 00   header: main, da 5, length 6, oporg be, no write port, rpt 1
/// e1 04      ld_regbl: dma, immbl 0, drc 0, ld; srcseladd s0, srcselbl s1, normal
/// c1 80      lower 2lo_1b6i_1b5i: s0 = t1 (the add64_32 pair t1:t2), s1 = sp0 (0 = 1024 dwords)
/// 87 0c 00   upper 1up_3b11i: s3 = idx0 7 (pixout[idx0 + 0])
/// 00 ff      ISS, word pad
/// ```
const PCO_TB_LD: [u8; 12] = [0x56, 0xa0, 0x00, 0xe1, 0x04, 0xc1, 0x80, 0x87, 0x0c, 0x00, 0x00, 0xff];

fn tb_ld() -> Ld {
    Ld { burst: LdBurst::Reg(op(SPECIAL, 0)), ..Ld::imm(op(IDX0, 7), op(TEMP, 1), 1) }
}

/// Every `ld` in Mesa's end-of-tile programs is the group above, and the
/// builder makes it byte for byte.
#[test]
fn ld_regbl_is_pcos_tile_buffer_load() {
    let mut seen = 0;
    for name in ["eot2_tb", "eot3_tb", "eot4_tb", "eot4_tb2", "eot4_tb3"] {
        let bytes = pco_program(name);
        let mut off = 0;
        while off < bytes.len() {
            let g = decode_group(&bytes, off).unwrap_or_else(|e| panic!("{name} @{off}: {e:?}"));
            if the_op(&g).0.starts_with("ld") {
                assert_eq!(&bytes[off..off + g.total], &PCO_TB_LD, "{name} @{off}");
                seen += 1;
            }
            off += g.total;
        }
    }
    /* one load per spilled target: 1 + 1 + 1 + 2 + 3 */
    assert_eq!(seen, 8, "one tile-buffer load per spilled target");
    assert_eq!(built(&tb_ld()), PCO_TB_LD);
}

/* The immediate-burst form, pinned from Mesa's tables (pco_isa.py
 * I_LD_IMMBL :2380-2394 over the I_BACKEND fields :1914-1926, types
 * :2027-2044; group_map(O_LD) pco_map.py:3301-3316). The group is pco's
 * ld.regbl group above with a 3-byte backend word and no s1:
 *
 *   header   da = 3 + 3, length in words, oporg be, rpt 1 (0x66 or 0x65, a0, 00)
 *   backend  byte 0: backend_op dma 0b111 | immbl 1 | drc | dma_op ld 0b001
 *                    = 0xf1 (drc 0), 0xf9 (drc 1)
 *            byte 1: srcseladd s0 (7:5) | burstlen bits 2:0 (4:2) | cachemode_ld (1:0)
 *            byte 2: rsvd2_ld 0 (7:1) | burstlen bit 3 (0)
 *            burstlen is F_UINT4_POS_WRAP: 1..16, 16 written as 0
 *            cachemode_ld: normal 0, bypass 1, force_line_fill 2
 *   lower    s0 = the address pair's first register (sh8: 88 08 00; t0: 40)
 *   upper    s3 = the destination (t2: 42)
 *   ISS 00, then the word pad ff when the body is odd.
 *
 * The whole group from sh8 into t2 is SH8_HDR, the backend word, SH8_TAIL;
 * the first two rows make tests/usc_exec_ld.rs's LD1_SH8_T2 and LD4_SH8_T2. */
const SH8_HDR: [u8; 3] = [0x66, 0xa0, 0x00];
const SH8_TAIL: [u8; 6] = [0x88, 0x08, 0x00, 0x42, 0x00, 0xff];
/// (burst, drc, cache mode, backend word)
const IMM: &[(u8, u8, LdCache, [u8; 3])] = &[
    (1, 0, LdCache::Normal, [0xf1, 0x04, 0x00]),
    (4, 0, LdCache::Normal, [0xf1, 0x10, 0x00]),
    (7, 0, LdCache::Normal, [0xf1, 0x1c, 0x00]),
    (8, 0, LdCache::Normal, [0xf1, 0x00, 0x01]),
    (16, 0, LdCache::Normal, [0xf1, 0x00, 0x00]),
    (5, 1, LdCache::Bypass, [0xf9, 0x15, 0x00]),
    (12, 0, LdCache::ForceLineFill, [0xf1, 0x12, 0x01]),
];

fn sh8_group(word: [u8; 3]) -> Vec<u8> {
    [&SH8_HDR[..], &word, &SH8_TAIL].concat()
}

/// The same loads from a temporary pair (pco always adds the offset into
/// temporaries first): a 1-byte lower section, so a 10-byte group. The
/// first two are tests/usc_exec_ld.rs's LD1_T0_T2 and LD4_T0_T2.
const IMM_T0: &[(u8, [u8; 10])] = &[
    (1, [0x65, 0xa0, 0x00, 0xf1, 0x04, 0x00, 0x40, 0x42, 0x00, 0xff]),
    (4, [0x65, 0xa0, 0x00, 0xf1, 0x10, 0x00, 0x40, 0x42, 0x00, 0xff]),
    (16, [0x65, 0xa0, 0x00, 0xf1, 0x00, 0x00, 0x40, 0x42, 0x00, 0xff]),
];

#[test]
fn ld_immbl_words_are_mesas() {
    for &(burst, drc, cache, word) in IMM {
        let ld = Ld { drc, cache, ..Ld::imm(op(TEMP, 2), op(SHARED, 8), burst) };
        assert_eq!(built(&ld), sh8_group(word), "ld drc{drc}, {burst}, sh8 -> t2, {cache:?}");
    }
    for &(burst, want) in IMM_T0 {
        assert_eq!(built(&Ld::imm(op(TEMP, 2), op(TEMP, 0), burst)), want, "ld drc0, {burst}, t0 -> t2");
    }
}

/// `olchk`, `cc`, `end` and `atom` land in the header and nowhere else:
/// olchk byte 1 bit 3, cc's low bit byte 1 bit 0 (`if(p0)` = 1), end byte 2
/// bit 7, atom byte 2 bit 3.
#[test]
fn ld_header_modifiers() {
    let base = Ld::imm(op(TEMP, 2), op(SHARED, 8), 1);
    let ld = Ld { olchk: true, cc: 1, end: true, atom: true, ..base };
    let mut want = sh8_group(IMM[0].3);
    want[1] = 0xa9;
    want[2] = 0x88;
    assert_eq!(built(&ld), want);
    let g = decode_group(&want, 0).unwrap();
    for (f, v) in [("olchk", 1), ("cc", 1), ("end", 1), ("atom", 1), ("rpt", 1), ("w0p", 0), ("w1p", 0)] {
        assert_eq!(g.hdr.get(f), Some(v), "{f}");
    }
}

/// Every form decodes to what it was built from, re-encodes byte for byte,
/// and lists as `ld` / `ld.regbl` with its fields.
#[test]
fn ld_decodes_lists_and_round_trips() {
    let regbl_t5 = Ld {
        burst: LdBurst::Reg(op(TEMP, 5)),
        cache: LdCache::Bypass,
        drc: 1,
        ..Ld::imm(op(TEMP, 2), op(SHARED, 300), 1)
    };
    let mut all: Vec<Ld> = vec![tb_ld(), regbl_t5];
    for burst in 1..=16u8 {
        for cache in [LdCache::Normal, LdCache::Bypass, LdCache::ForceLineFill] {
            for addr in [op(SHARED, 8), op(TEMP, 0), op(SHARED, 2046)] {
                all.push(Ld { cache, drc: burst & 1, ..Ld::imm(op(TEMP, 2), addr, burst) });
            }
        }
    }
    for ld in &all {
        let bytes = built(ld);
        let g = decode_group(&bytes, 0).unwrap_or_else(|e| panic!("{ld:?} {bytes:02x?}: {e:?}"));
        assert_eq!(g.total, bytes.len());
        assert_eq!(bytes_of(&g), bytes, "{ld:?}");
        let (name, form) = the_op(&g);
        let r = g.instrs().next().unwrap().2;
        assert_eq!(r.get("drc"), Some(ld.drc as u32));
        assert_eq!(r.get("srcseladd"), Some(0));
        let cache = match ld.cache {
            LdCache::Normal => 0,
            LdCache::Bypass => 1,
            LdCache::ForceLineFill => 2,
        };
        assert_eq!(r.get("cachemode_ld"), Some(cache));
        let lo = srcs(g.lower.as_ref().unwrap());
        let up = srcs(g.upper.as_ref().unwrap());
        assert_eq!(lo[0], Some(ld.addr));
        assert_eq!(up[3], Some(ld.dst));
        assert!(g.dst.is_none(), "the backend writes the destination itself");
        match ld.burst {
            LdBurst::Imm(n) => {
                assert_eq!((name, form), ("ld", "ld_immbl"));
                assert_eq!(r.get("burstlen"), Some(n as u32));
            }
            LdBurst::Reg(b) => {
                assert_eq!((name, form), ("ld.regbl", "ld_regbl"));
                assert_eq!(r.get("srcselbl"), Some(1));
                assert_eq!(lo[1], Some(b));
            }
        }
        let mut t = String::new();
        group_text(&mut t, 0, &g).unwrap();
        assert!(t.contains(&format!(" {name}\n")), "{t}");
        assert!(t.contains("oporg=be"), "{t}");
    }
}

/// What `ld` cannot encode is refused, not truncated.
#[test]
fn ld_out_of_range_is_refused() {
    for burst in [0u8, 17, 255] {
        assert!(
            matches!(build_ld(&Ld::imm(op(TEMP, 2), op(TEMP, 0), burst)), Err(BuildError::Unsupported(_))),
            "burst {burst}"
        );
    }
    let drc2 = Ld { drc: 2, ..Ld::imm(op(TEMP, 2), op(TEMP, 0), 1) };
    assert!(matches!(build_ld(&drc2), Err(BuildError::Unsupported(_))));
}
