// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2023 Collabora ltd.
//! The CSF firmware image (`mali_csffw.bin`): its header and its list of
//! sections, each loaded at a fixed MCU address in AS0
//! (driver/docs/nanopc-t6-mali.md, "The firmware (Z3)").
//!
//! Derived from Linux v6.18's drivers/gpu/drm/panthor/panthor_fw.c
//! (Copyright 2023 Collabora ltd.), used under the MIT option of its
//! `GPL-2.0 or MIT` licence (the design page's decision 1): the binary
//! format's definitions (panthor_fw.c:39-140) and the checks of
//! `panthor_fw_load` (:740-806), `panthor_fw_load_entry` (:692-738),
//! `panthor_fw_load_section_entry` (:489-633) and
//! `panthor_fw_read_build_info` (:635-666), in panthor's order. Each check
//! below says whether it is panthor's or ours.
//!
//! The firmware is never in the tree (as `pvr_fw_blob.h`): a build takes it
//! from the header `driver/tools/mkmalifw.sh` makes out of the board's file.
//! Here it is a byte slice. Nothing is copied: a [`Section`] names its bytes
//! by their offsets in the image.
//!
//! The board's file (Armbian 26.8.10, armbian-firmware,
//! /lib/firmware/arm/mali/arch10.8/mali_csffw.bin, 282624 bytes, sha256
//! a27847ea11f8efb3136340c3ba8aab413ae25145eeb4a7f64ff5edd829a2405b, the
//! same bytes as linux-firmware's at 930ef904: tools/cts/deps.toml) parses
//! into eight sections, one of them protected (skipped), and the build
//! string `git_sha: 95a25d71030715381f33105394285e1dcc860a65`; the test
//! `the_boards_file_parses_into_the_design_pages_table` holds it to the
//! design page's table.

/// `CSF_FW_BINARY_HEADER_MAGIC`.
pub const MAGIC: u32 = 0xc3f1_3a6e;
/// `CSF_FW_BINARY_HEADER_MAJOR_MAX`: the only header major panthor loads.
pub const HEADER_MAJOR_MAX: u8 = 0;
/// `sizeof(struct panthor_fw_binary_hdr)`: magic, minor, major, padding,
/// version hash, padding, size.
pub const HEADER_SIZE: usize = 20;
/// `sizeof(struct panthor_fw_binary_section_entry_hdr)`: flags, va start
/// and end, data start and end.
pub const SECTION_HEADER_SIZE: usize = 20;

/// Entry types (`enum panthor_fw_binary_entry_type`).
pub const ENTRY_TYPE_IFACE: u32 = 0;
pub const ENTRY_TYPE_CONFIG: u32 = 1;
pub const ENTRY_TYPE_FUTF_TEST: u32 = 2;
pub const ENTRY_TYPE_TRACE_BUFFER: u32 = 3;
pub const ENTRY_TYPE_TIMELINE_METADATA: u32 = 4;
pub const ENTRY_TYPE_BUILD_INFO_METADATA: u32 = 6;
/// `CSF_FW_BINARY_ENTRY_UPDATE`, `CSF_FW_BINARY_ENTRY_OPTIONAL`.
pub const ENTRY_UPDATE: u32 = 1 << 30;
pub const ENTRY_OPTIONAL: u32 = 1 << 31;

/// `CSF_FW_BINARY_ENTRY_TYPE(ehdr)`.
pub const fn entry_type(ehdr: u32) -> u32 {
    ehdr & 0xff
}
/// `CSF_FW_BINARY_ENTRY_SIZE(ehdr)`: the entry's bytes, its header word
/// included.
pub const fn entry_size(ehdr: u32) -> u32 {
    (ehdr >> 8) & 0xff
}

