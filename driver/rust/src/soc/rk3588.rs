// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Rockchip RK3588: the CRU clock gates a driver here needs open, the
//! GPU's clock as the CRU sets it, and the GPU power domain (PD_GPU) in the
//! PMU.
//!
//! Facts from Linux v6.18 (GPL-2.0: facts and file:line only, nothing
//! transcribed):
//!
//! - The CRU is at 0xfd7c0000 (rk3588-base.dtsi:742-744).
//! - `RK3588_CLKGATE_CON(x)` is CRU + 0x800 + 4·x, `RK3588_CLKSEL_CON(x)`
//!   CRU + 0x300 + 4·x, `RK3588_PLL_CON(x)` CRU + 4·x (drivers/clk/
//!   rockchip/clk.h:359-365).
//! - Every RK3588 gate is `GFLAGS` = `CLK_GATE_HIWORD_MASK |
//!   CLK_GATE_SET_TO_DISABLE` (clk-rk3588.c:522): a bit set is the clock
//!   gated, and a write changes only the bits whose twin in 31:16 is set.
//! - `PCLK_WDT0` is `CLKGATE_CON(15)` bit 0 and `TCLK_WDT0` bit 1, from
//!   xin24m (clk-rk3588.c:1050-1053).
//!
//! Measured on the NanoPC-T6 under Armbian (2026-10-18, read-only):
//! `CLKGATE_CON(15)` 0x00002ff8 (both WDT0 gates open), tclk_wdt0 24 MHz.
//! The GPU facts are with their items below (2026-10-19).

use crate::mmio::Mmio;

/// The CRU's physical base.
pub const CRU_BASE: usize = 0xfd7c_0000;

/// `RK3588_CLKGATE_CON(x)`.
pub const fn clkgate_con(x: usize) -> usize {
    0x800 + 4 * x
}

/// A gate: the `CLKGATE_CON` index and its bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
    pub con: usize,
    pub bit: u32,
}

pub const PCLK_WDT0: Gate = Gate { con: 15, bit: 0 };
pub const TCLK_WDT0: Gate = Gate { con: 15, bit: 1 };

/// The counter clock of WDT0: xin24m.
pub const TCLK_WDT0_HZ: u32 = 24_000_000;

/// The `CLKGATE_CON` index past the last one: `CLKGATE_CON(77)` is the
/// highest Linux v6.18 names (clk-rk3588.c), so this only catches a
/// mistyped index.
pub const CLKGATE_CON_COUNT: usize = 78;

/// Open `gate` in the CRU mapped at `cru`: `Ok(true)` when it was gated and
/// is now open, `Ok(false)` when it was already open (nothing written),
/// `Err(read_back)` when it still reads gated after the write.
///
/// # Panics
/// On an index past [`CLKGATE_CON_COUNT`] or a bit above 15: those are
/// programming errors, not states of the hardware.
pub fn gate_open(m: &impl Mmio, cru: usize, gate: Gate) -> Result<bool, u32> {
    assert!(gate.con < CLKGATE_CON_COUNT && gate.bit < 16, "{gate:?}");
    let addr = cru + clkgate_con(gate.con);
    let bit = 1u32 << gate.bit;
    if m.read32(addr) & bit == 0 {
        return Ok(false);
    }
    // the write-enable twin of the bit, the bit itself 0 (open)
    m.write32(addr, bit << 16);
    let now = m.read32(addr);
    if now & bit != 0 {
        return Err(now);
    }
    Ok(true)
}

/// `RK3588_CLKSEL_CON(x)`.
pub const fn clksel_con(x: usize) -> usize {
    0x300 + 4 * x
}

/// `RK3588_PLL_CON(x)`.
pub const fn pll_con(x: usize) -> usize {
    4 * x
}

/// `RK3588_MODE_CON0`: the PLLs' 2-bit modes (clk.h:360).
pub const MODE_CON0: usize = 0x280;

