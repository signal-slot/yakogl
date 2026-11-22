// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 48cb7272, 02c623ba, c558647b and f68094d6
//! (float colour targets, the extension list, mip-level completeness) on the
//! GC7000. The i.MX8MP whole tree (0fbe61a3) predates all four. dEQP-GLES2
//! reads GL_EXTENSIONS (functional.state_query, info), decides what
//! functional.fbo.completeness.* expects from it, and renders into mip levels
//! above 0 (functional.fbo.render.*, functional.texture.*). This file runs a
//! `report-es2` build through the real GL core and the real Vivante back end
//! over a stand-in OSAL that keeps every allocation in host memory. The
//! expected answers come from the ES 2.0 text and from the extensions the
//! build lists, not from what the build printed:
//!
//! * GL_EXTENSIONS is the exact list below: GL_EXT_color_buffer_float is not
//!   in it (48cb7272: only an ES 3.0 build with `color-buffer-float` on a
//!   capable back end lists it), GL_OES_texture_float is not, and
//!   GL_OES_texture_half_float, GL_EXT_color_buffer_half_float,
//!   GL_OES_fbo_render_mipmap and GL_OES_texture_npot are. The 34 NotSupported
//!   of docs/es2-cts-failures.md section C (EXT_clip_control, multisample,
//!   EXT_multisampled_render_to_texture, compressed sub-image formats) depend
//!   on those names staying absent;
//! * an FBO with an RGBA/UNSIGNED_BYTE, RGB/UNSIGNED_SHORT_5_6_5 or
//!   RGBA/UNSIGNED_SHORT_4_4_4_4 texture is complete at level 0 and at every
//!   level of a power-of-two chain (OES_fbo_render_mipmap), and of a
//!   non-power-of-two one (OES_texture_npot is listed; each level is
//!   max(1, n >> l));
//! * a FLOAT texture is refused by glTexImage2D (OES_texture_float is not
//!   listed), so no FBO can hold one;
//! * RGBA/HALF_FLOAT_OES and RGB/HALF_FLOAT_OES textures are colour
//!   attachments (GL_EXT_color_buffer_half_float is listed): complete;
//! * a draw into a level above 0 is not refused (`gc_refused_draws()` does
//!   not move) and ends in GL_NO_ERROR.
//!
//! What it cannot see: the GPU (the board's functional.fbo.* is that answer).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_fbo_complete
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL: host memory; every kick completes ------------------------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: usize,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        p.kicks += 1;
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 ES 2.0 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_npot_repeat.rs: addresses keep counting up */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- GL plumbing -------------------------------------------------------------------- */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn no_error(what: &str) {
    let e = err();
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn extensions() -> Vec<String> {
    // SAFETY: plain GL call; the string is NUL-terminated and static.
    let s = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char).to_str().unwrap().to_owned() };
    s.split(' ').map(str::to_owned).collect()
}

fn listed(name: &str) -> bool {
    extensions().iter().any(|e| e == name)
}

/// A new 2D texture bound to unit 0, NEAREST, CLAMP_TO_EDGE.
fn new_texture() -> GLuint {
    let mut t: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    t
}

/// Specify level `l` of the bound texture, no data; the call's error.
fn level(l: GLint, fmt: GLenum, ty: GLenum, w: GLsizei, h: GLsizei) -> GLenum {
    // SAFETY: a plain GL call; no client data.
    unsafe { glTexImage2D(GL_TEXTURE_2D, l, fmt as GLint, w, h, 0, fmt, ty, core::ptr::null()) };
    err()
}

/// Specify the whole chain of `w` x `h`, down to 1x1; its level count.
fn chain(fmt: GLenum, ty: GLenum, w: GLsizei, h: GLsizei) -> GLint {
    let mut n = 0;
    let (mut lw, mut lh) = (w, h);
    loop {
        let e = level(n, fmt, ty, lw, lh);
        assert_eq!(e, GL_NO_ERROR, "level {n} of {w}x{h} 0x{fmt:x}/0x{ty:x}: 0x{e:x}");
        n += 1;
        if lw == 1 && lh == 1 {
            return n;
        }
        lw = (lw >> 1).max(1);
        lh = (lh >> 1).max(1);
    }
}

/// The status of a new FBO with texture `t` level `l` as colour attachment 0,
/// and the attach call's own error.
fn status_of(t: GLuint, l: GLint) -> (GLenum, GLenum) {
    let mut fb: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, l);
        let e = glGetError();
        let s = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        (s, e)
    }
}

