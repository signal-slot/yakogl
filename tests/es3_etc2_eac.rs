// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The six ES 3.0 ETC2/EAC formats the GL core decodes at upload
//! (docs/notes/etc2-eac.md, plan #354): `R11_EAC`, `SIGNED_R11_EAC`,
//! `RG11_EAC`, `SIGNED_RG11_EAC`, `RGB8_PUNCHTHROUGH_ALPHA1_ETC2` and
//! `SRGB8_PUNCHTHROUGH_ALPHA1_ETC2`, through glCompressedTexImage2D / 3D and
//! glCompressedTexSubImage2D / 3D, on the three back ends that sample the
//! classes they are stored in (R16F, RGBA16F, RGBA8, SRGB8_ALPHA8).
//!
//! What is pinned: the stored class and the stored texels of every format
//! (against `gl::etc2`'s image decoders and `es3_formats::f32_to_half`, not
//! against `etc2_store`, the code under test), mip levels, partial edge
//! blocks, a sub-image, 2D-array layers, cube faces, a PIXEL_UNPACK_BUFFER
//! source with the unpack state left alone, and the refusals a decoded
//! texture has (glTexSubImage, glCopyTexSubImage, glGenerateMipmap, another
//! compressed name, 3D, a respecify). No GPU runs: the storage is host memory.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_eac
//! ```

#![cfg(all(
    feature = "host-test",
    not(feature = "report-es2"),
    any(feature = "gpu-v3d", feature = "gpu-powervr", feature = "gpu-vivante")
))]

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
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glCompressedTexImage2D, glCompressedTexImage3D, glCompressedTexSubImage2D,
    glCompressedTexSubImage3D, glCopyTexSubImage2D, glGenTextures, glGenerateMipmap, glTexImage2D, glTexSubImage2D,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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

/* ---- the formats, their data and the texels they decode to ---------------- */

const SIX: [GLenum; 6] = [
    GL_COMPRESSED_R11_EAC,
    GL_COMPRESSED_SIGNED_R11_EAC,
    GL_COMPRESSED_RG11_EAC,
    GL_COMPRESSED_SIGNED_RG11_EAC,
    GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
    GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
];

/// (bytes per 4x4 block, bytes per stored texel, stored class) of a name.
fn shape(name: GLenum) -> (usize, usize, u8) {
    match name {
        GL_COMPRESSED_R11_EAC | GL_COMPRESSED_SIGNED_R11_EAC => (8, 2, GLES_TEXFMT_R16F),
        GL_COMPRESSED_RG11_EAC | GL_COMPRESSED_SIGNED_RG11_EAC => (16, 8, GLES_TEXFMT_RGBA16F),
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 => (8, 4, GLES_TEXFMT_RGBA8),
        GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 => (8, 4, GLES_TEXFMT_SRGB8_A8),
        _ => unreachable!("{name:#x}"),
    }
}

/// `n` bytes of a reproducible pseudo-random stream; a block of random bytes
/// is a legal block of every one of the six formats.
fn blocks(n: usize, seed: u32) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(12345);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn compressed_size(name: GLenum, w: usize, h: usize) -> usize {
    w.div_ceil(4) * h.div_ceil(4) * shape(name).0
}

fn half(v: f32) -> [u8; 2] {
    f32_to_half(v).to_le_bytes()
}

/// The stored texels of the w x h image `data` of `name`, row-major and
/// tightly packed, from `gl::etc2`'s image decoders.
fn expected(name: GLenum, w: usize, h: usize, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    match name {
        GL_COMPRESSED_R11_EAC | GL_COMPRESSED_RG11_EAC => {
            let ch = if name == GL_COMPRESSED_R11_EAC { 1 } else { 2 };
            let mut v = vec![0u16; w * h * ch];
            assert!(etc2::decode_eac11_unsigned_image(w, h, ch, data, &mut v));
            for px in v.chunks(ch) {
                for c in px {
                    out.extend(half(f32::from(*c) / 65535.0));
                }
                if ch == 2 {
                    out.extend(half(0.0));
                    out.extend(half(1.0));
                }
            }
        }
        GL_COMPRESSED_SIGNED_R11_EAC | GL_COMPRESSED_SIGNED_RG11_EAC => {
            let ch = if name == GL_COMPRESSED_SIGNED_R11_EAC { 1 } else { 2 };
            let mut v = vec![0i16; w * h * ch];
            assert!(etc2::decode_eac11_signed_image(w, h, ch, data, &mut v));
            for px in v.chunks(ch) {
                for c in px {
                    out.extend(half((f32::from(*c) / 32767.0).max(-1.0)));
                }
                if ch == 2 {
                    out.extend(half(0.0));
                    out.extend(half(1.0));
                }
            }
        }
        _ => {
            out = vec![0u8; w * h * 4];
            assert!(etc2::decode_rgb_image(w, h, data, true, &mut out));
        }
    }
    out
}

