// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Rockchip RK3588: the CRU clock gates a driver here needs open, the
//! GPU's clock as the CRU sets it (read, and put on a rate of the GPU OPP
//! table when it is not on one), and the PMU power domains: one table
//! ([`DOMAINS`]: PD_GPU and the display's PD_VOP, PD_VO0, PD_VO1) and one
//! on / off sequence for all of them ([`pd_on`], [`pd_off`]). The display's
//! clocks and resets are in `rk3588_display`.
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
/// name: TF-A's rk3588 SCMI clock driver sets it to put the GPU on the
/// PVTPLL (TF-A v2.13.0, commit c17351450c8a, BSD-3-Clause:
/// plat/rockchip/rk3588/drivers/scmi/rk3588_clk.c:146 defines
/// `GPU_CLK_PATH_PVTPLL` as a write-masked set of bit 14, :145 the clear
/// that returns to the normal PLLs, :975-976 write it to CLKSEL_CON(158) in
/// `clk_gpu_set_rate` after the GPU GRF PVTPLL setup at :957-973). The T6's
/// BL31 is Rockchip's binary v1.48, so that source is a reference, not the
/// code that runs.
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
    /// The fields [`super::gpu_clock_set`] owns in CLKSEL_CON(158): the
    /// mux, the divider and the PVTPLL bit. Its write enables exactly these.
    pub const SET_MASK: u32 = PVTPLL | MUX_MASK | DIV_MASK;

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

/// `PLL_CON1` bit 13 of a `pll_rk3588`: 1 = the PLL is powered down, and
/// `PLL_CON6` bit 15: 1 = it is locked (clk-pll.c v6.18:865-866 `RK3588_PLLCON1_PWRDOWN`,
/// `RK3588_PLLCON6_LOCK_STATUS`). Linux powers a PLL down when its last
/// consumer is disabled and leaves its `MODE_CON0` field alone
/// (`rockchip_rk3588_pll_disable`, clk-pll.c v6.18:1019-1025), so a PLL in normal mode can have no
/// output.
///
/// Measured on the T6 under Armbian, 2026-11-11 (read-only), with the GPU
/// runtime-suspended after it had run at 850 MHz: CLKSEL_CON(158)
/// 0x00001f60 (npll, divider 1), NPLL `PLL_CON0` 0x1a9, `PLL_CON1` 0x2083
/// (powered down), `PLL_CON6` 0x4000 (not locked), MODE_CON0 0x545 (NPLL
/// normal), `clk_summary`: pll_npll enable count 0. CPLL, which other
/// clocks keep on: `PLL_CON1` 0x42, `PLL_CON6` 0x9c00.
pub const PLL_CON1_PWRDOWN: u32 = 1 << 13;
pub const PLL_CON6_LOCK: u32 = 1 << 15;

/// The GPU's clock as read from the CRU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuClock {
    pub sel158: u32,
    pub sel159: u32,
    pub gate66: u32,
    pub mode_con0: u32,
    /// The selected PLL's CON0..CON2 (0 when the source is not a PLL read here).
    pub pll_con: [u32; 3],
    /// The selected PLL's CON6 ([`PLL_CON6_LOCK`]), 0 like `pll_con`.
    pub pll_con6: u32,
    /// `CLK_GPU_SRC`'s rate: the PLL's rate over the divider. 0 when it is
    /// not known (the PVTPLL, SPLL, an unknown mux value, a PLL not in
    /// normal mode). It is the rate the PLL is set to: a PLL that is
    /// powered down ([`PLL_CON1_PWRDOWN`]) gives it only after [`pll_on`].
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
        c.pll_con6 = m.read32(cru + pll_con(pll.con + 6));
        if pll_mode(c.mode_con0, pll) == 1 {
            if let Some(hz) = pll_hz(c.pll_con[0], c.pll_con[1], c.pll_con[2]) {
                c.src_hz = hz / div as u64;
            }
        }
    }
    c
}

/// The GPU OPP table (rk3588-opp.dtsi:118-153): each OPP's rate and the
/// first value of its `opp-microvolt`, the least vdd_gpu_s0 it runs at.
pub const GPU_OPP: [(u64, u32); 8] = [
    (300_000_000, 675_000),
    (400_000_000, 675_000),
    (500_000_000, 675_000),
    (600_000_000, 675_000),
    (700_000_000, 700_000),
    (800_000_000, 750_000),
    (900_000_000, 800_000),
    (1_000_000_000, 850_000),
];

/// `hz` is exactly an OPP rate of [`GPU_OPP`].
pub const fn gpu_opp_rate(hz: u64) -> bool {
    let mut i = 0;
    while i < GPU_OPP.len() {
        if GPU_OPP[i].0 == hz {
            return true;
        }
        i += 1;
    }
    false
}

/// The least vdd_gpu_s0 in µV [`GPU_OPP`] gives a clock: the voltage of the
/// first OPP at or above it. `None` above the table's 1 GHz or for an
/// unknown rate (0).
pub const fn gpu_opp_min_uv(hz: u64) -> Option<u32> {
    if hz == 0 {
        return None;
    }
    let mut i = 0;
    while i < GPU_OPP.len() {
        if hz <= GPU_OPP[i].0 {
            return Some(GPU_OPP[i].1);
        }
        i += 1;
    }
    None
}

/// The one setting [`gpu_clock_set`] moves the GPU clock to: `CLK_GPU_SRC`
/// from the CPLL over 5, 300 MHz, the lowest OPP (675 mV, so any supply the
/// table covers at all covers it) and the state Linux leaves after devfreq's
/// lowest rate (the T6 reads 0x1f24). It is used only while the CPLL's
/// `PLL_CON` words read [`SET_CPLL_HZ`] in normal mode: GPLL's 1188 MHz
/// divides to no rate of the table, AUPLL and NPLL are not read for it (NPLL
/// is the GPU's own PLL and Linux powers it down with the GPU, measured
/// 2026-11-11), and the CPLL is the PLL Linux keeps on for other consumers.
pub const SET_SRC: GpuSrc = GpuSrc::Cpll;
pub const SET_DIV: u32 = 5;
pub const SET_HZ: u64 = 300_000_000;
pub const SET_CPLL_HZ: u64 = 1_500_000_000;

/// The word for CLKSEL_CON(158) that selects [`SET_SRC`] / [`SET_DIV`] with
/// the PVTPLL bit clear, write-enabling exactly [`gpu_clk::SET_MASK`]'s
/// fields: 0x40ff_0024.
pub const SET_SEL158: u32 = (gpu_clk::SET_MASK << 16) | ((SET_SRC as u32) << gpu_clk::MUX_SHIFT) | (SET_DIV - 1);

/// The word for CLKSEL_CON(159) that sets the stacks divider to 1
/// (field 0), write-enabling the divider field only: 0x001f_0000.
pub const SET_SEL159: u32 = gpu_clk::DIV_MASK << 16;

/// Why [`gpu_clock_set`] refused. Nothing was written for `NoTableRate`;
/// the others come after the write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockRefusal {
    /// The CRU's GPU source gives no OPP rate and the CPLL does not read
    /// [`SET_CPLL_HZ`] in normal mode (the rate it reads: 0 for none).
    NoTableRate(u64),
    /// A CLKSEL_CON read back without the fields written (its index, the
    /// word read).
    ReadBack(usize, u32),
    /// The CRU, read again after the writes, rates the source at this and
    /// not [`SET_HZ`].
    Rate(u64),
}

