// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 8M Plus HDMIMIX: its power domains, clock roots, the HDMI TX
//! block control (clock gates, resets, the PHY power-down pin) and the
//! parallel video interface (HTX_PVI) between LCDIF3 and the HDMI TX
//! controller.
//!
//! Written from the i.MX 8M Plus Applications Processor Reference Manual,
//! IMX8MPRM Rev. 3, 08/2024 ("RM"); register names and bits from hal_nxp
//! `PERI_GPC.h`, `PERI_GPC_PGC.h`, `PERI_CCM.h`, `PERI_HDMI_TX_BLK_CTL.h`,
//! `PERI_HTX_PVI.h` ([`super::regs`]). What the code relies on:
//!
//! - Power domains (5.2): HDMIMIX is PU14, its PGC at 0xE80 ("PGC for HDMI
//!   (PU14)"), HDMI PHY is PU15 at 0xEC0 (5.2.10 memory-map list); their
//!   software power-up triggers are `PU_PGC_SW_PUP_REQ` bits 14 and 15
//!   (5.2.10.37), the A53 mapping bits 16 and 17 (5.2.10.72), and HDMIMIX
//!   has an ADB400 handshake in `PU_PWRHSK`, request bit 13 ("Write 0 and
//!   wait according ackn before power down hdmimix") and acknowledge bit 29
//!   (5.2.10.57). The power-up itself is [`super::gpc`]'s.
//! - Clock roots (5.1, Table 5-1 "Clock Root Selects"): HDMI_APB_CLK_ROOT
//!   (slice 22, 0x8B00, at most 200 MHz; mux 010 = SYSTEM_PLL1_CLK),
//!   HDMI_AXI_CLK_ROOT (23, 0x8B80, at most 500 MHz; mux 111 =
//!   SYSTEM_PLL2_DIV2), HDMI_REF_266M_CLK_ROOT (120, 0xBC00, 266 MHz; mux
//!   100 = SYSTEM_PLL1_DIV3). Every HDMIMIX module clock is gated by
//!   `ccm_clk_enable_hdmi (CCGR95)` (Table 5-2 "System Clocks": HDMI TX
//!   controller, TX BLK CTRL, PVI, PAI, IRQ_STEER, LCDIF apb/b clocks).
//!   SYSTEM_PLL1_CLK is 800 MHz, SYSTEM_PLL1_DIV3 266, SYSTEM_PLL2_DIV2 500
//!   (5.1.5.1 "Input Clocks"); the PLLs are left as the boot firmware set
//!   them.
//! - Block control (13.11): `RTX_CLK_CTL0` / `RTX_CLK_CTL1` are the level-2
//!   clock enables of the HDMIMIX blocks, `RTX_CLK_CTL2..4` the fail-safe
//!   overrides, 00 = "Default clock gating logic" (13.11.3.1.7.2), left at
//!   reset; `RTX_RESET_CTL0` holds active-low resets, all asserted out of
//!   reset (reset value 0); each register has SET/CLR aliases at +4/+8
//!   (13.11.3.1.1). `GLOBAL_APB_CLK_EN` does not gate the block control
//!   itself, "since blk_ctl is required to be accessible at all times"
//!   (13.11.3.1.5.4). `TX_CONTROL0[TX_PHY_PDOWN]` "Drives PHY_PDOWN input of
//!   TX Phy" and resets to 1 (TX_CONTROL0 reset 18h, 13.11.3.1.1);
//!   `LCDIF_NOC_HURRY` lets the LCDIF's FIFO panic raise its NoC priority
//!   (13.11.3.1.13.4). `TX_STATUS0` mirrors the PHY's PLL lock, ready, clock
//!   ready and the HPD pin (13.11.3.1.15.4).
//! - PVI (13.13): `HTX_PVI_CTRL` reset 0037_7000h (all input and output
//!   strobes active high, background insertion on underflow and on FDCC
//!   enabled, the block off); `HTX_PVI_MODE` 2 = "Select the LCDIF Path",
//!   `HTX_PVI_EN` "Enable to block to receive incoming data",
//!   `TMG_GEN_EN` lets the timing generator re-insert HSYNC and VSYNC, the
//!   `INP_*` bits state the incoming polarities and the `OP_*` bits the
//!   outgoing ones (13.13.3.1.2.3).
//!
//! Not stated in IMX8MPRM Rev. 3, and therefore parameters measured from the
//! board dump (driver/docs/x2-hdmi-cold.md), never guessed here:
//! - Which value of `RTX_CLK_CTL1[HTX_PIPE_CLK_SEL]` selects "the Frac PLL
//!   clock output" and which "the PIXEL_CLK_OUT output from PHY", and of
//!   `HTXPHY_CLK_SEL` "24MHz clock" or "VPLL_PHY clock" for the PHY's
//!   reference (13.11.3.1.6.4). [`ClockSel`]: `None` leaves the reset value.
//! - What clock LCDIF3's `pix_clk` is (13.3.2.2 says only "Pixel Clock");
//!   the HDMIMIX figure 13-81 shows `lcdif_pixel_clk` behind
//!   `LCDIF_PIX_CLK_EN` and nothing upstream of it. The design note's
//!   reading is that it is the PHY's pixel clock output through
//!   `HTX_PIPE_CLK_SEL`.
//! - The timing generator's coordinate origin (13.13.3.1.6-10 give
//!   "Lower Right Corner" fields without a reference point), so it is not
//!   used: the PVI passes the LCDIF's strobes.
//! - A minimum reset pulse: none is given; [`RESET_PULSE_US`] is ours.
//! - The value scale of `LCDIF_NOC_HURRY` (3 bits, no encoding given).

