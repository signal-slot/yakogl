// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95's DSI transmit path behind the Synopsys host
//! (`ip::dw_mipi_dsi`): the NXP CSRs around it and the combo D-PHY (a
//! Synopsys D-PHY shared by the DSI host and the CSI-2 receiver 1) with its
//! PLL. This is the [`Phy`] the host calls.
//!
//! Sources. Register map and fields: hal_nxp (BSD-3-Clause, 1e91007b;
//! the CSRs through the generated `soc::imx95::regs`)
//! `mcux-sdk-ng/devices/i.MX/i.MX95/periph/PERI_CAMERA_DSI_STREAM_CSR.h`,
//! `PERI_CAMERA_DSI_MASTER_CSR.h`, `PERI_CAMERA_DSI_OR_CSI_PHY_CSR.h`,
//! `PERI_MIPI_DSI.h`; bases `MIMX9596_ca55_COMMON.h:810-830`. The PHY's
//! PLL and test-code sequence: hal_nxp `drivers/mipi_dsi_imx/fsl_mipi_dsi.c`
//! (`DSI_ConfigDphy` :1243-1317, `tx_dphy_write_control` :1212-1235, the
//! lookup tables :1064-1204, `DSI_DphyGetPllDivider` :522-563,
//! `HsLpTimingTable` :33-49), as upstream Zephyr's `dsi_nxp_dwc.c`
//! (Apache-2.0, af17c0c8, :108-129) calls it for the i.MX 95. Facts only
//! (GPL-2.0+, nothing copied), cited `imx95-mipi-dsi.c:N`: NXP's Linux
//! driver as Toradex's kernel carries it (linux-toradex 851ac0f9,
//! `drivers/gpu/drm/bridge/imx/imx95-mipi-dsi.c`), for the CSR writes the
//! hal path leaves out and the PLL constraints.
//!
//! The blocks ([`Bases`]): DSI stream CSR 0x4AD00000 (`DSI_HOST_CONFIGURATION`
//! +0x0: the pixel link format), DSI master CSR 0x4AD10000
//! (`DSI_CLOCK_GATING_CONTROL` +0x0, `DSI_PIXEL_LINK_CONTROL` +0x4: which
//! pixel link feeds the host, `DSI_CLOCK_SETTING` +0x8: the PHY PLL's
//! reference source), the PHY CSR 0x4AD20000 whose `COMBO_PHY_*` registers
//! at +0x100 are the combo PHY's (Linux's `mipi_tx_phy_csr` node at
//! 0x4AD20100, `imx95.dtsi:2221`), and the CSI-2 host 1 at 0x4AD40000
//! (`nxp,mipi-csi = <&mipi_csi1>`, `imx95.dtsi:2126`), whose PHY test
//! interface (+0x50/+0x54, `imx95-mipi-dsi.c:143-144`)
//! reaches the combo PHY's receive side.
//!
//! [`ComboPhy::init`], called by `dw_mipi_dsi::attach` after the host's PHY
//! registers and before `PHY_RSTZ` releases the PHY:
//!
//! 1. master `DSI_PIXEL_LINK_CONTROL` = the pixel link (0: Toradex's DSI
//!    overlay feeds the DSI from pixel interleaver channel 0, pixel link 0;
//!    Zephyr `dsi_nxp_dwc.c:112-113`; Linux's mux, `imx95-mipi-dsi.c:809`,
//!    `imx95.dtsi:2171`).
//! 2. master `DSI_CLOCK_SETTING` = 0: the PLL reference is the 24 MHz
//!    clock, not the pixel link clock (`imx95-mipi-dsi.c:823`; the field
//!    `Dphy_ref_clock_source` 0 = "24 MHz", PERI header).
//! 3. stream `DSI_HOST_CONFIGURATION` = 0: RGB 24-bit on the pixel link
//!    (`imx95-mipi-dsi.c:825-827`, the field's 0b000).
//! 4. if [`Tuning::rx_term_bypass`]: CSI-2 host 1 test code 0x302
//!    (`DIG_RDWR_RX_CLKLANE_LANE_1`) := 0x02, `HSRXENTERM_BYPASS_CLKLANE`,
//!    read back equal (`imx95-mipi-dsi.c:840-845`; the receive side's
//!    clock-lane termination, shared pins). hal's path does not do it.
//! 5. combo `COMBO_PHY_MODE_CONTROL` = `PHY_enable_ext` (the lanes' mask, if
//!    [`Tuning::lane_enables`]) | `PLL_clksel` 01 (clock generation) |
//!    `Tx_Rxz` 1 (DSI): 0x03, or 0xF3 for four
//!    lanes (`imx95-mipi-dsi.c:848-851`; hal writes 0x3, Zephyr :117 --
//!    `PHY_enable_ext` acts only with `phy_enable_byp`, which stays 0).
//! 6. combo `COMBO_PHY_FREQ_CONTROL` = `Phy_hsfreqrange` | `Phy_cfgclkfreqrange`
//!    (Zephyr :109-111, `imx95-mipi-dsi.c:706-708`).
//! 7. the PHY's test codes, in hal's `DSI_ConfigDphy` order, through the
//!    host's test interface ([`TestCode`], [`Pll::test_codes`]).
//! 8. if [`Tuning::turndisable`]: combo `COMBO_PHY_TEST_MODE_CONTROL` =
//!    `turndisable_0`
//!    (`imx95-mipi-dsi.c:859`).
//! 9. master `DSI_CLOCK_GATING_CONTROL`: the unused display's asynchronous
//!    FIFO gated, the PHY PLL clocks running (`imx95-mipi-dsi.c:861-862`).
//!
//! The PLL lock is reported in the host's `PHY_STATUS` (hal `DSI_PowerUp`
//! waits for it there), so the host waits for it as on the STM32MP25.
//!
//! Where hal_nxp and NXP's Linux driver disagree (the design note's
//! U13-U16, `driver/docs/imx95-display.md` 4(b)), the value is a named
//! field of [`Tuning`], hal's by default ([`Tuning::HAL`]: the permissive
//! sequence this module is written from, which Zephyr runs on the i.MX 95
//! M7); [`Tuning::LINUX_420`] holds the values NXP's Linux driver writes for
//! 420 Mbit/s, each a single value read from it (its tables are not copied):
//!
//! | what | hal ([`Tuning::HAL`]) | Linux at 420 Mbit/s | U |
//! |---|---|---|---|
//! | `hsfreqrange` | nearest band centre ([`hsfreqrange`]): 0x05 | 0x16 (`imx95-mipi-dsi.c:262-263`) | U13 |
//! | `vco_cntrl` where two rows overlap | the faster row's ([`vco_cntrl`]): 0x19 | 0x1F, the first row that fits (`:226-238`, `:601-631`) | U14 |
//! | `sr_osc_freq_target` | by PLL output ([`sr_osc_freq_target`]): 0x384 | 0x7D0 always (`:732-733`) | U15 |
//! | `PLL_17` power-on override | not set (0x0D) | `pll_pwron_ovr` and `_en` set (0xCD, `:791-793`) | U15 |
//! | `COMBO_PHY_MODE_CONTROL` lane enables | none (0x03, Zephyr :117) | lanes 0-3 (0xF3, `:848-851`) | U16 |
//! | `COMBO_PHY_TEST_MODE_CONTROL.turndisable_0` | not written | 0x2000 (`:859`) | U16 |
//! | the CSI-1 clock-lane termination bypass | not written | written and read back (`:840-846`) | U16 |
//!
//! Both sources agree on M = 70, N = 4 for 420 Mbit/s from 24 MHz. Linux
//! writes the PLL bytes read-modify-write; this module writes whole bytes,
//! as hal does.

