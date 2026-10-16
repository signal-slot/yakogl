// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The carveout's allocators.
//!
//! The carveout ([`YdPvrKm`]) is laid out by the C driver
//! (`driver/src/pvr_internal.h`): the backed part of the firmware heap (its
//! main region from offset 0, then its config region), the page-table area,
//! then the render pool. Three bump allocators hand it out:
//!
//! - the firmware heap ([`heap_alloc`]): objects the firmware reads through
//!   its shared-data window, addressed by heap offset;
//! - page-table pages ([`pt_alloc_page`], for [`super::mmu`]);
//! - the render pool ([`bo_alloc`] and its variants): pages mapped into one of
//!   the GPU heaps at a device-VA cursor the caller keeps per heap. Its large
//!   GENERAL blocks come from the other end ([`bo_alloc_top`]): the top
//!   region grows down from the carveout's end, and its floor is the bump's
//!   limit.
//!
//! Every allocation is zeroed and cleaned before it is handed out (the GPU
//! reads it from memory). Nothing is ever freed here; the render service keeps
//! its own free lists over the pool and can give the newest block back to the
//! bump ([`bo_rollback`]).
//!
//! Before docs/source-form.md step 7 this was C in `pvr_fw_boot.c`; the
//! firmware-heap allocator began in a port of Linux drm/imagination
//! (`pvr_fw.c`), the rest is ours. Same allocations, same order of cleans
//! (`tools/pvr/km`, the host check, compares the two).

use super::fwif::*;
use super::km::{km, YdPvrKm, PLACE_MAX};
use super::mmu;

/// `struct pvr_gpu_bo` (driver/src/pvr_types.h): a render-pool block mapped
/// into the GPU MMU. `cpu` null: nothing (allocation failed).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PvrGpuBo {
    /// Device virtual address.
    pub gpu: u64,
    /// CPU address in the carveout.
    pub cpu: *mut u8,
    /// Offset in its heap (the address relative to the heap's base).
    pub heap_off: u32,
}

const _: () = {
    assert!(core::mem::size_of::<PvrGpuBo>() == 24);
    assert!(core::mem::offset_of!(PvrGpuBo, heap_off) == 16);
};

impl PvrGpuBo {
    pub const NONE: PvrGpuBo = PvrGpuBo { gpu: 0, cpu: core::ptr::null_mut(), heap_off: 0 };
}

/// Leaf-entry flags of the render path (pvr_drv.h `PVR_L0_FLAGS_*`): data is
/// SLC-bypassed; the PM's own structures (freelist PFN arrays, the MList) are
/// also PM_META_PROTECT.
pub const L0_FLAGS_DATA: u64 = MMU_PT_DATA_SLC_BYPASS_CTRL_EN;
pub const L0_FLAGS_PM: u64 = MMU_PT_DATA_PM_META_PROTECT_EN | MMU_PT_DATA_SLC_BYPASS_CTRL_EN;
pub const L0_FLAGS_PM_CACHED: u64 = MMU_PT_DATA_PM_META_PROTECT_EN;

/// Zephyr's `ROUND_UP` (computed in 64 bits, as `unsigned long` is there).
pub const fn round_up(x: u64, align: u64) -> u64 {
    x.div_ceil(align) * align
}

const PAGE: u64 = 0x1000;

// ---- the firmware heap ---------------------------------------------------------

impl YdPvrKm {
    /// Firmware-heap offset -> carveout offset: the main region is backed 1:1
    /// from the carveout's base, the config region right after it.
    pub fn heap_carveout_off(&self, off: u32) -> u32 {
        if off < self.fw_config_off {
            off
        } else {
            self.fw_main_size.wrapping_add(off - self.fw_config_off)
        }
    }

    /// CPU address of a firmware-heap object.
    pub fn heap_cpu(&self, off: u32) -> *mut u8 {
        self.cpu_at(u64::from(self.heap_carveout_off(off)))
    }

    /// Its device virtual address (the firmware heap's GPU window).
    pub fn heap_gpu(&self, off: u32) -> u64 {
        FW_HEAP_BASE + u64::from(off)
    }

