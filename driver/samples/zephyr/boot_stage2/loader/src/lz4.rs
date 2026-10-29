// SPDX-License-Identifier: Apache-2.0
//! An LZ4 frame decoder (the frame and block formats of lz4's
//! doc/lz4_Frame_format.md and doc/lz4_Block_format.md), written from those
//! documents for this loader: the payload `lz4 -12` writes. Like the inflate,
//! the whole output is one flat buffer (the image at its load address), so a
//! match may reach back into an earlier block (linked blocks) without a
//! window, and nothing is allocated.
//!
//! Why LZ4 next to deflate: on the i.MX8MP's A53 the inflate of the route-3
//! image (2.2 MB to 7.6 MB) took 207 ms of the loader's 300 (2026-10-29,
//! eMMC boot, system counter); an LZ4 block is byte-aligned tokens and
//! copies, with no Huffman stage, for about 0.3 MB more stored.
//!
//! The frame's own checksums (header byte, block and content xxHash32) are
//! not verified: the header's CRC-32 of the stored bytes covers them, and the
//! image's CRC-32 covers the output. Every malformed input is an error,
//! never a panic or a write outside `dst`.

/// Why decoding stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lz4Error {
    /// No LZ4 frame magic, a frame version other than 01, reserved bits set,
    /// or a dictionary id (this loader has no dictionary).
    BadFrame,
    /// The input ended inside the frame.
    Truncated,
    /// The output would not fit `dst`.
    OutputFull,
    /// A match offset of 0 or before the start of the output, or a block
    /// that ends inside a sequence.
    BadSequence,
    /// The frame's content size is not the bytes decoded.
    ContentSize,
}

const MAGIC: u32 = 0x184D_2204;
const MIN_MATCH: usize = 4;

