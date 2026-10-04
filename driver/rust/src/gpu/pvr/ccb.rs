// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host <-> firmware command rings: the kernel CCB (host to firmware) and
//! the firmware CCB (firmware to host), in the layouts of the DDK's
//! `rgx_fwif_km.h` (`RGXFWIF_CCB_CTL`, `RGXFWIF_KCCB_CMD`,
//! `RGXFWIF_FWCCB_CMD`; offsets in [`super::fwif`], pinned).
//!
//! A ring is a control block (write offset, read offset, wrap mask, counted
//! in commands) and an array of fixed-size commands; the KCCB also has one
//! return word per slot, which the firmware writes when it has executed the
//! command in that slot.
//!
//! Posting ([`kccb_post`]) is: clear the slot's return word, write the
//! command, clean both, **full barrier**, advance the write offset and clean
//! it, **full barrier**, ring the doorbell (an MTS schedule of the GP data
//! master), barrier, and read the doorbell register back. The barrier between
//! publishing the write offset and the doorbell is the release barrier the
//! "lost doorbell" stall taught us to keep (tools/cts/RESULTS.md, 2026-09:
//! the firmware woke on the doorbell, found no new write offset in memory
//! and went back to sleep with the command unread); the readback after it is
//! the DDK's `__MTSScheduleWrite`, which makes sure the write has left the
//! CPU before the function returns.
//!
//! The DDK's own ring code (`rgxccb.c`, `rgxfwutils.c`) needs its
//! device-memory and sync layers; it was read for the layouts only. Before
//! docs/source-form.md step 7 this was C in `pvr_fw_boot.c` that began in a
//! port of Linux drm/imagination (`pvr_ccb.c`); same bytes, same cleans and
//! barriers in the same order, same register accesses (`tools/pvr/km`).

use super::freelist;
use super::fwif::*;
use super::km::{km, YdPvrKm};
use crate::mmio::Mmio;
use core::ffi::{c_char, CStr};

/// KCCB slots (the DDK's `RGXFWIF_KCCB_NUMCMDS_LOG2_DEFAULT` 7) and FWCCB
/// slots (`RGXFWIF_FWCCB_NUMCMDS_LOG2` 5).
pub const KCCB_NUM_CMDS: u32 = 128;
pub const FWCCB_NUM_CMDS: u32 = 32;

/// One kernel CCB command, as the firmware reads it.
pub type KccbCmd = [u8; KCCB_CMD_SIZE];

impl YdPvrKm {
    fn kctl(&self) -> *mut u8 {
        self.heap_cpu(self.kccb_ctl_off)
    }
    fn mts_schedule(&self) {
        self.bus().write32(self.regs + CR_MTS_SCHEDULE as usize, DM_GP & 0xF);
    }
    fn mts_readback(&self) {
        let _ = self.bus().read32(self.regs + CR_MTS_SCHEDULE as usize);
    }
}

/// Ring the KCCB doorbell: ask the firmware to look at the kernel CCB.
/// Idempotent (a firmware with nothing to do goes back to idling), so the
/// render service may ring it again while it waits.
pub fn kccb_doorbell(k: &YdPvrKm) {
    k.fence();
    k.mts_schedule();
    k.fence();
    k.mts_readback();
}

/// Post one command and return the slot it went into (the firmware answers
/// in that slot of the return array).
pub fn kccb_post(k: &mut YdPvrKm, cmd: &KccbCmd) -> u32 {
    let ctl = k.kctl();
    // SAFETY: the control block, a firmware-heap object of this context.
    let (slot, ro, wrap) =
        unsafe { (rd32(ctl, CCB_CTL_WRITE_OFFSET), rd32(ctl, CCB_CTL_READ_OFFSET), rd32(ctl, CCB_CTL_WRAP_MASK)) };
    let new_wo = slot.wrapping_add(1) & wrap;
    let dst = k.heap_cpu(k.kccb_cmds_off).wrapping_add(slot as usize * KCCB_CMD_SIZE);

    if k.kccb_posts < 40 {
        k.kccb_posts += 1;
        let t = u32::from_le_bytes([cmd[0], cmd[1], cmd[2], cmd[3]]);
        crate::km_log!(
            k,
            Dbg,
            "KCCB post #{}: slot {} cmd 0x{:08x} (wo {}->{}, ro {}, wrap 0x{:x})",
            k.kccb_posts,
            slot,
            t,
            slot,
            new_wo,
            ro,
            wrap
        );
    }
    // The firmware answers a command in its slot of the return array, and a
    // slot is reused every KCCB_NUM_CMDS commands: clear the previous answer
    // before the new command occupies it (the DDK does the same;
    // RGXFWIF_KCCB_RTN_SLOT_NO_RESPONSE is "set by host").
    let rtn = k.heap_cpu(k.kccb_rtn_off).wrapping_add(slot as usize * 4);
    // SAFETY: the return array has KCCB_NUM_CMDS words; slot <= wrap mask.
    unsafe { wr32(rtn, 0, KCCB_RTN_SLOT_NO_RESPONSE) };
    k.flush(rtn, 4);
    // SAFETY: the command array has a slot of KCCB_CMD_SIZE bytes here.
    unsafe { core::ptr::copy_nonoverlapping(cmd.as_ptr(), dst, KCCB_CMD_SIZE) };
    k.flush(dst, KCCB_CMD_SIZE);
    k.fence();
    // SAFETY: the control block.
    unsafe { wr32(ctl, CCB_CTL_WRITE_OFFSET, new_wo) };
    k.flush(ctl, CCB_CTL_SIZE);
    // The release barrier: the write offset is in memory before the doorbell.
    k.fence();
    k.mts_schedule();
    k.fence();
    k.mts_readback();
    slot
}