use crate::env::Env;
use crate::errno::{EINVAL, EIO};
use crate::ip::dw_mipi_dsi::{regs as dsi, DphyTiming, Link, Phy};
use crate::mmio::Mmio;
use crate::ylog;

/// Block bases (physical; `MIMX9596_ca55_COMMON.h:805-830`, imx95.dtsi).
pub const DSI_BASE: usize = 0x4acf_0000;
pub const STREAM_CSR_BASE: usize = 0x4ad0_0000;
pub const MASTER_CSR_BASE: usize = 0x4ad1_0000;
pub const PHY_CSR_BASE: usize = 0x4ad2_0000;
pub const CSI1_BASE: usize = 0x4ad4_0000;

/// The registers this module writes: offsets and fields from the generated
/// [`crate::soc::imx95::regs`] (hal_nxp's `PERI_CAMERA_DSI_*` headers),
/// composed into the values written; the CSI-2 host's test interface is not
/// in a hal_nxp header of this SoC and is stated here.
pub mod regs {
    use crate::mmio::field;
    use crate::soc::imx95::regs::{camera_dsi_master_csr as mst, camera_dsi_or_csi_phy_csr as phy};

    pub use crate::soc::imx95::regs::camera_dsi_master_csr::{
        DSI_CLOCK_GATING_CONTROL, DSI_CLOCK_SETTING, DSI_PIXEL_LINK_CONTROL,
    };
    pub use crate::soc::imx95::regs::camera_dsi_or_csi_phy_csr::{
        COMBO_PHY_FREQ_CONTROL, COMBO_PHY_MODE_CONTROL, COMBO_PHY_TEST_MODE_CONTROL, COMBO_PHY_TEST_MODE_STATUS,
    };
    pub use crate::soc::imx95::regs::camera_dsi_stream_csr::DSI_HOST_CONFIGURATION;

    /// `Pixel_link_format` 0b000: RGB 24-bit.
    pub const PIXEL_LINK_FORMAT_RGB24: u32 = 0;

    /// `Display_n_async_fifo`: 1 gates display n's pixel clock.
    pub const fn display_async_fifo(n: u32) -> u32 {
        if n & 1 == 0 {
            mst::dsi_clock_gating_control::DISPLAY_0_ASYNC_FIFO_MASK
        } else {
            mst::dsi_clock_gating_control::DISPLAY_1_ASYNC_FIFO_MASK
        }
    }
    pub const DPHY_PLL_CLKIN: u32 = mst::dsi_clock_gating_control::DPHY_PLL_CLKIN_MASK;
    pub const DPHY_PLL_CLKOUT: u32 = mst::dsi_clock_gating_control::DPHY_PLL_CLKOUT_MASK;
    pub const DPHY_PLL_CLKEXT: u32 = mst::dsi_clock_gating_control::DPHY_PLL_CLKEXT_MASK;

