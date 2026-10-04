// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 8M Plus display power domains: MEDIAMIX with its clock roots and
//! block control, and the two sub-domains a display needs, LCDIF1 and
//! MIPI_DSI1 (the latter with the MIPI_PHY1 power domain). Built from
//! [`gpc`], [`ccm`], [`anatop`] and [`media_blk`]; the references are theirs
//! (IMX8MPRM Rev. 3, 08/2024).
//!
//! MEDIAMIX on ([`pd_on`], steps as recorded in the status word):
//! 1. SYS_PLL2 and SYS_PLL1 run (read back, [`anatop::running_rate`]: the
//!    CCM Target Interface requires an active source, RM 5.1.6.4), then
//!    MEDIA_AXI_CLK_ROOT from SYSTEM_PLL2_CLK and MEDIA_APB_CLK_ROOT from
//!    SYSTEM_PLL1_CLK, each at the fastest rate not above its Table 5-1
//!    maximum (500 / 200 MHz): /2 and /4 for 1000 / 800 MHz;
//! 2. the MEDIA clock gate CCGR93 (Table 5-9), after its roots (RM 5.1.5.5);
//! 3. the MEDIAMIX PU domain ([`gpc::power_up`]);
//! 4. the block control's bus clock and reset (`SFT_EN_BUS_BLK_CLK`) -- the
//!    first access to MEDIA_BLK_CTRL, after its clock root and gate (its
//!    registers are on media_apb: RM 5.1.4 lists the MEDIAMIX APB clocks on
//!    MEDIA_APB_CLK_ROOT behind CCGR93) and after the power-up;
//! 5. the MEDIAMIX ADB400 acknowledge ([`gpc::adb_wait_up`]);
//! 6. the LCDIF read-hurry ([`media_blk::set_rd_hurry`]).
//!
//! Then per sub-domain: 7. MIPI_PHY1 powered (MIPI_DSI1 only); 8. its block
//! clocks on and resets released ([`media_blk::enable`]); 9. done.
//! [`pd_reset`] (10) pulses a sub-domain's resets once its clocks are set;
//! [`pd_off`] (11) is the reverse, and the last sub-domain off powers
//! MEDIAMIX down. 12 is VIDEO_PLL1, 13 the pixel clock root, 14 the PHY
//! reference root.
//!
//! A domain whose state says it is on is not touched again, so a chain the
//! previous kernel left running is adopted without a write: MEDIAMIX counts
//! as on when it is mapped to the A53, no power-up trigger is pending or
//! failed and the ADB400 has acknowledged (the NoC bridge into the domain is
//! up); MIPI_PHY1 when it is mapped, clean and its DSI consumer is clocked
//! and out of reset (this driver gates and resets the DSI before it powers
//! the PHY down). These are inferences from the RM's status bits (the GPC has
//! no "domain is powered" bit), not RM statements. PSR is not used for it:
//! it keeps reporting an earlier power-down until software clears it
//! (5.2.11.12), which the previous kernel need not have done.

use super::anatop::{self, Pll};
use super::ccm::{self, gate, mux, root, RootSetting};
use super::gpc::{self, PuDomain};
use super::media_blk;
use crate::env::Env;
use crate::errno::{EINVAL, EIO, ETIMEDOUT};
use crate::mmio::Mmio;

/// The four register blocks, as the shim mapped them.
#[derive(Clone, Copy, Debug)]
pub struct Bases {
    pub blk: usize,
    pub gpc: usize,
    pub ccm: usize,
    pub anatop: usize,
}

/// A sub-domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Pd {
    Lcdif1 = 0,
    MipiDsi1 = 1,
}

impl Pd {
    pub fn from_u32(v: u32) -> Option<Pd> {
        match v {
            0 => Some(Pd::Lcdif1),
            1 => Some(Pd::MipiDsi1),
            _ => None,
        }
    }

    fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// The steps of the module text.
pub mod step {
    pub const BUS_ROOTS: u32 = 1;
    pub const GATE: u32 = 2;
    pub const MEDIAMIX: u32 = 3;
    pub const BLK_BUS: u32 = 4;
    pub const ADB_ACK: u32 = 5;
    pub const RD_HURRY: u32 = 6;
    pub const MIPI_PHY1: u32 = 7;
    pub const BLK_BLOCK: u32 = 8;
    pub const ON: u32 = 9;
    pub const RESET: u32 = 10;
    pub const OFF: u32 = 11;
    pub const VIDEO_PLL1: u32 = 12;
    pub const PIXEL: u32 = 13;
    pub const PHY_REF: u32 = 14;
}

/// The status word ([`status`]): what the hardware says, for a record that
/// has to carry it home without a console.
pub mod st {
    pub const MEDIAMIX_MAPPED: u32 = 1 << 0;
    pub const MIPI_PHY1_MAPPED: u32 = 1 << 1;
    /// No MEDIAMIX power-up trigger pending or failed (5.2.10.37 / .45).
    pub const MEDIAMIX_PUP_CLEAN: u32 = 1 << 2;
    pub const MIPI_PHY1_PUP_CLEAN: u32 = 1 << 3;
    pub const ADB_REQ: u32 = 1 << 4;
    pub const ADB_ACK: u32 = 1 << 5;
    /// PCR clear on both PGCs.
    pub const PCR_CLEAR: u32 = 1 << 6;
    /// This driver found MEDIAMIX powered down (PSR) and powered it up.
    pub const POWERED_FROM_OFF: u32 = 1 << 7;
    pub const MEDIA_AXI_ROOT: u32 = 1 << 8;
    pub const MEDIA_APB_ROOT: u32 = 1 << 9;
    pub const MEDIA_GATE: u32 = 1 << 10;
    pub const DISP1_PIX_ROOT: u32 = 1 << 11;
    pub const PHY1_REF_ROOT: u32 = 1 << 12;
    pub const VIDEO_PLL1_RUNNING: u32 = 1 << 13;
    pub const SYS_PLL1_RUNNING: u32 = 1 << 14;
    pub const SYS_PLL2_RUNNING: u32 = 1 << 15;
    /// Bits 16..21: MEDIA_BLK_CTRL as last sampled (only read while
    /// MEDIAMIX is known to be up): bus, LCDIF1, MIPI_DSI1 clocked and out of
    /// reset, the D-PHY out of reset, the read-hurry, and "sampled".
    pub const BLK_BUS: u32 = 1 << 16;
    pub const BLK_LCDIF1: u32 = 1 << 17;
    pub const BLK_MIPI_DSI1: u32 = 1 << 18;
    pub const BLK_PHY: u32 = 1 << 19;
    pub const BLK_HURRY: u32 = 1 << 20;
    pub const BLK_SAMPLED: u32 = 1 << 21;
    pub const BLK_MASK: u32 = 0x3f << 16;
    /// Bits 24..31: the last operation, step in 27:24, failed in 28, the
    /// error in 31:29 (1 timeout, 2 I/O, 3 invalid, 4 other).
    pub const LAST_SHIFT: u32 = 24;
}

/// What a device remembers between calls (in its `data`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Track {
    /// Sub-domains on, [`Pd`] bits.
    pub users: u32,
    /// [`st::BLK_MASK`] bits of the last sample, plus [`st::POWERED_FROM_OFF`].
    pub sampled: u32,
    /// Bits 31:24 of the status word.
    pub last: u32,
}

fn errclass(e: i32) -> u32 {
    match -e {
        ETIMEDOUT => 1,
        EIO => 2,
        EINVAL => 3,
        _ => 4,
    }
}

impl Track {
    fn done(&mut self, step: u32) {
        self.last = step & 0xf;
    }

    fn fail(&mut self, step: u32, e: i32) -> i32 {
        self.last = (step & 0xf) | 1 << 4 | errclass(e) << 5;
        e
    }
}

fn mediamix_on(m: &impl Mmio, b: &Bases) -> bool {
    mediamix_on_at(m, b.gpc)
}

/// MEDIAMIX is on (the module text's inference): reads the GPC only.
fn mediamix_on_at(m: &impl Mmio, gpc_base: usize) -> bool {
    let s = gpc::state(m, gpc_base, &gpc::MEDIAMIX);
    s.mapped && s.pup_clean && s.adb_req && s.adb_ack
}

/// Why [`lcdif_unreachable`] says an LCDIF1 register access would not be
/// answered (bits; 0 = it would).
pub mod unreach {
    /// MEDIAMIX is not on (GPC: mapping, trigger, ADB400 request and ack).
    pub const MEDIAMIX: u32 = 1 << 0;
    /// MEDIA_APB_CLK_ROOT is disabled.
    pub const APB_ROOT: u32 = 1 << 1;
    /// The MEDIA gate (CCGR93) is closed for the A53 domain.
    pub const GATE: u32 = 1 << 2;
    /// MEDIA_BLK_CTRL has the LCDIF APB clock off or its reset asserted.
    pub const LCDIF_APB: u32 = 1 << 3;
}

