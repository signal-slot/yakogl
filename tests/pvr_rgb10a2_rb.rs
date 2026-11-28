// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P, BXS-4-64): a GL_RGB10_A2 renderbuffer is the 10/10/10/2
//! surface and reports 10/10/10/2 component sizes (dEQP-GLES3
//! state_query.rbo.renderbuffer_component_size_color and the fbo /
//! texture.format cases of the format, #313).
//!
//! The PowerVR renders and samples R10G10B10A2_UNORM as Mesa 26.2.1 does
//! (src/imagination, MIT): the TPU's A2R10B10G10 texture format
//! (pvr_arch_formats.c:223, `TEXF_RGB10A2` in src/pvr/backend.rs) and the
//! PBE's A2R10B10G10 pack over F16 halves (`pvr::backend::rt_class`,
//! docs/es3/pvr-formats.md 10.1). `PVR_SAMPLED_TEXFMTS` holds
//! `GLES_TEXFMT_RGB10A2`, so `rt_format_drawable(GLES_SURF_RGB10_A2)` is
//! true and the GL core keeps the format in its own class rather than in
//! RGBA8 (two bits lost, alpha widened, the sizes 8/8/8/8). Held here
//! through the GL API on a stand-in back end and the in-process compiler:
//!
//! - the renderbuffer's RED/GREEN/BLUE/ALPHA sizes are 10/10/10/2 and its
//!   attachment's the same, UNSIGNED_NORMALIZED; the implementation read
//!   format and type are RGBA / UNSIGNED_INT_2_10_10_10_REV;
//! - a clear and a draw hand the back end one target of `GLES_SURF_RGB10_A2`
//!   at the renderbuffer's store, and the draw's fragment program is
//!   compiled for the back end's class of the surface (the unsigned
//!   normalised halves class `SHC_RT_SRGB`);
//! - what the back end writes there (the stand-in packs the words the PBE
//!   would, A2R10B10G10 = R in bits 0-9, G 10-19, B 20-29, A 30-31) reads
//!   back word for word as UNSIGNED_INT_2_10_10_10_REV and converted as
//!   RGBA / UNSIGNED_BYTE;
//! - a GL_RGB10_A2 texture attachment is drawn as the same surface.
//!
//! HOST-ONLY: the stand-in writes the pixels; the GPU's words are not seen.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_rgb10a2_rb
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, GLES_SURF_RGB10_A2};
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
use yakogl::shaderc::{shc_blend_key_target, SHC_BLEND_KEY_REPLACE, SHC_RT_SRGB};

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

/// What the back end was handed, one entry a draw (or clear).
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    /// The fragment program's (`ShcShader::blend_key`), 0 without one.
    blend_key: u32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The 10/10/10/2 word the stand-in "GPU" writes for a draw's fragments.
static DRAWN: Mutex<u32> = Mutex::new(0);

/// The A2R10B10G10 word of a colour, as the PBE packs the clamped halves:
/// round(v * 1023) for R, G and B, round(v * 3) for A.
fn pack(c: [f32; 4]) -> u32 {
    let q = |v: f32, max: f32| (v.clamp(0.0, 1.0) * max).round() as u32;
    q(c[0], 1023.0) | q(c[1], 1023.0) << 10 | q(c[2], 1023.0) << 20 | q(c[3], 3.0) << 30
}

/// Records each draw; into a 10/10/10/2 target, a clear stores its packed
/// colour and a draw [`DRAWN`] in every pixel, rows at the pair-padded pitch.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = d.num_targets.max(1) as usize;
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        blend_key: d.fs.as_ref().map_or(0, |fs| fs.blend_key),
    });
    let t = &d.targets[0];
    if t.format != GLES_SURF_RGB10_A2 || t.gpu == 0 {
        return 0;
    }
    let word = if d.index_count != 0 {
        *DRAWN.lock().unwrap()
    } else if d.do_clear {
        pack(d.clear_rgba)
    } else {
        return 0;
    };
    let pitch = ((t.w + 1) & !1) as usize;
    let base = t.gpu as *mut u32;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            base.add(y * pitch + x).write_unaligned(word);
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
    name: c"pvr-rgb10a2-rb".as_ptr() as *const c_char,
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

