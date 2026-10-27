// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PLLs of the i.MX 8M Plus CCM_ANALOG block ("ANATOP"): VIDEO_PLL1,
//! which this driver programs for the pixel clock, and SYS_PLL1 / SYS_PLL2,
//! which it only reads to check that a clock root's source runs.
//!
//! From IMX8MPRM Rev. 3, 08/2024 ("RM"); register names and bits from
//! hal_nxp `PERI_CCM_ANALOG.h` ([`super::regs::ccm_analog`]).
//!
//! - The PLL formula, RM 5.1.5.4.4 "SSCG and Fractional PLLs":
//!   `FOUT = ((m + k/65536) x FIN) / (p x 2^s)`, `1 <= p <= 63`,
//!   `64 <= m <= 1023`, `0 <= s <= 6`, `-32768 <= k <= 32767`, k two's
//!   complement. VIDEO_PLL1 is one of the fractional PLLs (same section).
//!   The system PLLs have no DSM register (RM 5.1.8.30 / 5.1.8.33 list only
//!   MAIN/PRE/POST); their rate is the same formula with k = 0, which the RM
//!   does not state separately.
//! - FIN is 24M_REF_CLK when `PLL_REF_CLK_SEL` = 00 (RM 5.1.8.11 / 5.1.8.32),
//!   24 MHz (RM 5.1.5.1).
//! - `GEN_CTRL`: `PLL_LOCK` bit 31, `PLL_RST` bit 9 active low, `PLL_CLKE`
//!   the output gate, `PLL_BYPASS` / `PLL_EXT_BYPASS` (RM 5.1.8.11, 5.1.8.32);
//!   `FDIV_CTL0`: MAIN 21:12, PRE 9:4, POST 2:0 (5.1.8.12); `FDIV_CTL1`: DSM
//!   15:0 (5.1.8.13).
//!
//! Not stated in IMX8MPRM Rev. 3:
//! - the programming order (hold `PLL_RST` low, write the dividers, release,
//!   wait for `PLL_LOCK`) and the 3 us between the divider write and the
//!   release: the replaced driver's order, which ran on this board;
//! - the lock time; the wait is bounded at 1 ms ([`LOCK_POLL_US`]);
//! - the VCO range of the fractional PLL, so p and s are not searched: p = 2,
//!   s = 1 are the values the board's Linux runs VIDEO_PLL1 with
//!   (golden-linux-2026-09-21.txt: `FDIV_CTL0` 0x000ad021), and m / k follow
//!   from the formula ([`video_pll1_divs`]).

use super::regs::ccm_analog as ana;
use crate::env::Env;
use crate::errno::{EINVAL, ETIMEDOUT};
use crate::mmio::Mmio;

/// FIN, 24M_REF_CLK (RM 5.1.5.1).
pub const FIN_HZ: u64 = 24_000_000;
/// Bound on the lock wait, us (not from the RM; see the module text).
pub const LOCK_POLL_US: u32 = 1000;
/// p and s VIDEO_PLL1 is run with (see the module text).
pub const VIDEO_PLL1_P: u32 = 2;
pub const VIDEO_PLL1_S: u32 = 1;

/// The dividers of a PLL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divs {
    pub m: u32,
    pub p: u32,
    pub s: u32,
    pub k: i32,
}

impl Divs {
    /// In the RM's ranges.
    pub fn valid(self) -> bool {
        (64..=1023).contains(&self.m) && (1..=63).contains(&self.p) && self.s <= 6 && (-32768..=32767).contains(&self.k)
    }

    /// `FOUT` in Hz, rounded down.
    pub fn rate_hz(self) -> u64 {
        let num = FIN_HZ as i128 * (i128::from(self.m) * 65536 + i128::from(self.k));
        let den = 65536i128 * i128::from(self.p) * (1i128 << self.s);
        (num / den) as u64
    }

    pub fn fdiv_ctl0(self) -> u32 {
        ((self.m << ana::video_pll1_fdiv_ctl0::PLL_MAIN_DIV_SHIFT) & ana::video_pll1_fdiv_ctl0::PLL_MAIN_DIV_MASK)
            | ((self.p << ana::video_pll1_fdiv_ctl0::PLL_PRE_DIV_SHIFT) & ana::video_pll1_fdiv_ctl0::PLL_PRE_DIV_MASK)
            | ((self.s << ana::video_pll1_fdiv_ctl0::PLL_POST_DIV_SHIFT) & ana::video_pll1_fdiv_ctl0::PLL_POST_DIV_MASK)
    }

    pub fn fdiv_ctl1(self) -> u32 {
        (self.k as u32) & ana::video_pll1_fdiv_ctl1::PLL_DSM_MASK
    }