/// Whether the LCDIF1 registers can be read now without hanging the bus,
/// and if not, why ([`unreach`] bits; 0 = they can). A peripheral inside a
/// powered-down or unclocked MEDIAMIX does not answer: the access stalls the
/// core, and only a watchdog ends it (the 2026-10-03 netboot, kind STAGE
/// 0x13: the panel console's first LCDIF read on a U-Boot entry).
///
/// The order is the order of reachability: the GPC first (always
/// reachable), then the CCM (always reachable), and MEDIA_BLK_CTRL only once
/// those say it answers (its registers are on MEDIA_APB_CLK_ROOT behind
/// CCGR93 inside MEDIAMIX: the module text, step 4). Nothing is written.
/// The LCDIF needs MEDIAMIX, its bus clock and its own APB clock and reset
/// in MEDIA_BLK_CTRL (`SFT_EN_LCDIF_APB_CLK`): the register model's rule
/// too ([`crate::model`], `check_media_ip`).
pub fn lcdif_unreachable(m: &impl Mmio, gpc_base: usize, ccm_base: usize, blk_base: usize) -> u32 {
    let mut why = 0;
    if !mediamix_on_at(m, gpc_base) {
        why |= unreach::MEDIAMIX;
    }
    if !ccm::root_setting(m, ccm_base, root::MEDIA_APB).enabled {
        why |= unreach::APB_ROOT;
    }
    if !ccm::gate_runs(m, ccm_base, gate::MEDIA) {
        why |= unreach::GATE;
    }
    if why != 0 {
        return why;
    }
    if !media_blk::state(m, blk_base).on(media_blk::LCDIF1_APB) {
        why |= unreach::LCDIF_APB;
    }
    why
}

fn sample(m: &impl Mmio, b: &Bases, t: &mut Track) {
    let s = media_blk::state(m, b.blk);
    let mut v = st::BLK_SAMPLED;
    if s.on(media_blk::BUS) {
        v |= st::BLK_BUS;
    }
    if s.on(media_blk::LCDIF1) {
        v |= st::BLK_LCDIF1;
    }
    if s.on(media_blk::MIPI_DSI1) {
        v |= st::BLK_MIPI_DSI1;
    }
    if s.mipi_reset_div & media_blk::MIPI_M_RESETN != 0 {
        v |= st::BLK_PHY;
    }
    if s.hurry_set() {
        v |= st::BLK_HURRY;
    }
    t.sampled = (t.sampled & !st::BLK_MASK) | v;
}

/// Divide `src` down to at most `max` with the post divider only.
fn post_for(src: u32, max: u32) -> u32 {
    src.div_ceil(max).clamp(1, 64)
}

fn media_on(m: &impl Mmio, b: &Bases, env: &impl Env, t: &mut Track) -> Result<(), i32> {
    let (Some(sys1), Some(sys2)) =
        (anatop::running_rate(m, b.anatop, Pll::Sys1), anatop::running_rate(m, b.anatop, Pll::Sys2))
    else {
        crate::ylog!(env, Err, "media: SYS_PLL1/SYS_PLL2 not running; the media bus roots have no source");
        return Err(t.fail(step::BUS_ROOTS, -EIO));
    };
    let axi = RootSetting::on(mux::MEDIA_AXI_SYS_PLL2, 1, post_for(sys2, root::MEDIA_AXI_MAX_HZ));
    let apb = RootSetting::on(mux::MEDIA_APB_SYS_PLL1, 1, post_for(sys1, root::MEDIA_APB_MAX_HZ));
    ccm::set_root(m, b.ccm, root::MEDIA_AXI, axi).map_err(|e| t.fail(step::BUS_ROOTS, e))?;
    ccm::set_root(m, b.ccm, root::MEDIA_APB, apb).map_err(|e| t.fail(step::BUS_ROOTS, e))?;
    ccm::gate_on(m, b.ccm, gate::MEDIA).map_err(|e| t.fail(step::GATE, e))?;
    if !mediamix_on(m, b) {
        let up = gpc::power_up(m, b.gpc, env, &gpc::MEDIAMIX).map_err(|f| {
            crate::ylog!(env, Err, "media: MEDIAMIX power-up failed: {:?}", f);
            t.fail(step::MEDIAMIX, f.errno())
        })?;
        if up.was_down {
            t.sampled |= st::POWERED_FROM_OFF;
        }
        crate::ylog!(env, Inf, "media: MEDIAMIX powered up{}", if up.was_down { " from off" } else { "" });
    }
    media_blk::enable(m, b.blk, env, media_blk::BUS, false);
    t.done(step::BLK_BUS);
    gpc::adb_wait_up(m, b.gpc, env, &gpc::MEDIAMIX).map_err(|f| {
        crate::ylog!(env, Err, "media: MEDIAMIX ADB400 did not acknowledge");
        t.fail(step::ADB_ACK, f.errno())
    })?;
    media_blk::set_rd_hurry(m, b.blk);
    t.done(step::RD_HURRY);
    Ok(())
}

