// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! TI DSS7 ("K3" display subsystem): the DISPC register sequences one video
//! port and its pipelines need. SoC-independent: it gets the mapped base of
//! each region (`common`, one `vid`/`vidl1` pipeline, one `ovr`, one `vp`)
//! and nothing else; soc::am62p::dss is the AM62P's use of it.
//!
//! Written from the AM62Px TRM, TI SPRUJ83D (December 2023, revised January
//! 2026), section 12.9.1 "Display Subsystem (DSS)" and the register chapter
//! 14.8.9.1 ([`crate::soc::am62p::regs`]). What the code relies on, by
//! section:
//!
//! - Reset (12.9.1.4.1.3): "set the DSS_COMMON_DSS_SYSCONFIG[1] SOFTRESET
//!   bit to 0x1. The DSS_COMMON_DSS_SYSSTATUS[0] DISPC_FUNC_RESETDONE bit
//!   indicates that the software reset is complete ... the SOFTRESET bit is
//!   automatically reset. Software must ensure that the software reset
//!   completes before performing DISPC operations"; the video ports' reset
//!   completion is DISPC_VP_RESETDONE (bits 2:1 on this two-port DSS,
//!   14.8.9.1.2.3). So: wait for the power-on reset to be done, pulse
//!   SOFTRESET, then wait until SOFTRESET reads 0 *and* every RESETDONE bit
//!   reads 1 -- a RESETDONE that never dropped proves nothing.
//! - Shadow registers and GO (12.9.1.4.1.13, 14.8.9.1.2.271): the timing
//!   and pipeline registers are shadowed; "When the hardware reaches the end
//!   of the current frame and sees that the bit has been set by software, the
//!   new configuration is now the configuration used by the hardware".
//!   GOBIT reads 1 while "software has requested for synchronization ... and
//!   the hardware has not finished", 0 once it has. While the port is
//!   disabled nothing is latched by a frame end, so GO is not set then.
//! - Port enable (14.8.9.1.2.271 ENABLE): "wr:immediate"; clearing it stops
//!   the output "at the end of the frame".
//! - Timings (14.8.9.1.2.280-.282): PPL, LPP, HSW, HFP, HBP are "program to
//!   value minus one" with ranges 1..4096 (HSW, VSW 1..256); VFP and VBP are
//!   written as they are, 0..4095. PPL "In non-STALL mode, only values
//!   multiple of 8 pixels are valid" (STALLMODE is "Deprecated. Always
//!   write 0").
//! - Polarity (14.8.9.1.2.279): IHS/IVS/IEO 1 = active low; IPC 1 = data
//!   driven on the falling edge; ONOFF 1 + RF 1 = syncs driven on the rising
//!   edge; ALIGN 1 = HSYNC and VSYNC asserted together.
//! - DMA buffers (12.9.1.4.1.6.2): "For high threshold: BUFSIZE ... - 1. For
//!   low threshold: BUFSIZE ... - burst size", the largest burst being
//!   8x128 bits (12.9.1.4.1.6.1); "Preload value must be greater or equal to
//!   low threshold, and smaller or equal to high threshold"; BUFPRELOAD 0
//!   selects the PRELOAD register. PRELOAD is 12 bits.
//! - MFLAG (12.9.1.4.1.6.5): the pipeline's MFLAG rises below LT_MFLAG and
//!   falls above HT_MFLAG; MFLAG_CTRL 2 makes it dynamic. The thresholds are
//!   ours (the TRM gives none): LT a quarter, HT half of the buffer.
//! - Priorities (12.9.1.4.1.6.6): DSS_CBA_CFG "can only be modified by a
//!   secure host" -- never written here.
//! - Pixel fetch (12.9.1.4.1.6.1, 14.8.9.1.2.56-.60): address = base +
//!   x * bpp + y * (width * bpp + increment), where ROW_INC = 1 + increment
//!   and PIXEL_INC = 1 for consecutive pixels; BA is "Aligned on pixel size
//!   boundary"; BA_EXT holds address bits 47:32. PICTURE_SIZE and VID SIZE
//!   are minus one, 1..4096.
//! - Formats (12.9.1.4.1.7): RGB16-565 0x03, ARGB32-8888 0x07, xRGB32-8888
//!   0x27.
//! - Overlay (14.8.9.1.2.259): CHANNELIN 0 = VID, 1 = VIDL1.
//! - Interrupts (12.9.1.4.1.5, 14.8.9.1.2.4-.16): DISPC_IRQSTATUS VP_IRQ
//!   bit n = VPn+1, VID_IRQ bit 4 = VID, bit 5 = VIDL1; all status bits are
//!   write-1-to-clear; DISPC_IRQ_EOI "Write 1 to acknowledge a pulse IRQ"
//!   (the DSS lines are level, so EOI is harmless, written as documented).

