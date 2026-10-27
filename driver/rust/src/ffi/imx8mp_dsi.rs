// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The C entry points of the Verdin's display controller and DSI host:
//! `yakogl,imx8mp-lcdif` (ip::lcdifv3; driver/drivers/display/
//! display_yakogl_imx8mp_lcdif.c, also serving the old
//! `yakogl,imx8mp-lcdif-adopt` nodes) and `yakogl,samsung-dsim`
//! (ip::samsung_dsim; driver/drivers/mipi_dsi/dsi_yakogl_samsung.c).
//! Design: driver/docs/imx8mp-display-cold.md.

use crate::env::{Env, YdEnv};
use crate::errno::{EINVAL, ENODEV};
use crate::ip::lcdifv3::{self as lcdif, Adopted, Layer, Output, Panic};
use crate::ip::samsung_dsim::{self as dsim, Link};
use crate::mmio::{Hw, Mmio};
use crate::video::VideoMode;
use crate::ylog;
use core::mem::{offset_of, size_of};

// ---- LCDIFv3 ------------------------------------------------------------------

/// `YD_LCDIF_*` flags of `struct yd_lcdif_config`.
pub mod lcdif_flags {
    /// `CTRL[INV_PXCK]`: the consumer samples on the rising edge.
    pub const PXCK_RISING: u32 = 1 << 0;
    pub const HSYNC_LOW: u32 = 1 << 1;
    pub const VSYNC_LOW: u32 = 1 << 2;
    pub const DE_LOW: u32 = 1 << 3;
    /// Never configure: adopt a running layer or nothing.
    pub const ADOPT_ONLY: u32 = 1 << 4;
    /// `panic_low` / `panic_high` are set.
    pub const PANIC: u32 = 1 << 5;
}

/// `struct yd_lcdif_config`: from the devicetree.
#[repr(C)]
pub struct YdLcdifConfig {
    pub flags: u32,
    /// The size the layer is expected to run (adopt) / will run (cold).
    pub hactive: u32,
    pub vactive: u32,
    /// OR-ed into CTRLDESCL0_3 (reserved bits; risk R1). 0 per the RM/SDK.
    pub desc3_extra: u32,
    pub panic_low: u16,
    pub panic_high: u16,
}

/// `struct yd_lcdif_mode`: the timing the chain owner hands over.
#[repr(C)]
pub struct YdLcdifMode {
    pub pixel_clock_hz: u32,
    pub hactive: u32,
    pub hfront: u32,
    pub hsync: u32,
    pub hback: u32,
    pub vactive: u32,
    pub vfront: u32,
    pub vsync: u32,
    pub vback: u32,
}

/// `struct yd_lcdif_data`: per instance.
#[repr(C)]
pub struct YdLcdifData {
    /// The mapped register base.
    pub base: usize,
    /// The buffer scanned now (ours), 0 before the first.
    pub fb: u64,
    /// Adopted: the previous owner's buffer, pitch word and format word.
    pub prev_fb: u64,
    pub prev_desc3: u32,
    pub prev_desc5: u32,
    pub our_pitch: u32,
    pub pitch: u32,
    pub width: u32,
    pub height: u32,
    /// INT_STATUS_D0 of the last interrupt.
    pub irq_status: u32,
    pub on: u8,
    pub adopted: u8,
    /// Adopted: the previous owner's layer is captured.
    pub held: u8,
    /// Adopted: our pitch and format are in place.
    pub ours: u8,
}

/// `struct yd_lcdif_scanout`.
#[repr(C)]
pub struct YdLcdifScanout {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bytes_pp: u32,
    /// CTRLDESCL0_5[BPP] (0x4 RGB565, 0x9 ARGB8888, 0xA ABGR8888).
    pub bpp_code: u32,
    pub reserved: u32,
    pub phys: u64,
}

