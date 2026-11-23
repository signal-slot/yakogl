// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Steps D2 and D3 on the NanoPC-T6 (driver/docs/nanopc-t6-display.md,
//! section 2 route (a), section 4 and section 7 rows D2 and D3): adopt the
//! display a crash kexec left running, then show the GPU's window buffers on
//! it. Step D2: Linux 6.18 scans its frame buffer on VP0 through the VOP's
//! IOMMU, in Cluster0 (linear XRGB8888; board dump of 2026-11-12: Cluster0
//! `WIN_CTRL0` and `CLUSTER_CTRL` = 1, Esmart0 off, VP1-VP3 in STANDBY).
//! The adopt keeps Linux's power, clocks, PHY, HDMI controller and VP0
//! timing, and changes only the VOP's windows and its IOMMU: Cluster0 off,
//! paging off, Esmart0 VP0's only layer, showing a buffer of this image.
//!
//! [`adopt`] runs, in this order, and stops at the first step that refuses
//! ([`Stop`] names it):
//!
//! 1. Read what was left, writing nothing: PD_VOP powered and out of idle
//!    (`soc::rk3588::PdRegs`; a VOP register read with its domain off may
//!    hang the bus, U5), the VOP's CRU gates open (PD_VOP's clocks and
//!    HCLK_VOP), `VERSION_INFO`, VP0 running (STANDBY clear) with
//!    `DSP_CTRL` exactly what the build would write for HDMI RGB and
//!    `DSP_BG` black, VP0's four timing words those of one of [`MODES`],
//!    HDMI0 enabled and fed by VP0 (`DSP_IF_EN`), the Esmart windows'
//!    internal power on (`SYS_PD_CTRL` bit 7). Anything else is a state this
//!    code does not know, and it refuses by name.
//! 2. `vop2::take_over` keeping Esmart0: every window off, `REG_CFG_DONE`
//!    on the running ports, one frame start on each.
//! 3. `rk_iommu::paging_off` on both `vop_mmu` sets; a set that keeps paging
//!    is [`Stop::Iommu`].
//! 4. `vop2::overlay_single`: Esmart0 the only, bottom layer of VP0.
//! 5. `vop2::window_show`: Esmart0 on the caller's window, inside
//!    [`WINDOW_AREA`] at the screen's origin (the probe's window is
//!    [`probe_window`]); `REG_CFG_DONE` for VP0 (bit 15 | bit 16 | bit 0).
//! 6. One frame start, then the read-back of section 5's "our buffer is the
//!    one shown": `YRGB_MST`, `REGION0_CTRL` enable, the two overlay selects.
//!
//! The caller draws [`draw_pattern`] into the buffer and cleans it to the
//! point of coherency before [`adopt`] (the VOP does not snoop the CPU's
//! caches). [`status`] is section 5's other read-backs, all reads: the frame
//! start, underflow and bus error bits over a wait, the frame interval
//! against the caller's clock, the VOP's registers, the HPD level, the PHY
//! GRF status and, only when PD_VO1 is usable and PCLK_HDMITX0 is open, the
//! HDMI controller's clock monitor and video monitor words.
//!
//! Step D3 is the board layer's `board_show()` and `board_display_busy()`
//! over a [`Screen`] the caller keeps. The first [`show`] is the adopt, with
//! the window the GL side renders into, in place at the screen's origin: no
//! copy. The window keeps its own size, inside [`WINDOW_AREA`] in every
//! accepted mode. A later [`show`] of another buffer is `vop2::window_flip` (the
//! address, then `REG_CFG_DONE`) and returns without waiting for the frame
//! start that takes it; until then the display scans an earlier buffer, and
//! [`busy`] says which buffers that may be. The geometry is the first show's:
//! only the address is one register, taken whole at a frame start, so only
//! an address change cannot tear.
//!
//! Nothing here touches the GPU, the interrupt controller, the PHY or the
//! HDMI controller's registers beyond those reads.

use crate::errno::{ENODEV, ENOTSUP};
use crate::ip::dw_hdmi_qp;
use crate::ip::hdptx;
use crate::ip::rk_iommu;
use crate::ip::vop2::{self, cluster, esmart, ovl, sys, vp, Esmart, Format, HdmiPort, Vp, Window};
use crate::mmio::Mmio;
use crate::soc::rk3588::{clkgate_con, PdRegs, PD_VO1, PD_VOP};
use crate::soc::rk3588_display::clk as display_clk;
use crate::video::VideoMode;

