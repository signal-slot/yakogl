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

/// The display model for its blocks (FrameGen0's sequence complete is the
/// model's, in its DISPIRQ0 block: `Display::seq_after`); TPM5 and GPIO5 as
/// a register file (TPM5's PARAM reads a 32-bit counter, as the i.MX 95's).
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
const FGSLR: usize = dm::DPU + 0x2b_0090;
/// `DISPIRQ0_INTERRUPTENABLE0`, `_INTERRUPTCLEAR0`, `_INTERRUPTSTATUS0`
/// (`PERI_DISPLAY_SEERIS.h`, NXP's `dpu95-core.c` `dpu_base + 0x381000`),
/// the common-control `IRQ_INTERRUPTSTATUS0` and
/// `DPU95_IRQ_DISENGCFG_SEQCOMPLETE0` (`dpu95.h`), restated.
const DIRQ0_ENABLE0: u32 = 0x38_1008;
const DIRQ0_CLEAR0: u32 = 0x38_1020;
const DIRQ0_STATUS0: u32 = 0x38_102c;
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
    // dpu95_crtc_disable sends none)
    assert!(!log.borrow()[..gate].contains(&Ev::W(FGSLR, 1)));
    // in the display blocks' own log (reads too): FGENABLE 0, the
    // sequence complete in DISPIRQ0, the FGENSTS read, then the LDB gate
    let dlog = r.m.display.log();
    let at = |a: Access| dlog.iter().position(|x| *x == a).unwrap_or_else(|| panic!("{a:?} not in the log"));
    let fg_off = at(Access::W("dpu", 0x2b_008c, 0));
    let seq = dlog.iter().position(|x| matches!(x, Access::R("dpu", DIRQ0_STATUS0, v) if v & SEQ_BIT != 0)).unwrap();
    let ensts = dlog.iter().position(|x| matches!(x, Access::R("dpu", 0x2b_0094, _))).unwrap();
    let gate = at(Access::W("ldb", 0x0, 0x1E));
    assert!(fg_off < seq && seq < ensts && ensts < gate, "{dlog:?}");
    // the frame counter is read just before FGENABLE = 0 (for the
    // sequence wait's report), then before the first FGENSTS poll and again
    // when that wait ends, all before the LDB gate
    let ts: Vec<usize> =
        dlog.iter().enumerate().filter(|(_, x)| matches!(x, Access::R("dpu", 0x2b_0098, _))).map(|(i, _)| i).collect();
    let before_gate: Vec<usize> = ts.into_iter().filter(|&i| i < gate).collect();
    assert!(before_gate.len() == 3 && before_gate[0] < fg_off, "{dlog:?}");
    assert!(before_gate[1] > seq && before_gate[1] < ensts, "{dlog:?}");
    assert!(before_gate[2] > ensts, "{dlog:?}");
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

