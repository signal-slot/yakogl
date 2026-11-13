// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F-DK's display path, LTDC -> DesignWare DSI host with ST's
//! MP25 PHY wrapper -> ADV7535 -> HDMI, for the three Zephyr drivers
//! driver/drivers/display/display_yakogl_stm32_ltdc.c (`yakogl,stm32-ltdc`,
//! ip::stm32_ltdc), driver/drivers/mipi_dsi/dsi_yakogl_stm32.c
//! (`yakogl,stm32-dsi`, ip::dw_mipi_dsi over ip::stm32_dsi_phy) and
//! driver/drivers/display/bridge_adv7535.c (`yakogl,adv7535`, ip::adv7535),
//! plus the EDID mode choice (crate::video). Declarations:
//! include/yakogl_mp25_display.h, which also states the call order.
//!
//! Only the blocks' own registers and the ADV7535's I2C maps are touched
//! here. The RCC gates and kernel clocks, the resets, the SCMI clock and
//! voltage requests (HDMI 5 V, the DSI PHY supplies), the pins, the RIF
//! settings, the ADV7535 reset line and the I2C controller's own setup are
//! the caller's (soc::stm32mp2::display, item display-ffi), done before the
//! first call.
//!
//! The ADV7535's I2C goes through the `env`'s `i2c_write` / `i2c_write_read`
//! with ip::adv7535's bus ids (`YD_MP25_ADV7535_BUS_*`); the shim maps each id
//! to its 7-bit address on the board's bus (0x3d, 0x3c, 0x3f, 0x38 on the DK).

use crate::env::{Env, YdEnv};
use crate::errno::{EBUSY, EINVAL, ENODEV, ENOTSUP};
use crate::ip::adv7535::{self as adv, Bridge, Sink};
use crate::ip::dw_mipi_dsi::{self as dsi, DphyTiming, Link, Phy};
use crate::ip::stm32_dsi_phy::{self as dphy, MipiFormat, Pll};
use crate::ip::stm32_ltdc::{self as ltdc, Ltdc};
use crate::mmio::{Hw, Mmio};
use crate::video::{self, VideoMode, FHD1080_60, HD720_60};
use crate::ylog;
use core::mem::{offset_of, size_of};

// ---- the mode -------------------------------------------------------------------

/// `flags` of [`YdMp25Mode`].
pub const YD_MP25_MODE_HSYNC_LOW: u32 = 1 << 0;
pub const YD_MP25_MODE_VSYNC_LOW: u32 = 1 << 1;
pub const YD_MP25_MODE_DE_LOW: u32 = 1 << 2;

/// `struct yd_mp25_mode`: one video mode for all three blocks, in the
/// devicetree's porch terms (crate::video::VideoMode).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp25Mode {
    pub pixel_clock_hz: u32,
    pub hactive: u32,
    pub hfront: u32,
    pub hsync: u32,
    pub hback: u32,
    pub vactive: u32,
    pub vfront: u32,
    pub vsync: u32,
    pub vback: u32,
    /// `YD_MP25_MODE_*`.
    pub flags: u32,
}

/// `struct yd_mp25_hdmi_sink`: what the ADV7535 sends (ip::adv7535::Sink).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp25HdmiSink {
    /// 1: HDMI (AVI infoframe sent), 0: DVI.
    pub hdmi: u8,
    /// CTA-861 VIC (4 = 720p60, 16 = 1080p60), 0 for none.
    pub vic: u8,
    /// 1: 16:9 picture, 0: 4:3.
    pub aspect_16_9: u8,
    pub reserved: u8,
    pub refresh_hz: u32,
}

const _: () = assert!(size_of::<YdMp25Mode>() == 40);
const _: () = assert!(offset_of!(YdMp25Mode, pixel_clock_hz) == 0);
const _: () = assert!(offset_of!(YdMp25Mode, hactive) == 4);
const _: () = assert!(offset_of!(YdMp25Mode, hback) == 16);
const _: () = assert!(offset_of!(YdMp25Mode, vactive) == 20);
const _: () = assert!(offset_of!(YdMp25Mode, vback) == 32);
const _: () = assert!(offset_of!(YdMp25Mode, flags) == 36);
const _: () = assert!(size_of::<YdMp25HdmiSink>() == 8);
const _: () = assert!(offset_of!(YdMp25HdmiSink, vic) == 1);
const _: () = assert!(offset_of!(YdMp25HdmiSink, aspect_16_9) == 2);
const _: () = assert!(offset_of!(YdMp25HdmiSink, refresh_hz) == 4);

impl YdMp25Mode {
    fn video(&self) -> VideoMode {
        let f = |b| self.flags & b != 0;
        VideoMode {
            pixel_clock_hz: self.pixel_clock_hz,
            hactive: self.hactive,
            hfront: self.hfront,
            hsync: self.hsync,
            hback: self.hback,
            vactive: self.vactive,
            vfront: self.vfront,
            vsync: self.vsync,
            vback: self.vback,
            hsync_high: !f(YD_MP25_MODE_HSYNC_LOW),
            vsync_high: !f(YD_MP25_MODE_VSYNC_LOW),
            de_high: !f(YD_MP25_MODE_DE_LOW),
        }
    }

    fn from_video(m: &VideoMode) -> YdMp25Mode {
        YdMp25Mode {
            pixel_clock_hz: m.pixel_clock_hz,
            hactive: m.hactive,
            hfront: m.hfront,
            hsync: m.hsync,
            hback: m.hback,
            vactive: m.vactive,
            vfront: m.vfront,
            vsync: m.vsync,
            vback: m.vback,
            flags: if m.hsync_high { 0 } else { YD_MP25_MODE_HSYNC_LOW }
                | if m.vsync_high { 0 } else { YD_MP25_MODE_VSYNC_LOW }
                | if m.de_high { 0 } else { YD_MP25_MODE_DE_LOW },
        }
    }
}

fn rc(r: Result<(), i32>) -> i32 {
    r.err().unwrap_or(0)
}

// ---- LTDC -----------------------------------------------------------------------

/// `flags` of [`YdMp25LtdcConfig`]: `GCR.PCPOL` (`pixclk-falling`).
pub const YD_MP25_LTDC_PIXCLK_FALLING: u32 = 1 << 0;
/// `burst_bytes` is valid: write it to `L1BLCR` (ST's `st,burstlen` / 8).
pub const YD_MP25_LTDC_BURSTLEN: u32 = 1 << 1;

/// Layer formats (`pixel-format`); ip::stm32_ltdc has no 24-bit RGB888.
pub const YD_MP25_LTDC_FMT_ARGB8888: u32 = 0;
pub const YD_MP25_LTDC_FMT_XRGB8888: u32 = 1;
pub const YD_MP25_LTDC_FMT_RGB565: u32 = 2;

/// `struct yd_mp25_ltdc_config`: from the devicetree.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YdMp25LtdcConfig {
    /// The mapped registers (the shim's `device_map()`).
    pub base: usize,
    /// `YD_MP25_LTDC_*`.
    pub flags: u32,
    /// `BCCR`, 0x00RRGGBB.
    pub background: u32,
    /// `L1BLCR` with `YD_MP25_LTDC_BURSTLEN` (0 = the maximum).
    pub burst_bytes: u32,
    pub reserved: u32,
}

/// `struct yd_mp25_ltdc_data`: per instance.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp25LtdcData {
    /// `IDR` (0x040101 on the MP25), from the last probe.
    pub hwver: u32,
    /// The AXI bus width in bytes (`GC2R`): pitch and address alignment.
    pub bus_bytes: u32,
    /// `LCR`, 1..=4.
    pub layers: u32,
    /// Bytes between line starts, as configured.
    pub pitch: u32,
    /// `CFBLR`'s line length.
    pub line_length: u32,
    pub width: u32,
    pub height: u32,
    /// `YD_MP25_LTDC_FMT_*`.
    pub format: u32,
    /// `ISR` as the last [`yd_mp25_ltdc_isr`] read it.
    pub isr: u32,
    /// 1 while enabled.
    pub on: u8,
    /// 1 after a successful probe.
    pub probed: u8,
    /// 1 after a successful configure.
    pub configured: u8,
    pub reserved: u8,
    /// The buffer scanned out (or to be, before enable).
    pub fb: u64,
}

const _: () = assert!(size_of::<YdMp25LtdcConfig>() == 24);
const _: () = assert!(offset_of!(YdMp25LtdcConfig, base) == 0);
const _: () = assert!(offset_of!(YdMp25LtdcConfig, flags) == 8);
const _: () = assert!(offset_of!(YdMp25LtdcConfig, background) == 12);
const _: () = assert!(offset_of!(YdMp25LtdcConfig, burst_bytes) == 16);
const _: () = assert!(size_of::<YdMp25LtdcData>() == 48);
const _: () = assert!(offset_of!(YdMp25LtdcData, bus_bytes) == 4);
const _: () = assert!(offset_of!(YdMp25LtdcData, pitch) == 12);
const _: () = assert!(offset_of!(YdMp25LtdcData, format) == 28);
const _: () = assert!(offset_of!(YdMp25LtdcData, isr) == 32);
const _: () = assert!(offset_of!(YdMp25LtdcData, on) == 36);
const _: () = assert!(offset_of!(YdMp25LtdcData, configured) == 38);
const _: () = assert!(offset_of!(YdMp25LtdcData, fb) == 40);

/// The registers the extern entry points reach: the hardware on the board.
#[cfg(not(test))]
fn port() -> Hw {
    // SAFETY: `cfg->base` is the shim's `device_map()` range (the entry
    // points' contract).
    unsafe { Hw::new() }
}

/// In the host tests the entry points reach the register model installed
/// with `tests::with_display`, or plain memory when none is installed (the
/// pattern of `ffi::mp25_i2c`).
#[cfg(test)]
struct Port;

#[cfg(test)]
thread_local! {
    static DISPLAY: core::cell::Cell<Option<*const dyn Mmio>> = const { core::cell::Cell::new(None) };
}

#[cfg(test)]
fn port() -> Port {
    Port
}

#[cfg(test)]
impl Port {
    fn with<R>(&self, f: impl FnOnce(&dyn Mmio) -> R) -> R {
        match DISPLAY.with(|d| d.get()) {
            // SAFETY: `with_display` keeps the model alive while installed.
            Some(m) => f(unsafe { &*m }),
            // SAFETY: the caller's addresses are plain memory it owns.
            None => f(&unsafe { Hw::new() }),
        }
    }
}

#[cfg(test)]
impl Mmio for Port {
    fn read32(&self, a: usize) -> u32 {
        self.with(|m| m.read32(a))
    }
    fn write32(&self, a: usize, v: u32) {
        self.with(|m| m.write32(a, v))
    }
    fn read8(&self, a: usize) -> u8 {
        self.with(|m| m.read8(a))
    }
    fn write8(&self, a: usize, v: u8) {
        self.with(|m| m.write8(a, v))
    }
}

fn ltdc_format(f: u32) -> Result<ltdc::Format, i32> {
    match f {
        YD_MP25_LTDC_FMT_ARGB8888 => Ok(ltdc::Format::Argb8888),
        YD_MP25_LTDC_FMT_XRGB8888 => Ok(ltdc::Format::Xrgb8888),
        YD_MP25_LTDC_FMT_RGB565 => Ok(ltdc::Format::Rgb565),
        _ => Err(-ENOTSUP),
    }
}

/// The block as probed: `IDR`, `GC2R` and `LCR` reads only, so every call
/// re-reads them rather than keep a Rust value in C memory.
fn ltdc_of(m: &impl Mmio, c: &YdMp25LtdcConfig) -> Result<Ltdc, i32> {
    Ltdc::probe(m, c.base)
}

