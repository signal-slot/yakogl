// SPDX-License-Identifier: Apache-2.0
//! `yakogl-boot-stage2`: the logic of the Verdin AM62P second-stage loader
//! (docs/notes/am62p-direct-boot.md, "The second stage").
//!
//! The A53 SPL loads the loader from the eMMC boot partition (the u-boot.img
//! slot holds 1.49 MiB, too small for a GL image). The loader takes the real
//! image from its own .rodata or from a raw sector range of the eMMC user
//! area, checks it, places it and jumps to it. This crate is that check, the
//! placing and the jump; Zephyr's disk API, the log and the cache flush are
//! the C side's (`src/main.c`).
//!
//! The payload: one 512-byte header sector, then the stored bytes. Two header
//! versions, both read by this loader. Version 1, one image, little-endian:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 8 | magic `"YKSTAGE2"` |
//! | 8 | 4 | header version, 1 |
//! | 12 | 4 | header bytes, 512 |
//! | 16 | 4 | compression: 0 none, 1 raw deflate (RFC 1951) |
//! | 20 | 4 | reserved, 0 |
//! | 24 | 8 | load address |
//! | 32 | 8 | entry address |
//! | 40 | 8 | stored bytes (the payload after the header) |
//! | 48 | 8 | image bytes (after decompression) |
//! | 56 | 4 | CRC-32 of the stored bytes |
//! | 60 | 4 | CRC-32 of the image |
//! | 64 | 32 | name, ASCII, NUL padded (for the log) |
//! | 96 | 412 | zero |
//! | 508 | 4 | CRC-32 of bytes 0..508 |
//!
//! Version 2, up to [`MAX_SEGMENTS`] segments, each placed at its own load
//! address (the AM62P showcase: the A53 Supervisor, the R5F's DDR image and
//! its TCM blob, the three segments its kexec route loads):
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 8 | magic `"YKSTAGE2"` |
//! | 8 | 4 | header version, 2 |
//! | 12 | 4 | header bytes, 512 |
//! | 16 | 4 | segment count, 1..=8 |
//! | 20 | 4 | image parts P: the first P segments are the image (below); 0 reads as 1 |
//! | 24 | 8 | entry address (inside segment 0) |
//! | 32 | 8 | stored bytes: the payload after the header, a multiple of 512 |
//! | 40 | 24 | zero |
//! | 64 | 32 | name |
//! | 96 | 8 x 40 | segment table (unused entries zero) |
//! | 416 | 92 | zero |
//! | 508 | 4 | CRC-32 of bytes 0..508 |
//!
//! A segment entry:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 8 | load address |
//! | 8 | 4 | image bytes |
//! | 12 | 4 | footprint: the bytes of RAM the segment owns from its load address, at least the image bytes (segment 0: the arm64 header's image_size, so its .bss is not another segment's) |
//! | 16 | 4 | stored offset from the first byte after the header, a multiple of 512 |
//! | 20 | 4 | stored bytes |
//! | 24 | 4 | compression: 0 none, 1 raw deflate |
//! | 28 | 4 | CRC-32 of the stored bytes |
//! | 32 | 4 | CRC-32 of the image |
//! | 36 | 4 | zero bytes: the bytes of RAM right after the image bytes (`load + image bytes ..`) that the loader clears; image bytes + zero bytes <= footprint |
//!
//! The stored ranges are in table order and do not overlap; the footprints
//! lie in the loader's region, do not overlap each other and do not overlap
//! the staging buffer a compressed eMMC payload is read to. Segment 0 is an
//! arm64 Image and holds the entry. `tools/am62p-mkboot.py stage2img` writes
//! both versions; the CRC-32 is zlib's.
//!
//! Zero bytes and image parts (2026-11-13) keep zeros out of the payload. The
//! Verdin AM62P showcase image holds a 10.85 MB run of zeros in its file and
//! is followed in RAM by a 46.4 MB .bss that it cleared itself with the MMU
//! and caches off. The loader runs with both on and waits while the eMMC
//! transfers, so it clears those ranges instead (the C side, on a thread
//! under the reads): a run of zeros in the file is not stored, read or
//! CRC-ed, and an image that is only ever started by this loader, with a
//! payload whose zero bytes cover its .bss, may skip its own clear.
//!
//! With P > 1 the image is cut at its zero runs: segment 0 is its first
//! extent (the arm64 header, the entry) and segments 1..P-1 are the later
//! extents, each at its own address inside segment 0's footprint, each
//! followed by its zero bytes. For those parts: they start at or above the
//! end of segment 0's image bytes + zero bytes, end at or below the end of
//! segment 0's footprint, are in ascending address order, and their
//! footprint is exactly their image bytes + zero bytes. The overlap check
//! takes segment 0 against one of its parts with segment 0's image bytes +
//! zero bytes in place of its footprint; every other pair, and every segment
//! against the staging buffer, is checked with the footprints. So nothing
//! that is read or inflated lands in a zero range, and no two zero ranges
//! share a byte.
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod crc32;
pub mod inflate;