    /// `Tx_Rxz` 1: DSI mode; `PLL_clksel` 01: clock generation.
    pub const TX_RXZ_DSI: u32 = field(phy::combo_phy_mode_control::TX_RXZ_MASK, 1);
    pub const PLL_CLKSEL_GEN: u32 = field(phy::combo_phy_mode_control::PLL_CLKSEL_MASK, 0b01);
    pub const fn phy_enable_ext(lanes_mask: u32) -> u32 {
        field(phy::combo_phy_mode_control::PHY_ENABLE_EXT_MASK, lanes_mask)
    }
    pub const fn freq_control(hsfreqrange: u32, cfgclkfreqrange: u32) -> u32 {
        field(phy::combo_phy_freq_control::PHY_HSFREQRANGE_MASK, hsfreqrange)
            | field(phy::combo_phy_freq_control::PHY_CFGCLKFREQRANGE_MASK, cfgclkfreqrange)
    }
    pub const TURNDISABLE_0: u32 = phy::combo_phy_test_mode_control::TURNDISABLE_0_MASK;

    /// CSI-2 host: the PHY test interface (`imx95-mipi-dsi.c:143-144`).
    pub const CSI_PHY_TEST_CTRL0: u32 = 0x50;
    pub const CSI_PHY_TEST_CTRL1: u32 = 0x54;
}

/// The PHY's test codes this module writes (hal `fsl_mipi_dsi.h:26-46`,
/// the names `TX_DPHY_*`).
pub mod code {
    pub const TX_PLL_1: u16 = 0x15e;
    pub const TX_PLL_5: u16 = 0x162;
    pub const TX_PLL_9: u16 = 0x166;
    pub const TX_PLL_13: u16 = 0x16a;
    pub const TX_PLL_17: u16 = 0x16e;
    pub const TX_PLL_22: u16 = 0x173;
    pub const TX_PLL_23: u16 = 0x174;
    pub const TX_PLL_24: u16 = 0x175;
    pub const TX_PLL_25: u16 = 0x176;
    pub const TX_PLL_27: u16 = 0x178;
    pub const TX_PLL_28: u16 = 0x179;
    pub const TX_PLL_29: u16 = 0x17a;
    pub const TX_PLL_30: u16 = 0x17b;
    pub const TX_PLL_31: u16 = 0x17c;
    pub const TX_CB_0: u16 = 0x1aa;
    pub const TX_CB_1: u16 = 0x1ab;
    pub const TX_CB_2: u16 = 0x1ac;
    pub const TX_SLEW_5: u16 = 0x270;
    pub const TX_SLEW_6: u16 = 0x271;
    pub const TX_SLEW_7: u16 = 0x272;
    pub const TX_CLK_TERMLOWCAP: u16 = 0x402;
    /// The receive side's clock lane (`imx95-mipi-dsi.c:147-148`).
    pub const RX_CLKLANE_LANE_1: u16 = 0x302;
    pub const HSRXENTERM_BYPASS_CLKLANE: u8 = 1 << 1;
}

/// The blocks' addresses as the caller mapped them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub dsi: usize,
    pub stream: usize,
    pub master: usize,
    pub phy_csr: usize,
    pub csi1: usize,
}

impl Bases {
    /// The physical addresses (an identity-mapped image).
    pub const PHYS: Bases = Bases {
        dsi: DSI_BASE,
        stream: STREAM_CSR_BASE,
        master: MASTER_CSR_BASE,
        phy_csr: PHY_CSR_BASE,
        csi1: CSI1_BASE,
    };
}

// ---- the test interface -----------------------------------------------------

/// One write of the PHY test interface at `ctrl0`/`ctrl1`
/// (`PHY_TST_CTRL0`: TESTCLR 0, TESTCLK 1; `PHY_TST_CTRL1`: TESTDIN 7:0,
/// TESTDOUT 15:8, TESTEN 16): hal's `tx_dphy_write_control`
/// (fsl_mipi_dsi.c:1212-1235), the code's high byte through the extended
/// address code 0x00, its low byte, then the data on a rising TESTCLK with
/// TESTEN low.
pub fn test_write(m: &impl Mmio, ctrl0: usize, ctrl1: usize, test_code: u16, data: u8) {
    write_code(m, ctrl0, ctrl1, test_code);
    m.write32(ctrl1, u32::from(data));
    m.write32(ctrl0, dsi::PHY_TESTCLK);
}

