// SPDX-License-Identifier: Apache-2.0
//! A raw-deflate decoder (RFC 1951), written from the RFC for this loader:
//! the whole output is one flat buffer (the image at its load address), so
//! there is no sliding window, no allocation and no state between calls.
//!
//! Huffman codes are canonical (RFC 1951 3.2.2: codes of one length are
//! consecutive integers). A code of up to FAST_BITS is one table look-up on
//! the next bits of the stream; a longer one is decoded bit by bit against
//! the per-length counts. The bit-by-bit form alone decoded a 1.8 MB image in
//! about half a second on an x86 host (2026-10-21), which on the A53 at boot
//! would have been the largest part of the time to the first frame.
//!
//! Every malformed input is an error, never a panic or a write outside `dst`:
//! the stream comes from the eMMC and is only CRC-checked after decoding.

/// Why decoding stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflateError {
    /// The input ended inside a block.
    Truncated,
    /// The output would not fit `dst`.
    OutputFull,
    /// Block type 3.
    BadBlockType,
    /// A stored block's LEN and NLEN disagree.
    StoredLength,
    /// A code-length set that is over-subscribed, or incomplete where the RFC
    /// does not allow it.
    BadCode,
    /// A code no table holds, a length/distance symbol out of range, or a
    /// repeat with nothing to repeat.
    BadSymbol,
    /// A distance before the start of the output.
    DistanceTooFar,
}

const MAX_BITS: usize = 15;
const MAX_LCODES: usize = 286;
const MAX_DCODES: usize = 30;
const FIX_LCODES: usize = 288;
/// Codes up to this long are decoded by one table look-up; longer ones (rare:
/// zlib gives them to symbols it hardly uses) take the canonical bit-by-bit
/// path. 10 bits = a 2 KiB table per code, built once per block.
const FAST_BITS: u32 = 10;

struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    buf: u64,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Self {
        Bits { src, pos: 0, buf: 0, cnt: 0 }
    }

    /// Pull whole bytes into the buffer while there is room (it holds 64
    /// bits; after this at least 56 are valid unless the input has ended).
    #[inline]
    fn refill(&mut self) {
        while self.cnt <= 56 {
            let Some(&b) = self.src.get(self.pos) else { return };
            self.pos += 1;
            self.buf |= (b as u64) << self.cnt;
            self.cnt += 8;
        }
    }

    /// `n` (<= 32) bits, least significant first (RFC 1951 3.1.1).
    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        if self.cnt < n {
            self.refill();
            if self.cnt < n {
                return Err(InflateError::Truncated);
            }
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.cnt -= n;
        Ok(v)
    }

    /// The next `n` bits without consuming them; bits past the end of the
    /// input read as zero (consuming them is the error, in `drop_bits`).
    #[inline]
    fn peek(&mut self, n: u32) -> u32 {
        if self.cnt < n {
            self.refill();
        }
        (self.buf & ((1u64 << n) - 1)) as u32
    }

    #[inline]
    fn drop_bits(&mut self, n: u32) -> Result<(), InflateError> {
        if self.cnt < n {
            return Err(InflateError::Truncated);
        }
        self.buf >>= n;
        self.cnt -= n;
        Ok(())
    }

    /// Drop the bits left in the current byte (stored blocks start on a byte).
    fn align(&mut self) {
        let drop = self.cnt % 8;
        self.buf >>= drop;
        self.cnt -= drop;
    }
}

/// A canonical Huffman code: how many codes each length has, the symbols in
/// code order (RFC 1951 3.2.2: codes of one length are consecutive integers),
/// and a look-up table for the codes up to FAST_BITS long.
struct Huffman<const N: usize> {
    count: [u16; MAX_BITS + 1],
    symbol: [u16; N],
    /// Indexed by the next FAST_BITS bits of the stream (a deflate code is
    /// read most-significant bit first, so the index is the code reversed):
    /// `len << 9 | symbol`, or 0 when the code there is longer than FAST_BITS
    /// (symbol 0 with a code has len != 0, so 0 never means a code).
    fast: [u16; 1 << FAST_BITS],
}

