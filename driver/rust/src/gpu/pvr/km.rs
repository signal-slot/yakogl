// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The kernel side's context: `struct yd_pvr_km` (include/yakogl_drivers.h).
//!
//! One per GPU, owned by the C driver (`driver/src/pvr_fw_boot.c`), which
//! maps the carveout, fills the layout and the OS hooks and then hands it to
//! every entry point of [`super::mem`], [`super::mmu`], [`super::ccb`],
//! [`super::freelist`] and [`super::fwinit`]. All of their state lives in it
//! (the crate has no statics).
//!
//! The carveout is one physically contiguous block the GPU and the CPU share.
//! The CPU's mapping of it may be cached (the hooks clean and invalidate) or
//! Normal non-cacheable (the board's: the hooks are then cheap); the code
//! makes the same calls either way, the contract the C driver followed:
//! every write the GPU or the firmware must see is cleaned, every word the
//! firmware writes is invalidated before it is read, and a full barrier
//! (`dmb sy`, Zephyr's `barrier_dmem_fence_full()`) orders the two sides
//! where the C driver had one.

use super::rgxlayer::{Bus, YdPvrBus};
use core::ffi::c_void;
use core::fmt;
use core::mem::{offset_of, size_of};

/// `struct yd_pvr_km_ops`: what the kernel side borrows from the OS.
#[repr(C)]
pub struct YdPvrKmOps {
    pub ctx: *mut c_void,
    /// Clean `[addr, addr + len)` to memory (`sys_cache_data_flush_range`).
    pub flush: unsafe extern "C" fn(ctx: *mut c_void, addr: *const c_void, len: usize),
    /// Invalidate it (`sys_cache_data_invd_range`).
    pub invd: unsafe extern "C" fn(ctx: *mut c_void, addr: *const c_void, len: usize),
    /// A full memory barrier (`barrier_dmem_fence_full()`, `dmb sy`).
    pub fence: unsafe extern "C" fn(ctx: *mut c_void),
    /// `k_busy_wait`.
    pub busy_wait_us: unsafe extern "C" fn(ctx: *mut c_void, us: u32),
    /// One log line in the GPU driver's log module (Zephyr levels, 1 ERR ..
    /// 4 DBG); `buf` is not NUL-terminated.
    pub log: unsafe extern "C" fn(ctx: *mut c_void, level: u32, buf: *const u8, len: usize),
}

/// A placed span (`gpu_bo_place`'s overlap registry).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YdPvrSpan {
    pub va: u64,
    pub span: u32,
    pub pad: u32,
}

/// The number of placed spans the registry holds.
pub const PLACE_MAX: usize = 32;

