// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools record` - the host side of the result record
//! (driver/docs/one-method.md section 3).
//!
//!     yakogl-tools record <w0> <w1> <w2> <w3>         decode one record
//!     yakogl-tools record --classify <w0..w3>         one token: verdict, died_after=<stage>,
//!                                                     ended_no_picture, fingerprint or no_magic
//!     yakogl-tools record --line <w0..w3>             one line for a runner: class kind name resets tally
//!     yakogl-tools record --nvmem <file> [--classify|--line]   the 16 bytes of the SNVS nvmem file
//!     yakogl-tools record --log <file> [--classify|--line]     every `yk-record:` line of a console log;
//!                                                     the last one that is not a FINGERPRINT is the verdict
//!     yakogl-tools record crc <crc16 hex> [prefix]    dEQP case names whose name check is <crc16>
//!     yakogl-tools record crc16 <case name>           the name check of a case name (CRC-16/CCITT-FALSE)
//!     yakogl-tools record bitmap <prefix> <w1> <w2>   a kind-9 bitmap as names (filter = <prefix>*)
//!     yakogl-tools record sweep <first> <w1> <w2> <w3>  a gl_test counters record as case numbers
//!
//! The record itself (kinds, which kinds carry a tag, the stage table, the
//! classes, `describe`, the console line, the CRCs) is
//! driver/rust/src/board/record.rs, compiled into this binary as `record`
//! (main.rs): this file keeps no table of its own. What is here is what needs
//! the host: the dEQP case names behind a name check (the case list), the
//! field names `describe` leaves as numbers (the WDOG registers, console stage
//! 1's reasons), files and logs, and the runner's one line. (The PERF pages'
//! names were gc_slint's; nothing writes PERF since Step 1 deleted it, and
//! `describe` prints the six values.)
//!
//! It replaced the old Python decoder's `record` and `classify` (deleted in
//! Step 1); the classify tokens are that decoder's, pinned by
//! tools/testdata/record-decode.txt.
//! The dEQP case names come from DEQP_CASELIST when set, else
//! tools/cts/gles2-mustpass.txt. A name check can match two names; all of
//! them are printed.

use crate::record::{self, kind};
use crate::{py, repo_root, Error, R};

const USAGE: &str = "usage: yakogl-tools record <w0> <w1> <w2> <w3>
       yakogl-tools record --classify|--line <w0> <w1> <w2> <w3>
       yakogl-tools record --nvmem <file> [--classify|--line]
       yakogl-tools record --log <file> [--classify|--line]
       yakogl-tools record crc <crc16 hex> [prefix]
       yakogl-tools record crc16 <case name>
       yakogl-tools record bitmap <prefix> <w1> <w2>
       yakogl-tools record sweep <first> <w1> <w2> <w3>";

/// Two bits per item, item i at bits 2*(i % 16) of words[i / 16]. Items past
/// the words given read as 0 (no result): kind DEQP_CASES carries at most 32.
pub fn two_bits(words: &[u32], n: usize) -> Vec<u32> {
    (0..n).map(|i| words.get(i / 16).map(|w| (w >> (2 * (i % 16))) & 3).unwrap_or(0)).collect()
}

/// The case list the name checks are matched against.
pub struct CaseList {
    names: Vec<String>,
}

impl CaseList {
    pub fn from_text(text: &str) -> Self {
        CaseList { names: text.lines().map(py::strip).filter(|l| !l.is_empty()).map(String::from).collect() }
    }

    /// DEQP_CASELIST when set, else tools/cts/gles2-mustpass.txt.
    pub fn load() -> R<Self> {
        let path =
            py::env_set("DEQP_CASELIST").unwrap_or_else(|| py::join(&repo_root(), "tools/cts/gles2-mustpass.txt"));
        Ok(Self::from_text(&py::read_text(&path)?))
    }

    pub fn by_crc(&self, want: u16, prefix: &str) -> Vec<&str> {
        self.names
            .iter()
            .filter(|n| n.starts_with(prefix) && record::crc16_ccitt_false(n.as_bytes()) == want)
            .map(String::as_str)
            .collect()
    }
}

const DEQP_PREFIX: &str = "dEQP-GLES2.";

fn magic(w: &[u32; 4]) -> bool {
    w[0] & 0xffff_0000 == record::MAGIC
}

fn kind_low(w0: u32) -> (u8, u8) {
    ((w0 >> 8) as u8, w0 as u8)
}