/// A `w` x `w` GL_RGB10_A2 renderbuffer at GL_COLOR_ATTACHMENT0 of a new
/// framebuffer, both bound: (framebuffer, renderbuffer).
fn rgb10a2_renderbuffer(w: GLsizei) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGB10_A2, w, w);
        assert_eq!(glGetError(), GL_NO_ERROR, "glRenderbufferStorage(GL_RGB10_A2)");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (fb, rb)
}

fn release(fb: GLuint, rb: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(1, &rb);
    }
}

/// The 2_10_10_10_REV words of a `w` x `w` read of the bound framebuffer.
fn read_words(w: usize) -> Vec<u32> {
    let mut px = vec![0u32; w * w];
    // SAFETY: room for w x w packed pixels.
    unsafe {
        glReadPixels(
            0,
            0,
            w as GLsizei,
            w as GLsizei,
            GL_RGBA,
            GL_UNSIGNED_INT_2_10_10_10_REV,
            px.as_mut_ptr() as *mut c_void,
        )
    };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels RGBA / UNSIGNED_INT_2_10_10_10_REV");
    px
}

/// The class's bit is in the PowerVR's sampled set, so the surface is one
/// the GL core hands over.
#[test]
fn the_class_is_sampled_and_rendered() {
    assert_ne!(backend::PVR_SAMPLED_TEXFMTS & backend::gles_texfmt_bit(backend::GLES_TEXFMT_RGB10A2), 0);
    assert!(backend::rt_format_drawable(GLES_SURF_RGB10_A2));
}

/// dEQP state_query.rbo.renderbuffer_component_size_color: 10/10/10/2, no
/// depth or stencil, both on the renderbuffer and on its attachment.
#[test]
fn the_renderbuffer_reports_ten_ten_ten_two() {
    let _g = context();
    let (fb, rb) = rgb10a2_renderbuffer(4);
    let rbq = |pname| {
        let mut v = -1;
        // SAFETY: one GLint.
        unsafe { glGetRenderbufferParameteriv(GL_RENDERBUFFER, pname, &mut v) };
        v
    };
    assert_eq!(rbq(GL_RENDERBUFFER_INTERNAL_FORMAT), GL_RGB10_A2 as GLint);
    assert_eq!(
        [
            rbq(GL_RENDERBUFFER_RED_SIZE),
            rbq(GL_RENDERBUFFER_GREEN_SIZE),
            rbq(GL_RENDERBUFFER_BLUE_SIZE),
            rbq(GL_RENDERBUFFER_ALPHA_SIZE),
            rbq(GL_RENDERBUFFER_DEPTH_SIZE),
            rbq(GL_RENDERBUFFER_STENCIL_SIZE),
        ],
        [10, 10, 10, 2, 0, 0]
    );
    let aq = |pname| {
        let mut v = -1;
        // SAFETY: one GLint.
        unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, pname, &mut v) };
        v
    };
    assert_eq!(
        [
            aq(GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE),
            aq(GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE),
            aq(GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE),
            aq(GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE),
        ],
        [10, 10, 10, 2]
    );
    assert_eq!(aq(GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE), GL_UNSIGNED_NORMALIZED as GLint);
    let mut v = [0; 2];
    // SAFETY: one GLint each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut v[0]);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut v[1]);
    }
    assert_eq!(v, [GL_RGBA as GLint, GL_UNSIGNED_INT_2_10_10_10_REV as GLint]);
    release(fb, rb);
}