const TEX_FORMATS: [(&str, GLenum, GLenum); 3] = [
    ("RGBA/UNSIGNED_BYTE", GL_RGBA, GL_UNSIGNED_BYTE),
    ("RGB/UNSIGNED_SHORT_5_6_5", GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
    ("RGBA/UNSIGNED_SHORT_4_4_4_4", GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
];

/* ---- the cases ---------------------------------------------------------------------- */

/// The extension list of a report-es2 build, exactly.
#[test]
fn the_extension_list_of_the_es2_build_is_exact() {
    on_gc7000(|| {
        let want = [
            "GL_OES_EGL_image",
            "GL_OES_standard_derivatives",
            "GL_OES_compressed_ETC1_RGB8_texture",
            "GL_OES_compressed_ETC2_RGB8_texture",
            "GL_OES_compressed_ETC2_RGBA8_texture",
            "GL_OES_vertex_array_object",
            "GL_OES_mapbuffer",
            "GL_OES_fbo_render_mipmap",
            "GL_EXT_texture_format_BGRA8888",
            "GL_OES_rgb8_rgba8",
            "GL_OES_required_internalformat",
            "GL_EXT_texture_rg",
            "GL_OES_texture_half_float",
            "GL_EXT_color_buffer_half_float",
            "GL_NV_fbo_color_attachments",
            "GL_EXT_debug_marker",
            "GL_OES_texture_npot",
            "GL_OES_depth_texture",
            "GL_OES_packed_depth_stencil",
        ];
        let got = extensions();
        assert_eq!(got, want, "GL_EXTENSIONS of the ES 2.0 build");
        no_error("glGetString");
        for absent in [
            "GL_EXT_color_buffer_float",
            "GL_OES_texture_float",
            "GL_OES_texture_float_linear",
            "GL_EXT_clip_control",
            "GL_EXT_multisampled_render_to_texture",
            "GL_EXT_sRGB",
        ] {
            assert!(!got.iter().any(|e| e == absent), "{absent} must stay unlisted");
        }
    });
}

/// Level 0 and every level of a power-of-two chain, for the three ES 2.0
/// renderable texture formats.
#[test]
fn renderable_texture_formats_are_complete_at_every_pot_level() {
    on_gc7000(|| {
        assert!(listed("GL_OES_fbo_render_mipmap"));
        for (name, fmt, ty) in TEX_FORMATS {
            for (w, h) in [(32, 32), (32, 8), (8, 32), (16, 16)] {
                let t = new_texture();
                let n = chain(fmt, ty, w, h);
                for l in 0..n {
                    let (s, e) = status_of(t, l);
                    assert_eq!(e, GL_NO_ERROR, "{name} {w}x{h} level {l}: attach error 0x{e:x}");
                    assert_eq!(s, GL_FRAMEBUFFER_COMPLETE, "{name} {w}x{h} level {l}: status 0x{s:x}");
                }
            }
        }
    });
}

/// OES_texture_npot is listed, so a non-power-of-two chain is a legal
/// texture and each of its levels a complete colour attachment.
#[test]
fn renderable_texture_formats_are_complete_at_every_npot_level() {
    on_gc7000(|| {
        assert!(listed("GL_OES_texture_npot"), "the GC7000 claims OES_texture_npot (NON_POWER_OF_TWO)");
        for (name, fmt, ty) in TEX_FORMATS {
            for (w, h) in [(30, 30), (33, 17), (5, 12), (7, 1)] {
                let t = new_texture();
                let n = chain(fmt, ty, w, h);
                for l in 0..n {
                    let (s, e) = status_of(t, l);
                    assert_eq!(e, GL_NO_ERROR, "{name} {w}x{h} level {l}: attach error 0x{e:x}");
                    assert_eq!(s, GL_FRAMEBUFFER_COMPLETE, "{name} {w}x{h} level {l}: status 0x{s:x}");
                }
            }
        }
    });
}

/// OES_texture_float is not listed: ES 2.0's glTexImage2D takes no FLOAT
/// type, whatever the format, and nothing float becomes an attachment.
#[test]
fn float_textures_are_refused_without_oes_texture_float() {
    on_gc7000(|| {
        assert!(!listed("GL_OES_texture_float"));
        assert!(!listed("GL_EXT_color_buffer_float"));
        for fmt in [GL_RGBA, GL_RGB, GL_LUMINANCE, GL_ALPHA, GL_LUMINANCE_ALPHA] {
            let t = new_texture();
            let e = level(0, fmt, GL_FLOAT, 4, 4);
            assert_eq!(e, GL_INVALID_ENUM, "format 0x{fmt:x} with GL_FLOAT");
            /* no image was made: the texture is no complete attachment */
            let (s, _) = status_of(t, 0);
            assert_ne!(s, GL_FRAMEBUFFER_COMPLETE, "format 0x{fmt:x}: a texture with no image");
        }
        /* GL_RGBA32F storage is ES 3.0's, no ES 2.0 internal format */
        let t = new_texture();
        // SAFETY: a plain GL call; no client data.
        unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA32F as GLint, 4, 4, 0, GL_RGBA, GL_FLOAT, core::ptr::null()) };
        assert_ne!(err(), GL_NO_ERROR, "RGBA32F storage in an ES 2.0 context");
        let (s, _) = status_of(t, 0);
        assert_ne!(s, GL_FRAMEBUFFER_COMPLETE);
    });
}

