// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RK3588 GPU clock set path over every CLKSEL_CON(158) value (host
//! tests only).
//!
//! A CRU mock implements [`Mmio`]: CLKSEL_CON(158) and (159), CLKGATE_CON(66),
//! MODE_CON0 and the PLL_CON words of the GPLL, CPLL, AUPLL and NPLL. A write
//! to CLKSEL_CON takes the CRU's hiword mask (31:16 enable the bits 15:0 they
//! twin); every write is logged. A read of any other address panics, and so
//! does a write to one: the set path touches nothing else.
//!
//! The space walked is all 65536 values of CLKSEL_CON(158), not only the GPU
//! fields (mux, divider, PVTPLL bit), because the bits outside them must stay
//! as found. CLKSEL_CON(159) takes five values (stacks divider field 0, 1, 3,
//! 0x1f and all ones), the divider field being the only part the set path
//! reads or writes. Each pair runs under three CPLL states: 1500 MHz in
//! normal mode (what the T6 reads), 1188 MHz, and slow mode.
//!
//! For each: [`gpu_clock`] reads, [`gpu_clock_set`] runs, and
//! 1. every write goes to CLKSEL_CON(158) or (159), with a nonzero mask
//!    inside the fields the code owns, and moves no bit outside its mask;
//! 2. no PLL_CON, MODE_CON0, gate or GRF word is written (the log is
//!    checked, and the mock panics on any address it does not hold);
//! 3. on `Ok`, the clock read afterwards is an OPP rate with the stacks
//!    divider at 1, and a state that already was one is `kept` with no write;
//! 4. on `Err`, it is `NoTableRate` (the only refusal before a write; the
//!    read-back refusals need a register that does not take a write, which
//!    this CRU does) and nothing is written.

use super::rk3588::*;
use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::HashMap;

const SEL158: usize = CRU_BASE + clksel_con(158);
const SEL159: usize = CRU_BASE + clksel_con(159);

struct Cru {
    regs: RefCell<HashMap<usize, u32>>,
    writes: RefCell<Vec<(usize, u32)>>,
}

impl Cru {
    /// CPLL (`con0`, `con1`) and `mode_con0` as given; GPLL 1188 MHz, AUPLL
    /// 1500 MHz, NPLL 850 MHz (powered), all in normal mode.
    fn new(sel158: u32, sel159: u32, cpll: (u32, u32), mode_con0: u32) -> Cru {
        let mut r = HashMap::new();
        r.insert(SEL158, sel158);
        r.insert(SEL159, sel159);
        r.insert(CRU_BASE + clkgate_con(66), 0xd6);
        r.insert(CRU_BASE + MODE_CON0, mode_con0);
        for (pll, con0, con1) in [(GPLL, 0xc6, 0x42), (CPLL, cpll.0, cpll.1), (AUPLL, 0x7d, 0x41), (NPLL, 0x1a9, 0x83)]
        {
            for i in 0..7 {
                let v = match i {
                    0 => con0,
                    1 => con1,
                    6 => 0x9c00,
                    _ => 0,
                };
                r.insert(CRU_BASE + pll_con(pll.con + i), v);
            }
        }
        Cru { regs: RefCell::new(r), writes: RefCell::new(vec![]) }
    }
}

impl Mmio for Cru {
    fn read32(&self, addr: usize) -> u32 {
        *self.regs.borrow().get(&addr).unwrap_or_else(|| panic!("read of {addr:#x}"))
    }
    fn write32(&self, addr: usize, val: u32) {
        self.writes.borrow_mut().push((addr, val));
        assert!(addr == SEL158 || addr == SEL159, "write to {addr:#x}");
        let mut r = self.regs.borrow_mut();
        let old = r[&addr];
        let mask = val >> 16;
        r.insert(addr, (old & !mask) | (val & mask));
    }
    fn read8(&self, addr: usize) -> u8 {
        panic!("read8 of {addr:#x}")
    }
    fn write8(&self, addr: usize, _: u8) {
        panic!("write8 to {addr:#x}")
    }
}

