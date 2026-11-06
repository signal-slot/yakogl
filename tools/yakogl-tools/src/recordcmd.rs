// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools record` - the host side of the result record
//! (driver/docs/one-method.md section 3).
//!
//!     yakogl-tools record <w0> <w1> <w2> <w3>         decode one record
//!     yakogl-tools record --classify <w0..w3>         one token: verdict, died_after=<stage>,
//!                                                     ended_no_picture, fingerprint or no_magic
//!     yakogl-tools record --line <w0..w3>             one line for a runner: class kind name resets tally
//!     yakogl-tools record --words 3 ...               (with any of the above) a record of an image built with
//!                                                     CONFIG_GC_COMMON_RESULT_SNVS_KEEP_LPGPR0 (the Armadillo-X2:
//!                                                     LPGPR0 is U-Boot's boot count, w3 is never stored). Takes
//!                                                     w0 w1 w2 (a fourth word is ignored); every field that
//!                                                     lives in w3 prints as `-`, never a number: DEQP's cases
//!                                                     begun and first fail, DEQP_CASES' first fail, GPU resets
//!                                                     and fence timeouts, DEQP_STATUS bytes 9-12, PROBES p4 and
//!                                                     crc16, ... The line gains `words=3 begun=derived|tag|-`
//!                                                     before `tally=`: DEQP's begun is pass+fail+notsup+other
//!                                                     (derived), DEQP_CASES' is its tag. DEQP_FAILS pages hold
//!                                                     4 names (w1, w2; record.rs DEQP_FAILS_PER_PAGE_3W).
//!     yakogl-tools record --nvmem <file> [--classify|--line]   the 16 bytes of the SNVS nvmem file
//!     yakogl-tools record --log <file> [--classify|--line]     every `yk-record:` line of a console log;
//!                                                     the last one that is not a FINGERPRINT is the verdict
//!     yakogl-tools record crc <crc16 hex> [prefix]    case names whose name check is <crc16>,
//!                                                     starting with <prefix> (default: every name)
//!     yakogl-tools record crc16 <case name>           the name check of a case name (CRC-16/CCITT-FALSE)
//!     yakogl-tools record bitmap <prefix> <w1> <w2>   a kind-9 bitmap as names (filter = <prefix>*)
//!     yakogl-tools record sweep <first> <w1> <w2> <w3>  a gl_test counters record as case numbers
//!     yakogl-tools record msaa [--log f]... [--nvmem f]... [<w0..w3>]...   case 16's 4x probe pages
//!                                                     (msaarec.rs) as the console's x4 lines
//!     yakogl-tools record kickdiff [--log f]... [--nvmem f]... [<w0..w3>]...   cases 2 and 10's
//!                                                     KICK_DIFF pages (kickdiffrec.rs), one line a page
//!     yakogl-tools record glmark2-pages [--log f]... [--nvmem f]... [<w0..w3>]...   a glmark2 run's
//!                                                     GM_PAGE records (and the cluster's RATE) as the
//!                                                     console's result, glmark2-frame: and
//!                                                     glmark2-picture: lines (record.rs write_gm_lines)
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
//! The dEQP case names come from DEQP_CASELIST when set, else both
//! tools/cts/gles2-mustpass.txt and tools/cts/gles3-mustpass.txt: a name
//! check is the CRC-16 of the whole case name, module prefix included (the
//! dEQP sample's deqp_snvs.c), so one list decodes the records of an image
//! of either table. A name check can match two names; all of them are
//! printed, and the reader (tools/board.sh results_tsv) keeps the ones
//! inside the image's own case-table entry.

use crate::record::{self, kind};
use crate::{py, repo_root, Error, R};

