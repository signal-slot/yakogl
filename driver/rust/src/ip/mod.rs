// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Drivers for one IP block or part each, independent of the SoC around it:
//! they get the block's base address and its input clock rate (or, for a
//! part on a bus, the bus through [`crate::env::Env`]), and nothing else.
//! Register names and bits come from the generated map of the SoC the block
//! was first written for, or, for an external part, from its data sheet.

/// The ADV7535 HDMI transmitter on I2C, fed by MIPI DSI.
#[cfg(any(feature = "stm32mp2", test))]
pub mod adv7535;
#[cfg(any(feature = "am62p", test))]
pub mod cdns_dphy;
#[cfg(any(feature = "am62p", test))]
pub mod cdns_dsi;
/// D-PHY timing arithmetic, any SoC with a D-PHY transmitter.
#[cfg(any(feature = "imx8mp", feature = "stm32mp2", test))]
pub mod dphy_timing;
/// The DesignWare HDMI QP TX controller and its RK3588 GRF glue.
#[cfg(any(feature = "rk3588", test))]
pub mod dw_hdmi_qp;
#[cfg(any(feature = "imx8mp", test))]
pub mod dw_hdmi_tx;
/// The Synopsys DesignWare MIPI DSI host controller.
#[cfg(any(feature = "stm32mp2", test))]
pub mod dw_mipi_dsi;
/// The DesignWare APB watchdog, armed by an image that no earlier stage
/// armed (the RK3588's WDT0).
#[cfg(any(feature = "rk3588", test))]
pub mod dw_wdt;
#[cfg(any(feature = "imx8mp", test))]
pub mod imx8mp_hdmi_phy;
#[cfg(any(feature = "imx8mp", test))]
pub mod imx_pwm;
/// The i.MX WDOG, armed by an image that no earlier stage may have armed.
#[cfg(any(feature = "imx8mp", test))]
pub mod imx_wdog;
#[cfg(any(feature = "imx8mp", test))]
pub mod lcdifv3;
/// A 16550's transmit side, polled, as an earlier stage left it: the
/// NanoPC-T6's first output, before any driver.
#[cfg(any(feature = "rk3588", test))]
pub mod ns16550_tx;
/// The RK806 PMIC's read frames (read only), on any SPI host.
#[cfg(any(feature = "rk3588", test))]
pub mod rk806;
/// The Rockchip SPI controller as a polled host (the RK3588's SPI2).
#[cfg(any(feature = "rk3588", test))]
pub mod rockchip_spi;
#[cfg(any(feature = "imx8mp", test))]
pub mod samsung_dsim;
/// On I2C only, on any SoC: always built.
pub mod sn65dsi8x;
/// ST's DSI wrapper and its D-PHY PLL on the STM32MP25.
#[cfg(any(feature = "stm32mp2", test))]
pub mod stm32_dsi_phy;
/// The STM32MP25 GPIO bank: pin mux, drive, level and per-pin RIF.
#[cfg(any(feature = "stm32mp2", test))]
pub mod stm32_gpio;
/// The STM32 I2C v2 controller as a polled master.
#[cfg(any(feature = "stm32mp2", test))]
pub mod stm32_i2c;
/// ST's LCD-TFT display controller (LTDC).
#[cfg(any(feature = "stm32mp2", test))]
pub mod stm32_ltdc;
#[cfg(any(feature = "am62p", test))]
pub mod ti_dss7;
