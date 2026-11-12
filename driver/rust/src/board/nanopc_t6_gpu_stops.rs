// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! One host test per [`Stop`] cause of [`gpu_up`] that the tests of
//! `nanopc_t6_gpu.rs` do not reach: `SpiGate` (both gates), `Rk806Read` (at
//! the hand-over reads, the `POWER_EN0` poll after the enable and the
//! `BUCK1_ON_VSEL` read after it), `PowerDomain`, `Reset` and `L2`. Each test
//! drives `gpu_up` into exactly that stop over a small register mock (this
//! file's own, over the public [`Mmio`] trait), asserts the stop with its
//! refusal and detail as `gpu_up` defines them, and that nothing past the stop
//! was written or read. [`every_stop_variant_is_listed`] lists every variant
//! through an exhaustive `match`: a new variant does not compile until it is
//! placed there.

use super::nanopc_t6_gpu::*;
use crate::gpu::mali;
use crate::ip::{rk806, rockchip_spi};
use crate::mmio::Mmio;
use crate::soc::rk3588::{self, clkgate_con, clksel_con, pd_gpu, pll_con, CRU_BASE, MODE_CON0, PMU_BASE};
use std::cell::RefCell;
use std::collections::HashMap;

const SPI: usize = rk3588::SPI2_BASE;
const QOS: usize = rk3588::QOS_GPU[0];
const GPU: usize = 0xfb00_0000;
const B: Bases = Bases { cru: CRU_BASE, pmu: PMU_BASE, spi2: SPI, qos: QOS, gpu: GPU };

/// The T6 reduced to what the stops need: the CRU and PMU with their hiword
/// masks, the RK806 behind SPI2, and the GPU as plain registers. Knobs make
/// one step fail.
struct Board {
    regs: RefCell<HashMap<usize, u32>>,
    writes: RefCell<Vec<(usize, u32)>>,
    /// Reads in the GPU and QoS ranges.
    gpu_reads: RefCell<usize>,
    rk806: RefCell<HashMap<u16, u8>>,
    frame: RefCell<Vec<u8>>,
    rxq: RefCell<std::collections::VecDeque<u8>>,
    /// Receive-only phases seen (one per RK806 read).
    reads: RefCell<usize>,
    /// The RK806 answers no read from this one on.
    deaf_from_read: Option<usize>,
    /// Bits of CLKGATE_CON(14) whose writes are ignored.
    stuck_gate14: u32,
    /// Writes to `PWR_GATE` do not power PD_GPU.
    pd_dead: bool,
    /// `GPU_CMD` does not reset the GPU.
    reset_dead: bool,
    /// `GPU_INT_CLEAR` does not clear `RAWSTAT`.
    sticky_raw: bool,
    /// The L2 never reports ready.
    l2_dead: bool,
    l2_on: RefCell<bool>,
}

impl Board {
    /// Armbian's hand-over (CLKSEL_CON(158) 0x1f24, CPLL on, DCDC1 on at
    /// 675 mV), PD_GPU off.
    fn new() -> Board {
        let mut r = HashMap::new();
        for (a, v) in [
            (CRU_BASE + clksel_con(158), 0x1f24),
            (CRU_BASE + clksel_con(159), 0),
            (CRU_BASE + clkgate_con(66), 0xd6),
            (CRU_BASE + clkgate_con(14), 0),
            (CRU_BASE + MODE_CON0, 0x545),
            (CRU_BASE + pll_con(104), 0xfa),
            (CRU_BASE + pll_con(105), 0x42),
            (CRU_BASE + pll_con(106), 0),
            (CRU_BASE + pll_con(110), 0x9c00),
            (PMU_BASE + pd_gpu::PWR_GATE, 0xffff),
            (PMU_BASE + pd_gpu::STATUS, 0x2455_ffff),
            (PMU_BASE + pd_gpu::BUS_IDLE_REQ, 0x83cf),
            (PMU_BASE + pd_gpu::BUS_IDLE_ACK, 0x000a_9fff),
            (PMU_BASE + pd_gpu::BUS_IDLE_ST, 0x000a_9fff),
            (PMU_BASE + pd_gpu::REPAIR_STATUS, 0xfad5_0001),
            (GPU + mali::GPU_ID, mali::G610_GPU_ID),
            (GPU + mali::GPU_L2_PRESENT, 1),
            (GPU + mali::GPU_SHADER_PRESENT, 0x50005),
        ] {
            r.insert(a, v);
        }
        Board {
            regs: RefCell::new(r),
            writes: RefCell::new(vec![]),
            gpu_reads: RefCell::new(0),
            rk806: RefCell::new([(0x00, 0x0f), (0x06, 0xe0), (0x10, 0xa4), (0x1a, 0x1c)].into_iter().collect()),
            frame: RefCell::new(vec![]),
            rxq: RefCell::new(Default::default()),
            reads: RefCell::new(0),
            deaf_from_read: None,
            stuck_gate14: 0,
            pd_dead: false,
            reset_dead: false,
            sticky_raw: false,
            l2_dead: false,
            l2_on: RefCell::new(false),
        }
    }