/// 1024x768 at 60 Hz, 65 MHz, H 1024 1048 1184 1344, V 768 771 777 806,
/// both syncs negative (VESA DMT): what Linux runs on a display without an
/// EDID (driver/docs/nanopc-t6-display-dump-linux.txt).
pub const MODE_1024X768: VideoMode = VideoMode {
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

/// 1920x1080 at 60 Hz, 148.5 MHz, H 1920 2008 2052 2200, V 1080 1084 1089
/// 1125, both syncs positive (CEA-861 VIC 16): what Linux runs on a sink
/// whose EDID prefers it (driver/docs/nanopc-t6-display-dump-linux-1920x1080.txt).
pub const MODE_1920X1080: VideoMode = VideoMode {
    pixel_clock_hz: 148_500_000,
    hactive: 1920,
    hfront: 88,
    hsync: 44,
    hback: 148,
    vactive: 1080,
    vfront: 4,
    vsync: 5,
    vback: 36,
    hsync_high: true,
    vsync_high: true,
    de_high: true,
};

/// The modes the adopt accepts: VP0's timing must be one of them, else it
/// stops at [`Stop::Mode`]. The adopt keeps the mode it finds and never
/// writes a timing word. The cold bring-up (step D8) has its own mode
/// (`soc::rk3588_display_chain::t6`), since it programs the PHY itself.
pub const MODES: [VideoMode; 2] = [MODE_1024X768, MODE_1920X1080];

/// Where a window may lie: the screen's top left 1024x768, the smallest
/// active area of [`MODES`]. A window that fits is shown the same in every
/// accepted mode; the window's size never depends on the mode found. On a
/// larger screen the rest is VP0's background colour `DSP_BG`, which Linux
/// sets to 0 (black) when it starts the port (`ip::vop2` fact 8) and which
/// step 1 checks.
pub const WINDOW_AREA: (u32, u32) = (1024, 768);

/// The port, the window and the HDMI output the adopt uses.
pub const PORT: Vp = Vp::Vp0;
pub const WINDOW: Esmart = Esmart::E0;
pub const HDMI: HdmiPort = HdmiPort::Hdmi0;

/// The probes' frame buffer (`t6_display`, `t6_cold`; tools/probes/
/// nanopc_t6/common/t6_picture.h): 1024x768 XRGB8888. Its size is the
/// probes', not the mode's: it fills [`WINDOW_AREA`], and the cold
/// bring-up's mode has the same size.
pub const FB_WIDTH: u32 = 1024;
pub const FB_HEIGHT: u32 = 768;
/// The frame buffer's row stride in XRGB8888, a multiple of 64 bytes.
pub const FB_STRIDE: u32 = FB_WIDTH * 4;
/// The frame buffer's size in bytes.
pub const FB_BYTES: usize = FB_STRIDE as usize * FB_HEIGHT as usize;

/// Physical bases of the blocks (rk3588-base.dtsi; driver/docs/
/// nanopc-t6-display.md section 1). The VOP's registers below 0x2000 are
/// the only ones used; the two `vop_mmu` sets share one 4 KiB page.
pub const VOP_PHYS: usize = 0xfdd9_0000;
pub const VOP_MAP: usize = 0x2000;
pub const VOP_MMU_PAGE_PHYS: usize = 0xfdd9_7000;
pub const VOP_MMU_SETS: [usize; 2] = [0xe00, 0xf00];
pub const PMU_PHYS: usize = 0xfd8d_8000;
pub const CRU_PHYS: usize = crate::soc::rk3588::CRU_BASE;
pub const SYS_GRF_PHYS: usize = dw_hdmi_qp::SYS_GRF_BASE;
pub const PHY_GRF_PHYS: usize = hdptx::GRF0_BASE;
pub const HDMI_PHYS: usize = dw_hdmi_qp::TX0_BASE;

/// The colours of the pattern (XRGB8888: 0x00RRGGBB, bytes B, G, R, X in
/// memory). With R and B swapped the quadrants read blue, green, red, cyan:
/// a person tells the byte order from the picture.
pub const RED: u32 = 0x00ff_0000;
pub const GREEN: u32 = 0x0000_ff00;
pub const BLUE: u32 = 0x0000_00ff;
pub const YELLOW: u32 = 0x00ff_ff00;
pub const WHITE: u32 = 0x00ff_ffff;

/// The pattern of driver/docs/nanopc-t6-display.md section 5: four
/// quadrants, red top left, green top right, blue bottom left, yellow
/// bottom right, and a white border one pixel wide. `fb` holds `height`
/// rows of `stride_px` pixels; nothing past a row's `width` is written.
/// Returns false (nothing written) when `fb` is too short or `width` is
/// wider than the stride.
pub fn draw_pattern(fb: &mut [u32], width: usize, height: usize, stride_px: usize) -> bool {
    if width > stride_px || height == 0 || fb.len() < stride_px * (height - 1) + width {
        return false;
    }
    for y in 0..height {
        let row = &mut fb[y * stride_px..y * stride_px + width];
        for (x, px) in row.iter_mut().enumerate() {
            let edge = x == 0 || y == 0 || x == width - 1 || y == height - 1;
            *px = match (edge, x < width / 2, y < height / 2) {
                (true, _, _) => WHITE,
                (false, true, true) => RED,
                (false, false, true) => GREEN,
                (false, true, false) => BLUE,
                (false, false, false) => YELLOW,
            };
        }
    }
    true
}

/// The mapped (virtual) bases [`adopt`] and [`status`] use: the C side's
/// `struct yd_t6_display_bases`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bases {
    pub vop: usize,
    /// The two `vop_mmu` register sets.
    pub vop_mmu: [usize; 2],
    pub pmu: usize,
    pub cru: usize,
    pub sys_grf: usize,
    pub phy_grf: usize,
    pub hdmi: usize,
}

