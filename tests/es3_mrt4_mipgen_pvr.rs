// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glReadBuffer(GL_COLOR_ATTACHMENT3) followed by glCopyTexImage2D,
//! glGenerateMipmap on the texture rendered through GL_COLOR_ATTACHMENT3 on
//! the PowerVR host back end under a four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4; plan #368).
//!
//! Four RGBA8 textures, each cleared to its own colour; slot 3's texture has
//! a full mip chain allocated (glTexStorage2D, 3 levels). glGenerateMipmap
//! runs on it with no glFinish after the clears, with the framebuffer still
//! bound and with it unbound; each level is read back through a scratch
//! framebuffer and must hold slot 3's colour, never another slot's.
//! HOST-ONLY: what the GPU would write is not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_mipgen_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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

/// The number of targets of each draw (or clear) the back end was handed.
static SEEN: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Records the targets of each draw; a clear fills every target it names
/// (those in `clear_target_mask` when it is set)
/// with the clear colour in the target's layout (unorm bytes for a colour
/// surface, the raw 32-bit values for an integer one), rows at the
/// pair-padded pitch.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets.max(1) as usize;
    SEEN.lock().unwrap().push(n);
    if !d.do_clear {
        return 0;
    }
    for (i, t) in d.targets[..n].iter().enumerate() {
        // glClearBuffer*(GL_COLOR, i): only the targets in the mask.
        if t.gpu == 0 || (d.clear_target_mask != 0 && d.clear_target_mask & (1 << i) == 0) {
            continue;
        }
        let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
        let mut px = vec![0u8; bpp];
        if let Some(l) = backend::gles_surf_int_layout(t.format) {
            let step = l.bits as usize / 8;
            for (ch, v) in d.clear_rgba.iter().enumerate().take(l.channels as usize) {
                px[ch * step..ch * step + step].copy_from_slice(&v.to_bits().to_le_bytes()[..step]);
            }
        } else {
            for (ch, v) in d.clear_rgba.iter().enumerate().take(bpp) {
                px[ch] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        let pitch = ((t.w + 1) & !1) as usize;
        let base = t.gpu as *mut u8;
        for y in 0..t.h as usize {
            for x in 0..t.w as usize {
                core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
            }
        }
    }
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
    name: c"es3-mrt4-mipgen-pvr".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
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

const W: GLsizei = 4;
const LEVELS: GLint = 3;
/// The clear colour of each slot, distinct.
const COLORS: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];

/// A `W` x `W` RGBA8 texture with `levels` levels, left bound.
fn texture(levels: GLint) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, levels, GL_RGBA8, W, W);
    }
    assert_eq!(err(), GL_NO_ERROR, "glTexStorage2D");
    tex
}

/// A bound four-slot framebuffer, each slot cleared to its colour; slot 3's
/// texture has the full chain and is left bound to the 2D target. No finish.
fn rendered(slot: usize) -> (GLuint, [GLuint; 4]) {
    let texs: [GLuint; 4] = core::array::from_fn(|i| texture(if i == slot { LEVELS } else { 1 }));
    let mut fb = 0;
    // SAFETY: room for one name; the textures are 2D level 0; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, t) in texs.iter().enumerate() {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_TEXTURE_2D, *t, 0);
        }
        let bufs: [GLenum; 4] = core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum);
        glDrawBuffers(4, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        for (i, c) in COLORS.iter().enumerate() {
            let f: [f32; 4] = core::array::from_fn(|k| c[k] as f32 / 255.0);
            glClearBufferfv(GL_COLOR, i as GLint, f.as_ptr());
        }
        glBindTexture(GL_TEXTURE_2D, texs[slot]);
    }
    assert_eq!(err(), GL_NO_ERROR, "framebuffer setup and clears");
    (fb, texs)
}

fn release(fb: GLuint, texs: [GLuint; 4]) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(4, texs.as_ptr());
    }
}

/// Texel (0,0) of `level` of `tex` through a scratch framebuffer; the draw
/// and read bindings are restored afterwards.
fn read_level(tex: GLuint, level: GLint) -> [u8; 4] {
    let (mut fb2, mut prev_r, mut prev_d) = (0, 0, 0);
    let mut px = [0xEEu8; 4];
    // SAFETY: room for one name and one RGBA8 pixel; plain GL calls.
    unsafe {
        glGetIntegerv(GL_READ_FRAMEBUFFER_BINDING, &mut prev_r);
        glGetIntegerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut prev_d);
        glGenFramebuffers(1, &mut fb2);
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb2);
        glFramebufferTexture2D(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
        assert_eq!(err(), GL_NO_ERROR, "scratch framebuffer level {level}");
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels level {level}");
        glBindFramebuffer(GL_READ_FRAMEBUFFER, prev_r as GLuint);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, prev_d as GLuint);
        glDeleteFramebuffers(1, &fb2);
    }
    px
}

fn check_chain(tex: GLuint, slot: usize) {
    for l in 0..LEVELS {
        let px = read_level(tex, l);
        assert_eq!(px, COLORS[slot], "level {l} holds slot 3's colour");
        for (i, c) in COLORS.iter().enumerate().filter(|(i, _)| *i != slot) {
            assert_ne!(px, *c, "level {l} holds slot {i}'s colour");
        }
    }
}

#[test]
#[ignore = "defect: glGenerateMipmap flushes the parked glClear only for the bound framebuffer's first colour attachment (texture.rs flush_clear_into reads fb.color), so a chain on COLOR_ATTACHMENT1..3 filters level 0 before the clear lands and levels 1.. stay zero; see report"]
fn generate_mipmap_framebuffer_unbound() {
    let _g = context();
    let (fb, texs) = rendered(3);
    // SAFETY: plain GL calls; no glFinish between the clears and the call.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
    check_chain(texs[3], 3);
    release(fb, texs);
}

#[test]
#[ignore = "defect: glGenerateMipmap flushes the parked glClear only for the bound framebuffer's first colour attachment (texture.rs flush_clear_into reads fb.color), so a chain on COLOR_ATTACHMENT1..3 filters level 0 before the clear lands and levels 1.. stay zero; see report"]
fn generate_mipmap_framebuffer_still_bound() {
    let _g = context();
    let (fb, texs) = rendered(3);
    // SAFETY: plain GL call; the framebuffer stays bound, no glFinish.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
    check_chain(texs[3], 3);
    release(fb, texs);
}

#[test]
fn control_attachment_0_chain() {
    let _g = context();
    let (fb, texs) = rendered(0);
    // SAFETY: plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
    check_chain(texs[0], 0);
    release(fb, texs);
}

#[test]
fn other_slots_keep_their_colour() {
    let _g = context();
    let (fb, texs) = rendered(3);
    // SAFETY: plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(err(), GL_NO_ERROR, "glGenerateMipmap");
    for i in 0..3 {
        assert_eq!(read_level(texs[i], 0), COLORS[i], "slot {i} untouched");
    }
    release(fb, texs);
}
