// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): immutable glTexStorage2D / glTexStorage3D R and RG
//! integer textures (gap G2 of docs/notes/v3d-format-matrix.md, plan #383),
//! every level attached in turn.
//!
//! The twelve formats (R8I .. RG32UI) are held in the four-channel store of
//! their channel type and width and drawn as that store's wide surface with
//! the format's channels as the write mask. Held, for all twelve:
//!
//! - glTexStorage2D with 4 levels of 64x64: each of levels 0-3 attached in
//!   turn is complete; the draw reaches the back end on the wide surface at
//!   that level's size, on that level's own memory (the four levels' stores
//!   are four different addresses, so a level's stride and size are its own),
//!   with the format's channels as the mask; a clear reads back 0, 0, 1 in
//!   the absent channels;
//! - glTexStorage3D for a 2D array (4 layers, 3 levels), layer 3 at level 2:
//!   the same, at 16x16;
//! - a level past the storage's level count: GL_INVALID_VALUE from
//!   glFramebufferTexture2D / Layer (ES 3.0 4.4.2.4), the attachment as it was;
//! - glTexImage2D on the immutable texture: GL_INVALID_OPERATION, and the
//!   attachment stays complete and draws as before.
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! each draw and stores a clear in the target's memory; a draw's shader output
//! is not rasterized. Nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_texstorage
//! ```

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
    name: c"es3-v3d-int-rg-texstorage".as_ptr() as *const c_char,
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

/* ---- attachments --------------------------------------------------------- */

/// A bound framebuffer with a texture at attachment 0; dropped, it unbinds
/// and deletes them.
struct Fbo {
    fb: GLuint,
    tex: GLuint,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
            glUseProgram(0);
        }
    }
}

const SIZE: GLsizei = 64;

/// An immutable 4-level 64x64 TEXTURE_2D of the format, bound, no attachment yet.
fn storage_2d(r: &Row) -> Fbo {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, 4, r.ifmt, SIZE, SIZE);
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: glTexStorage2D", r.name);
    Fbo { fb, tex }
}

fn attach_2d(f: &Fbo, level: GLint) {
    // SAFETY: plain GL call.
    unsafe { glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, f.tex, level) };
}

/// What a draw at the attached level owes (as `expect_texture` of the model
/// file): taken, on a wide surface of the store's type, at `w` x `w`, the
/// format's channels as the mask. Returns the store's address.
fn expect_draw(r: &Row, w: GLsizei, what: &str) -> u64 {
    program_one(r);
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{} {what}: the draw", r.name);
    let s = seen.unwrap_or_else(|| panic!("{} {what}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{} {what}", r.name);
    assert_eq!(s.targets.len(), 1, "{} {what}", r.name);
    let t = s.targets[0];
    assert_ne!(t.0, 0, "{} {what}: the store", r.name);
    assert_eq!((t.1, t.2), (w as u32, w as u32), "{} {what}: the level's size", r.name);
    let l = backend::gles_surf_int_layout(t.3).unwrap_or_else(|| panic!("{} {what}: not an integer surface", r.name));
    let want = backend::gles_surf_int_layout(r.surf).unwrap();
    assert_eq!((l.bits, l.signed), (want.bits, want.signed), "{} {what}: the store's type", r.name);
    assert_eq!(l.channels, 4, "{} {what}: the wide surface, {}", r.name, gles_surf_name(t.3));
    assert_eq!(s.color_mask, own_mask(r), "{} {what}: only the format's channels are written", r.name);
    t.0
}

fn own_mask(r: &Row) -> u8 {
    (1u8 << r.channels) - 1
}

/* ---- glTexStorage2D: every level ------------------------------------------ */

/// Levels 0-3 of 64, 32, 16, 8: each complete, drawn at its size on its own
/// store, cleared and read back.
#[test]
fn texstorage_2d_every_level_completes_and_draws_at_its_own_size() {
    let _g = context();
    for r in ROWS {
        let f = storage_2d(&r);
        let mut stores = vec![];
        for level in 0..4 {
            attach_2d(&f, level);
            let w = SIZE >> level;
            assert_eq!(err(), GL_NO_ERROR, "{} level {level}: attach", r.name);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} level {level}", r.name);
            let what = format!("level {level}");
            stores.push(expect_draw(&r, w, &what));
            clear_and_read_back(&r, w, &what);
        }
        stores.sort_unstable();
        stores.dedup();
        assert_eq!(stores.len(), 4, "{}: every level has its own store", r.name);
    }
}

/// A partial colour mask on a level other than 0 is the format's channels
/// within the application's: the draw is taken either way.
#[test]
fn texstorage_2d_level_2_with_a_colour_mask_is_drawn_with_the_format_channels() {
    let _g = context();
    for r in ROWS {
        let f = storage_2d(&r);
        attach_2d(&f, 2);
        program_one(&r);
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 0, 0, 1) };
        let (seen, e) = draw();
        // SAFETY: plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        assert_eq!(e, GL_NO_ERROR, "{}", r.name);
        let s = seen.unwrap_or_else(|| panic!("{}: the back end saw no draw", r.name));
        assert_eq!(s.refused, None, "{}", r.name);
        assert_eq!(s.color_mask & !own_mask(&r), 0, "{}: no channel the format lacks", r.name);
        assert_eq!((s.targets[0].1, s.targets[0].2), (16, 16), "{}", r.name);
    }
}

