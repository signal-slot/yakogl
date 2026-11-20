// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The SCMI message layer the i.MX 95's power sequences share
//! ([`super::gpu_power`], [`super::display_power`]): one command to the
//! System Manager (SM) over a [`scmi::Transport`], its echo, status and length
//! checked, the SM's status kept by name; and the performance levels of a
//! domain (PERFORMANCE_DESCRIBE_LEVELS) with the choice of one. Every message
//! carries [`TOKEN`]: one channel, one message in flight, as Zephyr's own
//! drivers send them. Each sequence wraps a [`Fault`] in its own error, which
//! names the step (docs/notes/imx95-gpu-power.md, "the transport").

use crate::firmware::scmi::{self, Transport, MAX_MSG};

/// The token of every message.
pub const TOKEN: u16 = 0;

/// The most levels kept from PERFORMANCE_DESCRIBE_LEVELS (the SM has four
/// per SoC domain, `DEV_SM_NUM_PERF_LVL_SOC`: park, low, nominal,
/// overdrive; it returns at most four per reply, `PERF_MAX_PERFLEVELS`).
pub const MAX_LEVELS: usize = 8;
/// Words per level in a version 4 reply: value, power cost, attributes,
/// indicative frequency (kHz), level index.
const LEVEL_WORDS: usize = 5;
/// The parking level (`DEV_SM_PERF_LVL_PRK`, 24 MHz from the oscillator).
pub const PARK_LEVEL: u32 = 0;

/// A non-zero SCMI status (DEN0056 table 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NotSupported,
    InvalidParameters,
    Denied,
    NotFound,
    OutOfRange,
    Busy,
    CommsError,
    GenericError,
    HardwareError,
    ProtocolError,
    Other(i32),
}

impl Status {
    pub fn from_code(code: i32) -> Status {
        match code {
            scmi::STATUS_NOT_SUPPORTED => Status::NotSupported,
            scmi::STATUS_INVALID_PARAMETERS => Status::InvalidParameters,
            scmi::STATUS_DENIED => Status::Denied,
            scmi::STATUS_NOT_FOUND => Status::NotFound,
            scmi::STATUS_OUT_OF_RANGE => Status::OutOfRange,
            scmi::STATUS_BUSY => Status::Busy,
            scmi::STATUS_COMMS_ERROR => Status::CommsError,
            scmi::STATUS_GENERIC_ERROR => Status::GenericError,
            scmi::STATUS_HARDWARE_ERROR => Status::HardwareError,
            scmi::STATUS_PROTOCOL_ERROR => Status::ProtocolError,
            c => Status::Other(c),
        }
    }

    pub const fn code(self) -> i32 {
        match self {
            Status::NotSupported => scmi::STATUS_NOT_SUPPORTED,
            Status::InvalidParameters => scmi::STATUS_INVALID_PARAMETERS,
            Status::Denied => scmi::STATUS_DENIED,
            Status::NotFound => scmi::STATUS_NOT_FOUND,
            Status::OutOfRange => scmi::STATUS_OUT_OF_RANGE,
            Status::Busy => scmi::STATUS_BUSY,
            Status::CommsError => scmi::STATUS_COMMS_ERROR,
            Status::GenericError => scmi::STATUS_GENERIC_ERROR,
            Status::HardwareError => scmi::STATUS_HARDWARE_ERROR,
            Status::ProtocolError => scmi::STATUS_PROTOCOL_ERROR,
            Status::Other(c) => c,
        }
    }

    /// The status's name in DEN0056.
    pub const fn name(self) -> &'static str {
        match self {
            Status::NotSupported => "NOT_SUPPORTED",
            Status::InvalidParameters => "INVALID_PARAMETERS",
            Status::Denied => "DENIED",
            Status::NotFound => "NOT_FOUND",
            Status::OutOfRange => "OUT_OF_RANGE",
            Status::Busy => "BUSY",
            Status::CommsError => "COMMS_ERROR",
            Status::GenericError => "GENERIC_ERROR",
            Status::HardwareError => "HARDWARE_ERROR",
            Status::ProtocolError => "PROTOCOL_ERROR",
            Status::Other(_) => "unknown status",
        }
    }
}

