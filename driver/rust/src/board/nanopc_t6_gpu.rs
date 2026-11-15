// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NanoPC-T6's GPU power-on, the second half of Z1
//! (driver/docs/nanopc-t6-mali.md): read what the kexec from Linux handed
//! over (PD_GPU, the GPU clock in the CRU, vdd_gpu_s0 in the RK806), put the
//! clock on a rate of the GPU OPP table when it is not on one
//! ([`rk3588::gpu_clock_set`]: the PVTPLL, an unknown mux or a divider off
//! the table become CPLL / 5 = 300 MHz; Armbian's state is left as it is),
//! check the supply covers that rate, power the clock's PLL on when Linux
//! left it powered down, power PD_GPU on, take over a GPU found powered
//! ([`takeover::take_over`]), read the GPU's identity, soft reset it and
//! power its L2 on. Every value read is kept in [`GpuUp`] for the image to
//! print; the first step that refuses ends the run there.
//!
//! The supply's voltage is read, never written. A supply lower than the GPU
//! OPP table asks for the rate the clock is on stops the run before PD_GPU
//! is touched. The clock set is the only CRU write before the supply check
//! (the SPI2 gates aside), and it touches CLKSEL_CON(158) and (159) only.
//!
//! The starting states (driver/docs/nanopc-t6-mali.md, "Open": PD_GPU and
//! vdd_gpu_s0 at the U-Boot hand-over):
//!
//! - PD_GPU off, vdd_gpu_s0 on: Linux's kexec state (measured 2026-10-19).
//! - PD_GPU off, vdd_gpu_s0 off: possible after U-Boot, since the supply is
//!   boot-on but not always-on (rk806 fact 4). vdd_gpu_s0 is PD_GPU's own
//!   supply (Linux v6.18 rk3588-nanopc-t6.dtsi:628-630), so it comes on before the
//!   domain, as in Linux, where the domain's supply is enabled before the
//!   domain is powered. Before anything is written, `BUCK1_ON_VSEL` must
//!   already cover the OPP minimum of the rate [`rk3588::gpu_clock_set`]
//!   chose ([`Stop::SupplyBelowOpp`] otherwise). After the hand-over report
//!   the one RK806 write, [`rk806::dcdc1_enable_frame`], is sent; then
//!   `POWER_EN0` is polled until DCDC1 reads on within
//!   [`rk806::DCDC1_RAMP_POLL_BUDGET_US`] ([`Stop::SupplyRampTimeout`]
//!   otherwise, before any PMU write), `BUCK1_ON_VSEL` is read again and
//!   checked again, and only then is PD_GPU powered.
//! - PD_GPU on with its idle request released ([`PdGpuRegs::usable`]):
//!   left as it is, no PMU write ([`rk3588::pd_gpu_on`]); with vdd_gpu_s0
//!   on only. A domain that reads powered (`REPAIR_STATUS` bit 1) while
//!   DCDC1 reads off is no state the facts cover: [`Stop::SupplyOff`],
//!   refusal [`SUPPLY_OFF_DOMAIN_POWERED`], nothing written to the RK806.
//!   A domain powered with its idle request still set gets the request
//!   released by the same step.
//!
//! A domain that read powered at hand-over holds a GPU another kernel may
//! still run (the crash kexec: panthor's firmware on the MCU, its masks set,
//! its address spaces on Linux's tables). Right after the PD_GPU step, which
//! writes no PMU word for it but opens the GPU's clock gates the GPU's
//! registers need, [`takeover::take_over`] masks and clears the three
//! interrupt blocks, halts the MCU and resets the GPU (hard when the soft
//! reset times out); only then the QoS words, the identity, Z1's own soft
//! reset and the L2. A refusal there is [`Stop::TakeOver`]. A domain that
//! read off gets none of it: its GPU words are those of the reset and the L2
//! alone. PD_GPU is never powered off here, so no idle request is needed
//! for a cut.
//!
//! No PMU step is added for U-Boot's state: [`rk3588::pd_gpu_on`] writes
//! `PWR_GATE` and `BUS_IDLE_REQ` through their write masks and waits on the
//! result, so it does not depend on the other domains' bits or on whether
//! the GPU's idle request was set when the domain is off.

use crate::gpu::mali::takeover::{self, TakeOver};
use crate::gpu::mali::{self, Ident, L2On, Reset};
use crate::ip::{rk806, rockchip_spi};
use crate::mmio::Mmio;
use crate::soc::rk3588::{self, ClockRefusal, ClockSet, GpuClock, PdGpuRegs, PdOn, PllOn, QosPolicy};

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
/// 24 MHz (`mux_200m_150m_24m_p`, Linux v6.18 clk-rk3588.c:455, 1182-1184; the T6 reads
/// 198 MHz), so 200 keeps the bus at or below the PMIC's 1 MHz
/// (rk3588-nanopc-t6.dtsi:765) whichever parent Linux left selected.
pub const RK806_BAUDR: u32 = 200;

/// The RK806 on SPI2 chip select 0 (v6.18 rk3588-nanopc-t6.dtsi:763-766). A
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
    /// DCDC1 (vdd_gpu_s0) reads off while PD_GPU reads powered (`refusal`
    /// [`SUPPLY_OFF_DOMAIN_POWERED`], the detail `POWER_EN0`). Nothing was
    /// written to the RK806. An off DCDC1 with PD_GPU off is turned on
    /// instead ([`Stop::SupplyBelowOpp`], [`Stop::SupplyRampTimeout`]).
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
    /// DCDC1 reads off and `BUCK1_ON_VSEL` gives less than the OPP table
    /// asks for the rate the clock is on after [`rk3588::gpu_clock_set`].
    /// `refusal` 0: found before the enable, nothing was written to the
    /// RK806; [`SUPPLY_BELOW_AFTER_ENABLE`]: read again after the enable.
    /// The detail is `BUCK1_ON_VSEL` << 32 | the rate in Hz. No voltage is
    /// ever written.
    SupplyBelowOpp = 11,
    /// The DCDC1-enable frame was sent and `POWER_EN0` did not read DCDC1 on
    /// within [`rk806::DCDC1_RAMP_POLL_BUDGET_US`] (`refusal` 0, the detail
    /// the last `POWER_EN0`). Nothing was written to the PMU.
    SupplyRampTimeout = 12,
    /// The DCDC1-enable frame could not be sent (`refusal` is the
    /// [`rockchip_spi::SpiRefusal`], the detail its value). Whether it
    /// reached the PMIC is unknown; nothing was written to the PMU.
    Rk806Write = 13,
    /// The take-over of a GPU found powered ([`takeover::take_over`])
    /// refused (`refusal` is the [`mali::MaliRefusal`], the detail its
    /// value). No QoS word, no `GPU_CMD` past the take-over's and no L2
    /// power-on was written.
    TakeOver = 14,
}