use core::ffi::c_char;

pub const MAGIC: [u8; 8] = *b"YKSTAGE2";
/// The single-image header.
pub const VERSION_SINGLE: u32 = 1;
/// The segment-table header.
pub const VERSION_SEGMENTS: u32 = 2;
pub const HEADER_BYTES: usize = 512;
pub const SECTOR_BYTES: u64 = 512;
pub const COMPRESSION_NONE: u32 = 0;
pub const COMPRESSION_DEFLATE: u32 = 1;
pub const NAME_BYTES: usize = 32;
pub const MAX_SEGMENTS: usize = 8;
/// Where the version-2 segment table starts in the header, and its entries' size.
pub const SEGMENT_TABLE: usize = 96;
pub const SEGMENT_ENTRY_BYTES: usize = 40;

/// What the loader accepts, from its Kconfig (include/boot_stage2.h).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Bs2Limits {
    /// The loader's load address: the image's link address (version 1), the
    /// load address of segment 0 (version 2), and the start of the region.
    pub load: u64,
    /// The end (exclusive) of the identity-mapped region every segment must
    /// lie in.
    pub region_end: u64,
    /// The most bytes the source holds after the header sector.
    pub max_stored: u64,
    /// The staging buffer: a compressed eMMC payload is read there before
    /// it is inflated. A version-1 image must end below it (as before);
    /// version-2 segments must not overlap `staging .. staging + staging_bytes`.
    pub staging: u64,
    /// 0 when nothing is staged (the payload linked into the loader).
    pub staging_bytes: u64,
}

/// One segment, checked (include/boot_stage2.h).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bs2Segment {
    pub load: u64,
    pub image_bytes: u64,
    pub footprint: u64,
    /// From the first byte after the header.
    pub stored_offset: u64,
    pub stored_bytes: u64,
    pub compression: u32,
    pub stored_crc32: u32,
    pub image_crc32: u32,
    /// The bytes after the image bytes that the loader clears (version 2;
    /// inside the footprint).
    pub zero_bytes: u32,
}

/// The header, checked (include/boot_stage2.h). A version-1 header is one
/// segment at stored offset 0.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bs2Info {
    pub segments: [Bs2Segment; MAX_SEGMENTS],
    pub entry: u64,
    /// The payload after the header (all segments).
    pub stored_bytes: u64,
    pub version: u32,
    pub count: u32,
    /// NUL terminated (the last byte is always 0).
    pub name: [u8; NAME_BYTES + 1],
}

// The layouts include/boot_stage2.h pins too.
const _: () = assert!(core::mem::size_of::<Bs2Limits>() == 40);
const _: () = assert!(core::mem::size_of::<Bs2Segment>() == 56);
const _: () = assert!(core::mem::offset_of!(Bs2Segment, compression) == 40);
const _: () = assert!(core::mem::offset_of!(Bs2Segment, zero_bytes) == 52);
const _: () = assert!(core::mem::offset_of!(Bs2Info, entry) == 448);
const _: () = assert!(core::mem::offset_of!(Bs2Info, version) == 464);
const _: () = assert!(core::mem::offset_of!(Bs2Info, name) == 472);
const _: () = assert!(core::mem::size_of::<Bs2Info>() == 512);
const _: () = assert!(SEGMENT_TABLE + MAX_SEGMENTS * SEGMENT_ENTRY_BYTES <= 508);

