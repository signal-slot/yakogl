// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::verdin_imx95_dsi::dsi_output_up`] for the 10.1" panel's mode,
//! end to end on the host: the SCMI requests against the mock SM
//! (`soc::imx95::sm_mock`), the DSI host, CSRs, combo PHY test codes
//! and TPM6 against the register model (`soc::imx95::dsi_model`), the
//! SN65DSI83 against `model::BridgeModel`; one log holds all of them in
//! the order they happened.

use super::verdin_imx95_dsi::{self as v, dsi_output_down, dsi_output_up, Config, Error, PANEL};
use crate::env::{Env, Level};
use crate::errno::{EIO, ENODEV, ETIMEDOUT};
use crate::firmware::scmi::{self, Transport};
use crate::ip::dw_mipi_dsi::{regs as d, step, Failed};
use crate::ip::tpm_pwm::regs as t;
use crate::model::{golden_imx8mp_bridge, BridgeFaults, BridgeModel};
use crate::soc::imx95::combo_phy::{self, Tuning};
use crate::soc::imx95::dsi_model::{Ev, Log, Soc};
use crate::soc::imx95::dsi_power::{self, Step};
use crate::soc::imx95::sm::Status;
use crate::soc::imx95::sm_mock::{dsi_off, dsi_on, Sm, PERM_NONE};
use std::cell::RefCell;
use std::rc::Rc;
use std::vec::Vec;

/// The mock SM (shared with the register models), its requests also in
/// the run's log.
pub(crate) struct LogSm {
    pub sm: Rc<RefCell<Sm>>,
    pub log: Log,
}

impl Transport for LogSm {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        let h = scmi::word(tx, 0);
        self.log.borrow_mut().push(Ev::Scmi((h >> 10 & 0xff) as u8, (h & 0xff) as u8));
        self.sm.borrow_mut().transfer(tx, rx)
    }
}

/// The bridge's model as the shim's Env, its I2C writes and EN in the log.
pub(crate) struct LogEnv {
    pub bridge: BridgeModel,
    pub log: Log,
}

impl Env for LogEnv {
    fn busy_wait_us(&self, us: u32) {
        self.bridge.busy_wait_us(us)
    }
    fn sleep_ms(&self, ms: u32) {
        self.bridge.sleep_ms(ms)
    }
    fn log_str(&self, level: Level, line: &str) {
        self.bridge.log_str(level, line)
    }
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
        self.bridge.i2c_write(bus, buf)?;
        self.log.borrow_mut().push(Ev::I2c(buf[0], buf[1]));
        Ok(())
    }
    fn i2c_write_read(&self, bus: u32, w: &[u8], r: &mut [u8]) -> Result<(), i32> {
        self.bridge.i2c_write_read(bus, w, r)
    }
    fn gpio_set(&self, line: u32, value: bool) -> Result<(), i32> {
        self.log.borrow_mut().push(Ev::Gpio(line, value));
        self.bridge.gpio_set(line, value)
    }
}

struct Rig {
    log: Log,
    soc: Soc,
    sm: LogSm,
    env: LogEnv,
}

fn rig(faults: BridgeFaults) -> Rig {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sm = Rc::new(RefCell::new(Sm::verdin()));
    Rig {
        soc: Soc::new(&log, Config::VERDIN.bases, Config::VERDIN.tpm, sm.clone()),
        sm: LogSm { sm, log: log.clone() },
        env: LogEnv { bridge: BridgeModel::with_faults(faults), log: log.clone() },
        log,
    }
}

const DSI: usize = combo_phy::DSI_BASE;

fn pos(log: &Log, e: Ev) -> usize {
    log.borrow().iter().position(|x| *x == e).unwrap_or_else(|| panic!("{e:?} not in the log"))
}

fn rpos(log: &Log, f: impl Fn(&Ev) -> bool) -> usize {
    log.borrow().iter().rposition(f).unwrap()
}

