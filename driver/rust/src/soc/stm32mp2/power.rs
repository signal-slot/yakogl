// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Turning the STM32MP257F's GPU on and off from the non-secure side: two
//! SCMI requests to OP-TEE, then the RCC gate and reset ([`super::rcc`]),
//! then a wait for the GPU's idle. Plan #322, issues #7 and #1281.
//!
//! The order is galcore's, not etnaviv's (docs/notes/pre-study-stm32mp257f-dk.md
//! section 4 and "The etnaviv panic and what our image avoids"):
//! 1. PERFORMANCE (0x13), domain 1: PROTOCOL_VERSION, LEVEL_GET (the current
//!    level, read before any is set), LEVEL_SET, LEVEL_GET to read it back.
//!    OP-TEE refuses a level change while the GPU's supply is on
//!    (`stm32_gpu_opp.c:50-58`), so the level comes before the domain.
//! 2. POWER (0x11), domain 0 (`PD_SCMI_GPU`): PROTOCOL_VERSION,
//!    POWER_STATE_SET on, POWER_STATE_GET to read it back.
//! 3. `RCC_GPUCFGR`: gate on, reset asserted, reset released, one read back.
//! 4. `HI_IDLE_STATE` (0x004) through the caller's register read, at most
//!    [`Config::idle_reads`] times.
//!
//! Power-down is the reverse: reset asserted, gate off, then POWER
//! (PROTOCOL_VERSION, POWER_STATE_SET off, POWER_STATE_GET). The performance
//! level is left as it is: nothing was changed there that needs undoing
//! (with [`Level::Current`]), and a later power-up sets it again.
//!
//! Every SCMI message, request and reply, goes to the [`Log`] sink as one
//! line of hex bytes, the request before it is sent, so a panic on the
//! secure side leaves the last request on the console. Any failure of an
//! SCMI step (transport, non-zero status, a reply whose header is not the
//! request's, a short reply, an unexpected version or read-back) stops the
//! sequence at once with a typed [`Error`]: no RCC write follows it, and the
//! GPU is left as the failed step found it. Nothing here touches a clock
//! rate, a PLL or `CK_SCMI_PLL3`, and nothing reads the GPU at or above
//! [`REG_WINDOW`].
//!
//! The token is [`TOKEN`] on every message: one channel, one message in
//! flight, and the reference vectors (tools/testdata/optee-scmi) use it.

use super::rcc::GpuRcc;
use crate::firmware::scmi::{self, Transport, MAX_MSG};
use crate::mmio::Mmio;

/// The GPU's performance domain (#1281, `stm32mp255.dtsi:176`).
pub const PERF_DOMAIN: u32 = 1;
/// The GPU's power domain, `PD_SCMI_GPU` (`st,stm32mp25-power.h:9`).
pub const POWER_DOMAIN: u32 = 0;
/// The token of every message.
pub const TOKEN: u16 = 1;
/// The protocol major version both protocols must report (SCP-firmware's
/// 0x20000 for power domain and performance, `scmi_vectors.py`'s sources).
pub const VERSION_MAJOR: u32 = 2;

/// `VIVS_HI_IDLE_STATE` (`gpu/vivante/regs.rs`), the only GPU register read.
pub const IDLE_STATE: usize = 0x004;
/// Idle when every module but `AXI_LP` (bit 31) reports idle, as
/// `gpu::vivante::hw`'s reset checks it.
pub const IDLE_MASK: u32 = 0x7fff_ffff;
/// The GPU's register window (#1281: size 0x800); never read at or above it.
pub const REG_WINDOW: usize = 0x800;
const _: () = assert!(IDLE_STATE < REG_WINDOW);

/// The performance level to set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Set the level LEVEL_GET just read (galcore sets none of its own).
    Current,
    /// Set this level.
    Fixed(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub perf_domain: u32,
    pub power_domain: u32,
    pub level: Level,
    /// The most reads of `HI_IDLE_STATE` before [`Error::IdleTimeout`].
    pub idle_reads: u32,
}

impl Config {
    /// The STM32MP257F-DK.
    pub const DK: Config =
        Config { perf_domain: PERF_DOMAIN, power_domain: POWER_DOMAIN, level: Level::Current, idle_reads: 1000 };
}

