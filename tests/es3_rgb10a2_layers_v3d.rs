// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RGB10_A2 texture attachments other than level 0 of a 2D texture on the
//! V3D host back end, through the GL entry points only (plan #370,
//! rgb10a2-layers-api): a cube-map face other than +X, a 2D-array layer > 0
//! and a 3D slice > 0 (glFramebufferTextureLayer), and level 1 of a 2D
//! texture. For each: framebuffer completeness, glClearBufferfv with a
//! distinct value per channel read back by glReadPixels
//! RGBA/UNSIGNED_BYTE (the decoded word), the sibling faces, layers, slices
//! or levels untouched, and glBlitFramebuffer from the attachment into an
//! RGBA8 renderbuffer. The read pair (GL_IMPLEMENTATION_COLOR_READ_*) is
//! not asserted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rgb10a2_layers_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
/// The stand-in GPU: a clear draw into a `GLES_SURF_RGB10_A2` target writes
/// the clear's 10/10/10/2 word inside the draw's scissor box, for the
/// channels of `clear_color_mask` only. That is the V3D back end's job (the
/// TLB clear or the clear quad), which does not run on the host; the test
/// judges what the GL core hands it and what glReadPixels makes of the words
/// that result.
///
/// Quantisation rule: red, green and blue are `clamp(v, 0, 1) * 1023`
/// rounded half up, alpha `clamp(v, 0, 1) * 3` rounded half up (the GL core
/// has already rounded alpha to the target's 2 bits, `quantise_unorm`).
/// glReadPixels then decodes a field to a byte as (v * 255 + max / 2) / max.
unsafe extern "C" fn b_draw(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    if d.do_clear && d.target_format == backend::GLES_SURF_RGB10_A2 && d.target_gpu != 0 {
        let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n + 0.5) as u32;
        let c = d.clear_rgba;
        let new = [q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0)];
        let pitch = backend::draw_pitch(d) as usize;
        let base = d.target_gpu as *mut u32;
        let [sx, sy, sw, sh] = if d.scissor { d.scissor_rect } else { [0, 0, d.target_w as i32, d.target_h as i32] };
        for y in sy.max(0)..(sy + sh).min(d.target_h as i32) {
            for x in sx.max(0)..(sx + sw).min(d.target_w as i32) {
                let p = base.add(y as usize * pitch + x as usize);
                let w = *p;
                let old = [w & 0x3ff, w >> 10 & 0x3ff, w >> 20 & 0x3ff, w >> 30];
                let f: Vec<u32> =
                    (0..4).map(|i| if d.clear_color_mask >> i & 1 != 0 { new[i] } else { old[i] }).collect();
                *p = f[0] | f[1] << 10 | f[2] << 20 | f[3] << 30;
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
    name: c"es3-rgb10a2-layers".as_ptr() as *const c_char,
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
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn a2(r: u32, g: u32, b: u32, a: u32) -> u32 {
    r | g << 10 | b << 20 | a << 30
}

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

fn decoded(w: u32) -> [u8; 4] {
    [scale(w & 0x3ff, 1023), scale((w >> 10) & 0x3ff, 1023), scale((w >> 20) & 0x3ff, 1023), scale(w >> 30, 3)]
}

const N: GLsizei = 4;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Cube,
    Array,
    Volume,
    Level,
}

/// Sites (faces, layers, slices or levels) of a texture of `kind`.
fn sites(kind: Kind) -> u32 {
    match kind {
        Kind::Cube => 6,
        Kind::Array | Kind::Volume => 3,
        Kind::Level => 2,
    }
}

/// Edge of site `i`.
fn side(kind: Kind, i: u32) -> GLsizei {
    match kind {
        Kind::Level => N >> i,
        _ => N,
    }
}

/// The word every texel of site `i` starts with; one per site.
fn sentinel(i: u32) -> u32 {
    a2(1000 - i, 5 + i, 600, 1)
}

/// What glClearBufferfv writes: 300/1023, 700/1023, 50/1023, 2/3.
const CLEAR_WORD: u32 = 300 | 700 << 10 | 50 << 20 | 2 << 30;

fn target(kind: Kind) -> GLenum {
    match kind {
        Kind::Cube => GL_TEXTURE_CUBE_MAP,
        Kind::Array => GL_TEXTURE_2D_ARRAY,
        Kind::Volume => GL_TEXTURE_3D,
        Kind::Level => GL_TEXTURE_2D,
    }
}

/// The site the clear goes to: not the first.
fn picked(kind: Kind) -> u32 {
    match kind {
        Kind::Cube => 3,
        Kind::Array => 1,
        Kind::Volume => 2,
        Kind::Level => 1,
    }
}

fn make_texture(kind: Kind) -> GLuint {
    let mut tex = 0;
    let t = target(kind);
    // SAFETY: room for one name; every upload holds side x side (x depth) words.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(t, tex);
        let internal = GL_RGB10_A2 as GLint;
        let ty = GL_UNSIGNED_INT_2_10_10_10_REV;
        match kind {
            Kind::Cube | Kind::Level => {
                for i in 0..sites(kind) {
                    let s = side(kind, i);
                    let px = vec![sentinel(i); (s * s) as usize];
                    let (tgt, lvl) = match kind {
                        Kind::Cube => (GL_TEXTURE_CUBE_MAP_POSITIVE_X + i, 0),
                        _ => (GL_TEXTURE_2D, i as GLint),
                    };
                    glTexImage2D(tgt, lvl, internal, s, s, 0, GL_RGBA, ty, px.as_ptr() as *const c_void);
                }
            }
            Kind::Array | Kind::Volume => {
                let mut px = Vec::new();
                for i in 0..sites(kind) {
                    px.extend(vec![sentinel(i); (N * N) as usize]);
                }
                glTexImage3D(t, 0, internal, N, N, 3, 0, GL_RGBA, ty, px.as_ptr() as *const c_void);
            }
        }
        glTexParameteri(t, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "upload {kind:?}");
    tex
}

/// Attach site `i` of `tex` to colour attachment 0 of the bound framebuffer.
fn attach_site(kind: Kind, tex: GLuint, i: u32) {
    // SAFETY: plain GL calls.
    unsafe {
        match kind {
            Kind::Cube => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_CUBE_MAP_POSITIVE_X + i, tex, 0)
            }
            Kind::Array | Kind::Volume => {
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, i as GLint)
            }
            Kind::Level => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, i as GLint),
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "attach {kind:?} site {i}");
}

