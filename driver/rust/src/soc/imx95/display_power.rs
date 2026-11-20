// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95 display's power and pixel clock from the Cortex-A55: SCMI
//! requests to the System Manager (SM), in the style of [`super::gpu_power`]
//! and over the same message layer ([`super::sm`]). The facts are
//! driver/docs/imx95-display.md section 1 ("Who owns the display", the
//! clock table) and NXP's / Toradex's SM sources it cites; this module only
//! sends what they establish.
//!
//! The ids (SM `dev_sm_power.h`, `dev_sm_perf.h`, `dev_sm_clock.h`; Linux's
//! imx95 bindings carry the same numbers): power domain [`PD_DISPLAY`] 13,
//! performance domain [`PERF_DISP`] 12, clocks [`CLK_VIDEOPLL1_VCO`] 18,
//! [`CLK_VIDEOPLL1`] 19 and [`CLK_DISP1PIX`] 80 (the DPU's pixel clock,
//! `pix` of `nxp,imx95-dpu`). The A55's non-secure agent AP-NS holds all of
//! them at ALL on the Verdin iMX95 (Toradex's `config_scmi.h`). The display
//! block's bus clocks [`CLK_DISPAPB`] 75, [`CLK_DISPAXI`] 76 and
//! [`CLK_DISPOCRAM`] 78 belong to no agent: the SM runs them with the
//! DISPLAY domain and sets their rates from the DISP performance level
//! (`dev_sm_clock.c:1511-1519`, `dev_sm_perf.c` `DEV_SM_PerfDispFreqUpdate`);
//! an enable or rate request on them is DENIED. None of the three is ever
//! sent.
//!
//! The pixel clock is the DSI route's (Toradex's
//! `imx95-verdin-panel-cap-touch-10inch-dsi.dtso`, `&dpu`): DISP1PIX from
//! VIDEOPLL1, VIDEOPLL1_VCO 3,360 MHz, VIDEOPLL1 420 MHz, DISP1PIX the
//! mode's pixel clock (70 MHz, the root divider 6). The display block's
//! engine clock select then takes the CCM's clock (`dpu::EngineClock::Ccm`;
//! the LVDS route's LDB PLL is not handled here).
//!
//! The DISP performance level: Toradex's SM starts every performance domain
//! without a local PLL at overdrive (`BOARD_PERF_LEVEL` = ODV,
//! `brd_sm_tdx_common.c:107,163`; the design note, section 1), DISPAXI from
//! the 800 MHz parent, so no request is needed: by default
//! ([`Config::axi_hz`] `None`) the level is only read
//! (PERFORMANCE_LEVEL_GET) and reported. With a rate it is set: the fastest
//! level at or below it, never the parking level; the SM's DISP levels are
//! 400, 666.667 and 800 MHz of DISPAXI (MIMX95_elec_spec.h
//! `ES_*_KHZ_DISP`).
//!
//! Power-on, every read before the first write:
//! 1. POWER, PERFORMANCE and CLOCK PROTOCOL_VERSION: majors 3, 4 and 3
//!    (with a level asked for, PERFORMANCE_DESCRIBE_LEVELS of domain 12
//!    between the second and the third).
//! 2. POWER_STATE_SET domain 13 on, POWER_STATE_GET reads on.
//! 3. PERFORMANCE_LEVEL_GET domain 12; or, with a level asked for,
//!    PERFORMANCE_LEVEL_SET and PERFORMANCE_LEVEL_GET reading it back.
//! 4. CLOCK_PARENT_SET clock 80 to 19 (CLOCK_PARENT_GET reads 19), then
//!    CLOCK_RATE_SET 18, 19, 80 (flags 0: synchronous, rounded down), in
//!    that order: the parent before the rates and the VCO before its
//!    output, as Linux applies `assigned-clocks` (parents first, then
//!    rates). Both need EXCLUSIVE; AP-NS has ALL.
//! 5. CLOCK_CONFIG_SET 18, 19, 80 enabled, parents first, then
//!    CLOCK_RATE_GET 80: the pixel clock the SM made, kept as it reads; more
//!    than [`RATE_TOLERANCE_PPM`] from the rate asked for is
//!    [`Error::Rate`].
//!
//! Power-off (after the stream is stopped, [`super::display::stop`]):
//! CLOCK_CONFIG_SET 80, 19, 18 disabled, POWER_STATE_SET 13 off and read
//! back off. The level is left. VIDEOPLL1 also feeds the DSI PHY's PLL
//! bypass clock (`mipi_dsi` `assigned-clock-parents`) and the vote is the
//! agent's, not the module's: the DSI side is stopped first.
//!
//! A non-zero status stops the sequence at once, named in the error with
//! the clock it concerns (`CLOCK_PARENT_SET (0x14/0x0d) clock 80: SCMI
//! DENIED (-3)`), and is never retried.