const _: () = assert!(size_of::<YdLcdifConfig>() == 20);
const _: () = assert!(offset_of!(YdLcdifConfig, desc3_extra) == 12);
const _: () = assert!(offset_of!(YdLcdifConfig, panic_high) == 18);
const _: () = assert!(size_of::<YdLcdifMode>() == 36);
const _: () = assert!(offset_of!(YdLcdifMode, vback) == 32);
const _: () = assert!(size_of::<YdLcdifData>() == 56);
const _: () = assert!(offset_of!(YdLcdifData, fb) == 8);
const _: () = assert!(offset_of!(YdLcdifData, prev_desc3) == 24);
const _: () = assert!(offset_of!(YdLcdifData, irq_status) == 48);
const _: () = assert!(offset_of!(YdLcdifData, on) == 52);
const _: () = assert!(offset_of!(YdLcdifData, ours) == 55);
const _: () = assert!(size_of::<YdLcdifScanout>() == 32);
const _: () = assert!(offset_of!(YdLcdifScanout, bpp_code) == 16);
const _: () = assert!(offset_of!(YdLcdifScanout, phys) == 24);

fn adopted(d: &YdLcdifData) -> Adopted {
    Adopted {
        prev_phys: d.prev_fb,
        prev_desc3: d.prev_desc3,
        prev_desc5: d.prev_desc5,
        our_pitch: d.our_pitch,
        held: d.held != 0,
        ours: d.ours != 0,
    }
}

fn keep_adopted(d: &mut YdLcdifData, a: &Adopted) {
    d.prev_fb = a.prev_phys;
    d.prev_desc3 = a.prev_desc3;
    d.prev_desc5 = a.prev_desc5;
    d.our_pitch = a.our_pitch;
    d.held = a.held as u8;
    d.ours = a.ours as u8;
}

fn rc(r: Result<(), i32>) -> i32 {
    r.err().unwrap_or(0)
}

/// Program the layer and the timing (display off, layer off).
pub fn lcdif_configure(
    m: &impl Mmio,
    c: &YdLcdifConfig,
    d: &mut YdLcdifData,
    mode: &YdLcdifMode,
    fb: u64,
    pitch: u32,
) -> i32 {
    if c.flags & lcdif_flags::ADOPT_ONLY != 0 {
        return -ENODEV;
    }
    let f = |b| c.flags & b != 0;
    let vm = VideoMode {
        pixel_clock_hz: mode.pixel_clock_hz,
        hactive: mode.hactive,
        hfront: mode.hfront,
        hsync: mode.hsync,
        hback: mode.hback,
        vactive: mode.vactive,
        vfront: mode.vfront,
        vsync: mode.vsync,
        vback: mode.vback,
        hsync_high: !f(lcdif_flags::HSYNC_LOW),
        vsync_high: !f(lcdif_flags::VSYNC_LOW),
        de_high: !f(lcdif_flags::DE_LOW),
    };
    let out = Output {
        pxck_rising: f(lcdif_flags::PXCK_RISING),
        hsync_low: f(lcdif_flags::HSYNC_LOW),
        vsync_low: f(lcdif_flags::VSYNC_LOW),
        de_low: f(lcdif_flags::DE_LOW),
    };
    match lcdif::configure(m, d.base, &vm, &out, &Layer { phys: fb, pitch, desc3_extra: c.desc3_extra }) {
        Ok(p) => {
            d.fb = fb;
            d.pitch = p;
            d.width = mode.hactive;
            d.height = mode.vactive;
            d.adopted = 0;
            d.on = 0;
            0
        }
        Err(e) => e,
    }
}

pub fn lcdif_enable(m: &impl Mmio, c: &YdLcdifConfig, d: &mut YdLcdifData, env: &impl Env, vblank_irq: bool) -> i32 {
    let panic = (c.flags & lcdif_flags::PANIC != 0)
        .then_some(Panic { low: u32::from(c.panic_low), high: u32::from(c.panic_high) });
    lcdif::set_vblank_irq(m, d.base, vblank_irq);
    let r = lcdif::enable(m, d.base, env, panic);
    if r.is_ok() {
        d.on = 1;
    }
    rc(r)
}

pub fn lcdif_adopt(m: &impl Mmio, c: &YdLcdifConfig, d: &mut YdLcdifData, env: &impl Env, fb: u64, pitch: u32) -> i32 {
    let mut a = adopted(d);
    let r = lcdif::adopt(m, d.base, env, &mut a, (c.hactive, c.vactive), fb, pitch);
    keep_adopted(d, &a);
    match r {
        Ok(s) => {
            d.adopted = 1;
            d.on = 1;
            d.width = s.width;
            d.height = s.height;
            if fb == 0 {
                d.fb = s.phys;
                d.pitch = s.pitch;
            } else {
                d.fb = fb;
                d.pitch = a.our_pitch;
            }
            ylog!(
                env,
                Inf,
                "lcdif: adopted {}x{}, {} bpp at {:#x}{}",
                s.width,
                s.height,
                s.bytes_pp * 8,
                s.phys,
                if fb == 0 { ", scan-out kept" } else { ", now ours" }
            );
            0
        }
        Err(e) => {
            if e == -EINVAL {
                ylog!(env, Err, "lcdif: the running layer is not {}x{}", c.hactive, c.vactive);
            }
            e
        }
    }
}