/// A PLL of the `pll_rk3588` type: its first `PLL_CON` and its field in
/// `MODE_CON0` (clk-rk3588.c:661-672).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pll {
    pub con: usize,
    pub mode_shift: u32,
}

pub const GPLL: Pll = Pll { con: 112, mode_shift: 2 };
pub const CPLL: Pll = Pll { con: 104, mode_shift: 8 };
pub const AUPLL: Pll = Pll { con: 96, mode_shift: 6 };
pub const NPLL: Pll = Pll { con: 120, mode_shift: 0 };

/// The oscillator every PLL runs from.
pub const XIN24M_HZ: u64 = 24_000_000;

/// The rate of a `pll_rk3588` from its three `PLL_CON` words:
/// 24 MHz · (m + k / 65535) / p >> s, with m = CON0[9:0], p = CON1[5:0],
/// s = CON1[8:6], k = CON2[15:0] (clk-pll.c:856-864, 887-927). `None`
/// for p = 0 (no rate).
pub const fn pll_hz(con0: u32, con1: u32, con2: u32) -> Option<u64> {
    let m = (con0 & 0x3ff) as u64;
    let p = (con1 & 0x3f) as u64;
    let s = (con1 >> 6) & 0x7;
    let k = (con2 & 0xffff) as u64;
    if p == 0 {
        return None;
    }
    let mut r = XIN24M_HZ * m / p;
    if k != 0 {
        r += XIN24M_HZ * k / (p * 65535);
    }
    Some(r >> s)
}

/// What the CRU says of the GPU's clock: `CLK_GPU_SRC` is a 3-bit mux
/// (CLKSEL_CON(158)[7:5]: gpll, cpll, aupll, npll, spll) and a 5-bit
/// divider ([4:0]); `CLK_GPU`, `CLK_GPU_COREGROUP` and `CLK_GPU_STACKS` are
/// gates 4, 6 and 7 of CLKGATE_CON(66) after it, `CLK_GPU_SRC`'s own gate
/// is bit 1, and `CLK_GPU_STACKS` has its own divider in CLKSEL_CON(159)[4:0]
/// (clk-rk3588.c:1353-1363). Bit 14 of CLKSEL_CON(158) switches the GPU to
/// the GPU PVTPLL (a ring oscillator in the GPU GRF), a field Linux does not
/// name: TF-A's rk3588 SCMI clock writes it (`GPU_CLK_PATH_PVTPLL`,
/// plat/rockchip/rk3588/drivers/scmi/rk3588_clk.c, BSD-3-Clause; the T6's
/// BL31 is Rockchip's binary v1.48, so that source is a reference, not the
/// code that runs).
///
/// Measured on the T6 under Armbian, 2026-10-19 (read-only; the GPU forced
/// active through its runtime PM `control` for the second read):
/// CLKSEL_CON(158) 0x00001f24 (cpll, divider 4 + 1, bit 14 clear),
/// CLKSEL_CON(159) 0, CPLL 1500 MHz (CON0 0xfa, CON1 0x42), GPLL 1188 MHz
/// (0xc6, 0x42), MODE_CON0 0x545: 300 MHz, the rate `clk_summary` shows.
/// CLKGATE_CON(66) 0xd6 with the GPU runtime-suspended, 0x04 active.
pub mod gpu_clk {
    use super::Gate;

    pub const SEL_CON: usize = 158;
    pub const STACKS_SEL_CON: usize = 159;
    pub const DIV_MASK: u32 = 0x1f;
    pub const MUX_SHIFT: u32 = 5;
    pub const MUX_MASK: u32 = 0x7 << MUX_SHIFT;
    pub const PVTPLL: u32 = 1 << 14;

    pub const SRC: Gate = Gate { con: 66, bit: 1 };
    pub const CORE: Gate = Gate { con: 66, bit: 4 };
    pub const COREGROUP: Gate = Gate { con: 66, bit: 6 };
    pub const STACKS: Gate = Gate { con: 66, bit: 7 };
    /// In the order a clock follows its parent.
    pub const GATES: [Gate; 4] = [SRC, CORE, COREGROUP, STACKS];
}

