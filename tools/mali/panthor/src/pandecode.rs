// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Reads a pandecode trace with memory dumps (Mesa `PAN_MESA_DEBUG=trace,sync,dump`)
//! into what a replay needs: the command stream words pandecode printed, the
//! buffer dumps taken after each submit, and the render targets the decoded
//! framebuffers name.
//!
//! The format, as Mesa 26.2.1 writes it (the board ran 25.0.7; the parser
//! refuses what it does not recognise rather than guessing):
//! - a CS instruction pandecode interprets is a line of two spaces, eight
//!   bytes in hex (least significant first), two spaces per call depth plus
//!   two, and the mnemonic (`pandecode_interpret_cs`,
//!   src/panfrost/genxml/decode_csf.c:1787-1800; a kernel-mode queue starts
//!   at depth 1, so the submitted stream has four spaces and what it calls,
//!   such as the tiler OOM handler, six);
//! - `pandecode_dump_mappings` (src/panfrost/genxml/decode_common.c:331-349)
//!   prints `Buffer: <name> gpu <hex va>`, a blank line and `u_hexdump`'s
//!   rows (src/util/u_hexdump.h:13-48): `%06X  ` and 16 `%02X ` bytes, a `*`
//!   line for rows equal to the one before, the first row always printed.
//!   One call prints every mapping; consecutive sections are one dump.

use std::collections::BTreeMap;

/// One CS word with the line it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsLine {
    pub line: usize,
    pub depth: usize,
    pub word: u64,
    pub text: String,
}

/// A segment of the trace: the top-level stream words printed before the
/// dump that follows them (the last segment may have no dump).
#[derive(Debug, Default)]
pub struct Segment {
    pub cs: Vec<CsLine>,
    /// VA -> bytes of every buffer in the dump printed after this segment.
    pub dump: BTreeMap<u64, Vec<u8>>,
    /// The line of the first `Buffer:` of that dump (0: none).
    pub dump_line: usize,
}

/// A render target a decoded framebuffer names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderTarget {
    pub line: usize,
    pub afbc: bool,
    /// AFBC: the header address; linear: the RGB base.
    pub base: u64,
    pub row_stride: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Trace {
    pub segments: Vec<Segment>,
    pub render_targets: Vec<RenderTarget>,
    /// `Shader ... (GPU VA <va>) sz <n>` lines: (va, size).
    pub shaders: Vec<(u64, u64)>,
    /// (width, height) of each decoded `Framebuffer @` block, in order.
    pub framebuffers: Vec<(u32, u32)>,
}

/// `(call depth, word, mnemonic)` of a CS line; depth 1 is the submitted
/// stream.
fn cs_line(line: &str) -> Option<(usize, u64, String)> {
    let rest = line.strip_prefix("  ")?;
    // "xx xx xx xx xx xx xx xx" then the depth's spaces and the mnemonic
    if rest.len() < 23 + 4 || !rest.is_char_boundary(23) {
        return None;
    }
    let (hex, tail) = rest.split_at(23);
    let gap = tail.len() - tail.trim_start_matches(' ').len();
    if gap < 4 || !gap.is_multiple_of(2) || tail.len() == gap {
        return None;
    }
    let mut word = 0u64;
    for (i, b) in hex.split(' ').enumerate() {
        if b.len() != 2 || i >= 8 {
            return None;
        }
        word |= (u8::from_str_radix(b, 16).ok()? as u64) << (8 * i);
    }
    Some((gap / 2 - 1, word, tail.trim().to_string()))
}

fn hex_after(line: &str, key: &str) -> Option<u64> {
    let i = line.find(key)? + key.len();
    let s = line[i..].trim_start().trim_start_matches("0x");
    let end = s.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(s.len());
    u64::from_str_radix(&s[..end], 16).ok()
}

fn dec_after(line: &str, key: &str) -> Option<u64> {
    let i = line.find(key)? + key.len();
    let s = line[i..].trim_start();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse().ok()
}

/// Expand `u_hexdump` rows into bytes.
fn expand(rows: &[(usize, Option<[u8; 16]>)], at: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut pending = false;
    let mut last: Option<[u8; 16]> = None;
    for &(off, row) in rows {
        match row {
            None => pending = true,
            Some(bytes) => {
                if pending {
                    let l = last.ok_or_else(|| format!("line {at}: '*' before any row"))?;
                    while out.len() < off {
                        out.extend_from_slice(&l);
                    }
                    pending = false;
                }
                if out.len() != off {
                    return Err(format!("line {at}: row at {off:#x} after {:#x} bytes", out.len()));
                }
                out.extend_from_slice(&bytes);
                last = Some(bytes);
            }
        }
    }
    if pending {
        return Err(format!("line {at}: the dump ends in '*' (no closing row)"));
    }
    Ok(out)
}