    /// Its firmware address: the RISC-V firmware reads shared and config
    /// objects through its cached shared-data region 0xC, which it remaps to
    /// the heap's base itself. (Region 0xD is not remapped by this firmware:
    /// an address there faults.)
    ///
    /// With `fw_uncached` (`CONFIG_SAFEUI_GPU_PVR_FW_UNCACHED`) it is the
    /// uncached region 0x4 (rgx_riscv.h:103, :107-108) instead. Linux
    /// drm/imagination allocates every host-shared firmware object
    /// `PVR_BO_FW_FLAGS_DEVICE_UNCACHED` and, on RISC-V, hands the firmware
    /// the SHARED_UNCACHED_DATA region's address for it
    /// (pvr_fw_riscv.c:110-120). Through the cached window the firmware's
    /// writes can stay in its own data cache: on the AM62P a FWCCB slot read
    /// zero for 2 ms of invalidated re-reads while the write offset said
    /// it was written (docs/notes/pvr-render-pool.md, the
    /// check's A/B). Whether this firmware build maps region 0x4 over the heap
    /// is not readable: if it does not, its first read of a host object faults.
    pub fn heap_fwaddr(&self, off: u32) -> u32 {
        let base =
            if self.fw_uncached != 0 { RISCV_SHARED_UNCACHED_DATA0_BASE } else { RISCV_SHARED_CACHED_DATA0_BASE };
        base | off
    }

    /// Its physical address.
    pub fn heap_phys(&self, off: u32) -> u64 {
        self.phys + u64::from(self.heap_carveout_off(off))
    }
}

/// Allocate `size` bytes (rounded to 8) at `align` in the firmware heap's
/// main region, zeroed and cleaned; returns the heap offset. Running past the
/// backed main region is a broken invariant (the objects the driver makes
/// are fixed in number and well under it): it panics.
pub fn heap_alloc(k: &mut YdPvrKm, size: u32, align: u32) -> u32 {
    let off = round_up(u64::from(k.heap_next), u64::from(align)) as u32;
    k.heap_next = off.wrapping_add(round_up(u64::from(size), 8) as u32);
    assert!(k.heap_next <= k.fw_main_size, "firmware heap main region overflow");
    let p = k.heap_cpu(off);
    // SAFETY: inside the backed main region (asserted), which the carveout maps.
    unsafe { core::ptr::write_bytes(p, 0, size as usize) };
    k.flush(p, size as usize);
    off
}

// ---- page-table pages -----------------------------------------------------------

/// One 4 KiB page-table page from the page-table area, zeroed and cleaned
/// (the GPU's MMU walks the tables straight from memory); returns its carveout
/// offset. The area is sized for the heaps the driver maps; running out is a
/// broken invariant and panics.
pub fn pt_alloc_page(k: &mut YdPvrKm) -> u32 {
    let off = round_up(u64::from(k.pt_next), PAGE) as u32;
    k.pt_next = off + PAGE as u32;
    assert!(k.pt_next <= k.pt_area_end, "GPU page-table area overflow");
    let p = k.cpu_at(u64::from(off));
    // SAFETY: inside the page-table area (asserted).
    unsafe { core::ptr::write_bytes(p, 0, PAGE as usize) };
    k.flush(p, PAGE as usize);
    off
}

// ---- the render pool -------------------------------------------------------------

/// The device-VA offset of the render pool's top region in its heap: the
/// block at carveout offset `co` is mapped at heap offset `TOP_VA + co`, so
/// two blocks are adjacent in VA exactly when they are in the carveout, and a
/// page's mapping never changes. Above every offset the bump hands out (at
/// most the carveout's size) and every placed span ([`bo_place`]: below
/// 1 GiB).
pub const TOP_VA: u32 = 0x4000_0000;

impl YdPvrKm {
    /// The carveout's end for the render pool (page aligned).
    pub fn pool_end(&self) -> u64 {
        self.size & !(PAGE - 1)
    }