/// The parent `CLK_GPU_SRC`'s mux selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum GpuSrc {
    Gpll = 0,
    Cpll = 1,
    Aupll = 2,
    Npll = 3,
    Spll = 4,
    /// CLKSEL_CON(158) bit 14: the PVTPLL, whose rate the CRU does not say.
    Pvtpll = 8,
    /// A mux value past the five parents.
    Unknown = 9,
}

/// The GPU source the CRU selects, and its divider (the field + 1).
pub const fn gpu_src(sel158: u32) -> (GpuSrc, u32) {
    let div = (sel158 & gpu_clk::DIV_MASK) + 1;
    if sel158 & gpu_clk::PVTPLL != 0 {
        return (GpuSrc::Pvtpll, div);
    }
    let src = match (sel158 & gpu_clk::MUX_MASK) >> gpu_clk::MUX_SHIFT {
        0 => GpuSrc::Gpll,
        1 => GpuSrc::Cpll,
        2 => GpuSrc::Aupll,
        3 => GpuSrc::Npll,
        4 => GpuSrc::Spll,
        _ => GpuSrc::Unknown,
    };
    (src, div)
}

/// The GPU PLL the CRU selects, if it is one this module can read.
pub const fn gpu_src_pll(src: GpuSrc) -> Option<Pll> {
    match src {
        GpuSrc::Gpll => Some(GPLL),
        GpuSrc::Cpll => Some(CPLL),
        GpuSrc::Aupll => Some(AUPLL),
        GpuSrc::Npll => Some(NPLL),
        _ => None,
    }
}

/// The GPU's clock as read from the CRU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuClock {
    pub sel158: u32,
    pub sel159: u32,
    pub gate66: u32,
    pub mode_con0: u32,
    /// The selected PLL's CON0..CON2 (0 when the source is not a PLL read here).
    pub pll_con: [u32; 3],
    /// `CLK_GPU_SRC`'s rate: the PLL's rate over the divider. 0 when it is
    /// not known (the PVTPLL, SPLL, an unknown mux value, a PLL not in
    /// normal mode).
    pub src_hz: u64,
}

/// `MODE_CON0`'s field for a PLL: 0 xin24m, 1 the PLL's output, 2 xin32k
/// (clk-pll.c:20-23 `PLL_MODE_*`, 1103-1106; the fields are
/// clk-rk3588.c:661-672's).
pub const fn pll_mode(mode_con0: u32, pll: Pll) -> u32 {
    (mode_con0 >> pll.mode_shift) & 0x3
}

/// Read the GPU's clock from the CRU at `cru`. Nothing is written.
pub fn gpu_clock(m: &impl Mmio, cru: usize) -> GpuClock {
    let sel158 = m.read32(cru + clksel_con(gpu_clk::SEL_CON));
    let mut c = GpuClock {
        sel158,
        sel159: m.read32(cru + clksel_con(gpu_clk::STACKS_SEL_CON)),
        gate66: m.read32(cru + clkgate_con(66)),
        mode_con0: m.read32(cru + MODE_CON0),
        ..GpuClock::default()
    };
    let (src, div) = gpu_src(sel158);
    if let Some(pll) = gpu_src_pll(src) {
        for (i, w) in c.pll_con.iter_mut().enumerate() {
            *w = m.read32(cru + pll_con(pll.con + i));
        }
        if pll_mode(c.mode_con0, pll) == 1 {
            if let Some(hz) = pll_hz(c.pll_con[0], c.pll_con[1], c.pll_con[2]) {
                c.src_hz = hz / div as u64;
            }
        }
    }
    c
}

/// The PMU (`power-management@fd8d8000`, rk3588-base.dtsi:842-844).
pub const PMU_BASE: usize = 0xfd8d_8000;