/// The down waits for FrameGen0's sequence complete (DISPIRQ0
/// `INTERRUPTSTATUS0` bit 20, where Linux takes it) before it touches the
/// LDB, in Linux's `dpu95_crtc_disable` order: a stale bit cleared and the
/// event enabled in DISPIRQ0, `FGENABLE` = 0 with no `FGSLR` token, the bit
/// polled until it sets (here on the fifth read), cleared, the event
/// masked again, and only then the LDB's clocks gated.
#[test]
fn down_gates_the_ldb_only_after_frame_gen_reports_its_sequence_complete() {
    let mut r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.display.seq_after = Some(5);
    r.log.borrow_mut().clear();
    r.m.display.clear_log();
    r.env.lines.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    assert_eq!(lv::SEQ_COMPLETE0, SEQ_BIT);
    assert_eq!(lv::SEQ_COMPLETE_BUDGET_US, 1_000_000);
    assert_eq!(lv::DISP_IRQ0 + lv::disp_irq::INTERRUPTSTATUS0, DIRQ0_STATUS0);

    let dlog = r.m.display.log();
    let at = |a: Access| dlog.iter().position(|x| *x == a).unwrap_or_else(|| panic!("{a:?} not in the log"));
    let cleared = at(Access::W("dpu", DIRQ0_CLEAR0, SEQ_BIT));
    let enabled = at(Access::W("dpu", DIRQ0_ENABLE0, SEQ_BIT));
    let fg_off = at(Access::W("dpu", 0x2b_008c, 0));
    let masked = at(Access::W("dpu", DIRQ0_ENABLE0, 0));
    let gate = at(Access::W("ldb", 0x0, 0x1E));
    let polls: Vec<usize> = dlog
        .iter()
        .enumerate()
        .filter(|(i, a)| *i > fg_off && matches!(a, Access::R("dpu", DIRQ0_STATUS0, _)))
        .map(|(i, _)| i)
        .collect();
    assert!(cleared < enabled && enabled < fg_off, "{dlog:?}");
    // no shadow token between the stop and the gate
    assert!(!dlog[fg_off..gate].contains(&Access::W("dpu", 0x2b_0090, 1)), "{dlog:?}");
    // the common-control word is not what is waited on
    assert!(!dlog[fg_off..gate].iter().any(|a| matches!(a, Access::R("dpu", COMCTRL_STATUS0, _))), "{dlog:?}");
    // five polls, the fifth saw the bit, all before the gate; no LDB
    // register touched before the fifth
    assert_eq!(polls.len(), 5, "{dlog:?}");
    assert!(matches!(dlog[polls[4]], Access::R(_, _, v) if v & SEQ_BIT != 0), "{dlog:?}");
    assert!(polls[4] < gate, "{dlog:?}");
    assert!(!dlog[..polls[4]].iter().any(|a| matches!(a, Access::W("ldb", ..))), "{dlog:?}");
    // the bit cleared after it was seen, the event masked, then the gate
    let reclear = dlog.iter().rposition(|a| *a == Access::W("dpu", DIRQ0_CLEAR0, SEQ_BIT)).unwrap();
    assert!(polls[4] < reclear && reclear < masked && masked < gate, "{dlog:?}");
    assert_eq!(r.m.display.peek("dpu", DIRQ0_STATUS0) & SEQ_BIT, 0);
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
    let lines = r.env.lines.borrow();
    assert_eq!(
        &lines[..5],
        &[
            "verdin-imx95 X12 down: DISPIRQ0 LOCKSTATUS 0x0: unlocked, privilege off, freeze off",
            "verdin-imx95 X12 down: DISPIRQ0 ENABLE0 0x00100000 (wrote 0x00100000)",
            "verdin-imx95 X12 down: FGENABLE 0 (no FGSLR token), FGTIMESTAMP 0x0000c000",
            "verdin-imx95 X12 down: waiting <= 1000000 us for SeqComplete0 (DPU+0x38102c bit 20)",
            "verdin-imx95 X12 down: SeqComplete0 after 400 us (5 polls), FGENSTS stopped",
        ],
        "{lines:?}"
    );
    drop(lines);
    assert!(!sm.sm.borrow().on(13) && !sm.sm.borrow().enabled(38));
    assert!(r.m.display.refused().is_empty());
}

/// FrameGen0's sequence never completes: the down gives up after its 1 s,
/// returns the timeout with what the DPU read, and leaves the LDB's clocks
/// ungated and the display's power on.
#[test]
fn a_sequence_that_never_completes_times_out_and_leaves_the_ldb_clocks_on() {
    let mut r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.display.seq_after = None;
    r.log.borrow_mut().clear();
    let e = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap_err();
    let Error::SeqTimeout { waited_us, irq, comctrl, enable, lock, fg } = e else { panic!("{e:?}") };
    assert_eq!(waited_us, lv::SEQ_COMPLETE_BUDGET_US);
    assert_eq!((irq & SEQ_BIT, comctrl & SEQ_BIT), (0, 0));
    // the block was open: the enable landed
    assert_eq!((enable, lock), (SEQ_BIT, lv::Dirq0Lock(0)));
    let w = fg.stop.expect("the wait is in the error");
    assert_eq!(
        (w.waited_us, w.polls),
        (lv::SEQ_COMPLETE_BUDGET_US, lv::SEQ_COMPLETE_BUDGET_US / dpu::POLL_STEP_US + 1)
    );
    // the event is masked again on the way out
    assert_eq!(r.m.display.peek("dpu", DIRQ0_ENABLE0), 0);
    assert_eq!(fg.fgenable, 0);
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
    let lines = r.env.lines.borrow();
    let at = lines.iter().position(|l| l.contains("no SeqComplete0")).expect("a timeout line");
    assert_eq!(
        &lines[at..],
        &[
            "verdin-imx95 X12 down: no SeqComplete0 after 1000000 us (10001 polls)",
            "verdin-imx95 X12 down: FGENABLE 0 FGENSTS 0x0 FGCHSTAT 0x0",
            "verdin-imx95 X12 down: DISPIRQ0 STATUS0 0x00000000 ENABLE0 0x00100000",
            "verdin-imx95 X12 down: DISPIRQ0 LOCKSTATUS 0x0",
            "verdin-imx95 X12 down: COMCTRL IRQ_INTERRUPTSTATUS0 0x00000008",
            "verdin-imx95 X12: frame counter 3 -> 3 (0 frames): frozen, the pixel clock is dead",
            "verdin-imx95 X12: FGTIMESTAMP 0x0000c000 -> 0x0000c000",
            "verdin-imx95 X12 down: the LDB and the power stay on",
        ],
        "{lines:?}"
    );
}