    /// DCDC1 off at hand-over (U-Boot's state): `POWER_EN0` 0x0e.
    fn supply_off(mut self) -> Board {
        self.rk806.get_mut().insert(rk806::POWER_EN0, 0x0e);
        self
    }

    /// PD_GPU left on and out of idle (Armbian's values with the GPU active).
    fn domain_on(self) -> Board {
        for (a, v) in [
            (pd_gpu::PWR_GATE, 0xfffe),
            (pd_gpu::BUS_IDLE_REQ, 0x83ce),
            (pd_gpu::BUS_IDLE_ACK, 0x000a_9ffe),
            (pd_gpu::BUS_IDLE_ST, 0x000a_9ffe),
            (pd_gpu::REPAIR_STATUS, 0xfad5_0003),
        ] {
            self.regs.borrow_mut().insert(PMU_BASE + a, v);
        }
        self.regs.borrow_mut().insert(CRU_BASE + clkgate_con(66), 0x04);
        self
    }

    fn run(&self) -> GpuUp {
        gpu_up(self, B, &mut |_| {}, &mut |_| {})
    }

    /// RK806 frames that wrote (a write command has bit 7 set).
    fn rk806_write_frames(&self) -> usize {
        self.writes.borrow().iter().filter(|w| w.0 == SPI + rockchip_spi::TXDR && w.1 & 0x80 != 0).count()
    }

    fn writes_in(&self, lo: usize, len: usize) -> usize {
        self.writes.borrow().iter().filter(|w| (lo..lo + len).contains(&w.0)).count()
    }

    fn pmu_writes(&self) -> usize {
        self.writes_in(PMU_BASE, 0x400)
    }

    fn gpu_writes(&self) -> usize {
        self.writes_in(GPU, 0x10000)
    }
}

impl Mmio for Board {
    fn read32(&self, a: usize) -> u32 {
        if (GPU..GPU + 0x10000).contains(&a) || (QOS..QOS + 0x800).contains(&a) {
            *self.gpu_reads.borrow_mut() += 1;
        }
        match a {
            _ if a == SPI + rockchip_spi::RXFLR => self.rxq.borrow().len() as u32,
            _ if a == SPI + rockchip_spi::RXDR => u32::from(self.rxq.borrow_mut().pop_front().unwrap()),
            _ if a == GPU + mali::L2_READY => u32::from(*self.l2_on.borrow()),
            _ => *self.regs.borrow().get(&a).unwrap_or(&0),
        }
    }

