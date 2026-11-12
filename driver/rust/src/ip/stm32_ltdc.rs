// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ST's LCD-TFT display controller (LTDC, 0x4801_0000 on the STM32MP25);
//! follows Linux drm/stm/ltdc.c (+ ltdc.h), mainline master.
//!
//! One layer (L1) scanned out with a mode's timings, as the DK's
//! LTDC -> DSI -> ADV7535 -> HDMI path needs (docs/notes/pre-study-stm32mp257f-dk.md
//! P8, Q7). Clocks and resets are the SoC glue's, not this file's.
//!
//! Citations are `ltdc.c: function`; in mainline master (2116 lines) they are
//! `ltdc_crtc_atomic_enable` 776, `ltdc_crtc_atomic_disable` 799,
//! `ltdc_crtc_mode_set_nofb` 898 (timings 964-971, GCR 973-989, SSCR..LIPCR
//! 990-1007), `ltdc_plane_atomic_update` 1266 (window 1306-1313, PFCR 1330,
//! CACR 1334, BFCR 1337-1353, CFBAR 1365, CFBLR 1368-1378, CFBLNR 1382,
//! LXCR_LEN 1475-1487 and its reload), `ltdc_plane_atomic_disable` 1512,
//! `ltdc_encoder_disable` 1713, `ltdc_encoder_enable` 1727. What the code
//! relies on:
//!
//! - Version: `LTDC_IDR` (0x00) holds the hardware version; `ltdc_get_caps()`
//!   picks the layer register table from it. HWVER 0x040100/0x040101 (the
//!   MP25's `ltdc_layer_regs_a2`) puts layer 1 at 0x100 (`LAY_OFS_1`) and has
//!   a per-layer reload register `L1RCR` (0x108, `plane_reg_shadow`); the
//!   0x010200/0x010300 (a0, MP1 family) and 0x020101 (a1) layers start at
//!   0x80 and reload through the global `SRCR` (0x24). The pixel-format
//!   codes differ per version (`ltdc_pix_fmt_a0/a1/a2`): ARGB8888 and
//!   XRGB8888 are 0 everywhere, RGB565 is 2 (a0, a1) or 4 (a2).
//! - Timings (`ltdc_crtc_mode_set_nofb`): `hsync = hsync_end - hsync_start
//!   - 1`, `accum_hbp = htotal - hsync_start - 1`, `accum_act_w = accum_hbp
//!   + hdisplay`, `total_width = htotal - 1`, the same vertically; `SSCR`,
//!   `BPCR`, `AWCR`, `TWCR` are `(h << 16) | v` under mask 0x0FFF07FF;
//!   `LIPCR` = `accum_act_h + 1`. Polarity is `GCR` bits 31 `HSPOL` (set for
//!   a positive hsync), 30 `VSPOL`, 29 `DEPOL` (set for DE low), 28 `PCPOL`
//!   (pixel data driven on the falling edge), written under mask
//!   0xF0000000.
//! - Start (`ltdc_crtc_atomic_enable`, `ltdc_plane_atomic_update`,
//!   `ltdc_encoder_enable`): `BCCR` background, `IER |= 0x46`
//!   (FU warning/error, transfer error; the line interrupt is the vblank
//!   one), the layer's `LXCR_LEN`, a vertical-blanking reload (`L1RCR` or
//!   `SRCR` `VBR`, bit 1), then `FUT` (0x90) = 128 where the version has the
//!   FIFO threshold (a2), and `GCR_LTDCEN`. The reload bit is cleared by
//!   the hardware when the shadow registers are taken over, so it is the
//!   proof that frames are timed.
//! - Layer (`ltdc_plane_atomic_update`): `WHPCR`/`WVPCR` hold the window
//!   plus the back porch read from `BPCR` (`x1 + 1 + ahbp`, `x0 + 1 + ahbp`),
//!   `PFCR` the format code (mask 7), `CACR` = alpha >> 8, `BFCR` 0x607
//!   (0x405 for a format without alpha), `CFBAR` the address, `CFBLR` =
//!   `(pitch << 16) | line_length` with `line_length = cpp * width +
//!   bus_width / 8 - 1`, `CFBLNR` the line count. `bus_width` is
//!   `8 << GC2R[6:4]` (`ltdc_get_caps`).
//! - ltdc.c checks no pitch or address alignment (`ltdc_plane_atomic_check`
//!   only refuses scaling). The rule here is the safe one the line-length
//!   term implies: pitch and address multiples of the bus width in bytes.
//!   RM0457 is the authority; the board run (item board-display) confirms.
//! - Interrupt status (`ltdc_irq`): `ISR` (0x38) is read and written back to
//!   `ICR` (0x3C) to clear; `LIF` (bit 0) is the line interrupt at `LIPCR`
//!   (vblank), `FUWIF` (1) FIFO underrun warning, `TERRIF` (2) transfer
//!   error, `FUEIF` (6) FIFO underrun error. [`Ltdc::wait_frame`] polls
//!   `LIF` with the interrupt masked.
//! - Stop (`ltdc_plane_atomic_disable`, `ltdc_crtc_atomic_disable`,
//!   `ltdc_encoder_disable`): `LXCR &= ~0x113`, `CACR` = 0, `IER &= ~0x46`,
//!   reload (`L1RCR` `VBR`, or `SRCR` `IMR` immediate), then `GCR_LTDCEN`
//!   cleared.

