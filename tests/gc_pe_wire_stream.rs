// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the pixel engine words `emit_pe_state`
//! (src/vivante/backend.rs) writes now that it takes `DISABLE_ZS` from
//! src/vivante/pe_zs.rs and the colour COMPONENTS and OVERWRITE from
//! src/vivante/pe_overwrite.rs (docs/notes/gc-slow-terrain-stream-diff.md,
//! ranks 8 and 9), clears included.
//!
//! PE_DEPTH_CONFIG:
//!
//! * A target with no depth or stencil attachment (terrain's noise, normal,
//!   bloom and tilt passes) skips the stage: `DISABLE_ZS`, ALWAYS, no write,
//!   for a draw with GL_DEPTH_TEST on and for a clear quad alike (Mesa's
//!   0x01000700 there, instead of our old 0x00040711).
//! * With an attachment, the words are what they were: a draw tests as GL
//!   asks and writes with the test on; with depth and stencil tests off it
//!   has `DISABLE_ZS`; a clear quad tests ALWAYS, writes depth only when it
//!   clears it, and never has `DISABLE_ZS`.
//!
//! PE_COLOR_FORMAT: a full mask with no blending is OVERWRITE; blending or a
//! masked channel drops it; a GL_RGB target keeps its alpha byte (COMPONENTS
//! 0x7, no OVERWRITE) for a draw, and a clear of it writes all four
//! (its alpha, 1) with OVERWRITE.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs). The scissored clears below are what keeps a
//! clear a quad (tests/gc_rs_clear_stream.rs: a whole clear is an RS fill).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_pe_wire_stream
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GLES_CMP_ALWAYS, GLES_CMP_LESS};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{
    glBlendFunc, glClear, glColorMask, glDepthFunc, glEnable, glFinish, glGetError, glScissor, glViewport,
};
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

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0, 0.5, 0.25, 0.5); }\n";

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

/// A complete 64x64 framebuffer object with a `format` texture at colour 0
/// and no depth or stencil attachment, bound.
fn colour_only_fbo(format: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let mut tex: GLuint = 0;
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, 64, 64, 0, format, GL_UNSIGNED_BYTE, core::ptr::null());
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 64, 64);
    }
    no_error("the framebuffer object");
}

/// The value each of `regs` holds at the stream's one DRAW_INSTANCED, from
/// what `f` submits (u32::MAX: never written in that kick).
fn at_the_draw<const N: usize>(regs: [u32; N], f: impl FnOnce()) -> [u32; N] {
    with_platform(|p| p.kicks.clear());
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    no_error("the draw");
    let kicks = with_platform(|p| core::mem::take(&mut p.kicks));
    let mut found = Vec::new();
    for words in &kicks {
        let mut cur = [u32::MAX; N];
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    let base = (w & 0xffff) * 4;
                    for k in 0..count {
                        let r = base + 4 * k as u32;
                        if let Some(j) = regs.iter().position(|&x| x == r) {
                            cur[j] = words[i + 1 + k];
                        }
                    }
                    i += 1 + count;
                }
                GC_OP_DRAW_INSTANCED => {
                    found.push(cur);
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    assert_eq!(found.len(), 1, "one DRAW_INSTANCED");
    found[0]
}

fn draw() {
    // SAFETY: a plain GL call; setup() bound the program and the array.
    unsafe { glDrawArrays(GL_TRIANGLES, 0, 3) };
}

/// A clear under a scissor box smaller than the target: the clear quad.
fn scissored_clear(mask: GLbitfield) {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SCISSOR_TEST);
        glScissor(0, 0, 16, 16);
        glClear(mask);
    }
}

/* ---- the words --------------------------------------------------------------------- */

/// PE_DEPTH_CONFIG as emit_pe_state builds it: D24S8, linear.
const fn depth_config(func: u32, write: bool, disable_zs: bool) -> u32 {
    GC_PE_DEPTH_MODE_Z
        | GC_PE_DEPTH_FORMAT_D24S8
        | GC_PE_DEPTH_UNK18
        | pe_depth_func(func)
        | if write { GC_PE_DEPTH_WRITE_ENABLE } else { 0 }
        | if disable_zs { GC_PE_DEPTH_DISABLE_ZS } else { 0 }
}

/// COMPONENTS and OVERWRITE of a PE_COLOR_FORMAT word.
fn colour_bits(v: u32) -> (u32, bool) {
    let comp = (0..16).find(|&m| v & pe_color_format_components(0xf) == pe_color_format_components(m)).unwrap();
    (comp, v & GC_PE_COLOR_FORMAT_OVERWRITE != 0)
}

