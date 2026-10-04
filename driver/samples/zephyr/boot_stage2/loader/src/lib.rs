// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `yakogl-boot-stage2`: the logic of the Verdin AM62P second-stage loader
//! (docs/direct-boot.md, "The second stage").
//!
//! The A53 SPL loads the loader from the eMMC boot partition (the u-boot.img
//! slot holds 1.49 MiB, too small for a GL image). The loader reads the real
//! image from a raw sector range of the eMMC user area, checks it and jumps
//! to it. This crate is that check and the jump; Zephyr's disk API, the log
//! and the cache flush are the C side's (`src/main.c`).
//!
//! The image in the user area: one 512-byte header sector, then the stored
//! payload (sector aligned). The header, little-endian:
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
//! `tools/am62p-mkboot.py stage2img` writes it; the CRC-32 is zlib's.
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod crc32;
pub mod inflate;

use core::ffi::c_char;

pub const MAGIC: [u8; 8] = *b"YKSTAGE2";
pub const VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 512;
pub const COMPRESSION_NONE: u32 = 0;
pub const COMPRESSION_DEFLATE: u32 = 1;
pub const NAME_BYTES: usize = 32;

/// What the loader accepts, from its Kconfig (include/boot_stage2.h).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Bs2Limits {
    /// The only load address this loader serves (the image's link address).
    pub load: u64,
    /// The most bytes the user-area range holds after the header sector.
    pub max_stored: u64,
    /// The most bytes the image may have once decompressed (up to the
    /// staging buffer, or the end of the mapped region for a stored image).
    pub max_image: u64,
}

/// The header, checked (include/boot_stage2.h).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bs2Info {
    pub load: u64,
    pub entry: u64,
    pub stored_bytes: u64,
    pub image_bytes: u64,
    pub compression: u32,
    pub stored_crc32: u32,
    pub image_crc32: u32,
    /// NUL terminated (the last byte is always 0).
    pub name: [u8; NAME_BYTES + 1],
}

// The layouts include/boot_stage2.h pins too.
const _: () = assert!(core::mem::size_of::<Bs2Limits>() == 24);
const _: () = assert!(core::mem::offset_of!(Bs2Info, compression) == 32);
const _: () = assert!(core::mem::offset_of!(Bs2Info, name) == 44);
const _: () = assert!(core::mem::size_of::<Bs2Info>() == 80);

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
}

impl Bs2Error {
    fn from_code(code: i32) -> Option<Self> {
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
        ]
        .into_iter()
        .find(|e| *e as i32 == code)
    }

    /// NUL-terminated, for the C side's log.
    pub fn text(self) -> &'static str {
        match self {
            Bs2Error::BadArgument => "null pointer or short buffer\0",
            Bs2Error::BadMagic => "no YKSTAGE2 header at the configured sector\0",
            Bs2Error::BadHeaderCrc => "header CRC-32 mismatch\0",
            Bs2Error::BadVersion => "unknown header version or size\0",
            Bs2Error::BadCompression => "unknown compression\0",
            Bs2Error::WrongLoad => "load address is not the one this loader serves\0",
            Bs2Error::EntryOutside => "entry address outside the image\0",
            Bs2Error::StoredTooBig => "stored payload larger than the user-area range\0",
            Bs2Error::ImageTooBig => "image larger than the room at the load address\0",
            Bs2Error::SizesDisagree => "stored and image sizes disagree (or are zero)\0",
            Bs2Error::StoredCrc => "CRC-32 of the stored payload mismatch\0",
            Bs2Error::ImageCrc => "CRC-32 of the image mismatch\0",
            Bs2Error::Inflate => "deflate stream is malformed\0",
            Bs2Error::LengthMismatch => "decompressed length is not the header's\0",
            Bs2Error::NotArm64Image => "no arm64 Image header (ARM\\x64 at 0x38)\0",
        }
    }
}

fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn le64(b: &[u8], off: usize) -> u64 {
    (le32(b, off) as u64) | ((le32(b, off + 4) as u64) << 32)
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
    if le32(h, 8) != VERSION || le32(h, 12) as usize != HEADER_BYTES {
        return Err(Bs2Error::BadVersion);
    }
    let compression = le32(h, 16);
    if compression != COMPRESSION_NONE && compression != COMPRESSION_DEFLATE {
        return Err(Bs2Error::BadCompression);
    }
    let mut name = [0u8; NAME_BYTES + 1];
    name[..NAME_BYTES].copy_from_slice(&h[64..64 + NAME_BYTES]);
    let info = Bs2Info {
        load: le64(h, 24),
        entry: le64(h, 32),
        stored_bytes: le64(h, 40),
        image_bytes: le64(h, 48),
        compression,
        stored_crc32: le32(h, 56),
        image_crc32: le32(h, 60),
        name,
    };
    if info.load != limits.load {
        return Err(Bs2Error::WrongLoad);
    }
    if info.stored_bytes == 0
        || info.image_bytes == 0
        || (compression == COMPRESSION_NONE && info.stored_bytes != info.image_bytes)
    {
        return Err(Bs2Error::SizesDisagree);
    }
    if info.stored_bytes > limits.max_stored {
        return Err(Bs2Error::StoredTooBig);
    }
    if info.image_bytes > limits.max_image {
        return Err(Bs2Error::ImageTooBig);
    }
    if info.entry < info.load || info.entry - info.load >= info.image_bytes {
        return Err(Bs2Error::EntryOutside);
    }
    Ok(info)
}