/// Print-only: where KCCB slot `slot` and the KCCB write offset live (heap
/// offset, firmware address, physical address, 64-byte line numbers), the
/// read/write offsets, and the slot's command type and MMUCACHE value as the
/// host sees them.
pub fn kccb_probe_slot(k: &YdPvrKm, slot: u32, why: &str) {
    let stride = KCCB_CMD_SIZE as u32;
    // 64-byte lines assumed (the A53's); the absolute offsets are printed so
    // another line size can be applied to the log afterwards.
    let line = 64u32;
    let s_off = k.kccb_cmds_off + slot * stride;
    let s_end = s_off + stride - 1;
    let w_off = k.kccb_ctl_off + CCB_CTL_WRITE_OFFSET as u32;
    let ctl = k.kctl();
    let c = k.heap_cpu(s_off);

    k.invd(ctl, CCB_CTL_SIZE);
    k.invd(c, KCCB_CMD_SIZE);
    // SAFETY: the control block and a command slot.
    let (ro, wo, ty, val) = unsafe {
        (
            rd32(ctl, CCB_CTL_READ_OFFSET),
            rd32(ctl, CCB_CTL_WRITE_OFFSET),
            rd32(c, KCCB_CMD_TYPE),
            rd32(c, KCCB_MMUCACHE_SYNC_VALUE),
        )
    };
    crate::km_log!(
        k,
        Inf,
        "KCCB probe {}: slot {} at heap 0x{:x}..0x{:x} fw 0x{:08x} phys 0x{:x} lines {}..{} (first line {} the previous slot); \
         write_offset word heap 0x{:x} fw 0x{:08x} line {}; r/w {}/{}; slot holds cmd 0x{:08x} value {}",
        why,
        slot,
        s_off,
        s_end,
        k.heap_fwaddr(s_off),
        k.heap_phys(s_off),
        s_off / line,
        s_end / line,
        if slot != 0 && !s_off.is_multiple_of(line) { "shared with" } else { "not shared with" },
        w_off,
        k.heap_fwaddr(w_off),
        w_off / line,
        ro,
        wo,
        ty,
        val
    );
}

/// Service the firmware's requests on the FWCCB: freelist grow and
/// reconstruction (answered on the KCCB; the firmware stalls the data
/// masters until then), context-reset notifications (logged). Returns the
/// number of commands taken. A grow is served from the list's reserve or
/// the shared grow pool ([`fwccb_service_with`] adds the render pool).
pub fn fwccb_service(k: &mut YdPvrKm) -> i32 {
    fwccb_service_with(k, None, &[], None)
}

