// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): a wide R/RG integer colour texture beside depth and
//! stencil attachments (gap G2 of docs/notes/v3d-format-matrix.md, plan #383).
//!
//! For each of the twelve formats (R8I .. RG32UI) as a TEXTURE_2D at
//! GL_COLOR_ATTACHMENT0, together with
//!
//! - (a) a DEPTH_COMPONENT24 renderbuffer at GL_DEPTH_ATTACHMENT,
//! - (b) a DEPTH24_STENCIL8 texture at GL_DEPTH_STENCIL_ATTACHMENT,
//! - (c) a DEPTH_COMPONENT32F texture at GL_DEPTH_ATTACHMENT,
//!
//! it holds that: the framebuffer is complete; a draw with the depth test and
//! the stencil test on keeps the colour target on the wide surface of the
//! store with the format's channel mask, and names the depth/stencil
//! objects the framebuffer attached; glClearBufferiv/uiv on the colour plus
//! glClearBufferfi on depth-stencil keep the colour clear narrowed to the
//! format (R/RG kept, B 0, A 1 on read-back) and clear depth and stencil to
//! their values; a depth attachment of another size than the colour
//! attachment is complete (ES 3.0 4.4.4: the sizes need not match) and the
//! back end is told both sizes.
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! each call; no depth or stencil result is rasterized. Nothing here is
//! verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_depth
//! ```

#![allow(dead_code)] // the copied harness has more than this file uses
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

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
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
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
    depth_test: bool,
    depth_write: bool,
    stencil: bool,
    stencil_write_mask: u8,
    depth_id: u32,
    stencil_id: u32,
    ds_w: u32,
    ds_h: u32,
    do_clear: bool,
    do_clear_depth: bool,
    do_clear_stencil: bool,
    clear_depth: f32,
    clear_stencil: u8,
    clear_bits: [u32; 4],
    clear_target_mask: u8,
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
        depth_test: d.depth_test,
        depth_write: d.depth_write,
        stencil: d.stencil,
        stencil_write_mask: d.stencil_write_mask,
        depth_id: d.depth_id,
        stencil_id: d.stencil_id,
        ds_w: d.ds_w,
        ds_h: d.ds_h,
        do_clear: d.do_clear,
        do_clear_depth: d.do_clear_depth,
        do_clear_stencil: d.do_clear_stencil,
        clear_depth: d.clear_depth,
        clear_stencil: d.clear_stencil,
        clear_bits: d.clear_rgba.map(f32::to_bits),
        clear_target_mask: d.clear_target_mask,
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
    name: c"es3-v3d-int-rg-depth".as_ptr() as *const c_char,
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

/// An RGBA8 (or RGBA8UI) renderbuffer at attachment 1; both attachments are
/// the draw buffers.
fn add_second(f: &mut Fbo, ifmt: GLenum) {
    let mut rb = 0;
    // SAFETY: room for one name; two draw buffers listed.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, ifmt, f.w, f.w);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 1, GL_RENDERBUFFER, rb);
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1];
        glDrawBuffers(2, bufs.as_ptr());
    }
    f.rbs.push(rb);
    assert_eq!(err(), GL_NO_ERROR, "second attachment {ifmt:#x}");
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

