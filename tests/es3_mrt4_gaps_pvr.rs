// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `glDrawBuffers` lists with gaps that reach colour attachment 3, with the
//! PowerVR build and a four-target stand-in (plan #368 item `pvr-gaps4-test`;
//! RC14, docs/notes/mrt4-v3d-pvr.md section 2).
//!
//! `backend::HOST_DRAW_TARGETS` = 4 stands in for the knob. The GL core is
//! driven through the API into a recording back end (as `es3_msaa_pvr`): the
//! PowerVR's own draw needs the board's job memory, so what is checked is what
//! the core hands it (target list, `out_shift`, `clear_target_mask`), the
//! output-register placement `src/pvr/mrt_layout.rs` gives the written
//! targets, and a `glClearBufferfv` clear plus `glReadPixels` of attachment 3
//! (the stand-in writes the clear colour into the target's memory, which the
//! OSAL stand-in makes host memory).
//!
//! Lists, with four 8x8 RGBA8 attachments (the fourth alone in the last test):
//!
//! | list | targets to the back end | `out_shift` |
//! |---|---|---|
//! | {NONE, NONE, NONE, ATT3} | one, attachment 3 | 3 |
//! | {ATT0, NONE, NONE, ATT3} | four slots, 1 and 2 with gpu 0 | 0 |
//! | {NONE, ATT1, NONE, ATT3} | three slots from attachment 1, slot 1 gpu 0 | 1 |
//! | only attachment 3 attached, {NONE, NONE, NONE, ATT3} | one | 3 |
//!
//! Known defect (the `#[ignore]`d test): the PowerVR back end refuses any draw
//! with `out_shift` != 0 (`ENOTSUP`, "draw buffer 0 is GL_NONE while a later
//! draw buffer is written", review R-09), so the first and third list draw
//! nothing on the PowerVR, and the output-register placement for them would
//! need a remap of fragment output `k + out_shift` to register 0. Only the
//! list with attachment 0 written (the second) can be drawn. The register
//! placements below are what `mrt_layout::layout` gives the written targets;
//! `src/pvr/backend.rs` does not call it yet.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_gaps_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::mrt_layout::{layout, Place};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// What the back end was handed by one draw or clear.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    do_clear: bool,
    clear_target_mask: u8,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    out_shift: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    /* a clear writes its colour into the targets its mask names (RGBA8, the
     * whole 8x8 area), so a read-back has something to find */
    if d.do_clear && d.index_count == 0 {
        let px: [u8; 4] = d.clear_rgba.map(|c| (c * 255.0 + 0.5) as u8);
        for (k, t) in d.targets[..n].iter().enumerate() {
            if t.gpu != 0 && d.clear_target_mask & (1 << k) != 0 {
                for p in 0..64 {
                    core::ptr::copy_nonoverlapping(px.as_ptr(), (t.gpu as *mut u8).add(p * 4), 4);
                }
            }
        }
    }
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        do_clear: d.do_clear,
        clear_target_mask: d.clear_target_mask,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        out_shift: d.targets[0].out_shift,
    });
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-mrt4-gaps-pvr".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/// A bound 8x8 framebuffer with an RGBA8 renderbuffer on each attachment of
/// `which` (indices 0..4); the framebuffer and renderbuffers.
fn framebuffer(which: &[usize]) -> (GLuint, Vec<GLuint>) {
    let mut fb = 0;
    let mut rbs = Vec::new();
    // SAFETY: room for the names; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for &i in which {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, ATT[i], GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
    }
    (fb, rbs)
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glUseProgram(0);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
}

fn complete() -> bool {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE }
}

fn draw_buffers(bufs: &[GLenum]) {
    // SAFETY: `bufs.len()` readable enums.
    unsafe { glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers {bufs:x?}");
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

/// The current program: four float outputs, a distinct colour each.
fn program() {
    let fs = "#version 300 es\nprecision highp float;\n\
        layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
        layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
        void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); o1 = vec4(0.0, 1.0, 0.0, 1.0);\n\
        o2 = vec4(0.0, 0.0, 1.0, 1.0); o3 = vec4(1.0, 1.0, 0.0, 1.0); }\n";
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw the full-target triangle; what the back end was given for it.
fn draw() -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// The clear of draw buffer `buf` to `rgba`; what the back end was given.
fn clear_buffer(buf: GLint, rgba: [f32; 4]) -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: four readable floats.
    unsafe {
        glClearBufferfv(GL_COLOR, buf, rgba.as_ptr());
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.do_clear).expect("the clear reached the back end")
}

