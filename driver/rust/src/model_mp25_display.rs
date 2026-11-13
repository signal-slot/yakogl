// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' stateful model of the STM32MP25's LTDC (0x4801_0000) and
//! DSI (0x4800_0000: the DesignWare host in 0x000-0x3ff, ST's wrapper from
//! 0x400) (tests only).
//!
//! Like [`crate::model_mp25_i2c`] it is an [`Mmio`] that keeps register state
//! and answers only what the drivers poll, as far as ST's Linux v6.6-stm32mp
//! (ltdc.c, dw_mipi_dsi-stm.c) and dw-mipi-dsi.c say the silicon does. Where
//! the silicon would not take an access the model does not panic: it records
//! it in [`Mp25Display::violations`], so a test can assert there are none and
//! another can assert that one is caught. Offsets and bit positions come from
//! the IP modules' public constants (the modules' own tests pin them to the
//! Linux sources); the LTDC layer offsets are restated here.
//!
//! **The ordering rules** (violations), with the MP25's 1.41 wrapper as
//! `ip::stm32_dsi_phy` documents it. A regulator enable and a PLL lock bit
//! (`WISR`) belong to the MP1's wrapper (`WRPCR` at 0x430). On 1.41 the
//! supplies are the platform's and the PLL has no lock bit of its own, so the
//! rules are the ones ST's code has:
//! - the PLL dividers (`WPCR1`, `WRPCR0`, `WRPCR1`) are shadowed and take
//!   effect on a rising `WRPCR2.UPD`; a `WRPCR2.CLR` returns them to reset;
//! - the D-PHY leaves reset (`PHY_RSTZ` `UNSHUTDOWNZ | UNRSTZ`, the wrapper's
//!   `PCTLR.PWEN | DEN`) only with the configuration committed;
//! - `WRPCR2.PLLEN` rises only with the PHY out of reset (Linux waits for
//!   `PSR.PSSC` first) and the configuration committed;
//! - the host's `PHY_RSTZ.ENFORCEPLL` (dw-mipi-dsi.c `dphy_enable`) only with
//!   `PLLEN` set;
//! - `MODE_CFG` video mode only with the PLL locked (`PLLEN` and the PHY out
//!   of reset);
//! - the host's timing and DPI registers are written with `PWR_UP` low (Linux
//!   writes `RESET` first, `dw_mipi_dsi_init`).
//!
//! The LTDC layer registers (0x100-0x13f, `ltdc_layer_regs_a2`) are shadowed:
//! a write lands in the shadow (reads give it back), the active copy is taken
//! over on a reload, `L1RCR` (0x108) or `SRCR` (0x24): `IMR` at once, `VBR`
//! at the next frame, which the model lets come on the first read of the
//! reload register once `GCR.LTDCEN` is set. Violations: a layer enable
//! (`LXCR.LEN`) with a zero `CFBAR` or a zero pitch in `CFBLR`, whether in the
//! shadow when the register is written or in the copy a reload takes over;
//! the timing registers (`SSCR`..`TWCR`) written with `LTDCEN` high.
//!
//! Status as the block shows it: `LTDC_IDR` 0x040101, `LCR` 2 layers, `GC2R`
//! a 64-bit bus; `ISR.LIF` set while `LTDCEN`; the DSI `VERSION` 1.41;
//! `PHY_STATUS` `PHY_LOCK` and the clock lane's stop state follow the rules
//! above. Any other offset in the two blocks reads and writes as plain
//! storage; an access outside them, or a byte access, panics.

use crate::ip::dw_mipi_dsi::regs as h;
use crate::ip::stm32_dsi_phy::regs as w;
use crate::ip::stm32_ltdc as l;
use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The LTDC (stm32mp251.dtsi) and the DSI (stm32mp255.dtsi), 0x400 and 0x800
/// bytes.
pub const LTDC_BASE: usize = 0x4801_0000;
pub const DSI_BASE: usize = 0x4800_0000;
const LTDC_LEN: usize = 0x400;
const DSI_LEN: usize = 0x800;

const LAYER_FIRST: u32 = 0x100;
const LAYER_END: u32 = 0x140;
const L1RCR: u32 = 0x108;
const L1CR: u32 = 0x10c;
const L1CFBAR: u32 = 0x134;
const L1CFBLR: u32 = 0x138;
const DSI_VERSION_VALUE: u32 = w::HWVER_141 | 0x2a;

