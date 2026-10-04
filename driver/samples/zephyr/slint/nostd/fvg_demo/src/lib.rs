// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! no_std (alloc-only) FemtoVG demo staticlib for aarch64-unknown-none.
//!
//! Draws one real FemtoVG frame through an app-supplied OpenGL ES 2.0 context.
//! The C side creates/makes-current the GL context and passes a get-proc-address
//! function pointer; we build femtovg's `OpenGl` renderer over it, draw, and flush.
//! No fonts/text (deferred). No OS calls other than the caller's GL entry points.
#![no_std]

extern crate alloc;

use core::ffi::{c_char, c_void, CStr};

use femtovg::renderer::OpenGl;
use femtovg::{Canvas, Color, Paint, Path};

#[cfg(not(feature = "external_rt"))]
mod standalone_rt {
    use core::alloc::{GlobalAlloc, Layout};
    use core::ptr::null_mut;

    struct StubAlloc;
    unsafe impl GlobalAlloc for StubAlloc {
        unsafe fn alloc(&self, _l: Layout) -> *mut u8 {
            null_mut()
        }
        unsafe fn dealloc(&self, _p: *mut u8, _l: Layout) {}
    }
    #[global_allocator]
    static A: StubAlloc = StubAlloc;

    #[panic_handler]
    fn panic(_info: &core::panic::PanicInfo) -> ! {
        loop {
            core::hint::spin_loop();
        }
    }
}

/// Draw one FemtoVG frame through the caller's current GL ES 2.0 context.
/// Returns 0 on success, non-zero on failure.
#[no_mangle]
pub extern "C" fn fvg_draw_frame(
    get_proc: extern "C" fn(*const c_char) -> *const c_void,
    width: u32,
    height: u32,
) -> i32 {
    let renderer = match unsafe {
        OpenGl::new_from_function_cstr(|name: &CStr| get_proc(name.as_ptr()))
    } {
        Ok(r) => r,
        Err(_) => return 1,
    };

    let mut canvas: Canvas<OpenGl> = match Canvas::new(renderer) {
        Ok(c) => c,
        Err(_) => return 2,
    };

    canvas.set_size(width, height, 1.0);

    let w = width as f32;
    let h = height as f32;

    // Dark background clear.
    canvas.clear_rect(0, 0, width, height, Color::rgb(24, 24, 32));

    const DO_STROKE: bool = true;

    // (a) Filled rounded rect, solid colour.
    {
        let mut path = Path::new();
        path.rounded_rect(w * 0.08, h * 0.10, w * 0.36, h * 0.30, 16.0);
        let paint = Paint::color(Color::rgb(80, 160, 240));
        canvas.fill_path(&path, &paint);
    }

    // (b) Linear-gradient filled rect.
    {
        let x = w * 0.54;
        let y = h * 0.10;
        let rw = w * 0.38;
        let rh = h * 0.30;
        let mut path = Path::new();
        path.rect(x, y, rw, rh);
        let paint = Paint::linear_gradient(
            x,
            y,
            x,
            y + rh,
            Color::rgb(240, 80, 120),
            Color::rgb(250, 210, 90),
        );
        canvas.fill_path(&path, &paint);
    }

    // (c) Stroked path.
    if DO_STROKE {
        let mut path = Path::new();
        path.move_to(w * 0.10, h * 0.75);
        path.line_to(w * 0.30, h * 0.55);
        path.line_to(w * 0.50, h * 0.80);
        path.line_to(w * 0.70, h * 0.55);
        path.line_to(w * 0.90, h * 0.75);
        let mut paint = Paint::color(Color::rgb(120, 240, 160));
        paint.set_line_width(6.0);
        canvas.stroke_path(&path, &paint);
    }

    // Submit the frame to the GPU.
    canvas.flush();

    0
}