/// The runner's token, record.rs's `classify`.
pub fn classify(w: &[u32; 4]) -> String {
    record::classify(*w).to_string()
}

/// (passed, failed, notsup, other, begun) of a kind DEQP or DEQP_CASES
/// record; None for any other record. DEQP_CASES does not tell NotSupported
/// from any other status (both are 3), so they count as notsup.
pub fn tally(w: &[u32; 4]) -> Option<[u32; 5]> {
    if !magic(w) {
        return None;
    }
    match kind_low(w[0]) {
        (kind::DEQP, _) => Some([w[1] & 0xffff, w[1] >> 16, w[2] & 0xffff, w[2] >> 16, w[3] & 0xffff]),
        (kind::DEQP_CASES, low) => {
            let st = two_bits(&[w[1], w[2]], low as usize);
            let count = |v: u32| st.iter().filter(|&&s| s == v).count() as u32;
            Some([count(1), count(2), count(3), 0, u32::from(low)])
        }
        _ => None,
    }
}

/// The GPU reset (recovery) count a record carries: kind DEQP_CASES (w3
/// bits 12..15), the SWEEP counters record (w1 bits 8..15) and kind
/// GPU_STOP (w2). None when the record has no such field: absent, never 0.
pub fn resets(w: &[u32; 4]) -> Option<u32> {
    if !magic(w) {
        return None;
    }
    match kind_low(w[0]) {
        (kind::DEQP_CASES, _) => Some((w[3] >> 12) & 15),
        (kind::SWEEP, low) if u32::from(low) == record::SWEEP_TAG_LEAKS => Some((w[1] >> 8) & 255),
        (kind::GPU_STOP, _) => Some(w[2]),
        _ => None,
    }
}

/// The count of what a run could not measure, its own outcome beside the
/// verdict: kind GLMARK2's unsupported scenes (record::glmark2_counts). None when
/// the record has no such field: absent, never 0.
pub fn unsupported(w: &[u32; 4]) -> Option<u32> {
    if !magic(w) {
        return None;
    }
    match kind_low(w[0]) {
        (kind::GLMARK2, _) => Some(record::glmark2_counts(w[1], w[2])[3]),
        _ => None,
    }
}

/// The pass/fail a record carries, as the runner's `status=` word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    /// The kind has a status field, but its value is neither pass nor a
    /// failure the kind defines.
    Invalid,
    /// The kind carries no pass/fail (PICTURE, FINGERPRINT, the dEQP tallies,
    /// the breadcrumbs, ...): its verdict is the run's completion alone.
    NotApplicable,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Invalid => "invalid",
            Status::NotApplicable => "n/a",
        }
    }
}

/// The record's status: APP (w1's low byte) and PROBES (the tag) carry
/// STATUS_PASS / STATUS_FAIL; CASE carries CASE_PASS or one of the CASE_*
/// failures (w1's low byte); FAULT (the fatal handler ran) is a failure
/// whatever its words; GLMARK2 carries STATUS_PASS / STATUS_FAIL in w2 bits
/// 24..27. Every other kind is n/a (record.rs, KINDS).
pub fn status(w: &[u32; 4]) -> Status {
    if !magic(w) {
        return Status::NotApplicable;
    }
    let pass_fail = |v: u32| match v {
        record::STATUS_PASS => Status::Pass,
        record::STATUS_FAIL => Status::Fail,
        _ => Status::Invalid,
    };
    match kind_low(w[0]) {
        (kind::APP, _) => pass_fail(w[1] & 0xff),
        (kind::PROBES, low) => pass_fail(u32::from(low)),
        (kind::CASE, _) => match w[1] & 0xff {
            record::CASE_PASS => Status::Pass,
            record::CASE_CHECK
            | record::CASE_NO_CHECK
            | record::CASE_UNFINISHED
            | record::CASE_NO_DEVICE
            | record::CASE_SET_FAIL => Status::Fail,
            _ => Status::Invalid,
        },
        (kind::FAULT, _) => Status::Fail,
        /* glmark2's own verdict, w2 bits 24..27 (record.rs, glmark2_w2) */
        (kind::GLMARK2, _) => pass_fail((w[2] >> 24) & 0xf),
        _ => Status::NotApplicable,
    }
}

