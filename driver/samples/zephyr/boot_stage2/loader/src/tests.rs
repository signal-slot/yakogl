// SPDX-License-Identifier: Apache-2.0
//! Host tests of the header checks, the unpack and the cross-check with
//! `tools/am62p-mkboot.py stage2-vectors` (BS2_VECTORS): versions 1 and 2
//! first, then version 3 (the segment table).
use super::*;
use std::vec::Vec;

const LOAD: u64 = 0x8200_0000;
const LIMITS: Bs2Limits = Bs2Limits {
    load: LOAD,
    max_stored: 1 << 20,
    max_image: 1 << 22,
    region_end: LOAD + (64 << 20),
    staging: LOAD + (1 << 22),
    staging_bytes: 0,
};
/// The AM62P loader's own limits: region 0x82000000..0x97800000, the
/// default user-area range (sectors 64..8191) and staging at 0x90000000.
const AM62P: Bs2Limits = Bs2Limits {
    load: LOAD,
    max_stored: 8128 * 512 - 512,
    max_image: 0x9000_0000 - LOAD,
    region_end: 0x9780_0000,
    staging: 0x9000_0000,
    staging_bytes: 8128 * 512 - 512,
};

fn zimage(len: usize) -> Vec<u8> {
    let mut z: Vec<u8> = (0..len).map(|i| (i * 13 / 7) as u8).collect();
    z[0x38..0x3C].copy_from_slice(b"ARM\x64");
    z
}

/// A header in mkboot's layout (the Python writer is checked against the
/// same layout by the BS2_VECTORS test below).
fn header(info: &Bs2Info) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..8].copy_from_slice(&MAGIC);
    h[8..12].copy_from_slice(&VERSION.to_le_bytes());
    h[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    h[16..20].copy_from_slice(&info.compression.to_le_bytes());
    h[24..32].copy_from_slice(&info.load.to_le_bytes());
    h[32..40].copy_from_slice(&info.entry.to_le_bytes());
    h[40..48].copy_from_slice(&info.stored_bytes.to_le_bytes());
    h[48..56].copy_from_slice(&info.image_bytes.to_le_bytes());
    h[56..60].copy_from_slice(&info.stored_crc32.to_le_bytes());
    h[60..64].copy_from_slice(&info.image_crc32.to_le_bytes());
    h[64..96].copy_from_slice(&info.name[..32]);
    h[96..104].copy_from_slice(&info.zero_offset.to_le_bytes());
    h[104..112].copy_from_slice(&info.zero_bytes.to_le_bytes());
    let c = crc32::crc32(&h[..508]);
    h[508..].copy_from_slice(&c.to_le_bytes());
    h
}

fn info_for(image: &[u8]) -> Bs2Info {
    let mut name = [0u8; 33];
    name[..4].copy_from_slice(b"test");
    Bs2Info {
        load: 0x8200_0000,
        entry: 0x8200_0000,
        stored_bytes: image.len() as u64,
        image_bytes: image.len() as u64,
        compression: COMPRESSION_NONE,
        stored_crc32: crc32::crc32(image),
        image_crc32: crc32::crc32(image),
        name,
        zero_offset: 0,
        zero_bytes: 0,
        version: VERSION,
        count: 0,
        segments: [Bs2Segment::default(); MAX_SEGMENTS],
    }
}

#[test]
fn header_round_trip() {
    let z = zimage(5000);
    let info = info_for(&z);
    let h = header(&info);
    assert_eq!(parse_header(&h, &LIMITS), Ok(info));
    assert_eq!(unpack(&info, &z, &mut []), Ok(5000));
}

#[test]
fn zero_range_and_version_1() {
    let z = zimage(5000);
    let mut info = info_for(&z);
    info.zero_offset = 5000;
    info.zero_bytes = (1 << 22) - 5000;
    assert_eq!(parse_header(&header(&info), &LIMITS), Ok(info));
    // Version 1: bytes 96..112 are not a zero range, and no LZ4.
    let mut h = header(&info);
    h[8] = 1;
    let c = crc32::crc32(&h[..508]);
    h[508..].copy_from_slice(&c.to_le_bytes());
    let v1 = parse_header(&h, &LIMITS).unwrap();
    assert_eq!((v1.zero_offset, v1.zero_bytes, v1.version, v1.count), (0, 0, 1, 0));
    h[16] = COMPRESSION_LZ4 as u8;
    let c = crc32::crc32(&h[..508]);
    h[508..].copy_from_slice(&c.to_le_bytes());
    assert_eq!(parse_header(&h, &LIMITS), Err(Bs2Error::BadCompression));
}

