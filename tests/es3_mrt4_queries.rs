// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Framebuffer and draw-buffer queries at index 3 (fbquery4-test, plan #368),
//! through the GL API and the real GL core, under the four-target stand-in
//! `backend::HOST_DRAW_TARGETS` = 4 (RC14 of docs/es3-cts-failures.md), and
//! the same calls at 3 (today's answer). The neighbours
//! (es3_mrt4_gl_v3d.rs, es3_mrt4_gl_pvr.rs) hold the limits, GL_DRAW_BUFFER0-3
//! after a plain list of four, n = 5, GL_READ_BUFFER and the attach of
//! attachment 3 / 4; this file holds the rest of the query surface:
//!
//! - glGetFramebufferAttachmentParameteriv on GL_COLOR_ATTACHMENT3, every
//!   ES 3.0 pname, for RGBA8, SRGB8_ALPHA8, RGBA16F, RGBA8UI and RGB10_A2 as
//!   renderbuffer and as texture (ES 3.0 6.1.13: the object type/name, level /
//!   cube face / layer for a texture and INVALID_ENUM for a renderbuffer,
//!   component sizes, component type, colour encoding);
//! - GL_DRAW_BUFFER3 after glDrawBuffers lists with and without gaps;
//! - the glDrawBuffers errors (4.2.1): COLOR_ATTACHMENT4, an attachment out
//!   of place, GL_BACK on an FBO; the failed call leaves the list alone;
//! - GL_READ_BUFFER after glReadBuffer(GL_COLOR_ATTACHMENT3);
//! - the same at 3: attachment 3 is INVALID_ENUM / INVALID_OPERATION.
//!
//! The queries reach no back end draw, so one stand-in back end serves both
//! features. A defect is an `#[ignore]` naming the file:line at fault.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_queries
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_queries
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    any(feature = "gpu-v3d", feature = "gpu-powervr"),
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use core::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-mrt4-queries".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// A context whose back end answers `targets` draw targets.
fn context(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one int.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn draw_buffers(list: &[GLenum]) {
    // SAFETY: `list` is as long as it says.
    unsafe { glDrawBuffers(list.len() as GLsizei, list.as_ptr()) };
}

const ATT: [GLenum; 5] =
    [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT4];

/// A bound framebuffer object with nothing attached.
struct Fbo(GLuint);

fn fbo() -> Fbo {
    let mut fb = 0;
    // SAFETY: one name.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    Fbo(fb)
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: a name this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.0);
        }
    }
}

/// `glGetFramebufferAttachmentParameteriv`: (error, value; -1 when untouched).
fn attach(att: GLenum, pname: GLenum) -> (GLenum, GLint) {
    let mut v = -1;
    // SAFETY: one int.
    unsafe { glGetFramebufferAttachmentParameteriv(GL_FRAMEBUFFER, att, pname, &mut v) };
    (err(), v)
}

/// One colour format's ES 3.0 answers: sizes R, G, B, A, component type, encoding.
struct Fmt {
    name: &'static str,
    internal: GLenum,
    sizes: [GLint; 4],
    ctype: GLenum,
    enc: GLenum,
}

const FORMATS: [Fmt; 5] = [
    Fmt { name: "RGBA8", internal: GL_RGBA8, sizes: [8, 8, 8, 8], ctype: GL_UNSIGNED_NORMALIZED, enc: GL_LINEAR },
    Fmt {
        name: "SRGB8_ALPHA8",
        internal: GL_SRGB8_ALPHA8,
        sizes: [8, 8, 8, 8],
        ctype: GL_UNSIGNED_NORMALIZED,
        enc: GL_SRGB,
    },
    Fmt { name: "RGBA16F", internal: GL_RGBA16F, sizes: [16, 16, 16, 16], ctype: GL_FLOAT, enc: GL_LINEAR },
    Fmt { name: "RGBA8UI", internal: GL_RGBA8UI, sizes: [8, 8, 8, 8], ctype: GL_UNSIGNED_INT, enc: GL_LINEAR },
    Fmt {
        name: "RGB10_A2",
        internal: GL_RGB10_A2,
        /* V3D has the 10/10/10/2 surface; the PowerVR back end does not
         * (rt_format_drawable(GLES_SURF_RGB10_A2) is false), so GL_RGB10_A2 is held as RGBA8 and
         * src/gl/framebuffer.rs:2321 answers 8 (docs/notes/v3d-rgb10a2.md (a)): by design, as at
         * attachment 0 */
        sizes: if cfg!(feature = "gpu-v3d") { [10, 10, 10, 2] } else { [8, 8, 8, 8] },
        ctype: GL_UNSIGNED_NORMALIZED,
        enc: GL_LINEAR,
    },
];

