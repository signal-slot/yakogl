// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::verdin_imx95_display`] end to end: the DPU side against
//! `model_imx95_display`, the DSI side against `soc::imx95::dsi_model` and
//! the bridge model, both on one mock System Manager, every SCMI request,
//! register write and bridge write in one log: the design note's twelve
//! steps in their order, nothing refused for a domain that is off.

use super::verdin_imx95_display::{display_down, display_up, Config, Error};
use super::verdin_imx95_dsi_tests::{LogEnv, LogSm};
use crate::firmware::scmi;
use crate::ip::dw_mipi_dsi::regs as d;
use crate::ip::tpm_pwm::regs as t;
use crate::mmio::Mmio;
use crate::model::{golden_imx8mp_bridge, BridgeFaults, BridgeModel};
use crate::model_imx95_display::{self as dm, Display};
use crate::soc::imx95::combo_phy;
use crate::soc::imx95::display;
use crate::soc::imx95::dpu::Framebuffer;
use crate::soc::imx95::dsi_model::{Ev, Log, Soc};
use crate::soc::imx95::sm_mock::{dsi_off, dsi_on, Sm};
use std::cell::RefCell;
use std::rc::Rc;
use std::vec::Vec;

const FB: Framebuffer = Framebuffer { addr: 0x9000_0000, stride: 1280 * 4 };

/// The display blocks to the display model, everything else to the DSI
/// model; the display's writes also go into the joined log.
struct Joined {
    display: Display,
    soc: Soc,
}

fn display_block(a: usize) -> bool {
    (dm::CSR..dm::CSR + 0x1_0000).contains(&a)
        || (dm::IL..dm::IL + 0x2_0000).contains(&a)
        || (dm::DPU..dm::DPU + 0x40_0000).contains(&a)
}

impl Mmio for Joined {
    fn read32(&self, a: usize) -> u32 {
        if display_block(a) {
            self.display.read32(a)
        } else {
            self.soc.read32(a)
        }
    }
    fn write32(&self, a: usize, v: u32) {
        if display_block(a) {
            self.soc.log.borrow_mut().push(Ev::W(a, v));
            self.display.write32(a, v)
        } else {
            self.soc.write32(a, v)
        }
    }
    fn read8(&self, _: usize) -> u8 {
        unreachable!()
    }
    fn write8(&self, _: usize, _: u8) {
        unreachable!()
    }
}

struct Rig {
    log: Log,
    m: Joined,
    sm: LogSm,
    env: LogEnv,
}

fn rig() -> Rig {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sm = Rc::new(RefCell::new(Sm::verdin()));
    let cfg = Config::verdin(FB);
    Rig {
        m: Joined { display: Display::new(sm.clone()), soc: Soc::new(&log, cfg.dsi.bases, cfg.dsi.tpm, sm.clone()) },
        sm: LogSm { sm, log: log.clone() },
        env: LogEnv { bridge: BridgeModel::with_faults(BridgeFaults::default()), log: log.clone() },
        log,
    }
}

fn pos(log: &Log, e: Ev) -> usize {
    log.borrow().iter().position(|x| *x == e).unwrap_or_else(|| panic!("{e:?} not in the log"))
}

const FGENABLE: usize = dm::DPU + 0x2b_008c;
const PIXEL_LINK_CTRL: usize = dm::CSR + 0x8;

