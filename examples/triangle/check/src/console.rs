// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Zephyr console for the examples' Rust code (feature `printk`): a
//! `core::fmt` sink over printk. Only the image builds
//! (aarch64-unknown-none) turn the feature on; the host tests do not. Each
//! example keeps its own panic handler, so a panic line names the image.

use core::ffi::c_char;
use core::fmt::Write;

extern "C" {
    fn printk(fmt: *const c_char, ...);
}

/// A `core::fmt` sink that prints through printk, a line at a time (the
/// buffer flushes on a newline or when full).
pub struct Console {
    buf: [u8; 192],
    n: usize,
}

impl Console {
    pub const fn new() -> Self {
        Console { buf: [0; 192], n: 0 }
    }

    /// Print what is buffered as one line.
    pub fn flush(&mut self) {
        if self.n > 0 {
            self.buf[self.n] = 0;
            // SAFETY: a NUL-terminated buffer through "%s".
            unsafe { printk(c"%s\n".as_ptr(), self.buf.as_ptr() as *const c_char) };
            self.n = 0;
        }
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if b == b'\n' || self.n >= self.buf.len() - 1 {
                self.flush();
                if b == b'\n' {
                    continue;
                }
            }
            self.buf[self.n] = b;
            self.n += 1;
        }
        Ok(())
    }
}

/// One console line, `format!` style.
#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => {{
        let mut c = $crate::console::Console::new();
        let _ = core::fmt::Write::write_fmt(&mut c, format_args!($($arg)*));
        c.flush();
    }};
}