/// The runner's one line: `class=<classify> kind=<n|absent> name=<NAME|?>
/// status=<pass|fail|invalid|n/a> resets=<n|absent> unsupported=<n|absent>
/// tally=<passed,failed,notsup,other,begun|->` (tally last: tools/board.sh takes
/// the rest of the line after `tally=`).
pub fn line(w: &[u32; 4]) -> String {
    let (k, _) = kind_low(w[0]);
    let name = if magic(w) { record::kind_info(k).map(|d| d.name).unwrap_or("?") } else { "?" };
    format!(
        "class={} kind={} name={} status={} resets={} unsupported={} tally={}",
        classify(w),
        if magic(w) { k.to_string() } else { "absent".into() },
        name,
        status(w).word(),
        resets(w).map(|r| r.to_string()).unwrap_or_else(|| "absent".into()),
        unsupported(w).map(|r| r.to_string()).unwrap_or_else(|| "absent".into()),
        tally(w).map(|t| t.iter().map(u32::to_string).collect::<Vec<_>>().join(",")).unwrap_or_else(|| "-".into())
    )
}

fn wcr(v: u32) -> String {
    format!(
        "0x{v:04x} ({}, WT 0x{:02x} = {:.1} s)",
        if v & 4 != 0 { "WDE" } else { "stopped" },
        v >> 8,
        f64::from((v >> 8) + 1) / 2.0
    )
}

/// The decoded record: record.rs's `describe` line, then what only the host
/// can add (the case names behind name checks, the bits of a DEQP_CASES run,
/// the field names of the WDOG registers and console stage 1).
pub fn decode(w: &[u32; 4], cases: &CaseList) -> Vec<String> {
    let mut first = String::new();
    // describe writes into a String, which cannot fail
    let _ = record::describe(&mut first, *w);
    let mut out = vec![first];
    if !magic(w) {
        return out;
    }
    let [_, w1, w2, w3] = *w;
    let (k, low) = kind_low(w[0]);
    let names = |c: u32| {
        let n = cases.by_crc(c as u16, DEQP_PREFIX);
        if n.is_empty() {
            "(no name in the case list)".to_string()
        } else {
            n.join(" ")
        }
    };
    match k {
        kind::DEQP if w3 >> 16 != 0xffff => out.push(format!("  first fail {:04x}: {}", w3 >> 16, names(w3 >> 16))),
        kind::DEQP_CASES => {
            if w3 >> 16 != 0xffff {
                out.push(format!("  first fail {:04x}: {}", w3 >> 16, names(w3 >> 16)));
            }
            let bits: String =
                two_bits(&[w1, w2], usize::from(low)).iter().map(|&s| ['.', 'P', 'F', '?'][s as usize]).collect();
            out.push(format!("  bits per case (the filter's names in sorted order): {bits}"));
        }
        kind::DEQP_FAILS => {
            for word in [w1, w2, w3] {
                for c in [word & 0xffff, word >> 16] {
                    if c != 0xffff {
                        out.push(format!("  {c:04x} {}", names(c)));
                    }
                }
            }
        }
        kind::WDOG => {
            let (flags, refusal) = (w2 & 0xff, (w2 >> 8) & 0xff);
            let why = match refusal {
                0 => "none".to_string(),
                1 => "timeout not a 0.5 s step".to_string(),
                2 => "WMCR.PDE stuck".to_string(),
                3 => "WDE read back clear".to_string(),
                4 => "WT read back differs".to_string(),
                5 => "WDT read back clear".to_string(),
                r => format!("refusal {r}"),
            };
            let fl: Vec<&str> = [(1, "was-running"), (2, "cleared-PDE"), (4, "root-enabled"), (8, "gate-opened")]
                .iter()
                .filter(|(b, _)| flags & b != 0)
                .map(|(_, n)| *n)
                .collect();
            out.push(format!("  WCR found {}", wcr(w1 & 0xffff)));
            out.push(format!("  WCR now   {}", wcr(w1 >> 16)));
            out.push(format!(
                "  flags {}, refusal: {why}",
                if fl.is_empty() { "none".to_string() } else { fl.join(",") }
            ));
        }
        kind::STAGE if low == 0x14 => {
            let why: Vec<&str> = [
                (1, "MEDIAMIX off"),
                (2, "MEDIA_APB root off"),
                (4, "CCGR93 closed"),
                (8, "LCDIF APB clock off or in reset"),
            ]
            .iter()
            .filter(|(b, _)| w1 & 0xff & b != 0)
            .map(|(_, n)| *n)
            .collect();
            out.push(format!(
                "  console stage 1: {}",
                match w1 {
                    1 => "took over the running display".to_string(),
                    0 => "LCDIF answered, nothing to adopt".to_string(),
                    _ => format!("LCDIF not read: {}", if why.is_empty() { "?".to_string() } else { why.join(", ") }),
                }
            ));
        }
        _ => {}
    }
    out
}