/// `struct yd_pvr_km`.
#[repr(C)]
pub struct YdPvrKm {
    // ---- filled by the C driver before the first call
    /// The carveout, mapped.
    pub cpu: *mut u8,
    /// Its physical base and size.
    pub phys: u64,
    pub size: u64,
    pub ops: *const YdPvrKmOps,
    /// Null on the board (the hardware); a host check's recording bus.
    pub bus: *const YdPvrBus,
    /// The GPU registers, mapped.
    pub regs: usize,
    /// Carveout layout (pvr_internal.h): the backed part of the firmware
    /// heap's main region (`PVR_FW_HEAP_MAIN_PHYS`), the heap offset of its
    /// config region (`PVR_FW_HEAP_CONFIG_OFFSET`), the page-table area
    /// (`PVR_PT_AREA_OFFSET`, `_END`) and the start of the render pool
    /// (`PVR_RENDER_POOL_OFFSET`), all carveout offsets but the second.
    pub fw_main_size: u32,
    pub fw_config_off: u32,
    pub pt_area_off: u32,
    pub pt_area_end: u32,
    pub pool_off: u32,
    // ---- state (zero before the first call)
    /// Firmware-heap bump (heap offset of the next object).
    pub heap_next: u32,
    /// Page-table bump (carveout offset; 0: `pt_area_off`).
    pub pt_next: u32,
    /// Render-pool bump (carveout offset; 0: `pool_off`).
    pub pool_next: u32,
    /// The page catalogue (carveout offset; 0: not built).
    pub pc_off: u32,
    /// A mapping changed since the last [`super::mmu`] `take_dirty`.
    pub mmu_dirty: u32,
    pub place_n: u32,
    pub place: [YdPvrSpan; PLACE_MAX],
    // ---- the CCBs ([`super::ccb`]); heap offsets, set when they are built
    pub kccb_ctl_off: u32,
    pub kccb_cmds_off: u32,
    pub kccb_rtn_off: u32,
    pub fwccb_ctl_off: u32,
    pub fwccb_cmds_off: u32,
    /// Commands posted (the first 40 are named in the debug log).
    pub kccb_posts: u32,
    // ---- the freelists ([`super::freelist`])
    /// The PM-META protect heap's device-VA cursor (PFN arrays, MLists).
    pub pm_bump: u32,
    /// Grows' worth of pages a list keeps of its own when there is no shared
    /// pool (the driver sets 4).
    pub fl_reserve_mult: u32,
    pub fl_pool_pages: u32,
    pub fl_pool_used: u32,
    pub fl_pool_peak: u32,
    /// Grow requests refused (no pages left to give).
    pub fl_no_grow: u32,
    /// FWCCB grow requests seen and grows done.
    pub fl_grow_reqs: u32,
    pub fl_grows_done: u32,
    pub nfls: u32,
    /// The `grow_threshold` the next growable list is built with
    /// (`CONFIG_YAKOGL_DRV_GPU_PVR_FL_READY_PAGES`, [`super::freelist::ready_pages`]);
    /// 0, the default, keeps no ready pages.
    pub fl_ready_threshold: u32,
    pub fl_pool_phys: u64,
    pub fls: [YdPvrFl; FL_MAX],
    // ---- the FWIF structures ([`super::fwinit`]); heap offsets
    pub sysinit_off: u32,
    /// `RGXFWIF_TRACEBUF` and the thread-0 trace ring it points at.
    pub tracebuf_off: u32,
    pub trace_buf_off: u32,
    pub hwrinfobuf_off: u32,
    // ---- the grows' pages and the firmware's notices ([`super::freelist`])
    /// The first PFN of each grow's pages, per list, in the order the grows
    /// were served: what [`super::freelist::reconstruct`] lists again after
    /// a recovery. A grow past [`FL_GROWS_MAX`] is refused, by name.
    pub fl_grow_pfn: [[u32; FL_GROWS_MAX]; FL_MAX],
    /// A list's one-grow reserve, refilled from the render pool after a
    /// served grow (`CONFIG_YAKOGL_DRV_GPU_PVR_FL_GROW_ON_DEMAND`): the first PFN
    /// of `grow` pages on a 16 KiB boundary; 0: none.
    pub fl_spare_pfn: [u32; FL_MAX],
    /// KiB of the render pool the refills asked for and did not get.
    pub fl_refill_refused_kib: u32,
    /// Refills the render pool refused.
    pub fl_refills_refused: u32,
    /// The firmware's UPDATE_STATS notices (`RGXFWIF_FWCCB_CMD_UPDATE_STATS`),
    /// by element: index 1..=6 the DDK's `RGXFWIF_FWCCB_CMD_UPDATE_NUM_*`, 0
    /// any other value.
    pub fw_stats: [u32; 7],
    /// Why the render service stopped taking kicks ([`super::freelist`]
    /// `STOP_*`; 0: it has not), the list and the KiB the cause names, the
    /// fence a wait found it on, and whether the wait said so.
    pub stop_cause: u32,
    pub stop_fl: u32,
    pub stop_kib: u32,
    pub stop_fence: u32,
    pub stop_said: u32,
    // ---- the FWCCB check (`CONFIG_YAKOGL_DRV_GPU_PVR_FWCCB_CHECK`, [`super::ccb`])
    /// 1: a slot read without the command magic is re-read for a bounded
    /// time before it is consumed, and the write offset is watched for going
    /// back. 0: the old consumer.
    pub fwccb_check: u32,
    /// The last write offset read, and 1 once there is one.
    pub fwccb_last_wo: u32,
    pub fwccb_wo_seen: u32,
    /// Slots read without the magic; of those, filled within the bound.
    pub fwccb_empty: u32,
    pub fwccb_filled: u32,
    /// Times the write offset was found behind the last one read.
    pub fwccb_wo_back: u32,
    // ---- the firmware's view of the host's objects ([`super::mem`])
    /// 1: `heap_fwaddr` hands the firmware the uncached window
    /// (`RISCV_SHARED_UNCACHED_DATA0_BASE | off`), as Linux drm/imagination
    /// does for every host-shared object (`CONFIG_YAKOGL_DRV_GPU_PVR_FW_UNCACHED`).
    /// 0: the cached window, as before.
    pub fw_uncached: u32,
    /// A probe (`CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_SERVE_MAX`, never a product
    /// setting): a list that has served this many grows refuses the next
    /// ones, as the render pool refused glmark2's at 1280 pages (the lost
    /// command's sequence: three served, three refused, then the loss).
    /// 0: off.
    pub fl_probe_serve_max: u32,
    // ---- the render pool's top region ([`super::mem::bo_alloc_top`], [`super::pool`])
    /// GENERAL-heap requests of more than this many bytes are placed from
    /// the carveout's end downward, away from the small blocks the bump
    /// places from below (docs/notes/pvr-render-pool.md, "the
    /// top region"). 0: off, every block from the bump, as before.
    pub pool_top_above: u32,
    /// Bytes handed out from the top now: the region is `[size - pool_top,
    /// size)` (page-rounded size), and the bump's limit is its floor.
    pub pool_top: u32,
    /// Its most: every page in `[size - pool_top_hw, size)` has been mapped.
    pub pool_top_hw: u32,
    pub pad: u32,
}

