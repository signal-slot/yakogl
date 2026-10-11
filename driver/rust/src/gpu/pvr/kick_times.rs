// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Per-kick timestamps of the render service (feature `pvr-kick-times`,
//! Kconfig `CONFIG_SAFEUI_GPU_PVR_KICK_TIMES`;
//! tools/cts/log/investigations/pvr-fence-wait.md, "The kick path").
//!
//! For each kick: when `submit` was entered, how long the pre-kick MMU cache
//! invalidate took, when the kick was posted, when the host first saw its
//! geometry fence and its fragment fence, and when a wait first asked for its
//! fragment fence. The host sees a fence only when it reads it, so each
//! "seen" time comes with the time of the last read that did NOT see it: the
//! event happened between the two, and the gap is the error bar
//! ([`YdPvrKickUs::geom_err_us`], `frag_err_us`). A fence nobody read while
//! it moved (the backend found the kick complete without a wait) has the
//! whole time since the post as its error bar: such a value is an upper
//! bound, not a measurement, and a reader keeps it apart.
//!
//! Reads happen after the kick is posted and at every UFO read of `wait_for`
//! (every [`super::render::WAIT_POLL_US`]), for every kick in flight, so a
//! wait for kick N-1 also times kick N's geometry. The clock is the ARM
//! generic timer (`CNTVCT_EL0` at `CNTFRQ_EL0`), the one Zephyr's arm64 timer
//! uses. Without the feature nothing is stamped and the reader answers
//! -ENOTSUP.

#[cfg(feature = "pvr-kick-times")]
use super::km::YdPvrKm;
use super::render::YdPvrRender;
use crate::errno;

/// Kicks tracked at once (indexed by sequence modulo this).
pub const KT_RING: usize = 8;

/// One kick's stamps, in counter ticks (0: not yet).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct YdPvrKickStamp {
    pub t_submit: u64,
    pub t_posted: u64,
    /// The last read that did not see the geometry fence, and the first that
    /// did.
    pub t_geom_miss: u64,
    pub t_geom: u64,
    pub t_frag_miss: u64,
    pub t_frag: u64,
    /// Entry of the first `wait_for` that named this kick's fence.
    pub t_wait: u64,
    /// Ticks spent in the pre-kick MMU cache invalidate (its post and, under
    /// `mmu_ack`, the wait for its acknowledgement).
    pub mmu_ticks: u64,
    /// The kick's fence (0: the slot is free).
    pub seq: u32,
    /// 1 when an MMU cache invalidate was posted before this kick.
    pub mmu_synced: u32,
}

/// `struct yd_pvr_kick_times`: the ring and the last kick seen complete.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct YdPvrKickTimes {
    /// The counter's frequency (0: not started).
    pub hz: u32,
    /// Kicks seen complete.
    pub done: u32,
    /// Kicks whose ring slot was taken by a later kick before their fragment
    /// fence was seen (their times are lost).
    pub lost: u32,
    pub pad: u32,
    /// The fragment-fence time of the kick completed before `last` (0: none).
    pub prev_frag: u64,
    pub last: YdPvrKickStamp,
    pub ring: [YdPvrKickStamp; KT_RING],
}

/// `struct yd_pvr_kick_us`: the last completed kick, in microseconds.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct YdPvrKickUs {
    pub seq: u32,
    /// `submit` entry to the kick posted (the HWRT pick, the client CCB
    /// writes, the MMU cache invalidate, the KCCB post and doorbell).
    pub submit_us: u32,
    pub mmu_us: u32,
    pub mmu_synced: u32,
    /// Post to the geometry fence seen, and its error bar (the event lies in
    /// [kick_geom_us - geom_err_us, kick_geom_us]). Valid with
    /// `KICK_US_GEOM_SEEN`; without it the fence was never seen (its UFO slot
    /// was reused before a read).
    pub kick_geom_us: u32,
    pub geom_err_us: u32,
    /// Post to the fragment fence seen, and its error bar.
    pub kick_frag_us: u32,
    pub frag_err_us: u32,
    /// The previous completed kick's fragment fence to this one's (both as
    /// seen); valid with `KICK_US_PREV`.
    pub frag_frag_us: u32,
    /// The first wait that named this fence, to the fence seen; valid with
    /// `KICK_US_WAITED`.
    pub wait_us: u32,
    pub flags: u32,
    /// [`YdPvrKickTimes::done`] and `lost`.
    pub done: u32,
    pub lost: u32,
}

pub const KICK_US_GEOM_SEEN: u32 = 1 << 0;
pub const KICK_US_PREV: u32 = 1 << 1;
pub const KICK_US_WAITED: u32 = 1 << 2;

