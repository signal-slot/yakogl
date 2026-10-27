// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! DSS1 as the Verdin AM62P uses it: one video port (VP2 -> DSI on this
//! board, the devicetree's `video-port`) whose overlay has two layers:
//!
//! - layer 0, the VID pipeline: the application frame handed over by the
//!   A53's renderer (set_app_buffer / enable_app_plane);
//! - layer 1, the VIDL1 pipeline: the safety layer, ARGB8888, owned by this
//!   driver's caller, double-buffered (safety_flip).
//!
//! The hardware blends them, so the application can stop or hand over
//! garbage without touching the safety layer. This protocol is the replaced
//! C driver's (display_am62p_dss.c), an original part of this project kept
//! as it was (docs/source-form.md, group E: "the two-plane safety layer,
//! pending GO, sequence numbers, vsync callback, statistics"):
//!
//! - Both writers share one GO bit and never wait for it: if GO is still
//!   set, the new values ride along with the pending latch (`go_waits`).
//!   Waiting a frame per commit is what made both planes stutter.
//! - A commit marks itself pending *before* setting GO, so the vsync
//!   interrupt never sees a consumed GO without knowing what it latched.
//! - At a vsync with GO clear the pending application frame became the shown
//!   one (`app_seq_shown`, what the A53 uses to know which buffer it may
//!   draw into again) and the safety back buffer is off screen.
//! - If a vsync happened while a commit was being written, the latch may
//!   have taken a mix of old and new values, so the commit is written once
//!   more (idempotent).
//! - Which application buffers the display may still scan ([`app_busy`],
//!   for a renderer that flips faster than the panel latches): the thread
//!   that sets the application buffer works it out itself from the GO bit
//!   it reads and from counters around every GO write, not from the
//!   interrupt, so a late interrupt cannot make the answer too small
//!   (docs/notes/am62p-flip-latch.md).
//!
//! All mutable state is in [`DssData`] (the device's `data`), as atomics:
//! the interrupt and the thread both touch it, and this code holds no lock
//! (as the C did). The vsync semaphore and the callback stay in the C shim.

use crate::env::Env;
use crate::errno::{EINVAL, ENODEV};
use crate::ip::ti_dss7::{self as dss, Go, Output, Plane};
use crate::mmio::Mmio;
use crate::soc::am62p::regs::{ovr, vid};
use crate::video::VideoMode;
use crate::ylog;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};

/// `YD_AM62P_DSS_*` flags of [`DssConfig`].
pub mod flags {
    pub const HSYNC_LOW: u32 = 1 << 0;
    pub const VSYNC_LOW: u32 = 1 << 1;
    pub const DE_LOW: u32 = 1 << 2;
    pub const PIXCLK_NEGEDGE: u32 = 1 << 3;
    /// The device has an interrupt line (else wait_vsync polls).
    pub const HAS_IRQ: u32 = 1 << 4;
}

/// [`dss_isr`] result bits.
pub mod event {
    pub const VSYNC: u32 = 1 << 0;
    pub const SYNC_LOST: u32 = 1 << 1;
    pub const UNDERFLOW: u32 = 1 << 2;
}

/// `struct yd_am62p_dss_config`: from the devicetree.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DssConfig {
    pub pixel_clock_hz: u32,
    pub hactive: u16,
    pub hfp: u16,
    pub hsync: u16,
    pub hbp: u16,
    pub vactive: u16,
    pub vfp: u16,
    pub vsync: u16,
    pub vbp: u16,
    /// `flags::*`.
    pub flags: u32,
    /// Video port index, 0 = VP1.
    pub vp_index: u32,
    /// The safety layer's size (the devicetree's width/height).
    pub safety_width: u32,
    pub safety_height: u32,
}

/// `struct yd_am62p_dss_stats`: the counters of `struct am62p_dss_stats`.
#[repr(C)]
#[derive(Debug, Default)]
pub struct DssStats {
    pub vsyncs: AtomicU32,
    pub app_frames: AtomicU32,
    pub underflows: AtomicU32,
    pub sync_lost: AtomicU32,
    pub go_timeouts: AtomicU32,
    pub go_waits: AtomicU32,
    pub safety_flips: AtomicU32,
}