use super::ccm::{self, RootSetting};
use super::gpc::{self, Adb, PuDomain};
use super::regs::{ccm as ccmr, gpc as gpcr, hdmi_tx_blk_ctl as blk, htx_pvi as pvi};
use crate::env::Env;
use crate::errno::EINVAL;
use crate::mmio::{field, Mmio};
use crate::video::VideoMode;

/// HDMIMIX, PU14 (see the module text).
pub const HDMIMIX: PuDomain = PuDomain {
    name: "hdmimix",
    trigger: gpcr::pu_pgc_sw_pup_req::HDMIMIX_SW_PUP_REQ_MASK,
    a53_map: gpcr::pgc_cpu_a53_mapping::HDMIMIX_DOMAIN_MASK,
    pgc: 14,
    adb: Some(Adb {
        req: gpcr::pu_pwrhsk::GPC_HDMIMIX_NOC_PWRDNREQN_MASK,
        ack: gpcr::pu_pwrhsk::GPC_HDMIMIX_NOC_PWRDNACKN_MASK,
    }),
};

/// HDMI PHY, PU15.
pub const HDMI_PHY: PuDomain = PuDomain {
    name: "hdmi-phy",
    trigger: gpcr::pu_pgc_sw_pup_req::HDMI_PHY_SW_PUP_REQ_MASK,
    a53_map: gpcr::pgc_cpu_a53_mapping::HDMI_PHY_DOMAIN_MASK,
    pgc: 15,
    adb: None,
};

/// Clock root slices and mux inputs (Table 5-1).
pub mod root {
    pub const HDMI_APB: u32 = 22;
    pub const HDMI_AXI: u32 = 23;
    pub const HDMI_REF_266M: u32 = 120;
    /// HDMI_APB: 010 = SYSTEM_PLL1_CLK (800 MHz).
    pub const MUX_APB_SYS_PLL1: u32 = 0b010;
    /// HDMI_AXI: 111 = SYSTEM_PLL2_DIV2 (500 MHz).
    pub const MUX_AXI_SYS_PLL2_DIV2: u32 = 0b111;
    /// HDMI_REF_266M: 100 = SYSTEM_PLL1_DIV3 (266.67 MHz).
    pub const MUX_REF_SYS_PLL1_DIV3: u32 = 0b100;
}

/// `ccm_clk_enable_hdmi`, CCGR95 (Table 5-2).
pub const GATE_HDMI: u32 = 95;

