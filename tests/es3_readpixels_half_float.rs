// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The glReadPixels contract of the 16F colour targets as it is today, on
//! every GPU's ES 3.0 build (plan #360, test-rp-half): what the core-claim
//! change of `GL_EXT_color_buffer_float` must keep for the half-float side.
//!
//! `GL_EXT_color_buffer_half_float` is claimed in the ES 3.0 build
//! (src/gl/state.rs), so RGBA16F, RG16F and R16F are complete colour targets
//! as textures and as renderbuffers (tests/es3_float_target.rs). For each:
//!
//! - RGBA/FLOAT, the pair ES 3.0.6 4.3.2 names for a floating-point colour
//!   buffer, is accepted;
//! - `GL_IMPLEMENTATION_COLOR_READ_FORMAT` / `_TYPE` is RGBA/UNSIGNED_BYTE
//!   (src/gl/framebuffer.rs `implementation_color_read`), a pair of table 3.2,
//!   and glReadPixels takes it: the second pair 4.3.2 allows;
//! - every other pair of the table 3.2 format and type names is
//!   GL_INVALID_OPERATION ("the combination of format and type is
//!   unsupported"), and a name that is no format or type GL_INVALID_ENUM;
//! - a texture level the CPU wrote (glTexImage2D with GL_HALF_FLOAT) reads
//!   back through both pairs: RGBA/FLOAT gives the stored halves as floats,
//!   the channels the format lacks as G = 0, B = 0, A = 1 (4.3.2's
//!   conversion), and RGBA/UNSIGNED_BYTE the same values clamped to [0, 1].
//!   With level 1 attached to the READ framebuffer alone, the read comes
//!   from that level.
//!
//! The back end is a stand-in that draws nothing and keeps no image of its
//! own, so the GL core reads the texture's own (linear) storage on every GPU
//! build; the renderbuffers are only checked for the pairs, not values (no
//! clear reaches their storage here; tests/es3_readback_v3d.rs reads them
//! after a clear over the real V3D back end).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_readpixels_half_float
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_readpixels_half_float
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_readpixels_half_float
//! ```
//!
//! Not in a `report-es2` build: ES 2.0 has none of the sized float names.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::{glGetError, glGetIntegerv, glGetString, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host ---------------------------------------------- */

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
    name: c"es3-readpixels-half-float".as_ptr() as *const c_char,
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
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The GPU this build is for, as the messages name it.
const GPU: &str = if cfg!(feature = "gpu-powervr") {
    "powervr"
} else if cfg!(feature = "gpu-vivante") {
    "vivante"
} else if cfg!(feature = "gpu-v3d") {
    "v3d"
} else {
    "other"
};

/// A 16F colour format: its sized name, the unpack format glTexImage2D takes
/// with GL_HALF_FLOAT, its channel count and a label.
#[derive(Clone, Copy)]
struct Half {
    internalformat: GLenum,
    format: GLenum,
    channels: usize,
    label: &'static str,
}

/// The three 16F formats complete as both a texture and a renderbuffer today
/// (RGB16F's texture is not: tests/es3_float_target.rs).
const HALVES: [Half; 3] = [
    Half { internalformat: GL_RGBA16F, format: GL_RGBA, channels: 4, label: "RGBA16F" },
    Half { internalformat: GL_RG16F, format: GL_RG, channels: 2, label: "RG16F" },
    Half { internalformat: GL_R16F, format: GL_RED, channels: 1, label: "R16F" },
];

/// ES 3.0.6 table 3.2's format names (all but the depth ones go to
/// glReadPixels; those are refused as a pair, not as an unknown name).
const FORMATS: [GLenum; 13] = [
    GL_RGBA,
    GL_RGB,
    GL_RG,
    GL_RED,
    GL_RGBA_INTEGER,
    GL_RGB_INTEGER,
    GL_RG_INTEGER,
    GL_RED_INTEGER,
    GL_DEPTH_COMPONENT,
    GL_DEPTH_STENCIL,
    GL_LUMINANCE_ALPHA,
    GL_LUMINANCE,
    GL_ALPHA,
];

/// ES 3.0.6 table 3.2's type names.
const TYPES: [GLenum; 16] = [
    GL_UNSIGNED_BYTE,
    GL_BYTE,
    GL_UNSIGNED_SHORT,
    GL_SHORT,
    GL_UNSIGNED_INT,
    GL_INT,
    GL_HALF_FLOAT,
    GL_FLOAT,
    GL_UNSIGNED_SHORT_5_6_5,
    GL_UNSIGNED_SHORT_4_4_4_4,
    GL_UNSIGNED_SHORT_5_5_5_1,
    GL_UNSIGNED_INT_2_10_10_10_REV,
    GL_UNSIGNED_INT_10F_11F_11F_REV,
    GL_UNSIGNED_INT_5_9_9_9_REV,
    GL_UNSIGNED_INT_24_8,
    GL_FLOAT_32_UNSIGNED_INT_24_8_REV,
];

