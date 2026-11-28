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
use crate::model_imx95_display::{self as dm, Access, Display};
use crate::soc::imx95::display;
use crate::soc::imx95::dpu::{self, Framebuffer};
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

/// FrameGen0's sequence complete (bit 20 of display engine 0's
/// `DISPIRQ0_INTERRUPTSTATUS0`, DPU + 0x38102C) over the display model,
/// which does not have it: it sets on the `after`-th read of that word that
/// follows a write of `FGENABLE` = 0 (never when `after` is `None`) and
/// clears when written to `DISPIRQ0_INTERRUPTCLEAR0`. It never sets in the
/// common-control `IRQ_INTERRUPTSTATUS0` (DPU + 0x102C), as on the board
/// (record 20261128-0030-triangle-3dbc4ac9).
#[derive(Default)]
pub(crate) struct SeqComplete {
    pub(crate) after: Option<u32>,
    armed: bool,
    reads: u32,
    set: bool,
}

/// The display model for its blocks; TPM5 and GPIO5 as a register file
/// (TPM5's PARAM reads a 32-bit counter, as the i.MX 95's).
pub(crate) struct Joined {
    pub(crate) display: Display,
    pub(crate) seq: RefCell<SeqComplete>,
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
        if a == DIRQ0_STATUS0 {
            let v = self.display.read32(a);
            let mut q = self.seq.borrow_mut();
            if q.armed {
                q.reads += 1;
                if q.after.is_some_and(|n| q.reads >= n) {
                    q.armed = false;
                    q.set = true;
                }
            }
            return v | if q.set { SEQ_BIT } else { 0 };
        }
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
        if a == FGENABLE && v & 1 == 0 {
            let mut q = self.seq.borrow_mut();
            q.armed = true;
            q.reads = 0;
        }
        if a == DIRQ0_CLEAR0 && v & SEQ_BIT != 0 {
            self.seq.borrow_mut().set = false;
        }
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
        m: Joined {
            display: Display::new(sm.clone()),
            seq: RefCell::new(SeqComplete { after: Some(1), ..SeqComplete::default() }),
            regs: RefCell::new(HashMap::new()),
            log: log.clone(),
        },
        sm: LogSm { sm, log: log.clone() },
        env: LogEnv { log: log.clone(), lines: RefCell::new(Vec::new()) },
        log,
    }
}

fn pos(log: &Log, e: &Ev) -> usize {
    log.borrow().iter().position(|x| x == e).unwrap_or_else(|| panic!("{e:?} not in the log"))
}

