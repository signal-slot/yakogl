// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Cadence MIPI D-PHY transmitter with TI's WIZ wrapper (the AM62P's
//! DPHY_TX0, 0x301C_0000): PLL dividers, band, power-up.
//!
//! Written from the AM62Px TRM, TI SPRUJ83D (December 2023, revised January
//! 2026): 12.6.3 "MIPI D-PHY Transmitter", the register chapter 14.8.6.3
//! ([`crate::soc::am62p::regs::dphy`], [`crate::soc::am62p::regs::wiz`]) and
//! the DSI programming guide 12.9.2.7. What the TRM states and the code
//! relies on:
//!
//! - PLL (14.8.6.3.2.13 PLL_CTRL): IPDIV 1 / 2 / 4 / 8 for a reference of
//!   9.6-<19.2 / 19.2-<38.4 / 38.4-<76.8 / 76.8-<150 MHz; OPDIV 1 / 2 / 4 /
//!   8 / 16 for 2.5-1.25 Gbps / 1.24 Gbps-630 Mbps / 620-320 / 310-160 /
//!   150-80 Mbps (rates between those ranges are not covered and are
//!   refused); "Feedback divider value = ROUND ((Data Rate * 2 * pll_opdiv *
//!   pll_ipdiv) / PLL reference clock frequency)", FBDIV 10 bits; PLL_PD
//!   (reset 1) 0 = active, PLL_PSO 0 = power on; PLL_LOCK reads 1 when
//!   locked. The rounded divider may give a rate below the one asked for;
//!   the DSI host can absorb a byte clock faster than the pixel stream (by
//!   the HFP, 12.9.2.7.8.2) but not a slower one, so then the next divider
//!   up is taken.
//! - PSM clock (14.8.6.3.2.16 PSM_FREQ): "must be driven with a value such
//!   that the internal psm frequency of the divided psm clock is 1 MHz".
//! - Reset (14.8.6.3.2.15 RST_CTRL LANE_RSTB_CMN): "required to be released
//!   after APB register programming".
//! - Start (14.8.6.3.2.1 CMN_DIG_TBIT2): CMN0_O_CMN_SSM_EN "Enable CMN
//!   startup state machine", CMN0_O_CMN_TX_MODE_EN "Enable CMN TX related
//!   StateMachines"; the bandgap wait field (8:1) is kept as it is.
//! - Ready (14.8.6.3.2.14 STATUS O_CMN_READY): "System Should check this
//!   during Power up Initialisation". The PLL lock time is "defined by the
//!   PHY documentation" (12.9.2.7.3), which is not public: the waits here are
//!   generous bounds.
//! - Rates: up to 2.5 Gbps per lane (12.6.3.1.1), 80 Mbps at the bottom of
//!   the OPDIV table.
//!
//! What the TRM does not state, taken from TI's Zephyr driver of PR #114866
//! (vendored unmodified, third_party/zephyr-pr114866-2026-07-29-2473da6b/;
//! the values are generated into [`crate::soc::am62p::ti_pr114866`]):
//!
//! - the band table of PCS_TX_DIG_TBIT0 (the TRM names band 0, "Data Rate
//!   [80_100] MHz", only): [`band`];
//! - the PLL gm PWM divider values for CMN_DIG_TBIT10 (the TRM gives the
//!   fields only);
//! - where the WIZ registers are: the TRM's DPHY_TX summary lists them at
//!   offsets 0h-14h, TI's driver at 0xF00-0xF10. [`find_wiz`] looks for the
//!   wrapper's MOD_VER MODULE_ID (992h, "WIZ16B8M4CDT3 module ID", a TRM
//!   reset value) at TI's offset first, then at the TRM's, and refuses a
//!   region that has it at neither.

use crate::env::Env;
use crate::errno::{EINVAL, ENODEV, ETIMEDOUT};
use crate::mmio::{field, get, Mmio};
use crate::soc::am62p::regs::{dphy as r, wiz};
use crate::soc::am62p::ti_pr114866 as ti;
use crate::ylog;