    fn write32(&self, a: usize, v: u32) {
        self.writes.borrow_mut().push((a, v));
        if a == SPI + rockchip_spi::TXDR {
            let mut f = self.frame.borrow_mut();
            f.push(v as u8);
            if f[0] & 0x80 != 0 && f.len() == 4 {
                // the one write the run makes: POWER_EN0 with DCDC1 on
                assert_eq!(u16::from_le_bytes([f[1], f[2]]), rk806::POWER_EN0, "{f:02x?}");
                *self.rk806.borrow_mut().get_mut(&rk806::POWER_EN0).unwrap() |= rk806::DCDC1_ON;
            }
            // every frame sent clocks one byte back
            self.rxq.borrow_mut().push_back(0);
            return;
        }
        if a == SPI + rockchip_spi::SER && v == 0 {
            self.frame.borrow_mut().clear();
        }
        // a receive-only phase after a read command: the register's value
        if a == SPI + rockchip_spi::SSIENR
            && v == 1
            && self.regs.borrow().get(&(SPI + rockchip_spi::CTRLR0)).copied().unwrap_or(0) & rockchip_spi::CTRLR0_XFM_RO
                != 0
        {
            let n = {
                let mut n = self.reads.borrow_mut();
                *n += 1;
                *n - 1
            };
            if self.deaf_from_read.is_none_or(|from| n < from) {
                let f = self.frame.borrow().clone();
                assert_eq!((f.len(), f[0]), (3, 0), "a read command: {f:02x?}");
                let reg = u16::from_le_bytes([f[1], f[2]]);
                self.rxq.borrow_mut().push_back(self.rk806.borrow()[&reg]);
            }
        }
        if a == GPU + mali::L2_PWRON && v & 1 != 0 && !self.l2_dead {
            *self.l2_on.borrow_mut() = true;
        }
        if a == GPU + mali::GPU_CMD && v == mali::CMD_SOFT_RESET {
            if !self.reset_dead {
                *self.regs.borrow_mut().entry(GPU + mali::GPU_INT_RAWSTAT).or_insert(0) |= mali::IRQ_RESET_COMPLETED;
            }
            return;
        }
        if a == GPU + mali::GPU_INT_CLEAR {
            if !self.sticky_raw {
                *self.regs.borrow_mut().entry(GPU + mali::GPU_INT_RAWSTAT).or_insert(0) &= !v;
            }
            return;
        }
        let mut r = self.regs.borrow_mut();
        if (CRU_BASE..CRU_BASE + 0x1000).contains(&a) || (PMU_BASE..PMU_BASE + 0x400).contains(&a) {
            let mut mask = v >> 16;
            if a == CRU_BASE + clkgate_con(14) {
                mask &= !self.stuck_gate14;
            }
            let e = r.entry(a).or_insert(0);
            *e = (*e & !mask) | (v & mask);
            if a == PMU_BASE + pd_gpu::PWR_GATE && v == 1 << 16 && !self.pd_dead {
                *r.get_mut(&(PMU_BASE + pd_gpu::REPAIR_STATUS)).unwrap() |= 2;
            }
            if a == PMU_BASE + pd_gpu::BUS_IDLE_REQ && v == 1 << 16 {
                *r.get_mut(&(PMU_BASE + pd_gpu::BUS_IDLE_ACK)).unwrap() &= !1;
                *r.get_mut(&(PMU_BASE + pd_gpu::BUS_IDLE_ST)).unwrap() &= !1;
            }
        } else {
            r.insert(a, v);
        }
    }

    fn read8(&self, _: usize) -> u8 {
        unreachable!()
    }

    fn write8(&self, _: usize, _: u8) {
        unreachable!()
    }
}

fn rx_timeout() -> u32 {
    rockchip_spi::SpiRefusal::RxTimeout as u32
}

/// The control: the mock's starting states run through.
#[test]
fn the_mock_runs_through_from_its_starting_states() {
    for b in [Board::new(), Board::new().supply_off(), Board::new().domain_on()] {
        let o = b.run();
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
    }
}

#[test]
fn spi_gate_pclk_spi2_stays_closed() {
    let mut b = Board::new();
    b.stuck_gate14 = 1 << rk3588::PCLK_SPI2.bit;
    b.regs.get_mut().insert(CRU_BASE + clkgate_con(14), 1 << rk3588::PCLK_SPI2.bit);
    let o = b.run();
    // refusal the gate's bit, detail the word read back
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::SpiGate, 8, 1 << 8));
    assert_eq!(o.spi_opened, 0);
    // only the one write-enable twin of the gate went out
    assert_eq!(*b.writes.borrow(), [(CRU_BASE + clkgate_con(14), 1 << (8 + 16))]);
    assert_eq!(*b.gpu_reads.borrow(), 0);
}

#[test]
fn spi_gate_clk_spi2_stays_closed_after_pclk_opened() {
    let mut b = Board::new();
    let both = 1 << rk3588::PCLK_SPI2.bit | 1 << rk3588::CLK_SPI2.bit;
    b.stuck_gate14 = 1 << rk3588::CLK_SPI2.bit;
    b.regs.get_mut().insert(CRU_BASE + clkgate_con(14), both);
    let o = b.run();
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::SpiGate, 13, 1 << 13));
    assert_eq!(o.spi_opened, 1 << rk3588::PCLK_SPI2.bit);
    assert_eq!(b.writes.borrow().len(), 2, "{:x?}", b.writes.borrow());
    assert_eq!(b.rk806_write_frames() + b.pmu_writes() + b.gpu_writes(), 0);
    assert_eq!(*b.gpu_reads.borrow(), 0);
}