/// The four words of the SNVS nvmem file (LPGPR0..3, little-endian).
/// The animation's rate normalised by the window's pixels (record.rs
/// Throughput: FINGERPRINT, PERF page 0, APP), `None` when the run has no
/// such records: fps alone does not compare across panel sizes.
pub fn throughput_line(recs: &[[u32; 4]]) -> Option<String> {
    let t = record::Throughput::from_records(recs)?;
    let mut s = String::from("throughput: ");
    // a String cannot fail to take the text
    let _ = record::write_throughput(&mut s, &t);
    Some(s)
}

pub fn from_nvmem(b: &[u8]) -> R<[u32; 4]> {
    if b.len() < 16 {
        return Err(Error::Exit(format!("the nvmem file has {} bytes, a record needs 16", b.len())));
    }
    let word = |i: usize| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
    Ok([word(0), word(1), word(2), word(3)])
}

/// Every record line of a console log, in order: record.rs's `parse_line`
/// (`yk-record: ...`, anything before the prefix), and gl_triangle's legacy
/// `gl_triangle: record ...` (its own PICTURE line until the step 3 sample
/// replaces it).
pub fn from_log(text: &str) -> Vec<[u32; 4]> {
    const LEGACY: &str = "gl_triangle: record ";
    text.replace(['\0', '\r'], "")
        .lines()
        .filter_map(|l| {
            record::parse_line(l).or_else(|| {
                let at = l.find(LEGACY)?;
                record::parse_line(&format!("{}{}", record::LINE_PREFIX, &l[at + LEGACY.len()..]))
            })
        })
        .collect()
}

/// The verdict record of a log's records: the last one whose class is not
/// FINGERPRINT's (record.rs classify; a fingerprint is written beside a
/// verdict, never instead), else the last one.
pub fn verdict_record(recs: &[[u32; 4]]) -> Option<[u32; 4]> {
    recs.iter()
        .rev()
        .find(|w| record::classify(**w) != record::Class::Fingerprint)
        .copied()
        .or_else(|| recs.last().copied())
}

fn parse_word(s: &str) -> R<u32> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).map_err(|_| Error::Usage(format!("{USAGE}\n\n'{s}' is not a 32-bit hex word")))
}

fn words(a: &[String]) -> R<[u32; 4]> {
    if a.len() != 4 {
        return Err(Error::Usage(format!("{USAGE}\n\na record is four words")));
    }
    Ok([parse_word(&a[0])?, parse_word(&a[1])?, parse_word(&a[2])?, parse_word(&a[3])?])
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Decode,
    Classify,
    Line,
}

fn emit(w: &[u32; 4], mode: Mode, cases: &mut Option<CaseList>) -> R<()> {
    match mode {
        Mode::Classify => println!("{}", classify(w)),
        Mode::Line => println!("{}", line(w)),
        Mode::Decode => {
            if cases.is_none() {
                *cases = Some(CaseList::load()?);
            }
            if let Some(c) = cases.as_ref() {
                for l in decode(w, c) {
                    println!("{l}");
                }
            }
        }
    }
    Ok(())
}

