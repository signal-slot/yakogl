// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The CAMERA power domain and the clocks of the i.MX 95's DSI output, asked
//! of the System Manager (SM) over SCMI in the style of
//! [`super::display_power`] and over the same message layer ([`super::sm`]):
//! every read before the first write, a non-zero status stops the sequence
//! at once and is never retried, the error names the step and the clock.
//!
//! The DSI host (0x4ACF0000), the DSI stream and master CSRs (0x4AD00000,
//! 0x4AD10000), the combo PHY CSR (0x4AD20000) and the CSI-2 host whose
//! test interface reaches the combo PHY's receive side (0x4AD40000) sit in
//! the CAMERA mix: Linux's `imx95.dtsi` gives the DSI node and the master
//! CSR `power-domains = <&scmi_devpd IMX95_PD_CAMERA>` (nxp/tdx
//! `imx95.dtsi:2104-2225`), the other CSRs the camera mix's APB clock.
//!
//! The ids (SM `devices/MIMX95/sm/dev_sm_power.h`, `dev_sm_perf.h`,
//! `dev_sm_clock.h`; imx-sm-toradex 3aa8a77a):
//!
//! | what | id | AP-NS permission (Toradex `configs/verdin-imx95/config_scmi.h`) |
//! |---|---|---|
//! | power domain CAMERA | [`PD_CAMERA`] 3 | ALL (:458) |
//! | performance domain CAM | [`PERF_CAM`] 11 | ALL (:467) |
//! | clock MIPIPHYCFG (the PHY's cfg clock) | [`CLK_MIPIPHYCFG`] 66 | ALL (:294) |
//! | clock MIPIPHYPLLBYPASS | [`CLK_MIPIPHYPLLBYPASS`] 67 | ALL (:295): its parent set, never enabled |
//! | clock MIPIPHYPLLREF (the PHY PLL's reference) | [`CLK_MIPIPHYPLLREF`] 68 | ALL (:296) |
//! | clock TPM6 (the backlight PWM, `PWM_3_DSI`) | [`CLK_TPM6`] 155 | ALL (:310) |
//! | clock CAMAPB | [`CLK_CAMAPB`] 62 | none: the SM sets it from the CAM level (`dev_sm_perf.c:2090`, the bus-clock table); never sent |
//!
//! The pixel clock (DISP1PIX, 80, from VIDEOPLL1) and the DISPLAY domain
//! are [`super::display_power`]'s; this module runs after it
//! ([`super::display::prepare`]) and stops before it.
//!
//! The CAM level: as for DISP, Toradex's SM starts the domain at overdrive
//! (`BOARD_PERF_LEVEL` = ODV, `boards/tdx-common/sm/brd_sm_tdx_common.c:107,163`;
//! the SM's CAM levels are park 24 MHz, LOW 400, NOM 666.667, ODV 800 MHz,
//! `dev_sm_perf.c:891-917`, `MIMX95_elec_spec.h:194-196`). By default
//! ([`Config::cam_hz`] `None`) the level is only read and reported; with a
//! rate it is set to the fastest level at or below it, never the park
//! level.
//!
//! Power-on:
//! 1. POWER, PERFORMANCE and CLOCK PROTOCOL_VERSION: majors 3, 4 and 3
//!    (with a level asked for, PERFORMANCE_DESCRIBE_LEVELS of domain 11
//!    between the second and the third).
//! 2. POWER_STATE_SET domain 3 on, POWER_STATE_GET reads on.
//! 3. PERFORMANCE_LEVEL_GET domain 11; or, with a level asked for,
//!    PERFORMANCE_LEVEL_SET and PERFORMANCE_LEVEL_GET reading it back.
//! 4. CLOCK_PARENT_SET, each read back with CLOCK_PARENT_GET, to the parents
//!    Linux assigns (`imx95.dtsi:2115-2120`; the SM checks EXCLUSIVE,
//!    `rpc_scmi_clock.c:1554-1558`): MIPIPHYCFG and MIPIPHYPLLREF to the
//!    24 MHz oscillator ([`CLK_OSC24M`] 2), MIPIPHYPLLBYPASS to VIDEOPLL1
//!    ([`CLK_VIDEOPLL1`] 19). Mux 0 of those roots is already the
//!    oscillator (SM `devices/MIMX95/drivers/fsl_clock.c:264,269,274`), so
//!    on a board fresh from reset the first two change nothing; they make
//!    the state independent of what ran before. [`Config::set_parents`]
//!    false leaves the step out.
//! 5. For MIPIPHYCFG, MIPIPHYPLLREF and TPM6: CLOCK_CONFIG_SET enabled, then
//!    CLOCK_RATE_GET. The PHY's two clocks must read 24 MHz: the PHY's
//!    cfgclkfreqrange and PLL are computed for it (`combo_phy`); another
//!    rate is [`Error::Rate`]. TPM6's rate is returned as read
//!    ([`Up::tpm_hz`]; 0 is an error); the PWM divides it.
//!    MIPIPHYPLLBYPASS is not enabled: Linux's driver takes only `cfg`,
//!    `ref` and `pix` (`imx95-mipi-dsi.c:318-341`) and the PLL runs in
//!    clock-generation mode, so the bypass carries nothing (the design
//!    note's step 1 lists it among the enables; this module follows the
//!    driver).
//!
//! Power-off: TPM6, MIPIPHYPLLREF, MIPIPHYCFG disabled, then the domain off
//! and read back off. The CAM level is left as it is.

