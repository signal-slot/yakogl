// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Drivers for one IP block or part each, independent of the SoC around it:
//! they get the block's base address and its input clock rate (or, for a
//! part on a bus, the bus through [`crate::env::Env`]), and nothing else.
//! Register names and bits come from the generated map of the SoC the block
//! was first written for, or, for an external part, from its data sheet.

#[cfg(any(feature = "am62p", test))]
pub mod cdns_dphy;
#[cfg(any(feature = "am62p", test))]
pub mod cdns_dsi;
/// D-PHY timing arithmetic, any SoC with a D-PHY transmitter.
#[cfg(any(feature = "imx8mp", test))]
pub mod dphy_timing;
#[cfg(any(feature = "imx8mp", test))]
pub mod dw_hdmi_tx;
#[cfg(any(feature = "imx8mp", test))]
pub mod imx8mp_hdmi_phy;
#[cfg(any(feature = "imx8mp", test))]
pub mod imx_pwm;
/// The i.MX WDOG, armed by an image that no earlier stage may have armed.
#[cfg(any(feature = "imx8mp", test))]
pub mod imx_wdog;
#[cfg(any(feature = "imx8mp", test))]
pub mod lcdifv3;
#[cfg(any(feature = "imx8mp", test))]
pub mod samsung_dsim;
/// On I2C only, on any SoC: always built.
pub mod sn65dsi8x;
#[cfg(any(feature = "am62p", test))]
pub mod ti_dss7;
