// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RK3588 HDMI0 display chain brought up from cold: VOP2 VP0 with one
//! Esmart0 window -> DW HDMI QP TX0 -> HDPTX PHY0, for one fixed mode in DVI
//! (driver/docs/nanopc-t6-display.md, section 2 (c) and section 7 row D7).
//! The policy and the order are this project's; the register work is the
//! modules it calls: [`super::rk3588_display`] (power, clocks, resets, D4),
//! [`crate::ip::hdptx`] (the PHY, D5), [`crate::ip::dw_hdmi_qp`] (the
//! controller and its GRF glue, D6), [`crate::ip::vop2`] and
//! [`crate::ip::rk_iommu`] (D1).
//!
//! Stages ([`stage`]) are numbered so that a board with no picture can be
//! bisected: `stop_after` = N returns [`Error::Stopped`] once stage N is
//! done, leaving the hardware as it is. Every failure names its stage
//! ([`Failed`]).
//!
//! 1. START: the configuration is checked before anything is written: the
//!    mode against VP0 ([`vop2::vp_timing`]) and the controller
//!    ([`dw::tmds_char_rate`]), the PHY plan and image, the caller's buffer
//!    against the window ([`vop2::window_check`]). Then what runs is read,
//!    nothing written: the PMU and CRU say whether VOP2 can be read; if so,
//!    `DSP_CTRL` of VP0-VP3 says which ports run (review R3-S1 of D1: read
//!    before anything else touches the VOP). PD_VOP on but VOP2 not
//!    readable (a gate closed, a reset asserted) means a port may run
//!    unseen: refused ([`Error::VopUnreadable`]) unless `power_cycle`, which
//!    first opens VOP2's gates and releases its resets and reads again. A
//!    running port without `force_cold` stops the bring-up
//!    ([`Error::FoundRunning`]): taking a running picture over is route (a),
//!    step D2, not this. With `force_cold`, a running port this chain
//!    clocks again later (VP0) that runs no frame is refused
//!    ([`Error::Unclocked`]) unless `power_cycle`: it is in the middle of a
//!    frame, and a window disable or STANDBY written to it would land only
//!    when the DCLK stage gives its clock back, with paging still on. Then
//!    every window is disabled, `REG_CFG_DONE` written for each running
//!    port and the port put in STANDBY ([`vop2::standby`]); a port whose
//!    STANDBY never comes and that runs no frame has no pixel clock and
//!    fetches nothing (R3-S1), so it is left (a port this chain never
//!    clocks, or one whose state the power cycle drops). With `power_cycle` the
//!    PHY is powered off ([`hdptx::power_off`]), its APB reset asserted and
//!    the display domains switched off ([`display_off`]): every block
//!    starts from its reset state.
//! 2. POWER: [`display_on`]: ACLK_VOP's rate, PD_VOP, PD_VO1, the blocks'
//!    gates and resets.
//! 3. GRF: `PCLK_VO1GRF` (59/12) opened for vo1_grf, then the two words
//!    the VOP's HDMI0 route writes (`rockchip_vop2_reg.c:1690-1702`):
//!    vop_grf `GRF_VOP_CON2` bit 1 and vo1_grf `GRF_VO1_CON0` bits 6:5 (the
//!    sync field, [`dw::linux_sync_field`]), each read back.
//! 4. PHY: [`hdptx::power_on`]: the PLL at the plan's rate with the image's
//!    CMN words, then the lanes, the lock. The PHY's four resets are the
//!    CRU's ([`PhyHost`]).
//! 5. DCLK: VP0's pixel clock onto the PHY's PLL: `SRST_D_VOP0` released
//!    (rst-rk3588.c:535; the release word only), CLKSEL_CON(112) bits 8:7 =
//!    1 `clk_hdmiphy_pixel0` (clk-rk3588.c:2100-2103, :503) read back, gate
//!    52/13 opened. Linux moves VP0 onto the PHY's clock for every HDMI0
//!    mode up to 600 MHz (`rockchip_drm_vop2.c:1740-1774`).
//! 6. CONTROLLER: [`dw::bind`] (identity, GRF glue words 1-4, interrupt
//!    masks, timer base, glue word 5), then [`dw::enable`] for the sink.
//! 7. VOP: [`vop2::init`], [`rk_iommu::paging_off`] on both `vop_mmu` sets
//!    (the window's address is physical), every window off
//!    ([`vop2::windows_off`]), VP0 routed to HDMI0, its timing, started;
//!    Esmart0 made VP0's only layer ([`vop2::overlay_single`], which waits
//!    for a frame: the pixel clock must run) and shown on the caller's
//!    buffer ([`vop2::window_show`]).
//! 8. VIDEO: the section 5 read-backs: over two frames VP0 starts frames,
//!    does not underflow and raises no bus error; Esmart0's update is taken
//!    (`REG_CFG_DONE` bit 0 back to 0, `REGION0_YRGB_MST` = the buffer); the
//!    PHY reads locked, clock ready and ready; the controller's clock
//!    monitor reads locked. The hot-plug level is logged, not required: the
//!    T6's panel gives none (board, 2026-11-12; `dw_hdmi_qp` fact 9).
//!
//! Where this differs from Linux's order: Linux binds the controller at
//! probe, long before a mode is set, and runs the PHY's PLL (stages 1-3)
//! when the VOP sets its pixel clock, the PHY's lanes (stage 4) and the
//! controller's enable when the encoder is enabled (`ip::hdptx` facts 6, 7;
//! `ip::dw_hdmi_qp` fact 3). Here the PHY comes up whole before the clock
//! mux and the controller: the lanes and the lock do not depend on the VOP,
//! and a stage that ends with the PHY locked is the one a bisection needs.
//! The controller's bind needs nothing of the PHY.

