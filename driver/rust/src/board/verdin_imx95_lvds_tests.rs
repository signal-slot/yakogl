// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! [`super::verdin_imx95_lvds`] end to end: the display blocks and the LVDS
//! CSR against `model_imx95_display`, TPM5 and GPIO5 against a register file
//! that logs, all on one mock System Manager; every SCMI request and
//! register write in one log: the design note's section 4(b') steps in
//! their order, nothing refused for a domain that is off.

use super::verdin_imx95_lvds::{self as lv, display_down, display_up, Config, Error};
use crate::env::{Env, Level};
use crate::firmware::scmi::{self, Transport};
use crate::ip::tpm_pwm::regs as t;
use crate::mmio::Mmio;
use crate::model_imx95_display::{self as dm, Display};
use crate::soc::imx95::display;
use crate::soc::imx95::dpu::Framebuffer;
use crate::soc::imx95::sm::{Fault, Status};
use crate::soc::imx95::sm_mock::{Sm, PERM_NONE};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::string::String;
use std::vec::Vec;

pub(crate) const FB: Framebuffer = Framebuffer { addr: 0x9000_0000, stride: 1280 * 4 };

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ev {
    /// An SCMI request: protocol, message, payload.
    Scmi(u8, u8, Vec<u32>),
    /// A register write: address, value.
    W(usize, u32),
    /// An env wait, µs.
    Wait(u32),
}

pub(crate) type Log = Rc<RefCell<Vec<Ev>>>;

pub(crate) struct LogSm {
    pub(crate) sm: Rc<RefCell<Sm>>,
    log: Log,
}

impl Transport for LogSm {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        let n = self.sm.borrow_mut().transfer(tx, rx)?;
        let r = self.sm.borrow().log.last().cloned().unwrap();
        self.log.borrow_mut().push(Ev::Scmi(r.0, r.1, r.2));
        Ok(n)
    }
}

pub(crate) struct LogEnv {
    log: Log,
    pub(crate) lines: RefCell<Vec<String>>,
}

impl Env for LogEnv {
    fn busy_wait_us(&self, us: u32) {
        self.log.borrow_mut().push(Ev::Wait(us));
    }
    fn sleep_ms(&self, ms: u32) {
        self.log.borrow_mut().push(Ev::Wait(ms * 1000));
    }
    fn log_str(&self, _: Level, line: &str) {
        self.lines.borrow_mut().push(line.into());
    }
    // The X12 path has no I2C and no env GPIO: any use is a defect.
    fn i2c_write(&self, bus: u32, _: &[u8]) -> Result<(), i32> {
        panic!("an I2C write on bus {bus}")
    }
    fn i2c_write_read(&self, bus: u32, _: &[u8], _: &mut [u8]) -> Result<(), i32> {
        panic!("an I2C read on bus {bus}")
    }
    fn gpio_set(&self, line: u32, _: bool) -> Result<(), i32> {
        panic!("an env GPIO on line {line}")
    }
}

/// The display model for its blocks; TPM5 and GPIO5 as a register file
/// (TPM5's PARAM reads a 32-bit counter, as the i.MX 95's).
pub(crate) struct Joined {
    pub(crate) display: Display,
    regs: RefCell<HashMap<usize, u32>>,
    log: Log,
}

fn display_block(a: usize) -> bool {
    (dm::CSR..dm::CSR + 0x1_0000).contains(&a)
        || (dm::LDB..dm::LDB + 0x1_0000).contains(&a)
        || (dm::IL..dm::IL + 0x2_0000).contains(&a)
        || (dm::DPU..dm::DPU + 0x40_0000).contains(&a)
}

impl Mmio for Joined {
    fn read32(&self, a: usize) -> u32 {
        if display_block(a) {
            return self.display.read32(a);
        }
        assert!(
            (lv::TPM5_BASE..lv::TPM5_BASE + 0x1000).contains(&a)
                || (lv::GPIO5_BASE..lv::GPIO5_BASE + 0x1000).contains(&a),
            "a read outside the X12 path's blocks: {a:#x}"
        );
        if a == lv::TPM5_BASE + t::PARAM as usize {
            return 32 << 16 | 4;
        }
        *self.regs.borrow().get(&a).unwrap_or(&0)
    }
    fn write32(&self, a: usize, v: u32) {
        self.log.borrow_mut().push(Ev::W(a, v));
        if display_block(a) {
            self.display.write32(a, v)
        } else {
            assert!(
                (lv::TPM5_BASE..lv::TPM5_BASE + 0x1000).contains(&a)
                    || (lv::GPIO5_BASE..lv::GPIO5_BASE + 0x1000).contains(&a),
                "a write outside the X12 path's blocks: {a:#x}"
            );
            self.regs.borrow_mut().insert(a, v);
        }
    }
    fn read8(&self, _: usize) -> u8 {
        unreachable!()
    }
    fn write8(&self, _: usize, _: u8) {
        unreachable!()
    }
}

