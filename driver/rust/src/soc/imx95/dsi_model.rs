// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A register model of the i.MX 95's DSI output blocks for the host tests
//! (`combo_phy_tests`, `board::verdin_imx95_dsi_tests`): the DW host, the
//! stream, master and PHY CSRs, the CSI-2 host 1 and TPM6 as one address
//! space, every write logged in order. Joined to the mock System Manager
//! (`sm_mock`), as `model_imx95_display` is: an access to a CAMERA-mix
//! block (the host, the CSRs, the CSI-2 host) with the CAMERA domain (3)
//! off is refused (logged as [`Ev::Refused`], a write dropped, a read 0);
//! TPM6 is outside that mix.
//!
//! The two PHY test interfaces (the DSI host's `PHY_TST_CTRL0/1` and the
//! CSI-2 host's +0x50/+0x54) are decoded as the D-PHY latches them
//! (`imx95-mipi-dsi.c:436-498`'s description, facts only): with TESTEN high
//! a falling TESTCLK latches TESTDIN as a test code; with TESTEN low a
//! rising TESTCLK writes TESTDIN to the latched code, code 0x00 holding the
//! high byte of the codes that follow; TESTDOUT reads the latched code's
//! register; TESTCLR empties the registers. A test sees the PHY's
//! registers as (code, value), whatever pin sequence wrote them.

use super::combo_phy::{regs as c, Bases};
use super::sm_mock::Sm;
use crate::ip::dw_mipi_dsi::regs as d;
use crate::ip::tpm_pwm::regs as t;
use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::vec::Vec;

/// One event of a run, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    /// A 32-bit write: address, value.
    W(usize, u32),
    /// A PHY register written through a test interface ("dsi" or "csi"):
    /// test code, value.
    Code(&'static str, u16, u8),
    /// An SCMI request: protocol, message.
    Scmi(u8, u8),
    /// The bridge: an I2C register write, or its EN line.
    I2c(u8, u8),
    Gpio(u32, bool),
    /// An access the model refused (the CAMERA domain off): the address.
    Refused(usize),
}

pub type Log = Rc<RefCell<Vec<Ev>>>;

/// One test interface's pins and the PHY registers behind it.
#[derive(Default, Debug)]
pub struct TestIf {
    clk: bool,
    en: bool,
    din: u8,
    code: u8,
    page: u8,
    pub regs: HashMap<u16, u8>,
    /// A register whose writes are lost (reads 0).
    pub stuck: Option<u16>,
}

impl TestIf {
    /// A `CTRL0` write; returns the register written, if any.
    fn ctrl0(&mut self, v: u32) -> Option<(u16, u8)> {
        if v & d::PHY_TESTCLR != 0 {
            self.regs.clear();
            self.page = 0;
        }
        let clk = v & d::PHY_TESTCLK != 0;
        let mut wrote = None;
        if self.clk && !clk && self.en {
            self.code = self.din;
        } else if !self.clk && clk && !self.en {
            if self.code == 0 {
                self.page = self.din;
            } else {
                let at = u16::from(self.page) << 8 | u16::from(self.code);
                if self.stuck != Some(at) {
                    self.regs.insert(at, self.din);
                }
                wrote = Some((at, self.din));
            }
        }
        self.clk = clk;
        wrote
    }

    fn ctrl1(&mut self, v: u32) {
        self.en = v & d::PHY_TESTEN != 0;
        self.din = v as u8;
    }

    fn dout(&self) -> u8 {
        *self.regs.get(&(u16::from(self.page) << 8 | u16::from(self.code))).unwrap_or(&0)
    }
}

pub struct Soc {
    pub bases: Bases,
    pub tpm: usize,
    pub regs: RefCell<HashMap<usize, u32>>,
    pub dsi_if: RefCell<TestIf>,
    pub csi_if: RefCell<TestIf>,
    pub log: Log,
    /// The host's VERSION.
    pub version: u32,
    pub never_locks: bool,
    pub sm: Rc<RefCell<Sm>>,
}

/// A version the i.MX 95's host may read (U18: unknown until a board read).
pub const VERSION_131: u32 = d::HWVER_131 | 0x2a;

