// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Decoders for the partial-render stall dump (docs/notes/pvr-render-pool.md,
//! "Design: the firmware's partial render").
//!
//! When a render stops, the board log must say why: the firmware's assert
//! buffer, the HWRT's partial-render state, the command words the firmware was
//! given, and which allocation an address belongs to. Each is a pure function
//! over a byte or word slice, so it builds and tests on the host with no MMIO;
//! the caller reads the memory and logs the line. There is no allocator: a
//! decoder returns borrowed slices and fixed arrays, and a formatter writes
//! one log line into a [`core::fmt::Write`]. Nothing here is wired into the
//! stall path yet.
//!
//! Only offsets pinned in [`super::fwif`] are used. The words of the HWRT's
//! `sRTACtl` are printed raw: their inner offsets are not pinned yet.

use super::fwif::{
    FILE_INFO_BUF_INFO, FILE_INFO_BUF_LINE_NUM, FILE_INFO_BUF_PATH, HWRTDATA_CLEANUP_EXECUTED,
    HWRTDATA_CLEANUP_SUBMITTED, HWRTDATA_FLAGS, HWRTDATA_HAS_LAST_TA, HWRTDATA_PARTIAL_RENDERED, HWRTDATA_RTA_CTL,
    HWRTDATA_RTA_CTL_SIZE, HWRTDATA_SIZE, HWRTDATA_STATE,
};
use core::fmt::{self, Write};

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    let w = b.get(off..off + 4)?;
    Some(u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

/// The bytes of `b` before the first NUL (all of `b` without one).
fn cstr(b: &[u8]) -> &[u8] {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    &b[..n]
}

/// Write `b` as lossy ASCII: bytes outside printable ASCII show as `?`.
fn put_ascii(w: &mut dyn Write, b: &[u8]) -> fmt::Result {
    for &c in b {
        w.write_char(if (0x20..0x7f).contains(&c) { c as char } else { '?' })?;
    }
    Ok(())
}

/// A firmware assert: `RGXFWIF_FILE_INFO_BUF`, text borrowed from the buffer
/// and cut at its NUL.
#[derive(Debug, PartialEq, Eq)]
pub struct FwAssert<'a> {
    pub path: &'a [u8],
    pub info: &'a [u8],
    pub line: u32,
}

/// Decode the `RGXFWIF_FILE_INFO_BUF` at `TRACEBUF_ASSERT_BUF`. `None` when
/// the path is empty (no assert) or `b` is too short to hold the buffer.
pub fn assert_buf(b: &[u8]) -> Option<FwAssert<'_>> {
    let line = u32_at(b, FILE_INFO_BUF_LINE_NUM)?;
    let path = cstr(&b[FILE_INFO_BUF_PATH..FILE_INFO_BUF_INFO]);
    if path.is_empty() {
        return None;
    }
    let info = cstr(&b[FILE_INFO_BUF_INFO..FILE_INFO_BUF_LINE_NUM]);
    Some(FwAssert { path, info, line })
}

/// `render: stall: FW assert <path>:<line> <info>`
pub fn fmt_assert(w: &mut dyn Write, a: &FwAssert) -> fmt::Result {
    w.write_str("render: stall: FW assert ")?;
    put_ascii(w, a.path)?;
    write!(w, ":{} ", a.line)?;
    put_ascii(w, a.info)
}

/// Words of `sRTACtl`.
pub const RTA_CTL_WORDS: usize = HWRTDATA_RTA_CTL_SIZE / 4;

/// The partial-render state of an `RGXFWIF_HWRTDATA`.
#[derive(Debug, PartialEq, Eq)]
pub struct HwrtPrState {
    /// `eState`.
    pub state: u32,
    /// `ui32HWRTDataFlags`.
    pub flags: u32,
    /// `sCleanupState.ui32SubmittedCommands`.
    pub cleanup_submitted: u32,
    /// `sCleanupState.ui32ExecutedCommands`.
    pub cleanup_executed: u32,
    /// The words of `sRTACtl`, raw.
    pub rta_ctl: [u32; RTA_CTL_WORDS],
}

impl HwrtPrState {
    pub fn has_last_ta(&self) -> bool {
        self.flags & HWRTDATA_HAS_LAST_TA != 0
    }
    pub fn partial_rendered(&self) -> bool {
        self.flags & HWRTDATA_PARTIAL_RENDERED != 0
    }
}

/// Decode the HWRTDATA image `b` (`HWRTDATA_SIZE` bytes). `None` if shorter.
pub fn hwrt_pr_state(b: &[u8]) -> Option<HwrtPrState> {
    if b.len() < HWRTDATA_SIZE {
        return None;
    }
    let mut rta_ctl = [0u32; RTA_CTL_WORDS];
    for (i, w) in rta_ctl.iter_mut().enumerate() {
        *w = u32_at(b, HWRTDATA_RTA_CTL + 4 * i)?;
    }
    Some(HwrtPrState {
        state: u32_at(b, HWRTDATA_STATE)?,
        flags: u32_at(b, HWRTDATA_FLAGS)?,
        cleanup_submitted: u32_at(b, HWRTDATA_CLEANUP_SUBMITTED)?,
        cleanup_executed: u32_at(b, HWRTDATA_CLEANUP_EXECUTED)?,
        rta_ctl,
    })
}