/// The values the textures hold, as halves, four a texel (the formats with
/// fewer channels take the first ones). Each is exact in a half; some lie
/// outside [0, 1] so the RGBA/UNSIGNED_BYTE read shows its clamp.
const TEXELS: [[u16; 4]; 4] = [
    [0x3C00, 0x3800, 0x3400, 0x0000], /* 1.0, 0.5, 0.25, 0.0 */
    [0x4000, 0xB800, 0x3E00, 0x3C00], /* 2.0, -0.5, 1.5, 1.0 */
    [0x0000, 0x3C00, 0xC000, 0x3800], /* 0.0, 1.0, -2.0, 0.5 */
    [0x3A00, 0x3000, 0x4200, 0x3400], /* 0.75, 0.125, 3.0, 0.25 */
];

/// A half as the float it stands for (the values of [`TEXELS`] only: normal
/// numbers and zero).
fn half(h: u16) -> f32 {
    if h & 0x7fff == 0 {
        return if h & 0x8000 != 0 { -0.0 } else { 0.0 };
    }
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1f) as i32 - 15;
    let mant = 1.0 + (h & 0x3ff) as f32 / 1024.0;
    sign * mant * 2f32.powi(exp)
}

/// Texel `i` of a `w` x `h` image as the client hands it in: the first
/// `channels` halves of a [`TEXELS`] row, rows cycling through them.
fn client_image(f: Half, w: usize, h: usize) -> Vec<u16> {
    (0..w * h).flat_map(|i| TEXELS[i % TEXELS.len()][..f.channels].to_vec()).collect()
}

/// Texel `i` as RGBA/FLOAT reads it: the stored channels, then G = 0, B = 0,
/// A = 1 for those the format lacks (ES 3.0.6 4.3.2, table 3.13's
/// conversion).
fn expect_float(f: Half, i: usize) -> [f32; 4] {
    let mut v = [0.0, 0.0, 0.0, 1.0];
    for (ch, h) in TEXELS[i % TEXELS.len()][..f.channels].iter().enumerate() {
        v[ch] = half(*h);
    }
    v
}

/// The same texel read as RGBA/UNSIGNED_BYTE: clamped to [0, 1], scaled and
/// rounded.
fn expect_bytes(f: Half, i: usize) -> [u8; 4] {
    expect_float(f, i).map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

/// A new `w` x `h` texture of `f` with level `level` written by the CPU from
/// [`client_image`] (the levels above stay unwritten: an attachment needs
/// only its own level).
fn texture(f: Half, level: GLint, w: usize, h: usize) -> GLuint {
    let px = client_image(f, w, h);
    let mut t = 0;
    // SAFETY: room for one name; `px` holds w x h texels of `channels`
    // halves, the unpack alignment is 1.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(
            GL_TEXTURE_2D,
            level,
            f.internalformat as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            f.format,
            GL_HALF_FLOAT,
            px.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} texture level {level}", f.label);
    t
}

/// A colour attachment the test reads from: a texture or a renderbuffer of
/// `f`, attached as colour 0 of a new framebuffer bound to GL_FRAMEBUFFER.
enum Image {
    Texture(GLuint),
    Renderbuffer(GLuint),
}

struct Fb {
    fb: GLuint,
    image: Image,
}

impl Fb {
    fn texture(f: Half) -> Fb {
        let t = texture(f, 0, 16, 16);
        let mut fb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        }
        let fb = Fb { fb, image: Image::Texture(t) };
        fb.complete(f, "texture");
        fb
    }

    fn renderbuffer(f: Half) -> Fb {
        let (mut fb, mut rb) = (0, 0);
        // SAFETY: room for one name each; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, f.internalformat, 16, 16);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} renderbuffer storage", f.label);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
        let fb = Fb { fb, image: Image::Renderbuffer(rb) };
        fb.complete(f, "renderbuffer");
        fb
    }

    fn complete(&self, f: Half, kind: &str) {
        // SAFETY: plain GL call.
        let status = unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) };
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} {kind} attached", f.label);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{GPU}: {} {kind} as colour 0", f.label);
    }

    /// Run `body` once with a texture of `f` bound as the framebuffer's
    /// colour 0 and once with a renderbuffer, each made just before its turn
    /// and gone after it, with the kind's label.
    fn each(f: Half, mut body: impl FnMut(&str)) {
        for (make, kind) in [(Fb::texture as fn(Half) -> Fb, "texture"), (Fb::renderbuffer, "renderbuffer")] {
            let _fb = make(f);
            body(kind);
        }
    }
}