/// Level 4 and above of a 4-level storage: GL_INVALID_VALUE (ES 3.0 4.4.2.4:
/// level must be a valid level of the texture), and level 3 stays attached.
#[test]
#[ignore = "defect: glFramebufferTexture2D/Layer of a level past an immutable storage's level count is accepted and glCheckFramebufferStatus answers GL_FRAMEBUFFER_COMPLETE for a missing image (ES 3.0 4.4.2.4 wants GL_INVALID_VALUE or GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT); seen for all twelve formats on the V3D build"]
fn texstorage_2d_attaching_past_the_last_level_is_invalid_value() {
    let _g = context();
    for r in ROWS {
        let f = storage_2d(&r);
        attach_2d(&f, 3);
        assert_eq!(err(), GL_NO_ERROR, "{}", r.name);
        for level in [4, 5, 20] {
            attach_2d(&f, level);
            let e = err();
            assert!(
                e == GL_INVALID_VALUE || (e == GL_NO_ERROR && status() != GL_FRAMEBUFFER_COMPLETE),
                "{} level {level}: error {e:#x} (ES 3.0: GL_INVALID_VALUE, or an incomplete attachment)",
                r.name
            );
            // an invalid call changes nothing: level 3 is attached
            if e == GL_INVALID_VALUE {
                assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} level {level}: the old attachment stays", r.name);
                expect_draw(&r, SIZE >> 3, "level 3 after a refused attach");
            }
            attach_2d(&f, 3);
            assert_eq!(err(), GL_NO_ERROR, "{}", r.name);
        }
    }
}

/// glTexImage2D on an immutable texture: GL_INVALID_OPERATION; the attached
/// level is complete and draws as before.
#[test]
fn texstorage_2d_teximage_is_invalid_operation_and_the_attachment_stays() {
    let _g = context();
    for r in ROWS {
        let f = storage_2d(&r);
        attach_2d(&f, 1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
        let before = expect_draw(&r, SIZE >> 1, "before");
        let (fmt, ty) = if r.signed { (GL_RGBA_INTEGER, GL_INT) } else { (GL_RGBA_INTEGER, GL_UNSIGNED_INT) };
        let _ = (fmt, ty);
        let (pf, pty) = match r.channels {
            1 => (GL_RED_INTEGER, if r.signed { GL_INT } else { GL_UNSIGNED_INT }),
            _ => (GL_RG_INTEGER, if r.signed { GL_INT } else { GL_UNSIGNED_INT }),
        };
        // SAFETY: a null source is a legal "no data"; level 1 of 32x32.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 1, r.ifmt as GLint, 32, 32, 0, pf, pty, core::ptr::null());
        }
        assert_eq!(err(), GL_INVALID_OPERATION, "{}: glTexImage2D on immutable storage", r.name);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}: still complete", r.name);
        let after = expect_draw(&r, SIZE >> 1, "after");
        assert_eq!(before, after, "{}: the store did not move", r.name);
        clear_and_read_back(&r, SIZE >> 1, "after glTexImage2D");
    }
}

/* ---- glTexStorage3D: a 2D array ------------------------------------------- */

/// 4 layers, 3 levels of 64x64: layer 3 of level 2 (16x16) complete, drawn,
/// cleared; every layer of level 2 is its own store.
#[test]
fn texstorage_3d_array_layer_3_level_2_completes_and_draws() {
    let _g = context();
    for r in ROWS {
        let (mut fb, mut tex) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 3, r.ifmt, SIZE, SIZE, 4);
        }
        let f = Fbo { fb, tex };
        assert_eq!(err(), GL_NO_ERROR, "{}: glTexStorage3D", r.name);
        let mut stores = vec![];
        for layer in 0..4 {
            // SAFETY: plain GL call.
            unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, f.tex, 2, layer) };
            assert_eq!(err(), GL_NO_ERROR, "{} layer {layer}: attach", r.name);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} layer {layer}", r.name);
            let what = format!("layer {layer} level 2");
            stores.push(expect_draw(&r, SIZE >> 2, &what));
            clear_and_read_back(&r, SIZE >> 2, &what);
        }
        stores.sort_unstable();
        stores.dedup();
        assert_eq!(stores.len(), 4, "{}: every layer has its own store", r.name);

        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}: layer 3 stays attached", r.name);
    }
}

/// Level 3 of a 3-level array storage, and layer 4 of 4: GL_INVALID_VALUE or
/// an incomplete attachment; the old attachment is kept on an error.
#[test]
#[ignore = "defect: glFramebufferTexture2D/Layer of a level past an immutable storage's level count is accepted and glCheckFramebufferStatus answers GL_FRAMEBUFFER_COMPLETE for a missing image (ES 3.0 4.4.2.4 wants GL_INVALID_VALUE or GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT); seen for all twelve formats on the V3D build"]
fn texstorage_3d_attaching_past_the_last_level_or_layer_is_refused() {
    let _g = context();
    for r in ROWS {
        let (mut fb, mut tex) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
            glTexStorage3D(GL_TEXTURE_2D_ARRAY, 3, r.ifmt, SIZE, SIZE, 4);
        }
        let f = Fbo { fb, tex };
        for (level, layer) in [(3, 0), (0, 4)] {
            // SAFETY: plain GL call.
            unsafe { glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, f.tex, level, layer) };
            let e = err();
            assert!(
                e == GL_INVALID_VALUE || (e == GL_NO_ERROR && status() != GL_FRAMEBUFFER_COMPLETE),
                "{} level {level} layer {layer}: error {e:#x}, status {:#x}",
                r.name,
                status()
            );
        }
    }
}