use super::sm::{self, Fault, Level, LevelsFault, Resp, Status, MAX_LEVELS};
use crate::firmware::scmi::{self, Transport, CLOCK_PARENT_GET, CLOCK_PARENT_SET};

/// `DEV_SM_PD_DISPLAY` (dev_sm_power.h), Linux's `IMX95_PD_DISPLAY`.
pub const PD_DISPLAY: u32 = 13;
/// `DEV_SM_PERF_DISP` (dev_sm_perf.h), Linux's `IMX95_PERF_DISP`.
pub const PERF_DISP: u32 = 12;
/// `DEV_SM_CLK_VIDEOPLL1_VCO`, `IMX95_CLK_VIDEOPLL1_VCO`.
pub const CLK_VIDEOPLL1_VCO: u32 = 18;
/// `DEV_SM_CLK_VIDEOPLL1`, `IMX95_CLK_VIDEOPLL1`.
pub const CLK_VIDEOPLL1: u32 = 19;
/// `DEV_SM_CLK_DISP1PIX` (41 + 39), `IMX95_CLK_DISP1PIX`: the DPU's `pix`.
pub const CLK_DISP1PIX: u32 = 80;
/// `DEV_SM_CLK_DISPAPB` (41 + 34): SM-owned, never sent.
pub const CLK_DISPAPB: u32 = 75;
/// `DEV_SM_CLK_DISPAXI` (41 + 35): SM-owned, never sent.
pub const CLK_DISPAXI: u32 = 76;
/// `DEV_SM_CLK_DISPOCRAM` (41 + 37): SM-owned, never sent.
pub const CLK_DISPOCRAM: u32 = 78;

/// The protocol majors the SM reports (as [`super::gpu_power`]).
pub const POWER_MAJOR: u32 = 3;
pub const PERF_MAJOR: u32 = 4;
pub const CLOCK_MAJOR: u32 = 3;

/// How far the pixel clock may read from the rate asked for: 0.5 %.
pub const RATE_TOLERANCE_PPM: u64 = 5000;

/// The level Toradex's SM starts the DISP domain at (ODV, index 3).
pub const BOOT_LEVEL: u32 = 3;

/// The pixel clock's chain: VIDEOPLL1's VCO, its output, DISP1PIX.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelClock {
    pub vco_hz: u64,
    pub pll_hz: u64,
    pub pix_hz: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The DISPAXI rate asked for, Hz, rounded down to a DISP level; `None`
    /// leaves the level as the SM has it (read and reported only).
    pub axi_hz: Option<u64>,
    pub pixel: PixelClock,
}

impl Config {
    /// The Toradex Capacitive Touch Display 10.1" DSI on the Verdin's
    /// DSI_1 (`imx95-verdin-panel-cap-touch-10inch-dsi.dtso`: `&dpu`
    /// assigned-clock-rates 3,360 MHz and 420 MHz; panel-timing
    /// clock-frequency 70 MHz).
    pub const VERDIN_DSI_10INCH: Config =
        Config { axi_hz: None, pixel: PixelClock { vco_hz: 3_360_000_000, pll_hz: 420_000_000, pix_hz: 70_000_000 } };
}