/// A pixel of colour attachment `att` of the bound framebuffer, read back.
fn read_pixel(att: GLenum) -> [u8; 4] {
    let mut px = [0u8; 4];
    // SAFETY: room for one RGBA8 pixel; plain GL calls.
    unsafe {
        let mut fb: GLint = 0;
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut fb);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb as GLuint);
        glReadBuffer(att);
        glReadPixels(3, 3, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/// The PowerVR's register placement of the written targets of `d` (one
/// RGBA8 dword each, `mrt_layout`): (slot, first register) of each.
fn registers(d: &Seen) -> Vec<(usize, u8)> {
    let written: Vec<usize> = (0..d.targets.len()).filter(|&k| d.targets[k].0 != 0).collect();
    let l = layout(&vec![1u8; written.len()]).expect("up to four one-dword targets");
    assert_eq!(l.tile_buffers, 0, "four RGBA8 targets fit the four registers");
    written
        .iter()
        .enumerate()
        .map(|(i, &slot)| match l.place[i] {
            Place::Register { first } => (slot, first),
            p => panic!("{p:?}"),
        })
        .collect()
}

/// The addresses of the four attachments, from a clear of draw buffer 0 with
/// the default list.
fn addresses() -> Vec<u64> {
    draw_buffers(&ATT);
    clear_buffer(0, [0.0; 4]).targets.iter().map(|t| t.0).collect()
}

const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/* ---- {NONE, NONE, NONE, ATT3} ------------------------------------------- */

#[test]
fn none_none_none_att3_is_one_target_shifted_by_three() {
    let _g = context();
    let (fb, rbs) = framebuffer(&[0, 1, 2, 3]);
    assert!(complete());
    let all = addresses();
    assert_eq!(all.len(), 4);
    draw_buffers(&[GL_NONE, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    program();
    let d = draw();
    assert_eq!(d.targets.len(), 1, "the list starts at the first written draw buffer");
    assert_eq!((d.targets[0].0, d.out_shift), (all[3], 3), "attachment 3 takes fragment output 3");
    assert_eq!(registers(&d), [(0, 0)], "one target is laid out in register 0");
    release(fb, &rbs);
}

/// Draw buffer 3 clears attachment 3 and is target 0 of the dense list; the
/// other draw buffers name nothing.
#[test]
fn none_none_none_att3_clears_and_reads_back_attachment_3() {
    let _g = context();
    let (fb, rbs) = framebuffer(&[0, 1, 2, 3]);
    draw_buffers(&[GL_NONE, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    let a3 = clear_buffer(3, YELLOW);
    assert_eq!((a3.out_shift, a3.clear_target_mask, a3.targets.len()), (3, 1, 1), "draw buffer 3 is target 0");
    assert_ne!(a3.targets[0].0, 0);
    for b in 0..3 {
        let c = clear_buffer(b, YELLOW);
        assert_eq!(c.clear_target_mask, 1 << 7, "draw buffer {b} is not in the list: it names nothing");
    }
    assert_eq!(read_pixel(GL_COLOR_ATTACHMENT3), [255, 255, 0, 255]);
    release(fb, &rbs);
}

/* ---- {ATT0, NONE, NONE, ATT3} -------------------------------------------- */

#[test]
fn att0_none_none_att3_keeps_four_slots_with_holes() {
    let _g = context();
    let (fb, rbs) = framebuffer(&[0, 1, 2, 3]);
    let all = addresses();
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    program();
    let d = draw();
    let got: Vec<u64> = d.targets.iter().map(|t| t.0).collect();
    assert_eq!(got, [all[0], 0, 0, all[3]], "slots 1 and 2 are holes");
    assert_eq!(d.out_shift, 0, "slot 0 is written: nothing moves");
    /* the written targets take registers in order: output 0 to register 0
     * and output 3 to register 1 (mrt_layout holds no hole) */
    assert_eq!(registers(&d), [(0, 0), (3, 1)]);

    /* clears: draw buffer 3 is slot 3 (mask bit 3), the holes name nothing */
    let c = clear_buffer(3, YELLOW);
    assert_eq!((c.out_shift, c.clear_target_mask), (0, 1 << 3));
    assert_eq!(read_pixel(GL_COLOR_ATTACHMENT3), [255, 255, 0, 255]);
    let c = clear_buffer(1, YELLOW);
    assert_eq!((c.clear_target_mask, c.targets[1].0), (1 << 1, 0), "draw buffer 1 is a hole: its bit names no target");
    release(fb, &rbs);
}

/* ---- {NONE, ATT1, NONE, ATT3} -------------------------------------------- */

#[test]
fn none_att1_none_att3_starts_at_attachment_1() {
    let _g = context();
    let (fb, rbs) = framebuffer(&[0, 1, 2, 3]);
    let all = addresses();
    draw_buffers(&[GL_NONE, GL_COLOR_ATTACHMENT1, GL_NONE, GL_COLOR_ATTACHMENT3]);
    program();
    let d = draw();
    let got: Vec<u64> = d.targets.iter().map(|t| t.0).collect();
    assert_eq!(got, [all[1], 0, all[3]], "the list starts at attachment 1, slot 1 is a hole");
    assert_eq!(d.out_shift, 1, "attachment 1 takes fragment output 1");
    assert_eq!(registers(&d), [(0, 0), (2, 1)]);

    /* draw buffer 3 is slot 2 of the shifted list: mask bit 2 */
    let c = clear_buffer(3, YELLOW);
    assert_eq!((c.out_shift, c.clear_target_mask), (1, 1 << 2));
    assert_eq!(c.targets[2].0, all[3]);
    assert_eq!(read_pixel(GL_COLOR_ATTACHMENT3), [255, 255, 0, 255]);
    let c = clear_buffer(1, YELLOW);
    assert_eq!(c.clear_target_mask, 1, "draw buffer 1 is target 0");
    release(fb, &rbs);
}

/* ---- a framebuffer whose only colour attachment is 3 -------------------- */

#[test]
fn only_attachment_3_is_complete_and_is_the_frame_target() {
    let _g = context();
    let (fb, rbs) = framebuffer(&[3]);
    assert!(complete(), "ES 3.0 4.4.4.2 asks nothing of attachment 0");
    draw_buffers(&[GL_NONE, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3]);
    let c = clear_buffer(3, YELLOW);
    assert_eq!((c.targets.len(), c.out_shift, c.clear_target_mask), (1, 3, 1));
    let gpu3 = c.targets[0].0;
    assert_ne!(gpu3, 0);
    program();
    let d = draw();
    assert_eq!((d.targets.len(), d.targets[0].0, d.out_shift), (1, gpu3, 3));
    assert_eq!(registers(&d), [(0, 0)]);
    assert_eq!(read_pixel(GL_COLOR_ATTACHMENT3), [255, 255, 0, 255]);

    /* a list naming attachment 0, which has no image: today the error
     * comes at the flush (glFinish), not at the clear */
    draw_buffers(&[GL_COLOR_ATTACHMENT0]);
    SEEN.lock().unwrap().clear();
    // SAFETY: four readable floats.
    unsafe { glClearBufferfv(GL_COLOR, 0, YELLOW.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "the clear itself is accepted: the clear is recorded, not drawn");
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_INVALID_FRAMEBUFFER_OPERATION, "the flush finds no image behind draw buffer 0");
    assert!(SEEN.lock().unwrap().is_empty(), "nothing reached the back end");
    release(fb, &rbs);
}

/* ---- the defect ---------------------------------------------------------- */

/// The PowerVR back end draws `out_shift` != 0 lists with the output
/// remapped. Today `src/pvr/backend.rs` refuses them (`ENOTSUP`, review
/// R-09), so {NONE, NONE, NONE, ATT3} and {NONE, ATT1, NONE, ATT3} draw
/// nothing on the PowerVR. The draw itself needs the board's job memory, so
/// this test fails until the refusal is gone.
#[test]
#[ignore = "defect: the PowerVR back end refuses a draw whose draw buffer 0 is GL_NONE (out_shift != 0, review R-09), so lists that start past attachment 0 draw nothing"]
fn the_powervr_backend_draws_shifted_lists() {
    panic!(
        "src/pvr/backend.rs refuses a draw with out_shift != 0 (draw buffer 0 is GL_NONE while a later one is written)"
    );
}
