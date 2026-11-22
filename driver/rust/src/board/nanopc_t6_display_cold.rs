// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Step D8 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md, section 2
//! route (c) and section 7 row D8): the display chain brought up from cold
//! by this image, after a normal kexec. Linux's shutdown has then put VP0 in
//! STANDBY, gated its pixel and bus clocks and turned the IOMMU off; PD_VO1
//! stays on and PD_VOP may (section 2 (a)). Nothing of that is adopted:
//! [`cold`] runs [`chain::bring_up`] with the T6's configuration
//! ([`chain::t6::config`]: 1024x768 at 60 Hz, DVI, the D0 dump's PHY image)
//! on the caller's buffer, which Esmart0 of VP0 then shows.
//!
//! What [`cold`] adds to the chain, all on the environment's log, so that a
//! board run that hangs still says how far it came:
//!
//! 1. Before the chain, read only: PD_VOP's PMU words, whether PD_VOP and
//!    PD_VO1 are powered and usable, whether each display block can be
//!    read ([`rk3588_display::ready`]), DCLK_VOP0's mux and, only when the
//!    PHY can be read, its GRF `STATUS` (U4 of the design note: is the PHY
//!    PLL still locked after a normal kexec).
//! 2. The chain itself logs one line per stage before the stage's first
//!    access, and what it found at START.
//! 3. After it: the stage that stopped and why, or, when it ran through,
//!    what each stage reported (the domains, the PHY's lock, the
//!    controller's identity, `SYS_PD_CTRL` before and after (U6), the
//!    IOMMU, the overlay, the video check).
//!
//! The record crossing to C ([`Cold`]) holds only the stop: the stage, the
//! kind of error and its numbers. The section 5 read-backs after the picture
//! is up are the adopt's ([`super::nanopc_t6_display::status`]): the same
//! port, window and mode.
//!
//! The caller draws the buffer ([`super::nanopc_t6_display::draw_pattern`])
//! and cleans it to the point of coherency first: the chain points Esmart0
//! at it in its VOP stage, and the VOP does not snoop the CPU's caches.

use super::nanopc_t6_display::{self as display, Bases};
use crate::env::Env;
use crate::ip::hdptx;
use crate::mmio::Mmio;
use crate::soc::rk3588::{PdRegs, PD_VO1, PD_VOP};
use crate::soc::rk3588_display::{self as rk3588_display, clk, DisplayError, NotReady};
use crate::soc::rk3588_display_chain::{self as chain, stage, Error, Failed, Report};
use crate::ylog;

/// The mapped (virtual) bases of the chain: the C side's `struct
/// yd_t6_display_cold_bases`. `display` is the adopt's (the read-backs use
/// it); its `cru` must also map the PMU1 CRU at +0x30000 (the PHY's
/// reference clock and three of its resets).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColdBases {
    pub display: Bases,
    pub vop_grf: usize,
    pub vo1_grf: usize,
    /// The HDPTX PHY0's register file (0x2000 bytes).
    pub phy: usize,
}

impl ColdBases {
    /// The chain's view of the same mappings.
    pub fn chain(&self) -> chain::Bases {
        let d = &self.display;
        chain::Bases {
            cru: d.cru,
            pmu: d.pmu,
            sys_grf: d.sys_grf,
            vop_grf: self.vop_grf,
            vo1_grf: self.vo1_grf,
            phy: hdptx::Bases { phy: self.phy, grf: d.phy_grf },
            hdmi: d.hdmi,
            vop: d.vop,
            vop_mmu: d.vop_mmu,
        }
    }
}

/// The `flags` of [`cold`] (`YD_T6_DISPLAY_COLD_*`).
pub mod flags {
    /// The chain's `power_cycle`: every block off and in reset first.
    pub const POWER_CYCLE: u32 = 1 << 0;
    /// The chain's `force_cold`: stop a port found running.
    pub const FORCE_COLD: u32 = 1 << 1;
    pub const ALL: u32 = POWER_CYCLE | FORCE_COLD;
}

/// The kind of [`chain::Error`] that stopped the chain. The value crosses
/// to C (`YD_T6_DISPLAY_COLD_ERROR_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ColdError {
    /// The chain ran through.
    #[default]
    None = 0,
    /// `stop_after` reached.
    Stopped = 1,
    Mode = 2,
    /// `refusal`: the `hdptx::Refusal`, `detail`: its word.
    Phy = 3,
    /// `refusal`: the `vop2::Refusal`, `detail`: its read-back.
    Vop = 4,
    /// `detail`: the ports found running (bit n = VPn).
    FoundRunning = 5,
    /// As [`ColdError::Vop`].
    Standby = 6,
    /// `index`: 1 a clock, 2 a domain (`refusal`: the `PdRefusal`), 3 a
    /// gate; `detail`: the word read.
    Display = 7,
    /// `index`: the GRF word's address (low 32 bits), `detail`: the word read.
    Grf = 8,
    /// `detail`: the CLKGATE_CON read.
    Gate = 9,
    /// `detail`: the CLKSEL_CON(112) read.
    DclkMux = 10,
    /// `refusal`: the `dw_hdmi_qp::Refusal`, `detail`: its word.
    Hdmi = 11,
    /// `refusal`: the `rk_iommu::Refusal`, `index`: the set, `detail`: its `STATUS`.
    Iommu = 12,
    /// `detail`: the `video_check` bits that failed.
    Video = 13,
    /// `index`: 1 the domain, 2 a gate, 3 a reset kept VOP2 unreadable.
    VopUnreadable = 14,
    /// `detail`: the ports without a pixel clock (bit n = VPn).
    Unclocked = 15,
}