#[test]
fn header_refusals() {
    let z = zimage(5000);
    let info = info_for(&z);
    let h = header(&info);
    let mut bad = h;
    bad[0] = b'X';
    assert_eq!(parse_header(&bad, &LIMITS), Err(Bs2Error::BadMagic));
    let mut bad = h;
    bad[100] = 1;
    assert_eq!(parse_header(&bad, &LIMITS), Err(Bs2Error::BadHeaderCrc));
    assert_eq!(parse_header(&h[..511], &LIMITS), Err(Bs2Error::BadArgument));
    let refuse = |f: &dyn Fn(&mut Bs2Info, &mut [u8; 512]), want: Bs2Error| {
        let mut i = info;
        let mut hh = [0u8; 512];
        f(&mut i, &mut hh);
        let mut out = header(&i);
        if hh[8] != 0 {
            out[8..16].copy_from_slice(&hh[8..16]);
            let c = crc32::crc32(&out[..508]);
            out[508..].copy_from_slice(&c.to_le_bytes());
        }
        assert_eq!(parse_header(&out, &LIMITS), Err(want));
    };
    // Version 3 is another layout: this header under that number is refused
    // (its bytes 48..64, sizes and CRCs here, are reserved there).
    refuse(&|_, hh| hh[8..16].copy_from_slice(&[3, 0, 0, 0, 0, 2, 0, 0]), Bs2Error::BadVersion);
    refuse(&|_, hh| hh[8..16].copy_from_slice(&[4, 0, 0, 0, 0, 2, 0, 0]), Bs2Error::BadVersion);
    refuse(&|_, hh| hh[8..16].copy_from_slice(&[2, 0, 0, 0, 0, 1, 0, 0]), Bs2Error::BadVersion);
    refuse(&|i, _| i.compression = 3, Bs2Error::BadCompression);
    refuse(
        &|i, _| {
            i.zero_offset = 4999;
            i.zero_bytes = 100;
        },
        Bs2Error::ZeroRange,
    );
    refuse(
        &|i, _| {
            i.zero_offset = 5000;
            i.zero_bytes = (1 << 22) - 4999;
        },
        Bs2Error::ZeroRange,
    );
    refuse(
        &|i, _| {
            i.zero_offset = u64::MAX;
            i.zero_bytes = 2;
        },
        Bs2Error::ZeroRange,
    );
    refuse(&|i, _| i.load = 0x8080_0000, Bs2Error::WrongLoad);
    refuse(&|i, _| i.entry = 0x8200_0000 + 5000, Bs2Error::EntryOutside);
    refuse(&|i, _| i.entry = 0x81ff_fffc, Bs2Error::EntryOutside);
    refuse(
        &|i, _| {
            i.stored_bytes = (1 << 20) + 1;
            i.image_bytes = (1 << 20) + 1;
        },
        Bs2Error::StoredTooBig,
    );
    refuse(
        &|i, _| {
            i.compression = COMPRESSION_DEFLATE;
            i.image_bytes = (1 << 22) + 1;
        },
        Bs2Error::ImageTooBig,
    );
    refuse(&|i, _| i.image_bytes = 4999, Bs2Error::SizesDisagree);
    refuse(&|i, _| i.stored_bytes = 0, Bs2Error::SizesDisagree);
}

#[test]
fn unpack_refusals() {
    let z = zimage(5000);
    let info = info_for(&z);
    let mut flipped = z.clone();
    flipped[4000] ^= 0x10;
    assert_eq!(unpack(&info, &flipped, &mut []), Err(Bs2Error::StoredCrc));
    let mut i = info;
    i.image_crc32 ^= 1;
    assert_eq!(unpack(&i, &z, &mut []), Err(Bs2Error::ImageCrc));
    let plain = vec![0u8; 5000];
    let i = info_for(&plain);
    assert_eq!(unpack(&i, &plain, &mut []), Err(Bs2Error::NotArm64Image));
}

#[test]
fn strerror_names_every_code() {
    for code in -23..=-1 {
        let e = Bs2Error::from_code(code).expect("every code has a variant");
        assert!(e.text().ends_with('\0') && e.text().len() > 5);
    }
    assert!(Bs2Error::from_code(0).is_none());
    assert!(Bs2Error::from_code(-24).is_none());
}

/* ---- version 3: the segment table ---- */

/// An arm64 Image of `len` bytes whose header says it owns `image_size`
/// bytes of RAM (its .bss included).
fn zimage_sized(len: usize, image_size: u64) -> Vec<u8> {
    let mut z = zimage(len);
    z[16..24].copy_from_slice(&image_size.to_le_bytes());
    z
}

fn name(s: &str) -> [u8; NAME_BYTES + 1] {
    let mut n = [0u8; NAME_BYTES + 1];
    n[..s.len()].copy_from_slice(s.as_bytes());
    n
}

fn seal(h: &mut [u8; 512]) {
    let c = crc32::crc32(&h[..508]);
    h[508..].copy_from_slice(&c.to_le_bytes());
}

/// A version-3 Bs2Info as parse_header returns it.
fn info_v3(segments: [Bs2Segment; MAX_SEGMENTS], count: u32, stored_bytes: u64, nm: &str) -> Bs2Info {
    Bs2Info {
        load: segments[0].load,
        entry: segments[0].load,
        stored_bytes,
        image_bytes: 0,
        compression: 0,
        stored_crc32: 0,
        image_crc32: 0,
        name: name(nm),
        zero_offset: 0,
        zero_bytes: 0,
        version: VERSION_SEGMENTS,
        count,
        segments,
    }
}

/// A version-3 header from a Bs2Info with `parts` image parts, without any
/// check (mkboot's layout; the BS2_VECTORS test checks the Python writer
/// against the same one).
fn header_v3(info: &Bs2Info, parts: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..8].copy_from_slice(&MAGIC);
    h[8..12].copy_from_slice(&VERSION_SEGMENTS.to_le_bytes());
    h[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    h[16..20].copy_from_slice(&info.count.to_le_bytes());
    h[20..24].copy_from_slice(&parts.to_le_bytes());
    h[24..32].copy_from_slice(&info.load.to_le_bytes());
    h[32..40].copy_from_slice(&info.entry.to_le_bytes());
    h[40..48].copy_from_slice(&info.stored_bytes.to_le_bytes());
    h[64..96].copy_from_slice(&info.name[..32]);
    for (i, s) in info.segments.iter().enumerate().take(info.count.min(MAX_SEGMENTS as u32) as usize) {
        let e = &mut h[SEGMENT_TABLE + i * SEGMENT_ENTRY_BYTES..][..SEGMENT_ENTRY_BYTES];
        e[0..8].copy_from_slice(&s.load.to_le_bytes());
        e[8..12].copy_from_slice(&(s.image_bytes as u32).to_le_bytes());
        e[12..16].copy_from_slice(&(s.footprint as u32).to_le_bytes());
        e[16..20].copy_from_slice(&(s.stored_offset as u32).to_le_bytes());
        e[20..24].copy_from_slice(&(s.stored_bytes as u32).to_le_bytes());
        e[24..28].copy_from_slice(&s.compression.to_le_bytes());
        e[28..32].copy_from_slice(&s.stored_crc32.to_le_bytes());
        e[32..36].copy_from_slice(&s.image_crc32.to_le_bytes());
        e[36..40].copy_from_slice(&s.zero_bytes.to_le_bytes());
    }
    seal(&mut h);
    h
}