/// The message a failure happened in; the clock's id where it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    PowerVersion,
    PerfVersion,
    PerfDescribe,
    ClockVersion,
    PowerOn,
    PowerOnCheck,
    PerfLevelSet,
    PerfLevelCheck,
    PerfLevelGet,
    ParentSet(u32),
    ParentCheck(u32),
    RateSet(u32),
    Enable(u32),
    RateCheck(u32),
    Disable(u32),
    PowerOff,
    PowerOffCheck,
}

impl Step {
    /// The protocol and message id the step sends.
    pub const fn ids(self) -> (u8, u8) {
        match self {
            Step::PowerVersion => (scmi::PROTO_POWER, scmi::PROTOCOL_VERSION),
            Step::PerfVersion => (scmi::PROTO_PERF, scmi::PROTOCOL_VERSION),
            Step::PerfDescribe => (scmi::PROTO_PERF, scmi::PERF_DESCRIBE_LEVELS),
            Step::ClockVersion => (scmi::PROTO_CLOCK, scmi::PROTOCOL_VERSION),
            Step::PowerOn | Step::PowerOff => (scmi::PROTO_POWER, scmi::POWER_STATE_SET),
            Step::PowerOnCheck | Step::PowerOffCheck => (scmi::PROTO_POWER, scmi::POWER_STATE_GET),
            Step::PerfLevelSet => (scmi::PROTO_PERF, scmi::PERF_LEVEL_SET),
            Step::PerfLevelCheck | Step::PerfLevelGet => (scmi::PROTO_PERF, scmi::PERF_LEVEL_GET),
            Step::ParentSet(_) => (scmi::PROTO_CLOCK, CLOCK_PARENT_SET),
            Step::ParentCheck(_) => (scmi::PROTO_CLOCK, CLOCK_PARENT_GET),
            Step::RateSet(_) => (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_SET),
            Step::Enable(_) | Step::Disable(_) => (scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET),
            Step::RateCheck(_) => (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_GET),
        }
    }