/// What stopped [`cold`], in 32-bit words: the C side's `struct
/// yd_t6_display_cold`. All 0 when the chain ran through.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cold {
    /// The stage that stopped (`chain::stage`), 0 when none did.
    pub stage: u32,
    /// [`ColdError`], then its numbers.
    pub error: u32,
    pub refusal: u32,
    pub index: u32,
    pub detail: u32,
}

impl Cold {
    fn of(f: &Failed) -> Cold {
        let (error, refusal, index, detail) = match f.error {
            Error::Stopped => (ColdError::Stopped, 0, 0, 0),
            Error::Mode => (ColdError::Mode, 0, 0, 0),
            Error::Phy(r) => (ColdError::Phy, r.why as u32, 0, r.found),
            Error::Vop(r) => (ColdError::Vop, r.why as u32, 0, r.read_back),
            Error::FoundRunning(p) => (ColdError::FoundRunning, 0, 0, p),
            Error::Standby(r) => (ColdError::Standby, r.why as u32, 0, r.read_back),
            Error::Display(DisplayError::Clock(c)) => {
                let word = match c {
                    rk3588_display::ClockError::AclkVopNoRate(hz) => u32::try_from(hz).unwrap_or(u32::MAX),
                    rk3588_display::ClockError::ReadBack(_, w) => w,
                };
                (ColdError::Display, 0, 1, word)
            }
            Error::Display(DisplayError::Domain(e)) => (ColdError::Display, e.why as u32, 2, e.read),
            Error::Display(DisplayError::Gate(_, w)) => (ColdError::Display, 0, 3, w),
            Error::Grf(addr, w) => (ColdError::Grf, 0, addr as u32, w),
            Error::Gate(w) => (ColdError::Gate, 0, 0, w),
            Error::DclkMux(w) => (ColdError::DclkMux, 0, 0, w),
            Error::Hdmi(r) => (ColdError::Hdmi, r.why as u32, 0, r.found),
            Error::Iommu(r) => (ColdError::Iommu, r.why as u32, r.set as u32, r.status),
            Error::Video(bits) => (ColdError::Video, 0, 0, bits),
            Error::VopUnreadable(n) => {
                let kind = match n {
                    NotReady::DomainOff(_) => 1,
                    NotReady::Gated(_) => 2,
                    NotReady::InReset(_) => 3,
                };
                (ColdError::VopUnreadable, 0, kind, 0)
            }
            Error::Unclocked(p) => (ColdError::Unclocked, 0, 0, p),
        };
        Cold { stage: f.stage, error: error as u32, refusal, index, detail }
    }
}

/// Why [`cold`] did not start: its arguments, checked before anything is
/// read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A bit outside [`flags::ALL`].
    Flags(u32),
    /// A `stop_after` that is no bring-up stage (0 runs to the end).
    StopAfter(u32),
}

/// Bring the display up from cold on the buffer at physical `fb_pa` (the
/// mode's size, stride [`display::FB_STRIDE`]): see the module text.
/// `Ok(Cold)` with `stage` 0 when the chain ran through, else the stage
/// that stopped it (a `stop_after` is [`ColdError::Stopped`]).
pub fn cold(
    m: &impl Mmio,
    b: &ColdBases,
    env: &impl Env,
    flags: u32,
    stop_after: u32,
    fb_pa: u64,
) -> Result<Cold, Refused> {
    if flags & !flags::ALL != 0 {
        return Err(Refused::Flags(flags));
    }
    if stop_after > stage::VIDEO {
        return Err(Refused::StopAfter(stop_after));
    }
    before(m, b, env);
    let cfg = chain::t6::config(flags & flags::FORCE_COLD != 0, flags & flags::POWER_CYCLE != 0, stop_after);
    match chain::bring_up(m, &b.chain(), env, &cfg, fb_pa, display::FB_STRIDE) {
        Ok(r) => {
            report(env, &r);
            Ok(Cold::default())
        }
        Err(f) => {
            if f.error == Error::Stopped {
                ylog!(env, Inf, "t6 display: stopped after stage {} {} as asked", f.stage, stage::name(f.stage));
            } else {
                ylog!(env, Err, "t6 display: stage {} {} failed: {:?}", f.stage, stage::name(f.stage), f.error);
            }
            Ok(Cold::of(&f))
        }
    }
}