/// The three roots: APB 800 / 4 = 200 MHz, AXI 500 MHz, REF 266.67 MHz.
pub const ROOTS: [(u32, RootSetting); 3] = [
    (root::HDMI_APB, RootSetting::on(root::MUX_APB_SYS_PLL1, 4, 1)),
    (root::HDMI_AXI, RootSetting::on(root::MUX_AXI_SYS_PLL2_DIV2, 1, 1)),
    (root::HDMI_REF_266M, RootSetting::on(root::MUX_REF_SYS_PLL1_DIV3, 1, 1)),
];

/// `RTX_CLK_CTL0` enables: the global bus, reference and pixel clocks, the
/// IRQ steer, the HDMI NoC, and LCDIF3's APB, bus, PDI and pixel clocks.
pub const CLK0: u32 = blk::rtx_clk_ctl0::GLOBAL_APB_CLK_EN_MASK
    | blk::rtx_clk_ctl0::GLOBAL_B_CLK_EN_MASK
    | blk::rtx_clk_ctl0::GLOBAL_REF266M_CLK_EN_MASK
    | blk::rtx_clk_ctl0::GLOBAL_XTAL24M_CLK_EN_MASK
    | blk::rtx_clk_ctl0::GLOBAL_TX_PIX_CLK_EN_MASK
    | blk::rtx_clk_ctl0::IRQS_CLK_EN_MASK
    | blk::rtx_clk_ctl0::NOC_HDMI_CLK_EN_MASK
    | blk::rtx_clk_ctl0::LCDIF_APB_CLK_EN_MASK
    | blk::rtx_clk_ctl0::LCDIF_B_CLK_EN_MASK
    | blk::rtx_clk_ctl0::LCDIF_PDI_CLK_EN_MASK
    | blk::rtx_clk_ctl0::LCDIF_PIX_CLK_EN_MASK;

/// The part of [`CLK0`] the ADB400 acknowledge and the block registers need.
pub const CLK0_BUS: u32 = blk::rtx_clk_ctl0::GLOBAL_APB_CLK_EN_MASK
    | blk::rtx_clk_ctl0::GLOBAL_B_CLK_EN_MASK
    | blk::rtx_clk_ctl0::NOC_HDMI_CLK_EN_MASK;

/// `RTX_CLK_CTL1` enables: the TX controller's hpi, apb, pixel, sfr and
/// prep clocks, the PHY's apb and int clocks, the PVI pixel clock. Not the
/// audio (gpa), CEC, HDCP (esm, skp), TRNG, PAI or line-buffer clocks.
pub const CLK1: u32 = blk::rtx_clk_ctl1::TX_HPI_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_APB_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_PIXEL_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_SFR_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_PREP_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_PHY_APB_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_PHY_INT_CLK_EN_MASK
    | blk::rtx_clk_ctl1::TX_VID_LINK_PIX_CLK_EN_MASK;

/// The resets this driver owns in `RTX_RESET_CTL0` (all active low): the
/// NoC, LCDIF3 (async and APB), the TX controller (core and APB), the PHY's
/// APB, the IRQ steer, the PVI.
pub const RESETS: u32 = blk::rtx_reset_ctl0::NOC_RESET_N_MASK
    | blk::rtx_reset_ctl0::LCDIF_ASYNC_RESET_N_MASK
    | blk::rtx_reset_ctl0::LCDIF_APB_RESET_N_MASK
    | blk::rtx_reset_ctl0::TX_RSTZ_MASK
    | blk::rtx_reset_ctl0::TX_APBRSTZ_MASK
    | blk::rtx_reset_ctl0::TX_PHY_PRESETN_MASK
    | blk::rtx_reset_ctl0::IRQ_RESETN_MASK
    | blk::rtx_reset_ctl0::VID_LINK_SLV_RESETN_MASK;

/// Assert-to-release time of a reset pulse (ours; see the module text).
pub const RESET_PULSE_US: u32 = 10;

/// `HTX_PVI_CTRL` reset value (13.13.3.1.1).
pub const PVI_CTRL_RESET: u32 = 0x0037_7000;
/// `HTX_PVI_MODE`: the LCDIF path.
pub const PVI_MODE_LCDIF: u32 = 2;

