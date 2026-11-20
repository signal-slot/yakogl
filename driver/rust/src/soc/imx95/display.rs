// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Display stream 0 of the i.MX 95, the SoC half that is the same for any
//! output: power and pixel clock ([`super::display_power`]), the display
//! block's routing (engine clock select, pixel interleaver, pixel link), the
//! DPU's one plane ([`super::dpu`]), the flip and the proof without a
//! camera. The order is driver/docs/imx95-display.md section 4(b), "The
//! order, cold", steps 1 (this module's part), 3-6 and 9.
//!
//! The boundary towards the DSI side is two calls:
//! - [`prepare`] (steps 1, 3-6): when it returns [`Prepared`], the DISPLAY
//!   domain is on, DISP1PIX runs at the mode's pixel clock from VIDEOPLL1
//!   (read back in [`Prepared::pix_hz`]), the display engine 0 clock is the
//!   CCM's, the pixel interleaver is in bypass with channel 0 set, the DPU
//!   holds the mode and the plane, and pixel link 0 is valid and enabled
//!   (`PIXEL_LINK_CTRL` 0x3): stream 0 is routed to the DSI host, not yet
//!   running. The DSI side then does its steps 1 (CAMERA, the MIPI PHY
//!   clocks), 2, the camera-side CSRs of step 6 (master CSR pixel link
//!   select 0, clock setting, stream CSR RGB 24), 7 (host attach and PHY)
//!   and 8 (bridge), with [`Prepared::mode`] and [`Prepared::pix_hz`].
//! - [`start`] (step 9): FrameGen0 enabled, waited for `PrimSyncStat`.
//!   [`Stream0`] says stream 0 is running with this mode, routed to DSI;
//!   the DSI side then turns its host to video mode (step 10).
//!
//! [`flip`] puts another buffer on screen at the next frame; [`proof`]
//! reads what the hardware says it does; [`stop`] undoes it (after the DSI
//! side stopped).
//!
//! Registers: NXP's BSD-3 `PERI_DISPLAY_BLK_CTRL_DISPLAYMIX.h` and
//! `PERI_DISPLAY_INTERLEAVER.h` ([`super::regs`]); values: the design note's
//! table ("And in the display CSR"), Zephyr's Apache-2.0 `dsi_nxp_dwc.c:114-116`
//! (the pixel link), and as facts from NXP's GPL `imx95-pixel-link.c` (valid,
//! then enable; disabled the other way round) and `imx95-pixel-interleaver.c`
//! (channel reset 10-20 µs, then the polarities; bypass is control 0).

use super::display_power::{self, Up};
use super::dpu::{self, Crc, FgStatus, Framebuffer, ReadBack, Refusal, Route, Window};
use super::regs::{display_blk_ctrl_displaymix as csr, display_interleaver as il};
use crate::firmware::scmi::Transport;
use crate::mmio::{field, Mmio};
use crate::video::VideoMode;

/// `DISPLAY__BLK_CTRL_DISPLAYMIX_BASE` (MIMX9596 `_COMMON.h`).
pub const CSR_BASE: usize = 0x4B01_0000;
/// `DISPLAY__PIXEL_INTERLEAVER1_BASE`.
pub const INTERLEAVER_BASE: usize = 0x4B0D_0000;

/// `DSIP_CLK0_SEL` 0: "CCM.video_pll_clk", that is DISP1PIX.
pub const CLK_SEL_CCM: u32 = 0;
/// The interleaver channel of display stream 0.
pub const CHANNEL: u32 = 0;
/// The interleaver's reset pulse, µs (Linux holds it 10-20 µs).
pub const RESET_US: u32 = 10;
/// How long [`start`] waits for `PrimSyncStat` (Linux: 100 ms).
pub const SYNC_WAIT_US: u32 = 100_000;

/// The blocks' addresses as the image maps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub dpu: usize,
    pub csr: usize,
    pub interleaver: usize,
}

impl Bases {
    /// The physical addresses (an identity map).
    pub const PHYS: Bases = Bases { dpu: dpu::DPU_BASE, csr: CSR_BASE, interleaver: INTERLEAVER_BASE };
}

/// The Toradex Capacitive Touch Display 10.1" DSI (Riverdi
/// RVT101HVDNWC00-B behind an SN65DSI83; Toradex's
/// `imx95-verdin-panel-cap-touch-10inch-dsi.dtso` panel-timing, the
/// design note's section 4(b) "The display"): 1280x800, 70 MHz, H 36 24 64,
/// V 8 8 24, syncs active low, DE active high.
pub const VERDIN_DSI_10INCH: VideoMode = VideoMode {
    pixel_clock_hz: 70_000_000,
    hactive: 1280,
    hfront: 36,
    hsync: 24,
    hback: 64,
    vactive: 800,
    vfront: 8,
    vsync: 8,
    vback: 24,
    hsync_high: false,
    vsync_high: false,
    de_high: true,
};