/// The test-code half of [`test_write`].
fn write_code(m: &impl Mmio, ctrl0: usize, ctrl1: usize, test_code: u16) {
    let (clk, en) = (dsi::PHY_TESTCLK, dsi::PHY_TESTEN);
    // the extended address code 0x00, latched on TESTCLK falling, TESTEN high
    m.write32(ctrl0, 0);
    m.write32(ctrl1, 0);
    m.write32(ctrl1, en);
    m.write32(ctrl0, clk);
    m.write32(ctrl1, en);
    m.write32(ctrl0, 0);
    // its data, the code's high byte, on TESTCLK rising, TESTEN low
    m.write32(ctrl1, 0);
    m.write32(ctrl1, u32::from(test_code >> 8));
    m.write32(ctrl0, clk);
    // the code's low byte, latched on TESTCLK falling, TESTEN high
    m.write32(ctrl0, 0);
    m.write32(ctrl1, en);
    m.write32(ctrl0, clk);
    m.write32(ctrl1, en | u32::from(test_code & 0xff));
    m.write32(ctrl0, 0);
    m.write32(ctrl1, 0);
}

/// A read of the test interface: the code written as for a write, then
/// TESTDOUT (`PHY_TST_CTRL1` 15:8) read (`imx95-mipi-dsi.c:500-511`: the
/// protocol; the write half is hal's).
pub fn test_read(m: &impl Mmio, ctrl0: usize, ctrl1: usize, test_code: u16) -> u8 {
    write_code(m, ctrl0, ctrl1, test_code);
    (m.read32(ctrl1) >> dsi::PHY_TESTDOUT_SHIFT) as u8
}

/// One test-code write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TestCode {
    pub code: u16,
    pub data: u8,
}

// ---- the PLL ----------------------------------------------------------------

/// Lane rates the PHY runs (`imx95-mipi-dsi.c:163-164`: 80 to 2500 Mbit/s).
pub const LANE_MIN_BPS: u64 = 80_000_000;
pub const LANE_MAX_BPS: u64 = 2_500_000_000;
/// The PLL's M (64-625) and N (1-16) (hal :522-535, M = m + 2, N = n + 1,
/// m 62-623, n 0-15; `imx95-mipi-dsi.c:166-170`).
pub const M_MIN: u32 = 64;
pub const M_MAX: u32 = 625;
pub const N_MIN: u32 = 1;
pub const N_MAX: u32 = 16;
/// The PLL's input after N: 2 to 8 MHz (`imx95-mipi-dsi.c:593-595`; hal
/// does not check it).
pub const FIN_N_MIN_HZ: u64 = 2_000_000;
pub const FIN_N_MAX_HZ: u64 = 8_000_000;

/// `Pll_Set_Pll_Vco_Param` (hal :1184-1210): `pll_vco_cntrl` by the PLL
/// output in MHz, the first row the output is at or above. Bits 5:4 are the
/// output divider (00 /1 .. 11 /8, `imx95-mipi-dsi.c:555-564`).
pub fn vco_cntrl(fout_mhz: u32) -> u8 {
    const ROWS: [(u32, u8); 11] = [
        (1150, 1),
        (1100, 1),
        (630, 3),
        (420, 9),
        (320, 15),
        (210, 25),
        (160, 31),
        (105, 41),
        (80, 47),
        (53, 57),
        (40, 63),
    ];
    ROWS.iter().find(|r| fout_mhz >= r.0).map_or(63, |r| r.1)
}

/// The VCO's divide-down to the output: 1, 2, 4 or 8 from `vco_cntrl`'s
/// bits 5:4.
pub const fn vco_div(vco_cntrl: u8) -> u32 {
    1 << ((vco_cntrl >> 4) & 3)
}

/// `Pll_Set_Pll_Prop_Param` (hal :1109-1125): 14 from 1150 to 1250 MHz, 13
/// below.
pub fn prop_cntrl(fout_mhz: u32) -> u8 {
    if (1150..=1250).contains(&fout_mhz) {
        14
    } else {
        13
    }
}

/// `Pll_Set_Sr_Osc_Freq_Target` (hal :1127-1151).
pub fn sr_osc_freq_target(fout_mhz: u32) -> u16 {
    if fout_mhz > 1000 {
        0x7d0
    } else if fout_mhz > 500 {
        0x4e2
    } else {
        0x384
    }
}