/// `struct yd_am62p_dss_data`: per instance. The shim fills the five bases
/// and the two safety buffer addresses before [`dss_init`]; everything else
/// is this module's.
#[repr(C)]
#[derive(Debug, Default)]
pub struct DssData {
    pub common: usize,
    pub vid: usize,
    pub vidl1: usize,
    /// The overlay and port of `vp_index`.
    pub ovr: usize,
    pub vp: usize,
    /// The safety layer's two buffers (bus addresses; the A53 maps DDR 1:1).
    pub safety_buf: [u64; 2],
    /// The application plane as last committed (0 = none yet).
    pub app_addr: AtomicU64,
    pub app_width: AtomicU32,
    pub app_height: AtomicU32,
    pub app_stride: AtomicU32,
    pub app_format: AtomicU32,
    /// seq committed to the shadow registers / seq the plane is scanning.
    pub app_seq_pending: AtomicU32,
    pub app_seq_shown: AtomicU32,
    /// 1 while a committed application frame waits for its latch.
    pub app_commit_pending: AtomicU32,
    /// 1 while a flipped safety address waits for its latch.
    pub safety_flip_pending: AtomicU32,
    /// Index into `safety_buf` of the buffer scanned out (the other is the
    /// back buffer the renderer draws into).
    pub safety_front: AtomicU32,
    pub app_enabled: AtomicU32,
    pub output_enabled: AtomicU32,
    pub stats: DssStats,
    /// What the display may scan of the application plane ([`app_busy`]).
    /// Written by the thread that sets the application buffer only:
    /// the address last written to the shadow registers,
    pub app_addr_pending: AtomicU64,
    /// the one the plane scanned when GO was last seen clear (0: none),
    pub app_addr_shown: AtomicU64,
    /// and one more that may be scanned: a commit replaced it while it may
    /// have been latched (0: none).
    pub app_addr_unsure: AtomicU64,
    /// GO writes of every writer begun and ended ([`commit`]): a GO write by
    /// another writer between two reads of the GO bit shows in them.
    pub go_enter: AtomicU32,
    pub go_exit: AtomicU32,
    /// `go_enter` after the last application commit, and 1 when that
    /// commit set or found GO with no other GO write and no latch while it
    /// was written (its address cannot have been latched yet).
    pub app_go_mark: AtomicU32,
    pub app_go_clean: AtomicU32,
    /// 1 when [`app_busy`] cannot name the buffers (more than three may be
    /// scanned): every buffer is busy until GO is seen clear.
    pub app_busy_all: AtomicU32,
}

/// The application plane request (`struct am62p_dss_plane_cfg`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppPlane {
    pub seq: u32,
    pub plane: Plane,
}

impl DssConfig {
    pub fn mode(&self) -> VideoMode {
        VideoMode {
            pixel_clock_hz: self.pixel_clock_hz,
            hactive: self.hactive.into(),
            hfront: self.hfp.into(),
            hsync: self.hsync.into(),
            hback: self.hbp.into(),
            vactive: self.vactive.into(),
            vfront: self.vfp.into(),
            vsync: self.vsync.into(),
            vback: self.vbp.into(),
            hsync_high: self.flags & flags::HSYNC_LOW == 0,
            vsync_high: self.flags & flags::VSYNC_LOW == 0,
            de_high: self.flags & flags::DE_LOW == 0,
        }
    }

    fn output(&self) -> Output {
        Output { data_on_falling_edge: self.flags & flags::PIXCLK_NEGEDGE != 0 }
    }
}

impl DssData {
    /// The buffer the renderer may draw into.
    pub fn safety_back(&self) -> u64 {
        self.safety_buf[(self.safety_front.load(SeqCst) ^ 1) as usize & 1]
    }

    /// The buffer scanned out.
    pub fn safety_front_addr(&self) -> u64 {
        self.safety_buf[self.safety_front.load(SeqCst) as usize & 1]
    }

    fn safety_plane(&self, cfg: &DssConfig, addr: u64) -> Plane {
        Plane {
            addr,
            width: cfg.safety_width,
            height: cfg.safety_height,
            stride_bytes: cfg.safety_width.saturating_mul(4),
            format: vid::FMT_ARGB8888,
        }
    }
}