fn phy_on(m: &impl Mmio, b: &Bases) -> bool {
    let s = gpc::state(m, b.gpc, &gpc::MIPI_PHY1);
    let blk = media_blk::state(m, b.blk);
    s.mapped && s.pup_clean && blk.on(media_blk::MIPI_DSI1) && blk.mipi_reset_div & media_blk::MIPI_M_RESETN != 0
}

/// Turn a sub-domain on (module text).
pub fn pd_on(m: &impl Mmio, b: &Bases, env: &impl Env, t: &mut Track, pd: Pd) -> Result<(), i32> {
    media_on(m, b, env, t)?;
    match pd {
        Pd::Lcdif1 => {
            media_blk::enable(m, b.blk, env, media_blk::LCDIF1, false);
        }
        Pd::MipiDsi1 => {
            if !phy_on(m, b) {
                gpc::power_up(m, b.gpc, env, &gpc::MIPI_PHY1).map_err(|f| {
                    crate::ylog!(env, Err, "media: MIPI_PHY1 power-up failed: {:?}", f);
                    t.fail(step::MIPI_PHY1, f.errno())
                })?;
            }
            media_blk::enable(m, b.blk, env, media_blk::MIPI_DSI1, true);
        }
    }
    t.users |= pd.bit();
    sample(m, b, t);
    t.done(step::ON);
    Ok(())
}

/// Pulse a sub-domain's resets with its clocks on (MEDIAMIX must be on).
pub fn pd_reset(m: &impl Mmio, b: &Bases, env: &impl Env, t: &mut Track, pd: Pd) -> Result<(), i32> {
    if !mediamix_on(m, b) {
        return Err(t.fail(step::RESET, -EIO));
    }
    match pd {
        Pd::Lcdif1 => media_blk::pulse(m, b.blk, env, media_blk::LCDIF1, false),
        Pd::MipiDsi1 => media_blk::pulse(m, b.blk, env, media_blk::MIPI_DSI1, true),
    }
    sample(m, b, t);
    t.done(step::RESET);
    Ok(())
}

/// Turn a sub-domain off; the last one off powers MEDIAMIX down.
pub fn pd_off(m: &impl Mmio, b: &Bases, env: &impl Env, t: &mut Track, pd: Pd) -> Result<(), i32> {
    let up = mediamix_on(m, b);
    if up {
        match pd {
            Pd::Lcdif1 => media_blk::disable(m, b.blk, media_blk::LCDIF1, false),
            Pd::MipiDsi1 => media_blk::disable(m, b.blk, media_blk::MIPI_DSI1, true),
        }
    }
    if pd == Pd::MipiDsi1 {
        gpc::power_down(m, b.gpc, env, &gpc::MIPI_PHY1).map_err(|f| t.fail(step::OFF, f.errno()))?;
    }
    t.users &= !pd.bit();
    if up && t.users == 0 {
        gpc::power_down(m, b.gpc, env, &gpc::MEDIAMIX).map_err(|f| {
            crate::ylog!(env, Err, "media: MEDIAMIX power-down failed: {:?}", f);
            t.fail(step::OFF, f.errno())
        })?;
        t.sampled &= !st::BLK_MASK;
        crate::ylog!(env, Inf, "media: MEDIAMIX powered down");
    } else if up {
        sample(m, b, t);
    }
    t.done(step::OFF);
    Ok(())
}