/// Which step stopped [`adopt`]. The value crosses to C
/// (`YD_T6_DISPLAY_STOP_*`). Steps up to [`Stop::EsmartPower`] write
/// nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Stop {
    /// Every step ran; Esmart0 shows the buffer.
    #[default]
    None = 0,
    /// PD_VOP is not powered or not out of idle (`detail`: the PMU repair
    /// status). The VOP was not read.
    VopDomain = 1,
    /// A clock of PD_VOP, or HCLK_VOP, is gated (`index`: its bit in
    /// CLKGATE_CON(52), `detail`: the word). The VOP was not read.
    VopClock = 2,
    /// `VERSION_INFO` is not the RK3588's (`detail`).
    Version = 3,
    /// VP0 is in STANDBY: nothing scans out to adopt (`detail`: `DSP_CTRL`).
    Standby = 4,
    /// VP0's `DSP_CTRL` is not what the build writes for HDMI RGB (`index`
    /// 0), or its background `DSP_BG` is not black (`index` 1); `detail`:
    /// the word read.
    VpState = 5,
    /// VP0's timing is none of [`MODES`] (`index`: the first word that
    /// differs from the mode that matches the most words in order, 0
    /// `DSP_HTOTAL_HS_END`, 1 `DSP_HACT_ST_END`, 2 `DSP_VTOTAL_VS_END`, 3
    /// `DSP_VACT_ST_END`; `detail`: that word as read).
    Mode = 6,
    /// HDMI0 is not enabled or not fed by VP0 (`detail`: `DSP_IF_EN`).
    Route = 7,
    /// The Esmart windows' VOP-internal power is off (`detail`:
    /// `SYS_PD_CTRL`).
    EsmartPower = 8,
    /// `vop2::take_over` refused or timed out (`refusal`: the
    /// `vop2::Refusal`, `detail`: its read-back).
    TakeOver = 9,
    /// A `vop_mmu` set keeps paging (`refusal`: the `rk_iommu::Refusal`,
    /// `index`: the set, `detail`: its `STATUS`).
    Iommu = 10,
    /// `vop2::overlay_single` refused (`refusal`, `detail`).
    Overlay = 11,
    /// `vop2::window_show` refused the buffer (`refusal`).
    Window = 12,
    /// No frame start after the window's `REG_CFG_DONE` (`refusal`,
    /// `detail`).
    Frame = 13,
    /// A register reads other than written after that frame (`index`: 0
    /// `YRGB_MST`, 1 `REGION0_CTRL` enable, 2 `OVL_PORT_SEL`, 3
    /// `OVL_LAYER_SEL`; `detail`: the value read).
    Shown = 14,
}