fn deflate_stored(data: &[u8]) -> Vec<u8> {
    // One stored block per 65535 bytes: a valid raw-deflate stream that
    // needs no compressor (inflate's own tests cover the Huffman paths).
    let mut out = Vec::new();
    let chunks: Vec<&[u8]> = data.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        out.push(if i + 1 == chunks.len() { 1 } else { 0 });
        out.extend_from_slice(&(c.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        out.extend_from_slice(c);
    }
    out
}

fn lz4_stored(data: &[u8]) -> Vec<u8> {
    // One LZ4 frame of uncompressed blocks (lz4.rs's own tests cover the
    // sequences): magic, FLG (version 01, independent blocks), BD (4 MB),
    // the header checksum byte (not verified), the blocks, the end mark.
    let mut out = vec![0x04, 0x22, 0x4D, 0x18, 0x60, 0x70, 0x00];
    for c in data.chunks(1 << 22) {
        out.extend_from_slice(&(c.len() as u32 | 0x8000_0000).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

fn pack(data: &[u8], comp: u32) -> Vec<u8> {
    match comp {
        COMPRESSION_DEFLATE => deflate_stored(data),
        COMPRESSION_LZ4 => lz4_stored(data),
        _ => data.to_vec(),
    }
}

/// A payload of segments (data, load, compression): header + stored bytes,
/// in the layout mkboot writes (each segment's stored bytes sector aligned).
struct Payload {
    info: Bs2Info,
    file: Vec<u8>,
    images: Vec<Vec<u8>>,
}

fn payload(segs: &[(&[u8], u64, u32)]) -> Payload {
    let mut segments = [Bs2Segment::default(); MAX_SEGMENTS];
    let mut stored_all = Vec::new();
    for (i, (data, load, comp)) in segs.iter().enumerate() {
        let stored = pack(data, *comp);
        let footprint = if i == 0 { le64(data, 16).max(data.len() as u64) } else { data.len() as u64 };
        segments[i] = Bs2Segment {
            load: *load,
            image_bytes: data.len() as u64,
            footprint,
            stored_offset: stored_all.len() as u64,
            stored_bytes: stored.len() as u64,
            compression: *comp,
            stored_crc32: crc32::crc32(&stored),
            image_crc32: crc32::crc32(data),
            zero_bytes: 0,
        };
        stored_all.extend_from_slice(&stored);
        stored_all.resize(stored_all.len().div_ceil(512) * 512, 0);
    }
    let info = info_v3(segments, segs.len() as u32, stored_all.len() as u64, "multi");
    let mut file = header_v3(&info, 1).to_vec();
    file.extend_from_slice(&stored_all);
    Payload { info, file, images: segs.iter().map(|s| s.0.to_vec()).collect() }
}

/// Parse a payload of any version and produce its images in host buffers:
/// the one image of a version 1 or 2 payload, every segment of a version-3
/// one in table order.
fn load_all(file: &[u8], limits: &Bs2Limits) -> Result<Vec<Vec<u8>>, Bs2Error> {
    let info = parse_header(file, limits)?;
    let body = &file[HEADER_BYTES..];
    if info.version < VERSION_SEGMENTS {
        let stored = body.get(..info.stored_bytes as usize).ok_or(Bs2Error::BadArgument)?;
        let mut image = vec![0u8; info.image_bytes as usize];
        unpack(&info, stored, &mut image)?;
        if info.compression == COMPRESSION_NONE {
            image.copy_from_slice(stored);
        }
        return Ok(vec![image]);
    }
    let mut out = Vec::new();
    for i in 0..info.count as usize {
        let s = &info.segments[i];
        let stored = body
            .get(s.stored_offset as usize..(s.stored_offset + s.stored_bytes) as usize)
            .ok_or(Bs2Error::BadArgument)?;
        let mut image = vec![0u8; s.image_bytes as usize];
        unpack_segment(&info, i, stored, Some(&mut image))?;
        out.push(image);
    }
    Ok(out)
}

#[test]
fn v3_three_segments() {
    // The showcase's shape: an image with .bss past its file, then two
    // blobs above it (the R5F's DDR image and TCM blob).
    let sup = zimage_sized(70_000, 0x40_0000);
    let ddr: Vec<u8> = (0..30_000u32).map(|i| (i * 7 + 3) as u8).collect();
    let tcm: Vec<u8> = (0..288u32).map(|i| i as u8).collect();
    for comp in [COMPRESSION_NONE, COMPRESSION_DEFLATE, COMPRESSION_LZ4] {
        let p = payload(&[(&sup, LOAD, comp), (&ddr, 0x8e00_0000, comp), (&tcm, 0x8ec0_0000, comp)]);
        let info = parse_header(&p.file, &AM62P).expect("a valid three-segment payload");
        assert_eq!(info, p.info, "what the header says, nothing else");
        assert_eq!((info.count, info.version, info.load, info.entry), (3, 3, LOAD, LOAD));
        assert_eq!(info.segments[0].footprint, 0x40_0000);
        assert_eq!(load_all(&p.file, &AM62P), Ok(p.images.clone()), "compression {comp}");
    }
    // each segment its own compression
    let p = payload(&[
        (&sup, LOAD, COMPRESSION_LZ4),
        (&ddr, 0x8e00_0000, COMPRESSION_DEFLATE),
        (&tcm, 0x8ec0_0000, COMPRESSION_NONE),
    ]);
    assert_eq!(load_all(&p.file, &AM62P), Ok(p.images.clone()));
}

#[test]
fn v3_single_segment() {
    let z = zimage_sized(9000, 9000);
    let p = payload(&[(&z, LOAD, COMPRESSION_DEFLATE)]);
    assert_eq!(load_all(&p.file, &AM62P), Ok(vec![z.clone()]));
    // In place (the eMMC source reads an uncompressed segment straight to
    // its load address) and copied (the payload linked into the loader).
    let p = payload(&[(&z, LOAD, COMPRESSION_NONE)]);
    assert_eq!(unpack_segment(&p.info, 0, &z, None), Ok(9000));
    let mut copy = vec![0u8; 9000];
    assert_eq!(unpack_segment(&p.info, 0, &z, Some(&mut copy)), Ok(9000));
    assert_eq!(copy, z, "an uncompressed segment is copied to its destination");
}

#[test]
fn versions_do_not_cross() {
    let z = zimage_sized(5000, 5000);
    // The single-image unpack refuses a version-3 header's info and the
    // segment unpack a version 1 or 2 one.
    let p = payload(&[(&z, LOAD, COMPRESSION_NONE)]);
    assert_eq!(unpack(&p.info, &z, &mut []), Err(Bs2Error::BadArgument));
    let single = info_for(&z);
    assert_eq!(unpack_segment(&single, 0, &z, None), Err(Bs2Error::BadArgument));
    // A version-3 header under the number 1 or 2: zero where a single image
    // has its image size (48, reserved here), so the sizes disagree, or the
    // count at 16 is no compression.
    for (count_as_compression, want) in [(1u32, Bs2Error::SizesDisagree), (3, Bs2Error::BadCompression)] {
        for version in [1u8, 2] {
            let mut i = p.info;
            i.count = count_as_compression;
            let mut h = header_v3(&i, 1);
            h[8] = version;
            seal(&mut h);
            assert_eq!(parse_header(&h, &AM62P), Err(want), "version {version}");
        }
    }
    // The table as a work branch wrote it under the number 2 (entry at 24,
    // stored bytes at 32, bytes 40..64 zero): read as a single image with no
    // stored bytes.
    let mut h = [0u8; 512];
    h[..8].copy_from_slice(&MAGIC);
    h[8..12].copy_from_slice(&2u32.to_le_bytes());
    h[12..16].copy_from_slice(&512u32.to_le_bytes());
    h[16..20].copy_from_slice(&1u32.to_le_bytes());
    h[24..32].copy_from_slice(&LOAD.to_le_bytes());
    h[32..40].copy_from_slice(&5120u64.to_le_bytes());
    h[SEGMENT_TABLE..SEGMENT_TABLE + 8].copy_from_slice(&LOAD.to_le_bytes());
    seal(&mut h);
    assert_eq!(parse_header(&h, &AM62P), Err(Bs2Error::SizesDisagree));
}

#[test]
fn v3_refusals() {
    let sup = zimage_sized(70_000, 0x40_0000);
    let ddr = vec![0x5au8; 30_000];
    let tcm = vec![0xa5u8; 288];
    let good =
        payload(&[(&sup, LOAD, COMPRESSION_NONE), (&ddr, 0x8e00_0000, COMPRESSION_NONE), (&tcm, 0x8ec0_0000, 0)]);
    assert!(parse_header(&good.file, &AM62P).is_ok());
    let refuse = |what: &str, f: &dyn Fn(&mut Bs2Info), want: Bs2Error| {
        let mut i = good.info;
        f(&mut i);
        assert_eq!(parse_header(&header_v3(&i, 1), &AM62P), Err(want), "{what}");
    };
    // malformed
    refuse("no segments", &|i| i.count = 0, Bs2Error::SegmentCount);
    refuse("nine segments", &|i| i.count = 9, Bs2Error::SegmentCount);
    refuse("stored total not whole sectors", &|i| i.stored_bytes -= 1, Bs2Error::SegmentLayout);
    refuse("a segment past the payload", &|i| i.stored_bytes -= 512, Bs2Error::SegmentLayout);
    refuse("an unaligned stored offset", &|i| i.segments[1].stored_offset += 4, Bs2Error::SegmentLayout);
    refuse("stored ranges out of order", &|i| i.segments.swap(1, 2), Bs2Error::SegmentLayout);
    refuse(
        "stored ranges overlapping",
        &|i| i.segments[2].stored_offset = i.segments[1].stored_offset,
        Bs2Error::SegmentLayout,
    );
    refuse("zero bytes over the footprint", &|i| i.segments[1].zero_bytes = 1, Bs2Error::ZeroRange);
    refuse(
        "zero bytes one over segment 0's .bss",
        &|i| i.segments[0].zero_bytes = 0x40_0000 - 70_000 + 1,
        Bs2Error::ZeroRange,
    );
    refuse("an unknown compression", &|i| i.segments[2].compression = 3, Bs2Error::BadCompression);
    refuse("a zero-length segment", &|i| i.segments[2].image_bytes = 0, Bs2Error::SizesDisagree);
    refuse("uncompressed with stored != image", &|i| i.segments[2].image_bytes = 287, Bs2Error::SizesDisagree);
    refuse("footprint below the image", &|i| i.segments[1].footprint = 100, Bs2Error::SizesDisagree);
    refuse("payload over the source", &|i| i.stored_bytes = AM62P.max_stored + 512, Bs2Error::StoredTooBig);
    refuse("segment 0 not at the load address", &|i| i.segments[0].load = 0x8300_0000, Bs2Error::WrongLoad);
    refuse("the header's load address is not segment 0's", &|i| i.load = 0x8300_0000, Bs2Error::WrongLoad);
    refuse("entry past segment 0's image", &|i| i.entry = LOAD + 70_000, Bs2Error::EntryOutside);
    refuse("entry below the load address", &|i| i.entry = LOAD - 4, Bs2Error::EntryOutside);
    refuse("entry in another segment", &|i| i.entry = 0x8e00_0000, Bs2Error::EntryOutside);
    // overlapping
    refuse("a blob inside segment 0's .bss", &|i| i.segments[1].load = LOAD + 0x20_0000, Bs2Error::SegmentOverlap);
    refuse("two blobs on one byte", &|i| i.segments[2].load = 0x8e00_0000 + 30_000 - 1, Bs2Error::SegmentOverlap);
    refuse("a blob on the staging buffer", &|i| i.segments[2].load = 0x9000_0000, Bs2Error::SegmentOverlap);
    refuse(
        "a blob reaching into the staging buffer",
        &|i| i.segments[1].load = 0x9000_0000 - 29_999,
        Bs2Error::SegmentOverlap,
    );
    // out of RAM
    refuse("below the region", &|i| i.segments[1].load = 0x8000_0000, Bs2Error::SegmentOutside);
    refuse("in the GPU carveout", &|i| i.segments[2].load = 0x9780_0000, Bs2Error::SegmentOutside);
    refuse("ending one byte past the region", &|i| i.segments[2].load = 0x9780_0000 - 287, Bs2Error::SegmentOutside);
    refuse("wrapping the address space", &|i| i.segments[2].load = u64::MAX - 10, Bs2Error::SegmentOutside);
    // the last byte of the region is still in it
    let mut i = good.info;
    i.segments[2].load = 0x9780_0000 - 288;
    assert!(parse_header(&header_v3(&i, 1), &AM62P).is_ok());
    // without a staging buffer (the linked payload) the staging range is free
    let mut i = good.info;
    i.segments[2].load = 0x9000_0000;
    let embedded = Bs2Limits { staging_bytes: 0, ..AM62P };
    assert!(parse_header(&header_v3(&i, 1), &embedded).is_ok());
    // no image parts at all
    assert_eq!(parse_header(&header_v3(&good.info, 0), &AM62P), Err(Bs2Error::ImageParts));
    // a non-zero byte in each reserved range, and in an unused table entry
    for at in [48, 63, 416, 450, 507, SEGMENT_TABLE + 5 * SEGMENT_ENTRY_BYTES] {
        let mut h = header_v3(&good.info, 1);
        h[at] = 1;
        seal(&mut h);
        assert_eq!(parse_header(&h, &AM62P), Err(Bs2Error::BadVersion), "byte {at}");
    }
    // the header size
    let mut h = header_v3(&good.info, 1);
    h[12] = 1;
    seal(&mut h);
    assert_eq!(parse_header(&h, &AM62P), Err(Bs2Error::BadVersion));
}

#[test]
fn v3_unpack_refusals() {
    let sup = zimage_sized(70_000, 0x40_0000);
    let ddr = vec![0x5au8; 30_000];
    let p = payload(&[(&sup, LOAD, COMPRESSION_DEFLATE), (&ddr, 0x8e00_0000, COMPRESSION_DEFLATE)]);
    // a flipped stored byte in segment 1
    let mut f = p.file.clone();
    let off = 512 + p.info.segments[1].stored_offset as usize + 100;
    f[off] ^= 1;
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::StoredCrc));
    // a broken stream whose stored CRC matches
    let mut i = p.info;
    let mut f = p.file.clone();
    let s1 = 512 + i.segments[1].stored_offset as usize;
    f[s1] = 0x07; // block type 3
    i.segments[1].stored_crc32 = crc32::crc32(&f[s1..s1 + i.segments[1].stored_bytes as usize]);
    f[..512].copy_from_slice(&header_v3(&i, 1));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Inflate));
    // the same for an LZ4 segment: no frame magic
    let q = payload(&[(&sup, LOAD, COMPRESSION_LZ4)]);
    let mut i = q.info;
    let mut f = q.file.clone();
    f[512] ^= 1;
    i.segments[0].stored_crc32 = crc32::crc32(&f[512..512 + i.segments[0].stored_bytes as usize]);
    f[..512].copy_from_slice(&header_v3(&i, 1));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Lz4));
    // the image CRC
    let mut i = p.info;
    i.segments[1].image_crc32 ^= 1;
    let mut f = p.file.clone();
    f[..512].copy_from_slice(&header_v3(&i, 1));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::ImageCrc));
    // a decoded length that is not the header's
    let mut i = p.info;
    i.segments[1].image_bytes += 1;
    i.segments[1].footprint += 1;
    let mut f = p.file.clone();
    f[..512].copy_from_slice(&header_v3(&i, 1));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::LengthMismatch));
    // segment 0's .bss larger than the footprint the overlap check used
    let mut i = p.info;
    i.segments[0].footprint = 0x20_0000;
    let mut f = p.file.clone();
    f[..512].copy_from_slice(&header_v3(&i, 1));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Footprint));
    // segment 0 is not an arm64 Image
    let plain = vec![0u8; 4096];
    let q = payload(&[(&plain, LOAD, COMPRESSION_NONE), (&ddr, 0x8e00_0000, COMPRESSION_NONE)]);
    assert_eq!(load_all(&q.file, &AM62P), Err(Bs2Error::NotArm64Image));
    // other segments need no format
    let z = zimage_sized(4096, 4096);
    let q = payload(&[(&z, LOAD, COMPRESSION_NONE), (&plain, 0x8e00_0000, COMPRESSION_NONE)]);
    assert!(load_all(&q.file, &AM62P).is_ok());
    // the arguments
    let q = payload(&[(&z, LOAD, COMPRESSION_NONE)]);
    assert_eq!(unpack_segment(&q.info, 1, &z, None), Err(Bs2Error::BadArgument), "index past the count");
    assert_eq!(unpack_segment(&q.info, 0, &z[..4095], None), Err(Bs2Error::BadArgument), "short stored");
    let mut small = vec![0u8; 4095];
    assert_eq!(unpack_segment(&q.info, 0, &z, Some(&mut small)), Err(Bs2Error::BadArgument), "short image");
    // a compressed segment cannot be "in place"
    for comp in [COMPRESSION_DEFLATE, COMPRESSION_LZ4] {
        let q = payload(&[(&z, LOAD, comp)]);
        let stored = &q.file[512..512 + q.info.segments[0].stored_bytes as usize];
        assert_eq!(unpack_segment(&q.info, 0, stored, None), Err(Bs2Error::BadArgument));
    }
}