use crate::env::Env;
use crate::errno::{EINVAL, EIO};
use crate::mmio::{field, get, Mmio};
use crate::soc::am62p::regs::{common as c, ovr as o, vid as v, vp as p};
use crate::video::VideoMode;
use crate::ylog;

/// Reset waits: the TRM gives no time; 5 ms in 10 us steps.
pub const RESET_WAIT_US: u32 = 5_000;
const POLL_STEP_US: u32 = 10;

/// The largest DMA burst, in 128-bit words (12.9.1.4.1.6.1).
pub const MAX_BURST_WORDS: u32 = 8;

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, val: u32) {
    m.write32(base + off as usize, val)
}

fn modify(m: &impl Mmio, base: usize, off: u32, mask: u32, val: u32) {
    m.modify32(base + off as usize, mask, val)
}

/// The output side of a video port.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// IPC: data driven on the falling edge of the pixel clock.
    pub data_on_falling_edge: bool,
}

/// The four timing registers of a video port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VpTiming {
    pub timing_h: u32,
    pub timing_v: u32,
    pub pol_freq: u32,
    pub size_screen: u32,
}

/// Pack a mode into VP_TIMING_H/V, VP_POL_FREQ and VP_SIZE_SCREEN
/// (14.8.9.1.2.279-.282). Fails with -EINVAL for a mode the fields cannot
/// hold, or a width the port cannot scan (not a multiple of 8).
pub fn vp_timing(mode: &VideoMode, out: &Output) -> Result<VpTiming, i32> {
    let m = mode;
    let in_range = |x: u32, lo: u32, hi: u32| (lo..=hi).contains(&x);
    let ok = in_range(m.hactive, 8, 4096)
        && m.hactive.is_multiple_of(8)
        && in_range(m.vactive, 1, 4096)
        && in_range(m.hsync, 1, 256)
        && in_range(m.hfront, 1, 4096)
        && in_range(m.hback, 1, 4096)
        && in_range(m.vsync, 1, 256)
        && m.vfront <= 4095
        && m.vback <= 4095
        && m.pixel_clock_hz != 0;
    if !ok {
        return Err(-EINVAL);
    }
    let timing_h = field(p::TIMING_H_HSW, m.hsync - 1)
        | field(p::TIMING_H_HFP, m.hfront - 1)
        | field(p::TIMING_H_HBP, m.hback - 1);
    let timing_v =
        field(p::TIMING_V_VSW, m.vsync - 1) | field(p::TIMING_V_VFP, m.vfront) | field(p::TIMING_V_VBP, m.vback);
    let mut pol = p::POL_ALIGN | p::POL_ONOFF | p::POL_RF;
    if !m.de_high {
        pol |= p::POL_IEO;
    }
    if out.data_on_falling_edge {
        pol |= p::POL_IPC;
    }
    if !m.hsync_high {
        pol |= p::POL_IHS;
    }
    if !m.vsync_high {
        pol |= p::POL_IVS;
    }
    let size_screen = field(p::SCREEN_PPL, m.hactive - 1) | field(p::SCREEN_LPP, m.vactive - 1);
    Ok(VpTiming { timing_h, timing_v, pol_freq: pol, size_screen })
}

fn wait(m: &impl Mmio, env: &impl Env, base: usize, off: u32, mask: u32, want: u32, us: u32) -> bool {
    let mut t = 0;
    loop {
        if rd(m, base, off) & mask == want {
            return true;
        }
        if t >= us {
            return false;
        }
        env.busy_wait_us(POLL_STEP_US);
        t += POLL_STEP_US;
    }
}

const RESETDONE: u32 = c::SYSSTATUS_DISPC_FUNC_RESETDONE | c::SYSSTATUS_DISPC_VP_RESETDONE;

