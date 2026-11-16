// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `verify71` (src/shaderc/online/v3d/verify71.rs) against Mesa's own
//! verdicts: `tools/v3d/hazards71.tsv`, one line per hand-chosen 7.1
//! instruction sequence with the verdict of Mesa 26.2.1's `qpu_validate()`
//! at devinfo ver 71 (`tools/v3d/hazards71.c`, `tools/v3d/README.md`).
//!
//! Each row's words are decoded with codec71 and the program run through
//! `verify71` with `single_seg` from the row's `multi` token (Mesa's
//! `c->last_thrsw`). The test asks that verify71 accepts every sequence Mesa
//! accepts and rejects every sequence Mesa rejects, at the same instruction
//! and with Mesa's message as the rule (verify71 keeps Mesa's messages as its
//! rule names). The two end-of-program rules are reported by Mesa at the
//! instruction count and by verify71 at the last instruction; that is the one
//! difference in convention the comparison normalises.
//!
//! What does not agree is pinned by name in [`EXCEPTIONS`], each with the
//! reason and the Mesa rule it comes from. The test fails if the set of
//! exceptions changes in either direction: a new disagreement, or a pinned
//! one that now agrees (then its entry comes out). The kinds:
//!
//! * `MesaRejectsWeAccept`: a verify71 gap (a rule it does not transcribe);
//! * `MesaAcceptsWeReject`: a deliberate difference (verify71 also keeps the
//!   rules of Mesa's scheduler, marked "(ours)", which its validator never
//!   checks because the scheduler never builds what they refuse);
//! * `Undecodable`: codec71 refuses a word of the row, so verify71 never sees
//!   it. Where Mesa rejects the row too, the pipeline agrees on the verdict
//!   (the word is refused one stage earlier); where Mesa accepts it, that is
//!   a codec71 gap;
//! * `NoWords`: the row's words are `-` (Mesa's packer refused the sequence,
//!   or the fields were validated unpacked), so there is nothing to decode.
//!
//! Result on hazards71.tsv's 90 rows: 71 agree, 19 exceptions (3 TLB-Z gaps,
//! 8 deliberate scheduler rules, 4 words codec71 refuses, 4 rows without
//! words). No rejected row disagrees on the instruction or the rule.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_verify_oracle -- --nocapture
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-v3d", feature = "shaderc-online", feature = "no-shader-table"))]

use std::collections::BTreeSet;
use yakogl::shaderc::online::v3d::codec71::decode71;
use yakogl::shaderc::online::v3d::isa71::Instr71;
use yakogl::shaderc::online::v3d::verify71::verify71;

static HAZARDS_71: &str = include_str!("../tools/v3d/hazards71.tsv");

/// Mesa's verdict for a row.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mesa {
    Ok,
    Rejected {
        ip: usize,
        msg: String,
    },
    /// "pack refused": Mesa's packer has no word for the sequence.
    PackRefused,
}

/// One row of the table.
#[derive(Debug)]
struct Case {
    name: String,
    /// Validated as a multi-segment program (`c->last_thrsw` set).
    multi: bool,
    /// The words, with the `/z` mark (`is_tlb_z_write`); `None` for `-`.
    words: Option<Vec<(u64, bool)>>,
    verdict: Mesa,
}

