// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ST's DSI wrapper and its D-PHY PLL on the STM32MP25 (DSI at 0x4800_0000);
//! follows Linux drm/stm/dw_mipi_dsi-stm.c.
//!
//! Source: ST's tree, github.com/STMicroelectronics/linux branch
//! `v6.6-stm32mp` at 548f960c059b (2026-06-01). Mainline's copy of the file
//! has only the STM32MP1 wrapper (DSI IP 1.30 / 1.31). The MP25 is IP 1.41
//! (`HWVER_141`, `dw_mipi_dsi-stm.c:29`), with `compatible = "st,stm32mp25-dsi"`
//! (`:1257`) and the PHY ops `dw_mipi_dsi_stm_phy_141_ops` (`:1235-1241`).
//! Every `:NNN` below is a line of that file unless another file is named.
//!
//! The DT node (`arch/arm64/boot/dts/st/stm32mp255.dtsi:123-135`): `reg =
//! <0x48000000 0x800>`, one region for the DesignWare host (0x000-0x3ff) and
//! ST's wrapper (0x400 and up). Its clocks are `pclk` = `CK_BUS_DSI`, `ref` =
//! `CK_KER_DSIPHY` (the PLL reference) and `px_clk` = `CK_KER_LTDC`. Its reset
//! is `DSI_R`, and its RIFSC id is 81. The node is also a clock provider
//! (`#clock-cells = <0>`).
//!
//! What the 1.41 wrapper is, from the code:
//!
//! - **No regulator in the wrapper.** The MP1's `WRPCR` (0x430: `PLLEN`,
//!   `REGEN`, `BGREN`, with the `WISR.RRS` / `WISR.PLLLS` polls, `:539-551`)
//!   is not used on 1.41. 0x430 is `WPCR1` there (`:67`), and the PLL moved to
//!   `WRPCR0`-`WRPCR2` (`:72-90`). The supplies are external: `vdd` and
//!   `vdda18` (SCMI regulators on the DK, `stm32mp257f-dk.dts:329-330`),
//!   enabled by the platform (`:1296-1322`), not by a register here.
//! - **The PLL.** `out = ref * ndiv / (idf * odf)` (`:198-207`), `idf` 1-16,
//!   `ndiv` 64-625 (`:109-112`). The lane bit rate is twice the PLL output
//!   (`:345`, `:477-478`, `:694-695`). `odf`, `hsfreqrange`, the VCO range
//!   and the charge pump come from the band table [`BANDS`] (`dppa_map`,
//!   `:273-337`), keyed by the lane rate. The search is [`search`] (`:339-397`).
//! - **The enable.** Linux waits for the clock lane's stop state
//!   (`PSR.PSSC`, a host register, `:532-533`) and then sets `WRPCR2.PLLEN`
//!   (`:537`). On a timeout it only logs "let's continue" (`:534-535`). There
//!   is no lock poll in the wrapper. The PLL lock is the DesignWare host's
//!   `PHY_STATUS` poll in its `dphy_enable`, which belongs to
//!   [`crate::ip::dw_mipi_dsi`].
//! - **The clock it provides.** On the MP25 this is `txbyteclk` (`:772-777`),
//!   with parent `clk_phy_dsi` = `CK_KER_DSIPHY`. It does **not** feed the
//!   LTDC. Its user is the RCC's `clk_lanebyte` mux (`RCC_DSICFGR.DSIBLSEL`
//!   bit 12, input `DSI_TXBYTE` or `ck_ker_ltdc`;
//!   `drivers/clk/stm32/clk-stm32mp25.c:166-169`, `:233`, `:915-920`). The
//!   LTDC's pixel clock is `ck_ker_ltdc` = `FLEXGEN_27` (`clk-stm32mp25.c:903-906`).
//!   The MP1's `ck_dsi_phy` (`:765-770`), which fed the LTDC there, does not
//!   exist on the MP25. Linux gives `txbyteclk` the lane bit rate as its rate
//!   (`:622-623`, `:664-665`). The byte clock on the lanes is that rate / 8.
//!
//! The registers `PCTLR` (0xa0), `PSR` (0xb0) and `PTCR0` (0xb4) are in the
//! host's half (the DesignWare `PHY_RSTZ`, `PHY_STATUS` and `PHY_TST_CTRL0`).
//! ST's PHY ops write them (`:96-107`), so they are written here, in the
//! order ST's code does. The host driver writes the same registers again in
//! its own `dphy_enable`.
//!
//! Where this deliberately differs from Linux:
//!
//! - Linux's `set_rate` re-derives the PLL target from `lane_mbps`, the
//!   *whole Mbit/s* the first search achieved (`:478-479`, `:695`). For 720p60
//!   on 4 lanes (445.5 Mbit/s) the truncation to 445 moves the PLL to `idf
//!   15`, `ndiv 247` (444.6 Mbit/s) instead of the exact `idf 4`, `ndiv 66`.
//!   [`init`] programs the search for the requested rate itself, which is
//!   what `mode_valid` checked (`:1163`). The test
//!   `linux_set_rate_truncates_the_720p_four_lane_target` pins Linux's
//!   value.
//! - `dppa_map`'s search runs past its end for a lane rate of 2500 Mbit/s
//!   or more (`:352-358` leaves `i == ARRAY_SIZE`, then `:365` reads
//!   `dppa_map[63]`). [`band_index`] stays on the last row, and [`init`]
//!   refuses rates outside 80-2500 Mbit/s (`:127-128`, as `mode_valid` does
//!   at `:1146-1150`).
//! - `CCF = (ref_MHz - 17) * 4` (`:702`) is written unmasked into a 6-bit
//!   field (`WPCR1_CCF`, `:68`). [`ccf`] refuses a reference that does not
//!   fit (below 17 MHz, or 33 MHz and up). The 40 MHz HSE would give 92,
//!   which spills into bit 6. The DK's reference is `FLEXGEN_28` at 27 MHz
//!   (docs/notes/stm32mp257f-dk-display.md, the clock tree section), CCF 40.
//! - The `PSR.PSSC` timeout sets `PLLEN` as Linux does, then returns
//!   `-ETIMEDOUT`, not 0. The caller decides whether to go on.

use crate::env::Env;
use crate::errno::{EINVAL, ENODEV, ETIMEDOUT};
use crate::mmio::{field, get, Mmio};
use crate::ylog;

/// Register offsets from the DSI base and their fields.
pub mod regs {
    /// The host's version register: `VERSION` bits 31:8 (`:32-33`).
    pub const VERSION_REG: u32 = 0x00;
    pub const VERSION: u32 = 0xffff_ff00;
    /// IP 1.41, the MP25 (`:29`).
    pub const HWVER_141: u32 = 0x3134_3100;

    /// Host PHY control (DesignWare `PHY_RSTZ`) (`:100-104`).
    pub const PCTLR: u32 = 0xa0;
    pub const PCTLR_PWEN: u32 = 1 << 0;
    pub const PCTLR_DEN: u32 = 1 << 1;
    pub const PCTLR_CKEN: u32 = 1 << 2;
    pub const PCTLR_UCKEN: u32 = 1 << 3;
    /// Host PHY status (DesignWare `PHY_STATUS`): `PSSC`, clock lane stop
    /// state (`:106-107`).
    pub const PSR: u32 = 0xb0;
    pub const PSR_PSSC: u32 = 1 << 2;
    /// Host PHY test control 0 (DesignWare `PHY_TST_CTRL0`) (`:96-98`).
    pub const PTCR0: u32 = 0xb4;
    pub const PTCR0_TRSEN: u32 = 1 << 0;
    pub const PTCR0_TCKEN: u32 = 1 << 1;

