// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): an RGBA16F colour target (GL_EXT_color_buffer_half_float)
//! alone and beside another draw buffer (plan #360, item gc-mrt16f-test).
//! `src/vivante/wide_target.rs`, `src/vivante/backend.rs` (`wide_target`,
//! `draw_one`). Same stand-in OSAL and stream reading as
//! tests/es3_int_rt_mrt_gc.rs.
//!
//! What the back end does today, which these tests pin:
//!
//! - An RGBA16F target that is the only hardware target renders into the
//!   4x4-tiled shadow (padded stride, SINGLE_BUFFER(2)) and is untiled into
//!   the linear storage when the kick retires. A draw buffer set to GL_NONE
//!   beside it leaves one hardware target: the same.
//! - An RGBA16F target beside another draw buffer (RGBA8 at the other
//!   attachment) keeps the LINEAR path, where the PE misplaces 64-bit pixels
//!   (wide_target.rs "Not covered"). `draw_one` refuses only the 64-bit
//!   INTEGER targets there, so the draw is accepted and goes to the PE with
//!   no shadow: a known defect, pinned here so that a change to it shows.
//!   When it is fixed (a shadow per target, or a refusal), the last test is
//!   the one to change.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_mrt_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/// The value of `#define <name>` in driver/src/vivante/hw/state_3d.xml.h.
fn rnndb(name: &str) -> u32 {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/driver/src/vivante/hw/state_3d.xml.h");
    let text = std::fs::read_to_string(path).expect("the vendored rnndb header");
    let line = text
        .lines()
        .find(|l| l.split_whitespace().nth(1) == Some(name) && l.starts_with("#define"))
        .unwrap_or_else(|| panic!("{name} is in state_3d.xml.h"));
    let v = line.split_whitespace().nth(2).expect("a value");
    u32::from_str_radix(v.trim_start_matches("0x"), 16).expect("hex")
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| p.allocs.remove(&m.gpu));
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        p.kicks.push(words.to_vec());
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    with_platform(|p| {
        /* leaked, not freed, as tests/gc_npot_repeat.rs */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn program(fs: &str) -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

fn renderbuffer(internal: GLenum, w: u32, h: u32) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, internal, w as GLsizei, h as GLsizei);
    }
    assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(0x{internal:x}, {w}x{h})");
    rb
}

/// A framebuffer with `rbs` at colour attachments 0.., and `bufs` as its draw buffers.
fn framebuffer_with(rbs: &[GLuint], bufs: &[GLenum]) -> GLuint {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &rb) in rbs.iter().enumerate() {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR, "framebuffer");
    fb
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// Draw the quad over the `w` x `h` viewport with `fs`, finish, and return
/// the error and the writes of the streams submitted since.
fn draw(fs: &str, w: u32, h: u32) -> (GLenum, Vec<(u32, u32)>) {
    let kicks0 = with_platform(|p| p.kicks.len());
    // SAFETY: plain GL calls; QUAD is a static.
    let e = unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glUseProgram(program(fs));
        glDisable(GL_BLEND);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        let e = glGetError();
        glFinish();
        e
    };
    (e, with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect()))
}

fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
    w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
}

/// The SINGLE_BUFFER field of the last PE_LOGIC_OP the draw wrote.
fn single_buffer(w: &[(u32, u32)]) -> u32 {
    let logic = last(w, GC_PE_LOGIC_OP).expect("PE_LOGIC_OP");
    let mask = rnndb("VIVS_PE_LOGIC_OP_SINGLE_BUFFER__MASK");
    (logic & mask) >> mask.trailing_zeros()
}

/// (bytes tiled into shadows, bytes untiled out of them) since init.
fn counts() -> (u64, u64) {
    yakogl::vivante::wide_target::counts()
}

const FS_ONE: &str = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\nvoid main() { o = vec4(0.25, 0.5, 1.0, 2.0); }\n";
const FS_TWO: &str = "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\nvoid main() { o0 = vec4(0.25, 0.5, 1.0, 2.0); o1 = vec4(1.0, 0.0, 0.0, 1.0); }\n";

