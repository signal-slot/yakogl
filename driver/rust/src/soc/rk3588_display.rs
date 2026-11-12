// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RK3588 display path's power, clocks and resets: what must be on
//! before VOP2 (0xfdd90000), the DW HDMI QP TX0 (0xfde80000) and the HDPTX
//! PHY0 (0xfed60000) may be touched, and the order that turns it on and off
//! (driver/docs/nanopc-t6-display.md, section 1 "Power domains, clocks,
//! resets", step D4).
//!
//! The power domains are rows of [`super::rk3588::DOMAINS`], switched by
//! [`pd_on`] / [`pd_off`] as PD_GPU is. This module adds the CRU side.
//!
//! Facts from Linux v6.18 (GPL-2.0: facts and file:line only, nothing
//! transcribed):
//!
//! - CRU registers: `CLKSEL_CON(x)` 0x300 + 4x, `CLKGATE_CON(x)` 0x800 + 4x,
//!   `SOFTRST_CON(x)` 0xa00 + 4x; the PMU1 CRU is the same layout at CRU +
//!   0x30000 (0xfd7f0000) (drivers/clk/rockchip/clk.h:354, 364-366,
//!   381-383; rst-rk3588.c:22-23).
//! - Every mux, divider and gate takes a write-enable twin of its bits in
//!   31:16; a gate bit set is the clock gated (clk-rk3588.c:520-522).
//! - A reset is asserted by writing its bit and its twin, released by
//!   writing the twin alone (softrst.c:37-39, 69-70).
//!
//! The clocks and resets each block needs (clk-rk3588.c, rst-rk3588.c and
//! the nodes of rk3588-base.dtsi) are with [`Block`]; the rates are with
//! [`aclk_vop_set`]. A rate Linux leaves to the boot loader (the VO1 and VOP
//! bus roots' muxes, the HDMI eARC and reference dividers, the PHY reference
//! divider) is read, not set: only a board dump can give it.

use super::rk3588::{
    clkgate_con, clksel_con, gate_open, pd_off, pd_on, pll_con, pll_hz, pll_mode, Gate, PdError, PdOn, PdRegs, Pll,
    CPLL, GPLL, MODE_CON0, NPLL, PD_VO0, PD_VO1, PD_VOP, PLL_CON1_PWRDOWN,
};
use crate::mmio::Mmio;

/// The PMU1 CRU's offset from the CRU (`RK3588_PMU_CRU_BASE`, clk.h:354).
pub const PMU1_CRU: usize = 0x3_0000;

/// `RK3588_SOFTRST_CON(x)` (clk.h:366).
pub const fn softrst_con(x: usize) -> usize {
    0xa00 + 4 * x
}

/// The part of the CRU a gate, field or reset is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The CRU at 0xfd7c0000.
    Cru,
    /// The PMU1 CRU at CRU + [`PMU1_CRU`].
    Pmu1,
}

impl Part {
    /// The part's offset from the CRU base.
    pub const fn offset(self) -> usize {
        match self {
            Part::Cru => 0,
            Part::Pmu1 => PMU1_CRU,
        }
    }
}

/// A clock gate in either part of the CRU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clk {
    pub part: Part,
    pub gate: Gate,
}

const fn cru_gate(con: usize, bit: u32) -> Clk {
    Clk { part: Part::Cru, gate: Gate { con, bit } }
}

/// A soft reset: its `SOFTRST_CON` index and bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reset {
    pub part: Part,
    pub con: usize,
    pub bit: u32,
}

impl Reset {
    /// The register's offset from the CRU base.
    pub const fn offset(self) -> usize {
        self.part.offset() + softrst_con(self.con)
    }
    /// The word that asserts it.
    pub const fn assert_word(self) -> u32 {
        (1 << (self.bit + 16)) | (1 << self.bit)
    }
    /// The word that releases it.
    pub const fn release_word(self) -> u32 {
        1 << (self.bit + 16)
    }
}

const fn rst(con: usize, bit: u32) -> Reset {
    Reset { part: Part::Cru, con, bit }
}

/// A mux or divider field of a `CLKSEL_CON`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub part: Part,
    pub con: usize,
    pub shift: u32,
    pub width: u32,
}

impl Field {
    pub const fn mask(self) -> u32 {
        ((1 << self.width) - 1) << self.shift
    }
    /// The field of a `CLKSEL_CON` value, shifted down.
    pub const fn get(self, sel: u32) -> u32 {
        (sel & self.mask()) >> self.shift
    }
    /// The register's offset from the CRU base.
    pub const fn offset(self) -> usize {
        self.part.offset() + clksel_con(self.con)
    }
}

const fn sel(con: usize, shift: u32, width: u32) -> Field {
    Field { part: Part::Cru, con, shift, width }
}

/// The clocks (gate CON/bit, clk-rk3588.c line).
pub mod clk {
    use super::{cru_gate, Clk, Field, Part};
    use crate::soc::rk3588::Gate;

    /// `ACLK_VOP_ROOT` gate 52/0; mux [`ACLK_VOP_ROOT_MUX`] (gpll, cpll,
    /// dummy_aupll, npll, spll), divider [`ACLK_VOP_ROOT_DIV`] (:1908-1910,
    /// :445).
    pub const ACLK_VOP_ROOT: Clk = cru_gate(52, 0);
    pub const ACLK_VOP_ROOT_MUX: Field = super::sel(110, 5, 3);
    pub const ACLK_VOP_ROOT_DIV: Field = super::sel(110, 0, 5);
    /// `HCLK_VOP_ROOT` gate 52/2, mux CLKSEL_CON(110)[11:10] (:1914-1916).
    pub const HCLK_VOP_ROOT: Clk = cru_gate(52, 2);
    pub const HCLK_VOP_ROOT_MUX: Field = super::sel(110, 10, 2);
    /// `PCLK_VOP_ROOT` gate 52/3, mux CLKSEL_CON(110)[13:12] (:1917-1919).
    pub const PCLK_VOP_ROOT: Clk = cru_gate(52, 3);
    pub const PCLK_VOP_ROOT_MUX: Field = super::sel(110, 12, 2);
    /// `aclk_vop_sub_src`, ACLK_VOP's parent: CLKSEL_CON(115) bit 9, 0 =
    /// `aclk_vop_root`, 1 = `aclk_vop_div2_src` (:1926-1927, :502), a clock
    /// v6.18 names as a parent and defines nowhere.
    pub const ACLK_VOP_SUB_SRC: Field = super::sel(115, 9, 1);
    /// `HCLK_VOP` 52/8 from `hclk_vop_root` (:2087-2088).
    pub const HCLK_VOP: Clk = cru_gate(52, 8);
    /// `ACLK_VOP` 52/9 from `aclk_vop_sub_src` (:2089-2090).
    pub const ACLK_VOP: Clk = cru_gate(52, 9);
    /// `DCLK_VOP0_SRC` 52/10, mux CLKSEL_CON(111)[8:7] (gpll, cpll, v0pll,
    /// aupll), divider [6:0] (:2091-2093, :442). Not used while DCLK_VOP0
    /// runs from the PHY.
    pub const DCLK_VOP0_SRC: Clk = cru_gate(52, 10);
    /// `DCLK_VOP0` 52/13; mux [`DCLK_VOP0_MUX`]: 0 `dclk_vop0_src`, 1
    /// `clk_hdmiphy_pixel0`, 2 `clk_hdmiphy_pixel1` (:2100-2103, :503).
    pub const DCLK_VOP0: Clk = cru_gate(52, 13);
    pub const DCLK_VOP0_MUX: Field = super::sel(112, 7, 2);
    pub const DCLK_VOP0_FROM_HDMIPHY0: u32 = 1;

