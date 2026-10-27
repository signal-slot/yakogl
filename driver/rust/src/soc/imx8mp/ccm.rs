// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Clock roots and clock gates of the i.MX 8M Plus CCM.
//!
//! Register layout and fields: [`super::regs::ccm`] (hal_nxp `PERI_CCM.h`).
//! Root and gate indices of a peripheral come from the devicetree or from
//! the RM's tables; for the PWMs they are also hal_nxp `fsl_clock.h`'s
//! (MIMX8ML8): `kCLOCK_RootPwm3` is `ROOT[105]`, `kCLOCK_Pwm3 =
//! CCM_TUPLE(42U, 105U)` (gate `CCGR[42]`).
//!
//! Facts not in those headers, each with its citation in the i.MX 8M Plus
//! Applications Processor Reference Manual, Rev. 3, 08/2024 (IMX8MPRM
//! Rev. 3; document used, not copied):
//! - The 24 MHz oscillator runs at 24,000,000 Hz on this SoC (the header only
//!   names it `kCLOCK_Osc24MClk`). IMX8MPRM Rev. 3 §5.1.5.1 "Input Clocks":
//!   `24M_REF_CLK`, 24 (MHz), "24M oscillator clock output" (and mux value
//!   000 of the PWM roots in §5.1.2 "Clock Root Selects", Table 5-1). The
//!   RM gives the value in MHz only; the exact 24,000,000 Hz is the nominal
//!   crystal value.
//! - A root's rate is its mux input / (PRE_PODF + 1) / (POST_PODF + 1), from
//!   the "Divide by n" value lists of `PERI_CCM.h`. IMX8MPRM Rev. 3 §5.1.6.4
//!   "Target Interface": "Freq = (clock source freq)/(pre_div+1)/(post_div+1)";
//!   §5.1.5.3.1 "Clock Divider" ("Dividers perform a 1/(N+1) divide");
//!   §5.1.7.10 "Target Register (CCM_TARGET_ROOTn)", PRE_PODF and POST_PODF
//!   ("Divider value is n+1"). Note the RM disagrees with itself on the
//!   width of the peripheral post divider: §5.1.5.4.3 "Peripheral clock
//!   slice" says it is six bit, while the POST_PODF field text says "For
//!   Peripheral (IP), this field is 1 bit long". The PWM driver only writes
//!   0; the media pixel root (a peripheral slice) is run at POST_PODF 14 by
//!   the board's Linux (golden dump 0x3038be00 = 0x1100000e) and by
//!   soc::imx8mp::media, so the six-bit reading is the one the board bears out.
//!   Slice indices, offsets, maximum rates and mux inputs: §5.1.2 Table 5-1
//!   "Clock Root Table".
//! - `CCGR.SETTING0` is the gate's request for domain 0: IMX8MPRM Rev. 3
//!   §5.1.7.6 "CCM Clock Gating Register (CCM_CCGRn)", SETTING0 ("can only
//!   be written by domain 0"; 11 = "Domain clocks needed all the time"),
//!   and §5.1.6.3 "CCGR Interface". That the Cortex-A53 cluster is domain 0:
//!   §3.2.5.5 "Master Domain Assignment (RDC_MDAn)" and §3.2.4.1 "Master
//!   Assignment Registers" (Table 3-2: A53 platform = `RDC_MDA0`); `RDC_MDA0.DID`
//!   resets to 0 (domain 0). It is an RDC setting that firmware may
//!   change, not a fixed property of the SoC (§5.1.6.3: "Any CPU platform can
//!   be assigned to any domain by RDC").
//! - Rewriting MUX and the dividers of an enabled root is safe for the
//!   consumer (no runt pulse that could upset it). IMX8MPRM Rev. 3 §5.1.6.4
//!   "Target Interface": the interface's internal sequence "guarantees a
//!   clean clock on output without frequency overshoot" (it opens the gates,
//!   applies the dividers, switches the source, and a write "completes once
//!   the output clock is running at the desired setting"), provided the
//!   target source is active, which the 24 MHz oscillator is. The raw
//!   8-to-1 mux is not glitch-free and "Peripheral clock slices must be
//!   stopped to change the clock source" (§5.1.5.4.3), which the Target
//!   Interface does by itself; this module only ever writes `TARGET_ROOTn`,
//!   never the Normal Interface registers. The callers of [`set_root`] check
//!   that the target source runs (soc::imx8mp::anatop::running_rate).
//! - A root must be enabled for the LPCG it feeds to gate it actively
//!   (§5.1.5.5 "Clock gate control": "The ENABLE bit must be set for the clock
//!   root that LPCG is actively gating"): roots are set before their gates.
//! - `CCGR_SET` changes only the bits written as 1: IMX8MPRM Rev. 3 §5.1.6.1
//!   "Set, Clear, and Toggle register features" ("Writing 1 to the set
//!   register bits sets them to 1, while writing 0 has no effect") and
//!   §5.1.7.7 "CCM Clock Gating Register (CCM_CCGRn_SET)".