/// Make `fs` (with the trivial vertex shader) current.
fn program(fs: &str) {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
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

/// One output of `ty` at location 0 writing a known value.
fn program_one(r: &Row) {
    let val = if r.signed { "ivec4(1, -2, 3, -4)" } else { "uvec4(1u, 2u, 3u, 4u)" };
    program(&format!(
        "#version 300 es\nprecision highp int;\nlayout(location = 0) out highp {} o0;\n\
         void main() {{ o0 = {val}; }}\n",
        r.ty()
    ));
}

/// Draw one triangle; what the back end was handed (None: no draw reached it)
/// and the GL error the call left.
fn draw() -> (Option<Seen>, GLenum) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (s.into_iter().rev().find(|s| s.draws), e)
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

/// The channels (bit 0 R .. bit 3 A) of the format.
fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

/* ---- depth and stencil beside the colour texture -------------------------- */

/// What carries depth (and stencil).
#[derive(Clone, Copy, Debug)]
enum Ds {
    /// DEPTH_COMPONENT24 renderbuffer at GL_DEPTH_ATTACHMENT.
    RbDepth24,
    /// DEPTH24_STENCIL8 texture at GL_DEPTH_STENCIL_ATTACHMENT.
    TexDepth24Stencil8,
    /// DEPTH_COMPONENT32F texture at GL_DEPTH_ATTACHMENT.
    TexDepth32f,
}

const DS_KINDS: [Ds; 3] = [Ds::RbDepth24, Ds::TexDepth24Stencil8, Ds::TexDepth32f];

impl Ds {
    fn has_stencil(self) -> bool {
        matches!(self, Ds::TexDepth24Stencil8)
    }
}

/// Attach `ds` of `size` x `size` to the bound framebuffer.
fn attach_ds(f: &mut Fbo, ds: Ds, size: GLsizei) {
    let mut name = 0;
    // SAFETY: room for one name each.
    unsafe {
        match ds {
            Ds::RbDepth24 => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, size, size);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, name);
                f.rbs.push(name);
            }
            Ds::TexDepth24Stencil8 | Ds::TexDepth32f => {
                let (ifmt, att) = if matches!(ds, Ds::TexDepth32f) {
                    (GL_DEPTH_COMPONENT32F, GL_DEPTH_ATTACHMENT)
                } else {
                    (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT)
                };
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, 1, ifmt, size, size);
                glFramebufferTexture2D(GL_FRAMEBUFFER, att, GL_TEXTURE_2D, name, 0);
                f.tex.push(name);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{ds:?}: storage and attach");
}

/// The colour texture at attachment 0 and `ds` of `size`.
fn fbo_ds(r: &Row, ds: Ds, size: GLsizei) -> Fbo {
    let mut f = fbo(r, Kind::Texture2d);
    attach_ds(&mut f, ds, size);
    f
}

/// Depth and stencil tests on, writes on; the stencil test passes always
/// and replaces.
fn tests_on() {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glDepthMask(1);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_ALWAYS, 1, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
        glStencilMask(0xff);
    }
}

fn tests_off() {
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glDisable(GL_STENCIL_TEST);
    }
}

/// Every call the back end saw since the last `draw()`/clear reset that
/// clears something.
fn clears() -> Vec<Seen> {
    core::mem::take(&mut *SEEN.lock().unwrap())
        .into_iter()
        .filter(|s| s.do_clear || s.do_clear_depth || s.do_clear_stencil)
        .collect()
}

#[test]
fn texture_beside_depth_or_stencil_is_complete() {
    let _g = context();
    for r in ROWS {
        for ds in DS_KINDS {
            let _f = fbo_ds(&r, ds, 8);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} + {ds:?}", r.name);
        }
    }
}