const SIZE_PNAMES: [GLenum; 4] = [
    GL_FRAMEBUFFER_ATTACHMENT_RED_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_GREEN_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_BLUE_SIZE,
    GL_FRAMEBUFFER_ATTACHMENT_ALPHA_SIZE,
];

/// The pnames every attached object answers alike.
fn check_common(f: &Fmt, kind: &str) {
    let what = format!("{} {kind}", f.name);
    for (i, p) in SIZE_PNAMES.iter().enumerate() {
        assert_eq!(attach(GL_COLOR_ATTACHMENT3, *p), (GL_NO_ERROR, f.sizes[i]), "{what}: size {i}");
    }
    assert_eq!(
        attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_DEPTH_SIZE),
        (GL_NO_ERROR, 0),
        "{what}: depth size"
    );
    assert_eq!(
        attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE),
        (GL_NO_ERROR, 0),
        "{what}: stencil size"
    );
    assert_eq!(
        attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE),
        (GL_NO_ERROR, f.ctype as GLint),
        "{what}: component type"
    );
    assert_eq!(
        attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING),
        (GL_NO_ERROR, f.enc as GLint),
        "{what}: colour encoding"
    );
}

#[test]
fn attachment3_renderbuffer_every_pname() {
    let _g = context(4);
    for f in &FORMATS {
        let _f = fbo();
        let mut rb = 0;
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f.internal, 8, 8);
            assert_eq!(err(), GL_NO_ERROR, "{} storage", f.name);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb);
        }
        assert_eq!(err(), GL_NO_ERROR, "{} attach", f.name);
        assert_eq!(
            attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE),
            (GL_NO_ERROR, GL_RENDERBUFFER as GLint),
            "{}",
            f.name
        );
        assert_eq!(
            attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME),
            (GL_NO_ERROR, rb as GLint),
            "{}",
            f.name
        );
        /* level, cube face and layer belong to a texture: ES 3.0 6.1.13 says INVALID_ENUM */
        for p in [
            GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL,
            GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE,
            GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER,
        ] {
            assert_eq!(attach(GL_COLOR_ATTACHMENT3, p).0, GL_INVALID_ENUM, "{} renderbuffer {p:#x}", f.name);
        }
        check_common(f, "renderbuffer");
        // SAFETY: a name this test made.
        unsafe { glDeleteRenderbuffers(1, &rb) };
    }
}

#[test]
fn attachment3_texture_every_pname() {
    let _g = context(4);
    for f in &FORMATS {
        let _f = fbo();
        let mut tex = 0;
        // SAFETY: plain GL calls on names this test made.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexStorage2D(GL_TEXTURE_2D, 3, f.internal, 8, 8);
            assert_eq!(err(), GL_NO_ERROR, "{} storage", f.name);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_TEXTURE_2D, tex, 1);
        }
        assert_eq!(err(), GL_NO_ERROR, "{} attach", f.name);
        assert_eq!(
            attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE),
            (GL_NO_ERROR, GL_TEXTURE as GLint),
            "{}",
            f.name
        );
        assert_eq!(attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), (GL_NO_ERROR, tex as GLint));
        assert_eq!(
            attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LEVEL),
            (GL_NO_ERROR, 1),
            "{}",
            f.name
        );
        /* not a cube map, not layered: zero */
        assert_eq!(
            attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE),
            (GL_NO_ERROR, 0),
            "{}",
            f.name
        );
        assert_eq!(
            attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER),
            (GL_NO_ERROR, 0),
            "{}",
            f.name
        );
        check_common(f, "texture");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }
}