pub fn lcdif_set_fb(m: &impl Mmio, d: &mut YdLcdifData, env: &impl Env, fb: u64) -> i32 {
    if d.on == 0 {
        return -ENODEV;
    }
    let r = if d.adopted != 0 {
        let mut a = adopted(d);
        let r = lcdif::adopt_set_fb(m, d.base, env, &mut a, fb);
        keep_adopted(d, &a);
        if r.is_ok() {
            d.pitch = a.our_pitch;
        }
        r
    } else {
        lcdif::set_fb(m, d.base, env, fb)
    };
    if r.is_ok() {
        d.fb = fb;
    }
    rc(r)
}

/// Adopted: hand the previous owner's layer back. Cold: stop at a frame
/// boundary, display off.
pub fn lcdif_release(m: &impl Mmio, d: &mut YdLcdifData, env: &impl Env) -> i32 {
    if d.on == 0 {
        return 0;
    }
    let r = if d.adopted != 0 {
        let mut a = adopted(d);
        let r = lcdif::adopt_release(m, d.base, env, &mut a);
        keep_adopted(d, &a);
        r
    } else {
        lcdif::disable(m, d.base, env)
    };
    d.on = 0;
    rc(r)
}

pub fn lcdif_read_scanout(m: &impl Mmio, base: usize, out: &mut YdLcdifScanout) -> bool {
    match lcdif::read_scanout(m, base) {
        Some(s) => {
            *out = YdLcdifScanout {
                width: s.width,
                height: s.height,
                pitch: s.pitch,
                bytes_pp: s.bytes_pp,
                bpp_code: lcdif::bpp_code(m, base),
                reserved: 0,
                phys: s.phys,
            };
            true
        }
        None => false,
    }
}

/// # Safety
/// `cfg`, `data`, `env`, `mode` valid for the call; `data.base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_configure(
    cfg: *const YdLcdifConfig,
    data: *mut YdLcdifData,
    mode: *const YdLcdifMode,
    fb_phys: u64,
    pitch: u32,
) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { lcdif_configure(&Hw::new(), &*cfg, &mut *data, &*mode, fb_phys, pitch) }
}

/// # Safety
/// As [`yd_lcdif_configure`].
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_enable(
    cfg: *const YdLcdifConfig,
    data: *mut YdLcdifData,
    env: *const YdEnv,
    vblank_irq: bool,
) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { lcdif_enable(&Hw::new(), &*cfg, &mut *data, &*env, vblank_irq) }
}

/// # Safety
/// As [`yd_lcdif_configure`].
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_adopt(
    cfg: *const YdLcdifConfig,
    data: *mut YdLcdifData,
    env: *const YdEnv,
    fb_phys: u64,
    pitch: u32,
) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { lcdif_adopt(&Hw::new(), &*cfg, &mut *data, &*env, fb_phys, pitch) }
}

/// # Safety
/// As [`yd_lcdif_configure`].
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_set_fb(data: *mut YdLcdifData, env: *const YdEnv, fb_phys: u64) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { lcdif_set_fb(&Hw::new(), &mut *data, &*env, fb_phys) }
}

/// # Safety
/// As [`yd_lcdif_configure`].
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_release(data: *mut YdLcdifData, env: *const YdEnv) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { lcdif_release(&Hw::new(), &mut *data, &*env) }
}

/// Stateless, for callers that only hold a mapping (the panel console at
/// PRE_KERNEL_1, the chain facade's health word).
///
/// # Safety
/// `base` is a mapped LCDIFv3 register block.
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_is_running(base: usize) -> bool {
    // SAFETY: the caller's contract.
    lcdif::is_running(unsafe { &Hw::new() }, base)
}

/// # Safety
/// As [`yd_lcdif_is_running`].
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_layer_control(base: usize) -> u32 {
    // SAFETY: the caller's contract.
    lcdif::layer_control(unsafe { &Hw::new() }, base)
}

