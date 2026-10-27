// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! MEDIAMIX Block Control (MEDIA_BLK_CTRL) of the i.MX 8M Plus: the
//! software resets and clock enables of the blocks inside MEDIAMIX, the MIPI
//! D-PHY resets, and the LCDIF read-hurry.
//!
//! From IMX8MPRM Rev. 3, 08/2024, 13.2 "MEDIA BLK_CTRL"; names and bits from
//! hal_nxp `PERI_MEDIA_BLK_CTRL.h` ([`super::regs::media_blk_ctrl`]):
//! - `SFT_RSTN` (13.2.3.1.2): 0 = "software reset enable", 1 = disable, one
//!   bit per clock of a block; `CLK_EN` (13.2.3.1.3): 1 = clock enable.
//!   Figure 13-5 "Media BLK_CTRL Clocks" ties the `SFT_EN_LCDIF_*` gates to
//!   LCDIF1 (the unnumbered names are LCDIF1, the `LCDIF2_*` ones LCDIF2),
//!   `SFT_EN_MIPI_DSI_*` to MIPI_DSI1 and `SFT_EN_BUS_BLK_CLK` to the NoC.
//! - `MIPI_RESET_DIV.GPR_MIPI_M_RESETN` (13.2.3.1.4): the master (DSI)
//!   D-PHY reset, 0 = reset enable.
//! - `LCDIF_ARCACHE_CTRL.GPR_LCDIF_0_RD_HURRY` (13.2.3.1.14): the RM names
//!   the field and nothing else.
//!
//! Not stated in IMX8MPRM Rev. 3:
//! - the order of clock enable and reset release (13.2.2 only says the block
//!   "contains clock distribution and gating controls, as well as reset
//!   handling"): clocks are enabled first and the reset released after
//!   [`RESET_PULSE_US`], so the reset is seen by running clocks -- the order
//!   and time of the replaced driver, which ran on this board;
//! - what the read-hurry value means and which LCDIF "LCDIF_0" is: the value
//!   written is 7 in `GPR_LCDIF_0_RD_HURRY` only, the register's value under
//!   the board's Linux driving LCDIF1 (golden-linux-2026-09-21.txt:
//!   0x32ec004c = 0x00001c00). Risk R1 in docs/source-form.md.

use super::regs::media_blk_ctrl as blk;
use crate::env::Env;
use crate::mmio::Mmio;

/// The NoC bus clock of the block control (`SFT_EN_BUS_BLK_CLK*`).
pub const BUS: u32 = blk::clk_en::SFT_EN_BUS_BLK_CLK_MASK;
/// LCDIF1: pixel, APB and AXI clocks.
pub const LCDIF1: u32 = blk::clk_en::SFT_EN_LCDIF_PIXEL_CLK_MASK
    | blk::clk_en::SFT_EN_LCDIF_APB_CLK_MASK
    | blk::clk_en::SFT_EN_LCDIF_AXI_CLK_MASK;
/// The LCDIF's register clock alone (part of [`LCDIF1`]): what a register
/// access needs.
pub const LCDIF1_APB: u32 = blk::clk_en::SFT_EN_LCDIF_APB_CLK_MASK;
/// MIPI_DSI1: PCLK and CLKREF.
pub const MIPI_DSI1: u32 = blk::clk_en::SFT_EN_MIPI_DSI_PCLK_MASK | blk::clk_en::SFT_EN_MIPI_DSI_CLKREF_MASK;
/// The DSI D-PHY's reset in `MIPI_RESET_DIV`.
pub const MIPI_M_RESETN: u32 = blk::mipi_reset_div::GPR_MIPI_M_RESETN_MASK;
/// The read-hurry this driver writes (see the module text).
pub const LCDIF_0_RD_HURRY: u32 = 7;
/// Reset pulse and release delay, us (not from the RM).
pub const RESET_PULSE_US: u32 = 5;

// The SFT_RSTN bits sit where the CLK_EN bits do (13.2.3.1.2 / .3).
const _: () = assert!(blk::sft_rstn::SFT_EN_BUS_BLK_CLK_RESETN_MASK == BUS);
const _: () = assert!(
    blk::sft_rstn::SFT_EN_LCDIF_PIXEL_CLK_RESETN_MASK
        | blk::sft_rstn::SFT_EN_LCDIF_APB_CLK_RESETN_MASK
        | blk::sft_rstn::SFT_EN_LCDIF_AXI_CLK_RESETN_MASK
        == LCDIF1
);
const _: () = assert!(
    blk::sft_rstn::SFT_EN_MIPI_DSI_PCLK_RESETN_MASK | blk::sft_rstn::SFT_EN_MIPI_DSI_CLKREF_RESETN_MASK == MIPI_DSI1
);

