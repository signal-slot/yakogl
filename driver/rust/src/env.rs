// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The services a driver borrows from the operating system, as one table of C
//! function pointers (`struct yd_env`, include/yakogl_drivers.h).
//!
//! The Zephyr shim fills it with `k_busy_wait`, `k_msleep`, `LOG_*("%s")`,
//! `i2c_write_dt` / `i2c_write_read_dt` and `gpio_pin_set_dt`; the host tests
//! supply their own ([`crate::model`]). A driver never calls the OS any other
//! way, which is what lets the same code run against the register model.
//!
//! `bus` and `line` are indices the shim defines for its own device (its
//! first I2C bus, its second GPIO, ...): the Rust side never sees a Zephyr
//! `struct device`.

use core::ffi::c_void;
use core::fmt;

/// Log levels, Zephyr's numbering (`LOG_LEVEL_ERR` .. `LOG_LEVEL_DBG`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Level {
    Err = 1,
    Wrn = 2,
    Inf = 3,
    Dbg = 4,
}

/// `struct yd_env`. Every pointer is required (the shim gives a stub that
/// returns `-ENOTSUP` for a service its device does not have).
#[repr(C)]
pub struct YdEnv {
    pub ctx: *mut c_void,
    pub busy_wait_us: unsafe extern "C" fn(ctx: *mut c_void, us: u32),
    pub sleep_ms: unsafe extern "C" fn(ctx: *mut c_void, ms: u32),
    pub log: unsafe extern "C" fn(ctx: *mut c_void, level: u32, buf: *const u8, len: usize),
    pub i2c_write: unsafe extern "C" fn(ctx: *mut c_void, bus: u32, buf: *const u8, len: usize) -> i32,
    pub i2c_write_read: unsafe extern "C" fn(
        ctx: *mut c_void,
        bus: u32,
        wbuf: *const u8,
        wlen: usize,
        rbuf: *mut u8,
        rlen: usize,
    ) -> i32,
    pub gpio_set: unsafe extern "C" fn(ctx: *mut c_void, line: u32, value: u32) -> i32,
}

/// What the drivers call. Implemented for the C table and by the test model.
pub trait Env {
    fn busy_wait_us(&self, us: u32);
    fn sleep_ms(&self, ms: u32);
    /// One log line, already formatted (no trailing newline).
    fn log_str(&self, level: Level, line: &str);
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32>;
    fn i2c_write_read(&self, bus: u32, wbuf: &[u8], rbuf: &mut [u8]) -> Result<(), i32>;
    fn gpio_set(&self, line: u32, value: bool) -> Result<(), i32>;

    /// Format into a stack buffer and log it; a line longer than the buffer
    /// is cut and ends in "...".
    fn log(&self, level: Level, args: fmt::Arguments<'_>) {
        let mut b = LineBuf { buf: [0; LINE_MAX], len: 0, cut: false };
        let _ = fmt::write(&mut b, args);
        if b.cut {
            // Back to a character boundary that leaves room for the marker
            // (a UTF-8 continuation byte is 0b10xx_xxxx).
            let mut n = b.len.min(LINE_MAX - 3);
            while n > 0 && b.buf[n] & 0xc0 == 0x80 {
                n -= 1;
            }
            b.buf[n..n + 3].copy_from_slice(b"...");
            b.len = n + 3;
        }
        // Only whole UTF-8 sequences were copied, so this cannot fail; an
        // empty line is the fallback.
        self.log_str(level, core::str::from_utf8(&b.buf[..b.len]).unwrap_or(""));
    }
}

/// The longest line [`Env::log`] formats.
pub const LINE_MAX: usize = 160;

struct LineBuf {
    buf: [u8; LINE_MAX],
    len: usize,
    cut: bool,
}

impl fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = LINE_MAX - self.len;
        let mut n = s.len().min(room);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        if n < s.len() {
            self.cut = true;
        }
        Ok(())
    }
}

/// Log through any [`Env`]: `ylog!(env, Inf, "x {}", 1)`.
#[macro_export]
macro_rules! ylog {
    ($env:expr, $lvl:ident, $($arg:tt)*) => {
        $crate::env::Env::log($env, $crate::env::Level::$lvl, format_args!($($arg)*))
    };
}

fn errno_result(rc: i32) -> Result<(), i32> {
    if rc < 0 {
        Err(rc)
    } else {
        Ok(())
    }
}

impl Env for YdEnv {
    fn busy_wait_us(&self, us: u32) {
        // SAFETY: the shim's table; the function takes the shim's ctx.
        unsafe { (self.busy_wait_us)(self.ctx, us) }
    }
    fn sleep_ms(&self, ms: u32) {
        // SAFETY: as above.
        unsafe { (self.sleep_ms)(self.ctx, ms) }
    }
    fn log_str(&self, level: Level, line: &str) {
        // SAFETY: as above; the C side reads `len` bytes and keeps no pointer.
        unsafe { (self.log)(self.ctx, level as u32, line.as_ptr(), line.len()) }
    }
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
        // SAFETY: as above; `buf` is valid for `len` bytes for the call.
        errno_result(unsafe { (self.i2c_write)(self.ctx, bus, buf.as_ptr(), buf.len()) })
    }
    fn i2c_write_read(&self, bus: u32, wbuf: &[u8], rbuf: &mut [u8]) -> Result<(), i32> {
        // SAFETY: as above; `rbuf` is writable for `rlen` bytes for the call.
        errno_result(unsafe {
            (self.i2c_write_read)(self.ctx, bus, wbuf.as_ptr(), wbuf.len(), rbuf.as_mut_ptr(), rbuf.len())
        })
    }
    fn gpio_set(&self, line: u32, value: bool) -> Result<(), i32> {
        // SAFETY: as above.
        errno_result(unsafe { (self.gpio_set)(self.ctx, line, value as u32) })
    }
}

// The layout include/yakogl_drivers.h pins with _Static_assert (LP64).
const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(size_of::<YdEnv>() == 56);
    assert!(offset_of!(YdEnv, ctx) == 0);
    assert!(offset_of!(YdEnv, busy_wait_us) == 8);
    assert!(offset_of!(YdEnv, sleep_ms) == 16);
    assert!(offset_of!(YdEnv, log) == 24);
    assert!(offset_of!(YdEnv, i2c_write) == 32);
    assert!(offset_of!(YdEnv, i2c_write_read) == 40);
    assert!(offset_of!(YdEnv, gpio_set) == 48);
};
