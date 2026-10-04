// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! C entry points of the AM62P display path: DSS1 (`safeui,am62p-dss`, the
//! shim is driver/drivers/display/display_am62p_dss_rust.c; logic in
//! soc::am62p::dss over ip::ti_dss7) and the Cadence DSI host
//! (`safeui,cdns-dsi-j721e`, driver/drivers/mipi_dsi/dsi_cdns_rust.c;
//! ip::cdns_dsi + ip::cdns_dphy). The structs are pinned against
//! include/yakogl_drivers.h.
//!
//! The DSS entry points take the device's data as a shared pointer: the
//! interrupt and the thread both use it at once, which the atomics of
//! [`DssData`] are for (the C shim takes no lock, as the C driver did not).

use crate::env::YdEnv;
use crate::errno::EINVAL;
use crate::ip::cdns_dsi::{self, Bases, Request};
use crate::ip::ti_dss7::Plane;
use crate::mmio::Hw;
use crate::soc::am62p::dss::{self, AppPlane, DssConfig, DssData, DssStats};
use crate::video::VideoMode;
use core::mem::{offset_of, size_of};

// ---- DSS ---------------------------------------------------------------

/// `struct yd_am62p_dss_plane`: `struct am62p_dss_plane_cfg` with the
/// address widened.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdAm62pDssPlane {
    pub seq: u32,
    pub format: u32,
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub reserved: u32,
}

const _: () = assert!(size_of::<DssConfig>() == 36);
const _: () = assert!(offset_of!(DssConfig, hactive) == 4);
const _: () = assert!(offset_of!(DssConfig, vbp) == 18);
const _: () = assert!(offset_of!(DssConfig, flags) == 20);
const _: () = assert!(offset_of!(DssConfig, vp_index) == 24);
const _: () = assert!(offset_of!(DssConfig, safety_height) == 32);
const _: () = assert!(size_of::<DssStats>() == 28);
const _: () = assert!(size_of::<DssData>() == 136);
const _: () = assert!(offset_of!(DssData, vp) == 32);
const _: () = assert!(offset_of!(DssData, safety_buf) == 40);
const _: () = assert!(offset_of!(DssData, app_addr) == 56);
const _: () = assert!(offset_of!(DssData, app_format) == 76);
const _: () = assert!(offset_of!(DssData, app_seq_pending) == 80);
const _: () = assert!(offset_of!(DssData, app_seq_shown) == 84);
const _: () = assert!(offset_of!(DssData, app_commit_pending) == 88);
const _: () = assert!(offset_of!(DssData, safety_flip_pending) == 92);
const _: () = assert!(offset_of!(DssData, safety_front) == 96);
const _: () = assert!(offset_of!(DssData, app_enabled) == 100);
const _: () = assert!(offset_of!(DssData, output_enabled) == 104);
const _: () = assert!(offset_of!(DssData, stats) == 108);
const _: () = assert!(size_of::<YdAm62pDssPlane>() == 32);
const _: () = assert!(offset_of!(YdAm62pDssPlane, addr) == 8);
const _: () = assert!(offset_of!(YdAm62pDssPlane, stride_bytes) == 24);
// The flag and event bits as include/yakogl_drivers.h states them.
const _: () = {
    use dss::{event, flags};
    assert!(flags::HSYNC_LOW == 1 && flags::VSYNC_LOW == 2 && flags::DE_LOW == 4);
    assert!(flags::PIXCLK_NEGEDGE == 8 && flags::HAS_IRQ == 16);
    assert!(event::VSYNC == 1 && event::SYNC_LOST == 2 && event::UNDERFLOW == 4);
};

fn errno(r: Result<(), i32>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => e,
    }
}

/// Reset and program DSS1 (soc::am62p::dss::dss_init); the port stays off.
///
/// # Safety
/// `cfg`, `data`, `env` valid for the call; `data`'s bases mapped device
/// memory of DSS1's regions.
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_init(cfg: *const DssConfig, data: *const DssData, env: *const YdEnv) -> i32 {
    // SAFETY: the caller's contract.
    let (cfg, d, env, hw) = unsafe { (&*cfg, &*data, &*env, Hw::new()) };
    errno(dss::dss_init(&hw, env, cfg, d))
}

/// Point the application plane at a new buffer (next vsync).
///
/// # Safety
/// As [`yd_am62p_dss_init`]; `plane` valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_set_app_buffer(data: *const DssData, plane: *const YdAm62pDssPlane) -> i32 {
    // SAFETY: the caller's contract.
    let (d, p, hw) = unsafe { (&*data, &*plane, Hw::new()) };
    let app = AppPlane {
        seq: p.seq,
        plane: Plane { addr: p.addr, width: p.width, height: p.height, stride_bytes: p.stride_bytes, format: p.format },
    };
    errno(dss::set_app_buffer(&hw, d, &app))
}