/// Section flags (`CSF_FW_BINARY_IFACE_ENTRY_*`).
pub const IFACE_RD: u32 = 1 << 0;
pub const IFACE_WR: u32 = 1 << 1;
pub const IFACE_EX: u32 = 1 << 2;
pub const IFACE_CACHE_MODE_NONE: u32 = 0 << 3;
pub const IFACE_CACHE_MODE_CACHED: u32 = 1 << 3;
pub const IFACE_CACHE_MODE_UNCACHED_COHERENT: u32 = 2 << 3;
pub const IFACE_CACHE_MODE_CACHED_COHERENT: u32 = 3 << 3;
pub const IFACE_CACHE_MODE_MASK: u32 = 3 << 3;
pub const IFACE_PROT: u32 = 1 << 5;
pub const IFACE_SHARED: u32 = 1 << 30;
pub const IFACE_ZERO: u32 = 1 << 31;
/// `CSF_FW_BINARY_IFACE_ENTRY_SUPPORTED_FLAGS`.
pub const IFACE_SUPPORTED_FLAGS: u32 =
    IFACE_RD | IFACE_WR | IFACE_EX | IFACE_CACHE_MODE_MASK | IFACE_PROT | IFACE_SHARED | IFACE_ZERO;

/// `CSF_MCU_SHARED_REGION_START`, `_SIZE` (panthor_fw.c:199-200): the
/// shared interface's MCU window; the section that starts there is the
/// host's interface.
pub const MCU_SHARED_REGION_START: u32 = 0x0400_0000;
pub const MCU_SHARED_REGION_SIZE: u32 = 0x0400_0000;

/// The MCU's page: section addresses must be multiples of it (panthor
/// checks against its VM's page size, 4 KiB on the board's kernel and on
/// ours).
pub const MCU_PAGE: u32 = 0x1000;

/// The most sections an image may hold here (a fixed table: no allocator
/// in this crate). The board's file has eight, one of them protected.
pub const MAX_SECTIONS: usize = 16;

/// `"git_sha: "`, the prefix panthor accepts in a build-info entry.
const GIT_SHA_PREFIX: &[u8] = b"git_sha: ";

/// The image header (`struct panthor_fw_binary_hdr`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub minor: u8,
    pub major: u8,
    /// `version_hash`: on the board's file 0x01050000, the interface
    /// version 1.5.0 the GLB control block reports after boot.
    pub version_hash: u32,
    /// The bytes of the header and entry list (the section data lies after
    /// it, anywhere in the file).
    pub size: u32,
}

/// One interface section (`struct panthor_fw_binary_section_entry_hdr`):
/// `data_size()` bytes of the image copied to the start of
/// `[va_start, va_end)` in AS0, the rest zeroed when [`IFACE_ZERO`] says so.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub flags: u32,
    pub va_start: u32,
    pub va_end: u32,
    /// The data's range in the image, in bytes.
    pub data_start: u32,
    pub data_end: u32,
    /// Where the entry is in the image (for a record).
    pub entry_offset: u32,
}

impl Section {
    pub fn va_size(&self) -> u32 {
        self.va_end - self.va_start
    }
    pub fn data_size(&self) -> u32 {
        self.data_end - self.data_start
    }
    /// The section's bytes in `image`.
    pub fn data<'a>(&self, image: &'a [u8]) -> &'a [u8] {
        &image[self.data_start as usize..self.data_end as usize]
    }
    pub fn readable(&self) -> bool {
        self.flags & IFACE_RD != 0
    }
    pub fn writable(&self) -> bool {
        self.flags & IFACE_WR != 0
    }
    pub fn executable(&self) -> bool {
        self.flags & IFACE_EX != 0
    }
    pub fn zeroed(&self) -> bool {
        self.flags & IFACE_ZERO != 0
    }
    pub fn shared(&self) -> bool {
        self.flags & IFACE_SHARED != 0
    }
    pub fn protected(&self) -> bool {
        self.flags & IFACE_PROT != 0
    }
    pub fn cache_mode(&self) -> u32 {
        self.flags & IFACE_CACHE_MODE_MASK
    }
    /// How panthor maps it on a non-coherent GPU: every cache mode but
    /// `CACHED` is mapped uncached (panthor_fw.c:584-592, its TODO on the
    /// coherent modes). The RK3588's GPU is not coherent.
    pub fn gpu_cached(&self) -> bool {
        self.cache_mode() == IFACE_CACHE_MODE_CACHED
    }
}