/// # Safety
/// As [`yd_lcdif_is_running`]; `out` writable.
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_read_scanout(base: usize, out: *mut YdLcdifScanout) -> bool {
    // SAFETY: the caller's contract.
    unsafe { lcdif_read_scanout(&Hw::new(), base, &mut *out) }
}

/// A flip of a running layer on a mapping alone (the chain owner's adopt
/// path: a layer the previous kernel set up), waited for with the timer.
///
/// # Safety
/// As [`yd_lcdif_is_running`]; `env` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_flip(base: usize, env: *const YdEnv, fb_phys: u64) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { rc(lcdif::set_fb(&Hw::new(), base, &*env, fb_phys)) }
}

/// A flip whose wait counts register reads (no timer before PRE_KERNEL_2).
///
/// # Safety
/// As [`yd_lcdif_is_running`].
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_flip_polled(base: usize, fb_phys: u64, max_reads: u32) -> i32 {
    // SAFETY: the caller's contract.
    rc(lcdif::set_fb_polled(unsafe { &Hw::new() }, base, fb_phys, max_reads))
}

/// The interrupt: INT_STATUS_D0, read and cleared.
///
/// # Safety
/// `data` valid, `data.base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_lcdif_isr(data: *mut YdLcdifData) -> u32 {
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *data };
    // SAFETY: as above.
    let s = lcdif::take_status(unsafe { &Hw::new() }, d.base);
    d.irq_status = s;
    s
}

// ---- Samsung DSIM -------------------------------------------------------------

/// `YD_SAMSUNG_DSIM_*` flags of `struct yd_samsung_dsim_config`.
pub mod dsim_flags {
    pub const BURST: u32 = 1 << 0;
}

/// `yd_samsung_dsim_health()` bits, read from STATUS / ESCMODE.
pub mod dsim_health {
    pub const PLL_STABLE: u32 = 1 << 0;
    pub const HS_CLOCK: u32 = 1 << 1;
    /// Data lanes held in stop state (attached, video not started).
    pub const HELD: u32 = 1 << 2;
}

/// `struct yd_samsung_dsim_config`: from the devicetree.
#[repr(C)]
pub struct YdSamsungDsimConfig {
    pub pll_ref_hz: u32,
    /// burst-clock-frequency (0: the slowest rate that carries the stream).
    pub lane_rate_hz: u32,
    pub esc_max_hz: u32,
    pub lanes: u32,
    pub flags: u32,
}

/// `struct yd_samsung_dsim_mode`: the attach request.
#[repr(C)]
pub struct YdSamsungDsimMode {
    pub pixel_clock_hz: u32,
    pub hactive: u32,
    pub hfront: u32,
    pub hsync: u32,
    pub hback: u32,
    pub vactive: u32,
    pub vfront: u32,
    pub vsync: u32,
    pub vback: u32,
    /// 0 = the devicetree's.
    pub lanes: u32,
}

/// `struct yd_samsung_dsim_data`: per instance, what the last attach did.
#[repr(C)]
pub struct YdSamsungDsimData {
    pub base: usize,
    pub pixel_clock_hz: u32,
    pub lane_rate_hz: u32,
    pub byte_hz: u32,
    pub esc_hz: u32,
    pub plltmr: u32,
    pub pll_p: u32,
    pub pll_m: u32,
    pub pll_s: u32,
    pub phytiming: [u32; 3],
    /// samsung_dsim::step reached (or failed in, with `error`).
    pub step: u32,
    pub error: i32,
    pub attached: u8,
    pub reserved: [u8; 3],
}

const _: () = assert!(size_of::<YdSamsungDsimConfig>() == 20);
const _: () = assert!(offset_of!(YdSamsungDsimConfig, flags) == 16);
const _: () = assert!(size_of::<YdSamsungDsimMode>() == 40);
const _: () = assert!(offset_of!(YdSamsungDsimMode, lanes) == 36);
const _: () = assert!(size_of::<YdSamsungDsimData>() == 64);
const _: () = assert!(offset_of!(YdSamsungDsimData, pixel_clock_hz) == 8);
const _: () = assert!(offset_of!(YdSamsungDsimData, pll_p) == 28);
const _: () = assert!(offset_of!(YdSamsungDsimData, phytiming) == 40);
const _: () = assert!(offset_of!(YdSamsungDsimData, step) == 52);
const _: () = assert!(offset_of!(YdSamsungDsimData, attached) == 60);