/// # Safety
/// As [`yd_am62p_dss_init`].
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_enable_app_plane(data: *const DssData, on: bool) -> i32 {
    // SAFETY: the caller's contract.
    let (d, hw) = unsafe { (&*data, Hw::new()) };
    errno(dss::enable_app_plane(&hw, d, on))
}

/// # Safety
/// As [`yd_am62p_dss_init`].
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_safety_premultiplied(data: *const DssData, on: bool) {
    // SAFETY: the caller's contract.
    let (d, hw) = unsafe { (&*data, Hw::new()) };
    dss::safety_premultiplied(&hw, d, on)
}

/// # Safety
/// As [`yd_am62p_dss_init`].
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_safety_flip(data: *const DssData) {
    // SAFETY: the caller's contract.
    let (d, hw) = unsafe { (&*data, Hw::new()) };
    dss::safety_flip(&hw, d)
}

/// The safety layer's back buffer (where the renderer may draw).
///
/// # Safety
/// `data` valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_safety_back(data: *const DssData) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { &*data }.safety_back()
}

/// The safety layer's front buffer (scanned out).
///
/// # Safety
/// `data` valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_safety_front(data: *const DssData) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { &*data }.safety_front_addr()
}

/// # Safety
/// As [`yd_am62p_dss_init`].
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_output_enable(data: *const DssData, on: bool) {
    // SAFETY: the caller's contract.
    let (d, hw) = unsafe { (&*data, Hw::new()) };
    dss::output_enable(&hw, d, on)
}

/// The interrupt: returns `YD_AM62P_DSS_EV_*` bits.
///
/// # Safety
/// As [`yd_am62p_dss_init`]; runs in the ISR.
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_isr(cfg: *const DssConfig, data: *const DssData) -> u32 {
    // SAFETY: the caller's contract.
    let (cfg, d, hw) = unsafe { (&*cfg, &*data, Hw::new()) };
    dss::dss_isr(&hw, cfg, d)
}

/// Without an interrupt line: whether a vsync was pending (and is taken).
///
/// # Safety
/// As [`yd_am62p_dss_init`].
#[no_mangle]
pub unsafe extern "C" fn yd_am62p_dss_poll_vsync(cfg: *const DssConfig, data: *const DssData) -> bool {
    // SAFETY: the caller's contract.
    let (cfg, d, hw) = unsafe { (&*cfg, &*data, Hw::new()) };
    dss::poll_vsync(&hw, cfg, d)
}

// ---- Cadence DSI + D-PHY ---------------------------------------------------

/// `YD_CDNS_DSI_*` flags of [`YdCdnsDsiRequest`].
pub mod dsi_flags {
    pub const SYNC_PULSE: u32 = 1 << 0;
    pub const EOT: u32 = 1 << 1;
}

/// `struct yd_cdns_dsi_data`: per instance. The shim fills the bases.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct YdCdnsDsiData {
    pub dsi: usize,
    pub wrap: usize,
    pub dphy: usize,
    /// Where the D-PHY's WIZ registers start (init).
    pub wiz: u32,
    pub link_up: u32,
    /// The lane rate the link runs at, bit/s.
    pub rate: u64,
    pub ipdiv: u32,
    pub opdiv: u32,
    pub fbdiv: u32,
    pub band: u32,
    /// Bytes per line (HLINE x bpp / 8).
    pub line_bytes: u32,
    pub reserved: u32,
}

/// `struct yd_cdns_dsi_request`: what attach is asked for.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdCdnsDsiRequest {
    pub pixel_clock_hz: u32,
    pub hactive: u16,
    pub hfp: u16,
    pub hsync: u16,
    pub hbp: u16,
    pub vactive: u16,
    pub vfp: u16,
    pub vsync: u16,
    pub vbp: u16,
    pub bpp: u32,
    pub lanes: u32,
    /// `dsi_flags::*`.
    pub flags: u32,
    /// MIPI DSI data type of the pixel packets.
    pub datatype: u32,
    pub pll_ref_hz: u32,
    pub psm_hz: u32,
}

const _: () = assert!(size_of::<YdCdnsDsiData>() == 64);
const _: () = assert!(offset_of!(YdCdnsDsiData, wiz) == 24);
const _: () = assert!(offset_of!(YdCdnsDsiData, rate) == 32);
const _: () = assert!(offset_of!(YdCdnsDsiData, band) == 52);
const _: () = assert!(offset_of!(YdCdnsDsiData, line_bytes) == 56);
const _: () = assert!(size_of::<YdCdnsDsiRequest>() == 44);
const _: () = assert!(offset_of!(YdCdnsDsiRequest, vbp) == 18);
const _: () = assert!(offset_of!(YdCdnsDsiRequest, bpp) == 20);
const _: () = assert!(offset_of!(YdCdnsDsiRequest, psm_hz) == 40);
const _: () = assert!(dsi_flags::SYNC_PULSE == 1 && dsi_flags::EOT == 2);

