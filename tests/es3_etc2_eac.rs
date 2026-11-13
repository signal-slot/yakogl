// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The six ES 3.0 ETC2/EAC formats the GL core decodes at upload
//! (`R11_EAC`, `SIGNED_R11_EAC`, `RG11_EAC`, `SIGNED_RG11_EAC`,
//! `RGB8_PUNCHTHROUGH_ALPHA1_ETC2`, `SRGB8_PUNCHTHROUGH_ALPHA1_ETC2`;
//! docs/notes/etc2-eac.md, src/gl/texture.rs `decoded_format`), driven through
//! the GL entry points on a host back end (plan #354).
//!
//! The decoder `gl::etc2` is the oracle (its own tests compare it with Mesa's
//! `texcompress_etc.c`); what is pinned here is the GL core around it: the
//! stored texel class and bytes, every level, cube face and array layer, the
//! ragged edge blocks, sub-images, a PIXEL_UNPACK_BUFFER source and the unpack
//! state around it, the names GL_COMPRESSED_TEXTURE_FORMATS lists, the errors a
//! decoded texture has as a compressed one, and the half-float arithmetic of
//! 11-bit codes against dEQP's tolerance. The imageSize and argument errors of
//! all eleven names are `es3_etc2_negative`.
//!
//! The PowerVR build is the lightest that samples every class the decode
//! lands in (RGBA8, SRGB8_A8, R16F, RGBA16F); `report-es2` stores none of the
//! six and lists three names (src/gl/state.rs's own pins).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    GlesHeap, YakoglBackend, GLES_TEXFMT_R16F, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8, GLES_TEXFMT_SRGB8_A8,
};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3_formats::f32_to_half;
use yakogl::gl::etc2;
use yakogl::gl::names::{self, Kind};
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
    name: c"es3-etc2-eac".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
    flush: None,
    finish: None,
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
};

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/* ---- helpers -------------------------------------------------------------- */

/// The first error of the last command, with the queue emptied.
fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe {
        let first = glGetError();
        while glGetError() != GL_NO_ERROR {}
        first
    }
}

fn bind_new(target: GLenum) -> GLuint {
    let mut name: GLuint = 0;
    // SAFETY: one name written.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    assert_eq!(error(), GL_NO_ERROR, "bind_new");
    name
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// One name under test and what the GL core stores it as.
struct Decoded {
    name: &'static str,
    id: GLenum,
    /// Bytes per 4x4 block.
    block: usize,
    /// The texel class of the stored texture and its bytes a texel.
    class: u8,
    bpp: usize,
}

const DECODED: [Decoded; 6] = [
    Decoded { name: "R11_EAC", id: GL_COMPRESSED_R11_EAC, block: 8, class: GLES_TEXFMT_R16F, bpp: 2 },
    Decoded { name: "SIGNED_R11_EAC", id: GL_COMPRESSED_SIGNED_R11_EAC, block: 8, class: GLES_TEXFMT_R16F, bpp: 2 },
    Decoded { name: "RG11_EAC", id: GL_COMPRESSED_RG11_EAC, block: 16, class: GLES_TEXFMT_RGBA16F, bpp: 8 },
    Decoded {
        name: "SIGNED_RG11_EAC",
        id: GL_COMPRESSED_SIGNED_RG11_EAC,
        block: 16,
        class: GLES_TEXFMT_RGBA16F,
        bpp: 8,
    },
    Decoded {
        name: "RGB8_PUNCHTHROUGH_ALPHA1_ETC2",
        id: GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        block: 8,
        class: GLES_TEXFMT_RGBA8,
        bpp: 4,
    },
    Decoded {
        name: "SRGB8_PUNCHTHROUGH_ALPHA1_ETC2",
        id: GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        block: 8,
        class: GLES_TEXFMT_SRGB8_A8,
        bpp: 4,
    },
];

/// `n` bytes of a repeatable pseudo-random stream (xorshift) from `seed`:
/// blocks of every mode, EAC multipliers and indices, punch-through opaque
/// bits.
fn bytes(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 11) as u8
        })
        .collect()
}

fn image_bytes(w: usize, h: usize, block: usize) -> usize {
    w.div_ceil(4) * h.div_ceil(4) * block
}

/// The half float `v` as the GL core's conversion makes it.
fn half(v: f32) -> [u8; 2] {
    f32_to_half(v).to_le_bytes()
}