/// The refusal of [`Stop::ClockNotCovered`] when the GPU clock's PLL did not
/// lock after its power-on ([`rk3588::pll_on`]).
pub const CLOCK_PLL_NO_LOCK: u32 = 1;

/// The refusal of [`Stop::SupplyOff`]: PD_GPU reads powered with DCDC1 off.
pub const SUPPLY_OFF_DOMAIN_POWERED: u32 = 1;

/// The refusal of [`Stop::SupplyBelowOpp`] when `BUCK1_ON_VSEL`, read again
/// after DCDC1 came on, no longer covers the rate.
pub const SUPPLY_BELOW_AFTER_ENABLE: u32 = 1;

/// The wait between two `POWER_EN0` reads while DCDC1 comes on, in µs (each
/// read is itself 4 frames, about 32 µs at 1 MHz).
pub const SUPPLY_POLL_US: u32 = 100;

/// The least GPU mapping [`gpu_up`] needs at `Bases::gpu`: the take-over
/// reaches the MMU interrupt block ([`takeover::MAP_MIN`]); every other step
/// stays below 0x1000. The C side maps at least `YD_T6_GPU_MAP_MIN`, which
/// the header pins to this.
pub const GPU_MAP_MIN: usize = takeover::MAP_MIN;

/// The QoS words `gpu_up` writes after the power-on. Every Z1-Z4 board run
/// so far worked under Linux's zeroed QoS, so the U-Boot route applies the
/// same; `Reset` (the hardware reset values U-Boot leaves) is the
/// alternative a board session may try.
pub const GPU_QOS_POLICY: QosPolicy = QosPolicy::LinuxZero;

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
    /// The voltage the supply check holds against the rate:
    /// `BUCK1_ON_VSEL` as handed over, or as read again after the enable.
    pub vdd_gpu_uv: u32,
    /// The DCDC1-enable frame was sent (DCDC1 read off at hand-over).
    pub supply_enabled: bool,
    /// `POWER_EN0` and `BUCK1_ON_VSEL` read after the enable (the last
    /// `POWER_EN0` of the poll); 0 when no enable was sent.
    pub supply_after: [u8; 2],
    /// How long DCDC1 took to read on after the enable, in µs.
    pub supply_waited_us: u32,
    /// The power-on of the PLL `CLK_GPU_SRC` runs from.
    pub pll_on: PllOn,
    pub pd_on: PdOn,
    pub pd_after: PdGpuRegs,
    /// The take-over of a GPU found powered; `None` when PD_GPU read off at
    /// hand-over (or the run stopped before the step).
    pub take_over: Option<TakeOver>,
    /// `qos_gpu_m0..m3`, five words each, read after the power-on.
    pub qos: [[u32; 5]; 4],
    /// [`QosPolicy::id`] of the policy applied after the power-on; 0 when
    /// the run stopped before that step.
    pub qos_policy: u32,
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

    /// `vdd_gpu_uv` is at least the OPP table's voltage for the rate the
    /// clock is on (false for a rate not in the table).
    fn supply_covers_clock(&self) -> bool {
        min_uv_for(self.clock_set.src_hz).is_some_and(|uv| self.vdd_gpu_uv >= uv)
    }
}

/// The run (see the module). `delay_us` waits that many microseconds.
///
/// `on_handover` sees what the run read before its first write to the RK806,
/// the PMU or the GPU (the hand-over state, the supply and the PLL's
/// power-on; `supply_enabled` is still false there), so the image prints it
/// before a step that could fault.
pub fn gpu_up(m: &impl Mmio, b: Bases, delay_us: &mut impl FnMut(u32), on_handover: &mut impl FnMut(&GpuUp)) -> GpuUp {
    gpu_up_with(GPU_QOS_POLICY, m, b, delay_us, on_handover)
}