/// One line per SCMI message (and one per RCC read back and idle result).
pub trait Log {
    fn line(&mut self, s: &str);
}

/// The SCMI step a failure happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    PerfVersion,
    PerfLevelGet,
    PerfLevelSet,
    PerfLevelCheck,
    PowerVersion,
    PowerOn,
    PowerOnCheck,
    PowerOff,
    PowerOffCheck,
}

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The transport failed with its own code.
    Transport { step: Step, code: i32 },
    /// The agent answered with a non-zero status.
    Refused { step: Step, status: Status },
    /// The reply's header is not the request's.
    Echo { step: Step, sent: u32, got: u32 },
    /// The reply is shorter than header, status and the values asked for, or
    /// longer than [`MAX_MSG`].
    Length { step: Step, len: usize },
    /// PROTOCOL_VERSION's major is not [`VERSION_MAJOR`].
    Version { step: Step, version: u32 },
    /// LEVEL_GET or POWER_STATE_GET does not read back what was set.
    Readback { step: Step, want: u32, got: u32 },
    /// `RCC_GPUCFGR` read back with the gate off or the reset held.
    Rcc { raw: u32 },
    /// `HI_IDLE_STATE` was not idle in `reads` reads; `last` the last value.
    IdleTimeout { reads: u32, last: u32 },
}

/// What a power-up found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Up {
    /// The level LEVEL_GET read before any was set.
    pub level_before: u32,
    /// The level set and read back.
    pub level: u32,
    /// `RCC_GPUCFGR` after the reset was released.
    pub rcc: u32,
    /// The idle `HI_IDLE_STATE` and the reads it took.
    pub idle: u32,
    pub idle_reads: u32,
}

/// A line buffer: a prefix and up to [`MAX_MSG`] bytes in hex.
struct Line {
    buf: [u8; 16 + 2 * MAX_MSG],
    n: usize,
}

impl Line {
    fn new() -> Line {
        Line { buf: [0; 16 + 2 * MAX_MSG], n: 0 }
    }
    fn as_str(&self) -> &str {
        // Only ASCII is ever written.
        core::str::from_utf8(&self.buf[..self.n]).unwrap_or("")
    }
}

impl core::fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        let end = (self.n + b.len()).min(self.buf.len());
        self.buf[self.n..end].copy_from_slice(&b[..end - self.n]);
        self.n = end;
        Ok(())
    }
}

fn log_fmt<L: Log>(log: &mut L, args: core::fmt::Arguments) {
    let mut l = Line::new();
    let _ = core::fmt::Write::write_fmt(&mut l, args);
    log.line(l.as_str());
}

fn log_hex<L: Log>(log: &mut L, prefix: &str, bytes: &[u8]) {
    let mut l = Line::new();
    let _ = core::fmt::Write::write_str(&mut l, prefix);
    for b in bytes {
        let _ = core::fmt::Write::write_fmt(&mut l, format_args!("{:02x}", b));
    }
    log.line(l.as_str());
}

/// One command with [`TOKEN`]; `want` return values at least on success.
/// Logs `scmi > <request>` before the transfer and `scmi < <reply>` (or
/// `scmi ! transport <code>`) after it.
fn call<T: Transport, L: Log>(
    t: &mut T,
    log: &mut L,
    step: Step,
    proto: u8,
    msg: u8,
    words: &[u32],
    want: usize,
) -> Result<[u32; 4], Error> {
    let hdr = scmi::header(proto, msg, TOKEN);
    let mut tx = [0u8; MAX_MSG];
    // At most three payload words: always fits.
    let len = scmi::encode(&mut tx, hdr, words).map_err(|_| Error::Length { step, len: 0 })?;
    log_hex(log, "scmi > ", &tx[..len]);
    let mut rx = [0u8; MAX_MSG];
    let got = match t.transfer(&tx[..len], &mut rx) {
        Ok(n) => n,
        Err(code) => {
            log_fmt(log, format_args!("scmi ! transport {}", code));
            return Err(Error::Transport { step, code });
        }
    };
    log_hex(log, "scmi < ", &rx[..got.min(MAX_MSG)]);
    if !(4..=MAX_MSG).contains(&got) {
        return Err(Error::Length { step, len: got });
    }
    let echo = scmi::word(&rx, 0);
    if echo != hdr {
        return Err(Error::Echo { step, sent: hdr, got: echo });
    }
    if got < 8 {
        return Err(Error::Length { step, len: got });
    }
    let status = scmi::word(&rx, 1) as i32;
    if status != scmi::STATUS_SUCCESS {
        return Err(Error::Refused { step, status: Status::from_code(status) });
    }
    if (got - 8) / 4 < want {
        return Err(Error::Length { step, len: got });
    }
    let mut v = [0u32; 4];
    for (i, w) in v.iter_mut().enumerate().take(want) {
        *w = scmi::word(&rx, 2 + i);
    }
    Ok(v)
}