/// OES_texture_half_float and EXT_color_buffer_half_float are listed: RGBA
/// and RGB HALF_FLOAT_OES textures are colour-renderable (the second
/// extension's RGBA16F_EXT and RGB16F_EXT), at level 0 and above.
#[test]
fn half_float_textures_are_colour_attachments_as_listed() {
    on_gc7000(|| {
        assert!(listed("GL_OES_texture_half_float") && listed("GL_EXT_color_buffer_half_float"));
        for (name, fmt) in [("RGBA", GL_RGBA), ("RGB", GL_RGB)] {
            let t = new_texture();
            let n = chain(fmt, GL_HALF_FLOAT_OES, 16, 16);
            for l in 0..n {
                let (s, e) = status_of(t, l);
                assert_eq!(e, GL_NO_ERROR, "{name}/HALF_FLOAT_OES level {l}: attach error 0x{e:x}");
                assert_eq!(s, GL_FRAMEBUFFER_COMPLETE, "{name}/HALF_FLOAT_OES level {l}: status 0x{s:x}");
            }
        }
    });
}

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nuniform vec4 u;\nvoid main() { gl_FragColor = u; }\n";
static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

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

fn program() -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform4f(glGetUniformLocation(p, c"u".as_ptr()), 1.0, 0.0, 0.0, 1.0);
        p
    }
}

/// GL_OES_fbo_render_mipmap is listed, so a clear and a draw into level 1
/// (and the last level) of a chain are not refused and raise no error.
#[test]
fn a_draw_into_a_level_above_zero_is_not_refused() {
    on_gc7000(|| {
        let _p = program();
        for (name, fmt, ty) in TEX_FORMATS {
            for (w, h) in [(16, 16), (24, 10)] {
                let t = new_texture();
                let n = chain(fmt, ty, w, h);
                for l in [1, n - 1] {
                    let (mut lw, mut lh) = (w, h);
                    for _ in 0..l {
                        lw = (lw >> 1).max(1);
                        lh = (lh >> 1).max(1);
                    }
                    let before = gc_refused_draws();
                    let mut fb: GLuint = 0;
                    // SAFETY: plain GL calls; QUAD is a static.
                    unsafe {
                        glGenFramebuffers(1, &mut fb);
                        glBindFramebuffer(GL_FRAMEBUFFER, fb);
                        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, l);
                        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
                        glViewport(0, 0, lw, lh);
                        glClearColor(0.0, 0.0, 1.0, 1.0);
                        glClear(GL_COLOR_BUFFER_BIT);
                        glEnableVertexAttribArray(0);
                        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
                        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
                    }
                    no_error(&format!("{name} {w}x{h} level {l} clear and draw"));
                    // SAFETY: plain GL calls.
                    unsafe {
                        glFinish();
                        glBindFramebuffer(GL_FRAMEBUFFER, 0);
                        glDeleteFramebuffers(1, &fb);
                    }
                    no_error(&format!("{name} {w}x{h} level {l} finish"));
                    assert_eq!(gc_refused_draws(), before, "{name} {w}x{h}: the draw into level {l} was refused");
                }
            }
        }
        assert!(with_platform(|p| p.kicks) > 0, "the draws reached the back end");
    });
}
