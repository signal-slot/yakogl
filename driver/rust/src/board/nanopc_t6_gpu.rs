// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6's GPU power-on, the second half of Z1
//! (driver/docs/nanopc-t6-mali.md): read what the kexec from Linux handed
//! over (PD_GPU, the GPU clock in the CRU, vdd_gpu_s0 in the RK806), put the
//! clock on a rate of the GPU OPP table when it is not on one
//! ([`rk3588::gpu_clock_set`]: the PVTPLL, an unknown mux or a divider off
//! the table become CPLL / 5 = 300 MHz; Armbian's state is left as it is),
//! check the supply covers that rate, power the clock's PLL on when Linux
//! left it powered down, power PD_GPU on, read the GPU's identity, soft
//! reset it and power its L2 on. Every value read is kept in [`GpuUp`] for
//! the image to print; the first step that refuses ends the run there.
//!
//! The supply is read, never written: the RK806 frames this builds are reads
//! ([`crate::ip::rk806`]). A supply that is off, or lower than the GPU OPP
//! table asks for the rate the clock is on, stops the run before PD_GPU is
//! touched. The clock set is the only CRU write before the supply check
//! (the SPI2 gates aside), and it touches CLKSEL_CON(158) and (159) only.

use crate::gpu::mali::{self, Ident, L2On, Reset};
use crate::ip::{rk806, rockchip_spi};
use crate::mmio::Mmio;
use crate::soc::rk3588::{self, ClockRefusal, ClockSet, GpuClock, PdGpuRegs, PdOn, PllOn};

/// The physical blocks this touches, mapped by the caller.
#[derive(Clone, Copy, Debug)]
pub struct Bases {
    pub cru: usize,
    pub pmu: usize,
    pub spi2: usize,
    /// `qos_gpu_m0`; m1..m3 follow at 0x200 steps ([`rk3588::QOS_GPU`]).
    pub qos: usize,
    pub gpu: usize,
}

/// `BAUDR` for the RK806: `clk_spi2`'s parents are 200 MHz, 150 MHz and
/// 24 MHz (`mux_200m_150m_24m_p`, clk-rk3588.c:455, 1182-1184; the T6 reads
/// 198 MHz), so 200 keeps the bus at or below the PMIC's 1 MHz
/// (rk3588-nanopc-t6.dtsi:765) whichever parent Linux left selected.
pub const RK806_BAUDR: u32 = 200;

/// The RK806 on SPI2 chip select 0 (rk3588-nanopc-t6.dtsi:763-766). A
/// 1-value read is 4 frames, 32 µs at 1 MHz; the budget is generous.
pub const RK806_BUS: rockchip_spi::Host = rockchip_spi::Host { base: 0, cs: 0, baudr: RK806_BAUDR, budget_us: 1_000 };

/// The RK806 registers read: `POWER_EN0`, `POWER_SLP_EN0`, `BUCK1_CONFIG`,
/// `BUCK1_ON_VSEL`.
pub const RK806_READ: [u16; 4] = [rk806::POWER_EN0, rk806::POWER_SLP_EN0, rk806::BUCK1_CONFIG, rk806::BUCK1_ON_VSEL];

/// SPI2's registers as handed over, read before the first write:
/// `CTRLR0`, `CTRLR1`, `SSIENR`, `SER`, `BAUDR`, `SR`, `RXFLR`, `VERSION`.
/// Under Armbian (2026-10-19, between transfers): 0x2c01, 2, 0, 0, 0xc6,
/// 0x4c, 0, 0x00110002.
pub const SPI_FOUND: [usize; 8] = [
    rockchip_spi::CTRLR0,
    rockchip_spi::CTRLR1,
    rockchip_spi::SSIENR,
    rockchip_spi::SER,
    rockchip_spi::BAUDR,
    rockchip_spi::SR,
    rockchip_spi::RXFLR,
    rockchip_spi::VERSION,
];

/// The least vdd_gpu_s0 in µV the GPU OPP table gives a clock
/// ([`rk3588::GPU_OPP`], where the table lives).
pub use crate::soc::rk3588::gpu_opp_min_uv as min_uv_for;