/// [`fwccb_service`], with `ondemand` serving a grow the reserve and the
/// shared grow pool cannot ([`freelist::GrowSource`]). With `ondemand` (the
/// knob), an out-of-memory notice in a batch with no grow request, while a
/// list is at its maximum, stops the render service ([`at_maximum`]).
/// `hwrts`: the firmware-heap offsets of the HWRT datas a reconstruction
/// resets ([`freelist::reconstruct`]); none outside the render service.
/// `first_read` runs once, when the batch's first command is found and
/// before any command is handled or said (the render service samples the
/// kicks in flight there).
pub fn fwccb_service_with(
    k: &mut YdPvrKm,
    mut ondemand: Option<&mut freelist::GrowSource<'_>>,
    hwrts: &[u32],
    mut first_read: Option<&mut dyn FnMut(&YdPvrKm)>,
) -> i32 {
    let ctl = k.heap_cpu(k.fwccb_ctl_off);
    let ring = k.heap_cpu(k.fwccb_cmds_off);
    let mut serviced = 0i32;
    // grow requests, and the out-of-memory notices after the last of them
    let (mut grows, mut ooms_after) = (0u32, 0u32);

    k.invd(ctl, CCB_CTL_SIZE);
    loop {
        // SAFETY: the FWCCB control block.
        let (ro, wo, wrap) =
            unsafe { (rd32(ctl, CCB_CTL_READ_OFFSET), rd32(ctl, CCB_CTL_WRITE_OFFSET), rd32(ctl, CCB_CTL_WRAP_MASK)) };
        if k.fwccb_check != 0 {
            write_offset_check(k, wo, wrap);
        }
        if ro == wo {
            break;
        }
        if let Some(f) = first_read.take() {
            f(k);
        }
        let slot = ro;
        let c = ring.wrapping_add(slot as usize * FWCCB_CMD_SIZE);
        k.invd(c, FWCCB_CMD_SIZE);
        // SAFETY: a FWCCB slot.
        let mut ty = unsafe { rd32(c, FWCCB_CMD_TYPE) };
        if k.fwccb_check != 0 && ty & CMD_MAGIC_DWORD_MASK != CMD_MAGIC_DWORD_SHIFTED {
            ty = reread_empty_slot(k, c, slot, ro, wo);
        }
        let mut resp: KccbCmd = [0; KCCB_CMD_SIZE];

        if ty == FWCCB_CMD_FREELIST_GROW {
            // SAFETY: as above. The ID is 64-bit; ours are small.
            let id = unsafe { rd64(c, FWCCB_FL_GS_ID) } as u32;
            let mut words = [0u32; 6];
            for (i, w) in words.iter_mut().enumerate() {
                // SAFETY: as above; the command's first 24 bytes (for the log).
                *w = unsafe { rd32(c, 4 * i) };
            }
            freelist::grow(k, id, &words, &mut resp, ondemand.as_deref_mut());
            grows += 1;
            ooms_after = 0;
            serviced += 1;
        } else if ty == FWCCB_CMD_FREELISTS_RECONSTRUCTION {
            // SAFETY: as above.
            let mut n = unsafe { rd32(c, FWCCB_FL_RECON_COUNT) };
            if n > MAX_FREELISTS_TO_RECONSTRUCT {
                n = MAX_FREELISTS_TO_RECONSTRUCT;
            }
            put32(&mut resp, KCCB_CMD_TYPE, KCCB_CMD_FREELISTS_RECONSTRUCTION_UPDATE);
            put32(&mut resp, KCCB_FL_RECON_COUNT, n);
            for i in 0..n as usize {
                // SAFETY: as above; i < 18 IDs.
                let id = unsafe { rd64(c, FWCCB_FL_RECON_IDS + 8 * i) } as u32;
                freelist::reconstruct(k, id, hwrts);
                put64(&mut resp, KCCB_FL_RECON_IDS + 8 * i, u64::from(id));
            }
            crate::km_log!(k, Inf, "FWCCB: FREELISTS_RECON count={} -> reconstructed+ack", n);
            kccb_post(k, &resp);
            serviced += 1;
        } else if ty & 0xFF == FWCCB_CMD_CONTEXT_RESET_NOTIFICATION & 0xFF {
            // RGXFWIF_FWCCB_CMD_CONTEXT_RESET_DATA read as words: ctxid,
            // reason, DM, job ref, flags, pad, PC lo/hi, MMU status lo/hi.
            let mut u = [0u32; 10];
            for (i, w) in u.iter_mut().enumerate() {
                // SAFETY: as above; 40 bytes of the command's data.
                *w = unsafe { rd32(c, FWCCB_CMD_DATA + 4 * i) };
            }
            crate::km_log!(
                k,
                Err,
                "FWCCB: CONTEXT_RESET ctxid={} reason={} dm={} jobref={} flags=0x{:x} PC=0x{:08x}{:08x} MMU=0x{:08x}{:08x}",
                u[0],
                u[1],
                u[2],
                u[3],
                u[4],
                u[7],
                u[6],
                u[9],
                u[8]
            );
            serviced += 1;
        } else if ty == FWCCB_CMD_UPDATE_STATS {
            // SAFETY: as above; RGXFWIF_FWCCB_CMD_UPDATE_STATS_DATA.
            let (e, adj) = unsafe { (rd32(c, FWCCB_STATS_ELEMENT), rd32(c, FWCCB_STATS_ADJUST) as i32) };
            update_stats(k, e, adj);
            ooms_after += u32::from(e == FWCCB_STATS_OUT_OF_MEMORY);
            serviced += 1;
        } else {
            crate::km_log!(k, Inf, "FWCCB: cmd type=0x{:08x} (unhandled, skipping)", ty);
            serviced += 1;
        }

        // SAFETY: the control block.
        unsafe { wr32(ctl, CCB_CTL_READ_OFFSET, slot.wrapping_add(1) & wrap) };
        k.flush(ctl, CCB_CTL_SIZE);
        k.fence();
        k.invd(ctl, CCB_CTL_SIZE);
    }
    at_maximum(k, grows, ooms_after);
    serviced
}

