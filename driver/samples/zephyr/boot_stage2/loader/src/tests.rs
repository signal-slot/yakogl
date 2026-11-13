// SPDX-License-Identifier: Apache-2.0
//! Host tests of the header checks, the segment unpack and the cross-check
//! with `tools/am62p-mkboot.py stage2-vectors` (BS2_VECTORS).
use super::*;
use std::vec::Vec;

const LOAD: u64 = 0x8200_0000;
const LIMITS: Bs2Limits = Bs2Limits {
    load: LOAD,
    region_end: LOAD + (64 << 20),
    max_stored: 1 << 20,
    staging: LOAD + (1 << 22),
    staging_bytes: 0,
};
/// The AM62P loader's own limits: region 0x82000000..0x97800000, the
/// default user-area range (sectors 64..8191) and staging at 0x90000000.
const AM62P: Bs2Limits = Bs2Limits {
    load: 0x8200_0000,
    region_end: 0x9780_0000,
    max_stored: 8128 * 512 - 512,
    staging: 0x9000_0000,
    staging_bytes: 8128 * 512 - 512,
};

fn zimage(len: usize) -> Vec<u8> {
    let mut z: Vec<u8> = (0..len).map(|i| (i * 13 / 7) as u8).collect();
    z[0x38..0x3C].copy_from_slice(b"ARM\x64");
    z[16..24].copy_from_slice(&(len as u64).to_le_bytes());
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

/// A version-1 header in mkboot's layout (the Python writer is checked
/// against the same layout by the BS2_VECTORS test below).
fn header_v1(info: &Bs2Info) -> [u8; 512] {
    let s = &info.segments[0];
    let mut h = [0u8; 512];
    h[..8].copy_from_slice(&MAGIC);
    h[8..12].copy_from_slice(&VERSION_SINGLE.to_le_bytes());
    h[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    h[16..20].copy_from_slice(&s.compression.to_le_bytes());
    h[24..32].copy_from_slice(&s.load.to_le_bytes());
    h[32..40].copy_from_slice(&info.entry.to_le_bytes());
    h[40..48].copy_from_slice(&s.stored_bytes.to_le_bytes());
    h[48..56].copy_from_slice(&s.image_bytes.to_le_bytes());
    h[56..60].copy_from_slice(&s.stored_crc32.to_le_bytes());
    h[60..64].copy_from_slice(&s.image_crc32.to_le_bytes());
    h[64..96].copy_from_slice(&info.name[..32]);
    seal(&mut h);
    h
}

/// A version-2 header from a Bs2Info, without any check.
fn header_v2(info: &Bs2Info) -> [u8; 512] {
    header_v2_parts(info, 0)
}

/// The same with the header's image parts field (0: the field as it was
/// before image parts, one part).
fn header_v2_parts(info: &Bs2Info, parts: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..8].copy_from_slice(&MAGIC);
    h[8..12].copy_from_slice(&VERSION_SEGMENTS.to_le_bytes());
    h[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    h[16..20].copy_from_slice(&info.count.to_le_bytes());
    h[20..24].copy_from_slice(&parts.to_le_bytes());
    h[24..32].copy_from_slice(&info.entry.to_le_bytes());
    h[32..40].copy_from_slice(&info.stored_bytes.to_le_bytes());
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

fn info_v1(image: &[u8]) -> Bs2Info {
    let mut segments = [Bs2Segment::default(); MAX_SEGMENTS];
    segments[0] = Bs2Segment {
        load: LOAD,
        image_bytes: image.len() as u64,
        footprint: image.len() as u64,
        stored_offset: 0,
        stored_bytes: image.len() as u64,
        compression: COMPRESSION_NONE,
        stored_crc32: crc32::crc32(image),
        image_crc32: crc32::crc32(image),
        zero_bytes: 0,
    };
    Bs2Info {
        segments,
        entry: LOAD,
        stored_bytes: image.len() as u64,
        version: VERSION_SINGLE,
        count: 1,
        name: name("test"),
    }
}

fn deflate_fixed(data: &[u8]) -> Vec<u8> {
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
        let stored = if *comp == COMPRESSION_DEFLATE { deflate_fixed(data) } else { data.to_vec() };
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
    let info = Bs2Info {
        segments,
        entry: segs[0].1,
        stored_bytes: stored_all.len() as u64,
        version: VERSION_SEGMENTS,
        count: segs.len() as u32,
        name: name("multi"),
    };
    let mut file = header_v2(&info).to_vec();
    file.extend_from_slice(&stored_all);
    Payload { info, file, images: segs.iter().map(|s| s.0.to_vec()).collect() }
}

/// Parse and place every segment of a payload into host buffers; the
/// images, in table order.
fn load_all(file: &[u8], limits: &Bs2Limits) -> Result<Vec<Vec<u8>>, Bs2Error> {
    let info = parse_header(file, limits)?;
    let body = &file[HEADER_BYTES..];
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
fn v1_round_trip() {
    let z = zimage(5000);
    let info = info_v1(&z);
    let h = header_v1(&info);
    assert_eq!(parse_header(&h, &LIMITS), Ok(info));
    assert_eq!(unpack_segment(&info, 0, &z, None), Ok(5000));
    let mut copy = vec![0u8; 5000];
    assert_eq!(unpack_segment(&info, 0, &z, Some(&mut copy)), Ok(5000));
    assert_eq!(copy, z, "an uncompressed segment is copied to its destination");
}

#[test]
fn v1_refusals() {
    let z = zimage(5000);
    let info = info_v1(&z);
    let h = header_v1(&info);
    let mut bad = h;
    bad[0] = b'X';
    assert_eq!(parse_header(&bad, &LIMITS), Err(Bs2Error::BadMagic));
    let mut bad = h;
    bad[100] = 1;
    assert_eq!(parse_header(&bad, &LIMITS), Err(Bs2Error::BadHeaderCrc));
    assert_eq!(parse_header(&h[..511], &LIMITS), Err(Bs2Error::BadArgument));
    let mut bad = h;
    bad[8] = 3;
    seal(&mut bad);
    assert_eq!(parse_header(&bad, &LIMITS), Err(Bs2Error::BadVersion));
    let mut bad = h;
    bad[12] = 1;
    seal(&mut bad);
    assert_eq!(parse_header(&bad, &LIMITS), Err(Bs2Error::BadVersion));
    let refuse = |f: &dyn Fn(&mut Bs2Info), want: Bs2Error| {
        let mut i = info;
        f(&mut i);
        assert_eq!(parse_header(&header_v1(&i), &LIMITS), Err(want));
    };
    refuse(&|i| i.segments[0].compression = 2, Bs2Error::BadCompression);
    refuse(&|i| i.segments[0].load = 0x8080_0000, Bs2Error::WrongLoad);
    refuse(&|i| i.entry = LOAD + 5000, Bs2Error::EntryOutside);
    refuse(&|i| i.entry = LOAD - 4, Bs2Error::EntryOutside);
    refuse(
        &|i| {
            i.segments[0].stored_bytes = (1 << 20) + 1;
            i.segments[0].image_bytes = (1 << 20) + 1;
        },
        Bs2Error::StoredTooBig,
    );
    // version 1 keeps its bound: the image ends below the staging buffer
    refuse(
        &|i| {
            i.segments[0].compression = COMPRESSION_DEFLATE;
            i.segments[0].image_bytes = (1 << 22) + 1;
        },
        Bs2Error::ImageTooBig,
    );
    refuse(&|i| i.segments[0].image_bytes = 4999, Bs2Error::SizesDisagree);
    refuse(&|i| i.segments[0].stored_bytes = 0, Bs2Error::SizesDisagree);
}

#[test]
fn unpack_refusals() {
    let z = zimage(5000);
    let info = info_v1(&z);
    let mut flipped = z.clone();
    flipped[4000] ^= 0x10;
    assert_eq!(unpack_segment(&info, 0, &flipped, None), Err(Bs2Error::StoredCrc));
    let mut i = info;
    i.segments[0].image_crc32 ^= 1;
    assert_eq!(unpack_segment(&i, 0, &z, None), Err(Bs2Error::ImageCrc));
    let plain = vec![0u8; 5000];
    let i = info_v1(&plain);
    assert_eq!(unpack_segment(&i, 0, &plain, None), Err(Bs2Error::NotArm64Image));
    assert_eq!(unpack_segment(&info, 1, &z, None), Err(Bs2Error::BadArgument), "index past the count");
    assert_eq!(unpack_segment(&info, 0, &z[..4999], None), Err(Bs2Error::BadArgument), "short stored");
    let mut small = vec![0u8; 4999];
    assert_eq!(unpack_segment(&info, 0, &z, Some(&mut small)), Err(Bs2Error::BadArgument), "short image");
    // deflate cannot be "in place"
    let p = payload(&[(&z, LOAD, COMPRESSION_DEFLATE)]);
    let s = &p.info.segments[0];
    let stored = &p.file[512..512 + s.stored_bytes as usize];
    assert_eq!(unpack_segment(&p.info, 0, stored, None), Err(Bs2Error::BadArgument));
}

#[test]
fn v2_three_segments() {
    // The showcase's shape: an image with .bss past its file, then two
    // blobs above it (the R5F's DDR image and TCM blob).
    let mut sup = zimage(70_000);
    sup[16..24].copy_from_slice(&0x40_0000u64.to_le_bytes());
    let ddr: Vec<u8> = (0..30_000u32).map(|i| (i * 7 + 3) as u8).collect();
    let tcm: Vec<u8> = (0..288u32).map(|i| i as u8).collect();
    for comp in [COMPRESSION_NONE, COMPRESSION_DEFLATE] {
        let p = payload(&[(&sup, LOAD, comp), (&ddr, 0x8e00_0000, comp), (&tcm, 0x8ec0_0000, comp)]);
        let info = parse_header(&p.file, &AM62P).expect("a valid three-segment payload");
        assert_eq!((info.count, info.version, info.entry), (3, 2, LOAD));
        assert_eq!(info.segments[0].footprint, 0x40_0000);
        assert_eq!(load_all(&p.file, &AM62P), Ok(p.images.clone()), "compression {comp}");
    }
}

#[test]
fn v2_single_segment_reads_like_v1() {
    let z = zimage(9000);
    let p = payload(&[(&z, LOAD, COMPRESSION_DEFLATE)]);
    assert_eq!(load_all(&p.file, &AM62P), Ok(vec![z]));
}

#[test]
fn v2_refusals() {
    let mut sup = zimage(70_000);
    sup[16..24].copy_from_slice(&0x40_0000u64.to_le_bytes());
    let ddr = vec![0x5au8; 30_000];
    let tcm = vec![0xa5u8; 288];
    let good =
        payload(&[(&sup, LOAD, COMPRESSION_NONE), (&ddr, 0x8e00_0000, COMPRESSION_NONE), (&tcm, 0x8ec0_0000, 0)]);
    assert!(parse_header(&good.file, &AM62P).is_ok());
    let refuse = |what: &str, f: &dyn Fn(&mut Bs2Info), want: Bs2Error| {
        let mut i = good.info;
        f(&mut i);
        assert_eq!(parse_header(&header_v2(&i), &AM62P), Err(want), "{what}");
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
    refuse("an unknown compression", &|i| i.segments[2].compression = 7, Bs2Error::BadCompression);
    refuse("a zero-length segment", &|i| i.segments[2].image_bytes = 0, Bs2Error::SizesDisagree);
    refuse("uncompressed with stored != image", &|i| i.segments[2].image_bytes = 287, Bs2Error::SizesDisagree);
    refuse("footprint below the image", &|i| i.segments[1].footprint = 100, Bs2Error::SizesDisagree);
    refuse("payload over the source", &|i| i.stored_bytes = AM62P.max_stored + 512, Bs2Error::StoredTooBig);
    refuse("segment 0 not at the load address", &|i| i.segments[0].load = 0x8300_0000, Bs2Error::WrongLoad);
    refuse("entry past segment 0's image", &|i| i.entry = LOAD + 70_000, Bs2Error::EntryOutside);
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
    assert!(parse_header(&header_v2(&i), &AM62P).is_ok());
    // without a staging buffer (the linked payload) the staging range is free
    let mut i = good.info;
    i.segments[2].load = 0x9000_0000;
    let embedded = Bs2Limits { staging_bytes: 0, ..AM62P };
    assert!(parse_header(&header_v2(&i), &embedded).is_ok());
    // a non-zero byte in the header's reserved tail
    let mut h = header_v2(&good.info);
    h[450] = 1;
    seal(&mut h);
    assert_eq!(parse_header(&h, &AM62P), Err(Bs2Error::BadVersion));
    // an unused table entry that is not zero
    let mut h = header_v2(&good.info);
    h[SEGMENT_TABLE + 5 * SEGMENT_ENTRY_BYTES] = 1;
    seal(&mut h);
    assert_eq!(parse_header(&h, &AM62P), Err(Bs2Error::BadVersion));
}

#[test]
fn v2_unpack_refusals() {
    let mut sup = zimage(70_000);
    sup[16..24].copy_from_slice(&0x40_0000u64.to_le_bytes());
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
    f[..512].copy_from_slice(&header_v2(&i));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Inflate));
    // the image CRC
    let mut i = p.info;
    i.segments[1].image_crc32 ^= 1;
    let mut f = p.file.clone();
    f[..512].copy_from_slice(&header_v2(&i));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::ImageCrc));
    // segment 0's .bss larger than the footprint the overlap check used
    let mut i = p.info;
    i.segments[0].footprint = 0x20_0000;
    let mut f = p.file.clone();
    f[..512].copy_from_slice(&header_v2(&i));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Footprint));
    // segment 0 is not an arm64 Image
    let plain = vec![0u8; 4096];
    let q = payload(&[(&plain, LOAD, COMPRESSION_NONE), (&ddr, 0x8e00_0000, COMPRESSION_NONE)]);
    assert_eq!(load_all(&q.file, &AM62P), Err(Bs2Error::NotArm64Image));
    // other segments need no format
    let z = zimage(4096);
    let q = payload(&[(&z, LOAD, COMPRESSION_NONE), (&plain, 0x8e00_0000, COMPRESSION_NONE)]);
    assert!(load_all(&q.file, &AM62P).is_ok());
}

