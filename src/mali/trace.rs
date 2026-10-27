// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Readers for the board-trace excerpts under tools/mali/board-dump/: the
//! raw memory a dump printed and the command-stream words a trace printed.
//!
//! An excerpt is lines of a pandecode file, each block headed by
//! `## lines a-b` or, in one that holds two runs, `## <source> lines a-b`.
//! A memory block is `Buffer: memory_<va> gpu <va>` followed by hexdump rows
//! (offset, 16 bytes; a `*` row repeats the row before it up to the next row
//! printed). A command-stream word is an indented line of eight hex bytes,
//! least significant first, then pandecode's mnemonic.

use std::collections::BTreeMap;

/// The source named by a block header (`None` for `## lines a-b`) and the
/// first line number the block holds.
fn header(line: &str) -> Option<(Option<&str>, usize)> {
    let rest = line.strip_prefix("## ")?;
    let (name, range) = match rest.strip_prefix("lines ") {
        Some(range) => (None, range),
        None => {
            let (name, range) = rest.split_once(" lines ")?;
            (Some(name), range)
        }
    };
    let first = range.split('-').next()?.parse().ok()?;
    Some((name, first))
}

/// The lines of the blocks from `source` (`None`: every block), with their
/// line numbers in the source file.
fn lines_of<'a>(excerpt: &'a str, source: Option<&str>) -> Vec<(usize, &'a str)> {
    let mut out = Vec::new();
    let mut at: Option<usize> = None;
    for line in excerpt.lines() {
        if let Some((name, first)) = header(line) {
            at = if source.is_none() || name == source { Some(first) } else { None };
            continue;
        }
        if let Some(n) = at.as_mut() {
            out.push((*n, line));
            *n += 1;
        }
    }
    out
}

fn rows(lines: &[(usize, &str)]) -> BTreeMap<u64, [u8; 16]> {
    let mut rows = BTreeMap::new();
    let mut base: Option<u64> = None;
    let mut last: Option<(u64, [u8; 16])> = None;
    let mut repeat = false;
    for &(_, line) in lines {
        if let Some(rest) = line.strip_prefix("Buffer: memory_") {
            let va = rest.split_whitespace().last().expect("a GPU VA");
            base = Some(u64::from_str_radix(va, 16).expect("a hex VA"));
            last = None;
            repeat = false;
            continue;
        }
        let Some(b) = base else { continue };
        if line.trim() == "*" {
            repeat = true;
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(off) = it.next().and_then(|o| u64::from_str_radix(o, 16).ok()) else { continue };
        let bytes: Vec<u8> = it.map(|h| u8::from_str_radix(h, 16).expect("a hex byte")).collect();
        let Ok(row) = <[u8; 16]>::try_from(bytes) else { continue };
        if repeat {
            let (from, r) = last.expect("a * row follows a row");
            let mut at = from + 16;
            while at < b + off {
                rows.insert(at, r);
                at += 16;
            }
            repeat = false;
        }
        rows.insert(b + off, row);
        last = Some((b + off, row));
    }
    rows
}

/// Every block's hexdumps: GPU VA of each 16-byte row -> its bytes.
pub fn memory(excerpt: &str) -> BTreeMap<u64, [u8; 16]> {
    rows(&lines_of(excerpt, None))
}

/// The hexdumps of the blocks from one source.
pub fn memory_of(excerpt: &str, source: &str) -> BTreeMap<u64, [u8; 16]> {
    rows(&lines_of(excerpt, Some(source)))
}

/// `len` bytes at `va`; a byte the excerpt does not hold is a panic.
pub fn bytes(mem: &BTreeMap<u64, [u8; 16]>, va: u64, len: usize) -> Vec<u8> {
    (0..len as u64)
        .map(|i| {
            let a = va + i;
            mem.get(&(a & !15)).unwrap_or_else(|| panic!("{a:#x} is not in the excerpt"))[(a & 15) as usize]
        })
        .collect()
}

/// One printed command-stream word.
pub struct CsWord {
    /// Its line in the source file.
    pub line: usize,
    pub word: u64,
    /// pandecode's text after the bytes.
    pub mnemonic: String,
}

/// The printed command-stream words of one source, in order.
pub fn cs_words(excerpt: &str, source: &str) -> Vec<CsWord> {
    let mut out = Vec::new();
    for (line, text) in lines_of(excerpt, Some(source)) {
        /* indented: the shader disassembly lines pandecode also prints as
         * eight bytes and a mnemonic start in column 0 */
        if !text.starts_with("  ") {
            continue;
        }
        let mut it = text.split_whitespace();
        let mut b = [0u8; 8];
        let mut ok = true;
        for slot in b.iter_mut() {
            match it.next().filter(|h| h.len() == 2).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => *slot = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let mnemonic: Vec<&str> = it.collect();
        if mnemonic.is_empty() {
            continue;
        }
        out.push(CsWord { line, word: u64::from_le_bytes(b), mnemonic: mnemonic.join(" ") });
    }
    out
}

#[test]
fn the_readers_find_a_known_row_and_word() {
    const FRAME: &str = include_str!("../../tools/mali/board-dump/tri-frame-2026-10-18.txt");
    let mem = memory_of(FRAME, "linear");
    /* the batch CS's first word, REQ_RESOURCE, and the FS binary's first */
    assert_eq!(bytes(&mem, 0x7fff_fffc_c000, 8), [0x0f, 0, 0, 0, 0, 0, 0, 0x22]);
    assert_eq!(bytes(&mem, 0x7fff_ffff_c080, 4), [0xc0, 0x00, 0x00, 0x00]);
    /* inside a * run of the descriptor buffer */
    assert_eq!(bytes(&mem, 0x7fff_fffd_4c00, 16), [0u8; 16]);
    let words = cs_words(FRAME, "linear");
    assert_eq!(words[0].line, 1);
    assert_eq!(words[0].mnemonic, "MOVE d72, #0x800000015000");
    let last = words.last().expect("words");
    assert_eq!((last.line, last.mnemonic.as_str()), (791, "WAIT #1"));
    /* the afbc source is its own: its first MOVE d76 names 0x...3260 */
    let afbc = cs_words(FRAME, "afbc");
    assert!(afbc.iter().any(|w| w.mnemonic == "MOVE d76, #0x7FFFFFFD3260"));
    assert!(!words.iter().any(|w| w.mnemonic == "MOVE d76, #0x7FFFFFFD3260"));
}
