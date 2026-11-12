// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32MP257F's GPU power, identity and reset for C: what the mp2_gpu
//! probe (tools/probes/stm32mp257f_dk/mp2_gpu) calls. Plan #322, issues #7
//! and #1281. Declarations: driver/zephyr/lib/board/stm32mp2/include/
//! stm32mp2_gpu.h.
//!
//! - [`yd_mp2_gpu_power_on`]: OP-TEE opened (capabilities, the data page
//!   registered, a session to the SCMI pseudo-TA, the channel), then
//!   [`power::power_up`] (performance level, power domain 0, the RCC gate and
//!   reset, idle), then the session closed.
//! - [`yd_mp2_gpu_identify`]: [`hw::identify`] (the table-driven identity).
//! - [`yd_mp2_gpu_intr_check`]: `HI_INTR_ENBL` written, read back, then 0
//!   written again (no interrupt handler is connected).
//! - [`yd_mp2_gpu_reset_idle`]: the RCC reset asserted and released, then
//!   `HI_IDLE_STATE` read until idle, at most `idle_reads` times.
//!
//! Every SMC, every OP-TEE message frame and every SCMI request and reply is
//! one line to the caller's log callback, the request before it is sent.
//!
//! The GPU's registers are only reached through [`Window`]: an access at or
//! above [`power::REG_WINDOW`] (0x800, the state space) is refused without
//! touching the bus and reported as [`YD_MP2_GPU_ERR_WINDOW`].
//!
//! The generic functions (`*_with`) take the SMC conduit and the buses, so
//! the host tests run them against fakes; the `extern "C"` functions give
//! them [`HwSmc`] and [`Hw`].

use super::power::{self, Config, Level, Status, Step};
use super::rcc::GpuRcc;
use crate::env::{Env, Level as LogLevel};
use crate::errno::{EFAULT, EINVAL, EIO, ENODEV, ENOTSUP, ETIMEDOUT};
use crate::firmware::optee::{self, Client, Event, Shm, Smc};
use crate::gpu::vivante::hw;
use crate::gpu::vivante::regs::VIVS_HI_INTR_ENBL;
use crate::mmio::Mmio;
use core::cell::Cell;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};

/// `yd_mp2_gpu_log_fn`: one line, `len` bytes, not NUL-terminated.
pub type LogFn = unsafe extern "C" fn(ctx: *mut c_void, line: *const u8, len: usize);

/// `struct yd_mp2_gpu`: where everything is.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdMp2Gpu {
    /// The RCC's mapped address (`RCC_GPUCFGR` at +0x83C).
    pub rcc_base: usize,
    /// The GPU's mapped registers, [`power::REG_WINDOW`] bytes.
    pub gpu_base: usize,
    /// [`YD_MP2_GPU_SHM_SIZE`] bytes, 4 KiB aligned, normal cached memory.
    pub shm: *mut c_void,
    /// Its physical address, as OP-TEE is given it.
    pub shm_pa: u64,
    /// Optional.
    pub log: Option<LogFn>,
    pub log_ctx: *mut c_void,
    /// Bound of every idle poll; 0 takes [`Config::DK`]'s.
    pub idle_reads: u32,
    /// The SCMI channel (0 on the DK).
    pub channel: u32,
}

const _: () = assert!(size_of::<YdMp2Gpu>() == 56);
const _: () = assert!(offset_of!(YdMp2Gpu, rcc_base) == 0);
const _: () = assert!(offset_of!(YdMp2Gpu, gpu_base) == 8);
const _: () = assert!(offset_of!(YdMp2Gpu, shm) == 16);
const _: () = assert!(offset_of!(YdMp2Gpu, shm_pa) == 24);
const _: () = assert!(offset_of!(YdMp2Gpu, log) == 32);
const _: () = assert!(offset_of!(YdMp2Gpu, log_ctx) == 40);
const _: () = assert!(offset_of!(YdMp2Gpu, idle_reads) == 48);
const _: () = assert!(offset_of!(YdMp2Gpu, channel) == 52);

/// The shared memory's size and alignment.
pub const YD_MP2_GPU_SHM_SIZE: usize = 3 * optee::PAGE;
pub const YD_MP2_GPU_SHM_ALIGN: usize = optee::PAGE;
const _: () = assert!(size_of::<Shm>() == YD_MP2_GPU_SHM_SIZE);
const _: () = assert!(core::mem::align_of::<Shm>() == YD_MP2_GPU_SHM_ALIGN);

/// `err.kind`. `step` is a [`Step`] number (`YD_MP2_GPU_STEP_*`) for the
/// SCMI kinds, 1 (open) or 2 (close) for [`YD_MP2_GPU_ERR_OPTEE`].
pub const YD_MP2_GPU_ERR_NONE: u32 = 0;
/// `a`: the OP-TEE error's code (`optee::Error::code`).
pub const YD_MP2_GPU_ERR_OPTEE: u32 = 1;
/// `a`: the transport's code.
pub const YD_MP2_GPU_ERR_TRANSPORT: u32 = 2;
/// `a`: the SCMI status (negative).
pub const YD_MP2_GPU_ERR_REFUSED: u32 = 3;
/// `a`: the header sent, `b`: the header received.
pub const YD_MP2_GPU_ERR_ECHO: u32 = 4;
/// `a`: the reply's length.
pub const YD_MP2_GPU_ERR_LENGTH: u32 = 5;
/// `a`: the version reported.
pub const YD_MP2_GPU_ERR_VERSION: u32 = 6;
/// `a`: wanted, `b`: read back.
pub const YD_MP2_GPU_ERR_READBACK: u32 = 7;
/// `a`: `RCC_GPUCFGR` as read.
pub const YD_MP2_GPU_ERR_RCC: u32 = 8;
/// `a`: the reads made, `b`: the last `HI_IDLE_STATE`.
pub const YD_MP2_GPU_ERR_IDLE_TIMEOUT: u32 = 9;
/// `a`: the GPU offset refused (at or above 0x800, or below the base).
pub const YD_MP2_GPU_ERR_WINDOW: u32 = 10;

/// `err.step` for [`YD_MP2_GPU_ERR_OPTEE`].
pub const YD_MP2_GPU_OPTEE_OPEN: u32 = 1;
pub const YD_MP2_GPU_OPTEE_CLOSE: u32 = 2;

/// `struct yd_mp2_gpu_err`: why a call failed (all 0 when it did not).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2GpuErr {
    pub kind: u32,
    pub step: u32,
    pub a: u32,
    pub b: u32,
}