/// Host registers that Linux writes only with the host in reset.
const HOST_CONFIG: [u32; 18] = [
    h::CLKMGR_CFG,
    h::DPI_VCID,
    h::DPI_COLOR_CODING,
    h::DPI_CFG_POL,
    h::PCKHDL_CFG,
    h::VID_PKT_SIZE,
    h::VID_NUM_CHUNKS,
    h::VID_NULL_SIZE,
    h::VID_HSA_TIME,
    h::VID_HBP_TIME,
    h::VID_HLINE_TIME,
    h::VID_VSA_LINES,
    h::VID_VBP_LINES,
    h::VID_VFP_LINES,
    h::VID_VACTIVE_LINES,
    h::PHY_IF_CFG,
    h::PHY_TMR_CFG,
    h::PHY_TMR_LPCLK_CFG,
];

#[derive(Default)]
struct State {
    dsi: BTreeMap<u32, u32>,
    /// `WPCR1`, `WRPCR0`, `WRPCR1` as the PLL runs them (after `UPD`).
    pll_active: Option<[u32; 3]>,
    ltdc: BTreeMap<u32, u32>,
    /// The layer registers the pixel pipeline uses (taken at a reload).
    layer_active: BTreeMap<u32, u32>,
    reload_pending: bool,
    violations: Vec<String>,
}

/// The model; see the module text. Pass `&model` as the [`Mmio`].
#[derive(Default)]
pub struct Mp25Display {
    s: RefCell<State>,
}

impl State {
    fn violate(&mut self, what: String) {
        self.violations.push(what);
    }

    fn dsi_get(&self, off: u32) -> u32 {
        self.dsi.get(&off).copied().unwrap_or(0)
    }

    fn ltdc_get(&self, off: u32) -> u32 {
        self.ltdc.get(&off).copied().unwrap_or(0)
    }

    fn phy_out_of_reset(&self) -> bool {
        let rstz = self.dsi_get(h::PHY_RSTZ);
        rstz & (h::PHY_UNRSTZ | h::PHY_UNSHUTDOWNZ) == (h::PHY_UNRSTZ | h::PHY_UNSHUTDOWNZ)
    }

    fn pll_enabled(&self) -> bool {
        self.dsi_get(w::WRPCR2) & w::WRPCR2_PLLEN != 0
    }

    fn dsi_read(&self, off: u32) -> u32 {
        match off {
            h::VERSION => DSI_VERSION_VALUE,
            h::PHY_STATUS => {
                let mut v = self.dsi_get(off);
                let up = self.phy_out_of_reset() && self.pll_active.is_some();
                if up && self.dsi_get(h::PHY_RSTZ) & h::PHY_ENABLECLK != 0 {
                    v |= h::PHY_STOP_STATE_CLK_LANE;
                }
                if up && self.pll_enabled() {
                    v |= h::PHY_LOCK;
                }
                v
            }
            _ => self.dsi_get(off),
        }
    }

    fn dsi_write(&mut self, off: u32, v: u32) {
        let old = self.dsi_get(off);
        match off {
            h::VERSION | h::PHY_STATUS => {
                self.violate(format!("DSI: write to read-only {off:#x}"));
                return;
            }
            h::PHY_RSTZ => {
                let was = self.phy_out_of_reset();
                self.dsi.insert(off, v);
                if self.phy_out_of_reset() && !was && self.pll_active.is_none() {
                    self.violate(
                        "DSI: D-PHY left reset before the PLL configuration was committed (WRPCR2.UPD)".into(),
                    );
                }
                if v & h::PHY_ENFORCEPLL != 0 && !self.pll_enabled() {
                    self.violate("DSI: PHY_RSTZ.ENFORCEPLL with WRPCR2.PLLEN clear".into());
                }
                return;
            }
            h::MODE_CFG if v & h::ENABLE_CMD_MODE == 0 => {
                if !(self.pll_enabled() && self.phy_out_of_reset()) {
                    self.violate("DSI: video mode with the PLL not locked".into());
                }
            }
            w::WRPCR2 => {
                if v & w::WRPCR2_CLR != 0 && old & w::WRPCR2_CLR == 0 {
                    for o in [w::WPCR1, w::WRPCR0, w::WRPCR1] {
                        self.dsi.insert(o, 0);
                    }
                    self.pll_active = None;
                }
                if v & w::WRPCR2_UPD != 0 && old & w::WRPCR2_UPD == 0 {
                    let shadow = [self.dsi_get(w::WPCR1), self.dsi_get(w::WRPCR0), self.dsi_get(w::WRPCR1)];
                    if shadow == [0; 3] {
                        self.violate("DSI: WRPCR2.UPD with the PLL registers still at reset".into());
                    }
                    self.pll_active = Some(shadow);
                }
                if v & w::WRPCR2_PLLEN != 0 && old & w::WRPCR2_PLLEN == 0 {
                    if !self.phy_out_of_reset() {
                        self.violate("DSI: WRPCR2.PLLEN while the D-PHY is still in reset".into());
                    }
                    if self.pll_active.is_none() {
                        self.violate("DSI: WRPCR2.PLLEN before the PLL configuration was committed (UPD)".into());
                    }
                }
            }
            _ => {}
        }
        if HOST_CONFIG.contains(&off) && self.dsi_get(h::PWR_UP) & h::POWERUP != 0 {
            self.violate(format!("DSI: host register {off:#x} written with PWR_UP high"));
        }
        self.dsi.insert(off, v);
    }