/// Which step stopped the run. The value crosses to C (`YD_T6_GPU_STOP_*`;
/// the variants from 9 on are not there yet, a later change adds them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Stop {
    /// Every step ran: `GPU_ID` read, reset done, L2 ready.
    #[default]
    None = 0,
    /// An SPI2 clock gate stays closed.
    SpiGate = 1,
    /// An RK806 read failed (`refusal` is the [`rockchip_spi::SpiRefusal`]).
    Rk806Read = 2,
    /// DCDC1 (vdd_gpu_s0) reads off.
    SupplyOff = 3,
    /// The supply is below what the OPP table asks for the rate the clock is
    /// on after [`rk3588::gpu_clock_set`] (`refusal` 0, the detail is the
    /// rate); or the clock's PLL did not lock ([`CLOCK_PLL_NO_LOCK`], the
    /// detail is `PLL_CON6`).
    ClockNotCovered = 4,
    /// PD_GPU did not come on (`refusal` is the [`rk3588::PdRefusal`]).
    PowerDomain = 5,
    /// `GPU_ID` is not the G610's.
    NotG610 = 6,
    /// The soft reset did not complete.
    Reset = 7,
    /// The L2 did not come up (`refusal` is the [`mali::MaliRefusal`]).
    L2 = 8,
    /// The CRU's GPU clock is on no OPP rate and could not be put on one:
    /// the CPLL does not read 1500 MHz in normal mode
    /// ([`ClockRefusal::NoTableRate`]; `refusal` 0, the detail is the CPLL's
    /// rate, 0 for none). Nothing was written to the CRU for it.
    ClockSetRefused = 9,
    /// The clock was written but did not take: a CLKSEL_CON read back
    /// without the fields written (`refusal` its index, 158 or 159, the
    /// detail the word read; [`ClockRefusal::ReadBack`]), or the CRU rated
    /// the source at something other than 300 MHz afterwards (`refusal` 0,
    /// the detail that rate; [`ClockRefusal::Rate`]).
    ClockReadBack = 10,
}

/// The refusal of [`Stop::ClockNotCovered`] when the GPU clock's PLL did not
/// lock after its power-on ([`rk3588::pll_on`]).
pub const CLOCK_PLL_NO_LOCK: u32 = 1;

/// Everything the run read, in the order it read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUp {
    pub pd_before: PdGpuRegs,
    /// The GPU clock as handed over.
    pub clock: GpuClock,
    /// What was done to it: kept, or the words written and the rate read
    /// back ([`rk3588::gpu_clock_set`]). `clock_set.src_hz` is the rate the
    /// supply check and the OPP table are held against.
    pub clock_set: ClockSet,
    /// CLKGATE_CON(14) as found, and the SPI2 gates opened (bit mask of
    /// their bits).
    pub spi_gate14: u32,
    pub spi_opened: u32,
    /// SPI2 as handed over ([`SPI_FOUND`]).
    pub spi_found: [u32; 8],
    /// [`RK806_READ`], read as Linux reads them (`write_then_read`).
    pub rk806: [u8; 4],
    pub vdd_gpu_uv: u32,
    /// The power-on of the PLL `CLK_GPU_SRC` runs from.
    pub pll_on: PllOn,
    pub pd_on: PdOn,
    pub pd_after: PdGpuRegs,
    /// `qos_gpu_m0..m3`, five words each, read after the power-on.
    pub qos: [[u32; 5]; 4],
    pub ident: Ident,
    pub reset: Reset,
    /// After the reset.
    pub ident_after_reset: Ident,
    pub l2: L2On,
    pub stop: Stop,
    /// The refusal number of the step that stopped, 0 otherwise.
    pub refusal: u32,
    /// The value the refusal is about (a register, a count).
    pub detail: u64,
}

impl GpuUp {
    fn stopped(&mut self, stop: Stop, refusal: u32, detail: u64) {
        self.stop = stop;
        self.refusal = refusal;
        self.detail = detail;
    }
}