use super::rk3588::gate_open;
use super::rk3588_display::{
    clk, display_off, display_on, ready, reset, reset_assert, reset_release, DisplayError, DisplayOn, NotReady, VOP,
};
use crate::env::Env;
use crate::ip::dw_hdmi_qp::{self as dw, BindParams, Ident, Port, Sink};
use crate::ip::hdptx::{self, Host, Image, PhyReset, Plan, PowerOn};
use crate::ip::rk_iommu::{self, Off};
use crate::ip::vop2::{self, Esmart, FrameCheck, HdmiPort, Init, Overlay, ReadBack, Vp, Window};
use crate::mmio::Mmio;
use crate::video::VideoMode;
use crate::ylog;

/// Stage numbers (`stop_after`, and the stage a failure is reported at).
pub mod stage {
    /// The configuration checked, what runs read (and with `force_cold`
    /// stopped, with `power_cycle` powered off).
    pub const START: u32 = 1;
    /// Power domains, clocks and resets.
    pub const POWER: u32 = 2;
    /// The VOP's HDMI0 words in vop_grf and vo1_grf.
    pub const GRF: u32 = 3;
    /// The PHY's PLL locked and its lanes ready.
    pub const PHY: u32 = 4;
    /// DCLK_VOP0 on the PHY's PLL.
    pub const DCLK: u32 = 5;
    /// The HDMI controller bound and enabled.
    pub const CONTROLLER: u32 = 6;
    /// VP0 running with Esmart0 on the caller's buffer.
    pub const VOP: u32 = 7;
    /// The read-backs of the running picture.
    pub const VIDEO: u32 = 8;
}

/// The build's fixed mode: what Linux runs the T6's display at, since the
/// display gives no EDID (board, Linux 6.18, 2026-11-12, the DRM state's
/// mode line): 1024x768 at 60 Hz, pixel clock 65000 kHz, h 1024 1048 1184
/// 1344, v 768 771 777 806, both syncs negative. The design note's section
/// 3 expected 1080p; the board says otherwise.
pub const MODE_1024X768_60: VideoMode = VideoMode {
    pixel_clock_hz: 65_000_000,
    hactive: 1024,
    hfront: 24,
    hsync: 136,
    hback: 160,
    vactive: 768,
    vfront: 3,
    vsync: 6,
    vback: 29,
    hsync_high: false,
    vsync_high: false,
    de_high: true,
};

/// The checks of the VIDEO stage ([`Error::Video`]).
pub mod video_check {
    /// No frame start on VP0 within two frames.
    pub const NO_FRAME: u32 = 1 << 0;
    /// VP0 raised POST_BUF_EMPTY: the fetch fell behind.
    pub const UNDERFLOW: u32 = 1 << 1;
    /// SYS0 or SYS1 bus error: an AXI fetch failed.
    pub const BUS_ERROR: u32 = 1 << 2;
    /// Esmart0's `REGION0_YRGB_MST` does not read the buffer's address.
    pub const ADDRESS: u32 = 1 << 3;
    /// The PHY is not locked, clock ready and ready.
    pub const PHY: u32 = 1 << 4;
    /// The controller's clock monitor does not read locked.
    pub const CMU: u32 = 1 << 5;
}

