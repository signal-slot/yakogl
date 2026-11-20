// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D: the twelve R and RG integer textures (R8I .. RG32UI) as colour
//! attachments. `es3_formats` holds each in the four-channel store of its
//! channel type and width; the V3D build (`WIDE_INT_RT`, src/gl/framebuffer.rs)
//! draws and clears that store's surface with the format's channels as the
//! write mask, and reads the others back as 0, 0, 1. The back end is a
//! stand-in that records what `state::refusal`, `state::unsupported` and
//! `state::job_state` make of each draw; no GPU runs here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_narrow_int_rt_v3d
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
use yakogl::v3d::ffi::*;
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
    do_clear: bool,
    clear_rgba: [f32; 4],
    clear_target_mask: u8,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    color_mask: u8,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`, when the V3D takes the draw.
    job: Option<V3dJobState>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    let job = if d.index_count != 0 { state::job_state(d).ok() } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        do_clear: d.do_clear,
        clear_rgba: d.clear_rgba,
        clear_target_mask: d.clear_target_mask,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        color_mask: d.color_mask,
        refused,
        job,
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
    name: c"es3-narrow-int-rt".as_ptr() as *const c_char,
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

/* ---- the formats: GL name, surface, and the job words Mesa gives them ---- */

/// One of the twelve formats with the words an enabled path owes it.
struct Row {
    name: &'static str,
    internalformat: GLenum,
    /// `format` of glReadPixels / glTexSubImage2D.
    format: GLenum,
    /// `type` of glReadPixels.
    ty: GLenum,
    signed: bool,
    /// Bits a channel.
    bits: u32,
    channels: u32,
    /// The surface the GL core would hand the back end.
    surf: u8,
    /// Mesa `v3dx_format_table.c` rt column (:118-135) as `V3D_OUTPUT_IMAGE_FORMAT_*`.
    output: u32,
    /// `v3dX(get_internal_type_bpp_for_output_format)` (:225-345).
    internal_type: u32,
    internal_bpp: u32,
}

macro_rules! row {
    ($n:literal, $gl:ident, $f:ident, $t:ident, $signed:literal, $bits:literal, $ch:literal, $s:ident, $o:ident, $it:ident, $bpp:ident) => {
        Row {
            name: $n,
            internalformat: $gl,
            format: $f,
            ty: $t,
            signed: $signed,
            bits: $bits,
            channels: $ch,
            surf: $s,
            output: $o,
            internal_type: $it,
            internal_bpp: $bpp,
        }
    };
}

fn rows() -> Vec<Row> {
    vec![
        row!(
            "R8I",
            GL_R8I,
            GL_RED_INTEGER,
            GL_BYTE,
            true,
            8,
            1,
            GLES_SURF_R8I,
            V3D_OUTPUT_IMAGE_FORMAT_R8I,
            V3D_INTERNAL_TYPE_8I,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "R8UI",
            GL_R8UI,
            GL_RED_INTEGER,
            GL_UNSIGNED_BYTE,
            false,
            8,
            1,
            GLES_SURF_R8UI,
            V3D_OUTPUT_IMAGE_FORMAT_R8UI,
            V3D_INTERNAL_TYPE_8UI,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "R16I",
            GL_R16I,
            GL_RED_INTEGER,
            GL_SHORT,
            true,
            16,
            1,
            GLES_SURF_R16I,
            V3D_OUTPUT_IMAGE_FORMAT_R16I,
            V3D_INTERNAL_TYPE_16I,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "R16UI",
            GL_R16UI,
            GL_RED_INTEGER,
            GL_UNSIGNED_SHORT,
            false,
            16,
            1,
            GLES_SURF_R16UI,
            V3D_OUTPUT_IMAGE_FORMAT_R16UI,
            V3D_INTERNAL_TYPE_16UI,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "R32I",
            GL_R32I,
            GL_RED_INTEGER,
            GL_INT,
            true,
            32,
            1,
            GLES_SURF_R32I,
            V3D_OUTPUT_IMAGE_FORMAT_R32I,
            V3D_INTERNAL_TYPE_32I,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "R32UI",
            GL_R32UI,
            GL_RED_INTEGER,
            GL_UNSIGNED_INT,
            false,
            32,
            1,
            GLES_SURF_R32UI,
            V3D_OUTPUT_IMAGE_FORMAT_R32UI,
            V3D_INTERNAL_TYPE_32UI,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "RG8I",
            GL_RG8I,
            GL_RG_INTEGER,
            GL_BYTE,
            true,
            8,
            2,
            GLES_SURF_RG8I,
            V3D_OUTPUT_IMAGE_FORMAT_RG8I,
            V3D_INTERNAL_TYPE_8I,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "RG8UI",
            GL_RG8UI,
            GL_RG_INTEGER,
            GL_UNSIGNED_BYTE,
            false,
            8,
            2,
            GLES_SURF_RG8UI,
            V3D_OUTPUT_IMAGE_FORMAT_RG8UI,
            V3D_INTERNAL_TYPE_8UI,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "RG16I",
            GL_RG16I,
            GL_RG_INTEGER,
            GL_SHORT,
            true,
            16,
            2,
            GLES_SURF_RG16I,
            V3D_OUTPUT_IMAGE_FORMAT_RG16I,
            V3D_INTERNAL_TYPE_16I,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "RG16UI",
            GL_RG16UI,
            GL_RG_INTEGER,
            GL_UNSIGNED_SHORT,
            false,
            16,
            2,
            GLES_SURF_RG16UI,
            V3D_OUTPUT_IMAGE_FORMAT_RG16UI,
            V3D_INTERNAL_TYPE_16UI,
            V3D_INTERNAL_BPP_32
        ),
        row!(
            "RG32I",
            GL_RG32I,
            GL_RG_INTEGER,
            GL_INT,
            true,
            32,
            2,
            GLES_SURF_RG32I,
            V3D_OUTPUT_IMAGE_FORMAT_RG32I,
            V3D_INTERNAL_TYPE_32I,
            V3D_INTERNAL_BPP_64
        ),
        row!(
            "RG32UI",
            GL_RG32UI,
            GL_RG_INTEGER,
            GL_UNSIGNED_INT,
            false,
            32,
            2,
            GLES_SURF_RG32UI,
            V3D_OUTPUT_IMAGE_FORMAT_RG32UI,
            V3D_INTERNAL_TYPE_32UI,
            V3D_INTERNAL_BPP_64
        ),
    ]
}

/// The tile buffer's clear words for the app's four raw values, the way
/// `v3d_tlb_clear` (v3dx_draw.c, not vendored; docs/notes/v3d-int-clear.md)
/// packs them for an 8I/8UI, 16I/16UI or 32I/32UI internal type.
fn mesa_clear_words(bits: u32, v: [u32; 4]) -> [u32; 4] {
    match bits {
        8 => [(v[0] & 0xff) | (v[1] & 0xff) << 8 | (v[2] & 0xff) << 16 | (v[3] & 0xff) << 24, 0, 0, 0],
        16 => [(v[0] & 0xffff) | v[1] << 16, (v[2] & 0xffff) | v[3] << 16, 0, 0],
        _ => v,
    }
}

/// Raw values the clears use: negative as an int, above the narrow ranges.
const RAW: [u32; 4] = [0xffff_fff9, 7, 0x1_0003, 0x8000_0001];

/* ---- a texture attachment, a program, a draw, a clear ------------------- */

/// A bound framebuffer with one 8x8 level-0 texture of `ifmt` on colour
/// attachment 0, as dEQP's fbo.color.tex2d.* builds it.
struct TexFbo {
    fb: GLuint,
    tex: GLuint,
}

impl Drop for TexFbo {
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

/// Make the texture and attach it; the storage and the attach never fail.
fn tex_fbo(ifmt: GLenum) -> TexFbo {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, 1, ifmt, 8, 8);
        assert_eq!(err(), GL_NO_ERROR, "storage {ifmt:#x}");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "attach {ifmt:#x}");
    }
    TexFbo { fb, tex }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
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

/// The current program writes `ty` ("ivec4" or "uvec4") to output 0.
fn program(ty: &str) {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nlayout(location = 0) out {ty} o0;\n\
         void main() {{ o0 = {ty}(1, 2, 3, 4); }}\n"
    );
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
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

/// Draw one triangle; what the back end was handed (None: it was handed no
/// draw) and the GL error the call left.
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

/// glClearBuffer{i,ui}v of attachment 0 with [`RAW`]; the back end's clear
/// (None: it was handed none) and the GL error.
fn clear_buffer(signed: bool) -> (Option<Seen>, GLenum) {
    SEEN.lock().unwrap().clear();
    // SAFETY: four values each.
    unsafe {
        if signed {
            glClearBufferiv(GL_COLOR, 0, RAW.map(|w| w as i32).as_ptr());
        } else {
            glClearBufferuiv(GL_COLOR, 0, RAW.as_ptr());
        }
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (s.into_iter().rev().find(|s| s.do_clear), e)
}

/// glReadPixels of one pixel in the format's own format and type; the GL error.
fn read_pixel(r: &Row) -> GLenum {
    let mut px = [0u32; 4];
    // SAFETY: room for the 8 bytes of the widest pixel.
    unsafe { glReadPixels(0, 0, 1, 1, r.format, r.ty, px.as_mut_ptr() as *mut c_void) };
    err()
}

/* ---- the texture is drawn as its four-channel store ---------------------- */

/// The four-channel surface the store of `r` is (`es3_formats` widens an R or
/// RG integer texture to the four-channel class of its channel type and
/// width), and that surface's `V3D_INTERNAL_BPP_*`.
fn wide(r: &Row) -> (u8, u32) {
    match (r.bits, r.signed) {
        (8, true) => (GLES_SURF_RGBA8I, V3D_INTERNAL_BPP_32),
        (8, false) => (GLES_SURF_RGBA8UI, V3D_INTERNAL_BPP_32),
        (16, true) => (GLES_SURF_RGBA16I, V3D_INTERNAL_BPP_64),
        (16, false) => (GLES_SURF_RGBA16UI, V3D_INTERNAL_BPP_64),
        (32, true) => (GLES_SURF_RGBA32I, V3D_INTERNAL_BPP_128),
        _ => (GLES_SURF_RGBA32UI, V3D_INTERNAL_BPP_128),
    }
}

/// Storage and attach are accepted, and the framebuffer is complete for all
/// twelve: the GL core's `integer_texture_internalformat`
/// (src/gl/framebuffer.rs) takes the widened store where `WIDE_INT_RT` is
/// set, which the V3D build is. (`tex_fbo` asserts the storage and the
/// attach.)
#[test]
fn every_narrow_texture_is_a_complete_attachment() {
    let _g = context();
    for r in rows() {
        let _f = tex_fbo(r.internalformat);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
    }
}

/// Controls: the same formats as renderbuffers are complete on their own
/// surface (renderbuffer_surf_format), and the four-channel sibling texture
/// is complete: the class the narrow texture is drawn as.
#[test]
fn the_renderbuffer_and_the_four_channel_texture_are_complete() {
    let _g = context();
    for r in rows() {
        let (mut fb, mut rb) = (0, 0);
        // SAFETY: room for one name each.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, r.internalformat, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} renderbuffer", r.name);
        // SAFETY: names made above.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteRenderbuffers(1, &rb);
        }
    }
    for (name, ifmt) in
        [("RGBA8I", GL_RGBA8I), ("RGBA8UI", GL_RGBA8UI), ("RGBA16I", GL_RGBA16I), ("RGBA32UI", GL_RGBA32UI)]
    {
        let _f = tex_fbo(ifmt);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{name} texture");
    }
}

/* ---- the expected words, per format -------------------------------------- */

/// Everything a texture attachment of `r` does: complete; a draw reaches the
/// V3D on the four-channel surface of the store with the format's channels as
/// the write mask, and the job builder takes it with Mesa's row for that
/// surface (the format's internal type, the wide surface's bpp and bytes);
/// glClearBuffer reaches it with the tile buffer's words for the raw values
/// of the format's channels and 0, 0, 1 in the others; a read-back in the
/// format's own pair is no error. The format's own surface (`r.surf`,
/// `r.output`) stays the renderbuffer's.
fn expect_wide(r: &Row) {
    let f = tex_fbo(r.internalformat);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{}", r.name);
    let (surf, bpp) = wide(r);
    let own = (1u8 << r.channels) - 1;

    /* a draw writing an ivec4 / uvec4: the V3D takes it on the wide surface */
    program(if r.signed { "ivec4" } else { "uvec4" });
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{}", r.name);
    let s = seen.unwrap_or_else(|| panic!("{}: the back end saw no draw", r.name));
    assert_eq!(s.refused, None, "{}", r.name);
    assert_eq!(s.targets[0].1, surf, "{}: surface {}", r.name, gles_surf_name(s.targets[0].1));
    assert_eq!(s.color_mask, own, "{}: the format's channels only", r.name);
    let job = s.job.unwrap_or_else(|| panic!("{}: job_state refuses the draw", r.name));
    assert_eq!(job.colormask, u32::from(own), "{}: the job's colour mask", r.name);
    let row = state::rt_format(surf).unwrap();
    assert_eq!((row.internal_type, row.internal_bpp), (r.internal_type, bpp), "{}", r.name);
    assert_eq!(row.clamp, V3D_RENDER_TARGET_CLAMP_INT, "{}", r.name);
    assert_eq!(row.bytes, r.bits / 8 * 4, "{}: bytes a pixel of the wide store", r.name);
    assert_ne!(row.output, r.output, "{}: not the narrow surface's output format", r.name);
    /* the format's own surface keeps Mesa's row (the renderbuffer's) */
    let narrow = state::rt_format(r.surf).unwrap();
    assert_eq!(
        (narrow.output, narrow.internal_type, narrow.internal_bpp),
        (r.output, r.internal_type, r.internal_bpp),
        "{}: the narrow surface",
        r.name
    );

    /* the clear: the format's raw words, unconverted, and 0 (G, B) and 1
     * (A) in the channels it lacks (src/gl/draw.rs `int_clear_channels`) */
    let (seen, e) = clear_buffer(r.signed);
    assert_eq!(e, GL_NO_ERROR, "{}", r.name);
    let s = seen.unwrap_or_else(|| panic!("{}: the back end saw no clear", r.name));
    assert_eq!(s.clear_target_mask & 1, 1, "{}", r.name);
    assert_eq!(s.targets[0].1, surf, "{}", r.name);
    let mut want = RAW;
    for (ch, w) in want.iter_mut().enumerate().skip(r.channels as usize) {
        *w = u32::from(ch == 3);
    }
    assert_eq!(state::clear_words(surf, s.clear_rgba), mesa_clear_words(r.bits, want), "{}", r.name);

    /* the read-back: the class's pair ES 3.0 4.3.1 requires (RGBA_INTEGER with
     * INT or UNSIGNED_INT) is taken and reads 0, 0, 1 in the channels the
     * format lacks (this stand-in stores no clear, so the format's own
     * channels are not compared here: tests/es3_readback_v3d.rs reads them
     * through the V3D back end); the implementation's pair is taken; the
     * format's own pair (RED_INTEGER / BYTE ..) is not, as the
     * implementation's pair is the wide store's */
    let mut px = [0xdead_beefu32; 4];
    let ty = if r.signed { GL_INT } else { GL_UNSIGNED_INT };
    // SAFETY: room for one pixel of four words.
    unsafe { glReadPixels(0, 0, 1, 1, GL_RGBA_INTEGER, ty, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "{}: RGBA_INTEGER", r.name);
    for (ch, w) in px.iter().enumerate().skip(r.channels as usize) {
        assert_eq!(*w, u32::from(ch == 3), "{}: channel {ch} reads 0, 0, 1", r.name);
    }
    let (mut fmt, mut ity) = (0, 0);
    // SAFETY: one value each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut fmt);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut ity);
        glReadPixels(0, 0, 1, 1, fmt as GLenum, ity as GLenum, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: the implementation's pair {fmt:#x}/{ity:#x}", r.name);
    assert_eq!(read_pixel(r), GL_INVALID_OPERATION, "{}: the format's own pair", r.name);
    drop(f);
}

/// One test a format.
macro_rules! wide {
    ($($fn:ident $name:literal;)*) => {$(
        #[test]
        fn $fn() {
            let _g = context();
            expect_wide(rows().iter().find(|r| r.name == $name).unwrap());
        }
    )*};
}

wide! {
    r8i_texture_renders_clears_and_reads "R8I";
    r8ui_texture_renders_clears_and_reads "R8UI";
    rg8i_texture_renders_clears_and_reads "RG8I";
    rg8ui_texture_renders_clears_and_reads "RG8UI";
    r16i_texture_renders_clears_and_reads "R16I";
    r16ui_texture_renders_clears_and_reads "R16UI";
    rg16i_texture_renders_clears_and_reads "RG16I";
    rg16ui_texture_renders_clears_and_reads "RG16UI";
    r32i_texture_renders_clears_and_reads "R32I";
    r32ui_texture_renders_clears_and_reads "R32UI";
    rg32i_texture_renders_clears_and_reads "RG32I";
    rg32ui_texture_renders_clears_and_reads "RG32UI";
}