pub(crate) struct Rig {
    pub(crate) log: Log,
    pub(crate) m: Joined,
    pub(crate) sm: LogSm,
    pub(crate) env: LogEnv,
}

pub(crate) fn rig() -> Rig {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let sm = Rc::new(RefCell::new(Sm::verdin()));
    Rig {
        m: Joined { display: Display::new(sm.clone()), regs: RefCell::new(HashMap::new()), log: log.clone() },
        sm: LogSm { sm, log: log.clone() },
        env: LogEnv { log: log.clone(), lines: RefCell::new(Vec::new()) },
        log,
    }
}

fn pos(log: &Log, e: &Ev) -> usize {
    log.borrow().iter().position(|x| x == e).unwrap_or_else(|| panic!("{e:?} not in the log"))
}

const FGENABLE: usize = dm::DPU + 0x2b_008c;
const PSOR: usize = lv::GPIO5_BASE + lv::rgpio::PSOR as usize;
const PCOR: usize = lv::GPIO5_BASE + lv::rgpio::PCOR as usize;
const PDDR: usize = lv::GPIO5_BASE + lv::rgpio::PDDR as usize;

#[test]
fn the_board_facts() {
    // TPM5 at 0x42500000, clock 41 + 113; GPIO5 at 0x43850000, pin 4;
    // TPM5 channel 0, 150 Hz, 119 / 255 low-true
    assert_eq!((lv::TPM5_BASE, lv::CLK_TPM5, lv::GPIO5_BASE, lv::EN_PIN), (0x4250_0000, 41 + 113, 0x4385_0000, 4));
    assert_eq!((lv::BACKLIGHT.channel, lv::BACKLIGHT.period_ns, lv::BACKLIGHT.low_true), (0, 6_666_667, true));
    assert_eq!((lv::BACKLIGHT.level, lv::BACKLIGHT.max), (119, 255));
    let c = Config::verdin(FB);
    assert_eq!(c.stream.mode, display::VERDIN_LVDS_10INCH);
    assert!(c.stream.route.pixel_link());
}

#[test]
fn the_steps_in_the_notes_order() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    assert!(r.m.display.refused().is_empty(), "{:?}", r.m.display.refused());

    let log = &r.log;
    // step 1: DISPLAY and the LDB PLL; nothing of the CAMERA domain, no
    // DISP1PIX, no I2C
    let pd_on = pos(log, &Ev::Scmi(scmi::PROTO_POWER, scmi::POWER_STATE_SET, vec![0, 13, 0]));
    let pll = pos(log, &Ev::Scmi(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, vec![38, 1, 0]));
    assert!(!log
        .borrow()
        .iter()
        .any(|e| matches!(e, Ev::Scmi(_, _, w) if w.contains(&3) && w.len() == 3 && w[1] == 3)));
    assert!(!log.borrow().iter().any(|e| matches!(e, Ev::Scmi(p, m, w)
        if *p == scmi::PROTO_CLOCK && *m == scmi::CLOCK_CONFIG_SET && w[0] == 80)));
    // step 2: the engine clock = LDB PLL / 7; step 4: the interleaver; 5:
    // the DPU; the pixel link
    let sel = pos(log, &Ev::W(dm::CSR, 2));
    let il = pos(log, &Ev::W(dm::IL, 0x600));
    let pl = pos(log, &Ev::W(dm::CSR + 0x8, 0x3));
    // step 3: the LDB, its 100 ms restart
    let gates = pos(log, &Ev::W(dm::LDB, 0x14));
    let parked = pos(log, &Ev::W(dm::LDB + 0x8, 0x7));
    let wait = pos(log, &Ev::Wait(100_000));
    let phy = pos(log, &Ev::W(dm::LDB + 0x8, 0x1075));
    // step 5: FrameGen0 on
    let fg = pos(log, &Ev::W(FGENABLE, 1));
    // step 6: TPM5's clock, the PWM (channel 0 low-true, the counter
    // started), then the enable
    let tpm_clk = pos(log, &Ev::Scmi(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, vec![154, 1, 0]));
    let cnsc = pos(log, &Ev::W(lv::TPM5_BASE + t::cnsc(0) as usize, 0x24));
    let run = pos(log, &Ev::W(lv::TPM5_BASE + t::SC as usize, 0x08));
    let en = pos(log, &Ev::W(PSOR, 1 << 4));
    let dir = pos(log, &Ev::W(PDDR, 1 << 4));
    assert!(pd_on < pll && pll < sel && sel < il && il < pl);
    assert!(pl < gates && gates < parked && parked < wait && wait < phy && phy < fg);
    assert!(fg < tpm_clk && tpm_clk < cnsc && cnsc < run && run < en && en < dir);

    // what came up
    assert_eq!((up.stream.prepared.pix_hz, up.tpm_hz), (71_100_000, 24_000_000));
    assert_eq!(up.ldb, [0x14, 0x0008_2221, 0x1075, 0]);
    assert_eq!((up.backlight.modulo, up.backlight.cnv), (159_999, 74_666));
    assert_eq!(r.m.display.scanned(), Some(FB.addr));
    assert_eq!(
        r.env.lines.borrow().last().map(String::as_str),
        Some(
            "verdin-imx95 LVDS: 1280x800 at 71100 kHz (LDB PLL / 7), LDB 0x00000014 0x00082221 0x00001075 0x00000000, backlight 119/255 low-true"
        )
    );
    let _ = sm;
}

