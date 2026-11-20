// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95 (Toradex Verdin iMX95): glue for the Cortex-A55 side. The
//! GPU's clocks and power domain belong to the System Manager on the
//! Cortex-M33, which the A55 asks over SCMI ([`gpu_power`];
//! docs/notes/imx95-gpu-power.md).

pub mod gpu_power;
#[cfg(test)]
mod gpu_power_tests;
