// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-tools record msaa` - gl_test case 16's 4x probes, read from the
//! result records they were written to (tools/cts/log/investigations/gc-msaa-record.md).
//!
//!     yakogl-tools record msaa [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...
//!
//! Each run of the i.MX8MP gc_gl_test image with GC_GL_TEST_CASE = 16 and the
//! selector PERF_SEL | tag ends with one PERF record whose tag (0xf0..0xfd)
//! names a page of three words (gl_test.c, gl_test_msaa_page). This takes the
//! records of any number of runs, keeps the last page of each tag, and
//! prints the readings in the console's own words (gl_test.c, case_msaa_at and
//! msaa_print_map), so a log line and a record line can be compared by eye or
//! by diff. A page that was not read is said so on a `record:` line, never
//! printed as zeros.

use std::collections::BTreeMap;

use crate::record::{self, kind};
use crate::recordcmd::records_from_args;
use crate::{Error, R};

pub const USAGE: &str =
    "usage: yakogl-tools record msaa [--log <file>]... [--nvmem <file>]... [<w0> <w1> <w2> <w3>]...";

/// The page tags (gl_test.c, the block above gl_test_msaa_page).
pub const TAG_PIXELS: u8 = 0xf0;
pub const TAG_INFO: u8 = 0xf1;
pub const TAG_SCRATCH: u8 = 0xf2;
pub const TAG_CLEAR: u8 = 0xf3;
pub const TAG_DRAW: u8 = 0xf4;
pub const TAG_CLEAR_MAP: u8 = 0xf8;
pub const TAG_DRAW_MAP: u8 = 0xfb;
const TAG_LAST: u8 = 0xfd;

/// What the info page's first word says was taken (gl_test.c MSAA_REC_*).
pub const REC_PIXELS: u32 = 0x01;
pub const REC_SCRATCH: u32 = 0x02;
pub const REC_CLEAR: u32 = 0x04;
pub const REC_DRAW: u32 = 0x08;
pub const REC_CLEARMAP: u32 = 0x10;
pub const REC_DRAWMAP: u32 = 0x20;
/// Words of one map in the record: 256 chunks of 256 bytes.
pub const MAP_WORDS: usize = 8;

/// The last page of each tag in `recs` (a PERF record, tag 0xf0..0xfd).
pub fn pages(recs: &[[u32; 4]]) -> BTreeMap<u8, [u32; 3]> {
    let mut p = BTreeMap::new();
    for w in recs {
        if w[0] & 0xffff_0000 != record::MAGIC || (w[0] >> 8) as u8 != kind::PERF {
            continue;
        }
        let tag = w[0] as u8;
        if (TAG_PIXELS..=TAG_LAST).contains(&tag) {
            p.insert(tag, [w[1], w[2], w[3]]);
        }
    }
    p
}

fn map_lines(out: &mut Vec<String>, what: &str, tag: u8, n: u32, p: &BTreeMap<u8, [u32; 3]>) {
    let mut words = [0u32; MAP_WORDS];
    let mut have = true;
    for k in 0..3u8 {
        match p.get(&(tag + k)) {
            Some(w) => {
                for (i, v) in w.iter().enumerate() {
                    if let Some(d) = words.get_mut(3 * k as usize + i) {
                        *d = *v;
                    }
                }
            }
            None => have = false,
        }
    }
    if !have {
        out.push(format!("record: x4 {what} map: pages 0x{tag:02x}..0x{:02x} not read", tag + 2));
        return;
    }
    // the console prints up to 1024 chunks; the record keeps the first 256
    let shown = (n as usize).min(MAP_WORDS * 32);
    out.push(format!("gl_test:   x4 {what} map, {n} chunks of 256 bytes:"));
    let mut i = 0;
    while i < shown {
        let line: String =
            (i..shown.min(i + 64)).map(|c| if words[c / 32] >> (c % 32) & 1 != 0 { '#' } else { '.' }).collect();
        out.push(format!("gl_test:     {i:4} {line}"));
        i += 64;
    }
    if shown < n as usize {
        out.push(format!("record: x4 {what} map: only the first {shown} of {n} chunks are in the record"));
    }
}

