// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! LCDIFv3, the i.MX 8M Plus display controller (LCDIF1 0x32E80000, LCDIF2
//! 0x32E90000, LCDIF3 0x32FC6000: one IP, three instances). One linear
//! layer scanned out with a mode's timings; the base address comes from the
//! devicetree, so the Verdin's LCDIF1 (group E step 3) and the
//! Armadillo-X2's LCDIF3 (driver/docs/x2-hdmi-cold.md) are the same code.
//!
//! Written from the i.MX 8M Plus Applications Processor Reference Manual,
//! IMX8MPRM Rev. 3, 08/2024 ("RM"), section 13.3 "LCD Interface (LCDIF)";
//! register names and bits from hal_nxp `PERI_LCDIF.h`
//! ([`crate::soc::imx8mp::regs::lcdif`]). What the code relies on:
//!
//! - Reset: `CTRL` resets to 0x8000_0000, i.e. `SW_RESET` is set out of a
//!   power-on reset (13.3.3.1.1 memory map, reset value), and `SW_RESET`
//!   "1b - All LCDIF internal registers are forced into their reset state.
//!   User registers are not affected" (13.3.3.1.2.4). So a cold start must
//!   clear it, and a pulse of it does not lose the programming.
//! - Order: the software flow is display parameters, `DISP_PARA[DISP_ON]`,
//!   the DMA control parameters, `CTRLDESCL0_5[EN]`; an update of the DMA
//!   parameters is followed by `CTRLDESCL0_5[SHADOW_LOAD_EN]` (13.3.2.6,
//!   Figure 13-7).
//! - `SHADOW_LOAD_EN` is written 1 "after all DMA control registers are
//!   written"; the shadowed registers load "on VSYNC of next frame" and the
//!   hardware clears the bit when they are loaded (13.3.3.1.16.4). So a bit
//!   that clears proves frames are being timed -- the pixel clock runs --
//!   and a bit that stays set within two frames says it does not.
//! - Timing fields: porches and pulse widths have "a minimum value of 1"
//!   (13.3.3.1.5-7); `DISP_SIZE`, `HSYN_PARA`, `VSYN_PARA`,
//!   `VSYN_HSYN_WIDTH` are 16-bit fields.
//! - Polarity: `INV_HS`/`INV_VS`/`INV_DE` 0 = active high; `INV_PXCK`
//!   0 = "Display samples data on the falling edge", 1 = rising edge
//!   (13.3.3.1.2.4).
//! - Layer: `PITCH` is bytes between lines, "SW should align to 64B boundry"
//!   (13.3.3.1.13.4); the address "should be 64-bit aligned", 32 + 4 bits
//!   (13.3.3.1.14-15); `BPP` 1001b = 32 bpp ARGB8888, 0100b = RGB565,
//!   1010b = ABGR8888 (13.3.3.1.16.4). The layer width must be a multiple of
//!   the pixels per 32 bits (13.3.3.1.12.4): any width at 32 bpp.
//! - `DISP_PARA[DISP_MODE]` 01b = "Test Mode1 (BGND Color Display)": the
//!   background colour with no memory fetch (13.3.3.1.3.3), used for staging.
//! - `PANIC0_THRES`: thresholds in units of 128 bits of the input FIFO, 0..511
//!   (13.3.3.1.24.4).
//!
//! - Interrupts: `INT_ENABLE_D0[VS_BLANK_EN]` flags "vertical blanking
//!   period", `[VSYNC_EN]` "the beginning of a frame" (13.3.3.1.9); the
//!   status register is write-1-to-clear (13.3.3.1.8). LCDIF1 is Arm
//!   interrupt 5 (Table 7-1); LCDIF3's goes through the HDMI block's
//!   interrupt steering (Table 7-1, irq_steer input 8).
//!
//! Not stated in IMX8MPRM Rev. 3: the AXI burst fields that drivers outside
//! the RM put in `CTRLDESCL0_3` bits 22:20 and 17:16 (the RM says 31:16 are
//! reserved, 13.3.3.1.13.4). They are never set here unless the caller passes
//! them ([`Layer::desc3_extra`], measured from a board dump; risk R1 of
//! docs/source-form.md group E). NXP's own SDK driver (hal_nxp
//! `fsl_lcdifv3.c`, BSD-3, `LCDIFV3_SetLayerBufferConfig`) writes the stride
//! alone there, i.e. 0 in those bits, which is the default here. The same
//! driver's `LCDIFV3_EnablePlanePanic` gives the panic thresholds as "1/3
//! FIFO" and "2/3 FIFO" of 511 (170, 340), the values the Verdin's
//! devicetree passes.

