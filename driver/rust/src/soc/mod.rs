// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! SoC glue: register maps generated from the vendor's permissive headers,
//! and the clock and power steps a driver needs on that SoC.

#[cfg(any(feature = "imx8mp", test))]
pub mod imx8mp;

#[cfg(any(feature = "am62p", test))]
pub mod am62p;

#[cfg(any(feature = "rk3588", test))]
pub mod rk3588;