fn check_version<T: Transport, L: Log>(t: &mut T, log: &mut L, step: Step, proto: u8) -> Result<(), Error> {
    let version = call(t, log, step, proto, scmi::PROTOCOL_VERSION, &[], 1)?[0];
    if version >> 16 != VERSION_MAJOR {
        return Err(Error::Version { step, version });
    }
    Ok(())
}

/// POWER: version, POWER_STATE_SET `state`, POWER_STATE_GET equal to it.
fn power_state<T: Transport, L: Log>(
    t: &mut T,
    log: &mut L,
    domain: u32,
    state: u32,
    set: Step,
    check: Step,
) -> Result<(), Error> {
    check_version(t, log, Step::PowerVersion, scmi::PROTO_POWER)?;
    call(t, log, set, scmi::PROTO_POWER, scmi::POWER_STATE_SET, &[0, domain, state], 0)?;
    let got = call(t, log, check, scmi::PROTO_POWER, scmi::POWER_STATE_GET, &[domain], 1)?[0];
    if got != state {
        return Err(Error::Readback { step: check, want: state, got });
    }
    Ok(())
}

/// Power the GPU up: the performance level, power domain 0, the RCC gate and
/// reset, then idle (the module documentation has the order). `gpu_read`
/// reads a GPU register at an offset below [`REG_WINDOW`]; only
/// [`IDLE_STATE`] is asked for.
pub fn power_up<T: Transport, M: Mmio, L: Log, R: FnMut(usize) -> u32>(
    scmi: &mut T,
    rcc: &GpuRcc<M>,
    mut gpu_read: R,
    log: &mut L,
    cfg: &Config,
) -> Result<Up, Error> {
    let d = cfg.perf_domain;
    check_version(scmi, log, Step::PerfVersion, scmi::PROTO_PERF)?;
    let level_before = call(scmi, log, Step::PerfLevelGet, scmi::PROTO_PERF, scmi::PERF_LEVEL_GET, &[d], 1)?[0];
    let level = match cfg.level {
        Level::Current => level_before,
        Level::Fixed(l) => l,
    };
    call(scmi, log, Step::PerfLevelSet, scmi::PROTO_PERF, scmi::PERF_LEVEL_SET, &[d, level], 0)?;
    let got = call(scmi, log, Step::PerfLevelCheck, scmi::PROTO_PERF, scmi::PERF_LEVEL_GET, &[d], 1)?[0];
    if got != level {
        return Err(Error::Readback { step: Step::PerfLevelCheck, want: level, got });
    }
    power_state(scmi, log, cfg.power_domain, scmi::POWER_ON, Step::PowerOn, Step::PowerOnCheck)?;

    rcc.enable_clock();
    rcc.assert_reset();
    rcc.deassert_reset();
    let s = rcc.status();
    log_fmt(log, format_args!("rcc gpucfgr 0x{:08x}", s.raw));
    if !s.clock_enabled() || s.reset_asserted() {
        return Err(Error::Rcc { raw: s.raw });
    }

    let mut last = 0;
    for i in 1..=cfg.idle_reads {
        last = gpu_read(IDLE_STATE);
        if last & IDLE_MASK == IDLE_MASK {
            log_fmt(log, format_args!("gpu idle 0x{:08x} after {} reads", last, i));
            return Ok(Up { level_before, level, rcc: s.raw, idle: last, idle_reads: i });
        }
    }
    log_fmt(log, format_args!("gpu not idle 0x{:08x} after {} reads", last, cfg.idle_reads));
    Err(Error::IdleTimeout { reads: cfg.idle_reads, last })
}