/// What [`gpu_clock_set`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockSet {
    /// The CRU already gave an OPP rate with the stacks divider at 1:
    /// nothing written.
    pub kept: bool,
    /// The CPLL's rate as read for the decision (0 when not in normal mode
    /// or without a rate).
    pub cpll_hz: u64,
    /// The words written to CLKSEL_CON(158) and (159), hiword mask included;
    /// 0 for one not written.
    pub wrote: [u32; 2],
    /// CLKSEL_CON(158) and (159) after it (read back when written, as found
    /// when not).
    pub sel: [u32; 2],
    /// `CLK_GPU_SRC`'s rate after it: `found.src_hz` when kept, the CRU's
    /// rating of the read-back words when written.
    pub src_hz: u64,
}

/// Put the GPU clock on a rate of the OPP table, writing the CRU only when
/// it is not there. `found` is the clock as [`gpu_clock`] read it.
///
/// Kept, nothing written: `CLK_GPU_SRC` is a PLL this reads, in normal
/// mode, whose rate over the divider is exactly an OPP rate
/// ([`gpu_opp_rate`]), and the stacks divider (CLKSEL_CON(159)[4:0]) is 1.
/// Whether that PLL is powered is not this decision's: the caller powers it
/// ([`pll_on`]) as it does today.
///
/// Otherwise (the PVTPLL, SPLL, a mux value past the parents, a divider
/// off the table, a PLL out of normal mode) the clock is moved to
/// [`SET_SRC`] / [`SET_DIV`] = [`SET_HZ`], provided the CPLL reads
/// [`SET_CPLL_HZ`] in normal mode; else `NoTableRate`, nothing written.
///
/// The writes, in this order:
/// 1. CLKSEL_CON(158) := [`SET_SEL158`]. The mux, the divider and the
///    PVTPLL bit are fields of one register and move in one hiword-masked
///    write, so the GPU sees no intermediate setting (a PLL at divider 1,
///    for one). The bits outside [`gpu_clk::SET_MASK`] are not enabled and
///    keep their value (0x1f in 12:8 under Armbian).
/// 2. CLKSEL_CON(159) := [`SET_SEL159`], only when its divider field is
///    not 0. After 158, so the stacks clock (`clk_gpu_src` over this
///    divider) never runs faster than the source's final 300 MHz; the other
///    order would put it at the old source's rate for a moment.
///
/// Both words are read back and the CRU is rated again ([`gpu_clock`]); a
/// field that did not take or a rate other than [`SET_HZ`] is a refusal.
/// No `PLL_CON`, `MODE_CON0` or GPU GRF (PVTPLL) word is ever written: the
/// CPLL is used at the rate it has, and the PVTPLL is only deselected.
pub fn gpu_clock_set(m: &impl Mmio, cru: usize, found: &GpuClock) -> Result<ClockSet, ClockRefusal> {
    let mut out = ClockSet { sel: [found.sel158, found.sel159], src_hz: found.src_hz, ..ClockSet::default() };
    let cpll = [
        m.read32(cru + pll_con(CPLL.con)),
        m.read32(cru + pll_con(CPLL.con + 1)),
        m.read32(cru + pll_con(CPLL.con + 2)),
    ];
    if pll_mode(found.mode_con0, CPLL) == 1 {
        out.cpll_hz = pll_hz(cpll[0], cpll[1], cpll[2]).unwrap_or(0);
    }
    // src_hz is 0 unless the source is a PLL read here, in normal mode
    if gpu_opp_rate(found.src_hz) && found.sel159 & gpu_clk::DIV_MASK == 0 {
        out.kept = true;
        return Ok(out);
    }
    if out.cpll_hz != SET_CPLL_HZ {
        return Err(ClockRefusal::NoTableRate(out.cpll_hz));
    }
    let sel158 = cru + clksel_con(gpu_clk::SEL_CON);
    let sel159 = cru + clksel_con(gpu_clk::STACKS_SEL_CON);
    m.write32(sel158, SET_SEL158);
    out.wrote[0] = SET_SEL158;
    out.sel[0] = m.read32(sel158);
    if out.sel[0] & gpu_clk::SET_MASK != SET_SEL158 & 0xffff {
        return Err(ClockRefusal::ReadBack(gpu_clk::SEL_CON, out.sel[0]));
    }
    if found.sel159 & gpu_clk::DIV_MASK != 0 {
        m.write32(sel159, SET_SEL159);
        out.wrote[1] = SET_SEL159;
    }
    out.sel[1] = m.read32(sel159);
    if out.sel[1] & gpu_clk::DIV_MASK != 0 {
        return Err(ClockRefusal::ReadBack(gpu_clk::STACKS_SEL_CON, out.sel[1]));
    }
    out.src_hz = gpu_clock(m, cru).src_hz;
    if out.src_hz != SET_HZ {
        return Err(ClockRefusal::Rate(out.src_hz));
    }
    Ok(out)
}

/// Linux's wait for a `pll_rk3588` to lock: 1000 µs, where the lock takes
/// 250 input clock cycles typically and 500 at most (clk-pll.c v6.18:868-885
/// `rockchip_rk3588_pll_wait_lock`).
pub const PLL_LOCK_WAIT_US: u32 = 1_000;

/// What [`pll_on`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PllOn {
    /// The PLL read powered before anything was written (nothing written).
    pub was_on: bool,
    /// Microseconds the wait for the lock took.
    pub waited_us: u32,
    /// `PLL_CON6` once it read locked.
    pub con6: u32,
}

/// Power `pll` on as Linux does (clk-pll.c v6.18:1008-1017 `rockchip_rk3588_pll_enable`):
/// clear [`PLL_CON1_PWRDOWN`] through its write-enable twin, then wait for
/// [`PLL_CON6_LOCK`]. A PLL that reads powered is not written; its lock is
/// still waited for. The rate words and `MODE_CON0` are left as they are.
/// `Err(PLL_CON6)` when it did not lock within [`PLL_LOCK_WAIT_US`].
///
/// `delay_us` waits that many microseconds.
pub fn pll_on(m: &impl Mmio, cru: usize, pll: Pll, delay_us: &mut impl FnMut(u32)) -> Result<PllOn, u32> {
    let con1 = cru + pll_con(pll.con + 1);
    let con6 = cru + pll_con(pll.con + 6);
    let was_on = m.read32(con1) & PLL_CON1_PWRDOWN == 0;
    if !was_on {
        m.write32(con1, PLL_CON1_PWRDOWN << 16);
    }
    let waited_us =
        wait(m, con6, PLL_LOCK_WAIT_US, delay_us, |v| v & PLL_CON6_LOCK != 0).ok_or_else(|| m.read32(con6))?;
    Ok(PllOn { was_on, waited_us, con6: m.read32(con6) })
}

/// The PMU (`power-management@fd8d8000`, rk3588-base.dtsi:842-844).
pub const PMU_BASE: usize = 0xfd8d_8000;