impl Drop for Fb {
    fn drop(&mut self) {
        // SAFETY: the names this value made; plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            match self.image {
                Image::Texture(t) => glDeleteTextures(1, &t),
                Image::Renderbuffer(rb) => glDeleteRenderbuffers(1, &rb),
            }
            while glGetError() != GL_NO_ERROR {}
        }
    }
}

/// The error of a 1 x 1 glReadPixels at (0, 0) with `format` and `type_`,
/// into room for any pixel glReadPixels writes (16 bytes at most).
fn read_error(format: GLenum, type_: GLenum) -> GLenum {
    let mut px = [0u8; 64];
    // SAFETY: 64 bytes hold one pixel of any pair; the pack alignment is 1.
    unsafe { glReadPixels(0, 0, 1, 1, format, type_, px.as_mut_ptr() as *mut c_void) };
    err()
}

/// GL_IMPLEMENTATION_COLOR_READ_FORMAT and _TYPE of the bound framebuffer.
fn implementation_pair() -> (GLenum, GLenum) {
    let (mut f, mut t): (GLint, GLint) = (0, 0);
    // SAFETY: room for one integer each.
    unsafe {
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_FORMAT, &mut f);
        glGetIntegerv(GL_IMPLEMENTATION_COLOR_READ_TYPE, &mut t);
    }
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: GL_IMPLEMENTATION_COLOR_READ_FORMAT/TYPE");
    (f as GLenum, t as GLenum)
}

/// The whole `w` x `h` read buffer as RGBA/FLOAT.
fn read_floats(w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut px = vec![[f32::NAN; 4]; w * h];
    // SAFETY: room for w x h pixels of 16 bytes, rows packed (alignment 1).
    unsafe { glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: glReadPixels RGBA/FLOAT");
    px
}

/// The whole `w` x `h` read buffer as RGBA/UNSIGNED_BYTE.
fn read_bytes(w: usize, h: usize) -> Vec<[u8; 4]> {
    let mut px = vec![[0xa5u8; 4]; w * h];
    // SAFETY: room for w x h pixels of 4 bytes, rows packed (alignment 1).
    unsafe {
        glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void)
    };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: glReadPixels RGBA/UNSIGNED_BYTE");
    px
}

/// Every pixel of a `w` x `h` read of an image of `f` written from
/// [`client_image`] is what it was given, in both pairs.
fn assert_reads_the_image(f: Half, w: usize, h: usize, what: &str) {
    for (i, got) in read_floats(w, h).into_iter().enumerate() {
        assert_eq!(got, expect_float(f, i), "{GPU}: {} {what}, pixel {i} as RGBA/FLOAT", f.label);
    }
    for (i, got) in read_bytes(w, h).into_iter().enumerate() {
        assert_eq!(got, expect_bytes(f, i), "{GPU}: {} {what}, pixel {i} as RGBA/UNSIGNED_BYTE", f.label);
    }
}

/* ---- the claim the targets rest on --------------------------------------- */

/// The ES 3.0 build lists `GL_EXT_color_buffer_half_float`: without it the
/// 16F targets below would not be complete.
#[test]
fn half_float_colour_buffers_are_claimed() {
    let _g = context();
    // SAFETY: plain GL call; the string is the build's static, NUL-terminated.
    let s = unsafe { glGetString(GL_EXTENSIONS) };
    assert!(!s.is_null(), "{GPU}: glGetString(GL_EXTENSIONS)");
    // SAFETY: non-null and NUL-terminated (above).
    let s = unsafe { CStr::from_ptr(s as *const c_char) }.to_str().unwrap();
    assert!(s.split(' ').any(|e| e == "GL_EXT_color_buffer_half_float"), "{GPU}: GL_EXTENSIONS");
}

/* ---- the pairs ------------------------------------------------------------ */

/// ES 3.0.6 4.3.2: a floating-point colour buffer takes RGBA/FLOAT.
#[test]
fn rgba_float_is_accepted() {
    let _g = context();
    for f in HALVES {
        Fb::each(f, |kind| {
            assert_eq!(read_error(GL_RGBA, GL_FLOAT), GL_NO_ERROR, "{GPU}: {} {kind}, RGBA/FLOAT", f.label);
        });
    }
}

/// The implementation pair of a 16F read buffer is RGBA/UNSIGNED_BYTE today
/// (`implementation_color_read`: every non-integer buffer), a format and
/// type of table 3.2, and glReadPixels accepts it: it is the second pair
/// 4.3.2 allows, so RGBA/UNSIGNED_BYTE is no error on a 16F buffer.
#[test]
fn the_implementation_pair_is_accepted_and_consistent() {
    let _g = context();
    for f in HALVES {
        Fb::each(f, |kind| {
            let pair = implementation_pair();
            assert_eq!(pair, (GL_RGBA, GL_UNSIGNED_BYTE), "{GPU}: {} {kind}, the implementation pair", f.label);
            assert!(
                FORMATS.contains(&pair.0) && TYPES.contains(&pair.1),
                "{GPU}: {} {kind}: a table 3.2 pair",
                f.label
            );
            assert_eq!(read_error(pair.0, pair.1), GL_NO_ERROR, "{GPU}: {} {kind}, the implementation pair", f.label);
            /* asked again after the read: the same answer */
            assert_eq!(implementation_pair(), pair, "{GPU}: {} {kind}, asked twice", f.label);
        });
    }
}