    fn layer_enable_check(&mut self, ctx: &str, regs: &BTreeMap<u32, u32>) {
        let get = |o| regs.get(&o).copied().unwrap_or(0);
        if get(L1CR) & l::LXCR_LEN == 0 {
            return;
        }
        if get(L1CFBAR) == 0 {
            self.violate(format!("LTDC: layer enabled with CFBAR 0 ({ctx})"));
        }
        if get(L1CFBLR) >> 16 == 0 {
            self.violate(format!("LTDC: layer enabled with a zero pitch ({ctx})"));
        }
    }

    fn take_reload(&mut self) {
        let shadow: BTreeMap<u32, u32> = self.ltdc.range(LAYER_FIRST..LAYER_END).map(|(k, v)| (*k, *v)).collect();
        self.layer_enable_check("reload", &shadow);
        self.layer_active = shadow;
        self.reload_pending = false;
        self.ltdc.remove(&L1RCR);
        self.ltdc.remove(&l::SRCR);
    }

    fn ltdc_read(&mut self, off: u32) -> u32 {
        match off {
            l::IDR => l::HWVER_40101,
            l::LCR => 2,
            l::GC2R => 3 << 4,
            l::ISR => {
                if self.ltdc_get(l::GCR) & l::GCR_LTDCEN != 0 {
                    l::INT_LINE
                } else {
                    0
                }
            }
            L1RCR | l::SRCR => {
                // the next frame boundary, once the block runs
                if self.reload_pending && self.ltdc_get(l::GCR) & l::GCR_LTDCEN != 0 {
                    self.take_reload();
                }
                self.ltdc_get(off)
            }
            _ => self.ltdc_get(off),
        }
    }

    fn ltdc_write(&mut self, off: u32, v: u32) {
        match off {
            l::IDR | l::LCR | l::GC2R | l::ISR => {
                self.violate(format!("LTDC: write to read-only {off:#x}"));
            }
            l::ICR => {}
            L1RCR | l::SRCR => {
                if v & l::RCR_IMR != 0 {
                    self.take_reload();
                } else if v & l::RCR_VBR != 0 {
                    self.ltdc.insert(off, v & l::RCR_VBR);
                    self.reload_pending = true;
                }
            }
            l::SSCR | l::BPCR | l::AWCR | l::TWCR => {
                if self.ltdc_get(l::GCR) & l::GCR_LTDCEN != 0 {
                    self.violate(format!("LTDC: timing register {off:#x} written with LTDCEN high"));
                }
                self.ltdc.insert(off, v);
            }
            _ => {
                self.ltdc.insert(off, v);
                if (LAYER_FIRST..LAYER_END).contains(&off) {
                    let shadow = self.ltdc.clone();
                    self.layer_enable_check("shadow", &shadow);
                }
            }
        }
    }
}

impl Mp25Display {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every ordering rule the drivers broke so far, in order.
    pub fn violations(&self) -> Vec<String> {
        self.s.borrow().violations.clone()
    }

    /// A DSI register as read.
    pub fn dsi_reg(&self, off: u32) -> u32 {
        self.s.borrow().dsi_read(off)
    }

    /// An LTDC register as stored (layer registers: the shadow).
    pub fn ltdc_reg(&self, off: u32) -> u32 {
        self.s.borrow().ltdc_get(off)
    }