    /// Wrapper configuration (`:37-39`).
    pub const WCFGR: u32 = 0x400;
    pub const WCFGR_DSIM: u32 = 1 << 0;
    pub const WCFGR_COLMUX: u32 = 0b111 << 1;
    /// Wrapper control (`:41-42`).
    pub const WCR: u32 = 0x404;
    pub const WCR_DSIEN: u32 = 1 << 3;
    /// Wrapper PHY configuration 1, 1.41 only. The MP1's `WRPCR` sits at the
    /// same offset (`:52`, `:67-70`).
    pub const WPCR1: u32 = 0x430;
    pub const WPCR1_CCF: u32 = 0x3f;
    pub const WPCR1_HSFR: u32 = 0x7f << 8;
    pub const WPCR1_DLD: u32 = 1 << 16;
    /// PLL configuration 0: `IDF` holds idf - 1 and `NDIV` holds ndiv - 2
    /// (`:72-75`, written at `:716`, read back at `:601-602`).
    pub const WRPCR0: u32 = 0x434;
    pub const WRPCR0_IDF: u32 = 0xf;
    pub const WRPCR0_NDIV: u32 = 0x3ff << 4;
    pub const WRPCR0_PSC: u32 = 1 << 16;
    /// PLL configuration 1 (`:77-83`).
    pub const WRPCR1: u32 = 0x438;
    pub const WRPCR1_PROP: u32 = 0x3f;
    pub const WRPCR1_GMP: u32 = 0b11 << 6;
    pub const WRPCR1_INT: u32 = 0x3f << 8;
    pub const WRPCR1_BIAS: u32 = 0x7f << 16;
    pub const WRPCR1_VCO: u32 = 0xf << 24;
    /// The output divider as a power of two: odf = 2^ODF (`:606`).
    pub const WRPCR1_ODF: u32 = 0b11 << 28;
    /// PLL control 2 (`:85-90`).
    pub const WRPCR2: u32 = 0x43c;
    pub const WRPCR2_SEL: u32 = 0b11;
    pub const WRPCR2_PLLEN: u32 = 1 << 8;
    pub const WRPCR2_UPD: u32 = 1 << 16;
    pub const WRPCR2_CLR: u32 = 1 << 24;
    pub const WRPCR2_FPLL: u32 = 1 << 28;
}

use regs as r;

pub const IDF_MIN: u32 = 1; // :109
pub const IDF_MAX: u32 = 16; // :110
pub const NDIV_MIN: u32 = 64; // :111
pub const NDIV_MAX: u32 = 625; // :112
/// Lane bit rate limits of the 1.41 PHY, kbit/s (`:127-128`, set at
/// `:1356-1358`).
pub const LANE_MIN_KBPS: u32 = 80_000;
pub const LANE_MAX_KBPS: u32 = 2_500_000;
/// The VCO range ST's file declares (`:129-130`). Its code never checks it;
/// [`Pll::vco_khz`] lets a caller do so.
pub const FVCO_MIN_KHZ: u32 = 320_000;
pub const FVCO_MAX_KHZ: u32 = 1_250_000;
/// Linux's `WRPCR1` constants (`:706-708`).
pub const BIAS: u32 = 0x10;
pub const INT: u32 = 0x00;
pub const GMP: u32 = 0x01;
/// `readl_poll_timeout_atomic(..., SLEEP_US, TIMEOUT_US)` (`:133-134`).
pub const POLL_STEP_US: u32 = 1_000;
pub const POLL_TIMEOUT_US: u32 = 200_000;
/// Each `mdelay(1)` of the shadow-register and test-reset pulses
/// (`:417-427`, `:727-730`).
pub const PULSE_US: u32 = 1_000;

/// One row of `dppa_map` (`struct dphy_pll_parameter_map`, `:265-271`). The
/// row covers lane rates below `max_mbps` (`:353`). `odf` is the register
/// code; [`odf_divider`] gives the divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub max_mbps: u32,
    pub hs_freq: u8,
    pub odf: u8,
    pub vco: u8,
    pub prop: u8,
}

const fn b(max_mbps: u32, hs_freq: u8, odf: u8, vco: u8, prop: u8) -> Band {
    Band { max_mbps, hs_freq, odf, vco, prop }
}

/// `dppa_map` (`:273-337`), row by row.
#[rustfmt::skip]
pub const BANDS: [Band; 63] = [
    b(80, 0x00, 0x03, 0x0F, 0x0B),   // :274
    b(90, 0x10, 0x03, 0x0F, 0x0B),   // :275
    b(100, 0x20, 0x03, 0x0F, 0x0B),  // :276
    b(110, 0x30, 0x03, 0x09, 0x0B),  // :277
    b(120, 0x01, 0x03, 0x09, 0x0B),  // :278
    b(130, 0x11, 0x03, 0x09, 0x0B),  // :279
    b(140, 0x21, 0x03, 0x09, 0x0B),  // :280
    b(150, 0x31, 0x03, 0x09, 0x0B),  // :281
    b(160, 0x02, 0x02, 0x0F, 0x0B),  // :282
    b(170, 0x12, 0x02, 0x0F, 0x0B),  // :283
    b(180, 0x22, 0x02, 0x0F, 0x0B),  // :284
    b(190, 0x32, 0x02, 0x0F, 0x0B),  // :285
    b(205, 0x03, 0x02, 0x0F, 0x0B),  // :286
    b(220, 0x13, 0x02, 0x09, 0x0B),  // :287
    b(235, 0x23, 0x02, 0x09, 0x0B),  // :288
    b(250, 0x33, 0x02, 0x09, 0x0B),  // :289
    b(275, 0x04, 0x02, 0x09, 0x0B),  // :290
    b(300, 0x14, 0x02, 0x09, 0x0B),  // :291
    b(325, 0x25, 0x01, 0x0F, 0x0B),  // :292
    b(350, 0x35, 0x01, 0x0F, 0x0B),  // :293
    b(400, 0x05, 0x01, 0x0F, 0x0B),  // :294
    b(450, 0x16, 0x01, 0x09, 0x0B),  // :295
    b(500, 0x26, 0x01, 0x09, 0x0B),  // :296
    b(550, 0x37, 0x01, 0x09, 0x0B),  // :297
    b(600, 0x07, 0x01, 0x09, 0x0B),  // :298
    b(650, 0x18, 0x00, 0x0F, 0x0B),  // :299
    b(700, 0x28, 0x00, 0x0F, 0x0B),  // :300
    b(750, 0x39, 0x00, 0x0F, 0x0B),  // :301
    b(800, 0x09, 0x00, 0x0F, 0x0B),  // :302
    b(850, 0x19, 0x00, 0x09, 0x0B),  // :303
    b(900, 0x29, 0x00, 0x09, 0x0B),  // :304
    b(950, 0x3A, 0x00, 0x09, 0x0B),  // :305
    b(1000, 0x0A, 0x00, 0x09, 0x0B), // :306
    b(1050, 0x1A, 0x00, 0x09, 0x0B), // :307
    b(1100, 0x2A, 0x00, 0x09, 0x0B), // :308
    b(1150, 0x3B, 0x00, 0x09, 0x0B), // :309
    b(1200, 0x0B, 0x00, 0x09, 0x0B), // :310
    b(1250, 0x1B, 0x00, 0x09, 0x0B), // :311
    b(1300, 0x2B, 0x00, 0x03, 0x0B), // :312
    b(1350, 0x3C, 0x00, 0x03, 0x0B), // :313
    b(1400, 0x0C, 0x00, 0x03, 0x0B), // :314
    b(1450, 0x1C, 0x00, 0x03, 0x0B), // :315
    b(1500, 0x2C, 0x00, 0x03, 0x0B), // :316
    b(1550, 0x3D, 0x00, 0x03, 0x0B), // :317
    b(1600, 0x0D, 0x00, 0x03, 0x0B), // :318
    b(1650, 0x1D, 0x00, 0x03, 0x0B), // :319
    b(1700, 0x2E, 0x00, 0x03, 0x0B), // :320
    b(1750, 0x3E, 0x00, 0x03, 0x0B), // :321
    b(1800, 0x0E, 0x00, 0x03, 0x0B), // :322
    b(1850, 0x1E, 0x00, 0x03, 0x0B), // :323
    b(1900, 0x2F, 0x00, 0x03, 0x0B), // :324
    b(1950, 0x3F, 0x00, 0x03, 0x0B), // :325
    b(2000, 0x0F, 0x00, 0x03, 0x0B), // :326
    b(2050, 0x40, 0x00, 0x03, 0x0B), // :327
    b(2100, 0x41, 0x00, 0x03, 0x0B), // :328
    b(2150, 0x42, 0x00, 0x03, 0x0B), // :329
    b(2200, 0x43, 0x00, 0x01, 0x0B), // :330
    b(2250, 0x44, 0x00, 0x01, 0x0B), // :331
    b(2300, 0x45, 0x00, 0x01, 0x0C), // :332
    b(2350, 0x46, 0x00, 0x01, 0x0C), // :333
    b(2400, 0x47, 0x00, 0x01, 0x0C), // :334
    b(2450, 0x48, 0x00, 0x01, 0x0C), // :335
    b(2500, 0x49, 0x00, 0x01, 0x0C), // :336
];

