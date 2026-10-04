// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 8M Plus HDMI display chain brought up from cold: LCDIF3 -> PVI
//! -> DesignWare HDMI TX -> HDMI PHY, for one fixed mode. The policy and the
//! order are this project's (driver/docs/x2-hdmi-cold.md has the sequence
//! with its RM citations); the register work is the modules it calls:
//! [`super::hdmi`] (power, clocks, resets, PVI), [`crate::ip::imx8mp_hdmi_phy`],
//! [`crate::ip::dw_hdmi_tx`], [`crate::ip::lcdifv3`].
//!
//! Stages ([`stage`]) are numbered so that a board with no console can be
//! bisected: `stop_after` = N returns `-ECANCELED` once stage N is done,
//! leaving the hardware as it is, like `SAFEUI_DISPLAY_IMX8MP_STOP_AFTER` on
//! the Verdin. Two staging pictures need no pixel stream from memory:
//! [`TestPicture::HdmiForced`] (the controller's forced video after stage 5)
//! and [`TestPicture::LcdifBackground`] (LCDIF3's background-colour test mode
//! after stage 7).
//!
//! The order follows the dependencies: the PHY's PLL makes the pixel clock
//! the controller, the PVI and LCDIF3 run on (see [`super::hdmi`] on
//! `HTX_PIPE_CLK_SEL`), so it locks before any of them is started, and the
//! LCDIF -- whose first shadow load only completes with a running pixel
//! clock -- starts last.

use super::hdmi::{self, ClockSel, PowerReport};
use crate::env::Env;
use crate::errno::{ECANCELED, EINVAL, EIO};
use crate::ip::dw_hdmi_tx::{self as dw, PhyIf};
use crate::ip::imx8mp_hdmi_phy::{self as phy, Pll};
use crate::ip::lcdifv3::{self as lcdif, Layer, Output, Panic};
use crate::mmio::Mmio;
use crate::video::VideoMode;
use crate::ylog;

/// Stage numbers (`stop_after`, and the stage a failure is reported at).
pub mod stage {
    /// The configuration is checked and the PLL planned; a running chain is
    /// stopped (and powered off with `power_cycle`).
    pub const START: u32 = 1;
    /// CCM roots and gate, HDMIMIX (+ ADB400) and HDMI PHY domains.
    pub const POWER: u32 = 2;
    /// Block control: clocks, selects, reset pulse, PHY power-down released.
    pub const BLK: u32 = 3;
    /// The PHY PLL locked.
    pub const PHY: u32 = 4;
    /// The TX controller programmed.
    pub const CONTROLLER: u32 = 5;
    /// The PVI on the LCDIF path.
    pub const PVI: u32 = 6;
    /// LCDIF3 scanning (its first shadow load completed).
    pub const LCDIF: u32 = 7;
    /// Clocks present at the controller, PHY locked, no PVI FIFO error.
    pub const VIDEO: u32 = 8;
}

/// A picture that needs less of the chain (staging).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestPicture {
    None,
    /// After stage 5: the controller's forced video (`fc_dbgforce`).
    HdmiForced,
    /// After stage 7: LCDIF3's background colour, no memory fetch.
    LcdifBackground(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub lcdif: usize,
    pub hdmi: usize,
    pub phy: usize,
    pub blk: usize,
    pub pvi: usize,
    pub gpc: usize,
    pub ccm: usize,
}

impl Bases {
    fn soc(&self) -> hdmi::Bases {
        hdmi::Bases { blk: self.blk, pvi: self.pvi, gpc: self.gpc, ccm: self.ccm }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub mode: VideoMode,
    /// The largest pixel clock error accepted from the integer PLL.
    pub max_ppm: u32,
    pub clock_sel: ClockSel,
    pub phy_if: PhyIf,
    pub lcdif_pxck_rising: bool,
    pub desc3_extra: u32,
    pub noc_hurry: u32,
    pub panic: Option<Panic>,
    /// Power HDMIMIX and the PHY off first when they are found on.
    pub power_cycle: bool,
    /// 0 = run to the end.
    pub stop_after: u32,
    pub test: TestPicture,
}

/// What a bring-up did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub pll: Pll,
    pub power: PowerReport,
    pub pitch: u32,
    pub refresh_mhz: u32,
    /// The chain was running when we came, and was stopped.
    pub was_running: bool,
}