/// Wait for the power-on reset, then pulse SOFTRESET and wait for it to
/// complete (see the module text). Logs and returns -EIO when either does
/// not finish.
pub fn soft_reset(m: &impl Mmio, env: &impl Env, common: usize) -> Result<(), i32> {
    if !wait(m, env, common, c::DSS_SYSSTATUS, RESETDONE, RESETDONE, RESET_WAIT_US) {
        ylog!(env, Err, "DSS not out of reset (SYSSTATUS {:#010x})", rd(m, common, c::DSS_SYSSTATUS));
        return Err(-EIO);
    }
    modify(m, common, c::DSS_SYSCONFIG, c::SYSCONFIG_SOFTRESET, c::SYSCONFIG_SOFTRESET);
    let cleared = wait(m, env, common, c::DSS_SYSCONFIG, c::SYSCONFIG_SOFTRESET, 0, RESET_WAIT_US);
    if !cleared || !wait(m, env, common, c::DSS_SYSSTATUS, RESETDONE, RESETDONE, RESET_WAIT_US) {
        ylog!(
            env,
            Err,
            "DSS soft reset did not complete (SYSCONFIG {:#010x} SYSSTATUS {:#010x})",
            rd(m, common, c::DSS_SYSCONFIG),
            rd(m, common, c::DSS_SYSSTATUS)
        );
        return Err(-EIO);
    }
    Ok(())
}

/// DSS_REVISION (diagnostics).
pub fn revision(m: &impl Mmio, common: usize) -> u32 {
    rd(m, common, c::DSS_REVISION)
}

/// The FIFO thresholds a pipeline with `bufsize` 128-bit words gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fifo {
    pub high: u32,
    pub low: u32,
    pub preload: u32,
    pub mflag_high: u32,
    pub mflag_low: u32,
}

/// The thresholds of the module text, or -EINVAL for a buffer too small to
/// hold one burst above its low threshold (or a preload past 12 bits).
pub fn fifo_for(bufsize: u32) -> Result<Fifo, i32> {
    if !(2 * MAX_BURST_WORDS..=0x1_0000).contains(&bufsize) {
        return Err(-EINVAL);
    }
    let high = bufsize - 1;
    let low = bufsize - MAX_BURST_WORDS;
    let preload = low;
    if preload > v::PRELOAD_MASK {
        return Err(-EINVAL);
    }
    Ok(Fifo { high, low, preload, mflag_high: bufsize / 2, mflag_low: bufsize / 4 })
}

/// Program a pipeline's DMA thresholds, preload and MFLAG thresholds from
/// its BUFSIZE. Returns what was programmed.
pub fn pipe_fifo_init(m: &impl Mmio, vid: usize) -> Result<Fifo, i32> {
    let size = get(v::BUFSIZE, rd(m, vid, v::BUF_SIZE_STATUS));
    let f = fifo_for(size)?;
    wr(m, vid, v::BUF_THRESHOLD, field(v::BUFHIGHTHRESHOLD, f.high) | field(v::BUFLOWTHRESHOLD, f.low));
    wr(m, vid, v::MFLAG_THRESHOLD, field(v::HT_MFLAG, f.mflag_high) | field(v::LT_MFLAG, f.mflag_low));
    wr(m, vid, v::PRELOAD, field(v::PRELOAD_MASK, f.preload));
    modify(m, vid, v::ATTRIBUTES, v::ATTR_BUFPRELOAD, 0);
    Ok(f)
}

/// MFLAG dynamic for the DMA initiator port, MFLAG_START 0 (the default).
pub fn mflag_init(m: &impl Mmio, common: usize) {
    modify(
        m,
        common,
        c::DISPC_GLOBAL_MFLAG_ATTRIBUTE,
        c::MFLAG_CTRL | c::MFLAG_START,
        field(c::MFLAG_CTRL, c::MFLAG_CTRL_DYNAMIC),
    );
}

/// Program a port's timings: 24 data lines, the four timing registers, no
/// gamma / CSC / gating (VP_CONFIG 0: every field is either off at 0 or
/// "Deprecated. Always write 0"), and a black background in its overlay.
pub fn vp_configure(m: &impl Mmio, vp: usize, ovr: usize, t: &VpTiming) {
    modify(m, vp, p::CONTROL, p::CTRL_DATALINES, field(p::CTRL_DATALINES, p::DATALINES_24));
    wr(m, vp, p::TIMING_H, t.timing_h);
    wr(m, vp, p::TIMING_V, t.timing_v);
    wr(m, vp, p::POL_FREQ, t.pol_freq);
    wr(m, vp, p::SIZE_SCREEN, t.size_screen);
    wr(m, vp, p::CONFIG, 0);
    wr(m, ovr, o::DEFAULT_COLOR, 0);
    wr(m, ovr, o::DEFAULT_COLOR2, 0);
}

