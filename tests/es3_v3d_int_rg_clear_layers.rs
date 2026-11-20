// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): clears of R and RG integer cube faces and 3D slices
//! (gap G2 of docs/notes/v3d-format-matrix.md, plan #383; the layer half of
//! tests/es3_v3d_int_rg_clear.rs, which covers attachment 0, a 2D-array layer
//! and level 1, and the clear counterpart of
//! tests/es3_v3d_fbo_int_rg_layers.rs).
//!
//! An R or RG integer texture is held in and drawn as the four-channel store
//! of the same type and width; the GL core narrows a clear so the store's
//! absent channels stay G/B = 0 and A = 1 (`draw.rs` `int_clear_channels`).
//! Here the attachment is one face of a cube map (POSITIVE_X, NEGATIVE_Z) or
//! slice 2 of a 4-deep 3D texture, for all twelve formats (8, 16 and 32 bit,
//! signed and unsigned, R and RG):
//!
//! - glClearBufferiv/uiv {5, 6, 7, 8}: the back end is handed the clear on the
//!   store's surface with clear words whose absent channels are 0 (G, B) and
//!   1 (A), and glReadPixels(GL_RGBA_INTEGER) of the attachment returns the
//!   format's channels as cleared and 0, 0, 1 for the others;
//! - glClear (the float clear colour is undefined for an integer buffer, its
//!   B and A still must not reach the absent channels): the clear words are
//!   narrowed, the read-back shows 0, 0, 1 in the absent channels;
//! - the other faces and slices of the same texture are not written: each is
//!   attached in turn and reads back as it did before (all zero).
//!
//! What the stand-in sees, and what it cannot: as in tests/es3_v3d_int_rg_clear.rs,
//! it records each clear as the `GlesDraw` the back end is handed and writes
//! the clear in target 0's layout; it does not see the store's hidden
//! channels, so the narrowing is checked on the clear words
//! (`state::clear_words`). A 3D slice is rendered at the grid's row pitch, so
//! the stand-in strides rows by `backend::draw_pitch(d)`. The GPU is not
//! involved; nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_clear_layers
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::state;

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

/// One clear as the back end was handed it, and what the V3D made of it.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Seen {
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    do_clear: bool,
    /// The four raw values, as bit patterns (integer targets).
    clear_bits: [u32; 4],
    clear_color_mask: u8,
    clear_quad: bool,
    clear_target_mask: u8,
    /// `state::colour_needs_quad`: the quad path rather than the TLB clear.
    needs_quad: bool,
    /// `state::refusal` / `state::unsupported` (a clear has no job state).
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each clear; one of an integer target also stores the low bits of
/// the four raw values in target 0's layout, little endian, rows tightly
/// packed (the stand-in's own layout: the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        do_clear: d.do_clear,
        clear_bits: d.clear_rgba.map(f32::to_bits),
        clear_color_mask: d.clear_color_mask,
        clear_quad: d.clear_quad,
        clear_target_mask: d.clear_target_mask,
        needs_quad: state::colour_needs_quad(d),
        refused: state::refusal(d).or_else(|| state::unsupported(d)),
    });
    let t = &d.targets[0];
    let Some(l) = backend::gles_surf_int_layout(t.format) else { return 0 };
    if !d.do_clear || t.gpu == 0 {
        return 0;
    }
    let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
    let v = d.clear_rgba.map(f32::to_bits);
    let step = l.bits as usize / 8;
    let mut px = vec![0u8; bpp];
    for ch in 0..l.channels as usize {
        px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
    }
    let pitch = backend::draw_pitch(d) as usize;
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * pitch + x) * bpp), bpp);
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
    name: c"es3-v3d-int-rg-clear-layers".as_ptr() as *const c_char,
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
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glDisable(GL_SCISSOR_TEST);
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

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/* ---- the twelve formats -------------------------------------------------- */

/// One of the twelve R and RG integer formats.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
    /// The format's own surface (the renderbuffer's).
    surf: u8,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool, surf: u8) -> Row {
    Row { name, ifmt, channels, signed, surf }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true, GLES_SURF_R8I),
    row("R8UI", GL_R8UI, 1, false, GLES_SURF_R8UI),
    row("RG8I", GL_RG8I, 2, true, GLES_SURF_RG8I),
    row("RG8UI", GL_RG8UI, 2, false, GLES_SURF_RG8UI),
    row("R16I", GL_R16I, 1, true, GLES_SURF_R16I),
    row("R16UI", GL_R16UI, 1, false, GLES_SURF_R16UI),
    row("RG16I", GL_RG16I, 2, true, GLES_SURF_RG16I),
    row("RG16UI", GL_RG16UI, 2, false, GLES_SURF_RG16UI),
    row("R32I", GL_R32I, 1, true, GLES_SURF_R32I),
    row("R32UI", GL_R32UI, 1, false, GLES_SURF_R32UI),
    row("RG32I", GL_RG32I, 2, true, GLES_SURF_RG32I),
    row("RG32UI", GL_RG32UI, 2, false, GLES_SURF_RG32UI),
];