/// The pixel clock root setting nearest to `want` from `parents` (mux, Hz),
/// at most `max`; on a tie the earlier parent and the smaller pre divider
/// win.
pub fn plan_root(want: u32, parents: &[(u32, u32)], max: u32) -> Option<(RootSetting, u32)> {
    let mut best: Option<(RootSetting, u32, u32)> = None;
    for &(mx, hz) in parents {
        for pre in 1..=8u32 {
            for post in 1..=64u32 {
                let out = hz / (pre * post);
                if out > max || out == 0 {
                    continue;
                }
                let delta = out.abs_diff(want);
                if best.is_none_or(|b| delta < b.2) {
                    best = Some((RootSetting::on(mx, pre, post), out, delta));
                }
            }
        }
    }
    best.map(|b| (b.0, b.1))
}

/// The pixel clock root's candidate parents that are running, in order of
/// preference (RM Table 5-1, slice 124): VIDEO_PLL1, SYSTEM_PLL2_CLK,
/// SYSTEM_PLL1_CLK, 24M_REF_CLK.
fn pix_parents(m: &impl Mmio, b: &Bases) -> ([(u32, u32); 4], usize) {
    let mut out = [(0, 0); 4];
    let mut n = 0;
    for (mx, rate) in [
        (mux::DISP1_PIX_VIDEO_PLL1, anatop::running_rate(m, b.anatop, Pll::Video1)),
        (mux::DISP1_PIX_SYS_PLL2, anatop::running_rate(m, b.anatop, Pll::Sys2)),
        (mux::DISP1_PIX_SYS_PLL1, anatop::running_rate(m, b.anatop, Pll::Sys1)),
        (mux::OSC_24M, Some(ccm::OSC_24M_HZ)),
    ] {
        if let Some(hz) = rate {
            out[n] = (mx, hz);
            n += 1;
        }
    }
    (out, n)
}

/// Program VIDEO_PLL1 to `video_pll1_hz` (0: leave it) and the pixel clock
/// root to the rate nearest `want`. Returns the rate.
pub fn set_pixel_clock(
    m: &impl Mmio,
    b: &Bases,
    env: &impl Env,
    t: &mut Track,
    want: u32,
    video_pll1_hz: u32,
) -> Result<u32, i32> {
    if video_pll1_hz != 0 {
        match anatop::video_pll1_set(m, b.anatop, env, video_pll1_hz) {
            Ok(p) => {
                let d = p.divs;
                crate::ylog!(
                    env,
                    Inf,
                    "media: VIDEO_PLL1 {} Hz (m {} p {} s {} k {}){}",
                    d.rate_hz(),
                    d.m,
                    d.p,
                    d.s,
                    d.k,
                    if p.unchanged { ", already running" } else { ", locked" }
                );
                t.done(step::VIDEO_PLL1);
            }
            Err(e) => {
                crate::ylog!(env, Err, "media: VIDEO_PLL1 to {} Hz failed ({})", video_pll1_hz, e);
                return Err(t.fail(step::VIDEO_PLL1, e));
            }
        }
    }
    let (parents, n) = pix_parents(m, b);
    let Some((setting, hz)) = plan_root(want, &parents[..n], root::MEDIA_DISP1_PIX_MAX_HZ) else {
        return Err(t.fail(step::PIXEL, -EINVAL));
    };
    ccm::set_root(m, b.ccm, root::MEDIA_DISP1_PIX, setting).map_err(|e| t.fail(step::PIXEL, e))?;
    crate::ylog!(
        env,
        Inf,
        "media: disp1_pix mux {} /{}/{} = {} Hz (wanted {})",
        setting.mux,
        setting.pre,
        setting.post,
        hz,
        want
    );
    t.done(step::PIXEL);
    Ok(hz)
}

/// What the pixel clock root produces now, from its setting and its
/// parent's registers; 0 when it is off or its parent is not one this
/// module can read.
pub fn pixel_clock(m: &impl Mmio, b: &Bases) -> u32 {
    let r = ccm::root_setting(m, b.ccm, root::MEDIA_DISP1_PIX);
    if !r.enabled {
        return 0;
    }
    let parent = match r.mux {
        mux::OSC_24M => Some(ccm::OSC_24M_HZ),
        mux::DISP1_PIX_VIDEO_PLL1 => anatop::running_rate(m, b.anatop, Pll::Video1),
        mux::DISP1_PIX_SYS_PLL1 => anatop::running_rate(m, b.anatop, Pll::Sys1),
        mux::DISP1_PIX_SYS_PLL2 => anatop::running_rate(m, b.anatop, Pll::Sys2),
        _ => None,
    };
    parent.map_or(0, |hz| hz / r.pre / r.post)
}

