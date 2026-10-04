// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The one triangle sample's staticlib (driver/samples/zephyr/triangle,
//! driver/docs/one-method.md section 5, step 3): the C entry points main.c
//! calls, over the check crate (`triangle_check`, ../check), which is the one
//! definition of the scene and of the check.
//!
//! 1. [`triangle_scene`]: what to draw (vertices, clear colour, shader texts)
//!    and the window's size, guard rows and fill words.
//! 2. [`triangle_check`]: the check of the drawn window, printed line by line,
//!    and the PICTURE record's words.
#![no_std]

use core::ffi::{c_char, c_void};
use core::fmt::Write;

use triangle_check::{check, record, Frame, COLOR_TOL, EDGE_PX, GUARD_ROWS, PICTURE_SAMPLES};

extern "C" {
    fn printk(fmt: *const c_char, ...);
}

/// A `core::fmt` sink that prints through printk, a line at a time (the
/// buffer flushes on a newline or when full).
struct Pk {
    buf: [u8; 192],
    n: usize,
}

impl Pk {
    const fn new() -> Self {
        Pk { buf: [0; 192], n: 0 }
    }

    fn flush(&mut self) {
        if self.n > 0 {
            self.buf[self.n] = 0;
            // SAFETY: a NUL-terminated buffer through "%s".
            unsafe { printk(c"%s\n".as_ptr(), self.buf.as_ptr() as *const c_char) };
            self.n = 0;
        }
    }
}