/// The readings as console lines, in the order the console prints them.
pub fn decode(p: &BTreeMap<u8, [u32; 3]>) -> Vec<String> {
    let mut out = Vec::new();
    match p.get(&TAG_PIXELS) {
        Some(w) => out.push(format!("gl_test:   x4: inside 0x{:08x} outside 0x{:08x} edge 0x{:08x}", w[0], w[1], w[2])),
        None => out.push(format!("record: x4 pixels: page 0x{TAG_PIXELS:02x} not read")),
    }
    let Some(info) = p.get(&TAG_INFO) else {
        out.push(format!("record: x4 scratch, clear only, draw only, maps: info page 0x{TAG_INFO:02x} not read"));
        return out;
    };
    let (taken, bytes, map_n) = (info[0], info[1], info[2]);
    if taken & REC_PIXELS == 0 {
        out.push("record: the 4x half did not run: no probe was taken".into());
    }
    let summary = |out: &mut Vec<String>, bit: u32, tag: u8, label: &str, of: bool| {
        if taken & bit == 0 {
            return false;
        }
        let Some(w) = p.get(&tag) else {
            out.push(format!("record: x4 {label}: page 0x{tag:02x} not read"));
            return true;
        };
        if of {
            out.push(format!(
                "gl_test:   x4 {label}: {} non-zero dwords of {}, first [{}] = 0x{:08x}",
                w[0],
                bytes / 4,
                w[1],
                w[2]
            ));
        } else {
            out.push(format!(
                "gl_test:   x4 {label}: {bytes} bytes, {} non-zero dwords, first [{}] = 0x{:08x}",
                w[0], w[1], w[2]
            ));
        }
        true
    };
    if !summary(&mut out, REC_SCRATCH, TAG_SCRATCH, "scratch", false) && taken & REC_PIXELS != 0 {
        out.push("gl_test:   x4 scratch: none allocated (the draw did not take the 4x path)".into());
    }
    if !summary(&mut out, REC_CLEAR, TAG_CLEAR, "clear only", true) {
        out.push("record: x4 clear only: not taken (no scratch to empty)".into());
    } else if taken & REC_CLEARMAP != 0 {
        map_lines(&mut out, "clear only", TAG_CLEAR_MAP, map_n & 0xffff, p);
    }
    if !summary(&mut out, REC_DRAW, TAG_DRAW, "draw only", true) {
        out.push("record: x4 draw only: not taken (no scratch to empty)".into());
    } else if taken & REC_DRAWMAP != 0 {
        map_lines(&mut out, "draw only", TAG_DRAW_MAP, map_n >> 16, p);
    }
    out
}