pub(crate) fn ltdc_probe_with(m: &impl Mmio, env: &impl Env, c: &YdMp25LtdcConfig, d: &mut YdMp25LtdcData) -> i32 {
    d.probed = 0;
    match ltdc_of(m, c) {
        Ok(l) => {
            d.hwver = l.hwver;
            d.bus_bytes = l.bus_bytes;
            d.layers = l.layers;
            d.probed = 1;
            ylog!(env, Inf, "ltdc: version {:#08x}, {} layers, {}-byte bus", l.hwver, l.layers, l.bus_bytes);
            0
        }
        Err(e) => {
            ylog!(env, Err, "ltdc: IDR {:#010x} is no LTDC ltdc.c knows (clock, reset, RIF?)", m.read32(c.base));
            e
        }
    }
}

pub(crate) fn ltdc_configure_with(
    m: &impl Mmio,
    c: &YdMp25LtdcConfig,
    d: &mut YdMp25LtdcData,
    mode: &YdMp25Mode,
    fb: u64,
    pitch: u32,
    format: u32,
) -> i32 {
    let l = match ltdc_of(m, c) {
        Ok(l) => l,
        Err(e) => return e,
    };
    let fmt = match ltdc_format(format) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let out = ltdc::Output {
        pixel_data_negedge: c.flags & YD_MP25_LTDC_PIXCLK_FALLING != 0,
        background: c.background,
        burst_bytes: (c.flags & YD_MP25_LTDC_BURSTLEN != 0).then_some(c.burst_bytes),
    };
    match l.configure(m, &mode.video(), &out, &ltdc::Layer { phys: fb, pitch, format: fmt }) {
        Ok(k) => {
            d.pitch = k.pitch;
            d.line_length = k.line_length;
            d.width = mode.hactive;
            d.height = mode.vactive;
            d.format = format;
            d.fb = fb;
            d.configured = 1;
            0
        }
        Err(e) => e,
    }
}

pub(crate) fn ltdc_enable_with(m: &impl Mmio, env: &impl Env, c: &YdMp25LtdcConfig, d: &mut YdMp25LtdcData) -> i32 {
    if d.configured == 0 {
        return -EINVAL;
    }
    let r = ltdc_of(m, c).and_then(|l| l.enable(m, env));
    d.on = r.is_ok() as u8;
    rc(r)
}

pub(crate) fn ltdc_disable_with(m: &impl Mmio, env: &impl Env, c: &YdMp25LtdcConfig, d: &mut YdMp25LtdcData) -> i32 {
    d.on = 0;
    rc(ltdc_of(m, c).and_then(|l| l.disable(m, env)))
}

pub(crate) fn ltdc_set_fb_with(
    m: &impl Mmio,
    env: &impl Env,
    c: &YdMp25LtdcConfig,
    d: &mut YdMp25LtdcData,
    fb: u64,
) -> i32 {
    if d.on == 0 {
        return -ENODEV;
    }
    let r = ltdc_of(m, c).and_then(|l| l.set_fb(m, env, fb));
    if r.is_ok() {
        d.fb = fb;
    }
    rc(r)
}

/// Read the LTDC's version, bus width and layer count (`ltdc_get_caps`).
/// 0, or `-ENODEV` when `IDR` holds no version ltdc.c knows (unclocked, in
/// reset, or refused by the RIF).
///
/// # Safety
/// `cfg`, `data`, `env` valid for the call; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_probe(
    cfg: *const YdMp25LtdcConfig,
    data: *mut YdMp25LtdcData,
    env: *const YdEnv,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { ltdc_probe_with(&port(), &*env, &*cfg, &mut *data) }
}

/// Program the timings, polarities, background and layer 1 (`fb_phys`,
/// `pitch` bytes, 0 = width x bytes per pixel, `format` a
/// `YD_MP25_LTDC_FMT_*`), everything but the enables. Nothing is written on
/// `-EINVAL` (a mode past the LTDC's fields or its 150 MHz pad limit, a
/// pitch or address not a multiple of the bus width, an address above 4
/// GiB) or `-ENOTSUP` (a format the block is not driven with).
///
/// # Safety
/// As [`yd_mp25_ltdc_probe`]; `mode` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_configure(
    cfg: *const YdMp25LtdcConfig,
    data: *mut YdMp25LtdcData,
    mode: *const YdMp25Mode,
    fb_phys: u64,
    pitch: u32,
    format: u32,
) -> i32 {
    if cfg.is_null() || data.is_null() || mode.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { ltdc_configure_with(&port(), &*cfg, &mut *data, &*mode, fb_phys, pitch, format) }
}

/// Error interrupts unmasked, layer on, a vertical-blanking reload, `FUT`,
/// `LTDCEN`; waits up to two frames for the reload. `-ETIMEDOUT`: no frame
/// boundary came, the pixel clock (`ck_ker_ltdc`) is not running.
/// `-EINVAL` before a configure.
///
/// # Safety
/// As [`yd_mp25_ltdc_probe`].
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_enable(
    cfg: *const YdMp25LtdcConfig,
    data: *mut YdMp25LtdcData,
    env: *const YdEnv,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { ltdc_enable_with(&port(), &*env, &*cfg, &mut *data) }
}

/// Layer off, errors masked, an immediate reload, `LTDCEN` cleared.
///
/// # Safety
/// As [`yd_mp25_ltdc_probe`].
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_disable(
    cfg: *const YdMp25LtdcConfig,
    data: *mut YdMp25LtdcData,
    env: *const YdEnv,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { ltdc_disable_with(&port(), &*env, &*cfg, &mut *data) }
}

/// Point the running layer at `fb_phys` (same pitch and format) and wait
/// for the vertical-blanking reload. `-ENODEV` while not enabled.
///
/// # Safety
/// As [`yd_mp25_ltdc_probe`].
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_set_fb(
    cfg: *const YdMp25LtdcConfig,
    data: *mut YdMp25LtdcData,
    env: *const YdEnv,
    fb_phys: u64,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { ltdc_set_fb_with(&port(), &*env, &*cfg, &mut *data, fb_phys) }
}

/// Wait for one line interrupt at the end of the active area (polled,
/// `LIF` cleared first). `-ETIMEDOUT` after two frames.
///
/// # Safety
/// As [`yd_mp25_ltdc_probe`].
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_wait_frame(cfg: *const YdMp25LtdcConfig, env: *const YdEnv) -> i32 {
    if cfg.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (c, env, hw) = unsafe { (&*cfg, &*env, port()) };
    rc(ltdc_of(&hw, c).and_then(|l| l.wait_frame(&hw, env)))
}

/// `ISR`, read and written back to `ICR` (`ltdc_irq`); kept in `data->isr`.
/// Bits: 0 line, 1 FIFO underrun warning, 2 transfer error, 6 FIFO
/// underrun error. 0 when the block does not probe.
///
/// # Safety
/// `cfg`, `data` valid; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_ltdc_isr(cfg: *const YdMp25LtdcConfig, data: *mut YdMp25LtdcData) -> u32 {
    if cfg.is_null() || data.is_null() {
        return 0;
    }
    // SAFETY: the caller's contract.
    let (c, d, hw) = unsafe { (&*cfg, &mut *data, port()) };
    let s = match ltdc_of(&hw, c) {
        Ok(l) => {
            let s = l.status(&hw);
            l.clear_status(&hw, s);
            s
        }
        Err(_) => 0,
    };
    d.isr = s;
    s
}

/// Eight colour bars (ip::stm32_ltdc::fill_color_bars: white, yellow, cyan,
/// green, magenta, red, blue, black, 0xFFRRGGBB) into a buffer of `len_px`
/// 32-bit pixels, `stride_px` pixels per line. The CPU's writes still need a
/// cache clean before the LTDC reads them.
///
/// # Safety
/// `buf` writable for `len_px` pixels.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_fill_color_bars(
    buf: *mut u32,
    len_px: usize,
    width: u32,
    height: u32,
    stride_px: u32,
) -> i32 {
    if buf.is_null() || len_px == 0 {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let b = unsafe { core::slice::from_raw_parts_mut(buf, len_px) };
    ltdc::fill_color_bars(b, width as usize, height as usize, stride_px as usize);
    0
}

// ---- DSI host and PHY -------------------------------------------------------------

/// `format` of [`yd_mp25_dsi_attach`], Zephyr's `MIPI_DSI_PIXFMT_*` order.
pub const YD_MP25_DSI_FMT_RGB888: u32 = 0;
pub const YD_MP25_DSI_FMT_RGB666: u32 = 1;
pub const YD_MP25_DSI_FMT_RGB666_PACKED: u32 = 2;
pub const YD_MP25_DSI_FMT_RGB565: u32 = 3;

/// `mode_flags` of [`yd_mp25_dsi_attach`]: Linux's `MIPI_DSI_*` bits
/// (ip::dw_mipi_dsi::flags). Bit 9 is "no EoT packet", as in Linux.
pub const YD_MP25_DSI_MODE_VIDEO: u32 = dsi::flags::MODE_VIDEO;
pub const YD_MP25_DSI_MODE_VIDEO_BURST: u32 = dsi::flags::MODE_VIDEO_BURST;
pub const YD_MP25_DSI_MODE_VIDEO_SYNC_PULSE: u32 = dsi::flags::MODE_VIDEO_SYNC_PULSE;
pub const YD_MP25_DSI_MODE_VIDEO_HSE: u32 = dsi::flags::MODE_VIDEO_HSE;
pub const YD_MP25_DSI_MODE_NO_EOT_PACKET: u32 = dsi::flags::MODE_NO_EOT_PACKET;
pub const YD_MP25_DSI_CLOCK_NON_CONTINUOUS: u32 = dsi::flags::CLOCK_NON_CONTINUOUS;
/// What the ADV7535 asks for (adv7533.c).
pub const YD_MP25_DSI_MODE_ADV7535: u32 = dsi::flags::ADV7535;

/// `struct yd_mp25_dsi_config`: from the devicetree.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YdMp25DsiConfig {
    /// The mapped region: the DesignWare host (0x000) and ST's wrapper
    /// (0x400), one `reg` of 0x800.
    pub base: usize,
    /// `data-lanes`, used when attach passes 0.
    pub lanes: u32,
    /// `pll-ref-frequency`: `CK_KER_DSIPHY`, Hz (17-32 MHz; 27 on the DK).
    pub pll_ref_hz: u32,
    /// `burst-clock-frequency`: the lane rate, Hz; 0 derives it from the
    /// mode as ST's glue does (pixel x bpp / lanes, +20 % in burst).
    pub lane_rate_hz: u32,
    /// `pixel-clock-frequency`: the LTDC's real pixel clock, Hz; 0 takes the
    /// mode's.
    pub pixel_clock_hz: u32,
}

/// `struct yd_mp25_dsi_data`: per instance.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp25DsiData {
    /// The mode of the last enable.
    pub mode: YdMp25Mode,
    /// From attach.
    pub lanes: u32,
    pub format: u32,
    pub mode_flags: u32,
    /// What the last enable programmed: the PLL's lane rate (kbit/s), the
    /// rate the host's timings used (Mbit/s, truncated as Linux's
    /// `get_lane_mbps`), the PLL dividers, the escape clock divider and the
    /// line time in lane byte clocks.
    pub lane_kbps: u32,
    pub lane_mbps: u32,
    pub pll_idf: u32,
    pub pll_ndiv: u32,
    pub pll_odf: u32,
    pub esc_div: u32,
    pub hline: u32,
    /// The step the last enable failed in (ip::dw_mipi_dsi::step, 0 = none)
    /// and its errno.
    pub failed_step: u32,
    pub failed_errno: i32,
    /// 1 after attach.
    pub attached: u8,
    /// 1 while streaming video.
    pub on: u8,
    pub reserved: [u8; 2],
}