/// What went wrong with one command, before a sequence names the step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The transport failed with its own code.
    Transport(i32),
    /// The SM answered with a non-zero status. Not retried.
    Refused(Status),
    /// The reply's header is not the request's.
    Echo { sent: u32, got: u32 },
    /// The reply is shorter than header, status and the values asked for.
    Length(usize),
}

/// The values of a reply (the words after the status).
pub struct Resp {
    buf: [u8; MAX_MSG],
    n: usize,
}

impl Resp {
    /// Return value `i`.
    pub fn get(&self, i: usize) -> u32 {
        scmi::word(&self.buf, 2 + i)
    }
    /// The number of return values.
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// One command with [`TOKEN`]; at least `want` return values on success.
pub fn call<T: Transport>(t: &mut T, proto: u8, msg: u8, words: &[u32], want: usize) -> Result<Resp, Fault> {
    let hdr = scmi::header(proto, msg, TOKEN);
    let mut tx = [0u8; MAX_MSG];
    // The sequences send at most four payload words: always fits.
    let len = scmi::encode(&mut tx, hdr, words).map_err(|_| Fault::Length(0))?;
    let mut buf = [0u8; MAX_MSG];
    let got = t.transfer(&tx[..len], &mut buf).map_err(Fault::Transport)?;
    if !(4..=MAX_MSG).contains(&got) {
        return Err(Fault::Length(got));
    }
    let echo = scmi::word(&buf, 0);
    if echo != hdr {
        return Err(Fault::Echo { sent: hdr, got: echo });
    }
    if got < 8 {
        return Err(Fault::Length(got));
    }
    let status = scmi::word(&buf, 1) as i32;
    if status != scmi::STATUS_SUCCESS {
        return Err(Fault::Refused(Status::from_code(status)));
    }
    let n = (got - 8) / 4;
    if n < want {
        return Err(Fault::Length(got));
    }
    Ok(Resp { buf, n })
}

/// One level of a performance domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Level {
    /// The level index PERFORMANCE_LEVEL_SET takes.
    pub index: u32,
    /// The indicative frequency, kHz.
    pub khz: u32,
}

impl Level {
    pub const fn hz(self) -> u64 {
        self.khz as u64 * 1000
    }
}

/// Why the levels of a domain could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelsFault {
    Call(Fault),
    /// No level, a count the reply does not carry, or more than
    /// [`MAX_LEVELS`] in all.
    Levels {
        returned: u32,
        remaining: u32,
    },
}

/// PERFORMANCE_DESCRIBE_LEVELS of `domain`, from level 0, again from the
/// next level while the reply says levels remain. Returns the levels and
/// their count.
pub fn describe_levels<T: Transport>(t: &mut T, domain: u32) -> Result<([Level; MAX_LEVELS], usize), LevelsFault> {
    let mut levels = [Level::default(); MAX_LEVELS];
    let mut count = 0usize;
    loop {
        let r = call(t, scmi::PROTO_PERF, scmi::PERF_DESCRIBE_LEVELS, &[domain, count as u32], 1)
            .map_err(LevelsFault::Call)?;
        let w = r.get(0);
        let returned = w & 0xfff;
        let remaining = w >> 16;
        let k = returned as usize;
        if k == 0 || count + k > MAX_LEVELS || r.len() < 1 + LEVEL_WORDS * k {
            return Err(LevelsFault::Levels { returned, remaining });
        }
        for i in 0..k {
            let at = 1 + LEVEL_WORDS * i;
            levels[count + i] = Level { index: r.get(at + 4), khz: r.get(at + 3) };
        }
        count += k;
        if remaining == 0 {
            return Ok((levels, count));
        }
    }
}

/// The fastest level at or below `rate_hz`, never [`PARK_LEVEL`].
pub fn pick_level(levels: &[Level], rate_hz: u64) -> Option<Level> {
    levels.iter().copied().filter(|l| l.index != PARK_LEVEL && l.hz() <= rate_hz).max_by_key(|l| l.khz)
}