/// Grows a freelist can be served (the table [`YdPvrKm::fl_grow_pfn`]): the
/// 64 MiB carveout's lists can take (4096 - 512) / 256 = 14, and the shared
/// pool of a 1 GiB carveout 64.
pub const FL_GROWS_MAX: usize = 64;

/// The freelists tracked for the firmware's grow and reconstruction
/// requests.
pub const FL_MAX: usize = 6;

/// `struct yd_pvr_fl`: one freelist as the host knows it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YdPvrFl {
    pub id: u32,
    /// Its `RGXFWIF_FREELIST`: firmware address and heap offset.
    pub fw_addr: u32,
    pub heap_off: u32,
    /// The percentage of its pages it keeps as the firmware's ready pages
    /// (drm/imagination's `grow_threshold`; [`super::freelist::ready_pages`]).
    /// 0: none, every listed page is current.
    pub grow_threshold: u32,
    /// The PFN array, CPU address.
    pub arr: *mut u32,
    /// The first backing page.
    pub backing_phys: u64,
    /// Pages in the active stack.
    pub npages: u32,
    pub maxpages: u32,
    /// Pages the firmware may take before it asks (the DDK's ready pages).
    pub ready: u32,
    /// What one grow adds.
    pub grow: u32,
    /// Backing pages listed so far, and still unlisted behind them.
    pub backed: u32,
    pub reserve: u32,
    /// Grows served.
    pub grows: u32,
    /// KiB taken from the render pool for grows when the firmware asked
    /// (on-demand grows, [`super::freelist::grow`]; 0 without them).
    pub pool_kib: u32,
}

const _: () = {
    assert!(size_of::<YdPvrKmOps>() == 48);
    assert!(offset_of!(YdPvrKmOps, flush) == 8);
    assert!(offset_of!(YdPvrKmOps, invd) == 16);
    assert!(offset_of!(YdPvrKmOps, fence) == 24);
    assert!(offset_of!(YdPvrKmOps, busy_wait_us) == 32);
    assert!(offset_of!(YdPvrKmOps, log) == 40);
    assert!(size_of::<YdPvrSpan>() == 16);
    assert!(offset_of!(YdPvrKm, phys) == 8);
    assert!(offset_of!(YdPvrKm, size) == 16);
    assert!(offset_of!(YdPvrKm, ops) == 24);
    assert!(offset_of!(YdPvrKm, bus) == 32);
    assert!(offset_of!(YdPvrKm, regs) == 40);
    assert!(offset_of!(YdPvrKm, fw_main_size) == 48);
    assert!(offset_of!(YdPvrKm, pool_off) == 64);
    assert!(offset_of!(YdPvrKm, heap_next) == 68);
    assert!(offset_of!(YdPvrKm, place_n) == 88);
    assert!(offset_of!(YdPvrKm, place) == 96);
    assert!(offset_of!(YdPvrKm, kccb_ctl_off) == 608);
    assert!(offset_of!(YdPvrKm, kccb_posts) == 628);
    assert!(offset_of!(YdPvrKm, pm_bump) == 632);
    assert!(offset_of!(YdPvrKm, nfls) == 664);
    assert!(offset_of!(YdPvrKm, fl_pool_phys) == 672);
    assert!(offset_of!(YdPvrKm, fls) == 680);
    assert!(size_of::<YdPvrFl>() == 64);
    assert!(offset_of!(YdPvrFl, arr) == 16);
    assert!(offset_of!(YdPvrFl, backing_phys) == 24);
    assert!(offset_of!(YdPvrFl, npages) == 32);
    assert!(offset_of!(YdPvrFl, grows) == 56);
    assert!(offset_of!(YdPvrKm, sysinit_off) == 680 + 64 * FL_MAX);
    assert!(offset_of!(YdPvrKm, hwrinfobuf_off) == 680 + 64 * FL_MAX + 12);
    assert!(offset_of!(YdPvrKm, fl_grow_pfn) == 680 + 64 * FL_MAX + 16);
    assert!(offset_of!(YdPvrKm, fl_spare_pfn) == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX);
    assert!(offset_of!(YdPvrKm, fw_stats) == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX + 4 * FL_MAX + 8);
    assert!(offset_of!(YdPvrKm, stop_cause) == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX + 4 * FL_MAX + 36);
    assert!(offset_of!(YdPvrKm, fwccb_check) == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX + 4 * FL_MAX + 56);
    assert!(offset_of!(YdPvrKm, fw_uncached) == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX + 4 * FL_MAX + 80);
    assert!(
        offset_of!(YdPvrKm, pool_top_above) == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX + 4 * FL_MAX + 88
    );
    assert!(size_of::<YdPvrKm>() == 680 + 64 * FL_MAX + 16 + 4 * FL_GROWS_MAX * FL_MAX + 4 * FL_MAX + 104);
};