/// As the board after 211ed16a: the sequence complete never shows in the
/// common-control `IRQ_INTERRUPTSTATUS0`. A down that waited there (or on
/// anything but DISPIRQ0) would time out; this one completes, and the
/// model's sequence needs the stop to have come while FrameGen0 ran.
#[test]
fn the_sequence_complete_is_only_in_the_display_interrupt_block() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    // before the stop neither word has it
    assert_eq!(r.m.read32(dm::DPU + DIRQ0_STATUS0 as usize) & SEQ_BIT, 0);
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let dlog = r.m.display.log();
    assert!(dlog.iter().any(|a| matches!(a, Access::R("dpu", DIRQ0_STATUS0, v) if v & SEQ_BIT != 0)));
    assert!(!dlog.iter().any(|a| matches!(a, Access::R("dpu", COMCTRL_STATUS0, v) if v & SEQ_BIT != 0)));
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
    let at = lines.iter().position(|l| l.contains("FrameGen0 stopped after")).expect("a success line");
    assert!(lines[at].contains(" us (") && lines[at].contains(" polls)"), "{lines:?}");
    assert!(lines[at + 1].starts_with("verdin-imx95 X12: frame counter "), "{lines:?}");
    assert!(lines[at + 2].starts_with("verdin-imx95 X12: FGTIMESTAMP 0x"), "{lines:?}");
}

const DIRQ0_LOCKUNLOCK: u32 = 0x38_1000;
const DIRQ0_LOCKSTATUS: u32 = 0x38_1004;

/// The DISPIRQ0 lock words are where `PERI_DISPLAY_SEERIS.h` has them and
/// the keys are its (and Linux's `CMDSEQ_LOCKUNLOCK`) values.
#[test]
fn the_dispirq0_lock_words_and_keys() {
    assert_eq!(lv::DISP_IRQ0 + lv::dirq_lock::LOCKUNLOCK, DIRQ0_LOCKUNLOCK);
    assert_eq!(lv::DISP_IRQ0 + lv::dirq_lock::LOCKSTATUS, DIRQ0_LOCKSTATUS);
    assert_eq!((lv::dirq_lock::LOCK_KEY, lv::dirq_lock::UNLOCK_KEY), (dm::key::LOCK, dm::key::UNLOCK));
    assert_eq!(lv::dirq_lock::FREEZE_KEY, dm::key::FREEZE);
    assert_eq!(
        (lv::dirq_lock::PRIVILEGE_ON_KEY, lv::dirq_lock::PRIVILEGE_OFF_KEY),
        (dm::key::PRIVILEGE_ON, dm::key::PRIVILEGE_OFF)
    );
    let l = lv::Dirq0Lock(0x111);
    assert!(l.locked() && l.privileged() && l.frozen());
    assert!(!lv::Dirq0Lock(0).locked());
}

/// Lock DISPIRQ0 in the model the way a key writer would: the counter
/// from 1 to 0.
fn lock_dirq0(m: &Joined) {
    m.write32(dm::DPU + DIRQ0_LOCKUNLOCK as usize, lv::dirq_lock::LOCK_KEY);
    assert_eq!(m.read32(dm::DPU + DIRQ0_LOCKSTATUS as usize) & lv::dirq_lock::LOCK_STATUS, 1);
}