const _: () = {
    assert!(core::mem::size_of::<YdPvrKickStamp>() == 72);
    assert!(core::mem::size_of::<YdPvrKickTimes>() == 24 + 72 * (1 + KT_RING));
    assert!(core::mem::size_of::<YdPvrKickUs>() == 52);
    assert!(core::mem::offset_of!(YdPvrKickStamp, t_wait) == 48);
    assert!(core::mem::offset_of!(YdPvrKickStamp, seq) == 64);
    assert!(core::mem::offset_of!(YdPvrKickTimes, prev_frag) == 16);
    assert!(core::mem::offset_of!(YdPvrKickTimes, ring) == 24 + 72);
    assert!(core::mem::offset_of!(YdPvrKickUs, kick_geom_us) == 16);
    assert!(core::mem::offset_of!(YdPvrKickUs, frag_frag_us) == 32);
    assert!(core::mem::offset_of!(YdPvrKickUs, flags) == 40);
};

// ---- the clock -------------------------------------------------------------

#[cfg(all(any(feature = "pvr-kick-times", feature = "pvr-grow-scene"), target_arch = "aarch64", not(test)))]
fn now() -> u64 {
    let v: u64;
    // SAFETY: reads the virtual counter, readable at EL1 (Zephyr, the Linux
    // module); no memory is touched.
    unsafe { core::arch::asm!("isb", "mrs {}, cntvct_el0", out(reg) v, options(nomem, nostack)) };
    v
}
#[cfg(all(any(feature = "pvr-kick-times", feature = "pvr-grow-scene"), target_arch = "aarch64", not(test)))]
fn hz() -> u32 {
    let v: u64;
    // SAFETY: as above.
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) v, options(nomem, nostack)) };
    v as u32
}
/// A host build that is not a test has no counter: nothing is stamped.
#[cfg(all(any(feature = "pvr-kick-times", feature = "pvr-grow-scene"), not(target_arch = "aarch64"), not(test)))]
fn now() -> u64 {
    0
}
#[cfg(all(any(feature = "pvr-kick-times", feature = "pvr-grow-scene"), not(target_arch = "aarch64"), not(test)))]
fn hz() -> u32 {
    0
}
#[cfg(all(any(feature = "pvr-kick-times", feature = "pvr-grow-scene"), test))]
fn now() -> u64 {
    test_clock::get()
}
#[cfg(all(any(feature = "pvr-kick-times", feature = "pvr-grow-scene"), test))]
fn hz() -> u32 {
    1_000_000
}

/// The clock for the grow line's scene context ([`super::render`], feature
/// `pvr-grow-scene`): the counter now and its frequency, or `None` without
/// the feature (or on a host build that is not a test, which has no
/// counter).
pub fn clock() -> Option<(u64, u32)> {
    #[cfg(feature = "pvr-grow-scene")]
    {
        Some((now(), hz())).filter(|&(_, f)| f != 0)
    }
    #[cfg(not(feature = "pvr-grow-scene"))]
    {
        None
    }
}

// ---- stamping (no-ops without the feature) ----------------------------------

/// The time now, for [`posted`]'s `t_submit` and the MMU invalidate's span
/// (0 without the feature).
pub fn mark() -> u64 {
    #[cfg(feature = "pvr-kick-times")]
    {
        now()
    }
    #[cfg(not(feature = "pvr-kick-times"))]
    {
        0
    }
}

/// Kick `seq` was posted; `t_submit` from [`mark`] at `submit`'s entry, the
/// MMU cache invalidate's ticks and whether one was posted.
#[cfg(feature = "pvr-kick-times")]
pub fn posted(k: &YdPvrKm, r: &mut YdPvrRender, seq: u32, t_submit: u64, mmu_ticks: u64, mmu_synced: bool) {
    let kt = &mut r.kick_times;
    if kt.hz == 0 {
        kt.hz = hz();
    }
    if kt.hz == 0 || seq == 0 {
        return;
    }
    let e = &mut kt.ring[seq as usize % KT_RING];
    if e.seq != 0 {
        kt.lost = kt.lost.wrapping_add(1);
    }
    *e = YdPvrKickStamp {
        t_submit,
        t_posted: now(),
        mmu_ticks,
        seq,
        mmu_synced: mmu_synced as u32,
        ..Default::default()
    };
    poll(k, r);
}

/// A wait for `fence` begins.
#[cfg(feature = "pvr-kick-times")]
pub fn wait_begins(r: &mut YdPvrRender, fence: u32) {
    let t = now();
    let e = &mut r.kick_times.ring[fence as usize % KT_RING];
    if e.seq == fence && e.t_wait == 0 {
        e.t_wait = t;
    }
}

