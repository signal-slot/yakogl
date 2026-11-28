// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): an R or RG integer texture drawn as its wide store
//! behind glDrawBuffers gaps (attachment 2 or 3 with NONE before it), with the
//! format's channel mask, per-RT masks beside other targets and changing lists
//! (docs/notes/v3d-format-matrix.md G2 and G3; `fb_target_n`,
//! `fb_target_channel_mask_n` in src/gl/framebuffer.rs, `per_target_masks` in
//! src/gl/draw.rs).
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! what `state::refusal`, `state::unsupported` and `state::job_state` make of
//! each draw; nothing is rasterized and nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_drawbuffers
//! ```

#![allow(dead_code)]
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
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
    name: c"es3-v3d-int-rg-drawbuffers".as_ptr() as *const c_char,
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

/// A texture of format `r` (TEXTURE_2D, 8x8) at `att`.
fn attach_tex(f: &mut Fbo, ifmt: GLenum, att: GLenum) {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexStorage2D(GL_TEXTURE_2D, 1, ifmt, 8, 8);
        glFramebufferTexture2D(GL_FRAMEBUFFER, att, GL_TEXTURE_2D, name, 0);
    }
    f.tex.push(name);
    assert_eq!(err(), GL_NO_ERROR, "attach {ifmt:#x} at {att:#x}");
}

fn fbo_empty() -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    Fbo { fb, tex: vec![], rbs: vec![], w: 8 }
}