/// What [`adopt`] read and did, in 32-bit words only: the C side's `struct
/// yd_t6_display_adopt`. Fields after the step that stopped stay 0.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Adopt {
    /// [`Stop`], the refusal number of that step, which word or set, and the
    /// value it is about.
    pub stop: u32,
    pub refusal: u32,
    pub index: u32,
    pub detail: u32,

    /// As found. PD_VOP's PMU words (PWR, STATUS, IDLE_REQ, IDLE_ACK,
    /// IDLE_ST, REPAIR) and CLKGATE_CON(52).
    pub pd_vop: [u32; 6],
    pub cru_gate52: u32,
    pub version: u32,
    /// `DSP_CTRL` of VP0-VP3.
    pub dsp_ctrl: [u32; 4],
    /// VP0's `DSP_HTOTAL_HS_END`, `DSP_HACT_ST_END`, `DSP_VTOTAL_VS_END`,
    /// `DSP_VACT_ST_END`.
    pub timing: [u32; 4],
    pub dsp_if_en: u32,
    pub pd_ctrl: u32,
    pub layer_sel: u32,
    pub port_sel: u32,
    /// Cluster0's `WIN_CTRL0` and `CLUSTER_CTRL`; Esmart0's `REGION0_CTRL`.
    pub cluster0: [u32; 2],
    pub esmart0_ctrl: u32,
    /// Each `vop_mmu` set's `STATUS` and `DTE_ADDR`.
    pub mmu_status: [u32; 2],
    pub mmu_dte: [u32; 2],

    /// The take-over: the ports found running (bit n = VPn) and the windows
    /// turned off (`vop2::WindowMask`).
    pub running: u32,
    pub off: u32,
    /// The IOMMU: each set's `STATUS` at the end, and the `rk_iommu::Refusal`
    /// of a stall time-out that did not keep paging on (0 none).
    pub mmu_status_now: [u32; 2],
    pub mmu_timeout: u32,
    /// The overlay: `OVL_LAYER_SEL` and `OVL_PORT_SEL` written, µs until the
    /// port select was in force.
    pub ovl_layer_sel: u32,
    pub ovl_port_sel: u32,
    pub ovl_waited_us: u32,
    /// The window: the buffer's physical address and stride.
    pub fb: u32,
    pub stride: u32,
    /// `REG_CFG_DONE` read right after the window's write and after the next
    /// frame start (U8: whether VP0's bit clears when taken), and the µs
    /// waited for that frame start.
    pub cfg_done_written: u32,
    pub cfg_done_after: u32,
    pub frame_waited_us: u32,
    /// After that frame: Esmart0's `YRGB_MST` and `REGION0_CTRL`,
    /// `OVL_PORT_SEL`, `OVL_LAYER_SEL`.
    pub shown: [u32; 4],
}

impl Adopt {
    fn stopped(mut self, stop: Stop, refusal: u32, index: u32, detail: u32) -> Adopt {
        self.stop = stop as u32;
        self.refusal = refusal;
        self.index = index;
        self.detail = detail;
        self
    }
}

/// The window the adopt probe shows: its frame buffer ([`FB_WIDTH`] x
/// [`FB_HEIGHT`] XRGB8888, [`FB_BYTES`] at stride [`FB_STRIDE`]) at physical
/// `fb_pa`, at the screen's origin, whatever the mode.
pub fn probe_window(fb_pa: u64) -> Window {
    Window {
        addr: fb_pa,
        width: FB_WIDTH,
        height: FB_HEIGHT,
        stride_bytes: FB_STRIDE,
        x: 0,
        y: 0,
        format: Format::Xrgb8888,
    }
}

