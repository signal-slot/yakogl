// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The decode-back gate must reject the image main shipped before it existed.
//!
//! At 9c0f78a, usc1_triangle reported "29 groups in, 29 built, 29 decode
//! back" for texturefill.batchable.vert. That check was VACUOUS: it compared
//! the decoded destination bank with the expected one translated by the same
//! `Bank as u32` cast the builder had encoded it with, so a wrong mapping
//! agreed with itself -- every destination was a special register -- and it
//! looked at no source, no output address and no ISS routing.
//!
//! `OLD_IMAGE` is that program: the 376 bytes `build_shader` produced at
//! 9c0f78a (dumped with `USC2_DUMP`, the only change a test-side write,
//! 2026-09-23 04:20). Each instrument of the current gate is run on it and
//! must say no, and on today's image for the same source must say yes -- an
//! instrument that cannot tell the two apart vouches for neither.

#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]

mod usc_exec;

use std::collections::BTreeSet;
use usc_exec::{run_binary, Inputs};
use yakogl::shaderc::online::powervr::chain::{build_vertex, LIMITS};
use yakogl::shaderc::online::powervr::decode::{decode_group, Group};
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::isa_tables::ENUMS;
use yakogl::shaderc::online::powervr::operands::dsts;
use yakogl::shaderc::online::powervr::verify::skeleton;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};

const NAME: &str = "texturefill.batchable.vert";

const OLD_IMAGE: [u32; 94] = [
    0x40008235, 0x0008c3dc, 0x82350000, 0xc3dd4000, 0x01000008, 0xc0008236, 0x000842d8, 0xff020000, 0xc0008236,
    0x010842d9, 0xff000000, 0xc0008236, 0x020841d4, 0xff010000, 0xc0008236, 0x000841d5, 0xff020000, 0xc0008236,
    0x010840d0, 0xff000000, 0xc0008236, 0x020840d1, 0xff010000, 0x80008234, 0x02000060, 0x40008235, 0x00008280,
    0x8234ff03, 0x00618000, 0x82350000, 0x80814000, 0xff020000, 0x40008235, 0x0008c3cc, 0x82350000, 0xc3cd4000,
    0x01000008, 0x40008235, 0x0008c3ce, 0x82350400, 0xc3cf4000, 0x05000008, 0xc0008236, 0x000842c8, 0xff060000,
    0xc0008236, 0x010842c9, 0xff000000, 0xc0008236, 0x040842ca, 0xff010000, 0xc0008236, 0x050842cb, 0xff040000,
    0xc0008236, 0x060841c4, 0xff050000, 0xc0008236, 0x000841c5, 0xff060000, 0xc0008236, 0x010841c6, 0xff000000,
    0xc0008236, 0x040841c7, 0xff010000, 0xc0008236, 0x050840c0, 0xff040000, 0xc0008236, 0x060840c1, 0xff050000,
    0xc0008236, 0x000840c2, 0xff060000, 0xc0008236, 0x010840c3, 0xff000000, 0x40008235, 0x0010c084, 0xa0550600,
    0xc3000800, 0x00000000, 0x0800a055, 0x0000c200, 0xa0550000, 0xc4000804, 0x00000000, 0x0800a055, 0x0000c000,
    0xa0470000, 0x00000580, 0xfff3ff00, 0xffffffff,
];

fn bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn groups(b: &[u8]) -> Vec<Group> {
    let mut v = Vec::new();
    let mut off = 0;
    while off < b.len() {
        let g = decode_group(b, off).expect("decodes");
        off += g.total;
        let end = g.hdr.get("end") == Some(1);
        v.push(g);
        if end {
            break;
        }
    }
    v
}

fn op(g: &Group) -> &'static str {
    g.instrs().next().and_then(|(_, ph, r)| ops_for(r, ph).next()).map(|o| o.name).unwrap_or("?")
}

/// Per group: its operation, the destination's bank BY THE ISA ENUM'S NAME,
/// and for an output write its address, repeat and ISS is4.
/// (operation, destination bank name, output write's (imm_addr, rpt, is4)).
type Fields = (&'static str, Option<&'static str>, Option<(u32, u32, u32)>);

fn fields(gs: &[Group]) -> Vec<Fields> {
    let regbank = ENUMS.iter().find(|e| e.name == "regbank").unwrap();
    gs.iter()
        .map(|g| {
            let d = g.dst.as_ref().and_then(|d| dsts(d)[0]).and_then(|o| regbank.name_of(o.bank));
            let w = (op(g) == "uvsw.write").then(|| {
                let r = g.instrs().next().unwrap().2;
                (
                    r.get("imm_addr").unwrap_or(u32::MAX),
                    g.hdr.get("rpt").unwrap_or(1),
                    g.iss.as_ref().and_then(|i| i.get("is4")).unwrap_or(9),
                )
            });
            (op(g), d, w)
        })
        .collect()
}