/// PD_GPU in the PMU: `rk3588_pmu` and its `RK3588_PD_GPU` entry
/// (pm-domains.c:1277, 1291-1300; `DOMAIN_M_O_R`, :154-171). Each control
/// register takes a write-enable twin of its bit in 31:16.
///
/// - `PWR_GATE` 0x14c bit 0: 1 = powered off. On: write `1 << 16`.
/// - `REPAIR_STATUS` 0x290 bit 1: 1 = powered on (the domain has no
///   `status_mask`; `rockchip_pmu_domain_is_on`, :470-489).
/// - `BUS_IDLE_REQ` 0x10c bit 0: 1 = idle requested. Leave idle: write
///   `1 << 16`, then wait for `BUS_IDLE_ACK` 0x118 bit 0 and `BUS_IDLE_ST`
///   0x120 bit 0 to read 0 (`rockchip_pmu_set_idle_request`, :376-422).
/// - No memory repair (`m_status` 0) and no clock-ungate register
///   (`rk3588_pmu` has no `clk_ungate_offset`).
/// - The order of `rockchip_pd_power` (:620-660): the domain's clocks on
///   (`CLK_GPU`, `CLK_GPU_COREGROUP`, `CLK_GPU_STACKS`, rk3588-base.dtsi:
///   892-902), power on, wait, leave idle, restore the QoS registers. Each
///   wait is 10 ms in Linux. After the power-on Linux also tells the
///   firmware (SIP `ROCKCHIP_SIP_SUSPEND_MODE` 0x82000003,
///   `ROCKCHIP_SLEEP_PD_CONFIG`), which only configures system suspend;
///   this does not.
///
/// Measured on the T6 under Armbian, 2026-10-19: GPU runtime-suspended
/// (genpd "off-0"): `PWR_GATE` 0x0000ffff, `REPAIR_STATUS` 0xfad50001,
/// `BUS_IDLE_REQ` 0x83cf, `BUS_IDLE_ACK` 0x000a9fff, `BUS_IDLE_ST`
/// 0x000a9fff. Active (genpd "on"): 0xfffe, 0xfad50003, 0x83ce, 0x000a9ffe,
/// 0x000a9ffe: exactly bit 0 (bit 1 of `REPAIR_STATUS`) moves.
pub mod pd_gpu {
    pub const PWR_GATE: usize = 0x14c;
    pub const STATUS: usize = 0x180;
    pub const BUS_IDLE_REQ: usize = 0x10c;
    pub const BUS_IDLE_ACK: usize = 0x118;
    pub const BUS_IDLE_ST: usize = 0x120;
    pub const REPAIR_STATUS: usize = 0x290;

    pub const PWR_BIT: u32 = 1 << 0;
    pub const REPAIR_ON_BIT: u32 = 1 << 1;
    pub const IDLE_BIT: u32 = 1 << 0;

    /// Linux's wait for each step (pm-domains.c:396, 412, 562).
    pub const WAIT_US: u32 = 10_000;
}

/// The GPU QoS blocks restored after a power-on (`qos_gpu_m0..m3`,
/// rk3588-base.dtsi:1676-1694) and the five words of each
/// (`QOS_PRIORITY` 0x08 .. `QOS_EXTCONTROL` 0x18, pm-domains.c:86-90).
/// Under Armbian with the GPU active (2026-10-19): `PRIORITY` 0x80000000,
/// the other four 0, in all four blocks.
pub const QOS_GPU: [usize; 4] = [0xfdf3_5000, 0xfdf3_5200, 0xfdf3_5400, 0xfdf3_5600];
pub const QOS_WORDS: [usize; 5] = [0x08, 0x0c, 0x10, 0x14, 0x18];

/// The PMU's PD_GPU registers at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdGpuRegs {
    pub pwr_gate: u32,
    pub status: u32,
    pub idle_req: u32,
    pub idle_ack: u32,
    pub idle_st: u32,
    pub repair: u32,
}

