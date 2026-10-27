// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The one triangle sample's staticlib (examples/triangle,
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

use triangle_check::console::Console;
use triangle_check::{check, record, report, say, Frame, GUARD_ROWS};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // panic = "abort": print and spin; the board's watchdog resets the SoC at
    // the lifetime (yakogl_board.h).
    let mut c = Console::new();
    let _ = c.write_str("triangle: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(c, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(c, "{}", info.message());
    c.flush();
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
        say!("triangle: check: not checked: no glReadPixels");
        return -1;
    };
    if cpu.is_null() {
        say!("triangle: check: not checked: no window memory");
        return -1;
    }
    let n = (h as usize + guard_rows as usize) * w as usize;
    // SAFETY: the caller's contract.
    let pixels = unsafe { core::slice::from_raw_parts(cpu, n) };
    let frame = match Frame::new(pixels, w, h, guard_rows, format) {
        Ok(f) => f,
        Err(e) => {
            say!("triangle: check: not checked: {} ({w}x{h}, format {format:#x})", e.what());
            return -1;
        }
    };
    let o = check(&frame, |x, y| {
        let mut px = [0u8; 4];
        read_pixels(x as i32, y as i32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        px
    });
    let mut c = Console::new();
    let _ = report(&mut c, "triangle", &frame, &o);
    out.picture = o.picture;
    match o.verdict() {
        None => {
            out.status = record::STATUS_PASS;
            0
        }
        Some(f) => {
            out.failure = f as u8 as u32;
            1
        }
    }
}
