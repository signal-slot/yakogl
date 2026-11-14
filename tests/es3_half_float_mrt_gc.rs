// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): where an RGBA16F colour target is rendered when it is
//! not the only draw buffer (plan #360, design-gc).
//!
//! An RGBA16F target that is the only draw buffer renders into the 4x4-tiled
//! shadow of src/vivante/wide_target.rs, seen on the wire as PE_LOGIC_OP
//! SINGLE_BUFFER(2). With a second draw buffer (RGBA8 at COLOR_ATTACHMENT1)
//! the draw keeps the LINEAR path, SINGLE_BUFFER(1), where the PE misplaces
//! 64-bit pixels (wide_target.rs "Not covered"), and `draw_one` does not
//! refuse it, as it refuses RGBA16I/UI. `today_*` pin that; the ignored test
//! states the correct behaviour: a refused draw or one shared multi-target
//! shadow (both targets SINGLE_BUFFER(2)).
//!
//! How it runs: as tests/gc_pe_wire_stream.rs, the real GL core and the real
//! Vivante back end over a stand-in OSAL that keeps every submitted stream.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_mrt_gc
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_half_float_mrt_gc -- --ignored
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glCheckFramebufferStatus, glDrawBuffers, glFramebufferTexture2D, glGenFramebuffers,
};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glFinish, glGetError, glViewport};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_debug_rs_clear, gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are
/// process-global.
static SERIAL: Mutex<()> = Mutex::new(());

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
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs) */
        match p.allocs.remove(&m.gpu) {
            Some(a) => core::mem::forget(a.mem),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept and its fence raised; every kick completes at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let words = words.to_vec();
        p.kicks.push(words);
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

/* ---- GL plumbing ------------------------------------------------------------------- */

/// Run `case` on the real Vivante back end over the stand-in OSAL and a
/// fresh 64x64 context.
fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: SERIAL is held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        gc_debug_rs_clear(true);
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| core::mem::take(&mut p.faults));
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 c0;\nlayout(location = 1) out vec4 c1;\nvoid main() { c0 = vec4(1.0, 0.5, 0.25, 0.5); c1 = vec4(0.25); }\n";

/// One triangle's positions.
static TRI: [f32; 12] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0];

/// A program and a buffer of [`TRI`] at attribute 0, bound and enabled.
fn setup() {
    // SAFETY: plain GL calls; TRI is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glViewport(0, 0, 64, 64);
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&TRI) as GLsizeiptr,
            TRI.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
    }
    no_error("the setup");
}

fn draw() {
    // SAFETY: a plain GL call; setup() bound the program and the array.
    unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
}

/// A complete 64x64 framebuffer object, bound: an RGBA16F texture at colour
/// 0 and, with `second`, an RGBA8 texture at colour 1; both are draw buffers.
fn half_float_fbo(second: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let mut tex: [GLuint; 2] = [0; 2];
        glGenTextures(2, tex.as_mut_ptr());
        glBindTexture(GL_TEXTURE_2D, tex[0]);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F as GLint, 64, 64, 0, GL_RGBA, GL_HALF_FLOAT, core::ptr::null());
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex[0], 0);
        let mut bufs = [GL_COLOR_ATTACHMENT0, GL_NONE];
        if second {
            glBindTexture(GL_TEXTURE_2D, tex[1]);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT1, GL_TEXTURE_2D, tex[1], 0);
            bufs[1] = GL_COLOR_ATTACHMENT1;
        }
        glDrawBuffers(if second { 2 } else { 1 }, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 64, 64);
    }
    no_error("the framebuffer object");
}

/// One draw into that framebuffer: the PE_LOGIC_OP SINGLE_BUFFER field at
/// the stream's DRAW_INSTANCED (1 linear, 2 tiled shadow; None: no draw was
/// submitted) and the GL error the draw left.
fn draw_outcome(second: bool) -> (Option<u32>, GLenum) {
    setup();
    half_float_fbo(second);
    with_platform(|p| p.kicks.clear());
    draw();
    // SAFETY: plain GL calls.
    let err = unsafe {
        glFinish();
        glGetError()
    };
    let kicks = with_platform(|p| core::mem::take(&mut p.kicks));
    let mut single = None;
    for words in &kicks {
        let mut cur = None;
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        if base + 4 * k as u32 == GC_PE_LOGIC_OP {
                            cur = Some((words[i + 1 + k] >> 8) & 3);
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    single = cur;
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    (single, err)
}

/// An RGBA16F target that is the only draw buffer is tiled.
#[test]
fn today_a_lone_rgba16f_target_uses_the_tiled_shadow() {
    with_backend(|| assert_eq!(draw_outcome(false), (Some(2), GL_NO_ERROR)));
}

/// RGBA16F beside RGBA8: not refused, and on the linear path (the defect).
#[test]
fn today_rgba16f_beside_rgba8_stays_linear_and_is_not_refused() {
    with_backend(|| assert_eq!(draw_outcome(true), (Some(1), GL_NO_ERROR)));
}

/// Correct behaviour: the draw is refused (nothing on the wire and a GL
/// error, as RGBA16I/UI), or both targets go to the multi-target shadow.
#[test]
#[ignore = "GC 16F MRT defect, step 0 of the GC arm (plan #360)"]
fn rgba16f_beside_rgba8_is_refused_or_tiled() {
    with_backend(|| {
        let (single, err) = draw_outcome(true);
        let refused = single.is_none() && err != GL_NO_ERROR;
        let tiled = single == Some(2) && err == GL_NO_ERROR;
        assert!(refused || tiled, "the PE misplaces linear 64-bit pixels: SINGLE_BUFFER {single:?}, error 0x{err:x}");
    });
}