impl Soc {
    pub fn new(log: &Log, bases: Bases, tpm: usize, sm: Rc<RefCell<Sm>>) -> Soc {
        Soc {
            bases,
            tpm,
            regs: RefCell::new(HashMap::new()),
            dsi_if: RefCell::default(),
            csi_if: RefCell::default(),
            log: log.clone(),
            version: VERSION_131,
            never_locks: false,
            sm,
        }
    }

    /// An address in the CAMERA mix while the domain is off.
    fn refused(&self, a: usize) -> bool {
        let b = self.bases;
        let camera =
            [(b.dsi, 0x1_0000), (b.stream, 0x1_0000), (b.master, 0x1_0000), (b.phy_csr, 0x200), (b.csi1, 0x1_0000)]
                .iter()
                .any(|&(base, size)| (base..base + size).contains(&a));
        if camera && !self.sm.borrow().on(3) {
            self.log.borrow_mut().push(Ev::Refused(a));
            return true;
        }
        false
    }

    pub fn reg(&self, a: usize) -> u32 {
        *self.regs.borrow().get(&a).unwrap_or(&0)
    }

    /// The writes to one block (offset, value), in order.
    pub fn block_writes(&self, base: usize, size: usize) -> Vec<(u32, u32)> {
        self.log
            .borrow()
            .iter()
            .filter_map(|e| match *e {
                Ev::W(a, v) if (base..base + size).contains(&a) => Some(((a - base) as u32, v)),
                _ => None,
            })
            .collect()
    }

    /// The test codes written through one interface, in order.
    pub fn codes(&self, which: &str) -> Vec<(u16, u8)> {
        self.log
            .borrow()
            .iter()
            .filter_map(|e| match *e {
                Ev::Code(w, code, v) if w == which => Some((code, v)),
                _ => None,
            })
            .collect()
    }
}

impl Mmio for Soc {
    fn read32(&self, a: usize) -> u32 {
        if self.refused(a) {
            return 0;
        }
        let b = self.bases;
        let dsi = |off: u32| b.dsi + off as usize;
        if a == dsi(d::VERSION) {
            return self.version;
        }
        if a == dsi(d::PHY_STATUS) {
            let up = self.reg(dsi(d::PHY_RSTZ)) == 0xf;
            return if up { d::PHY_STOP_STATE_CLK_LANE | if self.never_locks { 0 } else { d::PHY_LOCK } } else { 0 };
        }
        if a == dsi(d::CMD_PKT_STATUS) {
            return d::GEN_CMD_EMPTY | d::GEN_PLD_W_EMPTY;
        }
        if a == dsi(d::PHY_TST_CTRL1) {
            return self.reg(a) & !0xff00 | u32::from(self.dsi_if.borrow().dout()) << 8;
        }
        if a == b.csi1 + c::CSI_PHY_TEST_CTRL1 as usize {
            return self.reg(a) & !0xff00 | u32::from(self.csi_if.borrow().dout()) << 8;
        }
        if a == self.tpm + t::PARAM as usize {
            return 32 << 16 | 4;
        }
        self.reg(a)
    }

    fn write32(&self, a: usize, v: u32) {
        if self.refused(a) {
            return;
        }
        let b = self.bases;
        self.log.borrow_mut().push(Ev::W(a, v));
        self.regs.borrow_mut().insert(a, v);
        let code = if a == b.dsi + d::PHY_TST_CTRL0 as usize {
            self.dsi_if.borrow_mut().ctrl0(v).map(|w| ("dsi", w))
        } else if a == b.dsi + d::PHY_TST_CTRL1 as usize {
            self.dsi_if.borrow_mut().ctrl1(v);
            None
        } else if a == b.csi1 + c::CSI_PHY_TEST_CTRL0 as usize {
            self.csi_if.borrow_mut().ctrl0(v).map(|w| ("csi", w))
        } else if a == b.csi1 + c::CSI_PHY_TEST_CTRL1 as usize {
            self.csi_if.borrow_mut().ctrl1(v);
            None
        } else {
            None
        };
        if let Some((which, (code, val))) = code {
            self.log.borrow_mut().push(Ev::Code(which, code, val));
        }
    }

    fn read8(&self, _: usize) -> u8 {
        unreachable!()
    }
    fn write8(&self, _: usize, _: u8) {
        unreachable!()
    }
}