impl Row {
    /// The read-back of a clear of 5, 6, 7, 8: the format's channels, then
    /// 0, 0, 1 for the absent ones (ES 3.0 4.3.2).
    fn expected(&self) -> [u32; 4] {
        if self.channels == 1 {
            [5, 0, 0, 1]
        } else {
            [5, 6, 0, 1]
        }
    }
    /// Bit k: the format has channel k (R, G).
    fn channel_mask(&self) -> u8 {
        if self.channels == 1 {
            0b0001
        } else {
            0b0011
        }
    }
}

/// `raw` with the channels the format lacks as the store must hold them:
/// 0 in G (R formats) and B, 1 in A. Bit patterns, as `clear_rgba`'s.
fn narrowed_bits(r: &Row, raw: [u32; 4]) -> [u32; 4] {
    let mut v = raw;
    for (ch, w) in v.iter_mut().enumerate() {
        if ch >= r.channels as usize {
            *w = u32::from(ch == 3);
        }
    }
    v
}

/// The clear words of `format` for raw clear values `bits`
/// (`state::clear_words`).
fn words(format: u8, bits: [u32; 4]) -> [u32; 4] {
    state::clear_words(format, bits.map(f32::from_bits))
}

/// Does this clear keep the application's B and A out of the store's absent
/// channels? Either the clear words of the target's own format are those of
/// the narrowed values (0, 0, 1 in the absent channels), or the clear's mask
/// leaves the absent channels alone (quad path only).
fn hidden_channels_clean(r: &Row, s: &Seen, t_format: u8) -> bool {
    let narrowed = words(t_format, s.clear_bits) == words(t_format, narrowed_bits(r, s.clear_bits));
    let masked_out = s.needs_quad && s.clear_color_mask & 0xf & !r.channel_mask() == 0 && s.clear_color_mask != 0;
    narrowed || masked_out
}

/* ---- a cube map or a 3D texture, one layer attached at a time ------------- */

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];
const SLICES: i32 = 4;

/// What the layers are made of.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// An 8x8 cube map; the layer is the face's index in [`FACES`].
    Cube,
    /// An 8x8x4 TEXTURE_3D; the layer is the slice.
    Volume3d,
}

impl Shape {
    fn layers(self) -> usize {
        match self {
            Shape::Cube => FACES.len(),
            Shape::Volume3d => SLICES as usize,
        }
    }
}

/// A texture of `shape` and a framebuffer; dropped, it unbinds and deletes them.
struct Layers {
    shape: Shape,
    fb: GLuint,
    tex: GLuint,
}

impl Drop for Layers {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
        }
    }
}

fn layers(r: &Row, shape: Shape) -> Layers {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenTextures(1, &mut tex);
        match shape {
            Shape::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, r.ifmt, 8, 8);
            }
            Shape::Volume3d => {
                glBindTexture(GL_TEXTURE_3D, tex);
                glTexStorage3D(GL_TEXTURE_3D, 1, r.ifmt, 8, 8, SLICES);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {shape:?}: storage", r.name);
    Layers { shape, fb, tex }
}

impl Layers {
    /// Attach layer `i` at GL_COLOR_ATTACHMENT0 of the bound framebuffer.
    fn attach(&self, r: &Row, i: usize) {
        // SAFETY: the face and slice named are there.
        unsafe {
            match self.shape {
                Shape::Cube => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, FACES[i], self.tex, 0),
                Shape::Volume3d => {
                    glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, self.tex, 0, i as GLint)
                }
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{} {:?} layer {i}: attach", r.name, self.shape);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {:?} layer {i}", r.name, self.shape);
    }
}