/* ---- GL helpers ------------------------------------------------------------- */

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn new_texture(target: GLenum) -> GLuint {
    let mut t = 0;
    // SAFETY: one name of room.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
    }
    t
}

fn ptr(d: &[u8]) -> *const c_void {
    d.as_ptr() as *const c_void
}

fn upload(target: GLenum, level: GLint, name: GLenum, w: usize, h: usize, data: &[u8]) {
    // SAFETY: `data` holds the image the size says.
    unsafe {
        glCompressedTexImage2D(target, level, name, w as GLsizei, h as GLsizei, 0, data.len() as GLsizei, ptr(data))
    };
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The stored texels of layer `layer` of level `level` (w x h), tightly
/// packed; a cube map's faces and an array's layers are stacked in rows.
fn stored(tex: GLuint, level: usize, layer: usize, w: usize, h: usize) -> Vec<u8> {
    let t = texture_object(tex);
    let bpp = t.bpp as usize;
    let pitch = t.level_pitch(w as u32) as usize;
    let base = t.level_off[level] as usize + layer * h * pitch * bpp;
    let mut out = Vec::new();
    for y in 0..h {
        let o = base + y * pitch * bpp;
        // SAFETY: inside the texture's host allocation; the level fits (asserted by the size).
        out.extend_from_slice(unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(o), w * bpp) });
    }
    out
}

/* ---- tests ------------------------------------------------------------------ */

#[test]
fn each_name_is_stored_as_its_class_and_decodes_to_the_reference_texels() {
    let _g = context();
    for name in SIX {
        for (w, h) in [(8usize, 8usize), (16, 4), (10, 6), (3, 5)] {
            let tex = new_texture(GL_TEXTURE_2D);
            let data = blocks(compressed_size(name, w, h), name ^ (w * 31 + h) as u32);
            upload(GL_TEXTURE_2D, 0, name, w, h, &data);
            assert_eq!(error(), GL_NO_ERROR, "{name:#x} {w}x{h}");
            let t = texture_object(tex);
            assert_eq!(t.texfmt, shape(name).2, "{name:#x}: stored class");
            assert_eq!(t.block_bytes, 0, "{name:#x}: ordinary texels, not blocks");
            assert_eq!(t.decoded_from as u32, name);
            assert_eq!((t.width as usize, t.height as usize), (w, h));
            assert_eq!(stored(tex, 0, 0, w, h), expected(name, w, h, &data), "{name:#x} {w}x{h}");
        }
    }
}

#[test]
fn mip_levels_decode_independently() {
    let _g = context();
    for name in SIX {
        let tex = new_texture(GL_TEXTURE_2D);
        let sizes = [(16usize, 16usize), (8, 8), (4, 4)];
        let mut datas = vec![];
        for (l, (w, h)) in sizes.iter().enumerate() {
            let data = blocks(compressed_size(name, *w, *h), name + l as u32);
            upload(GL_TEXTURE_2D, l as GLint, name, *w, *h, &data);
            assert_eq!(error(), GL_NO_ERROR, "{name:#x} level {l}");
            datas.push(data);
        }
        for (l, (w, h)) in sizes.iter().enumerate() {
            assert_eq!(stored(tex, l, 0, *w, *h), expected(name, *w, *h, &datas[l]), "{name:#x} level {l}");
        }
    }
}

#[test]
fn a_sub_image_replaces_whole_blocks_and_the_edge() {
    let _g = context();
    for name in SIX {
        let (w, h) = (16usize, 10usize);
        let tex = new_texture(GL_TEXTURE_2D);
        let base = blocks(compressed_size(name, w, h), 1);
        upload(GL_TEXTURE_2D, 0, name, w, h, &base);
        let mut want = expected(name, w, h, &base);
        let tb = shape(name).1;

        /* an interior box, 8x4 at (4, 4) */
        let sub = blocks(compressed_size(name, 8, 4), 2);
        // SAFETY: `sub` is the box's imageSize.
        unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 8, 4, name, sub.len() as GLsizei, ptr(&sub)) };
        assert_eq!(error(), GL_NO_ERROR, "{name:#x} interior");
        let texels = expected(name, 8, 4, &sub);
        for y in 0..4 {
            let d = ((4 + y) * w + 4) * tb;
            want[d..d + 8 * tb].copy_from_slice(&texels[y * 8 * tb..(y + 1) * 8 * tb]);
        }
        assert_eq!(stored(tex, 0, 0, w, h), want, "{name:#x} after the interior box");

        /* the bottom edge: 8x2 at (8, 8) reaches the level's edge (h = 10) */
        let sub = blocks(compressed_size(name, 8, 2), 3);
        // SAFETY: as above.
        unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 8, 8, 8, 2, name, sub.len() as GLsizei, ptr(&sub)) };
        assert_eq!(error(), GL_NO_ERROR, "{name:#x} edge");
        let texels = expected(name, 8, 2, &sub);
        for y in 0..2 {
            let d = ((8 + y) * w + 8) * tb;
            want[d..d + 8 * tb].copy_from_slice(&texels[y * 8 * tb..(y + 1) * 8 * tb]);
        }
        assert_eq!(stored(tex, 0, 0, w, h), want, "{name:#x} after the edge box");
    }
}