/// The high-speed transition times of the 1.41 PHY in lane byte clocks
/// (`struct hstt`, `:919-933`). They go into the host's `PHY_TMR_CFG` /
/// `PHY_TMR_LPCLK_CFG` (the `dw_mipi_dsi_dphy_timing` the PHY op returns).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HsTransition {
    pub max_mbps: u32,
    pub clk_lp2hs: u16,
    pub clk_hs2lp: u16,
    pub data_lp2hs: u16,
    pub data_hs2lp: u16,
}

const fn t(max_mbps: u32, clk_lp2hs: u16, clk_hs2lp: u16, data_lp2hs: u16, data_hs2lp: u16) -> HsTransition {
    HsTransition { max_mbps, clk_lp2hs, clk_hs2lp, data_lp2hs, data_hs2lp }
}

/// `hstt_phy_141_table` (`:936-1000`): `HSTT(maxfreq, c_lp2hs, c_hs2lp,
/// d_lp2hs, d_hs2lp)`, row by row.
#[rustfmt::skip]
pub const HS_TRANSITIONS: [HsTransition; 63] = [
    t(80, 21, 17, 15, 10),     // :937
    t(90, 23, 17, 16, 10),     // :938
    t(100, 22, 17, 16, 10),    // :939
    t(110, 25, 18, 17, 11),    // :940
    t(120, 26, 20, 18, 11),    // :941
    t(130, 27, 19, 19, 11),    // :942
    t(140, 27, 19, 19, 11),    // :943
    t(150, 28, 20, 20, 12),    // :944
    t(160, 30, 21, 22, 13),    // :945
    t(170, 30, 21, 23, 13),    // :946
    t(180, 31, 21, 23, 13),    // :947
    t(190, 32, 22, 24, 13),    // :948
    t(205, 35, 22, 25, 13),    // :949
    t(220, 37, 26, 27, 15),    // :950
    t(235, 38, 28, 27, 16),    // :951
    t(250, 41, 29, 30, 17),    // :952
    t(275, 43, 29, 32, 18),    // :953
    t(300, 45, 32, 35, 19),    // :954
    t(325, 48, 33, 36, 18),    // :955
    t(350, 51, 35, 40, 20),    // :956
    t(400, 59, 37, 44, 21),    // :957
    t(450, 65, 40, 49, 23),    // :958
    t(500, 71, 41, 54, 24),    // :959
    t(550, 77, 44, 57, 26),    // :960
    t(600, 82, 46, 64, 27),    // :961
    t(650, 87, 48, 67, 28),    // :962
    t(700, 94, 52, 71, 29),    // :963
    t(750, 99, 52, 75, 31),    // :964
    t(800, 105, 55, 82, 32),   // :965
    t(850, 110, 58, 85, 32),   // :966
    t(900, 115, 58, 88, 35),   // :967
    t(950, 120, 62, 93, 36),   // :968
    t(1000, 128, 63, 99, 38),  // :969
    t(1050, 132, 65, 102, 38), // :970
    t(1100, 138, 67, 106, 39), // :971
    t(1150, 146, 69, 112, 42), // :972
    t(1200, 151, 71, 117, 43), // :973
    t(1250, 153, 74, 120, 45), // :974
    t(1300, 160, 73, 124, 46), // :975
    t(1350, 165, 76, 130, 47), // :976
    t(1400, 172, 78, 134, 49), // :977
    t(1450, 177, 80, 138, 49), // :978
    t(1500, 183, 81, 143, 52), // :979
    t(1550, 191, 84, 147, 52), // :980
    t(1600, 194, 85, 152, 52), // :981
    t(1650, 201, 86, 155, 53), // :982
    t(1700, 208, 88, 161, 53), // :983
    t(1750, 212, 89, 165, 53), // :984
    t(1800, 220, 90, 171, 54), // :985
    t(1850, 223, 92, 175, 55), // :986
    t(1900, 231, 91, 180, 56), // :987
    t(1950, 236, 95, 185, 56), // :988
    t(2000, 243, 97, 190, 58), // :989
    t(2050, 248, 99, 194, 59), // :990
    t(2100, 252, 100, 199, 61), // :991
    t(2150, 259, 102, 204, 62), // :992
    t(2200, 266, 105, 210, 63), // :993
    t(2250, 269, 109, 213, 65), // :994
    t(2300, 272, 109, 217, 66), // :995
    t(2350, 281, 112, 225, 66), // :996
    t(2400, 283, 115, 226, 67), // :997
    t(2450, 282, 115, 226, 67), // :998
    t(2500, 281, 118, 227, 68), // :999
];

/// The wrapper's colour coding, `WCFGR.COLMUX` (`enum dsi_color`, `:115-122`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ColorCoding {
    Rgb565Conf1 = 0,
    Rgb565Conf2 = 1,
    Rgb565Conf3 = 2,
    Rgb666Conf1 = 3,
    Rgb666Conf2 = 4,
    Rgb888 = 5,
}

/// The MIPI DSI pixel formats, mapped as `dsi_color_from_mipi` does
/// (`:181-196`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipiFormat {
    Rgb888,
    Rgb666,
    Rgb666Packed,
    Rgb565,
}

impl MipiFormat {
    pub fn coding(self) -> ColorCoding {
        match self {
            MipiFormat::Rgb888 => ColorCoding::Rgb888,            // :184-185
            MipiFormat::Rgb666 => ColorCoding::Rgb666Conf2,       // :186-187
            MipiFormat::Rgb666Packed => ColorCoding::Rgb666Conf1, // :188-189
            MipiFormat::Rgb565 => ColorCoding::Rgb565Conf1,       // :190-191
        }
    }