/// PD_GPU in the PMU: `rk3588_pmu` and its `RK3588_PD_GPU` entry
/// (drivers/pmdomain/rockchip/pm-domains.c, v6.18: the `RK3588_PD_GPU` row :1277,
/// `rk3588_pmu` :1493-1506; `DOMAIN_M_O_R`, :154-171). Each control
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
/// - The order of `rockchip_pd_power` (:615-660): the domain's clocks on
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
    use super::{pmu, PD_GPU};

    pub const PWR_GATE: usize = pmu::PWR;
    pub const STATUS: usize = pmu::STATUS;
    pub const BUS_IDLE_REQ: usize = pmu::REQ;
    pub const BUS_IDLE_ACK: usize = pmu::ACK;
    pub const BUS_IDLE_ST: usize = pmu::IDLE;
    pub const REPAIR_STATUS: usize = pmu::REPAIR_STATUS;

    pub const PWR_BIT: u32 = PD_GPU.pwr;
    pub const REPAIR_ON_BIT: u32 = PD_GPU.repair;
    pub const IDLE_BIT: u32 = PD_GPU.idle;

    /// Linux's wait for each step (pm-domains.c v6.18: :404, :413, :599).
    pub const WAIT_US: u32 = pmu::WAIT_US;
}

/// The GPU QoS blocks restored after a power-on (`qos_gpu_m0..m3`,
/// rk3588-base.dtsi:1676-1694) and the five words of each
/// (`QOS_PRIORITY` 0x08 .. `QOS_EXTCONTROL` 0x18, pm-domains.c v6.18:86-90).
/// Under Armbian with the GPU active (2026-10-19): `PRIORITY` 0x80000000,
/// the other four 0, in all four blocks.
pub const QOS_GPU: [usize; 4] = [0xfdf3_5000, 0xfdf3_5200, 0xfdf3_5400, 0xfdf3_5600];
pub const QOS_WORDS: [usize; 5] = [0x08, 0x0c, 0x10, 0x14, 0x18];

/// The three QoS words the policy sets: `QOS_MODE` 0x0c, `QOS_BANDWIDTH`
/// 0x10, `QOS_SATURATION` 0x14 (indices 1..=3 of [`QOS_WORDS`]; pm-domains.c:
/// 86-90). `PRIORITY` and `EXTCONTROL` are never written.
pub const QOS_POLICY_WORDS: [usize; 3] = [0x0c, 0x10, 0x14];

/// What the GPU QoS words hold while a job runs (the question of
/// nanopc-t6-mali.md "Open"): the first Z4 frame runs once with each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QosPolicy {
    /// The reset values the image leaves: `MODE` 3, `BANDWIDTH` 0x12d,
    /// `SATURATION` 0x40.
    Reset,
    /// What Linux reads with the GPU active (2026-10-26): all three 0.
    LinuxZero,
}

impl QosPolicy {
    /// The id the Z1 record and the C selector carry: 1 `Reset`, 2
    /// `LinuxZero`; 0 is "none applied" in a record.
    pub const fn id(self) -> u32 {
        match self {
            QosPolicy::Reset => 1,
            QosPolicy::LinuxZero => 2,
        }
    }

    /// The policy of an [`id`](Self::id); `None` for any other number.
    pub const fn from_id(id: u32) -> Option<QosPolicy> {
        match id {
            1 => Some(QosPolicy::Reset),
            2 => Some(QosPolicy::LinuxZero),
            _ => None,
        }
    }

    /// The values for [`QOS_POLICY_WORDS`], in that order.
    pub const fn values(self) -> [u32; 3] {
        match self {
            QosPolicy::Reset => [3, 0x12d, 0x40],
            QosPolicy::LinuxZero => [0; 3],
        }
    }
}

/// Write `policy`'s three words into each of the four GPU QoS blocks
/// ([`QOS_GPU`]): twelve writes, nothing else. `base` is where the caller
/// mapped `qos_gpu_m0` (block i is at `base + 0x200 * i`, the stride of
/// [`QOS_GPU`]); it is not the physical address. The caller has powered
/// PD_GPU on first (the blocks sit in its domain).
pub fn qos_apply(m: &impl Mmio, base: usize, policy: QosPolicy) {
    for blk in 0..QOS_GPU.len() {
        for (off, v) in QOS_POLICY_WORDS.iter().zip(policy.values()) {
            m.write32(base + 0x200 * blk + off, v);
        }
    }
}

/// The PMU registers every power domain uses (`rk3588_pmu`, pm-domains.c
/// v6.18:1493-1506, offsets :1494-1502). A domain's power, memory-power and idle-request
/// registers are these plus the domain's word offset (0 or 4, its row in
/// [`DOMAINS`]); the idle acknowledge, idle state and repair status are one
/// word for all domains. Every control register takes a write-enable twin
/// of its bits in 31:16 (`DOMAIN_M_O_R`, :154-171: `pwr_w_mask`,
/// `req_w_mask`).
pub mod pmu {
    pub const PWR: usize = 0x14c;
    pub const STATUS: usize = 0x180;
    pub const REQ: usize = 0x10c;
    pub const IDLE: usize = 0x120;
    pub const ACK: usize = 0x118;
    pub const MEM_PWR: usize = 0x1a0;
    pub const CHAIN_STATUS: usize = 0x1f0;
    pub const MEM_STATUS: usize = 0x1f8;
    pub const REPAIR_STATUS: usize = 0x290;

    /// Linux's bound for every poll (pm-domains.c v6.18: :404, :413, :526,
    /// :540, :554, :599).
    pub const WAIT_US: u32 = 10_000;
    /// The pause after the memory chain reads on, before the memory power
    /// pulse (pm-domains.c v6.18:534).
    pub const MEM_SETTLE_US: u32 = 20;
}

/// A power domain of the PMU: one row of `rk3588_pm_domains`
/// (pm-domains.c v6.18:1276-1306, macro `DOMAIN_RK3588` :335-336, which
/// gives the idle acknowledge the idle state's mask), the clocks its node
/// names in rk3588-base.dtsi and the domain it sits in there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain {
    pub name: &'static str,
    /// `RK3588_PD_*` (include/dt-bindings/power/rk3588-power.h).
    pub id: u32,
    /// Word offset of the power and memory-power registers (`p_offset`).
    pub pwr_word: usize,
    /// Power-off bit (`pwr`): 1 = off.
    pub pwr: u32,
    /// Word offset of the memory status and chain status (`m_offset`).
    pub mem_word: usize,
    /// Memory status bit (`m_status`), 0 for none: 1 = memory off.
    pub mem: u32,
    /// Repair status bit (`r_status`): 1 = powered.
    pub repair: u32,
    /// Word offset of the idle request register (`r_offset`).
    pub req_word: usize,
    /// Idle request bits (`req`): 1 = idle requested.
    pub req: u32,
    /// Its bits in the idle acknowledge and the idle state (`idle`): 1 =
    /// acknowledged, idle.
    pub idle: u32,
    /// The domain this one sits in (its `id`), `None` at the top level.
    pub parent: Option<u32>,
    /// The CRU gates on while the domain switches: the node's `clocks` and
    /// the gates they hang from, a parent before its child.
    pub clocks: &'static [Gate],
}

