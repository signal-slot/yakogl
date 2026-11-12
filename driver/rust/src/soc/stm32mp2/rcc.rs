// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The non-secure side of turning the STM32MP257F's GPU on in the RCC: its
//! clock gate and its reset, both in `RCC_GPUCFGR`. Plan #322, issues #7 and
//! #1281.
//!
//! Sources, and how far they agree:
//! - Offset 0x83C, bit 1 the gate of both GPU clocks (`CK_BUS_GPU`, id 91, and
//!   `CK_KER_GPU`, id 340, one gate bit), bit 0 the reset (`GPU_R`, id 75):
//!   ST's galcore 6.4.21 platform code (r-galcore) and the open-source
//!   cross-check of TF-A, OP-TEE, Zephyr, U-Boot and Linux (r-rcc-src), as
//!   recorded in docs/notes/pre-study-stm32mp257f-dk.md section 4. The ids come
//!   from the device tree (#1281: `clocks = <&rcc 91>, <&rcc 340>`,
//!   `resets = <&rcc 75>`; `include/dt-bindings/reset/st,stm32mp25-rcc.h`).
//! - RM0457 could not be read (r-rm0457): **every offset and bit below is
//!   unconfirmed against the manual.**
//!
//! Reset polarity (`RCC_GPUCFGR` bit 0): assert sets the bit, deassert clears
//! it ([`RESET`] set = asserted). Derived from ST's reset driver, not read on
//! the board: Linux `drivers/clk/stm32/reset-stm32.c:54-100` and `:123-136`
//! (v6.6-stm32mp-r3.1) writes the bit to assert, clears it to deassert, then
//! waits for it to read back zero; its `set_clr` is 0 for `LTDC_R` and `DSI_R`
//! (docs/notes/stm32mp257f-dk-display.md, "Resets"). That is the same register
//! family, not the GPU's own entry (`GPU_R`, id 75), and RM0457 is unread, so
//! the GPU's polarity stays **unverified on the board** until the first
//! `RCC_GPUCFGR` read-back.
//!
//! The RCC base is a parameter (the device tree's `rcc` node); the caller
//! owns the power-domain order (SCMI first, then this module) and RIFSC.
//! Every change is a read-modify-write that keeps the other bits, and is
//! written even when the bit already has the wanted value.

use crate::mmio::Mmio;

/// `RCC_GPUCFGR` offset (r-galcore, r-rcc-src; unconfirmed from RM0457).
pub const GPUCFGR: usize = 0x83C;
/// Bit 0: the GPU reset (`GPU_R`, reset id 75); 1 = asserted (derived from
/// ST's reset driver, see the module documentation; not read on the board).
pub const RESET: u32 = 1 << 0;
/// Bit 1: the gate of both GPU clocks (`CK_BUS_GPU` 91 and `CK_KER_GPU` 340);
/// 1 = clocks run.
pub const CLOCK_EN: u32 = 1 << 1;

/// The GPU's RCC clock gate and reset.
pub struct GpuRcc<'a, M: Mmio> {
    mmio: &'a M,
    base: usize,
}

/// A snapshot of `RCC_GPUCFGR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub raw: u32,
}

impl Status {
    pub fn clock_enabled(&self) -> bool {
        self.raw & CLOCK_EN != 0
    }
    pub fn reset_asserted(&self) -> bool {
        self.raw & RESET != 0
    }
}

impl<'a, M: Mmio> GpuRcc<'a, M> {
    /// `base` is the RCC's address (the device tree's `rcc` node).
    pub fn new(mmio: &'a M, base: usize) -> Self {
        GpuRcc { mmio, base }
    }

    fn reg(&self) -> usize {
        self.base + GPUCFGR
    }

    pub fn enable_clock(&self) {
        self.mmio.modify32(self.reg(), CLOCK_EN, CLOCK_EN);
    }

    pub fn disable_clock(&self) {
        self.mmio.modify32(self.reg(), CLOCK_EN, 0);
    }

