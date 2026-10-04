// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GPU MMU's page tables: one context (the firmware's and every client's
//! mappings share it), 4 KiB pages, three levels, in the formats of the DDK's
//! `hwdefs/rogue/km/rgxmmudefs_km.h` (constants in [`super::fwif`]):
//!
//! - the page catalogue (PC): 1024 32-bit entries indexed by VA bits 39..30,
//!   each the page directory's 4 KiB page number shifted to bit 4
//!   (`RGX_MMUCTRL_PC_DATA_PD_BASE_SHIFT` / `_ALIGNSHIFT`) and VALID;
//! - a page directory (PD): 512 64-bit entries indexed by VA bits 29..21, each
//!   the page table's address in place (`RGX_MMUCTRL_PD_DATA_PT_BASE_CLRMSK`),
//!   page size 4 KiB (`RGX_MMUCTRL_PD_DATA_PAGE_SIZE_4KB`) and VALID;
//! - a page table (PT): 512 64-bit entries indexed by VA bits 20..12, each the
//!   page's address in place (`RGX_MMUCTRL_PT_DATA_PAGE_CLRMSK`), the caller's
//!   protection bits (SLC bypass, PM_META_PROTECT) and VALID.
//!
//! The tables are pages of the carveout's page-table area
//! ([`super::mem::pt_alloc_page`]); every entry written is cleaned (the MMU
//! reads it from memory). The firmware is told to drop its cached walks by the
//! render service's MMUCACHE command, sent when [`take_dirty`] says a mapping
//! changed. Nothing is ever unmapped: a mapping is replaced by mapping the
//! range again.
//!
//! The DDK's own page-table code (`services/server/common/mmu_common.c`) needs
//! the DDK's device-memory layer; it was read for the formats only. Before
//! docs/source-form.md step 7 this was C in `pvr_fw_boot.c` that began in a
//! port of Linux drm/imagination (`pvr_mmu.c`); same tables, byte for byte,
//! and the same cleans in the same order (`tools/pvr/km`).

use super::fwif::*;
use super::km::{km, YdPvrKm};
use super::mem::pt_alloc_page;

const PAGE: u64 = 0x1000;

fn pc_index(va: u64) -> usize {
    ((va >> MMU_VADDR_PC_INDEX_SHIFT) & u64::from(MMU_ENTRIES_PC - 1)) as usize
}
fn pd_index(va: u64) -> usize {
    ((va >> MMU_VADDR_PD_INDEX_SHIFT) & u64::from(MMU_ENTRIES_PD - 1)) as usize
}
fn pt_index(va: u64) -> usize {
    ((va >> MMU_VADDR_PT_INDEX_SHIFT) & u64::from(MMU_ENTRIES_PT - 1)) as usize
}

/// A catalogue entry for a page directory at `pd_phys`.
fn pc_entry(pd_phys: u64) -> u32 {
    (((pd_phys >> MMU_PC_DATA_PD_BASE_ALIGNSHIFT) << MMU_PC_DATA_PD_BASE_SHIFT) as u32) | MMU_PC_DATA_VALID_EN
}
/// The page directory a catalogue entry points at.
fn pc_entry_phys(e: u32) -> u64 {
    u64::from(e >> MMU_PC_DATA_PD_BASE_SHIFT) << MMU_PC_DATA_PD_BASE_ALIGNSHIFT
}
/// A directory entry for a 4 KiB-page table at `pt_phys`.
fn pd_entry(pt_phys: u64) -> u64 {
    (pt_phys & !MMU_PD_DATA_PT_BASE_CLRMSK) | MMU_PD_DATA_PAGE_SIZE_4KB | MMU_PD_DATA_VALID_EN
}
fn pd_entry_phys(e: u64) -> u64 {
    e & !MMU_PD_DATA_PT_BASE_CLRMSK
}
/// A table entry for the page at `phys`.
fn pt_entry(phys: u64, flags: u64) -> u64 {
    (phys & !MMU_PT_DATA_PAGE_CLRMSK) | flags | MMU_PT_DATA_VALID_EN
}
fn pt_entry_phys(e: u64) -> u64 {
    e & !MMU_PT_DATA_PAGE_CLRMSK
}