impl<const N: usize> Huffman<N> {
    /// Build from code lengths (0 = unused). Returns how much of the code
    /// space is left unassigned, in units of a 15-bit code: 0 for a complete
    /// code, > 0 for an incomplete one; an over-subscribed set is an error.
    fn new(lengths: &[u8]) -> Result<(Self, i32), InflateError> {
        let mut h = Huffman { count: [0; MAX_BITS + 1], symbol: [0; N], fast: [0; 1 << FAST_BITS] };
        for &l in lengths {
            h.count[l as usize] += 1;
        }
        if h.count[0] as usize == lengths.len() {
            return Ok((h, 0)); // no codes: complete, but decodes nothing
        }
        let mut left: i32 = 1;
        for &n in &h.count[1..] {
            left <<= 1;
            left -= n as i32;
            if left < 0 {
                return Err(InflateError::BadCode);
            }
        }
        let mut offs = [0u16; MAX_BITS + 1];
        for len in 1..MAX_BITS {
            offs[len + 1] = offs[len] + h.count[len];
        }
        // The first code of each length (RFC 1951 3.2.2 step 2; length 0,
        // the unused symbols, takes no code space).
        let mut next_code = [0u32; MAX_BITS + 1];
        let mut c = 0u32;
        for (len, nc) in next_code.iter_mut().enumerate().skip(1) {
            *nc = c;
            c = (c + h.count[len] as u32) << 1;
        }
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let len = l as usize;
                h.symbol[offs[len] as usize] = sym as u16;
                offs[len] += 1;
                let code = next_code[len];
                next_code[len] += 1;
                if len as u32 <= FAST_BITS {
                    // reverse the code's `len` bits: the stream delivers them
                    // first bit first into the low end of the index
                    let mut rev = 0u32;
                    for k in 0..len {
                        rev |= ((code >> k) & 1) << (len - 1 - k);
                    }
                    let entry = ((len as u16) << 9) | sym as u16;
                    let step = 1usize << len;
                    let mut i = rev as usize;
                    while i < (1 << FAST_BITS) {
                        h.fast[i] = entry;
                        i += step;
                    }
                }
            }
        }
        Ok((h, left))
    }

    /// One symbol: the table for a short code, else bit by bit: codes of
    /// length `len` are `first .. first + count[len]`, so a code is found when
    /// it falls in that range.
    #[inline]
    fn decode(&self, s: &mut Bits<'_>) -> Result<u16, InflateError> {
        let e = self.fast[s.peek(FAST_BITS) as usize];
        if e != 0 {
            s.drop_bits((e >> 9) as u32)?;
            return Ok(e & 0x1ff);
        }
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=MAX_BITS {
            code |= s.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - first < count {
                return Ok(self.symbol[(index + code - first) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(InflateError::BadSymbol)
    }
}

const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

struct Out<'a> {
    dst: &'a mut [u8],
    pos: usize,
}

/// Literal/length and distance symbols until end-of-block (RFC 1951 3.2.5).
fn codes<const L: usize, const D: usize>(
    s: &mut Bits<'_>,
    out: &mut Out<'_>,
    lencode: &Huffman<L>,
    distcode: &Huffman<D>,
) -> Result<(), InflateError> {
    loop {
        let sym = lencode.decode(s)? as usize;
        if sym < 256 {
            let Some(b) = out.dst.get_mut(out.pos) else {
                return Err(InflateError::OutputFull);
            };
            *b = sym as u8;
            out.pos += 1;
        } else if sym == 256 {
            return Ok(());
        } else {
            let i = sym - 257;
            if i >= LEN_BASE.len() {
                return Err(InflateError::BadSymbol); // 286, 287
            }
            let len = LEN_BASE[i] as usize + s.bits(LEN_EXTRA[i] as u32)? as usize;
            let d = distcode.decode(s)? as usize;
            if d >= DIST_BASE.len() {
                return Err(InflateError::BadSymbol); // 30, 31
            }
            let dist = DIST_BASE[d] as usize + s.bits(DIST_EXTRA[d] as u32)? as usize;
            if dist > out.pos {
                return Err(InflateError::DistanceTooFar);
            }
            if len > out.dst.len() - out.pos {
                return Err(InflateError::OutputFull);
            }
            let from = out.pos - dist;
            if dist >= len {
                // the source lies wholly before the bytes being written
                out.dst.copy_within(from..from + len, out.pos);
            } else {
                // Byte by byte: the source overlaps the bytes being written
                // (dist < len repeats a short run).
                for k in 0..len {
                    out.dst[out.pos + k] = out.dst[from + k];
                }
            }
            out.pos += len;
        }
    }
}

fn stored(s: &mut Bits<'_>, out: &mut Out<'_>) -> Result<(), InflateError> {
    s.align();
    let len = s.bits(16)?;
    let nlen = s.bits(16)?;
    if len != !nlen & 0xFFFF {
        return Err(InflateError::StoredLength);
    }
    let len = len as usize;
    // After align() the bit buffer holds whole bytes only; take them first.
    let mut n = 0;
    while n < len && s.cnt >= 8 {
        let b = s.bits(8)? as u8;
        let Some(o) = out.dst.get_mut(out.pos) else {
            return Err(InflateError::OutputFull);
        };
        *o = b;
        out.pos += 1;
        n += 1;
    }
    let rest = len - n;
    if rest > s.src.len() - s.pos {
        return Err(InflateError::Truncated);
    }
    if rest > out.dst.len() - out.pos {
        return Err(InflateError::OutputFull);
    }
    out.dst[out.pos..out.pos + rest].copy_from_slice(&s.src[s.pos..s.pos + rest]);
    out.pos += rest;
    s.pos += rest;
    Ok(())
}

fn fixed(s: &mut Bits<'_>, out: &mut Out<'_>) -> Result<(), InflateError> {
    let mut lengths = [0u8; FIX_LCODES];
    for (sym, l) in lengths.iter_mut().enumerate() {
        *l = match sym {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let (lencode, _) = Huffman::<FIX_LCODES>::new(&lengths)?;
    let (distcode, _) = Huffman::<MAX_DCODES>::new(&[5u8; MAX_DCODES])?;
    codes(s, out, &lencode, &distcode)
}

fn dynamic(s: &mut Bits<'_>, out: &mut Out<'_>) -> Result<(), InflateError> {
    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
    let nlen = s.bits(5)? as usize + 257;
    let ndist = s.bits(5)? as usize + 1;
    let ncode = s.bits(4)? as usize + 4;
    if nlen > MAX_LCODES || ndist > MAX_DCODES {
        return Err(InflateError::BadCode);
    }
    let mut lengths = [0u8; MAX_LCODES + MAX_DCODES];
    for &o in ORDER.iter().take(ncode) {
        lengths[o] = s.bits(3)? as u8;
    }
    let (lencode, left) = Huffman::<19>::new(&lengths[..19])?;
    if left != 0 {
        return Err(InflateError::BadCode); // the code-length code must be complete
    }
    let mut index = 0;
    while index < nlen + ndist {
        let sym = lencode.decode(s)?;
        if sym < 16 {
            lengths[index] = sym as u8;
            index += 1;
            continue;
        }
        let (value, repeat) = match sym {
            16 => {
                if index == 0 {
                    return Err(InflateError::BadSymbol); // nothing to repeat
                }
                (lengths[index - 1], 3 + s.bits(2)? as usize)
            }
            17 => (0, 3 + s.bits(3)? as usize),
            _ => (0, 11 + s.bits(7)? as usize),
        };
        if index + repeat > nlen + ndist {
            return Err(InflateError::BadSymbol);
        }
        lengths[index..index + repeat].fill(value);
        index += repeat;
    }
    if lengths[256] == 0 {
        return Err(InflateError::BadCode); // no end-of-block code
    }
    // An incomplete literal/length or distance code is allowed only when it
    // is a single code of length 1 (RFC 1951 3.2.7 permits one distance code).
    let (litcode, left) = Huffman::<MAX_LCODES>::new(&lengths[..nlen])?;
    if left > 0 && nlen - litcode.count[0] as usize != 1 {
        return Err(InflateError::BadCode);
    }
    let (distcode, left) = Huffman::<MAX_DCODES>::new(&lengths[nlen..nlen + ndist])?;
    if left > 0 && ndist - distcode.count[0] as usize != 1 {
        return Err(InflateError::BadCode);
    }
    codes(s, out, &litcode, &distcode)
}

/// Decode the raw-deflate stream `src` into `dst`. Returns the number of bytes
/// written. Bytes after the final block are ignored (the caller checks the
/// length and CRC of the result).
pub fn inflate(src: &[u8], dst: &mut [u8]) -> Result<usize, InflateError> {
    let mut s = Bits::new(src);
    let mut out = Out { dst, pos: 0 };
    loop {
        let last = s.bits(1)?;
        match s.bits(2)? {
            0 => stored(&mut s, &mut out)?,
            1 => fixed(&mut s, &mut out)?,
            2 => dynamic(&mut s, &mut out)?,
            _ => return Err(InflateError::BadBlockType),
        }
        if last == 1 {
            return Ok(out.pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Raw deflate streams made with Python's zlib (wbits=-15), see each.

    /// zlib.compressobj(0, DEFLATED, -15) of b"stored block, verbatim": one
    /// stored block (BTYPE 00).
    const STORED: &[u8] = &[
        0x01, 0x16, 0x00, 0xe9, 0xff, 0x73, 0x74, 0x6f, 0x72, 0x65, 0x64, 0x20, 0x62, 0x6c, 0x6f, 0x63, 0x6b, 0x2c,
        0x20, 0x76, 0x65, 0x72, 0x62, 0x61, 0x74, 0x69, 0x6d,
    ];

    /// zlib level 9 of b"abcabcabcabcabcabcabc hello hello hello": fixed
    /// Huffman (BTYPE 01) with overlapping matches.
    const FIXED: &[u8] = &[0x4b, 0x4c, 0x4a, 0x4e, 0xc4, 0x40, 0x0a, 0x19, 0xa9, 0x39, 0x39, 0xf9, 0xc8, 0x24, 0x00];

    /// zlib level 9 of DYNAMIC_TEXT below: one dynamic-Huffman block
    /// (BTYPE 10), 2223 bytes in, 255 out, zlib.crc32 0x4a120014.
    const DYNAMIC: &[u8] = &[
        0x95, 0xd5, 0x59, 0x52, 0xc3, 0x30, 0x10, 0x45, 0xd1, 0xff, 0xac, 0x42, 0x4b, 0x88, 0xba, 0x2d, 0x59, 0x62,
        0x37, 0x0c, 0x06, 0x02, 0x26, 0x86, 0x84, 0x30, 0xad, 0x9e, 0x02, 0xbd, 0x5e, 0xc0, 0xfd, 0x76, 0xdd, 0xd2,
        0x74, 0x24, 0xaf, 0x87, 0xe3, 0x92, 0xf6, 0x57, 0xe9, 0xfd, 0x71, 0x49, 0x6f, 0x97, 0xc3, 0xed, 0x73, 0xba,
        0x39, 0x6d, 0x9f, 0xc7, 0x74, 0xbf, 0x7d, 0xa5, 0xa7, 0xcb, 0xcb, 0xeb, 0x39, 0x6d, 0x1f, 0xcb, 0xe9, 0xff,
        0xf3, 0x7a, 0xfd, 0xf3, 0x9d, 0xee, 0xb6, 0x87, 0xb4, 0xdf, 0xad, 0x7f, 0x55, 0x66, 0x55, 0x1e, 0x95, 0xb1,
        0x6a, 0x1a, 0x95, 0xb3, 0xaa, 0x8f, 0x6a, 0x82, 0x33, 0xac, 0x23, 0x2b, 0x2c, 0xb3, 0x32, 0xb2, 0xca, 0x32,
        0xd7, 0x68, 0x33, 0xdc, 0x10, 0xad, 0xad, 0xb1, 0xac, 0x6a, 0x23, 0x3b, 0xcb, 0x9a, 0x4e, 0x2d, 0x43, 0x22,
        0xae, 0x0c, 0x1a, 0x31, 0xcd, 0x32, 0x53, 0x25, 0xb3, 0x3a, 0xe8, 0x64, 0x36, 0x75, 0x50, 0x4a, 0x64, 0x50,
        0x8a, 0xc7, 0x6e, 0x42, 0x2a, 0x35, 0xc6, 0x83, 0x56, 0xba, 0x64, 0x66, 0x88, 0xc5, 0xe3, 0xf8, 0xa0, 0x96,
        0x59, 0x2f, 0x83, 0x41, 0x2d, 0x59, 0xeb, 0x33, 0xc8, 0xa5, 0x68, 0x9e, 0x06, 0xb9, 0x74, 0x5d, 0x3d, 0x83,
        0x5c, 0x26, 0xf1, 0x34, 0xc8, 0xa5, 0xc7, 0xdb, 0x07, 0xbd, 0x4c, 0xb1, 0x3e, 0xe8, 0xa5, 0xc7, 0x3c, 0xa1,
        0x97, 0x12, 0xe7, 0x07, 0xbd, 0x34, 0x65, 0x90, 0x4b, 0x15, 0x4f, 0x87, 0x5c, 0x4c, 0xb7, 0xdd, 0x21, 0x97,
        0xa6, 0x79, 0x3a, 0xe4, 0x52, 0xe2, 0x27, 0x04, 0xb9, 0x98, 0x58, 0x3b, 0xe4, 0xd2, 0xf4, 0xc4, 0x3b, 0xe4,
        0x52, 0xc5, 0xcc, 0xe9, 0x9f, 0x28, 0xce, 0x01, 0x72, 0xc9, 0x31, 0x1e, 0xe5, 0xa2, 0xeb, 0xe7, 0xd4, 0x4b,
        0xdd, 0xfd, 0x02,
    ];

    /// The text DYNAMIC encodes: Python's
    /// b"".join(b"line %d: the quick brown fox jumps over the lazy dog %d\n" % (i, i*i % 97) for i in range(40)).
    fn dynamic_text() -> std::vec::Vec<u8> {
        let mut v = std::vec::Vec::new();
        for i in 0..40u32 {
            v.extend_from_slice(
                std::format!("line {}: the quick brown fox jumps over the lazy dog {}\n", i, i * i % 97).as_bytes(),
            );
        }
        v
    }

    #[test]
    fn dynamic_block() {
        let want = dynamic_text();
        assert_eq!(want.len(), 2223);
        assert_eq!(crate::crc32::crc32(&want), 0x4a12_0014);
        let mut out = [0u8; 4096];
        let n = inflate(DYNAMIC, &mut out).unwrap();
        assert_eq!(&out[..n], &want[..]);
    }

    #[test]
    fn dynamic_block_every_truncation_and_bit_flip_is_caught() {
        let mut out = [0u8; 4096];
        for cut in 0..DYNAMIC.len() - 1 {
            assert!(inflate(&DYNAMIC[..cut], &mut out).is_err(), "cut at {cut}");
        }
        // A flipped bit is either a decode error or a different output (which
        // the image CRC then catches); never a panic or an overrun.
        for i in 0..DYNAMIC.len() * 8 {
            let mut bad = DYNAMIC.to_vec();
            bad[i / 8] ^= 1 << (i % 8);
            if let Ok(n) = inflate(&bad, &mut out) {
                assert!(n <= out.len());
            }
        }
    }

    #[test]
    fn stored_block() {
        let mut out = [0u8; 64];
        let n = inflate(STORED, &mut out).unwrap();
        assert_eq!(&out[..n], b"stored block, verbatim");
    }

    #[test]
    fn fixed_block_with_overlapping_copies() {
        let mut out = [0u8; 64];
        let n = inflate(FIXED, &mut out).unwrap();
        assert_eq!(&out[..n], b"abcabcabcabcabcabcabc hello hello hello");
    }

    #[test]
    fn output_too_small_is_an_error() {
        let mut out = [0u8; 10];
        assert_eq!(inflate(FIXED, &mut out), Err(InflateError::OutputFull));
        assert_eq!(inflate(STORED, &mut out), Err(InflateError::OutputFull));
    }

    #[test]
    fn truncated_input_is_an_error() {
        let mut out = [0u8; 64];
        for cut in 0..FIXED.len() - 1 {
            assert!(inflate(&FIXED[..cut], &mut out).is_err(), "cut at {cut}");
        }
        for cut in 0..STORED.len() {
            assert!(inflate(&STORED[..cut], &mut out).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn block_type_3_and_bad_nlen() {
        let mut out = [0u8; 64];
        assert_eq!(inflate(&[0x07], &mut out), Err(InflateError::BadBlockType));
        let mut bad = STORED.to_vec();
        bad[3] ^= 1;
        assert_eq!(inflate(&bad, &mut out), Err(InflateError::StoredLength));
    }

    #[test]
    fn distance_before_start() {
        // Fixed block: length symbol 257 (len 3, code 0000001) then distance
        // code 0 (dist 1) with nothing written yet.
        // Bits LSB first: BFINAL=1, BTYPE=01, then 7-bit code 0000001
        // (MSB first), then 5-bit distance 00000.
        let mut out = [0u8; 16];
        let mut bits: u32 = 0;
        let mut n = 0;
        let mut put = |v: u32, len: u32, msb_first: bool| {
            for k in 0..len {
                let bit = if msb_first { (v >> (len - 1 - k)) & 1 } else { (v >> k) & 1 };
                bits |= bit << n;
                n += 1;
            }
        };
        put(1, 1, false);
        put(1, 2, false);
        put(1, 7, true);
        put(0, 5, true);
        let stream = bits.to_le_bytes();
        assert_eq!(inflate(&stream, &mut out), Err(InflateError::DistanceTooFar));
    }

    #[test]
    fn huffman_rejects_oversubscribed() {
        assert!(Huffman::<4>::new(&[1, 1, 1, 0]).is_err());
        let (_, left) = Huffman::<4>::new(&[1, 2, 2, 0]).unwrap();
        assert_eq!(left, 0);
        // incomplete: one length-2 code unused (counted in length-15 units)
        let (_, left) = Huffman::<4>::new(&[1, 2, 0, 0]).unwrap();
        assert_eq!(left, 1 << 13);
    }
}