/// Register bases this module uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub blk: usize,
    pub pvi: usize,
    pub gpc: usize,
    pub ccm: usize,
}

/// The two clock selects the RM does not decode (see the module text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockSel {
    pub htx_pipe: Option<bool>,
    pub htxphy: Option<bool>,
}

/// Which step of [`power_on`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Root(i32),
    Gate(i32),
    Gpc(&'static str, gpc::Fail),
}

impl Fail {
    pub fn errno(self) -> i32 {
        match self {
            Fail::Root(e) | Fail::Gate(e) => e,
            Fail::Gpc(_, f) => f.errno(),
        }
    }
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, v: u32) {
    m.write32(base + off as usize, v)
}

/// What [`power_on`] found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PowerReport {
    pub roots_written: u32,
    pub gate_opened: bool,
    pub hdmimix_was_down: bool,
    pub phy_was_down: bool,
}

/// Clock roots and gate, HDMIMIX (with its ADB400), the block control's bus
/// clocks, HDMI PHY. Afterwards the block control registers are reachable.
pub fn power_on(m: &impl Mmio, b: &Bases, env: &impl Env) -> Result<PowerReport, Fail> {
    let mut rep = PowerReport::default();
    for (i, (r, s)) in ROOTS.iter().enumerate() {
        if ccm::set_root(m, b.ccm, *r, *s).map_err(Fail::Root)? {
            rep.roots_written |= 1 << i;
        }
    }
    rep.gate_opened = ccm::gate_on(m, b.ccm, GATE_HDMI).map_err(Fail::Gate)?;
    rep.hdmimix_was_down = gpc::power_up(m, b.gpc, env, &HDMIMIX).map_err(|f| Fail::Gpc(HDMIMIX.name, f))?.was_down;
    wr(m, b.blk, blk::RTX_CLK_CTL0_SET, CLK0_BUS);
    gpc::adb_wait_up(m, b.gpc, env, &HDMIMIX).map_err(|f| Fail::Gpc(HDMIMIX.name, f))?;
    rep.phy_was_down = gpc::power_up(m, b.gpc, env, &HDMI_PHY).map_err(|f| Fail::Gpc(HDMI_PHY.name, f))?.was_down;
    Ok(rep)
}

/// The block control: every clock this path needs, the clock selects, a
/// reset pulse on the blocks this driver owns, the PHY out of power-down,
/// the LCDIF NoC hurry level. Needs [`power_on`] first.
pub fn blk_enable(m: &impl Mmio, base: usize, env: &impl Env, sel: &ClockSel, noc_hurry: u32) -> Result<(), i32> {
    if noc_hurry > 7 {
        return Err(-EINVAL);
    }
    wr(m, base, blk::RTX_CLK_CTL0_SET, CLK0);
    wr(m, base, blk::RTX_CLK_CTL1_SET, CLK1);
    let mut c1 = rd(m, base, blk::RTX_CLK_CTL1);
    let c1_was = c1;
    if let Some(v) = sel.htx_pipe {
        c1 = (c1 & !blk::rtx_clk_ctl1::HTX_PIPE_CLK_SEL_MASK)
            | if v { blk::rtx_clk_ctl1::HTX_PIPE_CLK_SEL_MASK } else { 0 };
    }
    if let Some(v) = sel.htxphy {
        c1 =
            (c1 & !blk::rtx_clk_ctl1::HTXPHY_CLK_SEL_MASK) | if v { blk::rtx_clk_ctl1::HTXPHY_CLK_SEL_MASK } else { 0 };
    }
    if c1 != c1_was {
        wr(m, base, blk::RTX_CLK_CTL1, c1);
    }
    wr(m, base, blk::RTX_RESET_CTL0_CLR, RESETS);
    env.busy_wait_us(RESET_PULSE_US);
    wr(m, base, blk::RTX_RESET_CTL0_SET, RESETS);
    wr(m, base, blk::TX_CONTROL0_CLR, blk::tx_control0::TX_PHY_PDOWN_MASK);
    let t = rd(m, base, blk::TX_CONTROL0);
    let want = (t & !blk::tx_control0::LCDIF_NOC_HURRY_MASK) | field(blk::tx_control0::LCDIF_NOC_HURRY_MASK, noc_hurry);
    if want != t {
        wr(m, base, blk::TX_CONTROL0, want);
    }
    Ok(())
}