/// [`gpu_up`] applying `qos` after the power-on instead of [`GPU_QOS_POLICY`]
/// (the C side picks it with `yd_t6_gpu_qos_policy`).
pub fn gpu_up_with(
    qos: QosPolicy,
    m: &impl Mmio,
    b: Bases,
    delay_us: &mut impl FnMut(u32),
    on_handover: &mut impl FnMut(&GpuUp),
) -> GpuUp {
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
        match rk806_read(m, &bus, reg, delay_us) {
            Ok(v) => o.rk806[i] = v,
            Err((why, detail)) => {
                o.stopped(Stop::Rk806Read, why, detail);
                return o;
            }
        }
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
    let supply_off = o.rk806[0] & rk806::DCDC1_ON == 0;
    if supply_off && o.pd_before.repair & rk3588::pd_gpu::REPAIR_ON_BIT != 0 {
        o.stopped(Stop::SupplyOff, SUPPLY_OFF_DOMAIN_POWERED, u64::from(o.rk806[0]));
        return o;
    }
    if !o.supply_covers_clock() {
        if supply_off {
            o.stopped(Stop::SupplyBelowOpp, 0, u64::from(o.rk806[3]) << 32 | o.clock_set.src_hz);
        } else {
            o.stopped(Stop::ClockNotCovered, 0, o.clock_set.src_hz);
        }
        return o;
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

    // vdd_gpu_s0 is PD_GPU's supply: on before the domain
    if supply_off && !supply_on(m, &bus, delay_us, &mut o) {
        return o;
    }
    match rk3588::pd_gpu_on(m, b.cru, b.pmu, delay_us) {
        Ok(on) => o.pd_on = on,
        Err((why, now)) => {
            o.stopped(Stop::PowerDomain, why as u32, u64::from(now));
            o.pd_after = PdGpuRegs::read(m, b.pmu);
            return o;
        }
    }
    o.pd_after = PdGpuRegs::read(m, b.pmu);
    // a GPU found powered may still run another kernel's firmware: quiet and
    // reset before anything else is written to it (or to its QoS blocks)
    if o.pd_before.repair & rk3588::pd_gpu::REPAIR_ON_BIT != 0 {
        let mut t = TakeOver::default();
        let r = takeover::take_over(m, b.gpu, delay_us, &mut t);
        o.take_over = Some(t);
        if let Err((why, detail)) = r {
            o.stopped(Stop::TakeOver, why as u32, detail);
            return o;
        }
    }
    rk3588::qos_apply(m, b.qos, qos);
    o.qos_policy = qos.id();
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

/// One RK806 register, read as Linux reads it (`write_then_read`); on a
/// refusal the [`Stop::Rk806Read`] refusal and detail.
fn rk806_read(
    m: &impl Mmio,
    bus: &rockchip_spi::Host,
    reg: u16,
    delay_us: &mut impl FnMut(u32),
) -> Result<u8, (u32, u64)> {
    let mut tx = [0u8; 4];
    let n = rk806::read_frame(reg, 1, &mut tx).expect("a 1-register read fits 4 bytes");
    let mut v = [0u8; 1];
    rockchip_spi::write_then_read(m, bus, &tx[..n - 1], &mut v, delay_us)
        .map_err(|(why, detail)| (why as u32, u64::from(reg) << 32 | u64::from(detail)))?;
    Ok(v[0])
}

/// DCDC1 on from off (see the module): the one RK806 write, `POWER_EN0`
/// polled until DCDC1 reads on, `BUCK1_ON_VSEL` read again and held against
/// the rate again. False when it stopped (`o` says why).
fn supply_on(m: &impl Mmio, bus: &rockchip_spi::Host, delay_us: &mut impl FnMut(u32), o: &mut GpuUp) -> bool {
    let mut tx = [0u8; 4];
    let n = rk806::dcdc1_enable_frame(&mut tx).expect("the enable frame fits 4 bytes");
    if let Err((why, detail)) = rockchip_spi::write(m, bus, &tx[..n], delay_us) {
        o.stopped(Stop::Rk806Write, why as u32, u64::from(detail));
        return false;
    }
    o.supply_enabled = true;
    let mut waited = 0;
    loop {
        match rk806_read(m, bus, rk806::POWER_EN0, delay_us) {
            Ok(v) => o.supply_after[0] = v,
            Err((why, detail)) => {
                o.stopped(Stop::Rk806Read, why, detail);
                return false;
            }
        }
        if o.supply_after[0] & rk806::DCDC1_ON != 0 {
            break;
        }
        if waited >= rk806::DCDC1_RAMP_POLL_BUDGET_US {
            o.stopped(Stop::SupplyRampTimeout, 0, u64::from(o.supply_after[0]));
            return false;
        }
        delay_us(SUPPLY_POLL_US);
        waited += SUPPLY_POLL_US;
    }
    o.supply_waited_us = waited;
    match rk806_read(m, bus, rk806::BUCK1_ON_VSEL, delay_us) {
        Ok(v) => o.supply_after[1] = v,
        Err((why, detail)) => {
            o.stopped(Stop::Rk806Read, why, detail);
            return false;
        }
    }
    o.vdd_gpu_uv = rk806::buck_uv(o.supply_after[1]);
    if !o.supply_covers_clock() {
        let detail = u64::from(o.supply_after[1]) << 32 | o.clock_set.src_hz;
        o.stopped(Stop::SupplyBelowOpp, SUPPLY_BELOW_AFTER_ENABLE, detail);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gpu::mali::{csf, mmu, MaliRefusal};
    use crate::soc::rk3588::{clkgate_con, clksel_con, pd_gpu, pll_con, CRU_BASE, MODE_CON0, PMU_BASE};
    use std::cell::RefCell;
    use std::collections::HashMap;

    const SPI: usize = rk3588::SPI2_BASE;
    /// A mapped base that is not the physical address, as under Zephyr.
    const QOS: usize = 0x7000_0000;
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
        rk806: RefCell<HashMap<u16, u8>>,
        /// The RK806 write frames received, whole.
        rk806_writes: RefCell<Vec<Vec<u8>>>,
        /// After an enable of DCDC1, the `POWER_EN0` reads that still find it
        /// off; `None`: it never comes on.
        dcdc1_ramp: Option<u32>,
        ramp_left: RefCell<Option<u32>>,
        /// The RK806 does not answer the frames of a write.
        rk806_write_deaf: bool,
        frame: RefCell<Vec<u8>>,
        rxq: RefCell<std::collections::VecDeque<u8>>,
        l2_on: RefCell<bool>,
        /// No PLL locks.
        pll_dead: bool,
        /// Registers that ignore writes (logged, not applied).
        stuck: Vec<usize>,
        /// After `MCU_CONTROL` := `DISABLE`, the `MCU_STATUS` reads that
        /// still find the MCU enabled; `None`: it never stops.
        mcu_halt_lag: Option<u32>,
        halt_left: RefCell<Option<u32>>,
        /// How many soft resets (from the first) do not complete; a hard
        /// reset completes unless `hard_reset_dead`.
        soft_resets_dead: RefCell<u32>,
        hard_reset_dead: bool,
        /// (command, the three `*_INT_MASK`, `MCU_STATUS`) at each reset
        /// command.
        resets: RefCell<Vec<(u32, [u32; 3], u32)>>,
        /// The highest GPU offset read or written, plus 4.
        gpu_end: std::cell::Cell<usize>,
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
                rk806: RefCell::new([(0x00, 0x0f), (0x06, 0xe0), (0x10, 0xa4), (0x1a, 0x1c)].into_iter().collect()),
                rk806_writes: RefCell::new(vec![]),
                dcdc1_ramp: Some(0),
                ramp_left: RefCell::new(None),
                rk806_write_deaf: false,
                frame: RefCell::new(vec![]),
                rxq: RefCell::new(Default::default()),
                l2_on: RefCell::new(false),
                pll_dead: false,
                stuck: vec![],
                mcu_halt_lag: Some(3),
                halt_left: RefCell::new(None),
                soft_resets_dead: RefCell::new(0),
                hard_reset_dead: false,
                resets: RefCell::new(vec![]),
                gpu_end: std::cell::Cell::new(0),
            }
        }
        /// After a crash kexec (driver/docs/nanopc-t6-display.md 2(a)): PD_GPU
        /// on and out of idle, and the GPU as panthor runs it: its interrupt
        /// masks (`GPU_INTERRUPTS_MASK`, panthor_gpu.c v6.18:43-47; the job
        /// mask ~0, panthor_fw.c:1033; the MMU's fault bits), the
        /// firmware's global interrupt and an MMU fault pending, the MCU
        /// enabled (`MCU_CONTROL` `AUTO`), AS0 on a table in Linux's memory.
        fn left_by_panthor() -> T6 {
            let t6 = T6::handed_over_domain_on();
            for (a, v) in [
                (mali::GPU_INT_MASK, 0x2_0103),
                (csf::JOB_INT_MASK, u32::MAX),
                (mmu::MMU_INT_MASK, mmu::MMU_INT_FAULTS),
                (csf::JOB_INT_RAWSTAT, csf::JOB_INT_GLOBAL_IF),
                (mmu::MMU_INT_RAWSTAT, 1),
                (csf::MCU_CONTROL, csf::MCU_CONTROL_AUTO),
                (mali::MCU_STATUS, csf::MCU_STATUS_ENABLED),
                (mmu::mmu_as(0) + mmu::AS_TRANSTAB, 0x1_2340_0000_u64 as u32),
                (mmu::mmu_as(0) + mmu::AS_TRANSCFG, 0x4200_01c6),
            ] {
                t6.regs.borrow_mut().insert(GPU + a, v);
            }
            t6
        }
        fn touch(&self, a: usize) {
            if (GPU..GPU + 0x10000).contains(&a) {
                self.gpu_end.set(self.gpu_end.get().max(a - GPU + 4));
            }
        }
        fn gpu_reg(&self, off: usize) -> u32 {
            *self.regs.borrow().get(&(GPU + off)).unwrap_or(&0)
        }
        fn masks(&self) -> [u32; 3] {
            takeover::IRQ_BLOCKS.map(|(_, _, mask)| self.gpu_reg(mask))
        }
        /// The hand-over of 2026-11-11: the GPU last ran at 850 MHz, so
        /// `CLK_GPU_SRC` is NPLL / 1 at 800 mV, and Linux powered NPLL down
        /// when it suspended the GPU. 850 MHz is no rate of the OPP table
        /// the code holds (v6.18 rk3588-opp.dtsi:118-153).
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
            t6.rk806.get_mut().insert(0x1a, 0x30);
            t6
        }
        /// A possible U-Boot hand-over: Armbian's, with DCDC1 off
        /// (`POWER_EN0` 0x0e) and its voltage register at `vsel`.
        fn handed_over_supply_off(vsel: u8) -> T6 {
            let mut t6 = T6::handed_over();
            t6.rk806.get_mut().extend([(0x00, 0x0e), (0x1a, vsel)]);
            t6
        }
        /// PD_GPU left on and out of idle (Armbian's values with the GPU
        /// active, 2026-10-19).
        fn handed_over_domain_on() -> T6 {
            let t6 = T6::handed_over();
            for (a, v) in [
                (pd_gpu::PWR_GATE, 0xfffe),
                (pd_gpu::BUS_IDLE_REQ, 0x83ce),
                (pd_gpu::BUS_IDLE_ACK, 0x000a_9ffe),
                (pd_gpu::BUS_IDLE_ST, 0x000a_9ffe),
                (pd_gpu::REPAIR_STATUS, 0xfad5_0003),
            ] {
                t6.regs.borrow_mut().insert(PMU_BASE + a, v);
            }
            t6.regs.borrow_mut().insert(CRU_BASE + clkgate_con(66), 0x04);
            t6
        }
        /// An RK806 read as Linux reads it: the value of `reg`, with
        /// DCDC1's ramp after an enable.
        fn rk806_answer(&self, reg: u16) -> u8 {
            if reg == rk806::POWER_EN0 {
                let mut left = self.ramp_left.borrow_mut();
                match *left {
                    Some(0) => {
                        *self.rk806.borrow_mut().get_mut(&reg).unwrap() |= rk806::DCDC1_ON;
                        *left = None;
                    }
                    Some(n) => *left = Some(n - 1),
                    None => {}
                }
            }
            self.rk806.borrow()[&reg]
        }
        /// A whole write frame: only `POWER_EN0`, applied through its
        /// write-enable twins (bits 7:4 for 3:0, rk806 fact 2); DCDC1's on
        /// bit follows `dcdc1_ramp`.
        fn rk806_write(&self, f: &[u8]) {
            self.rk806_writes.borrow_mut().push(f.to_vec());
            assert_eq!(f.len(), 4, "one-value writes only: {f:02x?}");
            assert_eq!(f[0], 0x80, "{f:02x?}");
            assert_eq!(u16::from_le_bytes([f[1], f[2]]), rk806::POWER_EN0, "a write to another register: {f:02x?}");
            let (we, v) = (f[3] >> 4, f[3] & 0x0f);
            let mut map = self.rk806.borrow_mut();
            let en = map.get_mut(&rk806::POWER_EN0).unwrap();
            let delayed = we & v & rk806::DCDC1_ON & !*en;
            *en = (*en & !we) | (v & we & !delayed);
            if delayed != 0 {
                *self.ramp_left.borrow_mut() = self.dcdc1_ramp;
            }
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
            self.touch(a);
            if (GPU..GPU + 0x10000).contains(&a) || (QOS..QOS + 0x800).contains(&a) {
                assert!(self.gpu_powered(), "read of {a:#x} with PD_GPU off: an external abort on the board");
            }
            // the MCU stops `mcu_halt_lag` reads after its disable; while it
            // runs, the firmware raises its global interrupt again
            if a == GPU + mali::MCU_STATUS {
                let mut left = self.halt_left.borrow_mut();
                match *left {
                    Some(0) => {
                        self.regs.borrow_mut().insert(a, csf::MCU_STATUS_DISABLED);
                        *left = None;
                    }
                    Some(n) => *left = Some(n - 1),
                    None => {}
                }
            }
            if a == GPU + csf::JOB_INT_RAWSTAT && self.gpu_reg(mali::MCU_STATUS) == csf::MCU_STATUS_ENABLED {
                *self.regs.borrow_mut().entry(a).or_insert(0) |= csf::JOB_INT_GLOBAL_IF;
            }
            match a {
                _ if a == SPI + rockchip_spi::RXFLR => self.rxq.borrow().len() as u32,
                _ if a == SPI + rockchip_spi::RXDR => u32::from(self.rxq.borrow_mut().pop_front().unwrap()),
                _ if a == GPU + mali::L2_READY => u32::from(*self.l2_on.borrow()),
                _ => *self.regs.borrow().get(&a).unwrap_or(&0),
            }
        }
        fn write32(&self, a: usize, v: u32) {
            self.touch(a);
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
                if f[0] & 0x80 != 0 {
                    // a write: whole after its value byte
                    if f.len() == 4 {
                        let whole = f.clone();
                        drop(f);
                        self.rk806_write(&whole);
                    }
                    if !self.rk806_write_deaf {
                        self.rxq.borrow_mut().push_back(0);
                    }
                    return;
                }
                // the RK806 answers the value in the fourth byte of a 1-value read
                let out = if f.len() == 4 {
                    assert_eq!(f[0], 0, "only 1-value reads: {f:02x?}");
                    let reg = u16::from_le_bytes([f[1], f[2]]);
                    drop(f);
                    self.rk806_answer(reg)
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
                assert_eq!(f[0], 0, "only 1-value reads here: {f:02x?}");
                let v = self.rk806_answer(u16::from_le_bytes([f[1], f[2]]));
                self.rxq.borrow_mut().push_back(v);
            }
            if a == GPU + mali::L2_PWRON && v & 1 != 0 {
                *self.l2_on.borrow_mut() = true;
            }
            // a reset completes at once (unless dead) and stops the MCU; it
            // keeps the masks and MCU_STATUS it met
            if a == GPU + mali::GPU_CMD && (v == mali::CMD_SOFT_RESET || v == mali::CMD_HARD_RESET) {
                self.resets.borrow_mut().push((v, self.masks(), self.gpu_reg(mali::MCU_STATUS)));
                let dead = if v == mali::CMD_SOFT_RESET {
                    let mut n = self.soft_resets_dead.borrow_mut();
                    let dead = *n > 0;
                    *n = n.saturating_sub(1);
                    dead
                } else {
                    self.hard_reset_dead
                };
                if !dead {
                    let mut r = self.regs.borrow_mut();
                    *r.entry(GPU + mali::GPU_INT_RAWSTAT).or_insert(0) |= mali::IRQ_RESET_COMPLETED;
                    r.insert(GPU + mali::MCU_STATUS, csf::MCU_STATUS_DISABLED);
                }
                return;
            }
            // an *_INT_CLEAR clears raw bits
            if let Some(&(raw, _, _)) = takeover::IRQ_BLOCKS.iter().find(|b| a == GPU + b.1) {
                *self.regs.borrow_mut().entry(GPU + raw).or_insert(0) &= !v;
                return;
            }
            if a == GPU + csf::MCU_CONTROL && v == csf::MCU_CONTROL_DISABLE {
                *self.halt_left.borrow_mut() = self.mcu_halt_lag;
            }
            // PD_GPU is cut only after its idle request (pm-domains.c v6.18:
            // 615-660): a power-off word with the request released is a bug
            if a == PMU_BASE + pd_gpu::PWR_GATE
                && v & (pd_gpu::PWR_BIT << 16 | pd_gpu::PWR_BIT) == (pd_gpu::PWR_BIT << 16 | pd_gpu::PWR_BIT)
            {
                let req = self.regs.borrow()[&(PMU_BASE + pd_gpu::BUS_IDLE_REQ)];
                assert!(req & 1 != 0, "PD_GPU cut without its idle request: {:#x}", req);
            }
            let mut r = self.regs.borrow_mut();
            if (CRU_BASE..CRU_BASE + 0x1000).contains(&a) || (PMU_BASE..PMU_BASE + 0x400).contains(&a) {
                let e = r.entry(a).or_insert(0);
                let mask = v >> 16;
                *e = (*e & !mask) | (v & mask);
                // PD_GPU follows its controls at once
                // and powers only with its supply, DCDC1, on
                if a == PMU_BASE + pd_gpu::PWR_GATE
                    && v == 1 << 16
                    && self.rk806.borrow()[&rk806::POWER_EN0] & rk806::DCDC1_ON != 0
                {
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
    fn each_qos_policy_is_applied_and_named_in_the_record() {
        for (policy, id, words) in [(QosPolicy::Reset, 1, [3, 0x12d, 0x40]), (QosPolicy::LinuxZero, 2, [0; 3])] {
            let t6 = T6::handed_over();
            let o = gpu_up_with(policy, &t6, B, &mut |_| {}, &mut |_| {});
            assert_eq!(o.stop, Stop::None, "{policy:?}");
            assert_eq!((o.qos_policy, policy.id()), (id, id));
            for blk in 0..4 {
                assert_eq!(o.qos[blk][1..4], words, "{policy:?} block {blk}");
                assert_eq!(o.qos[blk][0], 0x8000_0000);
            }
        }
        let o = gpu_up(&T6::handed_over(), B, &mut |_| {}, &mut |_| {});
        assert_eq!(o.qos_policy, GPU_QOS_POLICY.id());
    }

    #[test]
    fn a_run_stopped_before_the_qos_step_names_no_policy() {
        let mut t6 = T6::handed_over_domain_on();
        t6.rk806.get_mut().insert(rk806::POWER_EN0, 0x0e);
        let o = gpu_up_with(QosPolicy::Reset, &t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!(o.stop, Stop::SupplyOff);
        assert_eq!((o.qos_policy, o.qos), (0, [[0; 5]; 4]));
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
        let q = |blk: usize, off: usize| QOS + 0x200 * blk + off;
        for blk in 0..4 {
            for off in rk3588::QOS_POLICY_WORDS {
                assert!(t6.writes.borrow().contains(&(q(blk, off), 0)));
            }
        }
        assert!(t6.writes.borrow().iter().all(|w| !rk3588::QOS_GPU.iter().any(|&p| (p..p + 0x200).contains(&w.0))));
        assert_eq!(o.ident.gpu_id, 0xa867_0005);
        assert_eq!(o.ident_after_reset.shader_present, 0x50005);
        assert_eq!(o.l2.ready, 1);
        // nothing was written to the RK806 frame but reads
        let tx: Vec<u8> =
            t6.writes.borrow().iter().filter(|w| w.0 == SPI + rockchip_spi::TXDR).map(|w| w.1 as u8).collect();
        let want: Vec<u8> = [0x00u8, 0x06, 0x10, 0x1a].iter().flat_map(|&r| [0, r, 0]).collect();
        assert_eq!(tx, want);
    }

    /// The bus writes that start an RK806 write frame (its command byte has
    /// `RK806_CMD_WRITE`; a read command never has, rk806 fact 1).
    fn rk806_write_starts(w: &[(usize, u32)]) -> Vec<usize> {
        w.iter()
            .enumerate()
            .filter(|(_, w)| w.0 == SPI + rockchip_spi::TXDR && w.1 & 0x80 != 0)
            .map(|(i, _)| i)
            .collect()
    }

    fn no_pmu_write(t6: &T6) -> bool {
        t6.writes.borrow().iter().all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0))
    }

    #[test]
    fn the_handover_report_comes_before_any_pmu_gpu_or_rk806_write() {
        // Linux's state (no RK806 write at all) and a U-Boot state (one)
        for (t6, en0, enables) in [(T6::handed_over(), 0x0f, 0), (T6::handed_over_supply_off(0x1c), 0x0e, 1)] {
            let mut seen = None;
            let o = gpu_up(&t6, B, &mut |_| {}, &mut |h| seen = Some((*h, t6.writes.borrow().len())));
            let (h, writes_then) = seen.expect("on_handover was not called");
            assert_eq!((h.rk806, h.vdd_gpu_uv, h.clock.src_hz), ([en0, 0xe0, 0xa4, 0x1c], 675_000, 300_000_000));
            assert!(!h.supply_enabled);
            let w = t6.writes.borrow();
            let before = &w[..writes_then];
            assert!(
                before
                    .iter()
                    .all(|w| !(PMU_BASE..PMU_BASE + 0x400).contains(&w.0) && !(GPU..GPU + 0x10000).contains(&w.0)),
                "{before:x?}"
            );
            let starts = rk806_write_starts(&w);
            assert_eq!(starts.len(), enables, "{w:x?}");
            assert!(starts.iter().all(|&i| i >= writes_then), "an RK806 write before the report: {w:x?}");
            assert_eq!(o.stop, Stop::None);
        }
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

    // The starting states of the module: (a) is
    // from_the_handed_over_state_it_reads_the_supply_powers_and_finds_the_g610
    // (Linux's state, no RK806 write); (b), (b'), (b''), (c) follow.

    #[test]
    fn b_supply_off_covering_300_mhz_is_enabled_by_one_frame_then_the_domain_comes_on() {
        // 0x1c = 675 mV covers CPLL / 5 = 300 MHz (675 mV in the table)
        let mut t6 = T6::handed_over_supply_off(0x1c);
        t6.dcdc1_ramp = Some(3);
        let mut slept = 0;
        let o = gpu_up(&t6, B, &mut |us| slept += us, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!(o.rk806, [0x0e, 0xe0, 0xa4, 0x1c]);
        assert!(o.supply_enabled);
        assert_eq!((o.supply_after, o.supply_waited_us), ([0x0f, 0x1c], 3 * SUPPLY_POLL_US));
        assert_eq!(o.vdd_gpu_uv, 675_000);
        assert!(slept >= 3 * SUPPLY_POLL_US);
        // exactly one RK806 write frame, byte for byte rk806-enable's, sent whole
        assert_eq!(*t6.rk806_writes.borrow(), vec![vec![0x80, 0x00, 0x00, 0x11]]);
        let w = t6.writes.borrow();
        let starts = rk806_write_starts(&w);
        assert_eq!(starts.len(), 1);
        let tx: Vec<u8> =
            w[starts[0]..].iter().filter(|w| w.0 == SPI + rockchip_spi::TXDR).take(4).map(|w| w.1 as u8).collect();
        assert_eq!(tx, [0x80, 0x00, 0x00, 0x11]);
        // DCDC2..4 kept as they were, DCDC1 on
        assert_eq!(t6.rk806.borrow()[&rk806::POWER_EN0], 0x0f);
        // the write, then the POWER_EN0 polls and BUCK1_ON_VSEL, then the PMU
        let first_pmu = w.iter().position(|w| (PMU_BASE..PMU_BASE + 0x400).contains(&w.0)).unwrap();
        let last_spi = w.iter().rposition(|w| (SPI..SPI + 0x1000).contains(&w.0)).unwrap();
        assert!(starts[0] < last_spi && last_spi < first_pmu, "{w:x?}");
        assert!(o.pd_after.usable());
        assert!(!o.pd_on.was_on);
        assert_eq!(o.ident.gpu_id, 0xa867_0005);
        assert_eq!(o.l2.ready, 1);
    }

    #[test]
    fn b_prime_supply_off_below_the_opp_stops_without_a_write() {
        // 0x17 = 643.75 mV, under 300 MHz's 675 mV
        let t6 = T6::handed_over_supply_off(0x17);
        let mut reported = false;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| reported = true);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::SupplyBelowOpp, 0, 0x17 << 32 | 300_000_000));
        assert!(!reported && !o.supply_enabled);
        assert!(t6.rk806_writes.borrow().is_empty());
        assert!(rk806_write_starts(&t6.writes.borrow()).is_empty());
        assert!(no_pmu_write(&t6));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
    }

    #[test]
    fn b_double_prime_supply_that_never_ramps_stops_without_a_pmu_write() {
        let mut t6 = T6::handed_over_supply_off(0x1c);
        t6.dcdc1_ramp = None;
        let mut slept = 0;
        let o = gpu_up(&t6, B, &mut |us| slept += us, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::SupplyRampTimeout, 0, 0x0e));
        assert!(o.supply_enabled);
        assert_eq!(o.supply_after, [0x0e, 0]);
        assert!(slept >= rk806::DCDC1_RAMP_POLL_BUDGET_US);
        assert_eq!(t6.rk806_writes.borrow().len(), 1);
        assert!(no_pmu_write(&t6));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != CRU_BASE + clkgate_con(66)));
    }

    #[test]
    fn c_a_domain_left_on_is_not_written_in_the_pmu() {
        let t6 = T6::handed_over_domain_on();
        assert!(PdGpuRegs::read(&t6, PMU_BASE).usable());
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert_eq!(o.pd_on, PdOn { opened: 0, was_on: true, waited_us: [0; 3] });
        assert!(!o.supply_enabled);
        assert!(no_pmu_write(&t6));
        assert!(t6.rk806_writes.borrow().is_empty());
        assert_eq!(o.ident.gpu_id, 0xa867_0005);
    }

    #[test]
    fn a_domain_that_reads_powered_with_the_supply_off_is_refused_without_a_write() {
        let mut t6 = T6::handed_over_domain_on();
        t6.rk806.get_mut().insert(rk806::POWER_EN0, 0x0e);
        let mut reported = false;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| reported = true);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::SupplyOff, SUPPLY_OFF_DOMAIN_POWERED, 0x0e));
        assert!(!reported);
        assert!(t6.rk806_writes.borrow().is_empty());
        assert!(no_pmu_write(&t6));
    }

    #[test]
    fn a_voltage_that_reads_lower_after_the_enable_stops_before_the_pmu() {
        // covered at hand-over; the model's BUCK1_ON_VSEL drops when DCDC1 comes on
        struct Dropping(T6);
        impl Mmio for Dropping {
            fn read32(&self, a: usize) -> u32 {
                if self.0.rk806.borrow()[&rk806::POWER_EN0] & rk806::DCDC1_ON != 0 {
                    self.0.rk806.borrow_mut().insert(rk806::BUCK1_ON_VSEL, 0x10);
                }
                self.0.read32(a)
            }
            fn write32(&self, a: usize, v: u32) {
                self.0.write32(a, v)
            }
            fn read8(&self, _: usize) -> u8 {
                unreachable!()
            }
            fn write8(&self, _: usize, _: u8) {
                unreachable!()
            }
        }
        let d = Dropping(T6::handed_over_supply_off(0x1c));
        let o = gpu_up(&d, B, &mut |_| {}, &mut |_| {});
        assert_eq!(
            (o.stop, o.refusal, o.detail),
            (Stop::SupplyBelowOpp, SUPPLY_BELOW_AFTER_ENABLE, 0x10 << 32 | 300_000_000)
        );
        assert_eq!((o.supply_after, o.vdd_gpu_uv), ([0x0f, 0x10], 600_000));
        assert!(no_pmu_write(&d.0));
    }

    #[test]
    fn an_enable_frame_the_bus_does_not_take_stops_before_the_pmu() {
        let mut t6 = T6::handed_over_supply_off(0x1c);
        t6.rk806_write_deaf = true;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::Rk806Write, rockchip_spi::SpiRefusal::RxTimeout as u32, 0));
        assert!(!o.supply_enabled);
        assert!(no_pmu_write(&t6));
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
        t6.rk806.get_mut().insert(0x1a, 0x1c);
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

    /// `gpu_up` over PD_GPU x vdd_gpu_s0 x the clock as handed over. Every
    /// clock lands on CPLL / 5 = 300 MHz (the 675 mV supply covers it), so
    /// the outcome per (PD_GPU, DCDC1) is one row:
    ///
    /// | PD_GPU | DCDC1 | outcome                                       | SPI write    | PMU write |
    /// |--------|-------|-----------------------------------------------|--------------|-----------|
    /// | on     | on    | success, GPU id read                          | none         | none      |
    /// | on     | off   | `SupplyOff`, refusal 1, detail POWER_EN0 0x0e | none         | none      |
    /// | off    | on    | success, GPU id read                          | none         | yes       |
    /// | off    | off   | success, GPU id read                          | DCDC1 enable | yes       |
    ///
    /// The clock states: Armbian's 0x1f24 (kept), the PVTPLL (0x5f24), an
    /// unknown mux (0x1fe4) and a divider off the table (0x1f21, 750 MHz).
    #[test]
    fn gpu_up_over_pd_gpu_vdd_gpu_and_clock_start_states() {
        let clocks = [(0x1f24, true), (0x5f24, false), (0x1fe4, false), (0x1f21, false)];
        for pd_on in [true, false] {
            for vdd_on in [true, false] {
                for (sel, kept) in clocks {
                    let what = format!("pd_on={pd_on} vdd_on={vdd_on} sel158={sel:#x}");
                    let t6 = if pd_on { T6::handed_over_domain_on() } else { T6::handed_over() };
                    if !vdd_on {
                        t6.rk806.borrow_mut().insert(0x00, 0x0e);
                    }
                    t6.regs.borrow_mut().insert(SEL158, sel);
                    let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});

                    if pd_on && !vdd_on {
                        assert_eq!(
                            (o.stop, o.refusal, o.detail),
                            (Stop::SupplyOff, SUPPLY_OFF_DOMAIN_POWERED, 0x0e),
                            "{what}"
                        );
                    } else {
                        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0), "{what}");
                        assert_eq!(o.ident.gpu_id, mali::G610_GPU_ID, "{what}");
                        assert!(o.pd_after.usable(), "{what}");
                    }
                    // the clock is on the table whenever the run got past the clock set
                    assert_eq!(o.clock_set.src_hz, 300_000_000, "{what}");
                    assert!(min_uv_for(o.clock_set.src_hz).is_some(), "{what}");
                    assert_eq!(o.clock_set.kept, kept, "{what}");
                    assert_eq!(t6.regs.borrow()[&SEL158], 0x1f24, "{what}");

                    // the only SPI write frame is the DCDC1 enable, and only for an off DCDC1
                    let frames = t6.rk806_writes.borrow();
                    let enable = !pd_on && !vdd_on;
                    assert_eq!(frames.len(), usize::from(enable), "{what}: {frames:02x?}");
                    assert_eq!(o.supply_enabled, enable, "{what}");

                    let writes = t6.writes.borrow();
                    let in_pmu = |a: usize| (PMU_BASE..PMU_BASE + 0x400).contains(&a);
                    assert_eq!(writes.iter().any(|w| in_pmu(w.0)), !pd_on, "{what}");
                    // the CRU: CLKSEL_CON(158)/(159), and the gates of SPI2 and the GPU; no PLL word
                    for w in writes.iter().filter(|w| (CRU_BASE..CRU_BASE + 0x1000).contains(&w.0)) {
                        let ok = [clksel_con(158), clksel_con(159), clkgate_con(14), clkgate_con(66)]
                            .iter()
                            .any(|c| w.0 == CRU_BASE + c);
                        assert!(ok, "{what}: CRU write {:#x} := {:#x}", w.0, w.1);
                    }
                    let clksel_writes = writes.iter().filter(|w| w.0 == SEL158 || w.0 == CRU_BASE + clksel_con(159));
                    assert_eq!(clksel_writes.count(), usize::from(!kept), "{what}");
                }
            }
        }
    }

    // The take-over of a GPU found running (issue: after a crash kexec
    // Linux's devices stay on). Each test names the rule it holds.

    fn in_gpu(a: usize) -> bool {
        (GPU..GPU + 0x10000).contains(&a)
    }

    /// The index of the first write `f` matches, if any.
    fn first_write(t6: &T6, f: impl Fn(&(usize, u32)) -> bool) -> Option<usize> {
        t6.writes.borrow().iter().position(f)
    }

    #[test]
    fn every_reset_of_a_gpu_left_running_meets_the_masks_at_0_and_the_mcu_disabled() {
        let t6 = T6::left_by_panthor();
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        let resets = t6.resets.borrow();
        assert!(!resets.is_empty());
        for (cmd, masks, mcu) in resets.iter() {
            assert_eq!((masks, *mcu), (&[0; 3], csf::MCU_STATUS_DISABLED), "reset {cmd:#x}: {resets:x?}");
        }
        let t = o.take_over.expect("a GPU found powered is taken over");
        assert_eq!(t.masks, [0x2_0103, u32::MAX, mmu::MMU_INT_FAULTS]);
        assert_eq!(t.raw, [0, csf::JOB_INT_GLOBAL_IF, 1]);
        assert_eq!((t.mcu_status, t.mcu_waited_us, t.hard), (csf::MCU_STATUS_ENABLED, 3, false));
        // and Z1 went on to the G610 and its L2 as on any other route
        assert_eq!((o.ident.gpu_id, o.l2.ready), (mali::G610_GPU_ID, 1));
        assert_eq!(t6.masks(), [0; 3]);
    }

    #[test]
    fn the_first_gpu_words_mask_then_clear_then_halt_then_reset() {
        let t6 = T6::left_by_panthor();
        gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        let w = t6.writes.borrow();
        let gpu: Vec<(usize, u32)> = w.iter().filter(|w| in_gpu(w.0)).map(|w| (w.0 - GPU, w.1)).collect();
        let blocks = takeover::IRQ_BLOCKS;
        let n = blocks.len();
        // the masks before anything else, then the clears, then the halt
        assert!(gpu[..n].iter().all(|w| w.1 == 0 && blocks.iter().any(|b| b.2 == w.0)), "{gpu:x?}");
        assert!(gpu[n..2 * n].iter().all(|w| w.1 == u32::MAX && blocks.iter().any(|b| b.1 == w.0)), "{gpu:x?}");
        assert_eq!(gpu[2 * n], (csf::MCU_CONTROL, csf::MCU_CONTROL_DISABLE), "{gpu:x?}");
        let first_reset = gpu.iter().position(|w| w.0 == mali::GPU_CMD).unwrap();
        assert!(2 * n < first_reset);
    }

    #[test]
    fn the_take_over_reset_comes_before_the_qos_words_and_the_l2() {
        let t6 = T6::left_by_panthor();
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!(o.stop, Stop::None);
        let reset = first_write(&t6, |w| *w == (GPU + mali::GPU_CMD, mali::CMD_SOFT_RESET)).unwrap();
        let qos = first_write(&t6, |w| (QOS..QOS + 0x800).contains(&w.0)).unwrap();
        let l2 = first_write(&t6, |w| w.0 == GPU + mali::L2_PWRON).unwrap();
        assert!(reset < qos && qos < l2, "{:x?}", t6.writes.borrow());
        // the take-over's reset and Z1's own: two soft resets
        let cmds: Vec<u32> = t6.resets.borrow().iter().map(|r| r.0).collect();
        assert_eq!(cmds, vec![mali::CMD_SOFT_RESET; 2]);
    }

    /// The GPU words of `soft_reset` then `l2_power_on`, on a fresh model:
    /// what Z1 wrote to a GPU it found off before the take-over existed.
    fn reset_then_l2_words() -> Vec<(usize, u32)> {
        let t6 = T6::handed_over_domain_on();
        mali::soft_reset(&t6, GPU, &mut |_| {}).unwrap();
        mali::l2_power_on(&t6, GPU, &mut |_| {}).unwrap();
        let w = t6.writes.borrow();
        w.iter().filter(|w| in_gpu(w.0)).copied().collect()
    }

    #[test]
    fn a_gpu_found_off_is_written_the_same_gpu_words_as_without_the_take_over() {
        let want = reset_then_l2_words();
        for t6 in [T6::handed_over(), T6::handed_over_supply_off(0x1c), T6::handed_over_on_npll_900()] {
            let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
            assert_eq!(o.stop, Stop::None);
            assert_eq!(o.take_over, None);
            let w = t6.writes.borrow();
            let gpu: Vec<(usize, u32)> = w.iter().filter(|w| in_gpu(w.0)).copied().collect();
            assert_eq!(gpu, want);
        }
    }

    #[test]
    fn a_domain_powered_with_its_idle_request_set_is_released_before_the_take_over() {
        // the domain powered but its bus idle (a power-off cut short): the
        // GPU's registers abort until the request is released
        let t6 = T6::left_by_panthor();
        for (a, v) in
            [(pd_gpu::BUS_IDLE_REQ, 0x83cf), (pd_gpu::BUS_IDLE_ACK, 0x000a_9fff), (pd_gpu::BUS_IDLE_ST, 0x000a_9fff)]
        {
            t6.regs.borrow_mut().insert(PMU_BASE + a, v);
        }
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert!(o.take_over.is_some());
        let release = first_write(&t6, |w| *w == (PMU_BASE + pd_gpu::BUS_IDLE_REQ, 1 << 16)).unwrap();
        let first_gpu = first_write(&t6, |w| in_gpu(w.0)).unwrap();
        assert!(release < first_gpu);
        assert!(t6.resets.borrow().iter().all(|r| r.1 == [0; 3] && r.2 == csf::MCU_STATUS_DISABLED));
    }

    #[test]
    fn pd_gpu_is_never_cut_on_any_route() {
        let off = |t6: &T6| {
            t6.writes.borrow().iter().any(|w| w.0 == PMU_BASE + pd_gpu::PWR_GATE && w.1 & pd_gpu::PWR_BIT != 0)
        };
        for t6 in
            [T6::handed_over(), T6::handed_over_supply_off(0x1c), T6::handed_over_domain_on(), T6::left_by_panthor()]
        {
            let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
            assert_eq!(o.stop, Stop::None);
            assert!(!off(&t6), "{:x?}", t6.writes.borrow());
        }
    }

    /// No QoS word, no Z1 reset past the take-over's, no L2 power-on.
    fn nothing_past_the_take_over(t6: &T6, take_over_resets: usize) {
        let w = t6.writes.borrow();
        assert!(w.iter().all(|w| !(QOS..QOS + 0x800).contains(&w.0) && w.0 != GPU + mali::L2_PWRON), "{w:x?}");
        assert_eq!(t6.resets.borrow().len(), take_over_resets);
    }

    #[test]
    fn an_mcu_that_does_not_halt_is_reset_and_z1_goes_on() {
        let mut t6 = T6::left_by_panthor();
        t6.mcu_halt_lag = None;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        let t = o.take_over.unwrap();
        assert!(t.mcu_halt_timed_out);
        assert_eq!((t.mcu_status_halted, t.steps), (csf::MCU_STATUS_ENABLED, takeover::STEP_RESET));
        // the take-over's reset met the masks at 0 (the MCU still enabled: the
        // reset is what stops it), then Z1's own met it disabled
        let resets = t6.resets.borrow();
        assert_eq!(resets[0], (mali::CMD_SOFT_RESET, [0; 3], csf::MCU_STATUS_ENABLED));
        assert_eq!(resets[1], (mali::CMD_SOFT_RESET, [0; 3], csf::MCU_STATUS_DISABLED));
    }

    #[test]
    fn z1_touches_no_gpu_register_past_gpu_map_min() {
        let mut states = vec![T6::handed_over(), T6::handed_over_domain_on(), T6::left_by_panthor()];
        let mut stuck = T6::left_by_panthor();
        stuck.mcu_halt_lag = None;
        *stuck.soft_resets_dead.get_mut() = 1;
        states.push(stuck);
        let mut hi = 0;
        for t6 in states {
            gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
            hi = hi.max(t6.gpu_end.get());
        }
        // MMU_INT_MASK is the highest register Z1 touches
        assert_eq!(hi, mmu::MMU_INT_MASK + 4);
        assert!(hi <= GPU_MAP_MIN);
    }

    #[test]
    fn a_take_over_soft_reset_that_times_out_is_followed_by_a_hard_reset() {
        let t6 = T6::left_by_panthor();
        *t6.soft_resets_dead.borrow_mut() = 1;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::None, 0, 0));
        assert!(o.take_over.unwrap().hard);
        let cmds: Vec<u32> = t6.resets.borrow().iter().map(|r| r.0).collect();
        assert_eq!(cmds, vec![mali::CMD_SOFT_RESET, mali::CMD_HARD_RESET, mali::CMD_SOFT_RESET]);
    }

    #[test]
    fn a_take_over_whose_hard_reset_times_out_stops_before_the_power_on() {
        let mut t6 = T6::left_by_panthor();
        *t6.soft_resets_dead.get_mut() = 1;
        t6.hard_reset_dead = true;
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::TakeOver, MaliRefusal::HardResetTimeout as u32, 0));
        nothing_past_the_take_over(&t6, 2);
    }

    #[test]
    fn a_mask_that_stays_set_stops_z1_before_the_halt() {
        let mut t6 = T6::left_by_panthor();
        t6.stuck.push(GPU + mmu::MMU_INT_MASK);
        let o = gpu_up(&t6, B, &mut |_| {}, &mut |_| {});
        let detail = (mmu::MMU_INT_MASK as u64) << 32 | u64::from(mmu::MMU_INT_FAULTS);
        assert_eq!((o.stop, o.refusal, o.detail), (Stop::TakeOver, MaliRefusal::IrqMaskStuck as u32, detail));
        assert!(t6.writes.borrow().iter().all(|w| w.0 != GPU + csf::MCU_CONTROL));
        nothing_past_the_take_over(&t6, 0);
    }
}