/// Power the GPU down: reset asserted, gate off, then power domain 0 off and
/// read back. The performance level is not touched.
pub fn power_down<T: Transport, M: Mmio, L: Log>(
    scmi: &mut T,
    rcc: &GpuRcc<M>,
    log: &mut L,
    cfg: &Config,
) -> Result<(), Error> {
    rcc.assert_reset();
    rcc.disable_clock();
    log_fmt(log, format_args!("rcc gpucfgr 0x{:08x}", rcc.status().raw));
    power_state(scmi, log, cfg.power_domain, scmi::POWER_OFF, Step::PowerOff, Step::PowerOffCheck)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soc::stm32mp2::rcc;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    const VECTORS: &str = include_str!("../../../../../tools/testdata/optee-scmi/scmi_msgs.txt");

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn vectors() -> HashMap<&'static str, Vec<u8>> {
        VECTORS
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (n, h) = l.split_once(' ').unwrap();
                (n, hex(h))
            })
            .collect()
    }

    fn v(name: &str) -> Vec<u8> {
        vectors().get(name).unwrap_or_else(|| panic!("no vector {name}")).clone()
    }

    /// Everything that happened, in order: SCMI requests by the vector name
    /// they equal, RCC reads and writes, GPU register reads.
    #[derive(Debug, PartialEq, Clone)]
    enum Ev {
        Scmi(&'static str),
        RccR(u32),
        RccW(u32),
        Gpu(usize),
    }

    type Events = Rc<RefCell<Vec<Ev>>>;

    /// Answers each request with the next scripted reply after checking the
    /// request equals the named vector byte for byte.
    struct Script {
        steps: Vec<(&'static str, Result<Vec<u8>, i32>)>,
        ev: Events,
    }

    impl Transport for Script {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            assert!(!self.steps.is_empty(), "unscripted request {tx:02x?}");
            let (name, reply) = self.steps.remove(0);
            assert_eq!(tx, v(name).as_slice(), "request is not vector {name}");
            self.ev.borrow_mut().push(Ev::Scmi(name));
            let r = reply?;
            rx[..r.len()].copy_from_slice(&r);
            Ok(r.len())
        }
    }

    struct Regs {
        reg: RefCell<u32>,
        ev: Events,
    }

    const RCC_BASE: usize = 0x4420_0000;

    impl Mmio for Regs {
        fn read32(&self, addr: usize) -> u32 {
            assert_eq!(addr, RCC_BASE + rcc::GPUCFGR);
            let r = *self.reg.borrow();
            self.ev.borrow_mut().push(Ev::RccR(r));
            r
        }
        fn write32(&self, addr: usize, val: u32) {
            assert_eq!(addr, RCC_BASE + rcc::GPUCFGR);
            self.ev.borrow_mut().push(Ev::RccW(val));
            *self.reg.borrow_mut() = val;
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[derive(Default)]
    struct Lines(Vec<String>);
    impl Log for Lines {
        fn line(&mut self, s: &str) {
            self.0.push(s.to_string());
        }
    }

    /// The exchange: request vector name, reply vector name.
    fn ok(req: &'static str, rep: &str) -> (&'static str, Result<Vec<u8>, i32>) {
        (req, Ok(v(rep)))
    }

    fn pair(n: &'static str) -> (&'static str, Result<Vec<u8>, i32>) {
        let rep: &'static str = Box::leak(n.replacen("req_", "rep_", 1).into_boxed_str());
        ok(n, rep)
    }

    const UP: [&str; 7] = [
        "req_perf_protocol_version",
        "req_perf_level_get",
        "req_perf_level_set",
        "req_perf_level_get",
        "req_pd_protocol_version",
        "req_pd_state_set_on",
        "req_pd_state_get_on",
    ];

    struct Rig {
        ev: Events,
        t: Script,
        regs: Regs,
        log: Lines,
    }

    fn rig(steps: Vec<(&'static str, Result<Vec<u8>, i32>)>, rcc_init: u32) -> Rig {
        let ev: Events = Rc::default();
        Rig {
            t: Script { steps, ev: ev.clone() },
            regs: Regs { reg: RefCell::new(rcc_init), ev: ev.clone() },
            log: Lines::default(),
            ev,
        }
    }

    fn up_script() -> Vec<(&'static str, Result<Vec<u8>, i32>)> {
        UP.iter().map(|n| pair(n)).collect()
    }

    impl Rig {
        fn up(&mut self, cfg: &Config, idle: &[u32]) -> Result<Up, Error> {
            let r = GpuRcc::new(&self.regs, RCC_BASE);
            let ev = self.ev.clone();
            let mut idle = idle.iter().copied();
            power_up(
                &mut self.t,
                &r,
                |off| {
                    assert!(off < REG_WINDOW);
                    ev.borrow_mut().push(Ev::Gpu(off));
                    idle.next().unwrap()
                },
                &mut self.log,
                cfg,
            )
        }
        fn down(&mut self, cfg: &Config) -> Result<(), Error> {
            let r = GpuRcc::new(&self.regs, RCC_BASE);
            power_down(&mut self.t, &r, &mut self.log, cfg)
        }
        fn events(&self) -> Vec<Ev> {
            self.ev.borrow().clone()
        }
        fn rcc_touched(&self) -> bool {
            self.ev.borrow().iter().any(|e| matches!(e, Ev::RccR(_) | Ev::RccW(_) | Ev::Gpu(_)))
        }
    }

    fn scmi_evs(names: &[&'static str]) -> Vec<Ev> {
        names.iter().map(|n| Ev::Scmi(n)).collect()
    }

    #[test]
    fn constants() {
        assert_eq!((PERF_DOMAIN, POWER_DOMAIN, TOKEN), (1, 0, 1));
        assert_eq!(IDLE_STATE as u32, crate::gpu::vivante::regs::VIVS_HI_IDLE_STATE);
        assert_eq!(IDLE_MASK, !crate::gpu::vivante::regs::VIVS_HI_IDLE_STATE_AXI_LP);
        assert_eq!(Config::DK.level, Level::Current);
    }

    #[test]
    fn power_up_order_and_bytes() {
        let mut r = rig(up_script(), 0x0000_0100);
        let up = r.up(&Config::DK, &[0x0000_0000, 0x7fff_fffe, 0xffff_ffff]).unwrap();
        assert_eq!(up, Up { level_before: 800_000, level: 800_000, rcc: 0x102, idle: 0xffff_ffff, idle_reads: 3 });
        let mut want = scmi_evs(&UP);
        want.extend([
            // gate on, reset asserted, reset released (read-modify-writes)
            Ev::RccR(0x100),
            Ev::RccW(0x102),
            Ev::RccR(0x102),
            Ev::RccW(0x103),
            Ev::RccR(0x103),
            Ev::RccW(0x102),
            // read back
            Ev::RccR(0x102),
            Ev::Gpu(IDLE_STATE),
            Ev::Gpu(IDLE_STATE),
            Ev::Gpu(IDLE_STATE),
        ]);
        assert_eq!(r.events(), want);
        assert!(r.t.steps.is_empty());
    }

    #[test]
    fn power_up_log_lines() {
        let mut r = rig(up_script(), 0);
        r.up(&Config::DK, &[0x7fff_ffff]).unwrap();
        assert_eq!(
            r.log.0,
            [
                "scmi > 004c0400",
                "scmi < 004c04000000000000000200",
                "scmi > 084c040001000000",
                "scmi < 084c04000000000000350c00",
                "scmi > 074c04000100000000350c00",
                "scmi < 074c040000000000",
                "scmi > 084c040001000000",
                "scmi < 084c04000000000000350c00",
                "scmi > 00440400",
                "scmi < 004404000000000000000200",
                "scmi > 04440400000000000000000000000000",
                "scmi < 0444040000000000",
                "scmi > 0544040000000000",
                "scmi < 054404000000000000000000",
                "rcc gpucfgr 0x00000002",
                "gpu idle 0x7fffffff after 1 reads",
            ]
        );
    }

    #[test]
    fn fixed_level_is_set_after_the_current_is_read() {
        // The current level reads 400000; the request is the vector's 800000.
        let mut cur = v("rep_perf_level_get");
        cur[8..12].copy_from_slice(&400_000u32.to_le_bytes());
        let mut s = up_script();
        s[1] = ("req_perf_level_get", Ok(cur));
        let mut r = rig(s, 0);
        let up = r.up(&Config { level: Level::Fixed(800_000), ..Config::DK }, &[0x7fff_ffff]).unwrap();
        assert_eq!((up.level_before, up.level), (400_000, 800_000));
    }

    #[test]
    fn level_readback_mismatch_stops_before_rcc() {
        let mut other = v("rep_perf_level_get");
        other[8..12].copy_from_slice(&1u32.to_le_bytes());
        let mut s = up_script();
        s[3] = ("req_perf_level_get", Ok(other));
        s.truncate(4);
        let mut r = rig(s, 0);
        assert_eq!(r.up(&Config::DK, &[]), Err(Error::Readback { step: Step::PerfLevelCheck, want: 800_000, got: 1 }));
        assert!(!r.rcc_touched());
    }

    #[test]
    fn power_down_order_bytes_and_lines() {
        let s = vec![pair("req_pd_protocol_version"), pair("req_pd_state_set_off"), pair("req_pd_state_get_off")];
        let mut r = rig(s, 0x102);
        r.down(&Config::DK).unwrap();
        let mut want = vec![Ev::RccR(0x102), Ev::RccW(0x103), Ev::RccR(0x103), Ev::RccW(0x101), Ev::RccR(0x101)];
        want.extend(scmi_evs(&["req_pd_protocol_version", "req_pd_state_set_off", "req_pd_state_get_off"]));
        assert_eq!(r.events(), want);
        assert_eq!(
            r.log.0,
            [
                "rcc gpucfgr 0x00000101",
                "scmi > 00440400",
                "scmi < 004404000000000000000200",
                "scmi > 04440400000000000000000000000040",
                "scmi < 0444040000000000",
                "scmi > 0544040000000000",
                "scmi < 054404000000000000000040",
            ]
        );
    }

    /// Refuse step `at` of the power-up with `reply`; nothing after it is
    /// sent and the RCC is not touched.
    fn refused_at(at: usize, reply: Result<Vec<u8>, i32>) -> (Result<Up, Error>, Rig) {
        let mut s = up_script();
        s[at].1 = reply;
        s.truncate(at + 1);
        let mut r = rig(s, 0);
        let e = r.up(&Config::DK, &[]);
        assert_eq!(r.events(), scmi_evs(&UP[..=at]));
        assert!(!r.rcc_touched());
        assert!(r.t.steps.is_empty());
        (e, r)
    }

    fn status_reply(req: &str, code: i32) -> Vec<u8> {
        let mut b = v(req)[..4].to_vec();
        b.extend(code.to_le_bytes());
        b
    }

    #[test]
    fn not_supported_stops_before_rcc() {
        for at in 0..UP.len() {
            let (e, _) = refused_at(at, Ok(status_reply(UP[at], scmi::STATUS_NOT_SUPPORTED)));
            assert!(matches!(e, Err(Error::Refused { status: Status::NotSupported, .. })), "{at}: {e:?}");
        }
    }

    #[test]
    fn invalid_parameters_stops_before_rcc() {
        // The vector's own refusal reply, on POWER_STATE_SET.
        let (e, r) = refused_at(5, Ok(v("rep_pd_state_set_async_flag_rejected")));
        assert_eq!(e, Err(Error::Refused { step: Step::PowerOn, status: Status::InvalidParameters }));
        assert_eq!(r.log.0.last().unwrap(), "scmi < 04440400feffffff");
        let (e, _) = refused_at(2, Ok(v("rep_perf_level_set_out_of_range")));
        assert_eq!(e, Err(Error::Refused { step: Step::PerfLevelSet, status: Status::OutOfRange }));
        let (e, _) = refused_at(1, Ok(v("rep_perf_level_get_unknown_domain")));
        assert_eq!(e, Err(Error::Refused { step: Step::PerfLevelGet, status: Status::NotFound }));
    }

    #[test]
    fn denied_level_while_supply_is_on_stops_before_rcc() {
        let (e, _) = refused_at(2, Ok(status_reply("req_perf_level_set", scmi::STATUS_DENIED)));
        assert_eq!(e, Err(Error::Refused { step: Step::PerfLevelSet, status: Status::Denied }));
    }

    #[test]
    fn wrong_echo_stops_before_rcc() {
        // A reply that answers another message (POWER_STATE_GET's to the SET).
        let (e, _) = refused_at(5, Ok(v("rep_pd_state_get_on")));
        assert_eq!(e, Err(Error::Echo { step: Step::PowerOn, sent: 0x0004_4404, got: 0x0004_4405 }));
        // The token differs.
        let mut b = v("rep_perf_protocol_version");
        b[2] = 0x08;
        let (e, _) = refused_at(0, Ok(b));
        assert_eq!(e, Err(Error::Echo { step: Step::PerfVersion, sent: 0x0004_4c00, got: 0x0008_4c00 }));
    }

    #[test]
    fn transport_and_length_errors_stop_before_rcc() {
        let (e, r) = refused_at(4, Err(-7));
        assert_eq!(e, Err(Error::Transport { step: Step::PowerVersion, code: -7 }));
        assert_eq!(r.log.0.last().unwrap(), "scmi ! transport -7");
        let (e, _) = refused_at(6, Ok(v("rep_pd_state_get_on")[..4].to_vec()));
        assert_eq!(e, Err(Error::Length { step: Step::PowerOnCheck, len: 4 }));
        // Status success without the value asked for.
        let (e, _) = refused_at(1, Ok(status_reply("req_perf_level_get", 0)));
        assert_eq!(e, Err(Error::Length { step: Step::PerfLevelGet, len: 8 }));
    }

    #[test]
    fn wrong_version_stops_before_rcc() {
        let mut b = v("rep_pd_protocol_version");
        b[8..12].copy_from_slice(&0x0003_0000u32.to_le_bytes());
        let (e, _) = refused_at(4, Ok(b));
        assert_eq!(e, Err(Error::Version { step: Step::PowerVersion, version: 0x0003_0000 }));
    }

    #[test]
    fn domain_not_on_stops_before_rcc() {
        let (e, _) = refused_at(6, Ok(v("rep_pd_state_get_off")));
        assert_eq!(e, Err(Error::Readback { step: Step::PowerOnCheck, want: 0, got: 0x4000_0000 }));
    }

    #[test]
    fn idle_timeout() {
        let mut r = rig(up_script(), 0);
        let e = r.up(&Config { idle_reads: 3, ..Config::DK }, &[0, 1, 0x8000_0000]);
        assert_eq!(e, Err(Error::IdleTimeout { reads: 3, last: 0x8000_0000 }));
        assert_eq!(r.events().iter().filter(|e| **e == Ev::Gpu(IDLE_STATE)).count(), 3);
        assert_eq!(r.log.0.last().unwrap(), "gpu not idle 0x80000000 after 3 reads");
    }

    #[test]
    fn rcc_readback_that_holds_reset_is_an_error() {
        /// A register whose reset bit sticks.
        struct Sticky(RefCell<u32>);
        impl Mmio for Sticky {
            fn read32(&self, _: usize) -> u32 {
                *self.0.borrow()
            }
            fn write32(&self, _: usize, v: u32) {
                *self.0.borrow_mut() = v | rcc::RESET;
            }
            fn read8(&self, _: usize) -> u8 {
                unreachable!()
            }
            fn write8(&self, _: usize, _: u8) {
                unreachable!()
            }
        }
        let ev: Events = Rc::default();
        let mut t = Script { steps: up_script(), ev };
        let m = Sticky(RefCell::new(0));
        let mut log = Lines::default();
        let e = power_up(&mut t, &GpuRcc::new(&m, 0), |_| panic!("no GPU read"), &mut log, &Config::DK);
        assert_eq!(e, Err(Error::Rcc { raw: 3 }));
    }

    #[test]
    fn status_codes() {
        assert_eq!(Status::from_code(-1), Status::NotSupported);
        assert_eq!(Status::from_code(-2), Status::InvalidParameters);
        assert_eq!(Status::from_code(-10), Status::ProtocolError);
        assert_eq!(Status::from_code(-42), Status::Other(-42));
    }
}
