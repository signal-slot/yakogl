// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The MP2's I2C controller (`yakogl,stm32mp2-i2c`, ip::stm32_i2c) for the
//! Zephyr I2C driver driver/drivers/i2c/. A polled master: [`yd_mp25_i2c_init`]
//! programs `TIMINGR` and the filters, [`yd_mp25_i2c_write`],
//! [`yd_mp25_i2c_write_read`] and [`yd_mp25_i2c_transfer`] move bytes. The
//! clock gate, the reset, the pins and the RIF setting are not done here.
//! Declarations: include/yakogl_mp25_i2c.h.

use crate::env::{Env, YdEnv};
use crate::errno::{EINVAL, ENOTSUP};
use crate::ip::stm32_i2c::{self as i2c, Config};
#[cfg(not(test))]
use crate::mmio::Hw;
use crate::mmio::Mmio;
use crate::ylog;
use core::mem::{offset_of, size_of};

/// `flags` of [`YdMp25I2cConfig`]: `i2c-analog-filter` (the analog filter
/// stays on; clear it for `CR1.ANFOFF`).
pub const YD_MP25_I2C_ANALOG_FILTER: u32 = 1 << 0;

/// `struct yd_mp25_i2c_config`, from the devicetree node.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YdMp25I2cConfig {
    /// The controller's registers (the shim's `device_map()`).
    pub base: usize,
    /// `kernel-clock-frequency`, Hz.
    pub kernel_clock_hz: u32,
    /// `clock-frequency`: 100000, 400000 or 1000000.
    pub speed_hz: u32,
    /// `i2c-scl-rising-time-ns`; 0 takes Linux's 25.
    pub rise_ns: u32,
    /// `i2c-scl-falling-time-ns`; 0 takes Linux's 10.
    pub fall_ns: u32,
    /// `i2c-digital-filter-width-ns`; 0 for no digital filter.
    pub dnf_ns: u32,
    /// `YD_MP25_I2C_*`.
    pub flags: u32,
}

/// `struct yd_mp25_i2c_data`: what [`yd_mp25_i2c_init`] left.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdMp25I2cData {
    /// The `TIMINGR` value written.
    pub timingr: u32,
    /// The digital filter setting (`CR1.DNF`) written.
    pub dnf: u32,
}

/// `struct yd_mp25_i2c_msg`, Zephyr's `struct i2c_msg` on LP64.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdMp25I2cMsg {
    pub buf: *mut u8,
    pub len: u32,
    /// Zephyr's `I2C_MSG_*` (`YD_MP25_I2C_MSG_*`).
    pub flags: u8,
}

/// Zephyr's `I2C_MSG_WRITE` (0), `I2C_MSG_READ` (1), `I2C_MSG_STOP` (2),
/// `I2C_MSG_RESTART` (4).
pub const YD_MP25_I2C_MSG_WRITE: u8 = 0;
pub const YD_MP25_I2C_MSG_READ: u8 = 1;
pub const YD_MP25_I2C_MSG_STOP: u8 = 2;
pub const YD_MP25_I2C_MSG_RESTART: u8 = 4;

const _: () = assert!(size_of::<YdMp25I2cConfig>() == 32);
const _: () = assert!(offset_of!(YdMp25I2cConfig, base) == 0);
const _: () = assert!(offset_of!(YdMp25I2cConfig, kernel_clock_hz) == 8);
const _: () = assert!(offset_of!(YdMp25I2cConfig, speed_hz) == 12);
const _: () = assert!(offset_of!(YdMp25I2cConfig, rise_ns) == 16);
const _: () = assert!(offset_of!(YdMp25I2cConfig, fall_ns) == 20);
const _: () = assert!(offset_of!(YdMp25I2cConfig, dnf_ns) == 24);
const _: () = assert!(offset_of!(YdMp25I2cConfig, flags) == 28);
const _: () = assert!(size_of::<YdMp25I2cData>() == 8);
const _: () = assert!(offset_of!(YdMp25I2cData, timingr) == 0);
const _: () = assert!(offset_of!(YdMp25I2cData, dnf) == 4);
const _: () = assert!(size_of::<YdMp25I2cMsg>() == 16);
const _: () = assert!(offset_of!(YdMp25I2cMsg, buf) == 0);
const _: () = assert!(offset_of!(YdMp25I2cMsg, len) == 8);
const _: () = assert!(offset_of!(YdMp25I2cMsg, flags) == 12);