/// The loader's error codes (negative; `bs2_strerror` names them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Bs2Error {
    BadArgument = -1,
    BadMagic = -2,
    BadHeaderCrc = -3,
    BadVersion = -4,
    BadCompression = -5,
    WrongLoad = -6,
    EntryOutside = -7,
    StoredTooBig = -8,
    ImageTooBig = -9,
    SizesDisagree = -10,
    StoredCrc = -11,
    ImageCrc = -12,
    Inflate = -13,
    LengthMismatch = -14,
    NotArm64Image = -15,
    SegmentCount = -16,
    SegmentLayout = -17,
    SegmentOutside = -18,
    SegmentOverlap = -19,
    Footprint = -20,
    ZeroRange = -21,
    ImageParts = -22,
}

const ALL_ERRORS: [Bs2Error; 22] = {
    use Bs2Error::*;
    [
        BadArgument,
        BadMagic,
        BadHeaderCrc,
        BadVersion,
        BadCompression,
        WrongLoad,
        EntryOutside,
        StoredTooBig,
        ImageTooBig,
        SizesDisagree,
        StoredCrc,
        ImageCrc,
        Inflate,
        LengthMismatch,
        NotArm64Image,
        SegmentCount,
        SegmentLayout,
        SegmentOutside,
        SegmentOverlap,
        Footprint,
        ZeroRange,
        ImageParts,
    ]
};

impl Bs2Error {
    fn from_code(code: i32) -> Option<Self> {
        ALL_ERRORS.into_iter().find(|e| *e as i32 == code)
    }

    /// NUL-terminated, for the C side's log.
    pub fn text(self) -> &'static str {
        match self {
            Bs2Error::BadArgument => "null pointer, short buffer or bad segment index\0",
            Bs2Error::BadMagic => "no YKSTAGE2 header at the configured sector\0",
            Bs2Error::BadHeaderCrc => "header CRC-32 mismatch\0",
            Bs2Error::BadVersion => "unknown header version or size, or a reserved field is not zero\0",
            Bs2Error::BadCompression => "unknown compression\0",
            Bs2Error::WrongLoad => "load address (of segment 0) is not the one this loader serves\0",
            Bs2Error::EntryOutside => "entry address outside the image (segment 0)\0",
            Bs2Error::StoredTooBig => "stored payload larger than the source holds\0",
            Bs2Error::ImageTooBig => "image larger than the room at the load address\0",
            Bs2Error::SizesDisagree => "stored, image or footprint sizes disagree (or are zero)\0",
            Bs2Error::StoredCrc => "CRC-32 of the stored payload mismatch\0",
            Bs2Error::ImageCrc => "CRC-32 of the image mismatch\0",
            Bs2Error::Inflate => "deflate stream is malformed\0",
            Bs2Error::LengthMismatch => "decompressed length is not the header's\0",
            Bs2Error::NotArm64Image => "no arm64 Image header (ARM\\x64 at 0x38)\0",
            Bs2Error::SegmentCount => "segment count is 0 or over 8\0",
            Bs2Error::SegmentLayout => "segment stored ranges not sector aligned, out of order or past the payload\0",
            Bs2Error::SegmentOutside => "a segment lies outside the loader's RAM region\0",
            Bs2Error::SegmentOverlap => "segments overlap each other or the staging buffer\0",
            Bs2Error::Footprint => "the image's arm64 image_size is larger than its segment's footprint\0",
            Bs2Error::ZeroRange => "a segment's image bytes and zero bytes together are over its footprint\0",
            Bs2Error::ImageParts => {
                "image parts: more than the segments, or a part outside segment 0's footprint, on its image, out of order or with a footprint of its own\0"
            }
        }
    }
}

fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn le64(b: &[u8], off: usize) -> u64 {
    (le32(b, off) as u64) | ((le32(b, off + 4) as u64) << 32)
}

fn round_up_sector(n: u64) -> Option<u64> {
    Some(n.checked_add(SECTOR_BYTES - 1)? & !(SECTOR_BYTES - 1))
}

/// `[a, a + an)` and `[b, b + bn)` share a byte (lengths > 0, no overflow).
fn overlaps(a: u64, an: u64, b: u64, bn: u64) -> bool {
    a < b + bn && b < a + an
}