    /// The top region's floor: the bump's limit (carveout offset).
    pub fn pool_floor(&self) -> u64 {
        self.pool_end().saturating_sub(u64::from(self.pool_top))
    }

    /// Bytes between the bump and the top region's floor: what either end
    /// can still take.
    pub fn pool_gap(&self) -> u64 {
        self.pool_floor().saturating_sub(round_up(u64::from(self.pool_next), PAGE))
    }

    /// Whether the top region can be used: switched on, and every carveout
    /// offset fits a heap offset above [`TOP_VA`].
    pub fn pool_top_on(&self) -> bool {
        self.pool_top_above != 0 && self.size <= u64::from(u32::MAX - TOP_VA) + 1
    }
}

/// Bump-allocate `size` bytes (page rounded, zeroed, cleaned) from the render
/// pool and map them at `heap_base + *heap_bump` (page rounded) with leaf
/// flags `l0flags`; advances `*heap_bump`. The pool is the carveout's tail up
/// to the top region's floor: a request past it fails ([`PvrGpuBo::NONE`],
/// logged) rather than touching memory outside the carveout or a top block.
pub fn bo_alloc(k: &mut YdPvrKm, heap_base: u64, heap_bump: &mut u32, size: u32, l0flags: u64) -> PvrGpuBo {
    let msz = round_up(u64::from(size), PAGE) as u32;
    let co = round_up(u64::from(k.pool_next), PAGE) as u32;
    let hb = round_up(u64::from(*heap_bump), PAGE) as u32;
    let end = k.pool_floor();

    if u64::from(msz) > end || u64::from(co) > end - u64::from(msz) {
        crate::km_log!(
            k,
            Err,
            "render pool exhausted: {} KiB requested, {} KiB left",
            msz / 1024,
            (end.wrapping_sub(u64::from(co)) / 1024) as u32
        );
        return PvrGpuBo::NONE;
    }
    k.pool_next = co + msz;
    *heap_bump = hb.wrapping_add(msz);
    let p = k.cpu_at(u64::from(co));
    // SAFETY: [co, co + msz) is inside the carveout (checked above).
    unsafe { core::ptr::write_bytes(p, 0, msz as usize) };
    k.flush(p, msz as usize);
    mmu::map(k, heap_base + u64::from(hb), k.phys + u64::from(co), u64::from(msz), l0flags);
    PvrGpuBo { gpu: heap_base + u64::from(hb), cpu: p, heap_off: hb }
}

/// Allocate `size` bytes (page rounded, zeroed, cleaned) from the top of the
/// render pool: right under the top region's floor, which moves down by it,
/// mapped at `heap_base + TOP_VA + co` with [`L0_FLAGS_DATA`]. Only the pages
/// below the region's high-water are mapped: those above it already map the
/// same pages. [`PvrGpuBo::NONE`] when the gap to the bump is too small, with
/// no line (the caller decides what to try next).
pub fn bo_alloc_top(k: &mut YdPvrKm, heap_base: u64, size: u32) -> PvrGpuBo {
    let msz = round_up(u64::from(size), PAGE);
    if !k.pool_top_on() || msz == 0 || msz > k.pool_gap() {
        return PvrGpuBo::NONE;
    }
    let co = k.pool_floor() - msz;
    k.pool_top += msz as u32;
    let mapped = k.pool_end() - u64::from(k.pool_top_hw);
    if k.pool_top > k.pool_top_hw {
        k.pool_top_hw = k.pool_top;
    }
    let p = k.cpu_at(co);
    // SAFETY: [co, co + msz) is inside the carveout, between the bump and the
    // top region (checked above).
    unsafe { core::ptr::write_bytes(p, 0, msz as usize) };
    k.flush(p, msz as usize);
    let va = heap_base + u64::from(TOP_VA) + co;
    if co < mapped {
        mmu::map(k, va, k.phys + co, mapped.min(co + msz) - co, L0_FLAGS_DATA);
    }
    PvrGpuBo { gpu: va, cpu: p, heap_off: TOP_VA + co as u32 }
}