/// The blocks' addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub cru: usize,
    pub pmu: usize,
    pub sys_grf: usize,
    pub vop_grf: usize,
    pub vo1_grf: usize,
    pub phy: hdptx::Bases,
    pub hdmi: usize,
    pub vop: usize,
    pub vop_mmu: [usize; 2],
}

/// The RK3588's (design note section 1; the CRU and the PMU as
/// `soc::rk3588`).
pub const RK3588: Bases = Bases {
    cru: super::rk3588::CRU_BASE,
    pmu: super::rk3588::PMU_BASE,
    sys_grf: dw::SYS_GRF_BASE,
    vop_grf: dw::VOP_GRF_BASE,
    vo1_grf: dw::VO1_GRF_BASE,
    phy: hdptx::PHY0,
    hdmi: dw::TX0_BASE,
    vop: VOP.base,
    vop_mmu: [0xfdd9_7e00, 0xfdd9_7f00],
};

/// What one bring-up asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config<'a> {
    pub mode: VideoMode,
    /// The PHY's ROPLL dividers for the mode's TMDS rate (the board takes
    /// Linux's, read from its dump, `hdptx::PllWords::plan`).
    pub plan: Plan,
    /// The PHY's register image for the mode (the board dump's).
    pub image: Image<'a>,
    /// The controller's binding values (`CORE_ID`, the timer base).
    pub bind: BindParams,
    pub sink: Sink,
    /// Bring the chain up from cold even when a port is found running.
    pub force_cold: bool,
    /// Power the blocks off first (PHY off and in reset, domains off).
    pub power_cycle: bool,
    /// 0 = run to the end; N stops after stage N, [`stage::VIDEO`]
    /// included (the bring-up then returns [`Error::Stopped`], not a
    /// [`Report`]).
    pub stop_after: u32,
}

/// What was found before anything was written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// VOP2 could be read (PD_VOP usable, its clocks open, resets released).
    pub vop_reachable: bool,
    /// PD_VOP is usable but VOP2 could not be read: the gate or reset that
    /// stopped it. Whether a port runs is then not known.
    pub vop_unreadable: Option<NotReady>,
    /// `DSP_CTRL` of VP0-VP3 (0 when the VOP could not be read).
    pub dsp_ctrl: [u32; 4],
    /// The ports with STANDBY clear (bit n = VPn).
    pub running: u32,
    /// hdptxphy0_grf `STATUS`.
    pub phy_status: u32,
    /// With `force_cold`: the running ports put in STANDBY, and those whose
    /// STANDBY never came because they run no frame (no pixel clock).
    pub stopped: u32,
    pub unclocked: u32,
    /// With `power_cycle`: the PHY was powered off.
    pub phy_powered_off: bool,
}

/// What a bring-up did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub found: Found,
    pub power: DisplayOn,
    pub phy: PowerOn,
    pub ident: Ident,
    pub vop: Init,
    pub iommu: Off,
    pub overlay: Overlay,
    pub frame: FrameCheck,
    pub read_back: ReadBack,
    pub phy_status: hdptx::Status,
    pub hdmi_status: dw::Status,
}

/// Why a stage stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `stop_after` reached (not a failure).
    Stopped,
    /// The mode does not fit VP0 or the controller.
    Mode,
    /// The PHY's plan or image is refused.
    Phy(hdptx::Refused),
    /// The buffer is refused by the window (or a VOP2 step refused).
    Vop(vop2::Refused),
    /// A port runs and `force_cold` is not set (the ports, bit n = VPn).
    FoundRunning(u32),
    /// A running port did not stop: its STANDBY did not come although it
    /// runs frames.
    Standby(vop2::Refused),
    Display(DisplayError),
    /// A GRF word read back without the bits written (its address, the
    /// word read).
    Grf(usize, u32),
    /// A gate still reads gated after its write (the CLKGATE_CON read).
    Gate(u32),
    /// DCLK_VOP0's mux read back other than the PHY (the CLKSEL_CON read).
    DclkMux(u32),
    Hdmi(dw::Refused),
    Iommu(rk_iommu::Refused),
    /// The section 5 read-backs after the bring-up: the [`video_check`]
    /// bits that failed (the values are logged).
    Video(u32),
    /// PD_VOP is on but VOP2 cannot be read (a gate closed or a reset
    /// asserted): whether a port runs is not known.
    VopUnreadable(NotReady),
    /// A port this chain clocks (VP0) was found with STANDBY clear and no
    /// pixel clock (bit n = VPn).
    Unclocked(u32),
}