use crate::env::{Env, Level};
use crate::errno::{EINVAL, ENODEV, ETIMEDOUT};
use crate::mmio::{field, Mmio};
use crate::video::VideoMode;

/// Global registers (`ltdc.c`: `#define LTDC_*`).
pub const IDR: u32 = 0x00;
pub const SSCR: u32 = 0x08;
pub const BPCR: u32 = 0x0c;
pub const AWCR: u32 = 0x10;
pub const TWCR: u32 = 0x14;
pub const GCR: u32 = 0x18;
pub const GC2R: u32 = 0x20;
pub const SRCR: u32 = 0x24;
pub const BCCR: u32 = 0x2c;
pub const IER: u32 = 0x34;
pub const ISR: u32 = 0x38;
pub const ICR: u32 = 0x3c;
pub const LIPCR: u32 = 0x40;
pub const FUT: u32 = 0x90;

pub const HWVER_10200: u32 = 0x01_0200;
pub const HWVER_10300: u32 = 0x01_0300;
pub const HWVER_20101: u32 = 0x02_0101;
pub const HWVER_40100: u32 = 0x04_0100;
pub const HWVER_40101: u32 = 0x04_0101;

/// `GCR` bits.
pub const GCR_LTDCEN: u32 = 1 << 0;
pub const GCR_PCPOL: u32 = 1 << 28;
pub const GCR_DEPOL: u32 = 1 << 29;
pub const GCR_VSPOL: u32 = 1 << 30;
pub const GCR_HSPOL: u32 = 1 << 31;
const GCR_POL_MASK: u32 = 0xf000_0000;
/// `GC2R[6:4]`: the bus width, `8 << BW` bits.
const GC2R_BW: u32 = 0x70;
/// `SRCR` / `L1RCR`.
pub const RCR_IMR: u32 = 1 << 0;
pub const RCR_VBR: u32 = 1 << 1;
/// `IER` / `ISR` bits.
pub const INT_LINE: u32 = 1 << 0;
pub const INT_FU_WARN: u32 = 1 << 1;
pub const INT_TRANSFER_ERR: u32 = 1 << 2;
pub const INT_FU_ERR: u32 = 1 << 6;
/// The error interrupts `ltdc_crtc_atomic_enable` unmasks (0x46).
pub const IER_ERRORS: u32 = INT_FU_WARN | INT_TRANSFER_ERR | INT_FU_ERR;
/// `LXCR`: layer enable, colour keying, CLUT, horizontal mirror.
pub const LXCR_LEN: u32 = 1 << 0;
const LXCR_MASK: u32 = 0x113;
/// Timing registers' field masks (`(h << 16) | v`).
const TIMING_MASK: u32 = 0x0fff_07ff;
/// `FUT_DFT`.
pub const FUT_DEFAULT: u32 = 128;
/// `BFCR` for a format with alpha (the default) and without.
const BFCR_ALPHA: u32 = 0x607;
const BFCR_NO_ALPHA: u32 = 0x405;
const BFCR_MASK: u32 = 0x707;
const CACR_MASK: u32 = 0xff;
const PFCR_MASK: u32 = 0x7;