/// Bring DSS1 up to the point where only the port enable is missing:
/// resets, FIFOs, the port's timings, the safety layer on layer 1 (enabled,
/// scanning `safety_buf[0]`), the application layer 0 disabled, interrupts
/// (with `HAS_IRQ`). The port stays off (`output_enable`).
pub fn dss_init(m: &impl Mmio, env: &impl Env, cfg: &DssConfig, d: &DssData) -> Result<(), i32> {
    if cfg.vp_index > 1 || d.common == 0 || d.vid == 0 || d.vidl1 == 0 || d.ovr == 0 || d.vp == 0 {
        return Err(-ENODEV);
    }
    let timing = dss::vp_timing(&cfg.mode(), &cfg.output())?;
    d.safety_front.store(0, SeqCst);
    let safety = d.safety_plane(cfg, d.safety_buf[0]);
    if d.safety_buf[1] == 0 {
        return Err(-EINVAL);
    }
    dss::soft_reset(m, env, d.common)?;
    ylog!(env, Inf, "DSS rev {:#010x}", dss::revision(m, d.common));
    dss::mflag_init(m, d.common);
    for pipe in [d.vid, d.vidl1] {
        let f = dss::pipe_fifo_init(m, pipe)?;
        ylog!(env, Dbg, "pipe fifo high {} low {} preload {}", f.high, f.low, f.preload);
    }
    dss::vp_configure(m, d.vp, d.ovr, &timing);
    dss::plane_configure(m, d.vidl1, &safety, false)?;
    dss::plane_enable(m, d.vidl1, true);
    dss::ovr_layer(m, d.ovr, 1, ovr::CHANNEL_VIDL1, true);
    dss::ovr_layer(m, d.ovr, 0, ovr::CHANNEL_VID, false);
    dss::plane_enable(m, d.vid, false);
    if cfg.flags & flags::HAS_IRQ != 0 {
        dss::irq_enable(m, d.common, cfg.vp_index);
    }
    ylog!(
        env,
        Inf,
        "DSS ready: VP{} {}x{} @ {} Hz, safety layer ARGB8888 at {:#x}",
        cfg.vp_index + 1,
        cfg.hactive,
        cfg.vactive,
        cfg.pixel_clock_hz,
        d.safety_buf[0]
    );
    Ok(())
}

/// GO, counting a ride-along. Every GO write of every writer is between
/// `go_enter` and `go_exit` ([`app_busy`] reads them).
fn commit(m: &impl Mmio, d: &DssData) -> Go {
    d.go_enter.fetch_add(1, SeqCst);
    let go = dss::go(m, d.vp);
    d.go_exit.fetch_add(1, SeqCst);
    if go == Go::RodeAlong {
        d.stats.go_waits.fetch_add(1, SeqCst);
    }
    go
}

/// What the GO writes of one [`write_and_go`] found.
struct Written {
    /// The first pass's and the last pass's.
    first: Go,
    last: Go,
    /// A vsync went by while the first pass was written: it was written again.
    twice: bool,
}

/// Write `f` and GO; if a vsync went by meanwhile, once more.
fn write_and_go(m: &impl Mmio, d: &DssData, mut f: impl FnMut()) -> Written {
    let v0 = d.stats.vsyncs.load(SeqCst);
    f();
    let first = commit(m, d);
    if d.stats.vsyncs.load(SeqCst) == v0 {
        return Written { first, last: first, twice: false };
    }
    f();
    Written { first, last: commit(m, d), twice: true }
}