/// Waits for PLL lock and CMN ready: 1 ms each in 10 us steps (the C driver
/// and TI's both allow 1 ms; the TRM gives no number).
pub const LOCK_WAIT_US: u32 = 1_000;
const POLL_STEP_US: u32 = 10;

/// The PLL setting for one rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pll {
    pub ipdiv: u32,
    pub opdiv: u32,
    pub fbdiv: u32,
    /// The rate it gives, bit/s per lane.
    pub rate: u64,
    /// The PCS band of that rate.
    pub band: u32,
}

/// IPDIV for a reference clock (14.8.6.3.2.13).
pub fn ipdiv_for(ref_hz: u64) -> Option<u32> {
    match ref_hz {
        9_600_000..=19_199_999 => Some(1),
        19_200_000..=38_399_999 => Some(2),
        38_400_000..=76_799_999 => Some(4),
        76_800_000..=149_999_999 => Some(8),
        _ => None,
    }
}

/// OPDIV for a lane rate (14.8.6.3.2.13); None in the gaps between ranges.
pub fn opdiv_for(rate: u64) -> Option<u32> {
    match rate {
        1_250_000_000..=2_500_000_000 => Some(1),
        630_000_000..=1_240_000_000 => Some(2),
        320_000_000..=620_000_000 => Some(4),
        160_000_000..=310_000_000 => Some(8),
        80_000_000..=150_000_000 => Some(16),
        _ => None,
    }
}

/// The PCS band of a lane rate (vendored table; see the module text): band
/// `i` covers `[TX_BANDS_MBPS[i], TX_BANDS_MBPS[i + 1])` Mbit/s, the last
/// band its edge and above, as TI's `dphy_get_band_ctrl()` decides.
pub fn band(rate: u64) -> Option<u32> {
    let mbps = rate / 1_000_000;
    let t = &ti::TX_BANDS_MBPS;
    if mbps < u64::from(t[0]) {
        return None;
    }
    let i = t.windows(2).position(|w| mbps >= u64::from(w[0]) && mbps < u64::from(w[1]));
    Some(i.unwrap_or(t.len() - 1) as u32)
}

/// The PLL setting for `rate` bit/s per lane from a `ref_hz` reference.
pub fn pll_for(ref_hz: u64, rate: u64) -> Result<Pll, i32> {
    let ipdiv = ipdiv_for(ref_hz).ok_or(-EINVAL)?;
    let opdiv = opdiv_for(rate).ok_or(-EINVAL)?;
    let num = rate * 2 * u64::from(opdiv) * u64::from(ipdiv);
    let mut fbdiv = (num + ref_hz / 2) / ref_hz;
    let out = |fb: u64| ref_hz * fb / (2 * u64::from(opdiv) * u64::from(ipdiv));
    if out(fbdiv) < rate {
        fbdiv += 1;
    }
    let fbmax = u64::from(get(wiz::PLL_FBDIV, wiz::PLL_FBDIV));
    if fbdiv == 0 || fbdiv > fbmax {
        return Err(-EINVAL);
    }
    let rate_out = out(fbdiv);
    let band = band(rate_out).ok_or(-EINVAL)?;
    Ok(Pll { ipdiv, opdiv, fbdiv: fbdiv as u32, rate: rate_out, band })
}

/// PSM_CLOCK_FREQ for a PSM clock: the divider to 1 MHz, to the nearest.
pub fn psm_divider(psm_hz: u32) -> Result<u32, i32> {
    let div = (psm_hz + 500_000) / 1_000_000;
    if div == 0 || div > get(wiz::PSM_CLOCK_FREQ, wiz::PSM_CLOCK_FREQ) {
        return Err(-EINVAL);
    }
    Ok(div)
}

fn rd(m: &impl Mmio, base: usize, off: u32) -> u32 {
    m.read32(base + off as usize)
}

fn wr(m: &impl Mmio, base: usize, off: u32, val: u32) {
    m.write32(base + off as usize, val)
}