use crate::env::Env;
use crate::errno::{EINVAL, EIO, ETIMEDOUT};
use crate::mmio::{field, get, Mmio};
use crate::soc::imx8mp::regs::lcdif as r;
use crate::video::VideoMode;

/// `CTRLDESCL0_5[BPP]` values (13.3.3.1.16.4).
pub const BPP_RGB565: u32 = 0b0100;
pub const BPP_ARGB8888: u32 = 0b1001;
pub const BPP_ABGR8888: u32 = 0b1010;

/// `DISP_PARA[DISP_MODE]` (13.3.3.1.3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DispMode {
    Normal = 0,
    /// The background colour (`BGND_R/G/B`) everywhere.
    Background = 1,
    ColumnBars = 2,
    RowBars = 3,
}

/// What a running layer scans, read back from the controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scanout {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bytes_pp: u32,
    pub phys: u64,
}

/// The layer to scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub phys: u64,
    /// Bytes per line; 0 = width * 4.
    pub pitch: u32,
    /// OR-ed into `CTRLDESCL0_3` above `PITCH`: bits the RM marks reserved
    /// that a board dump shows set (0 = RM only). See the module text.
    pub desc3_extra: u32,
}

/// The output side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    /// `CTRL[INV_PXCK]`: the consumer samples on the rising edge.
    pub pxck_rising: bool,
    /// Drive HSYNC/VSYNC/DE active low (`INV_*`). The HDMI path drives all
    /// three active high and lets the next block re-time them.
    pub hsync_low: bool,
    pub vsync_low: bool,
    pub de_low: bool,
}

/// Bounds of the waits: a shadow load completes at the next VSYNC, so two
/// frames at the slowest mode the drivers run (24 Hz) is the limit.
pub const FRAME_WAIT_US: u32 = 2 * 41_667;
const POLL_STEP_US: u32 = 50;

/// The largest valid width/height read back (sanity of a register image).
const SANE_MAX: u32 = 8192;

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v)
}

/// Layer enabled and display on: the controller is scanning.
pub fn is_running(m: &impl Mmio, base: usize) -> bool {
    rd(m, base, r::CTRLDESCL0_5) & r::ctrldescl0_5::EN_MASK != 0
        && rd(m, base, r::DISP_PARA) & r::disp_para::DISP_ON_MASK != 0
}

/// `CTRLDESCL0_5` as it stands (diagnostics).
pub fn layer_control(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, r::CTRLDESCL0_5)
}

/// The layer the controller is scanning; `None` when it is not running or
/// its registers do not describe a plausible layer.
pub fn read_scanout(m: &impl Mmio, base: usize) -> Option<Scanout> {
    if !is_running(m, base) {
        return None;
    }
    let bytes_pp = match get(r::ctrldescl0_5::BPP_MASK, rd(m, base, r::CTRLDESCL0_5)) {
        BPP_ARGB8888 | BPP_ABGR8888 => 4,
        BPP_RGB565 => 2,
        _ => return None,
    };
    let d1 = rd(m, base, r::CTRLDESCL0_1);
    let width = get(r::ctrldescl0_1::WIDTH_MASK, d1);
    let height = get(r::ctrldescl0_1::HEIGHT_MASK, d1);
    let pitch = get(r::ctrldescl0_3::PITCH_MASK, rd(m, base, r::CTRLDESCL0_3));
    if width == 0 || height == 0 || width > SANE_MAX || height > SANE_MAX || pitch < width * bytes_pp {
        return None;
    }
    let hi = get(r::ctrldescl_high0_4::ADDR_HIGH_MASK, rd(m, base, r::CTRLDESCL_HIGH0_4));
    let phys = (u64::from(hi) << 32) | u64::from(rd(m, base, r::CTRLDESCL_LOW0_4));
    Some(Scanout { width, height, pitch, bytes_pp, phys })
}