pub fn main(args: &[String]) -> R<()> {
    let first = args.first().map(String::as_str).unwrap_or("");
    match first {
        "" | "-h" | "--help" => {
            println!("{USAGE}");
            return Ok(());
        }
        "crc" if args.len() >= 2 => {
            let want = u16::from_str_radix(args[1].trim_start_matches("0x"), 16)
                .map_err(|_| Error::Usage(format!("{USAGE}\n\n'{}' is not a CRC-16", args[1])))?;
            let prefix = args.get(2).map(String::as_str).unwrap_or(DEQP_PREFIX);
            for n in CaseList::load()?.by_crc(want, prefix) {
                println!("{n}");
            }
            return Ok(());
        }
        "crc16" if args.len() == 2 => {
            println!("{:04x}", record::crc16_ccitt_false(args[1].as_bytes()));
            return Ok(());
        }
        "bitmap" if args.len() == 4 => {
            let cl = CaseList::load()?;
            let mut names: Vec<&String> = cl.names.iter().filter(|n| n.starts_with(args[1].as_str())).collect();
            names.sort();
            let st = two_bits(&[parse_word(&args[2])?, parse_word(&args[3])?], names.len().min(32));
            for (n, s) in names.iter().zip(st) {
                println!("{:<4} {n}", ["none", "Pass", "Fail", "other"][s as usize]);
            }
            return Ok(());
        }
        "sweep" if args.len() == 5 => {
            let first: u64 = args[1].parse().map_err(|_| Error::Usage(USAGE.into()))?;
            let (w2, w3) = (parse_word(&args[3])?, parse_word(&args[4])?);
            let passed = u64::from(w3) | (u64::from(w2 >> 16) << 32);
            let fmt = |want: bool| {
                let v: Vec<String> =
                    (0..48).filter(|i| ((passed >> i) & 1 == 1) == want).map(|i| (first + i).to_string()).collect();
                format!("[{}]", v.join(", "))
            };
            println!("pass {}", fmt(true));
            println!("not passed (fail, skipped or no such case) {}", fmt(false));
            return Ok(());
        }
        _ => {}
    }
    let mut mode = Mode::Decode;
    let mut source: Option<(&str, &str)> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--classify" => mode = Mode::Classify,
            "--line" => mode = Mode::Line,
            "--nvmem" | "--log" => {
                let f = it.next().ok_or_else(|| Error::Usage(USAGE.into()))?;
                source = Some((a.as_str(), f.as_str()));
            }
            _ => rest.push(a.clone()),
        }
    }
    let mut cases = None;
    match source {
        None => emit(&words(&rest)?, mode, &mut cases),
        Some((_, _)) if !rest.is_empty() => Err(Error::Usage(format!("{USAGE}\n\nwords and a file together"))),
        Some(("--nvmem", f)) => emit(&from_nvmem(&py::read_bytes(f)?)?, mode, &mut cases),
        Some((_, f)) => {
            let recs = from_log(&py::lossy_text(&py::read_bytes(f)?));
            let Some(v) = verdict_record(&recs) else {
                // absent is its own value: no record line, never a zero record
                println!(
                    "{}",
                    if mode == Mode::Line {
                        "class=no_record kind=absent name=? status=n/a resets=absent unsupported=absent tally=-"
                    } else {
                        "no_record"
                    }
                );
                return Err(Error::Exit(format!("{f}: no yk-record line")));
            };
            if mode == Mode::Decode {
                for (i, w) in recs.iter().enumerate() {
                    println!("record {}/{}: {:08x} {:08x} {:08x} {:08x}", i + 1, recs.len(), w[0], w[1], w[2], w[3]);
                    emit(w, mode, &mut cases)?;
                }
                println!("verdict record: {:08x} {:08x} {:08x} {:08x}", v[0], v[1], v[2], v[3]);
                if let Some(t) = throughput_line(&recs) {
                    println!("{t}");
                }
                Ok(())
            } else {
                emit(&v, mode, &mut cases)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mustpass() -> CaseList {
        CaseList::from_text(&py::read_text(&py::join(&repo_root(), "tools/cts/gles2-mustpass.txt")).unwrap())
    }

    /// Every block of tools/testdata/record-decode.txt: the decode and the
    /// classify of one record. The classify lines are the old Python decoder's
    /// tokens at 7924b635 for the same records (so record.rs's classify is
    /// pinned to the old runner's); the decode lines are this tool's output,
    /// describe() first.
    #[test]
    fn decode_matches_golden() {
        let text = py::read_text(&py::join(&repo_root(), "tools/testdata/record-decode.txt")).unwrap();
        let cases = mustpass();
        let mut blocks = 0;
        let mut kinds_seen = std::collections::BTreeSet::new();
        let mut cur: Option<([u32; 4], Vec<String>)> = None;
        for l in text.lines().filter(|l| !l.starts_with("# ")) {
            if let Some(rest) = l.strip_prefix("### classify ") {
                let (w, lines) = cur.take().expect("a classify line without its record");
                assert_eq!(decode(&w, &cases), lines, "decode of {w:08x?}");
                assert_eq!(classify(&w), rest, "classify of {w:08x?}");
                blocks += 1;
                if magic(&w) {
                    kinds_seen.insert(kind_low(w[0]).0);
                }
            } else if let Some(rest) = l.strip_prefix("### ") {
                let v: Vec<u32> = rest.split(' ').map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
                cur = Some(([v[0], v[1], v[2], v[3]], Vec::new()));
            } else if let Some((_, lines)) = cur.as_mut() {
                lines.push(l.to_string());
            }
        }
        // the population: the golden file's 49 blocks (45 + GLMARK2 on- and off-screen and with
        // GLMARK2_SCENES + GPU_STOP), every kind of record.rs's table among them
        assert_eq!(blocks, 49, "the golden file's block count changed: re-derive the pin");
        let want: std::collections::BTreeSet<u8> = record::KINDS.iter().map(|k| k.value).chain([32]).collect();
        assert_eq!(kinds_seen, want, "every kind (and one unknown) has a golden block");
    }

    /// classify on records with known meaning: the two 2026-10-03 netboot
    /// board records (step C and its kexec control, imx8mp-netboot-plan.md
    /// "Board run 2026-10-03") and one of each class. Moved from
    /// the old Python decoder's self-test; run here on record.rs's classify.
    #[test]
    fn classify_known_records() {
        let cases: &[([u32; 4], &str)] = &[
            ([0x594B0D01, 0x773F773F, 0x00000001, 0x00002E30], "died_after=wdog_armed"),
            ([0x594B040E, 0xFF403F80, 0x35706C4E, 0x04000258], "verdict"),
            ([0x594B0D03, 0x773F773F, 0x00000001, 0x00002E30], "verdict"),
            ([0x594B0D03, 0x773F773F, 0x00000001, 0x00001248], "verdict"),
            ([0x594B0D02, 0x0000773F, 0x00000300, 0x00000010], "verdict"),
            ([0x594B0E13, 0, 11830, 0], "died_after=console_lcdif_read"),
            ([0x594B0E13, 0, 0x12E8, 0], "died_after=console_lcdif_read"),
            ([0x594B0E33, 0, 12100, 1], "died_after=gpu_power_gpumix"),
            ([0x594B0E3A, 0xFFFFFFED, 12200, 1], "died_after=gpu_init_done"),
            ([0x594B0E4F, 0, 15000, 4], "ended_no_picture"),
            ([0x594B0E7F, 0, 1, 0], "died_after=stage_0x7f"),
            ([0x594B0503, 0x104, 13000, 3], "died_after=render_step_hw"),
            ([0x594B0502, 0x0A1, 12900, 2], "died_after=kernel_up"),
            ([0x594B0101, 0x00070101, 1, 0], "verdict"),
            ([0x00000000, 0x53450003, 0, 0], "no_magic"),
            ([0x594B0F01, 0x1A2B3C4D, 2, 0x04000258], "fingerprint"),
        ];
        for (w, want) in cases {
            assert_eq!(classify(w), *want, "{w:08x?}");
        }
        for s in record::STAGES {
            let got = classify(&[0x594B0E00 | u32::from(s.value), 0, 0, 0]);
            let want = if s.value == record::STAGE_ENDED {
                "ended_no_picture".into()
            } else {
                format!("died_after={}", s.label)
            };
            assert_eq!(got, want, "stage 0x{:02x}", s.value);
        }
        assert!(matches!(record::classify([0x594B0E4F, 0, 0, 0]), record::Class::EndedNoPicture));
    }

    /// gc_result.h, the i.MX8MP samples' writer API, against record.rs: since
    /// the board layer (db659a88) its stage names are aliases of the
    /// generated yakogl_record.h's, so every GC_RESULT_STAGE_<X> must be
    /// YK_RECORD_STAGE_<X> and every named row of record.rs's STAGES (and
    /// GPU_POWER) must have its alias; the kind STAGE alias too. This took over
    /// the old Python decoder's --check-writer, which compared two tables.
    #[test]
    fn the_old_writer_names_are_record_rs_aliases() {
        let h = py::read_text(&py::join(&repo_root(), "driver/zephyr/lib/board/imx8mp/gc_result.h")).unwrap();
        let rx = py::re(r"(?m)^#define GC_RESULT_STAGE_(\w+)\s+(\S+)");
        let defs: std::collections::BTreeMap<String, String> =
            rx.captures_iter(&h).map(|c| (py::g(&c, 1).to_string(), py::g(&c, 2).to_string())).collect();
        for (name, val) in &defs {
            assert_eq!(val, &format!("YK_RECORD_STAGE_{name}"), "GC_RESULT_STAGE_{name} is not record.rs's alias");
        }
        let mut want: std::collections::BTreeSet<String> =
            record::STAGES.iter().filter_map(|s| s.name.map(String::from)).collect();
        want.insert("GPU_POWER".into());
        assert_eq!(defs.keys().cloned().collect::<std::collections::BTreeSet<_>>(), want, "the alias set");
        assert!(
            py::re(r"GC_RESULT_STAGE\s*=\s*YK_RECORD_KIND_STAGE,").is_match(&h),
            "gc_result.h's kind STAGE is not record.rs's"
        );
        // the population at db659a88: 28 named stages + GPU_POWER
        assert_eq!(defs.len(), 29, "the stage population changed: re-derive the pin");
    }

    /// The Pi 4 cluster row of 2026-10-03 as its console carries it: the
    /// throughput line from the FINGERPRINT, PERF and APP records.
    #[test]
    fn throughput_from_a_log() {
        let log = "yk-record: 594b0f05 5c41b81a 00000003 078002d0\n\
                   yk-record: 594b0c00 05010576 00320010 00010000\n\
                   yk-record: 594b0206 00000701 000020cb 0000ea6c\n\
                   yk-record: 594b0601 00000000 00000000 00000000\n";
        let recs = from_log(log);
        assert_eq!(recs.len(), 4);
        assert_eq!(
            throughput_line(&recs).as_deref(),
            Some(
                "throughput: 1920x720: 8395 frames in 60012 ms, 139.9 fps, 193.4 Mpx/s; draw 5.0 ms mean, 3.62 ms/Mpx"
            )
        );
        assert_eq!(throughput_line(&recs[2..]), None, "no FINGERPRINT, no PERF: no rate");
    }

    #[test]
    fn log_lines_and_the_verdict_record() {
        let log = "12:00:00 boot\n12:00:01 yk-record: 594b0e13 00000000 000012e8 00000000\r\n\
                   \0junk yk-record: 594b0601 1234abcd 5678ef01 9abc2345\n\
                   12:00:03 yk-record: 594b0f02 deadbeef 00000002 04000258\n";
        let recs = from_log(log);
        assert_eq!(recs.len(), 3);
        assert_eq!(verdict_record(&recs), Some([0x594b0601, 0x1234abcd, 0x5678ef01, 0x9abc2345]));
        assert_eq!(from_log("gl_triangle: record 594b0400 ff403f80 35706c4e 00400040\n").len(), 1);
        assert!(from_log("yk-record: 594b0e13 0000 short\n").is_empty());
        assert_eq!(verdict_record(&[]), None);
        let fp = [0x594b0f02, 0xdeadbeef, 2, 0x04000258];
        assert_eq!(verdict_record(&[fp]), Some(fp), "a lone fingerprint is still what the log holds");
    }

    #[test]
    fn nvmem_bytes_are_little_endian() {
        let b = [0x0e, 0x04, 0x4b, 0x59, 0x80, 0x3f, 0x40, 0xff, 0x4e, 0x6c, 0x70, 0x35, 0x58, 0x02, 0x00, 0x04];
        assert_eq!(from_nvmem(&b).unwrap(), [0x594b040e, 0xff403f80, 0x35706c4e, 0x04000258]);
        assert!(from_nvmem(&b[..12]).is_err());
    }

    #[test]
    fn tally_resets_and_line() {
        assert_eq!(tally(&[0x594b0811, 0x00020064, 0x00010003, 0x94050067]), Some([100, 2, 3, 1, 103]));
        assert_eq!(tally(&[0x594b0904, 0x96, 0, 0xa1e01001]), Some([2, 2, 0, 0, 4]));
        assert_eq!(tally(&[0x594b0601, 0, 0, 0]), None);
        assert_eq!(resets(&[0x594b0904, 0x96, 0, 0xa1e01001]), Some(1));
        assert_eq!(resets(&[0x594b07ff, 0x00010203, 0, 0]), Some(2));
        assert_eq!(resets(&[0x594b040e, 0, 0, 0]), None);
        assert_eq!(
            line(&[0x594b040e, 0xff403f80, 0x35706c4e, 0x04000258]),
            "class=verdict kind=4 name=PICTURE status=n/a resets=absent unsupported=absent tally=-"
        );
        assert_eq!(
            line(&[0, 0x53450003, 0, 0]),
            "class=no_magic kind=absent name=? status=n/a resets=absent unsupported=absent tally=-"
        );
        // the two board records of 2026-10-03 (Pi 4 ddd58c27, AM62P 9d4f63c8).
        // The Pi 4's w2 says status 2: its sample still counted an unsupported
        // scene as a failure (the pass rule had `out.unsupported == 0` until
        // 7b7f34eb). This line read status=n/a while status() did not map
        // GLMARK2; it reads the record's own word now.
        assert_eq!(
            line(&[0x594b1201, 0x000000c9, 0x021d0004, 0x03200258]),
            "class=verdict kind=18 name=GLMARK2 status=fail resets=absent unsupported=4 tally=-"
        );
        assert_eq!(unsupported(&[0x594b1201, 0x0000012f, 0x021d0202, 0x03200258]), Some(2));
        // with GLMARK2_SCENES (a synthetic record): the count is w2 bits 6..11, the
        // low bits are the second unsupported entry (19)
        assert_eq!(
            line(&[0x594b120e, 0x44960027, 0x22781093, 0x03200258]),
            "class=verdict kind=18 name=GLMARK2 status=fail resets=absent unsupported=2 tally=-"
        );
        assert_eq!(unsupported(&[0x594b040e, 0, 0, 0]), None, "a kind without the field: absent");
    }

    /// The status of each kind that carries one, both ways, and n/a for the
    /// rest (a fail must never read as a pass: board.sh's verdict=failed).
    #[test]
    fn status_per_kind() {
        let cases: [([u32; 4], Status); 17] = [
            ([0x594b0202, 0x0000_0001, 1, 0], Status::Pass),    // APP pass
            ([0x594b0202, 0x0000_0602, 1, 0], Status::Fail),    // APP fail, stage 6
            ([0x594b0202, 0x0000_0003, 1, 0], Status::Invalid), // APP status 3
            ([0x594b0601, 0x08420882, 0x00210842, 0x98080883], Status::Pass), // PROBES tag 1
            ([0x594b0602, 0x08420882, 0x00210842, 0x98080883], Status::Fail), // PROBES tag 2
            ([0x594b0600, 0, 0, 0], Status::Invalid),           // PROBES tag 0
            ([0x594b0101, 0x000aff01, 12, 0], Status::Pass),    // CASE 10 pass
            ([0x594b0101, 0x000a0302, 0xff, 0xfe], Status::Fail), // CASE 10 check 3
            ([0x594b0101, 0x000aff06, 3, 4], Status::Fail),     // CASE_SET_FAIL
            ([0x594b0101, 0x000aff00, 0, 0], Status::Invalid),  // CASE status 0
            ([0x594b0300, 0x96000045, 0x40001234, 0], Status::Fail), // FAULT
            // GLMARK2: the AM62P run of 2026-10-04 that printed "glmark2: FAIL rc=-5"
            // (score 533, 30 ran, 1 failed, 2 unsupported); board.sh read it as n/a
            ([0x594b1201, 0x0000_0215, 0x021e_0102, 0x0320_0258], Status::Fail),
            ([0x594b1201, 0x0000_0132, 0x1121_0000, 0x0780_0438], Status::Pass), // GLMARK2 pass, off-screen
            ([0x594b1201, 0x0000_0132, 0x0021_0000, 0x0780_0438], Status::Invalid), // GLMARK2 status 0
            ([0x594b040e, 0xff403f80, 0x35706c4e, 0x04000258], Status::NotApplicable), // PICTURE
            ([0x594b0811, 0x00020064, 0x00010003, 0x94050067], Status::NotApplicable), // DEQP
            ([0x12345678, 0, 0, 0], Status::NotApplicable),                      // no magic
        ];
        for (w, want) in cases {
            assert_eq!(status(&w), want, "{w:08x?}");
        }
        assert_eq!(
            line(&[0x594b0202, 0x0000_0602, 1, 0]),
            "class=verdict kind=2 name=APP status=fail resets=absent unsupported=absent tally=-"
        );
        assert_eq!(
            line(&[0x594b1201, 0x0000_0215, 0x021e_0102, 0x0320_0258]),
            "class=verdict kind=18 name=GLMARK2 status=fail resets=absent unsupported=2 tally=-"
        );
    }
}