    /// The message's name in DEN0056.
    pub const fn message(self) -> &'static str {
        match self {
            Step::PowerVersion | Step::PerfVersion | Step::ClockVersion => "PROTOCOL_VERSION",
            Step::PerfDescribe => "PERFORMANCE_DESCRIBE_LEVELS",
            Step::PowerOn | Step::PowerOff => "POWER_STATE_SET",
            Step::PowerOnCheck | Step::PowerOffCheck => "POWER_STATE_GET",
            Step::PerfLevelSet => "PERFORMANCE_LEVEL_SET",
            Step::PerfLevelCheck | Step::PerfLevelGet => "PERFORMANCE_LEVEL_GET",
            Step::ParentSet(_) => "CLOCK_PARENT_SET",
            Step::ParentCheck(_) => "CLOCK_PARENT_GET",
            Step::RateSet(_) => "CLOCK_RATE_SET",
            Step::Enable(_) | Step::Disable(_) => "CLOCK_CONFIG_SET",
            Step::RateCheck(_) => "CLOCK_RATE_GET",
        }
    }

    /// The clock the step concerns.
    pub const fn clock(self) -> Option<u32> {
        match self {
            Step::ParentSet(c)
            | Step::ParentCheck(c)
            | Step::RateSet(c)
            | Step::Enable(c)
            | Step::RateCheck(c)
            | Step::Disable(c) => Some(c),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The transport failed with its own code.
    Transport { step: Step, code: i32 },
    /// The SM answered with a non-zero status. Not retried.
    Refused { step: Step, status: Status },
    /// The reply's header is not the request's.
    Echo { step: Step, sent: u32, got: u32 },
    /// The reply is shorter than header, status and the values asked for.
    Length { step: Step, len: usize },
    /// PROTOCOL_VERSION's major is not the one this module was written for.
    Version { step: Step, version: u32 },
    /// A GET does not read back what the SET asked for.
    Readback { step: Step, want: u32, got: u32 },
    /// PERFORMANCE_DESCRIBE_LEVELS returned no level, a count its reply does
    /// not carry, or more than [`MAX_LEVELS`] in all.
    Levels { returned: u32, remaining: u32 },
    /// No running level is at or below the rate asked for.
    NoLevel { rate_hz: u64 },
    /// The pixel clock reads further than [`RATE_TOLERANCE_PPM`] from the
    /// rate asked for, or 0.
    Rate { want_hz: u64, got_hz: u64 },
}

impl Error {
    fn at(step: Step, f: Fault) -> Error {
        match f {
            Fault::Transport(code) => Error::Transport { step, code },
            Fault::Refused(status) => Error::Refused { step, status },
            Fault::Echo { sent, got } => Error::Echo { step, sent, got },
            Fault::Length(len) => Error::Length { step, len },
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let step = |f: &mut core::fmt::Formatter<'_>, s: Step| {
            let (p, m) = s.ids();
            write!(f, "{} (0x{:02x}/0x{:02x})", s.message(), p, m)?;
            match s.clock() {
                Some(c) => write!(f, " clock {}", c),
                None => Ok(()),
            }
        };
        match *self {
            Error::Transport { step: s, code } => {
                step(f, s)?;
                write!(f, ": transport error {}", code)
            }
            Error::Refused { step: s, status } => {
                step(f, s)?;
                write!(f, ": SCMI {} ({})", status.name(), status.code())
            }
            Error::Echo { step: s, sent, got } => {
                step(f, s)?;
                write!(f, ": reply header 0x{:08x}, sent 0x{:08x}", got, sent)
            }
            Error::Length { step: s, len } => {
                step(f, s)?;
                write!(f, ": reply of {} bytes", len)
            }
            Error::Version { step: s, version } => {
                step(f, s)?;
                write!(f, ": version 0x{:08x}", version)
            }
            Error::Readback { step: s, want, got } => {
                step(f, s)?;
                write!(f, ": read 0x{:x}, want 0x{:x}", got, want)
            }
            Error::Levels { returned, remaining } => {
                write!(f, "PERFORMANCE_DESCRIBE_LEVELS: {} returned, {} remaining", returned, remaining)
            }
            Error::NoLevel { rate_hz } => write!(f, "no display level at or below {} Hz", rate_hz),
            Error::Rate { want_hz, got_hz } => write!(f, "pixel clock reads {} Hz, asked {} Hz", got_hz, want_hz),
        }
    }
}

/// What a power-on did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    /// The DISP level's index, as read (set first when one was asked for).
    pub level: u32,
    /// The level chosen, when one was asked for.
    pub chosen: Option<Level>,
    /// CLOCK_RATE_GET of DISP1PIX after the enable.
    pub pix_hz: u64,
}

fn call<T: Transport>(t: &mut T, step: Step, words: &[u32], want: usize) -> Result<Resp, Error> {
    let (proto, msg) = step.ids();
    sm::call(t, proto, msg, words, want).map_err(|f| Error::at(step, f))
}

fn check_version<T: Transport>(t: &mut T, step: Step, major: u32) -> Result<(), Error> {
    let version = call(t, step, &[], 1)?.get(0);
    if version >> 16 != major {
        return Err(Error::Version { step, version });
    }
    Ok(())
}

fn rate_set<T: Transport>(t: &mut T, clock: u32, hz: u64) -> Result<(), Error> {
    call(t, Step::RateSet(clock), &[0, clock, hz as u32, (hz >> 32) as u32], 0).map(|_| ())
}

/// Within [`RATE_TOLERANCE_PPM`] of `want`, and not 0.
pub fn rate_ok(want: u64, got: u64) -> bool {
    got != 0 && got.abs_diff(want) * 1_000_000 <= want * RATE_TOLERANCE_PPM
}

/// The order the pixel clock's chain is enabled in (parents first) and
/// disabled in (the reverse).
const CHAIN: [u32; 3] = [CLK_VIDEOPLL1_VCO, CLK_VIDEOPLL1, CLK_DISP1PIX];