fn check_layer(mode: &VideoMode, l: &Layer) -> Result<u32, i32> {
    let pitch = if l.pitch == 0 { mode.hactive * 4 } else { l.pitch };
    if pitch < mode.hactive * 4 || pitch > 0xffff || !pitch.is_multiple_of(64) {
        return Err(-EINVAL);
    }
    if !l.phys.is_multiple_of(8) || l.phys >> 36 != 0 {
        return Err(-EINVAL);
    }
    if l.desc3_extra & r::ctrldescl0_3::PITCH_MASK != 0 {
        return Err(-EINVAL);
    }
    Ok(pitch)
}

fn write_addr(m: &impl Mmio, base: usize, phys: u64) {
    wr(m, base, r::CTRLDESCL_LOW0_4, phys as u32);
    wr(m, base, r::CTRLDESCL_HIGH0_4, field(r::ctrldescl_high0_4::ADDR_HIGH_MASK, (phys >> 32) as u32));
}

/// Pulse `CTRL[SW_RESET]` through the SET/CLR aliases and check it reads
/// clear afterwards (it is set out of power-on reset; see the module text).
pub fn soft_reset(m: &impl Mmio, base: usize) -> Result<(), i32> {
    wr(m, base, r::CTRL_SET, r::ctrl_set::SW_RESET_MASK);
    let _ = rd(m, base, r::CTRL);
    wr(m, base, r::CTRL_CLR, r::ctrl_clr::SW_RESET_MASK);
    if rd(m, base, r::CTRL) & r::ctrl::SW_RESET_MASK != 0 {
        return Err(-EIO);
    }
    Ok(())
}

/// Program everything but the enables: polarity, timings, the layer, CSC
/// bypassed, RGB line order, normal mode. The display stays off and the layer
/// disabled; [`enable`] starts them. Returns the pitch used.
pub fn configure(m: &impl Mmio, base: usize, mode: &VideoMode, out: &Output, layer: &Layer) -> Result<u32, i32> {
    mode.check()?;
    let pitch = check_layer(mode, layer)?;
    soft_reset(m, base)?;
    let ctrl = if out.hsync_low { r::ctrl::INV_HS_MASK } else { 0 }
        | if out.vsync_low { r::ctrl::INV_VS_MASK } else { 0 }
        | if out.de_low { r::ctrl::INV_DE_MASK } else { 0 }
        | if out.pxck_rising { r::ctrl::INV_PXCK_MASK } else { 0 };
    wr(m, base, r::CTRL, ctrl);
    // LINE_PATTERN 0000b RGB, DISP_MODE normal, DISP_ON 0, black background
    wr(m, base, r::DISP_PARA, 0);
    wr(
        m,
        base,
        r::DISP_SIZE,
        field(r::disp_size::DELTA_Y_MASK, mode.vactive) | field(r::disp_size::DELTA_X_MASK, mode.hactive),
    );
    wr(m, base, r::HSYN_PARA, field(r::hsyn_para::BP_H_MASK, mode.hback) | field(r::hsyn_para::FP_H_MASK, mode.hfront));
    wr(m, base, r::VSYN_PARA, field(r::vsyn_para::BP_V_MASK, mode.vback) | field(r::vsyn_para::FP_V_MASK, mode.vfront));
    wr(
        m,
        base,
        r::VSYN_HSYN_WIDTH,
        field(r::vsyn_hsyn_width::PW_V_MASK, mode.vsync) | field(r::vsyn_hsyn_width::PW_H_MASK, mode.hsync),
    );
    wr(
        m,
        base,
        r::CTRLDESCL0_1,
        field(r::ctrldescl0_1::HEIGHT_MASK, mode.vactive) | field(r::ctrldescl0_1::WIDTH_MASK, mode.hactive),
    );
    wr(m, base, r::CTRLDESCL0_3, layer.desc3_extra | field(r::ctrldescl0_3::PITCH_MASK, pitch));
    // the layer off, ARGB8888 (X ignored: no blending with one layer)
    wr(m, base, r::CTRLDESCL0_5, field(r::ctrldescl0_5::BPP_MASK, BPP_ARGB8888));
    wr(m, base, r::CSC0_CTRL, r::csc0_ctrl::BYPASS_MASK);
    write_addr(m, base, layer.phys);
    Ok(pitch)
}