    /// From `FDIV_CTL0` (the field positions are the same for every PLL,
    /// RM 5.1.8.12 / 5.1.8.30 / 5.1.8.33) and the DSM word (0 without one).
    pub fn decode(ctl0: u32, ctl1: u32) -> Divs {
        Divs {
            m: (ctl0 & ana::video_pll1_fdiv_ctl0::PLL_MAIN_DIV_MASK) >> ana::video_pll1_fdiv_ctl0::PLL_MAIN_DIV_SHIFT,
            p: (ctl0 & ana::video_pll1_fdiv_ctl0::PLL_PRE_DIV_MASK) >> ana::video_pll1_fdiv_ctl0::PLL_PRE_DIV_SHIFT,
            s: (ctl0 & ana::video_pll1_fdiv_ctl0::PLL_POST_DIV_MASK) >> ana::video_pll1_fdiv_ctl0::PLL_POST_DIV_SHIFT,
            k: i32::from((ctl1 & ana::video_pll1_fdiv_ctl1::PLL_DSM_MASK) as u16 as i16),
        }
    }
}

/// m and k for VIDEO_PLL1 at `hz` with p = 2, s = 1: the formula solved for
/// `m + k/65536 = hz x p x 2^s / FIN`, rounded to the nearest 1/65536, with a
/// k past +32767 folded into the next m (k is signed).
pub fn video_pll1_divs(hz: u32) -> Result<Divs, i32> {
    let (p, s) = (VIDEO_PLL1_P, VIDEO_PLL1_S);
    let scaled = (u64::from(hz) * u64::from(p << s) * 65536 + FIN_HZ / 2) / FIN_HZ;
    let mut m = (scaled >> 16) as u32;
    let mut k = (scaled & 0xffff) as i32;
    if k > 32767 {
        k -= 65536;
        m += 1;
    }
    let d = Divs { m, p, s, k };
    if hz == 0 || !d.valid() {
        return Err(-EINVAL);
    }
    Ok(d)
}

/// The PLLs this module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pll {
    Video1,
    Sys1,
    Sys2,
}

impl Pll {
    fn regs(self) -> (u32, u32, Option<u32>) {
        match self {
            Pll::Video1 => (ana::VIDEO_PLL1_GEN_CTRL, ana::VIDEO_PLL1_FDIV_CTL0, Some(ana::VIDEO_PLL1_FDIV_CTL1)),
            Pll::Sys1 => (ana::SYS_PLL1_GEN_CTRL, ana::SYS_PLL1_FDIV_CTL0, None),
            Pll::Sys2 => (ana::SYS_PLL2_GEN_CTRL, ana::SYS_PLL2_FDIV_CTL0, None),
        }
    }

    /// The `GEN_CTRL` bits that must read 1 for the main output to run:
    /// LOCK, RST (out of reset), CLKE (output gate open). The positions of
    /// CLKE differ between the fractional and the system PLLs.
    fn running_bits(self) -> u32 {
        match self {
            Pll::Video1 => {
                ana::video_pll1_gen_ctrl::PLL_LOCK_MASK
                    | ana::video_pll1_gen_ctrl::PLL_RST_MASK
                    | ana::video_pll1_gen_ctrl::PLL_CLKE_MASK
            }
            Pll::Sys1 | Pll::Sys2 => {
                ana::sys_pll2_gen_ctrl::PLL_LOCK_MASK
                    | ana::sys_pll2_gen_ctrl::PLL_RST_MASK
                    | ana::sys_pll2_gen_ctrl::PLL_CLKE_MASK
            }
        }
    }

    /// Bits that must read 0: bypasses, and PLL_REF_CLK_SEL (00 = 24M).
    fn off_bits(self) -> u32 {
        match self {
            Pll::Video1 => {
                ana::video_pll1_gen_ctrl::PLL_BYPASS_MASK
                    | ana::video_pll1_gen_ctrl::PLL_EXT_BYPASS_MASK
                    | ana::video_pll1_gen_ctrl::PLL_REF_CLK_SEL_MASK
            }
            Pll::Sys1 | Pll::Sys2 => {
                ana::sys_pll2_gen_ctrl::PLL_BYPASS_MASK
                    | ana::sys_pll2_gen_ctrl::PLL_EXT_BYPASS_MASK
                    | ana::sys_pll2_gen_ctrl::PLL_REF_CLK_SEL_MASK
            }
        }
    }
}

