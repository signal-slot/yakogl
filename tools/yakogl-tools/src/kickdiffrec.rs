// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools record kickdiff` - the KICK_DIFF pages of gl_test cases 2
//! and 10 (#9, #25), read from the result records they were written to
//! (docs/notes/gc-kick-diff.md, "How to run it on the board").
//!
//!     yakogl-tools record kickdiff [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...
//!
//! A run of the i.MX8MP gc_gl_test image with the selector
//! PERF_SEL | list << 8 | tag, tag 0xe0 | case, ends with one PERF record
//! whose tag names the case (gl_test.c, gl_test_kick_diff_page). This takes
//! the records of any number of runs and prints, for each page, the line the
//! console prints at the end of the armed case (gl_test.c, kick_diff_end),
//! then a `record:` line with what only the page carries: the case's own
//! result and its first failing check, and the first late check. Several
//! boots of one tag are several lines, in input order: a tally counts them.

use crate::record::{self, kind};
use crate::recordcmd::records_from_args;
use crate::{Error, R};

pub const USAGE: &str =
    "usage: yakogl-tools record kickdiff [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...";

/// The page tags: TAG_BASE | case (gl_test.c KICK_DIFF_TAG_BASE).
pub const TAG_BASE: u8 = 0xe0;
const TAG_LAST: u8 = 0xef;
/// w1's verdict byte when the case could not be armed: NOT_ARMED | n, the
/// arming returned -n (gl_test.c KICK_DIFF_NOT_ARMED).
pub const NOT_ARMED: u32 = 0x80;

/// One page as gl_test.c packs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub case: u8,
    /// 0 armed but no check, 1 stable, 2 late; NOT_ARMED | n not armed.
    pub verdict: u8,
    /// The first late check, from 1; 0 none.
    pub first_late: u8,
    /// The case's result: 1 pass, 2 fail, 3 neither.
    pub result: u8,
    /// The case's first failing check; 0xff none, 0xfe 254 or more.
    pub first_bad: u8,
    pub checks: u16,
    pub late: u16,
    pub max_changed: u32,
}

impl Page {
    pub fn from_words(tag: u8, w: [u32; 3]) -> Page {
        Page {
            case: tag - TAG_BASE,
            verdict: w[0] as u8,
            first_late: (w[0] >> 8) as u8,
            result: (w[0] >> 16) as u8,
            first_bad: (w[0] >> 24) as u8,
            checks: w[1] as u16,
            late: (w[1] >> 16) as u16,
            max_changed: w[2],
        }
    }
}

/// Every KICK_DIFF page in `recs` (a PERF record, tag 0xe0..0xef), in order.
pub fn pages(recs: &[[u32; 4]]) -> Vec<Page> {
    recs.iter()
        .filter(|w| w[0] & 0xffff_0000 == record::MAGIC && (w[0] >> 8) as u8 == kind::PERF)
        .filter(|w| (TAG_BASE..=TAG_LAST).contains(&(w[0] as u8)))
        .map(|w| Page::from_words(w[0] as u8, [w[1], w[2], w[3]]))
        .collect()
}

/// The page as lines: the console's own, then the record's.
pub fn decode(p: &Page) -> Vec<String> {
    let c = p.case;
    let v = u32::from(p.verdict);
    let mut out = Vec::new();
    if v & NOT_ARMED != 0 {
        out.push(format!("gl_test:   kick_diff case {c}: not armed (gc_kick_diff_arm_target -{})", v & 0x7f));
    } else {
        let what = match v {
            2 => "late".to_string(),
            1 => "stable".to_string(),
            0 => "no check".to_string(),
            n => format!("verdict {n} (not a value gl_test.c writes)"),
        };
        out.push(format!(
            "gl_test:   kick_diff case {c}: {what}, checks {} late {} max_changed {}",
            p.checks, p.late, p.max_changed
        ));
    }
    let result = match p.result {
        1 => "PASS".to_string(),
        2 => "FAIL".to_string(),
        3 => "neither pass nor fail".to_string(),
        n => format!("result {n} (not a value gl_test.c writes)"),
    };
    let bad = match p.first_bad {
        0xff => "no failing check".to_string(),
        0xfe => "first failing check 254 or later".to_string(),
        n => format!("first failing check {n}"),
    };
    let late = match p.first_late {
        0 => "no late check".to_string(),
        n => format!("first late check {n}"),
    };
    out.push(format!("record: case {c} {result}, {bad}; {late}"));
    out
}