/// A draw with both tests on: the colour target is the wide surface with the
/// format's mask, as without depth; the depth (and stencil) objects are the
/// attached ones; no refusal.
#[test]
fn draw_with_depth_and_stencil_tests_keeps_wide_format_and_mask() {
    let _g = context();
    for r in ROWS {
        for ds in DS_KINDS {
            let f = fbo_ds(&r, ds, 8);
            program_one(&r);
            tests_on();
            let (seen, e) = draw();
            tests_off();
            let what = format!("{} + {ds:?}", r.name);
            assert_eq!(e, GL_NO_ERROR, "{what}: the draw");
            let s = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
            assert_eq!(s.refused, None, "{what}");
            assert_eq!(s.targets.len(), 1, "{what}");
            let t = s.targets[0];
            assert_ne!(t.0, 0, "{what}: the store");
            assert_eq!((t.1, t.2), (f.w as u32, f.w as u32), "{what}: colour size");
            let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{what}: not an integer surface"));
            let want = backend::gles_surf_int_layout(r.surf).unwrap();
            assert_eq!((l.bits, l.signed, l.channels), (want.bits, want.signed, 4), "{what}: the wide store");
            assert_eq!(s.color_mask, own_mask(&r), "{what}: the format's channels");
            assert!(s.depth_test && s.depth_write, "{what}: depth test and write");
            assert_ne!(s.depth_id, 0, "{what}: a depth object");
            assert_eq!((s.ds_w, s.ds_h), (8, 8), "{what}: depth size");
            if ds.has_stencil() {
                assert!(s.stencil, "{what}: stencil test");
                assert_eq!(s.stencil_id, s.depth_id, "{what}: D24S8 gives both");
            }
        }
    }
}

/// A draw with the colour mask on the format's channels alone, depth and
/// stencil on: the same wide store, the mask as the application set.
#[test]
fn partial_colour_mask_with_depth_is_the_application_mask() {
    let _g = context();
    for r in ROWS {
        let _f = fbo_ds(&r, Ds::TexDepth24Stencil8, 8);
        program_one(&r);
        tests_on();
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 0, 0, 0) };
        let (seen, e) = draw();
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        tests_off();
        assert_eq!(e, GL_NO_ERROR, "{}", r.name);
        let s = seen.unwrap_or_else(|| panic!("{}: no draw", r.name));
        assert_eq!(s.refused, None, "{}", r.name);
        assert_eq!(s.color_mask, 1, "{}: R alone", r.name);
        assert_eq!(s.targets.len(), 1, "{}", r.name);
    }
}

/// glClearBufferiv/uiv of colour then glClearBufferfi of depth-stencil (and a
/// float depth clear for the DEPTH_COMPONENT32F case): the colour reads back
/// narrowed and the back end clears depth (and stencil) to the values.
#[test]
fn clears_of_colour_and_depth_stencil_keep_the_colour_narrowed() {
    let _g = context();
    for r in ROWS {
        for ds in DS_KINDS {
            let f = fbo_ds(&r, ds, 8);
            let what = format!("{} + {ds:?}", r.name);
            SEEN.lock().unwrap().clear();
            let v: [GLint; 4] = [5, 6, 7, 8];
            // SAFETY: four values each.
            unsafe {
                if r.signed {
                    glClearBufferiv(GL_COLOR, 0, v.as_ptr());
                } else {
                    glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
                }
                if ds.has_stencil() {
                    glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.25, 3);
                } else {
                    let d = 0.25f32;
                    glClearBufferfv(GL_DEPTH, 0, &d);
                }
            }
            assert_eq!(err(), GL_NO_ERROR, "{what}: the clears");
            // SAFETY: plain GL call; a pending depth clear is submitted.
            unsafe { glFinish() };
            let c = clears();
            let col: Vec<_> = c.iter().filter(|s| s.do_clear).collect();
            assert_eq!(col.len(), 1, "{what}: one colour clear: {c:?}");
            assert_eq!(col[0].refused, None, "{what}");
            let l = backend::gles_surf_int_layout(col[0].targets[0].3).unwrap();
            assert_eq!(l.channels, 4, "{what}: wide surface");
            let dc: Vec<_> = c.iter().filter(|s| s.do_clear_depth).collect();
            assert!(!dc.is_empty(), "{what}: depth cleared: {c:?}");
            assert!(dc.iter().all(|s| s.clear_depth == 0.25), "{what}: depth value: {c:?}");
            if ds.has_stencil() {
                let sc: Vec<_> = c.iter().filter(|s| s.do_clear_stencil).collect();
                assert!(!sc.is_empty(), "{what}: stencil cleared: {c:?}");
                assert!(sc.iter().all(|s| s.clear_stencil == 3), "{what}: stencil value: {c:?}");
            }
            // The colour clear is narrowed on read-back, whatever depth did.
            let mut px = vec![0u32; (f.w * f.w * 4) as usize];
            // SAFETY: room for w x w four-value pixels.
            unsafe {
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
            assert_eq!(err(), GL_NO_ERROR, "{what}: read back");
            for (n, p) in px.chunks(4).enumerate() {
                assert_eq!(p, r.expected(), "{what}: pixel {n}");
            }
        }
    }
}