#[test]
fn down_turns_the_backlight_off_first_then_the_ldb_then_the_stream_and_power() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.log.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let log = &r.log;
    let en_off = pos(log, &Ev::W(PCOR, 1 << 4));
    let tpm_off = pos(log, &Ev::Scmi(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, vec![154, 0, 0]));
    let phy_off = pos(log, &Ev::W(dm::LDB + 0x8, 0));
    let pl_off = pos(log, &Ev::W(dm::CSR + 0x8, 0x2));
    let fg_off = pos(log, &Ev::W(FGENABLE, 0));
    let pll_off = pos(log, &Ev::Scmi(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, vec![37, 0, 0]));
    let pd_off = pos(log, &Ev::Scmi(scmi::PROTO_POWER, scmi::POWER_STATE_SET, vec![0, 13, 0x4000_0000]));
    assert!(en_off < tpm_off && tpm_off < phy_off && phy_off < pl_off && pl_off < fg_off);
    assert!(fg_off < pll_off && pll_off < pd_off);
    assert!(!sm.sm.borrow().on(13) && !sm.sm.borrow().enabled(38) && !sm.sm.borrow().enabled(154));
    assert!(r.m.display.refused().is_empty());
}

#[test]
fn a_denied_backlight_clock_leaves_the_picture_up_and_dark() {
    let r = rig();
    let mut sm = r.sm;
    sm.sm.borrow_mut().clk_perm.insert(lv::CLK_TPM5, PERM_NONE);
    let e = display_up(&mut sm, &r.m, &r.env, &Config::verdin(FB)).unwrap_err();
    assert_eq!(e, Error::BacklightClock { msg: scmi::CLOCK_CONFIG_SET, fault: Fault::Refused(Status::Denied) });
    // FrameGen0 runs; neither the PWM nor the enable was touched
    assert!(r.log.borrow().contains(&Ev::W(FGENABLE, 1)));
    assert!(!r
        .log
        .borrow()
        .iter()
        .any(|e| matches!(*e, Ev::W(a, _) if (lv::TPM5_BASE..lv::TPM5_BASE + 0x1000).contains(&a))));
    assert!(!r
        .log
        .borrow()
        .iter()
        .any(|e| matches!(*e, Ev::W(a, _) if (lv::GPIO5_BASE..lv::GPIO5_BASE + 0x1000).contains(&a))));
}

#[test]
fn a_denied_ldb_pll_touches_no_register() {
    let r = rig();
    let mut sm = r.sm;
    sm.sm.borrow_mut().clk_perm.insert(38, PERM_NONE);
    let e = display_up(&mut sm, &r.m, &r.env, &Config::verdin(FB)).unwrap_err();
    assert!(matches!(e, Error::Display(display::Error::Power(_))), "{e:?}");
    assert!(!r.log.borrow().iter().any(|e| matches!(e, Ev::W(..))));
}

#[test]
fn a_dpu_that_never_syncs_leaves_the_backlight_dark() {
    let mut r = rig();
    r.m.display.faults.no_sync = true;
    let mut sm = r.sm;
    let e = display_up(&mut sm, &r.m, &r.env, &Config::verdin(FB)).unwrap_err();
    assert!(matches!(e, Error::Display(display::Error::NoSync(_))), "{e:?}");
    // the LDB was up; no TPM5 clock request, no PWM, no enable
    assert!(r.log.borrow().contains(&Ev::W(dm::LDB + 0x8, 0x1075)));
    assert!(!r.log.borrow().iter().any(|e| matches!(e, Ev::Scmi(_, _, w) if w.first() == Some(&154))));
    assert!(!r.log.borrow().iter().any(|e| matches!(*e, Ev::W(a, _) if a == PSOR)));
}
