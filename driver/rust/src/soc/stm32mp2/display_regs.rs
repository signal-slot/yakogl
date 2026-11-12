// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F-DK display path's SoC-level numbers: base addresses, RCC
//! enable/reset registers and bits, RIFSC and RIMU ids, SCMI ids. Constants
//! only; nothing here enables anything (plan #353, item soc-display-regs).
//!
//! Source tags; docs/notes/stm32mp257f-dk-display-bringup.md (the "note")
//! holds the full citations:
//! - `L:` ST's Linux (`stm32mp251.dtsi`, `stm32mp255.dtsi`,
//!   `stm32mp257f-dk.dts`, `drivers/clk/stm32/clk-stm32mp25*.c`,
//!   `include/dt-bindings/{clock,reset}/st,stm32mp25-rcc.h`).
//! - `O:` ST's OP-TEE (`core/include/dt-bindings/clock/stm32mp25-clks.h`,
//!   `core/include/dt-bindings/soc/stm32mp25-rifsc.h`,
//!   `core/drivers/firewall/stm32_rifsc.c`, the DK's `-rif.dtsi`).
//! - `U:` ST's U-Boot, `T:` ST's TF-A.
//!
//! RM0457 itself was not read: no value here is confirmed against it.
//! The gate (bit 1) and reset (bit 0) bits of every `RCC_xxxCFGR` below are
//! those of `rcc::CLOCK_EN` and `rcc::RESET`, referenced, not repeated.

pub use super::rcc::{CLOCK_EN as CFGR_GATE, RESET as CFGR_RESET};

// ---- Base addresses ---------------------------------------------------------

/// LTDC, 0x400 bytes (L:`stm32mp251.dtsi:2150-2177`).
pub const LTDC_BASE: usize = 0x4801_0000;
/// DSI host, 0x800 bytes (L:`stm32mp255.dtsi:123-133`).
pub const DSI_BASE: usize = 0x4800_0000;
/// I2C2, which carries the ADV7535 at 0x3d (L:`stm32mp251.dtsi:935-951`,
/// `stm32mp257f-dk.dts:428-460`).
pub const I2C2_BASE: usize = 0x4013_0000;
/// RCC (note: `RCC_LTDCCFGR` is 0x44200840).
pub const RCC_BASE: usize = 0x4420_0000;
/// SYSCFG (note: `SYSCFG_DISPLAYCLKCR` is 0x44235000 = base + 0x5000).
pub const SYSCFG_BASE: usize = 0x4423_0000;
/// RIFSC (note row 1; L:`stm32mp251.dtsi:434`).
pub const RIFSC_BASE: usize = 0x4208_0000;
/// GPIOB, ADV7535 reset PB6 and interrupt PB4 (L:`stm32mp251.dtsi:2771-2781`).
pub const GPIOB_BASE: usize = 0x4425_0000;
/// GPIOF, I2C2 SDA PF0 and SCL PF2 (L:`stm32mp251.dtsi:2819-2829`).
pub const GPIOF_BASE: usize = 0x4429_0000;

// ---- RCC registers (offsets from RCC_BASE) ----------------------------------

/// `RCC_I2C2CFGR` (L:`clk-stm32mp25.c:514`, `:2296`; offset
/// L:`stm32mp25_rcc.h:405`).
pub const RCC_I2C2CFGR: usize = 0x7A8;
/// `RCC_LTDCCFGR`: bus and kernel clock gate, reset (L:`clk-stm32mp25.c:550`,
/// `:897-907`, `:2329`).
pub const RCC_LTDCCFGR: usize = 0x840;
/// `RCC_DSICFGR`: gate, reset, and the two muxes below (L:`clk-stm32mp25.c:233-234`,
/// `:491`, `:2330`).
pub const RCC_DSICFGR: usize = 0x844;
/// `RCC_DSICFGR.DSIBLSEL`, bit 12: lane byte clock source; OP-TEE boot sets the
/// DSI PHY (O:`stm32mp257f-dk-ca35tdcid-rcc.dtsi:105-106`). A write keeps it.
pub const DSICFGR_DSIBLSEL: u32 = 1 << 12;
/// `RCC_DSICFGR.DSIPHYCKREFSEL`, bit 15: PHY reference, set = FLEXGEN_28
/// (same citations). A write keeps it.
pub const DSICFGR_DSIPHYCKREFSEL: u32 = 1 << 15;