/// A failure, with the stage it happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failed {
    pub stage: u32,
    pub error: Error,
}

fn at<E>(stage: u32, f: impl Fn(E) -> Error) -> impl Fn(E) -> Failed {
    move |e| Failed { stage, error: f(e) }
}

fn done(cfg: &Config, stage: u32) -> Result<(), Failed> {
    if cfg.stop_after == stage {
        Err(Failed { stage, error: Error::Stopped })
    } else {
        Ok(())
    }
}

/// The PHY's four resets in the CRU (`ip::hdptx` fact 3; D4's names), and
/// its waits on the environment's clock.
pub struct PhyHost<'a, M: Mmio, E: Env> {
    pub m: &'a M,
    pub cru: usize,
    pub env: &'a E,
}

impl<M: Mmio, E: Env> Host for PhyHost<'_, M, E> {
    fn reset(&self, r: PhyReset, asserted: bool) {
        let rst = match r {
            PhyReset::Apb => reset::P_HDPTX0,
            PhyReset::Init => reset::HDPTX0_INIT,
            PhyReset::Cmn => reset::HDPTX0_CMN,
            PhyReset::Lane => reset::HDPTX0_LANE,
        };
        if asserted {
            reset_assert(self.m, self.cru, rst);
        } else {
            reset_release(self.m, self.cru, rst);
        }
    }
    fn wait_us(&self, us: u32) {
        self.env.busy_wait_us(us);
    }
}

/// The window VP0 shows: the caller's buffer, filling the mode's active area.
pub fn window(mode: &VideoMode, addr: u64, stride_bytes: u32) -> Window {
    Window { addr, width: mode.hactive, height: mode.vactive, stride_bytes, x: 0, y: 0, format: vop2::Format::Xrgb8888 }
}

/// One frame of `mode`, in µs, rounded up.
pub fn frame_us(mode: &VideoMode) -> u32 {
    let px = u64::from(mode.htotal()) * u64::from(mode.vtotal()) * 1_000_000;
    px.div_ceil(u64::from(mode.pixel_clock_hz.max(1))) as u32
}

/// Read what runs (START, before any write).
fn find(m: &impl Mmio, b: &Bases) -> Found {
    let mut f = Found::default();
    // the PHY's GRF names no clock in Linux's tree; it is read only when the
    // PHY itself is reachable, as `power_down` does (design note U5)
    if ready(m, b.cru, b.pmu, &super::rk3588_display::HDPTX0).is_ok() {
        f.phy_status = m.read32(b.phy.grf + hdptx::grf::STATUS);
    }
    match ready(m, b.cru, b.pmu, &VOP) {
        Ok(()) => f.vop_reachable = true,
        Err(NotReady::DomainOff(_)) => {}
        Err(n) => f.vop_unreadable = Some(n),
    }
    if f.vop_reachable {
        for p in 0..4 {
            f.dsp_ctrl[p] = m.read32(b.vop + vop2::vp::base(p) + vop2::vp::DSP_CTRL);
            if f.dsp_ctrl[p] & vop2::vp::DSP_CTRL_STANDBY == 0 {
                f.running |= 1 << p;
            }
        }
    }
    f
}

const PORTS: [Vp; 4] = [Vp::Vp0, Vp::Vp1, Vp::Vp2, Vp::Vp3];

/// Make a powered VOP2 readable (its gates open, its resets released, as
/// D4's [`VOP`] lists them; CRU writes only), then read what runs again.
fn find_opened(m: &impl Mmio, b: &Bases) -> Result<Found, Failed> {
    for c in VOP.clocks {
        gate_open(m, b.cru + c.part.offset(), c.gate).map_err(at(stage::START, Error::Gate))?;
    }
    for r in VOP.resets {
        reset_release(m, b.cru, *r);
    }
    Ok(find(m, b))
}

/// The ports this chain gives a pixel clock (DCLK stage): VP0.
const CLOCKED_HERE: u32 = 1 << 0;

