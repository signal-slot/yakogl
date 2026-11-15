// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `verify71` (src/shaderc/online/v3d/verify71.rs) against Mesa 26.2.1's own
//! qpu_validate at ver 71, as `tools/v3d/hazards71.sh` recorded it in
//! `tools/v3d/hazards71/oracle.tsv`: for each program of the table, decoded
//! from its 64-bit words with `decode71`, the verdict (ok or the first refused
//! instruction) and Mesa's message as the rule name must be Mesa's.
//!
//! The table has no Mesa input at test time; the script that wrote it
//! compares it with a fresh Mesa build (a named gap without
//! `MESA_V3D_TREE`). A line whose flags carry `z=` needs `is_tlb_z_write`,
//! which a word does not hold and `verify71` does not transcribe, so it is
//! counted and skipped.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_hazards71_oracle`
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::decode71;
use yakogl::shaderc::online::v3d::verify71::verify71;

const TABLE: &str = include_str!("../tools/v3d/hazards71/oracle.tsv");

/// Programs where `verify71` and Mesa's validator answer differently, with the
/// reason. A name here must still differ, so the list cannot rot. Found by this
/// table on 2026-11-14; none is a program Mesa's own compiler emits.
const KNOWN_DIFFERENCES: &[(&str, &str)] = &[
    // Mesa reports the end-of-program rules after its loop, at ip == the
    // instruction count (qpu_validate.c, the two checks after
    // vir_for_each_block); verify71 reports the last instruction's index.
    ("no_program_end", "same message, ip is len-1 where Mesa's is len"),
    ("thrsw_without_last_thrsw", "same message, ip is len-1 where Mesa's is len"),
    // The scheduler's own rules (marked "ours" in verify71.rs): Mesa's
    // scheduler never builds these, its validator does not refuse them.
    ("ok_ldvary_in_thrsw_slot_1", "verify71 refuses LDVARY in a thread-end slot (ours)"),
    ("ok_ldvary_in_thrsw_instruction", "verify71 refuses LDVARY in the THRSW instruction (ours)"),
    ("ok_recip_op_in_thrsw_slot", "verify71 refuses an SFU operation in a THRSW slot (ours)"),
    // Both refuse the same instruction; verify71 tests the reserved magic
    // address before the SFU-in-THRSW-slot rule, Mesa the other way round.
    ("sfu_magic_write_in_thrsw_slot", "same ip, rule order: reserved waddr first"),
];

struct Row<'a> {
    name: &'a str,
    last: bool,
    z: bool,
    words: Vec<u64>,
    ok: bool,
    ip: usize,
    msg: &'a str,
}

fn rows() -> Vec<Row<'static>> {
    TABLE
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 7, "malformed oracle line {l:?}");
            assert_eq!(f[1], "71", "{}: the oracle is ver 71", f[0]);
            let words = f[3]
                .split(',')
                .map(|w| u64::from_str_radix(w.trim_start_matches("0x"), 16).expect("hex word"))
                .collect();
            Row {
                name: f[0],
                last: f[2].split(',').any(|x| x == "last"),
                z: f[2].split(',').any(|x| x.starts_with("z=")),
                words,
                ok: f[4] == "ok",
                ip: f[5].parse().unwrap_or(0),
                msg: f[6],
            }
        })
        .collect()
}

/// What `verify71` says about a row: `None` is ok. A word `decode71` refuses
/// (a magic address Mesa's table does not name) never reaches `verify71`: the
/// answer is then the rule Mesa gives it, at that instruction, and the row
/// counts as refused by the decoder.
fn ours(r: &Row, by_decoder: &mut usize) -> Option<(usize, &'static str)> {
    let mut prog = Vec::new();
    for (ip, &w) in r.words.iter().enumerate() {
        match decode71(w) {
            Ok(i) => prog.push(i),
            Err(_) => {
                *by_decoder += 1;
                return Some((ip, "write to a reserved magic waddr"));
            }
        }
    }
    verify71(&prog, !r.last).err().map(|e| (e.ip, e.rule))
}

#[test]
fn verify71_gives_mesas_verdicts() {
    let rows = rows();
    assert_eq!(rows.len(), 64, "the table's size is pinned: a new case is a new pin");
    let (mut agree, mut skipped, mut known) = (0, 0, 0);
    let mut wrong = Vec::new();
    let (mut refused, mut by_decoder) = (0, 0);
    for r in &rows {
        if r.z {
            skipped += 1;
            continue;
        }
        if !r.ok {
            refused += 1;
        }
        let want = (!r.ok).then_some((r.ip, r.msg));
        let got = ours(r, &mut by_decoder);
        let same = match (want, got) {
            (None, None) => true,
            (Some((wi, wm)), Some((gi, gm))) => wi == gi && wm.trim() == gm.trim(),
            _ => false,
        };
        let listed = KNOWN_DIFFERENCES.iter().any(|(n, _)| *n == r.name);
        match (same, listed) {
            (true, false) => agree += 1,
            (false, true) => known += 1,
            (true, true) => wrong.push(format!("{}: listed as a difference, but agrees", r.name)),
            (false, false) => wrong.push(format!("{}: Mesa {want:?}, verify71 {got:?}", r.name)),
        }
    }
    assert!(wrong.is_empty(), "verify71 and Mesa differ:\n{}", wrong.join("\n"));
    println!("hazards71 oracle: {agree} agree, {known} known differences, {skipped} skipped (z), {refused} refused by Mesa ({by_decoder} of them by the decoder)");
    assert_eq!(agree + known + skipped, rows.len());
    assert!(refused >= 30, "the table must keep refusing programs");
}