/// A plane's buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plane {
    /// Bus address of the first pixel (48 bits).
    pub addr: u64,
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    /// A `regs::vid::FMT_*` code.
    pub format: u32,
}

/// Bytes per pixel of a format this driver accepts.
pub fn bytes_per_pixel(format: u32) -> Option<u32> {
    match format {
        v::FMT_RGB565 => Some(2),
        v::FMT_ARGB8888 | v::FMT_XRGB8888 => Some(4),
        _ => None,
    }
}

/// Whether [`plane_configure`] would take this plane (-EINVAL if not).
pub fn check_plane(pl: &Plane) -> Result<(), i32> {
    plane_fields(pl).map(|_| ())
}

/// The fields of a plane, or -EINVAL.
fn plane_fields(pl: &Plane) -> Result<(u32, u32), i32> {
    let cpp = bytes_per_pixel(pl.format).ok_or(-EINVAL)?;
    let in_range = |x: u32| (1..=4096).contains(&x);
    let line = pl.width.checked_mul(cpp).ok_or(-EINVAL)?;
    if !in_range(pl.width)
        || !in_range(pl.height)
        || pl.addr == 0
        || pl.addr >> 48 != 0
        || !pl.addr.is_multiple_of(u64::from(cpp))
        || pl.stride_bytes < line
    {
        return Err(-EINVAL);
    }
    let size = field(v::MEMSIZEX, pl.width - 1) | field(v::MEMSIZEY, pl.height - 1);
    let row_inc = 1 + (pl.stride_bytes - line);
    Ok((size, row_inc))
}

/// Only the address of a plane: BA_0/BA_1 and their extensions.
pub fn plane_set_address(m: &impl Mmio, vid: usize, addr: u64) {
    let hi = field(v::BA_EXT, (addr >> 32) as u32);
    wr(m, vid, v::BA_0, addr as u32);
    wr(m, vid, v::BA_EXT_0, hi);
    wr(m, vid, v::BA_1, addr as u32);
    wr(m, vid, v::BA_EXT_1, hi);
}

/// Point a pipeline at a buffer, unscaled, opaque global alpha, straight
/// alpha. `full` is the VID pipeline (it has the scaler's VID_SIZE and
/// RESIZEENABLE); VIDL1 has neither. Does not enable the pipeline.
pub fn plane_configure(m: &impl Mmio, vid: usize, pl: &Plane, full: bool) -> Result<(), i32> {
    let (size, row_inc) = plane_fields(pl)?;
    modify(m, vid, v::ATTRIBUTES, v::ATTR_FORMAT, field(v::ATTR_FORMAT, pl.format));
    plane_set_address(m, vid, pl.addr);
    wr(m, vid, v::PICTURE_SIZE, size);
    wr(m, vid, v::PIXEL_INC, 1);
    wr(m, vid, v::ROW_INC, row_inc);
    if full {
        wr(m, vid, v::SIZE, field(v::SIZEX, pl.width - 1) | field(v::SIZEY, pl.height - 1));
        modify(m, vid, v::ATTRIBUTES, v::ATTR_RESIZEENABLE, 0);
    }
    wr(m, vid, v::GLOBAL_ALPHA, 0xff);
    modify(m, vid, v::ATTRIBUTES, v::ATTR_PREMULTIPLYALPHA, 0);
    Ok(())
}

pub fn plane_enable(m: &impl Mmio, vid: usize, on: bool) {
    modify(m, vid, v::ATTRIBUTES, v::ATTR_ENABLE, if on { v::ATTR_ENABLE } else { 0 });
}

pub fn plane_premultiplied(m: &impl Mmio, vid: usize, on: bool) {
    modify(m, vid, v::ATTRIBUTES, v::ATTR_PREMULTIPLYALPHA, if on { v::ATTR_PREMULTIPLYALPHA } else { 0 });
}

/// Connect `channel` (`regs::ovr::CHANNEL_*`) to overlay layer `layer` at
/// (0, 0), enabled or not.
pub fn ovr_layer(m: &impl Mmio, ovr: usize, layer: u32, channel: u32, on: bool) {
    let mask = o::ATTR_CHANNELIN | o::ATTR_POSX | o::ATTR_POSY | o::ATTR_ENABLE;
    let val = field(o::ATTR_CHANNELIN, channel) | if on { o::ATTR_ENABLE } else { 0 };
    modify(m, ovr, o::attributes(layer), mask, val);
}