/// The run (see the module). `delay_us` waits that many microseconds.
///
/// `on_handover` sees what the run read before its first write to the PMU or
/// the GPU (the hand-over state, the supply and the PLL's power-on), so the
/// image prints it before a step that could fault.
pub fn gpu_up(m: &impl Mmio, b: Bases, delay_us: &mut impl FnMut(u32), on_handover: &mut impl FnMut(&GpuUp)) -> GpuUp {
    let mut o = GpuUp { pd_before: PdGpuRegs::read(m, b.pmu), ..GpuUp::default() };
    o.clock = rk3588::gpu_clock(m, b.cru);
    o.spi_gate14 = m.read32(b.cru + rk3588::clkgate_con(14));

    for g in [rk3588::PCLK_SPI2, rk3588::CLK_SPI2] {
        match rk3588::gate_open(m, b.cru, g) {
            Ok(true) => o.spi_opened |= 1 << g.bit,
            Ok(false) => {}
            Err(now) => {
                o.stopped(Stop::SpiGate, g.bit, u64::from(now));
                return o;
            }
        }
    }
    for (i, off) in SPI_FOUND.into_iter().enumerate() {
        o.spi_found[i] = m.read32(b.spi2 + off);
    }
    let bus = RK806_BUS.at(b.spi2);
    for (i, reg) in RK806_READ.into_iter().enumerate() {
        let mut tx = [0u8; 4];
        let n = rk806::read_frame(reg, 1, &mut tx).expect("a 1-register read fits 4 bytes");
        let mut v = [0u8; 1];
        if let Err((why, detail)) = rockchip_spi::write_then_read(m, &bus, &tx[..n - 1], &mut v, delay_us) {
            o.stopped(Stop::Rk806Read, why as u32, u64::from(reg) << 32 | u64::from(detail));
            return o;
        }
        o.rk806[i] = v[0];
    }
    o.vdd_gpu_uv = rk806::buck_uv(o.rk806[3]);
    // the clock onto the OPP table first, so the supply is checked against
    // the rate the GPU will run at; the GPU's gates are closed meanwhile
    match rk3588::gpu_clock_set(m, b.cru, &o.clock) {
        Ok(set) => o.clock_set = set,
        Err(why) => {
            match why {
                ClockRefusal::NoTableRate(hz) => o.stopped(Stop::ClockSetRefused, 0, hz),
                ClockRefusal::ReadBack(con, word) => o.stopped(Stop::ClockReadBack, con as u32, u64::from(word)),
                ClockRefusal::Rate(hz) => o.stopped(Stop::ClockReadBack, 0, hz),
            }
            return o;
        }
    }
    if o.rk806[0] & rk806::DCDC1_ON == 0 {
        o.stopped(Stop::SupplyOff, 0, u64::from(o.rk806[0]));
        return o;
    }
    match min_uv_for(o.clock_set.src_hz) {
        Some(uv) if o.vdd_gpu_uv >= uv => {}
        _ => {
            o.stopped(Stop::ClockNotCovered, 0, o.clock_set.src_hz);
            return o;
        }
    }
    // Linux powers a PLL down with its last consumer, and the runtime-
    // suspended GPU can be that consumer: without the PLL the domain powers
    // but never leaves bus idle
    if let Some(pll) = rk3588::gpu_src_pll(rk3588::gpu_src(o.clock_set.sel[0]).0) {
        match rk3588::pll_on(m, b.cru, pll, delay_us) {
            Ok(on) => o.pll_on = on,
            Err(con6) => {
                o.stopped(Stop::ClockNotCovered, CLOCK_PLL_NO_LOCK, u64::from(con6));
                return o;
            }
        }
    }
    on_handover(&o);

    match rk3588::pd_gpu_on(m, b.cru, b.pmu, delay_us) {
        Ok(on) => o.pd_on = on,
        Err((why, now)) => {
            o.stopped(Stop::PowerDomain, why as u32, u64::from(now));
            o.pd_after = PdGpuRegs::read(m, b.pmu);
            return o;
        }
    }
    o.pd_after = PdGpuRegs::read(m, b.pmu);
    for (blk, words) in o.qos.iter_mut().enumerate() {
        for (w, off) in words.iter_mut().zip(rk3588::QOS_WORDS) {
            *w = m.read32(b.qos + 0x200 * blk + off);
        }
    }

    o.ident = mali::ident(m, b.gpu);
    if o.ident.gpu_id != mali::G610_GPU_ID {
        o.stopped(Stop::NotG610, 0, u64::from(o.ident.gpu_id));
        return o;
    }
    match mali::soft_reset(m, b.gpu, delay_us) {
        Ok(r) => o.reset = r,
        Err((why, raw)) => {
            o.stopped(Stop::Reset, why as u32, u64::from(raw));
            return o;
        }
    }
    o.ident_after_reset = mali::ident(m, b.gpu);
    match mali::l2_power_on(m, b.gpu, delay_us) {
        Ok(l2) => o.l2 = l2,
        Err((why, v)) => o.stopped(Stop::L2, why as u32, v),
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::soc::rk3588::{clkgate_con, clksel_con, pd_gpu, pll_con, CRU_BASE, MODE_CON0, PMU_BASE};
    use std::cell::RefCell;
    use std::collections::HashMap;

    const SPI: usize = rk3588::SPI2_BASE;
    const QOS: usize = rk3588::QOS_GPU[0];
    const GPU: usize = 0xfb00_0000;
    const B: Bases = Bases { cru: CRU_BASE, pmu: PMU_BASE, spi2: SPI, qos: QOS, gpu: GPU };

    /// The T6 as Armbian hands it over with the GPU runtime-suspended
    /// (2026-10-19 reads): the CRU and PMU with their hiword masks, SPI2
    /// with the RK806 behind it, the GPU QoS blocks and the GPU, which
    /// aborts (here: panics) when read with PD_GPU off. A PLL locks when its
    /// power-down bit is cleared, and PD_GPU leaves bus idle only while the
    /// GPU's clock runs (the run of 2026-11-11: the acknowledge stayed set
    /// with `CLK_GPU_SRC` on a powered-down NPLL).
    struct T6 {
        regs: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        rk806: HashMap<u16, u8>,
        frame: RefCell<Vec<u8>>,
        rxq: RefCell<std::collections::VecDeque<u8>>,
        l2_on: RefCell<bool>,
        /// No PLL locks.
        pll_dead: bool,
        /// Registers that ignore writes (logged, not applied).
        stuck: Vec<usize>,
    }

    const SEL158: usize = CRU_BASE + clksel_con(158);

    const PLLS: [rk3588::Pll; 4] = [rk3588::GPLL, rk3588::CPLL, rk3588::AUPLL, rk3588::NPLL];

    /// `CLK_GPU_SRC` has an output: its PLL is powered and locked.
    fn gpu_clock_runs(r: &HashMap<usize, u32>) -> bool {
        let (src, _) = rk3588::gpu_src(r[&(CRU_BASE + clksel_con(158))]);
        rk3588::gpu_src_pll(src).is_some_and(|p| {
            let con = |i| r.get(&(CRU_BASE + pll_con(p.con + i))).copied().unwrap_or(0);
            con(1) & rk3588::PLL_CON1_PWRDOWN == 0 && con(6) & rk3588::PLL_CON6_LOCK != 0
        })
    }

    impl T6 {
        fn handed_over() -> T6 {
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
            for blk in 0..4 {
                r.insert(QOS + 0x200 * blk + 0x08, 0x8000_0000);
            }
            T6 {
                regs: RefCell::new(r),
                writes: RefCell::new(vec![]),
                rk806: [(0x00, 0x0f), (0x06, 0xe0), (0x10, 0xa4), (0x1a, 0x1c)].into_iter().collect(),
                frame: RefCell::new(vec![]),
                rxq: RefCell::new(Default::default()),
                l2_on: RefCell::new(false),
                pll_dead: false,
                stuck: vec![],
            }
        }
        /// The hand-over of 2026-11-11: the GPU last ran at 850 MHz, so
        /// `CLK_GPU_SRC` is NPLL / 1 at 800 mV, and Linux powered NPLL down
        /// when it suspended the GPU. 850 MHz is no rate of the OPP table
        /// the code holds (rk3588-opp.dtsi:118-153).
        fn handed_over_on_npll() -> T6 {
            let mut t6 = T6::handed_over();
            for (a, v) in [
                (clksel_con(158), 0x1f60),
                (pll_con(120), 0x1a9),
                (pll_con(121), 0x2083),
                (pll_con(122), 0),
                (pll_con(126), 0x4000),
            ] {
                t6.regs.borrow_mut().insert(CRU_BASE + a, v);
            }
            t6.rk806.insert(0x1a, 0x30);
            t6
        }
        /// The same with NPLL at 900 MHz (`PLL_CON0` m = 450: not measured,
        /// an OPP rate so that the clock is kept on NPLL and the PLL's
        /// power-on stays under test).
        fn handed_over_on_npll_900() -> T6 {
            let t6 = T6::handed_over_on_npll();
            t6.regs.borrow_mut().insert(CRU_BASE + pll_con(120), 0x1c2);
            t6
        }
        fn gpu_powered(&self) -> bool {
            let r = self.regs.borrow();
            r[&(PMU_BASE + pd_gpu::REPAIR_STATUS)] & 2 != 0 && r[&(PMU_BASE + pd_gpu::BUS_IDLE_ST)] & 1 == 0
        }
    }

    impl Mmio for T6 {
        fn read32(&self, a: usize) -> u32 {
            if (GPU..GPU + 0x10000).contains(&a) || (QOS..QOS + 0x800).contains(&a) {
                assert!(self.gpu_powered(), "read of {a:#x} with PD_GPU off: an external abort on the board");
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
            // the CRU's PLL_CON and MODE_CON words (CRU + 0..0x300): the only
            // write the run may make there is a PLL's power-down bit
            if (CRU_BASE..CRU_BASE + 0x300).contains(&a) {
                assert!(
                    PLLS.iter().any(|p| a == CRU_BASE + pll_con(p.con + 1)) && v == rk3588::PLL_CON1_PWRDOWN << 16,
                    "a PLL_CON / MODE_CON write {a:#x} := {v:#x}"
                );
            }
            if self.stuck.contains(&a) {
                return;
            }
            if a == SPI + rockchip_spi::TXDR {
                let mut f = self.frame.borrow_mut();
                f.push(v as u8);
                // the RK806 answers the value in the fourth byte of a 1-value read
                let out = if f.len() == 4 {
                    assert_eq!(f[0], 0, "only 1-value reads, never a write: {f:02x?}");
                    self.rk806[&u16::from_le_bytes([f[1], f[2]])]
                } else {
                    0
                };
                self.rxq.borrow_mut().push_back(out);
                return;
            }
            if a == SPI + rockchip_spi::SER && v == 0 {
                self.frame.borrow_mut().clear();
            }
            // receive only: the frames after the command, MOSI idle
            if a == SPI + rockchip_spi::SSIENR
                && v == 1
                && self.regs.borrow().get(&(SPI + rockchip_spi::CTRLR0)).copied().unwrap_or(0)
                    & rockchip_spi::CTRLR0_XFM_RO
                    != 0
            {
                let f = self.frame.borrow().clone();
                assert_eq!(f.len(), 3, "a receive-only phase after a 3-byte command: {f:02x?}");
                assert_eq!(f[0], 0, "only 1-value reads, never a write: {f:02x?}");
                self.rxq.borrow_mut().push_back(self.rk806[&u16::from_le_bytes([f[1], f[2]])]);
            }
            if a == GPU + mali::L2_PWRON && v & 1 != 0 {
                *self.l2_on.borrow_mut() = true;
            }
            // a soft reset completes at once; GPU_INT_CLEAR clears raw bits
            if a == GPU + mali::GPU_CMD && v == mali::CMD_SOFT_RESET {
                *self.regs.borrow_mut().entry(GPU + mali::GPU_INT_RAWSTAT).or_insert(0) |= mali::IRQ_RESET_COMPLETED;
                return;
            }
            if a == GPU + mali::GPU_INT_CLEAR {
                *self.regs.borrow_mut().entry(GPU + mali::GPU_INT_RAWSTAT).or_insert(0) &= !v;
                return;
            }
            let mut r = self.regs.borrow_mut();
            if (CRU_BASE..CRU_BASE + 0x1000).contains(&a) || (PMU_BASE..PMU_BASE + 0x400).contains(&a) {
                let e = r.entry(a).or_insert(0);
                let mask = v >> 16;
                *e = (*e & !mask) | (v & mask);
                // PD_GPU follows its controls at once
                if a == PMU_BASE + pd_gpu::PWR_GATE && v == 1 << 16 {
                    *r.get_mut(&(PMU_BASE + pd_gpu::REPAIR_STATUS)).unwrap() |= 2;
                }
                for p in PLLS {
                    if a == CRU_BASE + pll_con(p.con + 1) && !self.pll_dead {
                        let down = r[&a] & rk3588::PLL_CON1_PWRDOWN != 0;
                        let con6 = r.entry(CRU_BASE + pll_con(p.con + 6)).or_insert(0);
                        *con6 = if down { *con6 & !rk3588::PLL_CON6_LOCK } else { *con6 | rk3588::PLL_CON6_LOCK };
                    }
                }
                if a == PMU_BASE + pd_gpu::BUS_IDLE_REQ && v == 1 << 16 && gpu_clock_runs(&r) {
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

    #[test]
    fn from_the_handed_over_state_it_reads_the_supply_powers_and_finds_the_g610() {
        let t6 = T6::handed_over();
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!(o.pd_before.repair & 2, 0);
        assert_eq!(o.clock.src_hz, 300_000_000);
        // 0x1f24 is on the table: kept, no CLKSEL_CON write at all
        assert_eq!(
            o.clock_set,
            ClockSet { kept: true, cpll_hz: 1_500_000_000, wrote: [0; 2], sel: [0x1f24, 0], src_hz: 300_000_000 }
        );
        assert!(
            t6.writes.borrow().iter().all(|w| !(CRU_BASE + 0x300..CRU_BASE + 0x800).contains(&w.0)),
            "{:x?}",
            t6.writes.borrow()
        );
        assert_eq!(t6.regs.borrow()[&SEL158], 0x1f24);
        // CPLL has other consumers: Linux left it on, and it is not written
        assert_eq!(o.pll_on, PllOn { was_on: true, waited_us: 0, con6: 0x9c00 });
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + pll_con(105)));
        // Linux keeps SPI2 runtime-active: its gates are open, none is written
        assert_eq!(o.spi_opened, 0);
        assert_eq!(o.rk806, [0x0f, 0xe0, 0xa4, 0x1c]);
        assert_eq!(o.vdd_gpu_uv, 675_000);
        assert!(o.pd_after.usable());
        assert_eq!(o.qos, [[0x8000_0000, 0, 0, 0, 0]; 4]);
        assert_eq!(o.ident.gpu_id, 0xa867_0005);
        assert_eq!(o.ident_after_reset.shader_present, 0x50005);
        assert_eq!(o.l2.ready, 1);
        // nothing was written to the RK806 frame but reads
        let tx: Vec<u8> =
            t6.writes.borrow().iter().filter(|w| w.0 == SPI + rockchip_spi::TXDR).map(|w| w.1 as u8).collect();
        let want: Vec<u8> = [0x00u8, 0x06, 0x10, 0x1a].iter().flat_map(|&r| [0, r, 0]).collect();
        assert_eq!(tx, want);
    }

    #[test]
    fn the_handover_report_comes_before_any_pmu_or_gpu_write() {
        let t6 = T6::handed_over();
        let mut seen = None;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |h| seen = Some((*h, t6.writes.borrow().len())));
        let (h, writes_then) = seen.expect("on_handover was not called");
        assert_eq!((h.rk806, h.vdd_gpu_uv, h.clock.src_hz), ([0x0f, 0xe0, 0xa4, 0x1c], 675_000, 300_000_000));
        let before = &t6.writes.borrow()[..writes_then];
        assert!(
            before.iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0) && !(GPU..GPU + 0x10000).contains(&w.0)),
            "{before:x?}"
        );
        assert_eq!(o.stop, Stop::None);
    }

    #[test]
    fn a_gpu_clock_on_a_powered_down_pll_is_powered_before_pd_gpu() {
        let t6 = T6::handed_over_on_npll_900();
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!((o.clock.src_hz, o.vdd_gpu_uv), (900_000_000, 800_000));
        assert_eq!((o.clock.pll_con, o.clock.pll_con6), ([0x1c2, 0x2083, 0], 0x4000));
        // 900 MHz is on the table: kept on NPLL, which is then powered
        assert_eq!((o.clock_set.kept, o.clock_set.src_hz, o.clock_set.wrote), (true, 900_000_000, [0; 2]));
        assert_eq!(o.pll_on, PllOn { was_on: false, waited_us: 0, con6: 0xc000 });
        assert!(o.pd_after.usable());
        assert_eq!(o.ident.gpu_id, 0xa867_0005);
        // the PLL's power-down bit is the first write after the supply is read
        let w = t6.writes.borrow();
        let pll = w.iter().position(|w| *w == (CRU_BASE + pll_con(121), 0x2000_0000)).expect("NPLL not powered");
        let first_gate = w.iter().position(|w| w.0 == CRU_BASE + clkgate_con(66)).unwrap();
        let last_spi = w.iter().rposition(|w| (SPI..SPI + 0x1000).contains(&w.0)).unwrap();
        assert!(last_spi < pll && pll < first_gate, "{w:x?}");
    }

    #[test]
    fn the_850_mhz_handover_is_moved_to_cpll_over_5_and_npll_stays_down() {
        let t6 = T6::handed_over_on_npll();
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!(o.clock.src_hz, 850_000_000);
        assert_eq!(
            o.clock_set,
            ClockSet {
                kept: false,
                cpll_hz: 1_500_000_000,
                wrote: [0x40ff_0024, 0],
                sel: [0x1f24, 0],
                src_hz: 300_000_000
            }
        );
        // the PLL powered is the CPLL, already on; NPLL is left as Linux left it
        assert_eq!(o.pll_on, PllOn { was_on: true, waited_us: 0, con6: 0x9c00 });
        assert_eq!(t6.regs.borrow()[&(CRU_BASE + pll_con(121))], 0x2083);
        assert!(o.pd_after.usable());
        assert_eq!(o.ident.gpu_id, 0xa867_0005);
        // the one CLKSEL_CON write comes after the RK806 read and before the gates
        let w = t6.writes.borrow();
        let sel: Vec<usize> = w.iter().enumerate().filter(|(_, w)| w.0 == SEL158).map(|(i, _)| i).collect();
        assert_eq!(sel.len(), 1, "{w:x?}");
        let first_gate = w.iter().position(|w| w.0 == CRU_BASE + clkgate_con(66)).unwrap();
        let last_spi = w.iter().rposition(|w| (SPI..SPI + 0x1000).contains(&w.0)).unwrap();
        assert!(last_spi < sel[0] && sel[0] < first_gate, "{w:x?}");
    }

    #[test]
    fn a_gpu_pll_that_does_not_lock_stops_before_the_pmu_is_written() {
        let mut t6 = T6::handed_over_on_npll_900();
        t6.pll_dead = true;
        let mut reported = false;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| reported = true);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::ClockNotCovered, CLOCK_PLL_NO_LOCK, 0x4000));
        assert!(!reported);
        assert!(t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0)));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
    }

    #[test]
    fn a_supply_that_reads_off_stops_before_the_pmu_is_written() {
        let mut t6 = T6::handed_over();
        t6.rk806.insert(0x00, 0x0e);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.detail), (Stop::SupplyOff, 0x0e));
        assert!(t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0)));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
    }

    /// The T6 with CLKSEL_CON(158) as given, run to the end.
    fn run_from(sel158: u32) -> (T6, GpuUp) {
        let t6 = T6::handed_over();
        t6.regs.borrow_mut().insert(SEL158, sel158);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        (t6, o)
    }

    #[test]
    fn a_clock_off_the_table_is_moved_to_cpll_over_5_before_the_supply_check() {
        // the PVTPLL; CPLL / 2 = 750 MHz and / 4 = 375 MHz; a mux past the parents
        for sel in [0x5f24, 0x1f21, 0x1f23, 0x1fa4, 0x1fe4] {
            let (t6, o) = run_from(sel);
            assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0), "{sel:#x}");
            assert_eq!(o.clock.sel158, sel);
            assert_eq!(
                o.clock_set,
                ClockSet {
                    kept: false,
                    cpll_hz: 1_500_000_000,
                    wrote: [0x40ff_0024, 0],
                    sel: [0x1f24, 0],
                    src_hz: 300_000_000
                },
                "{sel:#x}"
            );
            // exactly one CLKSEL_CON write, hiword-masked, and the 675 mV supply covers 300 MHz
            let sel_writes: Vec<_> = t6.writes.borrow().iter().filter(|w| w.0 == SEL158).copied().collect();
            assert_eq!(sel_writes, vec![(SEL158, 0x40ff_0024)], "{sel:#x}");
            assert_eq!((t6.regs.borrow()[&SEL158], o.vdd_gpu_uv), (0x1f24, 675_000));
            assert!(o.pd_after.usable());
        }
    }

    #[test]
    fn a_clock_the_supply_does_not_cover_stops_before_the_pmu_is_written() {
        // NPLL / 1 = 900 MHz, on the table, wants 800 mV; the supply gives 675 mV
        let mut t6 = T6::handed_over_on_npll_900();
        t6.rk806.insert(0x1a, 0x1c);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::ClockNotCovered, 0, 900_000_000));
        assert_eq!((o.clock_set.kept, o.vdd_gpu_uv), (true, 675_000));
        assert!(t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0)));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
    }

    #[test]
    fn a_cpll_at_another_rate_refuses_the_clock_set_without_a_cru_write() {
        let t6 = T6::handed_over();
        t6.regs.borrow_mut().insert(SEL158, 0x5f24);
        t6.regs.borrow_mut().insert(CRU_BASE + pll_con(104), 0xc6);
        let mut reported = false;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| reported = true);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::ClockSetRefused, 0, 1_188_000_000));
        assert_eq!(o.clock_set, ClockSet::default());
        assert!(!reported);
        // the RK806 was read; nothing was written to the CRU, the PMU or the GPU
        assert_eq!(o.rk806, [0x0f, 0xe0, 0xa4, 0x1c]);
        assert!(t6.writes.borrow().iter().all(|w| (SPI..SPI + 0x1000).contains(&w.0)), "{:x?}", t6.writes.borrow());
        assert_eq!(t6.regs.borrow()[&SEL158], 0x5f24);
    }

    #[test]
    fn a_clock_write_that_does_not_take_stops_before_the_pmu_is_written() {
        let mut t6 = T6::handed_over();
        t6.stuck.push(SEL158);
        t6.regs.borrow_mut().insert(SEL158, 0x5f24);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::ClockReadBack, 158, 0x5f24));
        assert!(t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0)));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + pll_con(105)));
    }

    #[test]
    fn another_gpu_id_stops_before_the_reset() {
        let t6 = T6::handed_over();
        t6.regs.borrow_mut().insert(GPU + mali::GPU_ID, 0xa867_0001);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.detail), (Stop::NotG610, 0xa867_0001));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != GPU + mali::GPU_CMD));
    }
}
