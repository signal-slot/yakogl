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

/// GO, counting a ride-along.
fn commit(m: &impl Mmio, d: &DssData) {
    if dss::go(m, d.vp) == Go::RodeAlong {
        d.stats.go_waits.fetch_add(1, SeqCst);
    }
}

/// Write `f` and GO; if a vsync went by meanwhile, once more.
fn write_and_go(m: &impl Mmio, d: &DssData, mut f: impl FnMut()) {
    for _ in 0..2 {
        let v0 = d.stats.vsyncs.load(SeqCst);
        f();
        commit(m, d);
        if d.stats.vsyncs.load(SeqCst) == v0 {
            break;
        }
    }
}

/// Point the application plane at a new buffer; it takes effect at the next
/// vsync (or at once while the port is off).
pub fn set_app_buffer(m: &impl Mmio, d: &DssData, app: &AppPlane) -> Result<(), i32> {
    // Checked before anything is marked pending or written.
    dss::check_plane(&app.plane)?;
    write_and_go(m, d, || {
        d.app_seq_pending.store(app.seq, SeqCst);
        d.app_commit_pending.store(1, SeqCst);
        // Cannot fail: the plane was checked above.
        let _ = dss::plane_configure(m, d.vid, &app.plane, true);
    });
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