/// One line: `render: stall: HWRT state=<n> flags=0x<hex>[ HAS_LAST_TA][
/// PARTIAL_RENDERED] cleanup submitted=<n> executed=<n> rta_ctl=<hex words>`.
pub fn fmt_hwrt_pr_state(w: &mut dyn Write, s: &HwrtPrState) -> fmt::Result {
    write!(w, "render: stall: HWRT state={} flags=0x{:08x}", s.state, s.flags)?;
    if s.has_last_ta() {
        w.write_str(" HAS_LAST_TA")?;
    }
    if s.partial_rendered() {
        w.write_str(" PARTIAL_RENDERED")?;
    }
    write!(w, " cleanup submitted={} executed={} rta_ctl=", s.cleanup_submitted, s.cleanup_executed)?;
    for (i, v) in s.rta_ctl.iter().enumerate() {
        write!(w, "{}{:08x}", if i == 0 { "" } else { " " }, v)?;
    }
    Ok(())
}

/// Words per line of a command dump.
pub const CMD_WORDS_PER_LINE: usize = 8;

/// Lines a dump of `words` takes.
pub fn cmd_word_lines(words: &[u32]) -> usize {
    words.len().div_ceil(CMD_WORDS_PER_LINE)
}

/// Line `n` of a command image as indexed words, 8 per line:
/// `render: stall: <label> dk[<i>]: w w w w w w w w`, `i` the index of the
/// line's first word (two columns wide, as the existing `c3d[]` lines). The
/// last line holds the remainder. Nothing is written past the last line.
pub fn cmd_words(w: &mut dyn Write, label: &str, words: &[u32], n: usize) -> fmt::Result {
    cmd_line(w, label, " dk[", words, n)
}

/// The same line in the form tools/cts/pvr-cmd3d-decode.py parses: its
/// pattern wants `c3d[`, `cmd3d[` or `cgeom[` directly before the index, so
/// a label cannot sit between the kind and the bracket. The script searches
/// the line, so the `render: stall: ` prefix is fine.
pub fn cmd_words_decodable(w: &mut dyn Write, kind: &str, words: &[u32], n: usize) -> fmt::Result {
    cmd_line(w, kind, "[", words, n)
}

fn cmd_line(w: &mut dyn Write, label: &str, open: &str, words: &[u32], n: usize) -> fmt::Result {
    let first = n * CMD_WORDS_PER_LINE;
    let chunk = words.get(first..).unwrap_or(&[]);
    let chunk = &chunk[..chunk.len().min(CMD_WORDS_PER_LINE)];
    if chunk.is_empty() {
        return Ok(());
    }
    write!(w, "render: stall: {}{}{:2}]:", label, open, first)?;
    for v in chunk {
        write!(w, " {:08x}", v)?;
    }
    Ok(())
}

/// The range `(name, base, size)` holding `addr`: its name and the offset.
pub fn name_addr<'a>(addr: u64, ranges: &[(&'a str, u64, u64)]) -> Option<(&'a str, u64)> {
    ranges.iter().find_map(|&(n, base, size)| {
        let off = addr.checked_sub(base)?;
        (off < size).then_some((n, off))
    })
}

