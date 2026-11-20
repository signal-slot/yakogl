// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Turning the i.MX 95's Mali-G310 on and off from the Cortex-A55: SCMI
//! requests to the System Manager (SM) on the Cortex-M33, which owns the
//! GPU's power domain, its clock root and the clock gate. The facts, each
//! cited (Linux, NXP's and Toradex's SM, Zephyr), are in
//! docs/notes/imx95-gpu-power.md; this module only sends what that note
//! establishes.
//!
//! The ids (SM `devices/MIMX95/sm/dev_sm_power.h`, `dev_sm_perf.h`,
//! `dev_sm_clock.h`; Linux's imx95 bindings carry the same numbers):
//! power domain [`PD_GPU`] 14, performance domain [`PERF_GPU`] 9, clock
//! [`CLK_GPU_CGC`] 174 (the gate after the GPU root). The A55's
//! non-secure agent ("AP-NS") may set all three on the Verdin iMX95's SM
//! configuration; it may touch neither the GPU root [`CLK_GPU`] (84) nor
//! [`CLK_GPUAPB`] (83), which the SM sets itself through the performance
//! domain and its bus-clock table. Neither is ever sent.
//!
//! Power-on, every read before the first write:
//! 1. POWER, PERFORMANCE and CLOCK PROTOCOL_VERSION: majors 3, 4 and 3
//!    (the SM's 0x30001, 0x40000, 0x30000).
//! 2. PERFORMANCE_DESCRIBE_LEVELS of domain 9 (from level 0, again from the
//!    next level while the reply says levels remain). The level chosen is
//!    the fastest whose frequency is at or below the rate asked for,
//!    never the parking level (index 0, 24 MHz): the SM's levels are
//!    discrete, so a rate between two levels is rounded down and one below
//!    the lowest running level is refused ([`Error::NoLevel`]) before
//!    anything is written.
//! 3. POWER_STATE_SET domain 14 on (flags 0: synchronous), then
//!    POWER_STATE_GET, which must read on. The SM loads the GPU mix's
//!    access controller configuration and releases the GPU reset itself on
//!    the way up (`dev_sm_config.c`, `DEV_SM_GpuConfigLoad`).
//! 4. PERFORMANCE_LEVEL_SET domain 9 to the level's index (the SM's
//!    domains are in level-indexing mode), then PERFORMANCE_LEVEL_GET,
//!    which must read the index back. This is the rate: the SM refuses
//!    CLOCK_RATE_SET on a gate (INVALID_PARAMETERS) and AP-NS may not set
//!    the root.
//! 5. CLOCK_CONFIG_SET clock 174 enabled (attributes 1, extended value 0),
//!    then CLOCK_RATE_GET clock 174: the rate the SM made, recorded as it
//!    reads ([`Up::rate_hz`]). The SM may round (a PLL's fraction); a rate
//!    above the level's, or 0, is [`Error::Rate`].
//!
//! Power-off: CLOCK_CONFIG_SET clock 174 disabled, POWER_STATE_SET domain 14
//! off, POWER_STATE_GET, which must read off. The performance level is
//! left as it is, as Linux's devfreq leaves it on suspend.
//!
//! Any failure stops the sequence at once with a typed [`Error`] naming the
//! step: a non-zero SCMI status (DENIED above all, [`Status::Denied`]) is
//! never retried and nothing after it is sent; what was already switched
//! on stays on (a caller that wants it off calls [`power_off`]). The
//! [`core::fmt::Display`] of an [`Error`] names the message and the status,
//! for example `POWER_STATE_SET (0x11/0x04): SCMI DENIED (-3)`.
//!
//! The transport is [`scmi::Transport`]: bytes in, bytes out, the status
//! word untouched. On the board it sits on Zephyr's `scmi_send_message`,
//! whose reply carries the raw status (the protocol drivers turn DENIED
//! into `-EACCES`, which would lose the name); on the host on a mock SM
//! (`sm_mock`). Every message carries [`TOKEN`]: one channel, one message
//! in flight, as Zephyr's own drivers send them. The message layer is
//! [`super::sm`], shared with [`super::display_power`].

use super::sm::{self, Fault, LevelsFault, Resp};
pub use super::sm::{pick_level, Level, Status, MAX_LEVELS, PARK_LEVEL, TOKEN};
use crate::firmware::scmi::{self, Transport};

/// `DEV_SM_PD_GPU` (dev_sm_power.h), Linux's `IMX95_PD_GPU`.
pub const PD_GPU: u32 = 14;
/// `DEV_SM_PERF_GPU` (dev_sm_perf.h), Linux's `IMX95_PERF_GPU`.
pub const PERF_GPU: u32 = 9;
/// `DEV_SM_CLK_GPU_CGC` (dev_sm_clock.h), Linux's `IMX95_CLK_GPU_CGC`
/// (41 + 123 + 10): the GPU clock gate, the panthor "core" clock.
pub const CLK_GPU_CGC: u32 = 174;
/// `DEV_SM_CLK_GPU`, the GPU clock root: SM-owned, never sent.
pub const CLK_GPU: u32 = 84;
/// `DEV_SM_CLK_GPUAPB`, the panthor "coregroup" clock: SM-owned, never sent.
pub const CLK_GPUAPB: u32 = 83;