#[test]
fn the_10_inch_panel_comes_up_in_order() {
    let mut r = rig(BridgeFaults::default());
    let up = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap();

    // SCMI, every request before the first register write
    assert_eq!(r.sm.sm.borrow().log, dsi_on());
    let last_scmi = rpos(&r.log, |e| matches!(e, Ev::Scmi(..)));
    let first_w = r.log.borrow().iter().position(|e| matches!(e, Ev::W(..))).unwrap();
    assert!(last_scmi < first_w);

    // the link: 420 Mbit/s, M 70, N 4; the host's line in lane byte clocks
    assert_eq!(up.pll.lane_bps, 420_000_000);
    assert_eq!((up.pll.m, up.pll.n, up.pll.fout_actual_hz), (70, 4, 210_000_000));
    assert_eq!(up.host.lane_mbps, 420);
    assert_eq!((up.host.hline, up.host.hsa, up.host.hbp), (1053, 18, 48));
    assert_eq!((up.host.vsa, up.host.vbp, up.host.vfp, up.host.vactive), (8, 24, 8, 800));
    assert_eq!((up.host.esc_div, up.host.settle_ms), (3, 34));
    assert_eq!(up.host_version, d::HWVER_131);

    // the host's registers as the design note's table gives them
    let reg = |off: u32| r.soc.reg(DSI + off as usize);
    assert_eq!(reg(d::DPI_COLOR_CODING), 0x5);
    assert_eq!(reg(d::DPI_CFG_POL), 0x6);
    assert_eq!(reg(d::PCKHDL_CFG), 0x1c, "no EoTp");
    assert_eq!(reg(d::VID_PKT_SIZE), 1280);
    assert_eq!(reg(d::CLKMGR_CFG), 3);
    assert_eq!(reg(d::PHY_TMR_CFG), 23 << 16 | 49);
    assert_eq!(reg(d::PHY_TMR_LPCLK_CFG), 40 << 16 | 65);
    assert_eq!(reg(d::PHY_IF_CFG), 0x2003);
    assert_eq!(reg(d::PHY_RSTZ), 0xf);
    assert_eq!(reg(d::VID_MODE_CFG), 0x3f02, "burst, LP in every blanking");
    assert_eq!(reg(d::MODE_CFG), 0, "video mode");
    assert_eq!(reg(d::LPCLK_CTRL), d::PHY_TXREQUESTCLKHS);

    // the PHY's test codes, hal's for 420 Mbit/s
    let (codes, n) = up.pll.test_codes();
    let want: Vec<_> = codes[..n].iter().map(|c| (c.code, c.data)).collect();
    assert_eq!(r.soc.codes("dsi"), want);
    assert_eq!(r.soc.reg(combo_phy::PHY_CSR_BASE + 0x104), 0x0005_001c);

    // the bridge's CSRs are the i.MX8MP board's Linux dump of the same
    // display, byte for byte over 0x00-0x3F
    let golden = golden_imx8mp_bridge();
    let img = r.env.bridge.image();
    assert_eq!(img[..0x40], golden[..0x40]);
    assert_eq!((up.bridge.plan.dsi_clk_range, up.bridge.plan.divider, up.bridge.plan.lvds_clk_hz), (42, 3, 70_000_000));
    assert_eq!(up.link_irq_stat, 0);

    // the order: PHY released, then the bridge's EN cycle, its CSRs and soft
    // reset, then video, then the link check, then the backlight
    let rstz = pos(&r.log, Ev::W(DSI + d::PHY_RSTZ as usize, 0xf));
    let en_low = pos(&r.log, Ev::Gpio(0, false));
    let en_high = pos(&r.log, Ev::Gpio(0, true));
    let soft_reset = pos(&r.log, Ev::I2c(0x09, 0x01));
    let video = pos(&r.log, Ev::W(DSI + d::MODE_CFG as usize, d::ENABLE_VIDEO_MODE));
    let clear = pos(&r.log, Ev::I2c(0xe5, 0xff));
    let pwm = pos(&r.log, Ev::W(Config::VERDIN.tpm + t::cnv(2) as usize, 74_666));
    let start = pos(&r.log, Ev::W(Config::VERDIN.tpm + t::SC as usize, 0x08));
    assert!(rstz < en_low && en_low < en_high && en_high < soft_reset);
    assert!(soft_reset < video && video < clear && clear < pwm && pwm < start);
    assert_eq!(up.backlight.cnv, 74_666);
}

#[test]
fn linuxs_phy_values_are_a_parameter() {
    let mut r = rig(BridgeFaults::default());
    let cfg = Config { tuning: Tuning::LINUX_420, ..Config::VERDIN };
    let up = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &cfg).unwrap();
    assert_eq!((up.pll.hsfreqrange, up.pll.vco_cntrl, up.pll.pll_17), (0x16, 0x1f, 0xcd));
    assert_eq!(r.soc.reg(combo_phy::PHY_CSR_BASE + 0x100), 0xf3);
    assert_eq!(r.soc.reg(combo_phy::PHY_CSR_BASE + 0x104), 0x0016_001c);
    assert_eq!(r.soc.reg(combo_phy::PHY_CSR_BASE + 0x108), 0x2000);
    assert_eq!(r.soc.codes("csi"), [(combo_phy::code::RX_CLKLANE_LANE_1, 0x02)]);
}