/// ES 3.0.6 4.3.2: every other combination is GL_INVALID_OPERATION, among
/// them RGBA/HALF_FLOAT (the storage's own type), RED/FLOAT and RG/FLOAT (the
/// format's own channels) and the integer pairs; a name that is no pixel
/// format or type is GL_INVALID_ENUM (2.5).
#[test]
fn every_other_pair_is_refused() {
    let _g = context();
    for f in HALVES {
        Fb::each(f, |kind| {
            let accepted = [(GL_RGBA, GL_FLOAT), implementation_pair()];
            for format in FORMATS {
                for type_ in TYPES.into_iter().filter(|&t| !accepted.contains(&(format, t))) {
                    assert_eq!(
                        read_error(format, type_),
                        GL_INVALID_OPERATION,
                        "{GPU}: {} {kind}, format 0x{format:x} type 0x{type_:x}",
                        f.label
                    );
                }
            }
            for (format, type_) in [(0x1234, GL_FLOAT), (GL_RGBA, 0x1234), (GL_RGBA16F, GL_FLOAT), (GL_RGBA, GL_RGBA)] {
                assert_eq!(
                    read_error(format, type_),
                    GL_INVALID_ENUM,
                    "{GPU}: {} {kind}, format 0x{format:x} type 0x{type_:x}",
                    f.label
                );
            }
        });
    }
}

/* ---- the values of a CPU-written image ------------------------------------ */

/// Level 0 of a texture glTexImage2D wrote with GL_HALF_FLOAT, as colour 0 of
/// the bound framebuffer, reads back what it was given.
#[test]
fn a_cpu_written_texture_reads_its_values() {
    let _g = context();
    for f in HALVES {
        let _fb = Fb::texture(f);
        assert_reads_the_image(f, 16, 16, "texture level 0");
    }
}

/// Level 1 of such a texture attached to the READ framebuffer alone (the
/// draw framebuffer stays the default one): glReadPixels reads that level,
/// 8 x 8, through the read buffer GL_COLOR_ATTACHMENT0.
#[test]
fn a_texture_level_on_the_read_framebuffer_reads_its_values() {
    let _g = context();
    for f in HALVES {
        let t = texture(f, 1, 8, 8);
        let mut fb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
            glFramebufferTexture2D(GL_READ_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 1);
            glReadBuffer(GL_COLOR_ATTACHMENT0);
            assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} level 1 on the read framebuffer", f.label);
            assert_eq!(
                glCheckFramebufferStatus(GL_READ_FRAMEBUFFER),
                GL_FRAMEBUFFER_COMPLETE,
                "{GPU}: {} level 1 as the read buffer",
                f.label
            );
        }
        assert_eq!(implementation_pair(), (GL_RGBA, GL_UNSIGNED_BYTE), "{GPU}: {} level 1", f.label);
        assert_eq!(read_error(GL_RGBA, GL_HALF_FLOAT), GL_INVALID_OPERATION, "{GPU}: {} level 1", f.label);
        assert_reads_the_image(f, 8, 8, "texture level 1 on the read framebuffer");
        // SAFETY: the names made above; plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &fb);
            glDeleteTextures(1, &t);
        }
        assert_eq!(err(), GL_NO_ERROR, "{GPU}: {} cleanup", f.label);
    }
}

/// A read past the image's edge leaves the pixels outside it as they were
/// (ES 3.0.6 4.3.2: undefined, here untouched) and fills the rest.
#[test]
fn a_read_past_the_edge_fills_only_the_inside() {
    let _g = context();
    let f = HALVES[0];
    let _fb = Fb::texture(f);
    let mut px = [[f32::NAN; 4]; 4];
    // SAFETY: room for 2 x 2 pixels of 16 bytes, rows packed (alignment 1).
    unsafe { glReadPixels(15, 15, 2, 2, GL_RGBA, GL_FLOAT, px.as_mut_ptr() as *mut c_void) };
    assert_eq!(err(), GL_NO_ERROR, "{GPU}: RGBA16F read at the corner");
    assert_eq!(px[0], expect_float(f, 15 * 16 + 15), "{GPU}: RGBA16F pixel (15, 15)");
    for (i, p) in px.iter().enumerate().skip(1) {
        assert!(p.iter().all(|v| v.is_nan()), "{GPU}: RGBA16F read pixel {i} outside the image written: {p:?}");
    }
}
