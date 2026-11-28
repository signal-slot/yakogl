// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): glBlitFramebuffer to and from a wide R or RG integer
//! texture (plan #383, gap G2 of docs/notes/v3d-format-matrix.md; dEQP-GLES3
//! fbo.color.repeated_clear.blit.tex2d.<format>).
//!
//! The twelve formats (R8I .. RG32UI) are stored widened to four channels as
//! textures and drawn as that wide store (`WIDE_INT_RT`,
//! `fb_target_channel_mask_n`, src/gl/framebuffer.rs); their renderbuffers
//! have their own narrow surface. ES 3.0.6 4.3.3: a NEAREST blit between
//! integer images of the same signedness is accepted and copies the values;
//! integer against non-integer, signed against unsigned, and GL_LINEAR with an
//! integer read buffer are GL_INVALID_OPERATION. The destination keeps only
//! its own channels (0, 0, 1 for the rest, an R image from an RG source never
//! receives G). Checked through glReadPixels(GL_RGBA_INTEGER) at attachment 0
//! as a 2D texture, a 2D-array layer, level 1 and a renderbuffer.
//!
//! What it cannot see: the GPU. The back end is a stand-in that stores a
//! clear in the target's memory; nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_blit_int_rg
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
/* ---- the blit -------------------------------------------------------------- */

const ALL: [Kind; 4] = [Kind::Renderbuffer, Kind::Texture2d, Kind::ArrayLayer, Kind::Level1];

/// Clear attachment 0 of the bound framebuffer to 5, 6, 7, 8.
fn clear5678(r: &Row) {
    let v: [GLint; 4] = [5, 6, 7, 8];
    // SAFETY: four values.
    unsafe {
        if r.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
    }
}

/// glBlitFramebuffer of the whole `sw` x `sw` read image into the `dw` x `dw`
/// draw image; the GL error. Leaves the draw framebuffer bound as read.
fn blit(src: &Fbo, dst: &Fbo, filter: GLenum) -> GLenum {
    // SAFETY: plain GL calls on framebuffers this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        let draw = [GL_COLOR_ATTACHMENT0];
        glDrawBuffers(1, draw.as_ptr());
        glBlitFramebuffer(0, 0, src.w, src.w, 0, 0, dst.w, dst.w, GL_COLOR_BUFFER_BIT, filter);
    }
    err()
}

/// glReadPixels(GL_RGBA_INTEGER) of `f`'s attachment 0, as 32-bit values.
fn read(r: &Row, f: &Fbo) -> Vec<u32> {
    let mut px = vec![0x5a5a_5a5au32; (f.w * f.w * 4) as usize];
    // SAFETY: room for w x w pixels of four 32-bit values.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, f.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(
            0,
            0,
            f.w,
            f.w,
            GL_RGBA_INTEGER,
            if r.signed { GL_INT } else { GL_UNSIGNED_INT },
            px.as_mut_ptr() as *mut c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: read back", r.name);
    px
}