/// Point the application plane at a new buffer; it takes effect at the next
/// vsync (or at once while the port is off).
///
/// It also keeps what [`app_busy`] reports, from what this thread sees of
/// the GO bit: a frame end latches the shadow registers only when it finds
/// GO set, and clears it; only a GO write sets it again, and every GO write
/// is counted in `go_enter` / `go_exit`. So with no GO write by anyone else:
///
/// - GO clear before this commit: the last commit was latched, the plane
///   scans exactly its address ([`app_settled`]);
/// - GO set before this commit and still set at its GO read, and set ever
///   since the last commit's GO (its mark): nothing was latched since that
///   commit wrote its address, so that buffer (`old`) was never scanned and
///   this commit drops it from the mailbox;
/// - GO set before and clear at the GO read: a frame end latched `old` or
///   this commit's own address while it was being written; the plane scans
///   one of the two.
///
/// Anything else (another writer's GO in between, a commit written twice,
/// the port off) keeps `old` busy until GO is seen clear.
pub fn set_app_buffer(m: &impl Mmio, d: &DssData, app: &AppPlane) -> Result<(), i32> {
    // Checked before anything is marked pending or written.
    dss::check_plane(&app.plane)?;
    let old = d.app_addr_pending.load(SeqCst);
    let mark = d.app_go_mark.load(SeqCst);
    let old_clean = d.app_go_clean.load(SeqCst) != 0;
    // GO writes so far, read before the GO bit: exit first, so a write in
    // progress shows as enter != exit.
    let exit0 = d.go_exit.load(SeqCst);
    let enter0 = d.go_enter.load(SeqCst);
    let set0 = dss::go_pending(m, d.vp);
    if !set0 {
        app_settled(d);
    }
    let w = write_and_go(m, d, || {
        d.app_seq_pending.store(app.seq, SeqCst);
        d.app_commit_pending.store(1, SeqCst);
        d.app_addr_pending.store(app.plane.addr, SeqCst);
        // Cannot fail: the plane was checked above.
        let _ = dss::plane_configure(m, d.vid, &app.plane, true);
    });
    let enter1 = d.go_enter.load(SeqCst);
    // No GO write since the reads above but this commit's own (one a pass).
    let own = if w.twice { 2 } else { 1 };
    let alone = enter0 == exit0 && enter1 == enter0.wrapping_add(own);
    // A frame end latched while a pass was written: GO was set before the
    // pass and its GO read found it clear (the second pass starts with the
    // first one's GO set).
    let latched = (set0 && w.first == Go::Armed) || (w.twice && w.last == Go::Armed);
    // `old`'s GO was still pending when this commit began: GO set ever
    // since `old`'s commit, which itself latched nothing.
    let old_pending = alone && set0 && old_clean && mark == enter0;
    if old != 0 && old != app.plane.addr {
        if old_pending && latched {
            // the plane scans `old` or this commit's address
            d.app_addr_shown.store(old, SeqCst);
            d.app_addr_unsure.store(0, SeqCst);
            d.app_busy_all.store(0, SeqCst);
        } else if !old_pending {
            app_keep_busy(d, old);
        }
        // else dropped: never scanned, and gone from the shadow registers
    }
    d.app_go_mark.store(enter1, SeqCst);
    d.app_go_clean.store(u32::from(alone && w.first != Go::NotRunning && !latched), SeqCst);
    d.app_addr.store(app.plane.addr, SeqCst);
    d.app_width.store(app.plane.width, SeqCst);
    d.app_height.store(app.plane.height, SeqCst);
    d.app_stride.store(app.plane.stride_bytes, SeqCst);
    d.app_format.store(app.plane.format, SeqCst);
    d.stats.app_frames.fetch_add(1, SeqCst);
    Ok(())
}

/// Show or hide the application plane. -ENODATA before a buffer was set.
pub fn enable_app_plane(m: &impl Mmio, d: &DssData, on: bool) -> Result<(), i32> {
    if on && d.app_addr.load(SeqCst) == 0 {
        return Err(-crate::errno::ENODATA);
    }
    dss::plane_enable(m, d.vid, on);
    dss::ovr_layer(m, d.ovr, 0, ovr::CHANNEL_VID, on);
    d.app_enabled.store(on.into(), SeqCst);
    commit(m, d);
    Ok(())
}

/// GO clear, seen by the thread that sets the application buffer after its
/// last commit: that commit's GO was consumed by a frame end after its
/// address was written, and nothing changed the address since, so the plane
/// scans exactly `app_addr_pending`.
fn app_settled(d: &DssData) {
    d.app_addr_shown.store(d.app_addr_pending.load(SeqCst), SeqCst);
    d.app_addr_unsure.store(0, SeqCst);
    d.app_busy_all.store(0, SeqCst);
}