fn bases(d: &YdCdnsDsiData) -> Bases {
    Bases { dsi: d.dsi, wrap: d.wrap, dphy: d.dphy }
}

pub(crate) fn request(r: &YdCdnsDsiRequest) -> Request {
    Request {
        mode: VideoMode {
            pixel_clock_hz: r.pixel_clock_hz,
            hactive: r.hactive.into(),
            hfront: r.hfp.into(),
            hsync: r.hsync.into(),
            hback: r.hbp.into(),
            vactive: r.vactive.into(),
            vfront: r.vfp.into(),
            vsync: r.vsync.into(),
            vback: r.vbp.into(),
            // Polarity is the DPI side's business (the DSS's VP).
            hsync_high: true,
            vsync_high: true,
            de_high: true,
        },
        bpp: r.bpp,
        lanes: r.lanes,
        sync_pulse: r.flags & dsi_flags::SYNC_PULSE != 0,
        eot: r.flags & dsi_flags::EOT != 0,
        datatype: r.datatype,
        pll_ref_hz: r.pll_ref_hz,
        psm_hz: r.psm_hz,
    }
}

/// Identify and quiesce the controller, find the D-PHY's WIZ.
///
/// # Safety
/// `data`, `env` valid for the call; the bases mapped device memory.
#[no_mangle]
pub unsafe extern "C" fn yd_cdns_dsi_init(data: *mut YdCdnsDsiData, env: *const YdEnv) -> i32 {
    // SAFETY: the caller's contract.
    let (d, env, hw) = unsafe { (&mut *data, &*env, Hw::new()) };
    match cdns_dsi::init(&hw, env, &bases(d)) {
        Ok(w) => {
            d.wiz = w;
            0
        }
        Err(e) => e,
    }
}

pub(crate) fn attach(
    m: &impl crate::mmio::Mmio,
    env: &impl crate::env::Env,
    d: &mut YdCdnsDsiData,
    r: &YdCdnsDsiRequest,
) -> i32 {
    if d.link_up != 0 {
        return -crate::errno::EBUSY;
    }
    match cdns_dsi::attach(m, env, &bases(d), d.wiz, &request(r)) {
        Ok(l) => {
            d.link_up = 1;
            d.rate = l.pll.rate;
            d.ipdiv = l.pll.ipdiv;
            d.opdiv = l.pll.opdiv;
            d.fbdiv = l.pll.fbdiv;
            d.band = l.pll.band;
            d.line_bytes = l.timings.line;
            0
        }
        Err(e) => e,
    }
}

/// Bring the link up and start video (ip::cdns_dsi::attach). -EBUSY while
/// attached.
///
/// # Safety
/// As [`yd_cdns_dsi_init`]; `req` valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_cdns_dsi_attach(
    data: *mut YdCdnsDsiData,
    env: *const YdEnv,
    req: *const YdCdnsDsiRequest,
) -> i32 {
    // SAFETY: the caller's contract.
    let (d, env, r, hw) = unsafe { (&mut *data, &*env, &*req, Hw::new()) };
    attach(&hw, env, d, r)
}

/// One direct write command. Returns the bytes sent or a negative errno.
///
/// # Safety
/// As [`yd_cdns_dsi_init`]; `buf` valid for `len` bytes (or `len` 0).
#[no_mangle]
pub unsafe extern "C" fn yd_cdns_dsi_transfer(
    data: *mut YdCdnsDsiData,
    env: *const YdEnv,
    channel: u8,
    datatype: u8,
    lp: bool,
    buf: *const u8,
    len: usize,
) -> i32 {
    // SAFETY: the caller's contract.
    let (d, env, hw) = unsafe { (&*data, &*env, Hw::new()) };
    if len > 0 && buf.is_null() {
        return -EINVAL;
    }
    let tx = if len == 0 {
        &[][..]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(buf, len) }
    };
    match cdns_dsi::transfer(&hw, env, &bases(d), channel, datatype, lp, tx) {
        Ok(n) => n as i32,
        Err(e) => e,
    }
}

/// # Safety
/// As [`yd_cdns_dsi_init`].
#[no_mangle]
pub unsafe extern "C" fn yd_cdns_dsi_detach(data: *mut YdCdnsDsiData) {
    // SAFETY: the caller's contract.
    let (d, hw) = unsafe { (&mut *data, Hw::new()) };
    cdns_dsi::detach(&hw, &bases(d));
    d.link_up = 0;
}