const FGENABLE: usize = dm::DPU + 0x2b_008c;
const FGSLR: usize = dm::DPU + 0x2b_0090;
/// `DISPIRQ0_INTERRUPTENABLE0`, `_INTERRUPTCLEAR0`, `_INTERRUPTSTATUS0`
/// (`PERI_DISPLAY_SEERIS.h`; NXP's `dpu95-core.c` `dpu_base + 0x381000`),
/// the common-control `IRQ_INTERRUPTSTATUS0` and
/// `kDPU_Group0DiSengcfgSeqComplete0Interrupt` (`fsl_dpu.h`), restated.
const DIRQ0_ENABLE0: usize = dm::DPU + 0x38_1008;
const DIRQ0_CLEAR0: usize = dm::DPU + 0x38_1020;
const DIRQ0_STATUS0: usize = dm::DPU + 0x38_102c;
const COMCTRL_STATUS0: u32 = 0x102c;
const SEQ_BIT: u32 = 1 << 20;
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
fn down_turns_the_backlight_off_first_then_stops_frame_gen_then_the_ldb_then_the_power() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.log.borrow_mut().clear();
    r.m.display.clear_log();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let log = &r.log;
    let en_off = pos(log, &Ev::W(PCOR, 1 << 4));
    let tpm_off = pos(log, &Ev::Scmi(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, vec![154, 0, 0]));
    let phy_off = pos(log, &Ev::W(dm::LDB + 0x8, 0));
    let gate = pos(log, &Ev::W(dm::LDB, 0x1E));
    let pl_off = pos(log, &Ev::W(dm::CSR + 0x8, 0x2));
    let fg_off = pos(log, &Ev::W(FGENABLE, 0));
    let pll_off = pos(log, &Ev::Scmi(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, vec![37, 0, 0]));
    let pd_off = pos(log, &Ev::Scmi(scmi::PROTO_POWER, scmi::POWER_STATE_SET, vec![0, 13, 0x4000_0000]));
    // FrameGen0 stops before any LDB register is touched
    assert!(en_off < tpm_off && tpm_off < fg_off && fg_off < phy_off);
    assert!(phy_off < gate && gate < pl_off);
    assert!(pl_off < pll_off && pll_off < pd_off);
    // no FGSLR shadow token while FrameGen0 stops (Linux's
    // dpu95_crtc_disable sends none); display::stop's comes after the gate
    assert!(!log.borrow()[..gate].contains(&Ev::W(FGSLR, 1)));
    // in the display blocks' own log (reads too): FGENABLE 0, SeqComplete0
    // seen in DISPIRQ0, the FGENSTS read, then the LDB gate
    let dlog = r.m.display.log();
    let at = |a: Access| dlog.iter().position(|x| *x == a).unwrap_or_else(|| panic!("{a:?} not in the log"));
    let fg_off = at(Access::W("dpu", 0x2b_008c, 0));
    let seq = at(Access::W("dpu", 0x38_1020, SEQ_BIT));
    let seq = dlog.iter().skip(seq + 1).position(|x| *x == Access::W("dpu", 0x38_1020, SEQ_BIT)).unwrap() + seq + 1;
    let ensts = dlog.iter().position(|x| matches!(x, Access::R("dpu", 0x2b_0094, _))).unwrap();
    let gate = at(Access::W("ldb", 0x0, 0x1E));
    assert!(fg_off < seq && seq < ensts && ensts < gate, "{dlog:?}");
    // the frame counter is read at both ends of each wait (the
    // SeqComplete0 wait, then the FGENSTS wait), all before the LDB gate
    let ts: Vec<usize> =
        dlog.iter().enumerate().filter(|(_, x)| matches!(x, Access::R("dpu", 0x2b_0098, _))).map(|(i, _)| i).collect();
    let before_gate: Vec<usize> = ts.into_iter().filter(|&i| i < gate).collect();
    assert!(before_gate.len() == 4 && before_gate[0] > fg_off && before_gate[1] < seq, "{dlog:?}");
    assert!(before_gate[2] > seq && before_gate[2] < ensts && before_gate[3] > ensts, "{dlog:?}");
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

/// FrameGen0's enable status clears only when the running frame ends, and a
/// frame ends only while the LDB's clocks run (board record
/// 20261127-1806-triangle-472630df: `NotStopped(fgensts: 1)`). Gating the
/// LDB first leaves the status at 1 for ever.
#[test]
fn down_waits_for_the_last_frame_before_it_gates_the_ldb() {
    let mut r = rig();
    r.m.display.faults.frame_end_needed = true;
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.log.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    assert!(!sm.sm.borrow().on(13));
    assert!(r.m.display.refused().is_empty());
}

/// When the stop fails the LDB is still down, the first error comes back and
/// the display's power stays on.
#[test]
fn a_frame_gen_that_never_stops_still_brings_the_ldb_down_and_returns_not_stopped() {
    let mut r = rig();
    r.m.display.faults.fg_stuck = true;
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.log.borrow_mut().clear();
    let e = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap_err();
    let Error::Display(display::Error::NotStopped(fg)) = e else { panic!("{e:?}") };
    // the wait ran the whole 1 s and the error carries the frame counter
    let w = fg.stop.expect("the frame counter is in the error");
    assert_eq!(w.waited_us, display::STOP_WAIT_US);
    assert!(w.frame_end() > w.frame_start(), "{w:?}");
    assert!(r.log.borrow().contains(&Ev::W(dm::LDB, 0x1E)));
    assert!(sm.sm.borrow().on(13));
}