/// The texels the decoder gives `data` (w x h, `d`'s blocks) in the bytes the
/// GL core stores them in: the oracle, `gl::etc2` (checked against Mesa's
/// decoder by its own tests) through the conversion docs/notes/etc2-eac.md
/// section 1 states.
fn expected(d: &Decoded, w: usize, h: usize, data: &[u8]) -> Vec<u8> {
    let signed = d.id == GL_COMPRESSED_SIGNED_R11_EAC || d.id == GL_COMPRESSED_SIGNED_RG11_EAC;
    match d.id {
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 | GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 => {
            let mut out = vec![0u8; w * h * 4];
            assert!(etc2::decode_rgb_image(w, h, data, true, &mut out));
            out
        }
        _ => {
            let channels = d.block / 8;
            let mut out = Vec::new();
            let value = |u: Option<u16>, s: Option<i16>| match (u, s) {
                (Some(v), _) => f32::from(v) / 65535.0,
                (_, Some(v)) => (f32::from(v) / 32767.0).max(-1.0),
                _ => unreachable!(),
            };
            let vals: Vec<f32> = if signed {
                let mut v = vec![0i16; w * h * channels];
                assert!(etc2::decode_eac11_signed_image(w, h, channels, data, &mut v));
                v.iter().map(|&x| value(None, Some(x))).collect()
            } else {
                let mut v = vec![0u16; w * h * channels];
                assert!(etc2::decode_eac11_unsigned_image(w, h, channels, data, &mut v));
                v.iter().map(|&x| value(Some(x), None)).collect()
            };
            for t in vals.chunks(channels) {
                for &c in t {
                    out.extend_from_slice(&half(c));
                }
                if channels == 2 {
                    /* RG16F is held as RGBA16F: (r, g, 0, 1) */
                    out.extend_from_slice(&half(0.0));
                    out.extend_from_slice(&half(1.0));
                }
            }
            out
        }
    }
}

/// The stored texels of the w x h image `layer` (a cube face, an array
/// layer) of level `level`, rows packed.
fn stored(name: GLuint, level: u32, layer: u32, w: usize, h: usize, bpp: usize) -> Vec<u8> {
    let t = texture_object(name);
    assert_eq!(t.bpp as usize, bpp, "bytes a texel");
    let (lw, lh) = ((t.width >> level).max(1), (t.height >> level).max(1));
    assert_eq!((lw as usize, lh as usize), (w, h), "level {level} size");
    let pitch = t.level_pitch(lw) as usize;
    let mut out = Vec::new();
    for r in 0..h {
        let y = layer as usize * lh as usize + r;
        let at = t.level_off[level as usize] as usize + y * pitch * bpp;
        // SAFETY: the level's rows lie inside the texture's storage.
        out.extend_from_slice(unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), w * bpp) });
    }
    out
}

fn image2d(target: GLenum, level: GLint, d: &Decoded, w: usize, h: usize, data: &[u8]) {
    // SAFETY: `data` holds imageSize bytes.
    unsafe {
        glCompressedTexImage2D(
            target,
            level,
            d.id,
            w as GLsizei,
            h as GLsizei,
            0,
            data.len() as GLsizei,
            data.as_ptr() as *const c_void,
        )
    };
    assert_eq!(error(), GL_NO_ERROR, "{}: {w}x{h} level {level}", d.name);
}

fn sub2d(d: &Decoded, level: GLint, (x, y): (usize, usize), (w, h): (usize, usize), data: &[u8]) -> GLenum {
    // SAFETY: `data` holds imageSize bytes.
    unsafe {
        glCompressedTexSubImage2D(
            GL_TEXTURE_2D,
            level,
            x as GLint,
            y as GLint,
            w as GLsizei,
            h as GLsizei,
            d.id,
            data.len() as GLsizei,
            data.as_ptr() as *const c_void,
        )
    };
    error()
}

fn each(body: impl Fn(&Decoded)) {
    let _g = context();
    for d in &DECODED {
        body(d);
    }
}

/* ---- the formats the build stores ------------------------------------------ */