/// Give a top block back to the gap when it lies at the top region's floor
/// (its heap offset is `TOP_VA` + the floor); true when given back.
pub fn bo_top_rollback(k: &mut YdPvrKm, heap_off: u32, size: u32) -> bool {
    let msz = round_up(u64::from(size), PAGE);
    if heap_off < TOP_VA || u64::from(heap_off - TOP_VA) != k.pool_floor() || msz > u64::from(k.pool_top) {
        return false;
    }
    k.pool_top -= msz as u32;
    true
}

/// A PM-protected allocation (a freelist's PFN array, an MList): the DDK
/// places each such PMR on a 16 KiB device-VA boundary (on-board capture:
/// global1 at ..0000, global2 at ..4000, local at ..8000, MList at ..c000), so
/// the cursor and the size are rounded to 16 KiB and the leaf flags are
/// [`L0_FLAGS_PM`] (PM_META_PROTECT and SLC bypass, what the DDK's
/// `RGXDerivePTEProt` gives an uncached PM allocation).
pub fn bo_alloc_pm(k: &mut YdPvrKm, heap_base: u64, heap_bump: &mut u32, size: u32) -> PvrGpuBo {
    *heap_bump = round_up(u64::from(*heap_bump), 0x4000) as u32;
    bo_alloc(k, heap_base, heap_bump, round_up(u64::from(size), 0x4000) as u32, L0_FLAGS_PM)
}

/// Give a block back to the bump when it is the newest one: both the heap's
/// device-VA cursor and the pool's physical cursor must end at it (a block can
/// be the newest in its heap while another heap has bumped the shared
/// physical cursor past it). Rewinds both or neither; true when rewound (the
/// caller must then not also put the block on a free list).
pub fn bo_rollback(k: &mut YdPvrKm, heap_bump: &mut u32, heap_off: u32, cpu: *const u8, size: u32) -> bool {
    let msz = round_up(u64::from(size), PAGE) as u32;
    if cpu.is_null() || msz == 0 {
        return false;
    }
    if u64::from(heap_off) + u64::from(msz) != u64::from(*heap_bump) {
        return false;
    }
    if !core::ptr::eq(cpu.wrapping_add(msz as usize), k.cpu_at(u64::from(k.pool_next))) {
        return false;
    }
    *heap_bump = heap_off;
    k.pool_next -= msz;
    true
}

/// Map a block at an exact heap offset (the page-aligned span covering
/// `[start_off, start_off + size)`), independent of any cursor; the returned
/// `cpu`/`gpu` point at `start_off` itself. Objects that share a page must be
/// placed by one call. The spans placed so far are kept, and an overlap, a
/// heap that is not one of the four render heaps, or unknown leaf flags are
/// broken invariants (panic).
pub fn bo_place(k: &mut YdPvrKm, heap_base: u64, start_off: u32, size: u32, l0flags: u64) -> PvrGpuBo {
    let pg = start_off & !0xFFF;
    let span = (round_up(u64::from(start_off) + u64::from(size), PAGE) - u64::from(pg)) as u32;
    let pgva = heap_base + u64::from(pg);
    let co = round_up(u64::from(k.pool_next), PAGE) as u32;

    assert!(
        heap_base == GENERAL_HEAP_BASE
            || heap_base == PDSCODEDATA_HEAP_BASE
            || heap_base == USCCODE_HEAP_BASE
            || heap_base == PMMETA_PROTECT_HEAP_BASE,
        "place: bad heap"
    );
    assert!(u64::from(start_off) + u64::from(size) <= 0x4000_0000, "place: offset out of the heap window");
    assert!(
        l0flags == L0_FLAGS_DATA || l0flags == L0_FLAGS_PM || l0flags == L0_FLAGS_PM_CACHED,
        "place: bad PTE flags"
    );
    let n = k.place_n as usize;
    for s in &k.place[..n] {
        assert!(pgva + u64::from(span) <= s.va || s.va + u64::from(s.span) <= pgva, "place: VA-page overlap");
    }
    assert!(n < PLACE_MAX, "place: registry full");
    k.place[n].va = pgva;
    k.place[n].span = span;
    k.place_n += 1;

    k.pool_next = co + span;
    assert!(u64::from(k.pool_next) <= k.pool_floor(), "place: pool overflow");
    let p = k.cpu_at(u64::from(co));
    // SAFETY: inside the carveout (asserted).
    unsafe { core::ptr::write_bytes(p, 0, span as usize) };
    k.flush(p, span as usize);
    mmu::map(k, pgva, k.phys + u64::from(co), u64::from(span), l0flags);
    PvrGpuBo {
        gpu: heap_base + u64::from(start_off),
        cpu: p.wrapping_add((start_off - pg) as usize),
        heap_off: start_off,
    }
}