// ---- SYSCFG -----------------------------------------------------------------

/// `SYSCFG_DISPLAYCLKCR` offset (L:`clk-stm32mp25-syscfg.c:13-17`, `:56-63`).
pub const SYSCFG_DISPLAYCLKCR: usize = 0x5000;
/// `DISPLAYCLKCR` bits 1:0.
pub const DISPLAYCLKCR_MASK: u32 = 0x3;
/// 0: `txbyteclk`, the DSI PHY's byte clock (same citation).
pub const DISPLAYCLKCR_TXBYTECLK: u32 = 0;
/// 1: `clk_pix_lvds` (same citation).
pub const DISPLAYCLKCR_LVDS: u32 = 1;
/// 2: `ck_ker_ltdc`, chosen by Linux and U-Boot for a DSI sink
/// (L:`ltdc.c:1122-1130`; U:`stm32_ltdc.c:887`).
pub const DISPLAYCLKCR_CK_KER_LTDC: u32 = 2;

// ---- Linux clock and reset ids (st,stm32mp25-rcc.h) -------------------------

/// `CK_BUS_LTDC` (L:`stm32mp251.dtsi:2150-2177`).
pub const LINUX_CK_BUS_LTDC: u32 = 206;
/// `CK_KER_LTDC` (same).
pub const LINUX_CK_KER_LTDC: u32 = 339;
/// `CK_BUS_DSI` (L:`stm32mp255.dtsi:123-133`).
pub const LINUX_CK_BUS_DSI: u32 = 205;
/// `CK_KER_DSIPHY` (same).
pub const LINUX_CK_KER_DSIPHY: u32 = 289;
/// `CK_KER_I2C2` (L:`stm32mp251.dtsi:935-951`).
pub const LINUX_CK_KER_I2C2: u32 = 263;
/// `LTDC_R` (L:`include/dt-bindings/reset/st,stm32mp25-rcc.h`).
pub const LINUX_RST_LTDC: u32 = 76;
/// `DSI_R` (same).
pub const LINUX_RST_DSI: u32 = 77;
/// `I2C2_R` (same).
pub const LINUX_RST_I2C2: u32 = 43;

// ---- RIFSC peripheral ids (O:stm32mp25-rifsc.h) -----------------------------

/// `RIFSC_RISC_SECCFGRx` offset for a peripheral id: 0x10 + 4 x (id / 32)
/// (O:`stm32_rifsc.c:25-28`).
pub const fn rifsc_seccfgr(id: u32) -> usize {
    0x10 + 4 * (id as usize / 32)
}
/// `PERx_CIDCFGR` offset for a peripheral id: 0x100 + 8 x id
/// (O:`stm32_rifsc.c:84`).
pub const fn rifsc_cidcfgr(id: u32) -> usize {
    0x100 + 8 * id as usize
}
/// I2C2 (O:`stm32mp25-rifsc.h:51`).
pub const RIFSC_I2C2: u32 = 42;
/// LTDC common registers, `cmn` (O:`stm32mp25-rifsc.h:85`).
pub const RIFSC_LTDC_CMN: u32 = 80;
/// LTDC layers 1 and 2, `l1l2` (O:`stm32mp25-rifsc.h:118`).
pub const RIFSC_LTDC_L1L2: u32 = 119;
/// LTDC layer 3, `l3` (O:`stm32mp25-rifsc.h:119`); the secure-layer areas at
/// 0xfe800000 go with it (pre-study :630, :645).
pub const RIFSC_LTDC_L3: u32 = 120;
/// LTDC rotation, `rot` (O:`stm32mp25-rifsc.h:120`).
pub const RIFSC_LTDC_ROT: u32 = 121;
/// DSI common registers (O:`stm32mp25-rifsc.h:86`).
pub const RIFSC_DSI_CMN: u32 = 81;
/// DSI `trig` (O:`stm32mp25-rifsc.h:121`).
pub const RIFSC_DSI_TRIG: u32 = 122;
/// DSI `rdfifo` (O:`stm32mp25-rifsc.h:122`).
pub const RIFSC_DSI_RDFIFO: u32 = 123;