fn reg(base: usize, off: u32) -> usize {
    base + off as usize
}

fn set(m: &impl Mmio, a: usize, bits: u32) -> bool {
    let v = m.read32(a);
    if v & bits == bits {
        return false;
    }
    m.write32(a, v | bits);
    true
}

fn clear(m: &impl Mmio, a: usize, bits: u32) -> bool {
    let v = m.read32(a);
    if v & bits == 0 {
        return false;
    }
    m.write32(a, v & !bits);
    true
}

/// Clocks `bits` on, then their resets released; nothing written for what
/// already is. With `phy`, the DSI D-PHY's reset is released after them.
/// Returns whether anything was written.
pub fn enable(m: &impl Mmio, base: usize, env: &impl Env, bits: u32, phy: bool) -> bool {
    let rst = reg(base, blk::SFT_RSTN);
    let clocked = set(m, reg(base, blk::CLK_EN), bits);
    if clocked && m.read32(rst) & bits != bits {
        env.busy_wait_us(RESET_PULSE_US);
    }
    let released = set(m, rst, bits);
    let phy_released = phy && set(m, reg(base, blk::MIPI_RESET_DIV), MIPI_M_RESETN);
    clocked || released || phy_released
}

/// A reset pulse with the clocks on: resets asserted (and the D-PHY's with
/// `phy`), clocks enabled, [`RESET_PULSE_US`], released.
pub fn pulse(m: &impl Mmio, base: usize, env: &impl Env, bits: u32, phy: bool) {
    clear(m, reg(base, blk::SFT_RSTN), bits);
    if phy {
        clear(m, reg(base, blk::MIPI_RESET_DIV), MIPI_M_RESETN);
    }
    set(m, reg(base, blk::CLK_EN), bits);
    env.busy_wait_us(RESET_PULSE_US);
    set(m, reg(base, blk::SFT_RSTN), bits);
    if phy {
        set(m, reg(base, blk::MIPI_RESET_DIV), MIPI_M_RESETN);
    }
}

/// Resets asserted (the D-PHY's first with `phy`), then clocks gated.
pub fn disable(m: &impl Mmio, base: usize, bits: u32, phy: bool) {
    if phy {
        clear(m, reg(base, blk::MIPI_RESET_DIV), MIPI_M_RESETN);
    }
    clear(m, reg(base, blk::SFT_RSTN), bits);
    clear(m, reg(base, blk::CLK_EN), bits);
}

/// `GPR_LCDIF_0_RD_HURRY` to [`LCDIF_0_RD_HURRY`], the rest of the register
/// kept. Returns whether it wrote.
pub fn set_rd_hurry(m: &impl Mmio, base: usize) -> bool {
    use blk::lcdif_arcache_ctrl as a;
    let r = reg(base, blk::LCDIF_ARCACHE_CTRL);
    let v = m.read32(r);
    let want = (v & !a::GPR_LCDIF_0_RD_HURRY_MASK) | (LCDIF_0_RD_HURRY << a::GPR_LCDIF_0_RD_HURRY_SHIFT);
    if v == want {
        return false;
    }
    m.write32(r, want);
    true
}

/// What the block control says, read only (MEDIAMIX must be powered).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub clk_en: u32,
    pub sft_rstn: u32,
    pub mipi_reset_div: u32,
    pub arcache: u32,
}

pub fn state(m: &impl Mmio, base: usize) -> State {
    State {
        clk_en: m.read32(reg(base, blk::CLK_EN)),
        sft_rstn: m.read32(reg(base, blk::SFT_RSTN)),
        mipi_reset_div: m.read32(reg(base, blk::MIPI_RESET_DIV)),
        arcache: m.read32(reg(base, blk::LCDIF_ARCACHE_CTRL)),
    }
}

impl State {
    pub fn on(self, bits: u32) -> bool {
        self.clk_en & bits == bits && self.sft_rstn & bits == bits
    }

    pub fn hurry_set(self) -> bool {
        use blk::lcdif_arcache_ctrl as a;
        (self.arcache & a::GPR_LCDIF_0_RD_HURRY_MASK) >> a::GPR_LCDIF_0_RD_HURRY_SHIFT == LCDIF_0_RD_HURRY
    }
}