#[test]
fn attachment3_cube_face_and_layer() {
    let _g = context(4);
    let _f = fbo();
    let (mut cube, mut arr) = (0, 0);
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glGenTextures(1, &mut cube);
        glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
        glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 8, 8);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_TEXTURE_CUBE_MAP_NEGATIVE_Z, cube, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(
        attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_CUBE_MAP_FACE),
        (GL_NO_ERROR, GL_TEXTURE_CUBE_MAP_NEGATIVE_Z as GLint)
    );
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glGenTextures(1, &mut arr);
        glBindTexture(GL_TEXTURE_2D_ARRAY, arr);
        glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, GL_RGBA8, 8, 8, 4);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, arr, 0, 2);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_TEXTURE_LAYER), (GL_NO_ERROR, 2));
    assert_eq!(attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), (GL_NO_ERROR, arr as GLint));
    // SAFETY: names this test made.
    unsafe {
        glDeleteTextures(1, &cube);
        glDeleteTextures(1, &arr);
    }
}

#[test]
fn attachment3_empty_is_none() {
    let _g = context(4);
    let _f = fbo();
    assert_eq!(attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE), (GL_NO_ERROR, GL_NONE as GLint));
    assert_eq!(attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME), (GL_NO_ERROR, 0));
    /* attachment 4 is no attachment point at 4 targets */
    assert_eq!(attach(GL_COLOR_ATTACHMENT4, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE).0, GL_INVALID_ENUM);
}

/* ---- glDrawBuffers / GL_DRAW_BUFFER3 / GL_READ_BUFFER ----------------------- */

#[test]
fn draw_buffer3_lists_with_and_without_gaps() {
    let _g = context(4);
    let _f = fbo();
    let n = GL_NONE;
    let cases: [([GLenum; 4], GLenum); 6] = [
        ([ATT[0], ATT[1], ATT[2], ATT[3]], ATT[3]),
        ([n, n, n, ATT[3]], ATT[3]),
        ([ATT[0], n, n, ATT[3]], ATT[3]),
        ([ATT[0], ATT[1], ATT[2], n], n),
        ([n, n, n, n], n),
        ([ATT[0], n, ATT[2], n], n),
    ];
    for (list, d3) in cases {
        draw_buffers(&list);
        assert_eq!(err(), GL_NO_ERROR, "{list:#x?}");
        for (i, l) in list.iter().enumerate() {
            assert_eq!(geti(GL_DRAW_BUFFER0 + i as GLenum), *l as GLint, "{list:#x?} DRAW_BUFFER{i}");
        }
        assert_eq!(geti(GL_DRAW_BUFFER3), d3 as GLint, "{list:#x?}");
    }
    /* a shorter list resets the later slots to GL_NONE */
    draw_buffers(&ATT[..4]);
    draw_buffers(&[ATT[0], ATT[1]]);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(geti(GL_DRAW_BUFFER2), GL_NONE as GLint);
    assert_eq!(geti(GL_DRAW_BUFFER3), GL_NONE as GLint);
}

/// ES 3.0 6.1.? table 6.?: the initial draw buffers of a framebuffer object are
/// COLOR_ATTACHMENT0 and then GL_NONE.
#[test]
#[ignore = "defect: src/gl/framebuffer.rs:184-185 (draw_buffer) answers COLOR_ATTACHMENTi for an object with no draw-buffer state yet; ES 3.0 says GL_NONE past buffer 0 (the doc comment at :176 records the choice)"]
fn fresh_fbo_draw_buffer3_is_none() {
    let _g = context(4);
    let _f = fbo();
    assert_eq!(geti(GL_DRAW_BUFFER0), GL_COLOR_ATTACHMENT0 as GLint);
    assert_eq!(geti(GL_DRAW_BUFFER3), GL_NONE as GLint);
}