/// What [`go`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Go {
    /// The port is off: the registers take effect when it is enabled.
    NotRunning,
    /// GO was clear and is now set.
    Armed,
    /// GO was still set from an earlier commit: these writes latch with it.
    RodeAlong,
}

/// Request the shadow registers of a running port be latched at the next
/// frame end. Never waits (two writers share one GO; see soc::am62p::dss).
pub fn go(m: &impl Mmio, vp: usize) -> Go {
    let ctl = rd(m, vp, p::CONTROL);
    if ctl & p::CTRL_ENABLE == 0 {
        Go::NotRunning
    } else if ctl & p::CTRL_GOBIT != 0 {
        Go::RodeAlong
    } else {
        modify(m, vp, p::CONTROL, p::CTRL_GOBIT, p::CTRL_GOBIT);
        Go::Armed
    }
}

/// GO set and not yet consumed by the hardware.
pub fn go_pending(m: &impl Mmio, vp: usize) -> bool {
    rd(m, vp, p::CONTROL) & p::CTRL_GOBIT != 0
}

pub fn vp_enable(m: &impl Mmio, vp: usize, on: bool) {
    modify(m, vp, p::CONTROL, p::CTRL_ENABLE, if on { p::CTRL_ENABLE } else { 0 });
}

pub fn vp_running(m: &impl Mmio, vp: usize) -> bool {
    rd(m, vp, p::CONTROL) & p::CTRL_ENABLE != 0
}

/// `VP_IRQSTATUS_n` / `VP_IRQENABLE_n` of port `idx` (0 = VP1).
const fn vp_status(idx: u32) -> u32 {
    c::VP_IRQSTATUS_0 + 4 * idx
}
const fn vp_enable_reg(idx: u32) -> u32 {
    c::VP_IRQENABLE_0 + 4 * idx
}
/// `VID_IRQSTATUS_n` / `VID_IRQENABLE_n`: 0 = VID, 1 = VIDL1.
const fn vid_status(pipe: u32) -> u32 {
    c::VID_IRQSTATUS_0 + 4 * pipe
}
const fn vid_enable_reg(pipe: u32) -> u32 {
    c::VID_IRQENABLE_0 + 4 * pipe
}
/// DISPC_IRQSTATUS bit of VP `idx` and of pipeline `pipe`.
const fn top_vp(idx: u32) -> u32 {
    1 << (c::IRQ_VP.trailing_zeros() + idx)
}
const fn top_vid(pipe: u32) -> u32 {
    1 << (c::IRQ_VID.trailing_zeros() + pipe)
}

/// Clear and enable the events the driver counts: VSYNC and SYNC LOST of
/// port `vp_idx`, buffer underflow of both pipelines, and their first-level
/// bits.
pub fn irq_enable(m: &impl Mmio, common: usize, vp_idx: u32) {
    wr(m, common, vp_status(vp_idx), u32::MAX);
    wr(m, common, vp_enable_reg(vp_idx), c::VP_IRQ_VSYNC | c::VP_IRQ_SYNCLOST);
    for pipe in 0..2 {
        wr(m, common, vid_status(pipe), u32::MAX);
        wr(m, common, vid_enable_reg(pipe), c::VID_IRQ_BUFFERUNDERFLOW);
    }
    wr(m, common, c::DISPC_IRQENABLE_SET, top_vp(vp_idx) | top_vid(0) | top_vid(1));
}

/// Events of one interrupt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Events {
    pub vsync: bool,
    pub sync_lost: bool,
    /// Buffer underflow of VID (0) and VIDL1 (1).
    pub underflow: [bool; 2],
}

/// Read, acknowledge (write-1-to-clear exactly what was read) and decode the
/// pending events of port `vp_idx`, then EOI.
pub fn irq_service(m: &impl Mmio, common: usize, vp_idx: u32) -> Events {
    let top = rd(m, common, c::DISPC_IRQSTATUS);
    let mut ev = Events::default();
    if top & top_vp(vp_idx) != 0 {
        let st = rd(m, common, vp_status(vp_idx));
        wr(m, common, vp_status(vp_idx), st);
        ev.vsync = st & c::VP_IRQ_VSYNC != 0;
        ev.sync_lost = st & c::VP_IRQ_SYNCLOST != 0;
    }
    for pipe in 0..2u32 {
        if top & top_vid(pipe) != 0 {
            let st = rd(m, common, vid_status(pipe));
            wr(m, common, vid_status(pipe), st);
            ev.underflow[pipe as usize] = st & c::VID_IRQ_BUFFERUNDERFLOW != 0;
        }
    }
    wr(m, common, c::DISPC_IRQSTATUS, top);
    wr(m, common, c::DISPC_IRQ_EOI, c::IRQ_EOI_EOI);
    ev
}