    /// A layer register as the pixel pipeline sees it (0 before a reload).
    pub fn ltdc_active(&self, off: u32) -> u32 {
        self.s.borrow().layer_active.get(&off).copied().unwrap_or(0)
    }

    /// The PLL registers (`WPCR1`, `WRPCR0`, `WRPCR1`) after a commit.
    pub fn pll_active(&self) -> Option<[u32; 3]> {
        self.s.borrow().pll_active
    }
}

enum Block {
    Dsi(u32),
    Ltdc(u32),
}

fn locate(addr: usize) -> Block {
    if (DSI_BASE..DSI_BASE + DSI_LEN).contains(&addr) {
        Block::Dsi((addr - DSI_BASE) as u32)
    } else if (LTDC_BASE..LTDC_BASE + LTDC_LEN).contains(&addr) {
        Block::Ltdc((addr - LTDC_BASE) as u32)
    } else {
        panic!("model_mp25_display: access at {addr:#x} is in neither the LTDC nor the DSI")
    }
}

impl Mmio for Mp25Display {
    fn read32(&self, addr: usize) -> u32 {
        match locate(addr) {
            Block::Dsi(o) => self.s.borrow().dsi_read(o),
            Block::Ltdc(o) => self.s.borrow_mut().ltdc_read(o),
        }
    }

    fn write32(&self, addr: usize, val: u32) {
        match locate(addr) {
            Block::Dsi(o) => self.s.borrow_mut().dsi_write(o, val),
            Block::Ltdc(o) => self.s.borrow_mut().ltdc_write(o, val),
        }
    }

    fn read8(&self, addr: usize) -> u8 {
        panic!("model_mp25_display: byte read at {addr:#x}")
    }