/// Why an image was refused. Each carries the value that decided it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwError {
    /// Shorter than the header (panthor's iterator: "Firmware too small").
    TooSmall { len: usize },
    /// Not [`MAGIC`] (panthor: "Invalid firmware magic").
    BadMagic(u32),
    /// A header major other than [`HEADER_MAJOR_MAX`] (panthor:
    /// "Unsupported firmware binary header version").
    UnsupportedMajor(u8),
    /// The header's size is past the file's end (panthor: "Firmware image
    /// is truncated").
    Truncated { size: u32, len: usize },
    /// An entry at `offset` not 32-bit aligned, or of a size that is not
    /// (panthor: "Firmware entry isn't 32 bit aligned").
    Unaligned { offset: u32, size: u32 },
    /// An entry smaller than its own header word, or one that runs past
    /// the header's size (panthor's sub-iterator refuses it; the zero size
    /// is ours: panthor would loop on it).
    BadEntrySize { offset: u32, size: u32 },
    /// A section entry too short for its 20-byte header (panthor's
    /// iterator: "Firmware too small").
    ShortSection { offset: u32 },
    /// `data.end < data.start` (panthor).
    DataReversed { offset: u32 },
    /// `va.end < va.start` (panthor).
    VaReversed { offset: u32 },
    /// The section's data is past the file's end (panthor: "file
    /// truncated?").
    DataOutsideFile { offset: u32, data_end: u32 },
    /// A section address not a multiple of [`MCU_PAGE`] (panthor:
    /// "virtual addresses not page aligned").
    VaUnaligned { offset: u32 },
    /// A flag outside [`IFACE_SUPPORTED_FLAGS`] (panthor: "Firmware
    /// contains interface with unsupported flags").
    UnsupportedFlags { offset: u32, flags: u32 },
    /// The section at [`MCU_SHARED_REGION_START`] without [`IFACE_SHARED`]
    /// (panthor: "Interface at 0x%llx must be shared").
    SharedNotShared { offset: u32 },
    /// More data than the section's MCU range holds (ours: panthor's
    /// `panthor_fw_init_section_mem` would copy past the buffer it
    /// allocated for the range).
    DataLargerThanVa { offset: u32 },
    /// Two loaded sections overlap in AS0 (ours: panthor's VM map would
    /// fail on it).
    Overlap { offset: u32 },
    /// More than [`MAX_SECTIONS`] loaded sections (ours).
    TooManySections,
    /// An entry type panthor does not know, without [`ENTRY_OPTIONAL`]
    /// (panthor: "Unsupported non-optional entry type").
    UnknownEntry { offset: u32, ehdr: u32 },
    /// No section at [`MCU_SHARED_REGION_START`] (panthor: "Shared
    /// interface region not found").
    NoSharedSection,
}

/// A parsed image: the sections to load, in the file's order.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    pub bytes: &'a [u8],
    pub header: Header,
    sections: [Section; MAX_SECTIONS],
    count: usize,
    /// Index in [`Image::sections`] of the shared interface.
    pub shared: usize,
    /// Protected-mode sections, skipped as panthor skips them ("Firmware
    /// protected mode entry not be supported, ignoring").
    pub protected_skipped: u32,
    /// Entries of the types panthor reads and ignores (config, trace
    /// buffers, ...), and optional unknown ones.
    pub entries_ignored: u32,
    /// Build-info entries panthor would ignore: a range outside the file
    /// ("Firmware build info corrupt", and it continues), a string without
    /// the prefix, or one not NUL-terminated.
    pub build_info_ignored: u32,
    /// The build-info string without its prefix and NUL, when the image has
    /// a well-formed one.
    pub git_sha: Option<&'a [u8]>,
}

impl<'a> Image<'a> {
    /// The sections to load, protected ones left out.
    pub fn sections(&self) -> &[Section] {
        &self.sections[..self.count]
    }
    /// The shared interface's section.
    pub fn shared_section(&self) -> &Section {
        &self.sections[self.shared]
    }
}