/// The reverse of [`blk_enable`] and [`power_on`]: PHY power-down pin,
/// resets asserted, the path's clocks off, then the PHY and HDMIMIX domains
/// down (HDMIMIX through its ADB400).
pub fn power_off(m: &impl Mmio, b: &Bases, env: &impl Env) -> Result<(), Fail> {
    wr(m, b.blk, blk::TX_CONTROL0_SET, blk::tx_control0::TX_PHY_PDOWN_MASK);
    wr(m, b.blk, blk::RTX_RESET_CTL0_CLR, RESETS);
    wr(m, b.blk, blk::RTX_CLK_CTL1_CLR, CLK1);
    wr(m, b.blk, blk::RTX_CLK_CTL0_CLR, CLK0 & !CLK0_BUS);
    gpc::power_down(m, b.gpc, env, &HDMI_PHY).map_err(|f| Fail::Gpc(HDMI_PHY.name, f))?;
    gpc::power_down(m, b.gpc, env, &HDMIMIX).map_err(|f| Fail::Gpc(HDMIMIX.name, f))?;
    Ok(())
}

/// HDMIMIX is up, mapped, its trigger clean and its ADB400 up: the block
/// control can be read. Reads the GPC only.
pub fn hdmimix_is_up(m: &impl Mmio, gpc_base: usize) -> bool {
    let s = gpc::state(m, gpc_base, &HDMIMIX);
    s.pup_clean && s.pcr_clear && !s.psr && s.adb_req && s.adb_ack
}

/// `TX_STATUS0`.
pub fn tx_status(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, blk::TX_STATUS0)
}

/// The block control holds what [`blk_enable`] sets.
pub fn blk_is_enabled(m: &impl Mmio, base: usize) -> bool {
    rd(m, base, blk::RTX_CLK_CTL0) & CLK0 == CLK0
        && rd(m, base, blk::RTX_CLK_CTL1) & CLK1 == CLK1
        && rd(m, base, blk::RTX_RESET_CTL0) & RESETS == RESETS
        && rd(m, base, blk::TX_CONTROL0) & blk::tx_control0::TX_PHY_PDOWN_MASK == 0
}

/// Pass LCDIF3's strobes through the PVI: LCDIF path, timing generator off,
/// inputs active high (as [`crate::ip::lcdifv3`] drives them for HDMI),
/// outputs with the mode's polarities. The other fields keep their reset
/// values.
pub fn pvi_configure(m: &impl Mmio, base: usize, mode: &VideoMode) {
    let own = pvi::htx_pvi_ctrl::HTX_PVI_EN_MASK
        | pvi::htx_pvi_ctrl::HTX_PVI_MODE_MASK
        | pvi::htx_pvi_ctrl::TMG_GEN_EN_MASK
        | pvi::htx_pvi_ctrl::INP_DE_POL_MASK
        | pvi::htx_pvi_ctrl::INP_HSYNC_POL_MASK
        | pvi::htx_pvi_ctrl::INP_VSYNC_POL_MASK
        | pvi::htx_pvi_ctrl::OP_DE_POL_MASK
        | pvi::htx_pvi_ctrl::OP_HSYNC_POL_MASK
        | pvi::htx_pvi_ctrl::OP_VSYNC_POL_MASK;
    let v = (PVI_CTRL_RESET & !own)
        | field(pvi::htx_pvi_ctrl::HTX_PVI_MODE_MASK, PVI_MODE_LCDIF)
        | pvi::htx_pvi_ctrl::INP_DE_POL_MASK
        | pvi::htx_pvi_ctrl::INP_HSYNC_POL_MASK
        | pvi::htx_pvi_ctrl::INP_VSYNC_POL_MASK
        | if mode.de_high { pvi::htx_pvi_ctrl::OP_DE_POL_MASK } else { 0 }
        | if mode.hsync_high { pvi::htx_pvi_ctrl::OP_HSYNC_POL_MASK } else { 0 }
        | if mode.vsync_high { pvi::htx_pvi_ctrl::OP_VSYNC_POL_MASK } else { 0 };
    // configured with the block off, then enabled
    wr(m, base, pvi::HTX_PVI_CTRL, v);
    wr(
        m,
        base,
        pvi::HTX_PVI_IRQ_CLR,
        pvi::htx_pvi_irq_clr::ASYNC_FIFO_OVRFLW_MASK | pvi::htx_pvi_irq_clr::ASYNC_FIFO_UNDRFLW_MASK,
    );
    wr(m, base, pvi::HTX_PVI_CTRL, v | pvi::htx_pvi_ctrl::HTX_PVI_EN_MASK);
}

