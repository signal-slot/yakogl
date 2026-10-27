// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NXP i.MX 8M Plus.
//!
//! [`regs`] is generated from hal_nxp's `PERI_*.h` (tools/hal-nxp/gen_regs.py;
//! never edit it by hand). [`ccm`] turns clock roots and gates on, [`anatop`]
//! reads and programs the PLLs, [`gpc`] powers PU domains, [`media_blk`] is
//! the MEDIAMIX block control and [`media`] the display power domains built
//! from them. [`hdmi`] is the HDMIMIX (power, clocks, resets, the PVI) and
//! [`hdmi_chain`] the HDMI display chain brought up from cold.

pub mod anatop;
pub mod ccm;
#[cfg(test)]
mod dsi_chain_tests;
pub mod gpc;
pub mod hdmi;
pub mod hdmi_chain;
#[cfg(test)]
mod hdmi_golden;
pub mod media;
pub mod media_blk;
#[rustfmt::skip]
pub mod regs;