impl YdPvrKm {
    fn pc(&self) -> *mut u32 {
        self.cpu_at(u64::from(self.pc_off)) as *mut u32
    }
    /// The page catalogue's physical address (the firmware's MMU context and
    /// BIF_CAT_BASE0 take it).
    pub fn pc_phys(&self) -> u64 {
        self.phys + u64::from(self.pc_off)
    }
}

fn rd32(p: *const u32) -> u32 {
    // SAFETY: callers pass table entries inside the carveout.
    unsafe { p.read_volatile() }
}
fn rd64(p: *const u64) -> u64 {
    // SAFETY: as above.
    unsafe { p.read_volatile() }
}

/// Allocate the page catalogue (once; again is a no-op).
pub fn init(k: &mut YdPvrKm) {
    if k.pc_off == 0 {
        k.pc_off = pt_alloc_page(k);
    }
}

/// Map `[va, va + size)` to `[phys, phys + size)` in 4 KiB pages with leaf
/// flags `l0flags`, allocating directories and tables as needed.
pub fn map(k: &mut YdPvrKm, va: u64, phys: u64, size: u64, l0flags: u64) {
    k.mmu_dirty = 1;
    let mut o = 0u64;
    while o < size {
        let v = va + o;
        let pc = k.pc().wrapping_add(pc_index(v));
        if rd32(pc) & MMU_PC_DATA_VALID_EN == 0 {
            let pd_phys = k.phys + u64::from(pt_alloc_page(k));
            // SAFETY: an entry of the catalogue page.
            unsafe { pc.write_volatile(pc_entry(pd_phys)) };
            k.flush(pc, 4);
        }
        let pd = (k.phys_to_cpu(pc_entry_phys(rd32(pc))) as *mut u64).wrapping_add(pd_index(v));
        if rd64(pd) & MMU_PD_DATA_VALID_EN == 0 {
            let pt_phys = k.phys + u64::from(pt_alloc_page(k));
            // SAFETY: an entry of a directory page we allocated.
            unsafe { pd.write_volatile(pd_entry(pt_phys)) };
            k.flush(pd, 8);
        }
        let pt = (k.phys_to_cpu(pd_entry_phys(rd64(pd))) as *mut u64).wrapping_add(pt_index(v));
        // SAFETY: an entry of a table page we allocated.
        unsafe { pt.write_volatile(pt_entry(phys + o, l0flags)) };
        k.flush(pt, 8);
        o += PAGE;
    }
}

/// True once if a mapping changed since the last call: what decides whether
/// a kick needs the MMU caches invalidated first (the DDK sends its MMUCACHE
/// command when a mapping changed, not on every kick).
pub fn take_dirty(k: &mut YdPvrKm) -> bool {
    let d = k.mmu_dirty != 0;
    k.mmu_dirty = 0;
    d
}

/// The leaf entry for `va` (0: unmapped). Diagnostics.
pub fn lookup(k: &YdPvrKm, va: u64) -> u64 {
    if k.pc_off == 0 {
        return 0;
    }
    let pce = rd32(k.pc().wrapping_add(pc_index(va)));
    if pce & MMU_PC_DATA_VALID_EN == 0 {
        return 0;
    }
    let pde = rd64((k.phys_to_cpu(pc_entry_phys(pce)) as *const u64).wrapping_add(pd_index(va)));
    if pde & MMU_PD_DATA_VALID_EN == 0 {
        return 0;
    }
    rd64((k.phys_to_cpu(pd_entry_phys(pde)) as *const u64).wrapping_add(pt_index(va)))
}