/// The registers the extern entry points reach: the hardware on the board.
#[cfg(not(test))]
fn port() -> Hw {
    // SAFETY: `cfg->base` is the shim's `device_map()` range (the entry
    // points' contract).
    unsafe { Hw::new() }
}

/// In the host tests the same entry points reach a register model installed
/// with `tests::with_bus`, as the sibling FFI modules' tests do.
#[cfg(test)]
struct Port;

#[cfg(test)]
thread_local! {
    static BUS: core::cell::Cell<Option<*const dyn Mmio>> = const { core::cell::Cell::new(None) };
}

#[cfg(test)]
fn port() -> Port {
    Port
}

#[cfg(test)]
impl Mmio for Port {
    fn read32(&self, a: usize) -> u32 {
        // SAFETY: `with_bus` keeps the model alive while it is installed.
        BUS.with(|b| unsafe { &*b.get().expect("no bus installed") }.read32(a))
    }
    fn write32(&self, a: usize, v: u32) {
        // SAFETY: as read32.
        BUS.with(|b| unsafe { &*b.get().expect("no bus installed") }.write32(a, v))
    }
    fn read8(&self, a: usize) -> u8 {
        // SAFETY: as read32.
        BUS.with(|b| unsafe { &*b.get().expect("no bus installed") }.read8(a))
    }
    fn write8(&self, a: usize, v: u8) {
        // SAFETY: as read32.
        BUS.with(|b| unsafe { &*b.get().expect("no bus installed") }.write8(a, v))
    }
}

fn ip_config(cfg: &YdMp25I2cConfig) -> Config {
    let mut c = Config::new(cfg.kernel_clock_hz, cfg.speed_hz);
    if cfg.rise_ns != 0 {
        c.rise_ns = cfg.rise_ns;
    }
    if cfg.fall_ns != 0 {
        c.fall_ns = cfg.fall_ns;
    }
    c.analog_filter = cfg.flags & YD_MP25_I2C_ANALOG_FILTER != 0;
    c.dnf_ns = cfg.dnf_ns;
    c
}

/// A byte slice of a C buffer; an empty one may come with a null pointer.
///
/// # Safety
/// `buf` is valid for `len` bytes when `len` != 0.
unsafe fn bytes<'a>(buf: *const u8, len: usize) -> Result<&'a [u8], i32> {
    if len == 0 {
        return Ok(&[]);
    }
    if buf.is_null() {
        return Err(-EINVAL);
    }
    // SAFETY: the caller's contract.
    Ok(unsafe { core::slice::from_raw_parts(buf, len) })
}

fn unit(r: Result<(), i32>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => e,
    }
}

pub(crate) fn init_with(m: &impl Mmio, env: &impl Env, cfg: &YdMp25I2cConfig, data: &mut YdMp25I2cData) -> i32 {
    match i2c::init(m, cfg.base, &ip_config(cfg)) {
        Ok(t) => {
            data.timingr = t.timingr();
            data.dnf = u32::from(t.dnf);
            ylog!(
                env,
                Inf,
                "mp25_i2c: {} Hz from {} Hz, TIMINGR {:#010x}",
                cfg.speed_hz,
                cfg.kernel_clock_hz,
                data.timingr
            );
            0
        }
        Err(e) => {
            ylog!(env, Err, "mp25_i2c: no timing for {} Hz from {} Hz ({})", cfg.speed_hz, cfg.kernel_clock_hz, e);
            e
        }
    }
}

/// One write, or a write and then a read.
///
/// # Safety
/// Each message's `buf` is valid for its `len` (writable for a read).
pub(crate) unsafe fn transfer_with(m: &impl Mmio, base: usize, addr: u16, msgs: &[YdMp25I2cMsg]) -> i32 {
    if addr > 0x7f {
        return -EINVAL;
    }
    let addr = addr as u8;
    let is_read = |x: &YdMp25I2cMsg| x.flags & YD_MP25_I2C_MSG_READ != 0;
    match msgs {
        [w] if !is_read(w) => {
            // SAFETY: the caller's contract.
            match unsafe { bytes(w.buf, w.len as usize) } {
                Ok(d) => unit(i2c::write(m, base, addr, d)),
                Err(e) => e,
            }
        }
        [w, r] if !is_read(w) && is_read(r) => {
            // SAFETY: the caller's contract.
            let wr = match unsafe { bytes(w.buf, w.len as usize) } {
                Ok(d) => d,
                Err(e) => return e,
            };
            if r.buf.is_null() {
                return -EINVAL;
            }
            // SAFETY: the caller's contract; `r.buf` is non-null.
            let rd = unsafe { core::slice::from_raw_parts_mut(r.buf, r.len as usize) };
            unit(i2c::write_read(m, base, addr, wr, rd))
        }
        [] => -EINVAL,
        // A read alone, a write after a read, three or more: ip::stm32_i2c
        // has no such transfer.
        _ => -ENOTSUP,
    }
}