/// A failure, with the stage it happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed {
    pub stage: u32,
    pub errno: i32,
}

/// Health bits ([`health`]); bit 0 is shared with the Verdin's
/// `IMX8MP_DISPLAY_HEALTH_LCDIF_RUNNING`.
pub mod health_bit {
    pub const LCDIF_RUNNING: u32 = 1 << 0;
    pub const HDMIMIX_UP: u32 = 1 << 8;
    pub const BLK_ENABLED: u32 = 1 << 9;
    pub const PHY_PLL_LOCK: u32 = 1 << 10;
    pub const PHY_READY: u32 = 1 << 11;
    pub const HPD: u32 = 1 << 12;
    pub const TMDS_CLOCK: u32 = 1 << 13;
    pub const PIXEL_CLOCK: u32 = 1 << 14;
    pub const PVI_ON: u32 = 1 << 15;
    pub const PVI_FIFO_ERROR: u32 = 1 << 16;
    pub const RX_SENSE: u32 = 1 << 17;
}

/// The LCDIF and the controller can be read: HDMIMIX up and the block
/// control holding their clocks and resets. Reading them otherwise is an
/// access to an unpowered or unclocked block.
fn reachable(m: &impl Mmio, b: &Bases) -> bool {
    hdmi::hdmimix_is_up(m, b.gpc) && hdmi::blk_is_enabled(m, b.blk)
}

/// Read back which parts of the chain are alive (nothing is written except
/// the controller's clock-present flags, which are cleared after reading so
/// that the next call sees whether the clocks are still there).
pub fn health(m: &impl Mmio, b: &Bases) -> u32 {
    use health_bit::*;
    let mut h = 0;
    if !hdmi::hdmimix_is_up(m, b.gpc) {
        return h;
    }
    h |= HDMIMIX_UP;
    if !hdmi::blk_is_enabled(m, b.blk) {
        return h;
    }
    h |= BLK_ENABLED;
    let t = hdmi::tx_status(m, b.blk);
    use super::regs::hdmi_tx_blk_ctl::tx_status0 as ts;
    if t & ts::TX_PHY_PLL_LOCK_MASK != 0 {
        h |= PHY_PLL_LOCK;
    }
    if t & ts::TX_PHY_RDY_MASK != 0 {
        h |= PHY_READY;
    }
    if t & ts::TX_HPD_STATUS_MASK != 0 {
        h |= HPD;
    }
    let s = dw::status(m, b.hdmi);
    dw::clear_clock_flags(m, b.hdmi);
    if s.tmds_clock {
        h |= TMDS_CLOCK;
    }
    if s.pixel_clock {
        h |= PIXEL_CLOCK;
    }
    if s.rx_sense != 0 {
        h |= RX_SENSE;
    }
    if hdmi::pvi_is_on(m, b.pvi) {
        h |= PVI_ON;
    }
    if hdmi::pvi_fifo_errors(m, b.pvi) != 0 {
        h |= PVI_FIFO_ERROR;
    }
    if lcdif::is_running(m, b.lcdif) {
        h |= LCDIF_RUNNING;
    }
    h
}

/// LCDIF3 is scanning (false whenever it cannot be read safely).
pub fn is_running(m: &impl Mmio, b: &Bases) -> bool {
    reachable(m, b) && lcdif::is_running(m, b.lcdif)
}

/// What LCDIF3 scans (None whenever it cannot be read safely).
pub fn scanout(m: &impl Mmio, b: &Bases) -> Option<lcdif::Scanout> {
    if reachable(m, b) {
        lcdif::read_scanout(m, b.lcdif)
    } else {
        None
    }
}

