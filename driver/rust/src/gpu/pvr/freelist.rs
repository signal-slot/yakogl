// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The parameter manager's freelists: the pages the tiler writes its
//! parameter buffer into, handed to the PM as a stack of page numbers.
//!
//! A freelist is three things: backing pages (TE-writable, SLC-bypassed), a
//! PFN array in the PM-META protect heap (the PM pops page numbers from its
//! high end: entries `[max - current, max)` are the active stack, entries
//! below it are free slots for grows) and the firmware's `RGXFWIF_FREELIST`
//! (rgx_fwif_km.h; offsets in [`super::fwif`]) in the firmware heap. The
//! values follow the DDK's `RGXCreateFreeList` / `RGXProcessRequestGrow` /
//! freelist reconstruction (rgxta3d.c), checked against an on-board capture
//! of the DDK's own triangle test: 16 KiB-aligned backing (the PM's 16 KiB
//! virtual pages are four consecutive 4 KiB ones), the stack base aligned
//! down to 16 bytes, one PMR per array on a 16 KiB device-VA boundary.
//!
//! The firmware asks for more pages on the FWCCB (FREELIST_GROW, see
//! [`super::ccb::fwccb_service`]); a grow comes from the list's own reserve
//! (backed, unlisted pages behind its backing) or, when the driver made one,
//! from a shared pool, whichever list asks. A grow that cannot be served is
//! answered with zero pages. What the firmware does then is unmeasured: an
//! earlier version of this comment said "it renders what it has tiled
//! (partial render)", and no run has tested it. What was measured
//! (docs/notes/pvr-render-pool.md, the probe) is a list at
//! its maximum: the firmware asks for no grow, and the kick never completes.
//!
//! Before docs/source-form.md step 7 this was C in `pvr_fw_boot.c` (ours;
//! the reconstruction began in a port of Linux drm/imagination's
//! `pvr_free_list.c`). [`reconstruct`] now follows that file's
//! `pvr_free_list_reconstruct` (Linux 7.3-rc5,
//! drivers/gpu/drm/imagination/pvr_free_list.c, "GPL-2.0-only OR MIT", used
//! here under MIT, Copyright (c) 2023 Imagination Technologies Ltd.). Same arrays, structures, answers and log lines
//! (`tools/pvr/km`). The compile-time bring-up switches of the C
//! (`PVR_FL_FPM`, `PVR_FL_BACKING_PROTECT`, `PVR_FL_BACKING_NO_16K`,
//! `PVR_FL_ARR_FAULT_TEST`) are gone; no build set them.

use super::ccb::{kccb_post, put32, KccbCmd};
use super::fwif::*;
use super::km::{km, YdPvrKm, FL_GROWS_MAX, FL_MAX};
use super::mem::{bo_alloc, bo_alloc_pm, bo_phys, heap_alloc, pool_align, round_up, PvrGpuBo, L0_FLAGS_DATA};
use core::fmt::Write;

fn by_id(k: &YdPvrKm, id: u32) -> Option<usize> {
    (0..k.nfls as usize).find(|&i| k.fls[i].id == id)
}

fn pfn(phys: u64) -> u32 {
    (phys >> 12) as u32
}

/// # Safety
/// `arr` a PFN array of at least `i + 1` entries.
unsafe fn arr_set(arr: *mut u32, i: u32, v: u32) {
    // SAFETY: the caller's contract.
    unsafe { arr.add(i as usize).write_volatile(v) }
}

/// # Safety
/// As [`arr_set`].
unsafe fn arr_get(arr: *const u32, i: u32) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { arr.add(i as usize).read_volatile() }
}

/// The shared grow pool: `pages` 4 KiB pages from the render pool (general
/// heap, 16 KiB aligned, the backing attributes of a freelist's own pages).
/// 0: no pool.
pub fn pool_init(k: &mut YdPvrKm, gen_b: &mut u32, pages: u32) {
    if pages == 0 {
        return;
    }
    pool_align(k, 0x4000);
    let bo = bo_alloc(k, GENERAL_HEAP_BASE, gen_b, pages.wrapping_mul(4096), L0_FLAGS_DATA);
    k.fl_pool_phys = bo_phys(k, &bo);
    k.fl_pool_pages = pages;
    k.fl_pool_used = 0;
    k.fl_pool_peak = 0;
    crate::km_log!(k, Inf, "freelist grow pool: {} pages ({} MiB) at phys 0x{:x}", pages, pages / 256, k.fl_pool_phys);
}

/// Mesa's grow threshold for the global freelists: Mesa 26.2.1
/// src/imagination/vulkan/pvr_arch_device.c:47-50
/// (`PVR_GLOBAL_FREE_LIST_GROW_THRESHOLD 13U`, "intended to be 12.5%, but
/// has been rounded up"), passed for the global list at :785. The local
/// list is created with 0 (pvr_arch_job_render.c:660); ours has no grow, and
/// [`ready_pages`] gives 0 for it whatever the threshold.
pub const GLOBAL_GROW_THRESHOLD: u32 = 13;

/// The firmware's ready pages of a list that holds `pages`: Linux
/// drm/imagination's `calculate_free_list_ready_pages_locked` (Linux 7.3-rc5,
/// drivers/gpu/drm/imagination/pvr_free_list.c, MIT): `threshold` percent of
/// the pages, at most one grow, rounded down to the 4-page alignment. The
/// firmware takes them when its current pages run out and asks for a grow at
/// the same time, so it renders on while the host answers. On the AM62P
/// (c18f9bfd) the grow answers' READY changed no word of the firmware's
/// acks: it kept each grow's own pages as ready either way
/// (docs/notes/pvr-render-pool.md, "The run with both
/// fixes").
pub fn ready_pages(pages: u32, grow: u32, threshold: u32) -> u32 {
    let r = (u64::from(pages) * u64::from(threshold) / 100) as u32;
    r.min(grow) & !3
}

/// Build a freelist of `npages` listed pages out of `maxpages`, growing by
/// `grow`, and track it under `fl_id` for the firmware's requests. Every
/// backed page except the grow reserve (`grow * fl_reserve_mult`) is listed
/// from the start. Of those, a growable list built with
/// `fl_ready_threshold` keeps [`ready_pages`] as the firmware's READY and
/// the rest as CURRENT (drm/imagination's `free_list_fw_init`); with 0
/// (the default) all are CURRENT. Returns the heap offset of its
/// `RGXFWIF_FREELIST`.
pub fn build(k: &mut YdPvrKm, gen_b: &mut u32, npages: u32, maxpages: u32, grow: u32, ready: u32, fl_id: u32) -> u32 {
    let reserve_pages = grow.wrapping_mul(k.fl_reserve_mult);
    let backing_pages = npages.wrapping_add(ready).wrapping_add(reserve_pages);
    let npages = backing_pages.wrapping_sub(reserve_pages);
    let threshold = if grow != 0 { k.fl_ready_threshold } else { 0 };
    let ready = ready_pages(npages, grow, threshold);
    let current = npages - ready;

    pool_align(k, 0x4000);
    let pages = bo_alloc(k, GENERAL_HEAP_BASE, gen_b, backing_pages.wrapping_mul(4096), L0_FLAGS_DATA);
    let mut pm = k.pm_bump;
    let arr = bo_alloc_pm(k, PMMETA_PROTECT_HEAP_BASE, &mut pm, maxpages.wrapping_mul(4));
    k.pm_bump = pm;
    assert!(!pages.cpu.is_null() && !arr.cpu.is_null(), "freelist: render pool exhausted");
    let pages_phys = bo_phys(k, &pages);
    let a = arr.cpu as *mut u32;
    // the stack base's byte offset, aligned down to 16 (BASE_ADDR_ALIGNSIZE)
    let byte_off = (maxpages.wrapping_sub(npages).wrapping_mul(4)) & !15;
    let ebase = byte_off / 4;

    for i in 0..maxpages {
        // SAFETY: the array has maxpages entries.
        unsafe { arr_set(a, i, 0) };
    }
    let cur_back = backing_pages - reserve_pages;
    for j in 0..cur_back {
        // SAFETY: ebase + cur_back <= maxpages for every list the driver builds.
        unsafe { arr_set(a, ebase + j, pfn(pages_phys + u64::from(j) * 4096)) };
    }

    let off = heap_alloc(k, FREELIST_SIZE as u32, 8);
    let fl = k.heap_cpu(off);
    // SAFETY: a fresh RGXFWIF_FREELIST in the firmware heap.
    unsafe {
        wr64(fl, FREELIST_DEVVADDR, arr.gpu);
        // The KM programs these: PM interactive mode is on for every Rogue
        // core (rgxdefs_km.h defines PM_INTERACTIVE_MODE unconditionally),
        // so the DDK's RGXCreateFreeList sets them too.
        // The current pages' base: the ready pages lie below it, the PM
        // takes them by moving the base down (`free_list_fw_init`).
        let cur_off = (maxpages.wrapping_sub(current).wrapping_mul(4)) & !15;
        wr64(fl, FREELIST_CURRENT_DEVVADDR, arr.gpu + u64::from(cur_off));
        wr32(fl, FREELIST_CURRENT_STACK_TOP, current.wrapping_sub(1));
        wr32(fl, FREELIST_MAX_PAGES, maxpages);
        wr32(fl, FREELIST_CURRENT_PAGES, current);
        wr32(fl, FREELIST_GROW_PAGES, grow);
        wr32(fl, FREELIST_READY_PAGES, ready);
        wr64(fl, FREELIST_ID, u64::from(fl_id));
        fl.add(FREELIST_GROW_PENDING).write_volatile(0);
        wr32(fl, FREELIST_FLAGS, 0);
        // 0: the default global parameter buffer
        wr32(fl, FREELIST_PM_GLOBAL_PB, 0);
    }
    if (k.nfls as usize) < FL_MAX {
        let fw_addr = k.heap_fwaddr(off);
        let t = &mut k.fls[k.nfls as usize];
        t.id = fl_id;
        t.fw_addr = fw_addr;
        t.heap_off = off;
        t.grow_threshold = threshold;
        t.arr = a;
        t.backing_phys = pages_phys;
        t.npages = current;
        t.maxpages = maxpages;
        t.ready = ready;
        t.grow = grow;
        t.backed = cur_back;
        t.reserve = reserve_pages;
        k.nfls += 1;
    }
    // SAFETY: as above.
    let (cur_dev, first, last) =
        unsafe { (rd64(fl, FREELIST_CURRENT_DEVVADDR), arr_get(a, ebase), arr_get(a, maxpages.wrapping_sub(1))) };
    crate::km_log!(
        k,
        Inf,
        "freelist id={} np={} max={} backing={} fpm=0 ebase={} arr.gpu=0x{:x} pages_phys=0x{:x} cur_dev=0x{:x} pfn[{}]=0x{:08x} pfn[{}]=0x{:08x}",
        fl_id,
        npages,
        maxpages,
        backing_pages,
        ebase,
        arr.gpu,
        pages_phys,
        cur_dev,
        ebase,
        first,
        maxpages.wrapping_sub(1),
        last
    );
    off
}

