// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95's LVDS display bridge (LDB) at the LVDS CSR (0x4B0C0000):
//! the PHY clock control, the pixel mapper and LVDS PHY 0, for one
//! single-channel LVDS panel on channel 0 from display input 0 (DI0, the
//! DPU's stream 0 through the pixel interleaver's channel 0). The design
//! note's section 4(b') step 3 (driver/docs/imx95-display.md).
//!
//! Sources, by their licence (docs/source-form.md):
//! - registers and fields: NXP's BSD-3 `PERI_LDB.h` (hal_nxp 1e91007b,
//!   generated into [`super::regs::ldb`]);
//! - the sequence: hal_nxp's BSD-3 `drivers/ldb_1/fsl_ldb.c` `LDB_Init`
//!   (:33-95): the PHY clock control, the pixel mapper, LVDS PHY "disabled"
//!   (`CH0_EN | LVDS_EN | BG_EN`: `LVDS_EN` 1 is "Disable" in the header),
//!   at least 100 ms, then the PHY enabled with pre-emphasis (`PRE_EMPH_EN`,
//!   `PRE_EMPH_ADJ` 3) and output current `CC_ADJ` 2;
//! - the values: the board's Linux under the Toradex 10.1" LVDS panel
//!   (2026-11-25, driver/docs/imx95-display-dump-linux.txt), decoded against
//!   `PERI_LDB.h`:
//!   - +0x0 `LVDS_PHY_CLOCK_CONTROL` = 0x14: the PHY divider 0 (PHY clock =
//!     PLL clock, single channel), serializer channel 0 and pixel mapper DI0
//!     clocked (bits 1 and 3 clear), channel 1 and DI1 not clocked (bits 2
//!     and 4 set; "1: not clocked" in the header). Linux's clk_summary says
//!     the same (`lvds_ch0_gate`, `lvds_di0_gate` on, `lvds_ch1_gate`,
//!     `lvds_di1_gate` off): the four bits are set-to-gate clock gates (the
//!     note's U4, answered).
//!   - +0x4 `PIXEL_MAPPER_CONTROL` = 0x00082221: `CH0_MODE` 1 (channel 0
//!     enabled, routed to DI0), `CH1_MODE` 0 (disabled), no split,
//!     `CH0_DATA_WIDTH` 1 (24 bit), `CH0_BIT_MAPPING` 0 (SPWG, the panel's
//!     `vesa-24`), `DI0_VSYNC_POLARITY` and `DI0_HSYNC_POLARITY` 1 (active
//!     low), `VSYNC_ADJ_EN` 1; the async FIFO off, the bandgap's external
//!     resistor.
//!   - +0x8 `LVDS0_CONTROL` = 0x1075: `fsl_ldb.c`'s final value exactly.
//!   - +0xC `LVDS1_CONTROL` = 0: PHY 1 off.
//!
//! Where `fsl_ldb.c` and the dump differ, [`Form`] names both. `LDB_Init`
//! is written for a panel on both channels: it routes channel 1 to DI0 too
//! (`CH1_MODE` 1, `CH1_DATA_WIDTH` 1), enables PHY 1 like PHY 0, sets only
//! the DI0 VSYNC polarity (no HSYNC polarity, no `VSYNC_ADJ_EN`) and never
//! writes the clock gates (their reset value stays). The dump settles what
//! this board's panel needs (one channel, PHY 0), so [`Form::Linux`] is the
//! default; [`Form::HalInit`] is `LDB_Init(base, 0, 0, LVDS_SPWG)` from reset,
//! kept to try if the dump's words do not light the panel cold.

use super::regs::ldb as r;
use crate::mmio::{field, Mmio};
use crate::video::VideoMode;

/// `LVDS_CSR` / `DISPLAY__LDB` base (MIMX9596 `_COMMON.h`; Linux's
/// `nxp,imx95-lvds-csr` reg).
pub const LDB_BASE: usize = 0x4B0C_0000;
/// The block's size as Linux maps it.
pub const LDB_SIZE: usize = 0x1_0000;

/// `fsl_ldb.c`'s wait between the PHY's "disabled" and its enable, µs
/// (`SDK_DelayAtLeastUs(100000, ...)`).
pub const PHY_RESTART_US: u32 = 100_000;

/// `CH0_MODE` / `CH1_MODE` 1: the channel enabled, routed to DI0.
const MODE_DI0: u32 = 1;
/// `fsl_ldb.c`'s PHY values: pre-emphasis 3, output current 2.
const PRE_EMPH_ADJ: u32 = 3;
const CC_ADJ: u32 = 2;

/// Which source's values ([`Form::Linux`] is the default; module text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// The board's Linux (the dump): channel 0 and PHY 0 only, both sync
    /// polarities, `VSYNC_ADJ_EN`, channel 1's and DI1's clocks gated.
    Linux,
    /// hal_nxp's `LDB_Init(base, 0, 0, LVDS_SPWG)` on a reset block.
    HalInit,
}

/// The four words and whether the clock control is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    /// `LVDS_PHY_CLOCK_CONTROL`, or left at its reset value.
    pub phy_clock: Option<u32>,
    pub mapper: u32,
    pub lvds0: u32,
    /// `LVDS1_CONTROL`, or not touched.
    pub lvds1: Option<u32>,
}