fn at(stage: u32) -> impl Fn(i32) -> Failed {
    move |errno| Failed { stage, errno }
}

fn done(cfg: &Config, stage: u32) -> Result<(), Failed> {
    if cfg.stop_after == stage {
        Err(Failed { stage, errno: -ECANCELED })
    } else {
        Ok(())
    }
}

/// Bring the chain up on `layer` (see the module text and the design note).
pub fn bring_up(m: &impl Mmio, b: &Bases, env: &impl Env, cfg: &Config, layer: &Layer) -> Result<Report, Failed> {
    use stage::*;
    // 1: check, plan, stop what runs
    cfg.mode.check().map_err(at(START))?;
    let pll = phy::plan(cfg.mode.pixel_clock_hz, cfg.max_ppm).ok_or(Failed { stage: START, errno: -EINVAL })?;
    let was_running = is_running(m, b);
    if was_running {
        // the old picture stops at a frame boundary; a timeout means its
        // clock had already gone, which the cold path does not need anyway
        let _ = lcdif::disable(m, b.lcdif, env);
    }
    if cfg.power_cycle && hdmi::hdmimix_is_up(m, b.gpc) {
        hdmi::power_off(m, &b.soc(), env).map_err(|f| at(START)(f.errno()))?;
    }
    ylog!(
        env,
        Inf,
        "hdmi: {}x{} pixel {} Hz -> PLL P {} M {} S {}: {} Hz ({} ppm){}",
        cfg.mode.hactive,
        cfg.mode.vactive,
        cfg.mode.pixel_clock_hz,
        pll.p,
        pll.m,
        pll.s,
        pll.tmds_hz,
        pll.err_ppm,
        if was_running { "; stopped the running chain" } else { "" }
    );
    done(cfg, START)?;

    // 2: power
    let power = hdmi::power_on(m, &b.soc(), env).map_err(|f| at(POWER)(f.errno()))?;
    done(cfg, POWER)?;

    // 3: block control
    hdmi::blk_enable(m, b.blk, env, &cfg.clock_sel, cfg.noc_hurry).map_err(at(BLK))?;
    done(cfg, BLK)?;

    // 4: PHY PLL
    if let Err(e) = phy::program(m, b.phy, env, &pll) {
        ylog!(env, Err, "hdmi: PHY PLL no lock, PHY_REG34 {:#04x}", phy::status(m, b.phy));
        return Err(at(PHY)(e));
    }
    done(cfg, PHY)?;

    // 5: controller
    dw::configure(m, b.hdmi, &cfg.mode, pll.tmds_hz, &cfg.phy_if).map_err(at(CONTROLLER))?;
    if cfg.test == TestPicture::HdmiForced {
        dw::force_video(m, b.hdmi, true);
    }
    done(cfg, CONTROLLER)?;

    // 6: PVI
    hdmi::pvi_configure(m, b.pvi, &cfg.mode);
    done(cfg, PVI)?;

    // 7: LCDIF3; its strobes active high into the PVI, which applies the
    // mode's polarities on the way out
    let out = Output { pxck_rising: cfg.lcdif_pxck_rising, hsync_low: false, vsync_low: false, de_low: false };
    let pitch = lcdif::configure(m, b.lcdif, &cfg.mode, &out, layer).map_err(at(LCDIF))?;
    lcdif::enable(m, b.lcdif, env, cfg.panic).map_err(at(LCDIF))?;
    if let TestPicture::LcdifBackground(rgb) = cfg.test {
        lcdif::test_mode(m, b.lcdif, lcdif::DispMode::Background, rgb);
    }
    done(cfg, LCDIF)?;

    // 8: is video flowing? Clear the clock-present flags, let a frame pass,
    // look again.
    dw::clear_clock_flags(m, b.hdmi);
    env.busy_wait_us(lcdif::FRAME_WAIT_US / 2);
    let h = health(m, b);
    use health_bit::*;
    let need = PHY_PLL_LOCK | TMDS_CLOCK | PIXEL_CLOCK | PVI_ON | LCDIF_RUNNING;
    if h & need != need || h & PVI_FIFO_ERROR != 0 {
        ylog!(env, Err, "hdmi: video check failed, health {:#x}", h);
        return Err(at(VIDEO)(-EIO));
    }
    if h & HPD == 0 {
        ylog!(env, Wrn, "hdmi: no hot-plug detect (assumed connected)");
    }
    let refresh_mhz = cfg.mode.refresh_mhz(pll.tmds_hz);
    ylog!(env, Inf, "hdmi: up, {}.{:03} Hz, health {:#x}", refresh_mhz / 1000, refresh_mhz % 1000, h);
    Ok(Report { pll, power, pitch, refresh_mhz, was_running })
}

