// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F-DK display path's SoC glue for C: what the board layer
//! and the mp2_display probe call before the display IP drivers. Plan #353,
//! item soc-display-host; issues #7 and #1281. Declarations:
//! driver/zephyr/lib/board/stm32mp2/include/stm32mp2_display.h.
//!
//! - `yd_mp2_display_rif_check`: [`display::rif_check`] and
//!   [`display::fb_check`], read only.
//! - `yd_mp2_display_power_on`: OP-TEE opened (capabilities, the data page
//!   registered, a session to the SCMI pseudo-TA, the channel), then
//!   [`display::power_on`] (RIF guard, supplies, clocks, RCC, SYSCFG), then
//!   the session closed.
//! - `yd_mp2_display_pins` runs [`display::pins`].
//! - `yd_mp2_display_adv_reset` runs [`display::adv_reset`].
//!
//! The order a caller follows: rif_check, power_on (the GPIO bank clocks
//! are among its SCMI clocks), pins, then the ADV7535's first I2C read if
//! wanted (open question 5 of the bring-up note), adv_reset.
//!
//! Every SMC, every OP-TEE message frame and every SCMI request and reply is
//! one line to the caller's log callback, the request before it is sent, as
//! `soc::stm32mp2::gpu_ffi` does. The generic functions (`*_with`, and
//! [`power_on_over`] below the OP-TEE session) take the conduit and the bus,
//! so the host tests run them on fakes and on `model_stm32mp2`; the
//! `extern "C"` functions (aarch64 only) give them `optee::HwSmc` and
//! [`Hw`].
//!
//! [`Hw`]: crate::mmio::Hw

use super::display::{self, Bases, Block, Config, Error, FbError, Op, Pin, PinState, Powered, Verdict};
use super::power::{self, Status};
use crate::errno::{EINVAL, EIO, ERANGE};
use crate::firmware::optee::{self, Client, Event, Shm, Smc};
use crate::firmware::scmi::{self, Transport};
use crate::ip::stm32_gpio::{self as gpio, CID_A35};
use crate::mmio::Mmio;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};

/// `yd_mp2_display_log_fn`: one line, `len` bytes, not NUL-terminated.
pub type LogFn = unsafe extern "C" fn(ctx: *mut c_void, line: *const u8, len: usize);
/// `yd_mp2_display_delay_fn`: wait at least `us` microseconds.
pub type DelayFn = unsafe extern "C" fn(ctx: *mut c_void, us: u32);

/// `struct yd_mp2_display`: where everything is.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdMp2Display {
    /// Mapped addresses: the RCC, SYSCFG, RIFSC, GPIOB and GPIOF.
    pub rcc_base: usize,
    pub syscfg_base: usize,
    pub rifsc_base: usize,
    pub gpiob_base: usize,
    pub gpiof_base: usize,
    /// [`YD_MP2_DISPLAY_SHM_SIZE`] bytes, 4 KiB aligned, normal cached memory.
    pub shm: *mut c_void,
    /// Its physical address, as OP-TEE is given it.
    pub shm_pa: u64,
    /// Optional.
    pub log: Option<LogFn>,
    pub log_ctx: *mut c_void,
    /// Required by power_on and adv_reset.
    pub delay_us: Option<DelayFn>,
    pub delay_ctx: *mut c_void,
    /// FLEXGEN_27's rate ([`Config::ltdc_rate_hz`]): 74250000 for 720p60,
    /// 148500000 for 1080p60, 0 for no `RATE_SET`.
    pub ltdc_rate_hz: u64,
    /// The SCMI channel (0 on the DK, the only one taken).
    pub channel: u32,
    /// Our compartment; 0 takes [`CID_A35`].
    pub cid: u32,
}

const _: () = assert!(size_of::<YdMp2Display>() == 104);
const _: () = assert!(offset_of!(YdMp2Display, gpiof_base) == 32);
const _: () = assert!(offset_of!(YdMp2Display, shm) == 40);
const _: () = assert!(offset_of!(YdMp2Display, shm_pa) == 48);
const _: () = assert!(offset_of!(YdMp2Display, log) == 56);
const _: () = assert!(offset_of!(YdMp2Display, log_ctx) == 64);
const _: () = assert!(offset_of!(YdMp2Display, delay_us) == 72);
const _: () = assert!(offset_of!(YdMp2Display, delay_ctx) == 80);
const _: () = assert!(offset_of!(YdMp2Display, ltdc_rate_hz) == 88);
const _: () = assert!(offset_of!(YdMp2Display, channel) == 96);
const _: () = assert!(offset_of!(YdMp2Display, cid) == 100);

/// The shared memory's size and alignment (as the GPU's).
pub const YD_MP2_DISPLAY_SHM_SIZE: usize = 3 * optee::PAGE;
pub const YD_MP2_DISPLAY_SHM_ALIGN: usize = optee::PAGE;
const _: () = assert!(size_of::<Shm>() == YD_MP2_DISPLAY_SHM_SIZE);
const _: () = assert!(core::mem::align_of::<Shm>() == YD_MP2_DISPLAY_SHM_ALIGN);

/// `err.kind`. `step` is a `YD_MP2_DISPLAY_STEP_*` for the SCMI kinds and
/// `id` the clock or voltage domain it names.
pub const YD_MP2_DISPLAY_ERR_NONE: u32 = 0;
/// `step` 1 open, 2 close; `a`: the OP-TEE error's code.
pub const YD_MP2_DISPLAY_ERR_OPTEE: u32 = 1;
/// `a`: the transport's code.
pub const YD_MP2_DISPLAY_ERR_TRANSPORT: u32 = 2;
/// `a`: the SCMI status (negative).
pub const YD_MP2_DISPLAY_ERR_REFUSED: u32 = 3;
/// `a`: the header sent, `b`: the header received.
pub const YD_MP2_DISPLAY_ERR_ECHO: u32 = 4;
/// `a`: the reply's length.
pub const YD_MP2_DISPLAY_ERR_LENGTH: u32 = 5;
/// `a`: wanted, `b`: read back (a mode, the enable bit, Hz).
pub const YD_MP2_DISPLAY_ERR_READBACK: u32 = 6;
/// A request the SCMI rules forbid; never sent.
pub const YD_MP2_DISPLAY_ERR_FORBIDDEN: u32 = 7;
/// `id`: the RIFSC entry, `a`: `YD_MP2_DISPLAY_RIF_*`.
pub const YD_MP2_DISPLAY_ERR_RIF: u32 = 8;
/// `id`: the RCC offset, `a`: the register as read.
pub const YD_MP2_DISPLAY_ERR_RCC: u32 = 9;
/// `a`: `RCC_DSICFGR` before, `b`: after.
pub const YD_MP2_DISPLAY_ERR_MUX: u32 = 10;
/// `a`: `DISPLAYCLKCR` as read.
pub const YD_MP2_DISPLAY_ERR_SYSCFG: u32 = 11;
/// `id`: the pin (`bank << 8 | number`, e.g. 'B' << 8 | 6), `a`: 1 out of
/// range, 2 secure, 3 foreign CID.
pub const YD_MP2_DISPLAY_ERR_PIN: u32 = 12;
/// `id`: the pin, `a`: its bank's `MODER`.
pub const YD_MP2_DISPLAY_ERR_PIN_MODE: u32 = 13;
/// `a`: `YD_MP2_DISPLAY_FB_*`.
pub const YD_MP2_DISPLAY_ERR_FB: u32 = 14;