/// Source cleared to 5, 6, 7, 8; the destination first cleared to 1, 2, 3, 4
/// (so a blit that did nothing shows), then blitted into.
fn expect_blit(src: &Row, sk: Kind, dst: &Row, dk: Kind) {
    let s = fbo(src, sk);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {sk:?}", src.name);
    clear5678(src);
    let d = fbo(dst, dk);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {dk:?}", dst.name);
    let v: [GLint; 4] = [1, 2, 3, 4];
    // SAFETY: four values; the destination is the bound framebuffer.
    unsafe {
        if dst.signed {
            glClearBufferiv(GL_COLOR, 0, v.as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
        }
    }
    let what = format!("{} {sk:?} -> {} {dk:?}", src.name, dst.name);
    assert_eq!(blit(&s, &d, GL_NEAREST), GL_NO_ERROR, "{what}: the blit");
    // The destination keeps its own channels; those the source lacks are
    // 0 for G and B (1 for A) when the destination has them: an R source into
    // an RG image gives G = 0.
    let mut want = dst.expected();
    if src.channels < dst.channels {
        want[1] = 0;
    }
    for (n, p) in read(dst, &d).chunks(4).enumerate() {
        assert_eq!(p, want, "{what}: pixel {n}");
    }
}

fn run(src_i: usize, dst_i: usize) {
    let _g = context();
    for sk in ALL {
        for dk in ALL {
            expect_blit(&ROWS[src_i], sk, &ROWS[dst_i], dk);
        }
    }
}

macro_rules! same_format {
    ($($name:ident: $i:expr,)*) => {$(
        #[test]
        fn $name() {
            run($i, $i);
        }
    )*};
}

same_format! {
    r8i_blits_to_r8i: 0,
    r8ui_blits_to_r8ui: 1,
    rg8i_blits_to_rg8i: 2,
    rg8ui_blits_to_rg8ui: 3,
    r16i_blits_to_r16i: 4,
    r16ui_blits_to_r16ui: 5,
    rg16i_blits_to_rg16i: 6,
    rg16ui_blits_to_rg16ui: 7,
    r32i_blits_to_r32i: 8,
    r32ui_blits_to_r32ui: 9,
    rg32i_blits_to_rg32i: 10,
    rg32ui_blits_to_rg32ui: 11,
}

/// An RG image into the R image of the same signedness (any width) and back:
/// the destination takes its own channels only.
#[test]
fn rg_into_r_and_r_into_rg_of_the_same_signedness() {
    let _g = context();
    for (rg, r) in [(2, 0), (3, 1), (6, 4), (7, 5), (10, 8), (11, 9)] {
        for (a, b) in [(rg, r), (r, rg)] {
            expect_blit(&ROWS[a], Kind::Texture2d, &ROWS[b], Kind::Texture2d);
            expect_blit(&ROWS[a], Kind::Renderbuffer, &ROWS[b], Kind::Level1);
        }
    }
}

/// An uploaded image blits with its values and a scaled NEAREST blit takes
/// the nearest texel: a 2x2 R32I texture doubled to 4x4.
#[test]
fn an_uploaded_r32i_image_is_scaled_by_nearest() {
    let _g = context();
    let r = &ROWS[8];
    let px: [i32; 4] = [-1, 2, i32::MIN, i32::MAX];
    let mut t = 0;
    // SAFETY: one name; four 32-bit texels.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_R32I as GLint, 2, 2, 0, GL_RED_INTEGER, GL_INT, px.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "upload");
    let mut s = Fbo { fb: 0, tex: vec![t], rbs: vec![], w: 2 };
    // SAFETY: names this test made.
    unsafe {
        glGenFramebuffers(1, &mut s.fb);
        glBindFramebuffer(GL_FRAMEBUFFER, s.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    let mut d = fbo(r, Kind::Renderbuffer);
    d.w = 8;
    assert_eq!(blit(&s, &d, GL_NEAREST), GL_NO_ERROR);
    let got = read(r, &d);
    for y in 0..8usize {
        for x in 0..8usize {
            let want = px[(y / 4) * 2 + x / 4];
            assert_eq!(&got[(y * 8 + x) * 4..][..4], [want as u32, 0, 0, 1], "pixel {x},{y}");
        }
    }
}

/// A texture of one signedness (or a non-integer format) against another.
#[test]
fn integer_against_non_integer_or_the_other_signedness_is_invalid_operation() {
    let _g = context();
    for kind in [Kind::Texture2d, Kind::Renderbuffer] {
        for (i, r) in ROWS.iter().enumerate() {
            let other = &ROWS[i ^ 1]; // the same format, the other signedness
            let a = fbo(r, kind);
            let b = fbo(other, kind);
            assert_eq!(blit(&a, &b, GL_NEAREST), GL_INVALID_OPERATION, "{} -> {}", r.name, other.name);
            assert_eq!(blit(&b, &a, GL_NEAREST), GL_INVALID_OPERATION, "{} -> {}", other.name, r.name);
        }
    }
    // An RGBA8 texture on either side.
    let mut t = 0;
    // SAFETY: one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 8, 8);
    }
    let mut n = Fbo { fb: 0, tex: vec![t], rbs: vec![], w: 8 };
    // SAFETY: names this test made.
    unsafe {
        glGenFramebuffers(1, &mut n.fb);
        glBindFramebuffer(GL_FRAMEBUFFER, n.fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "RGBA8");
    for r in &ROWS {
        let i = fbo(r, Kind::Texture2d);
        assert_eq!(blit(&i, &n, GL_NEAREST), GL_INVALID_OPERATION, "{} -> RGBA8", r.name);
        assert_eq!(blit(&n, &i, GL_NEAREST), GL_INVALID_OPERATION, "RGBA8 -> {}", r.name);
    }
}

/// GL_LINEAR with an integer read buffer is an error, whatever carries it.
#[test]
fn linear_with_an_integer_read_buffer_is_invalid_operation() {
    let _g = context();
    for kind in ALL {
        for r in &ROWS {
            let a = fbo(r, kind);
            let b = fbo(r, Kind::Texture2d);
            assert_eq!(blit(&a, &b, GL_LINEAR), GL_INVALID_OPERATION, "{} {kind:?}", r.name);
        }
    }
}