/// Read the geometry and fragment fences of every kick in flight and stamp
/// what moved; complete, in fence order, the kicks whose fragment fence is
/// seen.
#[cfg(feature = "pvr-kick-times")]
pub fn poll(k: &YdPvrKm, r: &mut YdPvrRender) {
    use super::render::{geom_ufo_slot, read_heap32};
    if r.kick_times.hz == 0 {
        return;
    }
    let t = now();
    k.invd(k.heap_cpu(r.fufo_off), 4);
    let frag = read_heap32(k, r.fufo_off, 0);
    for i in 0..KT_RING {
        let e = r.kick_times.ring[i];
        if e.seq == 0 || e.t_frag != 0 {
            continue;
        }
        if e.t_geom == 0 {
            let at = (geom_ufo_slot(e.seq) * 4) as usize;
            k.invd(k.heap_cpu(r.gufo_off).wrapping_add(at), 4);
            if read_heap32(k, r.gufo_off, at) == e.seq {
                r.kick_times.ring[i].t_geom = t;
            } else {
                r.kick_times.ring[i].t_geom_miss = t;
            }
        }
        if (frag.wrapping_sub(e.seq) as i32) >= 0 {
            r.kick_times.ring[i].t_frag = t;
        } else {
            r.kick_times.ring[i].t_frag_miss = t;
        }
    }
    // Complete in fence order: the oldest seen first.
    loop {
        let ring = &r.kick_times.ring;
        let mut pick: Option<usize> = None;
        for (i, e) in ring.iter().enumerate() {
            if e.seq != 0 && e.t_frag != 0 && pick.is_none_or(|p| (e.seq.wrapping_sub(ring[p].seq) as i32) < 0) {
                pick = Some(i);
            }
        }
        let Some(p) = pick else { break };
        let kt = &mut r.kick_times;
        kt.prev_frag = if kt.done != 0 { kt.last.t_frag } else { 0 };
        kt.last = kt.ring[p];
        kt.done = kt.done.wrapping_add(1);
        kt.ring[p] = YdPvrKickStamp::default();
    }
}

// ---- the reader ------------------------------------------------------------

fn us(ticks: u64, hz: u32) -> u32 {
    (u128::from(ticks) * 1_000_000 / u128::from(hz.max(1))).min(u128::from(u32::MAX)) as u32
}

/// The last completed kick in microseconds. 0; -ENOTSUP when built without
/// the feature or without a counter; -ENODATA before the first kick
/// completed.
pub fn last_kick_us(r: &YdPvrRender, out: &mut YdPvrKickUs) -> i32 {
    let kt = &r.kick_times;
    if !cfg!(feature = "pvr-kick-times") || kt.hz == 0 {
        return -errno::ENOTSUP;
    }
    if kt.done == 0 {
        return -errno::ENODATA;
    }
    let e = &kt.last;
    let hz = kt.hz;
    let mut o = YdPvrKickUs {
        seq: e.seq,
        submit_us: us(e.t_posted.saturating_sub(e.t_submit), hz),
        mmu_us: us(e.mmu_ticks, hz),
        mmu_synced: e.mmu_synced,
        kick_frag_us: us(e.t_frag.saturating_sub(e.t_posted), hz),
        frag_err_us: us(e.t_frag.saturating_sub(e.t_frag_miss.max(e.t_posted)), hz),
        done: kt.done,
        lost: kt.lost,
        ..Default::default()
    };
    if e.t_geom != 0 {
        o.flags |= KICK_US_GEOM_SEEN;
        o.kick_geom_us = us(e.t_geom.saturating_sub(e.t_posted), hz);
        o.geom_err_us = us(e.t_geom.saturating_sub(e.t_geom_miss.max(e.t_posted)), hz);
    }
    if kt.prev_frag != 0 {
        o.flags |= KICK_US_PREV;
        o.frag_frag_us = us(e.t_frag.saturating_sub(kt.prev_frag), hz);
    }
    if e.t_wait != 0 {
        o.flags |= KICK_US_WAITED;
        o.wait_us = us(e.t_frag.saturating_sub(e.t_wait), hz);
    }
    *out = o;
    0
}

/// # Safety
/// `r` and `out` valid (the render service's state, owned by
/// driver/src/pvr_render.c, not in use by a submit or wait for the call).
#[no_mangle]
pub unsafe extern "C" fn yd_pvr_render_last_kick_us(r: *const YdPvrRender, out: *mut YdPvrKickUs) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { last_kick_us(&*r, &mut *out) }
}

// Without the feature the hooks compile to nothing.
#[cfg(not(feature = "pvr-kick-times"))]
pub fn posted(_: &super::km::YdPvrKm, _: &mut YdPvrRender, _: u32, _: u64, _: u64, _: bool) {}
#[cfg(not(feature = "pvr-kick-times"))]
pub fn wait_begins(_: &mut YdPvrRender, _: u32) {}
#[cfg(not(feature = "pvr-kick-times"))]
pub fn poll(_: &super::km::YdPvrKm, _: &mut YdPvrRender) {}

#[cfg(test)]
pub mod test_clock {
    //! The tests' clock: microseconds a test sets (read at 1 MHz).
    use std::cell::Cell;
    thread_local!(static NOW: Cell<u64> = const { Cell::new(0) });
    pub fn set(us: u64) {
        NOW.with(|c| c.set(us));
    }
    pub fn get() -> u64 {
        NOW.with(|c| c.get())
    }
}