/// PD_GPU (rk3588-power.h:26, pm-domains.c:1277). Its clocks are
/// [`gpu_clk::GATES`] (rk3588-base.dtsi:892-902: `CLK_GPU`,
/// `CLK_GPU_COREGROUP`, `CLK_GPU_STACKS` and their source).
pub const PD_GPU: Domain = Domain {
    name: "gpu",
    id: 12,
    pwr_word: 0,
    pwr: 1 << 0,
    mem_word: 0,
    mem: 0,
    repair: 1 << 1,
    req_word: 0,
    req: 1 << 0,
    idle: 1 << 0,
    parent: None,
    clocks: &gpu_clk::GATES,
};

/// PD_VOP (rk3588-power.h:46, pm-domains.c:1294): VOP2 and its IOMMU.
/// Clocks (rk3588-base.dtsi:1020-1024): `PCLK_VOP_ROOT` 52/3,
/// `HCLK_VOP_ROOT` 52/2, `ACLK_VOP` 52/9 from `aclk_vop_root` 52/0 through
/// the `aclk_vop_sub_src` mux (clk-rk3588.c:1908-1919, 1926, 2089).
pub const PD_VOP: Domain = Domain {
    name: "vop",
    id: 24,
    pwr_word: 4,
    pwr: 1 << 1,
    mem_word: 0,
    mem: 1 << 25,
    repair: 1 << 16,
    req_word: 0,
    req: (1 << 13) | (1 << 14),
    idle: (1 << 13) | (1 << 14),
    parent: None,
    clocks: &[Gate { con: 52, bit: 0 }, Gate { con: 52, bit: 9 }, Gate { con: 52, bit: 2 }, Gate { con: 52, bit: 3 }],
};

/// PD_VO0 (rk3588-power.h:47, pm-domains.c:1295), inside PD_VOP
/// (rk3588-base.dtsi:1031-1042). The HDMI path does not use it; it is here
/// because PD_VOP goes off only after it. Clocks: `ACLK_VO0_ROOT` 55/0,
/// `HCLK_VO0_ROOT` 55/1, `HCLK_VO0_S_ROOT` 55/2, `PCLK_VO0_ROOT` 55/3,
/// `PCLK_VO0_S_ROOT` 55/4, `hclk_vo0` 55/5 (linked to `HCLK_VOP_ROOT`
/// 52/2), `aclk_hdcp0_pre` 55/9 (linked to `ACLK_VOP_LOW_ROOT` 52/1),
/// `ACLK_HDCP0` 55/12, `HCLK_HDCP0` 55/13 (clk-rk3588.c:1800-1814,
/// 1911-1916, 2328-2331, 2428-2429).
pub const PD_VO0: Domain = Domain {
    name: "vo0",
    id: 25,
    pwr_word: 4,
    pwr: 1 << 2,
    mem_word: 0,
    mem: 1 << 26,
    repair: 1 << 17,
    req_word: 0,
    req: 1 << 15,
    idle: 1 << 15,
    parent: Some(24),
    clocks: &[
        Gate { con: 52, bit: 1 },
        Gate { con: 52, bit: 2 },
        Gate { con: 55, bit: 0 },
        Gate { con: 55, bit: 1 },
        Gate { con: 55, bit: 2 },
        Gate { con: 55, bit: 3 },
        Gate { con: 55, bit: 4 },
        Gate { con: 55, bit: 5 },
        Gate { con: 55, bit: 9 },
        Gate { con: 55, bit: 12 },
        Gate { con: 55, bit: 13 },
    ],
};

/// PD_VO1 (rk3588-power.h:48, pm-domains.c:1296): the HDMI TX
/// controllers. Its idle request is bit 0 of the second request word, its
/// acknowledge and idle state bit 16. Clocks (rk3588-base.dtsi:1044-1056):
/// `ACLK_VO1USB_TOP_ROOT` 74/0 and `HCLK_VO1USB_TOP_ROOT` 74/2 (both
/// critical, the links of `aclk_hdcp1_pre` and `hclk_vo1`),
/// `PCLK_VO1_ROOT` 59/4, `PCLK_VO1_S_ROOT` 59/5, `HCLK_VO1_S_ROOT` 59/3,
/// `HCLK_HDCP1` 60/6 from `hclk_vo1` 59/9 from `hclk_vo1_root` 59/2,
/// `ACLK_HDCP1` 60/5 from `aclk_hdcp1_pre` 59/6 from `aclk_hdcp1_root`
/// 59/0, `ACLK_HDMIRX_ROOT` 59/1 (clk-rk3588.c:1890-1907, 1920-1925,
/// 2316-2319, 2430-2431).
pub const PD_VO1: Domain = Domain {
    name: "vo1",
    id: 26,
    pwr_word: 4,
    pwr: 1 << 3,
    mem_word: 0,
    mem: 1 << 27,
    repair: 1 << 18,
    req_word: 4,
    req: 1 << 0,
    idle: 1 << 16,
    parent: None,
    clocks: &[
        Gate { con: 74, bit: 0 },
        Gate { con: 74, bit: 2 },
        Gate { con: 59, bit: 4 },
        Gate { con: 59, bit: 5 },
        Gate { con: 59, bit: 3 },
        Gate { con: 59, bit: 2 },
        Gate { con: 59, bit: 9 },
        Gate { con: 60, bit: 6 },
        Gate { con: 59, bit: 0 },
        Gate { con: 59, bit: 6 },
        Gate { con: 60, bit: 5 },
        Gate { con: 59, bit: 1 },
    ],
};

/// The domains this module switches.
pub const DOMAINS: [Domain; 4] = [PD_GPU, PD_VOP, PD_VO0, PD_VO1];

/// A domain's PMU registers at one moment: the words its row selects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdRegs {
    pub pwr_gate: u32,
    pub status: u32,
    pub idle_req: u32,
    pub idle_ack: u32,
    pub idle_st: u32,
    pub repair: u32,
}

/// PD_GPU's registers ([`PdRegs`] of [`PD_GPU`]); the name the board code
/// and its record use.
pub type PdGpuRegs = PdRegs;

impl PdRegs {
    /// Read `d`'s words. Nothing is written; the PMU is always powered.
    pub fn of(m: &impl Mmio, pmu: usize, d: &Domain) -> PdRegs {
        PdRegs {
            pwr_gate: m.read32(pmu + pmu::PWR + d.pwr_word),
            status: m.read32(pmu + pmu::STATUS),
            idle_req: m.read32(pmu + pmu::REQ + d.req_word),
            idle_ack: m.read32(pmu + pmu::ACK),
            idle_st: m.read32(pmu + pmu::IDLE),
            repair: m.read32(pmu + pmu::REPAIR_STATUS),
        }
    }

    /// PD_GPU's words.
    pub fn read(m: &impl Mmio, pmu: usize) -> PdRegs {
        PdRegs::of(m, pmu, &PD_GPU)
    }

    /// `d` is powered (its repair status bit).
    pub const fn powered(&self, d: &Domain) -> bool {
        self.repair & d.repair != 0
    }