    /// `aclk_hdcp1_root` 59/0, mux CLKSEL_CON(128)[6:5], divider [4:0]
    /// (:1890-1892); the parent of `CLK_HDMITX0_REF`.
    pub const ACLK_HDCP1_ROOT: Clk = cru_gate(59, 0);
    /// `hclk_vo1_root` 59/2, mux CLKSEL_CON(128)[14:13] (:1896-1898).
    pub const HCLK_VO1_ROOT: Clk = cru_gate(59, 2);
    /// `pclk_vo1_root` 59/4, mux CLKSEL_CON(129)[3:2] (:1902-1904).
    pub const PCLK_VO1_ROOT: Clk = cru_gate(59, 4);
    /// `ACLK_VO1USB_TOP_ROOT` 74/0 and `HCLK_VO1USB_TOP_ROOT` 74/2, both
    /// critical (:1920-1925): the links of `aclk_hdcp1_pre` and `HCLK_VO1`.
    pub const ACLK_VO1USB_TOP_ROOT: Clk = cru_gate(74, 0);
    pub const HCLK_VO1USB_TOP_ROOT: Clk = cru_gate(74, 2);
    /// `HCLK_VO1` 59/9 from `hclk_vo1_root`, linked to
    /// `HCLK_VO1USB_TOP_ROOT` 74/2 (:2431).
    pub const HCLK_VO1: Clk = cru_gate(59, 9);
    /// `PCLK_VO1GRF` 59/12 from `pclk_vo1_root` (:2436): vo1_grf.
    pub const PCLK_VO1GRF: Clk = cru_gate(59, 12);
    /// `PCLK_HDMITX0` 60/11 from `pclk_vo1_root` (:1962-1963).
    pub const PCLK_HDMITX0: Clk = cru_gate(60, 11);
    /// `CLK_HDMITX0_EARC` 60/15, mux CLKSEL_CON(133) bit 6 (gpll, cpll),
    /// divider [5:1] (:1964-1966).
    pub const CLK_HDMITX0_EARC: Clk = cru_gate(60, 15);
    /// `CLK_HDMITX0_REF` 61/0 from `aclk_hdcp1_root` (:1967-1968).
    pub const CLK_HDMITX0_REF: Clk = cru_gate(61, 0);
    /// `CLK_HDMIHDP0` 73/12 from xin24m (:2075-2076).
    pub const CLK_HDMIHDP0: Clk = cru_gate(73, 12);

    /// `pclk_top_root` 1/1 (critical; :752-754): the parent of
    /// `PCLK_HDPTX0`.
    pub const PCLK_TOP_ROOT: Clk = cru_gate(1, 1);
    /// `PCLK_HDPTX0` 72/5 (:2079-2080): the PHY's "apb" clock.
    pub const PCLK_HDPTX0: Clk = cru_gate(72, 5);
    /// `CLK_USB2PHY_HDPTXRXPHY_REF`, the PHY's "ref" clock, in the PMU1
    /// CRU: gate 4/7, mux [`HDPTX_REF_MUX`] (0 xin24m, 1 ppll), divider
    /// [`HDPTX_REF_DIV`]; critical (:2278-2281, :515).
    pub const HDPTX_REF: Clk = Clk { part: Part::Pmu1, gate: Gate { con: 4, bit: 7 } };
    pub const HDPTX_REF_MUX: Field = Field { part: Part::Pmu1, con: 14, shift: 14, width: 1 };
    pub const HDPTX_REF_DIV: Field = Field { part: Part::Pmu1, con: 14, shift: 9, width: 5 };
}

/// The soft resets (rst-rk3588.c line).
pub mod reset {
    use super::{rst, Part, Reset};

    /// `SRST_H_VOP` 52/8, `SRST_A_VOP` 52/9, `SRST_D_VOP0` 52/13
    /// (:533-535).
    pub const H_VOP: Reset = rst(52, 8);
    pub const A_VOP: Reset = rst(52, 9);
    pub const D_VOP0: Reset = rst(52, 13);
    /// `SRST_P_HDMITX0` 60/11 (:596), `SRST_HDMITX0_REF` 61/0 (:599),
    /// `SRST_HDMIHDP0` 73/12 (:719).
    pub const P_HDMITX0: Reset = rst(60, 11);
    pub const HDMITX0_REF: Reset = rst(61, 0);
    pub const HDMIHDP0: Reset = rst(73, 12);
    /// `SRST_P_HDPTX0` 72/5 (:695), `SRST_HDPTX0_ROPLL` 72/12,
    /// `SRST_HDPTX0_LCPLL` 72/13, `SRST_HDPTX0` 72/14 (:702-704).
    pub const P_HDPTX0: Reset = rst(72, 5);
    pub const HDPTX0_ROPLL: Reset = rst(72, 12);
    pub const HDPTX0_LCPLL: Reset = rst(72, 13);
    pub const HDPTX0: Reset = rst(72, 14);
    /// In the PMU1 CRU: `SRST_HDPTX0_INIT` 3/11, `SRST_HDPTX0_CMN` 3/12,
    /// `SRST_HDPTX0_LANE` 3/13 (:788-790).
    pub const HDPTX0_INIT: Reset = Reset { part: Part::Pmu1, con: 3, bit: 11 };
    pub const HDPTX0_CMN: Reset = Reset { part: Part::Pmu1, con: 3, bit: 12 };
    pub const HDPTX0_LANE: Reset = Reset { part: Part::Pmu1, con: 3, bit: 13 };
}

/// A block of the display path and what its registers need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub name: &'static str,
    pub base: usize,
    pub size: usize,
    /// Its power domain (`None`: the PHY has none, rk3588-base.dtsi:
    /// 3118-3133).
    pub domain: Option<&'static super::rk3588::Domain>,
    /// The gates on before a register access, a parent before its child.
    pub clocks: &'static [Clk],
    /// The resets released before a register access.
    pub resets: &'static [Reset],
}

/// VOP2 (rk3588-base.dtsi:1369-1391: `vop`, 0x4200 bytes; clocks ACLK_VOP,
/// HCLK_VOP, DCLK_VOP0..3, PCLK_VOP_ROOT; PD_VOP; no resets in the node).
/// VP0's pixel clock (DCLK_VOP0) is not here: it runs from the PHY, which
/// the PHY bring-up starts.
pub const VOP: Block = Block {
    name: "vop",
    base: 0xfdd9_0000,
    size: 0x4200,
    domain: Some(&PD_VOP),
    clocks: &[clk::ACLK_VOP_ROOT, clk::ACLK_VOP, clk::HCLK_VOP_ROOT, clk::HCLK_VOP, clk::PCLK_VOP_ROOT],
    resets: &[reset::H_VOP, reset::A_VOP],
};

/// DW HDMI QP TX0 (rk3588-base.dtsi:1606-1627: 0x20000 bytes; clocks
/// PCLK_HDMITX0, CLK_HDMITX0_EARC, CLK_HDMITX0_REF, MCLK_I2S5_8CH_TX (audio,
/// not opened here), CLK_HDMIHDP0, HCLK_VO1; PD_VO1; resets HDMITX0_REF and
/// HDMIHDP0). `SRST_P_HDMITX0` is the register interface's reset
/// (rst-rk3588.c:596); it is released with the others. `HCLK_VO1` is a
/// linked gate whose link `HCLK_VO1USB_TOP_ROOT` 74/2 must run too, and
/// `ACLK_VO1USB_TOP_ROOT` 74/0 is the link of `aclk_hdcp1_pre` (clk-rk3588.c:
/// 1920-1925, 2430-2431): both are critical in Linux and opened with PD_VO1,
/// and checked here first.
pub const HDMI_TX0: Block = Block {
    name: "hdmi-tx0",
    base: 0xfde8_0000,
    size: 0x2_0000,
    domain: Some(&PD_VO1),
    clocks: &[
        clk::ACLK_VO1USB_TOP_ROOT,
        clk::HCLK_VO1USB_TOP_ROOT,
        clk::PCLK_VO1_ROOT,
        clk::PCLK_HDMITX0,
        clk::HCLK_VO1_ROOT,
        clk::HCLK_VO1,
        clk::ACLK_HDCP1_ROOT,
        clk::CLK_HDMITX0_REF,
        clk::CLK_HDMITX0_EARC,
        clk::CLK_HDMIHDP0,
    ],
    resets: &[reset::P_HDMITX0, reset::HDMITX0_REF, reset::HDMIHDP0],
};

/// HDPTX PHY0 (rk3588-base.dtsi:3118-3133: 0x2000 bytes; clocks "ref"
/// CLK_USB2PHY_HDPTXRXPHY_REF and "apb" PCLK_HDPTX0; no power domain). Its
/// register file needs the APB clock and `SRST_P_HDPTX0` released; the
/// other six PHY resets are steps of the PHY's own bring-up and are left
/// to it ([`reset::HDPTX0`] ..).
pub const HDPTX0: Block = Block {
    name: "hdptx0",
    base: 0xfed6_0000,
    size: 0x2000,
    domain: None,
    clocks: &[clk::PCLK_TOP_ROOT, clk::PCLK_HDPTX0, clk::HDPTX_REF],
    resets: &[reset::P_HDPTX0],
};