/// The adopt (see the module). `w` is the window Esmart0 shows at the end,
/// its buffer drawn and cleaned by the caller.
pub fn adopt(m: &impl Mmio, b: &Bases, w: &Window, delay_us: &mut impl FnMut(u32)) -> Adopt {
    let mut a = Adopt::default();
    let v = b.vop;

    // 1. what was left, read only
    let pd = PdRegs::of(m, b.pmu, &PD_VOP);
    a.pd_vop = [pd.pwr_gate, pd.status, pd.idle_req, pd.idle_ack, pd.idle_st, pd.repair];
    if !pd.usable_of(&PD_VOP) {
        return a.stopped(Stop::VopDomain, 0, 0, pd.repair);
    }
    // PD_VOP's clocks (soc::rk3588::PD_VOP) and the VOP's own register
    // clock HCLK_VOP (52/8, which also clocks the vop_mmu: rk3588-base.dtsi
    // :1428-1437), all in CLKGATE_CON(52); checked before any VOP or vop_mmu read
    a.cru_gate52 = m.read32(b.cru + clkgate_con(52));
    for g in PD_VOP.clocks.iter().chain([&display_clk::HCLK_VOP.gate]) {
        let word = if g.con == 52 { a.cru_gate52 } else { m.read32(b.cru + clkgate_con(g.con)) };
        if word & 1 << g.bit != 0 {
            return a.stopped(Stop::VopClock, 0, g.bit, word);
        }
    }
    a.version = vop2::version(m, v);
    if a.version != vop2::VERSION_RK3588 {
        return a.stopped(Stop::Version, 0, 0, a.version);
    }
    for (p, w) in a.dsp_ctrl.iter_mut().enumerate() {
        *w = m.read32(v + vp::base(p) + vp::DSP_CTRL);
    }
    let vb = v + vp::base(PORT.id());
    a.timing = [vp::DSP_HTOTAL_HS_END, vp::DSP_HACT_ST_END, vp::DSP_VTOTAL_VS_END, vp::DSP_VACT_ST_END]
        .map(|off| m.read32(vb + off));
    a.dsp_if_en = m.read32(v + sys::DSP_IF_EN);
    a.pd_ctrl = m.read32(v + sys::PD_CTRL);
    a.layer_sel = m.read32(v + ovl::LAYER_SEL);
    a.port_sel = m.read32(v + ovl::PORT_SEL);
    let c0 = v + vop2::Cluster::C0.base();
    a.cluster0 = [m.read32(c0 + cluster::WIN_CTRL0), m.read32(c0 + cluster::CTRL)];
    a.esmart0_ctrl = m.read32(v + WINDOW.base() + esmart::REGION0_CTRL);
    for i in 0..2 {
        a.mmu_status[i] = m.read32(b.vop_mmu[i] + rk_iommu::STATUS);
        a.mmu_dte[i] = m.read32(b.vop_mmu[i] + rk_iommu::DTE_ADDR);
    }

    let dsp = a.dsp_ctrl[PORT.id()];
    if dsp & vp::DSP_CTRL_STANDBY != 0 {
        return a.stopped(Stop::Standby, 0, 0, dsp);
    }
    // MODES are constants the tests pin: their words always exist. DSP_CTRL
    // depends on the port only, so every mode gives the same.
    let mut want = [[0; 4]; MODES.len()];
    let mut dsp_want = 0;
    for (w, mode) in want.iter_mut().zip(&MODES) {
        let Ok(t) = vop2::vp_timing(mode, PORT) else {
            return a.stopped(Stop::Mode, 0, 0, 0);
        };
        *w = [t.htotal_hs_end, t.hact_st_end, t.vtotal_vs_end, t.vact_st_end];
        dsp_want = t.dsp_ctrl;
    }
    if dsp != dsp_want {
        return a.stopped(Stop::VpState, 0, 0, dsp);
    }
    // outside the window the screen shows VP0's background
    let bg = m.read32(vb + vp::DSP_BG);
    if bg != 0 {
        return a.stopped(Stop::VpState, 0, 1, bg);
    }
    // how many words, in order, each mode matches; 4 is the whole mode
    let matched = want.map(|w| (0..4).find(|&i| a.timing[i] != w[i]).unwrap_or(4));
    let best = matched.into_iter().max().unwrap_or(0);
    if best < 4 {
        return a.stopped(Stop::Mode, 0, best as u32, a.timing[best]);
    }
    let route = vop2::hdmi_if(PORT, HDMI);
    if a.dsp_if_en & route.if_en_mask != route.if_en {
        return a.stopped(Stop::Route, 0, 0, a.dsp_if_en);
    }
    if a.pd_ctrl & sys::PD_ESMART != 0 {
        return a.stopped(Stop::EsmartPower, 0, 0, a.pd_ctrl);
    }

    // 2. Linux's windows off, taken at a frame boundary
    match vop2::take_over(m, v, Some(WINDOW), delay_us) {
        Ok(o) => {
            a.running = o.running;
            a.off = o.off;
        }
        Err(f) => {
            a.running = f.running;
            a.off = f.off;
            return a.stopped(Stop::TakeOver, f.refused.why as u32, 0, f.refused.read_back);
        }
    }

    // 3. nothing fetches through Linux's page tables any more
    match rk_iommu::paging_off(m, &b.vop_mmu, delay_us) {
        Ok(o) => {
            a.mmu_status_now = o.status_now;
            a.mmu_timeout = o.timeout.map_or(0, |r| r.why as u32);
        }
        Err(r) => {
            for i in 0..2 {
                a.mmu_status_now[i] = m.read32(b.vop_mmu[i] + rk_iommu::STATUS);
            }
            return a.stopped(Stop::Iommu, r.why as u32, r.set as u32, r.status);
        }
    }

    // 4. Esmart0 the only layer of VP0
    match vop2::overlay_single(m, v, PORT, WINDOW, delay_us) {
        Ok(o) => {
            a.ovl_layer_sel = o.layer_sel;
            a.ovl_port_sel = o.port_sel;
            a.ovl_waited_us = o.waited_us;
        }
        Err(r) => return a.stopped(Stop::Overlay, r.why as u32, 0, r.read_back),
    }

    // 5. our buffer
    if let Err(r) = vop2::window_show(m, v, PORT, WINDOW, w, WINDOW_AREA) {
        return a.stopped(Stop::Window, r.why as u32, 0, r.read_back);
    }
    // window_show refuses an address at or above 4 GiB
    a.fb = w.addr as u32;
    a.stride = w.stride_bytes;
    a.cfg_done_written = m.read32(v + sys::REG_CFG_DONE);

    // 6. one frame later the update is in force
    match vop2::wait_frame(m, v, PORT, delay_us) {
        Ok(us) => a.frame_waited_us = us,
        Err(r) => return a.stopped(Stop::Frame, r.why as u32, 0, r.read_back),
    }
    a.cfg_done_after = m.read32(v + sys::REG_CFG_DONE);
    let e0 = v + WINDOW.base();
    a.shown = [
        m.read32(e0 + esmart::REGION0_YRGB_MST),
        m.read32(e0 + esmart::REGION0_CTRL),
        m.read32(v + ovl::PORT_SEL),
        m.read32(v + ovl::LAYER_SEL),
    ];
    let ok = [
        a.shown[0] == a.fb,
        a.shown[1] & esmart::R0_ENABLE != 0,
        a.shown[2] == a.ovl_port_sel,
        a.shown[3] == a.ovl_layer_sel,
    ];
    if let Some(i) = ok.iter().position(|o| !o) {
        return a.stopped(Stop::Shown, 0, i as u32, a.shown[i]);
    }
    a
}