/// Where a grow takes its pages when neither the list's own reserve nor the
/// shared grow pool can serve it (`CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND`):
/// `bytes` from the render pool, mapped as a freelist's own backing, or
/// `None` when the pool refuses. The DDK allocates a grow's pages when the
/// firmware asks for them (RGXProcessRequestGrow), not from a reserve made at
/// creation; this is that, from the one pool this driver has.
pub type GrowSource<'a> = dyn FnMut(&mut YdPvrKm, u32) -> Option<PvrGpuBo> + 'a;

/// Why the render service stops taking kicks (`YdPvrKm::stop_cause`; the
/// board record GPU_STOP's tag). Every cause applies with or without
/// `CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND`. They were the knob's alone until
/// the AM62P's default configuration hung with the knob off and the watchdog
/// ended the run with no record (the common set's round 2, 52be99c3;
/// docs/notes/pvr-render-pool.md).
///
/// A grow the render pool refused is not one: the firmware renders on
/// without the pages (the AM62P, 91bde95d: 235 grows answered with zero
/// pages, every kick completed, bump high-poly at 1.96 times its frame
/// time; docs/notes/pvr-render-pool.md). It is counted
/// ("unserved") and said, and the run goes on.
///
/// The firmware's out-of-memory notice (UPDATE_STATS element 2, measured:
/// one per grow request) with no grow request beside it while a list is at
/// its maximum: the probe's case (0efa0642), where the kick never completed
/// and was not recovered in 61 s.
pub const STOP_AT_MAXIMUM: u32 = crate::board::record::GPU_STOP_AT_MAXIMUM;
/// A grow past the host's table of grows ([`FL_GROWS_MAX`]).
pub const STOP_GROW_TABLE_FULL: u32 = crate::board::record::GPU_STOP_GROW_TABLE_FULL;
/// A fence that did not move for `render::STOP_WAIT_MS` while the knob is
/// on: a wedge none of the other causes recognised. It ends the run with the
/// record before the board's watchdog does (fwccb-2 ended at the watchdog,
/// with no record).
pub const STOP_KICK_TIMEOUT: u32 = crate::board::record::GPU_STOP_KICK_TIMEOUT;

/// The cause as words, for the log.
pub fn stop_name(cause: u32) -> &'static str {
    match cause {
        STOP_GROW_TABLE_FULL => "a freelist grow past the host's table of grows",
        STOP_KICK_TIMEOUT => "a kick that did not complete in the stop's time",
        STOP_AT_MAXIMUM => "the parameter manager out of memory with its freelist at the maximum",
        _ => "not a stop cause",
    }
}

/// Stop the render service: the first cause stays (later ones are only
/// said), every later submit and wait returns -ESHUTDOWN at once
/// (`render::submit`, `render::wait_for`), and the C side ends the run with
/// the record (pvr_render.c, `pvr_render_set_stop_handler`).
pub fn stop(k: &mut YdPvrKm, cause: u32, fl: u32, kib: u32) {
    if k.stop_cause == 0 {
        (k.stop_cause, k.stop_fl, k.stop_kib) = (cause, fl, kib);
    }
    crate::km_log!(
        k,
        Err,
        "render: stopped: {} (list {}, {} KiB); no kick is waited for or taken from now on",
        stop_name(cause),
        fl,
        kib
    );
}

/// Where one grow's pages come from.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Src {
    /// The list's own reserve: backed, unlisted pages behind its backing.
    Own,
    /// The shared grow pool.
    Pool,
    /// The list's reserve as refilled from the render pool after its last
    /// grow ([`YdPvrKm::fl_spare_pfn`]).
    Spare,
    /// The render pool, asked now.
    OnDemand,
}

/// The bytes an on-demand grow or refill asks the render pool for: `grow`
/// pages and three more, so that `grow` pages can be listed from the first
/// 16 KiB boundary. The PM builds its 16 KiB virtual pages from four
/// consecutive 4 KiB ones, so each four listed pages must start on one (as
/// `pool_align` gives the lists' own backing).
pub(crate) fn ondemand_bytes(grow: u32) -> u32 {
    grow.wrapping_mul(4096).wrapping_add(0x3000)
}

/// One grow of list `i`: its source, its pages listed below the stack and
/// recorded in [`YdPvrKm::fl_grow_pfn`], the counters, and the line. Returns
/// the pages added (0: no grow). [`grow`] posts the answer.
///
/// The source is the list's own reserve, else the shared grow pool, else
/// (with `ondemand`) the reserve refilled after the last grow, else the
/// render pool now; with none of them the answer is zero pages. With
/// `ondemand` and no shared pool, a served grow then refills the list's
/// reserve with one grow's pages from the render pool ([`refill`]), so the
/// next grow does not depend on what the pool holds when the firmware asks.
fn serve(k: &mut YdPvrKm, i: usize, mut ondemand: Option<&mut GrowSource<'_>>) -> u32 {
    let id = k.fls[i].id;
    let mut t = k.fls[i];
    t.npages = t.npages.wrapping_add(t.ready);
    t.ready = 0;
    let room = t.grow != 0 && t.maxpages.wrapping_sub(t.npages) >= t.grow;
    let table = (t.grows as usize) < FL_GROWS_MAX;
    let own = t.reserve >= t.grow;
    let pool = !own && k.fl_pool_pages.wrapping_sub(k.fl_pool_used) >= t.grow;
    let spare = !own && !pool && k.fl_spare_pfn[i] != 0;
    // the probe's refusal (`fl_probe_serve_max`): a list with room and a free
    // table slot that has served its quota
    let probed = k.fl_probe_serve_max != 0 && t.grows >= k.fl_probe_serve_max;
    /* the source, and the grow's first page */
    let mut got: Option<(Src, u64)> = None;
    let mut asked_kib = 0u32;
    if room && table && !probed {
        if own {
            got = Some((Src::Own, t.backing_phys + u64::from(t.backed) * 4096));
        } else if pool {
            got = Some((Src::Pool, k.fl_pool_phys + u64::from(k.fl_pool_used) * 4096));
        } else if spare {
            got = Some((Src::Spare, u64::from(k.fl_spare_pfn[i]) << 12));
        } else if let Some(f) = ondemand.as_deref_mut() {
            let bytes = ondemand_bytes(t.grow);
            asked_kib = bytes / 1024;
            if let Some(bo) = f(k, bytes) {
                got = Some((Src::OnDemand, round_up(bo_phys(k, &bo), 0x4000)));
                t.pool_kib = t.pool_kib.wrapping_add(asked_kib);
            }
        }
    }
    let Some((how, src)) = got else {
        k.fls[i] = t;
        k.fl_no_grow += 1;
        // A refused grow is survived (STOP_AT_MAXIMUM's comment); a list with
        // no room, or a grow the table cannot record, is a stop.
        let stop_cause = if !room {
            Some(STOP_AT_MAXIMUM)
        } else if !table {
            Some(STOP_GROW_TABLE_FULL)
        } else {
            None
        };
        if room && table && probed {
            crate::km_log!(
                k,
                Wrn,
                "FWCCB: FREELIST_GROW id={} -> no grow: the probe's limit of {} served grows ({}/{} pages listed)",
                id,
                k.fl_probe_serve_max,
                t.npages,
                t.maxpages
            );
        } else if room && !table {
            crate::km_log!(
                k,
                Wrn,
                "FWCCB: FREELIST_GROW id={} -> no grow: {} grows served, the host's table of grows holds {} ({}/{} pages listed)",
                id,
                t.grows,
                FL_GROWS_MAX,
                t.npages,
                t.maxpages
            );
        } else if asked_kib != 0 {
            crate::km_log!(
                k,
                Wrn,
                "FWCCB: FREELIST_GROW id={} -> no grow: the render pool refused {} KiB ({}/{} pages listed)",
                id,
                asked_kib,
                t.npages,
                t.maxpages
            );
        } else {
            crate::km_log!(
                k,
                Wrn,
                "FWCCB: FREELIST_GROW id={} -> no grow (reserve {}, pool {}/{}, {}/{} pages)",
                id,
                t.reserve,
                k.fl_pool_used,
                k.fl_pool_pages,
                t.npages,
                t.maxpages
            );
        }
        if let Some(cause) = stop_cause.filter(|_| t.grow != 0) {
            stop(k, cause, id, asked_kib);
        }
        return 0;
    };
    let start = t.maxpages - t.npages - t.grow;
    for j in 0..t.grow {
        // SAFETY: start + grow <= maxpages (`room`).
        unsafe { arr_set(t.arr, start + j, pfn(src + u64::from(j) * 4096)) };
    }
    k.flush(t.arr.wrapping_add(start as usize), t.grow as usize * 4);
    k.fence();
    match how {
        Src::Own => {
            t.backed += t.grow;
            t.reserve -= t.grow;
        }
        Src::Pool => {
            k.fl_pool_used += t.grow;
            k.fl_pool_peak = k.fl_pool_used;
        }
        Src::Spare => k.fl_spare_pfn[i] = 0,
        Src::OnDemand => {}
    }
    k.fl_grow_pfn[i][t.grows as usize] = pfn(src);
    t.npages += t.grow;
    // drm/imagination's `pvr_free_list_grow`: the new total's share is kept
    // as the firmware's ready pages, the rest is current. The answer is
    // NEW = current + ready, READY = ready ([`grow`]). A refused grow keeps
    // ready 0, as upstream (the firmware already took them).
    t.ready = ready_pages(t.npages, t.grow, t.grow_threshold);
    t.npages -= t.ready;
    t.grows += 1;
    k.fl_grows_done += 1;
    k.fls[i] = t;
    match how {
        Src::Own | Src::Pool => crate::km_log!(
            k,
            Inf,
            "FWCCB: FREELIST_GROW id={} -> +{} pages ({} listed, {} in reserve, pool {}/{})",
            id,
            t.grow,
            t.npages + t.ready,
            t.reserve,
            k.fl_pool_used,
            k.fl_pool_pages
        ),
        Src::Spare => crate::km_log!(
            k,
            Inf,
            "FWCCB: FREELIST_GROW id={} -> +{} pages from its refilled reserve ({} listed, first page phys 0x{:x})",
            id,
            t.grow,
            t.npages + t.ready,
            src
        ),
        Src::OnDemand => crate::km_log!(
            k,
            Inf,
            "FWCCB: FREELIST_GROW id={} -> +{} pages from the render pool ({} listed, {} KiB taken on demand, first page phys 0x{:x})",
            id,
            t.grow,
            t.npages + t.ready,
            t.pool_kib,
            src
        ),
    }
    if let Some(f) = ondemand {
        refill(k, i, f);
    }
    t.grow
}