pub fn dsim_attach(
    m: &impl Mmio,
    c: &YdSamsungDsimConfig,
    d: &mut YdSamsungDsimData,
    env: &impl Env,
    mode: &YdSamsungDsimMode,
) -> i32 {
    let link = Link {
        mode: VideoMode {
            pixel_clock_hz: mode.pixel_clock_hz,
            hactive: mode.hactive,
            hfront: mode.hfront,
            hsync: mode.hsync,
            hback: mode.hback,
            vactive: mode.vactive,
            vfront: mode.vfront,
            vsync: mode.vsync,
            vback: mode.vback,
            hsync_high: true,
            vsync_high: true,
            de_high: true,
        },
        lanes: if mode.lanes != 0 { mode.lanes } else { c.lanes },
        pll_ref_hz: c.pll_ref_hz,
        lane_rate_hz: c.lane_rate_hz,
        esc_max_hz: c.esc_max_hz,
        burst: c.flags & dsim_flags::BURST != 0,
    };
    d.attached = 0;
    match dsim::attach(m, d.base, env, &link) {
        Ok(p) => {
            d.pixel_clock_hz = mode.pixel_clock_hz;
            d.lane_rate_hz = p.pms.fout_hz;
            d.byte_hz = p.byte_hz;
            d.esc_hz = p.esc_hz;
            d.plltmr = p.plltmr;
            d.pll_p = p.pms.p;
            d.pll_m = p.pms.m;
            d.pll_s = p.pms.s;
            d.phytiming = [
                m.read32(d.base + crate::soc::imx8mp::regs::mipi_dsi::DSI_PHYTIMING as usize),
                m.read32(d.base + crate::soc::imx8mp::regs::mipi_dsi::DSI_PHYTIMING1 as usize),
                m.read32(d.base + crate::soc::imx8mp::regs::mipi_dsi::DSI_PHYTIMING2 as usize),
            ];
            d.step = dsim::step::HELD;
            d.error = 0;
            d.attached = 1;
            ylog!(
                env,
                Inf,
                "dsim: PLL p {} m {} s {} -> {} Hz from {} Hz, byte {} Hz, esc {} Hz; data lanes held",
                p.pms.p,
                p.pms.m,
                p.pms.s,
                p.pms.fout_hz,
                c.pll_ref_hz,
                p.byte_hz,
                p.esc_hz
            );
            0
        }
        Err(f) => {
            d.step = f.step;
            d.error = f.errno;
            ylog!(
                env,
                Err,
                "dsim: attach failed in step {}: {} (STATUS {:#010x})",
                f.step,
                f.errno,
                dsim::status(m, d.base)
            );
            f.errno
        }
    }
}

pub fn dsim_video_start(m: &impl Mmio, d: &mut YdSamsungDsimData) -> i32 {
    if d.attached == 0 {
        return -ENODEV;
    }
    dsim::video_start(m, d.base);
    d.step = dsim::step::VIDEO;
    0
}

pub fn dsim_health(m: &impl Mmio, d: &YdSamsungDsimData) -> u32 {
    let mut h = 0;
    if dsim::pll_is_stable(m, d.base) {
        h |= dsim_health::PLL_STABLE;
    }
    if dsim::hs_clock_is_ready(m, d.base) {
        h |= dsim_health::HS_CLOCK;
    }
    if dsim::is_held(m, d.base) {
        h |= dsim_health::HELD;
    }
    h
}

/// # Safety
/// `cfg`, `data`, `env`, `mode` valid for the call; `data.base` mapped; the
/// block powered, clocked and out of reset (the caller's power domain).
#[no_mangle]
pub unsafe extern "C" fn yd_samsung_dsim_attach(
    cfg: *const YdSamsungDsimConfig,
    data: *mut YdSamsungDsimData,
    env: *const YdEnv,
    mode: *const YdSamsungDsimMode,
) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { dsim_attach(&Hw::new(), &*cfg, &mut *data, &*env, &*mode) }
}

/// # Safety
/// As [`yd_samsung_dsim_attach`].
#[no_mangle]
pub unsafe extern "C" fn yd_samsung_dsim_video_start(data: *mut YdSamsungDsimData) -> i32 {
    // SAFETY: the caller's contract.
    unsafe { dsim_video_start(&Hw::new(), &mut *data) }
}