fn parse() -> Vec<Case> {
    let mut out = Vec::new();
    for (n, line) in HAZARDS_71.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3, "hazards71.tsv line {}: {} columns", n + 1, cols.len());
        let mut toks = cols[0].split(' ').filter(|t| !t.is_empty()).peekable();
        let multi = toks.peek() == Some(&"multi");
        if multi {
            toks.next();
        }
        let toks: Vec<&str> = toks.collect();
        let words = if toks == ["-"] {
            None
        } else {
            Some(
                toks.iter()
                    .map(|t| {
                        let (hex, z) = match t.strip_suffix("/z") {
                            Some(h) => (h, true),
                            None => (*t, false),
                        };
                        let w = u64::from_str_radix(hex, 16)
                            .unwrap_or_else(|e| panic!("hazards71.tsv line {}: word {t}: {e}", n + 1));
                        (w, z)
                    })
                    .collect(),
            )
        };
        let verdict = match cols[1] {
            "ok" => Mesa::Ok,
            "pack refused" => Mesa::PackRefused,
            v => {
                let rest =
                    v.strip_prefix("ip ").unwrap_or_else(|| panic!("hazards71.tsv line {}: verdict {v:?}", n + 1));
                let (ip, msg) =
                    rest.split_once(": ").unwrap_or_else(|| panic!("hazards71.tsv line {}: verdict {v:?}", n + 1));
                Mesa::Rejected { ip: ip.parse().unwrap(), msg: msg.to_string() }
            }
        };
        out.push(Case { name: cols[2].to_string(), multi, words, verdict });
    }
    out
}

/// What our pipeline says of a row.
enum Ours {
    Accepted,
    Rejected { ip: usize, rule: &'static str },
    Undecodable { ip: usize, err: String },
    NoWords,
}

impl std::fmt::Display for Ours {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ours::Accepted => write!(f, "accepted"),
            Ours::Rejected { ip, rule } => write!(f, "rejected at {ip}: {rule}"),
            Ours::Undecodable { ip, err } => write!(f, "word {ip} undecodable: {err}"),
            Ours::NoWords => write!(f, "no words"),
        }
    }
}

fn run(case: &Case) -> Ours {
    let Some(words) = &case.words else { return Ours::NoWords };
    let mut prog: Vec<Instr71> = Vec::with_capacity(words.len());
    for (ip, (w, _z)) in words.iter().enumerate() {
        match decode71(*w) {
            Ok(i) => prog.push(i),
            Err(e) => return Ours::Undecodable { ip, err: format!("{e:?}") },
        }
    }
    match verify71(&prog, !case.multi) {
        Ok(()) => Ours::Accepted,
        Err(e) => Ours::Rejected { ip: e.ip, rule: e.rule },
    }
}

/// The ways a row can disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    MesaRejectsWeAccept,
    MesaAcceptsWeReject,
    /// Both reject, at another instruction or with another rule.
    DifferentRule,
    Undecodable,
    NoWords,
}

/// Mesa's two end-of-program rules report the instruction count as the ip
/// (qpu_validate.c:478-484, after the loop); verify71 reports the last
/// instruction.
const END_RULES: [&str; 2] = ["thread switch found without last-THRSW in program", "No program-end THRSW found"];

fn classify(case: &Case, ours: &Ours) -> Option<Kind> {
    let len = case.words.as_ref().map_or(0, |w| w.len());
    match (&case.verdict, ours) {
        (_, Ours::NoWords) => Some(Kind::NoWords),
        (_, Ours::Undecodable { .. }) => Some(Kind::Undecodable),
        (Mesa::Ok, Ours::Accepted) => None,
        (Mesa::Ok, Ours::Rejected { .. }) => Some(Kind::MesaAcceptsWeReject),
        (Mesa::Rejected { .. }, Ours::Accepted) => Some(Kind::MesaRejectsWeAccept),
        (Mesa::Rejected { ip, msg }, Ours::Rejected { ip: our_ip, rule }) => {
            let same_ip = ip == our_ip || (END_RULES.contains(&msg.as_str()) && *ip == len && *our_ip + 1 == len);
            (!(same_ip && msg == rule)).then_some(Kind::DifferentRule)
        }
        (Mesa::PackRefused, _) => unreachable!("a pack-refused row has words"),
    }
}