/// `addr` may be scanned: keep it busy until GO is seen clear.
fn app_keep_busy(d: &DssData, addr: u64) {
    let shown = d.app_addr_shown.load(SeqCst);
    let unsure = d.app_addr_unsure.load(SeqCst);
    if addr == shown || addr == unsure {
        return;
    }
    if shown == 0 {
        d.app_addr_shown.store(addr, SeqCst);
    } else if unsure == 0 {
        d.app_addr_unsure.store(addr, SeqCst);
    } else {
        d.app_busy_all.store(1, SeqCst);
    }
}

/// Which of `addrs` the application plane may still scan: bit k for
/// `addrs[k]` (at most 32). Exact or too large, never too small: a buffer
/// whose bit is clear is not read by the display until it is set as the
/// application buffer again. Never waits. Call it from the thread that sets
/// the application buffer.
pub fn app_busy(m: &impl Mmio, d: &DssData, addrs: &[u64]) -> u32 {
    if !dss::go_pending(m, d.vp) {
        app_settled(d);
    }
    let all = d.app_busy_all.load(SeqCst) != 0;
    let held = [d.app_addr_pending.load(SeqCst), d.app_addr_shown.load(SeqCst), d.app_addr_unsure.load(SeqCst)];
    addrs
        .iter()
        .take(32)
        .enumerate()
        .filter(|(_, a)| all || (**a != 0 && held.contains(a)))
        .fold(0, |mask, (k, _)| mask | 1 << k)
}

/// The safety layer's pixels are premultiplied (Slint's software renderer
/// writes them so); default straight alpha.
pub fn safety_premultiplied(m: &impl Mmio, d: &DssData, on: bool) {
    dss::plane_premultiplied(m, d.vidl1, on);
}

/// Present the back buffer at the next vsync; the old front buffer becomes
/// the back buffer, free once [`safety_back_free`] says so.
pub fn safety_flip(m: &impl Mmio, d: &DssData) {
    let next = d.safety_back();
    write_and_go(m, d, || {
        d.safety_flip_pending.store(1, SeqCst);
        dss::plane_set_address(m, d.vidl1, next);
    });
    d.safety_front.fetch_xor(1, SeqCst);
    d.stats.safety_flips.fetch_add(1, SeqCst);
}

pub fn safety_back_free(d: &DssData) -> bool {
    d.safety_flip_pending.load(SeqCst) == 0
}

pub fn output_enable(m: &impl Mmio, d: &DssData, on: bool) {
    dss::vp_enable(m, d.vp, on);
    d.output_enabled.store(on.into(), SeqCst);
}

/// The interrupt: count, settle the pending commits when the latch
/// happened, return `event::*` bits (the shim gives the semaphore and calls
/// the callback on VSYNC).
pub fn dss_isr(m: &impl Mmio, cfg: &DssConfig, d: &DssData) -> u32 {
    let ev = dss::irq_service(m, d.common, cfg.vp_index);
    let mut out = 0;
    if ev.vsync {
        d.stats.vsyncs.fetch_add(1, SeqCst);
        latch_settled(m, d);
        out |= event::VSYNC;
    }
    if ev.sync_lost {
        d.stats.sync_lost.fetch_add(1, SeqCst);
        out |= event::SYNC_LOST;
    }
    for u in ev.underflow {
        if u {
            d.stats.underflows.fetch_add(1, SeqCst);
            out |= event::UNDERFLOW;
        }
    }
    out
}

/// At a vsync: GO clear means the shadow registers were latched.
fn latch_settled(m: &impl Mmio, d: &DssData) {
    if dss::go_pending(m, d.vp) {
        return;
    }
    if d.app_commit_pending.swap(0, SeqCst) != 0 {
        d.app_seq_shown.store(d.app_seq_pending.load(SeqCst), SeqCst);
    }
    d.safety_flip_pending.store(0, SeqCst);
}

/// Without an interrupt line: one poll of the port's VSYNC status.
pub fn poll_vsync(m: &impl Mmio, cfg: &DssConfig, d: &DssData) -> bool {
    if !dss::poll_vsync(m, d.common, cfg.vp_index) {
        return false;
    }
    d.stats.vsyncs.fetch_add(1, SeqCst);
    true
}

#[cfg(test)]
mod tests;