/// An image of `len` bytes cut into parts at the given (offset, length)
/// zero runs, the way `am62p-mkboot.py stage2img --split-zeros` cuts one:
/// segment 0 and one continuation part after each run, the last part with
/// `clear_after` zero bytes past the image's last byte. Then `extra`
/// segments. Returns the payload and the image.
fn split_payload(len: usize, runs: &[(usize, usize)], clear_after: u32, extra: &[(&[u8], u64)]) -> (Payload, Vec<u8>) {
    let mut z = zimage_sized(len, 0x40_0000);
    for &(at, n) in runs {
        z[at..at + n].fill(0);
    }
    let mut segments = [Bs2Segment::default(); MAX_SEGMENTS];
    let mut stored_all = Vec::new();
    let mut images = Vec::new();
    let mut start = 0;
    let mut count = 0;
    let mut push = |data: &[u8], load: u64, footprint: u64, zero: u32| {
        segments[count] = Bs2Segment {
            load,
            image_bytes: data.len() as u64,
            footprint,
            stored_offset: stored_all.len() as u64,
            stored_bytes: data.len() as u64,
            compression: COMPRESSION_NONE,
            stored_crc32: crc32::crc32(data),
            image_crc32: crc32::crc32(data),
            zero_bytes: zero,
        };
        stored_all.extend_from_slice(data);
        stored_all.resize(stored_all.len().div_ceil(512) * 512, 0);
        images.push(data.to_vec());
        count += 1;
    };
    for k in 0..=runs.len() {
        let (end, zero) = match runs.get(k) {
            Some(&(at, n)) => (at, n as u32),
            None => (len, clear_after),
        };
        let data = &z[start..end];
        let footprint = if k == 0 { 0x40_0000 } else { data.len() as u64 + zero as u64 };
        push(data, LOAD + start as u64, footprint, zero);
        start = end + zero as usize;
    }
    for (data, load) in extra {
        push(data, *load, data.len() as u64, 0);
    }
    let info = info_v3(segments, count as u32, stored_all.len() as u64, "split");
    let mut file = header_v3(&info, runs.len() as u32 + 1).to_vec();
    file.extend_from_slice(&stored_all);
    (Payload { info, file, images }, z)
}