/// Bounds of the waits: a reload completes at the next vertical blank, so
/// two frames at 24 Hz is the limit.
pub const FRAME_WAIT_US: u32 = 2 * 41_667;
const POLL_STEP_US: u32 = 50;

/// Layer pixel formats (`ltdc_pix_fmt_*`). `Xrgb8888` is `Argb8888` with
/// the blender told there is no alpha (`BFCR` 0x405).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Argb8888,
    Xrgb8888,
    Rgb565,
}

impl Format {
    pub const fn bytes_pp(self) -> u32 {
        match self {
            Format::Argb8888 | Format::Xrgb8888 => 4,
            Format::Rgb565 => 2,
        }
    }

    const fn has_alpha(self) -> bool {
        matches!(self, Format::Argb8888)
    }

    /// The `PFCR[2:0]` code for a hardware version.
    const fn code(self, hwver: u32) -> u32 {
        match self {
            Format::Argb8888 | Format::Xrgb8888 => 0,
            Format::Rgb565 => {
                if hwver >= HWVER_40100 {
                    4
                } else {
                    2
                }
            }
        }
    }
}

/// Layer 1's registers for a hardware version (`ltdc_layer_regs_a0/a1/a2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerRegs {
    pub cr: u32,
    pub whpcr: u32,
    pub wvpcr: u32,
    pub pfcr: u32,
    pub cacr: u32,
    pub bfcr: u32,
    pub cfbar: u32,
    pub cfblr: u32,
    pub cfblnr: u32,
    /// The per-layer reload register (a2 only; others reload through `SRCR`).
    pub rcr: Option<u32>,
}

const REGS_A0: LayerRegs = LayerRegs {
    cr: 0x84,
    whpcr: 0x88,
    wvpcr: 0x8c,
    pfcr: 0x94,
    cacr: 0x98,
    bfcr: 0xa0,
    cfbar: 0xac,
    cfblr: 0xb0,
    cfblnr: 0xb4,
    rcr: None,
};
const REGS_A1: LayerRegs = LayerRegs {
    cr: 0x88,
    whpcr: 0x8c,
    wvpcr: 0x90,
    pfcr: 0x98,
    cacr: 0x9c,
    bfcr: 0xa4,
    cfbar: 0xac,
    cfblr: 0xb0,
    cfblnr: 0xb4,
    rcr: None,
};
const REGS_A2: LayerRegs = LayerRegs {
    cr: 0x10c,
    whpcr: 0x110,
    wvpcr: 0x114,
    pfcr: 0x11c,
    cacr: 0x120,
    bfcr: 0x128,
    cfbar: 0x134,
    cfblr: 0x138,
    cfblnr: 0x13c,
    rcr: Some(0x108),
};

/// The layer to scan: the buffer, its pitch (0 = `width * bytes_pp`) and format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub phys: u64,
    pub pitch: u32,
    pub format: Format,
}

/// The output side beyond the mode's own sync/DE polarity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Output {
    /// `GCR_PCPOL`: pixel data driven on the falling clock edge.
    pub pixel_data_negedge: bool,
    /// The background colour, 0x00RRGGBB (`BCCR`).
    pub background: u32,
}

/// What [`Ltdc::configure`] settled on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Configured {
    pub pitch: u32,
    pub line_length: u32,
}

/// A probed LTDC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ltdc {
    pub base: usize,
    pub hwver: u32,
    /// `8 << GC2R[6:4]` bits, in bytes.
    pub bus_bytes: u32,
    regs: LayerRegs,
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v)
}

fn upd(m: &impl Mmio, base: usize, off: u32, mask: u32, v: u32) {
    m.modify32(base + off as usize, mask, v)
}

impl Ltdc {
    /// Read `IDR` and `GC2R` (`ltdc_get_caps`). `Err(-ENODEV)` for a version
    /// ltdc.c does not know (a dead or unclocked block reads 0 or all-ones).
    pub fn probe(m: &impl Mmio, base: usize) -> Result<Ltdc, i32> {
        let hwver = rd(m, base, IDR);
        let regs = match hwver {
            HWVER_10200 | HWVER_10300 => REGS_A0,
            HWVER_20101 => REGS_A1,
            HWVER_40100 | HWVER_40101 => REGS_A2,
            _ => return Err(-ENODEV),
        };
        let bw = (rd(m, base, GC2R) & GC2R_BW) >> 4;
        Ok(Ltdc { base, hwver, bus_bytes: (8 << bw) / 8, regs })
    }