/// `err.step` for [`YD_MP2_DISPLAY_ERR_OPTEE`].
pub const YD_MP2_DISPLAY_OPTEE_OPEN: u32 = 1;
pub const YD_MP2_DISPLAY_OPTEE_CLOSE: u32 = 2;

/// `err.step` for the SCMI kinds.
pub const YD_MP2_DISPLAY_STEP_VOLTAGE_VERSION: u32 = 1;
pub const YD_MP2_DISPLAY_STEP_SUPPLY_GET: u32 = 2;
pub const YD_MP2_DISPLAY_STEP_HDMI_GET: u32 = 3;
pub const YD_MP2_DISPLAY_STEP_HDMI_SET: u32 = 4;
pub const YD_MP2_DISPLAY_STEP_HDMI_CHECK: u32 = 5;
pub const YD_MP2_DISPLAY_STEP_CLOCK_VERSION: u32 = 6;
pub const YD_MP2_DISPLAY_STEP_CLOCK_ENABLE: u32 = 7;
pub const YD_MP2_DISPLAY_STEP_CLOCK_RATE_SET: u32 = 8;
pub const YD_MP2_DISPLAY_STEP_CLOCK_ATTRIBUTES: u32 = 9;
pub const YD_MP2_DISPLAY_STEP_CLOCK_RATE_GET: u32 = 10;

/// A RIF entry's verdict.
pub const YD_MP2_DISPLAY_RIF_OK: u32 = 0;
pub const YD_MP2_DISPLAY_RIF_SECURE: u32 = 1;
pub const YD_MP2_DISPLAY_RIF_FOREIGN_CID: u32 = 2;

/// The framebuffer verdict.
pub const YD_MP2_DISPLAY_FB_OK: u32 = 0;
pub const YD_MP2_DISPLAY_FB_EMPTY: u32 = 1;
pub const YD_MP2_DISPLAY_FB_UNALIGNED: u32 = 2;
pub const YD_MP2_DISPLAY_FB_ABOVE_4G: u32 = 3;
pub const YD_MP2_DISPLAY_FB_IMAGE: u32 = 4;
pub const YD_MP2_DISPLAY_FB_SECURE_LAYER: u32 = 5;
pub const YD_MP2_DISPLAY_FB_NOT_ALLOWED: u32 = 6;
/// No range given (size 0): not checked.
pub const YD_MP2_DISPLAY_FB_SKIPPED: u32 = 7;

/// `struct yd_mp2_display_err`: why a call failed (all 0 when it did not).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2DisplayErr {
    pub kind: u32,
    pub step: u32,
    pub id: u32,
    pub a: u32,
    pub b: u32,
}

const _: () = assert!(size_of::<YdMp2DisplayErr>() == 20);

/// `struct yd_mp2_display_power`: what the power-on found.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2DisplayPower {
    pub voltage_version: u32,
    pub clock_version: u32,
    /// Bit i: [`display::SUPPLIES`]`[i]` (vddcore, v1v8, v3v3) reads on.
    pub supplies_on: u32,
    /// LDO5's `CONFIG_GET` mode before the set, and after it.
    pub hdmi_before: u32,
    pub hdmi: u32,
    /// `RATE_GET` of [`display::CLOCKS`] (GPIOB, GPIOF, FLEXGEN_12,
    /// FLEXGEN_28, FLEXGEN_27) in Hz, saturated at `u32::MAX`.
    pub rate_hz: [u32; 5],
    /// `RCC_I2C2CFGR`, `RCC_DSICFGR`, `RCC_LTDCCFGR` before and after.
    pub rcc_before: [u32; 3],
    pub rcc: [u32; 3],
    pub displayclkcr: u32,
    pub err: YdMp2DisplayErr,
}

const _: () = assert!(size_of::<YdMp2DisplayPower>() == 88);
const _: () = assert!(offset_of!(YdMp2DisplayPower, rate_hz) == 20);
const _: () = assert!(offset_of!(YdMp2DisplayPower, rcc_before) == 40);
const _: () = assert!(offset_of!(YdMp2DisplayPower, displayclkcr) == 64);
const _: () = assert!(offset_of!(YdMp2DisplayPower, err) == 68);

/// `struct yd_mp2_display_pins`: the pins as left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2DisplayPins {
    pub gpiob_moder: u32,
    pub gpiof_moder: u32,
    pub gpiof_afrl: u32,
    /// PB4's level, the ADV7535's interrupt (active low).
    pub adv_int: u32,
    pub err: YdMp2DisplayErr,
}

const _: () = assert!(size_of::<YdMp2DisplayPins>() == 36);
const _: () = assert!(offset_of!(YdMp2DisplayPins, err) == 16);

/// `struct yd_mp2_display_rif_entry`: one RIFSC entry as read.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2DisplayRifEntry {
    pub id: u32,
    pub secure: u32,
    pub privileged: u32,
    pub cidcfgr: u32,
    pub semcr: u32,
    /// `YD_MP2_DISPLAY_RIF_*`.
    pub verdict: u32,
}

const _: () = assert!(size_of::<YdMp2DisplayRifEntry>() == 24);

/// `struct yd_mp2_display_rif`: the RIF state and the framebuffer verdict.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2DisplayRif {
    /// Required (LTDC 80, 119, DSI 81, I2C2 42), then reported only (LTDC
    /// 120, 121, DSI 122, 123).
    pub entries: [YdMp2DisplayRifEntry; 8],
    /// Bit i: `entries[i]` is not ours.
    pub refused: u32,
    /// `RIFSC_RIMC_ATTRx` of RIMU 11, 12, 13, raw.
    pub rimc: [u32; 3],
    /// The RISAF4 region the framebuffer's base lies in (9 or 10), 0 when
    /// refused or not checked.
    pub fb_region: u32,
    /// `YD_MP2_DISPLAY_FB_*`.
    pub fb_verdict: u32,
    pub err: YdMp2DisplayErr,
}