/// Unlocked (the reset state): LOCKSTATUS is read before the block is
/// written, the down reports it unlocked, writes no key and its enable lands.
#[test]
fn an_unlocked_dispirq0_is_read_before_the_enable_and_reported() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.display.clear_log();
    r.env.lines.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let dlog = r.m.display.log();
    let at = |a: &Access| dlog.iter().position(|x| x == a).unwrap_or_else(|| panic!("{a:?} not in the log"));
    let read = at(&Access::R("dpu", DIRQ0_LOCKSTATUS, 0));
    let first_write =
        dlog.iter().position(|a| matches!(a, Access::W("dpu", o, _) if (0x38_1000..0x38_1038).contains(o))).unwrap();
    assert!(read < first_write, "{dlog:?}");
    assert!(!dlog.iter().any(|a| matches!(a, Access::W("dpu", DIRQ0_LOCKUNLOCK, _))), "no key written: {dlog:?}");
    // the enable read back as written
    assert!(dlog.contains(&Access::R("dpu", DIRQ0_ENABLE0, SEQ_BIT)), "{dlog:?}");
    let lines = r.env.lines.borrow();
    assert!(
        lines.contains(&"verdin-imx95 X12 down: DISPIRQ0 LOCKSTATUS 0x0: unlocked, privilege off, freeze off".into())
    );
    assert!(!lines.iter().any(|l| l.contains("DISPIRQ0 locked")), "{lines:?}");
    // the bit was cleared after the wait
    assert_eq!(r.m.display.peek("dpu", DIRQ0_STATUS0) & SEQ_BIT, 0);
}

/// Locked: the down says so and why (its clear and enable do not land;
/// Linux never unlocks the block, so it does not either), writes no key,
/// and still waits, as Linux would. The status latches without the
/// enable, so the wait sees the bit; the clear after it does not land.
#[test]
fn a_locked_dispirq0_is_reported_and_left_locked() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    lock_dirq0(&r.m);
    r.m.display.clear_log();
    r.env.lines.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let dlog = r.m.display.log();
    assert!(!dlog.iter().any(|a| matches!(a, Access::W("dpu", DIRQ0_LOCKUNLOCK, _))), "no key written: {dlog:?}");
    assert!(dlog.contains(&Access::R("dpu", DIRQ0_ENABLE0, 0)), "the enable did not land: {dlog:?}");
    assert_eq!(r.m.display.peek("dpu", DIRQ0_STATUS0) & SEQ_BIT, SEQ_BIT, "the clear did not land");
    // read once, locked, before anything was written to the block
    assert_eq!(dlog[0], Access::R("dpu", DIRQ0_LOCKSTATUS, 1), "{dlog:?}");
    let lines = r.env.lines.borrow();
    assert_eq!(
        &lines[..4],
        &[
            "verdin-imx95 X12 down: DISPIRQ0 LOCKSTATUS 0x1: locked, privilege off, freeze off",
            "verdin-imx95 X12 down: DISPIRQ0 locked: its CLEAR0 and ENABLE0 writes are ignored",
            "verdin-imx95 X12 down: Linux's dpu95 never unlocks DISPIRQ0; not unlocked here",
            "verdin-imx95 X12 down: DISPIRQ0 ENABLE0 0x00000000 (wrote 0x00100000)",
        ],
        "{lines:?}"
    );
}

/// Locked with a sequence complete left from before: the clear cannot take
/// it away, so the down says the wait cannot tell it from this stop's.
#[test]
fn a_stale_seq_complete_under_the_lock_is_reported() {
    let r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    // a stop and a restart leave the bit set
    r.m.write32(FGENABLE, 0);
    assert_eq!(r.m.read32(dm::DPU + DIRQ0_STATUS0 as usize) & SEQ_BIT, SEQ_BIT);
    r.m.write32(FGENABLE, 1);
    lock_dirq0(&r.m);
    r.env.lines.borrow_mut().clear();
    display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap();
    let lines = r.env.lines.borrow();
    assert!(
        lines.contains(&"verdin-imx95 X12 down: stale SeqComplete0 under the lock: the wait cannot tell".into()),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("SeqComplete0 after 0 us (1 polls)")), "{lines:?}");
}