    fn reload_reg(&self) -> u32 {
        self.regs.rcr.unwrap_or(SRCR)
    }

    /// Program everything but the enables: polarity, timings, background,
    /// the layer's window, format, address and pitch (`ltdc_crtc_mode_set_nofb`
    /// then `ltdc_plane_atomic_update` minus `LXCR_LEN`). Nothing is written
    /// when a check fails.
    pub fn configure(&self, m: &impl Mmio, mode: &VideoMode, out: &Output, layer: &Layer) -> Result<Configured, i32> {
        mode.check()?;
        if mode.hactive > 0xfff || mode.htotal() > 0x1000 || mode.vtotal() > 0x800 {
            return Err(-EINVAL);
        }
        let cpp = layer.format.bytes_pp();
        let line_length = cpp * mode.hactive + self.bus_bytes - 1;
        let pitch = if layer.pitch == 0 { cpp * mode.hactive } else { layer.pitch };
        if pitch < cpp * mode.hactive || pitch > 0xffff || !pitch.is_multiple_of(self.bus_bytes) {
            return Err(-EINVAL);
        }
        if line_length > 0x1fff || layer.phys >> 32 != 0 || !layer.phys.is_multiple_of(u64::from(self.bus_bytes)) {
            return Err(-EINVAL);
        }
        let b = self.base;
        let hsync = mode.hsync - 1;
        let vsync = mode.vsync - 1;
        let acc_hbp = mode.hsync + mode.hback - 1;
        let acc_vbp = mode.vsync + mode.vback - 1;
        let acc_w = acc_hbp + mode.hactive;
        let acc_h = acc_vbp + mode.vactive;
        let pol = if mode.hsync_high { GCR_HSPOL } else { 0 }
            | if mode.vsync_high { GCR_VSPOL } else { 0 }
            | if mode.de_high { 0 } else { GCR_DEPOL }
            | if out.pixel_data_negedge { GCR_PCPOL } else { 0 };
        upd(m, b, GCR, GCR_POL_MASK, pol);
        upd(m, b, SSCR, TIMING_MASK, (hsync << 16) | vsync);
        upd(m, b, BPCR, TIMING_MASK, (acc_hbp << 16) | acc_vbp);
        upd(m, b, AWCR, TIMING_MASK, (acc_w << 16) | acc_h);
        upd(m, b, TWCR, TIMING_MASK, ((mode.htotal() - 1) << 16) | (mode.vtotal() - 1));
        wr(m, b, LIPCR, acc_h + 1);
        wr(m, b, BCCR, out.background & 0x00ff_ffff);
        // the layer window: BPCR's back porch plus the window (x1 + 1 + ahbp)
        let r = &self.regs;
        upd(m, b, r.whpcr, 0x0fff_0fff, ((mode.hactive + acc_hbp) << 16) | (1 + acc_hbp));
        upd(m, b, r.wvpcr, 0x07ff_07ff, ((mode.vactive + acc_vbp) << 16) | (1 + acc_vbp));
        upd(m, b, r.pfcr, PFCR_MASK, layer.format.code(self.hwver));
        upd(m, b, r.cacr, CACR_MASK, 0xff);
        upd(m, b, r.bfcr, BFCR_MASK, if layer.format.has_alpha() { BFCR_ALPHA } else { BFCR_NO_ALPHA });
        wr(m, b, r.cfbar, layer.phys as u32);
        wr(m, b, r.cfblr, field(0xffff_0000, pitch) | line_length);
        upd(m, b, r.cfblnr, 0x7ff, mode.vactive);
        Ok(Configured { pitch, line_length })
    }

