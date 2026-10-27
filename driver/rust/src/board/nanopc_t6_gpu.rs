// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6's GPU power-on, the second half of Z1
//! (driver/docs/nanopc-t6-mali.md): read what the kexec from Linux handed
//! over (PD_GPU, the GPU clock in the CRU, vdd_gpu_s0 in the RK806), check
//! the supply covers the clock, power PD_GPU on, read the GPU's identity,
//! soft reset it and power its L2 on. Every value read is kept in [`GpuUp`]
//! for the image to print; the first step that refuses ends the run there.
//!
//! The supply is read, never written: the RK806 frames this builds are reads
//! ([`crate::ip::rk806`]). A supply that is off, or lower than the GPU OPP
//! table asks for the clock the CRU runs, stops the run before PD_GPU is
//! touched.

use crate::gpu::mali::{self, Ident, L2On, Reset};
use crate::ip::{rk806, rockchip_spi};
use crate::mmio::Mmio;
use crate::soc::rk3588::{self, GpuClock, PdGpuRegs, PdOn};

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

/// The least vdd_gpu_s0 in µV the GPU OPP table gives a clock (the first
/// value of `opp-microvolt` of the first OPP at or above it;
/// rk3588-opp.dtsi:118-153). `None` above the table's 1 GHz or for an
/// unknown rate (0).
pub const fn min_uv_for(hz: u64) -> Option<u32> {
    match hz {
        0 => None,
        1..=600_000_000 => Some(675_000),
        600_000_001..=700_000_000 => Some(700_000),
        700_000_001..=800_000_000 => Some(750_000),
        800_000_001..=900_000_000 => Some(800_000),
        900_000_001..=1_000_000_000 => Some(850_000),
        _ => None,
    }
}

/// Which step stopped the run. The value crosses to C (`YD_T6_GPU_STOP_*`).
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
    /// The CRU's GPU clock has no rate this can name, or the supply is below
    /// what the OPP table asks for it.
    ClockNotCovered = 4,
    /// PD_GPU did not come on (`refusal` is the [`rk3588::PdRefusal`]).
    PowerDomain = 5,
    /// `GPU_ID` is not the G610's.
    NotG610 = 6,
    /// The soft reset did not complete.
    Reset = 7,
    /// The L2 did not come up (`refusal` is the [`mali::MaliRefusal`]).
    L2 = 8,
}

/// Everything the run read, in the order it read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUp {
    pub pd_before: PdGpuRegs,
    pub clock: GpuClock,
    /// CLKGATE_CON(14) as found, and the SPI2 gates opened (bit mask of
    /// their bits).
    pub spi_gate14: u32,
    pub spi_opened: u32,
    /// SPI2 as handed over ([`SPI_FOUND`]).
    pub spi_found: [u32; 8],
    /// [`RK806_READ`], read as Linux reads them (`write_then_read`).
    pub rk806: [u8; 4],
    pub vdd_gpu_uv: u32,
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
/// the GPU (the hand-over state and the supply), so the image prints it
/// before a step that could fault.
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
    if o.rk806[0] & rk806::DCDC1_ON == 0 {
        o.stopped(Stop::SupplyOff, 0, u64::from(o.rk806[0]));
        return o;
    }
    match min_uv_for(o.clock.src_hz) {
        Some(uv) if o.vdd_gpu_uv >= uv => {}
        _ => {
            o.stopped(Stop::ClockNotCovered, 0, o.clock.src_hz);
            return o;
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

    #[test]
    fn the_opp_table_bounds_the_clock_by_the_supply() {
        assert_eq!(min_uv_for(300_000_000), Some(675_000));
        assert_eq!(min_uv_for(600_000_000), Some(675_000));
        assert_eq!(min_uv_for(600_000_001), Some(700_000));
        assert_eq!(min_uv_for(1_000_000_000), Some(850_000));
        assert_eq!(min_uv_for(1_000_000_001), None);
        assert_eq!(min_uv_for(0), None);
        // 198 MHz (GPLL / 6, panthor's probe-time rate) is inside the table
        assert_eq!(min_uv_for(198_000_000), Some(675_000));
    }

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
    /// aborts (here: panics) when read with PD_GPU off.
    struct T6 {
        regs: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        rk806: HashMap<u16, u8>,
        frame: RefCell<Vec<u8>>,
        rxq: RefCell<std::collections::VecDeque<u8>>,
        l2_on: RefCell<bool>,
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
            }
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

    #[test]
    fn from_the_handed_over_state_it_reads_the_supply_powers_and_finds_the_g610() {
        let t6 = T6::handed_over();
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!(o.pd_before.repair & 2, 0);
        assert_eq!(o.clock.src_hz, 300_000_000);
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
    fn a_supply_that_reads_off_stops_before_the_pmu_is_written() {
        let mut t6 = T6::handed_over();
        t6.rk806.insert(0x00, 0x0e);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.detail), (Stop::SupplyOff, 0x0e));
        assert!(t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0)));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
    }

    #[test]
    fn a_clock_the_supply_does_not_cover_stops_before_the_pmu_is_written() {
        // the PVTPLL: no rate the CRU can name
        let t6 = T6::handed_over();
        t6.regs.borrow_mut().insert(CRU_BASE + clksel_con(158), 0x5f24);
        assert_eq!(gpu_up(&t6, B, &mut |_| {}, &mut |_| {}).stop, Stop::ClockNotCovered);
        // CPLL / 2 = 750 MHz wants 750 mV; the supply gives 675 mV
        let t6 = T6::handed_over();
        t6.regs.borrow_mut().insert(CRU_BASE + clksel_con(158), 0x1f21);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.detail), (Stop::ClockNotCovered, 750_000_000));
        assert!(t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0)));
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
