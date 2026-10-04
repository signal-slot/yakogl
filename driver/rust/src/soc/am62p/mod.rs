// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The TI AM62P (AM62Px TRM, SPRUJ83D).
//!
//! [`regs`] are the display registers, written from the TRM with a citation
//! per constant. [`ti_pr114866`] is generated from TI's vendored Zephyr DSI
//! driver (tools/ti-zephyr/gen_dphy_data.py; never edit it by hand): the four
//! D-PHY facts the TRM does not state. [`dss`] is DSS1 as the Verdin AM62P
//! uses it: one video port, the application plane under the safety layer
//! (the two-plane protocol of the replaced C driver, kept as it was).

#[cfg(test)]
mod display_golden;
pub mod dss;
pub mod regs;
#[rustfmt::skip]
pub mod ti_pr114866;