fn wait_shadow_loaded(m: &impl Mmio, base: usize, env: &impl Env) -> Result<(), i32> {
    let mut waited = 0;
    loop {
        if rd(m, base, r::CTRLDESCL0_5) & r::ctrldescl0_5::SHADOW_LOAD_EN_MASK == 0 {
            return Ok(());
        }
        if waited >= FRAME_WAIT_US {
            return Err(-ETIMEDOUT);
        }
        env.busy_wait_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    }
}

/// FIFO panic watermarks, in 128-bit units of the input FIFO (0..=511).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panic {
    pub low: u32,
    pub high: u32,
}

/// Start scanning (13.3.2.6): panic thresholds, `DISP_ON`, then the layer
/// with a shadow load, and wait for that load. `Err(-ETIMEDOUT)` means no
/// frame boundary came within two frames: the pixel clock is not running.
pub fn enable(m: &impl Mmio, base: usize, env: &impl Env, panic: Option<Panic>) -> Result<(), i32> {
    if let Some(p) = panic {
        if p.low > 511 || p.high > 511 || p.low > p.high {
            return Err(-EINVAL);
        }
        wr(
            m,
            base,
            r::PANIC0_THRES,
            field(r::panic0_thres::PANIC_THRES_LOW_MASK, p.low) | field(r::panic0_thres::PANIC_THRES_HIGH_MASK, p.high),
        );
        wr(m, base, r::INT_ENABLE_D1, r::int_enable_d1::PLANE_PANIC_EN_MASK);
    }
    let dp = rd(m, base, r::DISP_PARA);
    wr(m, base, r::DISP_PARA, dp | r::disp_para::DISP_ON_MASK);
    let d5 = rd(m, base, r::CTRLDESCL0_5);
    wr(m, base, r::CTRLDESCL0_5, d5 | r::ctrldescl0_5::EN_MASK | r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
    wait_shadow_loaded(m, base, env)
}

/// Point the running layer at `phys` and wait for the controller to latch it.
pub fn set_fb(m: &impl Mmio, base: usize, env: &impl Env, phys: u64) -> Result<(), i32> {
    if !phys.is_multiple_of(8) || phys >> 36 != 0 {
        return Err(-EINVAL);
    }
    write_addr(m, base, phys);
    let d5 = rd(m, base, r::CTRLDESCL0_5);
    wr(m, base, r::CTRLDESCL0_5, d5 | r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
    wait_shadow_loaded(m, base, env)
}

/// Change the running layer's pitch and format (adopting a layer the
/// previous owner set up differently); takes effect with the next
/// [`set_fb`]'s shadow load.
pub fn set_layout(m: &impl Mmio, base: usize, pitch: u32, bpp: u32) -> Result<(), i32> {
    if pitch == 0 || pitch > 0xffff || !matches!(bpp, BPP_RGB565 | BPP_ARGB8888 | BPP_ABGR8888) {
        return Err(-EINVAL);
    }
    let d3 = rd(m, base, r::CTRLDESCL0_3);
    wr(m, base, r::CTRLDESCL0_3, (d3 & !r::ctrldescl0_3::PITCH_MASK) | field(r::ctrldescl0_3::PITCH_MASK, pitch));
    let d5 = rd(m, base, r::CTRLDESCL0_5);
    wr(m, base, r::CTRLDESCL0_5, (d5 & !r::ctrldescl0_5::BPP_MASK) | field(r::ctrldescl0_5::BPP_MASK, bpp));
    Ok(())
}

/// Stop: the layer off through a shadow load (so it stops at a frame
/// boundary), then the display, and the panic interrupt. A controller whose
/// clock has already stopped cannot complete the load; that is reported, and
/// the display is turned off anyway.
pub fn disable(m: &impl Mmio, base: usize, env: &impl Env) -> Result<(), i32> {
    let d5 = rd(m, base, r::CTRLDESCL0_5);
    wr(m, base, r::CTRLDESCL0_5, (d5 & !r::ctrldescl0_5::EN_MASK) | r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
    let res = wait_shadow_loaded(m, base, env);
    let dp = rd(m, base, r::DISP_PARA);
    wr(m, base, r::DISP_PARA, dp & !r::disp_para::DISP_ON_MASK);
    wr(m, base, r::INT_ENABLE_D1, 0);
    res
}

/// Select a test mode and its background colour (0xRRGGBB); `Normal` scans
/// the layer again.
pub fn test_mode(m: &impl Mmio, base: usize, mode: DispMode, rgb: u32) {
    let dp = rd(m, base, r::DISP_PARA);
    let keep = dp
        & !(r::disp_para::DISP_MODE_MASK
            | r::disp_para::BGND_R_MASK
            | r::disp_para::BGND_G_MASK
            | r::disp_para::BGND_B_MASK);
    wr(m, base, r::DISP_PARA, keep | field(r::disp_para::DISP_MODE_MASK, mode as u32) | (rgb & 0x00ff_ffff));
}

/// `INT_STATUS_D0` (VSYNC, UNDERRUN, ...), then cleared (write 1 to clear,
/// 13.3.3.1.8).
pub fn take_status(m: &impl Mmio, base: usize) -> u32 {
    let s = rd(m, base, r::INT_STATUS_D0);
    if s != 0 {
        wr(m, base, r::INT_STATUS_D0, s);
    }
    s
}

/// The vertical-blanking interrupt on or off: `INT_ENABLE_D0[VS_BLANK_EN]`,
/// "Enable Interrupt flag to indicate vertical blanking period"
/// (13.3.3.1.9). The other enables are kept.
pub fn set_vblank_irq(m: &impl Mmio, base: usize, on: bool) {
    let v = rd(m, base, r::INT_ENABLE_D0);
    let n = if on { v | r::int_enable_d0::VS_BLANK_EN_MASK } else { v & !r::int_enable_d0::VS_BLANK_EN_MASK };
    wr(m, base, r::INT_ENABLE_D0, n);
}

/// [`set_fb`] for a caller with no timer (PRE_KERNEL_1, before the system
/// clock runs): the shadow load is waited for by counting reads of
/// `CTRLDESCL0_5`, at most `max_reads` of them.
pub fn set_fb_polled(m: &impl Mmio, base: usize, phys: u64, max_reads: u32) -> Result<(), i32> {
    if !phys.is_multiple_of(8) || phys >> 36 != 0 {
        return Err(-EINVAL);
    }
    write_addr(m, base, phys);
    let d5 = rd(m, base, r::CTRLDESCL0_5);
    wr(m, base, r::CTRLDESCL0_5, d5 | r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
    for _ in 0..max_reads {
        if rd(m, base, r::CTRLDESCL0_5) & r::ctrldescl0_5::SHADOW_LOAD_EN_MASK == 0 {
            return Ok(());
        }
    }
    Err(-ETIMEDOUT)
}

/// `CTRLDESCL0_5[BPP]` of the running layer.
pub fn bpp_code(m: &impl Mmio, base: usize) -> u32 {
    get(r::ctrldescl0_5::BPP_MASK, rd(m, base, r::CTRLDESCL0_5))
}

/// A layer the previous kernel left scanning, taken over: what it was, to
/// give it back, and whether our pitch and format are in place yet.
///
/// The policy (src/display/imx8mp_lcdif_adopt.c, 2026-10-01/02): the
/// previous owner's address, pitch and format are captured once, at the
/// first adopt, so a second adopt (which finds our own buffer) cannot make
/// [`adopt_release`] hand back ours. Adopting without an address writes
/// nothing: physical address 0 is boot ROM, CAAM secure RAM, OCRAM_S and the
/// M7's TCM, and scanning it wedged the Armadillo-X2's interconnect. The
/// first buffer of ours then brings our pitch and XRGB8888 with it, in the
/// same shadow load as its address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Adopted {
    pub prev_phys: u64,
    pub prev_desc3: u32,
    pub prev_desc5: u32,
    /// The pitch of our buffers.
    pub our_pitch: u32,
    /// The previous owner's layer is captured.
    pub held: bool,
    /// The layer has our pitch and format.
    pub ours: bool,
}

fn apply_our_layout(m: &impl Mmio, base: usize, a: &mut Adopted) {
    wr(
        m,
        base,
        r::CTRLDESCL0_3,
        (a.prev_desc3 & !r::ctrldescl0_3::PITCH_MASK) | field(r::ctrldescl0_3::PITCH_MASK, a.our_pitch),
    );
    let d5 = a.prev_desc5 & !(r::ctrldescl0_5::BPP_MASK | r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
    wr(m, base, r::CTRLDESCL0_5, d5 | field(r::ctrldescl0_5::BPP_MASK, BPP_ARGB8888));
    a.ours = true;
}

/// Take over a running layer of `expect` = (width, height): `Err(-ENODEV)`
/// when nothing runs, `Err(-EINVAL)` when it runs another size. With
/// `fb == 0` nothing is written; otherwise the layer moves to `fb` (XRGB8888,
/// `pitch` bytes a line, 0 = width x 4).
pub fn adopt(
    m: &impl Mmio,
    base: usize,
    env: &impl Env,
    a: &mut Adopted,
    expect: (u32, u32),
    fb: u64,
    pitch: u32,
) -> Result<Scanout, i32> {
    let s = read_scanout(m, base).ok_or(-crate::errno::ENODEV)?;
    if (s.width, s.height) != expect {
        return Err(-EINVAL);
    }
    let pitch = if pitch == 0 { s.width * 4 } else { pitch };
    if pitch < s.width * 4 || pitch > 0xffff {
        return Err(-EINVAL);
    }
    if !a.held {
        a.prev_phys = s.phys;
        a.prev_desc3 = rd(m, base, r::CTRLDESCL0_3);
        a.prev_desc5 = rd(m, base, r::CTRLDESCL0_5);
        a.held = true;
    }
    a.our_pitch = pitch;
    if fb == 0 {
        a.ours = false;
        return Ok(s);
    }
    if !fb.is_multiple_of(8) || fb >> 36 != 0 {
        return Err(-EINVAL);
    }
    apply_our_layout(m, base, a);
    set_fb(m, base, env, fb)?;
    Ok(s)
}

/// Flip an adopted layer; the first buffer of ours brings our layout.
pub fn adopt_set_fb(m: &impl Mmio, base: usize, env: &impl Env, a: &mut Adopted, fb: u64) -> Result<(), i32> {
    if !a.held {
        return Err(-crate::errno::ENODEV);
    }
    if fb == 0 {
        return Err(-EINVAL); // never scan out of physical zero; see Adopted
    }
    if !a.ours {
        apply_our_layout(m, base, a);
    }
    set_fb(m, base, env, fb)
}

/// Give the previous owner its buffer, pitch and format back.
pub fn adopt_release(m: &impl Mmio, base: usize, env: &impl Env, a: &mut Adopted) -> Result<(), i32> {
    if !a.held {
        return Ok(());
    }
    wr(m, base, r::CTRLDESCL0_3, a.prev_desc3);
    wr(m, base, r::CTRLDESCL0_5, a.prev_desc5 & !r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
    a.ours = false;
    set_fb(m, base, env, a.prev_phys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TestEnv;
    use crate::model_hdmi::{HdmiFaults, HdmiModel, LCDIF};
    use crate::video::WUXGA_60_RB;

    const OUT: Output = Output { pxck_rising: false, hsync_low: false, vsync_low: false, de_low: false };
    const LAYER: Layer = Layer { phys: 0x9000_0000, pitch: 0, desc3_extra: 0 };

    #[test]
    fn layout_is_the_rm() {
        // 13.3.3.1.1: CTRL 0, DISP_PARA 0x10, CTRLDESCL0_1 0x200, _3 0x208,
        // LOW0_4 0x20c, HIGH0_4 0x210, _5 0x214, CSC0_CTRL 0x21c, PANIC 0x238
        assert_eq!((r::CTRL, r::CTRL_SET, r::CTRL_CLR, r::DISP_PARA), (0, 4, 8, 0x10));
        assert_eq!((r::CTRLDESCL0_1, r::CTRLDESCL0_3, r::CTRLDESCL_LOW0_4), (0x200, 0x208, 0x20c));
        assert_eq!(
            (r::CTRLDESCL_HIGH0_4, r::CTRLDESCL0_5, r::CSC0_CTRL, r::PANIC0_THRES),
            (0x210, 0x214, 0x21c, 0x238)
        );
        assert_eq!(field(r::ctrldescl0_5::BPP_MASK, BPP_ARGB8888), 0x0900_0000);
    }

    #[test]
    fn configure_then_enable_writes_the_mode_and_waits_for_the_load() {
        let m = HdmiModel::lcdif_only();
        let e = TestEnv::default();
        let b = m.base(LCDIF);
        assert_eq!(m.peek(LCDIF, r::CTRL), 0x8000_0000, "SW_RESET set out of reset");
        assert_eq!(configure(&m, b, &WUXGA_60_RB, &OUT, &LAYER), Ok(7680));
        assert_eq!(m.peek(LCDIF, r::CTRL), 0, "SW_RESET cleared, all sync active high");
        assert_eq!(m.peek(LCDIF, r::DISP_SIZE), (1200 << 16) | 1920);
        assert_eq!(m.peek(LCDIF, r::HSYN_PARA), (80 << 16) | 48);
        assert_eq!(m.peek(LCDIF, r::VSYN_PARA), (26 << 16) | 3);
        assert_eq!(m.peek(LCDIF, r::VSYN_HSYN_WIDTH), (6 << 16) | 32);
        assert_eq!(m.peek(LCDIF, r::CTRLDESCL0_3), 7680);
        assert_eq!(m.peek(LCDIF, r::CTRLDESCL_LOW0_4), 0x9000_0000);
        assert!(!is_running(&m, b));
        enable(&m, b, &e, Some(Panic { low: 170, high: 340 })).unwrap();
        assert!(is_running(&m, b));
        assert_eq!(m.peek(LCDIF, r::PANIC0_THRES), (170 << 16) | 340);
        // DISP_ON before the layer's EN (Figure 13-7)
        let w = m.writes();
        let on = w.iter().position(|x| x.1 == r::DISP_PARA && x.2 & (1 << 31) != 0).unwrap();
        let en = w.iter().position(|x| x.1 == r::CTRLDESCL0_5 && x.2 & (1 << 31) != 0).unwrap();
        assert!(on < en, "{w:?}");
        let s = read_scanout(&m, b).unwrap();
        assert_eq!(s, Scanout { width: 1920, height: 1200, pitch: 7680, bytes_pp: 4, phys: 0x9000_0000 });
    }

    #[test]
    fn no_pixel_clock_is_a_timeout_after_two_frames() {
        let m = HdmiModel::lcdif_only_with(HdmiFaults { no_vsync: true, ..Default::default() });
        let e = TestEnv::default();
        let b = m.base(LCDIF);
        configure(&m, b, &WUXGA_60_RB, &OUT, &LAYER).unwrap();
        assert_eq!(enable(&m, b, &e, None), Err(-ETIMEDOUT));
        assert!(*e.waited_us.borrow() >= u64::from(FRAME_WAIT_US));
    }

    #[test]
    fn bad_layers_are_refused_before_any_write() {
        let m = HdmiModel::lcdif_only();
        let b = m.base(LCDIF);
        for l in [
            Layer { pitch: 7000, ..LAYER },       // under width * 4
            Layer { pitch: 7680 + 32, ..LAYER },  // not 64-byte aligned
            Layer { phys: 0x9000_0004, ..LAYER }, // not 8-byte aligned
            Layer { phys: 1 << 36, ..LAYER },     // beyond 36 bits
            Layer { desc3_extra: 1, ..LAYER },    // inside PITCH
        ] {
            assert_eq!(configure(&m, b, &WUXGA_60_RB, &OUT, &l), Err(-EINVAL), "{l:?}");
        }
        assert!(m.writes().is_empty());
    }

    #[test]
    fn a_reset_that_does_not_clear_is_an_error() {
        let m = HdmiModel::lcdif_only_with(HdmiFaults { lcdif_reset_stuck: true, ..Default::default() });
        assert_eq!(configure(&m, m.base(LCDIF), &WUXGA_60_RB, &OUT, &LAYER), Err(-EIO));
    }

    #[test]
    fn flip_and_layout_and_disable() {
        let m = HdmiModel::lcdif_only();
        let e = TestEnv::default();
        let b = m.base(LCDIF);
        configure(&m, b, &WUXGA_60_RB, &OUT, &LAYER).unwrap();
        enable(&m, b, &e, None).unwrap();
        set_fb(&m, b, &e, 0x1_2345_6780).unwrap();
        assert_eq!(read_scanout(&m, b).unwrap().phys, 0x1_2345_6780);
        assert_eq!(set_fb(&m, b, &e, 0x9000_0001), Err(-EINVAL));
        set_layout(&m, b, 3840, BPP_RGB565).unwrap();
        assert_eq!(read_scanout(&m, b).unwrap().bytes_pp, 2);
        test_mode(&m, b, DispMode::Background, 0x00ff_0000);
        assert_eq!(m.peek(LCDIF, r::DISP_PARA), 0x8000_0000 | (1 << 24) | 0xff_0000);
        disable(&m, b, &e).unwrap();
        assert!(!is_running(&m, b));
        assert!(read_scanout(&m, b).is_none());
    }

    /// The previous kernel's layer: 1920x1200 RGB565 at 0x8000_0000.
    fn linux_left_it_running(m: &HdmiModel) -> usize {
        let e = TestEnv::default();
        let b = m.base(LCDIF);
        configure(m, b, &WUXGA_60_RB, &OUT, &Layer { phys: 0x8000_0000, pitch: 0, desc3_extra: 0 }).unwrap();
        set_layout(m, b, 3840, BPP_RGB565).unwrap();
        // (a model with no frame boundaries cannot finish the load; the
        // layer is on all the same)
        let _ = enable(m, b, &e, None);
        m.clear_log();
        b
    }

    #[test]
    fn adopting_without_an_address_writes_nothing_and_the_first_buffer_brings_our_layout() {
        let m = HdmiModel::lcdif_only();
        let e = TestEnv::default();
        let b = linux_left_it_running(&m);
        let mut a = Adopted::default();
        let s = adopt(&m, b, &e, &mut a, (1920, 1200), 0, 0).unwrap();
        assert_eq!((s.phys, s.bytes_pp), (0x8000_0000, 2));
        assert!(m.writes().is_empty(), "no address: no write (never scan physical zero)");
        assert!(!a.ours && a.held);
        // a second adopt finds the same layer and keeps the first capture
        adopt(&m, b, &e, &mut a, (1920, 1200), 0, 0).unwrap();
        assert_eq!(a.prev_phys, 0x8000_0000);
        assert_eq!(adopt_set_fb(&m, b, &e, &mut a, 0), Err(-EINVAL));
        adopt_set_fb(&m, b, &e, &mut a, 0x9000_0000).unwrap();
        let s = read_scanout(&m, b).unwrap();
        assert_eq!((s.phys, s.pitch, s.bytes_pp), (0x9000_0000, 7680, 4));
        // adopting again now finds our buffer; release still hands back Linux's
        adopt(&m, b, &e, &mut a, (1920, 1200), 0x9100_0000, 0).unwrap();
        adopt_release(&m, b, &e, &mut a).unwrap();
        let s = read_scanout(&m, b).unwrap();
        assert_eq!((s.phys, s.pitch, s.bytes_pp), (0x8000_0000, 3840, 2));
    }

    #[test]
    fn adopt_refuses_a_dark_or_differently_sized_layer() {
        let m = HdmiModel::lcdif_only();
        let e = TestEnv::default();
        let b = m.base(LCDIF);
        let mut a = Adopted::default();
        assert_eq!(adopt(&m, b, &e, &mut a, (1920, 1200), 0, 0), Err(-crate::errno::ENODEV));
        let b = linux_left_it_running(&m);
        assert_eq!(adopt(&m, b, &e, &mut a, (1280, 800), 0x9000_0000, 0), Err(-EINVAL));
        assert!(!a.held && m.writes().is_empty());
    }

    #[test]
    fn polled_flip_and_vblank_interrupt() {
        let m = HdmiModel::lcdif_only();
        let b = linux_left_it_running(&m);
        set_fb_polled(&m, b, 0x9000_0000, 10).unwrap();
        assert_eq!(read_scanout(&m, b).unwrap().phys, 0x9000_0000);
        assert_eq!(set_fb_polled(&m, b, 0x9000_0004, 10), Err(-EINVAL));
        set_vblank_irq(&m, b, true);
        assert_eq!(m.peek(LCDIF, r::INT_ENABLE_D0), r::int_enable_d0::VS_BLANK_EN_MASK);
        set_vblank_irq(&m, b, false);
        assert_eq!(m.peek(LCDIF, r::INT_ENABLE_D0), 0);

        let m = HdmiModel::lcdif_only_with(HdmiFaults { no_vsync: true, ..Default::default() });
        let b = linux_left_it_running(&m);
        assert_eq!(set_fb_polled(&m, b, 0x9000_0000, 10), Err(-ETIMEDOUT));
    }
}