/// The display blocks, in the order [`display_on`] opens them.
pub const BLOCKS: [Block; 3] = [VOP, HDMI_TX0, HDPTX0];

/// Why a block's registers may not be touched yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotReady {
    /// Its domain is not powered or not out of idle.
    DomainOff(&'static str),
    /// This gate reads gated.
    Gated(Clk),
    /// This reset reads asserted.
    InReset(Reset),
}

/// Whether `b`'s registers may be touched: its domain usable, its gates
/// open, its resets released (in that order; the first missing one).
/// Nothing is written.
pub fn ready(m: &impl Mmio, cru: usize, pmu: usize, b: &Block) -> Result<(), NotReady> {
    if let Some(d) = b.domain {
        if !PdRegs::of(m, pmu, d).usable_of(d) {
            return Err(NotReady::DomainOff(d.name));
        }
    }
    for c in b.clocks {
        if m.read32(cru + c.part.offset() + clkgate_con(c.gate.con)) & (1 << c.gate.bit) != 0 {
            return Err(NotReady::Gated(*c));
        }
    }
    for r in b.resets {
        if m.read32(cru + r.offset()) & (1 << r.bit) != 0 {
            return Err(NotReady::InReset(*r));
        }
    }
    Ok(())
}

/// Assert `r`.
pub fn reset_assert(m: &impl Mmio, cru: usize, r: Reset) {
    m.write32(cru + r.offset(), r.assert_word());
}

/// Release `r`.
pub fn reset_release(m: &impl Mmio, cru: usize, r: Reset) {
    m.write32(cru + r.offset(), r.release_word());
}

/// The rate the CRU sets ACLK_VOP to: 500 MHz (rk3588-base.dtsi:745-765,
/// `assigned-clocks` ACLK_VOP at :752, its rate at :762).
pub const ACLK_VOP_HZ: u64 = 500_000_000;

/// The one setting [`aclk_vop_set`] moves `aclk_vop_root` to: the CPLL over
/// 3. The CPLL reads 1500 MHz on the T6 ([`super::rk3588::SET_CPLL_HZ`],
/// measured under Linux); GPLL's 1188 MHz divides to no 500 MHz.
pub const ACLK_VOP_ROOT_CPLL: u32 = 1;
pub const ACLK_VOP_ROOT_DIV3: u32 = 3;

/// The words [`aclk_vop_set`] writes: CLKSEL_CON(110)[7:0] = mux 1 (cpll),
/// divider field 2 (0x00ff_0022), CLKSEL_CON(115) bit 9 = 0 (0x0200_0000).
pub const ACLK_VOP_SEL110: u32 = ((clk::ACLK_VOP_ROOT_MUX.mask() | clk::ACLK_VOP_ROOT_DIV.mask()) << 16)
    | (ACLK_VOP_ROOT_CPLL << 5)
    | (ACLK_VOP_ROOT_DIV3 - 1);
pub const ACLK_VOP_SEL115: u32 = clk::ACLK_VOP_SUB_SRC.mask() << 16;

/// The PLL `aclk_vop_root`'s mux value selects, if it is one this reads.
const fn aclk_vop_root_pll(mux: u32) -> Option<Pll> {
    match mux {
        0 => Some(GPLL),
        1 => Some(CPLL),
        3 => Some(NPLL),
        _ => None,
    }
}

/// The rate `pll` gives now: its `PLL_CON` rate when it is in normal mode
/// and not powered down ([`PLL_CON1_PWRDOWN`]: Linux powers a PLL down with
/// its last consumer and leaves its mode alone, see there); 0 otherwise.
fn pll_out_hz(m: &impl Mmio, cru: usize, pll: Pll) -> u64 {
    if pll_mode(m.read32(cru + MODE_CON0), pll) != 1 {
        return 0;
    }
    let w = [0, 1, 2].map(|i| m.read32(cru + pll_con(pll.con + i)));
    if w[1] & PLL_CON1_PWRDOWN != 0 {
        return 0;
    }
    pll_hz(w[0], w[1], w[2]).unwrap_or(0)
}

/// The rate of `aclk_vop_root` as the CRU sets it: its PLL (in normal mode,
/// powered) over the divider. 0 when this cannot say (dummy_aupll, spll, a
/// mux value past the parents, a PLL not in normal mode or powered down).
/// Nothing is written.
pub fn aclk_vop_root_hz(m: &impl Mmio, cru: usize) -> u64 {
    let sel110 = m.read32(cru + clksel_con(110));
    let Some(pll) = aclk_vop_root_pll(clk::ACLK_VOP_ROOT_MUX.get(sel110)) else {
        return 0;
    };
    pll_out_hz(m, cru, pll) / (clk::ACLK_VOP_ROOT_DIV.get(sel110) as u64 + 1)
}

/// Why the display clocks could not be set. Nothing was written for
/// `AclkVopNoRate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    /// ACLK_VOP is not at [`ACLK_VOP_HZ`] and the CPLL does not read 1500
    /// MHz in normal mode, powered (the CPLL rate read, 0 for none).
    AclkVopNoRate(u64),
    /// A CLKSEL_CON read back without the fields written (its offset from
    /// the CRU, the word read).
    ReadBack(usize, u32),
}

/// Put ACLK_VOP on [`ACLK_VOP_HZ`], writing only what is not there:
/// `aclk_vop_sub_src` on `aclk_vop_root`, `aclk_vop_root` on the CPLL over 3
/// ([`ACLK_VOP_SEL110`], [`ACLK_VOP_SEL115`]). `Ok(true)` when something was
/// written. Before the domain goes on: these clocks are PD_VOP's.
pub fn aclk_vop_set(m: &impl Mmio, cru: usize) -> Result<bool, ClockError> {
    let sel115 = cru + clk::ACLK_VOP_SUB_SRC.offset();
    let sub_ok = clk::ACLK_VOP_SUB_SRC.get(m.read32(sel115)) == 0;
    if sub_ok && aclk_vop_root_hz(m, cru) == ACLK_VOP_HZ {
        return Ok(false);
    }
    let cpll_hz = pll_out_hz(m, cru, CPLL);
    if cpll_hz != ACLK_VOP_HZ * ACLK_VOP_ROOT_DIV3 as u64 {
        return Err(ClockError::AclkVopNoRate(cpll_hz));
    }
    let sel110 = cru + clk::ACLK_VOP_ROOT_MUX.offset();
    m.write32(sel110, ACLK_VOP_SEL110);
    let fields = clk::ACLK_VOP_ROOT_MUX.mask() | clk::ACLK_VOP_ROOT_DIV.mask();
    let now = m.read32(sel110);
    if now & fields != ACLK_VOP_SEL110 & 0xffff {
        return Err(ClockError::ReadBack(clk::ACLK_VOP_ROOT_MUX.offset(), now));
    }
    if !sub_ok {
        m.write32(sel115, ACLK_VOP_SEL115);
        let now = m.read32(sel115);
        if clk::ACLK_VOP_SUB_SRC.get(now) != 0 {
            return Err(ClockError::ReadBack(clk::ACLK_VOP_SUB_SRC.offset(), now));
        }
    }
    Ok(true)
}

/// Why [`display_on`] or [`display_off`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayError {
    Clock(ClockError),
    /// A domain did not switch (the domain is named in it).
    Domain(PdError),
    /// A block gate still reads gated after its write (the CLKGATE_CON read).
    Gate(Clk, u32),
}

/// What [`display_on`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayOn {
    /// ACLK_VOP's rate was written.
    pub aclk_vop_set: bool,
    pub vop: PdOn,
    pub vo1: PdOn,
    /// The block gates it opened: bit i of entry b is `BLOCKS[b].clocks[i]`.
    pub opened: [u32; 3],
}

