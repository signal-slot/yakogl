// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Z2's AS0 over every section the firmware loads (driver/docs/nanopc-t6-mali.md,
//! "The firmware (Z3)", "Open, before the board run"): each section
//! [`fw::parse`] keeps (the board's `mali_csffw.bin`: seven, 896 KiB; the
//! protected-mode one is skipped there) mapped at its MCU address onto one
//! backing of the image's, with the protection panthor gives it, then AS0
//! enabled by Z2's sequence ([`super::nanopc_t6_mmu::as0_up`]). [`McuMem`] is
//! the board's [`McuMemory`]: where a section is for the CPU, and the clean
//! the Zephyr side supplies (`sys_cache_data_flush_range`).
//!
//! The protection comes from the section's own flags, as panthor derives a
//! firmware section's VM map flags in `panthor_fw_load_section_entry`
//! (Linux v6.18 drivers/gpu/drm/panthor/panthor_fw.c:584-592): not
//! `CSF_FW_BINARY_IFACE_ENTRY_WR` is `MAP_READONLY`, not `_EX` is
//! `MAP_NOEXEC`, and every cache mode but `CACHED` is `MAP_UNCACHED` (its
//! TODO: the coherent modes are mapped non-cacheable for now); panthor_mmu.c
//! `flags_to_prot` turns those into `IOMMU_READ` always, `IOMMU_WRITE`
//! unless read-only, `IOMMU_NOEXEC`, `IOMMU_CACHE` unless uncached
//! ([`pgtable::Prot`]). The read flag is not looked at: panthor maps every
//! section readable. The descriptor each gives is io-pgtable's
//! (`arm_lpae_prot_to_pte`, [`pgtable::page_desc`]).
//!
//! On the board's file that is: the three read-only data sections, the zeroed
//! one at 0x01000000 and the code read-only; 0x02000000 and the shared
//! section writable; only the code executable; all GPU-cached but the shared
//! section, whose mode is `CACHED_COHERENT` (so uncached on the T6, which is
//! not coherent). The CPU side of the shared section must be an uncached
//! mapping too (csf.rs [`McuMemory`]); that is the Zephyr side's to set up.

use core::cell::Cell;

use super::nanopc_t6_mmu::{as0_up, MmuUp, Stop, AS_MCU};
use crate::gpu::mali::csf::McuMemory;
use crate::gpu::mali::fw::{self, Section, MAX_SECTIONS};
use crate::gpu::mali::mmu::{self, FlushOp, Op};
use crate::gpu::mali::pgtable::{Prot, Table, Tables, Walk, PAGE};
use crate::mmio::Mmio;

/// The protection panthor maps `s` with (see the module).
pub fn section_prot(s: &Section) -> Prot {
    Prot { write: s.writable(), exec: s.executable(), cached: s.gpu_cached() }
}

/// What one section maps to: `size` bytes at physical `pa`, which the CPU
/// reaches at `cpu`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Backing {
    pub pa: u64,
    pub cpu: usize,
    pub size: u64,
}

/// One section with its backing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub va: u32,
    pub size: u32,
    pub prot: Prot,
    pub backing: Backing,
}

const NONE: Placed = Placed {
    va: 0,
    size: 0,
    prot: Prot { write: false, exec: false, cached: false },
    backing: Backing { pa: 0, cpu: 0, size: 0 },
};

/// Why a placement was refused, with the value that decided it (the
/// section's MCU address, or for [`SectionRefusal::Count`] the number of
/// backings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SectionRefusal {
    /// No sections, more than [`MAX_SECTIONS`], or not one backing per section.
    Count = 1,
    /// A protected-mode section: [`fw::parse`] skips them, nothing maps one.
    Protected = 2,
    /// A backing smaller than its section.
    Small = 3,
    /// A backing's physical or CPU address not 4 KiB aligned.
    Unaligned = 4,
    /// Two backings share physical or CPU bytes.
    Overlap = 5,
}

/// The sections with their backings, checked: what [`mmu_up_sections`] maps
/// and [`McuMem`] looks up.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    list: [Placed; MAX_SECTIONS],
    count: usize,
}