/// Step 1 of the module text: reads only, of the PMU, the CRU and (when the
/// PHY can be read) the PHY's GRF.
fn before(m: &impl Mmio, b: &ColdBases, env: &impl Env) {
    let d = &b.display;
    let vop = PdRegs::of(m, d.pmu, &PD_VOP);
    let vo1 = PdRegs::of(m, d.pmu, &PD_VO1);
    ylog!(
        env,
        Inf,
        "t6 display: before: PD_VOP PWR {:#010x} STATUS {:#010x} IDLE_REQ {:#010x} IDLE_ACK {:#010x} IDLE_ST {:#010x} REPAIR {:#010x}",
        vop.pwr_gate,
        vop.status,
        vop.idle_req,
        vop.idle_ack,
        vop.idle_st,
        vop.repair
    );
    ylog!(
        env,
        Inf,
        "t6 display: before: PD_VOP powered {} usable {}, PD_VO1 powered {} usable {} (IDLE_REQ {:#010x})",
        u32::from(vop.powered(&PD_VOP)),
        u32::from(vop.usable_of(&PD_VOP)),
        u32::from(vo1.powered(&PD_VO1)),
        u32::from(vo1.usable_of(&PD_VO1)),
        vo1.idle_req
    );
    for blk in &rk3588_display::BLOCKS {
        match rk3588_display::ready(m, d.cru, d.pmu, blk) {
            Ok(()) => ylog!(env, Inf, "t6 display: before: {} readable", blk.name),
            Err(n) => ylog!(env, Inf, "t6 display: before: {} not readable: {:?}", blk.name, n),
        }
    }
    let sel = m.read32(d.cru + clk::DCLK_VOP0_MUX.offset());
    let mux = clk::DCLK_VOP0_MUX.get(sel);
    if rk3588_display::ready(m, d.cru, d.pmu, &rk3588_display::HDPTX0).is_ok() {
        let status = m.read32(d.phy_grf + hdptx::grf::STATUS);
        ylog!(
            env,
            Inf,
            "t6 display: before: CLKSEL_CON(112) {:#010x} (DCLK_VOP0 mux {}), PHY GRF STATUS {:#010x}",
            sel,
            mux,
            status
        );
    } else {
        ylog!(env, Inf, "t6 display: before: CLKSEL_CON(112) {:#010x} (DCLK_VOP0 mux {}), PHY GRF not read", sel, mux);
    }
}

/// Step 3 of the module text for a chain that ran through.
fn report(env: &impl Env, r: &Report) {
    let f = &r.found;
    ylog!(
        env,
        Inf,
        "t6 display: START: running {:#x}, stopped {:#x}, no pixel clock {:#x}, PHY powered off {}",
        f.running,
        f.stopped,
        f.unclocked,
        u32::from(f.phy_powered_off)
    );
    let (v, o) = (&r.power.vop, &r.power.vo1);
    ylog!(
        env,
        Inf,
        "t6 display: POWER: ACLK_VOP set {}, PD_VOP was on {} waited {}/{}/{} us, PD_VO1 was on {} waited {}/{}/{} us",
        u32::from(r.power.aclk_vop_set),
        u32::from(v.was_on),
        v.waited_us[0],
        v.waited_us[1],
        v.waited_us[2],
        u32::from(o.was_on),
        o.waited_us[0],
        o.waited_us[1],
        o.waited_us[2]
    );
    ylog!(
        env,
        Inf,
        "t6 display: PHY: locked after {} us, CON0 {:#010x} STATUS {:#010x}",
        r.phy.lock_us,
        r.phy_status.con0,
        r.phy_status.status
    );
    let i = &r.ident;
    ylog!(
        env,
        Inf,
        "t6 display: CONTROLLER: CORE_ID {:#010x} VER_NUMBER {:#010x} VER_TYPE {:#010x} CONFIG {:#010x}",
        i.core_id,
        i.ver_number,
        i.ver_type,
        i.config
    );
    let s = &r.iommu;
    ylog!(
        env,
        Inf,
        "t6 display: VOP: VERSION_INFO {:#010x} SYS_PD_CTRL {:#010x} -> {:#010x}, vop_mmu STATUS {:#010x} {:#010x} -> {:#010x} {:#010x}",
        r.vop.version,
        r.vop.pd_found,
        r.vop.pd_now,
        s.status_found[0],
        s.status_found[1],
        s.status_now[0],
        s.status_now[1]
    );
    ylog!(
        env,
        Inf,
        "t6 display: VOP: overlay OVL_LAYER_SEL {:#010x} OVL_PORT_SEL {:#010x}, in force after {} us",
        r.overlay.layer_sel,
        r.overlay.port_sel,
        r.overlay.waited_us
    );
    let (fc, rb, h) = (&r.frame, &r.read_back, &r.hdmi_status);
    ylog!(
        env,
        Inf,
        "t6 display: VIDEO: frame start {} underflow {} bus error {} {}, YRGB_MST {:#010x}, REG_CFG_DONE {:#010x}, CMU_STATUS {:#010x}, HPD {}",
        u32::from(fc.frame_start),
        u32::from(fc.underflow),
        u32::from(fc.bus_error[0]),
        u32::from(fc.bus_error[1]),
        rb.yrgb_mst,
        rb.cfg_done,
        h.cmu_status,
        u32::from(h.hpd)
    );
}

#[cfg(test)]
mod tests;