#[test]
fn the_gate_rejects_the_image_main_shipped_before_it_and_accepts_todays() {
    let row = SHC_SOURCE_TABLE.iter().find(|r| r.name == NAME).expect("the source");
    let text = row.text.trim_end_matches('\0');
    let key = yakogl::shaderc::sha256::sha256(text.as_bytes());
    let blob = SHC_BLOB_TABLE.iter().find(|b| b.sha256 == key && b.blend_key == 0).expect("pco's blob");
    // SAFETY:  points at a static of the generated table.
    let pco = bytes(unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) });
    let old = bytes(&OLD_IMAGE);
    let new = store::with_scratch(|a| {
        let t = a.alloc_str(text).unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, t).expect("front end");
        let mut img = vec![0u8; 8192];
        let b = build_vertex(&l.module, LIMITS, a, &mut img).expect("builds");
        img.truncate(b.len);
        img
    })
    .expect("scratch");

    let (corpus, _) = usc_exec::corpus_skeletons();
    let unattested = |b: &[u8]| -> usize {
        groups(b)
            .iter()
            .filter(|g| {
                let mut k = String::new();
                skeleton(&mut k, g).unwrap();
                !corpus.contains(&k)
            })
            .count()
    };

    /* 1. Field by field against pco's program for the same source. The
     *    programs differ in length, so the comparison is per KIND of group:
     *    the banks every arithmetic destination names, and the output
     *    writes' (address, repeat, is4). */
    let dst_banks = |b: &[u8]| -> BTreeSet<&'static str> {
        fields(&groups(b)).iter().filter(|(o, _, _)| *o != "uvsw.write").filter_map(|(_, d, _)| *d).collect()
    };
    let is4 =
        |b: &[u8]| -> BTreeSet<u32> { fields(&groups(b)).iter().filter_map(|(_, _, w)| w.map(|w| w.2)).collect() };
    let dwords = |b: &[u8]| -> Vec<u32> {
        let mut v: Vec<u32> =
            fields(&groups(b)).iter().filter_map(|(_, _, w)| *w).flat_map(|(a, r, _)| a..a + r).collect();
        v.sort();
        v
    };
    println!("dst banks: old {:?} new {:?} pco {:?}", dst_banks(&old), dst_banks(&new), dst_banks(&pco));
    println!("output dwords: old {:?} new {:?} pco {:?}", dwords(&old), dwords(&new), dwords(&pco));
    println!("uvsw.write is4: old {:?} new {:?} pco {:?}", is4(&old), is4(&new), is4(&pco));
    assert_eq!(dst_banks(&pco), BTreeSet::from(["temp"]), "the reference's destinations are temporaries");
    assert_ne!(dst_banks(&old), dst_banks(&pco), "the old image's destination banks pass");
    assert_eq!(dst_banks(&new), dst_banks(&pco));
    assert_ne!(dwords(&old), dwords(&pco), "the old image's output addresses pass");
    assert_eq!(dwords(&new), dwords(&pco));
    assert_ne!(is4(&old), is4(&pco), "the old image's ISS routing passes");
    assert_eq!(is4(&new), is4(&pco));

    /* 2. Skeletons against the whole corpus. */
    let (u_old, u_new) = (unattested(&old), unattested(&new));
    println!("groups whose skeleton pco never emitted: old {u_old}, new {u_new}");
    assert!(u_old > 0, "the old image's wiring passes the corpus check");
    assert_eq!(u_new, 0);

    /* 3. Execution against pco's bytes. texturefill's two attributes and its
     *    uniform block sit at the same registers in both programs (vi_reg 0
     *    and 4, block offset = shared index), so one input set serves both. */
    for seed in 1..=3 {
        let inp = Inputs::new(seed);
        let theirs = run_binary(&pco, &inp, false).expect("pco's program runs");
        assert_eq!(run_binary(&new, &inp, true).expect("ours runs"), theirs, "today's image differs from pco's");
        let o = run_binary(&old, &inp, true);
        println!("seed {seed}: old image {:?}", o.as_ref().map(|m| m.len()));
        assert!(o.map_or(true, |m| m != theirs), "the old image computes pco's output");
    }
}