    /// Bits per pixel on the link (`mipi_dsi_pixel_format_to_bpp`).
    pub fn bpp(self) -> u32 {
        match self {
            MipiFormat::Rgb888 | MipiFormat::Rgb666 => 24,
            MipiFormat::Rgb666Packed => 18,
            MipiFormat::Rgb565 => 16,
        }
    }
}

/// The divider of an `odf` register code (`:365-374`, and `2^ODF` at `:606`).
pub const fn odf_divider(code: u8) -> u32 {
    match code {
        3 => 8,
        2 => 4,
        1 => 2,
        _ => 1,
    }
}

/// `dsi_pll_get_clkout_khz` (`:198-207`): `DIV_ROUND_CLOSEST(clkin * ndiv,
/// idf * odf)`, 0 for a zero divisor.
pub const fn clkout_khz(clkin_khz: u32, idf: u32, ndiv: u32, odf: u32) -> u32 {
    let div = idf as u64 * odf as u64;
    if div == 0 {
        return 0;
    }
    ((clkin_khz as u64 * ndiv as u64 + div / 2) / div) as u32
}

/// The `dppa_map` row for a lane rate in whole Mbit/s: the first row whose
/// `max_mbps` is above it (`:352-358`). It stays on the last row where Linux
/// would read one past it (see the module text).
pub fn band_index(lane_mbps: u32) -> usize {
    BANDS.iter().position(|b| lane_mbps < b.max_mbps).unwrap_or(BANDS.len() - 1)
}

/// The high-speed transition times for a lane rate in whole Mbit/s
/// (`dw_mipi_dsi_phy_141_get_timing`, `:1002-1018`; the last row above the
/// table, `:1012-1013`).
pub fn hs_transition(lane_mbps: u32) -> HsTransition {
    let i = HS_TRANSITIONS.iter().position(|h| lane_mbps < h.max_mbps).unwrap_or(HS_TRANSITIONS.len() - 1);
    HS_TRANSITIONS[i]
}

/// `WPCR1.CCF` for a reference clock: `(ref_MHz - 17) * 4` (`:702`, the
/// integer MHz of `pll_in_khz / 1000`). -EINVAL when it does not fit the
/// 6-bit field (`:68`).
pub fn ccf(ref_khz: u32) -> Result<u32, i32> {
    let mhz = ref_khz / 1000;
    if mhz < 17 {
        return Err(-EINVAL);
    }
    let v = (mhz - 17) * 4;
    if v > r::WPCR1_CCF {
        return Err(-EINVAL);
    }
    Ok(v)
}

/// One PLL setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pll {
    /// The reference it was computed for, kHz.
    pub ref_khz: u32,
    pub idf: u32,
    pub ndiv: u32,
    /// The divider (1, 2, 4, 8), not the register code.
    pub odf: u32,
    /// The [`BANDS`] row.
    pub band: usize,
}

impl Pll {
    /// The PLL output, kHz (`:198-207`).
    pub fn out_khz(&self) -> u32 {
        clkout_khz(self.ref_khz, self.idf, self.ndiv, self.odf)
    }

    /// The lane bit rate, kbit/s: twice the PLL output (`:477-478`).
    pub fn lane_kbps(&self) -> u32 {
        2 * self.out_khz()
    }

    /// The lane rate as Linux's `get_lane_mbps` reports it, whole Mbit/s
    /// (`:478`).
    pub fn lane_mbps(&self) -> u32 {
        self.lane_kbps() / 1000
    }

    /// The lane byte clock, Hz: the lane rate / 8. Linux gives the clock
    /// it provides (`txbyteclk`) the lane rate itself (`:622-623`). The RCC's
    /// `clk_lanebyte` takes it (see the module text).
    pub fn byte_clock_hz(&self) -> u32 {
        (u64::from(self.lane_kbps()) * 1000 / 8) as u32
    }

    /// The pixel clock the link carries, Hz: `DIV_ROUND_CLOSEST(1000 *
    /// pll_out * lanes * 2, bpp)` (`:1173`).
    pub fn pixel_clock_hz(&self, lanes: u32, bpp: u32) -> u32 {
        let n = 1000 * u64::from(self.out_khz()) * u64::from(lanes) * 2;
        let bpp = u64::from(bpp);
        ((n + bpp / 2) / bpp) as u32
    }

    /// The VCO, kHz: the PLL output times `odf`.
    pub fn vco_khz(&self) -> u32 {
        self.out_khz() * self.odf
    }

    /// The register values `set_rate` writes, in its order (`:702-720`):
    /// `WPCR1`, `WRPCR0`, `WRPCR1`.
    pub fn regs(&self) -> Result<[u32; 3], i32> {
        let band = BANDS[self.band];
        let wpcr1 = field(r::WPCR1_HSFR, u32::from(band.hs_freq)) | ccf(self.ref_khz)?; // :713
        let wrpcr0 = field(r::WRPCR0_NDIV, self.ndiv - 2) | field(r::WRPCR0_IDF, self.idf - 1); // :716
        let wrpcr1 = field(r::WRPCR1_ODF, u32::from(band.odf)) // :719, odf code from :710
            | field(r::WRPCR1_VCO, u32::from(band.vco))
            | field(r::WRPCR1_BIAS, BIAS)
            | field(r::WRPCR1_INT, INT)
            | field(r::WRPCR1_GMP, GMP)
            | field(r::WRPCR1_PROP, u32::from(band.prop));
        Ok([wpcr1, wrpcr0, wrpcr1])
    }
}

/// `dsi_phy_141_pll_get_params` (`:339-397`) for a PLL output of
/// `clkout_khz` (half the lane rate): the band from the lane rate `2 *
/// clkout / 1000` Mbit/s (`:345`, `:352-358`), `odf` from the band
/// (`:365-374`), then every `idf` 1-16 and `ndiv` 64-625 in that order. The
/// first strictly smaller error wins (`:385`), and an exact one ends the
/// search (`:391-392`). -EINVAL for a zero input (`:348-349`), or when
/// nothing comes within Linux's starting bound of 1 000 000 kHz (`:376`;
/// Linux would then go on with zero dividers).
pub fn search(ref_khz: u32, clkout_khz_target: u32) -> Result<Pll, i32> {
    if ref_khz == 0 || clkout_khz_target == 0 {
        return Err(-EINVAL);
    }
    let lane_mbps = 2 * clkout_khz_target / 1000;
    let band = band_index(lane_mbps);
    let odf = odf_divider(BANDS[band].odf);
    let mut best: Option<(u32, u32)> = None;
    let mut best_delta = 1_000_000u32;
    for idf in IDF_MIN..=IDF_MAX {
        for ndiv in NDIV_MIN..=NDIV_MAX {
            let delta = clkout_khz(ref_khz, idf, ndiv, odf).abs_diff(clkout_khz_target);
            if delta < best_delta {
                best = Some((idf, ndiv));
                best_delta = delta;
            }
            if delta == 0 {
                return Ok(Pll { ref_khz, idf, ndiv, odf, band });
            }
        }
    }
    best.map(|(idf, ndiv)| Pll { ref_khz, idf, ndiv, odf, band }).ok_or(-EINVAL)
}

/// The lane rate a mode needs, kbit/s: `pixel * bpp / lanes`, plus 20 % in
/// burst mode (`dw_mipi_dsi_stm_phy_141_mode_valid`, `:1140-1144`).
pub fn lane_kbps_for(pixel_khz: u32, bpp: u32, lanes: u32, burst: bool) -> u32 {
    let k = (u64::from(pixel_khz) * u64::from(bpp) / u64::from(lanes)) as u32;
    if burst {
        k * 12 / 10
    } else {
        k
    }
}