    /// `d` is powered and out of idle: no idle request bit, no acknowledge
    /// bit and no idle state bit of `d` set. Each of the three alone keeps
    /// the domain from being usable: an acknowledge without the idle
    /// state is a handshake still under way. Its blocks' registers can be
    /// read only when this holds.
    pub const fn usable_of(&self, d: &Domain) -> bool {
        self.powered(d) && self.idle_req & d.req == 0 && (self.idle_ack | self.idle_st) & d.idle == 0
    }

    /// PD_GPU is usable ([`PdRegs::usable_of`]).
    pub const fn usable(&self) -> bool {
        self.usable_of(&PD_GPU)
    }
}

/// Why a domain switch stopped. The value crosses to C, one number per
/// cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PdRefusal {
    /// A clock gate of the domain still reads gated after its write.
    GateStuck = 1,
    /// The repair status bit did not reach the state asked for within
    /// [`pmu::WAIT_US`].
    PowerTimeout = 2,
    /// The idle acknowledge did not reach the state asked for.
    IdleAckTimeout = 3,
    /// The idle state did not reach the state asked for.
    IdleTimeout = 4,
    /// The memory chain status did not read on (power-on memory reset).
    MemChainTimeout = 5,
    /// The memory status did not read off during the memory power pulse.
    MemOffTimeout = 6,
    /// The memory status did not read on again after it.
    MemOnTimeout = 7,
    /// The domain's parent is not usable: it goes on first.
    ParentOff = 8,
    /// A domain inside this one is still powered: it goes off first.
    ChildOn = 9,
}

/// A refused domain switch: which domain, why, and the word read last
/// (the register the step polled, the gate's CLKGATE_CON, or for
/// `ParentOff` / `ChildOn` the repair status).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PdError {
    /// The domain asked to switch.
    pub domain: &'static str,
    pub why: PdRefusal,
    pub read: u32,
    /// For `ParentOff` the parent that is not usable, for `ChildOn` the
    /// child that is still powered; `None` for the other causes.
    pub cause: Option<&'static str>,
}

/// What [`pd_on`] or [`pd_off`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdOn {
    /// The gates it opened. From [`pd_gpu_on`]: a bit mask of their
    /// CLKGATE_CON(66) bits. From [`pd_on`] / [`pd_off`]: bit i is the
    /// domain's `clocks[i]`.
    pub opened: u32,
    /// The domain already read in the state asked for before anything was
    /// written to the PMU (the power and idle steps skipped).
    pub was_on: bool,
    /// Microseconds each wait took: power, idle acknowledge, idle state.
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

/// The domain of [`DOMAINS`] with `id`.
fn domain_by_id(id: u32) -> Option<&'static Domain> {
    let mut i = 0;
    while i < DOMAINS.len() {
        if DOMAINS[i].id == id {
            return Some(&DOMAINS[i]);
        }
        i += 1;
    }
    None
}

/// Open `d`'s clocks, parent first: the mask of [`PdOn::opened`] by index.
fn open_clocks(m: &impl Mmio, cru: usize, d: &Domain) -> Result<u32, PdError> {
    let mut opened = 0;
    for (i, g) in d.clocks.iter().enumerate() {
        match gate_open(m, cru, *g) {
            Ok(true) => opened |= 1 << i,
            Ok(false) => {}
            Err(now) => return Err(PdError { domain: d.name, why: PdRefusal::GateStuck, read: now, cause: None }),
        }
    }
    Ok(opened)
}

/// Poll `bits` of the word at `addr` until they read `want`, or refuse with
/// `why` for `d`.
fn wait_bits(
    m: &impl Mmio,
    addr: usize,
    bits: u32,
    want: u32,
    d: &Domain,
    why: PdRefusal,
    delay_us: &mut impl FnMut(u32),
) -> Result<u32, PdError> {
    wait(m, addr, pmu::WAIT_US, delay_us, |v| v & bits == want).ok_or_else(|| PdError {
        domain: d.name,
        why,
        read: m.read32(addr),
        cause: None,
    })
}

/// Ask for idle (`idle` true) or release it, and wait for the acknowledge
/// and the idle state to follow (`rockchip_pmu_set_idle_request`,
/// pm-domains.c v6.18:376-422). The waited microseconds of the two polls.
///
/// The acknowledge poll is Linux's: all of `d.idle` set, or all clear
/// (:399-404). The idle-state poll asking for idle is Linux's too (all set,
/// `rockchip_pmu_domain_is_idle` :338-346 with :411-413). On release Linux
/// waits only for "not all set", which for PD_VOP's two bits passes with
/// one bus interface still idle; this waits for all clear, the state
/// [`PdRegs::usable_of`] asks before a block is touched and the state the
/// acknowledge poll just required. A bit that stays set is an
/// `IdleTimeout` for the domain, not a domain reported usable with half
/// its bus idle.
fn idle_request(
    m: &impl Mmio,
    pmu: usize,
    d: &Domain,
    idle: bool,
    delay_us: &mut impl FnMut(u32),
) -> Result<[u32; 2], PdError> {
    let req = d.req;
    m.write32(pmu + pmu::REQ + d.req_word, (req << 16) | if idle { req } else { 0 });
    let want = if idle { d.idle } else { 0 };
    let ack = wait_bits(m, pmu + pmu::ACK, d.idle, want, d, PdRefusal::IdleAckTimeout, delay_us)?;
    let st = wait_bits(m, pmu + pmu::IDLE, d.idle, want, d, PdRefusal::IdleTimeout, delay_us)?;
    Ok([ack, st])
}

/// The memory reset after a power-on of a domain whose memory read on
/// while the domain was off (`rockchip_pmu_domain_mem_reset`, pm-domains.c
/// v6.18:518-563, called from :591-595): wait for the memory chain, pause
/// [`pmu::MEM_SETTLE_US`], memory power off and wait for it, memory power
/// on and wait for it.
fn mem_reset(m: &impl Mmio, pmu: usize, d: &Domain, delay_us: &mut impl FnMut(u32)) -> Result<(), PdError> {
    let mem_status = pmu + pmu::MEM_STATUS + d.mem_word;
    let mem_pwr = pmu + pmu::MEM_PWR + d.pwr_word;
    wait_bits(m, pmu + pmu::CHAIN_STATUS + d.mem_word, d.mem, d.mem, d, PdRefusal::MemChainTimeout, delay_us)?;
    delay_us(pmu::MEM_SETTLE_US);
    m.write32(mem_pwr, (d.pwr << 16) | d.pwr);
    wait_bits(m, mem_status, d.mem, d.mem, d, PdRefusal::MemOffTimeout, delay_us)?;
    m.write32(mem_pwr, d.pwr << 16);
    wait_bits(m, mem_status, d.mem, 0, d, PdRefusal::MemOnTimeout, delay_us)?;
    Ok(())
}