/// `Pll_Set_Hs_Freqrange` (hal :1064-1107): the band whose upper bound (in
/// kbit/s, hal's MHz x 1000) the lane rate is at or below.
pub fn hsfreqrange(lane_bps: u64) -> u8 {
    const ROWS: [(u32, u8); 63] = [
        (82_500, 0),
        (92_500, 16),
        (102_500, 32),
        (112_500, 48),
        (122_500, 1),
        (132_500, 17),
        (142_500, 33),
        (152_500, 49),
        (162_500, 2),
        (172_500, 18),
        (182_500, 34),
        (192_500, 50),
        (207_500, 3),
        (222_500, 19),
        (237_500, 35),
        (262_500, 51),
        (287_500, 4),
        (312_500, 20),
        (337_500, 37),
        (375_000, 53),
        (425_000, 5),
        (475_000, 22),
        (525_000, 38),
        (575_000, 55),
        (625_000, 7),
        (675_000, 24),
        (725_000, 40),
        (775_000, 57),
        (825_000, 9),
        (875_000, 25),
        (925_000, 41),
        (975_000, 58),
        (1_025_000, 10),
        (1_075_000, 26),
        (1_125_000, 42),
        (1_175_000, 59),
        (1_225_000, 11),
        (1_275_000, 27),
        (1_325_000, 43),
        (1_375_000, 60),
        (1_425_000, 12),
        (1_475_000, 28),
        (1_525_000, 44),
        (1_575_000, 61),
        (1_625_000, 13),
        (1_675_000, 29),
        (1_725_000, 46),
        (1_775_000, 62),
        (1_825_000, 14),
        (1_875_000, 30),
        (1_925_000, 47),
        (1_975_000, 63),
        (2_025_000, 15),
        (2_075_000, 64),
        (2_125_000, 65),
        (2_175_000, 66),
        (2_225_000, 67),
        (2_275_000, 68),
        (2_325_000, 69),
        (2_375_000, 70),
        (2_425_000, 71),
        (2_475_000, 72),
        (2_525_000, 73),
    ];
    // hal compares in MHz as a double; kbit/s keeps the .5 MHz bounds exact
    let kbps = lane_bps.div_ceil(1000);
    ROWS.iter().find(|r| kbps <= u64::from(r.0)).map_or(0, |r| r.1)
}

/// `cfgclkfreqrange` from the PHY's cfg clock: (MHz - 17) x 4
/// (`imx95-mipi-dsi.c:665-672`, "DPHY Databook Table 4-5"); 24 MHz gives
/// 0x1C, the value Zephyr writes (`dsi_nxp_dwc.c:111`).
pub fn cfgclkfreqrange(cfg_hz: u64) -> Result<u8, i32> {
    let mhz = cfg_hz / 1_000_000;
    if !(17..=80).contains(&mhz) {
        return Err(-EINVAL);
    }
    Ok(((mhz - 17) * 4) as u8)
}

/// `HsLpTimingTable` (hal :33-49; the same rows as `imx95-mipi-dsi.c:1137`'s
/// Table A-4): lane Mbit/s, clock LP->HS, clock HS->LP, data LP->HS, data
/// HS->LP, in lane byte clocks.
const HS_LP_TIMING: [[u16; 5]; 63] = [
    [80, 21, 17, 15, 10],
    [90, 23, 17, 16, 10],
    [100, 22, 17, 16, 10],
    [110, 25, 18, 17, 11],
    [120, 26, 20, 18, 11],
    [130, 27, 19, 19, 11],
    [140, 27, 19, 19, 11],
    [150, 28, 20, 20, 12],
    [160, 30, 21, 22, 13],
    [170, 30, 21, 23, 13],
    [180, 31, 21, 23, 13],
    [190, 32, 22, 24, 13],
    [205, 35, 22, 25, 13],
    [220, 37, 26, 27, 15],
    [235, 38, 28, 27, 16],
    [250, 41, 29, 30, 17],
    [275, 43, 29, 32, 18],
    [300, 45, 32, 35, 19],
    [325, 48, 33, 36, 18],
    [350, 51, 35, 40, 20],
    [400, 59, 37, 44, 21],
    [450, 65, 40, 49, 23],
    [500, 71, 41, 54, 24],
    [550, 77, 44, 57, 26],
    [600, 82, 46, 64, 27],
    [650, 87, 48, 67, 28],
    [700, 94, 52, 71, 29],
    [750, 99, 52, 75, 31],
    [800, 105, 55, 82, 32],
    [850, 110, 58, 85, 32],
    [900, 115, 58, 88, 35],
    [950, 120, 62, 93, 36],
    [1000, 128, 63, 99, 38],
    [1050, 132, 65, 102, 38],
    [1100, 138, 67, 106, 39],
    [1150, 146, 69, 112, 42],
    [1200, 151, 71, 117, 43],
    [1250, 153, 74, 120, 45],
    [1300, 160, 73, 124, 46],
    [1350, 165, 76, 130, 47],
    [1400, 172, 78, 134, 49],
    [1450, 177, 80, 138, 49],
    [1500, 183, 81, 143, 52],
    [1550, 191, 84, 147, 52],
    [1600, 194, 85, 152, 52],
    [1650, 201, 86, 155, 53],
    [1700, 208, 88, 161, 53],
    [1750, 212, 89, 165, 53],
    [1800, 220, 90, 171, 54],
    [1850, 223, 92, 175, 54],
    [1900, 231, 91, 180, 55],
    [1950, 236, 95, 185, 56],
    [2000, 243, 97, 190, 56],
    [2050, 248, 99, 194, 58],
    [2100, 252, 100, 199, 59],
    [2150, 259, 102, 204, 61],
    [2200, 266, 105, 210, 62],
    [2250, 269, 109, 213, 63],
    [2300, 272, 109, 217, 65],
    [2350, 281, 112, 225, 66],
    [2400, 283, 115, 226, 66],
    [2450, 282, 115, 226, 67],
    [2500, 281, 118, 227, 67],
];