/// What stream 0 is to show and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    pub mode: VideoMode,
    pub route: Route,
    pub fb: Framebuffer,
    pub power: display_power::Config,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The mode or the buffer, before anything was sent or written.
    Refused(Refusal),
    /// The pixel clock asked for is not the mode's.
    Clock {
        mode_hz: u32,
        pix_hz: u64,
    },
    Power(display_power::Error),
    /// FrameGen0 did not run in sync within [`SYNC_WAIT_US`].
    NoSync(FgStatus),
    /// FrameGen0 did not stop within two frames.
    NotStopped(FgStatus),
    /// ExtDst0's shadow load did not come within three frames; the
    /// interrupt status read last.
    FlipTimeout(u32),
}

impl From<display_power::Error> for Error {
    fn from(e: display_power::Error) -> Error {
        Error::Power(e)
    }
}

/// Stream 0 configured and routed, not running ([`prepare`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub mode: VideoMode,
    pub route: Route,
    pub fb: Framebuffer,
    /// The DISP level read (set first when one was asked for).
    pub level: u32,
    /// DISP1PIX as the System Manager made it.
    pub pix_hz: u64,
}

/// Stream 0 running ([`start`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream0 {
    pub prepared: Prepared,
    /// The buffer on screen.
    pub fb: Framebuffer,
    /// µs [`start`] waited for the sync.
    pub sync_us: u32,
}

/// One frame of `mode` at `pix_hz`, µs (rounded up).
pub fn frame_us(mode: &VideoMode, pix_hz: u64) -> u32 {
    let px = u64::from(mode.htotal()) * u64::from(mode.vtotal());
    (px * 1_000_000).div_ceil(pix_hz.max(1)) as u32
}

fn w(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v);
}

/// Display engine 0's clock and the interleaver's mode (step 3).
fn route_clock(m: &impl Mmio, b: &Bases, route: Route) {
    match route {
        Route::Dsi => {
            m.modify32(
                b.csr + csr::DISPLAY_ENGINES_CLOCK_CONTROL as usize,
                csr::display_engines_clock_control::DSIP_CLK0_SEL_MASK,
                field(csr::display_engines_clock_control::DSIP_CLK0_SEL_MASK, CLK_SEL_CCM),
            );
            // Bypass: no split, stream 0 not interleaved.
            w(m, b.csr, csr::PIXEL_INTERLEAVER_CONTROL, 0);
        }
    }
}

/// Interleaver channel 0: reset, then the polarities of what the DPU sends
/// it (step 4). On the DSI route the DPU sends active-low syncs and an
/// active-high DE ([`dpu::timing`]); the interleaver's bits are 1 for
/// active low.
fn interleaver_channel(m: &impl Mmio, b: &Bases, route: Route, delay_us: &mut impl FnMut(u32)) {
    w(m, b.interleaver, il::swrst(CHANNEL), il::swrst::SW_RST_MASK);
    delay_us(RESET_US);
    w(m, b.interleaver, il::swrst(CHANNEL), 0);
    let ctrl = match route {
        Route::Dsi => il::ctrl::HSYNC_POLARITY_MASK | il::ctrl::VSYNC_POLARITY_MASK,
    };
    w(m, b.interleaver, il::ctrl(CHANNEL), ctrl);
}

/// Pixel link 0 on: valid, then enable (step 6, display CSR part).
fn pixel_link_on(m: &impl Mmio, b: &Bases) {
    let a = b.csr + csr::PIXEL_LINK_CTRL as usize;
    m.modify32(a, csr::pixel_link_ctrl::PL0_VALID_MASK, csr::pixel_link_ctrl::PL0_VALID_MASK);
    m.modify32(a, csr::pixel_link_ctrl::PL0_ENABLE_MASK, csr::pixel_link_ctrl::PL0_ENABLE_MASK);
}

/// Pixel link 0 off: enable, then valid.
fn pixel_link_off(m: &impl Mmio, b: &Bases) {
    let a = b.csr + csr::PIXEL_LINK_CTRL as usize;
    m.modify32(a, csr::pixel_link_ctrl::PL0_ENABLE_MASK, 0);
    m.modify32(a, csr::pixel_link_ctrl::PL0_VALID_MASK, 0);
}