/// An image of `len` bytes cut into parts at the given (offset, length)
/// zero runs, the way `am62p-mkboot.py stage2img --split-zeros` cuts one:
/// segment 0 and one continuation part after each run, the last part with
/// `clear_after` zero bytes past the image's last byte. Then `extra`
/// segments. Returns the payload and the image.
fn split_payload(len: usize, runs: &[(usize, usize)], clear_after: u32, extra: &[(&[u8], u64)]) -> (Payload, Vec<u8>) {
    let mut z = zimage(len);
    z[16..24].copy_from_slice(&0x40_0000u64.to_le_bytes());
    for &(at, n) in runs {
        z[at..at + n].fill(0);
    }
    let mut segments = [Bs2Segment::default(); MAX_SEGMENTS];
    let mut stored_all = Vec::new();
    let mut images = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for k in 0..=runs.len() {
        let (end, zero) = match runs.get(k) {
            Some(&(at, n)) => (at, n as u32),
            None => (len, clear_after),
        };
        let data = &z[start..end];
        segments[count] = Bs2Segment {
            load: LOAD + start as u64,
            image_bytes: data.len() as u64,
            footprint: if k == 0 { 0x40_0000 } else { data.len() as u64 + zero as u64 },
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
        start = end + zero as usize;
        count += 1;
    }
    for (data, load) in extra {
        segments[count] = Bs2Segment {
            load: *load,
            image_bytes: data.len() as u64,
            footprint: data.len() as u64,
            stored_offset: stored_all.len() as u64,
            stored_bytes: data.len() as u64,
            compression: COMPRESSION_NONE,
            stored_crc32: crc32::crc32(data),
            image_crc32: crc32::crc32(data),
            zero_bytes: 0,
        };
        stored_all.extend_from_slice(data);
        stored_all.resize(stored_all.len().div_ceil(512) * 512, 0);
        images.push(data.to_vec());
        count += 1;
    }
    let info = Bs2Info {
        segments,
        entry: LOAD,
        stored_bytes: stored_all.len() as u64,
        version: VERSION_SEGMENTS,
        count: count as u32,
        name: name("split"),
    };
    let mut file = header_v2_parts(&info, runs.len() as u32 + 1).to_vec();
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
fn v2_zero_bytes() {
    // One part: the image, then its .bss cleared by the loader. Up to the
    // footprint's last byte is accepted.
    let (p, z) = split_payload(70_000, &[], 0x40_0000 - 70_000, &[]);
    let info = parse_header(&p.file, &AM62P).expect("zero bytes up to the footprint");
    assert_eq!((info.count, info.segments[0].zero_bytes), (1, 0x40_0000 - 70_000));
    let ram = reassemble(&p.file, &AM62P, 1, 0x40_0000);
    assert_eq!(&ram[..70_000], &z[..]);
    assert!(ram[70_000..].iter().all(|&b| b == 0));
    // image parts 1 and 0 are the same header rule
    assert_eq!(parse_header(&header_v2_parts(&p.info, 1), &AM62P), Ok(info));
    assert_eq!(parse_header(&header_v2_parts(&p.info, 0), &AM62P), Ok(info));
    // a blob's zero bytes inside a footprint larger than its image
    let blob = vec![0x5au8; 3000];
    let (q, _) = split_payload(70_000, &[], 0, &[(&blob, 0x8e00_0000)]);
    let mut i = q.info;
    i.segments[1].footprint = 8192;
    i.segments[1].zero_bytes = 8192 - 3000;
    assert!(parse_header(&header_v2(&i), &AM62P).is_ok());
    i.segments[1].zero_bytes += 1;
    assert_eq!(parse_header(&header_v2(&i), &AM62P), Err(Bs2Error::ZeroRange));
}

#[test]
fn v2_image_parts() {
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
        assert_eq!(parse_header(&header_v2_parts(&i, parts), &AM62P), Err(want), "{what}");
    };
    let accept = |what: &str, parts: u32, f: &dyn Fn(&mut Bs2Info)| {
        let mut i = p.info;
        f(&mut i);
        assert!(parse_header(&header_v2_parts(&i, parts), &AM62P).is_ok(), "{what}");
    };
    refuse("more parts than segments", 6, &|_| {}, Bs2Error::ImageParts);
    accept("every segment a part needs them all inside segment 0", 3, &|i| i.count = 3);
    refuse("a blob counted as a part", 4, &|_| {}, Bs2Error::ImageParts);
    // Fewer parts than the table holds: the continuation is then an
    // ordinary segment inside segment 0's footprint.
    refuse("a part not counted", 2, &|_| {}, Bs2Error::SegmentOverlap);
    refuse("parts read as one (the old field value)", 0, &|_| {}, Bs2Error::SegmentOverlap);
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
    f[..512].copy_from_slice(&header_v2_parts(&i, 3));
    assert_eq!(load_all(&f, &AM62P), Err(Bs2Error::Footprint));
}

#[test]
fn strerror_names_every_code() {
    for code in -22..=-1 {
        let e = Bs2Error::from_code(code).expect("every code has a variant");
        assert!(e.text().ends_with('\0') && e.text().len() > 5);
    }
    assert!(Bs2Error::from_code(0).is_none());
    assert!(Bs2Error::from_code(-23).is_none());
}

/// The images `tools/am62p-mkboot.py stage2-vectors <dir>` writes (Python's
/// zlib as the compressor, so this is the cross-check of the two sides):
/// `ok-*.img` must load, `bad-*.img` must be refused. Without BS2_VECTORS
/// the test says so and checks nothing; tools/check.sh sets it and requires
/// the "verified" line.
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
            if images.len() > 1 {
                multi += 1;
            }
            let info = parse_header(&file, &AM62P).unwrap();
            if info.segments.iter().any(|s| s.zero_bytes != 0) {
                zeroed += 1;
            }
            ok += 1;
        } else {
            assert!(result.is_err(), "{stem} was accepted");
            // The version-2 refusals name the rule they break.
            let want = match stem.as_str() {
                "bad-v2-overlap-bss.img" | "bad-v2-overlap-blobs.img" | "bad-v2-on-staging.img" => {
                    Some(Bs2Error::SegmentOverlap)
                }
                "bad-v2-outside-ram.img" | "bad-v2-below-ram.img" => Some(Bs2Error::SegmentOutside),
                "bad-v2-unaligned.img" | "bad-v2-past-payload.img" => Some(Bs2Error::SegmentLayout),
                "bad-v2-no-segments.img" => Some(Bs2Error::SegmentCount),
                "bad-v2-footprint.img" => Some(Bs2Error::Footprint),
                "bad-v2-segment-crc.img" => Some(Bs2Error::StoredCrc),
                "bad-v2-zero-over-footprint.img" => Some(Bs2Error::ZeroRange),
                "bad-v2-parts-over-count.img"
                | "bad-v2-part-outside.img"
                | "bad-v2-part-on-occupied.img"
                | "bad-v2-parts-descending.img" => Some(Bs2Error::ImageParts),
                "bad-v2-blob-in-footprint.img" => Some(Bs2Error::SegmentOverlap),
                s if s.starts_with("bad-v2-") => panic!("{s}: no expected refusal listed"),
                _ => None,
            };
            if let Some(w) = want {
                assert_eq!(result, Err(w), "{stem}");
            }
            bad += 1;
        }
    }
    assert!(
        ok >= 3 && bad >= 3 && multi >= 1 && zeroed >= 2,
        "too few vectors in {dir:?}: {ok} ok ({multi} multi, {zeroed} with zero bytes), {bad} bad"
    );
    println!(
        "mkboot_vectors: {} images verified ({ok} loaded, {multi} of them multi-segment, {zeroed} with zero bytes, {bad} refused)",
        ok + bad
    );
}