/// Power `d` on in Linux's order (`rockchip_pd_power`, pm-domains.c
/// v6.18:615-660; `rockchip_do_pmu_set_power_domain`, :565-613): open the
/// domain's clocks, clear the power bit, the memory reset when the memory
/// read on before, wait for the repair status, release the idle request and
/// wait for the acknowledge and the idle state.
///
/// A domain already usable ([`PdRegs::usable_of`]) is not written (only its
/// gates are opened). One that is powered with its idle request still set
/// runs the sequence: the power wait passes at once and the idle step
/// releases the request; it gets no memory reset (that is for a domain
/// coming from off). A parent that is not usable refuses before anything is
/// written, naming the parent. Not done here: the domain's supply (PD_GPU's
/// vdd_gpu_s0, `need_regulator` true at pm-domains.c:1277; false for the
/// display rows), which the caller turns on first; the QoS restore (the
/// caller's) and the firmware notice (SIP `SUSPEND_MODE`, system suspend
/// only).
///
/// `delay_us` waits that many microseconds.
pub fn pd_on(
    m: &impl Mmio,
    cru: usize,
    pmu: usize,
    d: &Domain,
    delay_us: &mut impl FnMut(u32),
) -> Result<PdOn, PdError> {
    if let Some(p) = d.parent.and_then(domain_by_id) {
        let regs = PdRegs::of(m, pmu, p);
        if !regs.usable_of(p) {
            return Err(PdError { domain: d.name, why: PdRefusal::ParentOff, read: regs.repair, cause: Some(p.name) });
        }
    }
    let mut out = PdOn { opened: open_clocks(m, cru, d)?, ..PdOn::default() };
    let now = PdRegs::of(m, pmu, d);
    if now.usable_of(d) {
        out.was_on = true;
        return Ok(out);
    }
    let mem_reset_due = d.mem != 0 && !now.powered(d) && m.read32(pmu + pmu::MEM_STATUS + d.mem_word) & d.mem == 0;
    m.write32(pmu + pmu::PWR + d.pwr_word, d.pwr << 16);
    if mem_reset_due {
        mem_reset(m, pmu, d, delay_us)?;
    }
    out.waited_us[0] =
        wait_bits(m, pmu + pmu::REPAIR_STATUS, d.repair, d.repair, d, PdRefusal::PowerTimeout, delay_us)?;
    let [ack, st] = idle_request(m, pmu, d, false, delay_us)?;
    out.waited_us[1] = ack;
    out.waited_us[2] = st;
    Ok(out)
}

/// Power `d` off in Linux's order (`rockchip_pd_power`, pm-domains.c
/// v6.18:615-660): open the domain's clocks, ask for idle and wait for the
/// acknowledge and the idle state, set the power bit, wait for the repair
/// status to clear. The caller has saved what it needs from the domain's
/// blocks (their registers do not survive) and stopped them. A domain with
/// a powered domain inside it refuses before anything is written, naming
/// the child; one that reads off is not written to the PMU (its gates are
/// opened). The clocks are left open.
pub fn pd_off(
    m: &impl Mmio,
    cru: usize,
    pmu: usize,
    d: &Domain,
    delay_us: &mut impl FnMut(u32),
) -> Result<PdOn, PdError> {
    let repair = m.read32(pmu + pmu::REPAIR_STATUS);
    for c in DOMAINS.iter().filter(|c| c.parent == Some(d.id)) {
        if repair & c.repair != 0 {
            return Err(PdError { domain: d.name, why: PdRefusal::ChildOn, read: repair, cause: Some(c.name) });
        }
    }
    let mut out = PdOn { opened: open_clocks(m, cru, d)?, ..PdOn::default() };
    let now = PdRegs::of(m, pmu, d);
    if !now.powered(d) {
        out.was_on = true;
        return Ok(out);
    }
    let [ack, st] = idle_request(m, pmu, d, true, delay_us)?;
    out.waited_us[1] = ack;
    out.waited_us[2] = st;
    m.write32(pmu + pmu::PWR + d.pwr_word, (d.pwr << 16) | d.pwr);
    out.waited_us[0] = wait_bits(m, pmu + pmu::REPAIR_STATUS, d.repair, 0, d, PdRefusal::PowerTimeout, delay_us)?;
    Ok(out)
}