/// [`Screen::state`]: no show yet.
pub const SCREEN_NEW: u32 = 0;
/// [`Screen::state`]: the first show's adopt ran to its end; Esmart0 shows
/// this image's buffers.
pub const SCREEN_SHOWN: u32 = 1;
/// [`Screen::state`]: the first show's adopt stopped. Final: every later
/// show is refused. Any value other than the three is treated as this one.
pub const SCREEN_STOPPED: u32 = 2;

/// What [`show`] and [`busy`] keep between calls: the C side's `struct
/// yd_t6_screen`, all zero before the first show.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    /// The buffer of the last show that reached the VOP (the first show's
    /// when its adopt stopped): physical address.
    pub last: u64,
    /// Buffers shown before `last` that the display may still scan: the
    /// first `n_held` entries.
    pub held: [u64; 2],
    /// [`SCREEN_NEW`], [`SCREEN_SHOWN`] or [`SCREEN_STOPPED`].
    pub state: u32,
    /// The entries of `held` in use. 3: more buffers were shown inside one
    /// frame than `held` keeps, and every buffer counts as busy.
    pub n_held: u32,
    /// The first show's width, height and stride in bytes.
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// Shows that changed the address since the first one (wraps).
    pub flips: u32,
}

/// Show the `width` x `height` XRGB8888 buffer at physical `addr` (rows
/// `stride_bytes` apart, top row first) at the screen's origin. The first
/// call is [`adopt`] with this window, its record in `a`; a later call with
/// another address is a flip, which never waits (`a` is left as it is).
/// Returns 0, or
/// - `-ENOTSUP`: a window outside [`WINDOW_AREA`] or one the VOP cannot scan
///   (`vop2::window_check`), or one whose size or stride is not the first
///   show's. Nothing was read or
///   written; the screen stays as it was.
/// - `-ENODEV`: the adopt stopped (`a.stop` names the step), now or at the
///   first show. A screen that stopped is not tried again.
#[allow(clippy::too_many_arguments)]
pub fn show(
    m: &impl Mmio,
    b: &Bases,
    s: &mut Screen,
    addr: u64,
    width: u32,
    height: u32,
    stride_bytes: u32,
    delay_us: &mut impl FnMut(u32),
    a: &mut Adopt,
) -> i32 {
    if s.state != SCREEN_NEW && s.state != SCREEN_SHOWN {
        return -ENODEV;
    }
    let w = Window { addr, width, height, stride_bytes, x: 0, y: 0, format: Format::Xrgb8888 };
    if vop2::window_check(&w, WINDOW_AREA.0, WINDOW_AREA.1).is_err() {
        return -ENOTSUP;
    }
    if s.state == SCREEN_NEW {
        *a = adopt(m, b, &w, delay_us);
        let state = if a.stop == Stop::None as u32 { SCREEN_SHOWN } else { SCREEN_STOPPED };
        *s = Screen { last: addr, state, width, height, stride: stride_bytes, ..Screen::default() };
        return if state == SCREEN_SHOWN { 0 } else { -ENODEV };
    }
    if (width, height, stride_bytes) != (s.width, s.height, s.stride) {
        return -ENOTSUP;
    }
    if addr == s.last {
        return 0;
    }
    settle(m, b, s);
    // the check above is window_flip's own: it writes or refuses as a whole
    if vop2::window_flip(m, b.vop, PORT, WINDOW, &w, WINDOW_AREA).is_err() {
        return -ENOTSUP;
    }
    // The display scans `last`, or a buffer already held, until a frame
    // start takes this address. The frame start flag is cleared after
    // REG_CFG_DONE, so a flag set later is a frame start after the commit
    // (see settle).
    let n = s.n_held as usize;
    if n <= s.held.len() && !s.held[..n].contains(&s.last) {
        if n < s.held.len() {
            s.held[n] = s.last;
        }
        s.n_held += 1;
    }
    vop2::vp_clear(m, b.vop, PORT, sys::INT_FS);
    s.last = addr;
    s.flips = s.flips.wrapping_add(1);
    0
}

