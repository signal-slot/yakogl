// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Does a pending glClearBufferfv follow an attachment change? GL_RGBA8 and
//! GL_RGB10_A2 attachments on the V3D host back end, through the GL entry
//! points only (plan #370, clear-rebind-rgba8).
//!
//! ES 3.0 4.2.3: a clear affects the buffers of the framebuffer as bound at
//! the call. The GL core only records a clear and sends it at the next
//! flush (a glReadPixels, glFinish, draw), and glFramebufferTexture2D /
//! glFramebufferTextureLayer / glFramebufferRenderbuffer do not flush it
//! first, so the clear lands on whatever is attached at the flush. The
//! survey of `es3_rgb10a2_layers_v3d` (`clear_survives_reattach`) found this
//! by reading the code; these rows run it for both formats and for three
//! kinds of attachment: two 2D textures, two layers of a 2D-array texture
//! and two renderbuffers. Rows named `_control` read between the clear and
//! the rebind and must pass. Rows that show the defect are `#[ignore]`d with
//! the values observed.
//!
//! The stand-in back end applies a clear draw to a `GLES_SURF_RGBA8888` or a
//! `GLES_SURF_RGB10_A2` target (see `b_draw`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_clear_rebind_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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

/// The stand-in GPU: a clear draw into a `GLES_SURF_RGBA8888` target writes
/// the clear colour as bytes (`clamp(v, 0, 1) * 255` rounded half up, R first
/// as `rt_rgba8888_is_bgra` is false), into a `GLES_SURF_RGB10_A2` target as
/// its 10/10/10/2 word (R, G, B over 1023, A over 3, rounded half up), inside
/// the draw's scissor box and for the channels of `clear_color_mask` only.
unsafe extern "C" fn b_draw(d: *const backend::GlesDraw) -> i32 {
    let d = &*d;
    if !d.do_clear || d.target_gpu == 0 {
        return 0;
    }
    let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n + 0.5) as u32;
    let c = d.clear_rgba;
    let pitch = backend::draw_pitch(d) as usize;
    let [sx, sy, sw, sh] = if d.scissor { d.scissor_rect } else { [0, 0, d.target_w as i32, d.target_h as i32] };
    let on = |i: usize| d.clear_color_mask >> i & 1 != 0;
    if d.target_format == backend::GLES_SURF_RGB10_A2 {
        let new = [q(c[0], 1023.0), q(c[1], 1023.0), q(c[2], 1023.0), q(c[3], 3.0)];
        let base = d.target_gpu as *mut u32;
        for y in sy.max(0)..(sy + sh).min(d.target_h as i32) {
            for x in sx.max(0)..(sx + sw).min(d.target_w as i32) {
                let p = base.add(y as usize * pitch + x as usize);
                let w = *p;
                let old = [w & 0x3ff, w >> 10 & 0x3ff, w >> 20 & 0x3ff, w >> 30];
                let f: Vec<u32> = (0..4).map(|i| if on(i) { new[i] } else { old[i] }).collect();
                *p = f[0] | f[1] << 10 | f[2] << 20 | f[3] << 30;
            }
        }
    } else if d.target_format == backend::GLES_SURF_RGBA8888 {
        let base = d.target_gpu as *mut u8;
        for y in sy.max(0)..(sy + sh).min(d.target_h as i32) {
            for x in sx.max(0)..(sx + sw).min(d.target_w as i32) {
                let p = base.add((y as usize * pitch + x as usize) * 4);
                for (i, &ci) in c.iter().enumerate() {
                    if on(i) {
                        *p.add(i) = q(ci, 255.0) as u8;
                    }
                }
            }
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
    name: c"es3-clear-rebind".as_ptr() as *const c_char,
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
        glDisable(GL_SCISSOR_TEST);
        glColorMask(1, 1, 1, 1);
        glClearColor(0.0, 0.0, 0.0, 0.0);
        glFinish();
    }
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const N: GLsizei = 4;

#[derive(Clone, Copy, Debug)]
enum Fmt {
    Rgba8,
    Rgb10A2,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Tex2d,
    Layer,
    Rb,
}

/// What both images hold before the test clears the first one.
const SENT: [f32; 4] = [0.9, 0.1, 0.5, 1.0];
/// The clear under test.
const CLEAR: [f32; 4] = [0.2, 0.6, 0.8, 2.0 / 3.0];

fn scale(v: u32, max: u32) -> u8 {
    ((v * 255 + max / 2) / max) as u8
}

/// The bytes glReadPixels RGBA/UNSIGNED_BYTE gives for a pixel cleared to `c`.
fn bytes(fmt: Fmt, c: [f32; 4]) -> [u8; 4] {
    let q = |v: f32, n: f32| (v.clamp(0.0, 1.0) * n + 0.5) as u32;
    match fmt {
        Fmt::Rgba8 => [q(c[0], 255.0) as u8, q(c[1], 255.0) as u8, q(c[2], 255.0) as u8, q(c[3], 255.0) as u8],
        Fmt::Rgb10A2 => [
            scale(q(c[0], 1023.0), 1023),
            scale(q(c[1], 1023.0), 1023),
            scale(q(c[2], 1023.0), 1023),
            scale(q(c[3], 3.0), 3),
        ],
    }
}

/// The upload type glTexImage* accepts for the format.
fn upload_type(fmt: Fmt) -> GLenum {
    match fmt {
        Fmt::Rgba8 => GL_UNSIGNED_BYTE,
        Fmt::Rgb10A2 => GL_UNSIGNED_INT_2_10_10_10_REV,
    }
}

fn internal(fmt: Fmt) -> GLenum {
    match fmt {
        Fmt::Rgba8 => GL_RGBA8,
        Fmt::Rgb10A2 => GL_RGB10_A2,
    }
}

/// Two images of `kind` in one format (two textures, two layers of one
/// texture, or two renderbuffers) and the framebuffer they are attached to
/// in turn.
struct Pair {
    kind: Kind,
    fb: GLuint,
    names: [GLuint; 2],
}

impl Pair {
    fn new(fmt: Fmt, kind: Kind) -> Pair {
        let mut p = Pair { kind, fb: 0, names: [0; 2] };
        // SAFETY: room for the names; the uploads are null (storage only).
        unsafe {
            match kind {
                Kind::Tex2d => {
                    glGenTextures(2, p.names.as_mut_ptr());
                    for t in p.names {
                        glBindTexture(GL_TEXTURE_2D, t);
                        let (i, w) = (internal(fmt) as GLint, core::ptr::null());
                        glTexImage2D(GL_TEXTURE_2D, 0, i, N, N, 0, GL_RGBA, upload_type(fmt), w);
                        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                    }
                }
                Kind::Layer => {
                    glGenTextures(1, p.names.as_mut_ptr());
                    glBindTexture(GL_TEXTURE_2D_ARRAY, p.names[0]);
                    let (i, w) = (internal(fmt) as GLint, core::ptr::null());
                    glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, i, N, N, 2, 0, GL_RGBA, upload_type(fmt), w);
                    glTexParameteri(GL_TEXTURE_2D_ARRAY, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                }
                Kind::Rb => {
                    glGenRenderbuffers(2, p.names.as_mut_ptr());
                    for r in p.names {
                        glBindRenderbuffer(GL_RENDERBUFFER, r);
                        glRenderbufferStorage(GL_RENDERBUFFER, internal(fmt), N, N);
                    }
                }
            }
            glGenFramebuffers(1, &mut p.fb);
            glBindFramebuffer(GL_FRAMEBUFFER, p.fb);
        }
        assert_eq!(err(), GL_NO_ERROR, "setup {fmt:?} {kind:?}");
        p.attach(0);
        // SAFETY: plain GL calls.
        unsafe {
            glDrawBuffers(1, [GL_COLOR_ATTACHMENT0].as_ptr());
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{fmt:?} {kind:?}");
        }
        p
    }

    fn attach(&self, i: usize) {
        // SAFETY: plain GL calls on names made in `new`.
        unsafe {
            match self.kind {
                Kind::Tex2d => {
                    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, self.names[i], 0)
                }
                Kind::Layer => {
                    glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, self.names[0], 0, i as GLint)
                }
                Kind::Rb => {
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, self.names[i])
                }
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "attach {:?} {i}", self.kind);
    }

    fn clear(&self, c: [f32; 4]) {
        // SAFETY: `c` holds four floats.
        unsafe { glClearBufferfv(GL_COLOR, 0, c.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "glClearBufferfv");
    }

    /// The distinct pixels of the attached image (glReadPixels of N x N).
    fn read(&self) -> Vec<[u8; 4]> {
        let mut px = vec![0x77u8; (N * N) as usize * 4];
        // SAFETY: room for N x N RGBA8 pixels.
        unsafe { glReadPixels(0, 0, N, N, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
        assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
        let mut v: Vec<[u8; 4]> = px.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
        v.sort();
        v.dedup();
        v
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            match self.kind {
                Kind::Rb => glDeleteRenderbuffers(2, self.names.as_ptr()),
                _ => glDeleteTextures(2, self.names.as_ptr()),
            }
        }
    }
}

/// Both images are cleared to SENT and flushed. Image 0 is attached and
/// cleared to CLEAR, image 1 attached, and both read back (image 1, then
/// image 0 rebound). `between` reads image 0 right after the clear, which
/// flushes it before the rebind.
fn run(fmt: Fmt, kind: Kind, between: bool) {
    let _g = context();
    let p = Pair::new(fmt, kind);
    for i in [1, 0] {
        p.attach(i);
        p.clear(SENT);
        // SAFETY: plain GL call.
        unsafe { glFinish() };
    }
    let sent = vec![bytes(fmt, SENT)];
    let cleared = vec![bytes(fmt, CLEAR)];
    assert_eq!(p.read(), sent, "{fmt:?} {kind:?} image 0 before");
    p.attach(1);
    assert_eq!(p.read(), sent, "{fmt:?} {kind:?} image 1 before");
    p.attach(0);
    p.clear(CLEAR);
    if between {
        assert_eq!(p.read(), cleared, "{fmt:?} {kind:?} image 0 after its own clear");
    }
    p.attach(1);
    let b = p.read();
    p.attach(0);
    let a = p.read();
    assert_eq!(
        (a, b),
        (cleared, sent),
        "{fmt:?} {kind:?}: (image 0, image 1) after a clear of image 0 and a rebind to image 1"
    );
}

macro_rules! row {
    ($name:ident, $fmt:expr, $kind:expr, $between:expr) => {
        #[test]
        fn $name() {
            run($fmt, $kind, $between);
        }
    };
    ($name:ident, $fmt:expr, $kind:expr, $between:expr, $why:literal) => {
        #[test]
        #[ignore = $why]
        fn $name() {
            run($fmt, $kind, $between);
        }
    };
}

row!(rgba8_textures_control, Fmt::Rgba8, Kind::Tex2d, true);
row!(rgba8_layers_control, Fmt::Rgba8, Kind::Layer, true);
row!(rgba8_renderbuffers_control, Fmt::Rgba8, Kind::Rb, true);
row!(rgb10a2_textures_control, Fmt::Rgb10A2, Kind::Tex2d, true);
row!(rgb10a2_layers_control, Fmt::Rgb10A2, Kind::Layer, true);
row!(rgb10a2_renderbuffers_control, Fmt::Rgb10A2, Kind::Rb, true);
row!(
    rgba8_textures,
    Fmt::Rgba8,
    Kind::Tex2d,
    false,
    "defect: a pending clear lands on the attachment bound at the flush: image 0 reads [230, 26, 128, 255] (its sentinel) and image 1 reads [51, 153, 204, 170] (the clear), want the reverse"
);
row!(
    rgba8_layers,
    Fmt::Rgba8,
    Kind::Layer,
    false,
    "defect: a pending clear lands on the attachment bound at the flush: image 0 reads [230, 26, 128, 255] (its sentinel) and image 1 reads [51, 153, 204, 170] (the clear), want the reverse"
);
row!(
    rgba8_renderbuffers,
    Fmt::Rgba8,
    Kind::Rb,
    false,
    "defect: a pending clear lands on the attachment bound at the flush: image 0 reads [230, 26, 128, 255] (its sentinel) and image 1 reads [51, 153, 204, 170] (the clear), want the reverse"
);
row!(
    rgb10a2_textures,
    Fmt::Rgb10A2,
    Kind::Tex2d,
    false,
    "defect: a pending clear lands on the attachment bound at the flush: image 0 reads [230, 25, 128, 255] (its sentinel) and image 1 reads [51, 153, 204, 170] (the clear), want the reverse"
);
row!(
    rgb10a2_layers,
    Fmt::Rgb10A2,
    Kind::Layer,
    false,
    "defect: a pending clear lands on the attachment bound at the flush: image 0 reads [230, 25, 128, 255] (its sentinel) and image 1 reads [51, 153, 204, 170] (the clear), want the reverse"
);
row!(
    rgb10a2_renderbuffers,
    Fmt::Rgb10A2,
    Kind::Rb,
    false,
    "defect: a pending clear lands on the attachment bound at the flush: image 0 reads [230, 25, 128, 255] (its sentinel) and image 1 reads [51, 153, 204, 170] (the clear), want the reverse"
);