/// # Safety
/// As [`yd_samsung_dsim_attach`].
#[no_mangle]
pub unsafe extern "C" fn yd_samsung_dsim_detach(data: *mut YdSamsungDsimData) {
    // SAFETY: the caller's contract.
    let d = unsafe { &mut *data };
    // SAFETY: as above.
    dsim::disable(unsafe { &Hw::new() }, d.base);
    d.attached = 0;
    d.step = 0;
}

/// # Safety
/// As [`yd_samsung_dsim_attach`] (reads only).
#[no_mangle]
pub unsafe extern "C" fn yd_samsung_dsim_health(data: *const YdSamsungDsimData) -> u32 {
    // SAFETY: the caller's contract.
    unsafe { dsim_health(&Hw::new(), &*data) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, TestEnv, DSIM, LCDIF1};
    use crate::soc::imx8mp::media::tests::{bases, bring_up, cold};
    use crate::soc::imx8mp::media::Track;

    const MODE: YdLcdifMode = YdLcdifMode {
        pixel_clock_hz: 69_300_000,
        hactive: 1280,
        hfront: 36,
        hsync: 24,
        hback: 64,
        vactive: 800,
        vfront: 8,
        vsync: 8,
        vback: 24,
    };
    const LCFG: YdLcdifConfig = YdLcdifConfig {
        flags: lcdif_flags::PXCK_RISING | lcdif_flags::PANIC,
        hactive: 1280,
        vactive: 800,
        desc3_extra: 0,
        panic_low: 170,
        panic_high: 340,
    };
    const DCFG: YdSamsungDsimConfig = YdSamsungDsimConfig {
        pll_ref_hz: 12_000_000,
        lane_rate_hz: 426_600_000,
        esc_max_hz: 20_000_000,
        lanes: 4,
        flags: dsim_flags::BURST,
    };

    fn ldata(base: usize) -> YdLcdifData {
        YdLcdifData {
            base,
            fb: 0,
            prev_fb: 0,
            prev_desc3: 0,
            prev_desc5: 0,
            our_pitch: 0,
            pitch: 0,
            width: 0,
            height: 0,
            irq_status: 0,
            on: 0,
            adopted: 0,
            held: 0,
            ours: 0,
        }
    }

    fn up() -> Model {
        let m = Model::new();
        cold(&m);
        let e = TestEnv::default();
        bring_up(&m, &bases(&m), &e, &mut Track::default(), 1_039_500_000).unwrap();
        m
    }

    #[test]
    fn the_c_header_has_the_link_margin() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let pre = "#define YD_SAMSUNG_DSIM_LINK_MARGIN_PERMILLE ";
        let l = H.lines().find(|l| l.starts_with(pre)).expect("YD_SAMSUNG_DSIM_LINK_MARGIN_PERMILLE");
        let v: String = l[pre.len()..].chars().take_while(char::is_ascii_digit).collect();
        assert_eq!(v.parse::<u64>().unwrap(), dsim::LINK_MARGIN_PERMILLE);
    }

    #[test]
    fn the_c_header_names_the_same_bits() {
        const H: &str = include_str!("../../include/yakogl_drivers.h");
        let bit = |name: &str| -> u32 {
            let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            let n: String = l.split("<<").nth(1).unwrap().trim().chars().take_while(char::is_ascii_digit).collect();
            1 << n.parse::<u32>().unwrap()
        };
        use crate::soc::imx8mp::regs::lcdif::int_status_d0 as irq;
        for (name, v) in [
            ("YD_LCDIF_PXCK_RISING", lcdif_flags::PXCK_RISING),
            ("YD_LCDIF_HSYNC_LOW", lcdif_flags::HSYNC_LOW),
            ("YD_LCDIF_VSYNC_LOW", lcdif_flags::VSYNC_LOW),
            ("YD_LCDIF_DE_LOW", lcdif_flags::DE_LOW),
            ("YD_LCDIF_ADOPT_ONLY", lcdif_flags::ADOPT_ONLY),
            ("YD_LCDIF_PANIC", lcdif_flags::PANIC),
            ("YD_LCDIF_IRQ_VSYNC", irq::VSYNC_MASK),
            ("YD_LCDIF_IRQ_UNDERRUN", irq::UNDERRUN_MASK),
            ("YD_LCDIF_IRQ_VS_BLANK", irq::VS_BLANK_MASK),
            ("YD_SAMSUNG_DSIM_BURST", dsim_flags::BURST),
            ("YD_SAMSUNG_DSIM_H_PLL_STABLE", dsim_health::PLL_STABLE),
            ("YD_SAMSUNG_DSIM_H_HS_CLOCK", dsim_health::HS_CLOCK),
            ("YD_SAMSUNG_DSIM_H_HELD", dsim_health::HELD),
        ] {
            assert_eq!(bit(name), v, "{name}");
        }
    }

    #[test]
    fn through_the_c_structs_cold_then_adopt() {
        let m = up();
        let e = TestEnv::default();
        let mut l = ldata(m.base(LCDIF1));
        assert_eq!(lcdif_configure(&m, &LCFG, &mut l, &MODE, 0x9000_0000, 0), 0);
        assert_eq!(l.pitch, 5120);
        let mut d = YdSamsungDsimData {
            base: m.base(DSIM),
            pixel_clock_hz: 0,
            lane_rate_hz: 0,
            byte_hz: 0,
            esc_hz: 0,
            plltmr: 0,
            pll_p: 0,
            pll_m: 0,
            pll_s: 0,
            phytiming: [0; 3],
            step: 0,
            error: 0,
            attached: 0,
            reserved: [0; 3],
        };
        let dm = YdSamsungDsimMode {
            pixel_clock_hz: 69_300_000,
            hactive: 1280,
            hfront: 36,
            hsync: 24,
            hback: 64,
            vactive: 800,
            vfront: 8,
            vsync: 8,
            vback: 24,
            lanes: 0,
        };
        assert_eq!(dsim_video_start(&m, &mut d), -ENODEV, "not attached");
        assert_eq!(dsim_attach(&m, &DCFG, &mut d, &e, &dm), 0);
        assert_eq!((d.pll_p, d.pll_m, d.pll_s, d.lane_rate_hz, d.step), (5, 711, 2, 426_600_000, dsim::step::HELD));
        assert_eq!(d.phytiming, [0x306, 0x030d_0a04, 0x0003_0604]);
        assert_eq!(dsim_health(&m, &d), dsim_health::PLL_STABLE | dsim_health::HS_CLOCK | dsim_health::HELD);
        assert_eq!(lcdif_enable(&m, &LCFG, &mut l, &e, true), 0);
        assert_eq!(dsim_video_start(&m, &mut d), 0);
        assert_eq!(dsim_health(&m, &d), dsim_health::PLL_STABLE | dsim_health::HS_CLOCK);
        assert_eq!(lcdif_set_fb(&m, &mut l, &e, 0x9100_0000), 0);
        let mut s = YdLcdifScanout { width: 0, height: 0, pitch: 0, bytes_pp: 0, bpp_code: 0, reserved: 0, phys: 0 };
        assert!(lcdif_read_scanout(&m, l.base, &mut s));
        assert_eq!((s.width, s.height, s.pitch, s.bpp_code, s.phys), (1280, 800, 5120, 9, 0x9100_0000));

        // a second instance adopts what the first left running (the crash
        // route), with no address first: no write
        let ad = YdLcdifConfig { flags: lcdif_flags::ADOPT_ONLY, ..LCFG };
        let mut a = ldata(m.base(LCDIF1));
        assert_eq!(lcdif_configure(&m, &ad, &mut a, &MODE, 0x9000_0000, 0), -ENODEV, "adopt-only never configures");
        m.clear_log();
        assert_eq!(lcdif_adopt(&m, &ad, &mut a, &e, 0, 0), 0);
        assert!(m.writes().is_empty());
        assert_eq!((a.on, a.adopted, a.fb, a.held, a.ours), (1, 1, 0x9100_0000, 1, 0));
        assert_eq!(lcdif_set_fb(&m, &mut a, &e, 0x9200_0000), 0);
        assert_eq!(a.ours, 1);
        assert_eq!(lcdif_release(&m, &mut a, &e), 0);
        assert!(lcdif_read_scanout(&m, a.base, &mut s));
        assert_eq!(s.phys, 0x9100_0000, "given back");
        assert_eq!(
            lcdif_adopt(&m, &YdLcdifConfig { hactive: 1920, ..ad }, &mut ldata(m.base(LCDIF1)), &e, 0, 0),
            -EINVAL
        );
    }
}