// The values the header spells out.
const _: () = assert!(YD_MP25_DSI_MODE_VIDEO == 1 << 0 && YD_MP25_DSI_MODE_VIDEO_BURST == 1 << 1);
const _: () = assert!(YD_MP25_DSI_MODE_VIDEO_SYNC_PULSE == 1 << 2 && YD_MP25_DSI_MODE_VIDEO_HSE == 1 << 4);
const _: () = assert!(YD_MP25_DSI_MODE_NO_EOT_PACKET == 1 << 9 && YD_MP25_DSI_CLOCK_NON_CONTINUOUS == 1 << 10);
const _: () = assert!(YD_MP25_DSI_MODE_ADV7535 == 0x215);
const _: () = assert!(dsi::step::CHECK == 1 && dsi::step::PHY_RATE == 2 && dsi::step::PHY_INIT == 3);
const _: () = assert!(dsi::step::PHY_LOCK == 4 && dsi::step::STOP_STATE == 5);
const _: () = assert!(adv::BUS_MAIN == 0 && adv::BUS_DSI_CEC == 1 && adv::BUS_EDID == 2 && adv::BUS_PACKET == 3);
const _: () = assert!(adv::EDID_BLOCK == 128);
const _: () = assert!(size_of::<YdMp25DsiConfig>() == 24);
const _: () = assert!(offset_of!(YdMp25DsiConfig, lanes) == 8);
const _: () = assert!(offset_of!(YdMp25DsiConfig, pll_ref_hz) == 12);
const _: () = assert!(offset_of!(YdMp25DsiConfig, lane_rate_hz) == 16);
const _: () = assert!(offset_of!(YdMp25DsiConfig, pixel_clock_hz) == 20);
const _: () = assert!(size_of::<YdMp25DsiData>() == 92);
const _: () = assert!(offset_of!(YdMp25DsiData, lanes) == 40);
const _: () = assert!(offset_of!(YdMp25DsiData, mode_flags) == 48);
const _: () = assert!(offset_of!(YdMp25DsiData, lane_kbps) == 52);
const _: () = assert!(offset_of!(YdMp25DsiData, pll_odf) == 68);
const _: () = assert!(offset_of!(YdMp25DsiData, hline) == 76);
const _: () = assert!(offset_of!(YdMp25DsiData, failed_errno) == 84);
const _: () = assert!(offset_of!(YdMp25DsiData, attached) == 88);
const _: () = assert!(offset_of!(YdMp25DsiData, on) == 89);

fn dsi_format(f: u32) -> Result<(dsi::Format, MipiFormat), i32> {
    match f {
        YD_MP25_DSI_FMT_RGB888 => Ok((dsi::Format::Rgb888, MipiFormat::Rgb888)),
        YD_MP25_DSI_FMT_RGB666 => Ok((dsi::Format::Rgb666, MipiFormat::Rgb666)),
        YD_MP25_DSI_FMT_RGB666_PACKED => Ok((dsi::Format::Rgb666Packed, MipiFormat::Rgb666Packed)),
        YD_MP25_DSI_FMT_RGB565 => Ok((dsi::Format::Rgb565, MipiFormat::Rgb565)),
        _ => Err(-EINVAL),
    }
}

/// ST's 1.41 PHY ops (`dw_mipi_dsi_stm_phy_141_ops`) behind ip::dw_mipi_dsi's
/// [`Phy`]: the lane rate and PLL search of `get_lane_mbps`, the HS
/// transition table of `get_timing`, the wrapper and PLL bring-up of `init`,
/// `WCR.DSIEN` for power on/off. All in the same register region as the host.
struct Mp25Phy {
    base: usize,
    ref_hz: u32,
    /// The requested lane rate, kbit/s, 0 = from the mode.
    want_kbps: u32,
    format: MipiFormat,
    /// What [`Phy::lane_mbps`] settled on.
    lane_kbps: u32,
    pll: Option<Pll>,
}

impl Phy for Mp25Phy {
    fn lane_mbps(&mut self, _m: &impl Mmio, link: &Link) -> Result<u32, i32> {
        let bpp = link.format.bpp();
        let pixel_khz = link.mode.pixel_clock_hz / 1000;
        let burst = link.flags & dsi::flags::MODE_VIDEO_BURST != 0;
        let kbps =
            if self.want_kbps != 0 { self.want_kbps } else { dphy::lane_kbps_for(pixel_khz, bpp, link.lanes, burst) };
        if !(dphy::LANE_MIN_KBPS..=dphy::LANE_MAX_KBPS).contains(&kbps) {
            return Err(-EINVAL);
        }
        let pll = dphy::search(self.ref_hz / 1000, kbps / 2)?;
        // mode_valid (dw_mipi_dsi-stm.c:1177-1186): in non-burst mode the
        // link must carry the mode's pixel clock
        if self.want_kbps == 0 && !burst && !dphy::pixel_clock_within_tolerance(&pll, link.lanes, bpp, pixel_khz) {
            return Err(-EINVAL);
        }
        self.lane_kbps = kbps;
        self.pll = Some(pll);
        Ok(pll.lane_mbps())
    }

    fn timing(&self, lane_mbps: u32) -> Result<DphyTiming, i32> {
        let t = dphy::hs_transition(lane_mbps);
        Ok(DphyTiming {
            data_hs2lp: u32::from(t.data_hs2lp),
            data_lp2hs: u32::from(t.data_lp2hs),
            clk_hs2lp: u32::from(t.clk_hs2lp),
            clk_lp2hs: u32::from(t.clk_lp2hs),
        })
    }

    fn init(&mut self, m: &impl Mmio, env: &impl Env) -> Result<(), i32> {
        let pll = dphy::init(m, env, self.base, self.ref_hz, self.lane_kbps, self.format.coding())?;
        self.pll = Some(pll);
        Ok(())
    }

    fn power_on(&mut self, m: &impl Mmio) {
        dphy::power_on(m, self.base);
    }

    fn power_off(&mut self, m: &impl Mmio) {
        dphy::power_off(m, self.base);
    }
}

fn dsi_link(c: &YdMp25DsiConfig, d: &YdMp25DsiData) -> Result<(Link, Mp25Phy), i32> {
    let (format, mipi) = dsi_format(d.format)?;
    let mut mode = d.mode.video();
    if c.pixel_clock_hz != 0 {
        mode.pixel_clock_hz = c.pixel_clock_hz;
    }
    let link = Link { mode, lanes: d.lanes, channel: 0, format, flags: d.mode_flags };
    let phy = Mp25Phy {
        base: c.base,
        ref_hz: c.pll_ref_hz,
        want_kbps: c.lane_rate_hz / 1000,
        format: mipi,
        lane_kbps: 0,
        pll: None,
    };
    Ok((link, phy))
}

pub(crate) fn dsi_probe_with(m: &impl Mmio, env: &impl Env, c: &YdMp25DsiConfig) -> i32 {
    rc(dphy::probe(m, env, c.base).and_then(|()| dsi::probe(m, c.base).map(|_| ())))
}

pub(crate) fn dsi_attach_with(c: &YdMp25DsiConfig, d: &mut YdMp25DsiData, lanes: u32, format: u32, flags: u32) -> i32 {
    let lanes = if lanes == 0 { c.lanes } else { lanes };
    if !(1..=4).contains(&lanes) || dsi_format(format).is_err() || flags & YD_MP25_DSI_MODE_VIDEO == 0 {
        return -EINVAL;
    }
    if d.on != 0 {
        return -EBUSY;
    }
    d.lanes = lanes;
    d.format = format;
    d.mode_flags = flags;
    d.attached = 1;
    0
}

pub(crate) fn dsi_enable_with(
    m: &impl Mmio,
    env: &impl Env,
    c: &YdMp25DsiConfig,
    d: &mut YdMp25DsiData,
    mode: &YdMp25Mode,
) -> i32 {
    if d.attached == 0 {
        return -EINVAL;
    }
    if d.on != 0 {
        return -EBUSY;
    }
    d.mode = *mode;
    d.failed_step = 0;
    d.failed_errno = 0;
    let (link, mut phy) = match dsi_link(c, d) {
        Ok(x) => x,
        Err(e) => return e,
    };
    match dsi::attach(m, env, c.base, &mut phy, &link) {
        Ok(p) => {
            dsi::video_on(m, c.base, &link);
            let pll = phy.pll.unwrap_or(Pll { ref_khz: 0, idf: 0, ndiv: 0, odf: 0, band: 0 });
            d.lane_kbps = pll.lane_kbps();
            d.lane_mbps = p.lane_mbps;
            d.pll_idf = pll.idf;
            d.pll_ndiv = pll.ndiv;
            d.pll_odf = pll.odf;
            d.esc_div = p.esc_div;
            d.hline = p.hline;
            d.on = 1;
            0
        }
        Err(f) => {
            d.failed_step = f.step;
            d.failed_errno = f.errno;
            ylog!(env, Err, "mp25 dsi: enable failed in step {} ({})", f.step, f.errno);
            f.errno
        }
    }
}

pub(crate) fn dsi_disable_with(m: &impl Mmio, c: &YdMp25DsiConfig, d: &mut YdMp25DsiData) -> i32 {
    if d.on == 0 {
        return 0;
    }
    d.on = 0;
    match dsi_link(c, d) {
        Ok((link, mut phy)) => {
            dsi::disable(m, c.base, &mut phy, &link);
            0
        }
        Err(e) => e,
    }
}

/// The block's version is the MP25's DesignWare 1.41. 0, or `-ENODEV`
/// (unclocked, in reset, refused by the RIF).
///
/// # Safety
/// `cfg`, `env` valid; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_dsi_probe(cfg: *const YdMp25DsiConfig, env: *const YdEnv) -> i32 {
    if cfg.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { dsi_probe_with(&port(), &*env, &*cfg) }
}

/// The device on the link (Zephyr's `mipi_dsi_attach`): `lanes` (0 = the
/// config's), `format` a `YD_MP25_DSI_FMT_*`, `mode_flags` the
/// `YD_MP25_DSI_*` bits (video mode required). No register is touched.
/// `-EBUSY` while streaming.
///
/// # Safety
/// `cfg`, `data` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_dsi_attach(
    cfg: *const YdMp25DsiConfig,
    data: *mut YdMp25DsiData,
    lanes: u32,
    format: u32,
    mode_flags: u32,
) -> i32 {
    if cfg.is_null() || data.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { dsi_attach_with(&*cfg, &mut *data, lanes, format, mode_flags) }
}

/// dw-mipi-dsi.c's `mode_set` then `atomic_enable` for `mode`: the host's
/// DPI, packet, video and timing registers, the PHY timings, ST's wrapper
/// and PLL, `PHY_RSTZ`, the lock and clock-lane stop waits, two frames,
/// command mode, `WCR.DSIEN`, then video mode with the clock lane in HS.
/// On a failure `data->failed_step` / `failed_errno` say where. `-EINVAL`
/// before attach or for a rate outside the PHY.
///
/// # Safety
/// `cfg`, `data`, `env`, `mode` valid; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_dsi_enable(
    cfg: *const YdMp25DsiConfig,
    data: *mut YdMp25DsiData,
    env: *const YdEnv,
    mode: *const YdMp25Mode,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() || mode.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { dsi_enable_with(&port(), &*env, &*cfg, &mut *data, &*mode) }
}