const _: () = assert!(size_of::<YdMp2DisplayRif>() == 236);
const _: () = assert!(offset_of!(YdMp2DisplayRif, refused) == 192);
const _: () = assert!(offset_of!(YdMp2DisplayRif, rimc) == 196);
const _: () = assert!(offset_of!(YdMp2DisplayRif, fb_region) == 208);
const _: () = assert!(offset_of!(YdMp2DisplayRif, err) == 216);

fn step_no(op: Op) -> u32 {
    match op {
        Op::VoltageVersion => YD_MP2_DISPLAY_STEP_VOLTAGE_VERSION,
        Op::SupplyGet => YD_MP2_DISPLAY_STEP_SUPPLY_GET,
        Op::HdmiGet => YD_MP2_DISPLAY_STEP_HDMI_GET,
        Op::HdmiSet => YD_MP2_DISPLAY_STEP_HDMI_SET,
        Op::HdmiCheck => YD_MP2_DISPLAY_STEP_HDMI_CHECK,
        Op::ClockVersion => YD_MP2_DISPLAY_STEP_CLOCK_VERSION,
        Op::ClockEnable => YD_MP2_DISPLAY_STEP_CLOCK_ENABLE,
        Op::ClockRateSet => YD_MP2_DISPLAY_STEP_CLOCK_RATE_SET,
        Op::ClockAttributes => YD_MP2_DISPLAY_STEP_CLOCK_ATTRIBUTES,
        Op::ClockRateGet => YD_MP2_DISPLAY_STEP_CLOCK_RATE_GET,
    }
}