/// Sizes with and without padding to the 16x4 shadow.
const SIZES: [(u32, u32); 3] = [(17, 13), (16, 4), (5, 3)];

/// The one-target shadow path for an RGBA16F draw of `w` x `h`.
fn assert_shadowed(ws: &[(u32, u32)], w: u32, h: u32, untiled: u64) {
    let (pw, _) = yakogl::vivante::wide_target::padded(w, h);
    assert_eq!(last(ws, rnndb("VIVS_PE_COLOR_STRIDE")), Some(pw * 8), "{w}x{h}: the padded stride");
    assert_eq!(single_buffer(ws), 2, "{w}x{h}: SINGLE_BUFFER(2)");
    assert_eq!(untiled, u64::from(w * h * 8), "{w}x{h}: the pixels went back, not the padding");
    let addr = last(ws, pe_pipe_color_addr(0)).expect("PE_PIPE_COLOR_ADDR(0)");
    assert_eq!(addr & 0xfff, 0, "{w}x{h}: the shadow starts on a page");
}

/// RGBA16F alone: the tiled shadow, at sizes with and without padding.
#[test]
fn an_rgba16f_target_alone_uses_the_tiled_shadow() {
    for (w, h) in SIZES {
        on_gc7000(|| {
            let rb = renderbuffer(GL_RGBA16F, w, h);
            framebuffer_with(&[rb], &[GL_COLOR_ATTACHMENT0]);
            let u0 = counts().1;
            let (e, ws) = draw(FS_ONE, w, h);
            assert_eq!(e, GL_NO_ERROR, "{w}x{h}: drawn");
            assert_shadowed(&ws, w, h, counts().1 - u0);
        });
    }
}

/// RGBA16F at attachment 0 with attachment 1 not drawn (GL_NONE): one
/// hardware target, so the shadow serves it.
#[test]
fn an_rgba16f_target_beside_a_none_draw_buffer_uses_the_tiled_shadow() {
    on_gc7000(|| {
        let (w, h) = (17, 13);
        let a = renderbuffer(GL_RGBA16F, w, h);
        let b = renderbuffer(GL_RGBA8, w, h);
        framebuffer_with(&[a, b], &[GL_COLOR_ATTACHMENT0, GL_NONE]);
        let u0 = counts().1;
        let (e, ws) = draw(FS_ONE, w, h);
        assert_eq!(e, GL_NO_ERROR, "drawn");
        assert_shadowed(&ws, w, h, counts().1 - u0);
    });
}

/// RGBA16F + RGBA8 as two draw buffers, in either order: the draw is
/// accepted and reaches the PE on the LINEAR path, with no shadow (nothing
/// tiled in or untiled out, no SINGLE_BUFFER(2)). That is the defect of
/// wide_target.rs "Not covered": the PE misplaces 64-bit pixels there.
#[test]
fn an_rgba16f_target_beside_another_draw_buffer_stays_on_the_linear_path() {
    for (first, second) in [(GL_RGBA16F, GL_RGBA8), (GL_RGBA8, GL_RGBA16F)] {
        on_gc7000(|| {
            let (w, h) = (17, 13);
            let a = renderbuffer(first, w, h);
            let b = renderbuffer(second, w, h);
            framebuffer_with(&[a, b], &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1]);
            let (t0, u0) = counts();
            let (e, ws) = draw(FS_TWO, w, h);
            assert_eq!(e, GL_NO_ERROR, "0x{first:x} + 0x{second:x}: accepted, not refused");
            assert!(
                ws.iter()
                    .any(|&(r, _)| r == GC_PE_COLOR_FORMAT || r == pe_rt_config_8(1) || r == pe_pipe_color_addr(0)),
                "the draw reached the pixel engine"
            );
            let (t1, u1) = counts();
            assert_eq!((t1 - t0, u1 - u0), (0, 0), "no shadow was tiled or untiled");
            assert_ne!(single_buffer(&ws), 2, "not the SINGLE_BUFFER(2) of a shadow draw");
        });
    }
}