impl PdGpuRegs {
    /// Read them. Nothing is written; the PMU is always powered.
    pub fn read(m: &impl Mmio, pmu: usize) -> PdGpuRegs {
        PdGpuRegs {
            pwr_gate: m.read32(pmu + pd_gpu::PWR_GATE),
            status: m.read32(pmu + pd_gpu::STATUS),
            idle_req: m.read32(pmu + pd_gpu::BUS_IDLE_REQ),
            idle_ack: m.read32(pmu + pd_gpu::BUS_IDLE_ACK),
            idle_st: m.read32(pmu + pd_gpu::BUS_IDLE_ST),
            repair: m.read32(pmu + pd_gpu::REPAIR_STATUS),
        }
    }

    /// Powered (`REPAIR_STATUS` bit 1) and out of idle (no request, no
    /// acknowledge, not idle): the GPU's registers can be read.
    pub const fn usable(&self) -> bool {
        self.repair & pd_gpu::REPAIR_ON_BIT != 0
            && (self.idle_req | self.idle_ack | self.idle_st) & pd_gpu::IDLE_BIT == 0
    }
}

/// Why [`pd_gpu_on`] stopped. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PdRefusal {
    /// A GPU clock gate still reads gated after its write.
    GateStuck = 1,
    /// `REPAIR_STATUS` bit 1 did not read 1 within [`pd_gpu::WAIT_US`].
    PowerTimeout = 2,
    /// `BUS_IDLE_ACK` bit 0 did not clear.
    IdleAckTimeout = 3,
    /// `BUS_IDLE_ST` bit 0 did not clear.
    IdleTimeout = 4,
}

/// What [`pd_gpu_on`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdOn {
    /// The gates of [`gpu_clk::GATES`] it opened, as a bit mask of their
    /// CLKGATE_CON(66) bits.
    pub opened: u32,
    /// The domain read on before anything was written (power and idle
    /// steps skipped).
    pub was_on: bool,
    /// Microseconds each wait took (power, idle ack, idle).
    pub waited_us: [u32; 3],
}

/// Poll `cond` on the value at `addr` once per microsecond for at most
/// `budget_us`; the microseconds it took.
fn wait(
    m: &impl Mmio,
    addr: usize,
    budget_us: u32,
    delay_us: &mut impl FnMut(u32),
    cond: impl Fn(u32) -> bool,
) -> Option<u32> {
    for t in 0..=budget_us {
        if cond(m.read32(addr)) {
            return Some(t);
        }
        delay_us(1);
    }
    None
}

/// Power PD_GPU on in Linux's order (see [`pd_gpu`]): open the GPU clock
/// gates, clear the power gate, wait for the domain, leave bus idle, wait
/// for the acknowledge and the idle status. A domain already on and out of
/// idle is left as it is (only the gates are opened). The supply
/// (vdd_gpu_s0) is not touched: the caller checks it first. The QoS words
/// are not written: the caller reads them back.
///
/// `delay_us` waits that many microseconds.
pub fn pd_gpu_on(
    m: &impl Mmio,
    cru: usize,
    pmu: usize,
    delay_us: &mut impl FnMut(u32),
) -> Result<PdOn, (PdRefusal, u32)> {
    let mut out = PdOn::default();
    for g in gpu_clk::GATES {
        match gate_open(m, cru, g) {
            Ok(true) => out.opened |= 1 << g.bit,
            Ok(false) => {}
            Err(now) => return Err((PdRefusal::GateStuck, now)),
        }
    }
    if PdGpuRegs::read(m, pmu).usable() {
        out.was_on = true;
        return Ok(out);
    }
    m.write32(pmu + pd_gpu::PWR_GATE, pd_gpu::PWR_BIT << 16);
    let addr = pmu + pd_gpu::REPAIR_STATUS;
    out.waited_us[0] = wait(m, addr, pd_gpu::WAIT_US, delay_us, |v| v & pd_gpu::REPAIR_ON_BIT != 0)
        .ok_or((PdRefusal::PowerTimeout, m.read32(addr)))?;
    m.write32(pmu + pd_gpu::BUS_IDLE_REQ, pd_gpu::IDLE_BIT << 16);
    let addr = pmu + pd_gpu::BUS_IDLE_ACK;
    out.waited_us[1] = wait(m, addr, pd_gpu::WAIT_US, delay_us, |v| v & pd_gpu::IDLE_BIT == 0)
        .ok_or((PdRefusal::IdleAckTimeout, m.read32(addr)))?;
    let addr = pmu + pd_gpu::BUS_IDLE_ST;
    out.waited_us[2] = wait(m, addr, pd_gpu::WAIT_US, delay_us, |v| v & pd_gpu::IDLE_BIT == 0)
        .ok_or((PdRefusal::IdleTimeout, m.read32(addr)))?;
    Ok(out)
}

