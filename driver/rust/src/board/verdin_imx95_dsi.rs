// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95 on the Mallow: the DSI output on X4 to Toradex's
//! Capacitive Touch Display 10.1" DSI, from the pixel link to the backlight.
//! [`dsi_output_up`] is the one entry for this side: it takes the mode and
//! assumes the DPU's stream arrives on pixel link 0 (the DPU, the DISPLAY
//! domain, the pixel clock, the pixel interleaver and the display CSR's
//! pixel link are the DPU side's, `soc::imx95::display`). It is
//! [`dsi_prepare`] then [`dsi_start`]; `board::verdin_imx95_display` puts
//! the DPU's start between the two, as the design note's order has it.
//!
//! The display, from Toradex's overlay
//! `imx95-verdin-panel-cap-touch-10inch-dsi.dtso` (linux-toradex 851ac0f9):
//! a TI SN65DSI83 at 0x2C on `I2C_2_DSI` (LPI2C3) with four DSI lanes
//! (:76-108), its EN on the module's GPIO expander line 3 (`GPIO_10_DSI`,
//! `DSI_1_BKL_EN`, SODIMM 21; the expander is the PCAL6416 at 0x20 on
//! LPI2C2, `imx95-verdin.dtsi:406-420`), and behind it a Riverdi
//! RVT101HVDNWC00-B, 1280x800, VESA 24-bit (:30-57): [`PANEL`]. The
//! backlight is `pwms = <&tpm6 2 6666667 0>`, levels <0 45 63 88 119 158
//! 203 255>, default 4 (:22-28): [`BACKLIGHT`] on TPM6 (0x42510000,
//! `imx95.dtsi:835-841`). The DPU's pixel clock is VIDEOPLL1 420 MHz / 6 =
//! 70 MHz (the overlay's `assigned-clock-rates`, :70-75): the panel's rate
//! exactly.
//!
//! It is the display the Verdin iMX8MP drives (`driver/docs/imx8mp-display-cold.md`:
//! the same Riverdi panel and timings, an SN65DSI8x at 0x2C): the bridge's
//! board configuration is that board's ([`BRIDGE`], `ip::sn65dsi8x`'s
//! `IMX8MP` test case, checked there against the board's Linux dump), and
//! the CSRs this module writes for 420 Mbit/s are that dump's (the tests).
//!
//! The link: the SN65DSI83 asks its host for video, burst, no HSA, no EoT
//! packet, RGB888 (mainline `ti-sn65dsi83.c:974-976`, 0af298e6; facts only);
//! the lane rate is the pixel rate, 70 MHz x 24 / 4 = 420 Mbit/s, as NXP's
//! glue computes it (`phy_mipi_dphy_get_default_config`,
//! `imx95-mipi-dsi.c:1110`) and as the bridge expects (DSI clock = mode
//! clock x bpp / lanes / 2, `ti-sn65dsi83.c:355-362`): DSI clock 210 MHz,
//! CHA_DSI_CLK_RANGE 42, divider 3, LVDS 70 MHz.
//!
//! The order ([`dsi_prepare`]: 1-3, [`dsi_start`]: 4-6):
//! 1. SCMI: CAMERA on, the CAM level, the PHY clocks and TPM6
//!    (`soc::imx95::dsi_power`).
//! 2. The DSI host and combo PHY up, command mode, clock lane in HS
//!    (`soc::imx95::dsi::up`): SLLSEC1I 7.4.1 wants the clock lane in HS
//!    before EN rises.
//! 3. The bridge: EN cycle, ID, CSRs, PLL, soft reset (Table 7-2 Init seq
//!    3-7, `ip::sn65dsi8x::Bridge::start`).
//! 4. Video on (Init seq 8).
//! 5. The bridge's error status cleared and read (Init seq 9-11,
//!    `Bridge::check_link`): returned in [`Up::link_irq_stat`] (0 is a clean
//!    link); not an error, so a first board run still lights the panel.
//! 6. The backlight PWM (`ip::tpm_pwm`).
//!
//! The shim's [`Env`] serves the bridge: I2C bus `sn65dsi8x::BUS` is LPI2C3
//! (Zephyr's `nxp,lpi2c` driver), GPIO line `sn65dsi8x::LINE_EN` is the
//! expander's line 3 (Zephyr's PCAL64xxA driver). The pin mux of LPI2C3
//! (GPIO_IO28/29) and TPM6 channel 2 (GPIO_IO19) is the board devicetree's.