/// PHY "disabled" while it restarts (`fsl_ldb.c`'s first write).
pub const PHY_PARKED: u32 =
    r::lvds0_control::CH0_EN_MASK | r::lvds0_control::LVDS_EN_MASK | r::lvds0_control::BG_EN_MASK;

/// The PHY running: channel enabled, bandgap on, pre-emphasis 3, output
/// current 2 (`LVDS_EN` 0 is "enable").
pub const PHY_ON: u32 = r::lvds0_control::CH0_EN_MASK
    | r::lvds0_control::BG_EN_MASK
    | r::lvds0_control::PRE_EMPH_EN_MASK
    | (PRE_EMPH_ADJ << r::lvds0_control::PRE_EMPH_ADJ_SHIFT)
    | (CC_ADJ << r::lvds0_control::CC_ADJ_SHIFT);

/// Channel 1's serializer and DI1's mapper clock gated, the rest clocked,
/// the PHY clock = the PLL clock (single channel).
pub const CLOCKS_CH0_DI0: u32 = r::lvds_phy_clock_control::SERIALIZER_CH1_CLOCK_CTRL_MASK
    | r::lvds_phy_clock_control::PIXELMAPPER_DI1_CLOCK_CTRL_MASK;
/// Every serializer and mapper clock gated (the stop).
pub const CLOCKS_ALL_GATED: u32 = r::lvds_phy_clock_control::SERIALIZER_CH0_CLOCK_CTRL_MASK
    | r::lvds_phy_clock_control::SERIALIZER_CH1_CLOCK_CTRL_MASK
    | r::lvds_phy_clock_control::PIXELMAPPER_DI0_CLOCK_CTRL_MASK
    | r::lvds_phy_clock_control::PIXELMAPPER_DI1_CLOCK_CTRL_MASK;

/// The words for a single-channel 24-bit SPWG panel showing `mode` on
/// channel 0 from DI0. The sync polarity bits are those of what the
/// display engine sends (active low on this path, `dpu::timing`), which the
/// mode's flags say for the panel.
pub fn words(mode: &VideoMode, form: Form) -> Words {
    use r::pixel_mapper_control as pm;
    let ch0 = field(pm::CH0_MODE_MASK, MODE_DI0) | pm::CH0_DATA_WIDTH_MASK;
    match form {
        Form::Linux => {
            let mut mapper = ch0 | pm::VSYNC_ADJ_EN_MASK;
            if !mode.vsync_high {
                mapper |= pm::DI0_VSYNC_POLARITY_MASK;
            }
            if !mode.hsync_high {
                mapper |= pm::DI0_HSYNC_POLARITY_MASK;
            }
            Words { phy_clock: Some(CLOCKS_CH0_DI0), mapper, lvds0: PHY_ON, lvds1: None }
        }
        Form::HalInit => Words {
            phy_clock: None,
            mapper: ch0 | field(pm::CH1_MODE_MASK, MODE_DI0) | pm::CH1_DATA_WIDTH_MASK | pm::DI0_VSYNC_POLARITY_MASK,
            lvds0: PHY_ON,
            lvds1: Some(PHY_ON),
        },
    }
}

fn w(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v);
}

/// The LDB and LVDS PHY 0 up (`fsl_ldb.c`'s order, module text). The LDB
/// PLL must run (the pixel clock and the serial clock come from it) and the
/// DISPLAY domain be on.
pub fn up(m: &impl Mmio, base: usize, wd: &Words, delay_us: &mut impl FnMut(u32)) {
    if let Some(c) = wd.phy_clock {
        w(m, base, r::LVDS_PHY_CLOCK_CONTROL, c);
    }
    w(m, base, r::PIXEL_MAPPER_CONTROL, wd.mapper);
    w(m, base, r::LVDS0_CONTROL, PHY_PARKED);
    if wd.lvds1.is_some() {
        w(m, base, r::LVDS1_CONTROL, PHY_PARKED);
    }
    delay_us(PHY_RESTART_US);
    w(m, base, r::LVDS0_CONTROL, wd.lvds0);
    if let Some(v) = wd.lvds1 {
        w(m, base, r::LVDS1_CONTROL, v);
    }
}

/// The LVDS output off: the PHYs off, the mapper's channels disabled, every
/// clock gated.
pub fn down(m: &impl Mmio, base: usize) {
    w(m, base, r::LVDS0_CONTROL, 0);
    w(m, base, r::LVDS1_CONTROL, 0);
    w(m, base, r::PIXEL_MAPPER_CONTROL, 0);
    w(m, base, r::LVDS_PHY_CLOCK_CONTROL, CLOCKS_ALL_GATED);
}

/// The four words as they read (the proof, and the probe's identity).
pub fn read(m: &impl Mmio, base: usize) -> [u32; 4] {
    [r::LVDS_PHY_CLOCK_CONTROL, r::PIXEL_MAPPER_CONTROL, r::LVDS0_CONTROL, r::LVDS1_CONTROL]
        .map(|o| m.read32(base + o as usize))
}