use super::sm::{self, Fault, Level, LevelsFault, Resp, Status, MAX_LEVELS};
use crate::firmware::scmi::{self, Transport, CLOCK_PARENT_GET, CLOCK_PARENT_SET};

/// `DEV_SM_PD_CAMERA` (dev_sm_power.h:74), Linux's `IMX95_PD_CAMERA`.
pub const PD_CAMERA: u32 = 3;
/// `DEV_SM_PERF_CAM` (dev_sm_perf.h:75).
pub const PERF_CAM: u32 = 11;
/// `DEV_SM_CLK_CAMAPB` (dev_sm_clock.h:131): SM-owned, never sent.
pub const CLK_CAMAPB: u32 = 62;
/// `DEV_SM_CLK_MIPIPHYCFG` (dev_sm_clock.h:135).
pub const CLK_MIPIPHYCFG: u32 = 66;
/// `DEV_SM_CLK_MIPIPHYPLLBYPASS` (dev_sm_clock.h:136): its parent only.
pub const CLK_MIPIPHYPLLBYPASS: u32 = 67;
/// `DEV_SM_CLK_MIPIPHYPLLREF` (dev_sm_clock.h:137).
pub const CLK_MIPIPHYPLLREF: u32 = 68;
/// `DEV_SM_CLK_TPM6` (dev_sm_clock.h:224).
pub const CLK_TPM6: u32 = 155;

/// The rate both PHY clocks must run at: the 24 MHz oscillator.
pub const PHY_CLK_HZ: u64 = 24_000_000;

/// `DEV_SM_CLK_OSC24M` and `DEV_SM_CLK_VIDEOPLL1` (dev_sm_clock.h:70,87).
pub const CLK_OSC24M: u32 = 2;
pub const CLK_VIDEOPLL1: u32 = 19;
/// (clock, parent) of step 4, in order.
pub const PARENTS: [(u32, u32); 3] =
    [(CLK_MIPIPHYCFG, CLK_OSC24M), (CLK_MIPIPHYPLLREF, CLK_OSC24M), (CLK_MIPIPHYPLLBYPASS, CLK_VIDEOPLL1)];

/// The protocol majors the SM reports (as [`super::display_power`]).
pub const POWER_MAJOR: u32 = 3;
pub const PERF_MAJOR: u32 = 4;
pub const CLOCK_MAJOR: u32 = 3;

/// The clocks power-on enables, in order (power-off disables them in reverse).
pub const CLOCKS: [u32; 3] = [CLK_MIPIPHYCFG, CLK_MIPIPHYPLLREF, CLK_TPM6];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The CAM rate asked for, Hz, rounded down to a level; `None` leaves
    /// the level as the SM has it (read and reported only).
    pub cam_hz: Option<u64>,
    /// Step 4: the PHY clocks' parents.
    pub set_parents: bool,
}

impl Config {
    /// The Verdin iMX95: the level as Toradex's SM starts it, the parents set.
    pub const VERDIN: Config = Config { cam_hz: None, set_parents: true };
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
            Step::Enable(_) | Step::Disable(_) => "CLOCK_CONFIG_SET",
            Step::RateCheck(_) => "CLOCK_RATE_GET",
        }
    }

    /// The clock the step concerns.
    pub const fn clock(self) -> Option<u32> {
        match self {
            Step::ParentSet(c) | Step::ParentCheck(c) | Step::Enable(c) | Step::RateCheck(c) | Step::Disable(c) => {
                Some(c)
            }
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
    /// No running CAM level is at or below the rate asked for.
    NoLevel { rate_hz: u64 },
    /// A PHY clock does not read [`PHY_CLK_HZ`], or TPM6 reads 0.
    Rate { clock: u32, want_hz: u64, got_hz: u64 },
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
            Error::NoLevel { rate_hz } => write!(f, "no CAM level at or below {} Hz", rate_hz),
            Error::Rate { clock, want_hz, got_hz } => {
                write!(f, "clock {} reads {} Hz, want {} Hz", clock, got_hz, want_hz)
            }
        }
    }
}