const USAGE: &str = "usage: yakogl-tools record <w0> <w1> <w2> <w3>
       yakogl-tools record --classify|--line <w0> <w1> <w2> <w3>
       yakogl-tools record --nvmem <file> [--classify|--line]
       yakogl-tools record --log <file> [--classify|--line]
       yakogl-tools record --words 3 [--classify|--line] <w0> <w1> <w2>   (also with --nvmem, --log)
           a record of an image built with CONFIG_GC_COMMON_RESULT_SNVS_KEEP_LPGPR0 (the Armadillo-X2):
           w3 is not stored; every w3 field prints as '-'
       yakogl-tools record crc <crc16 hex> [prefix]
       yakogl-tools record crc16 <case name>
       yakogl-tools record bitmap <prefix> <w1> <w2>
       yakogl-tools record sweep <first> <w1> <w2> <w3>
       yakogl-tools record msaa [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...
       yakogl-tools record kickdiff [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...
       yakogl-tools record glmark2-pages [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...";

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

    /// DEQP_CASELIST when set, else tools/cts/gles2-mustpass.txt and
    /// tools/cts/gles3-mustpass.txt together (an image names its cases from
    /// one of the two tables; its record does not say which, the CRC does).
    pub fn load() -> R<Self> {
        if let Some(path) = py::env_set("DEQP_CASELIST") {
            return Ok(Self::from_text(&py::read_text(&path)?));
        }
        let mut text = py::read_text(&py::join(&repo_root(), "tools/cts/gles2-mustpass.txt"))?;
        text.push_str(&py::read_text(&py::join(&repo_root(), "tools/cts/gles3-mustpass.txt"))?);
        Ok(Self::from_text(&text))
    }

    /// The names whose CRC-16 is `want`, restricted to those starting with
    /// `prefix` (empty: every name in the list).
    pub fn by_crc(&self, want: u16, prefix: &str) -> Vec<&str> {
        self.names
            .iter()
            .filter(|n| n.starts_with(prefix) && record::crc16_ccitt_false(n.as_bytes()) == want)
            .map(String::as_str)
            .collect()
    }
}

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

/// The record's status: APP (w1's low byte), PROBES and RATE (the tag) carry
/// STATUS_PASS / STATUS_FAIL; CASE carries CASE_PASS or one of the CASE_*
/// failures (w1's low byte); FAULT (the fatal handler ran) and CHECK are
/// failures whatever their words; GLMARK2 carries STATUS_PASS / STATUS_FAIL in w2 bits
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
        (kind::PROBES, low) | (kind::RATE, low) => pass_fail(u32::from(low)),
        (kind::CASE, _) => match w[1] & 0xff {
            /* the set's summary: a pass when every case run passed, and
             * fields that disagree are not a reading (record.rs SetSummary) */
            record::CASE_SET => {
                let s = record::SetSummary::from_words(w[1], w[2], w[3]);
                if !s.consistent() {
                    Status::Invalid
                } else if s.passed_all() {
                    Status::Pass
                } else {
                    Status::Fail
                }
            }
            record::CASE_PASS => Status::Pass,
            record::CASE_CHECK
            | record::CASE_NO_CHECK
            | record::CASE_UNFINISHED
            | record::CASE_NO_DEVICE
            | record::CASE_SET_FAIL => Status::Fail,
            _ => Status::Invalid,
        },
        (kind::FAULT, _) => Status::Fail,
        /* a failure whatever its words (record.rs, CHECK) */
        (kind::CHECK, _) => Status::Fail,
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

// ---- --words 3: records of an image built with
// CONFIG_GC_COMMON_RESULT_SNVS_KEEP_LPGPR0 (the Armadillo-X2). The record is
// w0..w2 in LPGPR1..3; w3 is never stored, so whatever the reader was given
// for it is ignored and every field that lives in w3 is absent (`-`, never a
// number). The 4-word functions above are unchanged.

/// Placeholder printed for a field that lives in the word a three-word record
/// does not store.
pub const ABSENT: &str = "-";

/// The names a DEQP_FAILS page holds in a three-word record: four, in w1 and
/// w2, the low half first (record::DEQP_FAILS_PER_PAGE_3W); 0xffff is none.
pub fn fails_names_3w(w: &[u32; 4]) -> [u32; record::DEQP_FAILS_PER_PAGE_3W as usize] {
    [w[1] & 0xffff, w[1] >> 16, w[2] & 0xffff, w[2] >> 16]
}

/// `tally` of a three-word record: kind DEQP's cases begun (w3) is derived as
/// pass + fail + notsup + other, the only cases a finished tally counts;
/// kind DEQP_CASES keeps its begun (the tag). See `begun_source`.
pub fn tally3(w: &[u32; 4]) -> Option<[u32; 5]> {
    let mut t = tally(w)?;
    if kind_low(w[0]).0 == kind::DEQP {
        t[4] = t[0] + t[1] + t[2] + t[3];
    }
    Some(t)
}

/// Where `tally3`'s fifth number comes from: `derived` (DEQP: the sum of the
/// other four, w3 is gone), `tag` (DEQP_CASES) or `-` (no tally).
pub fn begun_source(w: &[u32; 4]) -> &'static str {
    match (magic(w), kind_low(w[0]).0) {
        (true, kind::DEQP) => "derived",
        (true, kind::DEQP_CASES) => "tag",
        _ => ABSENT,
    }
}

/// `resets` of a three-word record: DEQP_CASES keeps its GPU resets in w3,
/// so it has none; the other kinds are as in `resets`.
pub fn resets3(w: &[u32; 4]) -> Option<u32> {
    if magic(w) && kind_low(w[0]).0 == kind::DEQP_CASES {
        return None;
    }
    resets(w)
}

/// `status` of a three-word record: the CASE_SET summary compares passed with
/// cases run (w3), so it cannot be read: invalid, not a guess.
pub fn status3(w: &[u32; 4]) -> Status {
    if magic(w) && kind_low(w[0]).0 == kind::CASE && w[1] & 0xff == record::CASE_SET {
        return Status::Invalid;
    }
    status(w)
}

/// `line` for a three-word record: the 4-word line with `words=3` and
/// `begun=<derived|tag|->` before the tally (tools/board.sh takes the rest of
/// the line after `tally=`).
pub fn line3(w: &[u32; 4]) -> String {
    let (k, _) = kind_low(w[0]);
    let name = if magic(w) { record::kind_info(k).map(|d| d.name).unwrap_or("?") } else { "?" };
    format!(
        "class={} kind={} name={} status={} resets={} unsupported={} words=3 begun={} tally={}",
        classify(w),
        if magic(w) { k.to_string() } else { "absent".into() },
        name,
        status3(w).word(),
        resets3(w).map(|r| r.to_string()).unwrap_or_else(|| "absent".into()),
        unsupported(w).map(|r| r.to_string()).unwrap_or_else(|| "absent".into()),
        begun_source(w),
        tally3(w).map(|t| t.iter().map(u32::to_string).collect::<Vec<_>>().join(",")).unwrap_or_else(|| ABSENT.into())
    )
}

/// record.rs's `describe` for a three-word record: the same fields with
/// every w3 field as `-`. Fields of w1 and w2 are unchanged.
pub fn describe3(w: &[u32; 4]) -> String {
    let [w0, w1, w2, _] = *w;
    if !magic(w) {
        return format!("NO RECORD (w0 = {w0:08x})");
    }
    let (k, low) = kind_low(w0);
    let a = ABSENT;
    let (lo, hi) = (|x: u32| x & 0xffff, |x: u32| x >> 16);
    let Some(info) = record::kind_info(k) else {
        return format!("kind {k} low {low}: w1 {w1:08x} w2 {w2:08x} w3 {a} (not a known kind)");
    };
    let mut s = format!("{} ({k}) {} {low}: ", info.name, if info.low == record::Low::Seq { "seq" } else { "tag" });
    let none = |x: u32| if x == 0xffff { "none".to_string() } else { x.to_string() };
    let body = match k {
        kind::CASE if w1 & 0xff == record::CASE_SET => format!(
            "set status {} (passed or failed: not readable without cases run); passed {} of {a} run; \
             skipped {}, first skip case {a}; first failing case {}, second case {}",
            record::CASE_SET,
            lo(w2),
            (w1 >> 8) & 0xff,
            none(hi(w1)),
            none(hi(w2)),
        ),
        kind::CASE => {
            format!("case {} check {} status {} got 0x{w2:08x} want {a}", hi(w1), (w1 >> 8) & 0xff, w1 & 0xff)
        }
        kind::APP => format!("stage {} status {} frames {w2} app {a}", (w1 >> 8) & 0xff, w1 & 0xff),
        kind::FAULT => format!("esr 0x{w1:08x} (EC 0x{:02x}) elr 0x{w2:08x} far {a}", w1 >> 26),
        kind::PICTURE => format!("centre 0x{w1:08x} crc32 0x{w2:08x} size {a}"),
        kind::PROGRESS => {
            let mut t = format!("where 0x{w1:03x} (");
            let _ = record::write_where(&mut t, w1);
            format!("{t}) uptime {w2} ms count {a}")
        }
        kind::PROBES => format!(
            "status {low} p0 {:04x} p1 {:04x} p2 {:04x} p3 {:04x} p4 {a} crc16 {a} (RGB565)",
            lo(w1),
            hi(w1),
            lo(w2),
            hi(w2)
        ),
        kind::SWEEP if u32::from(low) == record::SWEEP_TAG_LEAKS => format!(
            "counters: lost work {} GPU resets {} fence timeouts {}; first leaker offset {} leakers {}; \
             pass bits {:04x} {a}",
            hi(w1),
            (w1 >> 8) & 0xff,
            w1 & 0xff,
            (w2 >> 8) & 0xff,
            w2 & 0xff,
            hi(w2)
        ),
        kind::SWEEP => format!("cases finished {low}, two bits per case {w1:08x} {w2:08x} {a}"),
        kind::DEQP => format!(
            "pass {} fail {} notsup {} other {} begun {a} first fail name check {a}",
            lo(w1),
            hi(w1),
            lo(w2),
            hi(w2)
        ),
        kind::DEQP_CASES => {
            let st = two_bits(&[w1, w2], usize::from(low).min(32));
            let n = |v: u32| st.iter().filter(|&&s| s == v).count();
            format!(
                "begun {low}: pass {} fail {} other {}; first fail index {a} name check {a}; GPU resets {a} \
                 fence timeouts {a}",
                n(1),
                n(2),
                n(3)
            )
        }
        kind::DEQP_FAILS => {
            let v: Vec<String> = fails_names_3w(w).iter().map(|c| format!("{c:04x}")).collect();
            format!("fails seen {low}: name checks {} (four to a page, w3 not stored)", v.join(" "))
        }
        kind::DEQP_STATUS => {
            let mut t = String::new();
            for b in w1.to_le_bytes().into_iter().chain(w2.to_le_bytes()).filter(|&b| b != 0) {
                t.push(if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' });
            }
            format!("case index {low} status '{t}' (first 8 of 12 bytes; bytes 9-12 {a})")
        }
        kind::PERF => {
            format!("experiment {} page {}: {} {} {} {} {a} {a}", low >> 3, low & 7, lo(w1), hi(w1), lo(w2), hi(w2))
        }
        kind::WDOG => format!(
            "tag {low} at uptime {a} ms; WCR found 0x{:04x} now 0x{:04x}; flags 0x{:02x} refusal {} WMCR 0x{:04x}",
            lo(w1),
            hi(w1),
            w2 & 0xff,
            (w2 >> 8) & 0xff,
            hi(w2)
        ),
        kind::STAGE => {
            let mut t = format!("0x{low:02x} ");
            let _ = record::write_stage(&mut t, low);
            format!("{t} value {} (0x{w1:08x}) uptime {w2} ms WDOG1 feeds {a}", w1 as i32)
        }
        kind::FINGERPRINT => format!("window crc32 0x{w1:08x} frames {w2} size {a}"),
        kind::GLMARK2 => {
            let [score, ran, failed, unsupported] = record::glmark2_counts(w1, w2);
            format!(
                "score {score} status {} scenes ran {ran} failed {failed} unsupported {unsupported} size {a}; \
                 GL errors {a} incomplete-texture draws {a}",
                (w2 >> 24) & 0xf
            )
        }
        kind::GPU_STOP => format!("cause {low} fence {w1} GPU recoveries {w2} list {a} KiB {a}"),
        kind::CHECK if u32::from(low) == record::CHECK_PROBES => format!(
            "probes: probe {} at ({},{}) reads {},{},{}, the reference {a} (diff {a}); {} probes failing",
            w1 >> 24,
            (w1 >> 12) & 0xfff,
            w1 & 0xfff,
            (w2 >> 16) & 0xff,
            (w2 >> 8) & 0xff,
            w2 & 0xff,
            w2 >> 24
        ),
        kind::CHECK if u32::from(low) == record::CHECK_RUN => format!(
            "run: the check passed, the run failed later: cause {} at frame {} (detail ms {a}); rc {}; \
             draws refused {}; keep high-water {a}, blocked on the GPU {a}",
            (w1 >> record::CHECK_RUN_CAUSE_SHIFT) & record::CHECK_RUN_CAUSE_MAX,
            w2 & record::CHECK_RUN_FRAME_MAX,
            (w1 & 0xff) as u8 as i8,
            (w1 >> record::CHECK_RUN_REFUSED_SHIFT) & record::CHECK_RUN_REFUSED_MAX
        ),
        kind::CHECK => format!("what {low}: {w1} (w2 {w2:08x}, w3 {a})"),
        kind::DISPLAY => {
            format!("pixel clock w1 0x{w1:08x} DSI lane rate {w2} kHz; IRQ_STAT {a} health {a} BPP {a} pitch {a}")
        }
        kind::GL_ERROR => {
            format!("error {low}: w1 0x{w1:08x} FNV-1a of the entry point 0x{w2:08x}; GL errors raised {a}")
        }
        kind::GM_PAGE => format!("tag 0x{low:02x}: w1 {w1:08x} w2 {w2:08x} w3 {a}"),
        kind::RATE => format!("status {low} frames {w1} in {w2} ms size {a}"),
        _ => String::new(),
    };
    s.push_str(&body);
    s
}

/// `decode` for a three-word record: `describe3`, then the case names behind
/// the name checks the record still holds (DEQP's first fail and DEQP_CASES'
/// are in w3: none), the DEQP_CASES bits and a DEQP_FAILS page's four names.
pub fn decode3(w: &[u32; 4], cases: &CaseList) -> Vec<String> {
    let mut out = vec![describe3(w)];
    if !magic(w) {
        return out;
    }
    let (k, low) = kind_low(w[0]);
    match k {
        kind::DEQP => out.push("  begun derived: pass + fail + notsup + other (w3 not stored)".to_string()),
        kind::DEQP_CASES => {
            let bits: String =
                two_bits(&[w[1], w[2]], usize::from(low)).iter().map(|&s| ['.', 'P', 'F', '?'][s as usize]).collect();
            out.push(format!("  bits per case (the filter's names in sorted order): {bits}"));
        }
        kind::DEQP_FAILS => {
            for c in fails_names_3w(w).into_iter().filter(|&c| c != 0xffff) {
                let n = cases.by_crc(c as u16, "");
                out.push(format!(
                    "  {c:04x} {}",
                    if n.is_empty() { "(no name in the case list)".to_string() } else { n.join(" ") }
                ));
            }
        }
        _ => {}
    }
    out
}

fn wcr(v: u32) -> String {
    format!(
        "0x{v:04x} ({}, WT 0x{:02x} = {:.1} s)",
        if v & 4 != 0 { "WDE" } else { "stopped" },
        v >> 8,
        f64::from((v >> 8) + 1) / 2.0
    )
}

/// The headers whose entry points kind GL_ERROR's hash is matched against.
const GL_HEADERS: [&str; 3] = [
    include_str!("../../../include/GLES3/gl32.h"),
    include_str!("../../../include/GLES2/gl2ext.h"),
    include_str!("../../../include/EGL/egl.h"),
];

/// The GL or EGL entry point whose name has FNV-1a `hash` (record::fnv1a32,
/// kind GL_ERROR's w2), from the headers' identifiers.
pub fn gl_entry_name(hash: u32) -> Option<&'static str> {
    GL_HEADERS
        .iter()
        .flat_map(|h| h.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
        .filter(|w| w.starts_with("gl") || w.starts_with("egl"))
        .find(|w| record::fnv1a32(w.as_bytes()) == hash)
}

/// The decoded record: record.rs's `describe` line, then what only the host
/// can add (the case names behind name checks, the bits of a DEQP_CASES run,
/// the field names of the WDOG registers and console stage 1, the entry point
/// of a GL_ERROR).
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
        let n = cases.by_crc(c as u16, "");
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
        kind::GL_ERROR if w1 >> record::GL_ERROR_KEPT_SHIFT > u32::from(low) => out.push(format!(
            "  entry point: {}",
            gl_entry_name(w2).unwrap_or("(no GL or EGL entry point of the headers has this FNV-1a)")
        )),
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

const GLMARK2_PAGES_USAGE: &str =
    "usage: yakogl-tools record glmark2-pages [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...";

/// The console lines of a glmark2 run decoded from the kind GM_PAGE records
/// of its page runs, and the cluster's `throughput:` from a RATE record
/// (record.rs write_gm_lines): what `tools/board.sh imx8mp gm-lines` writes
/// as glmark2-pages.txt. Records of other kinds are passed over.
pub fn glmark2_pages(recs: &[[u32; 4]]) -> String {
    let mut s = String::new();
    // a String cannot fail to take the text
    let _ = record::write_gm_lines(&mut s, recs);
    s
}

pub fn from_nvmem(b: &[u8]) -> R<[u32; 4]> {
    if b.len() < 16 {
        return Err(Error::Exit(format!("the nvmem file has {} bytes, a record needs 16", b.len())));
    }
    let word = |i: usize| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
    Ok([word(0), word(1), word(2), word(3)])
}

/// Every record line of a console log, in order: record.rs's `parse_line`
/// (`yk-record: ...`, anything before the prefix).
pub fn from_log(text: &str) -> Vec<[u32; 4]> {
    text.replace(['\0', '\r'], "").lines().filter_map(record::parse_line).collect()
}

/// The records a page decoder (`record msaa`, `record kickdiff`) is given:
/// `--log <file>` (every record line), `--nvmem <file>` (one record) and
/// loose hex words, four to a record, in any mix and order. None for
/// `-h`/`--help` (the usage is printed); a usage error for no record at all
/// or a word count that is not a multiple of four.
pub fn records_from_args(args: &[String], usage: &str) -> R<Option<Vec<[u32; 4]>>> {
    let mut recs: Vec<[u32; 4]> = Vec::new();
    let mut loose: Vec<u32> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{usage}");
                return Ok(None);
            }
            "--log" | "--nvmem" => {
                let f = it.next().ok_or_else(|| Error::Usage(usage.into()))?;
                let b = py::read_bytes(f)?;
                if a == "--log" {
                    recs.extend(from_log(&py::lossy_text(&b)));
                } else {
                    recs.push(from_nvmem(&b)?);
                }
            }
            w => {
                let t = w.trim_start_matches("0x").trim_start_matches("0X");
                let v = u32::from_str_radix(t, 16)
                    .map_err(|_| Error::Usage(format!("{usage}\n\n'{w}' is not a 32-bit hex word")))?;
                loose.push(v);
            }
        }
    }
    if !loose.len().is_multiple_of(4) {
        return Err(Error::Usage(format!("{usage}\n\nrecords are four words each; {} words given", loose.len())));
    }
    recs.extend(loose.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]));
    if recs.is_empty() {
        return Err(Error::Usage(usage.into()));
    }
    Ok(Some(recs))
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