/// MEDIA_MIPI_PHY1_REF_CLK_ROOT from 24M_REF_CLK (Table 5-1, slice 123) at
/// 24 MHz / n nearest below or at `hz`. Returns the rate.
pub fn set_phy_ref_clock(m: &impl Mmio, b: &Bases, t: &mut Track, hz: u32) -> Result<u32, i32> {
    if hz == 0 {
        return Err(t.fail(step::PHY_REF, -EINVAL));
    }
    let div = ccm::OSC_24M_HZ.div_ceil(hz).clamp(1, 64);
    ccm::set_root(m, b.ccm, root::MEDIA_MIPI_PHY1_REF, RootSetting::on(mux::OSC_24M, 1, div))
        .map_err(|e| t.fail(step::PHY_REF, e))?;
    t.done(step::PHY_REF);
    Ok(ccm::OSC_24M_HZ / div)
}

/// Open the clock roots and gates a kexec'd image needs before any driver
/// runs (roots: ENABLE only, mux and dividers kept; gates: domain 0 on).
/// Returns how many registers it wrote.
pub fn early_clocks(m: &impl Mmio, ccm_base: usize, roots: &[u32], gates: &[u32]) -> Result<u32, i32> {
    let mut n = 0;
    for &r in roots {
        n += u32::from(ccm::enable_root(m, ccm_base, r)?);
    }
    for &g in gates {
        n += u32::from(ccm::gate_on(m, ccm_base, g)?);
    }
    Ok(n)
}

/// The status word ([`st`]): GPC, CCM and ANATOP read live (always
/// readable), MEDIA_BLK_CTRL as last sampled, and the last operation.
pub fn status(m: &impl Mmio, b: &Bases, t: &Track) -> u32 {
    let mm = gpc::state(m, b.gpc, &gpc::MEDIAMIX);
    let ph = gpc::state(m, b.gpc, &gpc::MIPI_PHY1);
    let flag = |c: bool, bit: u32| if c { bit } else { 0 };
    let r = |idx| ccm::root_setting(m, b.ccm, idx);
    let axi = r(root::MEDIA_AXI);
    let apb = r(root::MEDIA_APB);
    flag(mm.mapped, st::MEDIAMIX_MAPPED)
        | flag(ph.mapped, st::MIPI_PHY1_MAPPED)
        | flag(mm.pup_clean, st::MEDIAMIX_PUP_CLEAN)
        | flag(ph.pup_clean, st::MIPI_PHY1_PUP_CLEAN)
        | flag(mm.adb_req, st::ADB_REQ)
        | flag(mm.adb_ack, st::ADB_ACK)
        | flag(mm.pcr_clear && ph.pcr_clear, st::PCR_CLEAR)
        | (t.sampled & (st::POWERED_FROM_OFF | st::BLK_MASK))
        | flag(axi.enabled && axi.mux == mux::MEDIA_AXI_SYS_PLL2, st::MEDIA_AXI_ROOT)
        | flag(apb.enabled && apb.mux == mux::MEDIA_APB_SYS_PLL1, st::MEDIA_APB_ROOT)
        | flag(ccm::gate_is_on(m, b.ccm, gate::MEDIA), st::MEDIA_GATE)
        | flag(r(root::MEDIA_DISP1_PIX).enabled, st::DISP1_PIX_ROOT)
        | flag(r(root::MEDIA_MIPI_PHY1_REF).enabled, st::PHY1_REF_ROOT)
        | flag(anatop::running_rate(m, b.anatop, Pll::Video1).is_some(), st::VIDEO_PLL1_RUNNING)
        | flag(anatop::running_rate(m, b.anatop, Pll::Sys1).is_some(), st::SYS_PLL1_RUNNING)
        | flag(anatop::running_rate(m, b.anatop, Pll::Sys2).is_some(), st::SYS_PLL2_RUNNING)
        | (t.last & 0xff) << st::LAST_SHIFT
}

/// The domains this module drives, for a caller that wants their names.
pub const DOMAINS: [PuDomain; 2] = [gpc::MEDIAMIX, gpc::MIPI_PHY1];

#[cfg(test)]
pub(crate) mod tests;