/// The rate a PLL's main output runs at, read from its registers; `None`
/// when it is not running from the 24 MHz reference (locked, out of reset,
/// output gate open, not bypassed) or its dividers are outside the RM's
/// ranges. This is the check the CCM's Target Interface asks for before a
/// root is switched to the PLL (RM 5.1.6.4).
pub fn running_rate(m: &impl Mmio, base: usize, pll: Pll) -> Option<u32> {
    let (gen, ctl0, ctl1) = pll.regs();
    let g = m.read32(base + gen as usize);
    if g & pll.running_bits() != pll.running_bits() || g & pll.off_bits() != 0 {
        return None;
    }
    let d = Divs::decode(m.read32(base + ctl0 as usize), ctl1.map_or(0, |o| m.read32(base + o as usize)));
    if !d.valid() {
        return None;
    }
    u32::try_from(d.rate_hz()).ok()
}

/// What [`video_pll1_set`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Programmed {
    pub divs: Divs,
    /// The PLL was already running at these dividers: nothing written.
    pub unchanged: bool,
}

/// Program VIDEO_PLL1 to `hz` (module text for the order). A PLL already
/// running at the computed dividers is left alone.
pub fn video_pll1_set(m: &impl Mmio, base: usize, env: &impl Env, hz: u32) -> Result<Programmed, i32> {
    use ana::video_pll1_gen_ctrl as g;
    let divs = video_pll1_divs(hz)?;
    let gen = base + ana::VIDEO_PLL1_GEN_CTRL as usize;
    let ctl0 = base + ana::VIDEO_PLL1_FDIV_CTL0 as usize;
    let ctl1 = base + ana::VIDEO_PLL1_FDIV_CTL1 as usize;
    if running_rate(m, base, Pll::Video1).is_some()
        && m.read32(ctl0) == divs.fdiv_ctl0()
        && m.read32(ctl1) & ana::video_pll1_fdiv_ctl1::PLL_DSM_MASK == divs.fdiv_ctl1()
    {
        return Ok(Programmed { divs, unchanged: true });
    }
    // hold in reset, reference 24M, no bypass
    let v = m.read32(gen) & !(g::PLL_RST_MASK | g::PLL_BYPASS_MASK | g::PLL_EXT_BYPASS_MASK | g::PLL_REF_CLK_SEL_MASK);
    m.write32(gen, v & !g::PLL_LOCK_MASK);
    m.write32(ctl0, (m.read32(ctl0) & !(divs_mask())) | divs.fdiv_ctl0());
    m.write32(ctl1, (m.read32(ctl1) & !ana::video_pll1_fdiv_ctl1::PLL_DSM_MASK) | divs.fdiv_ctl1());
    env.busy_wait_us(3);
    m.write32(gen, (m.read32(gen) & !g::PLL_LOCK_MASK) | g::PLL_RST_MASK);
    let mut locked = false;
    for _ in 0..LOCK_POLL_US {
        if m.read32(gen) & g::PLL_LOCK_MASK != 0 {
            locked = true;
            break;
        }
        env.busy_wait_us(1);
    }
    if !locked {
        return Err(-ETIMEDOUT);
    }
    // the output gate, once the PLL runs
    let v = m.read32(gen);
    if v & g::PLL_CLKE_MASK == 0 {
        m.write32(gen, (v & !g::PLL_LOCK_MASK) | g::PLL_CLKE_MASK);
    }
    Ok(Programmed { divs, unchanged: false })
}

