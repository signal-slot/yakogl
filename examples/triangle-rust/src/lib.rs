// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! triangle-rust: examples/triangle with every call made from Rust. The same
//! Gouraud triangle (red top, green bottom left, blue bottom right) through
//! the public OpenGL ES 2.0 API on YakoGL, into a 1024x600 window, on the
//! Raspberry Pi 4 (V3D), the Verdin AM62P (PowerVR) and the Verdin iMX8M
//! Plus (GC7000). The board is chosen by the Zephyr board only.
//!
//! What is drawn and what is expected are the check crate's
//! (../triangle/check), the one definition both examples use, so the two
//! leave the same PICTURE record:
//!
//! ```text
//! board_window(1024, 600 + guard rows)  sentinel window, guard rows
//! EGL up (RGBA8888, ES 2)               the window's 1024x600 only
//! compile + link, one draw, eglSwapBuffers, board_show()
//! check()                               every pixel classified (inside /
//!                                       outside / edge), guard rows,
//!                                       glReadPixels against memory, and
//!                                       PICTURE's words
//! board_result(PICTURE)
//! board_result(APP, fail)               only after a failed check
//! board_finish()
//! ```
//!
//! The records are examples/triangle's (its src/main.c says what each word
//! holds): PICTURE for a checked frame, then an APP fail record after a
//! failed check; a run that never reached the check stores no PICTURE, and
//! board_finish() stores its APP fail record.
#![no_std]

mod board;
mod gl;

use core::ffi::{c_char, c_int, c_void, CStr};
use core::fmt::Write;
use core::ptr::{null, null_mut};

use gl::*;
use triangle_check::console::Console;
use triangle_check::{check, record, report, say, Frame};

const NAME: &str = "triangle-rust";

/// Zephyr's errno values (lib/libc/minimal and picolibc agree).
const ENOENT: c_int = 2;
const EIO: c_int = 5;
const EINVAL: c_int = 22;
const EBADMSG: c_int = 77;

/// Where a run that died stopped (kind PROGRESS, WHERE_STAGE | n): the same
/// stages as examples/triangle.
#[derive(Clone, Copy)]
enum Stage {
    Window = 1,
    Egl,
    Program,
    Draw,
    Swapped,
    Checked,
}

fn progress(s: Stage) {
    // SAFETY: board_progress takes any value; main only.
    unsafe { board::board_progress(record::WHERE_STAGE | s as u32) };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // panic = "abort": print and spin; the board's watchdog resets the SoC at
    // the lifetime (yakogl_board.h).
    let mut c = Console::new();
    let _ = write!(c, "{NAME}: rust panic: ");
    if let Some(loc) = info.location() {
        let _ = write!(c, "{}:{}:{}: ", loc.file(), loc.line(), loc.column());
    }
    let _ = write!(c, "{}", info.message());
    c.flush();
    loop {
        core::hint::spin_loop();
    }
}

/// A NUL-terminated C string as text for the console.
fn text(p: *const c_char) -> &'static str {
    if p.is_null() {
        return "(null)";
    }
    // SAFETY: GL and EGL return NUL-terminated strings that live as long as
    // the context.
    unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("(not UTF-8)")
}

/// Each EGL call is checked on its own: eglGetError reports the LAST call.
fn egl_fail(call: &str) -> c_int {
    // SAFETY: no arguments.
    let err = unsafe { eglGetError() };
    say!("{NAME}: {call} failed: EGL error {err:#x}");
    -EIO
}

/// The window: the scene's size plus the guard rows, the sentinel in the
/// window and the guard word in the guard rows. Returns it with `h` the
/// window's own height (EGL and the screen get the window, not the guard
/// rows).
fn window() -> Result<board::Window, c_int> {
    let (w, h, guard_rows) = (triangle_check::WIDTH, triangle_check::HEIGHT, triangle_check::GUARD_ROWS);
    let mut win = board::Window { phys: 0, cpu: null_mut(), w: 0, h: 0, stride: 0, format: 0 };
    // SAFETY: `win` is writable.
    let ret = unsafe { board::board_window(w, h + guard_rows, &mut win) };
    if ret != 0 {
        say!("{NAME}: board_window({w}, {}) = {ret}", h + guard_rows);
        return Err(ret);
    }
    if win.stride != w * 4 {
        say!("{NAME}: board_window gave stride {}, not {}", win.stride, w * 4);
        return Err(-EINVAL);
    }
    let (win_px, all_px) = ((w * h) as usize, (w * (h + guard_rows)) as usize);
    // SAFETY: board_window gave `all_px` words at `cpu`, ours alone.
    let px = unsafe { core::slice::from_raw_parts_mut(win.cpu as *mut u32, all_px) };
    px[..win_px].fill(triangle_check::SENTINEL);
    px[win_px..].fill(triangle_check::GUARD);
    win.h = h;
    say!(
        "{NAME}: window {}x{} at phys {:#x}, format {:#x}, {guard_rows} guard rows",
        win.w,
        win.h,
        win.phys,
        win.format
    );
    Ok(win)
}