/// Stop every running port (START with `force_cold`): every window off,
/// `REG_CFG_DONE` for each running port, then each one into STANDBY. A
/// port whose STANDBY does not come is checked for frames: none means no
/// pixel clock, so nothing of it fetches (R3-S1), and it is left.
fn stop_running(m: &impl Mmio, b: &Bases, env: &impl Env, f: &mut Found) -> Result<(), Failed> {
    let mut delay = |us| env.busy_wait_us(us);
    vop2::windows_off(m, b.vop, 0xf, None);
    for p in PORTS.iter().filter(|p| f.running & 1 << p.id() != 0) {
        vop2::cfg_done(m, b.vop, *p);
    }
    for p in PORTS.iter().filter(|p| f.running & 1 << p.id() != 0) {
        match vop2::standby(m, b.vop, *p, &mut delay) {
            Ok(_) => f.stopped |= 1 << p.id(),
            Err(e) if e.why == vop2::Refusal::StandbyTimeout => {
                if vop2::wait_frame(m, b.vop, *p, &mut delay).is_ok() {
                    return Err(Failed { stage: stage::START, error: Error::Standby(e) });
                }
                f.unclocked |= 1 << p.id();
            }
            Err(e) => return Err(Failed { stage: stage::START, error: Error::Standby(e) }),
        }
    }
    Ok(())
}

/// Power the chain off from any state (START with `power_cycle`, and
/// [`shut_down`]): the PHY off when it is locked and its APB reset
/// asserted (its register file back to its reset state), then PD_VO1,
/// PD_VO0, PD_VOP off. The caller has stopped VOP2's ports.
fn power_down(m: &impl Mmio, b: &Bases, env: &impl Env) -> Result<bool, DisplayError> {
    let host = PhyHost { m, cru: b.cru, env };
    let phy_on = ready(m, b.cru, b.pmu, &super::rk3588_display::HDPTX0).is_ok() && hdptx::power_off(m, &host, &b.phy);
    reset_assert(m, b.cru, reset::P_HDPTX0);
    display_off(m, b.cru, b.pmu, &mut |us| env.busy_wait_us(us))?;
    Ok(phy_on)
}