fn hexdump_row(line: &str) -> Option<(usize, [u8; 16])> {
    let (off, rest) = line.split_once("  ")?;
    if off.len() != 6 {
        return None;
    }
    let off = usize::from_str_radix(off, 16).ok()?;
    let mut bytes = [0u8; 16];
    let mut n = 0;
    for b in rest.split_whitespace() {
        if n == 16 || b.len() != 2 {
            return None;
        }
        bytes[n] = u8::from_str_radix(b, 16).ok()?;
        n += 1;
    }
    (n == 16).then_some((off, bytes))
}

/// The lines nested under line `i` (more indented than it).
fn block<'a>(lines: &[&'a str], i: usize) -> Vec<&'a str> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let k = indent(lines[i]);
    lines[i + 1..].iter().take_while(|l| !l.trim().is_empty() && indent(l) > k).copied().collect()
}

/// A `Color Render Target 0:` block: the block format, the AFBC header, and
/// the `RGB:` view's base and row stride.
fn render_target(b: &[&str], line: usize) -> Result<RenderTarget, String> {
    let field = |key: &str| b.iter().find_map(|l| l.trim_start().strip_prefix(key).map(str::trim));
    let fmt = field("Writeback Block Format: ").ok_or_else(|| format!("line {line}: no block format"))?;
    let afbc = match fmt {
        "AFBC" => true,
        "Linear" => false,
        other => return Err(format!("line {line}: block format {other:?} is not handled")),
    };
    let rgb = b.iter().position(|l| l.trim() == "RGB:");
    let in_rgb =
        |key: &str| rgb.and_then(|r| b[r + 1..].iter().find_map(|l| l.trim_start().strip_prefix(key).map(str::trim)));
    let hex = |v: &str| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok();
    let base = if afbc { field("Header: ").and_then(hex) } else { in_rgb("Base: ").and_then(hex) };
    let base =
        base.ok_or_else(|| format!("line {line}: no {} address", if afbc { "AFBC header" } else { "RGB base" }))?;
    let row_stride = in_rgb("Row Stride: ").and_then(|v| v.parse().ok());
    Ok(RenderTarget { line, afbc, base, row_stride })
}

impl Trace {
    pub fn parse(text: &str) -> Result<Trace, String> {
        let mut t = Trace::default();
        let mut seg = Segment::default();
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];
            let n = i + 1;
            if let Some(rest) = line.strip_prefix("Buffer: ") {
                let va = hex_after(rest, " gpu ").ok_or_else(|| format!("line {n}: no 'gpu <va>'"))?;
                if seg.dump.is_empty() {
                    seg.dump_line = n;
                }
                // The rows run to the next blank line.
                let mut rows = Vec::new();
                let mut j = i + 1;
                while j < lines.len() && lines[j].is_empty() {
                    j += 1;
                }
                while j < lines.len() && !lines[j].is_empty() {
                    let l = lines[j];
                    if l == "*" {
                        rows.push((j + 1, None));
                    } else {
                        let (off, bytes) =
                            hexdump_row(l).ok_or_else(|| format!("line {}: not a hexdump row: {l:?}", j + 1))?;
                        rows.push((off, Some(bytes)));
                    }
                    j += 1;
                }
                let bytes = expand(&rows, n)?;
                if bytes.is_empty() {
                    return Err(format!("line {n}: buffer {va:#x} has no rows"));
                }
                if seg.dump.insert(va, bytes).is_some() {
                    return Err(format!("line {n}: buffer {va:#x} twice in one dump"));
                }
                i = j;
                continue;
            }
            if !seg.dump.is_empty() && !line.trim().is_empty() {
                t.segments.push(std::mem::take(&mut seg));
            }
            if let Some((depth, word, text)) = cs_line(line) {
                if depth == 1 {
                    seg.cs.push(CsLine { line: n, depth, word, text });
                }
            } else if line.contains("Shader 0x") && line.contains("(GPU VA ") {
                let va = hex_after(line, "(GPU VA ").ok_or_else(|| format!("line {n}: shader VA"))?;
                let sz = dec_after(line, ") sz ").ok_or_else(|| format!("line {n}: shader size"))?;
                t.shaders.push((va, sz));
            } else if line.trim_start().starts_with("Framebuffer @") {
                let b = block(&lines, i);
                let w =
                    b.iter().find_map(|l| l.trim_start().strip_prefix("Width: ").and_then(|v| v.trim().parse().ok()));
                let h =
                    b.iter().find_map(|l| l.trim_start().strip_prefix("Height: ").and_then(|v| v.trim().parse().ok()));
                match (w, h) {
                    (Some(w), Some(h)) => t.framebuffers.push((w, h)),
                    _ => return Err(format!("line {n}: a framebuffer without Width and Height")),
                }
            } else if line.trim_start().starts_with("Color Render Target 0:") {
                t.render_targets.push(render_target(&block(&lines, i), n)?);
            }
            i += 1;
        }
        if !seg.cs.is_empty() || !seg.dump.is_empty() {
            t.segments.push(seg);
        }
        Ok(t)
    }
}