/// The pinned disagreements: (case name, kind, why). Mesa line numbers are
/// Mesa 26.2.1's `src/broadcom/compiler/qpu_validate.c` unless named.
const EXCEPTIONS: &[(&str, Kind, &str)] = &[
    // verify71 gap: the three TLB-Z rules (qpu_validate.c:142-155 the
    // branch's implicit MSF read, :186-190 SETMSF, :192-196 MSF) need Mesa's
    // `is_tlb_z_write` mark on the qinst, which a word does not carry and
    // verify71's interface has no argument for (its header says so). The
    // table carries the mark as `/z`; a verify71 that takes the marks would
    // close this gap. `tlbz.msf_before_ok` and `tlbz.plain_branch_after_ok`
    // agree because nothing after the Z write reads MSF.
    ("tlbz.msf_after_rejected", Kind::MesaRejectsWeAccept, "qpu_validate.c:192-196, needs is_tlb_z_write"),
    ("tlbz.setmsf_after_rejected", Kind::MesaRejectsWeAccept, "qpu_validate.c:186-190, needs is_tlb_z_write"),
    ("tlbz.msf_branch_after_rejected", Kind::MesaRejectsWeAccept, "qpu_validate.c:142-155, needs is_tlb_z_write"),
    // Deliberate: Mesa's scheduler rules verify71 keeps ("(ours)"), which
    // qpu_validate() does not check. qpu_inst_valid_in_thrend_slot
    // (qpu_schedule.c:1979-2060) refuses, at ver >= 71, an LDVARY in the
    // thread-end THRSW or its two slots (:1993) and a read of rf2/rf3 there
    // (:2047-2048). hazards71.c's `add`/`mul` builders read rf1 and rf2, and
    // its single-segment programs have the thread end as their only THRSW,
    // so these rows are inside that window.
    ("thrsw.ldvary_slot1_ok", Kind::MesaAcceptsWeReject, "qpu_schedule.c:1993, LDVARY in a thread-end slot"),
    (
        "thrsw.ldvary_in_thrsw_instruction_ok",
        Kind::MesaAcceptsWeReject,
        "qpu_schedule.c:1993, LDVARY in the thread-end THRSW",
    ),
    (
        "thrend.magic_write_in_thrsw_ok",
        Kind::MesaAcceptsWeReject,
        "qpu_schedule.c:2047-2048, reads rf2 in the thread-end THRSW",
    ),
    (
        "thrend.add_rf5_slot1_ok_71",
        Kind::MesaAcceptsWeReject,
        "qpu_schedule.c:2047-2048, reads rf2 in thread-end slot 1",
    ),
    (
        "thrend.mul_rf5_slot2_ok_71",
        Kind::MesaAcceptsWeReject,
        "qpu_schedule.c:2047-2048, reads rf2 in thread-end slot 2",
    ),
    (
        "rtop.umul24_in_slot2_ok",
        Kind::MesaAcceptsWeReject,
        "qpu_schedule.c:2047-2048, UMUL24 reads rf2 in thread-end slot 2",
    ),
    // Deliberate: qpu_inst_before_thrsw_valid_in_delay_slot
    // (qpu_schedule.c:2070-2084, :2082 at ver >= 71) refuses an SFU
    // operation in a THRSW delay slot: its result would land in the other
    // thread. The validator only counts 4.2's magic SFU writes (:290-296).
    ("sfu.alu_recip_in_thrsw_slot_ok", Kind::MesaAcceptsWeReject, "qpu_schedule.c:2082, SFU op in THRSW delay slot 1"),
    // Deliberate: writes_too_soon_after_write (qpu_schedule.c:699-726, :721
    // at ver >= 71) refuses an explicit rf0 write right after an LDVARY,
    // whose delayed rf0 write lands then. Not a validator rule.
    (
        "rf0.ldvary_then_explicit_rf0_write_ok",
        Kind::MesaAcceptsWeReject,
        "qpu_schedule.c:721, rf0 written right after LDVARY",
    ),
    // codec71 refuses the word (DecodeError::MagicWaddrUnnamed: a magic write
    // to an address no 7.1 reference names; r0..r5 and the SFU addresses are
    // reserved there, qpu_validate.c:105-116). For the three rejected rows
    // the pipeline agrees with Mesa one stage earlier. For the `_ok` row
    // Mesa accepts a NOP operation with waddr R0 because
    // qpu_magic_waddr_matches (:118-134) skips a NOP's waddr; codec71 checks
    // the field whatever the operation. Mesa's packer never writes that word
    // (its NOP carries waddr NOP, 6), so it is a strictness, not a gap.
    ("sfu.magic_recip_rejected", Kind::Undecodable, "codec71 MagicWaddrUnnamed; Mesa qpu_validate.c:308-313"),
    ("waddr.r0_rejected", Kind::Undecodable, "codec71 MagicWaddrUnnamed; Mesa qpu_validate.c:308-313"),
    ("waddr.r4_rejected", Kind::Undecodable, "codec71 MagicWaddrUnnamed; Mesa qpu_validate.c:308-313"),
    (
        "waddr.nop_op_reserved_waddr_ok",
        Kind::Undecodable,
        "codec71 MagicWaddrUnnamed on a NOP's waddr; Mesa qpu_validate.c:118-134 skips it",
    ),
    // No words: Mesa's packer has no word for two small immediates
    // (`_unpackable`), and the `_raw_` rows validated the fields unpacked
    // (qpu_validate.c:204-218). verify71 has the rule (its unit tests cover
    // it on Instr71 values); the table cannot carry it as words.
    ("smallimm.a_and_d_raw_rejected", Kind::NoWords, "qpu_validate.c:213-218, no word carries two small immediates"),
    ("smallimm.a_and_b_raw_rejected", Kind::NoWords, "qpu_validate.c:213-218, no word carries two small immediates"),
    ("smallimm.a_and_d_unpackable", Kind::NoWords, "Mesa's packer refuses two small immediates"),
    ("smallimm.a_and_b_unpackable", Kind::NoWords, "Mesa's packer refuses two small immediates"),
];