/// Refill list `i`'s one-grow reserve from the render pool after a served
/// grow, when the list has no reserve of its own left, there is no shared
/// pool, and it has room for another grow. A refusal is counted and said; the
/// next grow of the list then asks the render pool when the firmware asks.
fn refill(k: &mut YdPvrKm, i: usize, f: &mut GrowSource<'_>) {
    let t = k.fls[i];
    let room = t.maxpages.wrapping_sub(t.npages.wrapping_add(t.ready)) >= t.grow;
    if t.reserve >= t.grow
        || k.fl_pool_pages != 0
        || k.fl_spare_pfn[i] != 0
        || !room
        || (t.grows as usize) >= FL_GROWS_MAX
    {
        return;
    }
    let bytes = ondemand_bytes(t.grow);
    match f(k, bytes) {
        Some(bo) => {
            let first = round_up(bo_phys(k, &bo), 0x4000);
            k.fl_spare_pfn[i] = pfn(first);
            k.fls[i].pool_kib = t.pool_kib.wrapping_add(bytes / 1024);
            crate::km_log!(
                k,
                Inf,
                "FWCCB: FREELIST_GROW id={} reserve refilled: {} KiB from the render pool ({} KiB taken on demand, first page phys 0x{:x})",
                t.id,
                bytes / 1024,
                k.fls[i].pool_kib,
                first
            );
        }
        None => {
            k.fl_refills_refused = k.fl_refills_refused.wrapping_add(1);
            k.fl_refill_refused_kib = k.fl_refill_refused_kib.wrapping_add(bytes / 1024);
            crate::km_log!(
                k,
                Wrn,
                "FWCCB: FREELIST_GROW id={} reserve not refilled: the render pool refused {} KiB; the next grow asks it again",
                t.id,
                bytes / 1024
            );
        }
    }
}

/// The physical address of list `t`'s page `p` in listing order: the pages
/// it was built with from its backing, then each served grow's pages from
/// [`YdPvrKm::fl_grow_pfn`]. For grows from the list's own reserve this is
/// `backing_phys + p * 4096`, the order the lists were always rebuilt in.
fn page_phys(k: &YdPvrKm, i: usize, p: u32) -> u64 {
    let t = &k.fls[i];
    let built = t.npages.wrapping_add(t.ready).wrapping_sub(t.grows.wrapping_mul(t.grow));
    if p < built || t.grow == 0 {
        return t.backing_phys + u64::from(p) * 4096;
    }
    let g = ((p - built) / t.grow) as usize;
    let first = k.fl_grow_pfn[i].get(g).copied().unwrap_or(0);
    (u64::from(first) << 12) + u64::from((p - built) % t.grow) * 4096
}

/// Answer a FREELIST_GROW for `id` (`c`: the first six words of the
/// firmware's command, for the log): the DDK's RGXProcessRequestGrow. The
/// ready pages become current, then one grow's worth of new pages is listed
/// below the stack ([`serve`]) and the firmware is told the delta and the new
/// totals on the KCCB.
pub fn grow(k: &mut YdPvrKm, id: u32, c: &[u32; 6], resp: &mut KccbCmd, ondemand: Option<&mut GrowSource<'_>>) {
    k.fl_grow_reqs += 1;
    let ti = by_id(k, id);
    if let Some(i) = ti.filter(|&i| k.fls[i].grow_threshold != 0) {
        fw_before_the_answer(k, i);
    }
    let delta = match ti {
        Some(i) => serve(k, i, ondemand),
        None => {
            crate::km_log!(k, Wrn, "FWCCB: FREELIST_GROW id={} -> unknown freelist", id);
            0
        }
    };
    let t = ti.map(|i| k.fls[i]);
    put32(resp, KCCB_CMD_TYPE, KCCB_CMD_FREELIST_GROW_UPDATE);
    put32(resp, KCCB_FL_GS_FWADDR, t.map_or(0, |t| t.fw_addr));
    put32(resp, KCCB_FL_GS_DELTA, delta);
    put32(resp, KCCB_FL_GS_NEW, t.map_or(0, |t| t.npages + t.ready));
    put32(resp, KCCB_FL_GS_READY, t.map_or(0, |t| t.ready));
    kccb_post(k, resp);

    // A grow is where every Qt Quick 3D scene used to stall: watch the
    // firmware's own answer to the update -- does it take the KCCB command,
    // and what does it then hold in the freelist structure?
    if let Some(t) = t {
        let kctl = k.heap_cpu(k.kccb_ctl_off);
        let fl = k.heap_cpu(t.heap_off);
        let mut ms = 0u32;
        while ms < 200 {
            k.invd(kctl, CCB_CTL_SIZE);
            // SAFETY: the KCCB control block.
            if unsafe { rd32(kctl, CCB_CTL_READ_OFFSET) == rd32(kctl, CCB_CTL_WRITE_OFFSET) } {
                break;
            }
            k.busy_wait_us(1000);
            ms += 1;
        }
        k.invd(fl, FREELIST_SIZE);
        let mut list = ListBuf::new();
        for f in &k.fls[..k.nfls as usize] {
            let _ = write!(list, "id={} fw=0x{:08x} ", f.id, f.fw_addr);
        }
        crate::km_log!(
            k,
            Dbg,
            "FWCCB: grow req: cmd dwords {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}; our freelists: {}",
            c[0],
            c[1],
            c[2],
            c[3],
            c[4],
            c[5],
            list.as_str()
        );
        // SAFETY: the KCCB control block and the freelist structure.
        unsafe {
            crate::km_log!(
                k,
                Inf,
                "FWCCB: grow ack: KCCB r/w={}/{} after {} ms; fl cur={} top={} ready={} pending={} alloc={}/{} base=0x{:x} cur_dev=0x{:x}",
                rd32(kctl, CCB_CTL_READ_OFFSET),
                rd32(kctl, CCB_CTL_WRITE_OFFSET),
                ms,
                rd32(fl, FREELIST_CURRENT_PAGES),
                rd32(fl, FREELIST_CURRENT_STACK_TOP),
                rd32(fl, FREELIST_READY_PAGES),
                fl.add(FREELIST_GROW_PENDING).read_volatile(),
                rd32(fl, FREELIST_ALLOCATED_PAGE_COUNT),
                rd32(fl, FREELIST_ALLOCATED_MMU_PAGE_COUNT),
                rd64(fl, FREELIST_DEVVADDR),
                rd64(fl, FREELIST_CURRENT_DEVVADDR)
            );
        }
    }
}

/// With the ready pages on (`CONFIG_SAFEUI_GPU_PVR_FL_READY_PAGES`: the
/// list's `grow_threshold` is not 0): print the firmware's own counts for
/// list `i` as it asks for a grow, before [`serve`] and before our answer
/// is posted. The ack line after the answer shows the same words, so the
/// pair shows what the firmware did with the answer. The open item "the
/// grow answer's READY word has no effect visible in the ack"
/// (docs/notes/pvr-render-pool.md) needs the words before: at
/// the first request they show whether the firmware took the READY that `build`
/// gave it. Reads only; nothing is written.
fn fw_before_the_answer(k: &YdPvrKm, i: usize) {
    let t = k.fls[i];
    let fl = k.heap_cpu(t.heap_off);
    k.invd(fl, FREELIST_SIZE);
    // SAFETY: the list's RGXFWIF_FREELIST.
    let (cur, top, ready, pending, alloc, alloc_mmu) = unsafe {
        (
            rd32(fl, FREELIST_CURRENT_PAGES),
            rd32(fl, FREELIST_CURRENT_STACK_TOP),
            rd32(fl, FREELIST_READY_PAGES),
            fl.add(FREELIST_GROW_PENDING).read_volatile(),
            rd32(fl, FREELIST_ALLOCATED_PAGE_COUNT),
            rd32(fl, FREELIST_ALLOCATED_MMU_PAGE_COUNT),
        )
    };
    crate::km_log!(
        k,
        Inf,
        "FWCCB: grow req id={}: fw before the answer: cur={} top={} ready={} pending={} alloc={}/{}; host {} current + {} ready",
        t.id,
        cur,
        top,
        ready,
        pending,
        alloc,
        alloc_mmu,
        t.npages,
        t.ready
    );
}