/// The 4-byte pixels of an RGBA8 image in a buffer, counted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PixelCount {
    pub red: usize,
    pub blue: usize,
    pub other: usize,
}

/// Count opaque red (ff 00 00 ff) and opaque blue (00 00 ff ff) pixels of a
/// `w` x `h` RGBA8 image with `stride` bytes a row; every other value is
/// "other". Refuses an image larger than the bytes.
pub fn count_rgba8(bytes: &[u8], w: usize, h: usize, stride: usize) -> Result<PixelCount, String> {
    if stride < w * 4 || (h > 0 && (h - 1) * stride + w * 4 > bytes.len()) {
        return Err(format!("{w}x{h} RGBA8 at stride {stride} does not fit {} bytes", bytes.len()));
    }
    let mut c = PixelCount::default();
    for y in 0..h {
        for x in 0..w {
            let o = y * stride + x * 4;
            match &bytes[o..o + 4] {
                [0xff, 0x00, 0x00, 0xff] => c.red += 1,
                [0x00, 0x00, 0xff, 0xff] => c.blue += 1,
                _ => c.other += 1,
            }
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"  00 50 01 00 00 80 48 01    MOVE d72, #0x800000015000
  00 00 00 00 00 48 00 30    HEAP_SET d72
  44 00 03 00 00 4c 5e 14      LOAD_MULTIPLE r94:r95, [d76 + 68]
  0f 00 00 00 00 00 00 22    REQ_RESOURCE.compute.fragment.tiler.idvs
Shader 0xffffb8150000 (GPU VA 7fffffffc000) sz 4096
  Framebuffer @7fffffeb6000:
    Parameters:
      Width: 64
      Height: 64
  Color Render Targets @7fffffeb60c0:
    Color Render Target 0:
      AFBC:
        Header: 0x7fffffec6000
      Writeback Block Format: Linear
      RGB:
        Base: 0x7fffffec6000
        Row Stride: 256
        Surface Stride: 0
  00 00 01 00 00 00 00 03    WAIT #1


Buffer: memory_7ffffffff000 gpu 7ffffffff000

000000  80 00 80 00 00 00 00 00 00 00 00 00 00 00 00 00
000010  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
*
000040  01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10


Buffer: memory_7fffffffe000 gpu 7fffffffe000

000000  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
*
000FF0  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00


  0f 00 00 00 00 00 00 22    REQ_RESOURCE.compute.fragment.tiler.idvs
"#;

    #[test]
    fn parses_streams_dumps_and_targets() {
        let t = Trace::parse(SAMPLE).unwrap();
        assert_eq!(t.segments.len(), 2);
        let s0 = &t.segments[0];
        let words: Vec<u64> = s0.cs.iter().map(|c| c.word).collect();
        // The nested LOAD_MULTIPLE (the OOM handler pandecode follows) is
        // not part of the submitted stream.
        assert_eq!(
            words,
            vec![0x0148_8000_0001_5000, 0x3000_4800_0000_0000, 0x2200_0000_0000_000f, 0x0300_0000_0001_0000]
        );
        assert_eq!(s0.dump.len(), 2);
        let b = &s0.dump[&0x7fff_ffff_f000];
        assert_eq!(b.len(), 0x50);
        assert_eq!(&b[0x20..0x30], &[0u8; 16], "a '*' repeats the row before it");
        assert_eq!(b[0x40], 1);
        assert_eq!(s0.dump[&0x7fff_ffff_e000].len(), 0x1000);
        assert_eq!(t.segments[1].cs.len(), 1);
        assert!(t.segments[1].dump.is_empty());
        assert_eq!(t.shaders, vec![(0x7fff_ffff_c000, 4096)]);
        assert_eq!(t.framebuffers, vec![(64, 64)]);
        assert_eq!(
            t.render_targets,
            vec![RenderTarget { line: 11, afbc: false, base: 0x7fff_ffec_6000, row_stride: Some(256) }]
        );
    }

    #[test]
    fn refuses_a_star_without_a_closing_row() {
        let bad = "Buffer: m gpu 1000\n\n000000  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 \n*\n\n";
        assert!(Trace::parse(bad).unwrap_err().contains("ends in '*'"));
    }

    #[test]
    fn counts_pixels_and_refuses_a_short_buffer() {
        let mut img = vec![0u8; 2 * 8];
        img[..4].copy_from_slice(&[0xff, 0, 0, 0xff]);
        img[4..8].copy_from_slice(&[0, 0, 0xff, 0xff]);
        img[8..12].copy_from_slice(&[0, 0, 0xff, 0xfe]);
        assert_eq!(count_rgba8(&img, 2, 2, 8).unwrap(), PixelCount { red: 1, blue: 1, other: 2 });
        assert!(count_rgba8(&img, 2, 3, 8).is_err());
    }
}