/// Turn the display path on, from any state the boot loader leaves:
///
/// 1. ACLK_VOP's rate ([`aclk_vop_set`]): a clock first, while PD_VOP may
///    still be off.
/// 2. PD_VOP, then PD_VO1 ([`pd_on`]: each opens its own clocks first, then
///    power, the memory reset when due, the repair status poll, the idle
///    release). Both are top-level domains (rk3588-base.dtsi:1020, 1044).
/// 3. Each block's gates, a parent before its child ([`BLOCKS`]).
/// 4. Each block's resets released (the release word only: a reset that is
///    already released stays so). Nothing is asserted here; a pulse is
///    [`reset_assert`] then [`reset_release`], the caller's choice.
///
/// After it, [`ready`] holds for every block of [`BLOCKS`]. Not done: the
/// QoS words of PD_VOP / PD_VO1 (Linux restores what it saved, this has
/// nothing saved), the GRF writes and the PHY's PLL (the drivers'), and
/// DCLK_VOP0's mux (set when the PHY runs, [`clk::DCLK_VOP0_MUX`]).
pub fn display_on(
    m: &impl Mmio,
    cru: usize,
    pmu: usize,
    delay_us: &mut impl FnMut(u32),
) -> Result<DisplayOn, DisplayError> {
    let mut out = DisplayOn { aclk_vop_set: aclk_vop_set(m, cru).map_err(DisplayError::Clock)?, ..Default::default() };
    out.vop = pd_on(m, cru, pmu, &PD_VOP, delay_us).map_err(DisplayError::Domain)?;
    out.vo1 = pd_on(m, cru, pmu, &PD_VO1, delay_us).map_err(DisplayError::Domain)?;
    for (b, blk) in BLOCKS.iter().enumerate() {
        for (i, c) in blk.clocks.iter().enumerate() {
            match gate_open(m, cru + c.part.offset(), c.gate) {
                Ok(true) => out.opened[b] |= 1 << i,
                Ok(false) => {}
                Err(now) => return Err(DisplayError::Gate(*c, now)),
            }
        }
    }
    for blk in BLOCKS {
        for r in blk.resets {
            reset_release(m, cru, *r);
        }
    }
    Ok(out)
}