fn word(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Parse `bytes` as `panthor_fw_load` walks it: the header, then one entry
/// after another up to the header's size.
pub fn parse(bytes: &[u8]) -> Result<Image<'_>, FwError> {
    if bytes.len() < HEADER_SIZE {
        return Err(FwError::TooSmall { len: bytes.len() });
    }
    let magic = word(bytes, 0);
    if magic != MAGIC {
        return Err(FwError::BadMagic(magic));
    }
    let header = Header { minor: bytes[4], major: bytes[5], version_hash: word(bytes, 8), size: word(bytes, 16) };
    if header.major != HEADER_MAJOR_MAX {
        return Err(FwError::UnsupportedMajor(header.major));
    }
    if header.size as usize > bytes.len() {
        return Err(FwError::Truncated { size: header.size, len: bytes.len() });
    }
    let mut img = Image {
        bytes,
        header,
        sections: [Section::default(); MAX_SECTIONS],
        count: 0,
        shared: usize::MAX,
        protected_skipped: 0,
        entries_ignored: 0,
        build_info_ignored: 0,
        git_sha: None,
    };
    let end = header.size as usize;
    let mut off = HEADER_SIZE;
    while off < end {
        if off + 4 > end {
            return Err(FwError::BadEntrySize { offset: off as u32, size: (end - off) as u32 });
        }
        let ehdr = word(bytes, off);
        let size = entry_size(ehdr);
        if !off.is_multiple_of(4) || !size.is_multiple_of(4) {
            return Err(FwError::Unaligned { offset: off as u32, size });
        }
        if (size as usize) < 4 || off + size as usize > end {
            return Err(FwError::BadEntrySize { offset: off as u32, size });
        }
        let body = &bytes[off + 4..off + size as usize];
        match entry_type(ehdr) {
            ENTRY_TYPE_IFACE => section_entry(&mut img, off as u32, body)?,
            ENTRY_TYPE_BUILD_INFO_METADATA => build_info(&mut img, off as u32, body)?,
            ENTRY_TYPE_CONFIG | ENTRY_TYPE_FUTF_TEST | ENTRY_TYPE_TRACE_BUFFER | ENTRY_TYPE_TIMELINE_METADATA => {
                img.entries_ignored += 1
            }
            _ if ehdr & ENTRY_OPTIONAL != 0 => img.entries_ignored += 1,
            _ => return Err(FwError::UnknownEntry { offset: off as u32, ehdr }),
        }
        off += size as usize;
    }
    if img.shared == usize::MAX {
        return Err(FwError::NoSharedSection);
    }
    Ok(img)
}

/// `panthor_fw_load_section_entry`'s checks, in its order; ours after
/// panthor's last one that can skip the entry (the protected-mode skip).
fn section_entry<'a>(img: &mut Image<'a>, offset: u32, body: &[u8]) -> Result<(), FwError> {
    if body.len() < SECTION_HEADER_SIZE {
        return Err(FwError::ShortSection { offset });
    }
    let s = Section {
        flags: word(body, 0),
        va_start: word(body, 4),
        va_end: word(body, 8),
        data_start: word(body, 12),
        data_end: word(body, 16),
        entry_offset: offset,
    };
    if s.data_end < s.data_start {
        return Err(FwError::DataReversed { offset });
    }
    if s.va_end < s.va_start {
        return Err(FwError::VaReversed { offset });
    }
    if s.data_end as usize > img.bytes.len() {
        return Err(FwError::DataOutsideFile { offset, data_end: s.data_end });
    }
    if !s.va_start.is_multiple_of(MCU_PAGE) || !s.va_end.is_multiple_of(MCU_PAGE) {
        return Err(FwError::VaUnaligned { offset });
    }
    if s.flags & !IFACE_SUPPORTED_FLAGS != 0 {
        return Err(FwError::UnsupportedFlags { offset, flags: s.flags });
    }
    if s.protected() {
        img.protected_skipped += 1;
        return Ok(());
    }
    if s.va_start == MCU_SHARED_REGION_START && !s.shared() {
        return Err(FwError::SharedNotShared { offset });
    }
    // ours
    if s.data_size() > s.va_size() {
        return Err(FwError::DataLargerThanVa { offset });
    }
    let loaded = &img.sections[..img.count];
    if s.va_size() != 0 && loaded.iter().any(|o| o.va_start < s.va_end && s.va_start < o.va_end) {
        return Err(FwError::Overlap { offset });
    }
    if img.count == MAX_SECTIONS {
        return Err(FwError::TooManySections);
    }
    if s.va_start == MCU_SHARED_REGION_START {
        img.shared = img.count;
    }
    img.sections[img.count] = s;
    img.count += 1;
    Ok(())
}