/// Program `TIMINGR`, the filters and PE (`stm32f7_i2c_hw_config`). 0, or
/// `-EINVAL` when no timing meets the spec for the clock and speed.
///
/// # Safety
/// `cfg`, `data`, `env` valid for the call; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_i2c_init(
    cfg: *const YdMp25I2cConfig,
    data: *mut YdMp25I2cData,
    env: *const YdEnv,
) -> i32 {
    if cfg.is_null() || data.is_null() || env.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (cfg, data, env) = unsafe { (&*cfg, &mut *data, &*env) };
    init_with(&port(), env, cfg, data)
}

/// START, `len` (1..=255) bytes to the 7-bit `addr`, STOP. 0, `-ENODEV` for
/// a NACK, `-EINVAL` for a bad length or address, `-EBUSY`/`-EIO` for the bus.
///
/// # Safety
/// `cfg` valid; `buf` valid for `len` bytes; `cfg->base` mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_i2c_write(cfg: *const YdMp25I2cConfig, addr: u16, buf: *const u8, len: usize) -> i32 {
    if cfg.is_null() {
        return -EINVAL;
    }
    let m = YdMp25I2cMsg { buf: buf as *mut u8, len: len.min(u32::MAX as usize) as u32, flags: YD_MP25_I2C_MSG_WRITE };
    // SAFETY: the caller's contract.
    unsafe { transfer_with(&port(), (*cfg).base, addr, &[m]) }
}

/// `wlen` bytes, a repeated START, `rlen` bytes, STOP; each 1..=255.
/// Errors as [`yd_mp25_i2c_write`].
///
/// # Safety
/// `cfg` valid; `wbuf` valid for `wlen`, `rbuf` writable for `rlen` bytes.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_i2c_write_read(
    cfg: *const YdMp25I2cConfig,
    addr: u16,
    wbuf: *const u8,
    wlen: usize,
    rbuf: *mut u8,
    rlen: usize,
) -> i32 {
    if cfg.is_null() {
        return -EINVAL;
    }
    let lim = |n: usize| n.min(u32::MAX as usize) as u32;
    let msgs = [
        YdMp25I2cMsg { buf: wbuf as *mut u8, len: lim(wlen), flags: YD_MP25_I2C_MSG_WRITE },
        YdMp25I2cMsg { buf: rbuf, len: lim(rlen), flags: YD_MP25_I2C_MSG_READ },
    ];
    // SAFETY: the caller's contract.
    unsafe { transfer_with(&port(), (*cfg).base, addr, &msgs) }
}