#[test]
fn rk806_read_of_the_handover_registers_fails() {
    for (n, reg) in RK806_READ.into_iter().enumerate() {
        let mut b = Board::new();
        b.deaf_from_read = Some(n);
        let o = b.run();
        // refusal the SPI refusal, detail the register << 32 | frames received
        assert_eq!(
            (o.stop, o.refusal, o.detail),
            (Stop::Rk806Read, rx_timeout(), u64::from(reg) << 32),
            "read {n} (register {reg:#x})"
        );
        assert_eq!(o.rk806[..n], [0x0f, 0xe0, 0xa4][..n]);
        assert_eq!(*b.reads.borrow(), n + 1);
        // no RK806 write, no CRU (clock set), PMU or GPU access past it
        assert_eq!(b.rk806_write_frames() + b.pmu_writes() + b.gpu_writes(), 0);
        assert_eq!(b.writes_in(CRU_BASE, 0x1000), 0);
        assert_eq!(*b.gpu_reads.borrow(), 0);
        assert!(!o.supply_enabled);
    }
}

#[test]
fn rk806_read_of_power_en0_after_the_enable_fails() {
    let mut b = Board::new().supply_off();
    // the four hand-over reads answer, the poll after the enable does not
    b.deaf_from_read = Some(RK806_READ.len());
    let o = b.run();
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::Rk806Read, rx_timeout(), u64::from(rk806::POWER_EN0) << 32));
    // the enable went out, once, and nothing after it
    assert!(o.supply_enabled);
    assert_eq!(b.rk806_write_frames(), 1);
    assert_eq!(*b.reads.borrow(), RK806_READ.len() + 1);
    assert_eq!(o.supply_after, [0, 0]);
    assert_eq!(b.pmu_writes() + b.gpu_writes(), 0);
    assert_eq!(*b.gpu_reads.borrow(), 0);
}

#[test]
fn rk806_read_of_buck1_on_vsel_after_the_enable_fails() {
    let mut b = Board::new().supply_off();
    // POWER_EN0 answers (DCDC1 on at once), BUCK1_ON_VSEL does not
    b.deaf_from_read = Some(RK806_READ.len() + 1);
    let o = b.run();
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::Rk806Read, rx_timeout(), u64::from(rk806::BUCK1_ON_VSEL) << 32));
    assert!(o.supply_enabled);
    assert_eq!(o.supply_after[0] & rk806::DCDC1_ON, rk806::DCDC1_ON);
    assert_eq!(o.supply_after[1], 0);
    assert_eq!(b.rk806_write_frames(), 1);
    assert_eq!(*b.reads.borrow(), RK806_READ.len() + 2);
    assert_eq!(b.pmu_writes() + b.gpu_writes(), 0);
    assert_eq!(*b.gpu_reads.borrow(), 0);
}

#[test]
fn power_domain_does_not_come_on() {
    let mut b = Board::new();
    b.pd_dead = true;
    let o = b.run();
    // refusal PdRefusal::PowerTimeout, detail REPAIR_STATUS as read
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::PowerDomain, rk3588::PdRefusal::PowerTimeout as u32, 0xfad5_0001));
    assert_eq!(o.pd_after.repair, 0xfad5_0001);
    // the power-gate write went out; the bus-idle request and the GPU did not
    assert_eq!(b.pmu_writes(), 1);
    assert_eq!(b.writes.borrow().last().unwrap(), &(PMU_BASE + pd_gpu::PWR_GATE, pd_gpu::PWR_BIT << 16));
    assert_eq!(b.gpu_writes(), 0);
    assert_eq!(*b.gpu_reads.borrow(), 0, "the QoS blocks and the GPU are not read with PD_GPU off");
}

#[test]
fn reset_does_not_complete() {
    let mut b = Board::new().domain_on();
    b.reset_dead = true;
    let o = b.run();
    // refusal MaliRefusal::ResetTimeout, detail RAWSTAT as read
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::Reset, mali::MaliRefusal::ResetTimeout as u32, 0));
    assert_eq!(o.ident.gpu_id, mali::G610_GPU_ID);
    // the reset was commanded; the L2 was not touched
    assert!(b.writes.borrow().contains(&(GPU + mali::GPU_CMD, mali::CMD_SOFT_RESET)));
    assert!(b.writes.borrow().iter().all(|w| w.0 != GPU + mali::L2_PWRON));
    assert_eq!(o.ident_after_reset, mali::Ident::default());
}