#[test]
fn the_twelve_steps_in_the_notes_order() {
    let mut r = rig();
    let cfg = Config::verdin(FB);
    let up = display_up(&mut r.sm, &r.m, &r.env, &cfg).unwrap();

    // nothing was touched with its domain off
    assert!(r.m.display.refused().is_empty(), "{:?}", r.m.display.refused());
    assert!(!r.log.borrow().iter().any(|e| matches!(e, Ev::Refused(_))));

    // step 1: the DPU side's SCMI, then the DSI side's (DISPLAY before CAMERA)
    let sm_log = r.sm.sm.borrow().log.clone();
    let split = sm_log.len() - dsi_on().len();
    assert_eq!(sm_log[split..], dsi_on()[..]);
    assert_eq!(sm_log[split - 1], (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_GET, vec![80]));

    // steps 3-6 (display CSR's pixel link), 6 (camera CSRs), 7, 8, 9, 10,
    // 11, 12
    let pl = pos(&r.log, Ev::W(PIXEL_LINK_CTRL, 0x3));
    // the last POWER_STATE_SET of the run is CAMERA's
    let camera_on =
        r.log.borrow().iter().rposition(|e| *e == Ev::Scmi(scmi::PROTO_POWER, scmi::POWER_STATE_SET)).unwrap();
    let pl_sel = pos(&r.log, Ev::W(combo_phy::MASTER_CSR_BASE + 0x4, 0));
    let rstz = pos(&r.log, Ev::W(combo_phy::DSI_BASE + d::PHY_RSTZ as usize, 0xf));
    let soft_reset = pos(&r.log, Ev::I2c(0x09, 0x01));
    let fg_on = r.log.borrow().iter().position(|e| matches!(*e, Ev::W(a, v) if a == FGENABLE && v != 0)).unwrap();
    let video = pos(&r.log, Ev::W(combo_phy::DSI_BASE + d::MODE_CFG as usize, d::ENABLE_VIDEO_MODE));
    let check = pos(&r.log, Ev::I2c(0xe5, 0xff));
    let backlight = pos(&r.log, Ev::W(cfg.dsi.tpm + t::SC as usize, 0x08));
    assert!(pl < camera_on, "the DPU side's routing before CAMERA");
    assert!(camera_on < pl_sel && pl_sel < rstz && rstz < soft_reset);
    assert!(soft_reset < fg_on && fg_on < video && video < check && check < backlight);

    // what came up
    assert_eq!(up.stream.prepared.pix_hz, 70_000_000);
    assert_eq!(r.m.display.scanned(), Some(FB.addr));
    assert_eq!((up.dsi.pll.m, up.dsi.pll.n, up.dsi.pll.fout_actual_hz * 2), (70, 4, 420_000_000));
    assert_eq!(up.dsi.host.hline, 1053);
    assert_eq!(up.dsi.link_irq_stat, 0);
    assert_eq!(r.env.bridge.image()[..0x40], golden_imx8mp_bridge()[..0x40]);
}

#[test]
fn down_stops_the_dsi_side_before_the_display() {
    let mut r = rig();
    let cfg = Config::verdin(FB);
    let up = display_up(&mut r.sm, &r.m, &r.env, &cfg).unwrap();
    r.sm.sm.borrow_mut().log.clear();
    r.log.borrow_mut().clear();
    display_down(&mut r.sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let sm_log = r.sm.sm.borrow().log.clone();
    assert_eq!(sm_log[..dsi_off().len()], dsi_off()[..], "CAMERA off first");
    assert_eq!(sm_log.last(), Some(&(scmi::PROTO_POWER, scmi::POWER_STATE_GET, vec![13])));
    let rstz = pos(&r.log, Ev::W(combo_phy::DSI_BASE + d::PHY_RSTZ as usize, 0));
    let pl_off = r.log.borrow().iter().position(|e| matches!(*e, Ev::W(a, _) if a == PIXEL_LINK_CTRL)).unwrap();
    let fg_off = pos(&r.log, Ev::W(FGENABLE, 0));
    assert!(rstz < pl_off && pl_off < fg_off);
    assert!(!r.sm.sm.borrow().on(3) && !r.sm.sm.borrow().on(13));
    assert!(r.m.display.refused().is_empty());
}

#[test]
fn a_dpu_that_never_syncs_leaves_the_host_in_command_mode() {
    let mut r = rig();
    r.m.display.faults.no_sync = true;
    let cfg = Config::verdin(FB);
    let e = display_up(&mut r.sm, &r.m, &r.env, &cfg).unwrap_err();
    assert!(matches!(e, Error::Display(display::Error::NoSync(_))), "{e:?}");
    assert_eq!(r.m.soc.reg(combo_phy::DSI_BASE + d::MODE_CFG as usize), d::ENABLE_CMD_MODE);
    assert!(r.m.soc.block_writes(cfg.dsi.tpm, 0x100).is_empty(), "backlight left dark");
}

#[test]
fn a_denied_camera_domain_is_the_dsi_sides_error_after_the_dpu_was_prepared() {
    let mut r = rig();
    r.sm.sm.borrow_mut().pd_perm.insert(3, 0);
    let e = display_up(&mut r.sm, &r.m, &r.env, &Config::verdin(FB)).unwrap_err();
    assert!(matches!(e, Error::Dsi(super::verdin_imx95_dsi::Error::Power(_))), "{e:?}");
    assert!(r.m.soc.block_writes(combo_phy::DSI_BASE, 0x1_0000).is_empty());
    assert!(!r.log.borrow().iter().any(|e| matches!(*e, Ev::W(a, v) if a == FGENABLE && v != 0)));
}