/// Parse and check a header sector against the loader's limits.
pub fn parse_header(sector: &[u8], limits: &Bs2Limits) -> Result<Bs2Info, Bs2Error> {
    if sector.len() < HEADER_BYTES {
        return Err(Bs2Error::BadArgument);
    }
    let h = &sector[..HEADER_BYTES];
    if h[..8] != MAGIC {
        return Err(Bs2Error::BadMagic);
    }
    if crc32::crc32(&h[..508]) != le32(h, 508) {
        return Err(Bs2Error::BadHeaderCrc);
    }
    if le32(h, 12) as usize != HEADER_BYTES {
        return Err(Bs2Error::BadVersion);
    }
    let mut name = [0u8; NAME_BYTES + 1];
    name[..NAME_BYTES].copy_from_slice(&h[64..64 + NAME_BYTES]);
    match le32(h, 8) {
        VERSION_SINGLE => parse_v1(h, limits, name),
        VERSION_SEGMENTS => parse_v2(h, limits, name),
        _ => Err(Bs2Error::BadVersion),
    }
}

/// Version 1: the checks as they were, then one segment.
fn parse_v1(h: &[u8], limits: &Bs2Limits, name: [u8; NAME_BYTES + 1]) -> Result<Bs2Info, Bs2Error> {
    let compression = le32(h, 16);
    if compression != COMPRESSION_NONE && compression != COMPRESSION_DEFLATE {
        return Err(Bs2Error::BadCompression);
    }
    let load = le64(h, 24);
    let entry = le64(h, 32);
    let stored_bytes = le64(h, 40);
    let image_bytes = le64(h, 48);
    if load != limits.load {
        return Err(Bs2Error::WrongLoad);
    }
    if stored_bytes == 0 || image_bytes == 0 || (compression == COMPRESSION_NONE && stored_bytes != image_bytes) {
        return Err(Bs2Error::SizesDisagree);
    }
    if stored_bytes > limits.max_stored {
        return Err(Bs2Error::StoredTooBig);
    }
    // The image ends below the staging buffer, or, without one above the
    // load address, at the region's end.
    let room = if limits.staging > load { limits.staging } else { limits.region_end }.saturating_sub(load);
    if image_bytes > room {
        return Err(Bs2Error::ImageTooBig);
    }
    if entry < load || entry - load >= image_bytes {
        return Err(Bs2Error::EntryOutside);
    }
    let mut segments = [Bs2Segment::default(); MAX_SEGMENTS];
    segments[0] = Bs2Segment {
        load,
        image_bytes,
        footprint: image_bytes,
        stored_offset: 0,
        stored_bytes,
        compression,
        stored_crc32: le32(h, 56),
        image_crc32: le32(h, 60),
        zero_bytes: 0,
    };
    Ok(Bs2Info { segments, entry, stored_bytes, version: VERSION_SINGLE, count: 1, name })
}