#[test]
fn reset_completion_bit_is_stuck_set() {
    let mut b = Board::new().domain_on();
    b.sticky_raw = true;
    b.regs.get_mut().insert(GPU + mali::GPU_INT_RAWSTAT, mali::IRQ_RESET_COMPLETED);
    let o = b.run();
    assert_eq!(
        (o.stop, o.refusal, o.detail),
        (Stop::Reset, mali::MaliRefusal::ResetBitStuck as u32, u64::from(mali::IRQ_RESET_COMPLETED))
    );
    // refused before the command: no reset was issued, the L2 was not touched
    assert!(b.writes.borrow().iter().all(|w| w.0 != GPU + mali::GPU_CMD && w.0 != GPU + mali::L2_PWRON));
}

#[test]
fn l2_does_not_become_ready() {
    let mut b = Board::new().domain_on();
    b.l2_dead = true;
    let o = b.run();
    // refusal MaliRefusal::L2ReadyTimeout, detail L2_READY as read
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::L2, mali::MaliRefusal::L2ReadyTimeout as u32, 0));
    // the reset completed; the power-on was asked for
    assert_eq!(o.ident_after_reset.gpu_id, mali::G610_GPU_ID);
    assert!(b.writes.borrow().contains(&(GPU + mali::L2_PWRON, 1)));
    assert_eq!(o.l2, mali::L2On::default());
}

#[test]
fn l2_is_absent() {
    let b = Board::new().domain_on();
    b.regs.borrow_mut().insert(GPU + mali::GPU_L2_PRESENT, 0);
    let o = b.run();
    assert_eq!((o.stop, o.refusal, o.detail), (Stop::L2, mali::MaliRefusal::NoL2 as u32, 0));
    assert!(b.writes.borrow().iter().all(|w| w.0 != GPU + mali::L2_PWRON));
}

/// Where a [`Stop`] variant is reached.
#[derive(Debug, PartialEq)]
enum Reached {
    /// In this file, by these tests.
    Here(&'static [&'static str]),
    /// In the tests of `nanopc_t6_gpu.rs`, or no stop at all.
    Module,
}

/// Every variant, through an exhaustive `match`: a new variant does not
/// compile until it is placed here, and with it a test.
fn reached(s: Stop) -> Reached {
    use Reached::*;
    match s {
        Stop::None => Module,
        Stop::SpiGate => Here(&["spi_gate_pclk_spi2_stays_closed", "spi_gate_clk_spi2_stays_closed_after_pclk_opened"]),
        Stop::Rk806Read => Here(&[
            "rk806_read_of_the_handover_registers_fails",
            "rk806_read_of_power_en0_after_the_enable_fails",
            "rk806_read_of_buck1_on_vsel_after_the_enable_fails",
        ]),
        Stop::SupplyOff => Module,
        Stop::ClockNotCovered => Module,
        Stop::PowerDomain => Here(&["power_domain_does_not_come_on"]),
        Stop::NotG610 => Module,
        Stop::Reset => Here(&["reset_does_not_complete", "reset_completion_bit_is_stuck_set"]),
        Stop::L2 => Here(&["l2_does_not_become_ready", "l2_is_absent"]),
        Stop::ClockSetRefused => Module,
        Stop::ClockReadBack => Module,
        Stop::SupplyBelowOpp => Module,
        Stop::SupplyRampTimeout => Module,
        Stop::Rk806Write => Module,
    }
}

#[test]
fn every_stop_variant_is_listed() {
    let all = [
        Stop::None,
        Stop::SpiGate,
        Stop::Rk806Read,
        Stop::SupplyOff,
        Stop::ClockNotCovered,
        Stop::PowerDomain,
        Stop::NotG610,
        Stop::Reset,
        Stop::L2,
        Stop::ClockSetRefused,
        Stop::ClockReadBack,
        Stop::SupplyBelowOpp,
        Stop::SupplyRampTimeout,
        Stop::Rk806Write,
    ];
    // the values are the enum's discriminants 0..=13, each once
    for (i, s) in all.into_iter().enumerate() {
        assert_eq!(s as usize, i, "{s:?}");
        if let Reached::Here(names) = reached(s) {
            assert!(!names.is_empty(), "{s:?}");
        }
    }
    assert_eq!(all.iter().filter(|&&s| reached(s) != Reached::Module).count(), 5);
}