fn divs_mask() -> u32 {
    ana::video_pll1_fdiv_ctl0::PLL_MAIN_DIV_MASK
        | ana::video_pll1_fdiv_ctl0::PLL_PRE_DIV_MASK
        | ana::video_pll1_fdiv_ctl0::PLL_POST_DIV_MASK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Faults, Model, TestEnv, ANATOP};

    #[test]
    fn the_formula_on_the_golden_values() {
        // golden-linux-2026-09-21.txt: VIDEO_PLL1 0x000ad021 / 0x4000
        let d = Divs::decode(0x000a_d021, 0x4000);
        assert_eq!(d, Divs { m: 173, p: 2, s: 1, k: 16384 });
        assert_eq!(d.rate_hz(), 1_039_500_000);
        // SYS_PLL1 0x00190032 = 800 MHz (RM 5.1.5.1: SYSTEM_PLL1_CLK 800)
        assert_eq!(Divs::decode(0x0019_0032, 0).rate_hz(), 800_000_000);
        // SYS_PLL2's reset dividers 0x000FA031 = 1000 MHz (RM 5.1.8.33, 5.1.5.1)
        assert_eq!(Divs::decode(0x000f_a031, 0).rate_hz(), 1_000_000_000);
        // a negative k
        assert_eq!(Divs::decode(0x000a_e021, 0xc000).k, -16384);
    }

    #[test]
    fn m_and_k_from_the_formula() {
        let d = video_pll1_divs(1_039_500_000).unwrap();
        assert_eq!(d, Divs { m: 173, p: 2, s: 1, k: 16384 });
        assert_eq!((d.fdiv_ctl0(), d.fdiv_ctl1()), (0x000a_d021, 0x4000));
        // a fraction above one half folds into the next m with a negative k
        let d = video_pll1_divs(1_190_000_000).unwrap(); // RM 5.1.5.1's nominal
        assert_eq!((d.m, d.k), (198, 21845));
        let d = video_pll1_divs(6 * 100_000_000 + 4_500_000).unwrap(); // 100.75 x 6
        assert_eq!((d.m, d.k), (101, -16384));
        assert_eq!(d.rate_hz(), 604_500_000);
        // every rate in the reachable range comes back within 6 MHz / 65536
        for hz in (400_000_000u32..=6_000_000_000u64.min(u64::from(u32::MAX)) as u32).step_by(7_777_777) {
            let d = video_pll1_divs(hz).unwrap();
            assert!(d.valid());
            assert!((d.rate_hz() as i64 - i64::from(hz)).abs() <= 92, "{hz} {d:?}");
        }
        // outside m 64..1023 at p 2 s 1
        assert_eq!(video_pll1_divs(300_000_000), Err(-EINVAL));
        assert_eq!(video_pll1_divs(0), Err(-EINVAL));
    }

    fn running(m: &Model) {
        m.poke(ANATOP, 0x94, 0x8aaa_aa00); // SYS_PLL1 GEN_CTRL as golden
        m.poke(ANATOP, 0x98, 0x0019_0032);
        m.poke(ANATOP, 0x104, 0x8aaa_aa00);
        m.poke(ANATOP, 0x108, 0x000f_a031);
    }

    #[test]
    fn running_rate_checks_lock_reset_gate_bypass() {
        let m = Model::new();
        running(&m);
        let b = m.base(ANATOP);
        assert_eq!(running_rate(&m, b, Pll::Sys1), Some(800_000_000));
        assert_eq!(running_rate(&m, b, Pll::Sys2), Some(1_000_000_000));
        m.poke(ANATOP, 0x104, 0x8aaa_aa10); // bypassed
        assert_eq!(running_rate(&m, b, Pll::Sys2), None);
        m.poke(ANATOP, 0x104, 0x0aaa_aa00); // not locked
        assert_eq!(running_rate(&m, b, Pll::Sys2), None);
        m.poke(ANATOP, 0x104, 0x8aaa_a200); // output gate closed
        assert_eq!(running_rate(&m, b, Pll::Sys2), None);
        // VIDEO_PLL1 at its reset value: bypassed, in reset
        m.poke(ANATOP, 0x28, 0x0000_2010);
        assert_eq!(running_rate(&m, b, Pll::Video1), None);
    }

    #[test]
    fn video_pll1_from_reset_holds_writes_releases_locks_opens() {
        let m = Model::new();
        let e = TestEnv::default();
        let b = m.base(ANATOP);
        m.poke(ANATOP, 0x28, 0x0000_2010); // RM 5.1.8.11 reset: CLKE, BYPASS
        m.poke(ANATOP, 0x2c, 0x0014_5032);
        let r = video_pll1_set(&m, b, &e, 1_039_500_000).unwrap();
        assert!(!r.unchanged);
        assert_eq!(m.peek(ANATOP, 0x28), 0x8000_2200, "golden GEN_CTRL");
        assert_eq!(m.peek(ANATOP, 0x2c), 0x000a_d021);
        assert_eq!(m.peek(ANATOP, 0x30), 0x4000);
        let w = m.writes();
        assert_eq!(w[0], (ANATOP, 0x28, 0x0000_2000), "reset held, bypass cleared first");
        assert_eq!(w.iter().filter(|x| x.1 == 0x28).nth(1), Some(&(ANATOP, 0x28, 0x0000_2200)));
        assert_eq!(running_rate(&m, b, Pll::Video1), Some(1_039_500_000));
        // a second call writes nothing
        m.clear_log();
        assert!(video_pll1_set(&m, b, &e, 1_039_500_000).unwrap().unchanged);
        assert!(m.writes().is_empty());
    }

    #[test]
    fn no_lock_is_a_timeout_not_a_hang() {
        let m = Model::with_faults(Faults { pll_no_lock: true, ..Default::default() });
        let e = TestEnv::default();
        assert_eq!(video_pll1_set(&m, m.base(ANATOP), &e, 1_039_500_000), Err(-ETIMEDOUT));
        assert!(*e.waited_us.borrow() >= u64::from(LOCK_POLL_US));
    }
}