fn check(sel158: u32, sel159: u32, cpll: (u32, u32), mode_con0: u32) {
    let ctx = format!("sel158 {sel158:#x} sel159 {sel159:#x} cpll {cpll:x?} mode {mode_con0:#x}");
    let cru = Cru::new(sel158, sel159, cpll, mode_con0);
    let found = gpu_clock(&cru, CRU_BASE);
    assert!(cru.writes.borrow().is_empty(), "{ctx}: the read wrote");
    let was_in_table = gpu_opp_rate(found.src_hz) && sel159 & gpu_clk::DIV_MASK == 0;
    let r = gpu_clock_set(&cru, CRU_BASE, &found);
    let writes = cru.writes.borrow().clone();
    let mut state = HashMap::from([(SEL158, sel158), (SEL159, sel159)]);
    for &(addr, val) in &writes {
        let own = match addr {
            SEL158 => gpu_clk::SET_MASK,
            SEL159 => gpu_clk::DIV_MASK,
            _ => panic!("{ctx}: a write to {addr:#x}"),
        };
        let mask = val >> 16;
        assert!(mask != 0 && mask & !own == 0, "{ctx}: mask {mask:#x} of a write to {addr:#x}");
        let old = state[&addr];
        state.insert(addr, (old & !mask) | (val & mask));
    }
    // what the mock holds is what the log makes of the start
    assert_eq!(cru.read32(SEL158), state[&SEL158], "{ctx}");
    assert_eq!(cru.read32(SEL159), state[&SEL159], "{ctx}");
    assert_eq!(cru.read32(SEL158) & !gpu_clk::SET_MASK, sel158 & !gpu_clk::SET_MASK, "{ctx}: bits outside the fields");
    match r {
        Ok(set) => {
            let now = gpu_clock(&cru, CRU_BASE);
            assert!(gpu_opp_rate(now.src_hz), "{ctx}: {} Hz is no OPP rate", now.src_hz);
            assert_eq!(now.sel159 & gpu_clk::DIV_MASK, 0, "{ctx}");
            assert_eq!(set.src_hz, now.src_hz, "{ctx}");
            assert_eq!(set.kept, was_in_table, "{ctx}");
            assert_eq!(writes.is_empty(), was_in_table, "{ctx}");
            if was_in_table {
                assert_eq!((now.sel158, now.sel159), (sel158, sel159), "{ctx}");
            }
        }
        Err(e) => {
            assert!(matches!(e, ClockRefusal::NoTableRate(_)), "{ctx}: {e:?}");
            assert!(!was_in_table && writes.is_empty(), "{ctx}: {e:?} after {writes:x?}");
        }
    }
}

#[test]
fn every_clksel_158_value_stays_in_158_159_and_the_opp_table() {
    for (cpll, mode_con0) in [((0xfa, 0x42), 0x545), ((0xc6, 0x42), 0x545), ((0xfa, 0x42), 0x445)] {
        for sel159 in [0, 1, 3, 0x1f, 0xffff] {
            for sel158 in 0..=0xffff {
                check(sel158, sel159, cpll, mode_con0);
            }
        }
    }
}

#[test]
fn armbians_state_is_kept_and_a_pvtpll_state_is_moved() {
    let cru = Cru::new(0x1f24, 0, (0xfa, 0x42), 0x545);
    let found = gpu_clock(&cru, CRU_BASE);
    assert_eq!(gpu_clock_set(&cru, CRU_BASE, &found).map(|s| s.kept), Ok(true));
    assert!(cru.writes.borrow().is_empty());
    let cru = Cru::new(0x1f24 | gpu_clk::PVTPLL, 0, (0xfa, 0x42), 0x545);
    let found = gpu_clock(&cru, CRU_BASE);
    assert!(gpu_clock_set(&cru, CRU_BASE, &found).is_ok());
    assert_eq!(*cru.writes.borrow(), vec![(SEL158, SET_SEL158)]);
}