fn overlaps(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < b.0 + b.1 && b.0 < a.0 + a.1
}

impl Placement {
    /// `backings[i]` for `sections[i]` (the image's [`fw::Image::sections`]).
    /// Every check runs before anything is mapped. A section of no size is
    /// kept and never mapped, as `csf::load_sections` skips it.
    pub fn new(sections: &[Section], backings: &[Backing]) -> Result<Placement, (SectionRefusal, u32)> {
        if sections.is_empty() || sections.len() > MAX_SECTIONS || backings.len() != sections.len() {
            return Err((SectionRefusal::Count, backings.len() as u32));
        }
        let mut p = Placement { list: [NONE; MAX_SECTIONS], count: 0 };
        for (s, b) in sections.iter().zip(backings) {
            if s.protected() {
                return Err((SectionRefusal::Protected, s.va_start));
            }
            if b.size < u64::from(s.va_size()) {
                return Err((SectionRefusal::Small, s.va_start));
            }
            if !b.pa.is_multiple_of(PAGE) || !(b.cpu as u64).is_multiple_of(PAGE) {
                return Err((SectionRefusal::Unaligned, s.va_start));
            }
            let size = u64::from(s.va_size());
            for q in p.placed() {
                let qs = u64::from(q.size);
                if overlaps((b.pa, size), (q.backing.pa, qs))
                    || overlaps((b.cpu as u64, size), (q.backing.cpu as u64, qs))
                {
                    return Err((SectionRefusal::Overlap, s.va_start));
                }
            }
            p.list[p.count] = Placed { va: s.va_start, size: s.va_size(), prot: section_prot(s), backing: *b };
            p.count += 1;
        }
        Ok(p)
    }

    pub fn placed(&self) -> &[Placed] {
        &self.list[..self.count]
    }

    /// The CPU address of `[va, va + len)`: inside one section, or `None`
    /// (outside every section, or across a section's end, even onto the
    /// next section's MCU address: their backings need not be adjacent).
    pub fn cpu(&self, va: u32, len: u32) -> Option<usize> {
        let p = self.placed().iter().find(|p| va >= p.va && va - p.va < p.size)?;
        let off = va - p.va;
        (u64::from(off) + u64::from(len) <= u64::from(p.size)).then(|| p.backing.cpu + off as usize)
    }
}

/// The MCU address the run walks in a section: page 1, offset 0x234 (for the
/// shared section, Z2's [`super::nanopc_t6_mmu::PROBE_VA`]), or page 0 of a
/// one-page section.
pub fn probe_va(p: &Placed) -> u64 {
    let off = if u64::from(p.size) > PAGE { 0x1234 } else { 0x234 };
    u64::from(p.va) + off
}

/// What [`mmu_up_sections`] read: Z2's record ([`MmuUp`]: `backing_pa` and
/// `walk` are the shared section's, `flush_pt` the first section's), the
/// walk of each section's [`probe_va`] and the flush after each section's
/// map, in the placement's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectionsUp {
    pub up: MmuUp,
    pub count: u32,
    pub walks: [Walk; MAX_SECTIONS],
    pub flush_pt: [Op; MAX_SECTIONS],
    /// Flushes after the map that completed (`count` when none stopped).
    pub flushed: u32,
}