/// Bring the chain up on the buffer at `fb` (physical, below 4 GiB) with
/// `stride_bytes` per line. See the module text for the stages.
pub fn bring_up(
    m: &impl Mmio,
    b: &Bases,
    env: &impl Env,
    cfg: &Config,
    fb: u64,
    stride_bytes: u32,
) -> Result<Report, Failed> {
    use stage::*;
    let mut delay = |us| env.busy_wait_us(us);

    // 1: check, then read what runs
    let timing = vop2::vp_timing(&cfg.mode, Vp::Vp0).map_err(|_| Failed { stage: START, error: Error::Mode })?;
    dw::tmds_char_rate(&cfg.mode).map_err(|_| Failed { stage: START, error: Error::Mode })?;
    cfg.image.check().map_err(at(START, Error::Phy))?;
    if !cfg.plan.is_valid() || cfg.plan.rate_hz != u64::from(cfg.mode.pixel_clock_hz) {
        let found = cfg.plan.rate_hz.min(u64::from(u32::MAX)) as u32;
        return Err(Failed { stage: START, error: Error::Phy(hdptx::Refused { why: hdptx::Refusal::Rate, found }) });
    }
    if Port::of_base(b.hdmi) != Some(Port::Hdmi0) {
        let refused = dw::Refused { why: dw::Refusal::PortMismatch, found: b.hdmi as u32 };
        return Err(Failed { stage: START, error: Error::Hdmi(refused) });
    }
    let win = window(&cfg.mode, fb, stride_bytes);
    let active = (cfg.mode.hactive, cfg.mode.vactive);
    vop2::window_check(&win, active.0, active.1).map_err(at(START, Error::Vop))?;
    let mut found = find(m, b);
    ylog!(
        env,
        Inf,
        "t6 display: found VOP {} DSP_CTRL {:#x} {:#x} {:#x} {:#x}, PHY STATUS {:#x}",
        if found.vop_reachable { "on" } else { "off" },
        found.dsp_ctrl[0],
        found.dsp_ctrl[1],
        found.dsp_ctrl[2],
        found.dsp_ctrl[3],
        found.phy_status
    );
    // PD_VOP on but VOP2 not readable: a port may run. Only a power cycle
    // may go on, and it first makes VOP2 readable to stop what runs.
    if let Some(n) = found.vop_unreadable {
        if !cfg.power_cycle {
            return Err(Failed { stage: START, error: Error::VopUnreadable(n) });
        }
        found = Found { vop_unreadable: Some(n), ..find_opened(m, b)? };
    }
    if found.running != 0 {
        if !cfg.force_cold {
            return Err(Failed { stage: START, error: Error::FoundRunning(found.running) });
        }
        // A port this chain clocks again (DCLK stage) that runs no frame is
        // in the middle of one: a window disable or STANDBY written now
        // would stay pending and land only when its clock is back, with
        // paging still on. Only the power cycle (PD_VOP off) drops such a
        // state, so without it the port is refused before anything of it
        // is written (the event clear of the frame wait aside).
        if !cfg.power_cycle {
            for p in PORTS.iter().filter(|p| found.running & CLOCKED_HERE & 1 << p.id() != 0) {
                match vop2::wait_frame(m, b.vop, *p, &mut delay) {
                    Ok(_) => {}
                    Err(e) if e.why == vop2::Refusal::FrameTimeout => {
                        return Err(Failed { stage: START, error: Error::Unclocked(1 << p.id()) });
                    }
                    Err(e) => return Err(Failed { stage: START, error: Error::Vop(e) }),
                }
            }
        }
        stop_running(m, b, env, &mut found)?;
        ylog!(env, Inf, "t6 display: stopped ports {:#x}, no pixel clock on {:#x}", found.stopped, found.unclocked);
    }
    if cfg.power_cycle {
        found.phy_powered_off = power_down(m, b, env).map_err(at(START, Error::Display))?;
    }
    done(cfg, START)?;

    // 2: power, clocks, resets
    let power = display_on(m, b.cru, b.pmu, &mut delay).map_err(at(POWER, Error::Display))?;
    done(cfg, POWER)?;

    // 3: the VOP's HDMI0 words in the GRFs
    gate_open(m, b.cru + clk::PCLK_VO1GRF.part.offset(), clk::PCLK_VO1GRF.gate).map_err(at(GRF, Error::Gate))?;
    let sync = dw::vo1_con0_sync_word(Port::Hdmi0, dw::linux_sync_field(&cfg.mode));
    for (addr, word) in
        [(b.vop_grf + dw::grf::VOP_CON2, vop2::hdmi_vop_grf(HdmiPort::Hdmi0)), (b.vo1_grf + dw::grf::VO1_CON0, sync)]
    {
        m.write32(addr, word);
        let now = m.read32(addr);
        let mask = word >> 16;
        if now & mask != word & mask {
            return Err(Failed { stage: GRF, error: Error::Grf(addr, now) });
        }
    }
    done(cfg, GRF)?;

    // 4: the PHY
    let host = PhyHost { m, cru: b.cru, env };
    let phy = match hdptx::power_on(m, &host, &b.phy, &cfg.plan, &cfg.image) {
        Ok(p) => p,
        Err(e) => {
            ylog!(env, Err, "t6 display: PHY {:?}, STATUS {:#x}", e.why, e.found);
            return Err(Failed { stage: PHY, error: Error::Phy(e) });
        }
    };
    done(cfg, PHY)?;

    // 5: VP0's pixel clock from the PHY
    reset_release(m, b.cru, reset::D_VOP0);
    let mux = clk::DCLK_VOP0_MUX;
    let sel = b.cru + mux.offset();
    m.write32(sel, mux.mask() << 16 | clk::DCLK_VOP0_FROM_HDMIPHY0 << mux.shift);
    let now = m.read32(sel);
    if mux.get(now) != clk::DCLK_VOP0_FROM_HDMIPHY0 {
        return Err(Failed { stage: DCLK, error: Error::DclkMux(now) });
    }
    gate_open(m, b.cru + clk::DCLK_VOP0.part.offset(), clk::DCLK_VOP0.gate).map_err(at(DCLK, Error::Gate))?;
    done(cfg, DCLK)?;

    // 6: the controller
    let bases = dw::Bases { ctrl: b.hdmi, sys_grf: b.sys_grf, vo1_grf: b.vo1_grf };
    let ident = dw::bind(m, &bases, Port::Hdmi0, &cfg.bind).map_err(at(CONTROLLER, Error::Hdmi))?;
    dw::enable(m, b.hdmi, cfg.sink).map_err(at(CONTROLLER, Error::Hdmi))?;
    done(cfg, CONTROLLER)?;

    // 7: VOP2
    let vop = vop2::init(m, b.vop).map_err(at(VOP, Error::Vop))?;
    let iommu = rk_iommu::paging_off(m, &b.vop_mmu, &mut delay).map_err(at(VOP, Error::Iommu))?;
    if let Some(t) = iommu.timeout {
        ylog!(env, Wrn, "t6 display: vop_mmu set {} {:?}, STATUS {:#x}; paging is off", t.set, t.why, t.status);
    }
    vop2::windows_off(m, b.vop, 0xf, None);
    vop2::hdmi_route(m, b.vop, Vp::Vp0, HdmiPort::Hdmi0);
    vop2::vp_set_timing(m, b.vop, &timing);
    vop2::vp_start(m, b.vop, &timing);
    let overlay = vop2::overlay_single(m, b.vop, Vp::Vp0, Esmart::E0, &mut delay).map_err(at(VOP, Error::Vop))?;
    vop2::window_show(m, b.vop, Vp::Vp0, Esmart::E0, &win, active).map_err(at(VOP, Error::Vop))?;
    done(cfg, VOP)?;

    // 8: the read-backs
    let frame =
        vop2::frame_check(m, b.vop, Vp::Vp0, 2 * frame_us(&cfg.mode), &mut delay).map_err(at(VIDEO, Error::Vop))?;
    let read_back = vop2::read_back(m, b.vop, Vp::Vp0, Esmart::E0);
    let phy_status = hdptx::status(m, &b.phy);
    let hdmi_status = dw::status(m, b.hdmi, b.sys_grf, Port::Hdmi0);
    let mut failed = 0;
    for (bad, bit) in [
        (!frame.frame_start, video_check::NO_FRAME),
        (frame.underflow, video_check::UNDERFLOW),
        (frame.bus_error.contains(&true), video_check::BUS_ERROR),
        (read_back.yrgb_mst != fb as u32, video_check::ADDRESS),
        (!phy_status.up(), video_check::PHY),
        (!hdmi_status.clocks_locked(), video_check::CMU),
    ] {
        if bad {
            failed |= bit;
        }
    }
    if failed != 0 {
        ylog!(
            env,
            Err,
            "t6 display: video check: raw {:#x} bus {:?} MST {:#x} CFG_DONE {:#x} PHY {:#x} CMU {:#x}",
            frame.raw,
            frame.bus_error,
            read_back.yrgb_mst,
            read_back.cfg_done,
            phy_status.status,
            hdmi_status.cmu_status
        );
        return Err(Failed { stage: VIDEO, error: Error::Video(failed) });
    }
    // Whether CFG_DONE's port bit clears once the update is taken is the
    // design note's open question U8: an observation, not a verdict.
    if read_back.cfg_done & 1 << Vp::Vp0.id() != 0 {
        ylog!(
            env,
            Wrn,
            "t6 display: REG_CFG_DONE {:#x}: VP0's bit still set after two frames (U8)",
            read_back.cfg_done
        );
    }
    if !hdmi_status.hpd {
        ylog!(env, Wrn, "t6 display: no hot-plug level: no cable or no powered sink");
    }
    done(cfg, VIDEO)?;
    ylog!(
        env,
        Inf,
        "t6 display: up, {}x{} at {:#x}, PHY lock {} us, CMU {:#x}",
        cfg.mode.hactive,
        cfg.mode.vactive,
        fb,
        phy.lock_us,
        hdmi_status.cmu_status
    );
    Ok(Report { found, power, phy, ident, vop, iommu, overlay, frame, read_back, phy_status, hdmi_status })
}

/// Stop the picture and power the chain down: VP0's window off and VP0 in
/// STANDBY when it runs, then [`power_down`].
pub fn shut_down(m: &impl Mmio, b: &Bases, env: &impl Env) -> Result<(), Failed> {
    let mut found = find(m, b);
    if found.vop_unreadable.is_some() {
        found = find_opened(m, b)?;
    }
    if found.running != 0 {
        stop_running(m, b, env, &mut found)?;
    }
    power_down(m, b, env).map_err(at(stage::START, Error::Display))?;
    Ok(())
}

pub mod t6;

#[cfg(test)]
mod tests;
