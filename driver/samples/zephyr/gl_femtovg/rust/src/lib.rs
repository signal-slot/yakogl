// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Zephyr runtime shim around the no_std FemtoVG demo crate (fvg_demo).
//!
//! fvg_demo is compiled with its `external_rt` feature, which drops its stub
//! allocator and panic handler so the integrator can supply real ones. This
//! staticlib provides both for the Zephyr link and re-exports `fvg_draw_frame`
//! so that symbol (and, transitively, femtovg/glow) ends up in libfvg_zephyr.a.
#![no_std]

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::{c_char, c_void};

extern "C" {
    /// Zephyr libc heap (CONFIG_COMMON_LIBC_MALLOC): C11 aligned_alloc/free.
    fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn printk(fmt: *const c_char, ...);
}

/// Global allocator backed by Zephyr's libc heap.
struct ZephyrAlloc;

unsafe impl GlobalAlloc for ZephyrAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // C11 aligned_alloc requires the alignment be supported and the size be
        // an integral multiple of the alignment; round the request up. Rust's
        // Layout already guarantees a power-of-two alignment.
        let align = layout.align().max(core::mem::size_of::<usize>());
        let size = (layout.size() + align - 1) & !(align - 1);
        aligned_alloc(align, size) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        free(ptr as *mut c_void);
    }
}

#[global_allocator]
static GLOBAL: ZephyrAlloc = ZephyrAlloc;

/// A core::fmt sink that prints through Zephyr printk. Buffers into a fixed
/// stack array and flushes a null-terminated "%s" on newline or when full, so
/// the panic message + location reach the console without std.
struct Pk {
    buf: [u8; 192],
    n: usize,
}

impl Pk {
    fn flush(&mut self) {
        if self.n > 0 {
            self.buf[self.n] = 0;
            unsafe {
                printk(b"%s\0".as_ptr() as *const c_char,
                       self.buf.as_ptr() as *const c_char);
            }
            self.n = 0;
        }
    }
}

impl core::fmt::Write for Pk {
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

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // No unwinding (panic = "abort"): log the location + message and spin. The
    // board's watchdog resets the SoC if we ever get here.
    use core::fmt::Write;
    let mut w = Pk { buf: [0; 192], n: 0 };
    let _ = w.write_str("fvg_zephyr: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(w, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(w, "{}", info.message());
    let _ = w.write_str("\n");
    w.flush();
    loop {
        core::hint::spin_loop();
    }
}

// Keep fvg_draw_frame reachable so the linker pulls it (and femtovg/glow) in.
pub use fvg_demo::fvg_draw_frame;