/// `DSI_GetDefaultDphyConfig`'s lookup (hal :454-470): the first row at or
/// above the lane rate, else the last.
pub fn timing(lane_mbps: u32) -> DphyTiming {
    let row = HS_LP_TIMING.iter().find(|r| lane_mbps <= u32::from(r[0])).unwrap_or(&HS_LP_TIMING[62]);
    DphyTiming {
        clk_lp2hs: u32::from(row[1]),
        clk_hs2lp: u32::from(row[2]),
        data_lp2hs: u32::from(row[3]),
        data_hs2lp: u32::from(row[4]),
    }
}

/// The values hal_nxp and NXP's Linux driver disagree on (module text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// `Phy_hsfreqrange`; `None` is hal's table ([`hsfreqrange`]).
    pub hsfreqrange: Option<u8>,
    /// `pll_vco_cntrl`; `None` is hal's table ([`vco_cntrl`]). Its bits 5:4
    /// set the VCO's divider either way.
    pub vco_cntrl: Option<u8>,
    /// `sr_osc_freq_target`; `None` is hal's ([`sr_osc_freq_target`]).
    pub sr_osc_freq_target: Option<u16>,
    /// `PLL_17` bits 7:6, `pll_pwron_ovr_rw` and `pll_pwron_ovr_en_rw`.
    pub pll_pwron_override: bool,
    /// `COMBO_PHY_MODE_CONTROL.PHY_enable_ext` with the lanes' mask.
    pub lane_enables: bool,
    /// `COMBO_PHY_TEST_MODE_CONTROL.turndisable_0` after the PLL.
    pub turndisable: bool,
    /// The CSI-1 clock-lane termination bypass, written and read back.
    pub rx_term_bypass: bool,
}

impl Tuning {
    /// hal_nxp's values (the default).
    pub const HAL: Tuning = Tuning {
        hsfreqrange: None,
        vco_cntrl: None,
        sr_osc_freq_target: None,
        pll_pwron_override: false,
        lane_enables: false,
        turndisable: false,
        rx_term_bypass: false,
    };
    /// NXP's Linux driver's values at 420 Mbit/s (module text; the
    /// hsfreqrange and vco_cntrl values hold for that rate only).
    pub const LINUX_420: Tuning = Tuning {
        hsfreqrange: Some(0x16),
        vco_cntrl: Some(0x1f),
        sr_osc_freq_target: Some(0x7d0),
        pll_pwron_override: true,
        lane_enables: true,
        turndisable: true,
        rx_term_bypass: true,
    };
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning::HAL
    }
}

/// `PLL_17`'s power-on override bits.
pub const PLL_PWRON_OVR: u8 = 0xc0;

/// The PLL for a lane rate, and every test code derived from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pll {
    pub ref_hz: u64,
    /// The lane rate asked for.
    pub lane_bps: u64,
    /// The PLL output asked for: half the lane rate (the clock lane is DDR,
    /// hal :1255, `imx95-mipi-dsi.c:549-553`).
    pub fout_hz: u64,
    /// The VCO: `fout` x [`vco_div`] (hal `Pll_Set_Pll_Vco_Freq` :1153-1182).
    pub vco_hz: u64,
    /// Feedback multiplier M and input divider N: VCO = ref x M / N.
    pub m: u32,
    pub n: u32,
    /// What the PLL makes: ref x M / (N x div), and the lane rate it gives.
    pub fout_actual_hz: u64,
    pub vco_cntrl: u8,
    pub prop_cntrl: u8,
    pub sr_osc_freq_target: u16,
    pub hsfreqrange: u8,
    /// `PLL_17` as written: `prop_cntrl`, plus [`PLL_PWRON_OVR`] if tuned so.
    pub pll_17: u8,
}

/// The PLL for `lane_bps` from `ref_hz`: the output's controls by hal's
/// tables unless `tuning` names them, then M and N for the VCO with the
/// smallest error, the smallest N on a tie (hal's search order, :541-560),
/// inside M 64-625, N 1-16 and ref / N 2-8 MHz.
pub fn pll(ref_hz: u64, lane_bps: u64, tuning: &Tuning) -> Result<Pll, i32> {
    if !(LANE_MIN_BPS..=LANE_MAX_BPS).contains(&lane_bps) || ref_hz == 0 {
        return Err(-EINVAL);
    }
    let fout_hz = lane_bps / 2;
    let fout_mhz = (fout_hz / 1_000_000) as u32;
    let vco_cntrl = tuning.vco_cntrl.unwrap_or_else(|| vco_cntrl(fout_mhz));
    let div = vco_div(vco_cntrl);
    let vco_hz = fout_hz * u64::from(div);
    let mut best: Option<(u64, u32, u32)> = None; // (VCO error in Hz, M, N)
    for n in N_MIN..=N_MAX {
        // 2 MHz <= ref / N <= 8 MHz, without rounding the quotient
        if !(FIN_N_MIN_HZ * u64::from(n)..=FIN_N_MAX_HZ * u64::from(n)).contains(&ref_hz) {
            continue;
        }
        let m = ((vco_hz * u64::from(n) + ref_hz / 2) / ref_hz) as u32;
        if !(M_MIN..=M_MAX).contains(&m) {
            continue;
        }
        let made = ref_hz * u64::from(m) / u64::from(n);
        let err = made.abs_diff(vco_hz);
        if best.is_none_or(|b| err < b.0) {
            best = Some((err, m, n));
        }
    }
    let (_, m, n) = best.ok_or(-EINVAL)?;
    let fout_actual_hz = ref_hz * u64::from(m) / (u64::from(n) * u64::from(div));
    let prop = prop_cntrl(fout_mhz);
    Ok(Pll {
        ref_hz,
        lane_bps,
        fout_hz,
        vco_hz,
        m,
        n,
        fout_actual_hz,
        vco_cntrl,
        prop_cntrl: prop,
        sr_osc_freq_target: tuning.sr_osc_freq_target.unwrap_or_else(|| sr_osc_freq_target(fout_mhz)),
        hsfreqrange: tuning.hsfreqrange.unwrap_or_else(|| hsfreqrange(lane_bps)),
        pll_17: prop | if tuning.pll_pwron_override { PLL_PWRON_OVR } else { 0 },
    })
}