/// What the loader leaves in RAM for the image: every part at its address
/// over a region of 0xEE bytes, its zero bytes cleared.
fn reassemble(file: &[u8], limits: &Bs2Limits, parts: usize, len: usize) -> Vec<u8> {
    let info = parse_header(file, limits).expect("a valid payload");
    let images = load_all(file, limits).expect("every segment loads");
    let mut ram = vec![0xEEu8; len];
    for (s, image) in info.segments.iter().zip(&images).take(parts) {
        let at = (s.load - limits.load) as usize;
        ram[at..at + image.len()].copy_from_slice(image);
        ram[at + image.len()..at + image.len() + s.zero_bytes as usize].fill(0);
    }
    ram
}

#[test]
fn v3_zero_bytes() {
    // One part: the image, then its .bss cleared by the loader. Up to the
    // footprint's last byte is accepted.
    let (p, z) = split_payload(70_000, &[], 0x40_0000 - 70_000, &[]);
    let info = parse_header(&p.file, &AM62P).expect("zero bytes up to the footprint");
    assert_eq!((info.count, info.segments[0].zero_bytes), (1, 0x40_0000 - 70_000));
    let ram = reassemble(&p.file, &AM62P, 1, 0x40_0000);
    assert_eq!(&ram[..70_000], &z[..]);
    assert!(ram[70_000..].iter().all(|&b| b == 0));
    // a blob's zero bytes inside a footprint larger than its image
    let blob = vec![0x5au8; 3000];
    let (q, _) = split_payload(70_000, &[], 0, &[(&blob, 0x8e00_0000)]);
    let mut i = q.info;
    i.segments[1].footprint = 8192;
    i.segments[1].zero_bytes = 8192 - 3000;
    assert!(parse_header(&header_v3(&i, 1), &AM62P).is_ok());
    i.segments[1].zero_bytes += 1;
    assert_eq!(parse_header(&header_v3(&i, 1), &AM62P), Err(Bs2Error::ZeroRange));
}