/// ES 3.0 requires all ten ETC2/EAC formats, and OES_compressed_ETC1_RGB8_
/// texture's name is the eleventh. GL_NUM_COMPRESSED_TEXTURE_FORMATS and
/// GL_COMPRESSED_TEXTURE_FORMATS list exactly the stored ones.
#[test]
fn the_eleven_names_are_listed() {
    let _g = context();
    let mut n = 0;
    // SAFETY: one value written.
    unsafe { glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n) };
    assert_eq!(n, 11);
    let mut list = [0 as GLint; 16];
    // SAFETY: the list is longer than any count.
    unsafe { glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr()) };
    let mut got: Vec<GLenum> = list[..11].iter().map(|&f| f as GLenum).collect();
    got.sort_unstable();
    let mut want = vec![
        GL_ETC1_RGB8_OES,
        GL_COMPRESSED_RGB8_ETC2,
        GL_COMPRESSED_SRGB8_ETC2,
        GL_COMPRESSED_RGBA8_ETC2_EAC,
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
        GL_COMPRESSED_R11_EAC,
        GL_COMPRESSED_SIGNED_R11_EAC,
        GL_COMPRESSED_RG11_EAC,
        GL_COMPRESSED_SIGNED_RG11_EAC,
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
    ];
    want.sort_unstable();
    assert_eq!(got, want);
    assert_eq!(error(), GL_NO_ERROR);
}

/* ---- glCompressedTexImage2D ------------------------------------------------ */

/// The decoded texels of a level 0 of every size, edge blocks cut, in the
/// class the name is stored as.
#[test]
fn image2d_holds_the_decoded_texels() {
    each(|d| {
        for (w, h) in [(4, 4), (8, 8), (6, 5), (1, 1), (13, 3), (32, 16), (2, 9)] {
            let name = bind_new(GL_TEXTURE_2D);
            let data = bytes(image_bytes(w, h, d.block), (w * 31 + h) as u32);
            image2d(GL_TEXTURE_2D, 0, d, w, h, &data);
            let t = texture_object(name);
            assert_eq!((t.texfmt, t.block_bytes, t.decoded_from), (d.class, 0, d.id as u16), "{}: class", d.name);
            assert_eq!(stored(name, 0, 0, w, h, d.bpp), expected(d, w, h, &data), "{}: {w}x{h}", d.name);
        }
    });
}

/// Every level of a chain is its own image, decoded at its own size.
#[test]
fn image2d_mip_levels_are_decoded_one_by_one() {
    each(|d| {
        let name = bind_new(GL_TEXTURE_2D);
        let mut datas = Vec::new();
        for level in 0..5usize {
            let s = 16 >> level;
            let data = bytes(image_bytes(s, s, d.block), 100 + level as u32);
            image2d(GL_TEXTURE_2D, level as GLint, d, s, s, &data);
            datas.push(data);
        }
        for level in 0..5usize {
            let s = 16 >> level;
            assert_eq!(
                stored(name, level as u32, 0, s, s, d.bpp),
                expected(d, s, s, &datas[level]),
                "{}: level {level}",
                d.name
            );
        }
        assert_eq!(texture_object(name).decoded_from, d.id as u16);
    });
}

/// A cube map's faces are one call each.
#[test]
fn image2d_cube_faces() {
    each(|d| {
        let name = bind_new(GL_TEXTURE_CUBE_MAP);
        let mut datas = Vec::new();
        for face in 0..6u32 {
            let data = bytes(image_bytes(8, 8, d.block), 200 + face);
            image2d(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, d, 8, 8, &data);
            datas.push(data);
        }
        for face in 0..6u32 {
            assert_eq!(
                stored(name, 0, face, 8, 8, d.bpp),
                expected(d, 8, 8, &datas[face as usize]),
                "{}: face {face}",
                d.name
            );
        }
    });
}

/// The unpack state and the PIXEL_UNPACK_BUFFER binding are the application's
/// again after the upload, and a buffer source is decoded as client memory is.
#[test]
fn image2d_from_a_pixel_unpack_buffer_leaves_the_unpack_state() {
    each(|d| {
        let (w, h) = (8, 8);
        let data = bytes(image_bytes(w, h, d.block), 300);
        let mut buf: GLuint = 0;
        let name = bind_new(GL_TEXTURE_2D);
        // SAFETY: a name written, then `data.len()` bytes copied.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, data.len() as isize, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 7);
            glPixelStorei(GL_UNPACK_SKIP_PIXELS, 3);
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                d.id,
                w as GLsizei,
                h as GLsizei,
                0,
                data.len() as GLsizei,
                core::ptr::null(),
            );
        }
        assert_eq!(error(), GL_NO_ERROR, "{}", d.name);
        let mut v = [0 as GLint; 5];
        // SAFETY: one value each.
        unsafe {
            glGetIntegerv(GL_UNPACK_ALIGNMENT, &mut v[0]);
            glGetIntegerv(GL_UNPACK_ROW_LENGTH, &mut v[1]);
            glGetIntegerv(GL_UNPACK_SKIP_PIXELS, &mut v[2]);
            glGetIntegerv(GL_PIXEL_UNPACK_BUFFER_BINDING, &mut v[3]);
        }
        assert_eq!(v[..4], [8, 7, 3, buf as GLint], "{}: the application's state", d.name);
        assert_eq!(stored(name, 0, 0, w, h, d.bpp), expected(d, w, h, &data), "{}", d.name);
        // SAFETY: restore the defaults for the next test.
        unsafe {
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
        }
    });
}

