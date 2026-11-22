// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): glCopyTexImage2D / glCopyTexSubImage2D /
//! glCopyTexSubImage3D from a wide R or RG integer attachment (plan #383,
//! gap G2 of docs/notes/v3d-format-matrix.md).
//!
//! The twelve formats (R8I .. RG32UI) are stored widened to four channels as
//! textures and drawn as that wide store (`WIDE_INT_RT`,
//! `fb_target_channel_mask_n`, src/gl/framebuffer.rs). ES 3.0 3.8.5: a copy
//! from an integer colour buffer needs an integer image of the same signedness
//! (anything else is GL_INVALID_OPERATION), the image may not have a channel
//! the buffer lacks, and the image keeps only its own channels. The source is
//! attachment 0 as a 2D texture, as a 2D-array layer and as level 1.
//!
//! Held: the errors, which come out right today (an integer source into a
//! normalised, float or other-signedness image, into an image with a channel
//! the source lacks, and a normalised source into an integer image), for every
//! call. Pinned with `#[ignore]`, each naming its cause: the copy itself,
//! which `es3_copy_compatible` (src/gl/texture.rs) refuses for every integer
//! image and `texture_copy_from_fb` could not make anyway (it reads GL_RGBA /
//! GL_UNSIGNED_BYTE, an error on an integer buffer). The ignored cases check
//! the copied texels: the destination's channels from the source, 0, 0, 1 for
//! the rest (an R image from an RG source never receives G).
//!
//! What it cannot see: the GPU. The back end is a stand-in that stores a
//! clear in the target's memory; nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_copytex_int_rg
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
#![allow(dead_code)]
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
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

/// What the back end was handed by one call, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    color_mask: u8,
    /// The job state's (colormask, rt_colormask_own, rt_colormask) of a
    /// draw the V3D takes.
    job: Option<(u32, bool, u32)>,
    /// `state::refusal` / `state::unsupported`, or the job builder's reason.
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each draw; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout, little endian, rows tightly
/// packed (the stand-in's own layout: the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d)).or_else(|| {
        if d.index_count != 0 {
            state::job_state(d).err()
        } else {
            None
        }
    });
    let job = (d.index_count != 0)
        .then(|| state::job_state(d).ok())
        .flatten()
        .map(|st| (st.colormask, st.rt_colormask_own, st.rt_colormask));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
        job,
        refused,
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
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * t.w as usize + x) * bpp), bpp);
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
    name: c"es3-v3d-copytex-int-rg".as_ptr() as *const c_char,
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
    fn ty(&self) -> &'static str {
        if self.signed {
            "ivec4"
        } else {
            "uvec4"
        }
    }
    /// The read-back of a clear of 5, 6, 7, 8: the format's channels, then
    /// 0, 0, 1 for the absent ones (ES 3.0 4.3.2).
    fn expected(&self) -> [u32; 4] {
        if self.channels == 1 {
            [5, 0, 0, 1]
        } else {
            [5, 6, 0, 1]
        }
    }
}

/* ---- attachments, a program, a draw, a clear, a read-back ----------------- */

/// What carries the format at attachment 0.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Renderbuffer,
    Texture2d,
    /// Layer 1 of a 3-layer TEXTURE_2D_ARRAY.
    ArrayLayer,
    /// Level 1 of a 3-level, 8x8 TEXTURE_2D.
    Level1,
}

/// A bound framebuffer with the format at attachment 0, and whatever else
/// was attached; dropped, it unbinds and deletes them.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
    /// Width of the attached level (the read-back's size).
    w: GLsizei,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let (mut fb, mut name) = (0, 0);
    let mut f = Fbo { fb: 0, tex: vec![], rbs: vec![], w: 8 };
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, r.ifmt, 8, 8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name);
                f.rbs.push(name);
            }
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, levels, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, level);
                f.tex.push(name);
                f.w = 8 >> level;
            }
            Kind::ArrayLayer => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D_ARRAY, name);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, 0, 1);
                f.tex.push(name);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    f
}