/// Linux's default clock tolerance, per mille (`:1020-1021`).
pub const CLOCK_TOLERANCE_PERMILLE: u32 = 5;

/// The non-burst check of `mode_valid` (`:1177-1186`): the pixel clock the
/// link carries within `CLOCK_TOLERANCE_PERMILLE` of the mode's.
pub fn pixel_clock_within_tolerance(pll: &Pll, lanes: u32, bpp: u32, pixel_khz: u32) -> bool {
    let px = u64::from(pll.pixel_clock_hz(lanes, bpp));
    let min = u64::from(pixel_khz) * u64::from(1000 - CLOCK_TOLERANCE_PERMILLE);
    let max = u64::from(pixel_khz) * u64::from(1000 + CLOCK_TOLERANCE_PERMILLE);
    (min..=max).contains(&px)
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, val: u32) {
    m.write32(base + off as usize, val)
}

/// `dsi_set` / `dsi_clear` / `dsi_update_bits` (`:165-179`): a read, then a
/// write of the whole register.
fn update(m: &impl Mmio, base: usize, off: u32, mask: u32, val: u32) {
    let v = rd(m, base, off);
    wr(m, base, off, (v & !mask) | val);
}

fn pulse(m: &impl Mmio, env: &impl Env, base: usize, off: u32, bit: u32) {
    update(m, base, off, bit, bit);
    env.busy_wait_us(PULSE_US);
    update(m, base, off, bit, 0);
    env.busy_wait_us(PULSE_US);
}

/// The DSI block's version is IP 1.41 (`:1344-1352`), or -ENODEV.
pub fn probe(m: &impl Mmio, env: &impl Env, base: usize) -> Result<(), i32> {
    let v = rd(m, base, r::VERSION_REG) & r::VERSION;
    if v != r::HWVER_141 {
        ylog!(env, Err, "DSI: version {:#010x}, not 1.41 ({:#010x})", v, r::HWVER_141);
        return Err(-ENODEV);
    }
    Ok(())
}

/// Program the PLL dividers and bring the PHY's control up, as `set_rate`
/// does on 1.41 (`:702-732`): `WPCR1` (HSFR, CCF), `WRPCR0`, `WRPCR1`,
/// `PCTLR = CKEN`, `WRPCR2.SEL = 1`, a pulse of `WRPCR2.UPD` (the shadow
/// registers copied), then `PCTLR |= PWEN | DEN`.
pub fn set_rate(m: &impl Mmio, env: &impl Env, base: usize, pll: &Pll) -> Result<(), i32> {
    let [wpcr1, wrpcr0, wrpcr1] = pll.regs()?;
    wr(m, base, r::WPCR1, wpcr1); // :713-714
    wr(m, base, r::WRPCR0, wrpcr0); // :716-717
    wr(m, base, r::WRPCR1, wrpcr1); // :719-720
    wr(m, base, r::PCTLR, r::PCTLR_CKEN); // :722
    update(m, base, r::WRPCR2, r::WRPCR2_SEL, 0x01); // :724
    pulse(m, env, base, r::WRPCR2, r::WRPCR2_UPD); // :726-730
    update(m, base, r::PCTLR, r::PCTLR_PWEN | r::PCTLR_DEN, r::PCTLR_PWEN | r::PCTLR_DEN); // :732
    Ok(())
}

/// `clk_enable` on 1.41 (`:531-537`): wait for `PSR.PSSC` (1 ms steps, 200
/// ms), then set `WRPCR2.PLLEN`. `PLLEN` is set even after a timeout, as
/// Linux does, but the timeout is returned as -ETIMEDOUT.
pub fn enable_pll(m: &impl Mmio, env: &impl Env, base: usize) -> Result<(), i32> {
    let mut waited = 0;
    let stop_state = loop {
        if rd(m, base, r::PSR) & r::PSR_PSSC != 0 {
            break true;
        }
        if waited >= POLL_TIMEOUT_US {
            break false;
        }
        env.busy_wait_us(POLL_STEP_US);
        waited += POLL_STEP_US;
    };
    if !stop_state {
        ylog!(env, Err, "DSI PHY: no clock lane stop state (PSR {:#010x}) in 200 ms", rd(m, base, r::PSR));
    }
    update(m, base, r::WRPCR2, r::WRPCR2_PLLEN, r::WRPCR2_PLLEN); // :537
    if stop_state {
        Ok(())
    } else {
        Err(-ETIMEDOUT)
    }
}

/// `clk_disable` on 1.41 (`:499-501`): clear `WRPCR2.PLLEN`.
pub fn disable_pll(m: &impl Mmio, base: usize) {
    update(m, base, r::WRPCR2, r::WRPCR2_PLLEN, 0);
}

/// Whether `WRPCR2.PLLEN` is set (`clk_is_enabled`, `:570-571`).
pub fn pll_enabled(m: &impl Mmio, base: usize) -> bool {
    rd(m, base, r::WRPCR2) & r::WRPCR2_PLLEN != 0
}

/// The PLL as the registers hold it (`recalc_rate`, `:598-606`): `idf` =
/// `WRPCR0.IDF + 1`, `ndiv` = `WRPCR0.NDIV + 2`, `odf` = `2^WRPCR1.ODF`.
pub fn read_pll(m: &impl Mmio, base: usize, ref_hz: u32) -> Pll {
    let w0 = rd(m, base, r::WRPCR0);
    let w1 = rd(m, base, r::WRPCR1);
    let ref_khz = ref_hz / 1000;
    let mut p = Pll { ref_khz, idf: get(r::WRPCR0_IDF, w0) + 1, ndiv: get(r::WRPCR0_NDIV, w0) + 2, odf: 1, band: 0 };
    p.odf = 1 << get(r::WRPCR1_ODF, w1);
    p.band = band_index(p.lane_mbps());
    p
}

/// Bring up the wrapper and the PHY PLL for `lane_kbps` per lane from a
/// `ref_hz` reference (`CK_KER_DSIPHY`). This is `dw_mipi_dsi_phy_141_init`
/// (`:399-442`) with the `clk_set_rate` and `clk_prepare_enable` of the
/// clock it provides done inline:
///
/// 1. video mode and colour coding in `WCFGR` (`:406-411`);
/// 2. `PCTLR = 0` (`:413`);
/// 3. a pulse of `WRPCR2.CLR` (shadow registers to reset values, `:415-420`);
/// 4. a pulse of `PTCR0.TRSEN` (the PHY test interface reset, `:422-427`);
/// 5. `PLLEN` cleared if it was set (`:429-430`, `:499-501`);
/// 6. [`set_rate`] (`:432-437`, `:693-732`);
/// 7. [`enable_pll`] (`:439`, `:531-537`).
///
/// The PLL target is `lane_kbps / 2` (`:1163`), refused outside 80-2500
/// Mbit/s (`:1146-1150`). Returns the PLL programmed. A step-7 timeout
/// returns -ETIMEDOUT after the registers are written as Linux leaves them.
pub fn init(
    m: &impl Mmio,
    env: &impl Env,
    base: usize,
    ref_hz: u32,
    lane_kbps: u32,
    coding: ColorCoding,
) -> Result<Pll, i32> {
    if !(LANE_MIN_KBPS..=LANE_MAX_KBPS).contains(&lane_kbps) {
        ylog!(env, Err, "DSI PHY: lane rate {} kbit/s outside {}..={}", lane_kbps, LANE_MIN_KBPS, LANE_MAX_KBPS);
        return Err(-EINVAL);
    }
    let ref_khz = ref_hz / 1000;
    ccf(ref_khz).inspect_err(|_| ylog!(env, Err, "DSI PHY: reference {} Hz outside 17-32 MHz", ref_hz))?;
    let pll = search(ref_khz, lane_kbps / 2)?;

    update(m, base, r::WCFGR, r::WCFGR_DSIM, 0); // :407
    update(m, base, r::WCFGR, r::WCFGR_COLMUX, (coding as u32) << 1); // :410-411
    wr(m, base, r::PCTLR, 0); // :413
    pulse(m, env, base, r::WRPCR2, r::WRPCR2_CLR); // :416-420
    pulse(m, env, base, r::PTCR0, r::PTCR0_TRSEN); // :423-427
    if pll_enabled(m, base) {
        disable_pll(m, base); // :429-430
    }
    set_rate(m, env, base, &pll)?;
    let locked = enable_pll(m, env, base);
    ylog!(
        env,
        Inf,
        "DSI PHY: ref {} kHz, idf {} ndiv {} odf {}: {} kbit/s per lane",
        ref_khz,
        pll.idf,
        pll.ndiv,
        pll.odf,
        pll.lane_kbps()
    );
    locked.map(|()| pll)
}

