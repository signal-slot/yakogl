// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `verify71` (src/shaderc/online/v3d/verify71.rs) against Mesa 26.2.1's own
//! `qpu_validate()` verdicts at V3D 7.1, the table `tools/v3d/hazards71.tsv`
//! (written by `tools/v3d/hazards71.sh`; its format is in the header of
//! `tools/v3d/hazards71.c`).
//!
//! Each row's words are decoded with `codec71::decode71` and run through
//! `verify71(&prog, single_seg)`: `single_seg` is false for a row that starts
//! with `multi` (Mesa's `c->last_thrsw` set) and true otherwise. verify71 must
//! accept every row Mesa accepts and reject every row Mesa rejects. The rows
//! that cannot be compared (`-`: Mesa's packer refused the sequence, or no
//! word can carry it) are counted apart.
//!
//! A real disagreement is a named entry of `EXCEPTIONS` with the Mesa rule
//! (qpu_validate.c, 26.2.1) and a verdict: "deliberate" (verify71 keeps a rule
//! of Mesa's scheduler or leaves a validator rule out on purpose) or "defect"
//! (a bug in verify71). The test fails when the set of disagreements differs
//! from `EXCEPTIONS` in either direction.
//!
//! Features: `--no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table`.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::v3d::codec71::decode71;
use yakogl::shaderc::online::v3d::verify71::verify71;

/// (case name, why verify71 and Mesa differ). Pinned: the test fails if a
/// disagreement is not listed or a listed one agrees.
///
/// All are deliberate differences; none is a verify71 defect. verify71 keeps
/// rules of Mesa's scheduler (qpu_schedule.c, marked "ours" in its header)
/// that qpu_validate.c does not check, and leaves the TLB-Z rules out.
const EXCEPTIONS: &[(&str, &str)] = &[
    // qpu_schedule.c:716-724, 1692-1744 (rf0 is the ldvary's implicit write;
    // qpu_validate.c has no rule for it).
    ("rf0.ldvary_then_explicit_rf0_write_ok", "ours: rf0 written right after a LDVARY"),
    // qpu_schedule.c: no SFU op in a THRSW delay slot; qpu_validate.c:290-296
    // counts only magic SFU writes (19..24), reserved at 7.1 (:104-115).
    ("sfu.alu_recip_in_thrsw_slot_ok", "ours: SFU operation in a THRSW delay slot"),
    // qpu_validate.c:299-306 allows LDVARY in the first delay slot and in the
    // THRSW itself; the scheduler never builds it at the thread end.
    ("thrsw.ldvary_slot1_ok", "ours: LDVARY in a thread-end slot"),
    ("thrsw.ldvary_in_thrsw_instruction_ok", "ours: LDVARY in a thread-end slot"),
    // qpu_validate.c:380-419 checks rf2/rf3 *writes* after THREND only; the
    // scheduler's rule also refuses an rf2/rf3 *read* in the thread-end slots
    // (the rows read rf2/rf3 as operand registers of their filler words).
    ("thrend.magic_write_in_thrsw_ok", "ours: RF2-3 read after THREND"),
    ("thrend.add_rf5_slot1_ok_71", "ours: RF2-3 read after THREND"),
    ("thrend.mul_rf5_slot2_ok_71", "ours: RF2-3 read after THREND"),
    ("rtop.umul24_in_slot2_ok", "ours: RF2-3 read after THREND"),
    // qpu_validate.c:142-155, 187-197 need qinst::is_tlb_z_write, which a
    // word does not carry; verify71 (as verify.rs) leaves them out.
    ("tlbz.msf_after_rejected", "TLB-Z rules left out (:142-155)"),
    ("tlbz.setmsf_after_rejected", "TLB-Z rules left out (:142-155)"),
    ("tlbz.msf_branch_after_rejected", "TLB-Z rules left out (:187-197)"),
    // qpu_validate.c:118-134 does not look at a NOP op's waddr; codec71
    // refuses a word whose magic waddr it cannot name, before verify71.
    ("waddr.nop_op_reserved_waddr_ok", "codec71 refuses an unnamed magic waddr"),
];

struct Row {
    multi: bool,
    words: Vec<u64>,
    has_z: bool,
    verdict: String,
    name: String,
}

fn parse(text: &str) -> (Vec<Row>, usize) {
    let mut rows = Vec::new();
    let mut uncomparable = 0;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 3, "bad row: {line}");
        if f[0] == "-" {
            uncomparable += 1;
            continue;
        }
        let mut multi = false;
        let mut has_z = false;
        let mut words = Vec::new();
        for t in f[0].split(' ') {
            if t == "multi" {
                multi = true;
                continue;
            }
            let (hex, z) = match t.strip_suffix("/z") {
                Some(h) => (h, true),
                None => (t, false),
            };
            has_z |= z;
            words.push(u64::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad word {t}")));
        }
        rows.push(Row { multi, words, has_z, verdict: f[1].to_string(), name: f[2].to_string() });
    }
    (rows, uncomparable)
}

#[test]
fn verify71_agrees_with_mesa_qpu_validate() {
    let text = include_str!("../tools/v3d/hazards71.tsv");
    let (rows, uncomparable) = parse(text);
    assert!(rows.len() >= 80, "table too short: {}", rows.len());
    let (mut agree, mut agree_ip, mut undecodable) = (0, 0, 0);
    // A sequence codec71 cannot decode never reaches verify71: it counts as
    // refused (the back end cannot emit it).
    let mut disagree: Vec<String> = Vec::new();
    for r in &rows {
        let mesa_ok = r.verdict == "ok";
        let prog: Result<Vec<_>, _> = r.words.iter().map(|&w| decode71(w)).collect();
        let ours = match prog {
            Ok(p) => verify71(&p, !r.multi).map_err(|e| (e.ip, e.rule.to_string())),
            Err(e) => {
                undecodable += 1;
                Err((usize::MAX, format!("codec71: {e:?}")))
            }
        };
        if ours.is_ok() == mesa_ok {
            agree += 1;
            if let Err(e) = &ours {
                let mesa_ip = r.verdict.strip_prefix("ip ").and_then(|s| s.split(':').next());
                if mesa_ip == Some(e.0.to_string().as_str()) {
                    agree_ip += 1;
                }
            }
        } else {
            eprintln!(
                "DISAGREE {}{}: mesa `{}`, verify71 {:?}",
                r.name,
                if r.has_z { " (TLB-Z mark)" } else { "" },
                r.verdict,
                ours
            );
            disagree.push(r.name.clone());
        }
    }
    println!(
        "verify71 vs hazards71: {} rows, {agree} agree ({agree_ip} rejections at Mesa's ip), \
         {} disagree, {undecodable} refused by codec71, {uncomparable} not comparable (pack refused / raw)",
        rows.len(),
        disagree.len(),
    );
    let mut want: Vec<&str> = EXCEPTIONS.iter().map(|(n, _)| *n).collect();
    want.sort_unstable();
    disagree.sort();
    assert_eq!(disagree, want, "the set of disagreements with Mesa changed");
    assert_eq!(undecodable, 4, "rows codec71 refuses to decode");
}