/// glClearBuffer{i,ui}v of attachment 0 with 5, 6, 7, 8, then glReadPixels of
/// the `w` x `w` attachment as GL_RGBA_INTEGER: each pixel must be
/// [`Row::expected`].
fn clear_and_read_back(r: &Row, w: GLsizei, what: &str) {
    let v: [GLint; 4] = [5, 6, 7, 8];
    let mut px = vec![0u32; (w * w * 4) as usize];
    // SAFETY: four values; room for w x w pixels of four 32-bit values.
    unsafe {
        if r.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
        glReadPixels(
            0,
            0,
            w,
            w,
            GL_RGBA_INTEGER,
            if r.signed { GL_INT } else { GL_UNSIGNED_INT },
            px.as_mut_ptr() as *mut c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {what}: clear and read back", r.name);
    for (n, p) in px.chunks(4).enumerate() {
        assert_eq!(p, r.expected(), "{} {what}: pixel {n}", r.name);
    }
}

/* ---- the copy ------------------------------------------------------------- */

/// Which call copies.
#[derive(Clone, Copy, Debug)]
enum Api {
    /// glCopyTexSubImage2D into a TEXTURE_2D of the destination format.
    Sub2d,
    /// glCopyTexImage2D defining the destination (ES 3.0 3.8.5 takes sized integer names).
    Image2d,
    /// glCopyTexSubImage3D into layer 2 of a 3-layer TEXTURE_2D_ARRAY.
    Sub3dLayer2,
    /// glCopyTexSubImage2D into level 1 of a 3-level TEXTURE_2D.
    Sub2dLevel1,
}

const APIS: [Api; 4] = [Api::Sub2d, Api::Image2d, Api::Sub3dLayer2, Api::Sub2dLevel1];
const KINDS: [Kind; 3] = [Kind::Texture2d, Kind::ArrayLayer, Kind::Level1];

/// Where a copy went: the GL error it left, and the destination
/// (target, texture, level, layer, size) to read.
struct Dest {
    err: GLenum,
    target: GLenum,
    tex: GLuint,
    level: GLint,
    layer: GLint,
    w: GLsizei,
}

/// Copy the whole (w x w) source attachment into a fresh destination texture
/// of `dst_ifmt`.
fn copy_into(api: Api, dst_ifmt: GLenum, w: GLsizei) -> Dest {
    let mut t = 0;
    // SAFETY: plain GL calls; one name; the storage is the size the copy writes.
    let (target, level, layer) = unsafe {
        glGenTextures(1, &mut t);
        match api {
            Api::Sub2d => {
                glBindTexture(GL_TEXTURE_2D, t);
                glTexStorage2D(GL_TEXTURE_2D, 1, dst_ifmt, w, w);
                glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, w, w);
                (GL_TEXTURE_2D, 0, 0)
            }
            Api::Image2d => {
                glBindTexture(GL_TEXTURE_2D, t);
                glCopyTexImage2D(GL_TEXTURE_2D, 0, dst_ifmt, 0, 0, w, w, 0);
                (GL_TEXTURE_2D, 0, 0)
            }
            Api::Sub3dLayer2 => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, t);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, dst_ifmt, w, w, 3);
                glCopyTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, 2, 0, 0, w, w);
                (GL_TEXTURE_2D_ARRAY, 0, 2)
            }
            Api::Sub2dLevel1 => {
                glBindTexture(GL_TEXTURE_2D, t);
                glTexStorage2D(GL_TEXTURE_2D, 3, dst_ifmt, 2 * w, 2 * w);
                glCopyTexSubImage2D(GL_TEXTURE_2D, 1, 0, 0, 0, 0, w, w);
                (GL_TEXTURE_2D, 1, 0)
            }
        }
    };
    Dest { err: err(), target, tex: t, level, layer, w }
}

/// Read the destination back through a framebuffer of its own, delete it, then
/// restore `back` as the bound framebuffer.
fn read_dest(dst: &Row, d: Dest, back: GLuint) -> Vec<u32> {
    let mut px = vec![0u32; (d.w * d.w * 4) as usize];
    let mut fb = 0;
    // SAFETY: names this test made; room for w x w pixels of four values.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if d.target == GL_TEXTURE_2D_ARRAY {
            glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, d.tex, d.level, d.layer);
        } else {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, d.tex, d.level);
        }
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}: the destination", dst.name);
        glReadPixels(
            0,
            0,
            d.w,
            d.w,
            GL_RGBA_INTEGER,
            if dst.signed { GL_INT } else { GL_UNSIGNED_INT },
            px.as_mut_ptr() as *mut c_void,
        );
        glBindFramebuffer(GL_FRAMEBUFFER, back);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &d.tex);
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: reading the destination", dst.name);
    px
}

/// Clear the source (5, 6, 7, 8), copy it into a `dst` image with `api`, and
/// require the copy to be taken and the destination to hold the source's
/// channels `dst` has, 0, 0, 1 for the rest.
fn expect_copy(src: &Row, kind: Kind, api: Api, dst: &Row) {
    let f = fbo(src, kind);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", src.name);
    let v: [GLint; 4] = [5, 6, 7, 8];
    // SAFETY: four values.
    unsafe {
        if src.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
    }
    let what = format!("{} {kind:?} -> {} by {api:?}", src.name, dst.name);
    let d = copy_into(api, dst.ifmt, f.w);
    assert_eq!(d.err, GL_NO_ERROR, "{what}: the copy");
    let px = read_dest(dst, d, f.fb);
    for (n, p) in px.chunks(4).enumerate() {
        assert_eq!(p, dst.expected(), "{what}: pixel {n}");
    }
}

/// The destination rows a source copies into: its own format and, for an RG
/// source, the R format of the same width and signedness (the image has no
/// G, and the source's G must not leak into it).
fn destinations(i: usize) -> Vec<Row> {
    let mut v = vec![ROWS[i]];
    if ROWS[i].channels == 2 {
        v.push(ROWS[i - 2]);
    }
    v
}