/// `struct yd_mp2_gpu_up`: what the power-on found.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2GpuUp {
    pub level_before: u32,
    pub level: u32,
    pub rcc: u32,
    pub idle: u32,
    pub idle_reads: u32,
    pub err: YdMp2GpuErr,
}

const _: () = assert!(size_of::<YdMp2GpuUp>() == 36);
const _: () = assert!(offset_of!(YdMp2GpuUp, idle_reads) == 16);
const _: () = assert!(offset_of!(YdMp2GpuUp, err) == 20);

/// `struct yd_mp2_gpu_id`: the identity registers, and what the table added.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2GpuId {
    pub model: u32,
    pub revision: u32,
    pub product_id: u32,
    pub eco_id: u32,
    pub customer_id: u32,
    pub date: u32,
    pub time: u32,
    pub features: u32,
    /// 1 when a row of `hw::DB` matched.
    pub from_table: u32,
    pub stream_count: u32,
    pub err: YdMp2GpuErr,
}

const _: () = assert!(size_of::<YdMp2GpuId>() == 56);
const _: () = assert!(offset_of!(YdMp2GpuId, from_table) == 32);
const _: () = assert!(offset_of!(YdMp2GpuId, err) == 40);

/// `struct yd_mp2_gpu_reset`: the reset to idle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp2GpuReset {
    /// `RCC_GPUCFGR` with the reset asserted, then after its release.
    pub rcc_held: u32,
    pub rcc: u32,
    pub idle: u32,
    pub idle_reads: u32,
    pub err: YdMp2GpuErr,
}

const _: () = assert!(size_of::<YdMp2GpuReset>() == 32);
const _: () = assert!(offset_of!(YdMp2GpuReset, err) == 16);

fn step_no(s: Step) -> u32 {
    match s {
        Step::PerfVersion => 1,
        Step::PerfLevelGet => 2,
        Step::PerfLevelSet => 3,
        Step::PerfLevelCheck => 4,
        Step::PowerVersion => 5,
        Step::PowerOn => 6,
        Step::PowerOnCheck => 7,
        Step::PowerOff => 8,
        Step::PowerOffCheck => 9,
    }
}

fn status_code(s: Status) -> i32 {
    use crate::firmware::scmi;
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

/// The C form of a [`power::Error`], and the errno it is returned as.
fn power_err(e: power::Error) -> (YdMp2GpuErr, i32) {
    use power::Error as E;
    let (kind, step, a, b) = match e {
        E::Transport { step, code } => (YD_MP2_GPU_ERR_TRANSPORT, step_no(step), code as u32, 0),
        E::Refused { step, status } => (YD_MP2_GPU_ERR_REFUSED, step_no(step), status_code(status) as u32, 0),
        E::Echo { step, sent, got } => (YD_MP2_GPU_ERR_ECHO, step_no(step), sent, got),
        E::Length { step, len } => (YD_MP2_GPU_ERR_LENGTH, step_no(step), len as u32, 0),
        E::Version { step, version } => (YD_MP2_GPU_ERR_VERSION, step_no(step), version, 0),
        E::Readback { step, want, got } => (YD_MP2_GPU_ERR_READBACK, step_no(step), want, got),
        E::Rcc { raw } => (YD_MP2_GPU_ERR_RCC, 0, raw, 0),
        E::IdleTimeout { reads, last } => (YD_MP2_GPU_ERR_IDLE_TIMEOUT, 0, reads, last),
    };
    let rc = if kind == YD_MP2_GPU_ERR_IDLE_TIMEOUT { -ETIMEDOUT } else { -EIO };
    (YdMp2GpuErr { kind, step, a, b }, rc)
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

/// [`hw::identify`]'s log, to the callback (level dropped).
struct LogEnv(CLog);

impl Env for LogEnv {
    fn busy_wait_us(&self, _: u32) {}
    fn sleep_ms(&self, _: u32) {}
    fn log_str(&self, _: LogLevel, line: &str) {
        self.0.emit(line);
    }
    fn i2c_write(&self, _: u32, _: &[u8]) -> Result<(), i32> {
        Err(-ENOTSUP)
    }
    fn i2c_write_read(&self, _: u32, _: &[u8], _: &mut [u8]) -> Result<(), i32> {
        Err(-ENOTSUP)
    }
    fn gpio_set(&self, _: u32, _: bool) -> Result<(), i32> {
        Err(-ENOTSUP)
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
/// reply are [`power`]'s `scmi >` / `scmi <` lines.
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

/// The GPU's register window: `base` to `base + 0x800`. An access outside
/// it does not reach the bus; reads give 0xdeadde00 (the interconnect's
/// absent pattern, which `hw::identify` refuses), and the first refused
/// offset is kept.
pub struct Window<'a, M: Mmio> {
    m: &'a M,
    base: usize,
    refused: Cell<Option<usize>>,
}

impl<'a, M: Mmio> Window<'a, M> {
    pub fn new(m: &'a M, base: usize) -> Self {
        Window { m, base, refused: Cell::new(None) }
    }

    fn ok(&self, addr: usize) -> bool {
        let inside = addr >= self.base && addr - self.base < power::REG_WINDOW;
        if !inside && self.refused.get().is_none() {
            self.refused.set(Some(addr.wrapping_sub(self.base)));
        }
        inside
    }

    /// The first offset refused, if any.
    pub fn refused(&self) -> Option<usize> {
        self.refused.get()
    }
}

impl<M: Mmio> Mmio for Window<'_, M> {
    fn read32(&self, addr: usize) -> u32 {
        if self.ok(addr) {
            self.m.read32(addr)
        } else {
            0xdead_de00
        }
    }
    fn write32(&self, addr: usize, val: u32) {
        if self.ok(addr) {
            self.m.write32(addr, val)
        }
    }
    fn read8(&self, addr: usize) -> u8 {
        if self.ok(addr) {
            self.m.read8(addr)
        } else {
            0
        }
    }
    fn write8(&self, addr: usize, val: u8) {
        if self.ok(addr) {
            self.m.write8(addr, val)
        }
    }
}

fn window_err(off: usize) -> YdMp2GpuErr {
    YdMp2GpuErr { kind: YD_MP2_GPU_ERR_WINDOW, step: 0, a: off as u32, b: 0 }
}

fn config(idle_reads: u32) -> Config {
    Config {
        idle_reads: if idle_reads == 0 { Config::DK.idle_reads } else { idle_reads },
        level: Level::Current,
        ..Config::DK
    }
}

/// The power-on over `smc`: OP-TEE opened on `channel`, [`power::power_up`],
/// the session closed (also after a failure). 0, `-EIO` (the reason in
/// `out.err`), `-ETIMEDOUT` when the GPU did not report idle, `-EFAULT` for a
/// GPU access outside the window.
#[allow(clippy::too_many_arguments)]
pub fn power_on_with<S: Smc, R: Mmio, G: Mmio>(
    smc: S,
    shm: &mut Shm,
    shm_pa: u64,
    channel: u32,
    rcc: &R,
    rcc_base: usize,
    gpu: &G,
    gpu_base: usize,
    idle_reads: u32,
    log: CLog,
    out: &mut YdMp2GpuUp,
) -> i32 {
    *out = YdMp2GpuUp::default();
    if channel != power::SCMI_CHANNEL {
        log.fmt(format_args!("optee channel {} refused: only {} carries the GPU", channel, power::SCMI_CHANNEL));
        let a = optee::Error::Unsupported.code() as u32;
        out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_OPTEE, step: YD_MP2_GPU_OPTEE_OPEN, a, b: channel };
        return -EINVAL;
    }
    let mut c = Client::new(LoggedSmc { smc, log }, shm, shm_pa);
    if let Err(e) = c.open(0, channel) {
        log.fmt(format_args!("optee open failed: {:?} (code {})", e, e.code()));
        out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_OPTEE, step: YD_MP2_GPU_OPTEE_OPEN, a: e.code() as u32, b: 0 };
        let _ = c.close();
        return -EIO;
    }
    let win = Window::new(gpu, gpu_base);
    let mut l = log;
    let r = power::power_up(
        &mut c,
        &GpuRcc::new(rcc, rcc_base),
        |off| win.read32(gpu_base + off),
        &mut l,
        &config(idle_reads),
    );
    let closed = c.close();
    if let Some(off) = win.refused() {
        out.err = window_err(off);
        return -EFAULT;
    }
    match r {
        Ok(up) => {
            out.level_before = up.level_before;
            out.level = up.level;
            out.rcc = up.rcc;
            out.idle = up.idle;
            out.idle_reads = up.idle_reads;
        }
        Err(e) => {
            let (err, rc) = power_err(e);
            out.err = err;
            if let power::Error::IdleTimeout { reads, .. } = e {
                out.idle_reads = reads;
            }
            return rc;
        }
    }
    if let Err(e) = closed {
        log.fmt(format_args!("optee close failed: {:?} (code {})", e, e.code()));
        out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_OPTEE, step: YD_MP2_GPU_OPTEE_CLOSE, a: e.code() as u32, b: 0 };
        return -EIO;
    }
    0
}