    fn write8(&self, addr: usize, _val: u8) {
        panic!("model_mp25_display: byte write at {addr:#x}")
    }
}

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ip::dw_mipi_dsi::{self as dsi, flags, Format};
    use crate::ip::stm32_dsi_phy::{self as phy, StmDsiPhy};
    use crate::model::TestEnv;
    use crate::video::HD720_60;

    /// `CK_KER_DSIPHY` on the DK: FLEXGEN_28, 27 MHz.
    const REF_HZ: u32 = 27_000_000;
    const FB: u64 = 0x9000_0000;

    fn link() -> dsi::Link {
        dsi::Link { mode: HD720_60, lanes: 4, channel: 0, format: Format::Rgb888, flags: flags::ADV7535 }
    }

    #[test]
    fn dk_720p_four_lanes_breaks_no_rule() {
        let m = Mp25Display::new();
        let env = TestEnv::default();
        let link = link();
        // pre_enable: the host and the PHY up in command mode
        assert_eq!(dsi::probe(&m, DSI_BASE), Ok(w::HWVER_141));
        phy::probe(&m, &env, DSI_BASE).unwrap();
        let mut p = StmDsiPhy::new(DSI_BASE, REF_HZ);
        let plan = dsi::attach(&m, &env, DSI_BASE, &mut p, &link).unwrap();
        // the CRTC: timings, the layer, the enables
        let ltdc = l::Ltdc::probe(&m, LTDC_BASE).unwrap();
        let layer = l::Layer { phys: FB, pitch: 0, format: l::Format::Xrgb8888 };
        let cfg = ltdc.configure(&m, &HD720_60, &l::Output::default(), &layer).unwrap();
        ltdc.enable(&m, &env).unwrap();
        // enable: video on
        dsi::video_on(&m, DSI_BASE, &link);

        assert_eq!(m.violations(), Vec::<String>::new());

        // LTDC: the mode (1280x720, 220 + 40 back porch, 20 + 5)
        assert_eq!(m.ltdc_reg(l::AWCR), ((259 + 1280) << 16) | (24 + 720));
        assert_eq!(m.ltdc_reg(l::TWCR), ((1650 - 1) << 16) | (750 - 1));
        assert_eq!(cfg.pitch, 5120);
        assert_eq!(m.ltdc_active(L1CFBAR), FB as u32);
        assert_eq!(m.ltdc_active(L1CFBLR) >> 16, 5120);
        assert_eq!(m.ltdc_active(L1CR) & l::LXCR_LEN, l::LXCR_LEN);
        assert!(ltdc.is_running(&m));
        assert_eq!(m.ltdc_reg(L1RCR) & l::RCR_VBR, 0, "the reload was taken");
        // DSI: four lanes, the active width, video mode, the PLL committed
        assert_eq!(m.dsi_reg(h::PHY_IF_CFG) & 0x3, 3);
        assert_eq!(m.dsi_reg(h::VID_PKT_SIZE), 1280);
        assert_eq!(m.dsi_reg(h::VID_VACTIVE_LINES), 720);
        assert_eq!(m.dsi_reg(h::MODE_CFG), h::ENABLE_VIDEO_MODE);
        assert_eq!(m.dsi_reg(h::PWR_UP), h::POWERUP);
        assert_eq!(m.dsi_reg(w::WCR) & w::WCR_DSIEN, w::WCR_DSIEN);
        assert!(phy::pll_enabled(&m, DSI_BASE));
        assert!(m.pll_active().is_some());
        let s = m.dsi_reg(h::PHY_STATUS);
        assert_eq!(s & (h::PHY_LOCK | h::PHY_STOP_STATE_CLK_LANE), h::PHY_LOCK | h::PHY_STOP_STATE_CLK_LANE);
        assert_eq!(plan.lane_mbps, p.pll().unwrap().lane_mbps());
    }

    #[test]
    fn the_layer_takes_effect_only_on_a_reload() {
        let m = Mp25Display::new();
        let env = TestEnv::default();
        let ltdc = l::Ltdc::probe(&m, LTDC_BASE).unwrap();
        let layer = l::Layer { phys: FB, pitch: 0, format: l::Format::Xrgb8888 };
        ltdc.configure(&m, &HD720_60, &l::Output::default(), &layer).unwrap();
        assert_eq!(m.ltdc_reg(L1CFBAR), FB as u32);
        assert_eq!(m.ltdc_active(L1CFBAR), 0, "still the shadow");
        ltdc.enable(&m, &env).unwrap();
        assert_eq!(m.ltdc_active(L1CFBAR), FB as u32);
        // a new address: shadow only until the frame boundary
        m.write32(LTDC_BASE + L1CFBAR as usize, 0xa000_0000);
        assert_eq!(m.ltdc_active(L1CFBAR), FB as u32);
        ltdc.set_fb(&m, &env, 0xa000_0000).unwrap();
        assert_eq!(m.ltdc_active(L1CFBAR), 0xa000_0000);
        assert!(m.violations().is_empty());
    }

    #[test]
    fn it_catches_out_of_order_writes() {
        // the PLL enabled with nothing committed and the PHY in reset: the
        // real entry point times out on the stop state and sets PLLEN as Linux
        let m = Mp25Display::new();
        let env = TestEnv::default();
        assert_eq!(phy::enable_pll(&m, &env, DSI_BASE), Err(-crate::errno::ETIMEDOUT));
        let v = m.violations();
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].contains("PLLEN while the D-PHY is still in reset"), "{v:?}");
        assert!(v[1].contains("PLLEN before the PLL configuration was committed"), "{v:?}");

        // a layer enabled without an address or a pitch
        let m = Mp25Display::new();
        let ltdc = l::Ltdc::probe(&m, LTDC_BASE).unwrap();
        ltdc.enable(&m, &env).unwrap();
        let v = m.violations();
        assert!(v.iter().any(|s| s.contains("CFBAR 0 (shadow)")), "{v:?}");
        assert!(v.iter().any(|s| s.contains("zero pitch")), "{v:?}");
        assert!(v.iter().any(|s| s.contains("CFBAR 0 (reload)")), "{v:?}");

        // the host's video mode before the PLL is up
        let m = Mp25Display::new();
        dsi::set_mode(&m, DSI_BASE, &link(), true);
        assert!(m.violations().iter().any(|s| s.contains("video mode with the PLL not locked")));
    }

    #[test]
    fn timings_with_the_controller_running_are_flagged() {
        let m = Mp25Display::new();
        let env = TestEnv::default();
        let ltdc = l::Ltdc::probe(&m, LTDC_BASE).unwrap();
        let layer = l::Layer { phys: FB, pitch: 0, format: l::Format::Xrgb8888 };
        ltdc.configure(&m, &HD720_60, &l::Output::default(), &layer).unwrap();
        ltdc.enable(&m, &env).unwrap();
        ltdc.configure(&m, &HD720_60, &l::Output::default(), &layer).unwrap();
        assert!(m.violations().iter().any(|s| s.contains("LTDCEN high")));
    }
}