#[test]
fn draw_buffers_errors_at_four() {
    let _g = context(4);
    let _f = fbo();
    draw_buffers(&ATT[..4]);
    assert_eq!(err(), GL_NO_ERROR);
    let n = GL_NONE;
    let bad: [(&str, Vec<GLenum>, GLenum); 6] = [
        ("n = 5", ATT.to_vec(), GL_INVALID_VALUE),
        ("ATT4 alone", vec![ATT[4]], GL_INVALID_OPERATION),
        ("ATT4 at index 3", vec![ATT[0], ATT[1], ATT[2], ATT[4]], GL_INVALID_OPERATION),
        ("ATT1 at index 0", vec![ATT[1]], GL_INVALID_OPERATION),
        ("ATT2 at index 3", vec![n, n, n, ATT[2]], GL_INVALID_OPERATION),
        ("GL_BACK on an FBO", vec![GL_BACK], GL_INVALID_OPERATION),
    ];
    for (what, list, e) in bad {
        draw_buffers(&list);
        assert_eq!(err(), e, "{what}");
        /* the failed call left the list alone */
        for i in 0..4 {
            assert_eq!(geti(GL_DRAW_BUFFER0 + i), (GL_COLOR_ATTACHMENT0 + i) as GLint, "{what}: DRAW_BUFFER{i}");
        }
    }
    /* GL_BACK as the fourth entry */
    draw_buffers(&[ATT[0], ATT[1], ATT[2], GL_BACK]);
    assert_eq!(err(), GL_INVALID_OPERATION);
    /* the default framebuffer: one entry, BACK or NONE */
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    draw_buffers(&[GL_BACK, GL_NONE]);
    assert_eq!(err(), GL_INVALID_OPERATION, "default framebuffer n = 2");
    draw_buffers(&[ATT[3]]);
    assert_eq!(err(), GL_INVALID_OPERATION, "default framebuffer ATT3");
}

#[test]
fn read_buffer_attachment3() {
    let _g = context(4);
    let _f = fbo();
    for a in ATT[..4].iter().rev() {
        // SAFETY: plain GL call.
        unsafe { glReadBuffer(*a) };
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(geti(GL_READ_BUFFER), *a as GLint);
    }
    // SAFETY: plain GL calls.
    unsafe {
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        glReadBuffer(GL_BACK);
    }
    assert_eq!(err(), GL_INVALID_OPERATION, "GL_BACK on an FBO");
    assert_eq!(geti(GL_READ_BUFFER), GL_COLOR_ATTACHMENT3 as GLint, "a failed glReadBuffer changes nothing");
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(GL_NONE) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(geti(GL_READ_BUFFER), GL_NONE as GLint);
}

/* ---- the same calls at three: ES 3.0 says what ---------------------------- */

#[test]
fn at_three_attachment3_is_out_of_range() {
    let _g = context(3);
    let _f = fbo();
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 3);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 3);
    draw_buffers(&ATT[..4]);
    assert_eq!(err(), GL_INVALID_VALUE, "n = 4");
    draw_buffers(&ATT[..3]);
    assert_eq!(err(), GL_NO_ERROR);
    /* COLOR_ATTACHMENTm, m >= MAX_COLOR_ATTACHMENTS */
    draw_buffers(&[ATT[3]]);
    assert_eq!(err(), GL_INVALID_OPERATION, "ATT3 alone");
    /* the draw-buffer state has no slot 3 */
    assert_eq!(geti(GL_DRAW_BUFFER2), GL_COLOR_ATTACHMENT2 as GLint);
    let _ = geti(GL_DRAW_BUFFER3);
    assert_eq!(err(), GL_INVALID_ENUM, "GL_DRAW_BUFFER3");
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT3) };
    assert_eq!(err(), GL_INVALID_OPERATION, "glReadBuffer ATT3");
    assert_eq!(geti(GL_READ_BUFFER), GL_COLOR_ATTACHMENT0 as GLint);
    /* the attachment point does not exist */
    assert_eq!(attach(GL_COLOR_ATTACHMENT3, GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE).0, GL_INVALID_ENUM);
    let mut rb = 0;
    // SAFETY: plain GL calls on a name this test made.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rb);
    }
    assert_eq!(err(), GL_INVALID_ENUM, "attach ATT3");
    // SAFETY: a name this test made.
    unsafe { glDeleteRenderbuffers(1, &rb) };
}