/// The down waits for FrameGen0's sequence complete where Linux takes it,
/// bit 20 of `DISPIRQ0_INTERRUPTSTATUS0` (DPU + 0x38102C), before it
/// touches the LDB: a stale bit is cleared and the bit enabled before
/// `FGENABLE` = 0, the bit is polled until it sets (here on the fifth read),
/// cleared again and masked, and only then are the LDB's clocks gated. The
/// common-control word (DPU + 0x102C) is not what the down waits on.
#[test]
fn down_gates_the_ldb_only_after_frame_gen_reports_its_sequence_complete() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.seq.borrow_mut().after = Some(5);
    r.log.borrow_mut().clear();
    r.m.display.clear_log();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    assert_eq!(lv::SEQ_COMPLETE0, SEQ_BIT);
    assert_eq!(lv::SEQ_COMPLETE_BUDGET_US, 1_000_000);

    let dlog = r.m.display.log();
    let at = |a: Access| dlog.iter().position(|x| *x == a).unwrap_or_else(|| panic!("{a:?} not in the log"));
    let cleared = at(Access::W("dpu", 0x38_1020, SEQ_BIT));
    let enabled = at(Access::W("dpu", 0x38_1008, SEQ_BIT));
    let fg_off = at(Access::W("dpu", 0x2b_008c, 0));
    let gate = at(Access::W("ldb", 0x0, 0x1E));
    let polls: Vec<usize> = dlog
        .iter()
        .enumerate()
        .filter(|(i, a)| *i > fg_off && matches!(a, Access::R("dpu", 0x38_102c, _)))
        .map(|(i, _)| i)
        .collect();
    assert!(cleared < enabled && enabled < fg_off, "{dlog:?}");
    // no common-control status read before the gate
    assert!(!dlog[..gate].iter().any(|a| matches!(a, Access::R("dpu", COMCTRL_STATUS0, _))), "{dlog:?}");
    // five polls, the fifth saw the bit, all before the gate; no LDB
    // register touched before the fifth
    assert!(polls.len() >= 5 && polls[4] < gate, "{dlog:?}");
    assert!(!dlog[..polls[4]].iter().any(|a| matches!(a, Access::W("ldb", ..))), "{dlog:?}");
    // the bit cleared after it was seen, then the gate
    let reclear = dlog.iter().rposition(|a| *a == Access::W("dpu", 0x38_1020, SEQ_BIT)).unwrap();
    let masked = at(Access::W("dpu", 0x38_1008, 0));
    assert!(polls[4] < reclear && reclear < masked && masked < gate, "{dlog:?}");
    assert_eq!(polls.iter().filter(|&&i| i < reclear).count(), 5, "{dlog:?}");
    assert!(!r.m.seq.borrow().set);
    // four poll steps waited between the five reads
    let waits: Vec<u32> = r
        .log
        .borrow()
        .iter()
        .filter_map(|e| match e {
            Ev::Wait(us) => Some(*us),
            _ => None,
        })
        .collect();
    assert_eq!(&waits[..4], &[dpu::POLL_STEP_US; 4]);
    assert!(r
        .env
        .lines
        .borrow()
        .iter()
        .any(|l| l.starts_with("verdin-imx95 LVDS down: SeqComplete0 after 400 us (5 polls, frame ")
            && l.ends_with("FGENSTS stopped")));
    assert!(!sm.sm.borrow().on(13) && !sm.sm.borrow().enabled(38));
    assert!(r.m.display.refused().is_empty());
}