/// The FWCCB check (`CONFIG_SAFEUI_GPU_PVR_FWCCB_CHECK`, `YdPvrKm::fwccb_check`):
/// the instrument for the named defect "FWCCB slot read empty (a lost
/// command)". On the AM62P the host twice read a slot whose type had no
/// command magic (0x00000000), skipped it, and the firmware's next grow
/// request was never answered (a71c9b8b, kt-3 of 157b76e3;
/// driver/docs/pvr-render-pool.md).
///
/// Linux drm/imagination's consumer (pvr_ccb.c `pvr_fwccb_process`, Linux
/// 7.3-rc5, "GPL-2.0-only OR MIT") reads the offsets with READ_ONCE, copies
/// the command, writes only `read_offset`, and drops a command without the
/// magic; its firmware objects are mapped uncached, so it needs no cache
/// operations. Ours reads through the CPU cache: the control block is
/// invalidated before each read of the offsets, the slot after the write
/// offset is read, and Zephyr's range operations end with a DSB
/// (include/zephyr/arch/arm64/cache.h:77-148, `arm64_dcache_range`), so the
/// slot's invalidate is complete before its type is read. The one thing
/// ours does that upstream cannot: `read_offset` (ours) and `write_offset`
/// (the firmware's) share one 64-byte line, and `flush(ctl)` after writing
/// `read_offset` writes back the whole line with our copy of
/// `write_offset`.
///
/// The three candidates, and what each prints:
/// - the firmware publishes the write offset before the command is visible:
///   the slot fills within the bound, "FWCCB: slot S read empty; type 0x...
///   after N us";
/// - our invalidate does not see what the firmware wrote: "FWCCB: slot S
///   read empty; still empty after 2000 us";
/// - our writeback moved the write offset back: "FWCCB: write offset went
///   back from X to Y" ([`write_offset_check`]).
///
/// Re-read slot `slot` (at `c`) every 20 us, for at most 2 ms, until its
/// type carries the command magic. Returns the type read last. A slot that
/// fills is then consumed as normal; one that does not is skipped as
/// before, now said.
fn reread_empty_slot(k: &mut YdPvrKm, c: *mut u8, slot: u32, ro: u32, wo: u32) -> u32 {
    const STEP_US: u32 = 20;
    const BOUND_US: u32 = 2000;
    k.fwccb_empty = k.fwccb_empty.wrapping_add(1);
    let mut waited = 0u32;
    let mut ty = 0u32;
    while waited < BOUND_US {
        k.busy_wait_us(STEP_US);
        waited += STEP_US;
        k.invd(c, FWCCB_CMD_SIZE);
        // SAFETY: a FWCCB slot.
        ty = unsafe { rd32(c, FWCCB_CMD_TYPE) };
        if ty & CMD_MAGIC_DWORD_MASK == CMD_MAGIC_DWORD_SHIFTED {
            k.fwccb_filled = k.fwccb_filled.wrapping_add(1);
            crate::km_log!(
                k,
                Wrn,
                "FWCCB: slot {} read empty; type 0x{:08x} after {} us (read offset {}, write offset {})",
                slot,
                ty,
                waited,
                ro,
                wo
            );
            return ty;
        }
    }
    crate::km_log!(
        k,
        Err,
        "FWCCB: slot {} read empty; still empty after {} us, type 0x{:08x} (read offset {}, write offset {})",
        slot,
        BOUND_US,
        ty,
        ro,
        wo
    );
    ty
}

/// The write offset is the firmware's: it only moves forward. One found
/// behind the last one read (by less than half the ring) can only come from
/// the host writing back its stale copy of the shared control line.
fn write_offset_check(k: &mut YdPvrKm, wo: u32, wrap: u32) {
    if k.fwccb_wo_seen != 0 {
        let back = k.fwccb_last_wo.wrapping_sub(wo) & wrap;
        if back != 0 && back <= wrap / 2 {
            k.fwccb_wo_back = k.fwccb_wo_back.wrapping_add(1);
            crate::km_log!(
                k,
                Err,
                "FWCCB: write offset went back from {} to {} (the host's writeback of the shared control line)",
                k.fwccb_last_wo,
                wo
            );
        }
    }
    k.fwccb_last_wo = wo;
    k.fwccb_wo_seen = 1;
}