/// How many test codes [`Pll::test_codes`] writes at most.
pub const TEST_CODES: usize = 22;

impl Pll {
    /// The test codes of hal's `DSI_ConfigDphy` (:1263-1315), in its order;
    /// `CB_2` only below 450 Mbit/s ("clkdiv_clk_en must be enabled").
    /// Returns the codes and their count.
    pub fn test_codes(&self) -> ([TestCode; TEST_CODES], usize) {
        use code::*;
        let t = |code, data| TestCode { code, data };
        let mreg = self.m - 2;
        let nreg = self.n - 1;
        let mut v = [t(0, 0); TEST_CODES];
        let mut k = 0;
        let mut push = |c: TestCode| {
            v[k] = c;
            k += 1;
        };
        push(t(TX_PLL_13, 0x03)); // pll_mpll_prog_rw 11
        push(t(TX_CB_1, 0x06)); // cb_sel_vref_lprx_rw 10
        push(t(TX_CB_0, 0x53)); // cb_sel_vrefcd_lprx_rw 10
        if self.lane_bps < 450_000_000 {
            push(t(TX_CB_2, 0x10)); // cb_clkdiv_clk_en_rw
        }
        push(t(TX_CLK_TERMLOWCAP, 0x02)); // txclk_term_lowcap_lp00_en_ovr_en 1, _ovr 0
        push(t(TX_SLEW_7, 0x00)); // sr_range 0
        push(t(TX_SLEW_6, (self.sr_osc_freq_target >> 8) as u8));
        push(t(TX_SLEW_5, self.sr_osc_freq_target as u8));
        push(t(TX_SLEW_7, 0x10)); // slew rate calibration on
        push(t(TX_PLL_29, ((mreg >> 8) & 0x3) as u8)); // pll_m_ovr_rw 9:8
        push(t(TX_PLL_28, mreg as u8)); // pll_m_ovr_rw 7:0
        push(t(TX_PLL_27, 0x80 | ((nreg & 0xf) << 3) as u8)); // pll_n_ovr_en_rw, pll_n_ovr_rw
        push(t(TX_PLL_30, 0x80 | ((self.vco_cntrl & 0x3f) << 1) | 0x01)); // vco_cntrl override, m override
        push(t(TX_PLL_1, 0x10)); // pll_cpbias_cntrl_rw
        push(t(TX_PLL_5, 0x01)); // pll_gmp_cntrl_rw 01, pll_int_cntrl_rw 0
        push(t(TX_PLL_22, 0x02)); // pll_th1_rw 7:0
        push(t(TX_PLL_23, 0x00)); // pll_th1_rw 9:8
        push(t(TX_PLL_24, 0x60)); // pll_th2_rw
        push(t(TX_PLL_25, 0x03)); // pll_th3_rw
        push(t(TX_PLL_9, 0x04)); // pll_lock_sel_rw
        push(t(TX_PLL_17, self.pll_17)); // pll_prop_cntrl_rw (and the power-on override)
        push(t(TX_PLL_31, 0x03)); // pll_clkouten_right_rw, _left_rw
        (v, k)
    }
}

// ---- the Phy ----------------------------------------------------------------

/// The combo PHY as the DSI host's [`Phy`].
#[derive(Clone, Copy, Debug)]
pub struct ComboPhy {
    pub bases: Bases,
    /// MIPIPHYPLLREF and MIPIPHYCFG as the SM reported them.
    pub ref_hz: u64,
    pub cfg_hz: u64,
    /// The pixel link feeding the host (0 or 1).
    pub pixel_link: u32,
    /// Added to the pixel-rate lane rate, per mille (0: Linux's rate,
    /// `phy_mipi_dphy_get_default_config`: pixel clock x bpp / lanes).
    pub margin_permille: u32,
    pub tuning: Tuning,
    /// Set by [`Phy::lane_mbps`].
    pub pll: Option<Pll>,
    pub lanes: u32,
}