/// FrameGen0's sequence never completes: the down gives up after its 1 s,
/// returns the timeout with what the DPU read, and leaves the LDB's clocks
/// ungated and the display's power on.
#[test]
fn a_sequence_that_never_completes_times_out_and_leaves_the_ldb_clocks_on() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.seq.borrow_mut().after = None;
    r.log.borrow_mut().clear();
    let e = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap_err();
    let Error::SeqTimeout(t) = e else { panic!("{e:?}") };
    let w = t.wait();
    assert_eq!(w.waited_us, lv::SEQ_COMPLETE_BUDGET_US);
    assert_eq!(w.irq & SEQ_BIT, 0);
    assert_eq!(t.fg.fgenable, 0);
    let log = r.log.borrow();
    let waited: u32 = log
        .iter()
        .filter_map(|e| match e {
            Ev::Wait(us) => Some(*us),
            _ => None,
        })
        .sum();
    assert_eq!(waited, lv::SEQ_COMPLETE_BUDGET_US);
    // FrameGen0 was told to stop; no LDB register was written after it
    let fg_off = log.iter().position(|e| *e == Ev::W(FGENABLE, 0)).unwrap();
    assert!(!log[fg_off..].iter().any(|e| matches!(*e, Ev::W(a, _) if (dm::LDB..dm::LDB + 0x1_0000).contains(&a))));
    drop(log);
    // the LDB still reads as the up left it (`ldb::up`: 0x14, not gated)
    assert_eq!(r.m.display.peek("ldb", 0x0), 0x14);
    assert!(sm.sm.borrow().on(13) && sm.sm.borrow().enabled(38));
    assert!(r.env.lines.borrow().iter().any(|l| l.contains("no SeqComplete0 in 1000000 us")));
}

/// The success line names the time waited and the frame counter values.
#[test]
fn a_stop_that_succeeds_logs_the_wait_and_the_frame_counter() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.env.lines.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let lines = r.env.lines.borrow();
    let l = lines.iter().find(|l| l.contains("FrameGen0 stopped after")).expect("a success line");
    assert!(l.contains(" us (") && l.contains("frame counter") && l.contains("FGTIMESTAMP 0x"), "{l}");
}