/// Power the display on: domain, level, pixel clock (the module
/// documentation has the order and the messages).
pub fn power_on<T: Transport>(t: &mut T, cfg: &Config) -> Result<Up, Error> {
    check_version(t, Step::PowerVersion, POWER_MAJOR)?;
    check_version(t, Step::PerfVersion, PERF_MAJOR)?;
    let chosen = match cfg.axi_hz {
        None => None,
        Some(hz) => {
            let (levels, count): ([Level; MAX_LEVELS], usize) =
                sm::describe_levels(t, PERF_DISP).map_err(|f| match f {
                    LevelsFault::Call(f) => Error::at(Step::PerfDescribe, f),
                    LevelsFault::Levels { returned, remaining } => Error::Levels { returned, remaining },
                })?;
            Some(sm::pick_level(&levels[..count], hz).ok_or(Error::NoLevel { rate_hz: hz })?)
        }
    };
    check_version(t, Step::ClockVersion, CLOCK_MAJOR)?;

    call(t, Step::PowerOn, &[0, PD_DISPLAY, scmi::POWER_ON], 0)?;
    let got = call(t, Step::PowerOnCheck, &[PD_DISPLAY], 1)?.get(0);
    if got != scmi::POWER_ON {
        return Err(Error::Readback { step: Step::PowerOnCheck, want: scmi::POWER_ON, got });
    }

    let level = match chosen {
        None => call(t, Step::PerfLevelGet, &[PERF_DISP], 1)?.get(0),
        Some(l) => {
            call(t, Step::PerfLevelSet, &[PERF_DISP, l.index], 0)?;
            let got = call(t, Step::PerfLevelCheck, &[PERF_DISP], 1)?.get(0);
            if got != l.index {
                return Err(Error::Readback { step: Step::PerfLevelCheck, want: l.index, got });
            }
            got
        }
    };

    call(t, Step::ParentSet(CLK_DISP1PIX), &[CLK_DISP1PIX, CLK_VIDEOPLL1], 0)?;
    let got = call(t, Step::ParentCheck(CLK_DISP1PIX), &[CLK_DISP1PIX], 1)?.get(0);
    if got != CLK_VIDEOPLL1 {
        return Err(Error::Readback { step: Step::ParentCheck(CLK_DISP1PIX), want: CLK_VIDEOPLL1, got });
    }
    rate_set(t, CLK_VIDEOPLL1_VCO, cfg.pixel.vco_hz)?;
    rate_set(t, CLK_VIDEOPLL1, cfg.pixel.pll_hz)?;
    rate_set(t, CLK_DISP1PIX, cfg.pixel.pix_hz)?;
    for c in CHAIN {
        call(t, Step::Enable(c), &[c, scmi::CLOCK_ENABLE, 0], 0)?;
    }
    let r = call(t, Step::RateCheck(CLK_DISP1PIX), &[CLK_DISP1PIX], 2)?;
    let pix_hz = r.get(0) as u64 | (r.get(1) as u64) << 32;
    if !rate_ok(cfg.pixel.pix_hz, pix_hz) {
        return Err(Error::Rate { want_hz: cfg.pixel.pix_hz, got_hz: pix_hz });
    }
    Ok(Up { level, chosen, pix_hz })
}

/// Power the display off: the pixel clock's chain, then the domain, read
/// back off. The level is left as it is.
pub fn power_off<T: Transport>(t: &mut T) -> Result<(), Error> {
    for c in CHAIN.iter().rev() {
        call(t, Step::Disable(*c), &[*c, scmi::CLOCK_DISABLE, 0], 0)?;
    }
    call(t, Step::PowerOff, &[0, PD_DISPLAY, scmi::POWER_OFF], 0)?;
    let got = call(t, Step::PowerOffCheck, &[PD_DISPLAY], 1)?.get(0);
    if got != scmi::POWER_OFF {
        return Err(Error::Readback { step: Step::PowerOffCheck, want: scmi::POWER_OFF, got });
    }
    Ok(())
}