/// Answer FREELISTS_RECONSTRUCTION for freelist `id`, after a hardware
/// recovery, as Linux drm/imagination's `pvr_free_list_reconstruct` does
/// (Linux 7.3-rc5, drivers/gpu/drm/imagination/pvr_free_list.c, MIT; see the
/// module comment):
/// - the PFN array is re-populated with every page the list holds;
/// - in the firmware structure only the stack top (from the firmware's own
///   current page count) and the two allocated counts are written. The
///   current, ready and grow-pending fields are the firmware's and stay as
///   they are. This code used to overwrite them, with the host's count of
///   listed pages (current + ready) and 0;
/// - every HWRT data in `hwrts` (heap offsets of the render service's built
///   members) that uses the list goes to `RGXFWIF_RTDATA_STATE_HWR` and loses
///   `HWRTDATA_HAS_LAST_TA` ([`reset_hwrts`]). This code used to leave them
///   as the reset found them. On the AM62P (4c25118c), after one recovery
///   each later one faulted the TA at VA 0x40, 536 times.
///
/// The pages are listed in our order (`page_phys`: the build, then each
/// grow, from the bottom of the stack), not upstream's (the build at the top,
/// each grow below). Both list the same set. With the firmware's counts left
/// alone, both put valid pages of the list under its stack. Ours keeps the
/// km records of the C for lists that grew from their own reserve.
/// An unknown ID is ignored.
pub fn reconstruct(k: &mut YdPvrKm, id: u32, hwrts: &[u32]) {
    let Some(i) = by_id(k, id) else {
        return;
    };
    let t = k.fls[i];
    let listed = t.npages.wrapping_add(t.ready);
    let start = t.maxpages.wrapping_sub(listed);
    for j in 0..t.maxpages {
        // SAFETY: the array has maxpages entries.
        unsafe { arr_set(t.arr, j, 0) };
    }
    // Every page the list holds, in listing order (`page_phys`): its
    // backing, then each grow's pages wherever they came from. Before the
    // grow table, this was `backing_phys + j`, which after a grow from the
    // shared or the render pool handed the PM pages of other objects.
    for j in 0..listed {
        // SAFETY: start + listed == maxpages.
        unsafe { arr_set(t.arr, start + j, pfn(page_phys(k, i, j))) };
    }
    k.flush(t.arr, t.maxpages as usize * 4);
    let fl = k.heap_cpu(t.heap_off);
    k.invd(fl, FREELIST_SIZE);
    // SAFETY: the list's RGXFWIF_FREELIST.
    let (current, ready) = unsafe {
        let current = rd32(fl, FREELIST_CURRENT_PAGES);
        wr32(fl, FREELIST_CURRENT_STACK_TOP, current.wrapping_sub(1));
        wr32(fl, FREELIST_ALLOCATED_PAGE_COUNT, 0);
        wr32(fl, FREELIST_ALLOCATED_MMU_PAGE_COUNT, 0);
        (current, rd32(fl, FREELIST_READY_PAGES))
    };
    k.flush(fl, FREELIST_SIZE);
    let reset = reset_hwrts(k, t.fw_addr, hwrts);
    k.fence();
    crate::km_log!(
        k,
        Inf,
        "freelist id={} reconstructed: {} pages listed; firmware current {} ready {}, stack top {}; {} HWRT datas reset to HWR",
        id,
        listed,
        current,
        ready,
        current.wrapping_sub(1),
        reset
    );
}

/// Every HWRT data in `hwrts` (firmware-heap offsets) whose freelists
/// include the one at firmware address `fl_fw_addr` goes to
/// `RGXFWIF_RTDATA_STATE_HWR` and loses `HWRTDATA_HAS_LAST_TA`: the reset of
/// the associated HWRTs in drm/imagination's `pvr_free_list_reconstruct`.
pub fn reset_hwrts(k: &YdPvrKm, fl_fw_addr: u32, hwrts: &[u32]) -> u32 {
    let mut n = 0;
    for &off in hwrts {
        let h = k.heap_cpu(off);
        k.invd(h, HWRTDATA_SIZE);
        // SAFETY: an HWRT data the render service built (256 bytes).
        let uses =
            (0..3).any(|n| unsafe { rd32(h, HWRTDATA_FREELISTS + n * HWRTDATA_FREELIST_PTR_SIZE) } == fl_fw_addr);
        if !uses {
            continue;
        }
        // SAFETY: as above.
        unsafe {
            wr32(h, HWRTDATA_STATE, RTDATA_STATE_HWR);
            let f = rd32(h, HWRTDATA_FLAGS);
            wr32(h, HWRTDATA_FLAGS, f & !HWRTDATA_HAS_LAST_TA);
        }
        k.flush(h, HWRTDATA_SIZE);
        n += 1;
    }
    n
}

/// Lab audit of the PFN arrays: the listed entries must hold PFNs of their
/// own pages (the backing, or a served grow's pages from another source, in
/// [`YdPvrKm::fl_grow_pfn`]) or 0, and every other entry must be zero; the
/// first four bad ones are logged. Returns how many there are.
pub fn audit(k: &YdPvrKm) -> u32 {
    let mut bad = 0u32;
    for (li, t) in k.fls[..k.nfls as usize].iter().enumerate() {
        let grows = &k.fl_grow_pfn[li][..(t.grows as usize).min(FL_GROWS_MAX)];
        let in_grow = |v: u32| grows.iter().any(|&g| g != 0 && v >= g && v - g < t.grow);
        let pfn_lo = (t.backing_phys >> 12) as u32;
        let pfn_hi = pfn_lo.wrapping_add(t.backed).wrapping_add(t.reserve);
        let listed = t.npages.wrapping_add(t.ready);
        // the first listed entry: the builder aligns the stack base down to 16 bytes
        let base = (t.maxpages.wrapping_sub(listed).wrapping_mul(4) & !15) / 4;
        k.invd(t.arr, t.maxpages as usize * 4);
        for i in 0..t.maxpages {
            // SAFETY: the array has maxpages entries.
            let v = unsafe { arr_get(t.arr, i) };
            let ok = if i >= base { (v >= pfn_lo && v < pfn_hi) || in_grow(v) || v == 0 } else { v == 0 };
            if !ok {
                if bad < 4 {
                    crate::km_log!(
                        k,
                        Err,
                        "freelist audit: id={} entry[{}]=0x{:08x} (listed from {}, backing pfn 0x{:x}..0x{:x})",
                        t.id,
                        i,
                        v,
                        base,
                        pfn_lo,
                        pfn_hi
                    );
                }
                bad += 1;
            }
        }
    }
    bad
}

/// A short line buffer (the freelist list of the grow log).
struct ListBuf {
    buf: [u8; 160],
    len: usize,
}

impl ListBuf {
    fn new() -> ListBuf {
        ListBuf { buf: [0; 160], len: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl Write for ListBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

// ---- C entry points ------------------------------------------------------------
//
// Every one: `k` is the C driver's `struct yd_pvr_km` (see [`km`]).

/// # Safety
/// See the section comment; `gen_b` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fl_pool_init(k: *mut YdPvrKm, gen_b: *mut u32, pages: u32) {
    // SAFETY: the caller's contract.
    unsafe { pool_init(km(k), &mut *gen_b, pages) }
}

/// # Safety
/// See the section comment; `gen_b` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fl_build(
    k: *mut YdPvrKm,
    gen_b: *mut u32,
    npages: u32,
    maxpages: u32,
    grow: u32,
    ready: u32,
    fl_id: u32,
) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { build(km(k), &mut *gen_b, npages, maxpages, grow, ready, fl_id) }
}