// ---- RIMU (bus masters) -----------------------------------------------------

/// `RIFSC_RIMC_ATTRx` offset: 0xC10 + 4 x n (O:`stm32_rifsc.c:102-107`).
pub const fn rifsc_rimc_attr(n: u32) -> usize {
    0xC10 + 4 * n as usize
}
/// RIMUs without a peripheral pairing, the likely LTDC masters: 11, 12, 13
/// (O:`stm32_rifsc.c:191-202`; DK file `-rif.dtsi:636-638`). Unconfirmed: no
/// source names the LTDC's RIMU (note, open question 1).
pub const RIMU_LTDC_CANDIDATES: [u32; 3] = [11, 12, 13];
/// The GPU's RIMU (O:`stm32_rifsc.c:183-186`; `-rif.dtsi:634`).
pub const RIMU_GPU: u32 = 9;

// ---- LTDC secure-layer areas (RISAF4 regions 13 and 14) ---------------------

/// `ltdc_sec_layer` (O:`stm32mp257f-dk-ca35tdcid-resmem.dtsi`, `-rif.dtsi:989`;
/// pre-study :630).
pub const LTDC_SEC_LAYER_BASE: u64 = 0xfe80_0000;
pub const LTDC_SEC_LAYER_SIZE: u64 = 8 << 20;
/// `ltdc_sec_rotation` (`-rif.dtsi:993`; pre-study :645).
pub const LTDC_SEC_ROTATION_BASE: u64 = 0xff00_0000;
pub const LTDC_SEC_ROTATION_SIZE: u64 = 16 << 20;

// ---- SCMI ids OP-TEE exposes ------------------------------------------------

