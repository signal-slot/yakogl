// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F (STM32MP257F-DK): glue for the Cortex-A35 side. Plan #322,
//! issues #7 and #1281.

pub mod display_regs;
#[cfg(any(feature = "vivante", test))]
pub mod gpu_ffi;
pub mod power;
#[cfg(test)]
mod power_model_tests;
pub mod rcc;
pub mod smc_wdt;