#[test]
fn a_2d_array_decodes_each_layer_and_a_layer_sub_image() {
    let _g = context();
    for name in SIX {
        let (w, h, d) = (8usize, 8usize, 3usize);
        let tex = new_texture(GL_TEXTURE_2D_ARRAY);
        let image = compressed_size(name, w, h);
        let data = blocks(image * d, 7);
        // SAFETY: `data` holds the d images.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                name,
                w as GLsizei,
                h as GLsizei,
                d as GLsizei,
                0,
                data.len() as GLsizei,
                ptr(&data),
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{name:#x}");
        for z in 0..d {
            assert_eq!(
                stored(tex, 0, z, w, h),
                expected(name, w, h, &data[z * image..(z + 1) * image]),
                "{name:#x} layer {z}"
            );
        }
        /* layer 1, the lower 4 rows */
        let sub = blocks(compressed_size(name, 8, 4), 9);
        // SAFETY: `sub` is the box's imageSize.
        unsafe {
            glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 4, 1, 8, 4, 1, name, sub.len() as GLsizei, ptr(&sub))
        };
        assert_eq!(error(), GL_NO_ERROR, "{name:#x} sub");
        let mut want = expected(name, w, h, &data[image..2 * image]);
        let tb = shape(name).1;
        let texels = expected(name, 8, 4, &sub);
        want[4 * w * tb..].copy_from_slice(&texels);
        assert_eq!(stored(tex, 0, 1, w, h), want, "{name:#x} layer 1 after the sub-image");
        assert_eq!(stored(tex, 0, 0, w, h), expected(name, w, h, &data[..image]), "{name:#x} layer 0 untouched");
    }
}

#[test]
fn cube_faces_decode_one_call_each() {
    let _g = context();
    for name in SIX {
        let (w, h) = (8usize, 8usize);
        let tex = new_texture(GL_TEXTURE_CUBE_MAP);
        let datas: Vec<Vec<u8>> = (0..6).map(|f| blocks(compressed_size(name, w, h), f as u32 + 40)).collect();
        for (f, d) in datas.iter().enumerate() {
            upload(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f as GLenum, 0, name, w, h, d);
            assert_eq!(error(), GL_NO_ERROR, "{name:#x} face {f}");
        }
        for (f, d) in datas.iter().enumerate() {
            assert_eq!(stored(tex, 0, f, w, h), expected(name, w, h, d), "{name:#x} face {f}");
        }
    }
}

#[test]
fn a_pixel_unpack_buffer_source_is_decoded_and_the_unpack_state_is_left_alone() {
    let _g = context();
    for name in SIX {
        let (w, h) = (8usize, 8usize);
        let tex = new_texture(GL_TEXTURE_2D);
        let data = blocks(compressed_size(name, w, h), 5);
        let mut buf = vec![0xAAu8; 16];
        buf.extend_from_slice(&data);
        let mut b = 0;
        // SAFETY: GL calls on the current context; `buf` is the buffer's size.
        unsafe {
            glGenBuffers(1, &mut b);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, b);
            glBufferData(GL_PIXEL_UNPACK_BUFFER, buf.len() as GLsizeiptr, ptr(&buf), GL_STATIC_DRAW);
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 100);
            glPixelStorei(GL_UNPACK_SKIP_PIXELS, 3);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                name,
                w as GLsizei,
                h as GLsizei,
                0,
                data.len() as GLsizei,
                16 as *const c_void,
            );
        }
        assert_eq!(error(), GL_NO_ERROR, "{name:#x}");
        let geti = |p| {
            let mut v = -1;
            // SAFETY: one value of room.
            unsafe { glGetIntegerv(p, &mut v) };
            v
        };
        assert_eq!(geti(GL_UNPACK_ROW_LENGTH), 100);
        assert_eq!(geti(GL_UNPACK_SKIP_PIXELS), 3);
        assert_eq!(geti(GL_UNPACK_ALIGNMENT), 8);
        assert_eq!(geti(GL_PIXEL_UNPACK_BUFFER_BINDING) as GLuint, b, "the buffer stays bound");
        assert_eq!(stored(tex, 0, 0, w, h), expected(name, w, h, &data), "{name:#x}");
        // SAFETY: restoring the defaults.
        unsafe {
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
            glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
            glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        }
    }
}