/// The protocol majors the SM reports (`PROTOCOL_VERSION` of
/// rpc_scmi_power.c, rpc_scmi_perf.c, rpc_scmi_clock.c).
pub const POWER_MAJOR: u32 = 3;
pub const PERF_MAJOR: u32 = 4;
pub const CLOCK_MAJOR: u32 = 3;

/// The GPU's OPP rates (Linux imx95.dtsi `gpu_opp_table`), equal to the SM's
/// LOW, NOM and ODV levels of domain 9.
pub const OPP_HZ: [u64; 3] = [500_000_000, 800_000_000, 1_000_000_000];
/// The rate a board asks for unless told otherwise: nominal drive, which
/// needs no overdrive voltage.
pub const DEFAULT_RATE_HZ: u64 = 800_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The GPU rate asked for, in Hz; rounded down to a level.
    pub rate_hz: u64,
}

impl Config {
    /// The Verdin iMX95.
    pub const VERDIN: Config = Config { rate_hz: DEFAULT_RATE_HZ };
}

/// The message a failure happened in.
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
    ClockEnable,
    ClockRate,
    ClockDisable,
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
            Step::PerfLevelCheck => (scmi::PROTO_PERF, scmi::PERF_LEVEL_GET),
            Step::ClockEnable | Step::ClockDisable => (scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET),
            Step::ClockRate => (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_GET),
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
            Step::PerfLevelCheck => "PERFORMANCE_LEVEL_GET",
            Step::ClockEnable | Step::ClockDisable => "CLOCK_CONFIG_SET",
            Step::ClockRate => "CLOCK_RATE_GET",
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
    /// CLOCK_RATE_GET read 0, or more than the level's rate.
    Rate { level_hz: u64, got_hz: u64 },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let step = |f: &mut core::fmt::Formatter<'_>, s: Step| {
            let (p, m) = s.ids();
            write!(f, "{} (0x{:02x}/0x{:02x})", s.message(), p, m)
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
            Error::NoLevel { rate_hz } => write!(f, "no GPU level at or below {} Hz", rate_hz),
            Error::Rate { level_hz, got_hz } => write!(f, "GPU clock reads {} Hz, level {} Hz", got_hz, level_hz),
        }
    }
}

/// What a power-on did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    pub requested_hz: u64,
    /// The level set and read back.
    pub level: Level,
    /// CLOCK_RATE_GET of the gate after the enable.
    pub rate_hz: u64,
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

/// One command of `step`; at least `want` return values on success.
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

/// PERFORMANCE_DESCRIBE_LEVELS of [`PERF_GPU`], every reply until none
/// remain. Returns the levels and their count.
fn describe_levels<T: Transport>(t: &mut T) -> Result<([Level; MAX_LEVELS], usize), Error> {
    sm::describe_levels(t, PERF_GPU).map_err(|f| match f {
        LevelsFault::Call(f) => Error::at(Step::PerfDescribe, f),
        LevelsFault::Levels { returned, remaining } => Error::Levels { returned, remaining },
    })
}

/// Power the GPU on: power domain, performance level, clock gate (the
/// module documentation has the order and the messages).
pub fn power_on<T: Transport>(t: &mut T, cfg: &Config) -> Result<Up, Error> {
    check_version(t, Step::PowerVersion, POWER_MAJOR)?;
    check_version(t, Step::PerfVersion, PERF_MAJOR)?;
    let (levels, count) = describe_levels(t)?;
    let level = pick_level(&levels[..count], cfg.rate_hz).ok_or(Error::NoLevel { rate_hz: cfg.rate_hz })?;
    check_version(t, Step::ClockVersion, CLOCK_MAJOR)?;

    call(t, Step::PowerOn, &[0, PD_GPU, scmi::POWER_ON], 0)?;
    let got = call(t, Step::PowerOnCheck, &[PD_GPU], 1)?.get(0);
    if got != scmi::POWER_ON {
        return Err(Error::Readback { step: Step::PowerOnCheck, want: scmi::POWER_ON, got });
    }

    call(t, Step::PerfLevelSet, &[PERF_GPU, level.index], 0)?;
    let got = call(t, Step::PerfLevelCheck, &[PERF_GPU], 1)?.get(0);
    if got != level.index {
        return Err(Error::Readback { step: Step::PerfLevelCheck, want: level.index, got });
    }

    call(t, Step::ClockEnable, &[CLK_GPU_CGC, scmi::CLOCK_ENABLE, 0], 0)?;
    let r = call(t, Step::ClockRate, &[CLK_GPU_CGC], 2)?;
    let rate_hz = r.get(0) as u64 | (r.get(1) as u64) << 32;
    if rate_hz == 0 || rate_hz > level.hz() {
        return Err(Error::Rate { level_hz: level.hz(), got_hz: rate_hz });
    }
    Ok(Up { requested_hz: cfg.rate_hz, level, rate_hz })
}

/// Power the GPU off: the clock gate, then the power domain, read back off.
/// The performance level is left as it is.
pub fn power_off<T: Transport>(t: &mut T) -> Result<(), Error> {
    call(t, Step::ClockDisable, &[CLK_GPU_CGC, scmi::CLOCK_DISABLE, 0], 0)?;
    call(t, Step::PowerOff, &[0, PD_GPU, scmi::POWER_OFF], 0)?;
    let got = call(t, Step::PowerOffCheck, &[PD_GPU], 1)?.get(0);
    if got != scmi::POWER_OFF {
        return Err(Error::Readback { step: Step::PowerOffCheck, want: scmi::POWER_OFF, got });
    }
    Ok(())
}