fn run_copies(api: Api, kind: Kind) {
    let _g = context();
    for i in 0..ROWS.len() {
        for dst in destinations(i) {
            expect_copy(&ROWS[i], kind, api, &dst);
        }
    }
}

macro_rules! copy_tests {
    ($($name:ident: $api:expr, $kind:expr;)*) => {$(
        /// Source and destination of the same signedness: the copy is taken
        /// and the destination holds the format's channels, 0, 0, 1 else.
        #[test]
        #[ignore = "src/gl/texture.rs:6477 es3_copy_compatible is false for every Int/Uint image (glCopyTex* answers GL_INVALID_OPERATION), and texture_copy_from_fb (src/gl/texture.rs:2615) reads GL_RGBA/GL_UNSIGNED_BYTE, an error on an integer buffer"]
        fn $name() {
            run_copies($api, $kind);
        }
    )*};
}

copy_tests! {
    sub_image_2d_from_texture_2d: Api::Sub2d, Kind::Texture2d;
    sub_image_2d_from_array_layer: Api::Sub2d, Kind::ArrayLayer;
    sub_image_2d_from_level_1: Api::Sub2d, Kind::Level1;
    copy_tex_image_2d_from_texture_2d: Api::Image2d, Kind::Texture2d;
    copy_tex_image_2d_from_array_layer: Api::Image2d, Kind::ArrayLayer;
    copy_tex_image_2d_from_level_1: Api::Image2d, Kind::Level1;
    sub_image_3d_layer_from_texture_2d: Api::Sub3dLayer2, Kind::Texture2d;
    sub_image_3d_layer_from_array_layer: Api::Sub3dLayer2, Kind::ArrayLayer;
    sub_image_3d_layer_from_level_1: Api::Sub3dLayer2, Kind::Level1;
    sub_image_2d_level_1_from_texture_2d: Api::Sub2dLevel1, Kind::Texture2d;
    sub_image_2d_level_1_from_array_layer: Api::Sub2dLevel1, Kind::ArrayLayer;
    sub_image_2d_level_1_from_level_1: Api::Sub2dLevel1, Kind::Level1;
}

/* ---- the errors (ES 3.0 3.8.5) -------------------------------------------- */

/// A copy of the bound source into `dst_ifmt` that must be GL_INVALID_OPERATION, by every call.
fn expect_refused(src: &str, w: GLsizei, dst_ifmt: GLenum, what: &str) {
    for api in APIS {
        let d = copy_into(api, dst_ifmt, w);
        assert_eq!(d.err, GL_INVALID_OPERATION, "{src} -> {what} by {api:?}");
        // SAFETY: the name copy_into made.
        unsafe { glDeleteTextures(1, &d.tex) };
        while err() != GL_NO_ERROR {}
    }
}

/// An integer source into a normalised or float image.
#[test]
fn an_integer_source_into_a_normalised_or_float_image_is_invalid_operation() {
    let _g = context();
    for r in ROWS {
        for kind in KINDS {
            let f = fbo(&r, kind);
            let src = format!("{} {kind:?}", r.name);
            for (ifmt, n) in
                [(GL_RGBA8, "RGBA8"), (GL_R8, "R8"), (GL_RG8, "RG8"), (GL_RGBA16F, "RGBA16F"), (GL_R32F, "R32F")]
            {
                expect_refused(&src, f.w, ifmt, n);
            }
        }
    }
}

/// An integer source into an integer image of the other signedness.
#[test]
fn an_integer_source_into_the_other_signedness_is_invalid_operation() {
    let _g = context();
    for r in ROWS {
        for kind in KINDS {
            let f = fbo(&r, kind);
            let (ifmt, n) = if r.signed { (GL_R8UI, "R8UI") } else { (GL_R8I, "R8I") };
            expect_refused(&format!("{} {kind:?}", r.name), f.w, ifmt, n);
        }
    }
}

/// An image with a channel the source lacks.
#[test]
fn an_integer_image_with_a_channel_the_source_lacks_is_invalid_operation() {
    let _g = context();
    for r in ROWS {
        for kind in KINDS {
            let f = fbo(&r, kind);
            let src = format!("{} {kind:?}", r.name);
            let (ifmt, n) = if r.signed { (GL_RGBA8I, "RGBA8I") } else { (GL_RGBA8UI, "RGBA8UI") };
            expect_refused(&src, f.w, ifmt, n);
            if r.channels == 1 {
                let (ifmt, n) = if r.signed { (GL_RG8I, "RG8I") } else { (GL_RG8UI, "RG8UI") };
                expect_refused(&src, f.w, ifmt, n);
            }
        }
    }
}

/// A normalised source into an integer image.
#[test]
fn a_normalised_source_into_an_integer_image_is_invalid_operation() {
    let _g = context();
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: names this test makes and deletes.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    for r in ROWS {
        expect_refused("RGBA8", 8, r.ifmt, r.name);
    }
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}