fn read_bound(w: GLsizei) -> Vec<[u8; 4]> {
    let mut px = vec![0u8; (w * w * 4) as usize];
    // SAFETY: `px` holds w x w RGBA8 pixels.
    unsafe { glReadPixels(0, 0, w, w, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

fn check_site(kind: Kind, fb: GLuint, tex: GLuint, i: u32, word: u32, what: &str) {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    attach_site(kind, tex, i);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE, "{kind:?} site {i}");
    let got = read_bound(side(kind, i));
    assert!(
        got.iter().all(|p| *p == decoded(word)),
        "{what}: {kind:?} site {i}: {:?} want {:?}",
        got[0],
        decoded(word)
    );
}

fn run(kind: Kind) {
    let _g = context();
    let tex = make_texture(kind);
    let (mut fb, mut rb, mut fb8) = (0, 0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glGenRenderbuffers(1, &mut rb);
        glGenFramebuffers(1, &mut fb8);
    }
    let pick = picked(kind);
    for i in 0..sites(kind) {
        check_site(kind, fb, tex, i, sentinel(i), "initial");
    }
    // Clear the picked site only.
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    attach_site(kind, tex, pick);
    // SAFETY: plain GL call.
    unsafe { glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr()) };
    let c = [300.0 / 1023.0, 700.0 / 1023.0, 50.0 / 1023.0, 2.0 / 3.0f32];
    // SAFETY: `c` holds four floats.
    unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv");
    // SAFETY: plain GL call. A pending clear goes out here, see `clear_survives_reattach`.
    unsafe { glFinish() };
    for i in 0..sites(kind) {
        let want = if i == pick { CLEAR_WORD } else { sentinel(i) };
        check_site(kind, fb, tex, i, want, "after clear");
    }
    // Blit the picked site into an RGBA8 renderbuffer.
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    attach_site(kind, tex, pick);
    let s = side(kind, pick);
    // SAFETY: plain GL calls on names made above.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, s, s);
        glBindFramebuffer(GL_FRAMEBUFFER, fb8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "RGBA8 destination");
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, fb8);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(0, 0, s, s, 0, 0, s, s, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR, "glBlitFramebuffer");
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_READ_FRAMEBUFFER, fb8) };
    let got = read_bound(s);
    assert!(
        got.iter().all(|p| *p == decoded(CLEAR_WORD)),
        "blit {kind:?}: {:?} want {:?}",
        got[0],
        decoded(CLEAR_WORD)
    );
    // The source site and its siblings are unchanged by the blit.
    for i in 0..sites(kind) {
        let want = if i == pick { CLEAR_WORD } else { sentinel(i) };
        check_site(kind, fb, tex, i, want, "after blit");
    }
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteFramebuffers(1, &fb8);
        glDeleteRenderbuffers(1, &rb);
        glDeleteTextures(1, &tex);
    }
}

#[test]
fn cube_face_attachment() {
    run(Kind::Cube);
}

#[test]
fn array_layer_attachment() {
    run(Kind::Array);
}

#[test]
fn volume_slice_attachment() {
    run(Kind::Volume);
}

#[test]
fn mip_level_attachment() {
    run(Kind::Level);
}

/// Defect: glClearBufferfv, then glFramebufferTextureLayer to another layer
/// and glReadPixels with no glFinish between. The clear is still pending
/// when the attachment changes, and it goes out into the layer attached at
/// the flush (layer 0 here), not the one that was attached at the call.
/// ES 3.0 4.2.3 clears the buffers of the framebuffer as bound at the call.
#[test]
#[ignore = "defect: a pending clear lands on the attachment bound at the flush, not at the glClearBufferfv call"]
fn clear_survives_reattach() {
    let _g = context();
    let tex = make_texture(Kind::Array);
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    attach_site(Kind::Array, tex, 1);
    let c = [300.0 / 1023.0, 700.0 / 1023.0, 50.0 / 1023.0, 2.0 / 3.0f32];
    // SAFETY: `c` holds four floats.
    unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
    check_site(Kind::Array, fb, tex, 0, sentinel(0), "layer 0 after a clear of layer 1");
    check_site(Kind::Array, fb, tex, 1, CLEAR_WORD, "layer 1 after its own clear");
}