#[test]
fn v3_image_parts() {
    // Three parts (two zero runs), a clear-after, and the showcase's two
    // blobs above the image.
    let ddr: Vec<u8> = (0..30_000u32).map(|i| (i * 7 + 3) as u8).collect();
    let tcm: Vec<u8> = (0..288u32).map(|i| i as u8).collect();
    let runs = [(8192, 16_384), (40_960, 12_288)];
    let (p, z) = split_payload(70_000, &runs, 100_000, &[(&ddr, 0x8e00_0000), (&tcm, 0x8ec0_0000)]);
    let info = parse_header(&p.file, &AM62P).expect("a valid split payload");
    assert_eq!(info.count, 5);
    assert_eq!(info.segments[0].footprint, 0x40_0000);
    assert_eq!(
        info.segments[..3].iter().map(|s| (s.load - LOAD, s.image_bytes, s.zero_bytes)).collect::<Vec<_>>(),
        [(0, 8192, 16_384), (24_576, 16_384, 12_288), (53_248, 16_752, 100_000)]
    );
    assert_eq!(load_all(&p.file, &AM62P), Ok(p.images.clone()));
    let ram = reassemble(&p.file, &AM62P, 3, 170_000);
    assert_eq!(&ram[..70_000], &z[..], "the parts and their zero runs are the image");
    assert!(ram[70_000..].iter().all(|&b| b == 0), "the clear-after range");
    // the stored payload does not hold the zero runs
    assert!(info.stored_bytes < 70_000 - 16_384 - 12_288 + 30_000 + 288 + 5 * 512);

    let refuse = |what: &str, parts: u32, f: &dyn Fn(&mut Bs2Info), want: Bs2Error| {
        let mut i = p.info;
        f(&mut i);
        assert_eq!(parse_header(&header_v3(&i, parts), &AM62P), Err(want), "{what}");
    };
    let accept = |what: &str, parts: u32, f: &dyn Fn(&mut Bs2Info)| {
        let mut i = p.info;
        f(&mut i);
        assert!(parse_header(&header_v3(&i, parts), &AM62P).is_ok(), "{what}");
    };
    refuse("more parts than segments", 6, &|_| {}, Bs2Error::ImageParts);
    refuse("no parts", 0, &|_| {}, Bs2Error::ImageParts);
    accept("every segment a part needs them all inside segment 0", 3, &|i| i.count = 3);
    refuse("a blob counted as a part", 4, &|_| {}, Bs2Error::ImageParts);
    // Fewer parts than the table holds: the continuation is then an
    // ordinary segment inside segment 0's footprint.
    refuse("a part not counted", 2, &|_| {}, Bs2Error::SegmentOverlap);
    refuse("the parts read as one", 1, &|_| {}, Bs2Error::SegmentOverlap);
    refuse("a part on segment 0's image", 3, &|i| i.segments[1].load = LOAD + 8191, Bs2Error::ImageParts);
    refuse("a part on segment 0's zero bytes", 3, &|i| i.segments[1].load = LOAD + 24_575, Bs2Error::ImageParts);
    refuse("a part on segment 0's longer zero bytes", 3, &|i| i.segments[0].zero_bytes += 1, Bs2Error::ImageParts);
    refuse(
        "a part ending past segment 0's footprint",
        3,
        &|i| i.segments[2].load = LOAD + 0x40_0000 - 116_751,
        Bs2Error::ImageParts,
    );
    accept("a part ending on segment 0's last byte", 3, &|i| i.segments[2].load = LOAD + 0x40_0000 - 116_752);
    refuse("a part above segment 0's footprint", 3, &|i| i.segments[2].load = 0x8d00_0000, Bs2Error::ImageParts);
    refuse(
        "parts in descending order",
        3,
        &|i| {
            let (a, b) = (i.segments[1].load, i.segments[2].load);
            i.segments[1].load = b + 0x10_0000;
            i.segments[2].load = a + 0x10_0000;
        },
        Bs2Error::ImageParts,
    );
    refuse("two parts at one address", 3, &|i| i.segments[2].load = i.segments[1].load, Bs2Error::ImageParts);
    refuse("a part with room after its zero bytes", 3, &|i| i.segments[1].footprint += 1, Bs2Error::ImageParts);
    refuse("a part's zero bytes over its footprint", 3, &|i| i.segments[1].zero_bytes += 1, Bs2Error::ZeroRange);
    refuse(
        "a part whose zero bytes reach the next part",
        3,
        &|i| {
            i.segments[1].zero_bytes += 1;
            i.segments[1].footprint += 1;
        },
        Bs2Error::SegmentOverlap,
    );
    refuse("a part reaching into the next part", 3, &|i| i.segments[1].load += 4096, Bs2Error::SegmentOverlap);
    // A segment that is not a part is still checked against segment 0's
    // whole footprint, and against the parts.
    refuse("a blob in segment 0's footprint", 3, &|i| i.segments[3].load = LOAD + 0x30_0000, Bs2Error::SegmentOverlap);
    refuse("a blob on a part", 3, &|i| i.segments[4].load = LOAD + 24_576 + 100, Bs2Error::SegmentOverlap);
    refuse("a blob on a part's zero bytes", 3, &|i| i.segments[4].load = LOAD + 45_000, Bs2Error::SegmentOverlap);
    refuse("a blob on the clear-after range", 3, &|i| i.segments[4].load = LOAD + 150_000, Bs2Error::SegmentOverlap);
    // segment 0 is checked against the staging buffer with its footprint
    let low = Bs2Limits { staging: LOAD + 0x20_0000, ..AM62P };
    assert_eq!(parse_header(&p.file, &low), Err(Bs2Error::SegmentOverlap), "staging in segment 0's .bss");
    // the entry stays in segment 0's image bytes, not in a part
    refuse("entry in a part", 3, &|i| i.entry = LOAD + 24_576, Bs2Error::EntryOutside);
    refuse("entry in segment 0's zero bytes", 3, &|i| i.entry = LOAD + 8192, Bs2Error::EntryOutside);
    // segment 0 is still the arm64 Image whose image_size fits the footprint
    let mut i = p.info;
    i.segments[0].footprint = 0x20_0000;
    let mut f = p.file.clone();
    f[..512].copy_from_slice(&header_v3(&i, 3));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Footprint));
}