/// Stop scanning and power the chain down.
pub fn shut_down(m: &impl Mmio, b: &Bases, env: &impl Env) -> Result<(), i32> {
    if reachable(m, b) {
        let _ = lcdif::disable(m, b.lcdif, env);
    }
    if hdmi::hdmimix_is_up(m, b.gpc) {
        hdmi::power_off(m, &b.soc(), env).map_err(|f| f.errno())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::health_bit::*;
    use super::*;
    use crate::model::TestEnv;
    use crate::model_hdmi::{HdmiFaults, HdmiModel, BLK, CCM, GPC, HDMI, LCDIF, PHY, PVI};
    use crate::video::WUXGA_60_RB;

    fn bases(m: &HdmiModel) -> Bases {
        Bases {
            lcdif: m.base(LCDIF),
            hdmi: m.base(HDMI),
            phy: m.base(PHY),
            blk: m.base(BLK),
            pvi: m.base(PVI),
            gpc: m.base(GPC),
            ccm: m.base(CCM),
        }
    }

    pub(crate) const CFG: Config = Config {
        mode: WUXGA_60_RB,
        max_ppm: 5000,
        clock_sel: ClockSel { htx_pipe: None, htxphy: None },
        phy_if: PhyIf { conf0: None, phyrstz: None },
        lcdif_pxck_rising: false,
        desc3_extra: 0,
        noc_hurry: 7,
        panic: Some(Panic { low: 170, high: 340 }),
        power_cycle: false,
        stop_after: 0,
        test: TestPicture::None,
    };
    const LAYER: Layer = Layer { phys: 0x9000_0000, pitch: 0, desc3_extra: 0 };

    #[test]
    fn cold_bring_up_runs_every_stage_in_order() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        assert_eq!(health(&m, &b), 0, "nothing reachable while HDMIMIX is off");
        let r = bring_up(&m, &b, &e, &CFG, &LAYER).unwrap();
        assert_eq!((r.pll.p, r.pll.m, r.pll.s), (7, 225, 1));
        assert_eq!(r.pitch, 7680);
        assert_eq!(r.refresh_mhz, 60_061, "154.286 MHz / (2080 x 1235)");
        assert!(!r.was_running);
        let h = health(&m, &b);
        assert_eq!(
            h & (LCDIF_RUNNING | HDMIMIX_UP | BLK_ENABLED | PHY_PLL_LOCK | PHY_READY | HPD | PVI_ON),
            LCDIF_RUNNING | HDMIMIX_UP | BLK_ENABLED | PHY_PLL_LOCK | PHY_READY | HPD | PVI_ON
        );
        assert_eq!(h & PVI_FIFO_ERROR, 0);
        // the order across blocks: power, block control, PHY, controller,
        // PVI, LCDIF
        let w = m.writes();
        let first = |blk| w.iter().position(|x| x.0 == blk).unwrap();
        assert!(first(GPC) < first(BLK));
        assert!(first(BLK) < first(PHY));
        assert!(first(PHY) < first(HDMI));
        assert!(first(HDMI) < first(PVI));
        assert!(first(PVI) < first(LCDIF));
        assert_eq!(scanout(&m, &b).unwrap().phys, 0x9000_0000);
    }

    #[test]
    fn stop_after_each_stage_leaves_later_blocks_untouched() {
        for (stage, untouched) in [
            (stage::START, GPC),
            (stage::POWER, PHY),
            (stage::BLK, PHY),
            (stage::PHY, HDMI),
            (stage::CONTROLLER, PVI),
            (stage::PVI, LCDIF),
        ] {
            let m = HdmiModel::new();
            let e = TestEnv::default();
            let b = bases(&m);
            let r = bring_up(&m, &b, &e, &Config { stop_after: stage, ..CFG }, &LAYER);
            assert_eq!(r, Err(Failed { stage, errno: -ECANCELED }));
            assert!(!m.writes().iter().any(|x| x.0 == untouched), "stage {stage}: {untouched} written");
        }
    }

    #[test]
    fn each_failure_names_its_stage() {
        for (f, stage) in [
            (HdmiFaults { gpc_pup_stuck: true, ..Default::default() }, stage::POWER),
            (HdmiFaults { adb_no_ack: true, ..Default::default() }, stage::POWER),
            (HdmiFaults { phy_no_lock: true, ..Default::default() }, stage::PHY),
            (HdmiFaults { no_vsync: true, ..Default::default() }, stage::LCDIF),
            (HdmiFaults { pvi_underflow: true, ..Default::default() }, stage::VIDEO),
        ] {
            let m = HdmiModel::with_faults(f);
            let e = TestEnv::default();
            let r = bring_up(&m, &bases(&m), &e, &CFG, &LAYER);
            assert_eq!(r.map_err(|x| x.stage), Err(stage));
        }
        // a mode the integer PLL cannot reach within the bound
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let r = bring_up(&m, &bases(&m), &e, &Config { max_ppm: 100, ..CFG }, &LAYER);
        assert_eq!(r, Err(Failed { stage: stage::START, errno: -EINVAL }));
        assert!(m.writes().is_empty());
    }

    #[test]
    fn a_running_chain_is_stopped_and_power_cycled_first() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        bring_up(&m, &b, &e, &CFG, &LAYER).unwrap();
        m.clear_log();
        let r =
            bring_up(&m, &b, &e, &Config { power_cycle: true, ..CFG }, &Layer { phys: 0xa000_0000, ..LAYER }).unwrap();
        assert!(r.was_running && r.power.hdmimix_was_down);
        assert_eq!(scanout(&m, &b).unwrap().phys, 0xa000_0000);
        // the layer was switched off before the domain went down
        let w = m.writes();
        let off = w.iter().position(|x| x.0 == LCDIF && x.1 == 0x214 && x.2 & (1 << 31) == 0).unwrap();
        let pdn = w.iter().position(|x| x.0 == GPC && x.1 == 0xe4).unwrap();
        assert!(off < pdn);
    }

    #[test]
    fn staging_pictures() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        let cfg = Config { test: TestPicture::HdmiForced, stop_after: stage::CONTROLLER, ..CFG };
        assert_eq!(bring_up(&m, &b, &e, &cfg, &LAYER).unwrap_err().stage, stage::CONTROLLER);
        assert_eq!(m.peek(HDMI, 0x1200), 1, "fc_dbgforce.forcevideo");
        let m = HdmiModel::new();
        let cfg = Config { test: TestPicture::LcdifBackground(0x0000ff), ..CFG };
        bring_up(&m, &bases(&m), &e, &cfg, &LAYER).unwrap();
        assert_eq!(m.peek(LCDIF, 0x10) & 0x03ff_ffff, (1 << 24) | 0xff);
    }

    #[test]
    fn shut_down_powers_off() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let b = bases(&m);
        bring_up(&m, &b, &e, &CFG, &LAYER).unwrap();
        shut_down(&m, &b, &e).unwrap();
        assert_eq!(health(&m, &b), 0);
        assert!(!is_running(&m, &b));
    }
}