/// EGL on the window, an ES 2 context current.
fn egl_up(win: &board::Window) -> Result<(EGLDisplay, EGLSurface), c_int> {
    // EGL_SINGLE_BUFFER: this example shows the window itself (board_show
    // after the swap) and reads it back as memory, so the window is rendered
    // in place and complete at every swap; the default, EGL_BACK_BUFFER,
    // would have EGL show its own buffers (gles_osal.h, present).
    let single_buffer = [EGL_RENDER_BUFFER, EGL_SINGLE_BUFFER, EGL_NONE];
    let cfg_attribs = [
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_ALPHA_SIZE,
        8,
        EGL_SURFACE_TYPE,
        EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_ES2_BIT,
        EGL_NONE,
    ];
    let ctx_attribs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    // EGL keeps the window's address for the surface's life: the run's.
    static mut NATIVE: NativeWindow = NativeWindow { phys: 0, width: 0, height: 0, stride_bytes: 0, format: 0 };
    // SAFETY: main only; nothing else touches NATIVE.
    let native = unsafe { &mut *core::ptr::addr_of_mut!(NATIVE) };
    *native = NativeWindow {
        phys: win.phys as u64,
        width: win.w,
        height: win.h,
        stride_bytes: win.stride,
        format: win.format,
    };
    // SAFETY: the attribute lists end with EGL_NONE and outlive the calls;
    // every out pointer is writable.
    unsafe {
        // every board: the Pi 4 board layer gives the V3D OSAL its clock
        // switch through the OSAL's default-platform hook
        let dpy = eglGetDisplay(EGL_DEFAULT_DISPLAY);
        if eglInitialize(dpy, null_mut(), null_mut()) == 0 {
            return Err(egl_fail("eglInitialize"));
        }
        let mut cfg: EGLConfig = null_mut();
        let mut n: EGLint = 0;
        if eglChooseConfig(dpy, cfg_attribs.as_ptr(), &mut cfg, 1, &mut n) == 0 {
            return Err(egl_fail("eglChooseConfig"));
        }
        if n != 1 {
            say!("{NAME}: eglChooseConfig: {n} configs for RGBA8888");
            return Err(-ENOENT);
        }
        let surf = eglCreateWindowSurface(dpy, cfg, native, single_buffer.as_ptr());
        if surf == EGL_NO_SURFACE {
            return Err(egl_fail("eglCreateWindowSurface"));
        }
        let ctx = eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
        if ctx == EGL_NO_CONTEXT {
            return Err(egl_fail("eglCreateContext"));
        }
        if eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
            return Err(egl_fail("eglMakeCurrent"));
        }
        say!(
            "{NAME}: GL_RENDERER {}, GL_VERSION {}",
            text(glGetString(GL_RENDERER) as *const c_char),
            text(glGetString(GL_VERSION) as *const c_char)
        );
        Ok((dpy, surf))
    }
}

/// An info log as text: the bytes up to the first NUL.
fn log_text(log: &[u8]) -> &str {
    let end = log.iter().position(|&b| b == 0).unwrap_or(log.len());
    core::str::from_utf8(&log[..end]).unwrap_or("(not UTF-8)")
}

/// One compiled shader, or None (the log is printed). `src` ends with a NUL
/// (the check crate's texts do).
fn shader(kind: GLenum, src: &str) -> Option<GLuint> {
    assert!(src.ends_with('\0'), "a shader text without its NUL");
    let mut ok: GLint = 0;
    // SAFETY: a current context; `src` is NUL-terminated, so no lengths.
    unsafe {
        let s = glCreateShader(kind);
        glShaderSource(s, 1, &(src.as_ptr() as *const c_char), null());
        glCompileShader(s);
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut log = [0u8; 160];
            glGetShaderInfoLog(s, log.len() as GLsizei, null_mut(), log.as_mut_ptr() as *mut c_char);
            let which = if kind == GL_VERTEX_SHADER { "vs" } else { "fs" };
            say!("{NAME}: compile ({which}) failed: {}", log_text(&log));
            return None;
        }
        Some(s)
    }
}

/// The program in use; returns the locations of a_pos and a_color.
fn program() -> Result<(GLuint, GLuint), c_int> {
    let vs = shader(GL_VERTEX_SHADER, triangle_check::VS_SRC);
    let fs = shader(GL_FRAGMENT_SHADER, triangle_check::FS_SRC);
    let (Some(vs), Some(fs)) = (vs, fs) else {
        return Err(-EINVAL);
    };
    let mut ok: GLint = 0;
    // SAFETY: a current context; the names are NUL-terminated.
    let (a_pos, a_color) = unsafe {
        let prog = glCreateProgram();
        glAttachShader(prog, vs);
        glAttachShader(prog, fs);
        glLinkProgram(prog);
        glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut log = [0u8; 200];
            glGetProgramInfoLog(prog, log.len() as GLsizei, null_mut(), log.as_mut_ptr() as *mut c_char);
            say!("{NAME}: link failed: {}", log_text(&log));
            return Err(-EINVAL);
        }
        glUseProgram(prog);
        (glGetAttribLocation(prog, c"a_pos".as_ptr()), glGetAttribLocation(prog, c"a_color".as_ptr()))
    };
    say!("{NAME}: a_pos at {a_pos}, a_color at {a_color}");
    if a_pos < 0 || a_color < 0 {
        say!("{NAME}: an attribute has no location");
        return Err(-EINVAL);
    }
    Ok((a_pos as GLuint, a_color as GLuint))
}