/// SPI2, the RK806's bus on the NanoPC-T6: `PCLK_SPI2` is CLKGATE_CON(14)
/// bit 8, `CLK_SPI2` bit 13 (clk-rk3588.c:1170-1184); its base 0xfeb20000
/// (rk3588-base.dtsi:2501-2503).
pub const SPI2_BASE: usize = 0xfeb2_0000;
pub const PCLK_SPI2: Gate = Gate { con: 14, bit: 8 };
pub const CLK_SPI2: Gate = Gate { con: 14, bit: 13 };

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// One CLKGATE_CON register with the hiword write mask.
    struct Cru {
        con15: RefCell<u32>,
        writes: RefCell<Vec<(usize, u32)>>,
        stuck: bool,
    }
    impl Mmio for Cru {
        fn read32(&self, addr: usize) -> u32 {
            assert_eq!(addr, CRU_BASE + 0x83c);
            *self.con15.borrow()
        }
        fn write32(&self, addr: usize, val: u32) {
            self.writes.borrow_mut().push((addr - CRU_BASE, val));
            if !self.stuck {
                let mask = val >> 16;
                let mut r = self.con15.borrow_mut();
                *r = (*r & !mask) | (val & mask);
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
    fn clkgate_con_15_is_at_0x83c() {
        assert_eq!(clkgate_con(15), 0x83c);
    }

    #[test]
    fn open_gates_as_armbian_left_them_are_not_written() {
        let c = Cru { con15: RefCell::new(0x2ff8), writes: RefCell::new(vec![]), stuck: false };
        assert_eq!(gate_open(&c, CRU_BASE, PCLK_WDT0), Ok(false));
        assert_eq!(gate_open(&c, CRU_BASE, TCLK_WDT0), Ok(false));
        assert!(c.writes.borrow().is_empty());
    }

    #[test]
    fn a_closed_gate_is_opened_through_its_write_mask_only() {
        let c = Cru { con15: RefCell::new(0x2ffb), writes: RefCell::new(vec![]), stuck: false };
        assert_eq!(gate_open(&c, CRU_BASE, TCLK_WDT0), Ok(true));
        assert_eq!(*c.writes.borrow(), vec![(0x83c, 0x0002_0000)]);
        assert_eq!(*c.con15.borrow(), 0x2ff9); // bit 0 untouched
        let c = Cru { con15: RefCell::new(0x2ffb), writes: RefCell::new(vec![]), stuck: true };
        assert_eq!(gate_open(&c, CRU_BASE, PCLK_WDT0), Err(0x2ffb));
    }

    /// The CRU and the PMU as the T6 read under Armbian, 2026-10-19, with
    /// the GPU runtime-suspended; a write with a hiword mask changes only
    /// the bits it enables. PD_GPU follows its control registers after
    /// `lag` reads of its status.
    struct Soc {
        regs: RefCell<std::collections::HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        lag: u32,
        reads_since: RefCell<u32>,
        /// The domain never reports power.
        dead: bool,
    }
    const PMU: usize = PMU_BASE;
    impl Soc {
        fn t6_suspended(lag: u32) -> Soc {
            let mut r = std::collections::HashMap::new();
            for (a, v) in [
                (CRU_BASE + clksel_con(158), 0x1f24),
                (CRU_BASE + clksel_con(159), 0),
                (CRU_BASE + clkgate_con(66), 0xd6),
                (CRU_BASE + MODE_CON0, 0x545),
                (CRU_BASE + pll_con(104), 0xfa),
                (CRU_BASE + pll_con(105), 0x42),
                (CRU_BASE + pll_con(106), 0),
                (PMU + pd_gpu::PWR_GATE, 0xffff),
                (PMU + pd_gpu::STATUS, 0x2455_ffff),
                (PMU + pd_gpu::BUS_IDLE_REQ, 0x83cf),
                (PMU + pd_gpu::BUS_IDLE_ACK, 0x000a_9fff),
                (PMU + pd_gpu::BUS_IDLE_ST, 0x000a_9fff),
                (PMU + pd_gpu::REPAIR_STATUS, 0xfad5_0001),
            ] {
                r.insert(a, v);
            }
            Soc { regs: RefCell::new(r), writes: RefCell::new(vec![]), lag, reads_since: RefCell::new(0), dead: false }
        }
        fn get(&self, a: usize) -> u32 {
            self.regs.borrow()[&a]
        }
    }
    impl Mmio for Soc {
        fn read32(&self, addr: usize) -> u32 {
            let mut n = self.reads_since.borrow_mut();
            *n += 1;
            let mut r = self.regs.borrow_mut();
            let gate = r[&(PMU + pd_gpu::PWR_GATE)];
            let req = r[&(PMU + pd_gpu::BUS_IDLE_REQ)];
            if *n > self.lag {
                if gate & 1 == 0 && !self.dead {
                    *r.get_mut(&(PMU + pd_gpu::REPAIR_STATUS)).unwrap() |= 2;
                }
                if req & 1 == 0 {
                    *r.get_mut(&(PMU + pd_gpu::BUS_IDLE_ACK)).unwrap() &= !1;
                    *r.get_mut(&(PMU + pd_gpu::BUS_IDLE_ST)).unwrap() &= !1;
                }
            }
            *r.get(&addr).unwrap_or_else(|| panic!("read of an unmodelled register {addr:#x}"))
        }
        fn write32(&self, addr: usize, val: u32) {
            self.writes.borrow_mut().push((addr, val));
            *self.reads_since.borrow_mut() = 0;
            let mut r = self.regs.borrow_mut();
            let e = r.get_mut(&addr).unwrap_or_else(|| panic!("write to an unmodelled register {addr:#x}"));
            let mask = val >> 16;
            *e = (*e & !mask) | (val & mask);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[test]
    fn the_pll_rates_the_t6_reads_are_1500_and_1188_mhz() {
        assert_eq!(pll_hz(0xfa, 0x42, 0), Some(1_500_000_000));
        assert_eq!(pll_hz(0xc6, 0x42, 0), Some(1_188_000_000));
        assert_eq!(pll_hz(0xfa, 0x40, 0), None);
        // fractional: k / 65535 of the step 24 MHz / p
        assert_eq!(pll_hz(100, 0x02, 65535), Some(1_212_000_000));
    }

    #[test]
    fn the_gpu_clock_armbian_leaves_is_cpll_over_5() {
        let soc = Soc::t6_suspended(0);
        let c = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_src(c.sel158), (GpuSrc::Cpll, 5));
        assert_eq!(c.pll_con, [0xfa, 0x42, 0]);
        assert_eq!(c.src_hz, 300_000_000);
        assert!(soc.writes.borrow().is_empty());
    }

    #[test]
    fn a_gpu_clock_the_cru_cannot_rate_is_0() {
        assert_eq!(gpu_src(0x1f24 | gpu_clk::PVTPLL).0, GpuSrc::Pvtpll);
        assert_eq!(gpu_src(0x4 << 5).0, GpuSrc::Spll);
        assert_eq!(gpu_src(0x5 << 5).0, GpuSrc::Unknown);
        assert_eq!(gpu_src_pll(GpuSrc::Pvtpll), None);
        // CPLL in slow mode (MODE_CON0 bits 9:8 = 0): no rate claimed
        let soc = Soc::t6_suspended(0);
        soc.regs.borrow_mut().insert(CRU_BASE + MODE_CON0, 0x445);
        assert_eq!(gpu_clock(&soc, CRU_BASE).src_hz, 0);
    }

    #[test]
    fn pd_gpu_on_follows_linux_order_from_the_suspended_state() {
        let soc = Soc::t6_suspended(3);
        assert!(!PdGpuRegs::read(&soc, PMU).usable());
        let mut slept = 0;
        let on = pd_gpu_on(&soc, CRU_BASE, PMU, &mut |us| slept += us).unwrap();
        assert_eq!(on.opened, (1 << 1) | (1 << 4) | (1 << 6) | (1 << 7));
        assert!(!on.was_on);
        assert_eq!(on.waited_us, [3, 3, 0]);
        assert_eq!(slept, 6);
        let w = soc.writes.borrow().clone();
        let g = CRU_BASE + clkgate_con(66);
        assert_eq!(
            w,
            vec![
                (g, 0x0002_0000),
                (g, 0x0010_0000),
                (g, 0x0040_0000),
                (g, 0x0080_0000),
                (PMU + pd_gpu::PWR_GATE, 0x0001_0000),
                (PMU + pd_gpu::BUS_IDLE_REQ, 0x0001_0000),
            ]
        );
        // the active state Armbian reads, bit for bit
        let now = PdGpuRegs::read(&soc, PMU);
        assert_eq!(
            (now.pwr_gate, now.idle_req, now.idle_ack, now.idle_st, now.repair),
            (0xfffe, 0x83ce, 0x000a_9ffe, 0x000a_9ffe, 0xfad5_0003)
        );
        assert!(now.usable());
        // 0x04: what Armbian reads active; bit 2 is no gate of clk-rk3588.c's GPU
        assert_eq!(soc.get(g), 0x04);
    }

    #[test]
    fn a_domain_that_is_already_on_is_not_written() {
        let soc = Soc::t6_suspended(0);
        for (a, v) in [
            (pd_gpu::PWR_GATE, 0xfffe),
            (pd_gpu::BUS_IDLE_REQ, 0x83ce),
            (pd_gpu::BUS_IDLE_ACK, 0xa9ffe),
            (pd_gpu::BUS_IDLE_ST, 0xa9ffe),
            (pd_gpu::REPAIR_STATUS, 0xfad5_0003),
        ] {
            soc.regs.borrow_mut().insert(PMU + a, v);
        }
        soc.regs.borrow_mut().insert(CRU_BASE + clkgate_con(66), 0x04);
        let on = pd_gpu_on(&soc, CRU_BASE, PMU, &mut |_| {}).unwrap();
        assert_eq!(on, PdOn { opened: 0, was_on: true, waited_us: [0; 3] });
        assert!(soc.writes.borrow().is_empty());
    }

    #[test]
    fn a_domain_that_never_powers_times_out_after_linux_10_ms() {
        let mut soc = Soc::t6_suspended(0);
        soc.dead = true;
        let mut slept = 0;
        assert_eq!(pd_gpu_on(&soc, CRU_BASE, PMU, &mut |us| slept += us), Err((PdRefusal::PowerTimeout, 0xfad5_0001)));
        assert_eq!(slept, pd_gpu::WAIT_US + 1);
        // it stopped there: the idle request was not released
        assert_eq!(soc.writes.borrow().last(), Some(&(PMU + pd_gpu::PWR_GATE, 0x0001_0000)));
    }
}