pub fn main(args: &[String]) -> R<()> {
    let Some(recs) = records_from_args(args, USAGE)? else {
        return Ok(());
    };
    let p = pages(&recs);
    if p.is_empty() {
        return Err(Error::Exit("no PERF record with a KICK_DIFF tag (0xe0..0xef) in the input".into()));
    }
    for page in &p {
        for l in decode(page) {
            println!("{l}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::py;

    /// The page gl_test.c's kick_diff_end writes, packed as it packs it.
    fn encode(case: u8, verdict: u32, first_late: u32, rc: i32, bad: i32, st: [u32; 3]) -> [u32; 4] {
        let result = match rc {
            0 => 1,
            -1 => 2,
            _ => 3,
        };
        let bad = if bad < 0 { 0xff } else { (bad as u32).min(0xfe) };
        [
            record::w0(kind::PERF, TAG_BASE + case),
            verdict | first_late.min(0xff) << 8 | result << 16 | bad << 24,
            st[0].min(0xffff) | st[1].min(0xffff) << 16,
            st[2],
        ]
    }

    #[test]
    fn a_late_failing_case_10_round_trips() {
        let r = [encode(10, 2, 8, -1, 23, [8, 1, 4])];
        let p = pages(&r);
        assert_eq!(
            p,
            [Page {
                case: 10,
                verdict: 2,
                first_late: 8,
                result: 2,
                first_bad: 23,
                checks: 8,
                late: 1,
                max_changed: 4
            }]
        );
        assert_eq!(
            decode(&p[0]),
            [
                "gl_test:   kick_diff case 10: late, checks 8 late 1 max_changed 4",
                "record: case 10 FAIL, first failing check 23; first late check 8",
            ]
        );
    }

    #[test]
    fn a_stable_passing_case_2_round_trips() {
        let p = pages(&[encode(2, 1, 0, 0, -1, [21, 0, 0])]);
        assert_eq!(
            decode(&p[0]),
            [
                "gl_test:   kick_diff case 2: stable, checks 21 late 0 max_changed 0",
                "record: case 2 PASS, no failing check; no late check",
            ]
        );
    }

    #[test]
    fn not_armed_names_the_arming_return() {
        let p = pages(&[encode(10, NOT_ARMED | 4, 0, 0, -1, [0, 0, 0])]);
        assert_eq!(decode(&p[0])[0], "gl_test:   kick_diff case 10: not armed (gc_kick_diff_arm_target -4)");
    }

    #[test]
    fn counters_saturate_and_large_check_numbers_say_so() {
        let p = pages(&[encode(10, 2, 300, -1, 400, [70000, 70000, 4096])]);
        assert_eq!((p[0].checks, p[0].late, p[0].first_late, p[0].first_bad), (0xffff, 0xffff, 0xff, 0xfe));
        assert!(decode(&p[0])[1].contains("first failing check 254 or later"));
    }

    #[test]
    fn every_page_is_kept_in_order_and_other_records_are_ignored() {
        let r = [
            encode(10, 1, 0, 0, -1, [8, 0, 0]),
            [record::w0(kind::CASE, 1), 0x000a_0001, 0, 0],
            [record::w0(kind::PERF, 0xf0), 1, 2, 3], // a case 16 page
            encode(10, 2, 8, -1, 23, [8, 1, 4]),
            encode(2, 1, 0, 0, -1, [21, 0, 0]),
        ];
        let p = pages(&r);
        assert_eq!(p.iter().map(|p| (p.case, p.verdict)).collect::<Vec<_>>(), [(10, 1), (10, 2), (2, 1)]);
    }

    /// gl_test.c is where the layout is written; the constants and the
    /// console line here must be the ones it has.
    #[test]
    fn the_layout_is_gl_test_cs() {
        let text = py::read_text(&py::join(&crate::repo_root(), "driver/tools/pvrlab/gl_test.c")).unwrap();
        let def = |name: &str| -> u32 {
            let rx = py::re(&format!(r"(?m)^#define {name}\s+0x([0-9a-f]+)u"));
            let c = rx.captures(&text).unwrap_or_else(|| panic!("{name} is not in gl_test.c"));
            u32::from_str_radix(c.get(1).unwrap().as_str(), 16).unwrap()
        };
        assert_eq!(def("KICK_DIFF_TAG_BASE"), u32::from(TAG_BASE));
        assert_eq!(def("KICK_DIFF_NOT_ARMED"), NOT_ARMED);
        for console in [
            r#""gl_test:   kick_diff case %d: %s, checks %u late %u max_changed %u\n""#,
            r#""gl_test:   kick_diff case %d: not armed (gc_kick_diff_arm_target %d)\n""#,
            r#""late" : s_kd_verdict == 1 ? "stable" : "no check""#,
        ] {
            assert!(text.contains(console), "gl_test.c no longer prints {console}");
        }
    }
}