#[test]
fn a_margin_raises_the_lane_rate_and_the_bridge_follows() {
    // the design note's fallback (U17): 432 Mbit/s, M 72, N 4
    let mut r = rig(BridgeFaults::default());
    let cfg = Config { margin_permille: 29, ..Config::VERDIN };
    let up = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &cfg).unwrap();
    assert_eq!(up.pll.lane_bps, 432_180_000);
    assert_eq!((up.pll.m, up.pll.n), (72, 4));
    assert_eq!(up.bridge.plan.divider, 3);
    assert_eq!(up.bridge.plan.dsi_clk_range, 43);
}

#[test]
fn a_denied_camera_domain_touches_no_register_and_no_bridge() {
    let mut r = rig(BridgeFaults::default());
    r.sm.sm.borrow_mut().pd_perm.insert(3, PERM_NONE);
    let e = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Power(dsi_power::Error::Refused { step: Step::PowerOn, status: Status::Denied }));
    assert!(r.log.borrow().iter().all(|e| matches!(e, Ev::Scmi(..))));
}

#[test]
fn a_host_below_1_31_is_refused_before_any_write() {
    let mut r = rig(BridgeFaults::default());
    r.soc.version = d::HWVER_130;
    let e = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Host(Failed { step: step::CHECK, errno: -ENODEV }));
    assert!(!r.log.borrow().iter().any(|e| matches!(e, Ev::W(..))));
}

#[test]
fn a_phy_that_never_locks_stops_before_the_bridge() {
    let mut r = rig(BridgeFaults::default());
    r.soc.never_locks = true;
    let e = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Host(Failed { step: step::PHY_LOCK, errno: -ETIMEDOUT }));
    assert!(!r.log.borrow().iter().any(|e| matches!(e, Ev::Gpio(..) | Ev::I2c(..))));
}

#[test]
fn a_bridge_that_never_answers_leaves_the_host_in_command_mode() {
    let mut r = rig(BridgeFaults { nack_reads_after_en: u32::MAX, ..Default::default() });
    let e = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap_err();
    assert_eq!(e, Error::Bridge(-EIO));
    assert_eq!(r.soc.reg(DSI + d::MODE_CFG as usize), d::ENABLE_CMD_MODE);
    assert!(r.soc.block_writes(Config::VERDIN.tpm, 0x100).is_empty(), "backlight left dark");
}

#[test]
fn link_errors_are_reported_and_the_backlight_still_comes_on() {
    let mut r = rig(BridgeFaults::default());
    r.env.bridge.set_irq_source(0x04); // CHA_SOT_BIT_ERR keeps being seen
    let up = dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap();
    assert_eq!(up.link_irq_stat, 0x04);
    assert_eq!(r.soc.reg(Config::VERDIN.tpm + t::SC as usize), 0x08);
}

#[test]
fn down_reverses_up() {
    let mut r = rig(BridgeFaults::default());
    dsi_output_up(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap();
    r.log.borrow_mut().clear();
    r.sm.sm.borrow_mut().log.clear();
    dsi_output_down(&mut r.sm, &r.soc, &r.env, &PANEL, &Config::VERDIN).unwrap();
    assert_eq!(r.sm.sm.borrow().log, dsi_off());
    let pwm_off = pos(&r.log, Ev::W(Config::VERDIN.tpm + t::SC as usize, 0));
    let en_low = pos(&r.log, Ev::Gpio(0, false));
    let gates = pos(&r.log, Ev::W(combo_phy::MASTER_CSR_BASE, 0x1f));
    let rstz = pos(&r.log, Ev::W(DSI + d::PHY_RSTZ as usize, 0));
    let scmi = r.log.borrow().iter().position(|e| matches!(e, Ev::Scmi(..))).unwrap();
    assert!(pwm_off < en_low && en_low < gates && gates < rstz && rstz < scmi);
    assert!(!r.sm.sm.borrow().on(3));
}

#[test]
fn the_panel_and_link_constants() {
    assert_eq!((PANEL.htotal(), PANEL.vtotal()), (1404, 840));
    assert_eq!(PANEL.refresh_mhz(PANEL.pixel_clock_hz), 59_354);
    let l = v::link(&PANEL);
    assert_eq!((l.lanes, l.mode.hsync_high, l.mode.vsync_high, l.mode.de_high), (4, false, false, true));
    assert_eq!(v::DSI_FLAGS, 0x283);
}