/// Locked, privileged and frozen, and the sequence never completes: the
/// timeout carries the lock and the enable as read back, and says the
/// enable did not land.
#[test]
fn a_locked_dispirq0_that_times_out_names_the_lock() {
    let mut r = rig();
    let mut sm = r.sm;
    let cfg = Config::verdin(FB);
    let up = display_up(&mut sm, &r.m, &r.env, &cfg).unwrap();
    r.m.write32(dm::DPU + DIRQ0_LOCKUNLOCK as usize, lv::dirq_lock::PRIVILEGE_ON_KEY);
    lock_dirq0(&r.m);
    r.m.write32(dm::DPU + DIRQ0_LOCKUNLOCK as usize, lv::dirq_lock::FREEZE_KEY);
    // frozen: an unlock key has no effect
    r.m.write32(dm::DPU + DIRQ0_LOCKUNLOCK as usize, lv::dirq_lock::UNLOCK_KEY);
    r.m.display.seq_after = None;
    r.env.lines.borrow_mut().clear();
    let e = display_down(&mut sm, &r.m, &r.env, &cfg, &up.stream).unwrap_err();
    let Error::SeqTimeout { enable, lock, .. } = e else { panic!("{e:?}") };
    assert_eq!((enable, lock), (0, lv::Dirq0Lock(0x111)));
    assert!(lock.locked() && lock.privileged() && lock.frozen());
    let lines = r.env.lines.borrow();
    assert_eq!(lines[0], "verdin-imx95 X12 down: DISPIRQ0 LOCKSTATUS 0x111: locked, privilege on, freeze on");
    assert!(
        lines.contains(&"verdin-imx95 X12 down: DISPIRQ0 STATUS0 0x00000000 ENABLE0 0x00000000".into()),
        "{lines:?}"
    );
    assert!(lines.contains(&"verdin-imx95 X12 down: DISPIRQ0 LOCKSTATUS 0x111".into()), "{lines:?}");
    assert!(lines.contains(&"verdin-imx95 X12 down: DISPIRQ0 was locked: the enable did not land".into()), "{lines:?}");
    assert_eq!(lines.last().unwrap(), "verdin-imx95 X12 down: the LDB and the power stay on");
}

/// Every line the down prints, on every path (completed, FrameGen0 still
/// running, timed out open, timed out locked and frozen, a stale bit under
/// the lock),
/// is one value group under the console's cut.
#[test]
fn every_down_line_fits_the_console() {
    let paths: [fn(&mut Rig); 5] = [
        |_| {},
        |r| r.m.display.faults.fg_stuck = true,
        |r| r.m.display.seq_after = None,
        |r| {
            r.m.write32(dm::DPU + DIRQ0_LOCKUNLOCK as usize, lv::dirq_lock::PRIVILEGE_ON_KEY);
            lock_dirq0(&r.m);
            r.m.write32(dm::DPU + DIRQ0_LOCKUNLOCK as usize, lv::dirq_lock::FREEZE_KEY);
            r.m.display.seq_after = None;
        },
        |r| {
            r.m.write32(FGENABLE, 0);
            r.m.read32(dm::DPU + DIRQ0_STATUS0 as usize);
            r.m.write32(FGENABLE, 1);
            lock_dirq0(&r.m);
        },
    ];
    let mut seen = 0;
    for (i, f) in paths.iter().enumerate() {
        let mut r = rig();
        let cfg = Config::verdin(FB);
        let up = display_up(&mut r.sm, &r.m, &r.env, &cfg).unwrap();
        f(&mut r);
        r.env.lines.borrow_mut().clear();
        let res = display_down(&mut r.sm, &r.m, &r.env, &cfg, &up.stream);
        assert_eq!(res.is_ok(), i == 0 || i == 4, "path {i}: {res:?}");
        let lines = r.env.lines.borrow();
        assert!(lines.len() >= 5, "path {i}: {lines:?}");
        for l in lines.iter() {
            assert!(l.chars().count() <= lv::DOWN_LINE_MAX, "path {i}: {} chars: {l}", l.chars().count());
            assert!(!l.ends_with("..."), "path {i}: cut: {l}");
        }
        seen += lines.len();
    }
    const _: () = assert!(lv::DOWN_LINE_MAX < 100);
    assert!(seen > 30);
}