/* ---- glCompressedTexSubImage2D --------------------------------------------- */

/// A box of whole blocks, and one that reaches the level's ragged edge, replace
/// exactly their texels.
#[test]
fn sub2d_replaces_its_box() {
    each(|d| {
        for ((w, h), (bx, by), (sw, sh)) in
            [((12, 8), (4, 4), (8, 4)), ((10, 6), (8, 4), (2, 2)), ((8, 8), (0, 0), (8, 8))]
        {
            let name = bind_new(GL_TEXTURE_2D);
            let base = bytes(image_bytes(w, h, d.block), 400);
            image2d(GL_TEXTURE_2D, 0, d, w, h, &base);
            let patch = bytes(image_bytes(sw, sh, d.block), 401);
            assert_eq!(sub2d(d, 0, (bx, by), (sw, sh), &patch), GL_NO_ERROR, "{}", d.name);
            let mut want = expected(d, w, h, &base);
            let patched = expected(d, sw, sh, &patch);
            for r in 0..sh {
                let dst = ((by + r) * w + bx) * d.bpp;
                want[dst..dst + sw * d.bpp].copy_from_slice(&patched[r * sw * d.bpp..(r + 1) * sw * d.bpp]);
            }
            assert_eq!(stored(name, 0, 0, w, h, d.bpp), want, "{}: {w}x{h} box ({bx},{by}) {sw}x{sh}", d.name);
        }
    });
}

/// ES 3.0 3.8.6: the sub-image names the format of the level's image. A
/// texture decoded from one name refuses the other names, and a texture whose
/// level 0 was respecified as an ordinary image refuses all of them.
#[test]
fn sub2d_must_name_the_decoded_format() {
    each(|d| {
        let other = DECODED.iter().find(|o| o.id != d.id).unwrap();
        bind_new(GL_TEXTURE_2D);
        let data = bytes(image_bytes(8, 8, d.block), 500);
        image2d(GL_TEXTURE_2D, 0, d, 8, 8, &data);
        let patch = bytes(image_bytes(4, 4, other.block), 501);
        assert_eq!(sub2d(other, 0, (0, 0), (4, 4), &patch), GL_INVALID_OPERATION, "{} over {}", other.name, d.name);
        let own = bytes(image_bytes(4, 4, d.block), 502);
        assert_eq!(sub2d(d, 0, (0, 0), (4, 4), &own), GL_NO_ERROR, "{}: its own name", d.name);

        /* respecified as the format it is stored as */
        let pixels = vec![0u8; 8 * 8 * 8];
        let (internal, format, type_) = match d.class {
            GLES_TEXFMT_R16F => (GL_R16F, GL_RED, GL_HALF_FLOAT),
            GLES_TEXFMT_RGBA16F => (GL_RG16F, GL_RG, GL_HALF_FLOAT),
            GLES_TEXFMT_SRGB8_A8 => (GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE),
            _ => (GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE),
        };
        // SAFETY: `pixels` is longer than the image.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 8, 8, 0, format, type_, pixels.as_ptr() as *const c_void)
        };
        assert_eq!(error(), GL_NO_ERROR, "{}", d.name);
        assert_eq!(sub2d(d, 0, (0, 0), (4, 4), &own), GL_INVALID_OPERATION, "{}: an ordinary level now", d.name);
    });
}

/// A decoded texture is an ordinary texture of its class for sampling and is
/// none to generate a chain of, to copy a framebuffer into or to render to
/// (ES 3.0 3.8.14 and 3.8.5: compressed formats).
#[test]
fn a_decoded_texture_is_not_generated_copied_or_rendered() {
    each(|d| {
        bind_new(GL_TEXTURE_2D);
        let data = bytes(image_bytes(8, 8, d.block), 600);
        image2d(GL_TEXTURE_2D, 0, d, 8, 8, &data);
        // SAFETY: plain GL calls.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        assert_eq!(error(), GL_INVALID_OPERATION, "{}: glGenerateMipmap", d.name);
        // SAFETY: plain GL call.
        unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 4, 4) };
        assert_eq!(error(), GL_INVALID_OPERATION, "{}: glCopyTexSubImage2D", d.name);
    });
}