/// The images `tools/am62p-mkboot.py stage2-vectors <dir>` writes (Python's
/// zlib and the lz4 command as the compressors, so this is the cross-check
/// of the two sides): `ok-*.img` must load, `bad-*.img` must be refused, a
/// `bad-v3-*.img` for the reason its name gives. Without BS2_VECTORS the
/// test says so and checks nothing; tools/check.sh sets it and requires the
/// "verified" line.
#[test]
fn mkboot_vectors() {
    let Some(dir) = std::env::var_os("BS2_VECTORS") else {
        println!("mkboot_vectors: BS2_VECTORS not set, nothing checked");
        return;
    };
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .expect("BS2_VECTORS is a directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "img"))
        .collect();
    names.sort();
    let (mut ok, mut bad, mut multi, mut zeroed) = (0, 0, 0, 0);
    for p in &names {
        let file = std::fs::read(p).unwrap();
        let stem = p.file_name().unwrap().to_string_lossy().into_owned();
        let result = load_all(&file, &AM62P);
        if stem.starts_with("ok-") {
            let images = result.unwrap_or_else(|e| panic!("{stem}: {e:?}"));
            let info = parse_header(&file, &AM62P).unwrap();
            assert_eq!(info.version >= VERSION_SEGMENTS, stem.starts_with("ok-v3-"), "{stem}: its header version");
            if images.len() > 1 {
                multi += 1;
            }
            if info.zero_bytes != 0 || info.segments.iter().any(|s| s.zero_bytes != 0) {
                zeroed += 1;
            }
            ok += 1;
        } else {
            assert!(result.is_err(), "{stem} was accepted");
            // The version-3 refusals name the rule they break.
            let want = match stem.as_str() {
                "bad-v3-overlap-bss.img" | "bad-v3-overlap-blobs.img" | "bad-v3-on-staging.img" => {
                    Some(Bs2Error::SegmentOverlap)
                }
                "bad-v3-outside-ram.img" | "bad-v3-below-ram.img" => Some(Bs2Error::SegmentOutside),
                "bad-v3-unaligned.img" | "bad-v3-past-payload.img" => Some(Bs2Error::SegmentLayout),
                "bad-v3-no-segments.img" => Some(Bs2Error::SegmentCount),
                "bad-v3-footprint.img" => Some(Bs2Error::Footprint),
                "bad-v3-segment-crc.img" => Some(Bs2Error::StoredCrc),
                "bad-v3-zero-over-footprint.img" => Some(Bs2Error::ZeroRange),
                "bad-v3-no-parts.img"
                | "bad-v3-parts-over-count.img"
                | "bad-v3-part-outside.img"
                | "bad-v3-part-on-occupied.img"
                | "bad-v3-parts-descending.img" => Some(Bs2Error::ImageParts),
                "bad-v3-blob-in-footprint.img" => Some(Bs2Error::SegmentOverlap),
                "bad-v3-load-field.img" => Some(Bs2Error::WrongLoad),
                "bad-v3-as-version-2.img" => Some(Bs2Error::SizesDisagree),
                "bad-v3-lz4.img" => Some(Bs2Error::Lz4),
                s if s.starts_with("bad-v3-") => panic!("{s}: no expected refusal listed"),
                _ => None,
            };
            if let Some(w) = want {
                assert_eq!(result, Err(w), "{stem}");
            }
            bad += 1;
        }
    }
    assert!(
        ok >= 3 && bad >= 3 && multi >= 1 && zeroed >= 3,
        "too few vectors in {dir:?}: {ok} ok ({multi} multi-segment, {zeroed} with a zero range), {bad} bad"
    );
    println!(
        "mkboot_vectors: {} images verified ({ok} loaded, {multi} of them multi-segment, {zeroed} with a zero range, {bad} refused)",
        ok + bad
    );
}

/// A payload as it goes to a board, through the loader's own checks, under
/// the limits that board's loader is built with: BS2_PAYLOAD=<stage2 .img>,
/// BS2_PAYLOAD_ZEPHYR=<the zephyr.bin it was made from>, and
/// BS2_PAYLOAD_LIMITS="<load>,<region end>,<max stored>,<staging>" (hex or
/// decimal; the loader's BOOT_STAGE2_LOAD, _REGION_END, the sector range's
/// bytes and _STAGING). The image is put together again from the segments
/// and their zero bytes (or, versions 1 and 2, taken as it is) and must
/// equal zephyr.bin; what is cleared past the image's last byte is the .bss
/// the loader clears. Without BS2_PAYLOAD the test says so and checks
/// nothing.
#[test]
fn board_payload() {
    let Some(path) = std::env::var_os("BS2_PAYLOAD") else {
        println!("board_payload: BS2_PAYLOAD not set, nothing checked");
        return;
    };
    let num = |s: &str| {
        let s = s.trim();
        match s.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).unwrap(),
            None => s.parse::<u64>().unwrap(),
        }
    };
    let l: Vec<u64> = std::env::var("BS2_PAYLOAD_LIMITS").expect("BS2_PAYLOAD_LIMITS").split(',').map(num).collect();
    // As src/main.c fills them for the eMMC source.
    let limits = Bs2Limits {
        load: l[0],
        max_stored: l[2],
        max_image: l[3] - l[0],
        region_end: l[1],
        staging: l[3],
        staging_bytes: l[2],
    };
    let file = std::fs::read(&path).unwrap();
    let zephyr = std::fs::read(std::env::var_os("BS2_PAYLOAD_ZEPHYR").expect("BS2_PAYLOAD_ZEPHYR")).unwrap();
    let info = parse_header(&file, &limits).unwrap_or_else(|e| panic!("header refused: {e:?}"));
    let images = load_all(&file, &limits).unwrap_or_else(|e| panic!("payload refused: {e:?}"));
    // The loader's memory from the load address: every byte 0xAA until a
    // segment or a zero range writes it.
    let mut ram = vec![0xAAu8; zephyr.len()];
    let mut cleared_after = 0u64;
    let mut place = |at: usize, image: &[u8], zero_at: usize, zero: u64| {
        ram[at..at + image.len()].copy_from_slice(image);
        let zero_end = zero_at + zero as usize;
        ram[zero_at.min(zephyr.len())..zero_end.min(zephyr.len())].fill(0);
        cleared_after = cleared_after.max((zero_end as u64).saturating_sub(zephyr.len() as u64));
    };
    if info.version < VERSION_SEGMENTS {
        place(0, &images[0], info.zero_offset as usize, info.zero_bytes);
    } else {
        for (s, image) in info.segments[..info.count as usize].iter().zip(&images) {
            let at = (s.load - limits.load) as usize;
            if at >= zephyr.len() {
                continue; // a segment of another core
            }
            place(at, image, at + image.len(), s.zero_bytes as u64);
        }
    }
    assert!(ram == zephyr, "the segments and zero ranges do not give zephyr.bin back");
    println!(
        "board_payload: header version {}, {} segment(s), {} stored bytes give the {} bytes of zephyr.bin back; {} bytes cleared after it",
        info.version,
        info.count.max(1),
        info.stored_bytes,
        zephyr.len(),
        cleared_after
    );
}