/// Where the WIZ wrapper's registers start inside the D-PHY region, found
/// by its MOD_VER: TI's driver's offset first, then the TRM summary's.
pub fn find_wiz(m: &impl Mmio, dphy: usize) -> Option<u32> {
    let ti_base = ti::WIZ_PLL_CTRL - wiz::PLL_CTRL;
    [ti_base, 0]
        .into_iter()
        .find(|&b| get(wiz::MODULE_ID, rd(m, dphy, b + wiz::MOD_VER)) == wiz::MODULE_ID_WIZ16B8M4CDT3)
}

fn wait(m: &impl Mmio, env: &impl Env, base: usize, off: u32, mask: u32) -> bool {
    let mut t = 0;
    loop {
        if rd(m, base, off) & mask == mask {
            return true;
        }
        if t >= LOCK_WAIT_US {
            return false;
        }
        env.busy_wait_us(POLL_STEP_US);
        t += POLL_STEP_US;
    }
}

/// Program and start the PHY for `pll` with a `psm_hz` PSM clock; `wiz` is
/// [`find_wiz`]'s base. Order: PSM divider, PWM dividers, PLL dividers (PLL
/// powered), then the common reset released "after APB register
/// programming", the band, the start state machines; then PLL lock and CMN
/// ready.
pub fn power_on(m: &impl Mmio, env: &impl Env, dphy: usize, wiz_base: u32, pll: &Pll, psm_hz: u32) -> Result<(), i32> {
    let psm = psm_divider(psm_hz)?;
    wr(m, dphy, wiz_base + wiz::PSM_FREQ, field(wiz::PSM_CLOCK_FREQ, psm));
    wr(
        m,
        dphy,
        r::CMN_DIG_TBIT10,
        field(r::PLL_GM_PWM_DIV_HIGH, ti::CMN_PWM_DIV_HIGH)
            | field(r::PLL_GM_PWM_DIV_LOW, ti::CMN_PWM_DIV_LOW)
            | field(r::PLL_BYTECLK_DIV, ti::CMN_PWM_BYTECLK_DIV),
    );
    // PLL_PD and PLL_PSO 0: the PLL powered and active.
    wr(
        m,
        dphy,
        wiz_base + wiz::PLL_CTRL,
        field(wiz::PLL_IPDIV, pll.ipdiv) | field(wiz::PLL_OPDIV, pll.opdiv) | field(wiz::PLL_FBDIV, pll.fbdiv),
    );
    wr(m, dphy, wiz_base + wiz::RST_CTRL, wiz::LANE_RSTB_CMN);
    wr(m, dphy, r::PCS_TX_DIG_TBIT0, field(r::PCS_BAND_CTL_REG_L, pll.band) | field(r::PCS_BAND_CTL_REG_R, pll.band));
    m.modify32(dphy + r::CMN_DIG_TBIT2 as usize, r::CMN_SSM_EN | r::CMN_TX_MODE_EN, r::CMN_SSM_EN | r::CMN_TX_MODE_EN);
    if !wait(m, env, dphy, wiz_base + wiz::PLL_CTRL, wiz::PLL_LOCK) {
        ylog!(
            env,
            Err,
            "D-PHY PLL did not lock (ipdiv {} opdiv {} fbdiv {}, PLL_CTRL {:#010x})",
            pll.ipdiv,
            pll.opdiv,
            pll.fbdiv,
            rd(m, dphy, wiz_base + wiz::PLL_CTRL)
        );
        return Err(-ETIMEDOUT);
    }
    if !wait(m, env, dphy, wiz_base + wiz::STATUS, wiz::O_CMN_READY) {
        ylog!(env, Err, "D-PHY O_CMN_READY not set (STATUS {:#010x})", rd(m, dphy, wiz_base + wiz::STATUS));
        return Err(-ETIMEDOUT);
    }
    ylog!(env, Inf, "D-PHY: {} bit/s per lane, band {}", pll.rate, pll.band);
    Ok(())
}