fn parse_v2(h: &[u8], limits: &Bs2Limits, name: [u8; NAME_BYTES + 1]) -> Result<Bs2Info, Bs2Error> {
    let table_end = SEGMENT_TABLE + MAX_SEGMENTS * SEGMENT_ENTRY_BYTES;
    if h[40..64].iter().chain(&h[table_end..508]).any(|&b| b != 0) {
        return Err(Bs2Error::BadVersion);
    }
    let count = le32(h, 16) as usize;
    if count == 0 || count > MAX_SEGMENTS {
        return Err(Bs2Error::SegmentCount);
    }
    // The field was reserved, 0, before image parts: 0 is one part.
    let parts = le32(h, 20).max(1) as usize;
    if parts > count {
        return Err(Bs2Error::ImageParts);
    }
    let entry = le64(h, 24);
    let stored_total = le64(h, 32);
    if !stored_total.is_multiple_of(SECTOR_BYTES) {
        return Err(Bs2Error::SegmentLayout);
    }
    if stored_total > limits.max_stored {
        return Err(Bs2Error::StoredTooBig);
    }
    let mut segments = [Bs2Segment::default(); MAX_SEGMENTS];
    let mut stored_end = 0u64;
    for (i, seg) in segments.iter_mut().enumerate() {
        let e = &h[SEGMENT_TABLE + i * SEGMENT_ENTRY_BYTES..SEGMENT_TABLE + (i + 1) * SEGMENT_ENTRY_BYTES];
        if i >= count {
            if e.iter().any(|&b| b != 0) {
                return Err(Bs2Error::BadVersion);
            }
            continue;
        }
        *seg = Bs2Segment {
            load: le64(e, 0),
            image_bytes: le32(e, 8) as u64,
            footprint: le32(e, 12) as u64,
            stored_offset: le32(e, 16) as u64,
            stored_bytes: le32(e, 20) as u64,
            compression: le32(e, 24),
            stored_crc32: le32(e, 28),
            image_crc32: le32(e, 32),
            zero_bytes: le32(e, 36),
        };
        if seg.compression != COMPRESSION_NONE && seg.compression != COMPRESSION_DEFLATE {
            return Err(Bs2Error::BadCompression);
        }
        if seg.stored_bytes == 0
            || seg.image_bytes == 0
            || seg.footprint < seg.image_bytes
            || (seg.compression == COMPRESSION_NONE && seg.stored_bytes != seg.image_bytes)
        {
            return Err(Bs2Error::SizesDisagree);
        }
        if seg.image_bytes + seg.zero_bytes as u64 > seg.footprint {
            return Err(Bs2Error::ZeroRange);
        }
        // In table order, sector aligned, inside the payload; whole sectors
        // of one segment never reach into the next.
        let end = round_up_sector(seg.stored_offset + seg.stored_bytes).ok_or(Bs2Error::SegmentLayout)?;
        if !seg.stored_offset.is_multiple_of(SECTOR_BYTES) || seg.stored_offset < stored_end || end > stored_total {
            return Err(Bs2Error::SegmentLayout);
        }
        stored_end = end;
        // Inside the region (the loader's identity map), no wrap.
        let top = seg.load.checked_add(seg.footprint).ok_or(Bs2Error::SegmentOutside)?;
        if seg.load < limits.load || top > limits.region_end {
            return Err(Bs2Error::SegmentOutside);
        }
    }
    if segments[0].load != limits.load {
        return Err(Bs2Error::WrongLoad);
    }
    let used = &segments[..count];
    let s0 = &segments[0];
    // What segment 0 itself writes: its image bytes and the zeros after them.
    // Its parts lie above that, inside its footprint, one after the other
    // (no sum here wraps: every segment's end was checked against the region).
    let occupied = s0.image_bytes + s0.zero_bytes as u64;
    for i in 1..parts {
        let p = &used[i];
        if p.load < s0.load + occupied
            || p.load + p.footprint > s0.load + s0.footprint
            || (i > 1 && p.load <= used[i - 1].load)
            || p.footprint != p.image_bytes + p.zero_bytes as u64
        {
            return Err(Bs2Error::ImageParts);
        }
    }
    for (i, a) in used.iter().enumerate() {
        if limits.staging_bytes != 0 && overlaps(a.load, a.footprint, limits.staging, limits.staging_bytes) {
            return Err(Bs2Error::SegmentOverlap);
        }
        for (j, b) in used.iter().enumerate().skip(i + 1) {
            // Segment 0 against one of its parts: the part is inside the
            // footprint by design, so only what segment 0 writes counts.
            let an = if i == 0 && j < parts { occupied } else { a.footprint };
            if overlaps(a.load, an, b.load, b.footprint) {
                return Err(Bs2Error::SegmentOverlap);
            }
        }
    }
    if entry < s0.load || entry - s0.load >= s0.image_bytes {
        return Err(Bs2Error::EntryOutside);
    }
    Ok(Bs2Info { segments, entry, stored_bytes: stored_total, version: VERSION_SEGMENTS, count: count as u32, name })
}

/// The image must be an arm64 Image (Zephyr's CONFIG_AARCH64_IMAGE_HEADER):
/// "ARM\x64" at 0x38. Catches a header that describes the wrong file.
pub fn check_arm64_image(image: &[u8]) -> Result<(), Bs2Error> {
    if image.len() < 64 || le32(image, 0x38) != 0x644D_5241 {
        return Err(Bs2Error::NotArm64Image);
    }
    Ok(())
}