/// The 8x8 attachment read as GL_RGBA_INTEGER.
fn read_px(r: &Row, what: &str) -> Vec<u32> {
    let mut px = vec![0u32; 8 * 8 * 4];
    // SAFETY: room for 8 x 8 pixels of four 32-bit values.
    unsafe {
        glReadPixels(
            0,
            0,
            8,
            8,
            GL_RGBA_INTEGER,
            if r.signed { GL_INT } else { GL_UNSIGNED_INT },
            px.as_mut_ptr() as *mut c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {what}: read back", r.name);
    px
}

/// Every layer but `except` reads back as an untouched one: zero in the
/// format's channels, 0, 0, 1 in the absent ones.
fn others_untouched(r: &Row, l: &Layers, except: usize, what: &str) {
    for i in (0..l.shape.layers()).filter(|&i| i != except) {
        l.attach(r, i);
        for (n, p) in read_px(r, what).chunks(4).enumerate() {
            assert_eq!(p, [0, 0, 0, 1], "{} {what}: layer {i} (untouched), pixel {n}", r.name);
        }
    }
}

/// The layers each shape is cleared at: the first and last face, slice 2.
fn targets(shape: Shape) -> Vec<usize> {
    match shape {
        Shape::Cube => vec![0, 5],
        Shape::Volume3d => vec![2],
    }
}

/// The back end's single clear is on an integer store of at least the
/// format's channels, not refused, and its clear words are those of the
/// narrowed values.
fn check_clear(r: &Row, shape: Shape, i: usize, c: &[Seen]) {
    assert_eq!(c.len(), 1, "{} {shape:?} layer {i}: one clear", r.name);
    let s = &c[0];
    assert_eq!(s.refused, None, "{} {shape:?} layer {i}", r.name);
    let t = s.targets[0];
    assert_eq!((t.1, t.2), (8, 8), "{} {shape:?} layer {i}: the layer's size", r.name);
    let l =
        backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{} {shape:?}: not an integer surface", r.name));
    assert!(l.channels >= r.channels, "{} {shape:?}: the store holds the format's channels", r.name);
    assert!(
        hidden_channels_clean(r, s, t.3),
        "{} {shape:?} layer {i}: clear words {:x?} carry the application's B/A (want {:x?}) in the hidden channels of {}",
        r.name,
        words(t.3, s.clear_bits),
        words(t.3, narrowed_bits(r, s.clear_bits)),
        gles_surf_name(t.3)
    );
}

/// glClearBuffer{i,ui}v {5, 6, 7, 8} of each target layer of `shape`.
fn clear_buffer_case(shape: Shape) {
    let _g = context();
    for r in ROWS {
        for i in targets(shape) {
            let l = layers(&r, shape);
            l.attach(&r, i);
            SEEN.lock().unwrap().clear();
            let v: [GLint; 4] = [5, 6, 7, 8];
            // SAFETY: four values.
            unsafe {
                if r.signed {
                    glClearBufferiv(GL_COLOR, 0, v.as_ptr());
                } else {
                    glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
                }
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {shape:?} layer {i}: the clear", r.name);
            let c = clears();
            check_clear(&r, shape, i, &c);
            assert_eq!(c[0].clear_bits[..r.channels as usize], [5, 6][..r.channels as usize]);
            assert_eq!(c[0].clear_target_mask & 1, 1, "{} {shape:?} layer {i}: draw buffer 0", r.name);
            l.attach(&r, i);
            for (n, p) in read_px(&r, "cleared layer").chunks(4).enumerate() {
                assert_eq!(p, r.expected(), "{} {shape:?} layer {i}: pixel {n}", r.name);
            }
            others_untouched(&r, &l, i, &format!("{shape:?} cleared layer {i}"));
        }
    }
}

/// glClear of each target layer of `shape`: the float colour is undefined for
/// an integer buffer, the absent channels are not.
fn glclear_case(shape: Shape) {
    let _g = context();
    for r in ROWS {
        for i in targets(shape) {
            let l = layers(&r, shape);
            l.attach(&r, i);
            SEEN.lock().unwrap().clear();
            // SAFETY: plain GL calls.
            unsafe {
                glClearColor(0.25, 0.5, 0.75, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {shape:?} layer {i}: glClear", r.name);
            let c = clears();
            check_clear(&r, shape, i, &c);
            l.attach(&r, i);
            let ch = r.channels as usize;
            for (n, p) in read_px(&r, "cleared layer").chunks(4).enumerate() {
                assert_eq!(p[ch..], r.expected()[ch..], "{} {shape:?} layer {i}: pixel {n}'s absent channels", r.name);
            }
            others_untouched(&r, &l, i, &format!("{shape:?} glClear of layer {i}"));
        }
    }
}

/// Cube faces POSITIVE_X and NEGATIVE_Z (fbo.color.texcube.*).
#[test]
fn cube_face_clearbuffer_keeps_0_0_1_in_the_absent_channels_and_other_faces_untouched() {
    clear_buffer_case(Shape::Cube);
}

/// Slice 2 of a 4-slice TEXTURE_3D (fbo.color.tex3d.*).
#[test]
fn texture_3d_slice_clearbuffer_keeps_0_0_1_in_the_absent_channels_and_other_slices_untouched() {
    clear_buffer_case(Shape::Volume3d);
}

#[test]
fn cube_face_glclear_keeps_0_0_1_in_the_absent_channels_and_other_faces_untouched() {
    glclear_case(Shape::Cube);
}

#[test]
fn texture_3d_slice_glclear_keeps_0_0_1_in_the_absent_channels_and_other_slices_untouched() {
    glclear_case(Shape::Volume3d);
}

/// The clears the back end saw since the last call (a clear is sent at
/// glFinish at the latest).
fn clears() -> Vec<Seen> {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    core::mem::take(&mut *SEEN.lock().unwrap()).into_iter().filter(|s| s.do_clear).collect()
}