use super::regs::ccm;
use crate::errno::EINVAL;
use crate::mmio::Mmio;

/// The 24 MHz oscillator (IMX8MPRM Rev. 3 §5.1.5.1, see the module text).
pub const OSC_24M_HZ: u32 = 24_000_000;

/// `CCGR.SETTINGn`: "Domain clocks needed all the time".
const SETTING_ALWAYS: u32 = 0b11;

/// The fields of `TARGET_ROOT` this module owns.
const ROOT_FIELDS: u32 = ccm::target_root::ENABLE_MASK
    | ccm::target_root::MUX_MASK
    | ccm::target_root::PRE_PODF_MASK
    | ccm::target_root::POST_PODF_MASK;

/// What [`root_on_osc24m`] did, for the caller's log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootChange {
    /// `TARGET_ROOT` before.
    pub was: u32,
    /// `TARGET_ROOT` was rewritten.
    pub rewrote_root: bool,
    /// The domain-0 gate was opened.
    pub opened_gate: bool,
}

/// Run clock root `root` from the 24 MHz oscillator undivided, enabled, and
/// open clock gate `gate` for domain 0. Returns the root's rate.
///
/// Writes only what differs: a root already on the oscillator at /1 /1 is
/// left alone, a gate already open is not written. The root is configured
/// before its gate is opened, so the consumer never sees a clock from the
/// previous mux setting through a gate this call opened.
pub fn root_on_osc24m(m: &impl Mmio, base: usize, root: u32, gate: u32) -> Result<(u32, RootChange), i32> {
    if root >= ccm::ROOT_COUNT || gate >= ccm::CCGR_COUNT {
        return Err(-EINVAL);
    }
    let troot = base + ccm::target_root(root) as usize;
    let was = m.read32(troot);
    // mux 0 (OSC_24M), PRE_PODF 0 and POST_PODF 0 (divide by 1), ENABLE 1
    let want = ccm::target_root::ENABLE_MASK;
    let rewrote_root = was & ROOT_FIELDS != want;
    if rewrote_root {
        m.write32(troot, (was & !ROOT_FIELDS) | want);
    }
    let setting0 = (m.read32(base + ccm::ccgr(gate) as usize) & ccm::ccgr::SETTING0_MASK) >> ccm::ccgr::SETTING0_SHIFT;
    let opened_gate = setting0 != SETTING_ALWAYS;
    if opened_gate {
        // CCGR_SET: only the bits written as 1 change.
        m.write32(
            base + ccm::ccgr_set(gate) as usize,
            (SETTING_ALWAYS << ccm::ccgr_set::SETTING0_SHIFT) & ccm::ccgr_set::SETTING0_MASK,
        );
    }
    Ok((OSC_24M_HZ, RootChange { was, rewrote_root, opened_gate }))
}