/// Which of the buffers at `addrs` the display may still scan: bit k for
/// `addrs[k]`, of the first 32. Too large is possible (a buffer is freed up
/// to one frame late), too small never. Never waits; writes no register.
/// - Before the first show: none.
/// - After an adopt that stopped: the buffer of that show (the adopt may
///   have stopped after it pointed Esmart0 at it). No register is read.
/// - Else the last buffer shown and the ones held since the last frame
///   start that took a flip (see `settle`).
pub fn busy(m: &impl Mmio, b: &Bases, s: &mut Screen, addrs: &[u64]) -> u32 {
    if s.state == SCREEN_NEW {
        return 0;
    }
    if s.state == SCREEN_SHOWN {
        settle(m, b, s);
    }
    let n = s.n_held as usize;
    let all = n > s.held.len();
    let mut bits = 0;
    for (k, a) in addrs.iter().take(32).enumerate() {
        if all || *a == s.last || s.held[..n].contains(a) {
            bits |= 1 << k;
        }
    }
    bits
}

/// Forget the held buffers once the display scans `last`: VP0 flagged a
/// frame start since the last flip cleared the flag, and VP0's bit of
/// `REG_CFG_DONE` reads 0. Linux tests the same two things before it
/// completes a flip (v6.18 rockchip_drm_vop2.c, `vop2_isr`: on `VP_INT_FS`
/// it reads `REG_CFG_DONE` and sends the event only when the port's bit is
/// clear). Whether the bit clears at the frame start is U8 of the design
/// note, which the adopt records on the board. Each answer is safe here: a
/// bit that always reads 0 leaves the frame start alone to decide, which is
/// enough because the flag is cleared after the `REG_CFG_DONE` write; a bit
/// that never clears keeps the buffers held, which EGL reports once as its
/// wait for a free buffer.
fn settle(m: &impl Mmio, b: &Bases, s: &mut Screen) {
    if s.n_held != 0 && vop2::vp_raw_status(m, b.vop, PORT) & sys::INT_FS != 0 && !vop2::cfg_pending(m, b.vop, PORT) {
        s.n_held = 0;
    }
}