/// Rows verify71 and Mesa agree on: 90 rows less the 19 exceptions.
const AGREE: usize = 71;

#[test]
fn verify71_agrees_with_mesa_qpu_validate_at_7_1() {
    let cases = parse();
    let mut agree = 0;
    let mut found: BTreeSet<(String, Kind)> = BTreeSet::new();
    let mut by_kind = std::collections::BTreeMap::<Kind, usize>::new();
    for case in &cases {
        let ours = run(case);
        match classify(case, &ours) {
            None => agree += 1,
            Some(kind) => {
                *by_kind.entry(kind).or_default() += 1;
                let pinned = EXCEPTIONS.iter().any(|(n, k, _)| *n == case.name && *k == kind);
                println!(
                    "{}{}: {kind:?}: mesa {:?}, ours {ours}",
                    if pinned { "" } else { "UNPINNED " },
                    case.name,
                    case.verdict
                );
                found.insert((case.name.clone(), kind));
            }
        }
    }
    println!("hazards71: {} rows, {agree} agree, {} exception(s): {by_kind:?}", cases.len(), found.len());
    assert_eq!(agree, AGREE, "the number of rows verify71 and Mesa agree on moved");
    let pinned: BTreeSet<(String, Kind)> = EXCEPTIONS.iter().map(|(n, k, _)| (n.to_string(), *k)).collect();
    let unpinned: Vec<_> = found.difference(&pinned).collect();
    let stale: Vec<_> = pinned.difference(&found).collect();
    assert!(unpinned.is_empty(), "verify71 disagrees with Mesa on rows not pinned in EXCEPTIONS: {unpinned:?}");
    assert!(stale.is_empty(), "EXCEPTIONS pins rows that now agree with Mesa: {stale:?}");
    assert!(cases.len() >= 90, "hazards71.tsv shrank to {} rows", cases.len());
    for (name, _, _) in EXCEPTIONS {
        assert!(cases.iter().any(|c| c.name == *name), "EXCEPTIONS names a row hazards71.tsv no longer has: {name}");
    }
}