/// Power, clock and route stream 0 and configure the DPU, not started (the
/// module documentation has the boundary). Everything is checked before the
/// first message: a refused mode or buffer sends and writes nothing.
pub fn prepare<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    b: &Bases,
    cfg: &StreamConfig,
    delay_us: &mut impl FnMut(u32),
) -> Result<Prepared, Error> {
    let timing = dpu::timing(&cfg.mode, cfg.route).map_err(Error::Refused)?;
    dpu::check_fb(&cfg.mode, &cfg.fb).map_err(Error::Refused)?;
    if cfg.power.pixel.pix_hz != u64::from(cfg.mode.pixel_clock_hz) {
        return Err(Error::Clock { mode_hz: cfg.mode.pixel_clock_hz, pix_hz: cfg.power.pixel.pix_hz });
    }
    let Up { level, pix_hz, .. } = display_power::power_on(t, &cfg.power)?;
    route_clock(m, b, cfg.route);
    interleaver_channel(m, b, cfg.route, delay_us);
    dpu::configure(m, b.dpu, &cfg.mode, &timing, &cfg.fb);
    match cfg.route {
        Route::Dsi => pixel_link_on(m, b),
    }
    Ok(Prepared { mode: cfg.mode, route: cfg.route, fb: cfg.fb, level, pix_hz })
}

/// Start FrameGen0 and wait until its primary channel is in sync.
pub fn start(m: &impl Mmio, b: &Bases, p: &Prepared, delay_us: &mut impl FnMut(u32)) -> Result<Stream0, Error> {
    dpu::start(m, b.dpu);
    let sync_us = dpu::wait_sync(m, b.dpu, SYNC_WAIT_US, delay_us).map_err(Error::NoSync)?;
    Ok(Stream0 { prepared: *p, fb: p.fb, sync_us })
}

/// Put `addr` (same size and stride) on screen at the next frame and wait
/// for it to be loaded.
pub fn flip(
    m: &impl Mmio,
    b: &Bases,
    s: &mut Stream0,
    addr: u64,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, Error> {
    let fb = Framebuffer { addr, stride: s.fb.stride };
    dpu::check_fb(&s.prepared.mode, &fb).map_err(Error::Refused)?;
    dpu::flip(m, b.dpu, addr);
    let budget = 3 * frame_us(&s.prepared.mode, s.prepared.pix_hz);
    let waited = dpu::wait_flip(m, b.dpu, budget, delay_us).map_err(Error::FlipTimeout)?;
    s.fb = fb;
    Ok(waited)
}

/// The proof without a camera (design note section 4, "Proof without a
/// camera"): the DPU's read-back, the display block's routing words, the
/// interleaver channel's FIFO overflow status, and Signature0's CRCs of the
/// whole active area once valid (`None` if not valid after two frames).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proof {
    pub dpu: ReadBack,
    pub clock_control: u32,
    pub interleaver_control: u32,
    pub pixel_link: u32,
    pub interleaver_status: u32,
    pub crc: Option<Crc>,
}

impl Proof {
    /// Everything [`prepare`] and [`start`] set reads back, and frames move.
    pub fn consistent(&self, s: &Stream0) -> bool {
        self.dpu.fg.in_sync()
            && self.dpu.frames_advance()
            && self.dpu.fetch_addr == s.fb.addr
            && self.pixel_link & (csr::pixel_link_ctrl::PL0_VALID_MASK | csr::pixel_link_ctrl::PL0_ENABLE_MASK)
                == csr::pixel_link_ctrl::PL0_VALID_MASK | csr::pixel_link_ctrl::PL0_ENABLE_MASK
            && self.clock_control & csr::display_engines_clock_control::DSIP_CLK0_SEL_MASK == CLK_SEL_CCM
    }
}

pub fn proof(m: &impl Mmio, b: &Bases, s: &Stream0, delay_us: &mut impl FnMut(u32)) -> Proof {
    let frame = frame_us(&s.prepared.mode, s.prepared.pix_hz);
    dpu::sig_start(m, b.dpu, &Window::full(&s.prepared.mode));
    let rb = dpu::read_back(m, b.dpu, frame, delay_us);
    let mut crc = dpu::sig_read(m, b.dpu);
    if crc.is_none() {
        delay_us(frame);
        crc = dpu::sig_read(m, b.dpu);
    }
    Proof {
        dpu: rb,
        clock_control: m.read32(b.csr + csr::DISPLAY_ENGINES_CLOCK_CONTROL as usize),
        interleaver_control: m.read32(b.csr + csr::PIXEL_INTERLEAVER_CONTROL as usize),
        pixel_link: m.read32(b.csr + csr::PIXEL_LINK_CTRL as usize),
        interleaver_status: m.read32(b.interleaver + il::is(CHANNEL) as usize),
        crc,
    }
}

/// Stop stream 0 and power the display off (the DSI side stopped first):
/// pixel link off, FrameGen0 off and read stopped, then
/// [`display_power::power_off`].
pub fn stop<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    b: &Bases,
    s: &Stream0,
    delay_us: &mut impl FnMut(u32),
) -> Result<(), Error> {
    pixel_link_off(m, b);
    dpu::stop(m, b.dpu);
    let budget = 2 * frame_us(&s.prepared.mode, s.prepared.pix_hz);
    dpu::wait_stopped(m, b.dpu, budget, delay_us).map_err(Error::NotStopped)?;
    display_power::power_off(t)?;
    Ok(())
}