#[test]
fn a_decoded_texture_takes_only_its_own_compressed_name() {
    let _g = context();
    let name = GL_COMPRESSED_R11_EAC;
    let tex = new_texture(GL_TEXTURE_2D);
    let data = blocks(compressed_size(name, 8, 8), 11);
    upload(GL_TEXTURE_2D, 0, name, 8, 8, &data);
    assert_eq!(error(), GL_NO_ERROR);
    let sub = blocks(8, 12);
    let sz = sub.len() as GLsizei;
    // SAFETY: GL calls with an 8-byte block for a 4x4 box.
    unsafe {
        /* another compressed name, even of the same block size */
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_SIGNED_R11_EAC, sz, ptr(&sub));
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "another name");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_RGB8_ETC2, sz, ptr(&sub));
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "a stored block format");
        /* a box off the block grid, outside the level, a wrong imageSize */
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 4, 4, name, sz, ptr(&sub));
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "unaligned offset");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 8, 0, 4, 4, name, sz, ptr(&sub));
        assert_eq!(glGetError(), GL_INVALID_VALUE, "outside the level");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, name, 7, ptr(&sub));
        assert_eq!(glGetError(), GL_INVALID_VALUE, "imageSize");
        /* an uncompressed write, a copy and a mipmap chain would make texels
         * that are not the compressed data */
        let px = [0u8; 32];
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RED, GL_HALF_FLOAT, ptr(&px));
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glTexSubImage2D");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 4, 4);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glCopyTexSubImage2D");
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glGenerateMipmap");
        /* a 3D texture has no ETC2/EAC form (ES 3.0 3.8.6) */
        glCompressedTexImage3D(GL_TEXTURE_3D, 0, name, 8, 8, 1, 0, 32, ptr(&data));
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "TEXTURE_3D");
    }
    assert_eq!(stored(tex, 0, 0, 8, 8), expected(name, 8, 8, &data), "the refusals changed nothing");

    /* the right name works, and a respecify as an ordinary format ends it */
    // SAFETY: as above.
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 4, 4, name, sz, ptr(&sub));
        assert_eq!(glGetError(), GL_NO_ERROR);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_R16F as GLint, 8, 8, 0, GL_RED, GL_HALF_FLOAT, core::ptr::null());
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    assert_eq!(texture_object(tex).decoded_from, 0, "respecified");
    // SAFETY: as above.
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, name, sz, ptr(&sub));
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "no longer a decoded texture");
        let px = [0u8; 32];
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RED, GL_HALF_FLOAT, ptr(&px));
        assert_eq!(glGetError(), GL_NO_ERROR, "an ordinary R16F texture again");
    }
}

#[test]
fn the_imagesize_rule_holds_before_the_storage_is_asked() {
    let _g = context();
    for name in SIX {
        new_texture(GL_TEXTURE_2D);
        let data = blocks(256, 3);
        // SAFETY: the sizes are wrong on purpose and no read happens before the check.
        unsafe {
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, name, 8, 8, 0, 1, ptr(&data));
            assert_eq!(glGetError(), GL_INVALID_VALUE, "{name:#x} imageSize");
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, name, 8, 8, 1, compressed_size(name, 8, 8) as GLsizei, ptr(&data));
            assert_eq!(glGetError(), GL_INVALID_VALUE, "{name:#x} border");
        }
    }
}

#[test]
fn the_ten_formats_are_listed() {
    let _g = context();
    let mut n = 0;
    let mut list = [0 as GLint; 16];
    // SAFETY: room for every value the widest pname writes.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr());
    }
    assert_eq!(n, 11, "ETC1 and ES 3.0's ten");
    let list = &list[..n as usize];
    for f in SIX.iter().chain(&[
        GL_COMPRESSED_RGB8_ETC2,
        GL_COMPRESSED_SRGB8_ETC2,
        GL_COMPRESSED_RGBA8_ETC2_EAC,
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
    ]) {
        assert!(list.contains(&(*f as GLint)), "{f:#x} listed");
    }
    assert_eq!(error(), GL_NO_ERROR);
}