/// `<name>+0x<off>` or `unmapped`.
pub fn fmt_addr(w: &mut dyn Write, r: Option<(&str, u64)>) -> fmt::Result {
    match r {
        Some((n, off)) => write!(w, "{}+0x{:x}", n, off),
        None => w.write_str("unmapped"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(f: impl FnOnce(&mut String) -> fmt::Result) -> String {
        let mut s = String::new();
        f(&mut s).unwrap();
        s
    }

    fn put(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn file_info(path: &[u8], info: &[u8], line: u32) -> Vec<u8> {
        let mut b = vec![0u8; FILE_INFO_BUF_LINE_NUM + 4];
        b[FILE_INFO_BUF_PATH..FILE_INFO_BUF_PATH + path.len()].copy_from_slice(path);
        b[FILE_INFO_BUF_INFO..FILE_INFO_BUF_INFO + info.len()].copy_from_slice(info);
        put(&mut b, FILE_INFO_BUF_LINE_NUM, line);
        b
    }

    #[test]
    fn assert_present() {
        let b = file_info(b"rgxta3d.c", b"bad \xffstate", 1234);
        let a = assert_buf(&b).unwrap();
        assert_eq!(a, FwAssert { path: b"rgxta3d.c", info: b"bad \xffstate", line: 1234 });
        assert_eq!(out(|w| fmt_assert(w, &a)), "render: stall: FW assert rgxta3d.c:1234 bad ?state");
    }

    #[test]
    fn assert_absent_or_short() {
        assert_eq!(assert_buf(&file_info(b"", b"stale", 7)), None);
        assert_eq!(assert_buf(&[0u8; 8]), None);
    }

    fn hwrt(flags: u32) -> Vec<u8> {
        let mut b = vec![0u8; HWRTDATA_SIZE];
        put(&mut b, HWRTDATA_STATE, 3);
        put(&mut b, HWRTDATA_FLAGS, flags);
        put(&mut b, HWRTDATA_CLEANUP_SUBMITTED, 5);
        put(&mut b, HWRTDATA_CLEANUP_EXECUTED, 4);
        for i in 0..HWRTDATA_RTA_CTL_SIZE / 4 {
            put(&mut b, HWRTDATA_RTA_CTL + 4 * i, 0x100 + i as u32);
        }
        b
    }

    fn state(flags: u32) -> HwrtPrState {
        hwrt_pr_state(&hwrt(flags)).unwrap()
    }

    #[test]
    fn hwrt_pr_flag_set() {
        let s = state(HWRTDATA_HAS_LAST_TA | HWRTDATA_PARTIAL_RENDERED);
        assert!(s.has_last_ta() && s.partial_rendered());
        assert_eq!((s.state, s.cleanup_submitted, s.cleanup_executed), (3, 5, 4));
        assert_eq!(s.rta_ctl.len(), HWRTDATA_RTA_CTL_SIZE / 4);
        let line = out(|w| fmt_hwrt_pr_state(w, &s));
        assert!(line.starts_with(
            "render: stall: HWRT state=3 flags=0x0000000c HAS_LAST_TA PARTIAL_RENDERED cleanup submitted=5 executed=4 rta_ctl=00000100 00000101 "
        ));
        assert!(line.ends_with("00000107"));
    }

    #[test]
    fn hwrt_pr_flag_clear() {
        let s = state(0);
        assert!(!s.has_last_ta() && !s.partial_rendered());
        assert!(out(|w| fmt_hwrt_pr_state(w, &s)).contains("flags=0x00000000 cleanup"));
        let only_ta = state(HWRTDATA_HAS_LAST_TA);
        assert!(only_ta.has_last_ta() && !only_ta.partial_rendered());
    }

    #[test]
    fn hwrt_short_is_none() {
        assert_eq!(hwrt_pr_state(&hwrt(0)[..HWRTDATA_SIZE - 1]), None);
    }

    fn lines(f: fn(&mut String, &str, &[u32], usize) -> fmt::Result, label: &str, w: &[u32]) -> Vec<String> {
        (0..cmd_word_lines(w)).map(|n| out(|s| f(s, label, w, n))).collect()
    }

    #[test]
    fn cmd_words_lines_and_indices() {
        let w: Vec<u32> = (0..19).collect();
        let l = lines(|s, a, b, n| cmd_words(s, a, b, n), "c3d", &w);
        assert_eq!(l.len(), 3);
        assert_eq!(
            l[0],
            "render: stall: c3d dk[ 0]: 00000000 00000001 00000002 00000003 00000004 00000005 00000006 00000007"
        );
        assert!(l[1].starts_with("render: stall: c3d dk[ 8]: 00000008 "));
        assert_eq!(l[2], "render: stall: c3d dk[16]: 00000010 00000011 00000012");
        assert_eq!(cmd_word_lines(&[]), 0);
        // Past the last line nothing is written.
        assert_eq!(out(|s| cmd_words(s, "c3d", &w, 3)), "");
        assert_eq!(lines(|s, a, b, n| cmd_words(s, a, b, n), "c3d", &w[..16]).len(), 2);
    }

    #[test]
    fn cmd_words_decodable_form() {
        let w: Vec<u32> = (0..16).collect();
        let l = lines(|s, a, b, n| cmd_words_decodable(s, a, b, n), "c3d", &w);
        assert_eq!(
            l[1],
            "render: stall: c3d[ 8]: 00000008 00000009 0000000a 0000000b 0000000c 0000000d 0000000e 0000000f"
        );
    }

    const R: [(&str, u64, u64); 2] = [("fw_heap", 0x1000, 0x100), ("hwrt", 0x2000, 0x40)];

    #[test]
    fn addr_at_start_end_and_outside() {
        assert_eq!(name_addr(0x1000, &R), Some(("fw_heap", 0)));
        assert_eq!(name_addr(0x10ff, &R), Some(("fw_heap", 0xff)));
        assert_eq!(name_addr(0x1100, &R), None);
        assert_eq!(name_addr(0x2000 + 0x3f, &R), Some(("hwrt", 0x3f)));
        assert_eq!(name_addr(0xfff, &R), None);
        assert_eq!(name_addr(0, &R), None);
        assert_eq!(out(|w| fmt_addr(w, name_addr(0x2010, &R))), "hwrt+0x10");
        assert_eq!(out(|w| fmt_addr(w, name_addr(0xfff, &R))), "unmapped");
    }
}