/// Power PD_GPU on: [`pd_on`] of [`PD_GPU`], with the opened gates as their
/// CLKGATE_CON(66) bits and the refusal as the pair the board record keeps.
/// The supply (vdd_gpu_s0) is not touched: the caller checks it first. The
/// QoS words are not written: the caller reads them back.
pub fn pd_gpu_on(
    m: &impl Mmio,
    cru: usize,
    pmu: usize,
    delay_us: &mut impl FnMut(u32),
) -> Result<PdOn, (PdRefusal, u32)> {
    let mut on = pd_on(m, cru, pmu, &PD_GPU, delay_us).map_err(|e| (e.why, e.read))?;
    let by_index = on.opened;
    on.opened = 0;
    for (i, g) in PD_GPU.clocks.iter().enumerate() {
        if by_index & (1 << i) != 0 {
            on.opened |= 1 << g.bit;
        }
    }
    Ok(on)
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

    /// The QoS blocks as plain registers, with a write log.
    struct Qos {
        regs: RefCell<std::collections::BTreeMap<usize, u32>>,
        writes: RefCell<Vec<usize>>,
    }
    impl Mmio for Qos {
        fn read32(&self, addr: usize) -> u32 {
            *self.regs.borrow().get(&addr).unwrap_or(&0)
        }
        fn write32(&self, addr: usize, val: u32) {
            self.writes.borrow_mut().push(addr);
            self.regs.borrow_mut().insert(addr, val);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    /// All four blocks in the image's state (reset values), then `policy`.
    fn qos_check(policy: QosPolicy, want: [u32; 3]) {
        // a mapped base that is not the physical address
        const MAPPED: usize = 0x7000_0000;
        let blocks = [0, 1, 2, 3].map(|i| MAPPED + 0x200 * i);
        let q = Qos { regs: RefCell::new(Default::default()), writes: RefCell::new(vec![]) };
        for b in blocks {
            for (o, v) in [(0x08, 0x8000_0000), (0x0c, 3), (0x10, 0x12d), (0x14, 0x40)] {
                q.regs.borrow_mut().insert(b + o, v);
            }
        }
        qos_apply(&q, MAPPED, policy);
        let expect: Vec<usize> = blocks.iter().flat_map(|b| [0x0c, 0x10, 0x14].map(|o| b + o)).collect();
        assert_eq!(*q.writes.borrow(), expect);
        for b in blocks {
            assert_eq!([q.read32(b + 0x0c), q.read32(b + 0x10), q.read32(b + 0x14)], want);
            assert_eq!(q.read32(b + 0x08), 0x8000_0000);
            assert_eq!(q.read32(b + 0x18), 0);
        }
    }

    #[test]
    fn qos_reset_policy_writes_only_mode_bandwidth_saturation() {
        qos_check(QosPolicy::Reset, [3, 0x12d, 0x40]);
    }

    #[test]
    fn qos_linux_zero_policy_writes_only_mode_bandwidth_saturation() {
        qos_check(QosPolicy::LinuxZero, [0, 0, 0]);
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
        /// Registers that ignore writes (logged, not applied).
        stuck: Vec<usize>,
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
                (CRU_BASE + pll_con(110), 0x9c00),
                (PMU + pd_gpu::PWR_GATE, 0xffff),
                (PMU + pd_gpu::STATUS, 0x2455_ffff),
                (PMU + pd_gpu::BUS_IDLE_REQ, 0x83cf),
                (PMU + pd_gpu::BUS_IDLE_ACK, 0x000a_9fff),
                (PMU + pd_gpu::BUS_IDLE_ST, 0x000a_9fff),
                (PMU + pd_gpu::REPAIR_STATUS, 0xfad5_0001),
            ] {
                r.insert(a, v);
            }
            Soc {
                regs: RefCell::new(r),
                writes: RefCell::new(vec![]),
                lag,
                reads_since: RefCell::new(0),
                dead: false,
                stuck: vec![],
            }
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
            if self.stuck.contains(&addr) {
                return;
            }
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
        assert_eq!(c.pll_con6, 0x9c00);
        assert_eq!(c.src_hz, 300_000_000);
        assert!(soc.writes.borrow().is_empty());
    }

    /// NPLL as the T6 read it on 2026-11-11, powered down by Linux.
    fn npll_down(soc: &Soc) {
        for (a, v) in [(120, 0x1a9), (121, 0x2083), (122, 0), (126, 0x4000)] {
            soc.regs.borrow_mut().insert(CRU_BASE + pll_con(a), v);
        }
    }

    #[test]
    fn a_powered_down_pll_is_powered_through_its_write_mask_and_waited_for() {
        let soc = Soc::t6_suspended(0);
        npll_down(&soc);
        soc.regs.borrow_mut().insert(CRU_BASE + clksel_con(158), 0x1f60);
        let c = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_src(c.sel158), (GpuSrc::Npll, 1));
        assert_eq!((c.pll_con, c.pll_con6, c.src_hz), ([0x1a9, 0x2083, 0], 0x4000, 850_000_000));

        // the lock comes 2 polls after the write
        let mut slept = 0;
        let on = pll_on(&soc, CRU_BASE, NPLL, &mut |us| {
            slept += us;
            if slept == 2 {
                *soc.regs.borrow_mut().get_mut(&(CRU_BASE + pll_con(126))).unwrap() |= PLL_CON6_LOCK;
            }
        });
        assert_eq!(on, Ok(PllOn { was_on: false, waited_us: 2, con6: 0xc000 }));
        assert_eq!(slept, 2);
        assert_eq!(*soc.writes.borrow(), vec![(CRU_BASE + pll_con(121), 0x2000_0000)]);
        // only bit 13 moved: the rate words are Linux's
        assert_eq!(soc.get(CRU_BASE + pll_con(121)), 0x83);
    }

    #[test]
    fn a_pll_that_is_powered_and_locked_is_not_written() {
        let soc = Soc::t6_suspended(0);
        assert_eq!(pll_on(&soc, CRU_BASE, CPLL, &mut |_| {}), Ok(PllOn { was_on: true, waited_us: 0, con6: 0x9c00 }));
        assert!(soc.writes.borrow().is_empty());
    }

    #[test]
    fn a_pll_that_never_locks_times_out_after_linux_1_ms() {
        let soc = Soc::t6_suspended(0);
        npll_down(&soc);
        let mut slept = 0;
        assert_eq!(pll_on(&soc, CRU_BASE, NPLL, &mut |us| slept += us), Err(0x4000));
        assert_eq!(slept, PLL_LOCK_WAIT_US + 1);
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
    fn the_opp_table_bounds_the_clock_by_the_supply() {
        assert_eq!(gpu_opp_min_uv(300_000_000), Some(675_000));
        assert_eq!(gpu_opp_min_uv(600_000_000), Some(675_000));
        assert_eq!(gpu_opp_min_uv(600_000_001), Some(700_000));
        assert_eq!(gpu_opp_min_uv(850_000_000), Some(800_000));
        assert_eq!(gpu_opp_min_uv(1_000_000_000), Some(850_000));
        assert_eq!(gpu_opp_min_uv(1_000_000_001), None);
        assert_eq!(gpu_opp_min_uv(0), None);
        // 198 MHz (GPLL / 6, panthor's probe-time rate) is under the table's
        // lowest voltage, but it is not an OPP rate
        assert_eq!(gpu_opp_min_uv(198_000_000), Some(675_000));
        assert!(!gpu_opp_rate(198_000_000));
        assert!(gpu_opp_rate(300_000_000) && gpu_opp_rate(1_000_000_000));
        assert!(!gpu_opp_rate(850_000_000) && !gpu_opp_rate(0));
        // the one setting: CPLL / 5 is the table's first row
        assert_eq!((SET_HZ, SET_CPLL_HZ / SET_DIV as u64), (GPU_OPP[0].0, SET_HZ));
        assert_eq!((SET_SEL158, SET_SEL159), (0x40ff_0024, 0x001f_0000));
    }

    const SEL158: usize = CRU_BASE + clksel_con(158);
    const SEL159: usize = CRU_BASE + clksel_con(159);

    /// The T6 under Armbian with CLKSEL_CON(158) as given, and the clock set
    /// from it: the result and the CRU writes.
    fn set_from(sel158: u32) -> (Soc, Result<ClockSet, ClockRefusal>) {
        let soc = Soc::t6_suspended(0);
        soc.regs.borrow_mut().insert(SEL158, sel158);
        let found = gpu_clock(&soc, CRU_BASE);
        let r = gpu_clock_set(&soc, CRU_BASE, &found);
        (soc, r)
    }

    #[test]
    fn the_clock_armbian_leaves_is_kept_without_a_write() {
        let (soc, r) = set_from(0x1f24);
        assert_eq!(
            r,
            Ok(ClockSet { kept: true, cpll_hz: 1_500_000_000, wrote: [0; 2], sel: [0x1f24, 0], src_hz: 300_000_000 })
        );
        assert!(soc.writes.borrow().is_empty(), "{:x?}", soc.writes.borrow());
        assert_eq!(soc.get(SEL158), 0x1f24);
    }

    /// The setting the T6's state moves to, whatever it started from.
    fn assert_moved_to_cpll_over_5(soc: &Soc, r: Result<ClockSet, ClockRefusal>) {
        assert_eq!(
            r,
            Ok(ClockSet {
                kept: false,
                cpll_hz: 1_500_000_000,
                wrote: [0x40ff_0024, 0],
                sel: [0x1f24, 0],
                src_hz: 300_000_000
            })
        );
        assert_eq!(*soc.writes.borrow(), vec![(SEL158, 0x40ff_0024)]);
        assert_eq!(soc.get(SEL158), 0x1f24);
        assert_eq!(gpu_clock(soc, CRU_BASE).src_hz, 300_000_000);
    }

    #[test]
    fn the_pvtpll_is_left_for_cpll_over_5_in_one_write() {
        let (soc, r) = set_from(0x1f24 | gpu_clk::PVTPLL);
        assert_moved_to_cpll_over_5(&soc, r);
    }

    #[test]
    fn a_divider_off_the_table_is_rewritten_to_5() {
        // CPLL / 2 = 750 MHz and / 4 = 375 MHz are no OPP rates
        for sel in [0x1f21, 0x1f23] {
            let (soc, r) = set_from(sel);
            assert_moved_to_cpll_over_5(&soc, r);
        }
        // CPLL / 3 = 500 MHz is one, kept
        let (soc, r) = set_from(0x1f22);
        assert_eq!(r.map(|s| (s.kept, s.src_hz)), Ok((true, 500_000_000)));
        assert!(soc.writes.borrow().is_empty());
    }

    #[test]
    fn a_mux_value_past_the_parents_is_rewritten() {
        for mux in 5..8 {
            let (soc, r) = set_from(0x1f04 | (mux << 5));
            assert_moved_to_cpll_over_5(&soc, r);
        }
        // SPLL (4) has no rate this reads either
        let (soc, r) = set_from(0x1f84);
        assert_moved_to_cpll_over_5(&soc, r);
    }

    #[test]
    fn the_stacks_divider_is_set_to_1_after_the_source() {
        let soc = Soc::t6_suspended(0);
        soc.regs.borrow_mut().insert(SEL158, 0x5f24);
        soc.regs.borrow_mut().insert(SEL159, 0x3);
        let found = gpu_clock(&soc, CRU_BASE);
        let r = gpu_clock_set(&soc, CRU_BASE, &found);
        assert_eq!(r.map(|s| (s.wrote, s.sel, s.src_hz)), Ok(([0x40ff_0024, 0x001f_0000], [0x1f24, 0], 300_000_000)));
        assert_eq!(*soc.writes.borrow(), vec![(SEL158, 0x40ff_0024), (SEL159, 0x001f_0000)]);
        // a source on the table with a stacks divider off 1 is written too
        let soc = Soc::t6_suspended(0);
        soc.regs.borrow_mut().insert(SEL159, 0x1);
        let found = gpu_clock(&soc, CRU_BASE);
        let r = gpu_clock_set(&soc, CRU_BASE, &found);
        assert_eq!(r.map(|s| (s.kept, s.wrote)), Ok((false, [0x40ff_0024, 0x001f_0000])));
    }

    #[test]
    fn a_cpll_at_another_rate_refuses_without_a_write() {
        // GPLL's 1188 MHz in the CPLL's words: / 5 = 237.6 MHz, no divider hits the table
        let soc = Soc::t6_suspended(0);
        soc.regs.borrow_mut().insert(SEL158, 0x5f24);
        soc.regs.borrow_mut().insert(CRU_BASE + pll_con(104), 0xc6);
        let found = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_clock_set(&soc, CRU_BASE, &found), Err(ClockRefusal::NoTableRate(1_188_000_000)));
        assert!(soc.writes.borrow().is_empty());
        // CPLL / 5 with the CPLL at 1188 MHz is off the table as well: refused, not kept
        soc.regs.borrow_mut().insert(SEL158, 0x1f24);
        let found = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_clock_set(&soc, CRU_BASE, &found), Err(ClockRefusal::NoTableRate(1_188_000_000)));
        // CPLL in slow mode (MODE_CON0 bits 9:8 = 0): no rate claimed for it
        let soc = Soc::t6_suspended(0);
        soc.regs.borrow_mut().insert(CRU_BASE + MODE_CON0, 0x445);
        let found = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_clock_set(&soc, CRU_BASE, &found), Err(ClockRefusal::NoTableRate(0)));
        assert!(soc.writes.borrow().is_empty());
    }

    #[test]
    fn a_read_back_that_does_not_match_refuses() {
        let mut soc = Soc::t6_suspended(0);
        soc.stuck.push(SEL158);
        soc.regs.borrow_mut().insert(SEL158, 0x5f24);
        let found = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_clock_set(&soc, CRU_BASE, &found), Err(ClockRefusal::ReadBack(158, 0x5f24)));
        assert_eq!(soc.writes.borrow().len(), 1);
        let mut soc = Soc::t6_suspended(0);
        soc.stuck.push(SEL159);
        soc.regs.borrow_mut().insert(SEL158, 0x5f24);
        soc.regs.borrow_mut().insert(SEL159, 0x2);
        let found = gpu_clock(&soc, CRU_BASE);
        assert_eq!(gpu_clock_set(&soc, CRU_BASE, &found), Err(ClockRefusal::ReadBack(159, 0x2)));
        assert_eq!(soc.writes.borrow().len(), 2);
    }

    #[test]
    fn the_set_path_never_writes_a_pll_con_mode_con_or_grf_word() {
        let states = [0x1f24, 0x5f24, 0x1f21, 0x1f23, 0x1fa4, 0x1fe4, 0x1f84, 0x1f60, 0x1f00];
        for sel in states {
            let soc = Soc::t6_suspended(0);
            npll_down(&soc);
            for (a, v) in [(112, 0xc6), (113, 0x42), (114, 0), (118, 0x9c00)] {
                soc.regs.borrow_mut().insert(CRU_BASE + pll_con(a), v);
            }
            soc.regs.borrow_mut().insert(SEL158, sel);
            // the Armbian state is the T6's own, with CLKSEL_CON(159) 0; the others start with an
            // out-of-table stacks divider
            soc.regs.borrow_mut().insert(SEL159, if sel == 0x1f24 { 0 } else { 0x1 });
            let found = gpu_clock(&soc, CRU_BASE);
            let r = gpu_clock_set(&soc, CRU_BASE, &found);
            assert!(r.is_ok(), "{sel:#x}: {r:?}");
            if sel == 0x1f24 {
                // Armbian's state is kept: no write at all
                assert!(soc.writes.borrow().is_empty(), "{:x?}", soc.writes.borrow());
            }
            for (a, v) in soc.writes.borrow().iter() {
                // the CRU's write-mask idiom: a non-zero mask in 31:16 and no
                // bit of 15:0 outside it
                let (mask, val) = (v >> 16, v & 0xffff);
                assert!(mask != 0, "{sel:#x}: write {v:#x} to {a:#x} has no write enable");
                assert!(val & !mask == 0, "{sel:#x}: write {v:#x} to {a:#x} sets bits outside its mask");
                assert!(*a == SEL158 || *a == SEL159, "{sel:#x}: a write to {a:#x}");
                // and in particular nothing in the PLL_CON / MODE_CON range (CRU + 0..0x300) or outside the CRU
                assert!(*a >= CRU_BASE + 0x300 && *a < CRU_BASE + 0x1000);
            }
        }
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
    fn a_powered_domain_with_its_idle_request_set_is_released_from_idle() {
        let soc = Soc::t6_suspended(0);
        for (a, v) in [(pd_gpu::PWR_GATE, 0xfffe), (pd_gpu::REPAIR_STATUS, 0xfad5_0003)] {
            soc.regs.borrow_mut().insert(PMU + a, v);
        }
        assert!(!PdGpuRegs::read(&soc, PMU).usable());
        let on = pd_gpu_on(&soc, CRU_BASE, PMU, &mut |_| {}).unwrap();
        assert!(!on.was_on);
        assert_eq!(on.waited_us, [0, 0, 0]);
        let pmu: Vec<_> = soc.writes.borrow().iter().filter(|w| w.0 >= PMU).copied().collect();
        assert_eq!(pmu, vec![(PMU + pd_gpu::PWR_GATE, 0x0001_0000), (PMU + pd_gpu::BUS_IDLE_REQ, 0x0001_0000)]);
        assert!(PdGpuRegs::read(&soc, PMU).usable());
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