    pub fn assert_reset(&self) {
        self.mmio.modify32(self.reg(), RESET, RESET);
    }

    pub fn deassert_reset(&self) {
        self.mmio.modify32(self.reg(), RESET, 0);
    }

    /// One read of `RCC_GPUCFGR`.
    pub fn status(&self) -> Status {
        Status { raw: self.mmio.read32(self.reg()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, PartialEq)]
    enum Op {
        R(usize, u32),
        W(usize, u32),
    }

    struct Fake {
        reg: Cell<u32>,
        log: RefCell<Vec<Op>>,
    }

    impl Fake {
        fn new(v: u32) -> Self {
            Fake { reg: Cell::new(v), log: RefCell::new(vec![]) }
        }
    }

    impl Mmio for Fake {
        fn read32(&self, addr: usize) -> u32 {
            self.log.borrow_mut().push(Op::R(addr, self.reg.get()));
            self.reg.get()
        }
        fn write32(&self, addr: usize, val: u32) {
            self.log.borrow_mut().push(Op::W(addr, val));
            self.reg.set(val);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    const BASE: usize = 0x4400_0000;
    const A: usize = BASE + 0x83C;

    #[test]
    fn layout() {
        assert_eq!(GPUCFGR, 0x83C);
        assert_eq!((RESET, CLOCK_EN), (1, 2));
    }

    #[test]
    fn enable_clock_keeps_other_bits() {
        let m = Fake::new(0xF000_0001);
        GpuRcc::new(&m, BASE).enable_clock();
        assert_eq!(*m.log.borrow(), [Op::R(A, 0xF000_0001), Op::W(A, 0xF000_0003)]);
    }

    #[test]
    fn disable_clock_keeps_other_bits() {
        let m = Fake::new(0xF000_0003);
        GpuRcc::new(&m, BASE).disable_clock();
        assert_eq!(*m.log.borrow(), [Op::R(A, 0xF000_0003), Op::W(A, 0xF000_0001)]);
    }

    #[test]
    fn assert_reset_keeps_other_bits() {
        let m = Fake::new(0x102);
        GpuRcc::new(&m, BASE).assert_reset();
        assert_eq!(*m.log.borrow(), [Op::R(A, 0x102), Op::W(A, 0x103)]);
    }

    #[test]
    fn deassert_reset_keeps_other_bits() {
        let m = Fake::new(0x103);
        GpuRcc::new(&m, BASE).deassert_reset();
        assert_eq!(*m.log.borrow(), [Op::R(A, 0x103), Op::W(A, 0x102)]);
    }

    #[test]
    fn writes_are_made_even_when_the_bit_is_already_right() {
        let m = Fake::new(0x2);
        GpuRcc::new(&m, BASE).enable_clock();
        assert_eq!(*m.log.borrow(), [Op::R(A, 2), Op::W(A, 2)]);
    }

    #[test]
    fn full_sequence_and_status() {
        let m = Fake::new(0);
        let r = GpuRcc::new(&m, BASE);
        r.assert_reset();
        r.enable_clock();
        r.deassert_reset();
        let s = r.status();
        assert!(s.clock_enabled() && !s.reset_asserted());
        r.disable_clock();
        assert_eq!(
            *m.log.borrow(),
            [
                Op::R(A, 0),
                Op::W(A, 1),
                Op::R(A, 1),
                Op::W(A, 3),
                Op::R(A, 3),
                Op::W(A, 2),
                Op::R(A, 2),
                Op::R(A, 2),
                Op::W(A, 0),
            ]
        );
    }

    #[test]
    fn only_the_gpucfgr_offset_is_touched() {
        let m = Fake::new(0);
        let r = GpuRcc::new(&m, 0x1000);
        r.enable_clock();
        r.status();
        let want = 0x1000 + 0x83C;
        assert!(m.log.borrow().iter().all(|o| match o {
            Op::R(a, _) | Op::W(a, _) => *a == want,
        }));
    }
}
