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
use crate::mmio::{Hw, Mmio};
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
    let (cfg, data, env, hw) = unsafe { (&*cfg, &mut *data, &*env, Hw::new()) };
    init_with(&hw, env, cfg, data)
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
    unsafe { transfer_with(&Hw::new(), (*cfg).base, addr, &[m]) }
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
    unsafe { transfer_with(&Hw::new(), (*cfg).base, addr, &msgs) }
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
    unsafe { transfer_with(&Hw::new(), (*cfg).base, addr, list) }
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
}