/// The PVI's FIFO under/overflow status bits (`HTX_PVI_IRQ_STATUS` 1:0).
pub fn pvi_fifo_errors(m: &impl Mmio, base: usize) -> u32 {
    rd(m, base, pvi::HTX_PVI_IRQ_STATUS)
        & (pvi::htx_pvi_irq_status::ASYNC_FIFO_OVRFLW_MASK | pvi::htx_pvi_irq_status::ASYNC_FIFO_UNDRFLW_MASK)
}

/// The PVI is enabled on the LCDIF path.
pub fn pvi_is_on(m: &impl Mmio, base: usize) -> bool {
    let c = rd(m, base, pvi::HTX_PVI_CTRL);
    c & pvi::htx_pvi_ctrl::HTX_PVI_EN_MASK != 0
        && c & pvi::htx_pvi_ctrl::HTX_PVI_MODE_MASK == field(pvi::htx_pvi_ctrl::HTX_PVI_MODE_MASK, PVI_MODE_LCDIF)
}

/// The CCM root offsets this module writes (for the dump list and tests).
pub fn root_offsets() -> [u32; 3] {
    [ccmr::target_root(root::HDMI_APB), ccmr::target_root(root::HDMI_AXI), ccmr::target_root(root::HDMI_REF_266M)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TestEnv;
    use crate::model_hdmi::{HdmiFaults, HdmiModel, BLK, CCM, GPC, PVI};
    use crate::soc::imx8mp::regs::gpc_pgc;
    use crate::video::WUXGA_60_RB;

    fn bases(m: &HdmiModel) -> Bases {
        Bases { blk: m.base(BLK), pvi: m.base(PVI), gpc: m.base(GPC), ccm: m.base(CCM) }
    }

    #[test]
    fn layout_is_the_rm() {
        assert_eq!(root_offsets(), [0x8b00, 0x8b80, 0xbc00]);
        assert_eq!(ccmr::ccgr(GATE_HDMI), 0x45f0);
        assert_eq!((gpc_pgc::pu_ctrl(HDMIMIX.pgc), gpc_pgc::pu_ctrl(HDMI_PHY.pgc)), (0xe80, 0xec0));
        assert_eq!((HDMIMIX.trigger, HDMIMIX.a53_map), (1 << 14, 1 << 16));
        assert_eq!(HDMIMIX.adb, Some(Adb { req: 1 << 13, ack: 1 << 29 }));
        assert_eq!((HDMI_PHY.trigger, HDMI_PHY.a53_map), (1 << 15, 1 << 17));
        assert_eq!(
            (blk::RTX_RESET_CTL0, blk::RTX_CLK_CTL0, blk::RTX_CLK_CTL1, blk::TX_CONTROL0, blk::TX_STATUS0),
            (0x20, 0x40, 0x50, 0x200, 0x230)
        );
        assert_eq!((pvi::HTX_PVI_CTRL, pvi::HTX_PVI_IRQ_STATUS, pvi::HTX_TMG_GEN_CFG), (0, 8, 0x40));
        assert_eq!(RESETS, 0x0041_1c31);
    }

    #[test]
    fn power_on_sets_roots_before_the_gate_before_the_domains() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        let rep = power_on(&m, &b, &e).unwrap();
        assert_eq!(rep.roots_written, 0b111);
        assert!(rep.gate_opened && rep.hdmimix_was_down && rep.phy_was_down);
        assert_eq!(m.peek(CCM, 0x8b00), 0x1203_0000, "SYS_PLL1 / 4");
        assert_eq!(m.peek(CCM, 0x8b80), 0x1700_0000);
        assert_eq!(m.peek(CCM, 0xbc00), 0x1400_0000);
        let w = m.writes();
        let pos = |blkname, off| w.iter().position(|x| x.0 == blkname && x.1 == off).unwrap();
        assert!(pos(CCM, 0xbc00) < pos(CCM, 0x45f4));
        assert!(pos(CCM, 0x45f4) < pos(GPC, 0xd8));
        assert!(hdmimix_is_up(&m, b.gpc));
        assert_eq!(m.powered() & (3 << 14), 3 << 14);
    }

    #[test]
    fn blk_enable_pulses_the_resets_after_the_clocks() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        power_on(&m, &b, &e).unwrap();
        m.clear_log();
        blk_enable(&m, b.blk, &e, &ClockSel::default(), 7).unwrap();
        let w = m.writes();
        let pos = |off| w.iter().position(|x| x.0 == BLK && x.1 == off).unwrap();
        assert!(pos(0x44) < pos(0x28) && pos(0x54) < pos(0x28), "clocks before the reset pulse: {w:?}");
        assert!(pos(0x28) < pos(0x24), "assert, then release");
        assert!(blk_is_enabled(&m, b.blk));
        assert_eq!(m.peek(BLK, 0x200) & 0x7008, 0x7000, "PDOWN low, hurry 7");
        // clock selects left alone by default
        assert!(!w.iter().any(|x| x.0 == BLK && x.1 == 0x50));
    }

    #[test]
    fn clock_selects_are_written_only_when_given() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        power_on(&m, &b, &e).unwrap();
        blk_enable(&m, b.blk, &e, &ClockSel { htx_pipe: Some(true), htxphy: Some(false) }, 0).unwrap();
        assert_eq!(m.peek(BLK, 0x50) & 0x1400, 0x1000);
        assert_eq!(blk_enable(&m, b.blk, &e, &ClockSel::default(), 8), Err(-EINVAL));
    }

    #[test]
    fn no_adb_ack_stops_before_the_phy_domain() {
        let m = HdmiModel::with_faults(HdmiFaults { adb_no_ack: true, ..Default::default() });
        let e = TestEnv::default();
        let b = bases(&m);
        let f = power_on(&m, &b, &e).unwrap_err();
        assert_eq!(f, Fail::Gpc("hdmimix", gpc::Fail::AdbTimeout));
        assert_eq!(m.powered() & HDMI_PHY.trigger, 0);
    }

    #[test]
    fn power_off_then_on_again() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        power_on(&m, &b, &e).unwrap();
        blk_enable(&m, b.blk, &e, &ClockSel::default(), 0).unwrap();
        power_off(&m, &b, &e).unwrap();
        assert_eq!(m.powered() & (3 << 14), 0);
        let rep = power_on(&m, &b, &e).unwrap();
        assert!(rep.hdmimix_was_down && rep.roots_written == 0 && !rep.gate_opened);
        assert!(!blk_is_enabled(&m, b.blk), "the block control lost its state with the domain");
    }

    #[test]
    fn pvi_takes_the_lcdif_path_with_the_mode_polarities() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        power_on(&m, &b, &e).unwrap();
        blk_enable(&m, b.blk, &e, &ClockSel::default(), 0).unwrap();
        pvi_configure(&m, b.pvi, &WUXGA_60_RB);
        // reset 0x377000 with OP_VSYNC low (-vsync), mode 2, enabled
        assert_eq!(m.peek(PVI, 0), 0x0033_7005);
        assert!(pvi_is_on(&m, b.pvi));
        assert_eq!(pvi_fifo_errors(&m, b.pvi), 0);
    }
}