/// A payload as it goes to a board, through the loader's own checks, under
/// the limits that board's loader is built with: BS2_PAYLOAD=<stage2 .img>,
/// BS2_PAYLOAD_ZEPHYR=<the zephyr.bin it was made from>, and
/// BS2_PAYLOAD_LIMITS="<load>,<region end>,<max stored>,<staging>" (hex or
/// decimal; the loader's BOOT_STAGE2_LOAD, _REGION_END, the sector range's
/// bytes and _STAGING). The image is put together again from the segments
/// and their zero bytes and must equal zephyr.bin; what follows the image's
/// last byte in the last part's zero bytes is the .bss the loader clears.
/// Without BS2_PAYLOAD the test says so and checks nothing.
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
    let limits = Bs2Limits { load: l[0], region_end: l[1], max_stored: l[2], staging: l[3], staging_bytes: l[2] };
    let file = std::fs::read(&path).unwrap();
    let zephyr = std::fs::read(std::env::var_os("BS2_PAYLOAD_ZEPHYR").expect("BS2_PAYLOAD_ZEPHYR")).unwrap();
    let info = parse_header(&file, &limits).unwrap_or_else(|e| panic!("header refused: {e:?}"));
    let images = load_all(&file, &limits).unwrap_or_else(|e| panic!("payload refused: {e:?}"));
    // The loader's memory from the load address: every byte 0xAA until a
    // segment or a zero range writes it.
    let mut ram = vec![0xAAu8; zephyr.len()];
    let mut cleared_after = 0u64;
    for (s, image) in info.segments[..info.count as usize].iter().zip(&images) {
        let at = (s.load - limits.load) as usize;
        if at >= ram.len() {
            continue; // a segment of another core
        }
        ram[at..at + image.len()].copy_from_slice(image);
        let zero_end = at + image.len() + s.zero_bytes as usize;
        for b in &mut ram[(at + image.len()).min(zephyr.len())..zero_end.min(zephyr.len())] {
            *b = 0;
        }
        cleared_after = cleared_after.max((zero_end as u64).saturating_sub(zephyr.len() as u64));
    }
    assert!(ram == zephyr, "the segments and zero ranges do not give zephyr.bin back");
    println!(
        "board_payload: {} segment(s), {} stored bytes give the {} bytes of zephyr.bin back; {} bytes cleared after it",
        info.count,
        info.stored_bytes,
        zephyr.len(),
        cleared_after
    );
}
