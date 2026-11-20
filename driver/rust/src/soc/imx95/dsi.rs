// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95's MIPI DSI output from the pixel link to the lanes: the
//! Synopsys DW MIPI DSI host (`ip::dw_mipi_dsi`, the STM32MP25's driver,
//! unchanged) with the NXP combo PHY ([`super::combo_phy`]) as its PHY. NXP's
//! glue is `nxp,imx95-mipi-dsi`, the DW core with NXP's phy_ops
//! (`imx95-mipi-dsi.c:23`, `:1224-1229`, `:1313`; facts only).
//!
//! What the i.MX 95 does differently from the STM32MP25, and where:
//!
//! - the version: [`dw_mipi_dsi::probe_at_least`] from 1.31 (the
//!   `PHY_TMR_CFG` layout hal_nxp's `PERI_MIPI_DSI.h` documents: lp2hs 9:0,
//!   hs2lp 25:16), not ST's list;
//! - the PHY: [`ComboPhy`], reached through the host's test interface and
//!   NXP's CSRs;
//! - the DPI polarity: NXP's phy init writes `DPI_CFG_POL` = VSYNC and HSYNC
//!   active low, DE active high, whatever the mode (`imx95-mipi-dsi.c:821`):
//!   the pixel link carries the syncs low. [`link`] gives the host a mode
//!   with those polarities, so the host writes that value once
//!   (dw-mipi-dsi.c:721) and the PHY need not write it again. The panel's
//!   own polarities are the bridge's business (its LVDS side).
//!
//! The DPU's stream on the pixel link (the DISPLAY domain, the pixel clock
//! DISP1PIX, the pixel interleaver and the display CSR's pixel link enable)
//! is the DPU side's (`soc::imx95::dpu*`); [`up`] assumes the pixel clock
//! runs at the mode's rate before video starts.

use super::combo_phy::{Bases, ComboPhy};
use crate::env::Env;
use crate::ip::dw_mipi_dsi::{self, regs as dsi_regs, Failed, Format, Link, Plan};
use crate::mmio::Mmio;
use crate::video::VideoMode;

/// The lowest host version this module drives (module text).
pub const MIN_VERSION: u32 = dsi_regs::HWVER_131;

/// The DSI link for a mode as the pixel link delivers it: the mode's
/// timings with the syncs active low and DE active high (module text).
pub fn link(mode: &VideoMode, lanes: u32, flags: u32) -> Link {
    let mode = VideoMode { hsync_high: false, vsync_high: false, de_high: true, ..*mode };
    Link { mode, lanes, channel: 0, format: Format::Rgb888, flags }
}

/// The host up in command mode with the clock lane in HS: version, then
/// `dw_mipi_dsi::attach` with the combo PHY. `phy` must carry the PHY
/// clocks' rates (`dsi_power::Up`); it keeps the PLL it chose.
pub fn up(m: &impl Mmio, env: &impl Env, phy: &mut ComboPhy, link: &Link) -> Result<(u32, Plan), Failed> {
    let version = dw_mipi_dsi::probe_at_least(m, phy.bases.dsi, MIN_VERSION)
        .map_err(|errno| Failed { step: dw_mipi_dsi::step::CHECK, errno })?;
    let plan = dw_mipi_dsi::attach(m, env, phy.bases.dsi, phy, link)?;
    Ok((version, plan))
}

/// Video on (the host's `atomic_enable`).
pub fn video_on(m: &impl Mmio, bases: &Bases, link: &Link) {
    dw_mipi_dsi::video_on(m, bases.dsi, link);
}

/// Back to command mode, the PHY off, host and PHY in reset.
pub fn down(m: &impl Mmio, phy: &mut ComboPhy, link: &Link) {
    dw_mipi_dsi::disable(m, phy.bases.dsi, phy, link);
}
