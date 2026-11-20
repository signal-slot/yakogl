// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95 (Toradex Verdin iMX95): glue for the Cortex-A55 side. The
//! GPU's and the display's clocks and power domains belong to the System
//! Manager on the Cortex-M33, which the A55 asks over SCMI ([`sm`], the
//! message layer; [`gpu_power`]; [`display_power`];
//! docs/notes/imx95-gpu-power.md). The display controller (DPU, "Seeris")
//! and the display block's routing are registers ([`dpu`], [`display`];
//! driver/docs/imx95-display.md), named from NXP's BSD-3 hal_nxp headers:
//! [`regs`] is generated (tools/hal-nxp/gen_regs.py --soc imx95; never edit
//! it by hand).

pub mod display;
pub mod display_power;
pub mod dpu;
pub mod gpu_power;
#[rustfmt::skip]
pub mod regs;
pub mod sm;

#[cfg(test)]
mod display_tests;
#[cfg(test)]
mod gpu_power_tests;
#[cfg(test)]
pub(crate) mod sm_mock;