/// The rate of `root` when it runs from the 24 MHz oscillator; `None` when it
/// is off or fed from anything else (a PLL this code does not track).
pub fn root_rate_if_osc24m(m: &impl Mmio, base: usize, root: u32) -> Option<u32> {
    if root >= ccm::ROOT_COUNT {
        return None;
    }
    let v = m.read32(base + ccm::target_root(root) as usize);
    let mux = (v & ccm::target_root::MUX_MASK) >> ccm::target_root::MUX_SHIFT;
    if v & ccm::target_root::ENABLE_MASK == 0 || mux != 0 {
        return None;
    }
    let pre = ((v & ccm::target_root::PRE_PODF_MASK) >> ccm::target_root::PRE_PODF_SHIFT) + 1;
    let post = ((v & ccm::target_root::POST_PODF_MASK) >> ccm::target_root::POST_PODF_SHIFT) + 1;
    Some(OSC_24M_HZ / pre / post)
}

/// Clock root slices (RM Table 5-1).
pub mod root {
    /// MEDIA_AXI_CLK_ROOT, offset 0x8A00, at most 500 MHz.
    pub const MEDIA_AXI: u32 = 20;
    /// MEDIA_APB_CLK_ROOT, offset 0x8A80, at most 200 MHz.
    pub const MEDIA_APB: u32 = 21;
    /// I2C2_CLK_ROOT, offset 0xAD80.
    pub const I2C2: u32 = 91;
    /// PWM3_CLK_ROOT, offset 0xB480.
    pub const PWM3: u32 = 105;
    /// WDOG_CLK_ROOT, offset 0xB900 (hal_nxp `fsl_clock.h` MIMX8ML8:
    /// `kCLOCK_RootWdog` is `ROOT[114]`).
    pub const WDOG: u32 = 114;
    /// MEDIA_MIPI_PHY1_REF_CLK_ROOT, offset 0xBD80, at most 125 MHz.
    pub const MEDIA_MIPI_PHY1_REF: u32 = 123;
    /// MEDIA_DISP1_PIX_CLK_ROOT, offset 0xBE00, at most 250 MHz.
    pub const MEDIA_DISP1_PIX: u32 = 124;

    pub const MEDIA_AXI_MAX_HZ: u32 = 500_000_000;
    pub const MEDIA_APB_MAX_HZ: u32 = 200_000_000;
    pub const MEDIA_MIPI_PHY1_REF_MAX_HZ: u32 = 125_000_000;
    pub const MEDIA_DISP1_PIX_MAX_HZ: u32 = 250_000_000;
}

/// Mux inputs (`TARGET_ROOT.MUX`) used here, per root (RM Table 5-1).
pub mod mux {
    /// Every root: 000 = 24M_REF_CLK.
    pub const OSC_24M: u32 = 0;
    /// MEDIA_AXI: 001 = SYSTEM_PLL2_CLK.
    pub const MEDIA_AXI_SYS_PLL2: u32 = 1;
    /// MEDIA_APB: 010 = SYSTEM_PLL1_CLK.
    pub const MEDIA_APB_SYS_PLL1: u32 = 2;
    /// MEDIA_DISP1_PIX: 001 = VIDEO_PLL_CLK, 100 = SYSTEM_PLL1_CLK,
    /// 101 = SYSTEM_PLL2_CLK.
    pub const DISP1_PIX_VIDEO_PLL1: u32 = 1;
    pub const DISP1_PIX_SYS_PLL1: u32 = 4;
    pub const DISP1_PIX_SYS_PLL2: u32 = 5;
}