/// Log levels (Zephyr's numbering).
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum Lvl {
    Err = 1,
    Wrn = 2,
    Inf = 3,
    Dbg = 4,
}

/// The longest line [`YdPvrKm::log`] formats. The driver's diagnostic lines
/// are long (a freelist line is over 200 characters); they go to the GPU
/// driver's own log, not through `struct yd_env`'s 160-character lines.
pub const KM_LINE_MAX: usize = 400;

struct Line {
    buf: [u8; KM_LINE_MAX],
    len: usize,
}

impl fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let n = s.len().min(KM_LINE_MAX - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// `km_log!(km, Inf, "x {}", 1)`.
#[macro_export]
macro_rules! km_log {
    ($km:expr, $lvl:ident, $($arg:tt)*) => {
        $km.log($crate::gpu::pvr::km::Lvl::$lvl, format_args!($($arg)*))
    };
}

impl YdPvrKm {
    fn ops(&self) -> &YdPvrKmOps {
        // SAFETY: required non-null (the type's contract).
        unsafe { &*self.ops }
    }

    /// The register bus.
    pub fn bus(&self) -> Bus<'_> {
        // SAFETY: null or the C side's hooks.
        Bus::new(unsafe { self.bus.as_ref() })
    }

    /// Clean `len` bytes at `p` to memory.
    pub fn flush<T>(&self, p: *const T, len: usize) {
        let o = self.ops();
        // SAFETY: the C side's hook; it reads nothing but the range bounds.
        unsafe { (o.flush)(o.ctx, p as *const c_void, len) }
    }

    /// Invalidate `len` bytes at `p`.
    pub fn invd<T>(&self, p: *const T, len: usize) {
        let o = self.ops();
        // SAFETY: as above.
        unsafe { (o.invd)(o.ctx, p as *const c_void, len) }
    }

    /// `barrier_dmem_fence_full()`.
    pub fn fence(&self) {
        let o = self.ops();
        // SAFETY: as above.
        unsafe { (o.fence)(o.ctx) }
    }

    pub fn busy_wait_us(&self, us: u32) {
        let o = self.ops();
        // SAFETY: as above.
        unsafe { (o.busy_wait_us)(o.ctx, us) }
    }

    /// Format one line (cut at [`KM_LINE_MAX`]) and log it.
    pub fn log(&self, lvl: Lvl, args: fmt::Arguments<'_>) {
        let mut l = Line { buf: [0; KM_LINE_MAX], len: 0 };
        let _ = fmt::write(&mut l, args);
        let o = self.ops();
        // SAFETY: as above; the C side copies `len` bytes and keeps nothing.
        unsafe { (o.log)(o.ctx, lvl as u32, l.buf.as_ptr(), l.len) }
    }

    /// The CPU address of carveout offset `off`.
    pub fn cpu_at(&self, off: u64) -> *mut u8 {
        debug_assert!(off <= self.size);
        self.cpu.wrapping_add(off as usize)
    }

    /// The carveout offset of a CPU address inside it.
    pub fn off_of(&self, p: *const u8) -> u64 {
        (p as usize).wrapping_sub(self.cpu as usize) as u64
    }

    /// The CPU address of physical address `phys` (inside the carveout).
    pub fn phys_to_cpu(&self, phys: u64) -> *mut u8 {
        self.cpu_at(phys.wrapping_sub(self.phys))
    }
}

/// # Safety
/// `km` is the C driver's `struct yd_pvr_km`, valid and not used by anyone
/// else for the call, its `ops` and `cpu` valid (the type's contract).
pub unsafe fn km<'a>(km: *mut YdPvrKm) -> &'a mut YdPvrKm {
    // SAFETY: the caller's contract.
    unsafe { &mut *km }
}