/// The SeqComplete0 wait enables the bit in DISPIRQ0 before `FGENABLE` = 0
/// and masks it again on the way out, also when it times out (Linux's
/// `enable_irq` / `disable_irq` around the wait).
#[test]
fn the_sequence_complete_is_enabled_only_around_the_wait() {
    for after in [Some(3), None] {
        let r = rig();
        let mut sm = r.sm;
        let cfg = Config::verdin(FB);
        let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
        r.m.seq.borrow_mut().after = after;
        r.log.borrow_mut().clear();
        let _ = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream);
        let log = r.log.borrow();
        let en: Vec<u32> = log
            .iter()
            .filter_map(|e| match *e {
                Ev::W(a, v) if a == DIRQ0_ENABLE0 => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(en, vec![SEQ_BIT, 0], "{after:?}");
        let on = pos(&r.log, &Ev::W(DIRQ0_ENABLE0, SEQ_BIT));
        let off = pos(&r.log, &Ev::W(DIRQ0_ENABLE0, 0));
        let fg_off = pos(&r.log, &Ev::W(FGENABLE, 0));
        assert!(on < fg_off && fg_off < off, "{after:?}");
    }
}

fn timeout(stuck: bool) -> (dpu::SeqTimeout, Vec<String>) {
    let mut r = rig();
    r.m.display.faults.fg_stuck = stuck;
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.seq.borrow_mut().after = None;
    r.env.lines.borrow_mut().clear();
    let e = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap_err();
    let Error::SeqTimeout(t) = e else { panic!("{e:?}") };
    let lines = r.env.lines.borrow().clone();
    (t, lines)
}

/// No line of a failed down is cut at `env::LINE_MAX` (the env marks a cut
/// line with a trailing "...").
fn whole(lines: &[String]) {
    for l in lines {
        assert!(l.len() < crate::env::LINE_MAX && !l.ends_with("..."), "{l}");
    }
}

/// The timeout path reports the frame counter and `FGTIMESTAMP` (frame and
/// line index) at the start and the end of the wait, the raw DISPIRQ0
/// status, the common-control word and the lock status, with the verdict:
/// with the pixel clock gone after the disable the counter is frozen.
#[test]
fn a_sequence_timeout_reports_the_frame_counter_the_timestamps_and_the_irq_status() {
    let (t, lines) = timeout(false);
    let w = t.wait();
    assert_eq!(
        (w.waited_us, w.polls),
        (lv::SEQ_COMPLETE_BUDGET_US, lv::SEQ_COMPLETE_BUDGET_US / dpu::POLL_STEP_US + 1)
    );
    assert!(w.frozen() && w.frames() == 0 && w.frame_start() > 0, "{w:?}");
    assert_eq!((w.ts_start, w.line_start()), (w.frame_start() << 14, 0));
    assert_eq!((t.fg.fgensts, t.comctrl & SEQ_BIT, t.lockstatus), (0, 0, 0));
    let f = w.frame_start();
    let want = [
        "verdin-imx95 LVDS down: FGENABLE 0 (no FGSLR token), waiting up to 1000000 us for SeqComplete0 in DISPIRQ0 (DPU+0x38102c bit 20)".to_string(),
        format!(
            "verdin-imx95 LVDS down: no SeqComplete0 in 1000000 us: COMCTRL 0x{:08x}, DISPIRQ0 LOCKSTATUS 0x0, FGCHSTAT 0x0; the LDB and power stay on",
            t.comctrl
        ),
        "verdin-imx95 X12: no SeqComplete0 after 1000000 us (10001 polls): FGENABLE 0 FGENSTS 0x0, DISPIRQ0 INTERRUPTSTATUS0 0x00000000".to_string(),
        format!("verdin-imx95 X12: frame counter {f} -> {f} (0 frames): frozen: the pixel clock is dead"),
        format!("verdin-imx95 X12: FGTIMESTAMP 0x{:08x} (frame {f} line 0) -> 0x{:08x} (frame {f} line 0)", w.ts_start, w.ts_end),
    ];
    assert_eq!(lines, want);
    whole(&lines);
}

/// A FrameGen0 that runs on after the disable: the counter moved over the
/// wait and the verdict says so.
#[test]
fn a_sequence_timeout_with_frame_gen_running_on_says_moving() {
    let (t, lines) = timeout(true);
    let w = t.wait();
    assert!(!w.frozen() && w.frames() > 0 && w.frame_end() > w.frame_start(), "{w:?}");
    assert_eq!(t.fg.fgensts, 1);
    assert!(lines.contains(&format!(
        "verdin-imx95 X12: no SeqComplete0 after 1000000 us (10001 polls): FGENABLE 0 FGENSTS 0x1, DISPIRQ0 INTERRUPTSTATUS0 0x{:08x}",
        w.irq
    )));
    assert!(lines.contains(&format!(
        "verdin-imx95 X12: frame counter {} -> {} ({} frames): moving: FrameGen0 still runs frames",
        w.frame_start(),
        w.frame_end(),
        w.frames()
    )));
    whole(&lines);
}

/// SeqComplete0 came but FGENSTS stays 1: the NotStopped path logs the same
/// report for the FGENSTS wait, before the LDB gate.
#[test]
fn a_frame_gen_that_never_stops_logs_the_same_report() {
    let mut r = rig();
    r.m.display.faults.fg_stuck = true;
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.env.lines.borrow_mut().clear();
    r.log.borrow_mut().clear();
    let e = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap_err();
    let Error::Display(display::Error::NotStopped(fg)) = e else { panic!("{e:?}") };
    let w = fg.stop.unwrap();
    let lines = r.env.lines.borrow();
    let want = [
        format!(
            "verdin-imx95 X12: FrameGen0 still running after 1000000 us (10001 polls): FGENABLE 0 FGENSTS 0x1, DISPIRQ0 INTERRUPTSTATUS0 0x{:08x}",
            w.irq
        ),
        format!(
            "verdin-imx95 X12: frame counter {} -> {} ({} frames): moving: FrameGen0 still runs frames",
            w.frame_start(),
            w.frame_end(),
            w.frames()
        ),
        format!(
            "verdin-imx95 X12: FGTIMESTAMP 0x{:08x} (frame {} line 0) -> 0x{:08x} (frame {} line 0)",
            w.ts_start,
            w.frame_start(),
            w.ts_end,
            w.frame_end()
        ),
    ];
    let at = lines.iter().position(|l| *l == want[0]).unwrap_or_else(|| panic!("{}", lines.join("\n")));
    assert_eq!(&lines[at..at + 3], &want[..]);
    whole(&lines);
}