/// The one frame: clear, the triangle, eglSwapBuffers (submits and waits).
fn draw(win: &board::Window, dpy: EGLDisplay, surf: EGLSurface, a_pos: GLuint, a_color: GLuint) -> Result<(), c_int> {
    // x, y, z, w, r, g, b, a per vertex, packed as the attributes read them
    let verts = triangle_check::VERTS;
    let [r, g, b, a] = triangle_check::CLEAR;
    let mut vbo: GLuint = 0;
    // SAFETY: a current context; glBufferData copies `verts` before it
    // returns; the attribute offsets are into that buffer.
    unsafe {
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&verts) as isize,
            verts.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glVertexAttribPointer(a_pos, 4, GL_FLOAT, 0, 32, null());
        glVertexAttribPointer(a_color, 4, GL_FLOAT, 0, 32, 16 as *const c_void);
        glEnableVertexAttribArray(a_pos);
        glEnableVertexAttribArray(a_color);

        glViewport(0, 0, win.w as GLsizei, win.h as GLsizei);
        glClearColor(r, g, b, a);
        glClear(GL_COLOR_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        progress(Stage::Draw);
        if eglSwapBuffers(dpy, surf) == 0 {
            return Err(egl_fail("eglSwapBuffers"));
        }
        let err = glGetError();
        if err != GL_NO_ERROR {
            say!("{NAME}: GL error {err:#x} during the frame");
            return Err(-EIO);
        }
    }
    Ok(())
}

/// One RGBA pixel at GL window coordinates, the context that drew current.
fn read_pixel(x: u32, y: u32) -> [u8; 4] {
    let mut px = [0u8; 4];
    // SAFETY: a current context; `px` holds one RGBA8 pixel.
    unsafe { glReadPixels(x as GLint, y as GLint, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    px
}

/// The run up to the records. Ok(the check's result) once the frame was
/// checked; Err(rc) when it stopped before.
fn run() -> Result<triangle_check::Outcome, c_int> {
    progress(Stage::Window);
    let win = window()?;
    progress(Stage::Egl);
    let (dpy, surf) = egl_up(&win)?;
    progress(Stage::Program);
    let (a_pos, a_color) = program()?;
    draw(&win, dpy, surf, a_pos, a_color)?;
    progress(Stage::Swapped);
    // the check reads the window, not the screen: go on after an error
    // SAFETY: `win` is the window board_window gave.
    let ret = unsafe { board::board_show(&win) };
    if ret != 0 {
        say!("{NAME}: board_show = {ret} (the window is not on the screen)");
    }
    // the guard rows follow the window in memory: `cpu` covers both
    let n = ((win.h + triangle_check::GUARD_ROWS) * win.w) as usize;
    // SAFETY: board_window gave `n` words at `cpu`; the GPU is done with them.
    let pixels = unsafe { core::slice::from_raw_parts(win.cpu as *const u32, n) };
    let frame = match Frame::new(pixels, win.w, win.h, triangle_check::GUARD_ROWS, win.format) {
        Ok(f) => f,
        Err(e) => {
            say!("{NAME}: check: not checked: {} ({}x{}, format {:#x})", e.what(), win.w, win.h, win.format);
            return Err(-EINVAL);
        }
    };
    let o = check(&frame, read_pixel);
    let _ = report(&mut Console::new(), NAME, &frame, &o);
    progress(Stage::Checked);
    Ok(o)
}

/// Zephyr's main thread: the run. Only [`triangle_rust_main`] is named.
#[no_mangle]
pub extern "C" fn main() -> c_int {
    triangle_rust_main()
}

/// The run and its records; does not return (board_finish). The image's
/// link asks for this symbol by name (CMakeLists.txt), which brings this
/// crate's object, and with it [`main`], into the link.
#[no_mangle]
pub extern "C" fn triangle_rust_main() -> ! {
    say!(
        "{NAME}: OpenGL ES 2.0 through YakoGL, {}x{} window, every call from Rust",
        triangle_check::WIDTH,
        triangle_check::HEIGHT
    );
    let rc = match run() {
        Ok(o) => {
            let [p1, p2, p3] = o.picture;
            // SAFETY: main only.
            unsafe { board::board_result(record::kind::PICTURE as u32, p1, p2, p3) };
            match o.verdict() {
                None => 0,
                Some(f) => {
                    // SAFETY: main only.
                    unsafe {
                        board::board_result(
                            record::kind::APP as u32,
                            (f as u32) << 8 | record::STATUS_FAIL,
                            1,
                            board::board_gpu_resets(),
                        )
                    };
                    -EBADMSG
                }
            }
        }
        Err(rc) => rc,
    };
    // SAFETY: a NUL-terminated name; main only; does not return.
    unsafe { board::board_finish(c"triangle-rust".as_ptr(), rc) }
}