/// `phy_power_on` (`:828-836`): set `WCR.DSIEN`.
pub fn power_on(m: &impl Mmio, base: usize) {
    update(m, base, r::WCR, r::WCR_DSIEN, r::WCR_DSIEN);
}

/// `phy_power_off` (`:838-848`): the PLL off (`clk_disable_unprepare`, then
/// `:499-501`), then clear `WCR.DSIEN`.
pub fn power_off(m: &impl Mmio, base: usize) {
    disable_pll(m, base);
    update(m, base, r::WCR, r::WCR_DSIEN, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::Level;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const BASE: usize = 0x4800_0000;
    /// The DK's PHY reference: `FLEXGEN_28` = PLL8 594 MHz / 22 = 27 MHz
    /// (docs/notes/stm32mp257f-dk-display.md, the clock tree section).
    const REF_HZ: u32 = 27_000_000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Ev {
        R(u32),
        W(u32, u32),
        Wait(u32),
    }
    use Ev::{Wait, R, W};

    /// A recording fake of the DSI region and the environment: registers
    /// hold what was written, `PSR` reads `psr`, and every read, write and
    /// wait lands in one ordered trace.
    #[derive(Default)]
    struct Pin {
        regs: RefCell<BTreeMap<u32, u32>>,
        trace: RefCell<Vec<Ev>>,
        lines: RefCell<Vec<(Level, String)>>,
    }

    impl Pin {
        fn with(init: &[(u32, u32)]) -> Pin {
            let p = Pin::default();
            p.regs.borrow_mut().extend(init.iter().copied());
            p
        }
        fn off(addr: usize) -> u32 {
            (addr - BASE) as u32
        }
        fn peek(&self, off: u32) -> u32 {
            self.regs.borrow().get(&off).copied().unwrap_or(0)
        }
        fn trace(&self) -> Vec<Ev> {
            self.trace.borrow().clone()
        }
        fn writes(&self) -> Vec<(u32, u32)> {
            self.trace().into_iter().filter_map(|e| if let W(o, v) = e { Some((o, v)) } else { None }).collect()
        }
    }

    impl Mmio for Pin {
        fn read32(&self, addr: usize) -> u32 {
            let o = Pin::off(addr);
            self.trace.borrow_mut().push(R(o));
            self.peek(o)
        }
        fn write32(&self, addr: usize, val: u32) {
            let o = Pin::off(addr);
            self.trace.borrow_mut().push(W(o, val));
            self.regs.borrow_mut().insert(o, val);
        }
        fn read8(&self, _addr: usize) -> u8 {
            unreachable!("32-bit block")
        }
        fn write8(&self, _addr: usize, _val: u8) {
            unreachable!("32-bit block")
        }
    }

    impl Env for Pin {
        fn busy_wait_us(&self, us: u32) {
            self.trace.borrow_mut().push(Wait(us));
        }
        fn sleep_ms(&self, ms: u32) {
            self.trace.borrow_mut().push(Wait(ms * 1000));
        }
        fn log_str(&self, level: Level, line: &str) {
            self.lines.borrow_mut().push((level, line.to_string()));
        }
        fn i2c_write(&self, _bus: u32, _buf: &[u8]) -> Result<(), i32> {
            unreachable!()
        }
        fn i2c_write_read(&self, _bus: u32, _w: &[u8], _r: &mut [u8]) -> Result<(), i32> {
            unreachable!()
        }
        fn gpio_set(&self, _line: u32, _value: bool) -> Result<(), i32> {
            unreachable!()
        }
    }

    #[test]
    fn tables_have_ascending_edges_and_match_row_for_row() {
        assert_eq!(BANDS.len(), 337 - 274, "dppa_map rows :274-336");
        assert_eq!(HS_TRANSITIONS.len(), 1000 - 937, "hstt rows :937-999");
        for (b, h) in BANDS.iter().zip(HS_TRANSITIONS.iter()) {
            assert_eq!(b.max_mbps, h.max_mbps);
        }
        assert!(BANDS.windows(2).all(|w| w[0].max_mbps < w[1].max_mbps));
        // spot rows against the file
        assert_eq!(BANDS[21], Band { max_mbps: 450, hs_freq: 0x16, odf: 1, vco: 9, prop: 0xb }); // :295
        assert_eq!(BANDS[30], Band { max_mbps: 900, hs_freq: 0x29, odf: 0, vco: 9, prop: 0xb }); // :304
        assert_eq!(BANDS[62], Band { max_mbps: 2500, hs_freq: 0x49, odf: 0, vco: 1, prop: 0xc }); // :336
        assert_eq!(HS_TRANSITIONS[0], t(80, 21, 17, 15, 10)); // :937
        assert_eq!(HS_TRANSITIONS[62], t(2500, 281, 118, 227, 68)); // :999
    }

    #[test]
    fn band_lookup_is_strictly_below_the_edge() {
        // :353 `lane_mbps < dppa_map[i].data_rate`
        assert_eq!(band_index(0), 0);
        assert_eq!(band_index(79), 0);
        assert_eq!(band_index(80), 1);
        assert_eq!(band_index(445), 21); // row 450
        assert_eq!(band_index(450), 22); // row 500
        assert_eq!(band_index(891), 30); // row 900
        assert_eq!(band_index(2499), 62);
        // Linux reads dppa_map[63] here; we stay on the last row
        assert_eq!(band_index(2500), 62);
        assert_eq!(hs_transition(445), t(450, 65, 40, 49, 23)); // :958
        assert_eq!(hs_transition(891), t(900, 115, 58, 88, 35)); // :967
        assert_eq!(hs_transition(9999), t(2500, 281, 118, 227, 68)); // :1012-1013
        assert_eq!([odf_divider(3), odf_divider(2), odf_divider(1), odf_divider(0)], [8, 4, 2, 1]);
        // :365-374
    }

    #[test]
    fn ccf_is_ref_mhz_minus_17_times_4_and_must_fit_six_bits() {
        // :702 ((pll_in_khz / 1000 - 17)) * 4
        assert_eq!(ccf(27_000), Ok(40));
        assert_eq!(ccf(17_000), Ok(0));
        assert_eq!(ccf(32_999), Ok(60));
        // 40 MHz HSE: 92 = 0b101_1100 does not fit WPCR1_CCF GENMASK(5, 0) (:68)
        assert_eq!(ccf(40_000), Err(-EINVAL));
        assert_eq!(ccf(16_999), Err(-EINVAL));
    }

    /// 1280x720@60 (CEA VIC 4, 74.25 MHz), RGB888, 4 lanes, non-burst (the
    /// ADV7535's flags, adv7533.c, have no MIPI_DSI_MODE_VIDEO_BURST).
    #[test]
    fn pll_720p60_rgb888_four_lanes() {
        // :1140 lane_kbps = 74250 * 24 / 4 = 445500 kbit/s; no 20 % (:1143)
        let lane = lane_kbps_for(74_250, 24, 4, false);
        assert_eq!(lane, 445_500);
        // :1163 the PLL target is lane_kbps / 2 = 222750 kHz.
        // :345 lane_mbps = 2 * 222750 / 1000 = 445 -> row 450 (:295):
        // odf code 1 -> odf 2 (:370-371).
        // :378-393 out = round(27000 * ndiv / (idf * 2)):
        //   idf 1: 13500 * ndiv >= 864000 at ndiv 64, never 222750;
        //   idf 2: 6750 * 64 = 432000 already too high;
        //   idf 3: 4500 * 64 = 288000 too high;
        //   idf 4: 3375 * ndiv = 222750 at ndiv 66: exact, the search ends.
        let p = search(REF_HZ / 1000, lane / 2).unwrap();
        assert_eq!((p.idf, p.ndiv, p.odf, p.band), (4, 66, 2, 21));
        assert_eq!(p.out_khz(), 222_750);
        assert_eq!(p.lane_kbps(), 445_500);
        assert_eq!(p.lane_mbps(), 445); // :478, what get_lane_mbps reports
        assert_eq!(p.vco_khz(), 445_500); // inside 320-1250 MHz (:129-130)
        assert_eq!(p.byte_clock_hz(), 55_687_500);
        // :1173 px = 1000 * 222750 * 4 * 2 / 24 = 74 250 000 Hz exactly
        assert_eq!(p.pixel_clock_hz(4, 24), 74_250_000);
        assert!(pixel_clock_within_tolerance(&p, 4, 24, 74_250));
        // :713 WPCR1 = hsfreq 0x16 << 8 | ccf 40; :716 WRPCR0 = (66 - 2) << 4
        // | (4 - 1); :719 WRPCR1 = odf code 1 << 28 | vco 9 << 24 | bias 0x10
        // << 16 | int 0 << 8 | gmp 1 << 6 | prop 0xb
        assert_eq!(p.regs(), Ok([0x0000_1628, 0x0000_0403, 0x1910_004b]));
    }

    /// What Linux programs for the same mode: `get_lane_mbps` stores the
    /// whole Mbit/s, 445 (:478-479), and `set_rate` searches again for 445 *
    /// 1000 / 2 = 222500 kHz (:695). No idf/ndiv hits 222500 exactly:
    /// idf 4 ndiv 66 gives 222750 (+250); idf 15 ndiv 247 gives
    /// round(27000 x 247 / 30) = 222300 (-200), the first strictly better one
    /// (:385).
    /// The result is 444.6 Mbit/s, 0.2 % slow. [`init`] does not do this.
    #[test]
    fn linux_set_rate_truncates_the_720p_four_lane_target() {
        let p = search(27_000, 445 * 1000 / 2).unwrap();
        assert_eq!((p.idf, p.ndiv, p.odf), (15, 247, 2));
        assert_eq!(p.out_khz(), 222_300);
        assert_eq!(p.pixel_clock_hz(4, 24), 74_100_000);
    }

    /// 1920x1080@60 (VIC 16, 148.5 MHz), RGB888, 4 lanes.
    #[test]
    fn pll_1080p60_148_5_mhz_four_lanes() {
        // :1140 lane_kbps = 148500 * 24 / 4 = 891000; PLL target 445500 kHz.
        // :345 lane_mbps 891 -> row 900 (:304): odf code 0 -> odf 1.
        // out = round(27000 * ndiv / idf): idf 1-3 give at least 576000 at
        // ndiv 64; idf 4: 6750 * 66 = 445500 exact.
        let lane = lane_kbps_for(148_500, 24, 4, false);
        assert_eq!(lane, 891_000);
        let p = search(27_000, lane / 2).unwrap();
        assert_eq!((p.idf, p.ndiv, p.odf, p.band), (4, 66, 1, 30));
        assert_eq!((p.lane_kbps(), p.lane_mbps()), (891_000, 891));
        assert_eq!(p.pixel_clock_hz(4, 24), 148_500_000);
        // 891 * 1000 / 2 = 445500: Linux's set_rate (:695) lands on the same
        assert_eq!(search(27_000, 891 * 1000 / 2), Ok(p));
        assert_eq!(p.regs(), Ok([0x0000_2928, 0x0000_0403, 0x0910_004b]));
    }

    /// 720p60 on 3 lanes, which the ADV7533/7535 code picks below 80 MHz
    /// (adv7533.c, docs/notes/stm32mp257f-dk-display.md, the modes table).
    #[test]
    fn pll_720p60_three_lanes() {
        // 74250 * 24 / 3 = 594000; target 297000; row 600 (:298), odf 2;
        // round(27000 * ndiv / (idf * 2)) = 13500 * ndiv / idf: idf 3 ndiv 66
        let p = search(27_000, lane_kbps_for(74_250, 24, 3, false) / 2).unwrap();
        assert_eq!((p.idf, p.ndiv, p.odf, p.band), (3, 66, 2, 24));
        assert_eq!(p.regs(), Ok([0x0000_0728, 0x0000_0402, 0x1910_004b]));
    }

    #[test]
    fn burst_adds_twenty_percent() {
        assert_eq!(lane_kbps_for(74_250, 24, 4, true), 534_600); // :1143-1144
    }

    #[test]
    fn search_rejects_zero_inputs() {
        assert_eq!(search(0, 222_750), Err(-EINVAL)); // :348-349
        assert_eq!(search(27_000, 0), Err(-EINVAL));
    }

    /// The whole `phy_141_init` + `set_rate` + `clk_enable` trace for 720p60
    /// on 4 lanes, from reset values (all zero) with the clock lane in stop
    /// state.
    #[test]
    fn init_sequence_720p_four_lanes() {
        let pin = Pin::with(&[(r::PSR, r::PSR_PSSC)]);
        let p = init(&pin, &pin, BASE, REF_HZ, 445_500, MipiFormat::Rgb888.coding()).unwrap();
        assert_eq!((p.idf, p.ndiv, p.odf), (4, 66, 2));
        let expected = vec![
            R(r::WCFGR),
            W(r::WCFGR, 0), // :407 DSIM cleared: video mode
            R(r::WCFGR),
            W(r::WCFGR, 5 << 1), // :410 COLMUX = DSI_RGB888 (5, :121)
            W(r::PCTLR, 0),      // :413
            R(r::WRPCR2),
            W(r::WRPCR2, r::WRPCR2_CLR), // :416
            Wait(1000),                  // :417
            R(r::WRPCR2),
            W(r::WRPCR2, 0), // :419
            Wait(1000),      // :420
            R(r::PTCR0),
            W(r::PTCR0, r::PTCR0_TRSEN), // :423
            Wait(1000),                  // :424
            R(r::PTCR0),
            W(r::PTCR0, 0),             // :426
            Wait(1000),                 // :427
            R(r::WRPCR2),               // :429, :571 is the PLL on? no
            W(r::WPCR1, 0x1628),        // :713-714
            W(r::WRPCR0, 0x403),        // :716-717
            W(r::WRPCR1, 0x1910_004b),  // :719-720
            W(r::PCTLR, r::PCTLR_CKEN), // :722
            R(r::WRPCR2),
            W(r::WRPCR2, 0x1), // :724 SEL = 1
            R(r::WRPCR2),
            W(r::WRPCR2, 0x1_0001), // :726 UPD
            Wait(1000),             // :727
            R(r::WRPCR2),
            W(r::WRPCR2, 0x1), // :729
            Wait(1000),        // :730
            R(r::PCTLR),
            W(r::PCTLR, 0x7), // :732 PWEN | DEN | CKEN
            R(r::PSR),        // :532 PSSC already set
            R(r::WRPCR2),
            W(r::WRPCR2, 0x101), // :537 PLLEN
        ];
        assert_eq!(pin.trace(), expected);
        assert!(pll_enabled(&pin, BASE));
        // read back as recalc_rate does (:598-606)
        let back = read_pll(&pin, BASE, REF_HZ);
        assert_eq!((back.idf, back.ndiv, back.odf), (4, 66, 2));
        assert_eq!(back.lane_kbps(), 445_500);
    }

    /// The MP25 wrapper has no regulator: nothing sets the MP1's WRPCR
    /// REGEN (bit 24) / BGREN (bit 28) at 0x430, and nothing polls WISR
    /// (0x40c) for RRS / PLLLS (:539-551 are the MP1 branch only).
    #[test]
    fn no_regulator_on_the_mp25() {
        let pin = Pin::with(&[(r::PSR, r::PSR_PSSC)]);
        init(&pin, &pin, BASE, REF_HZ, 891_000, ColorCoding::Rgb888).unwrap();
        let at_430: Vec<u32> = pin.writes().into_iter().filter(|w| w.0 == 0x430).map(|w| w.1).collect();
        assert_eq!(at_430, vec![0x2928], "only WPCR1 (hsfreq, ccf)");
        assert!(at_430.iter().all(|v| v & (1 << 24 | 1 << 28) == 0));
        assert!(!pin.trace().contains(&R(0x40c)));
    }

    #[test]
    fn a_running_pll_is_stopped_before_the_new_rate() {
        // :429-430 clk_disable_unprepare when txbyteclk is on: PLLEN cleared
        let pin = Pin::with(&[(r::PSR, r::PSR_PSSC), (r::WRPCR2, r::WRPCR2_PLLEN | 1)]);
        init(&pin, &pin, BASE, REF_HZ, 891_000, ColorCoding::Rgb888).unwrap();
        let w = pin.writes();
        let stop = w.iter().position(|&x| x == (r::WRPCR2, 0x1)).unwrap();
        let wpcr1 = w.iter().position(|x| x.0 == r::WPCR1).unwrap();
        assert!(stop < wpcr1, "{w:?}");
        assert_eq!(pin.peek(r::WRPCR2) & r::WRPCR2_PLLEN, r::WRPCR2_PLLEN, "on again at the end");
    }

    /// PSSC never comes: 201 reads 1 ms apart (200 ms, :133-134, :532-533),
    /// PLLEN set anyway (:537), -ETIMEDOUT and an error line.
    #[test]
    fn stop_state_timeout() {
        let pin = Pin::default();
        assert_eq!(enable_pll(&pin, &pin, BASE), Err(-ETIMEDOUT));
        let tr = pin.trace();
        let polls = tr.iter().filter(|e| **e == R(r::PSR)).count();
        // 201 polls and one more read for the log line
        assert_eq!(polls, 202);
        let waited: u32 = tr.iter().map(|e| if let Wait(us) = e { *us } else { 0 }).sum();
        assert_eq!(waited, POLL_TIMEOUT_US);
        assert_eq!(&tr[tr.len() - 2..], &[R(r::WRPCR2), W(r::WRPCR2, r::WRPCR2_PLLEN)]);
        assert_eq!(pin.lines.borrow()[0].0, Level::Err);

        // and through init: every register written as on success
        let pin = Pin::default();
        assert_eq!(init(&pin, &pin, BASE, REF_HZ, 445_500, ColorCoding::Rgb888), Err(-ETIMEDOUT));
        assert_eq!(pin.peek(r::WRPCR0), 0x403);
        assert_eq!(pin.peek(r::PCTLR), 0x7);
        assert!(pll_enabled(&pin, BASE));
    }

    #[test]
    fn stop_state_after_a_few_polls() {
        // the fake answers PSSC from the third read on
        struct Late(Pin, RefCell<u32>);
        impl Mmio for Late {
            fn read32(&self, addr: usize) -> u32 {
                if Pin::off(addr) == r::PSR {
                    *self.1.borrow_mut() += 1;
                    self.0.trace.borrow_mut().push(R(r::PSR));
                    return if *self.1.borrow() >= 3 { r::PSR_PSSC } else { 0 };
                }
                self.0.read32(addr)
            }
            fn write32(&self, addr: usize, val: u32) {
                self.0.write32(addr, val)
            }
            fn read8(&self, _: usize) -> u8 {
                unreachable!()
            }
            fn write8(&self, _: usize, _: u8) {
                unreachable!()
            }
        }
        let late = Late(Pin::default(), RefCell::new(0));
        assert_eq!(enable_pll(&late, &late.0, BASE), Ok(()));
        assert_eq!(
            late.0.trace(),
            vec![R(r::PSR), Wait(1000), R(r::PSR), Wait(1000), R(r::PSR), R(r::WRPCR2), W(r::WRPCR2, 0x100)]
        );
    }

    #[test]
    fn init_refuses_rates_and_references_outside_the_limits() {
        let pin = Pin::with(&[(r::PSR, r::PSR_PSSC)]);
        assert_eq!(init(&pin, &pin, BASE, REF_HZ, 79_999, ColorCoding::Rgb888), Err(-EINVAL)); // :127
        assert_eq!(init(&pin, &pin, BASE, REF_HZ, 2_500_001, ColorCoding::Rgb888), Err(-EINVAL)); // :128
        assert_eq!(init(&pin, &pin, BASE, 40_000_000, 891_000, ColorCoding::Rgb888), Err(-EINVAL)); // ccf 92
        assert!(pin.trace().is_empty(), "nothing touched");
    }

    #[test]
    fn power_on_and_off_toggle_dsien_and_the_pll() {
        let pin = Pin::with(&[(r::WRPCR2, 0x101)]);
        power_on(&pin, BASE); // :835
        assert_eq!(pin.writes(), vec![(r::WCR, r::WCR_DSIEN)]);
        power_off(&pin, BASE); // :844-847
        assert_eq!(pin.writes()[1..], [(r::WRPCR2, 0x1), (r::WCR, 0)]);
    }

    #[test]
    fn probe_wants_ip_1_41() {
        let pin = Pin::with(&[(r::VERSION_REG, 0x3134_312a)]); // low byte ignored (:33)
        assert_eq!(probe(&pin, &pin, BASE), Ok(()));
        let pin = Pin::with(&[(r::VERSION_REG, 0x3133_3100)]); // the MP1's 1.31
        assert_eq!(probe(&pin, &pin, BASE), Err(-ENODEV));
    }

    #[test]
    fn colour_codings_follow_dsi_color_from_mipi() {
        assert_eq!(MipiFormat::Rgb888.coding() as u32, 5);
        assert_eq!(MipiFormat::Rgb666.coding() as u32, 4);
        assert_eq!(MipiFormat::Rgb666Packed.coding() as u32, 3);
        assert_eq!(MipiFormat::Rgb565.coding() as u32, 0);
    }
}
