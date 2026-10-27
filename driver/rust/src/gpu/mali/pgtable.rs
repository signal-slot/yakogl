// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The page tables of a Mali address space, built by the CPU (Z2,
//! driver/docs/nanopc-t6-mali.md "Z2: AS0 and a cache flush").
//!
//! panthor gives a VM's tables to io-pgtable as `ARM_64_LPAE_S1`, the Arm
//! VMSAv8-64 stage-1 format, with a 4 KiB granule (panthor_mmu.c
//! `panthor_vm_create`; `AS_TRANSCFG_ADRMODE_AARCH64_4K` in
//! `panthor_vm_active`), not panfrost's `ARM_MALI_LPAE`. With the G610's
//! 48-bit VA (`MMU_FEATURES` 0x2830) that is four levels of 512 eight-byte
//! descriptors, each table one 4 KiB page; the root's physical address is
//! `AS_TRANSTAB` as it is (io-pgtable's `arm_lpae_s1_cfg.ttbr`).
//!
//! io-pgtable-arm.c is GPL only: what is here are the architecture's
//! descriptor fields (Arm ARM, VMSAv8-64 descriptors), with io-pgtable's
//! choices for a stage-1 table stated as facts with their place
//! (`arm_lpae_prot_to_pte`, `arm_lpae_install_table`), not its code. The
//! protection a mapping asks for is panthor's `flags_to_prot`
//! (panthor_mmu.c, MIT option).
//!
//! The builder never allocates: it takes a pool of table pages the image
//! owns and the pool's physical address, and finds a table again from the
//! address its parent descriptor holds, as the GPU's walk does.

/// A granule, and a table: 512 descriptors of 8 bytes.
pub const PAGE: u64 = 4096;
pub const ENTRIES: usize = 512;
pub type Table = [u64; ENTRIES];

/// The VA and PA widths of the G610 (`MMU_FEATURES` 0x2830: bits 7:0 VA,
/// 15:8 PA; panthor_regs.h `GPU_MMU_FEATURES_VA_BITS`/`_PA_BITS`).
pub const VA_BITS: u32 = 48;
pub const PA_BITS: u32 = 40;
/// Levels for 48 bits of VA with 9 bits a level over a 12-bit page.
pub const LEVELS: usize = 4;

/// Descriptor bits (VMSAv8-64 stage 1, 4 KiB granule).
pub mod desc {
    /// Bits 1:0. A table at levels 0-2 and a page at level 3 are both 0b11.
    pub const VALID: u64 = 1 << 0;
    pub const TABLE: u64 = 0b11;
    pub const PAGE: u64 = 0b11;
    /// `AttrIndx[2:0]`, bits 4:2: the byte of `MAIR` (here `AS_MEMATTR`).
    pub const ATTRINDX_SHIFT: u32 = 2;
    /// `AP[1]`, bit 6: unprivileged access too. `AP[2]`, bit 7: read only.
    pub const AP_UNPRIV: u64 = 1 << 6;
    pub const AP_RDONLY: u64 = 1 << 7;
    /// `SH[1:0]`, bits 9:8: outer shareable 0b10, inner shareable 0b11.
    pub const SH_OS: u64 = 2 << 8;
    pub const SH_IS: u64 = 3 << 8;
    /// The access flag, bit 10, and not-global, bit 11.
    pub const AF: u64 = 1 << 10;
    pub const NG: u64 = 1 << 11;
    /// `PXN` bit 53 and `UXN`/`XN` bit 54.
    pub const XN: u64 = 3 << 53;
    /// The output address, bits 47:12 (4 KiB granule).
    pub const ADDR: u64 = 0x0000_ffff_ffff_f000;
}

/// io-pgtable's `MAIR` bytes for a stage-1 table (io-pgtable-arm.c,
/// `ARM_LPAE_MAIR_ATTR_IDX_*` and the MAIR its stage-1 init builds): index 0
/// normal non-cacheable (0x44), 1 normal write-back (0xff), 2 device nGnRE
/// (0x04), 3 inner non-cacheable outer write-back (0xf4); 4-7 unused (0).
pub const ATTR_NC: u64 = 0;
pub const ATTR_CACHE: u64 = 1;
pub const ATTR_DEV: u64 = 2;
pub const MAIR: u64 = 0x44 | 0xff << 8 | 0x04 << 16 | 0xf4 << 24;