/// Produce segment `index` from its stored bytes and check it: inflate
/// `stored` into `image`, or copy it there (uncompressed), or, with `image`
/// None, check an uncompressed segment that was read straight to its load
/// address (`stored` is then the image). `image` must hold at least the
/// segment's image bytes and must not overlap `stored`. Segment 0 must be an
/// arm64 Image, and (version 2) its header's image_size must fit the
/// footprint the overlap checks used. The zero bytes are not touched here:
/// clearing them is the caller's (`src/main.c`). Returns the image's length.
pub fn unpack_segment(
    info: &Bs2Info,
    index: usize,
    stored: &[u8],
    image: Option<&mut [u8]>,
) -> Result<usize, Bs2Error> {
    if index >= info.count as usize || index >= MAX_SEGMENTS {
        return Err(Bs2Error::BadArgument);
    }
    let seg = &info.segments[index];
    let n = seg.image_bytes as usize;
    if stored.len() as u64 != seg.stored_bytes || image.as_ref().is_some_and(|d| d.len() < n) {
        return Err(Bs2Error::BadArgument);
    }
    /* The hardware CRC first; the table only if it disagrees, so a wrong
     * hardware path (never run before 2026-10-22) costs time, not the boot. */
    if crc32::crc32(stored) != seg.stored_crc32 && crc32::crc32_update_table(0, stored) != seg.stored_crc32 {
        return Err(Bs2Error::StoredCrc);
    }
    let img: &[u8] = match (seg.compression, image) {
        (COMPRESSION_DEFLATE, Some(dst)) => {
            let dst = &mut dst[..n];
            let got = inflate::inflate(stored, dst).map_err(|_| Bs2Error::Inflate)?;
            if got != n {
                return Err(Bs2Error::LengthMismatch);
            }
            if crc32::crc32(dst) != seg.image_crc32 {
                return Err(Bs2Error::ImageCrc);
            }
            dst
        }
        // Inflate needs a destination of its own.
        (COMPRESSION_DEFLATE, None) => return Err(Bs2Error::BadArgument),
        /* Uncompressed, the image is the stored bytes: the CRC over them was
         * taken above, so a second pass only repeats it (0.48 s for 66 MB with
         * the table). The header's two values must still agree. */
        (_, dst) => {
            if seg.image_crc32 != seg.stored_crc32 {
                return Err(Bs2Error::ImageCrc);
            }
            match dst {
                Some(d) => {
                    d[..n].copy_from_slice(stored);
                    &d[..n]
                }
                None => stored,
            }
        }
    };
    if index == 0 {
        check_arm64_image(img)?;
        if info.version >= VERSION_SEGMENTS && le64(img, 16) > seg.footprint {
            return Err(Bs2Error::Footprint);
        }
    }
    Ok(n)
}

/* ---- the C ABI (include/boot_stage2.h) ---- */

/// Parse the header sector `sector` (`len` >= 512) into `*out`. 0 or a
/// negative Bs2Error code.
///
/// # Safety
/// `sector` points at `len` readable bytes; `limits` and `out` are valid.
#[no_mangle]
pub unsafe extern "C" fn bs2_header_parse(
    sector: *const u8,
    len: usize,
    limits: *const Bs2Limits,
    out: *mut Bs2Info,
) -> i32 {
    if sector.is_null() || limits.is_null() || out.is_null() {
        return Bs2Error::BadArgument as i32;
    }
    // SAFETY: the caller guarantees the pointers and the length.
    let (s, l) = unsafe { (core::slice::from_raw_parts(sector, len), &*limits) };
    match parse_header(s, l) {
        Ok(info) => {
            // SAFETY: out is valid for a write.
            unsafe { out.write(info) };
            0
        }
        Err(e) => e as i32,
    }
}

/// Place segment `index` at its load address from its stored bytes at
/// `stored` (`info->segments[index].stored_bytes` of them): inflate or copy
/// them there, or, when `stored` is the load address itself (an uncompressed
/// segment read in place), only check them. 0 or a negative Bs2Error code.
///
/// # Safety
/// `info` is what bs2_header_parse wrote, with limits that describe memory
/// this loader may write (the segment's load address and image bytes lie in
/// its identity-mapped region); `stored` points at the segment's stored bytes.
#[no_mangle]
pub unsafe extern "C" fn bs2_segment_unpack(info: *const Bs2Info, index: u32, stored: *const u8) -> i32 {
    if info.is_null() || stored.is_null() {
        return Bs2Error::BadArgument as i32;
    }
    // SAFETY: as documented above.
    let info = unsafe { &*info };
    let index = index as usize;
    if index >= info.count as usize || index >= MAX_SEGMENTS {
        return Bs2Error::BadArgument as i32;
    }
    let seg = &info.segments[index];
    let (s, sn) = (stored as u64, seg.stored_bytes);
    // SAFETY: the caller guarantees `stored` holds the segment's stored bytes.
    let src = unsafe { core::slice::from_raw_parts(stored, sn as usize) };
    let r = if s == seg.load {
        unpack_segment(info, index, src, None)
    } else if overlaps(s, sn, seg.load, seg.image_bytes) {
        Err(Bs2Error::BadArgument)
    } else {
        // SAFETY: the header check put the segment inside the loader's
        // identity-mapped region; it does not overlap `stored` (above).
        let dst = unsafe { core::slice::from_raw_parts_mut(seg.load as usize as *mut u8, seg.image_bytes as usize) };
        unpack_segment(info, index, src, Some(dst))
    };
    match r {
        Ok(_) => 0,
        Err(e) => e as i32,
    }
}