/// Turn the display's domains off: PD_VO1, then PD_VO0 (inside PD_VOP; the
/// HDMI path does not use it, but PD_VOP goes off only after it), then
/// PD_VOP ([`pd_off`]: clocks on, idle request and its acknowledge, power
/// off, the repair status poll). The caller has stopped VOP2 (a VP in
/// standby at the end of a frame) and the HDMI TX first; every register of
/// these blocks is lost. The gates and resets are left as they are.
pub fn display_off(m: &impl Mmio, cru: usize, pmu: usize, delay_us: &mut impl FnMut(u32)) -> Result<(), DisplayError> {
    for d in [&PD_VO1, &PD_VO0, &PD_VOP] {
        pd_off(m, cru, pmu, d, delay_us).map_err(DisplayError::Domain)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::rk3588::{Domain, PdRefusal, DOMAINS, PD_GPU, PMU_BASE};
    use super::*;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};

    // The model restates every offset and bit from the Linux sources, not
    // from the code under test, so a wrong constant cannot agree with
    // itself (as model_hdmi.rs does).
    const CRU: usize = 0xfd7c_0000;
    const PMU: usize = 0xfd8d_8000;
    const PMU1CRU: usize = 0xfd7f_0000;
    // rk3588_pmu, pm-domains.c:1493-1506 (offsets :1494-1502)
    const PWR: usize = PMU + 0x14c;
    const REQ: usize = PMU + 0x10c;
    const ACK: usize = PMU + 0x118;
    const IDLE: usize = PMU + 0x120;
    const STATUS: usize = PMU + 0x180;
    const MEM_PWR: usize = PMU + 0x1a0;
    const CHAIN: usize = PMU + 0x1f0;
    const MEM_ST: usize = PMU + 0x1f8;
    const REPAIR: usize = PMU + 0x290;

    /// A modelled domain: name, power word/bit, memory bit, repair bit,
    /// request word/bits, idle bits, the gates (CLKGATE_CON, bit) it needs
    /// while it switches (pm-domains.c:1277, 1294-1296; rk3588-base.dtsi).
    struct Dom {
        name: &'static str,
        pwr: (usize, u32),
        mem: u32,
        repair: u32,
        req: (usize, u32),
        idle: u32,
        switch_gates: &'static [(usize, u32)],
    }
    const MODEL_DOMS: [Dom; 4] = [
        Dom {
            name: "gpu",
            pwr: (PWR, 1 << 0),
            mem: 0,
            repair: 1 << 1,
            req: (REQ, 1 << 0),
            idle: 1 << 0,
            switch_gates: &[(66, 1), (66, 4), (66, 6), (66, 7)],
        },
        Dom {
            name: "vop",
            pwr: (PWR + 4, 1 << 1),
            mem: 1 << 25,
            repair: 1 << 16,
            req: (REQ, 3 << 13),
            idle: 3 << 13,
            switch_gates: &[(52, 0), (52, 2), (52, 3), (52, 9)],
        },
        Dom {
            name: "vo0",
            pwr: (PWR + 4, 1 << 2),
            mem: 1 << 26,
            repair: 1 << 17,
            req: (REQ, 1 << 15),
            idle: 1 << 15,
            switch_gates: &[(55, 0), (55, 3), (55, 4), (55, 2), (55, 13), (55, 12), (52, 2)],
        },
        Dom {
            name: "vo1",
            pwr: (PWR + 4, 1 << 3),
            mem: 1 << 27,
            repair: 1 << 18,
            req: (REQ + 4, 1 << 0),
            idle: 1 << 16,
            switch_gates: &[(59, 4), (59, 5), (59, 3), (60, 6), (60, 5), (59, 1), (74, 2)],
        },
    ];

    /// A modelled block: its window, its domain, the gates (part offset,
    /// CLKGATE_CON, bit) and resets (part offset, SOFTRST_CON, bit) its
    /// registers need.
    struct Blk {
        name: &'static str,
        base: usize,
        size: usize,
        dom: Option<&'static str>,
        gates: &'static [(usize, usize, u32)],
        resets: &'static [(usize, usize, u32)],
    }
    const MODEL_BLKS: [Blk; 3] = [
        Blk {
            name: "vop",
            base: 0xfdd9_0000,
            size: 0x4200,
            dom: Some("vop"),
            gates: &[(0, 52, 8), (0, 52, 9), (0, 52, 3)],
            resets: &[(0, 52, 8), (0, 52, 9)],
        },
        Blk {
            name: "hdmi-tx0",
            base: 0xfde8_0000,
            size: 0x2_0000,
            dom: Some("vo1"),
            gates: &[(0, 60, 11), (0, 59, 9), (0, 61, 0), (0, 73, 12)],
            resets: &[(0, 60, 11), (0, 61, 0), (0, 73, 12)],
        },
        Blk { name: "hdptx0", base: 0xfed6_0000, size: 0x2000, dom: None, gates: &[(0, 72, 5)], resets: &[(0, 72, 5)] },
    ];

    /// The CRU, the PMU and the three blocks. CRU and PMU control words take
    /// the hiword write mask; the PMU status words follow the control words
    /// `lag` reads after the last write: a domain whose power bit is clear
    /// reads its memory chain on unless it is in `no_chain`, and powered
    /// (repair bit) unless it is `dead`; one in `stuck_on` reads powered
    /// whatever its power bit; a
    /// domain that is off or has its idle request set reads acknowledged and
    /// idle (the U-Boot words: PD_GPU off with no request reads ack and idle
    /// bit 0 set) unless `no_ack`; the memory status follows the memory
    /// power word; the idle state bits in `idle_stuck` stay set. A domain
    /// seen off loses its blocks' registers. A block
    /// access with its domain not usable, a gate closed or a reset asserted
    /// is refused: logged, a write dropped, a read 0. A PMU write that
    /// switches a domain while one of its switching gates is closed is
    /// refused the same way.
    struct Chip {
        regs: RefCell<HashMap<usize, u32>>,
        blocks: RefCell<BTreeMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        refused: RefCell<Vec<String>>,
        lag: u32,
        reads_since: RefCell<u32>,
        dead: Vec<&'static str>,
        no_ack: Vec<&'static str>,
        no_chain: Vec<&'static str>,
        stuck_on: Vec<&'static str>,
        idle_stuck: u32,
    }

    fn gate_addr(part: usize, con: usize) -> usize {
        CRU + part + 0x800 + 4 * con
    }
    fn rst_addr(part: usize, con: usize) -> usize {
        CRU + part + 0xa00 + 4 * con
    }

    impl Chip {
        /// Everything gated and in reset, the display domains off, the GPU
        /// off; the CPLL at 1500 MHz, GPLL 1188, NPLL 850, all in normal
        /// mode (MODE_CON0 0x545, the word U-Boot leaves).
        fn cold() -> Chip {
            let mut r = HashMap::new();
            for con in 0..78 {
                r.insert(gate_addr(0, con), 0xffff);
                r.insert(rst_addr(0, con), 0xffff);
                r.insert(CRU + 0x300 + 4 * con, 0);
            }
            for con in 78..180 {
                r.insert(CRU + 0x300 + 4 * con, 0);
            }
            for con in 0..6 {
                r.insert(gate_addr(0x3_0000, con), 0xffff);
                r.insert(rst_addr(0x3_0000, con), 0xffff);
            }
            r.insert(PMU1CRU + 0x300 + 4 * 14, 0);
            for (a, v) in [
                (CRU + 0x280, 0x545),
                (CRU + 4 * 104, 0xfa),
                (CRU + 4 * 105, 0x42),
                (CRU + 4 * 106, 0),
                (CRU + 4 * 112, 0xc6),
                (CRU + 4 * 113, 0x42),
                (CRU + 4 * 114, 0),
                (CRU + 4 * 120, 0x1a9),
                (CRU + 4 * 121, 0x83),
                (CRU + 4 * 122, 0),
                (PWR, 0xffff),
                (PWR + 4, 0xffff),
                (REQ, 0),
                (REQ + 4, 0),
                (ACK, 0),
                (IDLE, 0),
                (STATUS, 0),
                (MEM_PWR, 0),
                (MEM_PWR + 4, 0),
                (CHAIN, 0),
                (MEM_ST, 0),
                (REPAIR, 0),
            ] {
                r.insert(a, v);
            }
            let c = Chip {
                regs: RefCell::new(r),
                blocks: RefCell::new(BTreeMap::new()),
                writes: RefCell::new(vec![]),
                refused: RefCell::new(vec![]),
                lag: 2,
                reads_since: RefCell::new(0),
                dead: vec![],
                no_ack: vec![],
                no_chain: vec![],
                stuck_on: vec![],
                idle_stuck: 0,
            };
            c.settle();
            c
        }

        /// The PMU as U-Boot leaves it on the T6 (read at its prompt,
        /// 2026-11-12): 0x14c 0xfff9, 0x10c 0, 0x118 and 0x120 0x00021fff,
        /// 0x290 0xfff70001. Not read there, derived: 0x150 = 0x0011 (the
        /// repair word has RGA31 (bit 15) off, VOP, VO0, VO1 on and AUDIO
        /// off: power bits 0 and 4 set; the higher domains are not
        /// modelled) and 0x110 = 0
        /// (ack bit 16 is clear). The CRU as [`Chip::cold`], the display
        /// domains' switching gates and the blocks' gates open and their
        /// resets released (not read; the display state U-Boot leaves in
        /// the CRU is a dump item).
        fn uboot() -> Chip {
            let c = Chip::cold();
            for (a, v) in [
                (PWR, 0xfff9),
                (PWR + 4, 0x0011),
                (REQ, 0),
                (REQ + 4, 0),
                (ACK, 0x0002_1fff),
                (IDLE, 0x0002_1fff),
                (REPAIR, 0xfff7_0001),
            ] {
                c.regs.borrow_mut().insert(a, v);
            }
            for d in &MODEL_DOMS[1..] {
                for (con, bit) in d.switch_gates {
                    *c.regs.borrow_mut().get_mut(&gate_addr(0, *con)).unwrap() &= !(1 << bit);
                }
            }
            for b in &MODEL_BLKS {
                for (p, con, bit) in b.gates {
                    *c.regs.borrow_mut().get_mut(&gate_addr(*p, *con)).unwrap() &= !(1 << bit);
                }
                for (p, con, bit) in b.resets {
                    *c.regs.borrow_mut().get_mut(&rst_addr(*p, *con)).unwrap() &= !(1 << bit);
                }
            }
            c
        }

        fn get(&self, a: usize) -> u32 {
            self.regs.borrow()[&a]
        }

        fn powered(&self, d: &Dom) -> bool {
            self.get(REPAIR) & d.repair != 0
        }

        fn usable(&self, name: &str) -> bool {
            let d = MODEL_DOMS.iter().find(|d| d.name == name).unwrap();
            self.powered(d) && self.get(d.req.0) & d.req.1 == 0 && (self.get(ACK) | self.get(IDLE)) & d.idle == 0
        }

        /// Let the status words follow the control words.
        fn settle(&self) {
            let mut r = self.regs.borrow_mut();
            for d in &MODEL_DOMS {
                let pwr_on = r[&d.pwr.0] & d.pwr.1 == 0;
                let on = (pwr_on || self.stuck_on.contains(&d.name)) && !self.dead.contains(&d.name);
                let set = |r: &mut HashMap<usize, u32>, a: usize, bits: u32, v: bool| {
                    let w = r.get_mut(&a).unwrap();
                    *w = if v { *w | bits } else { *w & !bits };
                };
                set(&mut r, REPAIR, d.repair, on);
                if !self.no_chain.contains(&d.name) {
                    set(&mut r, CHAIN, d.mem, pwr_on);
                }
                let mem_off = r[&(MEM_PWR + d.pwr.0 - PWR)] & d.pwr.1 != 0;
                set(&mut r, MEM_ST, d.mem, mem_off);
                let idle = !on || r[&d.req.0] & d.req.1 != 0;
                if !self.no_ack.contains(&d.name) {
                    set(&mut r, ACK, d.idle, idle);
                    set(&mut r, IDLE, d.idle, idle);
                }
                *r.get_mut(&IDLE).unwrap() |= self.idle_stuck;
                if !on {
                    let mut blocks = self.blocks.borrow_mut();
                    for b in MODEL_BLKS.iter().filter(|b| b.dom == Some(d.name)) {
                        blocks.retain(|a, _| !(b.base..b.base + b.size).contains(a));
                    }
                }
            }
        }

        /// Why an access to the block at `addr` is refused, if it is.
        fn block_refusal(&self, b: &Blk) -> Option<String> {
            if let Some(d) = b.dom {
                if !self.usable(d) {
                    return Some(format!("{}: domain {d} not usable", b.name));
                }
            }
            for (p, con, bit) in b.gates {
                if self.get(gate_addr(*p, *con)) & (1 << bit) != 0 {
                    return Some(format!("{}: gate {con}/{bit} closed", b.name));
                }
            }
            for (p, con, bit) in b.resets {
                if self.get(rst_addr(*p, *con)) & (1 << bit) != 0 {
                    return Some(format!("{}: reset {con}/{bit} asserted", b.name));
                }
            }
            None
        }

        fn block_at(addr: usize) -> Option<&'static Blk> {
            MODEL_BLKS.iter().find(|b| (b.base..b.base + b.size).contains(&addr))
        }

        /// A PMU power or idle-request write switching a domain whose
        /// switching gates are not all open.
        fn check_switch(&self, addr: usize, val: u32) {
            for d in &MODEL_DOMS {
                let hits = (addr == d.pwr.0 && (val >> 16) & d.pwr.1 != 0)
                    || (addr == d.req.0 && (val >> 16) & d.req.1 != 0)
                    || (addr == MEM_PWR + d.pwr.0 - PWR && (val >> 16) & d.pwr.1 != 0);
                if !hits {
                    continue;
                }
                for (con, bit) in d.switch_gates {
                    if self.get(gate_addr(0, *con)) & (1 << bit) != 0 {
                        self.refused.borrow_mut().push(format!("{}: switched with gate {con}/{bit} closed", d.name));
                    }
                }
            }
        }
    }

    impl Mmio for Chip {
        fn read32(&self, addr: usize) -> u32 {
            *self.reads_since.borrow_mut() += 1;
            if *self.reads_since.borrow() > self.lag {
                self.settle();
            }
            if let Some(b) = Chip::block_at(addr) {
                if let Some(why) = self.block_refusal(b) {
                    self.refused.borrow_mut().push(format!("read {addr:#x}: {why}"));
                    return 0;
                }
                return *self.blocks.borrow().get(&addr).unwrap_or(&0);
            }
            *self.regs.borrow().get(&addr).unwrap_or_else(|| panic!("read of an unmodelled register {addr:#x}"))
        }
        fn write32(&self, addr: usize, val: u32) {
            self.writes.borrow_mut().push((addr, val));
            *self.reads_since.borrow_mut() = 0;
            if let Some(b) = Chip::block_at(addr) {
                if let Some(why) = self.block_refusal(b) {
                    self.refused.borrow_mut().push(format!("write {addr:#x}: {why}"));
                } else {
                    self.blocks.borrow_mut().insert(addr, val);
                }
                return;
            }
            self.check_switch(addr, val);
            let mut r = self.regs.borrow_mut();
            let e = r.get_mut(&addr).unwrap_or_else(|| panic!("write to an unmodelled register {addr:#x}"));
            let mask = val >> 16;
            *e = (*e & !mask) | (val & mask);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    fn no_delay(_: u32) {}

    #[test]
    fn the_domain_table_is_linux_v6_18() {
        // pm-domains.c:1277 (gpu), 1294 (vop), 1295 (vo0), 1296 (vo1):
        // p_offset, pwr, m_offset, m_status, r_status, r_offset, req, idle
        // (`status` is 0 in all four rows: "on" is the repair status);
        // rk3588-power.h:26, 46-48 the ids; rk3588-base.dtsi:892, 1020,
        // 1031 (inside vop), 1044 the tree. The clocks are checked against
        // the model below.
        let row = |name, id, pwr_word, pwr, mem, repair, req_word, req, idle, parent| Domain {
            name,
            id,
            pwr_word,
            pwr,
            mem_word: 0,
            mem,
            repair,
            req_word,
            req,
            idle,
            parent,
            clocks: &[],
        };
        let rows = [
            row("gpu", 12, 0x0, 1 << 0, 0, 1 << 1, 0x0, 1 << 0, 1 << 0, None),
            row("vop", 24, 0x4, 1 << 1, 1 << 25, 1 << 16, 0x0, (1 << 13) | (1 << 14), (1 << 13) | (1 << 14), None),
            row("vo0", 25, 0x4, 1 << 2, 1 << 26, 1 << 17, 0x0, 1 << 15, 1 << 15, Some(24)),
            row("vo1", 26, 0x4, 1 << 3, 1 << 27, 1 << 18, 0x4, 1 << 0, 1 << 16, None),
        ];
        assert_eq!(DOMAINS.len(), rows.len());
        for (d, r) in DOMAINS.iter().zip(rows) {
            assert_eq!(Domain { clocks: &[], ..*d }, r);
        }
        // and the model's restatement agrees with the table
        for (d, m) in DOMAINS.iter().zip(&MODEL_DOMS) {
            assert_eq!(d.name, m.name);
            assert_eq!((PMU_BASE + 0x14c + d.pwr_word, d.pwr), m.pwr);
            assert_eq!((PMU_BASE + 0x10c + d.req_word, d.req), m.req);
            assert_eq!((d.mem, d.repair, d.idle), (m.mem, m.repair, m.idle));
            // each gate Linux holds on while the domain switches is opened
            for (con, bit) in m.switch_gates {
                assert!(d.clocks.contains(&Gate { con: *con, bit: *bit }), "{}: {con}/{bit}", d.name);
            }
        }
    }

    #[test]
    fn the_words_uboot_leaves_have_the_display_domains_on_and_the_gpu_off() {
        let c = Chip::uboot();
        for (d, on) in [(&PD_GPU, false), (&PD_VOP, true), (&PD_VO0, true), (&PD_VO1, true)] {
            let r = PdRegs::of(&c, PMU, d);
            assert_eq!(r.powered(d), on, "{}", d.name);
            assert_eq!(r.usable_of(d), on, "{}", d.name);
        }
        // PD_GPU: power bit set (off), repair bit clear, and with no idle
        // request its acknowledge and idle state read set
        assert_eq!(c.get(PWR) & 1, 1);
        assert_eq!(c.get(ACK) & 1, 1);
        // the GPU's own domain bit is the only one of 0x14c's that stays off
        // among bits 0-2: NPU (1) and VCODEC (2) read on
        assert_eq!(c.get(PWR) & 0x7, 0x1);
        // nothing to do for the display domains in U-Boot's state
        let on = pd_on(&c, CRU, PMU, &PD_VOP, &mut no_delay).unwrap();
        assert!(on.was_on);
        let on = pd_on(&c, CRU, PMU, &PD_VO1, &mut no_delay).unwrap();
        assert!(on.was_on);
        assert!(c.writes.borrow().iter().all(|(a, _)| *a < PMU), "{:x?}", c.writes.borrow());
    }

    /// The index of the first write to `addr` whose enable bits include
    /// `bits`.
    fn at(w: &[(usize, u32)], addr: usize, bits: u32) -> usize {
        w.iter()
            .position(|(a, v)| *a == addr && (v >> 16) & bits == bits)
            .unwrap_or_else(|| panic!("no write to {addr:#x} enabling {bits:#x}: {w:x?}"))
    }

    #[test]
    fn display_on_from_cold_runs_clocks_power_poll_idle_then_gates_and_resets() {
        let c = Chip::cold();
        for b in &BLOCKS {
            assert!(ready(&c, CRU, PMU, b).is_err(), "{}", b.name);
        }
        let mut slept = 0;
        let on = display_on(&c, CRU, PMU, &mut |us| slept += us).unwrap();
        assert!(on.aclk_vop_set);
        assert!(!on.vop.was_on && !on.vo1.was_on);
        assert_eq!(*c.refused.borrow(), Vec::<String>::new());
        for b in &BLOCKS {
            assert_eq!(ready(&c, CRU, PMU, b), Ok(()), "{}", b.name);
        }
        let w = c.writes.borrow().clone();
        // ACLK_VOP's rate before anything of PD_VOP
        let sel = at(&w, CRU + 0x300 + 4 * 110, 0xff);
        assert_eq!(w[sel].1, 0x00ff_0022);
        // PD_VOP: its clocks, power, memory pulse, idle release
        let vop_gates = [(52, 0), (52, 9), (52, 2), (52, 3)].map(|(c, b)| at(&w, gate_addr(0, c), 1 << b));
        let vop_pwr = at(&w, PWR + 4, 1 << 1);
        let vop_mem_off = at(&w, MEM_PWR + 4, 1 << 1);
        let vop_idle = at(&w, REQ, 3 << 13);
        assert!(vop_gates.iter().all(|g| sel < *g && *g < vop_pwr), "{w:x?}");
        assert!(vop_pwr < vop_mem_off && vop_mem_off < vop_idle);
        assert_eq!(w[vop_pwr].1, 0x0002_0000);
        assert_eq!(w[vop_mem_off].1, 0x0002_0002);
        assert_eq!(w[vop_mem_off + 1], (MEM_PWR + 4, 0x0002_0000));
        assert_eq!(w[vop_idle].1, 0x6000_0000);
        // PD_VO1 after PD_VOP, the same shape; its request is word 2 bit 0
        let vo1_pwr = at(&w, PWR + 4, 1 << 3);
        let vo1_idle = at(&w, REQ + 4, 1);
        assert!(vop_idle < vo1_pwr && vo1_pwr < vo1_idle);
        assert_eq!(w[vo1_idle].1, 0x0001_0000);
        for (c, b) in [(59, 4), (60, 6), (60, 5), (74, 2)] {
            assert!(at(&w, gate_addr(0, c), 1 << b) < vo1_pwr);
        }
        // the block gates and the resets come last, resets after the gates
        let tx_gate = at(&w, gate_addr(0, 60), 1 << 11);
        let ref_gate = at(&w, gate_addr(0x3_0000, 4), 1 << 7);
        let first_rst = w.iter().position(|(a, _)| (CRU + 0xa00..CRU + 0xc00).contains(a)).unwrap();
        assert!(vo1_idle < tx_gate && tx_gate < first_rst && ref_gate < first_rst);
        let rsts: Vec<_> = w[first_rst..].to_vec();
        assert_eq!(
            rsts,
            vec![
                (rst_addr(0, 52), 0x0100_0000),
                (rst_addr(0, 52), 0x0200_0000),
                (rst_addr(0, 60), 0x0800_0000),
                (rst_addr(0, 61), 0x0001_0000),
                (rst_addr(0, 73), 0x1000_0000),
                (rst_addr(0, 72), 0x0020_0000),
            ]
        );
        // and now the blocks hold their registers
        c.write32(VOP.base + 0xc00, 0x8000_0000);
        assert_eq!(c.read32(VOP.base + 0xc00), 0x8000_0000);
        assert!(c.refused.borrow().is_empty());
        // the PHY's own resets were not touched
        assert_eq!(c.get(rst_addr(0x3_0000, 3)) & (7 << 11), 7 << 11);
        assert_eq!(c.get(rst_addr(0, 72)) & (7 << 12), 7 << 12);
        assert!(slept > 0);
    }

    #[test]
    fn display_on_in_uboots_state_writes_no_pmu_word() {
        let c = Chip::uboot();
        // ACLK_VOP already on CPLL / 3
        c.regs.borrow_mut().insert(CRU + 0x300 + 4 * 110, 0x22);
        let on = display_on(&c, CRU, PMU, &mut no_delay).unwrap();
        assert!(!on.aclk_vop_set && on.vop.was_on && on.vo1.was_on);
        assert!(c.writes.borrow().iter().all(|(a, _)| *a < PMU));
        assert!(c.refused.borrow().is_empty());
    }

    #[test]
    fn a_poll_that_never_answers_is_a_timeout_named_by_its_domain() {
        // VO1 never powers
        let mut c = Chip::cold();
        c.dead.push("vo1");
        let mut slept = 0;
        let r = display_on(&c, CRU, PMU, &mut |us| slept += us);
        match r {
            Err(DisplayError::Domain(PdError { domain: "vo1", why: PdRefusal::PowerTimeout, read, cause: None })) => {
                assert_eq!(read & (1 << 18), 0);
            }
            other => panic!("{other:?}"),
        }
        // VOP's idle acknowledge never clears
        let mut c = Chip::cold();
        c.no_ack.push("vop");
        let r = display_on(&c, CRU, PMU, &mut no_delay);
        assert!(matches!(r, Err(DisplayError::Domain(PdError { domain: "vop", why: PdRefusal::IdleAckTimeout, .. }))));
        // VOP's memory chain never reads on
        let mut c = Chip::cold();
        c.no_chain.push("vop");
        let r = display_on(&c, CRU, PMU, &mut no_delay);
        assert!(matches!(r, Err(DisplayError::Domain(PdError { domain: "vop", why: PdRefusal::MemChainTimeout, .. }))));
        // switching off: VO1's idle acknowledge never comes
        let mut c = Chip::uboot();
        c.lag = u32::MAX; // the status words never follow
        let r = pd_off(&c, CRU, PMU, &PD_VO1, &mut no_delay);
        assert!(matches!(r, Err(PdError { domain: "vo1", why: PdRefusal::IdleAckTimeout, .. })), "{r:?}");
    }

    #[test]
    fn the_wait_is_linux_10_ms() {
        let mut c = Chip::cold();
        c.dead.push("vop");
        let mut slept = 0u32;
        let r = pd_on(&c, CRU, PMU, &PD_VOP, &mut |us| slept += us);
        assert!(matches!(r, Err(PdError { domain: "vop", why: PdRefusal::PowerTimeout, .. })));
        // the memory pulse's 20 us, the two memory polls' few us, then the
        // power poll's 10001 one-us steps
        assert!((10_001 + 20..10_001 + 20 + 10).contains(&slept), "{slept}");
    }

    #[test]
    fn off_then_on_loses_the_blocks_registers() {
        let c = Chip::cold();
        display_on(&c, CRU, PMU, &mut no_delay).unwrap();
        c.write32(VOP.base + 0xc00, 0x8000_000f);
        c.write32(HDMI_TX0.base + 0x80, 428_571_429);
        c.write32(HDPTX0.base + 0x4, 0x55);
        display_off(&c, CRU, PMU, &mut no_delay).unwrap();
        assert!(!c.usable("vop") && !c.usable("vo0") && !c.usable("vo1"));
        // touching a block of an off domain is refused
        assert_eq!(c.read32(VOP.base + 0xc00), 0);
        assert_eq!(c.refused.borrow().len(), 1);
        c.refused.borrow_mut().clear();
        display_on(&c, CRU, PMU, &mut no_delay).unwrap();
        assert!(c.refused.borrow().is_empty(), "{:?}", c.refused.borrow());
        assert_eq!(c.read32(VOP.base + 0xc00), 0);
        assert_eq!(c.read32(HDMI_TX0.base + 0x80), 0);
        // the PHY has no domain: its register survives
        assert_eq!(c.read32(HDPTX0.base + 0x4), 0x55);
        assert!(c.refused.borrow().is_empty());
    }

    #[test]
    fn display_off_switches_children_first_with_their_clocks_on() {
        let c = Chip::uboot();
        display_off(&c, CRU, PMU, &mut no_delay).unwrap();
        assert!(c.refused.borrow().is_empty(), "{:?}", c.refused.borrow());
        let w = c.writes.borrow().clone();
        let vo1 = at(&w, PWR + 4, 1 << 3);
        let vo0 = at(&w, PWR + 4, 1 << 2);
        let vop = at(&w, PWR + 4, 1 << 1);
        assert!(vo1 < vo0 && vo0 < vop);
        // each preceded by its idle request (bit and twin)
        assert_eq!(w[at(&w, REQ + 4, 1)].1, 0x0001_0001);
        assert!(at(&w, REQ + 4, 1) < vo1);
        assert_eq!(w[at(&w, REQ, 3 << 13)].1, 0x6000_6000);
        assert!(at(&w, REQ, 1 << 15) < vo0 && vo0 < at(&w, REQ, 3 << 13));
        assert_eq!(w[vop].1, 0x0002_0002);
    }

    #[test]
    fn pd_vop_does_not_go_off_while_pd_vo0_is_on() {
        let c = Chip::uboot();
        let r = pd_off(&c, CRU, PMU, &PD_VOP, &mut no_delay);
        assert_eq!(r, Err(PdError { domain: "vop", why: PdRefusal::ChildOn, read: 0xfff7_0001, cause: Some("vo0") }));
        // nothing written at all: no PMU word and no CRU gate
        assert_eq!(*c.writes.borrow(), vec![]);
        // and PD_VO0 does not come on inside an off PD_VOP
        let c = Chip::cold();
        let r = pd_on(&c, CRU, PMU, &PD_VO0, &mut no_delay);
        assert_eq!(r, Err(PdError { domain: "vo0", why: PdRefusal::ParentOff, read: 0, cause: Some("vop") }));
        assert_eq!(*c.writes.borrow(), vec![]);
    }

    #[test]
    fn pd_off_waits_for_the_repair_bit_to_clear_and_times_out_by_domain() {
        // the repair bit clears 2 reads after the power write: waited for
        let c = Chip::uboot();
        let on = pd_off(&c, CRU, PMU, &PD_VO1, &mut no_delay).unwrap();
        assert!(!on.was_on);
        assert!(on.waited_us[0] > 0, "{on:?}");
        assert!(!c.usable("vo1") && c.get(REPAIR) & (1 << 18) == 0);
        // the repair bit never clears: refused, named by the domain, after
        // Linux's 10 ms
        let mut c = Chip::uboot();
        c.stuck_on.push("vo1");
        let mut slept = 0u32;
        let r = pd_off(&c, CRU, PMU, &PD_VO1, &mut |us| slept += us);
        assert_eq!(r, Err(PdError { domain: "vo1", why: PdRefusal::PowerTimeout, read: 0xfff7_0001, cause: None }));
        assert!(slept > 10_000, "{slept}");
        // the power write was made, the idle request before it
        let w = c.writes.borrow().clone();
        assert!(at(&w, REQ + 4, 1) < at(&w, PWR + 4, 1 << 3));
    }

    #[test]
    fn an_idle_acknowledge_alone_keeps_a_domain_from_being_usable() {
        // powered, no request, idle state clear, acknowledge bit 13 set: a
        // handshake under way
        let regs = PdRegs { repair: 1 << 16, idle_ack: 1 << 13, ..PdRegs::default() };
        assert!(regs.powered(&PD_VOP) && !regs.usable_of(&PD_VOP));
        let regs = PdRegs { repair: 1 << 16, idle_st: 1 << 14, ..PdRegs::default() };
        assert!(!regs.usable_of(&PD_VOP));
        let regs = PdRegs { repair: 1 << 16, idle_req: 1 << 13, ..PdRegs::default() };
        assert!(!regs.usable_of(&PD_VOP));
        assert!(PdRegs { repair: 1 << 16, ..PdRegs::default() }.usable_of(&PD_VOP));
        // the same through the chip: U-Boot's state with VOP's acknowledge
        // stuck set is not taken as on; the release is written and its
        // acknowledge poll refuses
        let mut c = Chip::uboot();
        c.no_ack.push("vop");
        *c.regs.borrow_mut().get_mut(&ACK).unwrap() |= 1 << 13;
        assert_eq!(ready(&c, CRU, PMU, &VOP), Err(NotReady::DomainOff("vop")));
        let r = pd_on(&c, CRU, PMU, &PD_VOP, &mut no_delay);
        assert!(matches!(r, Err(PdError { domain: "vop", why: PdRefusal::IdleAckTimeout, .. })), "{r:?}");
    }

    #[test]
    fn an_idle_release_with_one_bus_still_idle_is_refused() {
        // PD_VOP's two idle bits: bit 14 stays set after the release. Linux
        // would go on ("not all set"); this refuses, by domain.
        let mut c = Chip::cold();
        c.idle_stuck = 1 << 14;
        let r = display_on(&c, CRU, PMU, &mut no_delay);
        assert!(
            matches!(r, Err(DisplayError::Domain(PdError { domain: "vop", why: PdRefusal::IdleTimeout, .. }))),
            "{r:?}"
        );
        assert!(!c.usable("vop"));
    }

    #[test]
    fn hdmi_tx0_is_not_ready_with_its_vo1usb_link_gated() {
        let c = Chip::cold();
        display_on(&c, CRU, PMU, &mut no_delay).unwrap();
        assert_eq!(ready(&c, CRU, PMU, &HDMI_TX0), Ok(()));
        c.write32(gate_addr(0, 74), (1 << 18) | (1 << 2));
        assert_eq!(ready(&c, CRU, PMU, &HDMI_TX0), Err(NotReady::Gated(clk::HCLK_VO1USB_TOP_ROOT)));
    }

    #[test]
    fn aclk_vop_is_not_set_from_a_powered_down_cpll() {
        // CPLL in normal mode at 1500 MHz by its words, but PLL_CON1 bit 13
        // (powered down): no rate, refused, nothing written
        let c = Chip::cold();
        c.regs.borrow_mut().insert(CRU + 4 * 105, 0x2042);
        assert_eq!(aclk_vop_set(&c, CRU), Err(ClockError::AclkVopNoRate(0)));
        assert!(c.writes.borrow().is_empty());
        // and a root already on CPLL / 3 is not taken as 500 MHz then
        c.regs.borrow_mut().insert(CRU + 0x300 + 4 * 110, 0x22);
        assert_eq!(aclk_vop_root_hz(&c, CRU), 0);
    }

    #[test]
    fn a_block_touched_before_its_domain_clock_and_reset_is_refused() {
        let c = Chip::cold();
        c.write32(VOP.base, 1);
        assert_eq!(c.refused.borrow().last().unwrap(), &format!("write {:#x}: vop: domain vop not usable", VOP.base));
        assert_eq!(ready(&c, CRU, PMU, &VOP), Err(NotReady::DomainOff("vop")));
        pd_on(&c, CRU, PMU, &PD_VOP, &mut no_delay).unwrap();
        // the domain is on, HCLK_VOP still gated
        assert_eq!(ready(&c, CRU, PMU, &VOP), Err(NotReady::Gated(clk::HCLK_VOP)));
        c.read32(VOP.base);
        assert!(c.refused.borrow().last().unwrap().ends_with("gate 52/8 closed"));
        for g in VOP.clocks {
            gate_open(&c, CRU, g.gate).unwrap();
        }
        assert_eq!(ready(&c, CRU, PMU, &VOP), Err(NotReady::InReset(reset::H_VOP)));
        c.read32(VOP.base);
        assert!(c.refused.borrow().last().unwrap().ends_with("reset 52/8 asserted"));
        for r in VOP.resets {
            reset_release(&c, CRU, *r);
        }
        assert_eq!(ready(&c, CRU, PMU, &VOP), Ok(()));
        let n = c.refused.borrow().len();
        c.write32(VOP.base, 1);
        assert_eq!(c.read32(VOP.base), 1);
        assert_eq!(c.refused.borrow().len(), n);
        // an asserted reset closes it again
        reset_assert(&c, CRU, reset::A_VOP);
        assert_eq!(ready(&c, CRU, PMU, &VOP), Err(NotReady::InReset(reset::A_VOP)));
    }

    #[test]
    fn a_domain_switched_with_its_clocks_gated_is_refused_by_the_model() {
        // the model's own check: a power write with VOP's gates closed
        let c = Chip::cold();
        c.write32(PWR + 4, 0x0002_0000);
        assert!(c.refused.borrow()[0].starts_with("vop: switched with gate 52/0 closed"));
    }

    #[test]
    fn reset_words_and_addresses() {
        assert_eq!(reset::H_VOP.assert_word(), 0x0100_0100);
        assert_eq!(reset::H_VOP.release_word(), 0x0100_0000);
        assert_eq!(CRU + reset::D_VOP0.offset(), 0xfd7c_0ad0);
        assert_eq!(CRU + reset::HDMITX0_REF.offset(), 0xfd7c_0af4);
        assert_eq!(CRU + reset::HDPTX0.offset(), 0xfd7c_0b20);
        assert_eq!(reset::HDPTX0.assert_word(), 0x4000_4000);
        // the PMU1 CRU's SOFTRST_CON(3): 0xfd7f0000 + 0xa00 + 0xc
        assert_eq!(CRU + reset::HDPTX0_LANE.offset(), 0xfd7f_0a0c);
        assert_eq!(reset::HDPTX0_INIT.assert_word(), 0x0800_0800);
        assert_eq!(CRU + clk::HDPTX_REF.part.offset() + clkgate_con(clk::HDPTX_REF.gate.con), 0xfd7f_0810);
        assert_eq!(CRU + clk::HDPTX_REF_MUX.offset(), 0xfd7f_0338);
        // rst-rk3588.c's id form (con * 16 + bit) of the same resets
        for (r, id) in [(reset::A_VOP, 52 * 16 + 9), (reset::HDMIHDP0, 73 * 16 + 12), (reset::P_HDPTX0, 72 * 16 + 5)] {
            assert_eq!(r.con * 16 + r.bit as usize, id);
        }
    }

    #[test]
    fn aclk_vop_is_put_on_cpll_over_3_or_refused_without_a_write() {
        // already there: no write
        let c = Chip::cold();
        c.regs.borrow_mut().insert(CRU + 0x300 + 4 * 110, 0x22);
        assert_eq!(aclk_vop_root_hz(&c, CRU), 500_000_000);
        assert_eq!(aclk_vop_set(&c, CRU), Ok(false));
        assert!(c.writes.borrow().is_empty());
        // GPLL / 1 (the reset value of the field): written, and the
        // aclk_vop_sub_src mux moved off the undefined parent
        let c = Chip::cold();
        c.regs.borrow_mut().insert(CRU + 0x300 + 4 * 115, 1 << 9);
        assert_eq!(aclk_vop_root_hz(&c, CRU), 1_188_000_000);
        assert_eq!(aclk_vop_set(&c, CRU), Ok(true));
        assert_eq!(
            *c.writes.borrow(),
            vec![(CRU + 0x300 + 4 * 110, 0x00ff_0022), (CRU + 0x300 + 4 * 115, 0x0200_0000)]
        );
        assert_eq!(aclk_vop_root_hz(&c, CRU), 500_000_000);
        // the CPLL at another rate: refused, nothing written
        let c = Chip::cold();
        c.regs.borrow_mut().insert(CRU + 4 * 104, 0xc6);
        assert_eq!(aclk_vop_set(&c, CRU), Err(ClockError::AclkVopNoRate(1_188_000_000)));
        assert!(c.writes.borrow().is_empty());
        // and display_on stops there, before any domain write
        let r = display_on(&c, CRU, PMU, &mut no_delay);
        assert_eq!(r, Err(DisplayError::Clock(ClockError::AclkVopNoRate(1_188_000_000))));
        assert!(c.writes.borrow().is_empty());
    }

    #[test]
    fn the_gpu_domain_still_comes_on_alone() {
        let c = Chip::cold();
        let on = super::super::rk3588::pd_gpu_on(&c, CRU, PMU, &mut no_delay).unwrap();
        assert_eq!(on.opened, (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7));
        assert!(c.usable("gpu") && !c.usable("vop") && !c.usable("vo1"));
        assert!(c.refused.borrow().is_empty());
        // no word of the display domains moved
        assert_eq!(c.get(PWR + 4), 0xffff);
        assert_eq!(c.get(REQ + 4), 0);
    }
}