/// SCMI clock protocol (O:`core/drivers/scmi-msg/clock.h:22-26`).
pub const SCMI_PROTO_CLOCK: u8 = 0x14;
/// SCMI voltage-domain protocol (O:`voltage_domain.h:20-27`).
pub const SCMI_PROTO_VOLTAGE: u8 = 0x17;
/// `ICN_DISPLAY`, critical and always on, enable optional
/// (O:`stm32mp25-st-scmi-cfg.dtsi:70-75`).
pub const SCMI_CLK_ICN_DISPLAY: u32 = 3;
/// `FLEXGEN_12`, the I2C2 kernel clock, 100 MHz (O:`stm32mp25-clks.h:412`).
pub const SCMI_CLK_FLEXGEN_12: u32 = 12;
/// `FLEXGEN_27`, the LTDC kernel clock; `RATE_SET` allowed
/// (O:`stm32mp25-clks.h:427`, `stm32mp25-st-scmi-cfg.dtsi:243-249`).
pub const SCMI_CLK_FLEXGEN_27: u32 = 27;
/// `FLEXGEN_28`, the DSI PHY reference, 27 MHz, rate fixed
/// (O:`stm32mp25-clks.h:428`, `stm32mp25-st-scmi-cfg.dtsi:251-256`).
pub const SCMI_CLK_FLEXGEN_28: u32 = 28;
/// GPIOB bus clock (O:`stm32mp25-clks.h:489`).
pub const SCMI_CLK_GPIOB: u32 = 90;
/// GPIOF bus clock (O:`stm32mp25-clks.h:493`).
pub const SCMI_CLK_GPIOF: u32 = 94;
/// Voltage domain of BUCK2, vddcore, DSI `vdd` (L:`stm32mp257f-dk.dts:328-331`;
/// note row 18).
pub const SCMI_VOLT_VDDCORE: u32 = 11;
/// BUCK5, 1.8 V: ADV7535 and DSI `vdda18` (O:`stm32mp257f-dk.dts:106-110`).
pub const SCMI_VOLT_V1V8: u32 = 14;
/// BUCK7, 3.3 V: ADV7535 `v3p3` (O:`stm32mp257f-dk.dts:169-173`).
pub const SCMI_VOLT_V3V3: u32 = 16;
/// LDO5, the HDMI +5 V, `scmi_v5v_hdmi` (O:`stm32mp257f-dk.dts:293-300`).
pub const SCMI_VOLT_V5V_HDMI: u32 = 21;
/// Voltage `CONFIG_SET` value for on in architectural mode, 0 = off
/// (O:`scmi_voltage_domain.h:81-96`).
pub const SCMI_VOLT_CONFIG_ON: u32 = 0x7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases_match_dtsi() {
        assert_eq!(LTDC_BASE, 0x48010000);
        assert_eq!(DSI_BASE, 0x48000000);
        assert_eq!(I2C2_BASE, 0x40130000);
        assert_eq!(RIFSC_BASE, 0x42080000);
        assert_eq!(GPIOB_BASE, 0x44250000);
        assert_eq!(GPIOF_BASE, 0x44290000);
        assert_eq!(RCC_BASE + RCC_LTDCCFGR, 0x44200840);
        assert_eq!(RCC_BASE + RCC_DSICFGR, 0x44200844);
        assert_eq!(RCC_BASE + RCC_I2C2CFGR, 0x442007A8);
        assert_eq!(SYSCFG_BASE + SYSCFG_DISPLAYCLKCR, 0x44235000);
    }

    #[test]
    fn rcc_bits() {
        assert_eq!(CFGR_GATE, 1 << 1);
        assert_eq!(CFGR_RESET, 1 << 0);
        assert_eq!(DSICFGR_DSIBLSEL, 0x1000);
        assert_eq!(DSICFGR_DSIPHYCKREFSEL, 0x8000);
        assert_eq!(DISPLAYCLKCR_CK_KER_LTDC & !DISPLAYCLKCR_MASK, 0);
        assert_eq!(DISPLAYCLKCR_TXBYTECLK, 0);
        assert_eq!(DISPLAYCLKCR_LVDS, 1);
    }

    #[test]
    fn linux_ids() {
        assert_eq!([LINUX_CK_BUS_LTDC, LINUX_CK_KER_LTDC, LINUX_CK_BUS_DSI], [206, 339, 205]);
        assert_eq!([LINUX_CK_KER_DSIPHY, LINUX_CK_KER_I2C2], [289, 263]);
        assert_eq!([LINUX_RST_LTDC, LINUX_RST_DSI, LINUX_RST_I2C2], [76, 77, 43]);
    }

    #[test]
    fn rifsc_ids_and_registers() {
        assert_eq!(RIFSC_I2C2, 42);
        assert_eq!([RIFSC_LTDC_CMN, RIFSC_LTDC_L1L2, RIFSC_LTDC_L3, RIFSC_LTDC_ROT], [80, 119, 120, 121]);
        assert_eq!([RIFSC_DSI_CMN, RIFSC_DSI_TRIG, RIFSC_DSI_RDFIFO], [81, 122, 123]);
        assert_eq!(rifsc_seccfgr(42), 0x14);
        assert_eq!(rifsc_seccfgr(80), 0x18);
        assert_eq!(rifsc_seccfgr(119), 0x1C);
        assert_eq!(rifsc_cidcfgr(80), 0x380);
        assert_eq!(rifsc_rimc_attr(11), 0xC3C);
        assert_eq!(RIMU_LTDC_CANDIDATES, [11, 12, 13]);
        assert_eq!(RIMU_GPU, 9);
    }

    #[test]
    fn secure_layer_areas() {
        assert_eq!(LTDC_SEC_LAYER_BASE + LTDC_SEC_LAYER_SIZE, LTDC_SEC_ROTATION_BASE);
        assert_eq!(LTDC_SEC_ROTATION_BASE + LTDC_SEC_ROTATION_SIZE, 0x1_0000_0000);
    }

    #[test]
    fn scmi_ids() {
        assert_eq!([SCMI_PROTO_CLOCK, SCMI_PROTO_VOLTAGE], [0x14, 0x17]);
        assert_eq!(
            [
                SCMI_CLK_ICN_DISPLAY,
                SCMI_CLK_FLEXGEN_12,
                SCMI_CLK_FLEXGEN_27,
                SCMI_CLK_FLEXGEN_28,
                SCMI_CLK_GPIOB,
                SCMI_CLK_GPIOF
            ],
            [3, 12, 27, 28, 90, 94]
        );
        assert_eq!([SCMI_VOLT_VDDCORE, SCMI_VOLT_V1V8, SCMI_VOLT_V3V3, SCMI_VOLT_V5V_HDMI], [11, 14, 16, 21]);
        assert_eq!(SCMI_VOLT_CONFIG_ON, 7);
    }
}