/// Clock gates (RM 5.1.6.3 Table 5-9 "CCGR Mapping Table").
pub mod gate {
    /// CCM_CCGR14, GPIO4, 0x40E0.
    pub const GPIO4: u32 = 14;
    /// CCM_CCGR24, I2C2, 0x4180.
    pub const I2C2: u32 = 24;
    /// CCM_CCGR42, PWM3, 0x42A0.
    pub const PWM3: u32 = 42;
    /// CCM_CCGR83, WDOG1, 0x4530 (hal_nxp `fsl_clock.h` MIMX8ML8:
    /// `kCLOCK_Wdog1 = CCM_TUPLE(83U, 114U)`).
    pub const WDOG1: u32 = 83;
    /// CCM_CCGR93, MEDIA, 0x45D0: the gate of every MEDIAMIX clock root
    /// (RM 5.1.4 "System Clocks": `ccm_clk_enable_media (CCGR93)`).
    pub const MEDIA: u32 = 93;
}

/// The fields of a `TARGET_ROOT`, dividers as divide-by values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootSetting {
    pub enabled: bool,
    pub mux: u32,
    /// 1..=8
    pub pre: u32,
    /// 1..=64
    pub post: u32,
}

impl RootSetting {
    pub const fn on(mux: u32, pre: u32, post: u32) -> RootSetting {
        RootSetting { enabled: true, mux, pre, post }
    }

    fn encode(self) -> u32 {
        (if self.enabled { ccm::target_root::ENABLE_MASK } else { 0 })
            | ((self.mux << ccm::target_root::MUX_SHIFT) & ccm::target_root::MUX_MASK)
            | (((self.pre - 1) << ccm::target_root::PRE_PODF_SHIFT) & ccm::target_root::PRE_PODF_MASK)
            | (((self.post - 1) << ccm::target_root::POST_PODF_SHIFT) & ccm::target_root::POST_PODF_MASK)
    }

    fn decode(v: u32) -> RootSetting {
        RootSetting {
            enabled: v & ccm::target_root::ENABLE_MASK != 0,
            mux: (v & ccm::target_root::MUX_MASK) >> ccm::target_root::MUX_SHIFT,
            pre: ((v & ccm::target_root::PRE_PODF_MASK) >> ccm::target_root::PRE_PODF_SHIFT) + 1,
            post: ((v & ccm::target_root::POST_PODF_MASK) >> ccm::target_root::POST_PODF_SHIFT) + 1,
        }
    }
}

/// Read a root's fields.
pub fn root_setting(m: &impl Mmio, base: usize, root: u32) -> RootSetting {
    RootSetting::decode(m.read32(base + ccm::target_root(root) as usize))
}

/// Set a root through the Target Interface (RM 5.1.6.4); writes only when a
/// field differs and keeps the bits it does not own. Returns whether it
/// wrote.
pub fn set_root(m: &impl Mmio, base: usize, root: u32, want: RootSetting) -> Result<bool, i32> {
    if root >= ccm::ROOT_COUNT || !(1..=8).contains(&want.pre) || !(1..=64).contains(&want.post) || want.mux > 7 {
        return Err(-EINVAL);
    }
    let addr = base + ccm::target_root(root) as usize;
    let was = m.read32(addr);
    let v = want.encode();
    if was & ROOT_FIELDS == v {
        return Ok(false);
    }
    m.write32(addr, (was & !ROOT_FIELDS) | v);
    Ok(true)
}

/// Set a root's ENABLE and nothing else, through its SET alias (RM 5.1.6.1);
/// a root already enabled is not written. Returns whether it wrote.
pub fn enable_root(m: &impl Mmio, base: usize, root: u32) -> Result<bool, i32> {
    if root >= ccm::ROOT_COUNT {
        return Err(-EINVAL);
    }
    if m.read32(base + ccm::target_root(root) as usize) & ccm::target_root::ENABLE_MASK != 0 {
        return Ok(false);
    }
    m.write32(base + ccm::target_root_set(root) as usize, ccm::target_root_set::ENABLE_MASK);
    Ok(true)
}