/// Command mode, the PLL and `DSIEN` off, host and PHY held in reset
/// (`post_atomic_disable`). 0 when not streaming.
///
/// # Safety
/// `cfg`, `data` valid; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_dsi_disable(cfg: *const YdMp25DsiConfig, data: *mut YdMp25DsiData) -> i32 {
    if cfg.is_null() || data.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    unsafe { dsi_disable_with(&port(), &*cfg, &mut *data) }
}

// ---- ADV7535 ----------------------------------------------------------------------

/// The bus ids the ADV7535 code passes to the env's I2C calls; the shim maps
/// each to its 7-bit address (ip::adv7535 `BUS_*`).
pub const YD_MP25_ADV7535_BUS_MAIN: u32 = adv::BUS_MAIN;
pub const YD_MP25_ADV7535_BUS_DSI_CEC: u32 = adv::BUS_DSI_CEC;
pub const YD_MP25_ADV7535_BUS_EDID: u32 = adv::BUS_EDID;
pub const YD_MP25_ADV7535_BUS_PACKET: u32 = adv::BUS_PACKET;

/// `flags` of [`YdMp25Adv7535Config`]: `adi,disable-timing-generator`.
pub const YD_MP25_ADV7535_NO_TIMING_GEN: u8 = 1 << 0;

/// [`yd_mp25_choose_mode`]'s positive results.
pub const YD_MP25_MODE_FROM_EDID: i32 = 0;
pub const YD_MP25_MODE_FALLBACK_NO_EDID: i32 = 1;
pub const YD_MP25_MODE_FALLBACK_LIMIT: i32 = 2;

/// `struct yd_mp25_adv7535_config`: from the devicetree. The secondary
/// addresses are the ones written into the chip (main map 0x43, 0x45,
/// 0xE1); the shim's bus-id map must agree with them.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YdMp25Adv7535Config {
    /// `adi,dsi-lanes`, 2..=4; must equal the DSI host's lanes.
    pub lanes: u8,
    /// `YD_MP25_ADV7535_*`.
    pub flags: u8,
    /// `edid-reg`, `packet-reg`, `cec-reg` (7-bit).
    pub edid_addr: u8,
    pub packet_addr: u8,
    pub cec_addr: u8,
    pub reserved: [u8; 3],
}

/// `struct yd_mp25_adv7535_data`: per instance.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp25Adv7535Data {
    /// `CHIP_REVISION` (0x14 on the DK's part), from init.
    pub revision: u8,
    /// 1 when the last HPD read saw a sink.
    pub hpd: u8,
    /// 1 after a successful start, until stop.
    pub started: u8,
    /// 1 after a successful init.
    pub probed: u8,
}

const _: () = assert!(size_of::<YdMp25Adv7535Config>() == 8);
const _: () = assert!(offset_of!(YdMp25Adv7535Config, flags) == 1);
const _: () = assert!(offset_of!(YdMp25Adv7535Config, edid_addr) == 2);
const _: () = assert!(offset_of!(YdMp25Adv7535Config, packet_addr) == 3);
const _: () = assert!(offset_of!(YdMp25Adv7535Config, cec_addr) == 4);
const _: () = assert!(size_of::<YdMp25Adv7535Data>() == 4);
const _: () = assert!(offset_of!(YdMp25Adv7535Data, started) == 2);

fn adv_config(c: &YdMp25Adv7535Config) -> adv::Config {
    let mut a = adv::Config::new(c.lanes);
    a.use_timing_gen = c.flags & YD_MP25_ADV7535_NO_TIMING_GEN == 0;
    a.edid_addr = c.edid_addr;
    a.packet_addr = c.packet_addr;
    a.cec_addr = c.cec_addr;
    a
}

/// The LTDC's fields and pad limit (ip::stm32_ltdc::Ltdc::configure on the
/// MP25) and the ADV7535's clock and lane limits.
fn drivable(m: &VideoMode, lanes: u8) -> bool {
    m.check().is_ok()
        && m.hactive <= 0xfff
        && m.htotal() <= 0x1000
        && m.vtotal() <= 0x1000
        && m.pixel_clock_hz <= 150_000_000
        && (2..=4).contains(&lanes)
        && adv::mode_valid(m, lanes)
}

/// The sink for a chosen mode: the CTA-861 VIC when the mode is VIC 4 or 16,
/// HDMI only with an EDID that has an extension block (a base block alone
/// cannot say HDMI; Linux reads `is_hdmi` from the CTA extension, which
/// crate::video does not parse), else DVI.
fn sink_for(m: &VideoMode, edid_ext: bool) -> YdMp25HdmiSink {
    let vic = if *m == HD720_60 {
        4
    } else if *m == FHD1080_60 {
        16
    } else {
        0
    };
    let mhz = m.refresh_mhz(m.pixel_clock_hz);
    YdMp25HdmiSink {
        hdmi: (edid_ext && vic != 0) as u8,
        vic,
        aspect_16_9: (vic != 0) as u8,
        reserved: 0,
        refresh_hz: (mhz + 500) / 1000,
    }
}

/// The mode for an EDID (the preferred timing) or 720p60. Returns
/// `YD_MP25_MODE_*` and the mode and sink.
pub(crate) fn choose_mode(edid: &[u8], lanes: u8) -> (i32, VideoMode, YdMp25HdmiSink) {
    match video::edid_preferred(edid) {
        Ok(m) if drivable(&m, lanes) => (YD_MP25_MODE_FROM_EDID, m, sink_for(&m, edid[126] != 0)),
        Ok(_) => (YD_MP25_MODE_FALLBACK_LIMIT, HD720_60, sink_for(&HD720_60, edid[126] != 0)),
        Err(_) => (YD_MP25_MODE_FALLBACK_NO_EDID, HD720_60, sink_for(&HD720_60, false)),
    }
}

/// adv7511_probe for the ADV7535: `CHIP_REVISION`, the ADV7533 patch, the
/// secondary addresses, the CEC/DSI patch, powered down. `-EIO` when the
/// main map does not answer (no chip, still in reset, no 1.8/3.3 V),
/// `-EINVAL` for lanes outside 2..=4.
///
/// # Safety
/// `cfg`, `data`, `env` valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_adv7535_init(
    cfg: *const YdMp25Adv7535Config,
    data: *mut YdMp25Adv7535Data,
    env: *const YdEnv,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (c, d, env) = unsafe { (&*cfg, &mut *data, &*env) };
    *d = YdMp25Adv7535Data::default();
    match Bridge::new(env).init(&adv_config(c)) {
        Ok(f) => {
            d.revision = f.revision;
            d.probed = 1;
            0
        }
        Err(e) => e,
    }
}

/// The plug state (`STATUS` 0x42 bit 6; HPD override re-armed): 1 plugged,
/// 0 not, or a negative errno.
///
/// # Safety
/// `data`, `env` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_adv7535_hpd(data: *mut YdMp25Adv7535Data, env: *const YdEnv) -> i32 {
    if data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (d, env) = unsafe { (&mut *data, &*env) };
    match Bridge::new(env).hpd() {
        Ok(up) => {
            d.hpd = up as u8;
            i32::from(up)
        }
        Err(e) => e,
    }
}

/// EDID block `block` (128 bytes into `buf`), the part powered for the read
/// and down again. `-ENODEV` with HPD low, `-EIO` / `-ETIMEDOUT` from the
/// DDC, `-EBUSY` once started (the read would power the running part down:
/// read before [`yd_mp25_adv7535_start`]).
///
/// # Safety
/// `data`, `env` valid; `buf` writable for 128 bytes.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_adv7535_read_edid(
    data: *mut YdMp25Adv7535Data,
    env: *const YdEnv,
    block: u32,
    buf: *mut u8,
) -> i32 {
    if data.is_null() || env.is_null() || buf.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (d, env, out) = unsafe { (&mut *data, &*env, &mut *(buf as *mut [u8; adv::EDID_BLOCK])) };
    if d.started != 0 {
        return -EBUSY;
    }
    let r = Bridge::new(env).read_edid_block(block, out);
    if r == Err(-ENODEV) {
        d.hpd = 0;
    }
    rc(r)
}

/// The mode to drive: the EDID's preferred timing when it is good (VESA
/// header and checksum, a usable first descriptor) and within the LTDC (150
/// MHz, 4096 total) and the ADV7535 (148.5 MHz, 891 Mbit/s a lane on
/// `cfg->lanes`), else CTA-861 VIC 4, 1280x720@60. `edid` may be NULL (no
/// EDID read). Returns `YD_MP25_MODE_FROM_EDID`, `_FALLBACK_NO_EDID` or
/// `_FALLBACK_LIMIT`; `-EINVAL` for a NULL `cfg`, `mode` or `sink`. The sink
/// is HDMI only for VIC 4 or 16 with an EDID extension block present, else
/// DVI; the caller may override it.
///
/// # Safety
/// `edid` readable for `len` bytes when not NULL; the others valid.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_choose_mode(
    cfg: *const YdMp25Adv7535Config,
    edid: *const u8,
    len: usize,
    mode: *mut YdMp25Mode,
    sink: *mut YdMp25HdmiSink,
) -> i32 {
    if cfg.is_null() || mode.is_null() || sink.is_null() {
        return -EINVAL;
    }
    let e: &[u8] = if edid.is_null() {
        &[]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(edid, len) }
    };
    // SAFETY: the caller's contract.
    let (c, mode, sink) = unsafe { (&*cfg, &mut *mode, &mut *sink) };
    let (r, m, s) = choose_mode(e, c.lanes);
    *mode = YdMp25Mode::from_video(&m);
    *sink = s;
    r
}

/// adv7511_bridge_atomic_enable: power up with the DSI receiver (lanes,
/// timing generator), RGB without CSC, HDMI or DVI, the mode, the AVI
/// infoframe. The DSI host must already stream video with the clock lane
/// in HS ([`yd_mp25_dsi_enable`] returned 0). `-EINVAL` for a mode the part
/// cannot take on its lanes.
///
/// # Safety
/// `cfg`, `data`, `env`, `mode`, `sink` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_adv7535_start(
    cfg: *const YdMp25Adv7535Config,
    data: *mut YdMp25Adv7535Data,
    env: *const YdEnv,
    mode: *const YdMp25Mode,
    sink: *const YdMp25HdmiSink,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() || mode.is_null() || sink.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (c, d, env, m, s) = unsafe { (&*cfg, &mut *data, &*env, &*mode, &*sink) };
    let sink = Sink { hdmi: s.hdmi != 0, vic: s.vic, aspect_16_9: s.aspect_16_9 != 0, refresh_hz: s.refresh_hz };
    d.started = 0;
    let r = Bridge::new(env).start(&adv_config(c), &m.video(), &sink);
    d.started = r.is_ok() as u8;
    rc(r)
}