/// What a power-on did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    /// The CAM level's index, as read (set first when one was asked for).
    pub level: u32,
    /// The level chosen, when one was asked for.
    pub chosen: Option<Level>,
    /// MIPIPHYCFG and MIPIPHYPLLREF as read (both [`PHY_CLK_HZ`]).
    pub cfg_hz: u64,
    pub ref_hz: u64,
    /// TPM6 as read: the backlight PWM's counter clock.
    pub tpm_hz: u64,
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

/// CAMERA on, the CAM level, the PHY clocks and TPM6 (the module
/// documentation has the order and the messages).
pub fn power_on<T: Transport>(t: &mut T, cfg: &Config) -> Result<Up, Error> {
    check_version(t, Step::PowerVersion, POWER_MAJOR)?;
    check_version(t, Step::PerfVersion, PERF_MAJOR)?;
    let chosen = match cfg.cam_hz {
        None => None,
        Some(hz) => {
            let (levels, count): ([Level; MAX_LEVELS], usize) =
                sm::describe_levels(t, PERF_CAM).map_err(|f| match f {
                    LevelsFault::Call(f) => Error::at(Step::PerfDescribe, f),
                    LevelsFault::Levels { returned, remaining } => Error::Levels { returned, remaining },
                })?;
            Some(sm::pick_level(&levels[..count], hz).ok_or(Error::NoLevel { rate_hz: hz })?)
        }
    };
    check_version(t, Step::ClockVersion, CLOCK_MAJOR)?;

    call(t, Step::PowerOn, &[0, PD_CAMERA, scmi::POWER_ON], 0)?;
    let got = call(t, Step::PowerOnCheck, &[PD_CAMERA], 1)?.get(0);
    if got != scmi::POWER_ON {
        return Err(Error::Readback { step: Step::PowerOnCheck, want: scmi::POWER_ON, got });
    }

    let level = match chosen {
        None => call(t, Step::PerfLevelGet, &[PERF_CAM], 1)?.get(0),
        Some(l) => {
            call(t, Step::PerfLevelSet, &[PERF_CAM, l.index], 0)?;
            let got = call(t, Step::PerfLevelCheck, &[PERF_CAM], 1)?.get(0);
            if got != l.index {
                return Err(Error::Readback { step: Step::PerfLevelCheck, want: l.index, got });
            }
            got
        }
    };

    if cfg.set_parents {
        for (clock, parent) in PARENTS {
            call(t, Step::ParentSet(clock), &[clock, parent], 0)?;
            let got = call(t, Step::ParentCheck(clock), &[clock], 1)?.get(0);
            if got != parent {
                return Err(Error::Readback { step: Step::ParentCheck(clock), want: parent, got });
            }
        }
    }

    let mut rates = [0u64; 3];
    for (i, &clock) in CLOCKS.iter().enumerate() {
        call(t, Step::Enable(clock), &[clock, scmi::CLOCK_ENABLE, 0], 0)?;
        let r = call(t, Step::RateCheck(clock), &[clock], 2)?;
        let hz = r.get(0) as u64 | (r.get(1) as u64) << 32;
        let ok = if clock == CLK_TPM6 { hz != 0 } else { hz == PHY_CLK_HZ };
        if !ok {
            let want_hz = if clock == CLK_TPM6 { 0 } else { PHY_CLK_HZ };
            return Err(Error::Rate { clock, want_hz, got_hz: hz });
        }
        rates[i] = hz;
    }
    Ok(Up { level, chosen, cfg_hz: rates[0], ref_hz: rates[1], tpm_hz: rates[2] })
}

/// The clocks off (in reverse), then CAMERA off and read back off.
pub fn power_off<T: Transport>(t: &mut T) -> Result<(), Error> {
    for &clock in CLOCKS.iter().rev() {
        call(t, Step::Disable(clock), &[clock, scmi::CLOCK_DISABLE, 0], 0)?;
    }
    call(t, Step::PowerOff, &[0, PD_CAMERA, scmi::POWER_OFF], 0)?;
    let got = call(t, Step::PowerOffCheck, &[PD_CAMERA], 1)?.get(0);
    if got != scmi::POWER_OFF {
        return Err(Error::Readback { step: Step::PowerOffCheck, want: scmi::POWER_OFF, got });
    }
    Ok(())
}