/// What a mapping asks for, as panthor's VM_BIND flags say it
/// (`flags_to_prot`: not `MAP_NOEXEC` is executable, not `MAP_UNCACHED` is
/// `IOMMU_CACHE`, not `MAP_READONLY` is writable; panthor never asks for
/// `IOMMU_PRIV`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prot {
    pub write: bool,
    pub exec: bool,
    pub cached: bool,
}

/// The level-3 descriptor of a page at `pa` (4 KiB aligned) with `prot`:
/// the fields io-pgtable's `arm_lpae_prot_to_pte` and `arm_lpae_init_pte`
/// set for a stage-1 page: `nG`; `AP[1]` always (unprivileged); `AP[2]`
/// when not writable; `AttrIndx` the cached or the non-cacheable index;
/// inner shareable for a cached page, outer shareable otherwise; `XN` when
/// not executable; `AF`; the page type.
pub const fn page_desc(pa: u64, prot: Prot) -> u64 {
    let mut d = (pa & desc::ADDR) | desc::PAGE | desc::AF | desc::NG | desc::AP_UNPRIV;
    if !prot.write {
        d |= desc::AP_RDONLY;
    }
    if prot.cached {
        d |= ATTR_CACHE << desc::ATTRINDX_SHIFT | desc::SH_IS;
    } else {
        d |= ATTR_NC << desc::ATTRINDX_SHIFT | desc::SH_OS;
    }
    if !prot.exec {
        d |= desc::XN;
    }
    d
}

/// A table descriptor: the next table's address and the table type
/// (`arm_lpae_install_table`; `NSTable` only with a quirk panthor does not
/// set).
pub const fn table_desc(pa: u64) -> u64 {
    (pa & desc::ADDR) | desc::TABLE
}

/// The index of `va` at `level` (0 is the root): bits 47:39, 38:30, 29:21,
/// 20:12.
pub const fn index(va: u64, level: usize) -> usize {
    ((va >> (12 + 9 * (LEVELS - 1 - level) as u32)) & 0x1ff) as usize
}

/// Why a build stopped. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PtRefusal {
    /// The pool is empty, or its physical address is not 4 KiB aligned or
    /// does not fit [`PA_BITS`] with all its pages.
    BadPool = 1,
    /// A VA, PA or size is not 4 KiB aligned, or the size is 0.
    Unaligned = 2,
    /// The VA range leaves [`VA_BITS`], or the PA range [`PA_BITS`].
    OutOfRange = 3,
    /// Another table is needed and the pool has none left.
    PoolFull = 4,
    /// The page is mapped already: a mapping is never replaced.
    AlreadyMapped = 5,
    /// A table descriptor points outside the pool (it cannot be followed).
    ForeignTable = 6,
}

/// The tables of one address space in a pool of pages.
pub struct Tables<'a> {
    pages: &'a mut [Table],
    pa: u64,
    used: usize,
}

/// What [`Tables::walk`] read: the descriptor at each level, up to the
/// first that is not a table, and the address `va` translates to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Walk {
    pub desc: [u64; LEVELS],
    /// Levels read (1..=4).
    pub levels: u32,
    pub pa: Option<u64>,
}