/// A clear reaches the back end as the 10/10/10/2 surface at the
/// renderbuffer's store, and the words written there read back unchanged:
/// a red of 0.5 is 512 of 1023, which RGBA8 would have kept as 128 of 255.
#[test]
fn a_clear_round_trips_ten_bit_words() {
    let _g = context();
    let (fb, rb) = rgb10a2_renderbuffer(4);
    let colour = [0.5f32, 0.25, 1.0, 1.0 / 3.0];
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(colour[0], colour[1], colour[2], colour[3]);
        glClear(GL_COLOR_BUFFER_BIT);
    }
    let words = read_words(4);
    let seen = SEEN.lock().unwrap().clone();
    let clear = seen.iter().rev().find(|s| !s.draws).expect("the clear reached the back end");
    assert_eq!(clear.targets.len(), 1);
    let (gpu, w, h, fmt) = clear.targets[0];
    assert_ne!(gpu, 0, "the renderbuffer's store");
    assert_eq!((w, h, fmt), (4, 4, GLES_SURF_RGB10_A2), "the 10/10/10/2 surface");
    let want = pack(colour);
    assert_eq!(want, 512 | 256 << 10 | 1023 << 20 | 1 << 30);
    for (i, &p) in words.iter().enumerate() {
        assert_eq!(p, want, "pixel {i}: {p:#010x}");
    }
    // SAFETY: plain GL call.
    unsafe { glClearColor(0.0, 0.0, 0.0, 0.0) };
    release(fb, rb);
}

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 o;\n\
                  void main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n";

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

/// A linked, current program of [`VS`] and [`FS`].
fn program() -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
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
        p
    }
}

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
}

/// The draw hands the back end the 10/10/10/2 surface with a fragment
/// program compiled for the PowerVR's class of it (four halves the PBE packs
/// A2R10B10G10); the shader's colour as the PBE stores it reads back as
/// those words, and as RGBA / UNSIGNED_BYTE converted from ten bits.
#[test]
fn a_draw_round_trips_through_the_ten_bit_surface() {
    let _g = context();
    let (fb, rb) = rgb10a2_renderbuffer(4);
    let p = program();
    *DRAWN.lock().unwrap() = pack([0.25, 0.5, 0.75, 1.0]);
    SEEN.lock().unwrap().clear();
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR, "the draw");
    let d = SEEN.lock().unwrap().iter().rev().find(|s| s.draws).cloned().expect("the draw reached the back end");
    assert_eq!(d.targets.len(), 1);
    assert_eq!((d.targets[0].1, d.targets[0].2, d.targets[0].3), (4, 4, GLES_SURF_RGB10_A2));
    assert_eq!(d.blend_key, SHC_BLEND_KEY_REPLACE | shc_blend_key_target(SHC_RT_SRGB, 0), "the halves class");

    let words = read_words(4);
    let want = 256 | 512 << 10 | 767 << 20 | 3 << 30;
    assert_eq!(*DRAWN.lock().unwrap(), want);
    for (i, &w) in words.iter().enumerate() {
        assert_eq!(w, want, "pixel {i}: {w:#010x}");
    }
    let mut px = [0u8; 4 * 4 * 4];
    // SAFETY: room for 4x4 RGBA8 pixels.
    unsafe { glReadPixels(0, 0, 4, 4, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels RGBA / UNSIGNED_BYTE");
    for (i, p) in px.chunks(4).enumerate() {
        /* 256/1023, 512/1023, 767/1023 and 3/3 of 255, rounded */
        assert_eq!(p, [64, 128, 191, 255], "pixel {i}");
    }
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
    release(fb, rb);
}

/// A GL_RGB10_A2 texture at GL_COLOR_ATTACHMENT0 is drawn as the same
/// surface, at the texture's store.
#[test]
fn a_texture_attachment_is_the_same_surface() {
    let _g = context();
    let (mut tex, mut fb) = (0, 0);
    // SAFETY: room for one name each.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGB10_A2, 4, 4);
        assert_eq!(glGetError(), GL_NO_ERROR, "glTexStorage2D(GL_RGB10_A2)");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    let p = program();
    SEEN.lock().unwrap().clear();
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR, "the draw");
    let d = SEEN.lock().unwrap().iter().rev().find(|s| s.draws).cloned().expect("the draw reached the back end");
    assert_ne!(d.targets[0].0, 0, "the texture's store");
    assert_eq!(d.targets[0].3, GLES_SURF_RGB10_A2);
    // SAFETY: names made by the test.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
    }
}