impl Write for Pk {
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

/// One console line.
macro_rules! line {
    ($($arg:tt)*) => {{
        let mut pk = Pk::new();
        let _ = write!(pk, $($arg)*);
        pk.flush();
    }};
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // panic = "abort": print and spin; the board's watchdog resets the SoC at
    // the lifetime (yakogl_board.h).
    let mut w = Pk::new();
    let _ = w.write_str("triangle: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(w, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(w, "{}", info.message());
    w.flush();
    loop {
        core::hint::spin_loop();
    }
}

/// What main.c draws and into what (main.c `struct triangle_scene`).
#[repr(C)]
pub struct Scene {
    /// x, y, z, w, r, g, b, a per vertex, three vertices.
    pub verts: [f32; 24],
    pub clear: [f32; 4],
    /// NUL-terminated GLSL ES 1.00.
    pub vs: *const c_char,
    pub fs: *const c_char,
    pub w: u32,
    pub h: u32,
    pub guard_rows: u32,
    pub sentinel: u32,
    pub guard: u32,
}

const _: () = assert!(core::mem::size_of::<Scene>() == 152);

/// Fill `out` with the scene.
///
/// # Safety
/// `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn triangle_scene(out: *mut Scene) {
    let mut verts = [0f32; 24];
    for (i, v) in triangle_check::VERTS.iter().enumerate() {
        verts[8 * i..8 * i + 8].copy_from_slice(v);
    }
    let scene = Scene {
        verts,
        clear: triangle_check::CLEAR,
        vs: triangle_check::VS_SRC.as_ptr() as *const c_char,
        fs: triangle_check::FS_SRC.as_ptr() as *const c_char,
        w: triangle_check::WIDTH,
        h: triangle_check::HEIGHT,
        guard_rows: GUARD_ROWS,
        sentinel: triangle_check::SENTINEL,
        guard: triangle_check::GUARD,
    };
    // SAFETY: the caller's contract.
    unsafe { out.write(scene) };
}

/// The check's result (main.c `struct triangle_result`).
#[repr(C)]
pub struct CheckResult {
    /// record STATUS_PASS (1) or STATUS_FAIL (2).
    pub status: u32,
    /// The first failure (`triangle_check::Failure` as u8, 1..=6), 0 for a
    /// pass: the APP record's stage byte for a failed check.
    pub failure: u32,
    /// PICTURE's w1..w3.
    pub picture: [u32; 3],
}

const _: () = assert!(core::mem::size_of::<CheckResult>() == 20);

const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
/// glReadPixels' C signature.
type ReadPixels = extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

/// R, G, B, A bytes as one word, R in the low byte.
fn word(p: [u8; 4]) -> u32 {
    u32::from_le_bytes(p)
}

/// Check the window as it stands after the swap. `cpu` is the window's
/// memory as board_window gave it: h + guard_rows rows of w pixels, rows
/// packed, the guard rows after the window. `format` is the native window's
/// format word. `read_pixels` is glReadPixels, the context that drew the
/// window current.
///
/// Returns 0 for a pass, 1 for a failed check (the printed lines say what
/// failed), -1 when the frame could not be checked (the reason is printed;
/// `out` then holds a FAIL status, failure 0 and zero words).
///
/// # Safety
/// `cpu` is readable for (h + guard_rows) * w words and nothing writes it
/// during the call; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn triangle_check(
    cpu: *const u32,
    w: u32,
    h: u32,
    guard_rows: u32,
    format: u32,
    read_pixels: Option<ReadPixels>,
    out: *mut CheckResult,
) -> i32 {
    // SAFETY: the caller's contract.
    let out = unsafe { &mut *out };
    *out = CheckResult { status: record::STATUS_FAIL, failure: 0, picture: [0; 3] };
    let Some(read_pixels) = read_pixels else {
        line!("triangle: check: not checked: no glReadPixels");
        return -1;
    };
    if cpu.is_null() {
        line!("triangle: check: not checked: no window memory");
        return -1;
    }
    let n = (h as usize + guard_rows as usize) * w as usize;
    // SAFETY: the caller's contract.
    let pixels = unsafe { core::slice::from_raw_parts(cpu, n) };
    let frame = match Frame::new(pixels, w, h, guard_rows, format) {
        Ok(f) => f,
        Err(e) => {
            line!("triangle: check: not checked: {} ({w}x{h}, format {format:#x})", e.what());
            return -1;
        }
    };
    let o = check(&frame, |x, y| {
        let mut px = [0u8; 4];
        read_pixels(x as i32, y as i32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        px
    });
    line!(
        "triangle: window {w}x{h}: {} pixels = {} inside + {} outside + {} edge (band {EDGE_PX} px)",
        o.total,
        o.n_in,
        o.n_out,
        o.n_edge
    );
    line!("triangle:   inside wrong {} (largest channel diff {}, tolerance {COLOR_TOL})", o.bad_in, o.worst_in);
    line!("triangle:   outside wrong {}, edge wrong {} (edge drawn {})", o.bad_out, o.bad_edge, o.edge_drawn);
    if let Some(f) = o.first_wrong {
        line!(
            "triangle:   first wrong at GL {},{}: 0x{:08x}, expected 0x{:08x} (R in the low byte)",
            f.x,
            f.y,
            word(f.got),
            word(f.want)
        );
    }
    line!("triangle:   guard rows: {} of {} pixels changed", o.guard_bad, w * guard_rows);
    line!("triangle:   glReadPixels against memory: {} of {} points differ", o.reads_bad, o.reads);
    if let Some((x, y, gl)) = o.first_read_bad {
        line!(
            "triangle:   first at GL {x},{y}: glReadPixels 0x{:08x}, memory 0x{:08x}",
            word(gl),
            word(frame.at_gl(x, y))
        );
    }
    let [p1, p2, p3] = o.picture;
    line!("triangle: picture: centre 0x{p1:08x}, column crc 0x{p2:08x} ({PICTURE_SAMPLES} samples), size 0x{p3:08x}");
    out.picture = o.picture;
    match o.verdict() {
        None => {
            out.status = record::STATUS_PASS;
            line!("triangle: check PASS (every pixel read back)");
            0
        }
        Some(f) => {
            out.failure = f as u8 as u32;
            line!("triangle: check FAIL: {} (code {})", f.what(), f as u8);
            1
        }
    }
}