    fn wait_reload(&self, m: &impl Mmio, env: &impl Env, bit: u32) -> Result<(), i32> {
        let mut waited = 0;
        loop {
            if rd(m, self.base, self.reload_reg()) & bit == 0 {
                return Ok(());
            }
            if waited >= FRAME_WAIT_US {
                return Err(-ETIMEDOUT);
            }
            env.busy_wait_us(POLL_STEP_US);
            waited += POLL_STEP_US;
        }
    }

    /// Start scanning: error interrupts unmasked, the layer on, a vertical
    /// blanking reload requested, the FIFO threshold, then `LTDCEN`; waits for
    /// the hardware to take the reload. `Err(-ETIMEDOUT)` means no frame
    /// boundary came within two frames: the pixel clock is not running.
    pub fn enable(&self, m: &impl Mmio, env: &impl Env) -> Result<(), i32> {
        let b = self.base;
        upd(m, b, IER, IER_ERRORS, IER_ERRORS);
        upd(m, b, self.regs.cr, LXCR_MASK, LXCR_LEN);
        wr(m, b, self.reload_reg(), RCR_VBR);
        if self.hwver >= HWVER_40100 {
            wr(m, b, FUT, FUT_DEFAULT);
        }
        upd(m, b, GCR, GCR_LTDCEN, GCR_LTDCEN);
        let res = self.wait_reload(m, env, RCR_VBR);
        if res.is_err() {
            env.log(Level::Err, format_args!("ltdc: reload not taken, no pixel clock?"));
        }
        res
    }

    /// Stop: the layer off with constant alpha 0, errors masked, the change
    /// reloaded immediately where there is a global reload (`SRCR` `IMR`) or
    /// at the next blank (`L1RCR` `VBR`, waited for), then `LTDCEN` cleared.
    /// A controller whose clock stopped cannot take the reload; that is
    /// reported and `LTDCEN` is cleared anyway.
    pub fn disable(&self, m: &impl Mmio, env: &impl Env) -> Result<(), i32> {
        let b = self.base;
        upd(m, b, self.regs.cr, LXCR_MASK, 0);
        upd(m, b, self.regs.cacr, CACR_MASK, 0);
        upd(m, b, IER, IER_ERRORS, 0);
        let res = if self.regs.rcr.is_some() {
            wr(m, b, self.reload_reg(), RCR_VBR);
            self.wait_reload(m, env, RCR_VBR)
        } else {
            wr(m, b, SRCR, RCR_IMR);
            Ok(())
        };
        upd(m, b, GCR, GCR_LTDCEN, 0);
        res
    }

    /// Point the running layer at `phys` and wait for the reload.
    pub fn set_fb(&self, m: &impl Mmio, env: &impl Env, phys: u64) -> Result<(), i32> {
        if phys >> 32 != 0 || !phys.is_multiple_of(u64::from(self.bus_bytes)) {
            return Err(-EINVAL);
        }
        wr(m, self.base, self.regs.cfbar, phys as u32);
        wr(m, self.base, self.reload_reg(), RCR_VBR);
        self.wait_reload(m, env, RCR_VBR)
    }

    /// Whether the controller is enabled with the layer on.
    pub fn is_running(&self, m: &impl Mmio) -> bool {
        rd(m, self.base, GCR) & GCR_LTDCEN != 0 && rd(m, self.base, self.regs.cr) & LXCR_LEN != 0
    }

    /// `ISR` as it stands.
    pub fn status(&self, m: &impl Mmio) -> u32 {
        rd(m, self.base, ISR)
    }

    /// Clear `ISR` bits (write to `ICR`, as `ltdc_irq`).
    pub fn clear_status(&self, m: &impl Mmio, bits: u32) {
        wr(m, self.base, ICR, bits)
    }

    /// Wait for one line interrupt (`LIPCR`, the end of the active area):
    /// clears `LIF`, then polls `ISR` for it. `Err(-ETIMEDOUT)` after two frames.
    pub fn wait_frame(&self, m: &impl Mmio, env: &impl Env) -> Result<(), i32> {
        self.clear_status(m, INT_LINE);
        let mut waited = 0;
        loop {
            if self.status(m) & INT_LINE != 0 {
                return Ok(());
            }
            if waited >= FRAME_WAIT_US {
                return Err(-ETIMEDOUT);
            }
            env.busy_wait_us(POLL_STEP_US);
            waited += POLL_STEP_US;
        }
    }
}