fn le32(b: &[u8], at: usize) -> Result<u32, Lz4Error> {
    let s = b.get(at..at + 4).ok_or(Lz4Error::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Decode one LZ4 frame from `src` into `dst`; the bytes written.
/// Bytes after the frame's end mark (and content checksum) are refused.
pub fn decode_frame(src: &[u8], dst: &mut [u8]) -> Result<usize, Lz4Error> {
    if le32(src, 0)? != MAGIC {
        return Err(Lz4Error::BadFrame);
    }
    let flg = *src.get(4).ok_or(Lz4Error::Truncated)?;
    let bd = *src.get(5).ok_or(Lz4Error::Truncated)?;
    // FLG: version (2 bits) 01, block independence, block checksum, content
    // size, content checksum, reserved 0, dictionary id.
    if flg >> 6 != 0b01 || flg & 0x02 != 0 || flg & 0x01 != 0 || bd & 0x8F != 0 {
        return Err(Lz4Error::BadFrame);
    }
    let block_checksum = flg & 0x10 != 0;
    let content_checksum = flg & 0x04 != 0;
    let mut i = 6;
    let content_size = if flg & 0x08 != 0 {
        let s = src.get(i..i + 8).ok_or(Lz4Error::Truncated)?;
        i += 8;
        Some(u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
    } else {
        None
    };
    i += 1; // the header checksum byte
    let mut o = 0usize;
    loop {
        let word = le32(src, i)?;
        i += 4;
        if word == 0 {
            break; // EndMark
        }
        let len = (word & 0x7FFF_FFFF) as usize;
        let block = src.get(i..i + len).ok_or(Lz4Error::Truncated)?;
        if word & 0x8000_0000 != 0 {
            // stored uncompressed
            dst.get_mut(o..o + len).ok_or(Lz4Error::OutputFull)?.copy_from_slice(block);
            o += len;
        } else {
            o = decode_block(block, dst, o)?;
        }
        i += len + if block_checksum { 4 } else { 0 };
    }
    if content_checksum {
        i += 4;
    }
    if i != src.len() {
        return Err(if i > src.len() { Lz4Error::Truncated } else { Lz4Error::BadFrame });
    }
    if content_size.is_some_and(|n| n != o as u64) {
        return Err(Lz4Error::ContentSize);
    }
    Ok(o)
}

/// A length's continuation bytes: each adds up to 255, the last is below 255.
fn more_len(src: &[u8], i: &mut usize, mut n: usize) -> Result<usize, Lz4Error> {
    loop {
        let b = *src.get(*i).ok_or(Lz4Error::BadSequence)?;
        *i += 1;
        n += b as usize;
        if b != 255 {
            return Ok(n);
        }
    }
}

/// Decode one compressed block into `dst` from `o` on; the new end.
fn decode_block(src: &[u8], dst: &mut [u8], mut o: usize) -> Result<usize, Lz4Error> {
    let mut i = 0;
    loop {
        let token = *src.get(i).ok_or(Lz4Error::BadSequence)?;
        i += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            lit = more_len(src, &mut i, lit)?;
        }
        let from = src.get(i..i + lit).ok_or(Lz4Error::BadSequence)?;
        dst.get_mut(o..o + lit).ok_or(Lz4Error::OutputFull)?.copy_from_slice(from);
        i += lit;
        o += lit;
        if i == src.len() {
            return Ok(o); // the last sequence is literals only
        }
        let off = src.get(i..i + 2).ok_or(Lz4Error::BadSequence)?;
        let off = u16::from_le_bytes([off[0], off[1]]) as usize;
        i += 2;
        let mut len = (token & 0x0F) as usize;
        if len == 15 {
            len = more_len(src, &mut i, len)?;
        }
        len += MIN_MATCH;
        if off == 0 || off > o {
            return Err(Lz4Error::BadSequence);
        }
        if o + len > dst.len() {
            return Err(Lz4Error::OutputFull);
        }
        let start = o - off;
        if off >= len {
            dst.copy_within(start..start + len, o);
        } else {
            // An overlapping match repeats the last `off` bytes: copy in
            // chunks of the period, each a non-overlapping copy of the bytes
            // already written.
            let mut done = 0;
            while done < len {
                let n = (len - done).min(off + done);
                dst.copy_within(start..start + n, o + done);
                done += n;
            }
        }
        o += len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// A frame around `blocks` (FLG 0x60: version 01, independent blocks,
    /// no checksums, no content size; BD 0x70: 4 MB blocks).
    fn frame(blocks: &[(bool, &[u8])]) -> Vec<u8> {
        let mut f = MAGIC.to_le_bytes().to_vec();
        f.extend_from_slice(&[0x60, 0x70, 0x00]);
        for (raw, b) in blocks {
            let w = b.len() as u32 | if *raw { 0x8000_0000 } else { 0 };
            f.extend_from_slice(&w.to_le_bytes());
            f.extend_from_slice(b);
        }
        f.extend_from_slice(&0u32.to_le_bytes());
        f
    }

    #[test]
    fn literals_and_matches() {
        // "abcd" + match(off 4, len 8) + literals "xyz":
        // token 0x44 lit 4, match 4+4 = 8; then token 0x30, 3 literals.
        let block = [0x44, b'a', b'b', b'c', b'd', 4, 0, 0x30, b'x', b'y', b'z'];
        let mut out = [0u8; 32];
        let n = decode_frame(&frame(&[(false, &block)]), &mut out).unwrap();
        assert_eq!(&out[..n], b"abcdabcdabcdxyz");
    }

    #[test]
    fn overlapping_run_and_long_lengths() {
        // "a" + match(off 1, len 15+4+300) + literal "b": a run of 320 'a'.
        let block = [0x1F, b'a', 1, 0, 255, 45, 0x10, b'b'];
        let mut out = [0u8; 400];
        let n = decode_frame(&frame(&[(false, &block)]), &mut out).unwrap();
        assert_eq!(n, 321);
        assert!(out[..320].iter().all(|&c| c == b'a'));
        assert_eq!(out[320], b'b');
    }

    #[test]
    fn linked_blocks_and_stored_blocks() {
        // block 3 matches back into block 1, across the stored block 2.
        let b1 = [0x60, b'h', b'e', b'l', b'l', b'o', b' '];
        let b2 = [0x01, 9, 0, 0x10, b'!'];
        let mut out = [0u8; 64];
        let n = decode_frame(&frame(&[(false, &b1), (true, b"raw"), (false, &b2)]), &mut out).unwrap();
        assert_eq!(&out[..n], b"hello rawhello!");
    }

    #[test]
    fn refusals() {
        let mut out = [0u8; 16];
        let ok = [0x44, b'a', b'b', b'c', b'd', 4, 0, 0x30, b'x', b'y', b'z'];
        assert_eq!(decode_frame(&frame(&[(false, &ok)]), &mut out[..14]), Err(Lz4Error::OutputFull));
        let far = [0x40, b'a', b'b', b'c', b'd', 9, 0, 0x00];
        assert_eq!(decode_frame(&frame(&[(false, &far)]), &mut out), Err(Lz4Error::BadSequence));
        let zero = [0x40, b'a', b'b', b'c', b'd', 0, 0, 0x00];
        assert_eq!(decode_frame(&frame(&[(false, &zero)]), &mut out), Err(Lz4Error::BadSequence));
        let cut = [0x44, b'a', b'b', b'c', b'd', 4];
        assert_eq!(decode_frame(&frame(&[(false, &cut)]), &mut out), Err(Lz4Error::BadSequence));
        let mut f = frame(&[(false, &ok)]);
        f[4] = 0xA0; // version 10
        assert_eq!(decode_frame(&f, &mut out), Err(Lz4Error::BadFrame));
        let mut f = frame(&[(false, &ok)]);
        f.push(0); // bytes after the end mark
        assert_eq!(decode_frame(&f, &mut out), Err(Lz4Error::BadFrame));
        let f = frame(&[(false, &ok)]);
        assert_eq!(decode_frame(&f[..f.len() - 2], &mut out), Err(Lz4Error::Truncated));
        assert_eq!(decode_frame(b"\x04\x22\x4d\x19", &mut out), Err(Lz4Error::BadFrame));
    }
}
