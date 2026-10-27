// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The femtovg example's staticlib (examples/femtovg): the C entry points
//! main.c calls, over the check crate (`femtovg_check`, ../check), which is
//! the one definition of the scene and of the check.
//!
//! 1. [`femtovg_scene`]: the window's size, guard rows and fill words.
//! 2. [`femtovg_draw`]: the scene drawn with femtovg over glow, through the
//!    GL context the caller made current.
//! 3. [`femtovg_check`]: the check of the drawn window, printed line by
//!    line, and the PICTURE record's words.
//!
//! femtovg allocates through the global allocator here, Zephyr's C heap
//! (CONFIG_COMMON_LIBC_MALLOC).
#![no_std]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::{c_char, c_void, CStr};
use core::fmt::Write;

use femtovg::renderer::OpenGl;
use femtovg::{Canvas, Color, Paint, Path};
use femtovg_check::console::Console;
use femtovg_check::{box_px, check, point_px, record, report, say, Frame, GUARD_ROWS};

extern "C" {
    /// Zephyr's C heap: C11 aligned_alloc and free.
    fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

struct ZephyrAlloc;

// SAFETY: aligned_alloc returns memory of at least the size asked for at the
// alignment asked for, or null; free takes what it returned.
unsafe impl GlobalAlloc for ZephyrAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // C11 aligned_alloc wants a size that is a multiple of the alignment
        // (a power of two, Layout's guarantee): round the size up.
        let align = layout.align().max(core::mem::size_of::<usize>());
        let size = (layout.size() + align - 1) & !(align - 1);
        // SAFETY: as above.
        unsafe { aligned_alloc(align, size) as *mut u8 }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: `ptr` came from alloc.
        unsafe { free(ptr as *mut c_void) }
    }
}

#[global_allocator]
static GLOBAL: ZephyrAlloc = ZephyrAlloc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // panic = "abort": print and spin; the board's watchdog resets the SoC at
    // the lifetime (yakogl_board.h).
    let mut c = Console::new();
    let _ = c.write_str("femtovg: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(c, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(c, "{}", info.message());
    c.flush();
    loop {
        core::hint::spin_loop();
    }
}

/// The window main.c makes (main.c `struct femtovg_scene`).
#[repr(C)]
pub struct Scene {
    pub w: u32,
    pub h: u32,
    pub guard_rows: u32,
    pub sentinel: u32,
    pub guard: u32,
}

const _: () = assert!(core::mem::size_of::<Scene>() == 20);

/// Fill `out` with the scene.
///
/// # Safety
/// `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn femtovg_scene(out: *mut Scene) {
    let scene = Scene {
        w: femtovg_check::WIDTH,
        h: femtovg_check::HEIGHT,
        guard_rows: GUARD_ROWS,
        sentinel: femtovg_check::SENTINEL,
        guard: femtovg_check::GUARD,
    };
    // SAFETY: the caller's contract.
    unsafe { out.write(scene) };
}

fn rgb(c: [u8; 3]) -> Color {
    Color::rgb(c[0], c[1], c[2])
}

/// eglGetProcAddress' C signature.
type GetProc = extern "C" fn(*const c_char) -> *const c_void;

/// Draw the scene into the current GL ES 2.0 context's `w` x `h` drawable:
/// the clear, the rounded rect, the gradient rect and the polyline, then
/// femtovg's flush (the GL calls are made; the caller swaps).
///
/// Returns 0, 1 when femtovg's renderer could not be created (its programs
/// did not all build; main.c's survey names the first that fails), or 2
/// when the canvas could not.
#[no_mangle]
pub extern "C" fn femtovg_draw(get_proc: Option<GetProc>, w: u32, h: u32) -> i32 {
    let Some(get_proc) = get_proc else {
        return 1;
    };
    // SAFETY: get_proc is eglGetProcAddress with a context current: every
    // name it answers is that GL entry point.
    let renderer = match unsafe { OpenGl::new_from_function_cstr(|name: &CStr| get_proc(name.as_ptr())) } {
        Ok(r) => r,
        Err(_) => return 1,
    };
    let mut canvas: Canvas<OpenGl> = match Canvas::new(renderer) {
        Ok(c) => c,
        Err(_) => return 2,
    };
    canvas.set_size(w, h, 1.0);
    canvas.clear_rect(0, 0, w, h, rgb(femtovg_check::BG));

    let [x, y, rw, rh] = box_px(femtovg_check::RRECT_BOX, w, h);
    let mut path = Path::new();
    path.rounded_rect(x, y, rw, rh, femtovg_check::RRECT_RADIUS);
    canvas.fill_path(&path, &Paint::color(rgb(femtovg_check::RRECT)));

    let [x, y, rw, rh] = box_px(femtovg_check::GRAD_BOX, w, h);
    let mut path = Path::new();
    path.rect(x, y, rw, rh);
    let paint =
        Paint::linear_gradient(x, y, x, y + rh, rgb(femtovg_check::GRAD_TOP), rgb(femtovg_check::GRAD_BOTTOM));
    canvas.fill_path(&path, &paint);

    let mut path = Path::new();
    for (i, &p) in femtovg_check::STROKE_PTS.iter().enumerate() {
        let [px, py] = point_px(p, w, h);
        if i == 0 {
            path.move_to(px, py);
        } else {
            path.line_to(px, py);
        }
    }
    let mut paint = Paint::color(rgb(femtovg_check::STROKE));
    paint.set_line_width(femtovg_check::STROKE_WIDTH);
    canvas.stroke_path(&path, &paint);

    canvas.flush();
    0
}

/// The check's result (main.c `struct femtovg_result`).
#[repr(C)]
pub struct CheckResult {
    /// record STATUS_PASS (1) or STATUS_FAIL (2).
    pub status: u32,
    /// The first failure (`femtovg_check::Failure` as u8, 1..=9), 0 for a
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
pub unsafe extern "C" fn femtovg_check(
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
        say!("femtovg: check: not checked: no glReadPixels");
        return -1;
    };
    if cpu.is_null() {
        say!("femtovg: check: not checked: no window memory");
        return -1;
    }
    let n = (h as usize + guard_rows as usize) * w as usize;
    // SAFETY: the caller's contract.
    let pixels = unsafe { core::slice::from_raw_parts(cpu, n) };
    let frame = match Frame::new(pixels, w, h, guard_rows, format) {
        Ok(f) => f,
        Err(e) => {
            say!("femtovg: check: not checked: {} ({w}x{h}, format {format:#x})", e.what());
            return -1;
        }
    };
    let o = check(&frame, |x, y| {
        let mut px = [0u8; 4];
        read_pixels(x as i32, y as i32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        px
    });
    let mut c = Console::new();
    let _ = report(&mut c, &frame, &o);
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