/* ---- glCompressedTexImage3D, a 2D array ------------------------------------ */

/// The layers of a 2D array are decoded one image each, and a sub-image names
/// its layers.
#[test]
fn array_2d_layers() {
    each(|d| {
        let (w, h, layers) = (8usize, 8usize, 3usize);
        let name = bind_new(GL_TEXTURE_2D_ARRAY);
        let image = image_bytes(w, h, d.block);
        let data = bytes(image * layers, 700);
        // SAFETY: `data` holds imageSize bytes.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                d.id,
                w as GLsizei,
                h as GLsizei,
                layers as GLsizei,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{}", d.name);
        for z in 0..layers {
            assert_eq!(
                stored(name, 0, z as u32, w, h, d.bpp),
                expected(d, w, h, &data[z * image..(z + 1) * image]),
                "{}: layer {z}",
                d.name
            );
        }
        /* layer 1's right half */
        let patch = bytes(image_bytes(4, 8, d.block), 701);
        // SAFETY: `patch` holds imageSize bytes.
        unsafe {
            glCompressedTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                4,
                0,
                1,
                4,
                8,
                1,
                d.id,
                patch.len() as GLsizei,
                patch.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{}: sub-image", d.name);
        let mut want = expected(d, w, h, &data[image..2 * image]);
        let patched = expected(d, 4, 8, &patch);
        for r in 0..h {
            want[(r * w + 4) * d.bpp..(r * w + 8) * d.bpp]
                .copy_from_slice(&patched[r * 4 * d.bpp..(r + 1) * 4 * d.bpp]);
        }
        assert_eq!(stored(name, 0, 1, w, h, d.bpp), want, "{}: layer 1 patched", d.name);
        assert_eq!(stored(name, 0, 0, w, h, d.bpp), expected(d, w, h, &data[..image]), "{}: layer 0 untouched", d.name);
    });
}

/// ES 3.0 allows no ETC2/EAC 3D texture.
#[test]
fn texture_3d_is_invalid_operation() {
    each(|d| {
        bind_new(GL_TEXTURE_3D);
        let data = bytes(image_bytes(8, 8, d.block) * 2, 800);
        // SAFETY: `data` holds imageSize bytes.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_3D,
                0,
                d.id,
                8,
                8,
                2,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_INVALID_OPERATION, "{}", d.name);
    });
}

/* ---- the numbers dEQP's tolerance is judged against ------------------------ */

fn half_to_f32(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) & 1, i32::from((h >> 10) & 0x1f), u32::from(h & 0x3ff));
    let v = match e {
        0 => m as f32 * 2f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + m as f32 / 1024.0) * 2f32.powi(e - 15),
    };
    if s == 1 {
        -v
    } else {
        v
    }
}

/// docs/notes/etc2-eac.md section 3: the half float the 11-bit value is stored
/// as is within 2^-12 of the 16-bit value dEQP's reference decodes to
/// (`extend11To16`), which is 3% of the tightest tolerance texture.wrap uses
/// for these formats (0.0078585, nearest-only unsigned), and every one of the
/// 2048 (unsigned) and 2047 (signed) codes keeps a value of its own.
#[test]
fn eleven_bit_codes_survive_the_half_float() {
    const TIGHTEST: f32 = 0.007_858_5;
    let (mut prev, mut worst) = (-1.0f32, 0.0f32);
    for v in 0u32..2048 {
        let ext = (v << 5) | (v >> 6);
        let exact = ext as f32 / 65535.0;
        let stored = half_to_f32(f32_to_half(exact));
        assert!(stored > prev, "code {v}: distinct from the code below");
        prev = stored;
        worst = worst.max((stored - exact).abs());
        assert!((stored * 2047.0).round() as u32 == v, "code {v}: reads back as itself");
    }
    assert!(worst <= 2f32.powi(-12), "unsigned worst error {worst}");
    assert!(worst < TIGHTEST * 0.04, "within 4% of the tolerance");
    let mut prev = -2.0f32;
    for m in -1023i32..=1023 {
        let mag = m.unsigned_abs();
        let ext = ((mag << 5) | (mag >> 5)) as f32 / 32767.0;
        let exact = if m < 0 { -ext } else { ext }.max(-1.0);
        let stored = half_to_f32(f32_to_half(exact));
        assert!(stored > prev, "signed code {m}: distinct from the code below");
        prev = stored;
        assert!((stored - exact).abs() <= 2f32.powi(-12), "signed code {m}");
    }
}