/// Z2's run with every placed section mapped (see the module): AS0 as found,
/// the flush round trip, one `map` per section with [`section_prot`], a walk
/// of each section's [`probe_va`] back to its backing, the clean of the used
/// table pages, the enable, the read-back and fault check, then the flush
/// after the map. A map that refuses stops at [`Stop::Tables`] (`detail`:
/// the section's MCU address), a walk off its backing at [`Stop::Walk`].
///
/// The flush after the map is one per section, over that section's range:
/// panthor maps each firmware section as its own kernel BO
/// (`panthor_kernel_bo_create` in `panthor_fw_load_section_entry`), and its
/// `panthor_vm_map_pages` (panthor_mmu.c) flushes the range it has just
/// mapped, one map at a time; Z2's single-section run does the same once.
#[allow(clippy::too_many_arguments)]
pub fn mmu_up_sections(
    m: &impl Mmio,
    gpu: usize,
    mmu_features: u32,
    pool: &mut [Table],
    tables_pa: u64,
    placement: &Placement,
    clean: &mut impl FnMut(usize, usize) -> i32,
    delay_us: &mut impl FnMut(u32),
) -> SectionsUp {
    let placed = placement.placed();
    let mut s = SectionsUp { count: placed.len() as u32, ..SectionsUp::default() };
    s.up.tables_pa = tables_pa;
    let shared = placed.iter().position(|p| p.va == fw::MCU_SHARED_REGION_START);
    if let Some(i) = shared {
        s.up.backing_pa = placed[i].backing.pa;
    }
    let walks = &mut s.walks;
    let build = |t: &mut Tables, o: &mut MmuUp| {
        for p in placed.iter().filter(|p| p.size != 0) {
            if let Err(why) = t.map(u64::from(p.va), p.backing.pa, u64::from(p.size), p.prot) {
                o.stopped(Stop::Tables, why as u32, u64::from(p.va));
                return false;
            }
        }
        for (i, p) in placed.iter().enumerate().filter(|(_, p)| p.size != 0) {
            let va = probe_va(p);
            walks[i] = t.walk(va);
            if shared == Some(i) {
                o.walk = walks[i];
            }
            if walks[i].pa != Some(p.backing.pa + (va - u64::from(p.va))) {
                o.walk = walks[i];
                o.stopped(Stop::Walk, walks[i].levels, walks[i].pa.unwrap_or(0));
                return false;
            }
        }
        true
    };
    if !as0_up(m, gpu, mmu_features, pool, tables_pa, &mut s.up, build, clean, delay_us) {
        return s;
    }
    for (i, p) in placed.iter().enumerate().filter(|(_, p)| p.size != 0) {
        match mmu::hw_do_operation(m, gpu, AS_MCU, u64::from(p.va), u64::from(p.size), FlushOp::Pt, delay_us) {
            Ok(op) => s.flush_pt[i] = op,
            Err((why, v)) => {
                s.up.stopped(Stop::FlushPt, why as u32, u64::from(v));
                return s;
            }
        }
        s.flushed += 1;
    }
    s.up.flush_pt = s.flush_pt[0];
    s
}

/// The board's [`McuMemory`] over a [`Placement`]: `cpu` is
/// [`Placement::cpu`]; `clean` calls the given cleaner (on the board,
/// `sys_cache_data_flush_range`: a clean to the point of coherency) and
/// keeps the first rc that is not 0 ([`McuMem::clean_rc`]), since the
/// trait's clean returns nothing.
pub struct McuMem<'a, C: Fn(usize, usize) -> i32> {
    placement: &'a Placement,
    cleaner: C,
    rc: Cell<i32>,
}

impl<'a, C: Fn(usize, usize) -> i32> McuMem<'a, C> {
    pub fn new(placement: &'a Placement, cleaner: C) -> Self {
        McuMem { placement, cleaner, rc: Cell::new(0) }
    }
    /// 0, or the first failed clean's rc.
    pub fn clean_rc(&self) -> i32 {
        self.rc.get()
    }
}