/// `CONFIG_SAFEUI_GPU_PVR_FL_READY_PAGES`: with `on` non-zero the growable
/// lists built from now on keep [`GLOBAL_GROW_THRESHOLD`] percent as the
/// firmware's ready pages; 0 keeps none (the default).
///
/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fl_set_ready_pages(k: *mut YdPvrKm, on: u32) {
    // SAFETY: the caller's contract.
    unsafe { km(k) }.fl_ready_threshold = if on != 0 { GLOBAL_GROW_THRESHOLD } else { 0 };
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fl_audit(k: *const YdPvrKm) -> u32 {
    // SAFETY: the caller's contract.
    audit(unsafe { &*k })
}

#[cfg(test)]
mod grow_tests {
    //! Gate 1 of the on-demand grow
    //! (docs/notes/pvr-render-pool.md): where a grow's pages
    //! come from, how they are listed, and what is counted and said, on a host
    //! carveout. [`serve`] is the grow without the KCCB post.
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::cell::RefCell;
    use std::string::String;
    use std::vec::Vec;

    std::thread_local! {
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop_fence(_: *mut c_void) {}
    unsafe extern "C" fn nop_wait(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn log(_: *mut c_void, _: u32, buf: *const u8, len: usize) {
        // SAFETY: the crate passes `len` valid bytes.
        let s = unsafe { core::slice::from_raw_parts(buf, len) };
        LOG.with(|l| l.borrow_mut().push(String::from_utf8_lossy(s).into_owned()));
    }

    const PHYS: u64 = 0x9780_0000;
    const MAX: u32 = 4096;
    const GROW: u32 = 256;

    /// A km with one global list as the 64 MiB carveout builds it with the
    /// knob on: 512 pages listed and backed, no reserve, no shared pool.
    struct Rig {
        mem: Vec<u8>,
        arr: Vec<u32>,
        _ops: Box<YdPvrKmOps>,
        k: Box<YdPvrKm>,
    }

    impl Rig {
        fn new(reserve: u32) -> Rig {
            LOG.with(|l| l.borrow_mut().clear());
            let mut mem = std::vec![0u8; 8 << 20];
            let mut arr = std::vec![0u32; MAX as usize];
            let ops = Box::new(YdPvrKmOps {
                ctx: core::ptr::null_mut(),
                flush: nop_range,
                invd: nop_range,
                fence: nop_fence,
                busy_wait_us: nop_wait,
                log,
            });
            // SAFETY: all-zero is the documented state before the first call.
            let mut k: Box<YdPvrKm> = unsafe { Box::new(core::mem::zeroed()) };
            k.ops = &*ops;
            k.cpu = mem.as_mut_ptr();
            k.phys = PHYS;
            k.size = mem.len() as u64;
            let f = &mut k.fls[0];
            f.id = 1;
            f.arr = arr.as_mut_ptr();
            f.backing_phys = PHYS + 0x10_0000;
            f.npages = 512;
            f.maxpages = MAX;
            f.grow = GROW;
            f.backed = 512;
            f.reserve = reserve;
            f.heap_off = 0x1000; // its RGXFWIF_FREELIST, below the Blocks
            k.nfls = 1;
            Rig { mem, arr, _ops: ops, k }
        }

        fn logs(&self) -> Vec<String> {
            LOG.with(|l| l.borrow().clone())
        }

        /// The listed page numbers of the last grow: the GROW entries below
        /// the stack it was listed under.
        fn listed(&self, npages_before: u32) -> Vec<u32> {
            let start = (MAX - npages_before - GROW) as usize;
            self.arr[start..start + GROW as usize].to_vec()
        }
    }

    /// Without the knob nothing changes: the reserve serves the grow, the
    /// line is the one the km records hold, and no source is asked.
    #[test]
    fn a_grow_from_the_reserve_is_as_before_and_asks_no_source() {
        let mut r = Rig::new(1024);
        let mut asked = 0;
        let mut src = |_: &mut YdPvrKm, _: u32| {
            asked += 1;
            None
        };
        let d = serve(&mut r.k, 0, Some(&mut src));
        assert_eq!((d, asked), (GROW, 0));
        assert_eq!((r.k.fls[0].backed, r.k.fls[0].reserve, r.k.fls[0].pool_kib), (768, 768, 0));
        assert_eq!(r.listed(512)[0], ((PHYS + 0x10_0000 + 512 * 4096) >> 12) as u32);
        assert_eq!(r.logs(), ["FWCCB: FREELIST_GROW id=1 -> +256 pages (768 listed, 768 in reserve, pool 0/0)"]);
    }

    /// A render pool for the tests: block n starts at carveout offset
    /// 0x5000 + n * 0x104000 (off a 16 KiB boundary on purpose); call n is
    /// refused when `refuse(n)`. `asked` keeps every request's bytes.
    struct Blocks {
        base: *mut u8,
        asked: Vec<u32>,
        refuse: fn(usize) -> bool,
    }

    impl Blocks {
        fn new(r: &mut Rig, refuse: fn(usize) -> bool) -> Blocks {
            Blocks { base: r.mem.as_mut_ptr(), asked: Vec::new(), refuse }
        }
        /// Block n's carveout offset, and its first page on a 16 KiB boundary.
        fn off(n: usize) -> u64 {
            0x5000 + n as u64 * 0x10_4000
        }
        fn first_page(n: usize) -> u64 {
            round_up(PHYS + Self::off(n), 0x4000)
        }
        fn take(&mut self, bytes: u32) -> Option<PvrGpuBo> {
            let n = self.asked.len();
            self.asked.push(bytes);
            (!(self.refuse)(n)).then(|| PvrGpuBo {
                gpu: 0x80_0000_0000 + Self::off(n),
                cpu: self.base.wrapping_add(Self::off(n) as usize),
                heap_off: Self::off(n) as u32,
            })
        }
    }

    fn pages_from(first: u64) -> Vec<u32> {
        (0..GROW).map(|j| ((first >> 12) + u64::from(j)) as u32).collect()
    }

    /// With no reserve and no shared pool, the grow takes grow + 3 pages from
    /// the source and lists GROW consecutive pages from the first 16 KiB
    /// boundary (the source's block starts off it here on purpose); then the
    /// list's reserve is refilled with a second block the same way.
    #[test]
    fn an_on_demand_grow_lists_256_pages_from_a_16_kib_boundary() {
        let mut r = Rig::new(0);
        let mut b = Blocks::new(&mut r, |_| false);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        let d = serve(&mut r.k, 0, Some(&mut src));
        assert_eq!(d, GROW);
        assert_eq!(b.asked, [GROW * 4096 + 0x3000; 2], "the grow, then the refill");
        let first = Blocks::first_page(0);
        assert_eq!(first, PHYS + 0x8000, "0x...5000 rounded up to 16 KiB");
        assert_eq!(r.listed(512), pages_from(first));
        assert_eq!(first % 0x4000, 0, "every PM page starts on 16 KiB");
        let f = r.k.fls[0];
        assert_eq!((f.npages, f.backed, f.reserve, f.pool_kib, f.grows), (768, 512, 0, 2072, 1));
        assert_eq!(r.k.fl_grow_pfn[0][0], (first >> 12) as u32);
        assert_eq!(u64::from(r.k.fl_spare_pfn[0]) << 12, Blocks::first_page(1));
        assert_eq!(
            r.logs(),
            [
                std::format!(
                    "FWCCB: FREELIST_GROW id=1 -> +256 pages from the render pool (768 listed, 1036 KiB taken on demand, first page phys 0x{:x})",
                    first
                ),
                std::format!(
                    "FWCCB: FREELIST_GROW id=1 reserve refilled: 1036 KiB from the render pool (2072 KiB taken on demand, first page phys 0x{:x})",
                    Blocks::first_page(1)
                ),
            ]
        );
    }

    /// The pool refusing is a "no grow" with its own line, nothing listed,
    /// nothing counted as taken, and no stop (the firmware renders on).
    #[test]
    fn a_refused_on_demand_grow_is_no_grow_with_its_own_line() {
        let mut r = Rig::new(0);
        let mut src = |_: &mut YdPvrKm, _: u32| None;
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), 0);
        let f = r.k.fls[0];
        assert_eq!((f.npages, f.pool_kib, f.grows, r.k.fl_no_grow), (512, 0, 0, 1));
        assert!(r.arr.iter().all(|&v| v == 0), "nothing listed");
        assert_eq!(
            r.logs(),
            ["FWCCB: FREELIST_GROW id=1 -> no grow: the render pool refused 1036 KiB (512/4096 pages listed)"]
        );
    }

    /// Without a source (the knob off, or the C entry point) and no
    /// reserve, the answer is the old "no grow" line.
    #[test]
    fn with_no_source_and_no_reserve_the_answer_is_the_old_no_grow() {
        let mut r = Rig::new(0);
        assert_eq!(serve(&mut r.k, 0, None), 0);
        assert_eq!(r.logs(), ["FWCCB: FREELIST_GROW id=1 -> no grow (reserve 0, pool 0/0, 512/4096 pages)"]);
    }

    /// A list at its maximum asks no source: there is no room to list into.
    #[test]
    fn a_full_list_asks_no_source() {
        let mut r = Rig::new(0);
        r.k.fls[0].npages = MAX;
        let mut asked = 0;
        let mut src = |_: &mut YdPvrKm, _: u32| {
            asked += 1;
            None
        };
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), 0);
        assert_eq!(asked, 0);
    }

    /// The knob's sizing (one grow kept, `fl_reserve_mult` 1) and glmark2's
    /// two grows (gate 3): the first from the list's own reserve, the second
    /// from the reserve refilled after the first. The render pool is asked
    /// only by the refills, never while the firmware waits.
    #[test]
    fn the_reserve_serves_each_grow_and_is_refilled_after_it() {
        let mut r = Rig::new(GROW);
        let mut b = Blocks::new(&mut r, |_| false);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW);
        assert_eq!(r.listed(512), pages_from(PHYS + 0x10_0000 + 512 * 4096), "the own reserve, behind the backing");
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW);
        assert_eq!(r.listed(768), pages_from(Blocks::first_page(0)), "the refilled reserve");
        assert_eq!(b.asked, [GROW * 4096 + 0x3000; 2], "two refills, one after each grow");
        let f = r.k.fls[0];
        assert_eq!((f.npages, f.backed, f.reserve, f.pool_kib, f.grows), (1024, 768, 0, 2072, 2));
        assert_eq!((r.k.fl_no_grow, r.k.fl_refills_refused, r.k.fl_grows_done), (0, 0, 2));
        assert_eq!(
            r.k.fl_grow_pfn[0][..3],
            [((PHYS + 0x10_0000 + 512 * 4096) >> 12) as u32, (Blocks::first_page(0) >> 12) as u32, 0]
        );
        assert_eq!(u64::from(r.k.fl_spare_pfn[0]) << 12, Blocks::first_page(1));
        assert_eq!(
            r.logs(),
            [
                std::string::ToString::to_string("FWCCB: FREELIST_GROW id=1 -> +256 pages (768 listed, 0 in reserve, pool 0/0)"),
                std::format!(
                    "FWCCB: FREELIST_GROW id=1 reserve refilled: 1036 KiB from the render pool (1036 KiB taken on demand, first page phys 0x{:x})",
                    Blocks::first_page(0)
                ),
                std::format!(
                    "FWCCB: FREELIST_GROW id=1 -> +256 pages from its refilled reserve (1024 listed, first page phys 0x{:x})",
                    Blocks::first_page(0)
                ),
                std::format!(
                    "FWCCB: FREELIST_GROW id=1 reserve refilled: 1036 KiB from the render pool (2072 KiB taken on demand, first page phys 0x{:x})",
                    Blocks::first_page(1)
                ),
            ]
        );
    }

    /// A refused refill is counted and said, and the next grow asks the
    /// render pool again when the firmware asks: served when the pool has
    /// pages by then.
    #[test]
    fn a_refused_refill_is_counted_and_the_next_grow_asks_again() {
        let mut r = Rig::new(GROW);
        let mut b = Blocks::new(&mut r, |n| n == 0);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW);
        assert_eq!((r.k.fl_refills_refused, r.k.fl_refill_refused_kib, r.k.fl_spare_pfn[0]), (1, 1036, 0));
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW, "the pool has pages again");
        assert_eq!(r.listed(768), pages_from(Blocks::first_page(1)));
        assert_eq!(b.asked.len(), 3, "the refused refill, the grow, its refill");
        assert_eq!(r.k.fl_no_grow, 0);
        let l = r.logs();
        assert_eq!(
            l[1],
            "FWCCB: FREELIST_GROW id=1 reserve not refilled: the render pool refused 1036 KiB; the next grow asks it again"
        );
        assert!(
            l[2].starts_with("FWCCB: FREELIST_GROW id=1 -> +256 pages from the render pool (1024 listed, "),
            "{l:?}"
        );
    }

    /// The refusal the guarantee leaves: the refill refused, and the pool
    /// refuses again when the firmware asks. Zero pages, one refusal, the
    /// render pool's line.
    #[test]
    fn a_grow_is_refused_only_when_the_refill_and_the_grow_are_refused() {
        let mut r = Rig::new(GROW);
        let mut b = Blocks::new(&mut r, |_| true);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW, "the first grow has its reserve");
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), 0);
        assert_eq!((r.k.fl_no_grow, r.k.fl_refills_refused, r.k.fls[0].npages), (1, 1, 768));
        assert_eq!(
            r.logs().last().map(String::as_str),
            Some("FWCCB: FREELIST_GROW id=1 -> no grow: the render pool refused 1036 KiB (768/4096 pages listed)")
        );
    }

    /// A grow past the table of grows is refused by its own line, and no
    /// source is asked: the table is what reconstruction lists again.
    #[test]
    fn a_grow_past_the_table_is_refused_by_name() {
        let mut r = Rig::new(0);
        r.k.fls[0].grows = FL_GROWS_MAX as u32;
        let mut asked = 0;
        let mut src = |_: &mut YdPvrKm, _: u32| {
            asked += 1;
            None
        };
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), 0);
        assert_eq!((asked, r.k.fl_no_grow), (0, 1));
        assert_eq!(
            r.logs(),
            [
                "FWCCB: FREELIST_GROW id=1 -> no grow: 64 grows served, the host's table of grows holds 64 (512/4096 pages listed)",
                "render: stopped: a freelist grow past the host's table of grows (list 1, 0 KiB); no kick is waited for or taken from now on"
            ]
        );
    }

    /// No refill with a shared grow pool (its grows come from there), nor
    /// when the list has no room for another grow.
    #[test]
    fn no_refill_with_a_shared_pool_or_at_the_maximum() {
        let mut r = Rig::new(0);
        r.k.fl_pool_pages = 1024;
        r.k.fl_pool_phys = PHYS + 0x40_0000;
        let mut b = Blocks::new(&mut r, |_| false);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW);
        assert_eq!(r.listed(512), pages_from(PHYS + 0x40_0000));
        assert!(b.asked.is_empty(), "the shared pool served it; nothing to refill");

        let mut r = Rig::new(0);
        r.k.fls[0].npages = MAX - GROW;
        let mut b = Blocks::new(&mut r, |_| false);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), GROW);
        assert_eq!(r.k.fls[0].npages, MAX);
        assert_eq!(b.asked.len(), 1, "the grow; at the maximum there is no next grow to refill for");
        assert_eq!(r.k.fl_spare_pfn[0], 0);
    }

    /// Reconstruction lists exactly the pages the list holds: its backing,
    /// then each grow's pages wherever they came from. Before the table it
    /// listed `backing_phys + j`, which after the refilled reserve's grow are
    /// 256 pages of whatever follows the backing.
    #[test]
    fn reconstruction_lists_the_pages_the_list_holds() {
        let mut r = Rig::new(GROW);
        let mut b = Blocks::new(&mut r, |_| false);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        serve(&mut r.k, 0, Some(&mut src));
        serve(&mut r.k, 0, Some(&mut src));
        r.arr.iter_mut().for_each(|v| *v = 0xdead);
        reconstruct(&mut r.k, 1, &[]);
        let backing = (PHYS + 0x10_0000) >> 12;
        let start = (MAX - 1024) as usize;
        let mut want: Vec<u32> = (0..768).map(|j| (backing + j) as u32).collect();
        want.extend(pages_from(Blocks::first_page(0)));
        assert_eq!(r.arr[start..], want[..]);
        assert!(r.arr[..start].iter().all(|&v| v == 0), "nothing below the stack");
        assert_eq!(audit(&r.k), 0, "every listed page is the list's own");
        r.arr[start + 900] = (backing + 2000) as u32;
        assert_eq!(audit(&r.k), 1, "a page past the backing that no grow gave is flagged");
    }

    /// With grows from the list's own reserve only, reconstruction writes
    /// what it always wrote (`backing_phys + j`), so the km records hold.
    #[test]
    fn reconstruction_after_own_grows_is_the_old_listing() {
        let mut r = Rig::new(1024);
        serve(&mut r.k, 0, None);
        serve(&mut r.k, 0, None);
        reconstruct(&mut r.k, 1, &[]);
        let backing = (PHYS + 0x10_0000) >> 12;
        let start = (MAX - 1024) as usize;
        let want: Vec<u32> = (0..1024).map(|j| (backing + j) as u32).collect();
        assert_eq!(r.arr[start..], want[..]);
    }

    /// UPDATE_STATS: counted by element, said by its DDK name; a value
    /// outside the DDK's list is counted in slot 0 and said as such.
    #[test]
    fn update_stats_is_counted_by_element_and_named() {
        let mut r = Rig::new(0);
        super::super::ccb::update_stats(&mut r.k, 2, 1);
        super::super::ccb::update_stats(&mut r.k, 2, 1);
        super::super::ccb::update_stats(&mut r.k, 9, -1);
        assert_eq!(r.k.fw_stats, [1, 0, 2, 0, 0, 0, 0]);
        assert_eq!(
            r.logs(),
            [
                "FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 1)",
                "FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 2)",
                "FWCCB: UPDATE_STATS element=9 (not a DDK element) adjust=-1 (seen 1)",
            ]
        );
    }

    /// With the knob, the refusal the guarantee leaves is counted and said,
    /// and is not a stop: the AM62P survived 235 of them (91bde95d).
    #[test]
    fn a_refused_grow_is_counted_and_not_a_stop_with_the_knob() {
        let mut r = Rig::new(GROW);
        let mut b = Blocks::new(&mut r, |_| true);
        let mut src = |_: &mut YdPvrKm, bytes: u32| b.take(bytes);
        serve(&mut r.k, 0, Some(&mut src));
        assert_eq!(serve(&mut r.k, 0, Some(&mut src)), 0);
        assert_eq!((r.k.stop_cause, r.k.fl_no_grow, r.k.fl_refills_refused), (0, 1, 1));
        assert!(r.logs().iter().all(|l| !l.starts_with("render: stopped")), "{:?}", r.logs());
        assert_eq!(
            r.logs().last().map(String::as_str),
            Some("FWCCB: FREELIST_GROW id=1 -> no grow: the render pool refused 1036 KiB (768/4096 pages listed)")
        );
    }

    /// Without the knob a refusal is the old "no grow" and nothing more:
    /// the knob-off probe of the zero-page answer must see what the firmware
    /// does with it.
    #[test]
    fn without_the_knob_a_refused_grow_is_no_stop() {
        let mut r = Rig::new(0);
        assert_eq!(serve(&mut r.k, 0, None), 0);
        assert_eq!(r.k.stop_cause, 0);
    }

    /// Without the knob, a grow the list has no room for is the at-maximum
    /// stop, as with it (the reserve x4 default at its maximum).
    #[test]
    fn without_the_knob_a_grow_at_the_maximum_is_a_stop() {
        let mut r = Rig::new(0);
        r.k.fls[0].npages = MAX;
        assert_eq!(serve(&mut r.k, 0, None), 0);
        assert_eq!((r.k.stop_cause, r.k.stop_fl), (STOP_AT_MAXIMUM, 1));
    }

    /// The probe's quota (`fl_probe_serve_max`): grows are served up to it,
    /// then refused by name, and a refusal with room is no stop.
    #[test]
    fn the_probe_refuses_grows_past_its_quota_by_name() {
        let mut r = Rig::new(4 * GROW);
        r.k.fl_probe_serve_max = 2;
        assert_eq!(serve(&mut r.k, 0, None), GROW);
        assert_eq!(serve(&mut r.k, 0, None), GROW);
        assert_eq!(serve(&mut r.k, 0, None), 0);
        assert_eq!((r.k.stop_cause, r.k.fl_no_grow, r.k.fls[0].grows), (0, 1, 2));
        assert_eq!(
            r.logs().last().map(String::as_str),
            Some("FWCCB: FREELIST_GROW id=1 -> no grow: the probe's limit of 2 served grows (1024/4096 pages listed)")
        );
    }

    /// A grow past the table is its own cause; the first cause stays when a
    /// second one follows (both are said).
    #[test]
    fn the_table_is_its_own_stop_cause_and_the_first_cause_stays() {
        let mut r = Rig::new(0);
        r.k.fls[0].grows = FL_GROWS_MAX as u32;
        let mut src = |_: &mut YdPvrKm, _: u32| None;
        serve(&mut r.k, 0, Some(&mut src));
        assert_eq!(r.k.stop_cause, STOP_GROW_TABLE_FULL);
        stop(&mut r.k, STOP_AT_MAXIMUM, 1, 0);
        assert_eq!(r.k.stop_cause, STOP_GROW_TABLE_FULL);
        assert_eq!(r.logs().iter().filter(|l| l.starts_with("render: stopped: ")).count(), 2);
    }

    /// The at-maximum stop needs all three: an out-of-memory notice, no grow
    /// request in the batch, and a list at its maximum.
    #[test]
    fn the_at_maximum_stop_needs_the_notice_alone_and_a_full_list() {
        let mut r = Rig::new(0);
        r.k.fls[0].npages = MAX;
        super::super::ccb::at_maximum(&mut r.k, 1, 1);
        assert_eq!(r.k.stop_cause, 0, "a grow in the batch: the notice is the grow's");
        super::super::ccb::at_maximum(&mut r.k, 0, 0);
        assert_eq!(r.k.stop_cause, 0, "no notice");
        r.k.fls[0].npages = MAX - GROW;
        super::super::ccb::at_maximum(&mut r.k, 0, 1);
        assert_eq!(r.k.stop_cause, 0, "room for a grow");
        r.k.fls[0].npages = MAX - GROW + 1;
        super::super::ccb::at_maximum(&mut r.k, 0, 1);
        assert_eq!((r.k.stop_cause, r.k.stop_fl), (STOP_AT_MAXIMUM, 1), "less than a grow left");
    }

    /// The list's firmware address, and two HWRT datas for the
    /// reconstruction tests (firmware-heap offsets; the Rig maps heap offsets
    /// 1:1 onto its buffer, below the Blocks).
    const FL_FW: u32 = 0xC000_1000;
    const HWRT_A: u32 = 0x2000;
    const HWRT_B: u32 = 0x2100;

    fn w32(r: &mut Rig, off: u32, at: usize, v: u32) {
        // SAFETY: inside the Rig's buffer.
        unsafe { wr32(r.k.heap_cpu(off), at, v) }
    }
    fn r32(r: &Rig, off: u32, at: usize) -> u32 {
        // SAFETY: as above.
        unsafe { rd32(r.k.heap_cpu(off), at) }
    }

    /// A list grown twice to 1024 listed pages, whose firmware structure says
    /// what the AM62P's did after the second grow (4c25118c: "cur=768 ...
    /// ready=256"), with a grow pending and pages allocated.
    fn grown_twice() -> Rig {
        let mut r = Rig::new(1024);
        r.k.fls[0].fw_addr = FL_FW;
        serve(&mut r.k, 0, None);
        serve(&mut r.k, 0, None);
        let fl = r.k.fls[0].heap_off;
        w32(&mut r, fl, FREELIST_CURRENT_PAGES, 768);
        w32(&mut r, fl, FREELIST_READY_PAGES, 256);
        w32(&mut r, fl, FREELIST_CURRENT_STACK_TOP, 700);
        w32(&mut r, fl, FREELIST_ALLOCATED_PAGE_COUNT, 100);
        w32(&mut r, fl, FREELIST_ALLOCATED_MMU_PAGE_COUNT, 50);
        // SAFETY: the list's RGXFWIF_FREELIST in the Rig's buffer.
        unsafe { r.k.heap_cpu(fl).add(FREELIST_GROW_PENDING).write_volatile(1) };
        r
    }

    /// The reconstruction writes the stack top from the firmware's own
    /// current count and zeroes the allocated counts. Current, ready and
    /// grow-pending stay the firmware's (drm/imagination's
    /// pvr_free_list_reconstruct). Before, it wrote current 1024 (the host's
    /// listed count, current + ready), ready 0 and pending 0.
    #[test]
    fn reconstruction_leaves_the_firmware_counts_and_writes_the_stack_top() {
        let mut r = grown_twice();
        reconstruct(&mut r.k, 1, &[]);
        let fl = r.k.fls[0].heap_off;
        assert_eq!(r32(&r, fl, FREELIST_CURRENT_PAGES), 768, "current stays the firmware's");
        assert_eq!(r32(&r, fl, FREELIST_READY_PAGES), 256, "ready stays the firmware's");
        // SAFETY: as in grown_twice.
        assert_eq!(unsafe { r.k.heap_cpu(fl).add(FREELIST_GROW_PENDING).read_volatile() }, 1, "grow pending stays");
        assert_eq!(r32(&r, fl, FREELIST_CURRENT_STACK_TOP), 767, "the stack top from the firmware's current");
        assert_eq!(
            (r32(&r, fl, FREELIST_ALLOCATED_PAGE_COUNT), r32(&r, fl, FREELIST_ALLOCATED_MMU_PAGE_COUNT)),
            (0, 0)
        );
        assert_eq!(r.k.fls[0].npages, 1024, "the host's count is unchanged");
        assert_eq!(audit(&r.k), 0, "every listed page is the list's own");
    }

    /// Every HWRT data that uses the list goes to RTDATA_STATE_HWR and loses
    /// HAS_LAST_TA; one that does not use it is left alone. Before, no HWRT
    /// was touched, and after one recovery on the AM62P every later one
    /// faulted the TA at VA 0x40 (4c25118c, 536 recoveries).
    #[test]
    fn reconstruction_resets_every_hwrt_that_uses_the_list() {
        let mut r = grown_twice();
        for (off, fl) in [(HWRT_A, FL_FW), (HWRT_B, 0xC000_9000)] {
            w32(&mut r, off, HWRTDATA_FREELISTS + GLOBAL_FREELIST * HWRTDATA_FREELIST_PTR_SIZE, fl);
            w32(&mut r, off, HWRTDATA_STATE, 1); // KICKTA
            w32(&mut r, off, HWRTDATA_FLAGS, HWRTDATA_HAS_LAST_TA | 0x1);
        }
        reconstruct(&mut r.k, 1, &[HWRT_A, HWRT_B]);
        assert_eq!(
            r.logs().last().map(String::as_str),
            Some("freelist id=1 reconstructed: 1024 pages listed; firmware current 768 ready 256, stack top 767; 1 HWRT datas reset to HWR")
        );
        assert_eq!((r32(&r, HWRT_A, HWRTDATA_STATE), r32(&r, HWRT_A, HWRTDATA_FLAGS)), (RTDATA_STATE_HWR, 0x1));
        assert_eq!(
            (r32(&r, HWRT_B, HWRTDATA_STATE), r32(&r, HWRT_B, HWRTDATA_FLAGS)),
            (1, HWRTDATA_HAS_LAST_TA | 0x1),
            "an HWRT that does not use the list is left alone"
        );
    }
}