/// `panthor_fw_read_build_info`: `struct panthor_fw_build_info_hdr`
/// (meta_start, meta_size) names a string `git_sha: <hex>`, NUL-terminated,
/// somewhere in the file. Every defect in it is ignored, as panthor ignores
/// it ("We don't need the build info, so continue"); only a body too short
/// for the two words refuses, as panthor's iterator would.
fn build_info<'a>(img: &mut Image<'a>, offset: u32, body: &[u8]) -> Result<(), FwError> {
    if body.len() < 8 {
        return Err(FwError::ShortSection { offset });
    }
    let (start, size) = (word(body, 0) as usize, word(body, 4) as usize);
    let Some(meta) = start.checked_add(size).and_then(|e| img.bytes.get(start..e)) else {
        img.build_info_ignored += 1;
        return Ok(());
    };
    if meta.len() > GIT_SHA_PREFIX.len() && meta.starts_with(GIT_SHA_PREFIX) && meta[meta.len() - 1] == 0 {
        let s = &meta[GIT_SHA_PREFIX.len()..meta.len() - 1];
        let n = s.iter().rposition(|&c| c != b' ').map_or(0, |i| i + 1);
        img.git_sha = Some(&s[..n]);
    } else {
        img.build_info_ignored += 1;
    }
    Ok(())
}

/// Synthetic images for the host tests (here and in `csf`).
#[cfg(test)]
pub(crate) mod synth {
    use super::*;

    /// One section entry's words: ehdr, flags, va, data, then a name word.
    pub fn section(flags: u32, va: (u32, u32), data: (u32, u32)) -> [u32; 7] {
        [ENTRY_TYPE_IFACE | 28 << 8, flags, va.0, va.1, data.0, data.1, 0]
    }

    /// An image: the header, `entries` (each a list of words), then
    /// `payload` at `payload_at`.
    pub fn image(entries: &[&[u32]], payload_at: usize, payload: &[u8]) -> std::vec::Vec<u8> {
        let mut v = std::vec::Vec::new();
        let size = HEADER_SIZE + entries.iter().map(|e| e.len() * 4).sum::<usize>();
        for w in [MAGIC, 0x0000_0003, 0x0105_0000, 0, size as u32] {
            v.extend_from_slice(&w.to_le_bytes());
        }
        for e in entries {
            for w in e.iter() {
                v.extend_from_slice(&w.to_le_bytes());
            }
        }
        assert!(payload_at >= v.len());
        v.resize(payload_at, 0);
        v.extend_from_slice(payload);
        v
    }