/// [`hw::identify`] through the window. 0 or `-ENODEV` (no answer), or
/// `-EFAULT` for an access outside the window.
pub fn identify_with<G: Mmio>(gpu: &G, gpu_base: usize, log: CLog, out: &mut YdMp2GpuId) -> i32 {
    *out = YdMp2GpuId::default();
    let win = Window::new(gpu, gpu_base);
    let r = hw::identify(&win, gpu_base, &LogEnv(log));
    if let Some(off) = win.refused() {
        out.err = window_err(off);
        return -EFAULT;
    }
    match r {
        Ok(id) => {
            out.model = id.model;
            out.revision = id.revision;
            out.product_id = id.product_id;
            out.eco_id = id.eco_id;
            out.customer_id = id.customer_id;
            out.date = id.date;
            out.time = id.time;
            out.features = id.features;
            out.from_table = id.from_table as u32;
            out.stream_count = id.stream_count;
            0
        }
        Err(e) => {
            out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_READBACK, step: 0, a: e as u32, b: 0 };
            if e == -ENODEV {
                -ENODEV
            } else {
                -EIO
            }
        }
    }
}

/// `HI_INTR_ENBL`: `value` written, read back into `readback`, 0 written.
/// 0 when the read back equals `value`, else `-EIO`.
pub fn intr_check_with<G: Mmio>(gpu: &G, gpu_base: usize, value: u32, log: CLog, readback: &mut u32) -> i32 {
    let win = Window::new(gpu, gpu_base);
    let a = gpu_base + VIVS_HI_INTR_ENBL as usize;
    win.write32(a, value);
    *readback = win.read32(a);
    win.write32(a, 0);
    log.fmt(format_args!("gpu intr_enbl wrote 0x{:08x} read 0x{:08x}, then 0", value, *readback));
    if *readback == value {
        0
    } else {
        -EIO
    }
}

/// The RCC reset asserted, read, released, read; then `HI_IDLE_STATE` until
/// idle, at most `idle_reads` (0: [`Config::DK`]'s) reads. 0, `-EIO` when
/// `RCC_GPUCFGR` does not read back as written (gate on, reset held, then
/// released), `-ETIMEDOUT` when the GPU did not report idle.
pub fn reset_idle_with<R: Mmio, G: Mmio>(
    rcc: &R,
    rcc_base: usize,
    gpu: &G,
    gpu_base: usize,
    idle_reads: u32,
    log: CLog,
    out: &mut YdMp2GpuReset,
) -> i32 {
    *out = YdMp2GpuReset::default();
    let r = GpuRcc::new(rcc, rcc_base);
    r.assert_reset();
    let held = r.status();
    log.fmt(format_args!("rcc gpucfgr 0x{:08x} (reset held)", held.raw));
    r.deassert_reset();
    let s = r.status();
    log.fmt(format_args!("rcc gpucfgr 0x{:08x}", s.raw));
    out.rcc_held = held.raw;
    out.rcc = s.raw;
    if !held.clock_enabled() || !held.reset_asserted() {
        out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_RCC, step: 1, a: held.raw, b: 0 };
        return -EIO;
    }
    if !s.clock_enabled() || s.reset_asserted() {
        out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_RCC, step: 2, a: s.raw, b: 0 };
        return -EIO;
    }
    let win = Window::new(gpu, gpu_base);
    let n = config(idle_reads).idle_reads;
    let mut last = 0;
    for i in 1..=n {
        last = win.read32(gpu_base + power::IDLE_STATE);
        if last & power::IDLE_MASK == power::IDLE_MASK {
            log.fmt(format_args!("gpu idle 0x{:08x} after {} reads", last, i));
            out.idle = last;
            out.idle_reads = i;
            return 0;
        }
    }
    log.fmt(format_args!("gpu not idle 0x{:08x} after {} reads", last, n));
    out.idle = last;
    out.idle_reads = n;
    out.err = YdMp2GpuErr { kind: YD_MP2_GPU_ERR_IDLE_TIMEOUT, step: 0, a: n, b: last };
    -ETIMEDOUT
}