/// Zephyr's `i2c_transfer` message list: one write, or a write then a read.
/// STOP/RESTART flags are implied by that shape. `-ENOTSUP` for any other
/// list (a read alone, three messages), else as [`yd_mp25_i2c_write`].
///
/// # Safety
/// `cfg` valid; `msgs` valid for `num_msgs` entries, each buffer valid for
/// its `len`.
#[no_mangle]
pub unsafe extern "C" fn yd_mp25_i2c_transfer(
    cfg: *const YdMp25I2cConfig,
    msgs: *mut YdMp25I2cMsg,
    num_msgs: u8,
    addr: u16,
) -> i32 {
    if cfg.is_null() || (msgs.is_null() && num_msgs != 0) {
        return -EINVAL;
    }
    let list: &[YdMp25I2cMsg] = if num_msgs == 0 {
        &[]
    } else {
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts(msgs, num_msgs as usize) }
    };
    // SAFETY: the caller's contract.
    unsafe { transfer_with(&port(), (*cfg).base, addr, list) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ip::stm32_i2c::*;
    use crate::model::TestEnv;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    const BASE: usize = 0x4013_0000;

    /// A controller with one device at `addr`; ISR follows the last START.
    struct Bus {
        addr: u8,
        regs: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
        rx: RefCell<VecDeque<u8>>,
        left: RefCell<u32>,
        read: RefCell<bool>,
        autoend: RefCell<bool>,
        nack: RefCell<bool>,
        active: RefCell<bool>,
        stop: RefCell<bool>,
    }

    impl Bus {
        fn new(addr: u8, rx: &[u8]) -> Bus {
            Bus {
                addr,
                regs: RefCell::new(HashMap::new()),
                writes: RefCell::new(vec![]),
                rx: RefCell::new(rx.iter().copied().collect()),
                left: RefCell::new(0),
                read: RefCell::new(false),
                autoend: RefCell::new(false),
                nack: RefCell::new(false),
                active: RefCell::new(false),
                stop: RefCell::new(false),
            }
        }
    }

    impl Mmio for Bus {
        fn read32(&self, a: usize) -> u32 {
            match a - BASE {
                ISR => {
                    if *self.nack.borrow() {
                        return ISR_NACKF | ISR_TXE | if *self.stop.borrow() { ISR_STOPF } else { 0 };
                    }
                    if !*self.active.borrow() {
                        return 0;
                    }
                    let v = match (*self.read.borrow(), *self.left.borrow()) {
                        (false, l) if l > 0 => ISR_TXIS,
                        (true, l) if l > 0 => ISR_RXNE,
                        _ if *self.autoend.borrow() => ISR_STOPF,
                        _ => ISR_TC,
                    };
                    v | ISR_BUSY
                }
                o => *self.regs.borrow().get(&o).unwrap_or(&0),
            }
        }
        fn write32(&self, a: usize, val: u32) {
            let off = a - BASE;
            self.writes.borrow_mut().push((off, val));
            match off {
                CR2 if val & CR2_START != 0 => {
                    *self.read.borrow_mut() = val & CR2_RD_WRN != 0;
                    *self.autoend.borrow_mut() = val & CR2_AUTOEND != 0;
                    *self.left.borrow_mut() = (val & CR2_NBYTES) >> 16;
                    *self.active.borrow_mut() = true;
                    if ((val & CR2_SADD7) >> 1) as u8 != self.addr {
                        *self.nack.borrow_mut() = true;
                        *self.stop.borrow_mut() = true;
                    }
                }
                ICR => {
                    if val & ICR_NACKCF != 0 {
                        *self.nack.borrow_mut() = false;
                    }
                    if val & ICR_STOPCF != 0 {
                        *self.stop.borrow_mut() = false;
                        *self.active.borrow_mut() = false;
                    }
                }
                _ => {
                    self.regs.borrow_mut().insert(off, val);
                }
            }
        }
        fn read8(&self, _a: usize) -> u8 {
            *self.left.borrow_mut() -= 1;
            self.rx.borrow_mut().pop_front().expect("RXDR with nothing queued")
        }
        fn write8(&self, a: usize, val: u8) {
            *self.left.borrow_mut() -= 1;
            self.writes.borrow_mut().push((a - BASE, u32::from(val)));
        }
    }

    fn cfg() -> YdMp25I2cConfig {
        YdMp25I2cConfig {
            base: BASE,
            kernel_clock_hz: 64_000_000,
            speed_hz: 400_000,
            rise_ns: 0,
            fall_ns: 0,
            dnf_ns: 0,
            flags: YD_MP25_I2C_ANALOG_FILTER,
        }
    }

    fn msg(buf: &mut [u8], flags: u8) -> YdMp25I2cMsg {
        YdMp25I2cMsg { buf: buf.as_mut_ptr(), len: buf.len() as u32, flags }
    }

    fn xfer(m: &Bus, addr: u16, msgs: &[YdMp25I2cMsg]) -> i32 {
        // SAFETY: the buffers belong to the caller's locals.
        unsafe { transfer_with(m, BASE, addr, msgs) }
    }

    fn cr2(addr: u8, rd: bool, n: u32, autoend: bool) -> u32 {
        (u32::from(addr) << 1)
            | if rd { CR2_RD_WRN } else { 0 }
            | (n << 16)
            | if autoend { CR2_AUTOEND } else { 0 }
            | CR2_START
    }

    #[test]
    fn init_programs_the_timing_ip_computes() {
        let m = Bus::new(0x3d, &[]);
        let env = TestEnv::default();
        let mut d = YdMp25I2cData::default();
        assert_eq!(init_with(&m, &env, &cfg(), &mut d), 0);
        let want = compute_timing(&Config::new(64_000_000, 400_000)).unwrap().timingr();
        assert_eq!(d.timingr, want);
        assert_eq!(d.timingr, 0x0070424c);
        let w = m.writes.borrow();
        assert_eq!(w[1], (TIMINGR, want));
        assert_eq!(w.last(), Some(&(CR1, CR1_PE)));
        assert_eq!(env.lines.borrow().len(), 1);
    }

    #[test]
    fn init_carries_the_filter_and_rise_fall_properties() {
        let m = Bus::new(0x3d, &[]);
        let mut c = cfg();
        c.flags = 0;
        c.rise_ns = 100;
        c.fall_ns = 12;
        let mut d = YdMp25I2cData::default();
        assert_eq!(init_with(&m, &TestEnv::default(), &c, &mut d), 0);
        assert_eq!(m.writes.borrow().last(), Some(&(CR1, CR1_ANFOFF | CR1_PE)));
        let mut want = Config::new(64_000_000, 400_000);
        (want.rise_ns, want.fall_ns, want.analog_filter) = (100, 12, false);
        assert_eq!(d.timingr, compute_timing(&want).unwrap().timingr());
    }

    #[test]
    fn init_refuses_an_unreachable_speed() {
        let m = Bus::new(0x3d, &[]);
        let mut c = cfg();
        c.speed_hz = 3_000_000;
        let env = TestEnv::default();
        assert_eq!(init_with(&m, &env, &c, &mut YdMp25I2cData::default()), -EINVAL);
        assert!(m.writes.borrow().is_empty());
        assert_eq!(env.lines.borrow().len(), 1);
    }

    fn up() -> Bus {
        let m = Bus::new(0x3d, &[0xaa, 0xbb]);
        assert_eq!(init_with(&m, &TestEnv::default(), &cfg(), &mut YdMp25I2cData::default()), 0);
        m.writes.borrow_mut().clear();
        m
    }

    #[test]
    fn write_sequence() {
        let m = up();
        let mut b = [0x41, 0x10];
        assert_eq!(xfer(&m, 0x3d, &[msg(&mut b, 0)]), 0);
        assert_eq!(
            *m.writes.borrow(),
            vec![(CR2, cr2(0x3d, false, 2, true)), (TXDR, 0x41), (TXDR, 0x10), (ICR, ICR_STOPCF)]
        );
    }

    #[test]
    fn write_read_sequence() {
        let m = up();
        let (mut w, mut r) = ([0x94], [0u8; 2]);
        let msgs = [msg(&mut w, 0), msg(&mut r, YD_MP25_I2C_MSG_READ | YD_MP25_I2C_MSG_STOP)];
        assert_eq!(xfer(&m, 0x3d, &msgs), 0);
        assert_eq!(r, [0xaa, 0xbb]);
        assert_eq!(
            *m.writes.borrow(),
            vec![(CR2, cr2(0x3d, false, 1, false)), (TXDR, 0x94), (CR2, cr2(0x3d, true, 2, true)), (ICR, ICR_STOPCF)]
        );
    }

    #[test]
    fn nack_is_negative_errno() {
        let m = up();
        let mut b = [1, 2];
        assert_eq!(xfer(&m, 0x50, &[msg(&mut b, 0)]), -crate::errno::ENODEV);
    }

    #[test]
    fn bad_arguments() {
        let m = up();
        let mut big = [0u8; 256];
        assert_eq!(xfer(&m, 0x3d, &[msg(&mut big, 0)]), -EINVAL);
        assert_eq!(xfer(&m, 0x3d, &[msg(&mut [], 0)]), -EINVAL);
        let mut b = [1];
        assert_eq!(xfer(&m, 0x80, &[msg(&mut b, 0)]), -EINVAL);
        assert_eq!(xfer(&m, 0x3d, &[]), -EINVAL);
        let null = YdMp25I2cMsg { buf: core::ptr::null_mut(), len: 2, flags: 0 };
        assert_eq!(xfer(&m, 0x3d, &[null]), -EINVAL);
        assert!(m.writes.borrow().is_empty());
    }

    #[test]
    fn unsupported_shapes() {
        let m = up();
        let mut b = [0u8; 2];
        assert_eq!(xfer(&m, 0x3d, &[msg(&mut b, YD_MP25_I2C_MSG_READ)]), -ENOTSUP);
        let (mut x, mut y, mut z) = ([1u8], [2u8], [3u8]);
        let three = [msg(&mut x, 0), msg(&mut y, 0), msg(&mut z, 0)];
        assert_eq!(xfer(&m, 0x3d, &three), -ENOTSUP);
    }

    #[test]
    fn extern_entries_check_pointers_first() {
        let c = cfg();
        let mut d = YdMp25I2cData::default();
        let b = [1u8];
        let mut r = [0u8];
        // SAFETY: every call stops at its argument check before any access.
        unsafe {
            assert_eq!(yd_mp25_i2c_init(core::ptr::null(), &mut d, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_i2c_init(&c, &mut d, core::ptr::null()), -EINVAL);
            assert_eq!(yd_mp25_i2c_write(core::ptr::null(), 0x3d, b.as_ptr(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_write(&c, 0x3d, core::ptr::null(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_write(&c, 0x80, b.as_ptr(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_write_read(&c, 0x3d, b.as_ptr(), 1, core::ptr::null_mut(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_write_read(core::ptr::null(), 0x3d, b.as_ptr(), 1, r.as_mut_ptr(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_transfer(&c, core::ptr::null_mut(), 1, 0x3d), -EINVAL);
            assert_eq!(yd_mp25_i2c_transfer(&c, core::ptr::null_mut(), 0, 0x3d), -EINVAL);
        }
    }

    use crate::errno::ENODEV;
    use crate::model_mp25_i2c::{edid_block, Adv7535Config, Adv7535Handle, EdidMode, Mp25I2c};

    /// Run `f` with `bus` behind the extern entry points.
    fn with_bus<R>(bus: &Mp25I2c, f: impl FnOnce() -> R) -> R {
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                BUS.with(|b| b.set(None));
            }
        }
        let p: *const (dyn Mmio + '_) = bus;
        // SAFETY: only the lifetime is erased; `Clear` removes the pointer
        // before this function returns, so `bus` outlives every use.
        let p = unsafe { core::mem::transmute::<*const (dyn Mmio + '_), *const dyn Mmio>(p) };
        BUS.with(|b| b.set(Some(p)));
        let _clear = Clear;
        f()
    }

    extern "C" fn nop_wait(_: *mut core::ffi::c_void, _: u32) {}
    extern "C" fn nop_log(_: *mut core::ffi::c_void, _: u32, _: *const u8, _: usize) {}
    extern "C" fn nop_i2c_w(_: *mut core::ffi::c_void, _: u32, _: *const u8, _: usize) -> i32 {
        -1
    }
    extern "C" fn nop_i2c_wr(_: *mut core::ffi::c_void, _: u32, _: *const u8, _: usize, _: *mut u8, _: usize) -> i32 {
        -1
    }
    extern "C" fn nop_gpio(_: *mut core::ffi::c_void, _: u32, _: u32) -> i32 {
        -1
    }

    fn yd_env() -> YdEnv {
        YdEnv {
            ctx: core::ptr::null_mut(),
            busy_wait_us: nop_wait,
            sleep_ms: nop_wait,
            log: nop_log,
            i2c_write: nop_i2c_w,
            i2c_write_read: nop_i2c_wr,
            gpio_set: nop_gpio,
        }
    }

    fn extern_init(bus: &Mp25I2c, c: &YdMp25I2cConfig) -> (i32, YdMp25I2cData) {
        let (mut d, e) = (YdMp25I2cData::default(), yd_env());
        // SAFETY: all three point at live locals; the bus is installed.
        let rc = with_bus(bus, || unsafe { yd_mp25_i2c_init(c, &mut d, &e) });
        (rc, d)
    }

    fn up_dk() -> (Mp25I2c, Adv7535Handle) {
        let (bus, adv) = Mp25I2c::dk(Adv7535Config::default());
        assert_eq!(extern_init(&bus, &cfg()).0, 0);
        (bus, adv)
    }

    #[test]
    fn extern_init_programs_timingr_cr1_and_data() {
        let (bus, _adv) = Mp25I2c::dk(Adv7535Config::default());
        let mut c = cfg();
        c.flags = 0;
        c.dnf_ns = 100;
        let (rc, d) = extern_init(&bus, &c);
        assert_eq!(rc, 0);
        let mut want = Config::new(64_000_000, 400_000);
        want.analog_filter = false;
        want.dnf_ns = 100;
        let t = compute_timing(&want).unwrap();
        assert_eq!(bus.timingr(), t.timingr());
        assert_eq!(d, YdMp25I2cData { timingr: t.timingr(), dnf: u32::from(t.dnf) });
        assert_ne!(d.dnf, 0);
        let cr1 = bus.cr1();
        assert_ne!(cr1 & CR1_ANFOFF, 0);
        assert_ne!(cr1 & CR1_PE, 0);
        assert_eq!((cr1 & CR1_DNF) >> CR1_DNF.trailing_zeros(), d.dnf);
    }

    #[test]
    fn extern_init_analog_filter_flag_keeps_the_filter_on() {
        let (bus, _adv) = Mp25I2c::dk(Adv7535Config::default());
        assert_eq!(extern_init(&bus, &cfg()).0, 0);
        assert_eq!(bus.cr1() & CR1_ANFOFF, 0);
        assert_ne!(bus.cr1() & CR1_PE, 0);
    }

    #[test]
    fn extern_init_zero_rise_fall_take_linuxs_25_and_10() {
        let (b0, _a0) = Mp25I2c::dk(Adv7535Config::default());
        let (b1, _a1) = Mp25I2c::dk(Adv7535Config::default());
        let mut c = cfg();
        assert_eq!(extern_init(&b0, &c).0, 0);
        (c.rise_ns, c.fall_ns) = (25, 10);
        assert_eq!(extern_init(&b1, &c).0, 0);
        assert_eq!(b0.timingr(), b1.timingr());
        let mut want = Config::new(64_000_000, 400_000);
        (want.rise_ns, want.fall_ns) = (25, 10);
        assert_eq!(b0.timingr(), compute_timing(&want).unwrap().timingr());
        // A different rise time reaches the timing.
        let (b2, _a2) = Mp25I2c::dk(Adv7535Config::default());
        (c.rise_ns, c.fall_ns) = (300, 10);
        assert_eq!(extern_init(&b2, &c).0, 0);
        assert_ne!(b0.timingr(), b2.timingr());
    }

    #[test]
    fn extern_init_unreachable_speed_is_einval_and_leaves_data() {
        let (bus, _adv) = Mp25I2c::dk(Adv7535Config::default());
        let mut c = cfg();
        c.speed_hz = 3_000_000;
        let (rc, d) = extern_init(&bus, &c);
        assert_eq!(rc, -EINVAL);
        assert_eq!(d, YdMp25I2cData::default());
        assert_eq!(bus.timingr(), 0);
    }

    #[test]
    fn extern_write_reaches_the_adv7535() {
        let (bus, adv) = up_dk();
        let c = cfg();
        // Main map 0x41 (POWER) := 0x10.
        let b = [0x41u8, 0x10];
        // SAFETY: live locals, bus installed.
        let rc = with_bus(&bus, || unsafe { yd_mp25_i2c_write(&c, 0x3d, b.as_ptr(), b.len()) });
        assert_eq!(rc, 0);
        assert_eq!(adv.reg(0x41), 0x10);
        assert!(adv.main_writes().contains(&(0x41, 0x10)));
    }

    #[test]
    fn extern_write_read_reads_back_a_register() {
        let (bus, adv) = up_dk();
        let c = cfg();
        let w = [0x41u8, 0x22];
        let (p, mut r) = ([0x41u8], [0u8; 1]);
        // SAFETY: live locals, bus installed.
        let (rc1, rc2) = with_bus(&bus, || unsafe {
            (
                yd_mp25_i2c_write(&c, 0x3d, w.as_ptr(), w.len()),
                yd_mp25_i2c_write_read(&c, 0x3d, p.as_ptr(), 1, r.as_mut_ptr(), 1),
            )
        });
        assert_eq!((rc1, rc2), (0, 0));
        assert_eq!(r[0], 0x22);
        assert_eq!(r[0], adv.reg(0x41));
        // The chip revision, as ST's U-Boot reads it.
        let (z, mut rev) = ([0u8], [0u8]);
        // SAFETY: as above.
        let rc = with_bus(&bus, || unsafe { yd_mp25_i2c_write_read(&c, 0x3d, z.as_ptr(), 1, rev.as_mut_ptr(), 1) });
        assert_eq!((rc, rev[0]), (0, 0x14));
    }

    #[test]
    fn extern_write_read_returns_the_edid_at_0x3f() {
        let (bus, _adv) = up_dk();
        let c = cfg();
        let (z, mut edid) = ([0u8], [0u8; 128]);
        // SAFETY: live locals, bus installed.
        let rc = with_bus(&bus, || unsafe {
            yd_mp25_i2c_write_read(&c, 0x3f, z.as_ptr(), 1, edid.as_mut_ptr(), edid.len())
        });
        assert_eq!(rc, 0);
        assert_eq!(edid.to_vec(), edid_block(EdidMode::P720));
    }

    #[test]
    fn extern_transfer_takes_zephyrs_message_list() {
        let (bus, adv) = up_dk();
        let c = cfg();
        let mut w = [0x41u8, 0x5a];
        let mut one = [msg(&mut w, 0)];
        // SAFETY: live locals, bus installed.
        let rc = with_bus(&bus, || unsafe { yd_mp25_i2c_transfer(&c, one.as_mut_ptr(), 1, 0x3d) });
        assert_eq!(rc, 0);
        assert_eq!(adv.reg(0x41), 0x5a);
        let (mut p, mut r) = ([0u8], [0u8; 4]);
        let mut two = [msg(&mut p, 0), msg(&mut r, YD_MP25_I2C_MSG_READ | YD_MP25_I2C_MSG_STOP)];
        // SAFETY: as above.
        let rc = with_bus(&bus, || unsafe { yd_mp25_i2c_transfer(&c, two.as_mut_ptr(), 2, 0x3f) });
        assert_eq!(rc, 0);
        assert_eq!(r, edid_block(EdidMode::P720)[..4]);
        // A read alone is -ENOTSUP.
        let mut rd = [msg(&mut r, YD_MP25_I2C_MSG_READ)];
        // SAFETY: as above.
        let rc = with_bus(&bus, || unsafe { yd_mp25_i2c_transfer(&c, rd.as_mut_ptr(), 1, 0x3f) });
        assert_eq!(rc, -ENOTSUP);
    }

    #[test]
    fn extern_nack_on_an_absent_address_is_enodev_and_the_bus_recovers() {
        let (bus, adv) = up_dk();
        let c = cfg();
        let (b, mut r) = ([1u8, 2], [0u8; 2]);
        // SAFETY: live locals, bus installed.
        let (w, wr) = with_bus(&bus, || unsafe {
            (
                yd_mp25_i2c_write(&c, 0x50, b.as_ptr(), 2),
                yd_mp25_i2c_write_read(&c, 0x50, b.as_ptr(), 1, r.as_mut_ptr(), 2),
            )
        });
        assert_eq!((w, wr), (-ENODEV, -ENODEV));
        // The next transfer to the ADV7535 still works.
        let ok = [0x41u8, 0x33];
        // SAFETY: as above.
        let rc = with_bus(&bus, || unsafe { yd_mp25_i2c_write(&c, 0x3d, ok.as_ptr(), 2) });
        assert_eq!(rc, 0);
        assert_eq!(adv.reg(0x41), 0x33);
    }

    #[test]
    fn extern_length_and_address_errors_touch_no_register() {
        let (bus, _adv) = up_dk();
        bus.clear_log();
        let c = cfg();
        let mut big = [0u8; 256];
        // SAFETY: live locals, bus installed.
        with_bus(&bus, || unsafe {
            assert_eq!(yd_mp25_i2c_write(&c, 0x3d, big.as_ptr(), 256), -EINVAL);
            assert_eq!(yd_mp25_i2c_write(&c, 0x3d, big.as_ptr(), 0), -EINVAL);
            assert_eq!(yd_mp25_i2c_write(&c, 0x3d, core::ptr::null(), 0), -EINVAL);
            assert_eq!(yd_mp25_i2c_write(&c, 0x80, big.as_ptr(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_write_read(&c, 0x3d, big.as_ptr(), 1, big.as_mut_ptr(), 256), -EINVAL);
            assert_eq!(yd_mp25_i2c_write_read(&c, 0x3d, big.as_ptr(), 1, big.as_mut_ptr(), 0), -EINVAL);
            assert_eq!(yd_mp25_i2c_write_read(&c, 0x3d, big.as_ptr(), 256, big.as_mut_ptr(), 1), -EINVAL);
            assert_eq!(yd_mp25_i2c_transfer(&c, core::ptr::null_mut(), 0, 0x3d), -EINVAL);
        });
        assert!(bus.accesses().is_empty());
    }
}