fn words(a: &[String], three: bool) -> R<[u32; 4]> {
    if three && a.len() == 3 {
        return Ok([parse_word(&a[0])?, parse_word(&a[1])?, parse_word(&a[2])?, 0]);
    }
    if a.len() != 4 {
        return Err(Error::Usage(format!("{USAGE}\n\na record is four words (three with --words 3)")));
    }
    Ok([parse_word(&a[0])?, parse_word(&a[1])?, parse_word(&a[2])?, parse_word(&a[3])?])
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Decode,
    Classify,
    Line,
}

/// A record's words as text: w3 is `-` for a three-word record.
fn words_text(w: &[u32; 4], three: bool) -> String {
    if three {
        format!("{:08x} {:08x} {:08x} {ABSENT}", w[0], w[1], w[2])
    } else {
        format!("{:08x} {:08x} {:08x} {:08x}", w[0], w[1], w[2], w[3])
    }
}

fn emit(w: &[u32; 4], mode: Mode, three: bool, cases: &mut Option<CaseList>) -> R<()> {
    // a three-word record has no w3: whatever the reader gave is not read
    let w = &if three { [w[0], w[1], w[2], 0] } else { *w };
    match mode {
        Mode::Classify => println!("{}", classify(w)),
        Mode::Line if three => println!("{}", line3(w)),
        Mode::Line => println!("{}", line(w)),
        Mode::Decode => {
            if cases.is_none() {
                *cases = Some(CaseList::load()?);
            }
            if let Some(c) = cases.as_ref() {
                for l in if three { decode3(w, c) } else { decode(w, c) } {
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
            let prefix = args.get(2).map(String::as_str).unwrap_or("");
            for n in CaseList::load()?.by_crc(want, prefix) {
                println!("{n}");
            }
            return Ok(());
        }
        "msaa" => return crate::msaarec::main(&args[1..]),
        "kickdiff" => return crate::kickdiffrec::main(&args[1..]),
        "glmark2-pages" => {
            let Some(recs) = records_from_args(&args[1..], GLMARK2_PAGES_USAGE)? else {
                return Ok(());
            };
            print!("{}", glmark2_pages(&recs));
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
    let mut three = false;
    let mut source: Option<(&str, &str)> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--classify" => mode = Mode::Classify,
            "--line" => mode = Mode::Line,
            "--words" => {
                three = match it.next().map(String::as_str) {
                    Some("3") => true,
                    Some("4") => false,
                    _ => return Err(Error::Usage(format!("{USAGE}\n\n--words takes 3 or 4"))),
                }
            }
            "--nvmem" | "--log" => {
                let f = it.next().ok_or_else(|| Error::Usage(USAGE.into()))?;
                source = Some((a.as_str(), f.as_str()));
            }
            _ => rest.push(a.clone()),
        }
    }
    let mut cases = None;
    match source {
        None => emit(&words(&rest, three)?, mode, three, &mut cases),
        Some((_, _)) if !rest.is_empty() => Err(Error::Usage(format!("{USAGE}\n\nwords and a file together"))),
        Some(("--nvmem", f)) => emit(&from_nvmem(&py::read_bytes(f)?)?, mode, three, &mut cases),
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
                    println!("record {}/{}: {}", i + 1, recs.len(), words_text(w, three));
                    emit(w, mode, three, &mut cases)?;
                }
                println!("verdict record: {}", words_text(&v, three));
                if let Some(t) = throughput_line(&recs) {
                    println!("{t}");
                }
                Ok(())
            } else {
                emit(&v, mode, three, &mut cases)
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

    /// The library's draw steps (src/draw_steps.rs) are record.rs's
    /// CHECK_RUN_STEP_* from CHECK_RUN_STEP_GL_PROGRAM, in order and all of
    /// them: the cluster copies the library's totals into the record by
    /// number, so a step added or moved on one side only would be named
    /// as another step on the board.
    #[test]
    fn the_librarys_draw_steps_are_the_records() {
        use yakogl::draw_steps::{Step, RECORD_STEP_BASE, STEPS};
        let pairs = [
            (Step::GlProgram, record::CHECK_RUN_STEP_GL_PROGRAM),
            (Step::GlUniforms, record::CHECK_RUN_STEP_GL_UNIFORMS),
            (Step::GlAttribs, record::CHECK_RUN_STEP_GL_ATTRIBS),
            (Step::GlTextures, record::CHECK_RUN_STEP_GL_TEXTURES),
            (Step::GlState, record::CHECK_RUN_STEP_GL_STATE),
            (Step::GcRest, record::CHECK_RUN_STEP_GC_REST),
            (Step::GcDs, record::CHECK_RUN_STEP_GC_DS),
            (Step::GcTiledCopy, record::CHECK_RUN_STEP_GC_TILED_COPY),
            (Step::GcFrame, record::CHECK_RUN_STEP_GC_FRAME),
            (Step::GcUniforms, record::CHECK_RUN_STEP_GC_UNIFORMS),
            (Step::GcVertex, record::CHECK_RUN_STEP_GC_VERTEX),
            (Step::GcEmit, record::CHECK_RUN_STEP_GC_EMIT),
        ];
        assert_eq!(RECORD_STEP_BASE, record::CHECK_RUN_STEP_GL_PROGRAM);
        assert_eq!(pairs.len(), STEPS, "every library step is paired");
        assert_eq!(record::CHECK_RUN_STEPS as usize, RECORD_STEP_BASE as usize + STEPS, "and no record step is left");
        for (s, c) in pairs {
            assert_eq!(s as u32 + RECORD_STEP_BASE, c, "{s:?}");
        }
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
        // the population: the golden file's 59 blocks (45 + GLMARK2 on- and off-screen, with
        // GLMARK2_SCENES and with GLMARK2_GL + GPU_STOP + four CHECK: a probe, and CHECK_RUN for a fence
        // timeout, a frame over the budget with its slowest step, and nothing measured
        // + DISPLAY + three set records: CASE_SET failed and passed, and the earlier
        // CASE_SET_FAIL form + GL_ERROR + GM_PAGE and RATE), every kind of record.rs's table among them
        assert_eq!(blocks, 61, "the golden file's block count changed: re-derive the pin");
        let want: std::collections::BTreeSet<u8> = record::KINDS.iter().map(|k| k.value).chain([32]).collect();
        assert_eq!(kinds_seen, want, "every kind (and one unknown) has a golden block");
    }

    /// A GLES3 name check decodes from the default case list. The i.MX8MP
    /// dEQP table image (docs/es3/board-recipe-imx8mp.md) has no console: a
    /// failing case comes back as the CRC-16 of its whole name and nothing
    /// else. That list held only GLES2 names and `crc` only looked for the
    /// GLES2 prefix, so a GLES3 record's failures stayed `crc:<hex>`;
    /// tools/cts/gles3-mustpass.txt is in the default list now.
    #[test]
    fn a_gles3_name_check_decodes() {
        // the default list of CaseList::load() with DEQP_CASELIST unset,
        // built directly so an operator's DEQP_CASELIST cannot change it
        // (load() itself is exercised end to end by tools/board_selftest.sh)
        let mut text = py::read_text(&py::join(&repo_root(), "tools/cts/gles2-mustpass.txt")).unwrap();
        text.push_str(&py::read_text(&py::join(&repo_root(), "tools/cts/gles3-mustpass.txt")).unwrap());
        let cl = CaseList::from_text(&text);
        let name = "dEQP-GLES3.functional.attribute_location.bind.float";
        let crc = record::crc16_ccitt_false(name.as_bytes());
        assert!(cl.by_crc(crc, "").contains(&name), "the default list holds the GLES3 names");
        assert!(cl.by_crc(crc, "dEQP-GLES2.").is_empty(), "a prefix still restricts");
        // the row's DEQP record: w1 failed|passed, w3 the first fail | begun
        let w = [0x594b0801, 0x00020001, 0, (u32::from(crc) << 16) | 3];
        assert!(decode(&w, &cl).iter().any(|l| l.contains(name)), "first fail named: {:?}", decode(&w, &cl));
        // and a DEQP_FAILS page names its six checks (w1..w3, 16 bits each)
        let p = [0x594b0a02, u32::from(crc), u32::MAX, u32::MAX];
        assert!(decode(&p, &cl).iter().any(|l| l.contains(name)), "page check named");
        // GLES2 records decode as before (the golden file, tools/board.sh results_tsv)
        let n2 = "dEQP-GLES2.functional.attribute_location.bind.float";
        assert!(cl.by_crc(record::crc16_ccitt_false(n2.as_bytes()), "").contains(&n2));
    }

    /// classify on records with known meaning: the two 2026-10-03 netboot
    /// board records (step C and its kexec control,
    /// docs/notes/imx8mp-netboot.md "Board run 2026-10-03")
    /// and one of each class. Moved from
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

    /// `record glmark2-pages` on the synthetic page session of
    /// driver/rust/tests/gm-record (its runs' snvs.txt, the words board.sh
    /// passes) prints that fixture's glmark2-pages.txt; RATE's status is its
    /// tag and GM_PAGE carries none.
    #[test]
    fn glmark2_pages_of_the_fixture() {
        let dir = py::join(&repo_root(), "driver/rust/tests/gm-record");
        let mut runs: Vec<_> = std::fs::read_dir(py::join(&dir, "runs")).unwrap().map(|e| e.unwrap().path()).collect();
        runs.sort();
        let mut words: Vec<String> = Vec::new();
        for r in &runs {
            words.extend(std::fs::read_to_string(r.join("snvs.txt")).unwrap().split_whitespace().map(String::from));
        }
        let recs = records_from_args(&words, GLMARK2_PAGES_USAGE).unwrap().unwrap();
        assert_eq!(recs.len(), runs.len());
        assert_eq!(glmark2_pages(&recs), py::read_text(&py::join(&dir, "glmark2-pages.txt")).unwrap());
        let rate = |status: u32| [record::w0(kind::RATE, status as u8), 3600, 60_012, record::size_word(1280, 800)];
        assert_eq!(status(&rate(record::STATUS_PASS)), Status::Pass);
        assert_eq!(status(&rate(record::STATUS_FAIL)), Status::Fail);
        assert_eq!(status(&rate(0)), Status::Invalid);
        let page = recs.iter().find(|w| kind_low(w[0]).0 == kind::GM_PAGE).unwrap();
        assert_eq!(
            line(page),
            "class=verdict kind=24 name=GM_PAGE status=n/a resets=absent unsupported=absent tally=-"
        );
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
        assert!(from_log("gl_triangle: record 594b0400 ff403f80 35706c4e 00400040\n").is_empty());
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
        let cases: [([u32; 4], Status); 23] = [
            ([0x594b0202, 0x0000_0001, 1, 0], Status::Pass),    // APP pass
            ([0x594b0202, 0x0000_0602, 1, 0], Status::Fail),    // APP fail, stage 6
            ([0x594b0202, 0x0000_0003, 1, 0], Status::Invalid), // APP status 3
            ([0x594b0601, 0x08420882, 0x00210842, 0x98080883], Status::Pass), // PROBES tag 1
            ([0x594b0602, 0x08420882, 0x00210842, 0x98080883], Status::Fail), // PROBES tag 2
            ([0x594b0600, 0, 0, 0], Status::Invalid),           // PROBES tag 0
            ([0x594b1501, 0x0724814f, 0x01002533, 0x14001133], Status::Fail), // CHECK: a failure always
            ([0x594b0101, 0x000aff01, 12, 0], Status::Pass),    // CASE 10 pass
            ([0x594b0101, 0x000a0302, 0xff, 0xfe], Status::Fail), // CASE 10 check 3
            ([0x594b0101, 0x000aff06, 3, 4], Status::Fail),     // CASE_SET_FAIL
            ([0x594b0101, 0x000aff00, 0, 0], Status::Invalid),  // CASE status 0
            // CASE_SET: 48 of 48 run with case 16 skipped is a pass; 47 of 48
            // (first failure 44) a failure; nothing run is not a pass; a
            // failure named while every case passed is not a reading
            ([0x594b010e, 0xffff_0107, 0xffff_0030, 0x0010_0030], Status::Pass),
            ([0x594b010e, 0x002c_0107, 0xffff_002f, 0x0010_0030], Status::Fail),
            ([0x594b010e, 0xffff_3107, 0xffff_0000, 0x0010_0000], Status::Fail),
            ([0x594b010e, 0x002c_0107, 0xffff_0030, 0x0010_0030], Status::Invalid),
            ([0x594b0300, 0x96000045, 0x40001234, 0], Status::Fail), // FAULT
            // GLMARK2: the AM62P run of 2026-10-04 that printed "glmark2: FAIL rc=-5"
            // (score 533, 30 ran, 1 failed, 2 unsupported); board.sh read it as n/a
            ([0x594b1201, 0x0000_0215, 0x021e_0102, 0x0320_0258], Status::Fail),
            ([0x594b1201, 0x0000_0132, 0x1121_0000, 0x0780_0438], Status::Pass), // GLMARK2 pass, off-screen
            ([0x594b1201, 0x0000_0132, 0x0021_0000, 0x0780_0438], Status::Invalid), // GLMARK2 status 0
            ([0x594b040e, 0xff403f80, 0x35706c4e, 0x04000258], Status::NotApplicable), // PICTURE
            ([0x594b160f, 0x80010eb4, 0x00068268, 0x593f0000], Status::NotApplicable), // DISPLAY
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

    /// --words 3: w3 is never stored, so nothing may depend on it. Every
    /// kind of record.rs's table (and one unknown), every variant a tag
    /// picks: the same text whatever w3 holds.
    #[test]
    fn words3_ignores_w3_for_every_kind() {
        let cases = CaseList::from_text("dEQP-GLES2.a\n");
        for k in record::KINDS.iter().map(|k| k.value).chain([32]) {
            for low in [1u32, record::SWEEP_TAG_LEAKS, record::CHECK_PROBES, record::CHECK_RUN] {
                for w1 in [0x1234_5601u32, record::CASE_SET | 0x0001_0000] {
                    let w = |w3: u32| [0x594b0000 | u32::from(k) << 8 | low, w1, 0x0002_0001, w3];
                    // emit() zeroes w3 for --words 3: the functions are fed what it feeds
                    let z = |mut x: [u32; 4]| {
                        x[3] = 0;
                        x
                    };
                    let (a, b) = (z(w(0)), z(w(0xdead_beef)));
                    assert_eq!(describe3(&a), describe3(&b));
                    assert_eq!(line3(&a), line3(&b));
                    let d = decode3(&b, &cases).join("\n");
                    assert!(!d.to_lowercase().contains("deadbeef"), "kind {k}: {d}");
                }
            }
        }
    }

    #[test]
    fn words3_marks_the_w3_fields_absent() {
        let a = ABSENT;
        let d = |w: [u32; 4]| describe3(&w);
        // DEQP: cases begun and the first failing name check are w3's
        let w = [0x594b0801, 0x0002_0064, 0x0001_0003, 0];
        assert!(d(w).ends_with(&format!("begun {a} first fail name check {a}")), "{}", d(w));
        // DEQP_CASES: the first failure and the counters are w3's, begun is the tag
        assert_eq!(
            d([0x594b0904, 0b01_10_01_01, 0, 0]),
            format!(
                "DEQP_CASES (9) tag 4: begun 4: pass 3 fail 1 other 0; first fail index {a} name check {a}; \
                 GPU resets {a} fence timeouts {a}"
            )
        );
        // DEQP_STATUS: bytes 9-12 are w3's
        let w = [0x594b0b05, u32::from_le_bytes(*b"Inte"), u32::from_le_bytes(*b"rnal"), 0];
        assert!(d(w).contains("status 'Internal' (first 8 of 12 bytes; bytes 9-12 -)"), "{}", d(w));
        // PROBES: p4 and the crc
        let w = [0x594b0601, 0x2222_1111, 0x4444_3333, 0];
        assert_eq!(d(w), "PROBES (6) tag 1: status 1 p0 1111 p1 2222 p2 3333 p3 4444 p4 - crc16 - (RGB565)");
        // kinds whose w3 is a size, a count, an uptime, a want value
        assert!(d([0x594b040e, 1, 2, 0]).contains("size -"));
        assert!(d([0x594b0503, 0x0a0, 77, 0]).ends_with("count -"));
        assert!(d([0x594b0d01, 0, 0, 0]).contains("uptime - ms"));
        assert!(d([0x594b0c00, 1, 2, 0]).ends_with("- -"));
        assert!(d([0x594b0701, 1, 2, 0]).ends_with(" -"));
        assert!(d([0x594b0201, 0x0000_0201, 3, 0]).ends_with("frames 3 app -"));
        assert!(d([0x594b0103, 0x0003_0001, 5, 0]).ends_with("want -"));
        // a bad magic is as before
        assert_eq!(d([0x12345678, 0, 0, 0]), "NO RECORD (w0 = 12345678)");
    }

    #[test]
    fn words3_tally_line_and_fails_pages() {
        // DEQP: begun is derived and the line says so
        let w = [0x594b0801, 0x0002_0064, 0x0001_0003, 0];
        assert_eq!(tally3(&w), Some([100, 2, 3, 1, 106]));
        assert_eq!(
            line3(&w),
            "class=verdict kind=8 name=DEQP status=n/a resets=absent unsupported=absent words=3 begun=derived \
             tally=100,2,3,1,106"
        );
        // the 4-word reading is unchanged: w3 there is cases begun
        assert_eq!(tally(&[0x594b0801, 0x0002_0064, 0x0001_0003, 0x94050067]), Some([100, 2, 3, 1, 0x67]));
        // DEQP_CASES: begun is the tag, resets are absent (w3's)
        let w = [0x594b0904, 0b01_10_01_01, 0, 0];
        assert_eq!(tally3(&w), Some([3, 1, 0, 0, 4]));
        assert_eq!(resets3(&w), None);
        assert_eq!(
            line3(&w),
            "class=verdict kind=9 name=DEQP_CASES status=n/a resets=absent unsupported=absent words=3 begun=tag \
             tally=3,1,0,0,4"
        );
        // no tally
        assert!(line3(&[0x594b0202, 0x0000_0602, 1, 0]).contains("words=3 begun=- tally=-"));
        // DEQP_FAILS: four names, w1 low first, then w2; 0xffff none
        let w = [0x594b0a07, 0x2222_1111, 0xffff_3333, 0];
        assert_eq!(fails_names_3w(&w), [0x1111, 0x2222, 0x3333, 0xffff]);
        let cl = CaseList::from_text("dEQP-GLES2.x\n");
        let crc = u32::from(record::crc16_ccitt_false(b"dEQP-GLES2.x"));
        let out = decode3(&[0x594b0a07, 0xffff_0000 | crc, 0xffff_ffff, 0], &cl);
        assert_eq!(
            out[0],
            format!("DEQP_FAILS (10) tag 7: fails seen 7: name checks {crc:04x} ffff ffff ffff (four to a page, w3 not stored)")
        );
        assert_eq!(out[1..], [format!("  {crc:04x} dEQP-GLES2.x")]);
        assert_eq!(record::DEQP_FAILS_PER_PAGE_3W, 4);
    }

    /// A three-word name page with 1, 2, 3 and 4 names; unused slots are
    /// 0xffff (a whole unused word 0xffffffff) and are not listed.
    #[test]
    fn words3_fails_pages_with_one_to_four_names() {
        let names = ["dEQP-GLES2.a", "dEQP-GLES2.b", "dEQP-GLES2.c", "dEQP-GLES2.d"];
        let cl = CaseList::from_text(&(names.join("\n") + "\n"));
        let c: Vec<u32> = names.iter().map(|n| u32::from(record::crc16_ccitt_false(n.as_bytes()))).collect();
        let pages = [
            [0xffff_0000 | c[0], 0xffff_ffff],
            [(c[1] << 16) | c[0], 0xffff_ffff],
            [(c[1] << 16) | c[0], 0xffff_0000 | c[2]],
            [(c[1] << 16) | c[0], (c[3] << 16) | c[2]],
        ];
        for (n, p) in pages.iter().enumerate() {
            let w = [0x594b0a01, p[0], p[1], 0];
            let got = fails_names_3w(&w);
            assert_eq!(got.iter().filter(|&&x| x != 0xffff).count(), n + 1);
            let out = decode3(&w, &cl);
            assert_eq!(out.len(), n + 2, "{out:?}");
            for (i, l) in out[1..].iter().enumerate() {
                assert_eq!(*l, format!("  {:04x} {}", c[i], names[i]));
            }
            // w3 is never read
            assert_eq!(decode3(&[w[0], w[1], w[2], 0x1234_5678], &cl), out);
        }
        // an empty page (0xffffffff twice) names nothing
        assert_eq!(decode3(&[0x594b0a01, 0xffff_ffff, 0xffff_ffff, 0], &cl).len(), 1);
    }

    /// A four-word record decodes as before: the page holds 6 names, the row
    /// has its cases begun and first failing check.
    #[test]
    fn words4_fails_page_and_row_unchanged() {
        let cl = CaseList::from_text("dEQP-GLES2.x\n");
        let d = decode(&[0x594b0a01, 0x2222_1111, 0x4444_3333, 0x6666_5555], &cl).join("\n");
        assert!(d.contains("name checks 1111 2222 3333 4444 5555 6666"), "{d}");
        let d = decode(&[0x594b0801, 0x0002_0064, 0x0001_0003, 0x94050067], &cl).join("\n");
        assert!(d.contains("begun 103") && !d.contains("not stored"), "{d}");
        // CASE_SET cannot be read without cases run
        assert_eq!(status3(&[0x594b010e, 0xffff_0107, 0xffff_0030, 0]), Status::Invalid);
        assert_eq!(status3(&[0x594b0300, 0, 0, 0]), Status::Fail);
    }

    #[test]
    fn words3_arguments() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(words(&a(&["594b0801", "1", "2"]), true).unwrap(), [0x594b0801, 1, 2, 0]);
        assert_eq!(words(&a(&["594b0801", "1", "2", "dead"]), true).unwrap()[3], 0xdead);
        assert!(words(&a(&["594b0801", "1", "2"]), false).is_err());
        assert_eq!(words_text(&[1, 2, 3, 0], true), "00000001 00000002 00000003 -");
        assert_eq!(words_text(&[1, 2, 3, 0], false), "00000001 00000002 00000003 00000000");
    }
}