impl<C: Fn(usize, usize) -> i32> McuMemory for McuMem<'_, C> {
    fn cpu(&self, va: u32, len: u32) -> Option<usize> {
        self.placement.cpu(va, len)
    }
    fn clean(&self, cpu: usize, len: usize) {
        let rc = (self.cleaner)(cpu, len);
        if rc != 0 && self.rc.get() == 0 {
            self.rc.set(rc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::mali::csf::load_sections;
    use crate::gpu::mali::fw::synth::{image, section};
    use crate::gpu::mali::fw::*;
    use crate::gpu::mali::mmu::tests::{Gpu, BASE};
    use crate::gpu::mali::mmu::{mmu_as, AS_COMMAND, AS_COMMAND_LOCK, AS_COMMAND_UNLOCK, AS_COMMAND_UPDATE};
    use crate::gpu::mali::pgtable::{PtRefusal, ENTRIES};
    use crate::gpu::mali::GPU_CMD;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const TABLES_PA: u64 = 0x1040_0000;
    /// Where the CPU reaches the backings in the tests (any 4 KiB aligned
    /// address: the RAM model below is keyed by it).
    const CPU: usize = 0x4000_0000;

    /// The board's file as `fw::parse` lists it (the order of the file),
    /// with the expectation of each: (MCU VA, size, flags, the backing's
    /// PA, writable, executable, GPU-cached, the level-3 descriptor of the
    /// first and of the last page). The backings lie one after the other
    /// in VA order from 0x10500000, as the probe's `.bss` would hold them;
    /// 896 KiB end at 0x105e0000. The descriptor words are spelled out:
    /// 0xfc7 read only, cached (`AP[2]`, `AttrIndx` 1, inner shareable),
    /// 0xf47 the same writable, 0xe43 writable and uncached (`AttrIndx` 0,
    /// outer shareable), bits 54:53 `XN` on all but the code.
    #[allow(clippy::type_complexity)]
    const BOARD: [(u32, u32, u32, u64, bool, bool, bool, u64, u64); 7] = [
        (
            0x0040_0000,
            0x1_0000,
            IFACE_RD | IFACE_CACHE_MODE_CACHED,
            0x1051_0000,
            false,
            false,
            true,
            0x0060_0000_1051_0fc7,
            0x0060_0000_1051_ffc7,
        ),
        (
            0x0041_0000,
            0x1_0000,
            IFACE_RD | IFACE_CACHE_MODE_CACHED,
            0x1052_0000,
            false,
            false,
            true,
            0x0060_0000_1052_0fc7,
            0x0060_0000_1052_ffc7,
        ),
        (
            0x0000_0000,
            0x1_0000,
            IFACE_RD | IFACE_CACHE_MODE_CACHED,
            0x1050_0000,
            false,
            false,
            true,
            0x0060_0000_1050_0fc7,
            0x0060_0000_1050_ffc7,
        ),
        (
            0x0080_0000,
            0x2_0000,
            IFACE_RD | IFACE_EX | IFACE_CACHE_MODE_CACHED,
            0x1053_0000,
            false,
            true,
            true,
            0x0000_0000_1053_0fc7,
            0x0000_0000_1054_ffc7,
        ),
        (
            0x0200_0000,
            0x4_0000,
            IFACE_RD | IFACE_WR | IFACE_CACHE_MODE_CACHED | IFACE_ZERO,
            0x1059_0000,
            true,
            false,
            true,
            0x0060_0000_1059_0f47,
            0x0060_0000_105c_ff47,
        ),
        (
            0x0100_0000,
            0x4_0000,
            IFACE_RD | IFACE_CACHE_MODE_CACHED | IFACE_ZERO,
            0x1055_0000,
            false,
            false,
            true,
            0x0060_0000_1055_0fc7,
            0x0060_0000_1058_ffc7,
        ),
        (
            0x0400_0000,
            0x1_0000,
            IFACE_RD | IFACE_WR | IFACE_CACHE_MODE_CACHED_COHERENT | IFACE_SHARED | IFACE_ZERO,
            0x105d_0000,
            true,
            false,
            false,
            0x0060_0000_105d_0e43,
            0x0060_0000_105d_fe43,
        ),
    ];

    /// The board's section list in a synthetic file: each section of
    /// [`BOARD`] with some data (none for the zeroed ones but the shared
    /// section's 0x2000 bytes), plus the protected section parse skips.
    fn board_file() -> Vec<u8> {
        let mut entries = Vec::new();
        let mut at = 0x1000u32;
        for &(va, size, flags, ..) in &BOARD {
            let n = match va {
                0x0400_0000 => 0x2000,
                _ if flags & IFACE_ZERO != 0 => 0,
                _ => 0x300 + (va >> 16),
            };
            entries.push(section(flags, (va, va + size), (at, at + n)));
            at += n;
        }
        entries.insert(
            5,
            section(IFACE_RD | IFACE_WR | IFACE_PROT | IFACE_CACHE_MODE_CACHED, (0x0300_0000, 0x0304_0000), (0, 0)),
        );
        let refs: Vec<&[u32]> = entries.iter().map(|e| &e[..]).collect();
        let payload: Vec<u8> = (0..at - 0x1000).map(|i| (i * 7 + 3) as u8).collect();
        image(&refs, 0x1000, &payload)
    }

    fn backings(sections: &[Section]) -> Vec<Backing> {
        sections
            .iter()
            .map(|s| {
                let pa = BOARD.iter().find(|b| b.0 == s.va_start).unwrap().3;
                Backing { pa, cpu: CPU + (pa - 0x1050_0000) as usize, size: u64::from(s.va_size()) }
            })
            .collect()
    }

    fn run(g: &Gpu, pool: &mut [Table], p: &Placement, cleans: &mut Vec<(usize, usize)>) -> SectionsUp {
        mmu_up_sections(
            g,
            BASE,
            0x2830,
            pool,
            TABLES_PA,
            p,
            &mut |a, n| {
                cleans.push((a, n));
                0
            },
            &mut |_| {},
        )
    }

    #[test]
    fn the_protection_is_the_sections_own_flags_as_panthor_reads_them() {
        let b = board_file();
        let img = parse(&b).unwrap();
        assert_eq!(img.protected_skipped, 1);
        assert_eq!(img.sections().len(), 7);
        for (s, want) in img.sections().iter().zip(&BOARD) {
            assert_eq!((s.va_start, s.va_size(), s.flags), (want.0, want.1, want.2));
            assert_eq!(section_prot(s), Prot { write: want.4, exec: want.5, cached: want.6 }, "{:#x}", s.va_start);
        }
        // the read flag decides nothing (panthor maps every section readable),
        // and only CACHED is cached: the coherent modes are not on the T6
        let s = |flags| Section { flags, va_start: 0, va_end: 0x1000, ..Section::default() };
        assert_eq!(section_prot(&s(IFACE_WR)), section_prot(&s(IFACE_RD | IFACE_WR)));
        for mode in [IFACE_CACHE_MODE_NONE, IFACE_CACHE_MODE_UNCACHED_COHERENT, IFACE_CACHE_MODE_CACHED_COHERENT] {
            assert!(!section_prot(&s(IFACE_RD | mode)).cached);
        }
        assert_eq!(img.sections().iter().map(|s| s.va_size()).sum::<u32>(), 896 * 1024);
    }

    #[test]
    fn every_section_comes_up_in_as0_on_nine_table_pages() {
        let b = board_file();
        let img = parse(&b).unwrap();
        let p = Placement::new(img.sections(), &backings(img.sections())).unwrap();
        let g = Gpu::new(1);
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut cleans = vec![];
        let o = run(&g, &mut pool, &p, &mut cleans);
        assert_eq!((o.up.stop, o.up.refusal, o.up.detail), (Stop::None, 0, 0));
        assert_eq!((o.count, o.flushed), (7, 7));
        // 1 L0, 1 L1, 1 L2, 6 L3 (0x400000 and 0x410000 share one): 9 of 16
        assert_eq!(o.up.tables_used, 9);
        assert_eq!(cleans, vec![(pool.as_ptr() as usize, 9 * 4096)]);
        assert!(pool[9..].iter().all(|t| t.iter().all(|&d| d == 0)));
        // the shared section's walk is Z2's
        assert_eq!(o.up.backing_pa, 0x105d_0000);
        assert_eq!(o.up.walk.pa, Some(0x105d_1234));
        assert_eq!(o.up.walk.desc[3], 0x0060_0000_105d_1e43);
        // each section's probe walks back to its backing
        for (i, want) in BOARD.iter().enumerate() {
            assert_eq!(o.walks[i].pa, Some(want.3 + 0x1234), "{:#x}", want.0);
        }
        // the flush after the map, one per section over its range
        let locks: Vec<u64> = o.flush_pt[..7].iter().map(|op| op.lockaddr).collect();
        assert_eq!(locks, [0x0040_000f, 0x0041_000f, 0x0000_000f, 0x0080_0010, 0x0200_0011, 0x0100_0011, 0x0400_000f]);
        let a = mmu_as(0) + AS_COMMAND;
        let cmds: Vec<(usize, u32)> = g.sequence().into_iter().filter(|w| w.0 == GPU_CMD || w.0 == a).collect();
        let mut want = vec![
            (GPU_CMD, 0x3304),
            (a, AS_COMMAND_LOCK),
            (GPU_CMD, 0x3304),
            (a, AS_COMMAND_UNLOCK),
            (a, AS_COMMAND_UPDATE),
        ];
        for _ in 0..7 {
            want.extend([(a, AS_COMMAND_LOCK), (GPU_CMD, 0x0304), (a, AS_COMMAND_UNLOCK)]);
        }
        assert_eq!(cmds, want);
        assert_eq!(
            (o.up.after.transtab, o.up.after.memattr, o.up.after.transcfg),
            (TABLES_PA, 0x8f8f_8f8f_8c4c_8f4c, 0x4200_01c6)
        );
    }

    #[test]
    fn every_page_descriptor_follows_panthors_rules() {
        let b = board_file();
        let img = parse(&b).unwrap();
        let p = Placement::new(img.sections(), &backings(img.sections())).unwrap();
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut t = Tables::new(&mut pool, TABLES_PA).unwrap();
        for q in p.placed() {
            t.map(u64::from(q.va), q.backing.pa, u64::from(q.size), q.prot).unwrap();
        }
        for &(va, size, _, pa, _, _, _, first, last) in &BOARD {
            let (va, size) = (u64::from(va), u64::from(size));
            assert_eq!(t.walk(va).desc[3], first, "{va:#x}");
            assert_eq!(t.walk(va + size - PAGE).desc[3], last, "{va:#x}");
            // every page in between: the first page's attributes over its own PA
            let attrs = first & !(pa & 0x0000_ffff_ffff_f000);
            for off in (0..size).step_by(PAGE as usize) {
                let w = t.walk(va + off);
                assert_eq!(w.desc[3], attrs | (pa + off), "{:#x}", va + off);
                assert_eq!(w.pa, Some(pa + off));
            }
        }
        // the protected section's range and the gaps stay unmapped
        for va in [0x0001_0000, 0x0042_0000, 0x0082_0000, 0x0104_0000, 0x0204_0000, 0x0300_0000, 0x0401_0000] {
            assert_eq!(t.walk(va).pa, None, "{va:#x}");
        }
        assert_eq!(t.used(), 9);
    }

    #[test]
    fn the_cpu_side_is_the_backing_inside_one_section_only() {
        let b = board_file();
        let img = parse(&b).unwrap();
        let p = Placement::new(img.sections(), &backings(img.sections())).unwrap();
        let mem = McuMem::new(&p, |_, _| 0);
        let sh = CPU + 0xd_0000;
        assert_eq!(mem.cpu(0x0400_0000, 0x1_0000), Some(sh));
        assert_eq!(mem.cpu(0x0400_fffc, 4), Some(sh + 0xfffc));
        assert_eq!(mem.cpu(0x0400_fffc, 8), None, "across the shared section's end");
        assert_eq!(mem.cpu(0x0401_0000, 4), None, "past the last section");
        assert_eq!(mem.cpu(0x0000_0000, 4), Some(CPU));
        assert_eq!(mem.cpu(0x0000_ffff, 1), Some(CPU + 0xffff));
        assert_eq!(mem.cpu(0x0001_0000, 1), None, "a gap between sections");
        // 0x0040ffff and 0x00410000 are adjacent MCU addresses of two
        // sections: a range across them is refused all the same
        assert_eq!(mem.cpu(0x0040_fff0, 0x20), None);
        assert_eq!(mem.cpu(0x0041_0000, 0x1_0000), Some(CPU + 0x2_0000));
        assert_eq!(mem.cpu(0x0080_0000, 0x2_0001), None);
        assert_eq!(mem.cpu(0x0200_0000, 0x4_0000), Some(CPU + 0x9_0000));
        assert_eq!(mem.cpu(0x0300_0000, 4), None, "the protected section is not mapped");
        assert_eq!(mem.cpu(u32::MAX, 1), None);
    }

    #[test]
    fn a_bad_backing_or_a_small_pool_is_refused_before_as0_is_touched() {
        let b = board_file();
        let img = parse(&b).unwrap();
        let s = img.sections();
        let good = backings(s);
        let with = |i: usize, f: &dyn Fn(&mut Backing)| {
            let mut v = good.clone();
            f(&mut v[i]);
            Placement::new(s, &v).map(|_| ())
        };
        // 0x00800000, the code: 128 KiB
        assert_eq!(with(3, &|b| b.size = 0x1_f000), Err((SectionRefusal::Small, 0x0080_0000)));
        assert_eq!(with(3, &|b| b.pa += 0x800), Err((SectionRefusal::Unaligned, 0x0080_0000)));
        assert_eq!(with(3, &|b| b.cpu += 8), Err((SectionRefusal::Unaligned, 0x0080_0000)));
        // the code's backing over the end of 0x00000000's (in the list before it)
        assert_eq!(with(3, &|b| b.pa = 0x1050_f000), Err((SectionRefusal::Overlap, 0x0080_0000)));
        assert_eq!(with(3, &|b| b.cpu = CPU + 0xf000), Err((SectionRefusal::Overlap, 0x0080_0000)));
        // a backing larger than its section is fine
        assert_eq!(with(6, &|b| b.size = 0x2_0000), Ok(()));
        assert_eq!(Placement::new(s, &good[..6]).map(|_| ()), Err((SectionRefusal::Count, 6)));
        assert_eq!(Placement::new(&[], &[]).map(|_| ()), Err((SectionRefusal::Count, 0)));
        let prot =
            Section { flags: IFACE_RD | IFACE_PROT, va_start: 0x0300_0000, va_end: 0x0304_0000, ..Section::default() };
        assert_eq!(
            Placement::new(&[prot], &[Backing { pa: 0x1060_0000, cpu: CPU, size: 0x4_0000 }]).map(|_| ()),
            Err((SectionRefusal::Protected, 0x0300_0000))
        );

        // 8 pages hold the tables of the first six sections in the file's
        // order, not the shared section's, the last
        let p = Placement::new(s, &good).unwrap();
        let g = Gpu::new(0);
        let mut pool = vec![[0u64; ENTRIES]; 8];
        let mut cleans = vec![];
        let o = run(&g, &mut pool, &p, &mut cleans);
        assert_eq!((o.up.stop, o.up.refusal, o.up.detail), (Stop::Tables, PtRefusal::PoolFull as u32, 0x0400_0000));
        assert_eq!(o.up.tables_used, 8);
        assert!(cleans.is_empty());
        assert!(g.writes.borrow().iter().all(|w| !(0x2400..0x2440).contains(&w.0)), "AS0 written");
    }

    /// RAM at CPU addresses, for `load_sections`' writes.
    #[derive(Default)]
    struct Ram(RefCell<HashMap<usize, u8>>);

    impl Mmio for Ram {
        fn read32(&self, a: usize) -> u32 {
            u32::from_le_bytes([0, 1, 2, 3].map(|i| self.read8(a + i)))
        }
        fn write32(&self, a: usize, v: u32) {
            for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                self.write8(a + i, b);
            }
        }
        fn read8(&self, a: usize) -> u8 {
            *self.0.borrow().get(&a).unwrap_or(&0xa5)
        }
        fn write8(&self, a: usize, v: u8) {
            self.0.borrow_mut().insert(a, v);
        }
    }

    /// Load `img` through [`McuMem`] and check that every section's bytes
    /// are its data then zeroes, at its backing, and that the cleans cover
    /// every byte written.
    fn load_and_check(img: &Image, p: &Placement) {
        let cleans = RefCell::new(vec![]);
        let mem = McuMem::new(p, |a, n| {
            cleans.borrow_mut().push((a, n));
            0
        });
        let ram = Ram::default();
        load_sections(&ram, &mem, img).unwrap();
        assert_eq!(mem.clean_rc(), 0);
        let cleans = cleans.into_inner();
        // one clean per section that has data or is zeroed, over all of it
        let want: Vec<(usize, usize)> = img
            .sections()
            .iter()
            .filter(|s| s.data_size() != 0 || s.zeroed())
            .map(|s| (p.cpu(s.va_start, s.va_size()).unwrap(), s.va_size() as usize))
            .collect();
        assert_eq!(cleans, want);
        let written = ram.0.borrow();
        for a in written.keys() {
            assert!(cleans.iter().any(|&(c, n)| (c..c + n).contains(a)), "{a:#x} written, not cleaned");
        }
        for s in img.sections() {
            let cpu = p.cpu(s.va_start, s.va_size()).unwrap();
            let data = s.data(img.bytes);
            for (i, &b) in data.iter().enumerate() {
                assert_eq!(written.get(&(cpu + i)), Some(&b));
            }
            if s.zeroed() {
                assert!(
                    (data.len()..s.va_size() as usize).all(|i| written.get(&(cpu + i)) == Some(&0)),
                    "{:#x}",
                    s.va_start
                );
            }
        }
        // and nothing else was written
        let loaded: usize =
            img.sections().iter().map(|s| if s.zeroed() { s.va_size() } else { s.data_size() } as usize).sum();
        assert_eq!(written.len(), loaded);
    }

    #[test]
    fn load_sections_through_the_board_memory_cleans_every_loaded_byte() {
        let b = board_file();
        let img = parse(&b).unwrap();
        let p = Placement::new(img.sections(), &backings(img.sections())).unwrap();
        load_and_check(&img, &p);
        // a failed clean is kept, the first one
        let n = Cell::new(0i32);
        let mem = McuMem::new(&p, |_, _| {
            n.set(n.get() + 1);
            -n.get()
        });
        load_sections(&Ram::default(), &mem, &img).unwrap();
        assert_eq!(mem.clean_rc(), -1);
    }

    /// The board's file (`MALI_CSFFW`, `tools/cts/deps-env.sh --build
    /// MALI_CSFFW` fetches it): its parsed sections mapped and loaded as
    /// above. Skipped, with a line on stderr, when `MALI_CSFFW` is not set
    /// or names no file.
    #[test]
    fn the_boards_file_maps_into_as0_section_by_section() {
        let Some(path) = std::env::var_os("MALI_CSFFW") else {
            eprintln!("skipped: MALI_CSFFW (the board's mali_csffw.bin) is not set");
            return;
        };
        if !std::path::Path::new(&path).is_file() {
            eprintln!("skipped: MALI_CSFFW={} is not a file", path.to_string_lossy());
            return;
        }
        let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.to_string_lossy()));
        let img = parse(&b).unwrap();
        let p = Placement::new(img.sections(), &backings(img.sections())).unwrap();
        let g = Gpu::new(0);
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut cleans = vec![];
        let o = run(&g, &mut pool, &p, &mut cleans);
        assert_eq!((o.up.stop, o.up.tables_used, o.count, o.flushed), (Stop::None, 9, 7, 7));
        assert_eq!(o.up.walk.desc[3], 0x0060_0000_105d_1e43);
        // the descriptors of the first and last page of each section
        let mut pool = vec![[0u64; ENTRIES]; 16];
        let mut t = Tables::new(&mut pool, TABLES_PA).unwrap();
        for q in p.placed() {
            t.map(u64::from(q.va), q.backing.pa, u64::from(q.size), q.prot).unwrap();
        }
        for (s, want) in img.sections().iter().zip(&BOARD) {
            assert_eq!((s.va_start, s.va_size(), s.flags), (want.0, want.1, want.2));
            let (va, size) = (u64::from(s.va_start), u64::from(s.va_size()));
            assert_eq!((t.walk(va).desc[3], t.walk(va + size - PAGE).desc[3]), (want.7, want.8), "{va:#x}");
        }
        load_and_check(&img, &p);
    }
}