/// What [`status`] read: section 5 of driver/docs/nanopc-t6-display.md, in
/// 32-bit words only (the C side's `struct yd_t6_display_status`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The `vop2::Refusal` of the frame check or of a frame wait, 0 none;
    /// and its read-back.
    pub refusal: u32,
    pub detail: u32,
    /// Over the check's wait: a frame start (1), an underflow (1), a bus
    /// error on `SYS0` and `SYS1` (1), and `VP_INT_RAW_STATUS` at its end.
    pub frame_start: u32,
    pub underflow: u32,
    pub bus_error: [u32; 2],
    pub raw: u32,
    /// The frame starts timed and the µs between the first and the last on
    /// the caller's clock.
    pub frames: u32,
    pub frames_us: u32,
    /// `vop2::ReadBack` of VP0 and Esmart0, in its field order: version,
    /// `SYS_PD_CTRL`, `REG_CFG_DONE`, `DSP_IF_EN`, `DSP_CTRL`,
    /// `DSP_HTOTAL_HS_END`, `DSP_VTOTAL_VS_END`, `OVL_PORT_SEL`,
    /// `OVL_LAYER_SEL`, `REGION0_CTRL`, `YRGB_MST`, `REGION0_VIR`,
    /// `VP_INT_RAW_STATUS`, `SYS0_INT_STATUS`, `SYS1_INT_STATUS`.
    pub vop: [u32; 15],
    /// sys_grf `SOC_STATUS1` (HPD level of HDMI0 in bit 16) and the HDPTX
    /// PHY0 GRF `STATUS` (`PLL_LOCK_DONE` 3, `PHY_CLK_RDY` 2, `PHY_RDY` 1).
    pub soc_status1: u32,
    pub phy_status: u32,
    /// PD_VO1 usable and PCLK_HDMITX0 open (1), then, only when they are,
    /// the controller's `CMU_STATUS`, `CMU_VIDQPCLK_FREQ`,
    /// `CMU_LINKQPCLK_FREQ` and `VIDEO_MONITOR_STATUS0-6`.
    pub vo1_usable: u32,
    pub cmu: [u32; 3],
    pub monitor: [u32; dw_hdmi_qp::VIDEO_MONITOR_WORDS],
    /// CLKGATE_CON(60), read when PD_VO1 is usable: PCLK_HDMITX0 in bit 11.
    pub cru_gate60: u32,
}

/// Section 5's read-backs (see the module), all reads except the event
/// clears of VP0: `vop2::frame_check` over `wait_us`, then `frames` frame
/// starts timed with `now_us`, then the registers.
pub fn status(
    m: &impl Mmio,
    b: &Bases,
    wait_us: u32,
    frames: u32,
    now_us: &mut impl FnMut() -> u64,
    delay_us: &mut impl FnMut(u32),
) -> Status {
    let mut s = Status::default();
    let v = b.vop;
    match vop2::frame_check(m, v, PORT, wait_us, delay_us) {
        Ok(f) => {
            s.frame_start = f.frame_start.into();
            s.underflow = f.underflow.into();
            s.bus_error = f.bus_error.map(u32::from);
            s.raw = f.raw;
        }
        Err(r) => (s.refusal, s.detail) = (r.why as u32, r.read_back),
    }
    if s.refusal == 0 && frames > 0 {
        let mut t0 = 0;
        for n in 0..=frames {
            if let Err(r) = vop2::wait_frame(m, v, PORT, delay_us) {
                (s.refusal, s.detail) = (r.why as u32, r.read_back);
                break;
            }
            let t = now_us();
            if n == 0 {
                t0 = t;
            } else {
                s.frames = n;
                s.frames_us = u32::try_from(t - t0).unwrap_or(u32::MAX);
            }
        }
    }
    let r = vop2::read_back(m, v, PORT, WINDOW);
    s.vop = [
        r.version,
        r.pd_ctrl,
        r.cfg_done,
        r.dsp_if_en,
        r.dsp_ctrl,
        r.htotal_hs_end,
        r.vtotal_vs_end,
        r.port_sel,
        r.layer_sel,
        r.region0_ctrl,
        r.yrgb_mst,
        r.vir,
        r.raw_status,
        r.sys0_int_status,
        r.sys1_int_status,
    ];
    s.soc_status1 = m.read32(b.sys_grf + dw_hdmi_qp::grf::SOC_STATUS1);
    s.phy_status = m.read32(b.phy_grf + hdptx::grf::STATUS);
    // the controller's register clock PCLK_HDMITX0 (60/11, Linux v6.18
    // clk-rk3588.c:1962-1963) must be open too: a read with it gated may
    // stall or fault the bus (review S2)
    let pclk = display_clk::PCLK_HDMITX0.gate;
    let vo1 = PdRegs::of(m, b.pmu, &PD_VO1).usable_of(&PD_VO1);
    if vo1 {
        s.cru_gate60 = m.read32(b.cru + clkgate_con(pclk.con));
    }
    if vo1 && s.cru_gate60 & 1 << pclk.bit == 0 {
        s.vo1_usable = 1;
        s.cmu = [dw_hdmi_qp::reg::CMU_STATUS, dw_hdmi_qp::reg::CMU_VIDQPCLK_FREQ, dw_hdmi_qp::reg::CMU_LINKQPCLK_FREQ]
            .map(|off| m.read32(b.hdmi + off));
        for (i, w) in s.monitor.iter_mut().enumerate() {
            *w = m.read32(b.hdmi + dw_hdmi_qp::reg::VIDEO_MONITOR_STATUS0 + 4 * i);
        }
    }
    s
}

#[cfg(test)]
mod tests;