use crate::env::Env;
use crate::firmware::scmi::Transport;
use crate::ip::dw_mipi_dsi::{flags, Failed, Link, Plan as HostPlan};
use crate::ip::sn65dsi8x::{self, Bridge};
use crate::ip::tpm_pwm::{self, Pwm};
use crate::mmio::Mmio;
use crate::soc::imx95::combo_phy::{Bases, ComboPhy, Pll, Tuning};
use crate::soc::imx95::{display, dsi, dsi_power};
use crate::video::VideoMode;
use crate::ylog;

/// The Riverdi panel (overlay :39-53): 70 MHz, H 1280 36 24 64, V 800 8 8
/// 24, HSYNC and VSYNC active low, DE active high; the DPU side's constant.
pub const PANEL: VideoMode = display::VERDIN_DSI_10INCH;

/// `data-lanes = <1 2 3 4>` (overlay :97).
pub const LANES: u32 = 4;
/// What the SN65DSI83 asks of its host (module text).
pub const DSI_FLAGS: u32 =
    flags::MODE_VIDEO | flags::MODE_VIDEO_BURST | flags::MODE_VIDEO_NO_HSA | flags::MODE_NO_EOT_PACKET;

/// The bridge's board facts: the Verdin iMX8MP's for the same display (module
/// text): four lanes, 24 bpp VESA (`data-mapping = "vesa-24"`: format 2),
/// 200 ohm termination and positive LVDS syncs as that board's Linux runs
/// them (its dump: 0x18 = 0x18, 0x1A = 0x03), EN wired, no adapter reset.
pub const BRIDGE: sn65dsi8x::Board = sn65dsi8x::Board {
    lanes: LANES as u8,
    lvds_24bpp: true,
    format1: false,
    term_100_ohm: false,
    hs_neg: false,
    vs_neg: false,
    de_neg: false,
    has_en: true,
    has_reset: false,
};

/// TPM6 (`imx95.dtsi:835-841`).
pub const TPM6_BASE: usize = 0x4251_0000;
/// The backlight at its default level (module text).
pub const BACKLIGHT: Pwm = Pwm { channel: 2, period_ns: 6_666_667, level: 119, max: 255 };

/// Where the blocks are and what to ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub bases: Bases,
    pub tpm: usize,
    pub power: dsi_power::Config,
    pub backlight: Pwm,
    /// Lane rate above the pixel rate, per mille (0: Linux's rate; the
    /// design note's fallback if the picture breaks is 432 Mbit/s, 2.9 %,
    /// U17).
    pub margin_permille: u32,
    /// The combo PHY values hal_nxp and Linux disagree on (U13-U16).
    pub tuning: Tuning,
}

impl Config {
    pub const VERDIN: Config = Config {
        bases: Bases::PHYS,
        tpm: TPM6_BASE,
        power: dsi_power::Config::VERDIN,
        backlight: BACKLIGHT,
        margin_permille: 0,
        tuning: Tuning::HAL,
    };
}

/// The step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The SCMI requests (`dsi_power`); nothing after them was touched.
    Power(dsi_power::Error),
    /// The DSI host or the combo PHY (`dw_mipi_dsi::Failed`: the host's step
    /// and errno).
    Host(Failed),
    /// The bridge did not start (errno); video was not started.
    Bridge(i32),
    /// The backlight PWM (errno); the picture is up.
    Backlight(i32),
}

/// What [`dsi_output_up`] brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    pub power: dsi_power::Up,
    pub host_version: u32,
    pub host: HostPlan,
    pub pll: Pll,
    pub bridge: sn65dsi8x::Started,
    /// The bridge's IRQ_STAT after the video started: 0 is a clean link.
    pub link_irq_stat: u8,
    pub backlight: tpm_pwm::Plan,
}

/// The bridge's view of a mode (Table 7-9's fields).
pub fn bridge_mode(mode: &VideoMode) -> Result<sn65dsi8x::Mode, i32> {
    let f = |v: u32| u16::try_from(v).map_err(|_| -crate::errno::EINVAL);
    Ok(sn65dsi8x::Mode {
        pixel_clock_hz: mode.pixel_clock_hz,
        hactive: f(mode.hactive)?,
        vactive: f(mode.vactive)?,
        hsync: f(mode.hsync)?,
        vsync: f(mode.vsync)?,
        hbp: f(mode.hback)?,
        vbp: f(mode.vback)?,
        hfp: f(mode.hfront)?,
        vfp: f(mode.vfront)?,
    })
}