/// The image must be an arm64 Image (Zephyr's CONFIG_AARCH64_IMAGE_HEADER):
/// "ARM\x64" at 0x38. Catches a header that describes the wrong file.
pub fn check_arm64_image(image: &[u8]) -> Result<(), Bs2Error> {
    if image.len() < 64 || le32(image, 0x38) != 0x644D_5241 {
        return Err(Bs2Error::NotArm64Image);
    }
    Ok(())
}

/// Decompress (or take) the stored payload into the image buffer and check
/// both CRCs. `stored` and `image` must not overlap for a deflate payload;
/// for COMPRESSION_NONE they are the same bytes and only the CRC is checked.
pub fn unpack(info: &Bs2Info, stored: &[u8], image: &mut [u8]) -> Result<usize, Bs2Error> {
    if crc32::crc32(stored) != info.stored_crc32 {
        return Err(Bs2Error::StoredCrc);
    }
    let n = match info.compression {
        COMPRESSION_DEFLATE => inflate::inflate(stored, image).map_err(|_| Bs2Error::Inflate)?,
        _ => stored.len(),
    };
    if n as u64 != info.image_bytes {
        return Err(Bs2Error::LengthMismatch);
    }
    let img = if info.compression == COMPRESSION_DEFLATE { &image[..n] } else { stored };
    if crc32::crc32(img) != info.image_crc32 {
        return Err(Bs2Error::ImageCrc);
    }
    check_arm64_image(img)?;
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

/// Check the stored payload and produce the image: inflate `stored` into
/// `image` (capacity `image_cap`) or, uncompressed, check it where it is
/// (`image` may then be `stored`). 0 or a negative Bs2Error code.
///
/// # Safety
/// `info` is what bs2_header_parse wrote; `stored` points at
/// `info->stored_bytes` readable bytes; `image` at `image_cap` writable bytes,
/// not overlapping `stored` unless the image is uncompressed.
#[no_mangle]
pub unsafe extern "C" fn bs2_unpack(info: *const Bs2Info, stored: *const u8, image: *mut u8, image_cap: usize) -> i32 {
    if info.is_null() || stored.is_null() || image.is_null() {
        return Bs2Error::BadArgument as i32;
    }
    // SAFETY: as documented above.
    let info = unsafe { &*info };
    let stored = unsafe { core::slice::from_raw_parts(stored, info.stored_bytes as usize) };
    let r = if info.compression == COMPRESSION_DEFLATE {
        // SAFETY: as documented above (not overlapping `stored`).
        let image = unsafe { core::slice::from_raw_parts_mut(image, image_cap) };
        unpack(info, stored, image)
    } else {
        unpack(info, stored, &mut [])
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

/// The text of a code bs2_header_parse / bs2_unpack returned.
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
mod tests {
    use super::*;
    use std::vec::Vec;

    const LIMITS: Bs2Limits = Bs2Limits { load: 0x8200_0000, max_stored: 1 << 20, max_image: 1 << 22 };

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
        refuse(&|_, hh| hh[8..16].copy_from_slice(&[2, 0, 0, 0, 0, 2, 0, 0]), Bs2Error::BadVersion);
        refuse(&|i, _| i.compression = 2, Bs2Error::BadCompression);
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
        for code in -15..=-1 {
            let e = Bs2Error::from_code(code).expect("every code has a variant");
            assert!(e.text().ends_with('\0') && e.text().len() > 5);
        }
        assert!(Bs2Error::from_code(0).is_none());
        assert!(Bs2Error::from_code(-16).is_none());
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
        let limits =
            Bs2Limits { load: 0x8200_0000, max_stored: 8128 * 512 - 512, max_image: 0x9000_0000 - 0x8200_0000 };
        let (mut ok, mut bad) = (0, 0);
        for p in &names {
            let file = std::fs::read(p).unwrap();
            let stem = p.file_name().unwrap().to_string_lossy().into_owned();
            let result = parse_header(&file, &limits).and_then(|info| {
                let stored = &file[HEADER_BYTES..HEADER_BYTES + info.stored_bytes as usize];
                let mut image = vec![0u8; info.image_bytes as usize];
                unpack(&info, stored, &mut image)
            });
            if stem.starts_with("ok-") {
                assert!(result.is_ok(), "{stem}: {result:?}");
                ok += 1;
            } else {
                assert!(result.is_err(), "{stem} was accepted");
                bad += 1;
            }
        }
        assert!(ok >= 3 && bad >= 3, "too few vectors in {dir:?}: {ok} ok, {bad} bad");
        println!("mkboot_vectors: {} images verified ({ok} loaded, {bad} refused)", ok + bad);
    }
}