fn draw_buffers(list: &[GLenum]) {
    // SAFETY: the list outlives the call.
    unsafe { glDrawBuffers(list.len() as GLsizei, list.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers {list:x?}");
}

fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

fn val(r: &Row) -> &'static str {
    if r.signed {
        "ivec4(1, -2, 3, -4)"
    } else {
        "uvec4(1u, 2u, 3u, 4u)"
    }
}

/// Outputs at the given locations: (location, type, value).
fn program_outs(outs: &[(u32, &str, &str)]) {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (l, t, _) in outs {
        s += &format!("layout(location = {l}) out highp {t} o{l};\n");
    }
    s += "void main() {\n";
    for (l, _, v) in outs {
        s += &format!("o{l} = {v};\n");
    }
    s += "}\n";
    program(&s);
}

const NONE: GLenum = GL_NONE;
const A0: GLenum = GL_COLOR_ATTACHMENT0;
const A2: GLenum = GL_COLOR_ATTACHMENT0 + 2;
const A3: GLenum = GL_COLOR_ATTACHMENT0 + 3;
const RGBA8_VAL: &str = "vec4(0.25, 0.5, 0.75, 1.0)";

fn taken(what: &str, seen: Option<Seen>, e: GLenum) -> Seen {
    assert_eq!(e, GL_NO_ERROR, "{what}");
    let s = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
    assert_eq!(s.refused, None, "{what}");
    s
}

fn is_wide_of(r: &Row, t: (u64, u32, u32, u8), what: &str) {
    assert_ne!(t.0, 0, "{what}: a store");
    let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{what}: not an integer surface"));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!((l.bits, l.signed, l.channels), (want.bits, want.signed, 4), "{what}: {}", gles_surf_name(t.3));
}

const LEADING_NONE: &str = "draw buffer 0 is GL_NONE while a later draw buffer is written";

/// A draw whose draw buffer 0 is GL_NONE: today the V3D refuses it by name
/// (state.rs `unsupported`, out_shift != 0: its shader output i goes to RT i),
/// while the GL core still builds the one wide target and the format's mask.
/// Returns the refused draw.
fn refused_by_name(what: &str, seen: Option<Seen>, e: GLenum) -> Seen {
    assert_eq!(e, GL_NO_ERROR, "{what}");
    let s = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
    assert_eq!(s.refused, Some(LEADING_NONE), "{what}");
    s
}

/// Case 1: The texture only at ATTACHMENT2, {NONE, NONE, ATTACHMENT2}: complete,
/// drawn, as the wide surface with the format's channels.
#[test]
fn texture_only_at_attachment_2_has_the_wide_target_and_format_mask_but_is_refused_by_name() {
    let _g = context();
    for r in ROWS {
        let mut f = fbo_empty();
        attach_tex(&mut f, r.ifmt, A2);
        draw_buffers(&[NONE, NONE, A2]);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
        program_outs(&[(2, r.ty(), val(&r))]);
        let (seen, e) = draw();
        let what = format!("{} at attachment 2", r.name);
        let s = refused_by_name(&what, seen, e);
        assert_eq!(s.targets.len(), 1, "{what}: one target, the gaps before it are dropped");
        is_wide_of(&r, s.targets[0], &what);
        assert_eq!(s.color_mask, own_mask(&r), "{what}: the format's channels");
    }
}

/// 1b. The same, the application's glColorMask ANDed with the format's.
#[test]
fn partial_colour_mask_at_attachment_2_is_anded_with_the_format_mask_refused_by_name() {
    let _g = context();
    for r in ROWS {
        let mut f = fbo_empty();
        attach_tex(&mut f, r.ifmt, A2);
        draw_buffers(&[NONE, NONE, A2]);
        program_outs(&[(2, r.ty(), val(&r))]);
        for (m, name) in [([1, 0, 1, 1], "R,B,A"), ([0, 1, 1, 1], "G,B,A"), ([0, 0, 1, 1], "B,A")] {
            // SAFETY: plain GL call.
            unsafe { glColorMask(m[0], m[1], m[2], m[3]) };
            let app = m[0] | m[1] << 1 | m[2] << 2 | m[3] << 3;
            let (seen, e) = draw();
            let what = format!("{} at attachment 2, glColorMask {name}", r.name);
            let s = refused_by_name(&what, seen, e);
            assert_eq!(s.targets.len(), 1, "{what}");
            assert_eq!(s.color_mask, app & own_mask(&r), "{what}: application mask AND the format's");
        }
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
    }
}

/// Case 2: ATTACHMENT3 texture beside an RGBA8 texture at ATTACHMENT0,
/// {ATTACHMENT0, NONE, NONE, ATTACHMENT3}.
#[test]
fn texture_at_attachment_3_beside_rgba8_at_0_gets_its_own_mask_and_format() {
    let _g = context();
    for r in [ROWS[0], ROWS[7], ROWS[10]] {
        let mut f = fbo_empty();
        attach_tex(&mut f, GL_RGBA8, A0);
        attach_tex(&mut f, r.ifmt, A3);
        draw_buffers(&[A0, NONE, NONE, A3]);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
        program_outs(&[(0, "vec4", RGBA8_VAL), (3, r.ty(), val(&r))]);
        let (seen, e) = draw();
        let what = format!("{} at attachment 3 beside RGBA8 at 0", r.name);
        let s = taken(&what, seen, e);
        assert_eq!(s.targets.len(), 4, "{what}: targets up to the highest memory");
        assert_eq!(s.targets[1].0, 0, "{what}: gap 1 has no memory");
        assert_eq!(s.targets[2].0, 0, "{what}: gap 2 has no memory");
        assert_ne!(s.targets[0].0, 0, "{what}");
        assert!(backend::gles_surf_int_layout(s.targets[0].3).is_none(), "{what}: RT 0 is not an integer surface");
        is_wide_of(&r, s.targets[3], &what);
        let j = s.job.unwrap_or_else(|| panic!("{what}: no job state"));
        assert_eq!(j.0, 0xf, "{what}: RT 0 writes four channels");
        assert!(j.1, "{what}: a mask of its own per RT");
        let m = u32::from(own_mask(&r));
        assert_eq!(
            (j.2 >> 8) & 0xf,
            m,
            "{what}: RT 3 (nibble 2 of rt_colormask) the format's channels (rt_colormask {:#x})",
            j.2
        );
        assert_eq!(j.2 & 0xff, 0, "{what}: the gaps write nothing (rt_colormask {:#x})", j.2);
    }
}

/// Case 3: glDrawBuffers changing between draws on one FBO.
#[test]
fn draw_buffers_changing_between_draws_move_the_targets_formats_and_masks() {
    let _g = context();
    for r in [ROWS[0], ROWS[7], ROWS[10]] {
        let mut f = fbo_empty();
        attach_tex(&mut f, GL_RGBA8, A0);
        attach_tex(&mut f, r.ifmt, A3);
        let m = u32::from(own_mask(&r));

        // a. only the integer texture
        draw_buffers(&[NONE, NONE, NONE, A3]);
        program_outs(&[(3, r.ty(), val(&r))]);
        let (seen, e) = draw();
        let what = format!("{} step a", r.name);
        let s = refused_by_name(&what, seen, e);
        assert_eq!(s.targets.len(), 1, "{what}");
        is_wide_of(&r, s.targets[0], &what);
        assert_eq!(s.color_mask, own_mask(&r), "{what}");

        // b. only the RGBA8 texture
        draw_buffers(&[A0]);
        program_outs(&[(0, "vec4", RGBA8_VAL)]);
        let (seen, e) = draw();
        let what = format!("{} step b", r.name);
        let s = taken(&what, seen, e);
        assert_eq!(s.targets.len(), 1, "{what}");
        assert!(backend::gles_surf_int_layout(s.targets[0].3).is_none(), "{what}: RGBA8, not the integer store");
        assert_eq!(s.color_mask, 0xf, "{what}: the format mask of the integer target does not stay");

        // c. both with gaps
        draw_buffers(&[A0, NONE, NONE, A3]);
        program_outs(&[(0, "vec4", RGBA8_VAL), (3, r.ty(), val(&r))]);
        let (seen, e) = draw();
        let what = format!("{} step c", r.name);
        let s = taken(&what, seen, e);
        assert_eq!(s.targets.len(), 4, "{what}");
        let j = s.job.unwrap_or_else(|| panic!("{what}: no job state"));
        assert_eq!((j.0, (j.2 >> 8) & 0xf), (0xf, m), "{what}: RT 0 and RT 3 masks");

        // d. back to the integer texture alone
        draw_buffers(&[NONE, NONE, NONE, A3]);
        program_outs(&[(3, r.ty(), val(&r))]);
        let (seen, e) = draw();
        let what = format!("{} step d", r.name);
        let s = refused_by_name(&what, seen, e);
        assert_eq!(s.targets.len(), 1, "{what}");
        is_wide_of(&r, s.targets[0], &what);
        assert_eq!(s.color_mask, own_mask(&r), "{what}");
    }
}

/// Case 4: A partial glColorMask beside another target: each RT's mask is the
/// application's ANDed with that target's channels.
#[test]
fn partial_colour_mask_beside_rgba8_is_anded_per_target() {
    let _g = context();
    for r in [ROWS[0], ROWS[7], ROWS[10]] {
        let mut f = fbo_empty();
        attach_tex(&mut f, GL_RGBA8, A0);
        attach_tex(&mut f, r.ifmt, A3);
        draw_buffers(&[A0, NONE, NONE, A3]);
        program_outs(&[(0, "vec4", RGBA8_VAL), (3, r.ty(), val(&r))]);
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 0, 1, 1) };
        let (seen, e) = draw();
        let what = format!("{} at 3 beside RGBA8, glColorMask R,B,A", r.name);
        let s = taken(&what, seen, e);
        let j = s.job.unwrap_or_else(|| panic!("{what}: no job state"));
        assert_eq!(j.0, 0xd, "{what}: RT 0 the application's mask");
        assert_eq!((j.2 >> 8) & 0xf, 0xd & u32::from(own_mask(&r)), "{what}: RT 3");
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
    }
}

/// Under GL_BLEND with uneven masks the tip refuses by name (one blend enable
/// for every RT). Item v3d-per-rt-blend-enable may lift this; then this test
/// is to be flipped to "taken".
#[test]
fn blend_with_uneven_masks_is_refused_by_name_or_taken() {
    let _g = context();
    let r = ROWS[0];
    let mut f = fbo_empty();
    attach_tex(&mut f, GL_RGBA8, A0);
    attach_tex(&mut f, r.ifmt, A3);
    draw_buffers(&[A0, NONE, NONE, A3]);
    program_outs(&[(0, "vec4", RGBA8_VAL), (3, r.ty(), val(&r))]);
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_BLEND) };
    let (seen, e) = draw();
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_BLEND) };
    match seen {
        None => assert_ne!(e, GL_NO_ERROR, "no draw and no GL error"),
        Some(s) => match s.refused {
            Some(why) => assert!(!why.is_empty()),
            None => assert_eq!(e, GL_NO_ERROR),
        },
    }
}