pub fn main(args: &[String]) -> R<()> {
    let Some(recs) = records_from_args(args, USAGE)? else {
        return Ok(());
    };
    let p = pages(&recs);
    if p.is_empty() {
        return Err(Error::Exit("no PERF record with a case 16 probe tag (0xf0..0xfd) in the input".into()));
    }
    for l in decode(&p) {
        println!("{l}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::py;

    fn rec(tag: u8, w: [u32; 3]) -> [u32; 4] {
        [record::w0(kind::PERF, tag), w[0], w[1], w[2]]
    }

    /// A hand-made run of the three summary pages and one map: the probe
    /// words of a 4x frame whose draw landed and whose clear did not.
    fn hand_made() -> Vec<[u32; 4]> {
        vec![
            rec(TAG_PIXELS, [0xffff00ff, 0xff000000, 0xff800080]),
            rec(
                TAG_INFO,
                [REC_PIXELS | REC_SCRATCH | REC_CLEAR | REC_DRAW | REC_CLEARMAP | REC_DRAWMAP, 65536, 70 << 16 | 70],
            ),
            rec(TAG_SCRATCH, [8192, 16, 0x0000ffff]),
            rec(TAG_CLEAR, [0, 0, 0]),
            rec(TAG_DRAW, [8192, 16, 0xff00ff]),
            // clear-only map: empty
            rec(TAG_CLEAR_MAP, [0, 0, 0]),
            rec(TAG_CLEAR_MAP + 1, [0, 0, 0]),
            rec(TAG_CLEAR_MAP + 2, [0, 0, 0]),
            // draw-only map: chunks 0..3 and 64 written
            rec(TAG_DRAW_MAP, [0xf, 0, 1]),
            rec(TAG_DRAW_MAP + 1, [0, 0, 0]),
            rec(TAG_DRAW_MAP + 2, [0, 0, 0]),
        ]
    }

    #[test]
    fn decodes_in_the_consoles_words() {
        let got = decode(&pages(&hand_made()));
        assert_eq!(got[0], "gl_test:   x4: inside 0xffff00ff outside 0xff000000 edge 0xff800080");
        assert_eq!(got[1], "gl_test:   x4 scratch: 65536 bytes, 8192 non-zero dwords, first [16] = 0x0000ffff");
        assert_eq!(got[2], "gl_test:   x4 clear only: 0 non-zero dwords of 16384, first [0] = 0x00000000");
        assert_eq!(got[3], "gl_test:   x4 clear only map, 70 chunks of 256 bytes:");
        assert_eq!(got[4], format!("gl_test:        0 {}", ".".repeat(64)));
        assert_eq!(got[5], format!("gl_test:       64 {}", ".".repeat(6)));
        assert_eq!(got[6], "gl_test:   x4 draw only: 8192 non-zero dwords of 16384, first [16] = 0x00ff00ff");
        assert_eq!(got[7], "gl_test:   x4 draw only map, 70 chunks of 256 bytes:");
        assert_eq!(got[8], format!("gl_test:        0 {}{}", "####", ".".repeat(60)));
        assert_eq!(got[9], format!("gl_test:       64 {}{}", "#", ".".repeat(5)));
        assert_eq!(got.len(), 10);
    }

    #[test]
    fn a_missing_page_is_said_not_zeroed() {
        let mut r = hand_made();
        r.retain(|w| w[0] as u8 != TAG_DRAW && w[0] as u8 != TAG_CLEAR_MAP + 1);
        let got = decode(&pages(&r));
        assert!(got.contains(&"record: x4 draw only: page 0xf4 not read".to_string()), "{got:?}");
        assert!(got.contains(&"record: x4 clear only map: pages 0xf8..0xfa not read".to_string()), "{got:?}");
        // the pixels and the scratch are still there
        assert!(got[0].starts_with("gl_test:   x4: inside"));
        // no info page: only the pixels can be decoded
        let only = decode(&pages(&r[..1]));
        assert_eq!(only.len(), 2);
        assert!(only[1].contains("info page 0xf1 not read"));
    }

    #[test]
    fn no_scratch_is_the_consoles_none_allocated() {
        let r = [rec(TAG_PIXELS, [0xff000000, 0xff000000, 0xff000000]), rec(TAG_INFO, [REC_PIXELS, 0, 0])];
        let got = decode(&pages(&r));
        assert_eq!(got[1], "gl_test:   x4 scratch: none allocated (the draw did not take the 4x path)");
    }

    #[test]
    fn the_last_page_of_a_tag_wins_and_other_records_are_ignored() {
        let r = [
            rec(TAG_PIXELS, [1, 2, 3]),
            [record::w0(kind::CASE, 1), 0x0010_0000, 0, 0],
            rec(0x00, [9, 9, 9]), // a PERF page of the old frame-time breakdown
            rec(TAG_PIXELS, [4, 5, 6]),
        ];
        let p = pages(&r);
        assert_eq!(p.len(), 1);
        assert_eq!(p[&TAG_PIXELS], [4, 5, 6]);
    }

    /// gl_test.c is where the layout is written; the constants here must be
    /// the ones it has.
    #[test]
    fn the_layout_is_gl_test_cs() {
        let text = py::read_text(&py::join(&crate::repo_root(), "driver/tools/pvrlab/gl_test.c")).unwrap();
        let def = |name: &str| -> u32 {
            let rx = py::re(&format!(r"(?m)^#define {name}\s+0x([0-9a-f]+)u|^#define {name}\s+(\d+)u"));
            let c = rx.captures(&text).unwrap_or_else(|| panic!("{name} is not in gl_test.c"));
            match c.get(1) {
                Some(h) => u32::from_str_radix(h.as_str(), 16).unwrap(),
                None => c.get(2).unwrap().as_str().parse().unwrap(),
            }
        };
        assert_eq!(def("MSAA_REC_PIXELS"), REC_PIXELS);
        assert_eq!(def("MSAA_REC_SCRATCH"), REC_SCRATCH);
        assert_eq!(def("MSAA_REC_CLEAR"), REC_CLEAR);
        assert_eq!(def("MSAA_REC_DRAW"), REC_DRAW);
        assert_eq!(def("MSAA_REC_CLEARMAP"), REC_CLEARMAP);
        assert_eq!(def("MSAA_REC_DRAWMAP"), REC_DRAWMAP);
        assert_eq!(def("MSAA_MAP_WORDS") as usize, MAP_WORDS);
        for t in ["0xf0u", "0xf1u", "0xf2u", "0xf4u", "0xf8u", "0xfdu"] {
            assert!(
                text.contains(&format!("tag == {t}"))
                    || text.contains(&format!("tag >= {t}"))
                    || text.contains(&format!("tag <= {t}")),
                "tag {t}"
            );
        }
    }
}