/// CRC-32 (zlib's) of `len` bytes.
///
/// # Safety
/// `data` points at `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn bs2_crc32(data: *const u8, len: usize) -> u32 {
    if data.is_null() {
        return 0;
    }
    // SAFETY: the caller guarantees the length.
    crc32::crc32(unsafe { core::slice::from_raw_parts(data, len) })
}

/// The text of a code bs2_header_parse / bs2_segment_unpack returned.
#[no_mangle]
pub extern "C" fn bs2_strerror(code: i32) -> *const c_char {
    match Bs2Error::from_code(code) {
        Some(e) => e.text().as_ptr() as *const c_char,
        None => c"unknown error".as_ptr(),
    }
}

/// Hand the CPU to the loaded image, in the state the A53 SPL hands one over
/// in -- MMU, data and instruction caches off -- but at EL1 (Zephyr's reset
/// path dropped the loader from EL2; the image's reset path takes EL1 entry,
/// with EL2 already set up by the loader's own reset path).
///
/// The caller has masked interrupts, quiesced its devices and cleaned and
/// invalidated the whole data cache to the point of coherency
/// (sys_cache_data_flush_and_invd_all). This routine then, from registers
/// only: masks DAIF, stops both EL1 timers, turns SCTLR_EL1.M/C/I off,
/// invalidates the instruction cache and the EL1 TLB and branches to `entry`
/// with x0..x3 = 0. The loader's own code is identity mapped (its
/// KERNEL_VM_BASE is its RAM base), so the instruction after the SCTLR write
/// is fetched from the same physical address.
///
/// # Safety
/// `entry` is the entry point of an image that has been written and made
/// visible to instruction fetch as described; this never returns.
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
#[no_mangle]
pub unsafe extern "C" fn bs2_jump(entry: u64) -> ! {
    // SAFETY: the caller's contract above; the sequence touches no memory.
    unsafe {
        core::arch::asm!(
            "msr daifset, #0xf",
            "msr cntv_ctl_el0, xzr",
            "msr cntp_ctl_el0, xzr",
            "isb",
            "mrs x9, sctlr_el1",
            "bic x9, x9, #0x1",    // M: MMU off
            "bic x9, x9, #0x4",    // C: data cache off
            "bic x9, x9, #0x1000", // I: instruction cache off
            "msr sctlr_el1, x9",
            "isb",
            "ic iallu",
            "tlbi vmalle1",
            "dsb sy",
            "isb",
            "mov x0, xzr",
            "mov x1, xzr",
            "mov x2, xzr",
            "mov x3, xzr",
            "br x10",
            // x10, not a free register: x9 is scratch above and x0..x3 are
            // zeroed, so the entry must be in none of them.
            in("x10") entry,
            options(noreturn, nostack),
        )
    }
}

/// The panic handler of the staticlib: hand the location to the C side,
/// which prints it and stops. Nothing here panics on purpose (every input
/// error is a Bs2Error); this is for a broken invariant only.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    unsafe extern "C" {
        fn bs2_panic(file: *const u8, file_len: usize, line: u32) -> !;
    }
    let (file, line) = match info.location() {
        Some(l) => (l.file(), l.line()),
        None => ("?", 0),
    };
    // SAFETY: the C side reads `file_len` bytes from `file` and never returns.
    unsafe { bs2_panic(file.as_ptr(), file.len(), line) }
}

#[cfg(test)]
mod tests;