/// Power the part down (`adv7511_power_off` with the DSI receiver off).
///
/// # Safety
/// `data`, `env` valid.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_adv7535_stop(data: *mut YdMp25Adv7535Data, env: *const YdEnv) -> i32 {
    if data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (d, env) = unsafe { (&mut *data, &*env) };
    d.started = 0;
    rc(Bridge::new(env).power_down())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Level;
    use crate::errno::{EIO, ETIMEDOUT};
    use crate::ip::dw_mipi_dsi::regs as dr;
    use crate::ip::stm32_dsi_phy::regs as pr;
    use crate::ip::stm32_i2c;
    use crate::model_mp25_display::Mp25Display;
    use crate::model_mp25_i2c::{self as i2cm, Adv7535Config, Adv7535Handle, EdidMode, Mp25I2c};
    use core::ffi::c_void;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::string::String;
    use std::vec::Vec;

    const LTDC: usize = 0x4801_0000;
    const DSI: usize = 0x4800_0000;
    const FB: u64 = 0x9000_0000;
    /// The DK's PHY reference, FLEXGEN_28 at 27 MHz (stm32_dsi_phy.rs).
    const REF_HZ: u32 = 27_000_000;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Ev {
        Ltdc(u32, u32),
        Dsi(u32, u32),
        /// An I2C write: 7-bit address, register, value.
        I2c(u8, u8, u8),
    }

    /// The LTDC (as ip::stm32_ltdc's tests model it: reload bits clear and
    /// `LIF` reads set while `LTDCEN` is set) and the DSI region (a register
    /// file; `PHY_STATUS`/`PSR` report lock and the clock lane's stop state
    /// once the PHY control has `PWEN | DEN | CKEN`), on one ordered log.
    struct Soc {
        regs: RefCell<BTreeMap<usize, u32>>,
        log: RefCell<Vec<Ev>>,
    }

    impl Soc {
        fn new() -> Soc {
            let s = Soc { regs: RefCell::default(), log: RefCell::default() };
            s.poke(LTDC + ltdc::IDR as usize, ltdc::HWVER_40101);
            s.poke(LTDC + ltdc::GC2R as usize, 3 << 4); // 64-bit bus
            s.poke(LTDC + ltdc::LCR as usize, 3);
            s.poke(DSI + dr::VERSION as usize, dr::HWVER_141 | 0x2a);
            s
        }
        fn poke(&self, a: usize, v: u32) {
            self.regs.borrow_mut().insert(a, v);
        }
        fn peek(&self, a: usize) -> u32 {
            self.regs.borrow().get(&a).copied().unwrap_or(0)
        }
        fn ltdc_on(&self) -> bool {
            self.peek(LTDC + ltdc::GCR as usize) & ltdc::GCR_LTDCEN != 0
        }
    }

    impl Mmio for Soc {
        fn read32(&self, a: usize) -> u32 {
            let mut v = self.peek(a);
            if (LTDC..LTDC + 0x1000).contains(&a) {
                let off = (a - LTDC) as u32;
                if off == 0x108 && self.ltdc_on() && v & ltdc::RCR_VBR != 0 {
                    v &= !ltdc::RCR_VBR;
                    self.poke(a, v);
                }
                if off == ltdc::ISR && self.ltdc_on() {
                    v |= ltdc::INT_LINE;
                }
            } else if a == DSI + dr::PHY_STATUS as usize {
                let ctl = self.peek(DSI + dr::PHY_RSTZ as usize);
                let up = pr::PCTLR_PWEN | pr::PCTLR_DEN | pr::PCTLR_CKEN;
                v = if ctl & up == up { dr::PHY_LOCK | dr::PHY_STOP_STATE_CLK_LANE } else { 0 };
            } else {
                assert!((DSI..DSI + 0x800).contains(&a), "read outside the blocks: {a:#x}");
            }
            v
        }
        fn write32(&self, a: usize, v: u32) {
            if (LTDC..LTDC + 0x1000).contains(&a) {
                let off = (a - LTDC) as u32;
                self.log.borrow_mut().push(Ev::Ltdc(off, v));
                if off == ltdc::ICR {
                    let isr = self.peek(LTDC + ltdc::ISR as usize) & !v;
                    self.poke(LTDC + ltdc::ISR as usize, isr);
                } else if off == 0x108 && v & ltdc::RCR_IMR != 0 {
                    self.poke(a, v & !ltdc::RCR_IMR);
                } else {
                    self.poke(a, v);
                }
            } else {
                assert!((DSI..DSI + 0x800).contains(&a), "write outside the blocks: {a:#x}");
                self.log.borrow_mut().push(Ev::Dsi((a - DSI) as u32, v));
                self.poke(a, v);
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    /// What the test's `struct yd_env` points at: the I2C2 model with the
    /// DK's ADV7535, the shim's bus-id map, the shared log, the log lines.
    struct Ctx<'a> {
        bus: Mp25I2c,
        soc: &'a Soc,
        lines: RefCell<Vec<(u32, String)>>,
        slept_ms: RefCell<u32>,
    }

    /// The shim's bus-id map, as the DK's devicetree has the addresses.
    fn addr(bus: u32) -> Option<u8> {
        match bus {
            adv::BUS_MAIN => Some(0x3d),
            adv::BUS_DSI_CEC => Some(0x3c),
            adv::BUS_EDID => Some(0x3f),
            adv::BUS_PACKET => Some(0x38),
            _ => None,
        }
    }

    fn ctx<'a>(p: *mut c_void) -> &'a Ctx<'a> {
        // SAFETY: every env below is made by `env()` over a live `Ctx`.
        unsafe { &*(p as *const Ctx) }
    }

    unsafe extern "C" fn busy(_: *mut c_void, _: u32) {}
    unsafe extern "C" fn sleep(p: *mut c_void, ms: u32) {
        *ctx(p).slept_ms.borrow_mut() += ms;
    }
    unsafe extern "C" fn log(p: *mut c_void, level: u32, buf: *const u8, len: usize) {
        // SAFETY: the Rust side passes a str of `len` bytes.
        let s = unsafe { std::str::from_utf8(core::slice::from_raw_parts(buf, len)).unwrap() };
        ctx(p).lines.borrow_mut().push((level, s.into()));
    }
    unsafe extern "C" fn i2c_w(p: *mut c_void, bus: u32, buf: *const u8, len: usize) -> i32 {
        let c = ctx(p);
        let Some(a) = addr(bus) else { return -EINVAL };
        // SAFETY: the Rust side passes a slice of `len` bytes.
        let b = unsafe { core::slice::from_raw_parts(buf, len) };
        if len == 2 {
            c.soc.log.borrow_mut().push(Ev::I2c(a, b[0], b[1]));
        }
        rc(stm32_i2c::write(&c.bus, i2cm::BASE, a, b))
    }
    unsafe extern "C" fn i2c_wr(p: *mut c_void, bus: u32, w: *const u8, wl: usize, r: *mut u8, rl: usize) -> i32 {
        let c = ctx(p);
        let Some(a) = addr(bus) else { return -EINVAL };
        // SAFETY: the Rust side passes slices of these lengths.
        let (w, r) = unsafe { (core::slice::from_raw_parts(w, wl), core::slice::from_raw_parts_mut(r, rl)) };
        rc(stm32_i2c::write_read(&c.bus, i2cm::BASE, a, w, r))
    }
    unsafe extern "C" fn gpio(_: *mut c_void, _: u32, _: u32) -> i32 {
        -ENOTSUP
    }

    fn env(c: &Ctx) -> YdEnv {
        YdEnv {
            ctx: c as *const Ctx as *mut c_void,
            busy_wait_us: busy,
            sleep_ms: sleep,
            log,
            i2c_write: i2c_w,
            i2c_write_read: i2c_wr,
            gpio_set: gpio,
        }
    }

    /// The DK's I2C2 with the ADV7535, the controller set up (the SoC glue's
    /// and the I2C driver's part), `DDC_STATUS` (0xc8) = 2 so the EDID map is
    /// read without the segment handshake the model does not answer.
    fn rig<'a>(soc: &'a Soc, cfg: Adv7535Config) -> (Ctx<'a>, Adv7535Handle) {
        let (bus, adv) = Mp25I2c::dk(cfg);
        stm32_i2c::init(&bus, i2cm::BASE, &stm32_i2c::Config::new(100_000_000, 400_000)).unwrap();
        if !adv.in_reset() {
            stm32_i2c::write(&bus, i2cm::BASE, 0x3d, &[adv::reg::DDC_STATUS, 2]).unwrap();
        }
        (Ctx { bus, soc, lines: RefCell::default(), slept_ms: RefCell::new(0) }, adv)
    }

    const LTDC_CFG: YdMp25LtdcConfig =
        YdMp25LtdcConfig { base: LTDC, flags: 0, background: 0, burst_bytes: 0, reserved: 0 };
    const DSI_CFG: YdMp25DsiConfig =
        YdMp25DsiConfig { base: DSI, lanes: 4, pll_ref_hz: REF_HZ, lane_rate_hz: 0, pixel_clock_hz: 0 };
    const ADV_CFG: YdMp25Adv7535Config = YdMp25Adv7535Config {
        lanes: 4,
        flags: 0,
        edid_addr: 0x3f,
        packet_addr: 0x38,
        cec_addr: 0x3c,
        reserved: [0; 3],
    };

    fn first(log: &[Ev], f: impl Fn(&Ev) -> bool) -> usize {
        log.iter().position(f).expect("event not in the log")
    }
    fn last(log: &[Ev], f: impl Fn(&Ev) -> bool) -> usize {
        log.iter().rposition(f).expect("event not in the log")
    }

    #[test]
    fn bring_up_720p60_in_order_through_all_three() {
        let soc = Soc::new();
        let (c, adv) = rig(&soc, Adv7535Config::default());
        let e = env(&c);
        let (mut ad, mut ld, mut dd) =
            (YdMp25Adv7535Data::default(), YdMp25LtdcData::default(), YdMp25DsiData::default());
        let mut edid = [0u8; 128];
        let (mut mode, mut sink) = (YdMp25Mode::default(), YdMp25HdmiSink::default());
        // SAFETY: every pointer is a local of this test.
        unsafe {
            // the bridge: probe, plug state, EDID, the mode
            assert_eq!(yd_mp25_adv7535_init(&ADV_CFG, &mut ad, &e), 0);
            assert_eq!((ad.revision, ad.probed), (0x14, 1));
            assert_eq!(yd_mp25_adv7535_hpd(&mut ad, &e), 1);
            assert_eq!(yd_mp25_adv7535_read_edid(&mut ad, &e, 0, edid.as_mut_ptr()), 0);
            assert_eq!(edid[..], i2cm::edid_block(EdidMode::P720)[..]);
            assert_eq!(yd_mp25_choose_mode(&ADV_CFG, edid.as_ptr(), 128, &mut mode, &mut sink), YD_MP25_MODE_FROM_EDID);
        }
        assert_eq!(mode, YdMp25Mode::from_video(&HD720_60));
        // the model's EDID has no extension block: DVI, VIC 4 kept
        assert_eq!(sink, YdMp25HdmiSink { hdmi: 0, vic: 4, aspect_16_9: 1, reserved: 0, refresh_hz: 60 });
        soc.log.borrow_mut().clear();

        // LTDC, then DSI, then the bridge
        assert_eq!(ltdc_probe_with(&soc, &e, &LTDC_CFG, &mut ld), 0);
        assert_eq!((ld.hwver, ld.bus_bytes, ld.layers), (ltdc::HWVER_40101, 8, 3));
        assert_eq!(ltdc_configure_with(&soc, &LTDC_CFG, &mut ld, &mode, FB, 0, YD_MP25_LTDC_FMT_XRGB8888), 0);
        assert_eq!((ld.pitch, ld.width, ld.height, ld.fb), (5120, 1280, 720, FB));
        assert_eq!(ltdc_enable_with(&soc, &e, &LTDC_CFG, &mut ld), 0);
        assert_eq!(ld.on, 1);
        assert_eq!(dsi_probe_with(&soc, &e, &DSI_CFG), 0);
        assert_eq!(dsi_attach_with(&DSI_CFG, &mut dd, 0, YD_MP25_DSI_FMT_RGB888, YD_MP25_DSI_MODE_ADV7535), 0);
        assert_eq!(dsi_enable_with(&soc, &e, &DSI_CFG, &mut dd, &mode), 0);
        // SAFETY: as above.
        unsafe {
            assert_eq!(yd_mp25_adv7535_start(&ADV_CFG, &mut ad, &e, &mode, &sink), 0);
        }
        assert_eq!(ad.started, 1);

        // the DSI result: 445.5 Mbit/s from 27 MHz exactly (idf 4, ndiv 66,
        // odf 2; docs/notes/stm32mp257f-dk-display-bringup.md's table), the
        // host's timings from Linux's truncated 445
        assert_eq!((dd.lane_kbps, dd.lane_mbps), (445_500, 445));
        assert_eq!((dd.pll_idf, dd.pll_ndiv, dd.pll_odf), (4, 66, 2));
        assert_eq!((dd.esc_div, dd.hline, dd.on, dd.failed_step), (3, 1238, 1, 0));

        let log = soc.log.borrow();
        let ltdc_en = first(&log, |x| matches!(x, Ev::Ltdc(o, v) if *o == ltdc::GCR && v & ltdc::GCR_LTDCEN != 0));
        let dsi_first = first(&log, |x| matches!(x, Ev::Dsi(..)));
        let video = first(&log, |x| matches!(x, Ev::Dsi(o, 0) if *o == dr::MODE_CFG));
        let dsien = first(&log, |x| matches!(x, Ev::Dsi(o, v) if *o == pr::WCR && v & pr::WCR_DSIEN != 0));
        let pwr_up = last(&log, |x| matches!(x, Ev::Dsi(o, 1) if *o == dr::PWR_UP));
        let adv_first = first(&log, |x| matches!(x, Ev::I2c(..)));
        let lanes = first(&log, |x| matches!(x, Ev::I2c(0x3c, 0x1c, _)));
        assert!(ltdc_en < dsi_first, "the LTDC streams before the DSI host starts");
        assert!(dsien < video && video < pwr_up, "PHY on, then video mode, then PWR_UP");
        assert!(pwr_up < adv_first, "the DSI host streams before the ADV7535's first write");
        assert_eq!(log[lanes], Ev::I2c(0x3c, 0x1c, 4 << 4));
        assert!(log[dsi_first..].iter().all(|x| !matches!(x, Ev::Ltdc(..))));
        drop(log);

        // the end state of each block
        assert_eq!(soc.peek(LTDC + 0x134), FB as u32); // L1CFBAR
        assert_eq!(soc.peek(DSI + dr::MODE_CFG as usize), 0);
        assert_eq!(soc.peek(DSI + dr::LPCLK_CTRL as usize) & dr::PHY_TXREQUESTCLKHS, dr::PHY_TXREQUESTCLKHS);
        assert_eq!(adv.reg(adv::reg::POWER) & adv::reg::POWER_DOWN, 0);
        assert_eq!(adv.reg(adv::reg::HDMI_CFG) & adv::reg::HDMI_CFG_MODE_MASK, adv::reg::HDMI_CFG_MODE_DVI);
        assert_eq!(adv.cec(0x28), (1650 >> 4) as u8); // timing generator htotal

        // a flip and a frame
        assert_eq!(ltdc_set_fb_with(&soc, &e, &LTDC_CFG, &mut ld, FB + 0x40_0000), 0);
        assert_eq!(soc.peek(LTDC + 0x134), (FB + 0x40_0000) as u32);

        // and down again, the bridge first
        // SAFETY: as above.
        unsafe {
            let mut buf = [0u8; 128];
            assert_eq!(yd_mp25_adv7535_read_edid(&mut ad, &e, 0, buf.as_mut_ptr()), -EBUSY);
            assert_eq!(yd_mp25_adv7535_stop(&mut ad, &e), 0);
        }
        assert_eq!(adv.reg(adv::reg::POWER) & adv::reg::POWER_DOWN, adv::reg::POWER_DOWN);
        assert_eq!(dsi_disable_with(&soc, &DSI_CFG, &mut dd), 0);
        assert_eq!(soc.peek(DSI + dr::PHY_RSTZ as usize), 0);
        assert_eq!(soc.peek(DSI + pr::WCR as usize) & pr::WCR_DSIEN, 0);
        assert_eq!(ltdc_disable_with(&soc, &e, &LTDC_CFG, &mut ld), 0);
        assert!(!soc.ltdc_on());
        assert_eq!((ad.started, dd.on, ld.on), (0, 0, 0));
    }

    #[test]
    fn three_lanes_and_1080p_from_the_edid() {
        // ST's 720p on three lanes (bring-up note): 594 Mbit/s, idf 3, ndiv 66
        let soc = Soc::new();
        let mut dd = YdMp25DsiData::default();
        let m = YdMp25Mode::from_video(&HD720_60);
        let e = crate::model::TestEnv::default();
        assert_eq!(dsi_attach_with(&DSI_CFG, &mut dd, 3, YD_MP25_DSI_FMT_RGB888, YD_MP25_DSI_MODE_ADV7535), 0);
        assert_eq!(dsi_enable_with(&soc, &e, &DSI_CFG, &mut dd, &m), 0);
        assert_eq!((dd.lanes, dd.lane_kbps, dd.pll_idf, dd.pll_ndiv, dd.pll_odf), (3, 594_000, 3, 66, 2));
        assert_eq!(soc.peek(DSI + dr::PHY_IF_CFG as usize) & 3, 2);

        // a 1080p60 sink is taken as its EDID says
        let (r, mode, sink) = choose_mode(&i2cm::edid_block(EdidMode::P1080), 4);
        assert_eq!((r, mode), (YD_MP25_MODE_FROM_EDID, FHD1080_60));
        assert_eq!((sink.vic, sink.refresh_hz), (16, 60));
        // with an extension block flagged it is an HDMI sink
        let mut ext = [0u8; 256];
        ext[..128].copy_from_slice(&i2cm::edid_block(EdidMode::P1080));
        ext[126] = 1;
        ext[127] = ext[127].wrapping_sub(1);
        assert_eq!(choose_mode(&ext, 4).2.hdmi, 1);
        // 1080p60 needs 891 Mbit/s a lane: not on two lanes
        let (r, mode, _) = choose_mode(&i2cm::edid_block(EdidMode::P1080), 2);
        assert_eq!((r, mode), (YD_MP25_MODE_FALLBACK_LIMIT, HD720_60));
    }

    #[test]
    fn a_bad_edid_falls_back_to_720p60() {
        let mut bad = i2cm::edid_block(EdidMode::P1080);
        bad[127] ^= 1; // checksum
        let (mut mode, mut sink) = (YdMp25Mode::default(), YdMp25HdmiSink::default());
        // SAFETY: locals.
        unsafe {
            assert_eq!(
                yd_mp25_choose_mode(&ADV_CFG, bad.as_ptr(), bad.len(), &mut mode, &mut sink),
                YD_MP25_MODE_FALLBACK_NO_EDID
            );
            assert_eq!(mode, YdMp25Mode::from_video(&HD720_60));
            assert_eq!(sink, YdMp25HdmiSink { hdmi: 0, vic: 4, aspect_16_9: 1, reserved: 0, refresh_hz: 60 });
            // no EDID at all, a short one
            let mut m2 = YdMp25Mode::default();
            assert_eq!(
                yd_mp25_choose_mode(&ADV_CFG, core::ptr::null(), 0, &mut m2, &mut sink),
                YD_MP25_MODE_FALLBACK_NO_EDID
            );
            assert_eq!(m2, mode);
            assert_eq!(
                yd_mp25_choose_mode(&ADV_CFG, bad.as_ptr(), 100, &mut m2, &mut sink),
                YD_MP25_MODE_FALLBACK_NO_EDID
            );
            assert_eq!(m2, mode);
        }
        // a sink past the LTDC's 150 MHz: a 2560x1440@60 preferred timing
        let mut big = i2cm::edid_block(EdidMode::P1080);
        // 241.5 MHz, 2560 + 160, 1440 + 41 (CVT-RB)
        big[54..72].copy_from_slice(&[
            0x56, 0x5e, 0x00, 0xa0, 0xa0, 0xa0, 0x29, 0x50, 0x30, 0x20, 0x35, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1e,
        ]);
        big[127] = 0u8.wrapping_sub(big[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b)));
        assert!(video::edid_preferred(&big).is_ok());
        let (r, m, _) = choose_mode(&big, 4);
        assert_eq!((r, m), (YD_MP25_MODE_FALLBACK_LIMIT, HD720_60));
    }

    #[test]
    fn the_whole_edid_path_falls_back_when_the_sink_has_none() {
        let soc = Soc::new();
        let (c, adv) = rig(&soc, Adv7535Config { edid: vec![0xff; 128], ..Adv7535Config::default() });
        let e = env(&c);
        let mut ad = YdMp25Adv7535Data::default();
        let mut edid = [0u8; 128];
        let (mut mode, mut sink) = (YdMp25Mode::default(), YdMp25HdmiSink::default());
        // SAFETY: locals.
        unsafe {
            assert_eq!(yd_mp25_adv7535_init(&ADV_CFG, &mut ad, &e), 0);
            assert_eq!(yd_mp25_adv7535_read_edid(&mut ad, &e, 0, edid.as_mut_ptr()), 0);
            assert_eq!(yd_mp25_choose_mode(&ADV_CFG, edid.as_ptr(), 128, &mut mode, &mut sink), 1);
            assert_eq!(mode, YdMp25Mode::from_video(&HD720_60));
            // unplugged: no EDID read, -ENODEV
            adv.set_hpd(false);
            assert_eq!(yd_mp25_adv7535_hpd(&mut ad, &e), 0);
            assert_eq!(yd_mp25_adv7535_read_edid(&mut ad, &e, 0, edid.as_mut_ptr()), -ENODEV);
        }
    }

    #[test]
    fn an_absent_adv7535_is_a_negative_errno() {
        let soc = Soc::new();
        let (c, _adv) = rig(&soc, Adv7535Config { in_reset: true, ..Adv7535Config::default() });
        let e = env(&c);
        let mut ad = YdMp25Adv7535Data::default();
        // SAFETY: locals.
        unsafe {
            assert_eq!(yd_mp25_adv7535_init(&ADV_CFG, &mut ad, &e), -EIO);
            assert_eq!(ad.probed, 0);
            assert!(yd_mp25_adv7535_hpd(&mut ad, &e) < 0);
            let m = YdMp25Mode::from_video(&HD720_60);
            let s = YdMp25HdmiSink::default();
            assert!(yd_mp25_adv7535_start(&ADV_CFG, &mut ad, &e, &m, &s) < 0);
            assert_eq!(ad.started, 0);
        }
        assert!(c.lines.borrow().iter().any(|(l, s)| *l == Level::Err as u32 && s.contains("no answer")));
    }

    #[test]
    fn null_pointers_are_einval() {
        let soc = Soc::new();
        let (c, _adv) = rig(&soc, Adv7535Config::default());
        let e = env(&c);
        c.bus.clear_log();
        let (mut ld, mut dd, mut ad) =
            (YdMp25LtdcData::default(), YdMp25DsiData::default(), YdMp25Adv7535Data::default());
        let (mut m, mut s) = (YdMp25Mode::default(), YdMp25HdmiSink::default());
        let mut b = [0u8; 128];
        // SAFETY: every call stops at its argument check before any access.
        unsafe {
            assert_eq!(yd_mp25_ltdc_probe(core::ptr::null(), &mut ld, &e), -EINVAL);
            assert_eq!(yd_mp25_ltdc_probe(&LTDC_CFG, core::ptr::null_mut(), &e), -EINVAL);
            assert_eq!(yd_mp25_ltdc_probe(&LTDC_CFG, &mut ld, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_ltdc_configure(&LTDC_CFG, &mut ld, core::ptr::null(), FB, 0, 0), -EINVAL);
            assert_eq!(yd_mp25_ltdc_enable(&LTDC_CFG, &mut ld, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_ltdc_disable(core::ptr::null(), &mut ld, &e), -EINVAL);
            assert_eq!(yd_mp25_ltdc_set_fb(&LTDC_CFG, core::ptr::null_mut(), &e, FB), -EINVAL);
            assert_eq!(yd_mp25_ltdc_wait_frame(core::ptr::null(), &e), -EINVAL);
            assert_eq!(yd_mp25_ltdc_isr(core::ptr::null(), &mut ld), 0);
            assert_eq!(yd_mp25_fill_color_bars(core::ptr::null_mut(), 16, 4, 4, 4), -EINVAL);
            assert_eq!(yd_mp25_dsi_probe(&DSI_CFG, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_dsi_attach(core::ptr::null(), &mut dd, 4, 0, YD_MP25_DSI_MODE_ADV7535), -EINVAL);
            assert_eq!(yd_mp25_dsi_enable(&DSI_CFG, &mut dd, &e, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_dsi_disable(&DSI_CFG, core::ptr::null_mut()), -EINVAL);
            assert_eq!(yd_mp25_adv7535_init(core::ptr::null(), &mut ad, &e), -EINVAL);
            assert_eq!(yd_mp25_adv7535_hpd(&mut ad, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_adv7535_read_edid(&mut ad, &e, 0, core::ptr::null_mut()), -EINVAL);
            assert_eq!(yd_mp25_choose_mode(&ADV_CFG, b.as_mut_ptr(), 128, core::ptr::null_mut(), &mut s), -EINVAL);
            assert_eq!(yd_mp25_choose_mode(core::ptr::null(), b.as_ptr(), 128, &mut m, &mut s), -EINVAL);
            assert_eq!(yd_mp25_adv7535_start(&ADV_CFG, &mut ad, &e, &m, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_adv7535_stop(core::ptr::null_mut(), &e), -EINVAL);
        }
        // nothing reached the bus
        assert!(c.bus.bus().is_empty());
    }

    #[test]
    fn state_errors_before_the_hardware() {
        let soc = Soc::new();
        let e = crate::model::TestEnv::default();
        let (mut ld, mut dd) = (YdMp25LtdcData::default(), YdMp25DsiData::default());
        let m = YdMp25Mode::from_video(&HD720_60);
        // enable before configure, flip before enable
        assert_eq!(ltdc_enable_with(&soc, &e, &LTDC_CFG, &mut ld), -EINVAL);
        assert_eq!(ltdc_set_fb_with(&soc, &e, &LTDC_CFG, &mut ld, FB), -ENODEV);
        // RGB888 has no LTDC format; a misaligned pitch is the block's -EINVAL
        assert_eq!(ltdc_configure_with(&soc, &LTDC_CFG, &mut ld, &m, FB, 0, 7), -ENOTSUP);
        assert_eq!(ltdc_configure_with(&soc, &LTDC_CFG, &mut ld, &m, FB, 5124, YD_MP25_LTDC_FMT_XRGB8888), -EINVAL);
        // DSI: enable before attach, bad attach arguments
        assert_eq!(dsi_enable_with(&soc, &e, &DSI_CFG, &mut dd, &m), -EINVAL);
        assert_eq!(dsi_attach_with(&DSI_CFG, &mut dd, 5, 0, YD_MP25_DSI_MODE_ADV7535), -EINVAL);
        assert_eq!(dsi_attach_with(&DSI_CFG, &mut dd, 4, 9, YD_MP25_DSI_MODE_ADV7535), -EINVAL);
        assert_eq!(dsi_attach_with(&DSI_CFG, &mut dd, 4, 0, 0), -EINVAL);
        assert!(soc.log.borrow().is_empty());
        // a reference outside 17-32 MHz stops in the PHY's init
        let cfg = YdMp25DsiConfig { pll_ref_hz: 40_000_000, ..DSI_CFG };
        assert_eq!(dsi_attach_with(&cfg, &mut dd, 4, 0, YD_MP25_DSI_MODE_ADV7535), 0);
        assert_eq!(dsi_enable_with(&soc, &e, &cfg, &mut dd, &m), -EINVAL);
        assert_eq!((dd.failed_step, dd.on), (dsi::step::PHY_INIT, 0));
        // an unclocked block reads as no LTDC / no DSI
        let dead = Soc::new();
        dead.poke(LTDC + ltdc::IDR as usize, 0);
        dead.poke(DSI, 0);
        assert_eq!(ltdc_probe_with(&dead, &e, &LTDC_CFG, &mut ld), -ENODEV);
        assert_eq!(dsi_probe_with(&dead, &e, &DSI_CFG), -ENODEV);
        let mut ld = YdMp25LtdcData::default();
        assert_eq!(ltdc_configure_with(&dead, &LTDC_CFG, &mut ld, &m, FB, 0, 0), -ENODEV);
    }

    #[test]
    fn the_ltdc_entry_points_on_a_register_image() {
        // the extern functions through Hw, over plain memory: no reload ever
        // clears, so enable times out after writing what it writes
        let mut img = vec![0u32; 0x400 / 4];
        img[(ltdc::IDR / 4) as usize] = ltdc::HWVER_40101;
        img[(ltdc::GC2R / 4) as usize] = 3 << 4;
        img[(ltdc::LCR / 4) as usize] = 2;
        let cfg = YdMp25LtdcConfig { base: img.as_mut_ptr() as usize, ..LTDC_CFG };
        let soc = Soc::new();
        let (c, _adv) = rig(&soc, Adv7535Config::default());
        let e = env(&c);
        let mut d = YdMp25LtdcData::default();
        let m = YdMp25Mode::from_video(&HD720_60);
        // SAFETY: `cfg.base` is `img`, 0x400 bytes, live for the calls.
        unsafe {
            assert_eq!(yd_mp25_ltdc_probe(&cfg, &mut d, &e), 0);
            assert_eq!(yd_mp25_ltdc_configure(&cfg, &mut d, &m, FB, 0, YD_MP25_LTDC_FMT_ARGB8888), 0);
            assert_eq!(yd_mp25_ltdc_enable(&cfg, &mut d, &e), -ETIMEDOUT);
            assert_eq!(d.on, 0);
            assert_eq!(yd_mp25_ltdc_isr(&cfg, &mut d), 0);
            assert_eq!(yd_mp25_ltdc_disable(&cfg, &mut d, &e), 0);
        }
        let r = |off: u32| img[(off / 4) as usize];
        assert_eq!(r(0x134), FB as u32); // L1CFBAR
        assert_eq!(r(0x138), (5120 << 16) | (5120 + 8 - 1)); // L1CFBLR
        assert_eq!(r(ltdc::TWCR), (1649 << 16) | 749);
        assert_eq!(r(ltdc::GCR) & ltdc::GCR_LTDCEN, 0);
        assert!(c.lines.borrow().iter().any(|(_, s)| s.contains("reload not taken")));
    }

    /// Run `f` with `m` behind the LTDC and DSI extern entry points.
    fn with_display<R>(m: &Mp25Display, f: impl FnOnce() -> R) -> R {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                DISPLAY.with(|d| d.set(None));
            }
        }
        let p: *const (dyn Mmio + '_) = m;
        // SAFETY: only the lifetime is erased; `Clear` removes the pointer
        // before this function returns, so `m` outlives every use.
        let p = unsafe { core::mem::transmute::<*const (dyn Mmio + '_), *const dyn Mmio>(p) };
        DISPLAY.with(|d| d.set(Some(p)));
        let _clear = Clear;
        f()
    }

    /// What the chain left behind, for the end-to-end tests.
    struct Chain {
        ld: YdMp25LtdcData,
        dd: YdMp25DsiData,
        ad: YdMp25Adv7535Data,
        mode: YdMp25Mode,
        sink: YdMp25HdmiSink,
        choice: i32,
    }

    /// A Zephyr driver's order over the extern entry points only, on the
    /// display model and the I2C model behind `e`: DSI probe and attach
    /// (`lanes`, RGB888, the ADV7535's video-mode flags), the bridge's init /
    /// HPD / EDID, the mode, then the LTDC (probe, configure an XRGB8888
    /// layer with pitch ROUND_UP(W * 4, 64), enable; Linux's
    /// `ltdc_crtc_atomic_enable` precedes the bridges' `pre_enable`), the DSI
    /// host (`dw_mipi_dsi_bridge_atomic_enable`, `dw_mipi_dsi_stm_*`) and the
    /// ADV7535's start (`adv7533_mode_set`, `adv7511_power_on`).
    fn bring_up(m: &Mp25Display, e: &YdEnv, lanes: u32) -> Chain {
        let mut ch = Chain {
            ld: YdMp25LtdcData::default(),
            dd: YdMp25DsiData::default(),
            ad: YdMp25Adv7535Data::default(),
            mode: YdMp25Mode::default(),
            sink: YdMp25HdmiSink::default(),
            choice: -1,
        };
        let cfg = YdMp25DsiConfig { lanes, ..DSI_CFG };
        let acfg = YdMp25Adv7535Config { lanes: lanes as u8, ..ADV_CFG };
        let mut edid = [0u8; 128];
        // SAFETY: every pointer is a local of this function or of the caller.
        with_display(m, || unsafe {
            assert_eq!(yd_mp25_dsi_probe(&cfg, e), 0);
            assert_eq!(
                yd_mp25_dsi_attach(&cfg, &mut ch.dd, lanes, YD_MP25_DSI_FMT_RGB888, YD_MP25_DSI_MODE_ADV7535),
                0
            );
            assert_eq!(yd_mp25_adv7535_init(&acfg, &mut ch.ad, e), 0);
            assert_eq!(yd_mp25_adv7535_hpd(&mut ch.ad, e), 1);
            assert_eq!(yd_mp25_adv7535_read_edid(&mut ch.ad, e, 0, edid.as_mut_ptr()), 0);
            ch.choice = yd_mp25_choose_mode(&acfg, edid.as_ptr(), 128, &mut ch.mode, &mut ch.sink);
            let pitch = (ch.mode.hactive * 4).next_multiple_of(64);
            assert_eq!(yd_mp25_ltdc_probe(&LTDC_CFG, &mut ch.ld, e), 0);
            assert_eq!(
                yd_mp25_ltdc_configure(&LTDC_CFG, &mut ch.ld, &ch.mode, FB, pitch, YD_MP25_LTDC_FMT_XRGB8888),
                0
            );
            assert_eq!(yd_mp25_ltdc_enable(&LTDC_CFG, &mut ch.ld, e), 0);
            assert_eq!(yd_mp25_dsi_enable(&cfg, &mut ch.dd, e, &ch.mode), 0);
            assert_eq!(yd_mp25_adv7535_start(&acfg, &mut ch.ad, e, &ch.mode, &ch.sink), 0);
        });
        ch
    }

    /// The final state both modes share: no ordering violation, the PLL and
    /// the LTDC running, the lanes and the PHY as `dw_mipi_dsi_phy_init` /
    /// `dw_mipi_dsi_stm_phy_init` set them, the layer taken over.
    fn assert_running(m: &Mp25Display, ch: &Chain, lanes: u32) {
        assert_eq!(m.violations(), Vec::<String>::new());
        assert!(m.pll_active().is_some(), "the DSI PLL is committed");
        assert_ne!(m.ltdc_reg(ltdc::GCR) & ltdc::GCR_LTDCEN, 0);
        assert_eq!((ch.ld.on, ch.dd.on, ch.ad.started), (1, 1, 1));
        assert_eq!(m.dsi_reg(dr::PHY_IF_CFG) & 3, lanes - 1); // N_LANES
        assert_ne!(m.dsi_reg(dr::PWR_UP), 0);
        assert_eq!(m.dsi_reg(dr::MODE_CFG), 0); // video mode
        assert_ne!(m.dsi_reg(dr::PHY_RSTZ) & dr::PHY_ENFORCEPLL, 0);
        let (w, h) = (ch.mode.hactive, ch.mode.vactive);
        assert_eq!((ch.ld.pitch, ch.ld.width, ch.ld.height), ((w * 4).next_multiple_of(64), w, h));
        assert_eq!(m.ltdc_active(0x134), FB as u32); // L1CFBAR taken over by the reload
        assert_ne!(m.ltdc_active(0x10c) & ltdc::LXCR_LEN, 0); // L1CR.LEN
    }

    /// Linux sequence mirrored: ltdc.c `ltdc_crtc_mode_set_nofb` /
    /// `ltdc_crtc_atomic_enable`, dw_mipi_dsi-stm.c `dw_mipi_dsi_phy_init`,
    /// dw-mipi-dsi.c `dw_mipi_dsi_bridge_atomic_enable` and adv7533.c
    /// `adv7533_mode_set`, at the DK's 1280x720@60 on four lanes with an EDID
    /// that prefers it (VIC 4, 74.25 MHz, 445.5 Mbit/s a lane).
    #[test]
    fn e2e_720p60_on_four_lanes_through_the_c_entry_points() {
        let soc = Soc::new();
        let (c, adv) = rig(&soc, Adv7535Config::default());
        let e = env(&c);
        let m = Mp25Display::new();
        let ch = bring_up(&m, &e, 4);
        assert_eq!(ch.choice, YD_MP25_MODE_FROM_EDID);
        assert_eq!(ch.mode, YdMp25Mode::from_video(&HD720_60));
        assert_running(&m, &ch, 4);
        // ltdc_crtc_mode_set_nofb: TWCR = (htotal - 1) << 16 | (vtotal - 1)
        assert_eq!(m.ltdc_reg(ltdc::TWCR), (1649 << 16) | 749);
        // dw_mipi_dsi_phy_init: 445.5 Mbit/s from 27 MHz (idf 4, ndiv 66, odf 2)
        assert_eq!((ch.dd.lane_kbps, ch.dd.lane_mbps), (445_500, 445));
        assert_eq!((ch.dd.pll_idf, ch.dd.pll_ndiv, ch.dd.pll_odf), (4, 66, 2));
        assert_eq!(m.dsi_reg(dr::VID_PKT_SIZE) & dr::VID_PKT_SIZE_MASK, 1280);
        assert_eq!(m.dsi_reg(dr::VID_VACTIVE_LINES), 720);
        // adv7533_mode_set: four lanes in the CEC map, powered up, DVI
        assert_eq!(adv.cec(0x1c), 4 << 4);
        assert_eq!(adv.reg(adv::reg::POWER) & adv::reg::POWER_DOWN, 0);
        assert_eq!(adv.cec(0x28), (1650 >> 4) as u8);
    }

    /// As the 720p test, at the mode the farm's KVM prefers: 1920x1080@60
    /// (VIC 16, 148.5 MHz, under the LTDC's 150 MHz pad limit) on four lanes,
    /// 891 Mbit/s a lane; ltdc.c's timing registers for 2200 x 1125.
    #[test]
    fn e2e_1080p60_on_four_lanes_through_the_c_entry_points() {
        let soc = Soc::new();
        let (c, adv) = rig(&soc, Adv7535Config { edid: i2cm::edid_block(EdidMode::P1080), ..Adv7535Config::default() });
        let e = env(&c);
        let m = Mp25Display::new();
        let ch = bring_up(&m, &e, 4);
        assert_eq!(ch.choice, YD_MP25_MODE_FROM_EDID);
        assert_eq!(ch.mode, YdMp25Mode::from_video(&FHD1080_60));
        assert_eq!((ch.sink.vic, ch.sink.refresh_hz), (16, 60));
        assert_running(&m, &ch, 4);
        assert_eq!(ch.ld.pitch, 7680);
        assert_eq!(m.ltdc_reg(ltdc::TWCR), (2199 << 16) | 1124);
        assert_eq!(ch.dd.lane_mbps, 891);
        assert_eq!(m.dsi_reg(dr::VID_PKT_SIZE) & dr::VID_PKT_SIZE_MASK, 1920);
        assert_eq!(m.dsi_reg(dr::VID_VACTIVE_LINES), 1080);
        assert_eq!(adv.cec(0x28), (2200 >> 4) as u8);
    }

    /// The fallback of adv7511_drv.c `adv7511_get_modes` when the sink gives
    /// no usable EDID (a block with a bad checksum): the chain comes up in
    /// 1280x720@60 on the same four lanes, as the 720p test does.
    #[test]
    fn e2e_a_bad_edid_runs_the_720p_fallback() {
        let soc = Soc::new();
        let mut bad = i2cm::edid_block(EdidMode::P1080);
        bad[127] ^= 1; // checksum
        let (c, _adv) = rig(&soc, Adv7535Config { edid: bad, ..Adv7535Config::default() });
        let e = env(&c);
        let m = Mp25Display::new();
        let ch = bring_up(&m, &e, 4);
        assert_eq!(ch.choice, YD_MP25_MODE_FALLBACK_NO_EDID);
        assert_eq!(ch.mode, YdMp25Mode::from_video(&HD720_60));
        assert_running(&m, &ch, 4);
        assert_eq!(m.ltdc_reg(ltdc::TWCR), (1649 << 16) | 749);
        assert_eq!(ch.dd.lane_mbps, 445);
    }

    /// The orders a driver can get wrong, as the ABI and the model judge them:
    /// ltdc.c has no ordering against the DSI (Linux enables the CRTC first,
    /// as `bring_up` does), so the LTDC streaming into a DSI that is not yet
    /// enabled is legal and leaves no violation; what the ABI refuses is a
    /// step before its predecessor (`ltdc_crtc_atomic_enable` before
    /// `mode_set_nofb`, `dw_mipi_dsi_bridge_atomic_enable` before attach), and
    /// the model reports timing registers written under a running LTDC
    /// (ltdc.c programs them with `LTDCEN` clear). The bridge keeps no state
    /// the ABI checks: its start on a chip that answers needs no init first.
    #[test]
    fn e2e_wrong_order_is_refused_or_reported() {
        let soc = Soc::new();
        let (c, adv) = rig(&soc, Adv7535Config::default());
        let e = env(&c);
        let m = Mp25Display::new();
        let mode = YdMp25Mode::from_video(&HD720_60);
        let (mut ld, mut dd) = (YdMp25LtdcData::default(), YdMp25DsiData::default());
        // SAFETY: locals.
        with_display(&m, || unsafe {
            // enable / flip before configure, DSI enable before attach:
            // errors, nothing streaming
            assert_eq!(yd_mp25_ltdc_probe(&LTDC_CFG, &mut ld, &e), 0);
            assert_eq!(yd_mp25_ltdc_enable(&LTDC_CFG, &mut ld, &e), -EINVAL);
            assert_eq!(yd_mp25_ltdc_set_fb(&LTDC_CFG, &mut ld, &e, FB), -ENODEV);
            assert_eq!(yd_mp25_dsi_enable(&DSI_CFG, &mut dd, &e, &mode), -EINVAL);
            assert_eq!((dd.on, ld.on), (0, 0));
            assert_eq!(m.ltdc_reg(ltdc::GCR) & ltdc::GCR_LTDCEN, 0);
            assert_eq!(m.dsi_reg(dr::PWR_UP), 0);
            // the LTDC streams before the DSI is attached: legal
            let pitch = 5120;
            assert_eq!(yd_mp25_ltdc_configure(&LTDC_CFG, &mut ld, &mode, FB, pitch, YD_MP25_LTDC_FMT_XRGB8888), 0);
            assert_eq!(yd_mp25_ltdc_enable(&LTDC_CFG, &mut ld, &e), 0);
            assert_eq!(m.violations(), Vec::<String>::new());
            // a second configure under the running LTDC: reported by the model
            assert_eq!(yd_mp25_ltdc_configure(&LTDC_CFG, &mut ld, &mode, FB, pitch, YD_MP25_LTDC_FMT_XRGB8888), 0);
        });
        let v = m.violations();
        assert!(!v.is_empty() && v.iter().all(|s| s.contains("written with LTDCEN high")), "{v:?}");
        assert_eq!(adv.reg(adv::reg::POWER) & adv::reg::POWER_DOWN, adv::reg::POWER_DOWN);
    }

    /// Teardown, as Linux's disable path orders it (adv7511 `power_off`, then
    /// dw-mipi-dsi.c `dw_mipi_dsi_bridge_post_atomic_disable`, then ltdc.c
    /// `ltdc_crtc_atomic_disable`): the models are idle, a second stop and
    /// disable are harmless, and the chain comes up again.
    #[test]
    fn e2e_stop_and_disable_leave_the_models_idle() {
        let soc = Soc::new();
        let (c, adv) = rig(&soc, Adv7535Config::default());
        let e = env(&c);
        let m = Mp25Display::new();
        let mut ch = bring_up(&m, &e, 4);
        // SAFETY: locals.
        with_display(&m, || unsafe {
            assert_eq!(yd_mp25_adv7535_stop(&mut ch.ad, &e), 0);
            assert_eq!(yd_mp25_dsi_disable(&DSI_CFG, &mut ch.dd), 0);
            assert_eq!(yd_mp25_ltdc_disable(&LTDC_CFG, &mut ch.ld, &e), 0);
            // again: nothing to undo
            assert_eq!(yd_mp25_adv7535_stop(&mut ch.ad, &e), 0);
            assert_eq!(yd_mp25_dsi_disable(&DSI_CFG, &mut ch.dd), 0);
            assert_eq!(yd_mp25_ltdc_disable(&LTDC_CFG, &mut ch.ld, &e), 0);
        });
        assert_eq!(m.violations(), Vec::<String>::new());
        assert_eq!((ch.ad.started, ch.dd.on, ch.ld.on), (0, 0, 0));
        assert_eq!(adv.reg(adv::reg::POWER) & adv::reg::POWER_DOWN, adv::reg::POWER_DOWN);
        assert_eq!(m.ltdc_reg(ltdc::GCR) & ltdc::GCR_LTDCEN, 0);
        assert_eq!(m.dsi_reg(dr::PWR_UP), 0);
        assert_eq!(m.dsi_reg(dr::PHY_RSTZ), 0);
        assert_eq!(m.dsi_reg(pr::WCR) & pr::WCR_DSIEN, 0);
        // up again on the same models: the same end state
        let again = bring_up(&m, &e, 4);
        assert_running(&m, &again, 4);
    }

    #[test]
    fn color_bars_through_the_entry_point() {
        let mut fb = vec![0u32; 16 * 2 + 4];
        // SAFETY: `fb` holds `len` pixels.
        unsafe {
            assert_eq!(yd_mp25_fill_color_bars(fb.as_mut_ptr(), fb.len(), 16, 2, 18), 0);
        }
        assert_eq!(fb[0], 0xffff_ffff);
        assert_eq!(fb[15], 0xff00_0000);
        assert_eq!(fb[16], 0); // past the width
        assert_eq!(fb[18], 0xffff_ffff); // the second line
    }
}