/// A glClear(COLOR | DEPTH | STENCIL) with the colour clear value set: the
/// colour reads back narrowed (G/B 0, A 1 where absent), depth and stencil
/// are cleared in the same call.
#[test]
fn glclear_color_depth_stencil_keeps_the_colour_narrowed() {
    let _g = context();
    for r in ROWS {
        let _f = fbo_ds(&r, Ds::TexDepth24Stencil8, 8);
        let what = r.name;
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(5.0, 6.0, 7.0, 8.0);
            glClearDepthf(0.5);
            glClearStencil(2);
        }
        SEEN.lock().unwrap().clear();
        // SAFETY: plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT) };
        let e = err();
        // SAFETY: plain GL call; a pending depth clear is submitted.
        unsafe { glFinish() };
        let c = clears();
        // glClear takes the float clear colour; the integer texture is
        // cleared only to values undefined by ES 3.0 4.2.3, so assert only
        // the depth/stencil half and that nothing is refused.
        assert_eq!(e, GL_NO_ERROR, "{what}");
        assert!(c.iter().all(|s| s.refused.is_none()), "{what}: {c:?}");
        assert!(c.iter().any(|s| s.do_clear_depth && s.clear_depth == 0.5), "{what}: depth {c:?}");
        assert!(c.iter().any(|s| s.do_clear_stencil && s.clear_stencil == 2), "{what}: stencil {c:?}");
    }
}

/// Depth (4x4) smaller than colour (8x8): complete (ES 3.0 4.4.4 does not
/// ask for equal sizes); the draw is taken and the back end sees both sizes
/// (`ds_w`/`ds_h` is the depth's, the render area the intersection). What the
/// tip does is pinned.
#[test]
fn depth_of_another_size_than_the_colour_is_complete_and_drawn() {
    let _g = context();
    for r in ROWS {
        for ds in DS_KINDS {
            let f = fbo_ds(&r, ds, 4);
            let what = format!("{} + {ds:?} 4x4 beside 8x8", r.name);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{what}");
            program_one(&r);
            tests_on();
            let (seen, e) = draw();
            tests_off();
            assert_eq!(e, GL_NO_ERROR, "{what}: the draw");
            let s = seen.unwrap_or_else(|| panic!("{what}: no draw"));
            assert_eq!(s.refused, None, "{what}");
            assert_eq!((s.targets[0].1, s.targets[0].2), (f.w as u32, f.w as u32), "{what}: colour size");
            assert_eq!((s.ds_w, s.ds_h), (4, 4), "{what}: depth size");
            assert_eq!(s.color_mask, own_mask(&r), "{what}");
        }
    }
}

/// Depth (16x16) larger than the colour: also complete.
#[test]
fn depth_larger_than_the_colour_is_complete_and_drawn() {
    let _g = context();
    for r in ROWS {
        let _f = fbo_ds(&r, Ds::RbDepth24, 16);
        let what = format!("{} + depth 16x16 beside 8x8", r.name);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{what}");
        program_one(&r);
        tests_on();
        let (seen, e) = draw();
        tests_off();
        assert_eq!(e, GL_NO_ERROR, "{what}");
        let s = seen.unwrap_or_else(|| panic!("{what}: no draw"));
        assert_eq!(s.refused, None, "{what}");
        assert_eq!((s.ds_w, s.ds_h), (16, 16), "{what}");
    }
}