/// The PHY found at `dphy`, or -ENODEV.
pub fn probe(m: &impl Mmio, env: &impl Env, dphy: usize) -> Result<u32, i32> {
    match find_wiz(m, dphy) {
        Some(b) => {
            ylog!(env, Inf, "D-PHY: WIZ at +{:#x} (MOD_VER {:#010x})", b, rd(m, dphy, b + wiz::MOD_VER));
            Ok(b)
        }
        None => {
            ylog!(
                env,
                Err,
                "D-PHY: no WIZ MOD_VER at {:#x} or 0 ({:#010x}, {:#010x})",
                ti::WIZ_PLL_CTRL - wiz::PLL_CTRL,
                rd(m, dphy, ti::WIZ_PLL_CTRL - wiz::PLL_CTRL),
                rd(m, dphy, 0)
            );
            Err(-ENODEV)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from driver/tests/display_calc (test_cdns_dphy_pll_25mhz_ref):
    /// the RVT70's 307.2 Mbit/s per lane from the 25 MHz reference.
    #[test]
    fn pll_rvt70_matches_display_calc() {
        let p = pll_for(25_000_000, 307_200_000).unwrap();
        assert_eq!((p.ipdiv, p.opdiv), (2, 8));
        // ROUND(307.2e6 * 32 / 25e6) = ROUND(393.216) = 393 gives 307.03
        // Mbit/s, below the request: the next divider, 394, as display_calc
        // (which rounded up) had it.
        assert_eq!(p.fbdiv, 394);
        assert_eq!(p.rate, 25_000_000 * 394 / 32);
        assert_eq!(p.band, 5); // 240..320 Mbit/s
        assert_eq!(pll_for(25_000_000, 50_000_000), Err(-EINVAL));
    }

    #[test]
    fn pll_rounds_as_the_trm_says_when_that_is_fast_enough() {
        // 300 Mbit/s: 300e6 * 32 / 25e6 = 384 exactly.
        let p = pll_for(25_000_000, 300_000_000).unwrap();
        assert_eq!((p.fbdiv, p.rate), (384, 300_000_000));
        // 301 Mbit/s: 385.28 rounds to 385 -> 300.78 < 301: 386.
        assert_eq!(pll_for(25_000_000, 301_000_000).unwrap().fbdiv, 386);
        // 300.5 Mbit/s: 384.64 rounds to 385 -> 300.78 >= 300.5: 385.
        assert_eq!(pll_for(25_000_000, 300_500_000).unwrap().fbdiv, 385);
    }

    #[test]
    fn divider_ranges_follow_the_trm_table() {
        assert_eq!(ipdiv_for(9_599_999), None);
        assert_eq!(ipdiv_for(9_600_000), Some(1));
        assert_eq!(ipdiv_for(19_200_000), Some(2));
        assert_eq!(ipdiv_for(25_000_000), Some(2));
        assert_eq!(ipdiv_for(76_800_000), Some(8));
        assert_eq!(ipdiv_for(150_000_000), None);
        assert_eq!(opdiv_for(2_500_000_000), Some(1));
        assert_eq!(opdiv_for(1_245_000_000), None); // between 1.24 G and 1.25 G
        assert_eq!(opdiv_for(625_000_000), None);
        assert_eq!(opdiv_for(315_000_000), None);
        assert_eq!(opdiv_for(155_000_000), None);
        assert_eq!(opdiv_for(80_000_000), Some(16));
        assert_eq!(opdiv_for(79_999_999), None);
        assert_eq!(pll_for(25_000_000, 315_000_000), Err(-EINVAL));
        assert_eq!(pll_for(5_000_000, 300_000_000), Err(-EINVAL));
    }

    #[test]
    fn bands_from_the_vendored_table() {
        assert_eq!(band(79_999_999), None);
        assert_eq!(band(80_000_000), Some(0));
        assert_eq!(band(99_999_999), Some(0));
        assert_eq!(band(100_000_000), Some(1));
        assert_eq!(band(307_812_500), Some(5));
        assert_eq!(band(2_499_000_000), Some(22));
        assert_eq!(band(2_500_000_000), Some(23));
    }

    #[test]
    fn psm_divider_is_to_one_mhz() {
        assert_eq!(psm_divider(25_000_000), Ok(25));
        assert_eq!(psm_divider(19_200_000), Ok(19));
        assert_eq!(psm_divider(400_000), Err(-EINVAL));
        assert_eq!(psm_divider(300_000_000), Err(-EINVAL));
    }
}