impl<'a> Tables<'a> {
    /// Take `pages` at physical `pa` and make page 0 the root, every
    /// descriptor of every page invalid.
    pub fn new(pages: &'a mut [Table], pa: u64) -> Result<Tables<'a>, PtRefusal> {
        let end = pa.checked_add(pages.len() as u64 * PAGE);
        if pages.is_empty() || !pa.is_multiple_of(PAGE) || end.is_none_or(|e| e > 1 << PA_BITS) {
            return Err(PtRefusal::BadPool);
        }
        for t in pages.iter_mut() {
            t.fill(0);
        }
        Ok(Tables { pages, pa, used: 1 })
    }

    /// `AS_TRANSTAB`: the root's physical address.
    pub fn root_pa(&self) -> u64 {
        self.pa
    }

    /// Pages of the pool in use (the root counts).
    pub fn used(&self) -> usize {
        self.used
    }

    /// The pages in use, for the cache clean before the GPU walks them.
    pub fn used_pages(&self) -> &[Table] {
        &self.pages[..self.used]
    }

    /// The page of the pool a table descriptor's address names.
    fn page_of(&self, d: u64) -> Result<usize, PtRefusal> {
        let a = d & desc::ADDR;
        if a < self.pa || (a - self.pa) / PAGE >= self.used as u64 {
            return Err(PtRefusal::ForeignTable);
        }
        Ok(((a - self.pa) / PAGE) as usize)
    }

    /// Map `size` bytes at `va` to `pa` with `prot`, one level-3 page per
    /// 4 KiB. All checks run before the first descriptor is written, except
    /// a full pool, which can stop a mapping half done (the tables then hold
    /// what was mapped; nothing invalid).
    pub fn map(&mut self, va: u64, pa: u64, size: u64, prot: Prot) -> Result<(), PtRefusal> {
        if size == 0 || !va.is_multiple_of(PAGE) || !pa.is_multiple_of(PAGE) || !size.is_multiple_of(PAGE) {
            return Err(PtRefusal::Unaligned);
        }
        let in_range = |base: u64, bits: u32| base.checked_add(size).is_some_and(|e| e <= 1 << bits);
        if !in_range(va, VA_BITS) || !in_range(pa, PA_BITS) {
            return Err(PtRefusal::OutOfRange);
        }
        for off in (0..size).step_by(PAGE as usize) {
            if self.walk(va + off).pa.is_some() {
                return Err(PtRefusal::AlreadyMapped);
            }
        }
        for off in (0..size).step_by(PAGE as usize) {
            let v = va + off;
            let mut t = 0usize;
            for level in 0..LEVELS - 1 {
                let i = index(v, level);
                let d = self.pages[t][i];
                t = if d & desc::VALID != 0 {
                    self.page_of(d)?
                } else {
                    if self.used == self.pages.len() {
                        return Err(PtRefusal::PoolFull);
                    }
                    let n = self.used;
                    self.used += 1;
                    self.pages[t][i] = table_desc(self.pa + n as u64 * PAGE);
                    n
                };
            }
            self.pages[t][index(v, LEVELS - 1)] = page_desc(pa + off, prot);
        }
        Ok(())
    }

    /// Walk `va` from the root as the GPU would: follow table descriptors
    /// through levels 0-2, then the page at level 3.
    pub fn walk(&self, va: u64) -> Walk {
        let mut w = Walk::default();
        let mut t = 0usize;
        for level in 0..LEVELS {
            let d = self.pages[t][index(va, level)];
            w.desc[level] = d;
            w.levels = level as u32 + 1;
            if d & desc::VALID == 0 {
                return w;
            }
            if level == LEVELS - 1 {
                if d & 0b11 == desc::PAGE {
                    w.pa = Some((d & desc::ADDR) | (va & (PAGE - 1)));
                }
                return w;
            }
            // levels 0-2 hold only tables here (no blocks are built)
            if d & 0b11 != desc::TABLE {
                return w;
            }
            match self.page_of(d) {
                Ok(n) => t = n,
                Err(_) => return w,
            }
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POOL_PA: u64 = 0x1040_0000;
    const RW_CACHED: Prot = Prot { write: true, exec: false, cached: true };

    fn pool(n: usize) -> Vec<Table> {
        vec![[0xdead_beef_dead_beef; ENTRIES]; n]
    }

    #[test]
    fn a_cached_writable_page_has_the_fields_io_pgtable_sets() {
        // nG | AF | SH_IS | AP_UNPRIV | AttrIndx 1 | page, XN
        assert_eq!(page_desc(0x1050_0000, RW_CACHED), 0x0060_0000_1050_0f47);
        // read only, executable, uncached: AP[2], AttrIndx 0, outer shareable, no XN
        assert_eq!(page_desc(0x1050_0000, Prot { write: false, exec: true, cached: false }), 0x0000_0000_1050_0ec3);
        assert_eq!(table_desc(0x1040_1000), 0x1040_1003);
        // the low bits of an address never reach the descriptor's attributes
        assert_eq!(page_desc(0x1050_0fff, RW_CACHED), page_desc(0x1050_0000, RW_CACHED));
    }

    #[test]
    fn the_indices_of_a_48_bit_va() {
        let va = 0x0000_8040_2010_1234u64;
        assert_eq!([index(va, 0), index(va, 1), index(va, 2), index(va, 3)], [0x100, 0x100, 0x100, 0x101]);
        assert_eq!(
            [index(0x0400_1234, 0), index(0x0400_1234, 1), index(0x0400_1234, 2), index(0x0400_1234, 3)],
            [0, 0, 0x20, 1]
        );
    }

    #[test]
    fn the_mcu_shared_section_maps_in_four_tables_and_walks_back() {
        let mut p = pool(8);
        let mut t = Tables::new(&mut p, POOL_PA).unwrap();
        assert!(t.used_pages()[0].iter().all(|&d| d == 0), "the pool is cleared");
        t.map(0x0400_0000, 0x1050_0000, 0x1_0000, RW_CACHED).unwrap();
        assert_eq!(t.used(), 4);
        let w = t.walk(0x0400_1234);
        assert_eq!(
            w,
            Walk {
                desc: [POOL_PA + 0x1003, POOL_PA + 0x2003, POOL_PA + 0x3003, 0x0060_0000_1050_1f47],
                levels: 4,
                pa: Some(0x1050_1234),
            }
        );
        // every page of the range, and nothing past it
        for i in 0..16 {
            assert_eq!(t.walk(0x0400_0000 + i * PAGE).pa, Some(0x1050_0000 + i * PAGE));
        }
        assert_eq!(t.walk(0x0401_0000).pa, None);
        assert_eq!(t.walk(0x0401_0000).levels, 4);
        assert_eq!(t.walk(0x0800_0000).levels, 3);
        assert_eq!(t.walk(0x80_0000_0000).levels, 1);
        // the descriptors are in the pages, where the GPU reads them
        assert_eq!(p[3][1], 0x0060_0000_1050_1f47);
        assert_eq!(p[0][0], POOL_PA + 0x1003);
        assert_eq!(p[0][1], 0);
    }

    #[test]
    fn mappings_share_tables_and_are_never_replaced() {
        let mut p = pool(8);
        let mut t = Tables::new(&mut p, POOL_PA).unwrap();
        t.map(0x0000_0000, 0x1060_0000, PAGE, Prot { write: false, exec: false, cached: true }).unwrap();
        t.map(0x0080_0000, 0x1061_0000, 2 * PAGE, Prot { write: false, exec: true, cached: true }).unwrap();
        // one L0, L1, L2 and two L3 (the two ranges are 8 MiB apart)
        assert_eq!(t.used(), 5);
        assert_eq!(t.map(0x0080_1000, 0x1070_0000, PAGE, RW_CACHED), Err(PtRefusal::AlreadyMapped));
        assert_eq!(t.walk(0x0080_1000).pa, Some(0x1061_1000));
        assert_eq!(t.walk(0x0080_1000).desc[3] & desc::XN, 0);
    }

    #[test]
    fn a_full_pool_and_bad_arguments_are_refused() {
        let mut p = pool(3);
        let mut t = Tables::new(&mut p, POOL_PA).unwrap();
        assert_eq!(t.map(0x0400_0000, 0x1050_0000, PAGE, RW_CACHED), Err(PtRefusal::PoolFull));
        assert_eq!(t.map(0x0400_0800, 0x1050_0000, PAGE, RW_CACHED), Err(PtRefusal::Unaligned));
        assert_eq!(t.map(0x0400_0000, 0x1050_0000, 0, RW_CACHED), Err(PtRefusal::Unaligned));
        assert_eq!(t.map(0xffff_ffff_f000, 0x1050_0000, 2 * PAGE, RW_CACHED), Err(PtRefusal::OutOfRange));
        assert_eq!(t.map(0x0400_0000, 0xff_ffff_f000, 2 * PAGE, RW_CACHED), Err(PtRefusal::OutOfRange));
        assert!(Tables::new(&mut [], POOL_PA).is_err());
        assert!(Tables::new(&mut pool(1), POOL_PA + 8).is_err());
        assert!(Tables::new(&mut pool(2), (1 << PA_BITS) - PAGE).is_err());
    }

    #[test]
    fn a_table_outside_the_pool_is_not_followed() {
        let mut p = pool(4);
        {
            let mut t = Tables::new(&mut p, POOL_PA).unwrap();
            t.map(0x0400_0000, 0x1050_0000, PAGE, RW_CACHED).unwrap();
        }
        p[0][0] = table_desc(0x2000_0000);
        let mut t = Tables { pages: &mut p, pa: POOL_PA, used: 4 };
        assert_eq!(t.walk(0x0400_0000).pa, None);
        assert_eq!(t.walk(0x0400_0000).levels, 1);
        assert_eq!(t.map(0x0400_1000, 0x1050_1000, PAGE, RW_CACHED), Err(PtRefusal::ForeignTable));
    }
}