/* ---- PE_DEPTH_CONFIG ---------------------------------------------------------------- */

#[test]
fn a_draw_with_no_depth_attachment_skips_the_zs_stage() {
    with_backend(|| {
        setup();
        colour_only_fbo(GL_RGBA);
        let [d] = at_the_draw([GC_PE_DEPTH_CONFIG], || {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
            }
            draw();
        });
        assert_eq!(d, depth_config(GLES_CMP_ALWAYS, false, true));
        assert_eq!(d, 0x0104_0711, "Mesa's 0x01000700 plus our MODE_Z, D24S8 and UNK18 bits");
    });
}

#[test]
fn a_clear_quad_with_no_depth_attachment_skips_the_zs_stage() {
    with_backend(|| {
        colour_only_fbo(GL_RGBA);
        let [d] = at_the_draw([GC_PE_DEPTH_CONFIG], || scissored_clear(GL_COLOR_BUFFER_BIT));
        assert_eq!(d, depth_config(GLES_CMP_ALWAYS, false, true), "not the old 0x00040711");
    });
}

#[test]
fn a_draw_with_the_default_depth_tests_as_gl_asks() {
    with_backend(|| {
        setup();
        let [d] = at_the_draw([GC_PE_DEPTH_CONFIG], || {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
            }
            draw();
        });
        assert_eq!(d, depth_config(GLES_CMP_LESS, true, false));
    });
}

#[test]
fn a_draw_with_no_depth_or_stencil_test_skips_the_zs_stage() {
    with_backend(|| {
        setup();
        let [d] = at_the_draw([GC_PE_DEPTH_CONFIG], draw);
        assert_eq!(d, depth_config(GLES_CMP_ALWAYS, false, true));
    });
}

#[test]
fn a_depth_clear_quad_writes_depth_with_always() {
    with_backend(|| {
        let [d] = at_the_draw([GC_PE_DEPTH_CONFIG], || scissored_clear(GL_DEPTH_BUFFER_BIT));
        assert_eq!(d, depth_config(GLES_CMP_ALWAYS, true, false));
    });
}

#[test]
fn a_colour_clear_quad_with_a_depth_attachment_writes_no_depth() {
    with_backend(|| {
        let [d] = at_the_draw([GC_PE_DEPTH_CONFIG], || scissored_clear(GL_COLOR_BUFFER_BIT));
        assert_eq!(d, depth_config(GLES_CMP_ALWAYS, false, false));
    });
}

/* ---- PE_COLOR_FORMAT ---------------------------------------------------------------- */

#[test]
fn a_full_mask_without_blending_is_overwrite() {
    with_backend(|| {
        setup();
        let [c] = at_the_draw([GC_PE_COLOR_FORMAT], draw);
        assert_eq!(colour_bits(c), (0xf, true));
    });
}

#[test]
fn blending_drops_overwrite() {
    with_backend(|| {
        setup();
        let [c] = at_the_draw([GC_PE_COLOR_FORMAT], || {
            // SAFETY: plain GL calls.
            unsafe {
                glEnable(GL_BLEND);
                glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            }
            draw();
        });
        assert_eq!(colour_bits(c), (0xf, false));
    });
}

#[test]
fn a_masked_channel_drops_overwrite() {
    with_backend(|| {
        setup();
        let [c] = at_the_draw([GC_PE_COLOR_FORMAT], || {
            // SAFETY: a plain GL call.
            unsafe { glColorMask(1, 0, 1, 1) };
            draw();
        });
        assert_eq!(colour_bits(c), (0xd, false));
    });
}

#[test]
fn a_draw_into_an_rgb_target_keeps_its_alpha_byte() {
    with_backend(|| {
        setup();
        colour_only_fbo(GL_RGB);
        let [c] = at_the_draw([GC_PE_COLOR_FORMAT], draw);
        assert_eq!(colour_bits(c), (0x7, false));
    });
}

#[test]
fn a_clear_quad_of_an_rgb_target_writes_all_four_with_overwrite() {
    with_backend(|| {
        colour_only_fbo(GL_RGB);
        let [c] = at_the_draw([GC_PE_COLOR_FORMAT], || scissored_clear(GL_COLOR_BUFFER_BIT));
        assert_eq!(colour_bits(c), (0xf, true));
    });
}