/// Lab audit: every valid leaf must point into the carveout or into one of
/// the allowed physical ranges `[lo[i], hi[i])` (the scan-out buffers); a
/// leaf anywhere else would let the GPU write over the kernel. The first four
/// are logged; returns how many there are.
pub fn audit(k: &YdPvrKm, lo: &[u64], hi: &[u64]) -> u32 {
    if k.pc_off == 0 {
        return 0;
    }
    let mut bad = 0u32;
    for l2 in 0..MMU_ENTRIES_PC as usize {
        let pce = rd32(k.pc().wrapping_add(l2));
        if pce & MMU_PC_DATA_VALID_EN == 0 {
            continue;
        }
        let pd = k.phys_to_cpu(pc_entry_phys(pce)) as *const u64;
        for l1 in 0..MMU_ENTRIES_PD as usize {
            let pde = rd64(pd.wrapping_add(l1));
            if pde & MMU_PD_DATA_VALID_EN == 0 {
                continue;
            }
            let pt = k.phys_to_cpu(pd_entry_phys(pde)) as *const u64;
            for l0 in 0..MMU_ENTRIES_PT as usize {
                let pte = rd64(pt.wrapping_add(l0));
                if pte & MMU_PT_DATA_VALID_EN == 0 {
                    continue;
                }
                let phys = pt_entry_phys(pte);
                let ok = (phys >= k.phys && phys < k.phys + k.size)
                    || lo.iter().zip(hi).any(|(&a, &b)| phys >= a && phys < b);
                if !ok {
                    if bad < 4 {
                        let va = ((l2 as u64) << 30) | ((l1 as u64) << 21) | ((l0 as u64) << 12);
                        crate::km_log!(
                            k,
                            Err,
                            "mmu audit: VA 0x{:x} -> phys 0x{:x} (pte 0x{:x}) outside the carveout",
                            va,
                            phys,
                            pte
                        );
                    }
                    bad += 1;
                }
            }
        }
    }
    bad
}

/// The firmware heap: its GPU window -> the carveout. Only the main region's
/// first `fw_main_size` bytes and the config region at the top of the 32 MiB
/// window are backed. The firmware's code and data pages are plain RW with
/// SLC bypass (the RISC-V core then sees the CPU-cleaned memory), not
/// PM_META_PROTECT, which is for the PM's own structures only.
pub fn map_fw_heap(k: &mut YdPvrKm) {
    let flags = MMU_PT_DATA_SLC_BYPASS_CTRL_EN;
    let cfg = k.fw_config_off;
    map(k, FW_HEAP_BASE, k.phys, u64::from(k.fw_main_size), flags);
    map(k, k.heap_gpu(cfg), k.heap_phys(cfg), u64::from(FW_HEAP_SIZE - cfg), flags);
    crate::km_log!(
        k,
        Inf,
        "MMU: FW heap mapped ({} KiB main + {} KiB config of the 32 MiB window), PC root phys=0x{:x}",
        k.fw_main_size / 1024,
        (FW_HEAP_SIZE - cfg) / 1024,
        k.pc_phys()
    );
}

// ---- C entry points ------------------------------------------------------------
//
// Every one: `k` is the C driver's `struct yd_pvr_km` (see [`km`]).

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_init(k: *mut YdPvrKm) {
    // SAFETY: the caller's contract.
    init(unsafe { km(k) })
}

/// # Safety
/// See the section comment; [`init`] has run.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_map(k: *mut YdPvrKm, va: u64, phys: u64, size: u64, l0flags: u64) {
    // SAFETY: the caller's contract.
    map(unsafe { km(k) }, va, phys, size, l0flags)
}

/// # Safety
/// See the section comment; [`init`] has run.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_map_fw_heap(k: *mut YdPvrKm) {
    // SAFETY: the caller's contract.
    map_fw_heap(unsafe { km(k) })
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_take_dirty(k: *mut YdPvrKm) -> bool {
    // SAFETY: the caller's contract.
    take_dirty(unsafe { km(k) })
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_lookup(k: *const YdPvrKm, va: u64) -> u64 {
    // SAFETY: the caller's contract.
    lookup(unsafe { &*k }, va)
}

/// # Safety
/// See the section comment; `lo` and `hi` valid for `n` entries (or `n` 0).
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_audit(k: *const YdPvrKm, lo: *const u64, hi: *const u64, n: u32) -> u32 {
    let (lo, hi) = if n == 0 || lo.is_null() || hi.is_null() {
        (&[][..], &[][..])
    } else {
        // SAFETY: the caller's contract.
        unsafe { (core::slice::from_raw_parts(lo, n as usize), core::slice::from_raw_parts(hi, n as usize)) }
    };
    // SAFETY: the caller's contract.
    audit(unsafe { &*k }, lo, hi)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_mmu_pc_phys(k: *const YdPvrKm) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { &*k }.pc_phys()
}