/// The at-maximum stop: the firmware said "out of memory" with no grow
/// request left to explain it, and a list that grows is at its maximum.
/// `ooms_after` counts the out-of-memory notices after the batch's last grow
/// request (all of them when it had none). Each grow request brings one
/// notice of its own (every board run: at least one per request). So the
/// stop needs one notice in a batch without a grow (the probe, 0efa0642: one
/// notice, list 1 at 512 of 512), or two after the last grow (fwccb-2: the
/// twelfth grow to 4096, then two notices; the kick never completed).
/// The knob-off control's fourth notice (two after refract's grow, list 1 at
/// 1280 of 4096) has room, so it is no stop.
///
/// The rule before returned whenever the batch held any grow, and fwccb-2's
/// two notices at 4096 were never considered (driver/docs/pvr-render-pool.md,
/// "The runaway grow and the stop that did not fire").
pub fn at_maximum(k: &mut YdPvrKm, grows: u32, ooms_after: u32) {
    let needed = if grows == 0 { 1 } else { 2 };
    if ooms_after < needed {
        return;
    }
    let full = k.fls[..(k.nfls as usize).min(super::km::FL_MAX)]
        .iter()
        .find(|f| f.grow != 0 && f.maxpages.wrapping_sub(f.npages.wrapping_add(f.ready)) < f.grow)
        .map(|f| f.id);
    if let Some(id) = full {
        freelist::stop(k, freelist::STOP_AT_MAXIMUM, id, 0);
    }
}

/// The DDK's names of `RGXFWIF_FWCCB_CMD_UPDATE_STATS_TYPE`, from
/// [`FWCCB_STATS_PARTIAL_RENDERS`] to [`FWCCB_STATS_TDM_STORES`] in order.
const STATS_NAMES: [&str; 6] =
    ["partial renders", "out of memory", "TA stores", "3D stores", "CDM stores", "TDM stores"];
const _: () = assert!(STATS_NAMES.len() == (FWCCB_STATS_TDM_STORES - FWCCB_STATS_PARTIAL_RENDERS + 1) as usize);
// fw_stats: slot 0 for any other value, then one per element
const _: () = assert!(FWCCB_STATS_PARTIAL_RENDERS == 1 && FWCCB_STATS_TDM_STORES == 6);

/// An UPDATE_STATS notice: the firmware asks the host to add `adj` to one of
/// a process's statistics. The host keeps no process statistics; it counts
/// the notices by element ([`YdPvrKm::fw_stats`], 0 for a value outside the
/// DDK's list) and says each one, so a run shows what the firmware reported
/// (an out-of-memory notice came with every grow on the AM62P,
/// driver/docs/pvr-render-pool.md, the probe).
pub fn update_stats(k: &mut YdPvrKm, e: u32, adj: i32) {
    let known = (FWCCB_STATS_PARTIAL_RENDERS..=FWCCB_STATS_TDM_STORES).contains(&e);
    let slot = if known { e as usize } else { 0 };
    k.fw_stats[slot] = k.fw_stats[slot].wrapping_add(1);
    let name = if known { STATS_NAMES[(e - FWCCB_STATS_PARTIAL_RENDERS) as usize] } else { "not a DDK element" };
    crate::km_log!(k, Inf, "FWCCB: UPDATE_STATS element={} ({}) adjust={} (seen {})", e, name, adj, k.fw_stats[slot]);
}