/// Without an interrupt line: a VSYNC pending in port `vp_idx`'s status is
/// taken (acknowledged) and reported.
pub fn poll_vsync(m: &impl Mmio, common: usize, vp_idx: u32) -> bool {
    let st = rd(m, common, vp_status(vp_idx));
    if st & c::VP_IRQ_VSYNC == 0 {
        return false;
    }
    wr(m, common, vp_status(vp_idx), st);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Toradex Riverdi RVT70HSDNWCA0 (1024x600) the Verdin AM62P drives:
    /// the board devicetree's `&dss1` mode.
    pub const RVT70: VideoMode = VideoMode {
        pixel_clock_hz: 51_200_000,
        hactive: 1024,
        hfront: 160,
        hsync: 5,
        hback: 160,
        vactive: 600,
        vfront: 12,
        vsync: 10,
        vback: 23,
        hsync_high: false,
        vsync_high: false,
        de_high: true,
    };

    /// Ported from driver/tests/display_calc (test_dss_vp_timing): the same
    /// register values as dss_vp_regs_from_mode, now from the TRM's fields.
    #[test]
    fn vp_timing_rvt70_matches_display_calc() {
        let t = vp_timing(&RVT70, &Output::default()).unwrap();
        assert_eq!(t.timing_h, 4 | (159 << 8) | (159 << 20));
        assert_eq!(t.timing_v, 9 | (12 << 8) | (23 << 20));
        assert_eq!(t.size_screen, 1023 | (599 << 16));
        assert_eq!(t.pol_freq, (1 << 18) | (1 << 17) | (1 << 16) | (1 << 13) | (1 << 12));
    }

    #[test]
    fn vp_timing_polarities_and_limits() {
        let hi = VideoMode { hsync_high: true, vsync_high: true, de_high: false, ..RVT70 };
        let t = vp_timing(&hi, &Output { data_on_falling_edge: true }).unwrap();
        assert_eq!(t.pol_freq, p::POL_ALIGN | p::POL_ONOFF | p::POL_RF | p::POL_IEO | p::POL_IPC);
        // Field limits from 14.8.9.1.2.280-.282.
        let max = VideoMode { hsync: 256, hfront: 4096, hback: 4096, vsync: 256, vfront: 4095, vback: 4095, ..RVT70 };
        let t = vp_timing(&max, &Output::default()).unwrap();
        assert_eq!(t.timing_h, 0xffff_ffff);
        assert_eq!(t.timing_v, 0xffff_ffff);
        for bad in [
            VideoMode { hsync: 257, ..RVT70 },
            VideoMode { hfront: 0, ..RVT70 },
            VideoMode { hback: 4097, ..RVT70 },
            VideoMode { vsync: 0, ..RVT70 },
            VideoMode { vfront: 4096, ..RVT70 },
            VideoMode { hactive: 1020, ..RVT70 }, // not a multiple of 8
            VideoMode { vactive: 4097, ..RVT70 },
            VideoMode { pixel_clock_hz: 0, ..RVT70 },
        ] {
            assert_eq!(vp_timing(&bad, &Output::default()), Err(-EINVAL), "{bad:?}");
        }
        // VFP and VBP may be 0 (0..4095).
        assert!(vp_timing(&VideoMode { vfront: 0, vback: 0, ..RVT70 }, &Output::default()).is_ok());
    }

    #[test]
    fn fifo_thresholds_follow_the_trm() {
        // BUFSIZE reset value A00h (14.8.9.1.2.45).
        let f = fifo_for(0xa00).unwrap();
        assert_eq!((f.high, f.low, f.preload), (0x9ff, 0x9f8, 0x9f8));
        assert!(f.low <= f.preload && f.preload <= f.high);
        assert!(f.mflag_low < f.mflag_high && f.mflag_high <= f.low);
        assert_eq!(fifo_for(15), Err(-EINVAL));
        // A merged buffer whose preload would not fit 12 bits.
        assert_eq!(fifo_for(0x2000), Err(-EINVAL));
    }
}