impl ComboPhy {
    pub fn new(bases: Bases, ref_hz: u64, cfg_hz: u64) -> ComboPhy {
        ComboPhy { bases, ref_hz, cfg_hz, pixel_link: 0, margin_permille: 0, tuning: Tuning::HAL, pll: None, lanes: 0 }
    }

    /// The lane rate the link needs.
    pub fn lane_bps(&self, link: &Link) -> u64 {
        let raw = u64::from(link.mode.pixel_clock_hz) * u64::from(link.format.bpp()) / u64::from(link.lanes.max(1));
        raw + raw * u64::from(self.margin_permille) / 1000
    }

    fn dsi_test_write(&self, m: &impl Mmio, c: TestCode) {
        let b = self.bases.dsi;
        test_write(m, b + dsi::PHY_TST_CTRL0 as usize, b + dsi::PHY_TST_CTRL1 as usize, c.code, c.data);
    }
}

impl Phy for ComboPhy {
    fn lane_mbps(&mut self, _m: &impl Mmio, link: &Link) -> Result<u32, i32> {
        let bps = self.lane_bps(link);
        let p = pll(self.ref_hz, bps, &self.tuning)?;
        self.pll = Some(p);
        self.lanes = link.lanes;
        // DIV_ROUND_UP(hs_clk_rate, USEC_PER_SEC), imx95-mipi-dsi.c:1110
        Ok(bps.div_ceil(1_000_000) as u32)
    }

    fn timing(&self, lane_mbps: u32) -> Result<DphyTiming, i32> {
        Ok(timing(lane_mbps))
    }

    fn init(&mut self, m: &impl Mmio, env: &impl Env) -> Result<(), i32> {
        let p = self.pll.ok_or(-EINVAL)?;
        let cfgrange = cfgclkfreqrange(self.cfg_hz)?;
        let b = self.bases;
        let wr = |base: usize, off: u32, v: u32| m.write32(base + off as usize, v);

        wr(b.master, regs::DSI_PIXEL_LINK_CONTROL, self.pixel_link & 1);
        wr(b.master, regs::DSI_CLOCK_SETTING, 0);
        wr(b.stream, regs::DSI_HOST_CONFIGURATION, regs::PIXEL_LINK_FORMAT_RGB24);

        if self.tuning.rx_term_bypass {
            let (c0, c1) = (b.csi1 + regs::CSI_PHY_TEST_CTRL0 as usize, b.csi1 + regs::CSI_PHY_TEST_CTRL1 as usize);
            test_write(m, c0, c1, code::RX_CLKLANE_LANE_1, code::HSRXENTERM_BYPASS_CLKLANE);
            let got = test_read(m, c0, c1, code::RX_CLKLANE_LANE_1);
            if got != code::HSRXENTERM_BYPASS_CLKLANE {
                ylog!(env, Err, "combo PHY: RX clock lane termination bypass reads {:#04x}", got);
                return Err(-EIO);
            }
        }

        let lanes_mask = if self.tuning.lane_enables { (1u32 << self.lanes.clamp(1, 4)) - 1 } else { 0 };
        wr(
            b.phy_csr,
            regs::COMBO_PHY_MODE_CONTROL,
            regs::phy_enable_ext(lanes_mask) | regs::PLL_CLKSEL_GEN | regs::TX_RXZ_DSI,
        );
        wr(b.phy_csr, regs::COMBO_PHY_FREQ_CONTROL, regs::freq_control(p.hsfreqrange.into(), cfgrange.into()));

        let (codes, n) = p.test_codes();
        for &c in &codes[..n] {
            self.dsi_test_write(m, c);
        }

        if self.tuning.turndisable {
            wr(b.phy_csr, regs::COMBO_PHY_TEST_MODE_CONTROL, regs::TURNDISABLE_0);
        }
        wr(b.master, regs::DSI_CLOCK_GATING_CONTROL, regs::display_async_fifo(1 - (self.pixel_link & 1)));
        ylog!(
            env,
            Inf,
            "combo PHY: {} kbit/s a lane, ref {} kHz x {} / {} / {} = {} kHz, vco_cntrl {:#04x}, hsfreqrange {:#04x}",
            p.lane_bps / 1000,
            p.ref_hz / 1000,
            p.m,
            p.n,
            vco_div(p.vco_cntrl),
            p.fout_actual_hz / 1000,
            p.vco_cntrl,
            p.hsfreqrange
        );
        Ok(())
    }

    fn power_off(&mut self, m: &impl Mmio) {
        let b = self.bases;
        // imx95-mipi-dsi.c:873-877: every gate set, the PHY back to DSI mode
        // with its PLL clocks stopped
        m.write32(
            b.master + regs::DSI_CLOCK_GATING_CONTROL as usize,
            regs::DPHY_PLL_CLKEXT
                | regs::DPHY_PLL_CLKOUT
                | regs::DPHY_PLL_CLKIN
                | regs::display_async_fifo(0)
                | regs::display_async_fifo(1),
        );
        m.write32(b.phy_csr + regs::COMBO_PHY_MODE_CONTROL as usize, regs::TX_RXZ_DSI);
    }
}