/// The C entry points: the hardware conduit and buses.
#[cfg(target_arch = "aarch64")]
mod abi {
    use super::*;
    use crate::firmware::optee::HwSmc;
    use crate::mmio::Hw;

    /// The `struct yd_mp2_gpu` behind a C pointer, or `None`.
    ///
    /// # Safety
    /// `g` is null or points to a valid `struct yd_mp2_gpu`.
    unsafe fn cfg<'a>(g: *const YdMp2Gpu) -> Option<&'a YdMp2Gpu> {
        // SAFETY: the caller's contract.
        unsafe { g.as_ref() }
    }

    fn clog(g: &YdMp2Gpu) -> CLog {
        CLog { f: g.log, ctx: g.log_ctx }
    }

    /// # Safety
    /// `g` and `out` valid; `g->rcc_base` and `g->gpu_base` mapped device memory
    /// (the RCC's page with `RCC_GPUCFGR`, the GPU's 0x800 bytes); `g->shm`
    /// [`YD_MP2_GPU_SHM_SIZE`] bytes of normal memory owned by the caller for
    /// the call, at physical address `g->shm_pa`; the secure monitor routes
    /// OP-TEE's SMCs.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_gpu_power_on(g: *const YdMp2Gpu, out: *mut YdMp2GpuUp) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(g), Some(out)) = (unsafe { cfg(g) }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        let p = g.shm as *mut Shm;
        if p.is_null()
            || !(p as usize).is_multiple_of(YD_MP2_GPU_SHM_ALIGN)
            || !g.shm_pa.is_multiple_of(YD_MP2_GPU_SHM_ALIGN as u64)
        {
            return -EINVAL;
        }
        // SAFETY: the caller's contract; aligned and sized as Shm (any bytes are
        // a valid Shm).
        let shm = unsafe { &mut *p };
        // SAFETY: the caller's contract.
        let (smc, hw) = unsafe { (HwSmc::new(), Hw::new()) };
        power_on_with(smc, shm, g.shm_pa, g.channel, &hw, g.rcc_base, &hw, g.gpu_base, g.idle_reads, clog(g), out)
    }

    /// # Safety
    /// `g` and `out` valid; `g->gpu_base` mapped device memory (0x800 bytes),
    /// the GPU powered.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_gpu_identify(g: *const YdMp2Gpu, out: *mut YdMp2GpuId) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(g), Some(out)) = (unsafe { cfg(g) }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        identify_with(&hw, g.gpu_base, clog(g), out)
    }

    /// # Safety
    /// `g` and `readback` valid; `g->gpu_base` mapped device memory, the GPU
    /// powered.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_gpu_intr_check(g: *const YdMp2Gpu, value: u32, readback: *mut u32) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(g), Some(readback)) = (unsafe { cfg(g) }, unsafe { readback.as_mut() }) else {
            return -EINVAL;
        };
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        intr_check_with(&hw, g.gpu_base, value, clog(g), readback)
    }

    /// # Safety
    /// `g` and `out` valid; `g->rcc_base` and `g->gpu_base` mapped device
    /// memory, the GPU powered.
    #[no_mangle]
    pub unsafe extern "C" fn yd_mp2_gpu_reset_idle(g: *const YdMp2Gpu, out: *mut YdMp2GpuReset) -> i32 {
        // SAFETY: the caller's contract.
        let (Some(g), Some(out)) = (unsafe { cfg(g) }, unsafe { out.as_mut() }) else {
            return -EINVAL;
        };
        // SAFETY: the caller's contract.
        let hw = unsafe { Hw::new() };
        reset_idle_with(&hw, g.rcc_base, &hw, g.gpu_base, g.idle_reads, clog(g), out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::vivante::regs::*;
    use std::boxed::Box;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::format;
    use std::rc::Rc;
    use std::string::{String, ToString};
    use std::vec;
    use std::vec::Vec;

    // The vectors of mp2-scmi-vectors (tools/optee/scmi_vectors.py), with
    // their addresses: arg page 0x88000000, page list 0x88001000, data page
    // 0x88002000, cookie 1, session 1, channel 0.
    const FILES: [&str; 4] = [
        include_str!("../../../../../tools/testdata/optee-scmi/smc.txt"),
        include_str!("../../../../../tools/testdata/optee-scmi/msg_arg.txt"),
        include_str!("../../../../../tools/testdata/optee-scmi/invoke_frames.txt"),
        include_str!("../../../../../tools/testdata/optee-scmi/scmi_msgs.txt"),
    ];
    const PA: u64 = 0x8800_0000;
    const RCC_BASE: usize = 0x4420_0000;
    const RCC_REG: usize = RCC_BASE + 0x83C;
    const GPU_BASE: usize = 0x4828_0000;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn hexs(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn v(name: &str) -> Vec<u8> {
        for f in FILES {
            for l in f.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let (n, h) = l.split_once(' ').unwrap();
                if n == name {
                    return hex(h);
                }
            }
        }
        panic!("no vector {name}")
    }

    fn regs(name: &str) -> [u64; 8] {
        let b = v(name);
        assert_eq!(b.len(), 64);
        core::array::from_fn(|i| u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap()))
    }

    /// The vectors' open-session frame with the login REE_KERNEL (param 1's
    /// c), which ST's SCMI pseudo-TA requires (as optee/tests.rs).
    fn ree_kernel(mut b: Vec<u8>) -> Vec<u8> {
        b[88..96].copy_from_slice(&optee::LOGIN_REE_KERNEL.to_le_bytes());
        b
    }

    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    /// Everything that reached a bus or the secure side, in order.
    #[derive(Debug, PartialEq, Clone)]
    enum Ev {
        Smc(&'static str),
        RccR(u32),
        RccW(u32),
        GpuR(usize),
        GpuW(usize, u32),
    }

    type Events = Rc<RefCell<Vec<Ev>>>;

    /// The RCC's `RCC_GPUCFGR` and the GPU's 0x800 bytes of registers. Any
    /// access to the GPU's state space (offset 0x800 and above) panics, as
    /// does any other address.
    struct Bus {
        rcc: RefCell<u32>,
        gpu: RefCell<HashMap<usize, u32>>,
        /// `HI_IDLE_STATE`'s answers, in order (then 0x7fffffff).
        idle: RefCell<VecDeque<u32>>,
        /// The bits of `HI_INTR_ENBL` that hold a write.
        intr_mask: u32,
        ev: Events,
    }

    impl Bus {
        fn new(rcc: u32, idle: &[u32]) -> Bus {
            Bus {
                rcc: RefCell::new(rcc),
                gpu: RefCell::default(),
                idle: RefCell::new(idle.iter().copied().collect()),
                intr_mask: !0,
                ev: Events::default(),
            }
        }
        fn events(&self) -> Vec<Ev> {
            self.ev.borrow().clone()
        }
        fn gpu_off(addr: usize) -> usize {
            assert!(
                (GPU_BASE..GPU_BASE + 0x1_0000).contains(&addr),
                "address 0x{addr:x} is neither the RCC's nor the GPU's"
            );
            let off = addr - GPU_BASE;
            assert!(off < 0x800, "GPU state space access at offset 0x{off:x}");
            off
        }
    }

    impl Mmio for Bus {
        fn read32(&self, addr: usize) -> u32 {
            if addr == RCC_REG {
                let r = *self.rcc.borrow();
                self.ev.borrow_mut().push(Ev::RccR(r));
                return r;
            }
            let off = Bus::gpu_off(addr);
            self.ev.borrow_mut().push(Ev::GpuR(off));
            if off == power::IDLE_STATE {
                return self.idle.borrow_mut().pop_front().unwrap_or(0x7fff_ffff);
            }
            self.gpu.borrow().get(&off).copied().unwrap_or(0)
        }
        fn write32(&self, addr: usize, val: u32) {
            if addr == RCC_REG {
                self.ev.borrow_mut().push(Ev::RccW(val));
                *self.rcc.borrow_mut() = val;
                return;
            }
            let off = Bus::gpu_off(addr);
            self.ev.borrow_mut().push(Ev::GpuW(off, val));
            let val = if off == VIVS_HI_INTR_ENBL as usize { val & self.intr_mask } else { val };
            self.gpu.borrow_mut().insert(off, val);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    /// One SMC the secure side expects: the registers, and the arg page,
    /// page list and data page as they must be; then its answer.
    struct Call {
        name: &'static str,
        call: [u64; 8],
        arg: Option<Vec<u8>>,
        list: Option<Vec<u8>>,
        data: Option<Vec<u8>>,
        ret: [u64; 8],
        arg_rep: Option<Vec<u8>>,
        data_rep: Option<Vec<u8>>,
    }

    /// The secure side: checks each SMC against the script, records it in
    /// the bus's event list (so SMCs and register accesses are in one
    /// order), answers it. The script left over is handed back on drop.
    struct FakeSmc {
        script: VecDeque<Call>,
        ev: Events,
        rest: Rc<RefCell<VecDeque<Call>>>,
    }

    impl Smc for FakeSmc {
        fn smc(&mut self, regs: [u64; 8], shm: &mut Shm) -> [u64; 8] {
            let c = self.script.pop_front().unwrap_or_else(|| panic!("unscripted SMC {regs:x?}"));
            assert_eq!(regs, c.call, "{}: registers", c.name);
            if let Some(a) = &c.arg {
                assert_eq!(hexs(&shm.arg[..a.len()]), hexs(a), "{}: message frame", c.name);
            }
            if let Some(l) = &c.list {
                assert_eq!(shm.list[..], l[..], "{}: page list", c.name);
            }
            if let Some(d) = &c.data {
                assert_eq!(hexs(&shm.data[..d.len()]), hexs(d), "{}: SCMI request", c.name);
            }
            self.ev.borrow_mut().push(Ev::Smc(c.name));
            if let Some(a) = &c.arg_rep {
                shm.arg[..a.len()].copy_from_slice(a);
            }
            if let Some(d) = &c.data_rep {
                shm.data[optee::DATA_RX..optee::DATA_RX + d.len()].copy_from_slice(d);
            }
            c.ret
        }
    }

    impl Drop for FakeSmc {
        fn drop(&mut self) {
            *self.rest.borrow_mut() = core::mem::take(&mut self.script);
        }
    }

    fn std_call(name: &'static str, req: Vec<u8>, rep: Vec<u8>) -> Call {
        Call {
            name,
            call: regs("call_with_rpc_arg"),
            arg: Some(req),
            list: None,
            data: None,
            ret: regs("ret_call_ok"),
            arg_rep: Some(rep),
            data_rep: None,
        }
    }

    fn open_calls() -> Vec<Call> {
        let caps = Call {
            name: "exchange_capabilities",
            call: regs("call_exchange_capabilities"),
            arg: None,
            list: None,
            data: None,
            ret: regs("ret_exchange_capabilities"),
            arg_rep: None,
            data_rep: None,
        };
        let mut reg = std_call("register_shm", v("register_shm_req"), v("register_shm_reply"));
        reg.list = Some(v("page_list_one_page"));
        vec![
            caps,
            reg,
            std_call("open_session", ree_kernel(v("open_session_req")), ree_kernel(v("open_session_reply"))),
            std_call("get_channel_handle", v("invoke_get_channel_handle_req"), v("invoke_get_channel_handle_reply")),
        ]
    }

    fn close_calls() -> Vec<Call> {
        vec![
            std_call("close_session", v("close_session_req"), v("close_session_reply")),
            std_call("unregister_shm", v("unregister_shm_req"), v("unregister_shm_reply")),
        ]
    }

    /// One PROCESS_MSG_CHANNEL carrying `req_<n>` and answered `rep_<n>`.
    fn scmi(n: &str) -> Call {
        let mut c = std_call(leak(n.to_string()), v(&format!("invoke_{n}_req")), v(&format!("invoke_{n}_reply")));
        c.data = Some(v(&format!("req_{n}")));
        c.data_rep = Some(v(&format!("rep_{n}")));
        c
    }

    /// The power-up's messages, in the order st-power's tests pin.
    const UP: [&str; 7] = [
        "perf_protocol_version",
        "perf_level_get",
        "perf_level_set",
        "perf_level_get",
        "pd_protocol_version",
        "pd_state_set_on",
        "pd_state_get_on",
    ];
    const OPEN: [&str; 4] = ["exchange_capabilities", "register_shm", "open_session", "get_channel_handle"];
    const CLOSE: [&str; 2] = ["close_session", "unregister_shm"];

    fn up_calls() -> Vec<Call> {
        UP.iter().map(|n| scmi(n)).collect()
    }

    fn with_open_close(mid: Vec<Call>) -> Vec<Call> {
        let mut s = open_calls();
        s.extend(mid);
        s.extend(close_calls());
        s
    }

    unsafe extern "C" fn collect(ctx: *mut c_void, line: *const u8, len: usize) {
        // SAFETY: ctx is the test's Vec<String>; line and len the callback's.
        let (lines, b) = unsafe { (&mut *(ctx as *mut Vec<String>), core::slice::from_raw_parts(line, len)) };
        lines.push(String::from_utf8(b.to_vec()).unwrap());
    }

    fn clog(lines: &mut Vec<String>) -> CLog {
        CLog { f: Some(collect), ctx: lines as *mut Vec<String> as *mut c_void }
    }

    /// [`power_on_with`] over the script; returns the code and what of the
    /// script was not used.
    fn power_on(
        calls: Vec<Call>,
        bus: &Bus,
        idle_reads: u32,
        lines: &mut Vec<String>,
        out: &mut YdMp2GpuUp,
    ) -> (i32, usize) {
        let rest: Rc<RefCell<VecDeque<Call>>> = Rc::default();
        let smc = FakeSmc { script: calls.into(), ev: bus.ev.clone(), rest: rest.clone() };
        let mut shm = Box::new(Shm::new());
        let rc = power_on_with(smc, &mut shm, PA, 0, bus, RCC_BASE, bus, GPU_BASE, idle_reads, clog(lines), out);
        let left = rest.borrow().len();
        (rc, left)
    }

    fn smc_evs(names: &[&'static str]) -> Vec<Ev> {
        names.iter().map(|n| Ev::Smc(n)).collect()
    }

    #[test]
    fn power_on_order_and_bytes() {
        let bus = Bus::new(0x100, &[0, 0x7fff_fffe, 0xffff_ffff]);
        let mut lines = vec![];
        let mut out = YdMp2GpuUp::default();
        assert_eq!(power_on(with_open_close(up_calls()), &bus, 0, &mut lines, &mut out), (0, 0));
        assert_eq!(
            out,
            YdMp2GpuUp {
                level_before: 800_000,
                level: 800_000,
                rcc: 0x102,
                idle: 0xffff_ffff,
                idle_reads: 3,
                err: YdMp2GpuErr::default()
            }
        );
        let mut want = smc_evs(&OPEN);
        want.extend(smc_evs(&UP));
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
            Ev::GpuR(power::IDLE_STATE),
            Ev::GpuR(power::IDLE_STATE),
            Ev::GpuR(power::IDLE_STATE),
        ]);
        want.extend(smc_evs(&CLOSE));
        assert_eq!(bus.events(), want);
    }

    /// Every line the C callback receives, byte for byte: the SMCs, the
    /// message frames (the vectors), the SCMI messages, the RCC and idle.
    #[test]
    fn power_on_log_bytes() {
        let bus = Bus::new(0, &[0x7fff_ffff]);
        let mut lines = vec![];
        let mut out = YdMp2GpuUp::default();
        assert_eq!(power_on(with_open_close(up_calls()), &bus, 0, &mut lines, &mut out), (0, 0));

        let std_smc = "optee smc 0x32000012 0x0 0x88000000 0x0 -> 0x0 0x0 0x0 0x0".to_string();
        let frame = |req: Vec<u8>, rep: Vec<u8>| -> Vec<String> {
            vec![format!("optee msg > {}", hexs(&req)), std_smc.clone(), format!("optee msg < {}", hexs(&rep))]
        };
        let mut want = vec!["optee smc 0xb2000009 0x0 0x0 0x0 -> 0x0 0x74 0x45 0x4".to_string()];
        want.extend(frame(v("register_shm_req"), v("register_shm_reply")));
        want.extend(frame(ree_kernel(v("open_session_req")), ree_kernel(v("open_session_reply"))));
        want.extend(frame(v("invoke_get_channel_handle_req"), v("invoke_get_channel_handle_reply")));
        for n in UP {
            want.push(format!("scmi > {}", hexs(&v(&format!("req_{n}")))));
            want.extend(frame(v(&format!("invoke_{n}_req")), v(&format!("invoke_{n}_reply"))));
            want.push(format!("scmi < {}", hexs(&v(&format!("rep_{n}")))));
        }
        want.push("rcc gpucfgr 0x00000002".to_string());
        want.push("gpu idle 0x7fffffff after 1 reads".to_string());
        want.extend(frame(v("close_session_req"), v("close_session_reply")));
        want.extend(frame(v("unregister_shm_req"), v("unregister_shm_reply")));
        assert_eq!(lines, want);

        // The SCMI lines are st-power's, literally.
        let scmi: Vec<&str> = lines.iter().filter(|l| l.starts_with("scmi")).map(|l| l.as_str()).collect();
        assert_eq!(
            scmi,
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
            ]
        );
    }

    #[test]
    fn power_on_refused_stops_before_rcc_and_closes() {
        let mut mid: Vec<Call> = up_calls().into_iter().take(6).collect();
        mid[5].arg_rep = Some(v("invoke_pd_state_set_async_flag_rejected_reply"));
        mid[5].data_rep = Some(v("rep_pd_state_set_async_flag_rejected"));
        let bus = Bus::new(0, &[]);
        let mut lines = vec![];
        let mut out = YdMp2GpuUp::default();
        assert_eq!(power_on(with_open_close(mid), &bus, 0, &mut lines, &mut out), (-EIO, 0));
        assert_eq!(out.err, YdMp2GpuErr { kind: YD_MP2_GPU_ERR_REFUSED, step: 6, a: -2i32 as u32, b: 0 });
        let mut want = smc_evs(&OPEN);
        want.extend(smc_evs(&UP[..6]));
        want.extend(smc_evs(&CLOSE));
        assert_eq!(bus.events(), want);
        assert!(lines.contains(&"scmi < 04440400feffffff".to_string()));
    }

    #[test]
    fn optee_open_refused_stops_before_scmi() {
        let mut s = open_calls();
        s.truncate(3);
        s[2].arg_rep = Some(v("open_session_unknown_uuid_reply"));
        s.push(std_call("unregister_shm", v("unregister_shm_req"), v("unregister_shm_reply")));
        let bus = Bus::new(0, &[]);
        let mut lines = vec![];
        let mut out = YdMp2GpuUp::default();
        assert_eq!(power_on(s, &bus, 0, &mut lines, &mut out), (-EIO, 0));
        assert_eq!(
            out.err,
            YdMp2GpuErr { kind: YD_MP2_GPU_ERR_OPTEE, step: YD_MP2_GPU_OPTEE_OPEN, a: 0xffff_0008, b: 0 }
        );
        assert_eq!(bus.events(), smc_evs(&["exchange_capabilities", "register_shm", "open_session", "unregister_shm"]));
        assert!(lines.iter().any(|l| l.starts_with("optee open failed: Tee")));
    }

    #[test]
    fn power_on_refuses_another_channel_before_any_smc() {
        // Channel 1 is the Cortex-M33's SMT mailbox (mod_optee_mbx.c:88-89).
        let bus = Bus::new(0, &[]);
        let mut lines = vec![];
        let mut out = YdMp2GpuUp::default();
        let smc = FakeSmc { script: VecDeque::new(), ev: bus.ev.clone(), rest: Rc::default() };
        let mut shm = Box::new(Shm::new());
        let rc = power_on_with(smc, &mut shm, PA, 1, &bus, RCC_BASE, &bus, GPU_BASE, 0, clog(&mut lines), &mut out);
        assert_eq!(rc, -EINVAL);
        assert_eq!(out.err, YdMp2GpuErr { kind: YD_MP2_GPU_ERR_OPTEE, step: YD_MP2_GPU_OPTEE_OPEN, a: 0x101, b: 1 });
        assert!(bus.events().is_empty());
        assert_eq!(lines, ["optee channel 1 refused: only 0 carries the GPU"]);
    }

    #[test]
    fn power_on_idle_timeout() {
        let bus = Bus::new(0, &[0, 1, 0x8000_0000]);
        let mut lines = vec![];
        let mut out = YdMp2GpuUp::default();
        assert_eq!(power_on(with_open_close(up_calls()), &bus, 3, &mut lines, &mut out), (-ETIMEDOUT, 0));
        assert_eq!(out.err, YdMp2GpuErr { kind: YD_MP2_GPU_ERR_IDLE_TIMEOUT, step: 0, a: 3, b: 0x8000_0000 });
        assert_eq!(bus.events().iter().filter(|e| **e == Ev::GpuR(power::IDLE_STATE)).count(), 3);
        assert_eq!(bus.events().last(), Some(&Ev::Smc("unregister_shm")));
    }

    fn st_gpu(bus: &Bus) {
        let mut g = bus.gpu.borrow_mut();
        g.insert(VIVS_HI_CHIP_IDENTITY as usize, 0x0e00_0000);
        g.insert(VIVS_HI_CHIP_MODEL as usize, 0x8000);
        g.insert(VIVS_HI_CHIP_REV as usize, 0x6205);
        g.insert(VIVS_HI_CHIP_PRODUCT_ID as usize, 0x8_0003);
        g.insert(VIVS_HI_CHIP_ECO_ID as usize, 0);
        g.insert(VIVS_HI_CHIP_CUSTOMER_ID as usize, 0x15);
        g.insert(VIVS_HI_CHIP_DATE as usize, 0x2021_0202);
        g.insert(VIVS_HI_CHIP_TIME as usize, 0x1234_5678);
        g.insert(VIVS_HI_CHIP_FEATURE as usize, 0xe028_7cad);
    }

    #[test]
    fn identify_st_row() {
        let bus = Bus::new(0, &[]);
        st_gpu(&bus);
        let mut lines = vec![];
        let mut out = YdMp2GpuId::default();
        assert_eq!(identify_with(&bus, GPU_BASE, clog(&mut lines), &mut out), 0);
        assert_eq!(
            (out.model, out.revision, out.product_id, out.eco_id, out.customer_id, out.from_table, out.stream_count),
            (0x8000, 0x6205, 0x8_0003, 0, 0x15, 1, 16)
        );
        assert_eq!((out.date, out.time, out.features), (0x2021_0202, 0x1234_5678, 0xe028_7cad));
        let reads: Vec<Ev> = [
            VIVS_HI_CHIP_IDENTITY,
            VIVS_HI_CHIP_MODEL,
            VIVS_HI_CHIP_REV,
            VIVS_HI_CHIP_DATE,
            VIVS_HI_CHIP_TIME,
            VIVS_HI_CHIP_CUSTOMER_ID,
            VIVS_HI_CHIP_PRODUCT_ID,
            VIVS_HI_CHIP_ECO_ID,
            VIVS_HI_CHIP_FEATURE,
            VIVS_HI_CHIP_MINOR_FEATURE_0,
            VIVS_HI_CHIP_MINOR_FEATURE_1,
            VIVS_HI_CHIP_MINOR_FEATURE_2,
            VIVS_HI_CHIP_MINOR_FEATURE_3,
            VIVS_HI_CHIP_MINOR_FEATURE_4,
            VIVS_HI_CHIP_MINOR_FEATURE_5,
        ]
        .iter()
        .map(|o| Ev::GpuR(*o as usize))
        .collect();
        assert_eq!(bus.events(), reads);
        assert!(lines[0].starts_with("identity 0x0e000000: GC8000 rev 0x6205 product 0x80003 customer 0x15 eco 0"));
    }

    #[test]
    fn identify_dark_gpu_is_enodev() {
        let bus = Bus::new(0, &[]);
        let mut lines = vec![];
        let mut out = YdMp2GpuId::default();
        assert_eq!(identify_with(&bus, GPU_BASE, clog(&mut lines), &mut out), -ENODEV);
        assert_eq!(out.err.kind, YD_MP2_GPU_ERR_READBACK);
    }

    #[test]
    fn intr_check_writes_reads_and_clears() {
        let bus = Bus::new(0, &[]);
        let mut lines = vec![];
        let mut rb = 0;
        assert_eq!(intr_check_with(&bus, GPU_BASE, 0xffff_ffff, clog(&mut lines), &mut rb), 0);
        assert_eq!(rb, 0xffff_ffff);
        let a = VIVS_HI_INTR_ENBL as usize;
        assert_eq!(bus.events(), [Ev::GpuW(a, 0xffff_ffff), Ev::GpuR(a), Ev::GpuW(a, 0)]);
        assert_eq!(lines, ["gpu intr_enbl wrote 0xffffffff read 0xffffffff, then 0"]);

        let mut bus = Bus::new(0, &[]);
        bus.intr_mask = 0xffff;
        assert_eq!(intr_check_with(&bus, GPU_BASE, 0xffff_ffff, clog(&mut lines), &mut rb), -EIO);
        assert_eq!(rb, 0xffff);
    }

    #[test]
    fn reset_idle_order_and_lines() {
        let bus = Bus::new(0x2, &[0x7fff_fffe, 0x7fff_ffff]);
        let mut lines = vec![];
        let mut out = YdMp2GpuReset::default();
        assert_eq!(reset_idle_with(&bus, RCC_BASE, &bus, GPU_BASE, 0, clog(&mut lines), &mut out), 0);
        assert_eq!(
            out,
            YdMp2GpuReset { rcc_held: 3, rcc: 2, idle: 0x7fff_ffff, idle_reads: 2, err: YdMp2GpuErr::default() }
        );
        assert_eq!(
            bus.events(),
            [
                Ev::RccR(2),
                Ev::RccW(3),
                Ev::RccR(3),
                Ev::RccR(3),
                Ev::RccW(2),
                Ev::RccR(2),
                Ev::GpuR(power::IDLE_STATE),
                Ev::GpuR(power::IDLE_STATE),
            ]
        );
        assert_eq!(
            lines,
            ["rcc gpucfgr 0x00000003 (reset held)", "rcc gpucfgr 0x00000002", "gpu idle 0x7fffffff after 2 reads"]
        );
    }

    #[test]
    fn reset_idle_with_the_gate_off_is_an_rcc_error() {
        let bus = Bus::new(0, &[]);
        let mut lines = vec![];
        let mut out = YdMp2GpuReset::default();
        assert_eq!(reset_idle_with(&bus, RCC_BASE, &bus, GPU_BASE, 0, clog(&mut lines), &mut out), -EIO);
        assert_eq!(out.err, YdMp2GpuErr { kind: YD_MP2_GPU_ERR_RCC, step: 1, a: 1, b: 0 });
        assert!(!bus.events().iter().any(|e| matches!(e, Ev::GpuR(_) | Ev::GpuW(..))));
    }

    #[test]
    fn reset_idle_timeout() {
        let bus = Bus::new(0x2, &[0, 0, 0]);
        let mut lines = vec![];
        let mut out = YdMp2GpuReset::default();
        assert_eq!(reset_idle_with(&bus, RCC_BASE, &bus, GPU_BASE, 3, clog(&mut lines), &mut out), -ETIMEDOUT);
        assert_eq!(out.err, YdMp2GpuErr { kind: YD_MP2_GPU_ERR_IDLE_TIMEOUT, step: 0, a: 3, b: 0 });
        assert_eq!(lines.last().unwrap(), "gpu not idle 0x00000000 after 3 reads");
    }

    /// The fake itself refuses the state space: what every test above ran
    /// against.
    #[test]
    #[should_panic(expected = "GPU state space access at offset 0x800")]
    fn the_fake_panics_at_0x800() {
        Bus::new(0, &[]).read32(GPU_BASE + 0x800);
    }

    /// The window stops any access at or above 0x800 (or below the base)
    /// before the bus, which would panic.
    #[test]
    fn window_refuses_the_state_space() {
        let bus = Bus::new(0, &[]);
        let w = Window::new(&bus, GPU_BASE);
        assert_eq!(w.read32(GPU_BASE + 0x800), 0xdead_de00);
        w.write32(GPU_BASE + 0xffc, 1);
        w.write32(GPU_BASE + 0x1_0000, 1);
        assert!(bus.events().is_empty());
        assert_eq!(w.refused(), Some(0x800));
        assert_eq!(w.read32(GPU_BASE + 0x7fc), 0);
        assert_eq!(bus.events(), [Ev::GpuR(0x7fc)]);
        let w = Window::new(&bus, GPU_BASE);
        assert_eq!(w.read32(GPU_BASE - 4), 0xdead_de00);
        assert_eq!(w.refused(), Some(usize::MAX - 3));
    }

    /// Identity, interrupt check and reset in a row: no access at or above
    /// 0x800 (the bus would have panicked), none refused.
    #[test]
    fn no_entry_point_reaches_the_state_space() {
        let bus = Bus::new(0x2, &[]);
        st_gpu(&bus);
        let mut lines = vec![];
        let mut id = YdMp2GpuId::default();
        let mut rb = 0;
        let mut rs = YdMp2GpuReset::default();
        assert_eq!(identify_with(&bus, GPU_BASE, clog(&mut lines), &mut id), 0);
        assert_eq!(intr_check_with(&bus, GPU_BASE, 1, clog(&mut lines), &mut rb), 0);
        assert_eq!(reset_idle_with(&bus, RCC_BASE, &bus, GPU_BASE, 0, clog(&mut lines), &mut rs), 0);
        assert!(bus.events().iter().all(|e| match e {
            Ev::GpuR(o) | Ev::GpuW(o, _) => *o < power::REG_WINDOW,
            _ => true,
        }));
    }

    #[test]
    fn error_mapping() {
        use power::Error as E;
        assert_eq!(
            power_err(E::Echo { step: Step::PowerOn, sent: 1, got: 2 }),
            (YdMp2GpuErr { kind: YD_MP2_GPU_ERR_ECHO, step: 6, a: 1, b: 2 }, -EIO)
        );
        assert_eq!(
            power_err(E::Readback { step: Step::PerfLevelCheck, want: 3, got: 4 }),
            (YdMp2GpuErr { kind: YD_MP2_GPU_ERR_READBACK, step: 4, a: 3, b: 4 }, -EIO)
        );
        assert_eq!(power_err(E::Rcc { raw: 3 }).0.kind, YD_MP2_GPU_ERR_RCC);
        assert_eq!(power_err(E::IdleTimeout { reads: 1, last: 0 }).1, -ETIMEDOUT);
        assert_eq!(
            power_err(E::Refused { step: Step::PowerOffCheck, status: Status::Other(-42) }).0,
            YdMp2GpuErr { kind: YD_MP2_GPU_ERR_REFUSED, step: 9, a: -42i32 as u32, b: 0 }
        );
        for c in -10..=-1 {
            assert_eq!(status_code(Status::from_code(c)), c);
        }
    }
}
