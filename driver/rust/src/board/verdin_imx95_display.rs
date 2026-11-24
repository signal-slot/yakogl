// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95 on the Mallow, the whole picture on X4: the DPU's
//! stream 0 (`soc::imx95::display`) and the DSI output
//! (`board::verdin_imx95_dsi`) in the order of the design note
//! (driver/docs/imx95-display.md section 4(b), "The order, cold"):
//!
//! | note step | what | here |
//! |---|---|---|
//! | 1 (DISPLAY), 3-6 | DISPLAY on, DISP1PIX 70 MHz from VIDEOPLL1, display CSR, interleaver, DPU configured not started, `PIXEL_LINK_CTRL` | `display::prepare` |
//! | 1 (CAMERA), 6 (camera CSRs), 7, 8 | CAMERA on, the PHY clocks and TPM6; master and stream CSRs, the DW host and combo PHY; the bridge (EN low >= 10 ms, then its CSRs) | `verdin_imx95_dsi::dsi_prepare`, with `display::Prepared::mode` at the pixel clock the SM made (`pix_hz`) |
//! | 9 | FrameGen0 on, wait for `PrimSyncStat` | `display::start` |
//! | 10, 11, 12 | host to video mode, the bridge's link check, the backlight | `verdin_imx95_dsi::dsi_start` |
//!
//! Step 2 of the note (`GPIO_10_DSI` low for 10 ms, backlight at 0) is in
//! step 8: `Bridge::start` drives EN low for 10 ms before it raises it, and
//! the backlight pin is not driven before step 12 (the Mallow's 10K to
//! ground keeps it dark).
//!
//! [`display_down`] reverses it: the DSI side first
//! (`verdin_imx95_dsi::dsi_output_down`), then `display::stop`, which the
//! DPU side requires to run after the DSI side stopped.

use super::verdin_imx95_dsi::{self as dsi, PANEL};
use crate::env::Env;
use crate::firmware::scmi::Transport;
use crate::mmio::Mmio;
use crate::soc::imx95::display::{self, Stream0, StreamConfig};
use crate::soc::imx95::display_power;
use crate::soc::imx95::dpu::{Form, Framebuffer, Route};
use crate::video::VideoMode;

/// Both halves' configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub display: display::Bases,
    pub stream: StreamConfig,
    pub dsi: dsi::Config,
}

impl Config {
    /// The 10.1" panel showing `fb`.
    pub const fn verdin(fb: Framebuffer) -> Config {
        Config {
            display: display::Bases::PHYS,
            stream: StreamConfig {
                mode: PANEL,
                route: Route::Dsi,
                fb,
                power: display_power::Config::VERDIN_DSI_10INCH,
                form: Form::Hal,
            },
            dsi: dsi::Config::VERDIN,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The DPU side (steps 1, 3-6, 9).
    Display(display::Error),
    /// The DSI side (steps 1, 6-8, 10-12).
    Dsi(dsi::Error),
}

/// What [`display_up`] brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    pub stream: Stream0,
    pub dsi: dsi::Up,
}

/// The mode the DSI side runs: the DPU's, at the pixel clock the SM made.
pub fn dsi_mode(p: &display::Prepared) -> VideoMode {
    VideoMode { pixel_clock_hz: p.pix_hz as u32, ..p.mode }
}

/// The picture on the 10.1" panel (the module documentation's order). On an
/// error what was already switched on stays on; [`display_down`] takes it
/// down.
pub fn display_up<T: Transport>(t: &mut T, m: &impl Mmio, env: &impl Env, cfg: &Config) -> Result<Up, Error> {
    let mut delay = |us: u32| env.busy_wait_us(us);
    let prepared = display::prepare(t, m, &cfg.display, &cfg.stream, &mut delay).map_err(Error::Display)?;
    let p = dsi::dsi_prepare(t, m, env, &dsi_mode(&prepared), &cfg.dsi).map_err(Error::Dsi)?;
    let stream = display::start(m, &cfg.display, &prepared, &mut delay).map_err(Error::Display)?;
    let up = dsi::dsi_start(m, env, &p, &cfg.dsi).map_err(Error::Dsi)?;
    Ok(Up { stream, dsi: up })
}

/// The DSI side down, then stream 0 and the display's power.
pub fn display_down<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    cfg: &Config,
    stream: &Stream0,
) -> Result<(), Error> {
    dsi::dsi_output_down(t, m, env, &dsi_mode(&stream.prepared), &cfg.dsi).map_err(Error::Dsi)?;
    let mut delay = |us: u32| env.busy_wait_us(us);
    display::stop(t, m, &cfg.display, stream, &mut delay).map_err(Error::Display)
}