/// A little-endian word into a command being built.
pub fn put32(cmd: &mut KccbCmd, off: usize, v: u32) {
    cmd[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn put64(cmd: &mut KccbCmd, off: usize, v: u64) {
    cmd[off..off + 8].copy_from_slice(&v.to_le_bytes());
}

// ---- C entry points ------------------------------------------------------------
//
// Every one: `k` is the C driver's `struct yd_pvr_km` (see [`km`]), its CCBs
// built.

/// # Safety
/// See the section comment; `cmd` valid for `sizeof(RGXFWIF_KCCB_CMD)` bytes.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_kccb_post(k: *mut YdPvrKm, cmd: *const u8) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { kccb_post(km(k), &*(cmd as *const KccbCmd)) }
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_kccb_doorbell(k: *const YdPvrKm) {
    // SAFETY: the caller's contract.
    kccb_doorbell(unsafe { &*k })
}

/// # Safety
/// See the section comment; `why` a NUL-terminated string (or null).
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_kccb_probe_slot(k: *const YdPvrKm, slot: u32, why: *const c_char) {
    let why = if why.is_null() {
        ""
    } else {
        // SAFETY: the caller's contract.
        unsafe { CStr::from_ptr(why) }.to_str().unwrap_or("?")
    };
    // SAFETY: the caller's contract.
    kccb_probe_slot(unsafe { &*k }, slot, why)
}

/// # Safety
/// See the section comment.
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_fwccb_service(k: *mut YdPvrKm) -> i32 {
    // SAFETY: the caller's contract.
    fwccb_service(unsafe { km(k) })
}

#[cfg(test)]
mod at_maximum_tests {
    //! The at-maximum stop on the FWCCB batch the AM62P sent in fwccb-2
    //! (build/fwccb-am62p-20261004-054022/fwccb-glmark2-on-2): twelve grow
    //! requests, served back to back from 1024 to 4096 listed pages, each with
    //! its out-of-memory notice (two after the first), then two more notices
    //! at 4096 of 4096. The kick never completed and the board's watchdog
    //! ended the run.
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use crate::gpu::pvr::mem::PvrGpuBo;
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::cell::RefCell;
    use std::string::String;
    use std::vec::Vec;

    const FCTL: u32 = 0x100;
    const RING: u32 = 0x200; // 32 slots of 160 bytes, to 0x1600
    const KCTL: u32 = 0x1800;
    const KCMDS: u32 = 0x1840;
    const KRTN: u32 = 0x1a00;
    const FL: u32 = 0x2000;

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

    /// Run fwccb-2's batch shape through the service, with list 1 starting at
    /// `listed` pages of 4096 and its own reserve serving the grows, and a
    /// render-pool source that is never needed. Returns the stop cause and
    /// the log.
    fn run(grow_notices: &[u32], tail_notices: u32, listed: u32) -> (u32, Vec<String>) {
        LOG.with(|l| l.borrow_mut().clear());
        let mut mem = std::vec![0u32; 0x3000 / 4];
        let mut regs = std::vec![0u32; 0x1000];
        let mut arr = std::vec![0u32; 4096];
        let w = |m: &mut Vec<u32>, off: u32, v: u32| m[off as usize / 4] = v;
        let mut slot = 0u32;
        let mut post = |m: &mut Vec<u32>, ty: u32, a: u32| {
            let c = RING + slot * FWCCB_CMD_SIZE as u32;
            w(m, c + FWCCB_CMD_TYPE as u32, ty);
            if ty == FWCCB_CMD_FREELIST_GROW {
                w(m, c + FWCCB_FL_GS_ID as u32, a);
            } else {
                w(m, c + FWCCB_STATS_ELEMENT as u32, a);
                w(m, c + FWCCB_STATS_ADJUST as u32, 1);
            }
            slot += 1;
        };
        for &n in grow_notices {
            post(&mut mem, FWCCB_CMD_FREELIST_GROW, 1);
            for _ in 0..n {
                post(&mut mem, FWCCB_CMD_UPDATE_STATS, FWCCB_STATS_OUT_OF_MEMORY);
            }
        }
        for _ in 0..tail_notices {
            post(&mut mem, FWCCB_CMD_UPDATE_STATS, FWCCB_STATS_OUT_OF_MEMORY);
        }
        w(&mut mem, FCTL + CCB_CTL_WRITE_OFFSET as u32, slot);
        w(&mut mem, FCTL + CCB_CTL_WRAP_MASK as u32, FWCCB_NUM_CMDS - 1);
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
        k.size = (mem.len() * 4) as u64;
        k.ops = &*ops;
        k.fw_main_size = (mem.len() * 4) as u32;
        k.fw_config_off = u32::MAX;
        k.regs = regs.as_mut_ptr() as usize;
        (k.fwccb_ctl_off, k.fwccb_cmds_off) = (FCTL, RING);
        (k.kccb_ctl_off, k.kccb_cmds_off, k.kccb_rtn_off) = (KCTL, KCMDS, KRTN);
        let f = &mut k.fls[0];
        (f.id, f.npages, f.maxpages, f.grow) = (1, listed, 4096, 256);
        (f.backed, f.reserve, f.heap_off) = (listed, 4096 - listed, FL);
        f.arr = arr.as_mut_ptr();
        k.nfls = 1;
        let mut src = |_: &mut YdPvrKm, _: u32| -> Option<PvrGpuBo> { None };
        fwccb_service_with(&mut k, Some(&mut src), &[], None);
        let cause = k.stop_cause;
        drop(regs);
        (cause, LOG.with(|l| l.borrow().clone()))
    }

    /// fwccb-2's batch: twelve grows to 4096 (two notices after the first,
    /// one after each other), then two more notices. The list is at its
    /// maximum with notices beyond the last grow's own: the stop.
    #[test]
    fn fwccb_2s_batch_stops_at_the_maximum() {
        let mut notices = std::vec![1u32; 12];
        notices[0] = 2;
        let (cause, log) = run(&notices, 1, 1024);
        assert_eq!(cause, freelist::STOP_AT_MAXIMUM, "log:\n{}", log.join("\n"));
        assert!(log.iter().any(|l| l.contains("+256 pages (4096 listed")), "the twelfth grow reached 4096: {log:?}");
        assert_eq!(
            log.last().map(String::as_str),
            Some(
                "render: stopped: the parameter manager out of memory with its freelist at the maximum (list 1, 0 KiB); no kick is waited for or taken from now on"
            )
        );
    }

    /// The same batch with room left (the knob-off control's refract: a grow
    /// to 1280 of 4096, then two notices): no stop.
    #[test]
    fn two_notices_after_a_grow_with_room_left_are_no_stop() {
        let (cause, _) = run(&[2], 0, 1024);
        assert_eq!(cause, 0);
    }

    /// A grow that reaches the maximum with only its own notice: no stop (the
    /// firmware may need nothing more).
    #[test]
    fn a_grow_to_the_maximum_with_its_own_notice_is_no_stop() {
        let (cause, _) = run(&[1], 0, 3840);
        assert_eq!(cause, 0);
    }
}

#[cfg(test)]
mod fwccb_check_tests {
    //! The FWCCB check on a host rig whose "firmware" writes a slot's type
    //! only after a number of 20 us waits: the slot is published (the write
    //! offset is ahead) before its command is visible.
    use super::*;
    use crate::gpu::pvr::km::YdPvrKmOps;
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::cell::{Cell, RefCell};
    use std::string::String;
    use std::vec::Vec;

    const CTL: u32 = 0x100;
    const RING: u32 = 0x200;

    struct Fw {
        /// The slot's type word, written after `fill_after` waits (None:
        /// never).
        slot_type: *mut u32,
        fill_after: Option<u32>,
        fill_with: u32,
        waits: Cell<u32>,
        lines: RefCell<Vec<String>>,
    }

    unsafe extern "C" fn nop_range(_: *mut c_void, _: *const c_void, _: usize) {}
    unsafe extern "C" fn nop(_: *mut c_void) {}
    unsafe extern "C" fn busy_wait(ctx: *mut c_void, _: u32) {
        // SAFETY: ctx is the test's Fw.
        let f = unsafe { &*(ctx as *const Fw) };
        f.waits.set(f.waits.get() + 1);
        if f.fill_after == Some(f.waits.get()) {
            // SAFETY: the slot's type word in the test's buffer.
            unsafe { f.slot_type.write_volatile(f.fill_with) };
        }
    }
    unsafe extern "C" fn log(ctx: *mut c_void, _: u32, b: *const u8, n: usize) {
        // SAFETY: ctx is the test's Fw; b is valid for n bytes.
        let f = unsafe { &*(ctx as *const Fw) };
        let t = String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned();
        f.lines.borrow_mut().push(t);
    }

    struct Rig {
        mem: Vec<u32>,
        fw: Box<Fw>,
        _ops: Box<YdPvrKmOps>,
        k: Box<YdPvrKm>,
    }

    /// A FWCCB of 32 slots with one command published in slot 0: an
    /// UPDATE_STATS (element 2) whose type word is 0 until the "firmware"
    /// writes it.
    fn rig(check: bool, fill_after: Option<u32>) -> Rig {
        let mut mem = std::vec![0u32; 0x2000 / 4];
        let cpu = mem.as_mut_ptr() as *mut u8;
        let w = |m: &mut Vec<u32>, off: u32, v: u32| m[off as usize / 4] = v;
        w(&mut mem, RING + FWCCB_STATS_ELEMENT as u32, FWCCB_STATS_OUT_OF_MEMORY);
        w(&mut mem, RING + FWCCB_STATS_ADJUST as u32, 1);
        w(&mut mem, CTL + CCB_CTL_WRITE_OFFSET as u32, 1);
        w(&mut mem, CTL + CCB_CTL_WRAP_MASK as u32, FWCCB_NUM_CMDS - 1);
        let fw = Box::new(Fw {
            slot_type: cpu.wrapping_add((RING as usize) + FWCCB_CMD_TYPE) as *mut u32,
            fill_after,
            fill_with: FWCCB_CMD_UPDATE_STATS,
            waits: Cell::new(0),
            lines: RefCell::new(Vec::new()),
        });
        let ops = Box::new(YdPvrKmOps {
            ctx: &*fw as *const Fw as *mut c_void,
            flush: nop_range,
            invd: nop_range,
            fence: nop,
            busy_wait_us: busy_wait,
            log,
        });
        // SAFETY: all-zero is the documented state before the first call.
        let mut k: Box<YdPvrKm> = Box::new(unsafe { core::mem::zeroed() });
        k.cpu = cpu;
        k.size = (mem.len() * 4) as u64;
        k.ops = &*ops;
        k.fw_main_size = (mem.len() * 4) as u32;
        k.fw_config_off = u32::MAX;
        k.fwccb_ctl_off = CTL;
        k.fwccb_cmds_off = RING;
        k.fwccb_check = u32::from(check);
        Rig { mem, fw, _ops: ops, k }
    }

    impl Rig {
        fn log(&self) -> Vec<String> {
            self.fw.lines.borrow().clone()
        }
        fn read_offset(&self) -> u32 {
            self.mem[(CTL as usize + CCB_CTL_READ_OFFSET) / 4]
        }
    }

    /// With the check, a slot published before its command is visible is
    /// re-read until it fills (here after 3 waits, 60 us) and then consumed:
    /// the notice is counted. Without the check (today's consumer, below)
    /// the same slot is skipped and the command is lost.
    #[test]
    fn an_empty_slot_that_fills_is_reread_and_consumed() {
        let mut r = rig(true, Some(3));
        assert_eq!(fwccb_service(&mut r.k), 1);
        assert_eq!(
            r.log(),
            [
                "FWCCB: slot 0 read empty; type 0x2abc006b after 60 us (read offset 0, write offset 1)",
                "FWCCB: UPDATE_STATS element=2 (out of memory) adjust=1 (seen 1)",
            ]
        );
        assert_eq!((r.k.fw_stats[2], r.k.fwccb_empty, r.k.fwccb_filled), (1, 1, 1));
        assert_eq!(r.read_offset(), 1);
    }

    /// The known negative: without the check the slot is skipped as
    /// "unhandled" and its command is lost, which is what the AM62P did at
    /// a71c9b8b's and kt-3's refused grows.
    #[test]
    fn without_the_check_the_empty_slot_is_skipped_and_its_command_lost() {
        let mut r = rig(false, Some(3));
        assert_eq!(fwccb_service(&mut r.k), 1);
        assert_eq!(r.log(), ["FWCCB: cmd type=0x00000000 (unhandled, skipping)"]);
        assert_eq!((r.k.fw_stats[2], r.fw.waits.get()), (0, 0));
        assert_eq!(r.read_offset(), 1, "consumed");
    }

    /// A slot that never fills is skipped after the bound, said as such.
    #[test]
    fn a_slot_that_stays_empty_is_said_after_the_bound() {
        let mut r = rig(true, None);
        assert_eq!(fwccb_service(&mut r.k), 1);
        let l = r.log();
        assert_eq!(
            l[0],
            "FWCCB: slot 0 read empty; still empty after 2000 us, type 0x00000000 (read offset 0, write offset 1)"
        );
        assert_eq!(l[1], "FWCCB: cmd type=0x00000000 (unhandled, skipping)");
        assert_eq!((r.fw.waits.get(), r.k.fwccb_empty, r.k.fwccb_filled), (100, 1, 0));
    }

    /// The write offset only moves forward; found behind the last one read
    /// it is said (only the host's writeback can do that). Forward across the
    /// ring's end is not "back".
    #[test]
    fn a_write_offset_that_went_back_is_said() {
        let mut r = rig(true, Some(1));
        fwccb_service(&mut r.k); // reads write offset 1, consumes slot 0
        let w = |r: &mut Rig, at: usize, v: u32| r.mem[(CTL as usize + at) / 4] = v;
        w(&mut r, CCB_CTL_READ_OFFSET, 0);
        w(&mut r, CCB_CTL_WRITE_OFFSET, 0);
        r.fw.lines.borrow_mut().clear();
        fwccb_service(&mut r.k);
        assert_eq!(
            r.log(),
            ["FWCCB: write offset went back from 1 to 0 (the host's writeback of the shared control line)"]
        );
        // forward over the end of the ring: 31 -> 0 is not back
        (r.k.fwccb_last_wo, r.k.fwccb_wo_back) = (31, 0);
        r.fw.lines.borrow_mut().clear();
        fwccb_service(&mut r.k);
        assert!(r.log().is_empty(), "{:?}", r.log());
        assert_eq!(r.k.fwccb_wo_back, 0);
    }
}