/// The DSI link the host runs for `mode`.
pub fn link(mode: &VideoMode) -> Link {
    dsi::link(mode, LANES, DSI_FLAGS)
}

/// What [`dsi_prepare`] brought up: the host in command mode with the
/// clock lane in HS, the bridge configured and soft-reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub power: dsi_power::Up,
    pub link: Link,
    pub host_version: u32,
    pub host: HostPlan,
    pub pll: Pll,
    pub bridge: sn65dsi8x::Started,
}

/// Steps 1-3 for `mode`: SCMI, the host and PHY, the bridge. On an error
/// what was already switched on stays on; [`dsi_output_down`] takes it
/// down.
pub fn dsi_prepare<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    mode: &VideoMode,
    cfg: &Config,
) -> Result<Prepared, Error> {
    let bmode = bridge_mode(mode).map_err(Error::Bridge)?;
    let power = dsi_power::power_on(t, &cfg.power).map_err(Error::Power)?;

    let link = link(mode);
    let mut phy = ComboPhy::new(cfg.bases, power.ref_hz, power.cfg_hz);
    phy.margin_permille = cfg.margin_permille;
    phy.tuning = cfg.tuning;
    let (host_version, host) = dsi::up(m, env, &mut phy, &link).map_err(Error::Host)?;
    let pll = phy.pll.ok_or(Error::Host(Failed { step: crate::ip::dw_mipi_dsi::step::PHY_RATE, errno: -1 }))?;

    // the rate on the wire: what the PLL makes
    let bridge = Bridge::new(env).start(&BRIDGE, &bmode, (pll.fout_actual_hz * 2) as u32).map_err(Error::Bridge)?;
    Ok(Prepared { power, link, host_version, host, pll, bridge })
}

/// Steps 4-6, once the pixel link carries the stream: video on, the
/// bridge's link check, the backlight.
pub fn dsi_start(m: &impl Mmio, env: &impl Env, p: &Prepared, cfg: &Config) -> Result<Up, Error> {
    dsi::video_on(m, &cfg.bases, &p.link);
    let link_irq_stat = Bridge::new(env).check_link().map_err(Error::Bridge)?;
    let backlight = tpm_pwm::start(m, cfg.tpm, &cfg.backlight, p.power.tpm_hz).map_err(Error::Backlight)?;
    let mode = &p.link.mode;
    ylog!(
        env,
        Inf,
        "verdin-imx95 DSI: {}x{} at {} kHz, {} lanes at {} kbit/s, bridge IRQ_STAT {:#04x}, backlight {}/{}",
        mode.hactive,
        mode.vactive,
        mode.pixel_clock_hz / 1000,
        LANES,
        p.pll.fout_actual_hz * 2 / 1000,
        link_irq_stat,
        cfg.backlight.level,
        cfg.backlight.max
    );
    Ok(Up {
        power: p.power,
        host_version: p.host_version,
        host: p.host,
        pll: p.pll,
        bridge: p.bridge,
        link_irq_stat,
        backlight,
    })
}

/// DSI output up for `mode` ([`dsi_prepare`], then [`dsi_start`]), with the
/// DPU's stream already on the pixel link.
pub fn dsi_output_up<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    mode: &VideoMode,
    cfg: &Config,
) -> Result<Up, Error> {
    let p = dsi_prepare(t, m, env, mode, cfg)?;
    dsi_start(m, env, &p, cfg)
}

/// The reverse: backlight off, the bridge's EN low, the host and PHY down,
/// then the SCMI power-off.
pub fn dsi_output_down<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    mode: &VideoMode,
    cfg: &Config,
) -> Result<(), Error> {
    tpm_pwm::stop(m, cfg.tpm);
    Bridge::new(env).stop(&BRIDGE).map_err(Error::Bridge)?;
    let mut phy = ComboPhy::new(cfg.bases, dsi_power::PHY_CLK_HZ, dsi_power::PHY_CLK_HZ);
    dsi::down(m, &mut phy, &link(mode));
    dsi_power::power_off(t).map_err(Error::Power)
}