fn status_code(s: Status) -> i32 {
    match s {
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

fn verdict_no(v: Verdict) -> u32 {
    match v {
        Verdict::Ok => YD_MP2_DISPLAY_RIF_OK,
        Verdict::Secure => YD_MP2_DISPLAY_RIF_SECURE,
        Verdict::ForeignCid => YD_MP2_DISPLAY_RIF_FOREIGN_CID,
    }
}

fn fb_no(e: FbError) -> u32 {
    match e {
        FbError::Empty => YD_MP2_DISPLAY_FB_EMPTY,
        FbError::Unaligned => YD_MP2_DISPLAY_FB_UNALIGNED,
        FbError::Above4G => YD_MP2_DISPLAY_FB_ABOVE_4G,
        FbError::Image => YD_MP2_DISPLAY_FB_IMAGE,
        FbError::SecureLayer => YD_MP2_DISPLAY_FB_SECURE_LAYER,
        FbError::NotAllowed => YD_MP2_DISPLAY_FB_NOT_ALLOWED,
    }
}

fn pin_no(p: Pin) -> u32 {
    (p.bank as u32) << 8 | p.pin
}

/// The C form of a [`display::Error`]; every one is returned as `-EIO`.
fn display_err(e: Error) -> YdMp2DisplayErr {
    let s = |st: display::Step| (step_no(st.op), st.id);
    let (kind, (step, id), a, b) = match e {
        Error::Rif { id, verdict } => (YD_MP2_DISPLAY_ERR_RIF, (0, id), verdict_no(verdict), 0),
        Error::Transport { step, code } => (YD_MP2_DISPLAY_ERR_TRANSPORT, s(step), code as u32, 0),
        Error::Refused { step, status } => (YD_MP2_DISPLAY_ERR_REFUSED, s(step), status_code(status) as u32, 0),
        Error::Echo { step, sent, got } => (YD_MP2_DISPLAY_ERR_ECHO, s(step), sent, got),
        Error::Length { step, len } => (YD_MP2_DISPLAY_ERR_LENGTH, s(step), len as u32, 0),
        Error::Forbidden { step } => (YD_MP2_DISPLAY_ERR_FORBIDDEN, s(step), 0, 0),
        Error::Readback { step, want, got } => (YD_MP2_DISPLAY_ERR_READBACK, s(step), want, got),
        Error::Rcc { block, raw } => (YD_MP2_DISPLAY_ERR_RCC, (0, block.cfgr() as u32), raw, 0),
        Error::Mux { before, after } => (YD_MP2_DISPLAY_ERR_MUX, (0, Block::Dsi.cfgr() as u32), before, after),
        Error::Syscfg { raw } => (YD_MP2_DISPLAY_ERR_SYSCFG, (0, 0), raw, 0),
        Error::Pin { pin, why } => {
            let a = match why {
                gpio::Error::Range => 1,
                gpio::Error::Secure => 2,
                gpio::Error::ForeignCid => 3,
            };
            (YD_MP2_DISPLAY_ERR_PIN, (0, pin_no(pin)), a, 0)
        }
        Error::PinMode { pin, moder } => (YD_MP2_DISPLAY_ERR_PIN_MODE, (0, pin_no(pin)), moder, 0),
    };
    YdMp2DisplayErr { kind, step, id, a, b }
}

/// The C log callback; a missing one drops the lines.
#[derive(Clone, Copy)]
pub struct CLog {
    pub f: Option<LogFn>,
    pub ctx: *mut c_void,
}

impl CLog {
    fn emit(&self, s: &str) {
        if let Some(f) = self.f {
            // SAFETY: the caller's callback takes `len` bytes at `line` for
            // the duration of the call.
            unsafe { f(self.ctx, s.as_ptr(), s.len()) }
        }
    }

    fn fmt(&self, args: core::fmt::Arguments<'_>) {
        let mut l = Line::new();
        let _ = core::fmt::Write::write_fmt(&mut l, args);
        self.emit(l.as_str());
    }

    fn hex(&self, prefix: &str, bytes: &[u8]) {
        let mut l = Line::new();
        let _ = core::fmt::Write::write_str(&mut l, prefix);
        for b in bytes {
            let _ = core::fmt::Write::write_fmt(&mut l, format_args!("{:02x}", b));
        }
        self.emit(l.as_str());
    }
}

impl power::Log for CLog {
    fn line(&mut self, s: &str) {
        self.emit(s);
    }
}

/// Longest message frame (four parameters) in hex, and a prefix.
const LINE_MAX: usize = 16 + 2 * optee::arg_size(optee::RPC_ARG_PARAMS);

struct Line {
    buf: [u8; LINE_MAX],
    n: usize,
}

impl Line {
    fn new() -> Line {
        Line { buf: [0; LINE_MAX], n: 0 }
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

/// The SMC conduit with OP-TEE's events on the log: each SMC as
/// `optee smc <x0..x3> -> <a0..a3>`, each frame as `optee msg > <hex>`
/// before the call and `optee msg < <hex>` after it. The SCMI request and
/// reply are [`display`]'s `scmi >` / `scmi <` lines.
pub struct LoggedSmc<S: Smc> {
    pub smc: S,
    pub log: CLog,
}

impl<S: Smc> Smc for LoggedSmc<S> {
    fn smc(&mut self, regs: [u64; 8], shm: &mut Shm) -> [u64; 8] {
        self.smc.smc(regs, shm)
    }

    fn log(&mut self, ev: &Event<'_>) {
        match *ev {
            Event::Smc { call, ret } => self.log.fmt(format_args!(
                "optee smc 0x{:x} 0x{:x} 0x{:x} 0x{:x} -> 0x{:x} 0x{:x} 0x{:x} 0x{:x}",
                call[0], call[1], call[2], call[3], ret[0], ret[1], ret[2], ret[3]
            )),
            Event::Msg { done: false, frame } => self.log.hex("optee msg > ", frame),
            Event::Msg { done: true, frame } => self.log.hex("optee msg < ", frame),
            Event::ScmiRequest(_) | Event::ScmiReply(_) => {}
        }
    }
}

fn clog(d: &YdMp2Display) -> CLog {
    CLog { f: d.log, ctx: d.log_ctx }
}

fn bases(d: &YdMp2Display) -> Bases {
    Bases { rcc: d.rcc_base, syscfg: d.syscfg_base, rifsc: d.rifsc_base, gpiob: d.gpiob_base, gpiof: d.gpiof_base }
}

fn cid(d: &YdMp2Display) -> u32 {
    if d.cid == 0 {
        CID_A35
    } else {
        d.cid
    }
}

/// The delay callback as a closure; `None` when there is none.
fn delay(d: &YdMp2Display) -> Option<impl FnMut(u32)> {
    let (f, ctx) = (d.delay_us?, d.delay_ctx);
    // SAFETY: the caller's callback waits `us` microseconds with its context.
    Some(move |us| unsafe { f(ctx, us) })
}

fn sat(hz: u64) -> u32 {
    hz.min(u32::MAX as u64) as u32
}

fn power_out(p: &Powered, out: &mut YdMp2DisplayPower) {
    out.voltage_version = p.voltage_version;
    out.clock_version = p.clock_version;
    out.supplies_on = p.supplies_on.iter().enumerate().fold(0, |m, (i, &on)| m | (on as u32) << i);
    out.hdmi_before = p.hdmi_before;
    out.hdmi = p.hdmi;
    out.rate_hz = p.rates.map(sat);
    out.rcc_before = p.rcc_before;
    out.rcc = p.rcc;
    out.displayclkcr = p.displayclkcr;
}

/// [`display::power_on`] over an open SCMI transport: 0, `-EIO` (the
/// reason in `out.err`), `-EINVAL` without a delay callback.
pub fn power_on_over<T: Transport, M: Mmio>(t: &mut T, m: &M, d: &YdMp2Display, out: &mut YdMp2DisplayPower) -> i32 {
    *out = YdMp2DisplayPower::default();
    let Some(wait) = delay(d) else {
        return -EINVAL;
    };
    let cfg = Config { cid: cid(d), ltdc_rate_hz: d.ltdc_rate_hz };
    let mut p = Powered::default();
    let r = display::power_on(t, m, &bases(d), &cfg, wait, &mut clog(d), &mut p);
    power_out(&p, out);
    match r {
        Ok(()) => 0,
        Err(e) => {
            out.err = display_err(e);
            -EIO
        }
    }
}

/// The power-on over `smc`: OP-TEE opened on `d.channel`,
/// [`power_on_over`], the session closed (also after a failure). 0, `-EIO`
/// (the reason in `out.err`), `-EINVAL` (no delay callback, or a channel
/// other than 0: no SMC is made).
pub fn power_on_with<S: Smc, M: Mmio>(
    smc: S,
    shm: &mut Shm,
    m: &M,
    d: &YdMp2Display,
    out: &mut YdMp2DisplayPower,
) -> i32 {
    *out = YdMp2DisplayPower::default();
    let log = clog(d);
    if d.delay_us.is_none() {
        log.fmt(format_args!("display power_on needs a delay callback"));
        return -EINVAL;
    }
    if d.channel != power::SCMI_CHANNEL {
        log.fmt(format_args!("optee channel {} refused: only {} is the A35's", d.channel, power::SCMI_CHANNEL));
        let a = optee::Error::Unsupported.code() as u32;
        out.err =
            YdMp2DisplayErr { kind: YD_MP2_DISPLAY_ERR_OPTEE, step: YD_MP2_DISPLAY_OPTEE_OPEN, id: 0, a, b: d.channel };
        return -EINVAL;
    }
    let mut c = Client::new(LoggedSmc { smc, log }, shm, d.shm_pa);
    if let Err(e) = c.open(0, d.channel) {
        log.fmt(format_args!("optee open failed: {:?} (code {})", e, e.code()));
        let _ = c.close();
        out.err = YdMp2DisplayErr {
            kind: YD_MP2_DISPLAY_ERR_OPTEE,
            step: YD_MP2_DISPLAY_OPTEE_OPEN,
            id: 0,
            a: e.code() as u32,
            b: 0,
        };
        return -EIO;
    }
    let rc = power_on_over(&mut c, m, d, out);
    let closed = c.close();
    if rc != 0 {
        return rc;
    }
    if let Err(e) = closed {
        log.fmt(format_args!("optee close failed: {:?} (code {})", e, e.code()));
        out.err = YdMp2DisplayErr {
            kind: YD_MP2_DISPLAY_ERR_OPTEE,
            step: YD_MP2_DISPLAY_OPTEE_CLOSE,
            id: 0,
            a: e.code() as u32,
            b: 0,
        };
        return -EIO;
    }
    0
}

fn pins_out(s: &PinState, out: &mut YdMp2DisplayPins) {
    out.gpiob_moder = s.gpiob_moder;
    out.gpiof_moder = s.gpiof_moder;
    out.gpiof_afrl = s.gpiof_afrl;
    out.adv_int = s.adv_int as u32;
}

/// [`display::pins`]: 0, or `-EIO` (a pin refused, nothing written).
pub fn pins_with<M: Mmio>(m: &M, d: &YdMp2Display, out: &mut YdMp2DisplayPins) -> i32 {
    *out = YdMp2DisplayPins::default();
    match display::pins(m, &bases(d), cid(d), &mut clog(d)) {
        Ok(s) => {
            pins_out(&s, out);
            0
        }
        Err(e) => {
            out.err = display_err(e);
            -EIO
        }
    }
}

/// [`display::adv_reset`]: 0, `-EIO` (PB6 refused or not an output),
/// `-EINVAL` without a delay callback.
pub fn adv_reset_with<M: Mmio>(m: &M, d: &YdMp2Display, out: &mut YdMp2DisplayPins) -> i32 {
    *out = YdMp2DisplayPins::default();
    let Some(wait) = delay(d) else {
        return -EINVAL;
    };
    match display::adv_reset(m, &bases(d), cid(d), wait, &mut clog(d)) {
        Ok(s) => {
            pins_out(&s, out);
            0
        }
        Err(e) => {
            out.err = display_err(e);
            -EIO
        }
    }
}

/// [`display::rif_check`] and, when `fb_size` is not 0,
/// [`display::fb_check`]; read only. 0, `-EIO` (a required entry is not
/// ours; err RIF), `-ERANGE` (the framebuffer range is refused; err FB).
pub fn rif_check_with<M: Mmio>(m: &M, d: &YdMp2Display, fb_base: u64, fb_size: u64, out: &mut YdMp2DisplayRif) -> i32 {
    *out = YdMp2DisplayRif::default();
    let log = clog(d);
    let rep = display::rif_check(m, d.rifsc_base, cid(d), &mut clog(d));
    for (i, e) in rep.entries.iter().enumerate() {
        out.entries[i] = YdMp2DisplayRifEntry {
            id: e.id,
            secure: e.secure as u32,
            privileged: e.privileged as u32,
            cidcfgr: e.cidcfgr,
            semcr: e.semcr,
            verdict: verdict_no(e.verdict),
        };
        if e.verdict != Verdict::Ok {
            out.refused |= 1 << i;
        }
    }
    out.rimc = rep.rimc;
    let fb = if fb_size == 0 {
        out.fb_verdict = YD_MP2_DISPLAY_FB_SKIPPED;
        Ok(())
    } else {
        match display::fb_check(fb_base, fb_size) {
            Ok(region) => {
                out.fb_region = region;
                log.fmt(format_args!("fb 0x{:x} + 0x{:x}: risaf4 region {}", fb_base, fb_size, region));
                Ok(())
            }
            Err(e) => {
                out.fb_verdict = fb_no(e);
                log.fmt(format_args!("fb 0x{:x} + 0x{:x} refused: {:?}", fb_base, fb_size, e));
                Err(e)
            }
        }
    };
    if let Some(e) = rep.refused() {
        out.err = display_err(Error::Rif { id: e.id, verdict: e.verdict });
        return -EIO;
    }
    if let Err(e) = fb {
        out.err = YdMp2DisplayErr { kind: YD_MP2_DISPLAY_ERR_FB, step: 0, id: 0, a: fb_no(e), b: 0 };
        return -ERANGE;
    }
    0
}

/// The C entry points: the hardware conduit and buses.
#[cfg(target_arch = "aarch64")]
mod abi {
    use super::*;
    use crate::firmware::optee::HwSmc;
    use crate::mmio::Hw;

    /// # Safety
    /// `d` and `out` valid; `d`'s bases mapped device memory (the RCC,
    /// SYSCFG and RIFSC pages); `d->shm` [`YD_MP2_DISPLAY_SHM_SIZE`] bytes of
    /// normal memory owned by the caller for the call, at physical address
    /// `d->shm_pa`; the secure monitor routes OP-TEE's SMCs.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_display_power_on(d: *const YdMp2Display, out: *mut YdMp2DisplayPower) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(d), Some(out)) = (unsafe { d.as_ref() }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        let p = d.shm as *mut Shm;
        if p.is_null()
            || !(p as usize).is_multiple_of(YD_MP2_DISPLAY_SHM_ALIGN)
            || !d.shm_pa.is_multiple_of(YD_MP2_DISPLAY_SHM_ALIGN as u64)
        {
            return -EINVAL;
        }
        // SAFETY: the caller's contract; aligned and sized as Shm (any bytes
        // are a valid Shm).
        let shm = unsafe { &mut *p };
        // SAFETY: the caller's contract.
        let (smc, hw) = unsafe { (HwSmc::new(), Hw::new()) };
        power_on_with(smc, shm, &hw, d, out)
    }

    /// # Safety
    /// `d` and `out` valid; `d->gpiob_base` and `d->gpiof_base` mapped device
    /// memory with the bank clocks running (after power_on).
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_display_pins(d: *const YdMp2Display, out: *mut YdMp2DisplayPins) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(d), Some(out)) = (unsafe { d.as_ref() }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        pins_with(&hw, d, out)
    }

    /// # Safety
    /// As `yd_mp2_display_pins`.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_display_adv_reset(d: *const YdMp2Display, out: *mut YdMp2DisplayPins) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(d), Some(out)) = (unsafe { d.as_ref() }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        adv_reset_with(&hw, d, out)
    }

    /// # Safety
    /// `d` and `out` valid; `d->rifsc_base` mapped device memory.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_display_rif_check(
        d: *const YdMp2Display,
        fb_base: u64,
        fb_size: u64,
        out: *mut YdMp2DisplayRif,
    ) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(d), Some(out)) = (unsafe { d.as_ref() }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        rif_check_with(&hw, d, fb_base, fb_size, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_stm32mp2::{Mp2Config, Mp2Model};
    use crate::soc::stm32mp2::display_regs::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const HEADER: &str = include_str!("../../../../zephyr/lib/board/stm32mp2/include/stm32mp2_display.h");

    /// A register file: reads and writes recorded, BSRR folded into ODR.
    #[derive(Default)]
    struct Bus {
        regs: RefCell<BTreeMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
    }

    impl Bus {
        fn set(&self, a: usize, v: u32) {
            self.regs.borrow_mut().insert(a, v);
        }
    }

    impl Mmio for Bus {
        fn read32(&self, a: usize) -> u32 {
            self.regs.borrow().get(&a).copied().unwrap_or(0)
        }
        fn write32(&self, a: usize, v: u32) {
            self.writes.borrow_mut().push((a, v));
            self.regs.borrow_mut().insert(a, v);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    /// What the callbacks saw: log lines and delays.
    #[derive(Default)]
    struct Seen {
        lines: Vec<String>,
        delays: Vec<u32>,
    }

    unsafe extern "C" fn log_cb(ctx: *mut c_void, line: *const u8, len: usize) {
        // SAFETY: the tests pass a `Seen` and the module a valid line.
        let (s, b) = unsafe { (&mut *(ctx as *mut Seen), core::slice::from_raw_parts(line, len)) };
        s.lines.push(String::from_utf8(b.to_vec()).unwrap());
    }

    unsafe extern "C" fn delay_cb(ctx: *mut c_void, us: u32) {
        // SAFETY: the tests pass a `Seen`.
        unsafe { (*(ctx as *mut Seen)).delays.push(us) }
    }

    fn dev(seen: &mut Seen) -> YdMp2Display {
        let ctx = seen as *mut Seen as *mut c_void;
        YdMp2Display {
            rcc_base: RCC_BASE,
            syscfg_base: SYSCFG_BASE,
            rifsc_base: RIFSC_BASE,
            gpiob_base: GPIOB_BASE,
            gpiof_base: GPIOF_BASE,
            shm: core::ptr::null_mut(),
            shm_pa: 0x8800_0000,
            log: Some(log_cb),
            log_ctx: ctx,
            delay_us: Some(delay_cb),
            delay_ctx: ctx,
            ltdc_rate_hz: 74_250_000,
            channel: 0,
            cid: 0,
        }
    }

    fn model() -> Mp2Model {
        Mp2Model::with_config(Mp2Config { display_scmi: true, ..Default::default() })
    }

    /// The transport the model serves, checking that the request was logged
    /// (the last line) before it reached the server.
    struct LoggedFirst<'a> {
        inner: crate::model_stm32mp2::Mp2Scmi<'a>,
        seen: *const Seen,
    }

    impl Transport for LoggedFirst<'_> {
        fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
            // SAFETY: the test's Seen outlives the call; no &mut is live
            // while the transport runs.
            let last = unsafe { (*self.seen).lines.last().cloned() }.unwrap_or_default();
            let hex: String = tx.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(last, format!("scmi > {hex}"), "request not logged before it was sent");
            self.inner.transfer(tx, rx)
        }
    }

    #[test]
    fn display_ffi_power_on_over_the_model() {
        let m = model();
        let bus = Bus::default();
        bus.set(RCC_BASE + RCC_DSICFGR, DSICFGR_DSIBLSEL | DSICFGR_DSIPHYCKREFSEL);
        let mut seen = Seen::default();
        let d = dev(&mut seen);
        let mut out = YdMp2DisplayPower::default();
        let mut t = LoggedFirst { inner: m.scmi(), seen: d.log_ctx as *const Seen };
        assert_eq!(power_on_over(&mut t, &bus, &d, &mut out), 0);
        assert_eq!(out.err, YdMp2DisplayErr::default());
        assert_eq!(out.supplies_on, 0b111);
        assert_eq!((out.hdmi_before, out.hdmi), (0, 7));
        assert_eq!(out.rate_hz, [0, 0, 100_000_000, 27_000_000, 74_250_000]);
        assert_eq!(out.rcc, [2, 0x9002, 2]);
        assert_eq!(out.displayclkcr, 2);
        assert_eq!(seen.delays, [display::RCC_RESET_US; 3]);
        // The rules: no clock disable, no voltage off, RATE_SET on 27 only.
        let mut rate_sets = 0;
        for (tx, _) in m.messages() {
            let h = scmi::word(&tx, 0);
            let (msg, proto) = ((h & 0xff) as u8, ((h >> 10) & 0xff) as u8);
            let w: Vec<u32> = (1..tx.len() / 4).map(|i| scmi::word(&tx, i)).collect();
            assert!(!display::forbidden(proto, msg, &w));
            match (proto, msg) {
                (scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET) => assert_eq!(w[1], scmi::CLOCK_ENABLE),
                (scmi::PROTO_VOLTAGE, scmi::VOLTAGE_CONFIG_SET) => assert_eq!(w, [21, 7]),
                (scmi::PROTO_CLOCK, scmi::CLOCK_RATE_SET) => {
                    assert_eq!(w, [0, 27, 74_250_000, 0]);
                    rate_sets += 1;
                }
                _ => {}
            }
        }
        assert_eq!(rate_sets, 1);
        assert_eq!(m.messages().len(), 24);
        assert!(m.faults().is_empty());
        assert!(m.voltage_on(21) && m.clock_enabled(27) && m.clock_enabled(12) && m.clock_enabled(28));
    }

    #[test]
    fn display_ffi_power_on_errors_in_c_form() {
        let m = model();
        m.force_status(scmi::PROTO_CLOCK, scmi::CLOCK_CONFIG_SET, scmi::STATUS_DENIED);
        let bus = Bus::default();
        let mut seen = Seen::default();
        let d = dev(&mut seen);
        let mut out = YdMp2DisplayPower::default();
        assert_eq!(power_on_over(&mut m.scmi(), &bus, &d, &mut out), -EIO);
        assert_eq!(
            out.err,
            YdMp2DisplayErr {
                kind: YD_MP2_DISPLAY_ERR_REFUSED,
                step: YD_MP2_DISPLAY_STEP_CLOCK_ENABLE,
                id: SCMI_CLK_GPIOB,
                a: scmi::STATUS_DENIED as u32,
                b: 0
            }
        );
        // LDO5 was switched on before the clocks; nothing reached the RCC.
        assert_eq!(out.hdmi, 7);
        assert!(bus.writes.borrow().is_empty());

        // No delay callback: -EINVAL, nothing sent.
        let m = model();
        let mut d2 = dev(&mut seen);
        d2.delay_us = None;
        assert_eq!(power_on_over(&mut m.scmi(), &bus, &d2, &mut out), -EINVAL);
        assert!(m.messages().is_empty());

        // A secure LTDC entry: err RIF with the id and the verdict.
        let m = model();
        bus.set(RIFSC_BASE + rifsc_seccfgr(RIFSC_LTDC_CMN), 1 << (RIFSC_LTDC_CMN % 32));
        let d = dev(&mut seen);
        assert_eq!(power_on_over(&mut m.scmi(), &bus, &d, &mut out), -EIO);
        assert_eq!(
            out.err,
            YdMp2DisplayErr { kind: YD_MP2_DISPLAY_ERR_RIF, step: 0, id: 80, a: YD_MP2_DISPLAY_RIF_SECURE, b: 0 }
        );
        assert!(m.messages().is_empty());
    }

    /// A secure side that refuses the first SMC.
    struct Refuse {
        calls: usize,
    }
    impl Smc for Refuse {
        fn smc(&mut self, _: [u64; 8], _: &mut Shm) -> [u64; 8] {
            self.calls += 1;
            [0xffff_ffff, 0, 0, 0, 0, 0, 0, 0]
        }
    }

    #[test]
    fn display_ffi_power_on_with_channel_and_open_failures() {
        let bus = Bus::default();
        let mut seen = Seen::default();
        let mut shm = Shm::new();
        let mut out = YdMp2DisplayPower::default();
        // Channel 1 (the M33's): refused before any SMC.
        let mut d = dev(&mut seen);
        d.channel = 1;
        let mut smc = Refuse { calls: 0 };
        assert_eq!(power_on_with(&mut smc, &mut shm, &bus, &d, &mut out), -EINVAL);
        assert_eq!(smc.calls, 0);
        assert_eq!((out.err.kind, out.err.step, out.err.b), (YD_MP2_DISPLAY_ERR_OPTEE, YD_MP2_DISPLAY_OPTEE_OPEN, 1));
        // OP-TEE does not answer the capabilities: -EIO, the SMC logged,
        // nothing written.
        let d = dev(&mut seen);
        let mut smc = Refuse { calls: 0 };
        assert_eq!(power_on_with(&mut smc, &mut shm, &bus, &d, &mut out), -EIO);
        assert_eq!(smc.calls, 1);
        assert_eq!((out.err.kind, out.err.step), (YD_MP2_DISPLAY_ERR_OPTEE, YD_MP2_DISPLAY_OPTEE_OPEN));
        assert!(seen.lines.iter().any(|l| l.starts_with("optee smc 0xb2000009 ")), "{:?}", seen.lines);
        assert!(bus.writes.borrow().is_empty());
    }

    impl Smc for &mut Refuse {
        fn smc(&mut self, r: [u64; 8], s: &mut Shm) -> [u64; 8] {
            (**self).smc(r, s)
        }
    }

    #[test]
    fn display_ffi_pins_and_adv_reset() {
        let bus = Bus::default();
        let mut seen = Seen::default();
        let d = dev(&mut seen);
        let mut out = YdMp2DisplayPins::default();
        // Before pins: PB6 is not an output.
        assert_eq!(adv_reset_with(&bus, &d, &mut out), -EIO);
        assert_eq!(out.err.kind, YD_MP2_DISPLAY_ERR_PIN_MODE);
        assert_eq!(out.err.id, ('B' as u32) << 8 | 6);
        assert_eq!(pins_with(&bus, &d, &mut out), 0);
        assert_eq!(out.gpiof_afrl, 0x909);
        assert_eq!(out.gpiob_moder, 1 << 12);
        assert_eq!(out.gpiof_moder, 2 << 4 | 2);
        assert_eq!(adv_reset_with(&bus, &d, &mut out), 0);
        assert_eq!(seen.delays, [1000, 10_000]);
        // A foreign PF2: refused, nothing more written.
        let bus = Bus::default();
        bus.set(GPIOF_BASE + gpio::r::cidcfgr(2), gpio::r::CID_CFEN | 2 << 4);
        assert_eq!(pins_with(&bus, &d, &mut out), -EIO);
        assert_eq!(
            out.err,
            YdMp2DisplayErr { kind: YD_MP2_DISPLAY_ERR_PIN, step: 0, id: ('F' as u32) << 8 | 2, a: 3, b: 0 }
        );
        assert!(bus.writes.borrow().is_empty());
    }

    #[test]
    fn display_ffi_rif_check_entries_and_fb() {
        let bus = Bus::default();
        let mut seen = Seen::default();
        let d = dev(&mut seen);
        let mut out = YdMp2DisplayRif::default();
        assert_eq!(rif_check_with(&bus, &d, 0xb000_0000, 32 << 20, &mut out), 0);
        assert_eq!(out.entries.map(|e| e.id), [80, 119, 81, 42, 120, 121, 122, 123]);
        assert_eq!((out.refused, out.fb_region, out.fb_verdict), (0, 9, YD_MP2_DISPLAY_FB_OK));
        assert_eq!(rif_check_with(&bus, &d, 0, 0, &mut out), 0);
        assert_eq!(out.fb_verdict, YD_MP2_DISPLAY_FB_SKIPPED);
        assert_eq!(rif_check_with(&bus, &d, 0xfe80_0000, 0x1000, &mut out), -ERANGE);
        assert_eq!(
            out.err,
            YdMp2DisplayErr { kind: YD_MP2_DISPLAY_ERR_FB, a: YD_MP2_DISPLAY_FB_SECURE_LAYER, ..Default::default() }
        );
        // An optional entry refused: reported, not an error.
        bus.set(RIFSC_BASE + rifsc_seccfgr(RIFSC_LTDC_L3), 1 << (RIFSC_LTDC_L3 % 32));
        assert_eq!(rif_check_with(&bus, &d, 0, 0, &mut out), 0);
        assert_eq!(out.refused, 1 << 4);
        // A required one: -EIO.
        bus.set(RIFSC_BASE + rifsc_cidcfgr(RIFSC_I2C2), display::CIDCFGR_CFEN | 2 << 4);
        assert_eq!(rif_check_with(&bus, &d, 0, 0, &mut out), -EIO);
        assert_eq!(out.refused, 1 << 4 | 1 << 3);
        assert_eq!(
            out.err,
            YdMp2DisplayErr { kind: YD_MP2_DISPLAY_ERR_RIF, step: 0, id: 42, a: YD_MP2_DISPLAY_RIF_FOREIGN_CID, b: 0 }
        );
        assert!(bus.writes.borrow().is_empty());
    }

    /// The `#define`s the header must carry, with the crate's values.
    fn defines() -> Vec<(&'static str, u32)> {
        vec![
            ("YD_MP2_DISPLAY_SHM_SIZE", YD_MP2_DISPLAY_SHM_SIZE as u32),
            ("YD_MP2_DISPLAY_SHM_ALIGN", YD_MP2_DISPLAY_SHM_ALIGN as u32),
            ("YD_MP2_DISPLAY_ERR_NONE", YD_MP2_DISPLAY_ERR_NONE),
            ("YD_MP2_DISPLAY_ERR_OPTEE", YD_MP2_DISPLAY_ERR_OPTEE),
            ("YD_MP2_DISPLAY_ERR_TRANSPORT", YD_MP2_DISPLAY_ERR_TRANSPORT),
            ("YD_MP2_DISPLAY_ERR_REFUSED", YD_MP2_DISPLAY_ERR_REFUSED),
            ("YD_MP2_DISPLAY_ERR_ECHO", YD_MP2_DISPLAY_ERR_ECHO),
            ("YD_MP2_DISPLAY_ERR_LENGTH", YD_MP2_DISPLAY_ERR_LENGTH),
            ("YD_MP2_DISPLAY_ERR_READBACK", YD_MP2_DISPLAY_ERR_READBACK),
            ("YD_MP2_DISPLAY_ERR_FORBIDDEN", YD_MP2_DISPLAY_ERR_FORBIDDEN),
            ("YD_MP2_DISPLAY_ERR_RIF", YD_MP2_DISPLAY_ERR_RIF),
            ("YD_MP2_DISPLAY_ERR_RCC", YD_MP2_DISPLAY_ERR_RCC),
            ("YD_MP2_DISPLAY_ERR_MUX", YD_MP2_DISPLAY_ERR_MUX),
            ("YD_MP2_DISPLAY_ERR_SYSCFG", YD_MP2_DISPLAY_ERR_SYSCFG),
            ("YD_MP2_DISPLAY_ERR_PIN", YD_MP2_DISPLAY_ERR_PIN),
            ("YD_MP2_DISPLAY_ERR_PIN_MODE", YD_MP2_DISPLAY_ERR_PIN_MODE),
            ("YD_MP2_DISPLAY_ERR_FB", YD_MP2_DISPLAY_ERR_FB),
            ("YD_MP2_DISPLAY_OPTEE_OPEN", YD_MP2_DISPLAY_OPTEE_OPEN),
            ("YD_MP2_DISPLAY_OPTEE_CLOSE", YD_MP2_DISPLAY_OPTEE_CLOSE),
            ("YD_MP2_DISPLAY_STEP_VOLTAGE_VERSION", YD_MP2_DISPLAY_STEP_VOLTAGE_VERSION),
            ("YD_MP2_DISPLAY_STEP_SUPPLY_GET", YD_MP2_DISPLAY_STEP_SUPPLY_GET),
            ("YD_MP2_DISPLAY_STEP_HDMI_GET", YD_MP2_DISPLAY_STEP_HDMI_GET),
            ("YD_MP2_DISPLAY_STEP_HDMI_SET", YD_MP2_DISPLAY_STEP_HDMI_SET),
            ("YD_MP2_DISPLAY_STEP_HDMI_CHECK", YD_MP2_DISPLAY_STEP_HDMI_CHECK),
            ("YD_MP2_DISPLAY_STEP_CLOCK_VERSION", YD_MP2_DISPLAY_STEP_CLOCK_VERSION),
            ("YD_MP2_DISPLAY_STEP_CLOCK_ENABLE", YD_MP2_DISPLAY_STEP_CLOCK_ENABLE),
            ("YD_MP2_DISPLAY_STEP_CLOCK_RATE_SET", YD_MP2_DISPLAY_STEP_CLOCK_RATE_SET),
            ("YD_MP2_DISPLAY_STEP_CLOCK_ATTRIBUTES", YD_MP2_DISPLAY_STEP_CLOCK_ATTRIBUTES),
            ("YD_MP2_DISPLAY_STEP_CLOCK_RATE_GET", YD_MP2_DISPLAY_STEP_CLOCK_RATE_GET),
            ("YD_MP2_DISPLAY_RIF_OK", YD_MP2_DISPLAY_RIF_OK),
            ("YD_MP2_DISPLAY_RIF_SECURE", YD_MP2_DISPLAY_RIF_SECURE),
            ("YD_MP2_DISPLAY_RIF_FOREIGN_CID", YD_MP2_DISPLAY_RIF_FOREIGN_CID),
            ("YD_MP2_DISPLAY_FB_OK", YD_MP2_DISPLAY_FB_OK),
            ("YD_MP2_DISPLAY_FB_EMPTY", YD_MP2_DISPLAY_FB_EMPTY),
            ("YD_MP2_DISPLAY_FB_UNALIGNED", YD_MP2_DISPLAY_FB_UNALIGNED),
            ("YD_MP2_DISPLAY_FB_ABOVE_4G", YD_MP2_DISPLAY_FB_ABOVE_4G),
            ("YD_MP2_DISPLAY_FB_IMAGE", YD_MP2_DISPLAY_FB_IMAGE),
            ("YD_MP2_DISPLAY_FB_SECURE_LAYER", YD_MP2_DISPLAY_FB_SECURE_LAYER),
            ("YD_MP2_DISPLAY_FB_NOT_ALLOWED", YD_MP2_DISPLAY_FB_NOT_ALLOWED),
            ("YD_MP2_DISPLAY_FB_SKIPPED", YD_MP2_DISPLAY_FB_SKIPPED),
        ]
    }

    #[test]
    fn display_ffi_the_c_header_matches() {
        let h: Vec<String> = HEADER.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
        let has = |s: &str| h.iter().any(|l| l.starts_with(s));
        for (name, v) in defines() {
            assert!(has(&format!("#define {name} {v}u")), "#define {name} {v}u");
        }
        for (s, n) in [
            ("yd_mp2_display", size_of::<YdMp2Display>()),
            ("yd_mp2_display_err", size_of::<YdMp2DisplayErr>()),
            ("yd_mp2_display_power", size_of::<YdMp2DisplayPower>()),
            ("yd_mp2_display_pins", size_of::<YdMp2DisplayPins>()),
            ("yd_mp2_display_rif_entry", size_of::<YdMp2DisplayRifEntry>()),
            ("yd_mp2_display_rif", size_of::<YdMp2DisplayRif>()),
        ] {
            assert!(has(&format!("_Static_assert(sizeof(struct {s}) == {n},")), "sizeof {s} {n}");
        }
        // Every exported function, with the crate's signature.
        for decl in [
            "int32_t yd_mp2_display_power_on(const struct yd_mp2_display *d, struct yd_mp2_display_power *out);",
            "int32_t yd_mp2_display_pins(const struct yd_mp2_display *d, struct yd_mp2_display_pins *out);",
            "int32_t yd_mp2_display_adv_reset(const struct yd_mp2_display *d, struct yd_mp2_display_pins *out);",
            "int32_t yd_mp2_display_rif_check(const struct yd_mp2_display *d, uint64_t fb_base, uint64_t fb_size,",
        ] {
            assert!(has(decl), "{decl}");
        }
        assert!(has("struct yd_mp2_display_rif *out);"));
        let exported = include_str!("display_ffi.rs").matches(concat!("extern \"C\" fn ", "yd_mp2_display_")).count();
        assert_eq!(exported, 4, "a new entry point needs its declaration above");
    }
}
