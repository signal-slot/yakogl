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
//! - RM0457 could not be read (r-rm0457): the offset and the bit names
//!   (`GPURST` bit 0, `GPUEN` bit 1, `GPULPEN` bit 2) now come from the headers
//!   of TF-A (`v2.10-stm32mp-r3.1`, `stm32mp25_rcc.h`) and OP-TEE
//!   (`4.0.0-stm32mp-r3.1`), as `model_stm32mp2.rs` cites them; only the
//!   manual's meaning and reset value of the bits are unconfirmed.
//!
//! Reset polarity (`RCC_GPUCFGR` bit 0), **read on the board**: set = asserted
//! ([`RESET`]), clear = released. The DK session of 2026-11-21 (record
//! `20261121-1107-mp2_gpu-9e520020`, commits 2da6568d and 226cd9a2, step 4 of
//! docs/notes/stm32mp257f-dk-bringup.md) read 0x7 with the reset held and 0x6
//! after the release, and the GPU then reached idle. The first run
//! (`20261121-1059-mp2_gpu-e0ee66cb`) read the register once right after the
//! clearing write and still saw 0x7, so the release is not instant;
//! [`GpuRcc::wait_reset_released`] (2da6568d) reads until bit 0 clears. This
//! matches ST's reset driver, Linux `drivers/clk/stm32/reset-stm32.c:54-100`
//! and `:123-136` (v6.6-stm32mp-r3.1): write the bit to assert, clear it to
//! deassert, then wait for it to read back zero.
//!
//! Bit 2 (`GPULPEN` in the TF-A and OP-TEE headers; presumably the GPU's
//! clock low-power enable) was already set at entry on the board (0x6, bit 1
//! set too) and the probe does not write it. Its meaning and reset value are
//! **unconfirmed** (RM0457's text for it is unread). Every change here is a
//! read-modify-write, so bit 2 is preserved; nothing clears it.
//!
//! The RCC base is a parameter (the device tree's `rcc` node); the caller
//! owns the power-domain order (SCMI first, then this module) and RIFSC.
//! Every change is a read-modify-write that keeps the other bits, and is
//! written even when the bit already has the wanted value.

use crate::mmio::Mmio;

/// `RCC_GPUCFGR` offset (r-galcore, r-rcc-src; unconfirmed from RM0457).
pub const GPUCFGR: usize = 0x83C;
/// Bit 0: the GPU reset (`GPU_R`, reset id 75); 1 = asserted (matches
/// ST's reset driver and read on the board: 0x7 held, 0x6 released; see the
/// module documentation).
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

    /// After [`Self::deassert_reset`]: read `RCC_GPUCFGR` until the reset bit
    /// reads clear, at most `max` reads (at least one). Returns the last read
    /// and the number of reads. The release is not immediate on the board:
    /// read once right after the clearing write, the register showed
    /// 0x00000007 (reset still set) on the DK (2026-11-21), and ST's reset
    /// driver also waits for the bit to read back zero after clearing it.
    pub fn wait_reset_released(&self, max: u32) -> (Status, u32) {
        let mut s = self.status();
        let mut n = 1;
        while s.reset_asserted() && n < max {
            s = self.status();
            n += 1;
        }
        (s, n)
    }
}

/// Most reads of `RCC_GPUCFGR` [`GpuRcc::wait_reset_released`] makes before
/// the reset counts as held (a bound in reads, not time: the release time
/// has not been measured; the count the board needed is logged).
pub const RELEASE_READS: u32 = 100_000;

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

    /// A register whose reset bit reads set for `lag` reads after it was
    /// cleared, as the DK's read right after the release showed (0x7).
    struct Lagging {
        reg: Cell<u32>,
        lag: Cell<u32>,
        reads: Cell<u32>,
    }
    impl Mmio for Lagging {
        fn read32(&self, _: usize) -> u32 {
            self.reads.set(self.reads.get() + 1);
            if self.lag.get() > 0 {
                self.lag.set(self.lag.get() - 1);
                return self.reg.get() | RESET;
            }
            self.reg.get()
        }
        fn write32(&self, _: usize, val: u32) {
            self.reg.set(val);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[test]
    fn wait_reset_released_reads_until_the_bit_clears() {
        let m = Lagging { reg: Cell::new(0x6), lag: Cell::new(3), reads: Cell::new(0) };
        let (s, n) = GpuRcc::new(&m, BASE).wait_reset_released(RELEASE_READS);
        assert_eq!((s.raw, n, m.reads.get()), (0x6, 4, 4));
        assert!(s.clock_enabled() && !s.reset_asserted());
    }

    #[test]
    fn wait_reset_released_stops_at_the_bound_and_reads_at_least_once() {
        let m = Lagging { reg: Cell::new(0x6), lag: Cell::new(u32::MAX), reads: Cell::new(0) };
        let (s, n) = GpuRcc::new(&m, BASE).wait_reset_released(5);
        assert_eq!((s.raw, n, m.reads.get()), (0x7, 5, 5));
        let m = Lagging { reg: Cell::new(0x2), lag: Cell::new(0), reads: Cell::new(0) };
        assert_eq!(GpuRcc::new(&m, BASE).wait_reset_released(0).1, 1);
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