/// Round the render pool's physical cursor up to `align` (a freelist's
/// backing must start on a 16 KiB boundary: the PM builds its 16 KiB virtual
/// pages from four consecutive 4 KiB ones).
pub fn pool_align(k: &mut YdPvrKm, align: u32) {
    k.pool_next = round_up(u64::from(k.pool_next), u64::from(align)) as u32;
}

/// Physical address of a block.
pub fn bo_phys(k: &YdPvrKm, bo: &PvrGpuBo) -> u64 {
    k.phys.wrapping_add(k.off_of(bo.cpu))
}

// ---- C entry points (pvr_drv.h's API is wrappers over these) ----------------------
//
// Every one: `k` is the C driver's `struct yd_pvr_km` (see [`km`]).

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_heap_alloc(k: *mut YdPvrKm, size: u32, align: u32) -> u32 {
    // SAFETY: the caller's contract.
    heap_alloc(unsafe { km(k) }, size, align)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_heap_cpu(k: *const YdPvrKm, off: u32) -> *mut u8 {
    // SAFETY: the caller's contract.
    unsafe { &*k }.heap_cpu(off)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_heap_fwaddr(k: *const YdPvrKm, off: u32) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { &*k }.heap_fwaddr(off)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_heap_gpu(k: *const YdPvrKm, off: u32) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { &*k }.heap_gpu(off)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_heap_phys(k: *const YdPvrKm, off: u32) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { &*k }.heap_phys(off)
}

/// # Safety
/// See the section comment; `heap_bump` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_bo_alloc(
    k: *mut YdPvrKm,
    heap_base: u64,
    heap_bump: *mut u32,
    size: u32,
    l0flags: u64,
) -> PvrGpuBo {
    // SAFETY: the caller's contract.
    unsafe { bo_alloc(km(k), heap_base, &mut *heap_bump, size, l0flags) }
}

/// # Safety
/// See the section comment; `heap_bump` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_bo_alloc_pm(
    k: *mut YdPvrKm,
    heap_base: u64,
    heap_bump: *mut u32,
    size: u32,
) -> PvrGpuBo {
    // SAFETY: the caller's contract.
    unsafe { bo_alloc_pm(km(k), heap_base, &mut *heap_bump, size) }
}

/// # Safety
/// See the section comment; `heap_bump` null or valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_bo_rollback(
    k: *mut YdPvrKm,
    heap_bump: *mut u32,
    heap_off: u32,
    cpu: *const u8,
    size: u32,
) -> bool {
    // SAFETY: the caller's contract.
    match unsafe { heap_bump.as_mut() } {
        Some(b) => bo_rollback(unsafe { km(k) }, b, heap_off, cpu, size),
        None => false,
    }
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_bo_place(
    k: *mut YdPvrKm,
    heap_base: u64,
    start_off: u32,
    size: u32,
    l0flags: u64,
) -> PvrGpuBo {
    // SAFETY: the caller's contract.
    bo_place(unsafe { km(k) }, heap_base, start_off, size, l0flags)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_pool_align(k: *mut YdPvrKm, align: u32) {
    // SAFETY: the caller's contract.
    pool_align(unsafe { km(k) }, align)
}

/// # Safety
/// See the section comment; `bo` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_bo_phys(k: *const YdPvrKm, bo: *const PvrGpuBo) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { bo_phys(&*k, &*bo) }
}