/// Eight colour bars (white, yellow, cyan, green, magenta, red, blue, black;
/// 0xFFRRGGBB, opaque) across `buf`, `width` pixels per line, `stride_px`
/// pixels between line starts. Bar edges are at `i * width / 8`. Lines past
/// the buffer's end are left alone.
pub fn fill_color_bars(buf: &mut [u32], width: usize, height: usize, stride_px: usize) {
    const BARS: [u32; 8] =
        [0xffff_ffff, 0xffff_ff00, 0xff00_ffff, 0xff00_ff00, 0xffff_00ff, 0xffff_0000, 0xff00_00ff, 0xff00_0000];
    if width == 0 || stride_px < width {
        return;
    }
    for line in buf.chunks_mut(stride_px).take(height) {
        for (x, px) in line.iter_mut().take(width).enumerate() {
            *px = BARS[x * 8 / width];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TestEnv;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const B: usize = 0x4801_0000;
    const FB: u64 = 0x9000_0000;

    /// 1280x720@60, CEA VIC 4: 74.25 MHz, H 1280 110 40 220 (+), V 720 5 5 20 (+).
    const M720: VideoMode = VideoMode {
        pixel_clock_hz: 74_250_000,
        hactive: 1280,
        hfront: 110,
        hsync: 40,
        hback: 220,
        vactive: 720,
        vfront: 5,
        vsync: 5,
        vback: 20,
        hsync_high: true,
        vsync_high: true,
        de_high: true,
    };

    /// Recording register file with just enough LTDC behaviour: reload bits
    /// clear when the block is enabled (or at once for `IMR`), `LIF` reads
    /// set while enabled.
    struct Fake {
        regs: RefCell<BTreeMap<usize, u32>>,
        log: RefCell<Vec<(bool, u32, u32)>>,
        rcr: Option<u32>,
        no_clock: bool,
    }

    impl Fake {
        fn new(idr: u32, bw: u32) -> Fake {
            let f = Fake {
                regs: RefCell::default(),
                log: RefCell::default(),
                rcr: (idr >= HWVER_40100).then_some(0x108),
                no_clock: false,
            };
            f.regs.borrow_mut().insert(B + IDR as usize, idr);
            f.regs.borrow_mut().insert(B + GC2R as usize, bw << 4);
            f
        }
        fn mp25() -> Fake {
            Fake::new(HWVER_40101, 3)
        }
        fn writes(&self) -> Vec<(u32, u32)> {
            self.log.borrow().iter().filter(|e| e.0).map(|e| (e.1, e.2)).collect()
        }
        fn clear_log(&self) {
            self.log.borrow_mut().clear();
        }
        fn peek(&self, off: u32) -> u32 {
            self.regs.borrow().get(&(B + off as usize)).copied().unwrap_or(0)
        }
        fn reload_off(&self) -> u32 {
            self.rcr.unwrap_or(SRCR)
        }
    }

    impl Mmio for Fake {
        fn read32(&self, addr: usize) -> u32 {
            let off = (addr - B) as u32;
            let mut v = self.regs.borrow().get(&addr).copied().unwrap_or(0);
            let on = self.peek(GCR) & GCR_LTDCEN != 0 && !self.no_clock;
            if off == self.reload_off() && on && v & RCR_VBR != 0 {
                v &= !RCR_VBR;
                self.regs.borrow_mut().insert(addr, v);
            }
            if off == ISR && on {
                v |= INT_LINE;
            }
            self.log.borrow_mut().push((false, off, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = (addr - B) as u32;
            self.log.borrow_mut().push((true, off, val));
            let mut r = self.regs.borrow_mut();
            if off == ICR {
                *r.entry(B + ISR as usize).or_default() &= !val;
            } else if off == self.reload_off() && val & RCR_IMR != 0 {
                r.insert(addr, val & !RCR_IMR);
            } else {
                r.insert(addr, val);
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    const LAYER: Layer = Layer { phys: FB, pitch: 0, format: Format::Xrgb8888 };

    #[test]
    fn probe_reads_the_version_and_the_bus_width() {
        // ltdc.c ltdc_get_caps: IDR picks the layer table, GC2R[6:4] the bus width
        let f = Fake::mp25();
        let l = Ltdc::probe(&f, B).unwrap();
        assert_eq!((l.hwver, l.bus_bytes), (HWVER_40101, 8));
        assert_eq!(Ltdc::probe(&Fake::new(0, 0), B), Err(-ENODEV));
    }

    #[test]
    fn mp25_1280x720_xrgb8888_register_sequence() {
        let f = Fake::mp25();
        let e = TestEnv::default();
        let l = Ltdc::probe(&f, B).unwrap();
        f.clear_log();
        let c = l.configure(&f, &M720, &Output::default(), &LAYER).unwrap();
        assert_eq!(c, Configured { pitch: 5120, line_length: 5127 });
        // ltdc_crtc_mode_set_nofb: GCR polarity (HSPOL|VSPOL, DE high), then
        // SSCR/BPCR/AWCR/TWCR, LIPCR; hsync 40-1, vsync 5-1, accum_hbp
        // 40+220-1, accum_vbp 5+20-1, +1280/+720, total 1649/749, LIPCR 745
        // (ltdc_crtc_atomic_enable: BCCR 0); ltdc_plane_atomic_update:
        // WHPCR/WVPCR = x1+1+ahbp, x0+1+ahbp; PFCR 0 (a2 ARGB/XRGB), CACR
        // 0xff, BFCR 0x405 (no alpha), CFBAR, CFBLR = pitch<<16 | cpp*w+8-1,
        // CFBLNR (a2: layer 1 at 0x100, ltdc_layer_regs_a2)
        let want = [
            (GCR, GCR_HSPOL | GCR_VSPOL),
            (SSCR, (39 << 16) | 4),
            (BPCR, (259 << 16) | 24),
            (AWCR, (1539 << 16) | 744),
            (TWCR, (1649 << 16) | 749),
            (LIPCR, 745),
            (BCCR, 0),
            (0x110, (1539 << 16) | 260),
            (0x114, (744 << 16) | 25),
            (0x11c, 0),
            (0x120, 0xff),
            (0x128, 0x405),
            (0x134, 0x9000_0000),
            (0x138, (5120 << 16) | 5127),
            (0x13c, 720),
        ];
        assert_eq!(f.writes(), want);
        assert!(!l.is_running(&f));
        f.clear_log();
        // ltdc_crtc_atomic_enable (IER |= 0x46), ltdc_plane_atomic_update
        // (LXCR_LEN, RCR = VBR), ltdc_encoder_enable (FUT = 128, LTDCEN)
        l.enable(&f, &e).unwrap();
        assert_eq!(
            f.writes(),
            [(IER, 0x46), (0x10c, 1), (0x108, RCR_VBR), (FUT, 128), (GCR, GCR_HSPOL | GCR_VSPOL | GCR_LTDCEN)]
        );
        assert!(l.is_running(&f));
        assert_eq!(f.peek(0x108) & RCR_VBR, 0, "reload taken");
        l.wait_frame(&f, &e).unwrap();
    }

    #[test]
    fn formats_follow_the_per_version_tables() {
        // ltdc.c ltdc_pix_fmt_a2: ARGB8888 0, RGB565 4; a0/a1: RGB565 2
        let f = Fake::mp25();
        let l = Ltdc::probe(&f, B).unwrap();
        let rgb = Layer { phys: FB, pitch: 0, format: Format::Rgb565 };
        assert_eq!(l.configure(&f, &M720, &Output::default(), &rgb).unwrap().pitch, 2560);
        assert_eq!(f.peek(0x11c), 4);
        let argb = Layer { phys: FB, pitch: 0, format: Format::Argb8888 };
        l.configure(&f, &M720, &Output::default(), &argb).unwrap();
        assert_eq!((f.peek(0x11c), f.peek(0x128)), (0, 0x607));
        let g = Fake::new(HWVER_20101, 3);
        let l = Ltdc::probe(&g, B).unwrap();
        l.configure(&g, &M720, &Output::default(), &rgb).unwrap();
        assert_eq!((g.peek(0x98), g.peek(0xb0)), (2, (2560 << 16) | 2567));
    }

    #[test]
    fn bad_pitch_or_address_is_refused_before_any_write() {
        // ltdc.c checks neither; the rule is the bus width in bytes (the
        // line-length term), see the module text
        let f = Fake::mp25();
        let l = Ltdc::probe(&f, B).unwrap();
        f.clear_log();
        let o = Output::default();
        for bad in [
            Layer { pitch: 5124, ..LAYER },     // not a multiple of 8
            Layer { pitch: 5112, ..LAYER },     // under width * 4
            Layer { phys: FB + 4, ..LAYER },    // address
            Layer { phys: 1 << 32, ..LAYER },   // beyond 32 bits
            Layer { pitch: 0x1_0000, ..LAYER }, // CFBP is 16 bits
        ] {
            assert_eq!(l.configure(&f, &M720, &o, &bad), Err(-EINVAL), "{bad:?}");
        }
        assert!(f.writes().is_empty());
        assert_eq!(l.set_fb(&f, &TestEnv::default(), FB + 4), Err(-EINVAL));
        // a padded pitch is fine and lands in CFBLR
        let ok = Layer { pitch: 5248, ..LAYER };
        assert_eq!(l.configure(&f, &M720, &o, &ok).unwrap().pitch, 5248);
        assert_eq!(f.peek(0x138), (5248 << 16) | 5127);
    }

    #[test]
    fn disable_clears_the_layer_then_ltdcen() {
        // ltdc_plane_atomic_disable (LXCR &= ~0x113, CACR = 0, RCR = VBR),
        // ltdc_crtc_atomic_disable (IER &= ~0x46), ltdc_encoder_disable
        let f = Fake::mp25();
        let e = TestEnv::default();
        let l = Ltdc::probe(&f, B).unwrap();
        l.configure(&f, &M720, &Output::default(), &LAYER).unwrap();
        l.enable(&f, &e).unwrap();
        f.clear_log();
        l.disable(&f, &e).unwrap();
        assert_eq!(f.writes(), [(0x10c, 0), (0x120, 0), (IER, 0), (0x108, RCR_VBR), (GCR, GCR_HSPOL | GCR_VSPOL)]);
        assert!(!l.is_running(&f));
    }

    #[test]
    fn disable_on_a_global_reload_version_uses_imr() {
        // no plane_reg_shadow: ltdc_crtc_atomic_disable writes SRCR = IMR
        let f = Fake::new(HWVER_10300, 2);
        let e = TestEnv::default();
        let l = Ltdc::probe(&f, B).unwrap();
        assert_eq!(l.bus_bytes, 4);
        l.configure(&f, &M720, &Output::default(), &LAYER).unwrap();
        l.enable(&f, &e).unwrap();
        f.clear_log();
        l.disable(&f, &e).unwrap();
        let w = f.writes();
        assert_eq!(w[..4], [(0x84, 0), (0x98, 0), (IER, 0), (SRCR, RCR_IMR)]);
        assert_eq!(w[4].0, GCR);
    }

    #[test]
    fn no_pixel_clock_times_out_but_disable_still_stops() {
        let mut f = Fake::mp25();
        f.no_clock = true;
        let e = TestEnv::default();
        let l = Ltdc::probe(&f, B).unwrap();
        l.configure(&f, &M720, &Output::default(), &LAYER).unwrap();
        assert_eq!(l.enable(&f, &e), Err(-ETIMEDOUT));
        assert!(*e.waited_us.borrow() >= u64::from(FRAME_WAIT_US));
        assert_eq!(l.wait_frame(&f, &e), Err(-ETIMEDOUT));
        assert_eq!(l.disable(&f, &e), Err(-ETIMEDOUT));
        assert_eq!(f.peek(GCR) & GCR_LTDCEN, 0);
    }

    #[test]
    fn color_bars_are_eight_equal_bars() {
        let mut buf = vec![0u32; 18 * 2];
        fill_color_bars(&mut buf, 16, 2, 18);
        assert_eq!(buf[0], 0xffff_ffff);
        assert_eq!(buf[2], 0xffff_ff00);
        assert_eq!(buf[15], 0xff00_0000);
        assert_eq!(buf[16], 0, "padding untouched");
        assert_eq!(buf[18 + 12], 0xff00_00ff);
        fill_color_bars(&mut buf, 0, 2, 18);
    }
}