/// Domain 0's setting of a gate is 3 (needed in every mode).
pub fn gate_is_on(m: &impl Mmio, base: usize, gate: u32) -> bool {
    gate < ccm::CCGR_COUNT
        && (m.read32(base + ccm::ccgr(gate) as usize) & ccm::ccgr::SETTING0_MASK) >> ccm::ccgr::SETTING0_SHIFT
            == SETTING_ALWAYS
}

/// Domain 0's setting of a gate is not 0: the clock runs while the A53
/// domain is in RUN mode (RM 5.1.7.x CCGR: 1 RUN, 2 RUN and WAIT, 3 always;
/// the reset value is 2).
pub fn gate_runs(m: &impl Mmio, base: usize, gate: u32) -> bool {
    gate < ccm::CCGR_COUNT
        && (m.read32(base + ccm::ccgr(gate) as usize) & ccm::ccgr::SETTING0_MASK) >> ccm::ccgr::SETTING0_SHIFT != 0
}

/// Open a gate for domain 0 through its SET alias; an open gate is not
/// written. Returns whether it wrote.
pub fn gate_on(m: &impl Mmio, base: usize, gate: u32) -> Result<bool, i32> {
    if gate >= ccm::CCGR_COUNT {
        return Err(-EINVAL);
    }
    if gate_is_on(m, base, gate) {
        return Ok(false);
    }
    m.write32(
        base + ccm::ccgr_set(gate) as usize,
        (SETTING_ALWAYS << ccm::ccgr_set::SETTING0_SHIFT) & ccm::ccgr_set::SETTING0_MASK,
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, CCM};

    #[test]
    fn layout_is_the_headers() {
        // kCLOCK_RootPwm3 = ROOT[105].TARGET_ROOT, kCLOCK_Pwm3 = CCGR[42]
        assert_eq!(ccm::target_root(105), 0xb480);
        assert_eq!(ccm::ccgr(42), 0x42a0);
        assert_eq!(ccm::ccgr_set(42), 0x42a4);
        // kCLOCK_RootWdog = ROOT[114], kCLOCK_Wdog1 = CCGR[83]
        assert_eq!(ccm::target_root(root::WDOG), 0xb900);
        assert_eq!(ccm::ccgr(gate::WDOG1), 0x4530);
    }

    #[test]
    fn a_root_on_a_pll_is_moved_to_the_oscillator_then_the_gate_opens() {
        let m = Model::new();
        // mux 2 (SYS_PLL1/5), pre /2, post /3, enabled; gate closed
        m.poke(CCM, 0xb480, 0x1201_0002);
        let (hz, ch) = root_on_osc24m(&m, m.base(CCM), 105, 42).unwrap();
        assert_eq!(hz, 24_000_000);
        assert!(ch.rewrote_root && ch.opened_gate);
        assert_eq!(ch.was, 0x1201_0002);
        assert_eq!(m.writes(), vec![(CCM, 0xb480, 0x1000_0000), (CCM, 0x42a4, 0x3)]);
        assert_eq!(root_rate_if_osc24m(&m, m.base(CCM), 105), Some(24_000_000));
    }

    #[test]
    fn nothing_is_written_when_root_and_gate_are_already_right() {
        let m = Model::new();
        m.poke(CCM, 0xb480, 0x1000_0000);
        m.poke(CCM, 0x42a0, 0x3);
        let (_, ch) = root_on_osc24m(&m, m.base(CCM), 105, 42).unwrap();
        assert!(!ch.rewrote_root && !ch.opened_gate);
        assert!(m.writes().is_empty());
    }

    #[test]
    fn reserved_bits_of_the_root_are_kept() {
        let m = Model::new();
        m.poke(CCM, 0xb480, 0x8000_0000 | 0x0700_0000);
        root_on_osc24m(&m, m.base(CCM), 105, 42).unwrap();
        assert_eq!(m.peek(CCM, 0xb480), 0x9000_0000);
    }

    #[test]
    fn out_of_range_indices_are_refused_without_access() {
        let m = Model::new();
        assert_eq!(root_on_osc24m(&m, m.base(CCM), 142, 42), Err(-EINVAL));
        assert_eq!(root_on_osc24m(&m, m.base(CCM), 105, 192), Err(-EINVAL));
        assert!(m.accesses().is_empty());
    }

    #[test]
    fn rate_reads_the_dividers_and_refuses_other_parents() {
        let m = Model::new();
        m.poke(CCM, 0xb480, 0x1001_0002); // osc /2 /3
        assert_eq!(root_rate_if_osc24m(&m, m.base(CCM), 105), Some(4_000_000));
        m.poke(CCM, 0xb480, 0x1700_0000); // VIDEO_PLL1
        assert_eq!(root_rate_if_osc24m(&m, m.base(CCM), 105), None);
        m.poke(CCM, 0xb480, 0x0000_0000); // off
        assert_eq!(root_rate_if_osc24m(&m, m.base(CCM), 105), None);
    }

    #[test]
    fn the_rm_tables() {
        // Table 5-1 offsets and Table 5-9 gate offsets
        assert_eq!(ccm::target_root(root::MEDIA_AXI), 0x8a00);
        assert_eq!(ccm::target_root(root::MEDIA_APB), 0x8a80);
        assert_eq!(ccm::target_root(root::I2C2), 0xad80);
        assert_eq!(ccm::target_root(root::MEDIA_MIPI_PHY1_REF), 0xbd80);
        assert_eq!(ccm::target_root(root::MEDIA_DISP1_PIX), 0xbe00);
        assert_eq!(ccm::ccgr(gate::MEDIA), 0x45d0);
        assert_eq!(ccm::ccgr(gate::I2C2), 0x4180);
        assert_eq!(ccm::ccgr(gate::GPIO4), 0x40e0);
    }

    #[test]
    fn set_root_encodes_writes_once_and_keeps_reserved_bits() {
        let m = Model::new();
        m.poke(CCM, 0x8a00, 0x8000_0000);
        let want = RootSetting::on(mux::MEDIA_AXI_SYS_PLL2, 1, 2);
        assert_eq!(set_root(&m, m.base(CCM), root::MEDIA_AXI, want), Ok(true));
        assert_eq!(m.peek(CCM, 0x8a00), 0x9100_0001);
        assert_eq!(root_setting(&m, m.base(CCM), root::MEDIA_AXI), want);
        m.clear_log();
        assert_eq!(set_root(&m, m.base(CCM), root::MEDIA_AXI, want), Ok(false));
        assert!(m.writes().is_empty());
        assert_eq!(set_root(&m, m.base(CCM), root::MEDIA_AXI, RootSetting::on(0, 9, 1)), Err(-EINVAL));
        assert_eq!(set_root(&m, m.base(CCM), root::MEDIA_AXI, RootSetting::on(0, 1, 65)), Err(-EINVAL));
    }

    #[test]
    fn enable_root_and_gate_on_use_the_set_aliases() {
        let m = Model::new();
        m.poke(CCM, 0xad80, 0x0100_0003); // I2C2 disabled with a mux and divider
        assert_eq!(enable_root(&m, m.base(CCM), root::I2C2), Ok(true));
        assert_eq!(m.peek(CCM, 0xad80), 0x1100_0003, "only ENABLE changed");
        assert_eq!(gate_on(&m, m.base(CCM), gate::I2C2), Ok(true));
        assert_eq!(m.writes(), vec![(CCM, 0xad84, 0x1000_0000), (CCM, 0x4184, 0x3)]);
        assert!(gate_is_on(&m, m.base(CCM), gate::I2C2));
        m.clear_log();
        assert_eq!(enable_root(&m, m.base(CCM), root::I2C2), Ok(false));
        assert_eq!(gate_on(&m, m.base(CCM), gate::I2C2), Ok(false));
        assert!(m.writes().is_empty());
    }
}