    /// The board's file in miniature: code, read-only data, a zeroed
    /// region, a protected one, and the shared interface whose 0x2000
    /// bytes of data hold the GLB control block before boot.
    pub fn small() -> std::vec::Vec<u8> {
        let mut payload = std::vec![0u8; 0x2100];
        for (i, b) in payload[..0x100].iter_mut().enumerate() {
            *b = i as u8;
        }
        let shared = &mut payload[0x100..];
        // GLB control before boot, as the board's file has it: version 0,
        // input 0x04002000, output 0x04006000, 8 groups of stride 0xa0,
        // instr_features 0x71
        for (o, w) in [(8, 0x0400_2000u32), (12, 0x0400_6000), (16, 8), (20, 0xa0), (28, 0x71)] {
            shared[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
        image(
            &[
                &section(IFACE_RD | IFACE_EX | IFACE_CACHE_MODE_CACHED, (0x80_0000, 0x82_0000), (0x400, 0x480)),
                &section(IFACE_RD | IFACE_CACHE_MODE_CACHED, (0, 0x1_0000), (0x480, 0x500)),
                &section(IFACE_RD | IFACE_WR | IFACE_CACHE_MODE_CACHED | IFACE_ZERO, (0x200_0000, 0x204_0000), (0, 0)),
                &section(IFACE_RD | IFACE_WR | IFACE_PROT | IFACE_CACHE_MODE_CACHED, (0x300_0000, 0x304_0000), (0, 0)),
                &section(
                    IFACE_RD | IFACE_WR | IFACE_CACHE_MODE_CACHED_COHERENT | IFACE_SHARED | IFACE_ZERO,
                    (0x400_0000, 0x401_0000),
                    (0x500, 0x2500),
                ),
                // a trace buffer and an optional unknown entry: read past
                &[ENTRY_TYPE_TRACE_BUFFER | ENTRY_OPTIONAL | 8 << 8, 0],
                &[9 | ENTRY_OPTIONAL | 8 << 8, 0],
                // build info: "git_sha: 0123abcd \0" at 0x2500
                &[ENTRY_TYPE_BUILD_INFO_METADATA | ENTRY_OPTIONAL | 12 << 8, 0x2500, 19],
            ],
            0x400,
            &[&payload[..], b"git_sha: 0123abcd \0"].concat(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::synth::*;
    use super::*;

    #[test]
    fn a_synthetic_image_parses_section_by_section() {
        let b = small();
        let img = parse(&b).unwrap();
        assert_eq!(
            img.header,
            Header { minor: 3, major: 0, version_hash: 0x0105_0000, size: 20 + 5 * 28 + 8 + 8 + 12 }
        );
        let s = img.sections();
        assert_eq!(s.len(), 4);
        assert_eq!((s[0].va_start, s[0].va_size(), s[0].data_size()), (0x80_0000, 0x2_0000, 0x80));
        assert!(s[0].executable() && !s[0].writable() && s[0].gpu_cached());
        assert_eq!(s[1].data(&b), &(0x80..=0xff).collect::<std::vec::Vec<u8>>()[..]);
        assert!(s[2].zeroed() && s[2].writable() && s[2].data_size() == 0);
        assert_eq!(img.protected_skipped, 1);
        assert_eq!(img.shared, 3);
        let sh = img.shared_section();
        assert_eq!((sh.va_start, sh.va_size(), sh.data_size()), (MCU_SHARED_REGION_START, 0x1_0000, 0x2000));
        assert!(sh.shared() && sh.zeroed() && !sh.gpu_cached());
        assert_eq!((img.entries_ignored, img.build_info_ignored), (2, 0));
        assert_eq!(img.git_sha, Some(&b"0123abcd"[..]));
    }

    #[test]
    fn each_header_defect_is_refused_by_name() {
        let good = small();
        assert_eq!(parse(&good[..19]).unwrap_err(), FwError::TooSmall { len: 19 });
        let mut b = good.clone();
        b[0] ^= 1;
        assert_eq!(parse(&b).unwrap_err(), FwError::BadMagic(MAGIC ^ 1));
        let mut b = good.clone();
        b[5] = 1;
        assert_eq!(parse(&b).unwrap_err(), FwError::UnsupportedMajor(1));
        let mut b = good.clone();
        b[16..20].copy_from_slice(&(good.len() as u32 + 4).to_le_bytes());
        assert_eq!(parse(&b).unwrap_err(), FwError::Truncated { size: good.len() as u32 + 4, len: good.len() });
    }

    fn one(entries: &[&[u32]]) -> Result<(u32, u32), FwError> {
        let mut all: std::vec::Vec<&[u32]> = entries.to_vec();
        let shared =
            section(IFACE_RD | IFACE_WR | IFACE_SHARED | IFACE_ZERO, (MCU_SHARED_REGION_START, 0x401_0000), (0, 0));
        all.push(&shared);
        parse(&image(&all, 0x200, &[0; 0x100])).map(|i| (i.entries_ignored, i.build_info_ignored))
    }

    #[test]
    fn each_entry_defect_is_refused_by_name_in_panthors_order() {
        const RW: u32 = IFACE_RD | IFACE_WR;
        let at = HEADER_SIZE as u32;
        assert_eq!(one(&[&section(RW, (0, 0x1000), (8, 4))]), Err(FwError::DataReversed { offset: at }));
        assert_eq!(one(&[&section(RW, (0x2000, 0x1000), (0, 0))]), Err(FwError::VaReversed { offset: at }));
        assert_eq!(
            one(&[&section(RW, (0, 0x1000), (0x200, 0x400))]),
            Err(FwError::DataOutsideFile { offset: at, data_end: 0x400 })
        );
        assert_eq!(one(&[&section(RW, (0x800, 0x1000), (0, 0))]), Err(FwError::VaUnaligned { offset: at }));
        assert_eq!(
            one(&[&section(RW | 1 << 6, (0, 0x1000), (0, 0))]),
            Err(FwError::UnsupportedFlags { offset: at, flags: RW | 1 << 6 })
        );
        // a defect panthor checks before the protected-mode skip refuses a
        // protected section too; one it does not check is never reached
        assert_eq!(
            one(&[&section(RW | IFACE_PROT, (0x800, 0x1000), (0, 0))]),
            Err(FwError::VaUnaligned { offset: at })
        );
        assert_eq!(one(&[&section(RW | IFACE_PROT, (0, 0), (0x200, 0x204))]), Ok((0, 0)));
        assert_eq!(one(&[&section(RW, (0, 0), (0x200, 0x204))]), Err(FwError::DataLargerThanVa { offset: at }));
        assert_eq!(
            one(&[&section(RW, (0, 0x2000), (0, 0)), &section(RW, (0x1000, 0x3000), (0, 0))]),
            Err(FwError::Overlap { offset: at + 28 })
        );
        assert_eq!(one(&[&[ENTRY_TYPE_IFACE | 12 << 8, 0, 0]]), Err(FwError::ShortSection { offset: at }));
        assert_eq!(one(&[&[ENTRY_TYPE_IFACE | 6 << 8, 0]]), Err(FwError::Unaligned { offset: at, size: 6 }));
        assert_eq!(one(&[&[ENTRY_TYPE_CONFIG]]), Err(FwError::BadEntrySize { offset: at, size: 0 }));
        assert_eq!(one(&[&[5 | 8 << 8, 0]]), Err(FwError::UnknownEntry { offset: at, ehdr: 5 | 8 << 8 }));
        assert_eq!(one(&[&[5 | ENTRY_OPTIONAL | 8 << 8, 0]]), Ok((1, 0)));
        // build info: a range outside the file, a string without the prefix
        // and one without its NUL are ignored, as panthor ignores them
        assert_eq!(one(&[&[ENTRY_TYPE_BUILD_INFO_METADATA | 12 << 8, 0x2f0, 0x20]]), Ok((0, 1)));
        assert_eq!(one(&[&[ENTRY_TYPE_BUILD_INFO_METADATA | 12 << 8, 0x200, 4]]), Ok((0, 1)));
        assert_eq!(one(&[&[ENTRY_TYPE_BUILD_INFO_METADATA | 8 << 8, 0]]), Err(FwError::ShortSection { offset: at }));
        let mut all = std::vec::Vec::new();
        for i in 0..MAX_SECTIONS as u32 {
            all.push(section(IFACE_RD, (0x1000 * i, 0x1000 * i + 0x1000), (0, 0)));
        }
        let refs: std::vec::Vec<&[u32]> = all.iter().map(|e| &e[..]).collect();
        assert_eq!(one(&refs), Err(FwError::TooManySections));
    }

    #[test]
    fn a_build_string_without_its_nul_is_ignored() {
        let mut b = small();
        let n = b.len();
        b[n - 1] = b'x';
        let img = parse(&b).unwrap();
        assert_eq!((img.git_sha, img.build_info_ignored), (None, 1));
    }

    #[test]
    fn the_shared_region_must_be_there_and_shared() {
        let rw = IFACE_RD | IFACE_WR;
        let b = image(&[&section(rw, (0, 0x1000), (0, 0))], 0x100, &[]);
        assert_eq!(parse(&b).unwrap_err(), FwError::NoSharedSection);
        let b = image(&[&section(rw, (MCU_SHARED_REGION_START, 0x401_0000), (0, 0))], 0x100, &[]);
        assert_eq!(parse(&b).unwrap_err(), FwError::SharedNotShared { offset: HEADER_SIZE as u32 });
    }

    #[test]
    fn an_entry_list_cut_inside_an_entry_is_refused() {
        let mut b = small();
        // the header's size ends 4 bytes into the last entry
        let size = word(&b, 16) - 8;
        b[16..20].copy_from_slice(&size.to_le_bytes());
        assert!(matches!(parse(&b).unwrap_err(), FwError::BadEntrySize { size: 12, .. }));
    }

    /// The board's file, from `MALI_CSFFW` (tools/check.sh runs this with
    /// it set, and reports a gap when the file is absent; the general test
    /// row leaves it ignored, so a host without the firmware passes nothing
    /// in its name).
    #[test]
    #[ignore = "needs MALI_CSFFW, the board's mali_csffw.bin (tools/check.sh runs it, or reports a gap)"]
    fn the_boards_file_parses_into_the_design_pages_table() {
        let path = std::env::var("MALI_CSFFW").expect("MALI_CSFFW: the path of the board's mali_csffw.bin");
        let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(b.len(), 282_624, "{path}: not the board's file (282624 bytes)");
        let img = parse(&b).unwrap();
        assert_eq!(img.header, Header { minor: 3, major: 0, version_hash: 0x0105_0000, size: 792 });
        let table: std::vec::Vec<(u32, u32, u32)> =
            img.sections().iter().map(|s| (s.va_start, s.va_size(), s.flags)).collect();
        // driver/docs/nanopc-t6-mali.md, "The firmware (Z3)"
        assert_eq!(
            table,
            [
                (0x0040_0000, 0x1_0000, IFACE_RD | IFACE_CACHE_MODE_CACHED),
                (0x0041_0000, 0x1_0000, IFACE_RD | IFACE_CACHE_MODE_CACHED),
                (0x0000_0000, 0x1_0000, IFACE_RD | IFACE_CACHE_MODE_CACHED),
                (0x0080_0000, 0x2_0000, IFACE_RD | IFACE_EX | IFACE_CACHE_MODE_CACHED),
                (0x0200_0000, 0x4_0000, IFACE_RD | IFACE_WR | IFACE_CACHE_MODE_CACHED | IFACE_ZERO),
                (0x0100_0000, 0x4_0000, IFACE_RD | IFACE_CACHE_MODE_CACHED | IFACE_ZERO),
                (
                    0x0400_0000,
                    0x1_0000,
                    IFACE_RD | IFACE_WR | IFACE_CACHE_MODE_CACHED_COHERENT | IFACE_SHARED | IFACE_ZERO
                ),
            ]
        );
        assert_eq!((img.protected_skipped, img.shared), (1, 6));
        let sh = img.shared_section();
        assert_eq!((sh.data_start, sh.data_size()), (0x43000, 0x2000));
        // the GLB control block before boot: version 0 (the firmware writes
        // it), features 0, input and output pages, 8 groups of stride 0xa0,
        // instr_features 0x71 (what the board's dmesg prints after boot)
        let d = sh.data(&b);
        assert_eq!([0, 4, 8, 12, 16, 20, 28].map(|o| word(d, o)), [0, 0, 0x0400_2000, 0x0400_6000, 8, 0xa0, 0x71]);
        // group 0's control block, and its stream 0's
        assert_eq!([4, 8, 20, 24].map(|o| word(d, 0x1000 + o)), [0x0400_3000, 0x0400_7000, 8, 0xc]);
        assert_eq!([4, 8].map(|o| word(d, 0x1040 + o)), [0x0400_3400, 0x0400_7200]);
        assert_eq!(img.git_sha, Some(&b"95a25d71030715381f33105394285e1dcc860a65"[..]));
        // 2 config, 8 trace buffers, 1 timeline, 4 optional unknown (5, 7,
        // 8, 9); the one build-info entry is well formed
        assert_eq!((img.entries_ignored, img.build_info_ignored), (15, 0));
    }
}