#[cfg(test)]
mod ready_pages_tests {
    //! The firmware's ready pages (`CONFIG_SAFEUI_GPU_PVR_FL_READY_PAGES`):
    //! what [`build`] writes into the `RGXFWIF_FREELIST` and what [`grow`]
    //! answers on the KCCB, on a host carveout with the real bump and MMU. The
    //! list is list 1 as the 64 MiB carveout builds it with the knob on: 448
    //! current + 64 ready = 512 listed of 4096, grow 256.
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use crate::gpu::pvr::mmu;
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::cell::RefCell;
    use std::string::String;
    use std::vec::Vec;

    std::thread_local! {
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop(_: *mut c_void) {}
    unsafe extern "C" fn nop_wait(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn log(_: *mut c_void, _: u32, b: *const u8, n: usize) {
        // SAFETY: the crate passes n valid bytes.
        let t = String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned();
        LOG.with(|l| l.borrow_mut().push(t));
    }

    // The firmware heap's main region is the carveout's first 64 KiB.
    const KCTL: u32 = 0x1800;
    const KCMDS: u32 = 0x1840;
    const KRTN: u32 = 0x1a00;
    const HEAP: u32 = 0x4000;
    const MAIN: u32 = 0x1_0000;
    const POOL_OFF: u32 = 0x10_0000;

    struct Rig {
        _mem: Vec<u64>,
        _regs: Vec<u32>,
        _ops: Box<YdPvrKmOps>,
        k: Box<YdPvrKm>,
        off: u32,
    }

    /// `rig(0, 1)` with the firmware handed the uncached window.
    fn rig_uncached() -> Rig {
        rig_with(0, 1, 1)
    }

    /// List 1 built with `threshold` and `reserve` grows of its own.
    fn rig(threshold: u32, reserve: u32) -> Rig {
        rig_with(threshold, reserve, 0)
    }

    fn rig_with(threshold: u32, reserve: u32, fw_uncached: u32) -> Rig {
        LOG.with(|l| l.borrow_mut().clear());
        let mut mem = std::vec![0u64; (12 << 20) / 8];
        let mut regs = std::vec![0u32; 0x1000];
        let ops = Box::new(YdPvrKmOps {
            ctx: core::ptr::null_mut(),
            flush: nop_range,
            invd: nop_range,
            fence: nop,
            busy_wait_us: nop_wait,
            log,
        });
        // SAFETY: all-zero is the documented state before the first call.
        let mut k: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
        k.cpu = mem.as_mut_ptr() as *mut u8;
        k.phys = 0x9000_0000;
        k.size = (mem.len() * 8) as u64;
        k.ops = &*ops;
        k.regs = regs.as_mut_ptr() as usize;
        (k.fw_main_size, k.fw_config_off, k.heap_next) = (MAIN, u32::MAX, HEAP);
        (k.kccb_ctl_off, k.kccb_cmds_off, k.kccb_rtn_off) = (KCTL, KCMDS, KRTN);
        (k.pt_area_off, k.pt_area_end, k.pt_next) = (MAIN, POOL_OFF, MAIN);
        (k.pool_off, k.pool_next) = (POOL_OFF, POOL_OFF);
        mmu::init(&mut k);
        k.fl_reserve_mult = reserve;
        k.fl_ready_threshold = threshold;
        k.fw_uncached = fw_uncached;
        let mut gen_b = 0u32;
        let off = build(&mut k, &mut gen_b, 448, 4096, 256, 64, 1);
        Rig { _mem: mem, _regs: regs, _ops: ops, k, off }
    }

    impl Rig {
        /// The firmware's CURRENT, READY and stack top, and the current
        /// pages' base as an offset into the PFN array.
        fn fw(&self) -> (u32, u32, u32, u64) {
            let fl = self.k.heap_cpu(self.off);
            // SAFETY: the list's RGXFWIF_FREELIST.
            unsafe {
                (
                    rd32(fl, FREELIST_CURRENT_PAGES),
                    rd32(fl, FREELIST_READY_PAGES),
                    rd32(fl, FREELIST_CURRENT_STACK_TOP),
                    rd64(fl, FREELIST_CURRENT_DEVVADDR) - rd64(fl, FREELIST_DEVVADDR),
                )
            }
        }

        /// One FREELIST_GROW for list 1, refused when the reserve is gone;
        /// returns the KCCB answer's DELTA, NEW and READY words.
        fn grow(&mut self) -> (u32, u32, u32) {
            let mut resp: KccbCmd = [0; KCCB_CMD_SIZE];
            grow(&mut self.k, 1, &[0; 6], &mut resp, None);
            let c = self.k.heap_cpu(KCMDS);
            // SAFETY: KCCB slot 0 (wrap mask 0), which grow just posted.
            unsafe { (rd32(c, KCCB_FL_GS_DELTA), rd32(c, KCCB_FL_GS_NEW), rd32(c, KCCB_FL_GS_READY)) }
        }

        fn host(&self) -> (u32, u32) {
            (self.k.fls[0].npages, self.k.fls[0].ready)
        }

        /// The PFN array's entries that are set, as one range.
        fn listed(&self) -> (usize, usize) {
            let t = self.k.fls[0];
            // SAFETY: the array has maxpages entries.
            let a = unsafe { core::slice::from_raw_parts(t.arr, t.maxpages as usize) };
            let first = a.iter().position(|&v| v != 0).unwrap_or(a.len());
            (first, a[first..].iter().filter(|&&v| v != 0).count())
        }
    }

    /// The firmware address of a host object: region 0xC (the cached
    /// window) by default, region 0x4 (the uncached window) with
    /// `fw_uncached`. Checked on the address the firmware is handed for a
    /// freelist (HWRT data and the grow answers carry it).
    #[test]
    fn the_firmware_is_handed_the_uncached_window_with_the_knob() {
        let r = rig(0, 1);
        assert_eq!(r.k.fw_uncached, 0);
        assert_eq!(r.k.fls[0].fw_addr >> 28, 0xC, "0x{:08x}", r.k.fls[0].fw_addr);
        assert_eq!(r.k.heap_fwaddr(0x1234), 0xC000_1234);
        let mut k2: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
        k2.fw_uncached = 1;
        assert_eq!(k2.heap_fwaddr(0x1234), 0x4000_1234);
        assert_eq!(RISCV_SHARED_UNCACHED_DATA0_BASE, 0x4000_0000);
        let u = rig_uncached();
        assert_eq!(u.k.fls[0].fw_addr >> 28, 0x4, "0x{:08x}", u.k.fls[0].fw_addr);
        assert_eq!(u.k.fls[0].fw_addr & 0x0fff_ffff, r.k.fls[0].fw_addr & 0x0fff_ffff, "the same object");
    }

    /// drm/imagination's share: 13 % of the pages, at most a grow, a
    /// multiple of 4. The local list (no grow) gets none.
    #[test]
    fn the_share_is_upstreams() {
        let r = |p, g| ready_pages(p, g, GLOBAL_GROW_THRESHOLD);
        assert_eq!((r(512, 256), r(768, 256), r(1024, 256), r(4096, 256)), (64, 96, 132, 256));
        assert_eq!((r(2048, 1024), r(32, 0), ready_pages(512, 256, 0)), (264, 0, 0));
    }

    /// With the threshold, build lists 512 pages and tells the firmware 448
    /// of them are current and 64 ready: the stack top and the current base
    /// follow the current pages, the ready ones lie below the base. Today's
    /// build (READY 0, 512 current) fails this.
    #[test]
    fn build_writes_the_share_as_ready() {
        let r = rig(GLOBAL_GROW_THRESHOLD, 1);
        assert_eq!(r.fw(), (448, 64, 447, (4096 - 448) * 4));
        assert_eq!(r.host(), (448, 64));
        assert_eq!(r.listed(), (4096 - 512, 512), "every page of the 512 is listed");
        assert_eq!(r.k.fls[0].grow_threshold, GLOBAL_GROW_THRESHOLD);
    }

    /// Without it, build is the old one: 512 current, none ready.
    #[test]
    fn without_the_threshold_build_is_as_before() {
        let r = rig(0, 1);
        assert_eq!(r.fw(), (512, 0, 511, (4096 - 512) * 4));
        assert_eq!(r.host(), (512, 0));
        assert_eq!(r.listed(), (4096 - 512, 512));
    }

    /// Each served grow takes the ready pages back into current, lists 256
    /// more, keeps the new total's share as ready and answers NEW as current
    /// plus ready, READY as ready (`pvr_free_list_process_grow_req`). A refused
    /// grow answers NEW = every listed page and READY 0, as upstream. Today's
    /// answer (READY 0 after every grow) fails the first assertion.
    #[test]
    fn a_grow_answers_the_new_share_as_ready() {
        let mut r = rig(GLOBAL_GROW_THRESHOLD, 2);
        assert_eq!(r.grow(), (256, 768, 96));
        assert_eq!(r.host(), (672, 96));
        assert_eq!(r.listed(), (4096 - 768, 768));
        assert_eq!(r.grow(), (256, 1024, 132));
        assert_eq!(r.host(), (892, 132));
        assert_eq!(r.grow(), (0, 1024, 0), "no reserve left, no source: refused");
        assert_eq!(r.host(), (1024, 0));
    }

    /// With the ready pages on, the firmware's own counts are printed as it
    /// asks, before the answer: here as the firmware leaves them when its
    /// out-of-memory event took the 64 ready pages (current 512, ready 0, a
    /// grow pending). The host's counts follow.
    #[test]
    fn the_firmwares_counts_before_the_answer_are_said() {
        let mut r = rig(GLOBAL_GROW_THRESHOLD, 1);
        let fl = r.k.heap_cpu(r.off);
        // SAFETY: the list's RGXFWIF_FREELIST in the rig's heap.
        unsafe {
            wr32(fl, FREELIST_CURRENT_PAGES, 512);
            wr32(fl, FREELIST_CURRENT_STACK_TOP, 507);
            wr32(fl, FREELIST_READY_PAGES, 0);
            fl.add(FREELIST_GROW_PENDING).write_volatile(1);
            wr32(fl, FREELIST_ALLOCATED_PAGE_COUNT, 4);
        }
        LOG.with(|l| l.borrow_mut().clear());
        r.grow();
        let l = LOG.with(|l| l.borrow().clone());
        assert_eq!(
            l.first().map(String::as_str),
            Some("FWCCB: grow req id=1: fw before the answer: cur=512 top=507 ready=0 pending=1 alloc=4/0; host 448 current + 64 ready")
        );
    }

    /// Without the ready pages the line is absent: the grow's lines are the
    /// ones the km records hold.
    #[test]
    fn without_the_threshold_no_before_line() {
        let mut r = rig(0, 1);
        LOG.with(|l| l.borrow_mut().clear());
        r.grow();
        let l = LOG.with(|l| l.borrow().clone());
        assert!(!l.iter().any(|x| x.contains("fw before the answer")), "{l:?}");
        assert!(l[0].starts_with("FWCCB: FREELIST_GROW id=1 -> +256 pages"), "{l:?}");
    }

    /// Without the threshold the answers are the old ones: READY 0.
    #[test]
    fn without_the_threshold_a_grow_answers_ready_0() {
        let mut r = rig(0, 1);
        assert_eq!(r.grow(), (256, 768, 0));
        assert_eq!(r.host(), (768, 0));
    }

    /// A reconstruction lists every page the list holds, current and ready
    /// (upstream inserts all nodes, then takes the ready pages off the
    /// host's current count). Listing only the current pages would leave
    /// the top of the stack empty.
    #[test]
    fn reconstruction_lists_current_and_ready_pages() {
        let mut r = rig(GLOBAL_GROW_THRESHOLD, 1);
        r.grow();
        LOG.with(|l| l.borrow_mut().clear());
        reconstruct(&mut r.k, 1, &[]);
        assert_eq!(r.listed(), (4096 - 768, 768));
        assert_eq!(audit(&r.k), 0);
        let l = LOG.with(|l| l.borrow().clone());
        assert_eq!(
            l.last().map(String::as_str),
            Some("freelist id=1 reconstructed: 768 pages listed; firmware current 448 ready 64, stack top 447; 0 HWRT datas reset to HWR")
        );
    }
}
