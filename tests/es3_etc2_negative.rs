// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The compressed-upload error rules of the GL core for the ETC2/EAC names it
//! knows, driven through the GL entry points on a host back end (item
//! etc2-negative-test of plan #354).
//!
//! The expectations come from OpenGL ES 3.0.6 3.8.6 (glCompressedTexImage*,
//! glCompressedTexSubImage*, "Compressed Texture Images") and from dEQP's
//! `es3fNegativeTextureApiTests.cpp` (VK-GL-CTS 93bca01:
//! `compressedteximage2d_invalid_size`, `_neg_level`, `_max_level`,
//! `_neg_width_height`, `_invalid_border`, `compressedteximage3d_*`,
//! `compressedtexsubimage2d_invalid_size`, `_neg_offset`, `_neg_width_height`,
//! `_invalid_offset`/`_invalid_width_height`, `_invalid_format`,
//! `compressedtexsubimage3d_*`), not on what the code returns. A case where
//! the GL core disagrees with the spec is `#[ignore]`d with the section.
//!
//! The formats are one table, [`FORMATS`]: an item that adds a compressed name
//! appends one row (its enum, bytes per 4x4 block, whether the name has a
//! sub-image form) and every test below covers it. The five rows are the
//! names `compressed_format_info` (src/gl/texture.rs) has a class for; the
//! sRGB pair is stored only where the back end samples it, so the build is
//! the PowerVR one (`PVR_SAMPLED_TEXFMTS` has both), the lightest that stores
//! all five; the errors do not depend on the GPU.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_negative
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/// One compressed name under test.
struct Fmt {
    name: &'static str,
    /// The internalformat enum.
    id: GLenum,
    /// Bytes per 4x4 block (ES 3.0 table 3.17).
    block: usize,
    /// Has a glCompressedTexSubImage form. ETC1 has none: OES_compressed_
    /// ETC1_RGB8_texture says so, so a sub-image of it is GL_INVALID_OPERATION.
    sub_image: bool,
}

/// The table. Append a row to cover a new name everywhere.
const FORMATS: &[Fmt] = &[
    Fmt { name: "ETC1_RGB8_OES", id: GL_ETC1_RGB8_OES, block: 8, sub_image: false },
    Fmt { name: "RGB8_ETC2", id: GL_COMPRESSED_RGB8_ETC2, block: 8, sub_image: true },
    Fmt { name: "RGBA8_ETC2_EAC", id: GL_COMPRESSED_RGBA8_ETC2_EAC, block: 16, sub_image: true },
    Fmt { name: "SRGB8_ETC2", id: GL_COMPRESSED_SRGB8_ETC2, block: 8, sub_image: true },
    Fmt { name: "SRGB8_ALPHA8_ETC2_EAC", id: GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, block: 16, sub_image: true },
];

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
    name: c"es3-etc2-negative".as_ptr() as *const c_char,
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

/// A fresh texture bound to `target`, the error queue empty.
fn bind_new(target: GLenum) {
    let mut name: GLuint = 0;
    // SAFETY: one name written.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    assert_eq!(error(), GL_NO_ERROR, "bind_new");
}

/// The bytes of a w x h image of `block`-byte 4x4 blocks.
fn size_of(w: usize, h: usize, block: usize) -> usize {
    w.div_ceil(4) * h.div_ceil(4) * block
}

/// glCompressedTexImage2D of a w x h image whose imageSize is right.
fn image2d(target: GLenum, level: GLint, f: &Fmt, w: GLsizei, h: GLsizei) -> GLenum {
    let data = vec![0u8; size_of(w as usize, h as usize, f.block)];
    // SAFETY: `data` holds imageSize bytes.
    unsafe {
        glCompressedTexImage2D(target, level, f.id, w, h, 0, data.len() as GLsizei, data.as_ptr() as *const c_void)
    };
    error()
}

/// glCompressedTexImage2D with every argument given, over a buffer longer
/// than any imageSize used here.
fn image2d_raw(f: &Fmt, level: GLint, (w, h): (GLsizei, GLsizei), border: GLint, size: GLsizei) -> GLenum {
    let data = vec![0u8; 1024];
    // SAFETY: `data` is longer than any imageSize used.
    unsafe { glCompressedTexImage2D(GL_TEXTURE_2D, level, f.id, w, h, border, size, data.as_ptr() as *const c_void) };
    error()
}

/// glCompressedTexSubImage2D of a box, with the imageSize of the box unless
/// `size` is given.
fn sub2d(
    level: GLint,
    (x, y): (GLint, GLint),
    (w, h): (GLsizei, GLsizei),
    format: GLenum,
    block: usize,
    size: Option<GLsizei>,
) -> GLenum {
    let data = vec![0u8; 1024];
    let size = size.unwrap_or(size_of(w.max(0) as usize, h.max(0) as usize, block) as GLsizei);
    // SAFETY: `data` is longer than any imageSize used.
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, level, x, y, w, h, format, size, data.as_ptr() as *const c_void)
    };
    error()
}

/// glCompressedTexSubImage3D of a box of `f` in a 2D array.
fn sub3d(f: &Fmt, (x, y, z): (GLint, GLint, GLint), (w, h, d): (GLsizei, GLsizei, GLsizei), size: GLsizei) -> GLenum {
    let data = vec![0u8; 1024];
    // SAFETY: `data` is longer than any imageSize used.
    unsafe {
        glCompressedTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, x, y, z, w, h, d, f.id, size, data.as_ptr() as *const c_void)
    };
    error()
}

/// glCompressedTexImage3D with every argument given.
fn image3d_raw(
    target: GLenum,
    f: &Fmt,
    level: GLint,
    (w, h, d): (GLsizei, GLsizei, GLsizei),
    border: GLint,
    size: GLsizei,
) -> GLenum {
    let data = vec![0u8; 1024];
    // SAFETY: `data` is longer than any imageSize used.
    unsafe { glCompressedTexImage3D(target, level, f.id, w, h, d, border, size, data.as_ptr() as *const c_void) };
    error()
}

/// A 2D texture of `f`, `w` x `h`, at level 0.
fn texture(f: &Fmt, w: GLsizei, h: GLsizei) {
    bind_new(GL_TEXTURE_2D);
    assert_eq!(image2d(GL_TEXTURE_2D, 0, f, w, h), GL_NO_ERROR, "{}: {w}x{h} level 0", f.name);
}

/// Run `body` for every row of the table, with the context held.
fn each(body: impl Fn(&Fmt)) {
    let _g = context();
    for f in FORMATS {
        body(f);
    }
}

/// Run `body` for every row that has a sub-image form.
fn each_sub(body: impl Fn(&Fmt)) {
    each(|f| {
        if f.sub_image {
            body(f)
        }
    });
}

fn check(f: &Fmt, what: &str, got: GLenum, want: GLenum) {
    assert_eq!(got, want, "{}: {what}: got {got:#x}, want {want:#x}", f.name);
}

/// The first level past GL_MAX_TEXTURE_SIZE's chain.
fn level_past_max() -> GLint {
    let mut max_size = 0;
    // SAFETY: one value written.
    unsafe { glGetIntegerv(GL_MAX_TEXTURE_SIZE, &mut max_size) };
    max_size.ilog2() as GLint + 1
}

/* ---- glCompressedTexImage2D ------------------------------------------------ */

/// ES 3.0.6 3.8.6: imageSize is ceil(w/4) * ceil(h/4) * block bytes, else
/// GL_INVALID_VALUE (dEQP compressedteximage2d_invalid_size).
#[test]
fn image2d_wrong_image_size() {
    each(|f| {
        bind_new(GL_TEXTURE_2D);
        let right = size_of(8, 8, f.block) as GLsizei;
        for size in [0, right - 1, right + 1, right * 2, f.block as GLsizei, -1] {
            check(f, &format!("8x8 imageSize {size}"), image2d_raw(f, 0, (8, 8), 0, size), GL_INVALID_VALUE);
        }
        /* a partial edge block still counts whole */
        let five = size_of(5, 5, f.block) as GLsizei;
        check(f, "5x5 right size", image2d_raw(f, 0, (5, 5), 0, five), GL_NO_ERROR);
        check(f, "5x5 one byte short", image2d_raw(f, 0, (5, 5), 0, five - 1), GL_INVALID_VALUE);
        check(f, "zero-sized level", image2d(GL_TEXTURE_2D, 1, f, 0, 0), GL_NO_ERROR);
    });
}

/// ES 3.0.6 3.8.6: a negative level is GL_INVALID_VALUE, and so is a level
/// above log2(GL_MAX_TEXTURE_SIZE) (dEQP compressedteximage2d_neg_level,
/// _max_level).
#[test]
fn image2d_level_range() {
    each(|f| {
        bind_new(GL_TEXTURE_2D);
        check(f, "level -1", image2d(GL_TEXTURE_2D, -1, f, 4, 4), GL_INVALID_VALUE);
        check(f, "level 0", image2d(GL_TEXTURE_2D, 0, f, 4, 4), GL_NO_ERROR);
        check(f, "first level past the chain", image2d(GL_TEXTURE_2D, level_past_max(), f, 4, 4), GL_INVALID_VALUE);
        check(f, "level 31", image2d(GL_TEXTURE_2D, 31, f, 4, 4), GL_INVALID_VALUE);
        check(f, "level 64", image2d(GL_TEXTURE_2D, 64, f, 4, 4), GL_INVALID_VALUE);
    });
}

/// ES 3.0.6 3.8.6: negative width or height is GL_INVALID_VALUE (dEQP
/// compressedteximage2d_neg_width_height).
#[test]
fn image2d_negative_size() {
    each(|f| {
        bind_new(GL_TEXTURE_2D);
        for (w, h) in [(-1, 4), (4, -1), (-4, -4)] {
            check(f, &format!("{w}x{h}"), image2d_raw(f, 0, (w, h), 0, 0), GL_INVALID_VALUE);
        }
    });
}

/// ES 3.0.6 3.8.6: border must be 0, else GL_INVALID_VALUE (dEQP
/// compressedteximage2d_invalid_border).
#[test]
fn image2d_nonzero_border() {
    each(|f| {
        bind_new(GL_TEXTURE_2D);
        let size = size_of(4, 4, f.block) as GLsizei;
        for border in [1, -1, 4] {
            check(f, &format!("border {border}"), image2d_raw(f, 0, (4, 4), border, size), GL_INVALID_VALUE);
        }
    });
}

/// The format of glCompressedTexImage2D must be a compressed one, else
/// GL_INVALID_ENUM, and so is a target the 2D entry point does not take.
#[test]
fn image2d_enum_errors() {
    let _g = context();
    bind_new(GL_TEXTURE_2D);
    // SAFETY: null data with imageSize 0 is never read.
    unsafe {
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, 4, 4, 0, 0, core::ptr::null());
        assert_eq!(error(), GL_INVALID_ENUM, "an uncompressed format");
        glCompressedTexImage2D(GL_TEXTURE_2D_ARRAY, 0, GL_COMPRESSED_RGB8_ETC2, 4, 4, 0, 8, core::ptr::null());
        assert_eq!(error(), GL_INVALID_ENUM, "a 2D array target in the 2D entry point");
    }
}

/* ---- glCompressedTexImage3D / glCompressedTexSubImage3D ----------------------- */

/// ES 3.0.6 3.8.6: an ETC2/EAC format has no 3D form: glCompressedTexImage3D
/// and glCompressedTexSubImage3D with GL_TEXTURE_3D are GL_INVALID_OPERATION
/// (dEQP compressedteximage3d_invalid_operation_etc2 and the sub-image
/// twin).
#[test]
fn texture_3d_is_invalid_operation() {
    each(|f| {
        bind_new(GL_TEXTURE_3D);
        let one = f.block as GLsizei;
        check(f, "TexImage3D GL_TEXTURE_3D", image3d_raw(GL_TEXTURE_3D, f, 0, (4, 4, 1), 0, one), GL_INVALID_OPERATION);
        let data = vec![0u8; 64];
        // SAFETY: `data` is longer than the imageSize.
        unsafe {
            glCompressedTexSubImage3D(GL_TEXTURE_3D, 0, 0, 0, 0, 4, 4, 1, f.id, one, data.as_ptr() as *const c_void)
        };
        check(f, "TexSubImage3D GL_TEXTURE_3D", error(), GL_INVALID_OPERATION);
    });
}

/// ES 3.0.6 3.8.6: a 2D array takes the ETC2/EAC formats (its layers are 2D
/// images): the whole array, then a layer of it.
#[test]
fn array_2d_is_accepted() {
    each_sub(|f| {
        bind_new(GL_TEXTURE_2D_ARRAY);
        let layer = f.block as GLsizei;
        check(
            f,
            "TexImage3D 2D array 4x4x2",
            image3d_raw(GL_TEXTURE_2D_ARRAY, f, 0, (4, 4, 2), 0, 2 * layer),
            GL_NO_ERROR,
        );
        check(f, "TexSubImage3D layer 1", sub3d(f, (0, 0, 1), (4, 4, 1), layer), GL_NO_ERROR);
        check(f, "TexSubImage3D both layers", sub3d(f, (0, 0, 0), (4, 4, 2), 2 * layer), GL_NO_ERROR);
    });
}

/// The argument errors of the 3D entry points on a 2D array (dEQP
/// compressedteximage3d_invalid_size, _neg_level, _neg_width_height_depth,
/// _invalid_border and the sub-image twins).
#[test]
fn array_2d_argument_errors() {
    each_sub(|f| {
        bind_new(GL_TEXTURE_2D_ARRAY);
        let layer = f.block as GLsizei;
        let at = GL_TEXTURE_2D_ARRAY;
        check(f, "imageSize of one layer for two", image3d_raw(at, f, 0, (4, 4, 2), 0, layer), GL_INVALID_VALUE);
        check(f, "imageSize 0", image3d_raw(at, f, 0, (4, 4, 1), 0, 0), GL_INVALID_VALUE);
        check(f, "negative imageSize", image3d_raw(at, f, 0, (4, 4, 1), 0, -1), GL_INVALID_VALUE);
        check(f, "negative level", image3d_raw(at, f, -1, (4, 4, 1), 0, layer), GL_INVALID_VALUE);
        check(f, "level past the chain", image3d_raw(at, f, level_past_max(), (4, 4, 1), 0, layer), GL_INVALID_VALUE);
        for dims in [(-1, 4, 1), (4, -1, 1), (4, 4, -1)] {
            check(f, &format!("size {dims:?}"), image3d_raw(at, f, 0, dims, 0, 0), GL_INVALID_VALUE);
        }
        check(f, "border 1", image3d_raw(at, f, 0, (4, 4, 1), 1, layer), GL_INVALID_VALUE);
        check(
            f,
            "uncompressed format",
            {
                // SAFETY: null data with imageSize 0 is never read.
                unsafe { glCompressedTexImage3D(at, 0, GL_RGBA8, 4, 4, 1, 0, 0, core::ptr::null()) };
                error()
            },
            GL_INVALID_ENUM,
        );

        check(f, "array 4x4x2", image3d_raw(at, f, 0, (4, 4, 2), 0, 2 * layer), GL_NO_ERROR);
        check(f, "sub: layer past the end", sub3d(f, (0, 0, 2), (4, 4, 1), layer), GL_INVALID_VALUE);
        check(f, "sub: depth past the end", sub3d(f, (0, 0, 1), (4, 4, 2), 2 * layer), GL_INVALID_VALUE);
        check(f, "sub: negative layer", sub3d(f, (0, 0, -1), (4, 4, 1), layer), GL_INVALID_VALUE);
        check(f, "sub: negative depth", sub3d(f, (0, 0, 0), (4, 4, -1), 0), GL_INVALID_VALUE);
        check(f, "sub: wrong imageSize", sub3d(f, (0, 0, 0), (4, 4, 2), layer), GL_INVALID_VALUE);
        check(f, "sub: x not a block multiple", sub3d(f, (2, 0, 0), (2, 4, 1), layer), GL_INVALID_OPERATION);
        check(f, "sub: width off the edge", sub3d(f, (0, 0, 0), (3, 4, 1), layer), GL_INVALID_OPERATION);
    });
}

/* ---- glCompressedTexSubImage2D ---------------------------------------------- */

/// ES 3.0.6 3.8.6: imageSize of a sub-image is its own box's size, else
/// GL_INVALID_VALUE (dEQP compressedtexsubimage2d_invalid_size).
#[test]
fn sub2d_wrong_image_size() {
    each_sub(|f| {
        texture(f, 8, 8);
        let b = f.block;
        check(f, "right size", sub2d(0, (0, 0), (4, 4), f.id, b, None), GL_NO_ERROR);
        for size in [0, b as GLsizei - 1, b as GLsizei + 1, 4 * b as GLsizei, -1] {
            check(f, &format!("imageSize {size}"), sub2d(0, (0, 0), (4, 4), f.id, b, Some(size)), GL_INVALID_VALUE);
        }
    });
}

/// ES 3.0.6 3.8.6, ETC2/EAC: xoffset and yoffset are multiples of 4, and
/// width and height are multiples of 4 or reach the level's edge, else
/// GL_INVALID_OPERATION (dEQP compressedtexsubimage2d_invalid_offset and
/// _invalid_width_height); a box outside the level is GL_INVALID_VALUE (dEQP
/// _neg_offset, _neg_width_height).
#[test]
fn sub2d_block_alignment() {
    each_sub(|f| {
        /* 10x10: three block columns, the last of which holds two texels */
        texture(f, 10, 10);
        let (b, id) = (f.block, f.id);
        for (x, y) in [(1, 0), (0, 1), (2, 2), (3, 4), (6, 0), (0, 5)] {
            check(f, &format!("offset {x},{y}"), sub2d(0, (x, y), (4, 4), id, b, None), GL_INVALID_OPERATION);
        }
        for (w, h) in [(3, 4), (4, 3), (1, 1), (5, 4), (4, 6)] {
            check(
                f,
                &format!("size {w}x{h} off the edge"),
                sub2d(0, (0, 0), (w, h), id, b, None),
                GL_INVALID_OPERATION,
            );
        }
        /* the level's edge: a 2-texel box in the last block, the whole level */
        check(f, "edge block 2x2 at 8,8", sub2d(0, (8, 8), (2, 2), id, b, None), GL_NO_ERROR);
        check(f, "whole level", sub2d(0, (0, 0), (10, 10), id, b, None), GL_NO_ERROR);
        check(f, "6 wide at 4 reaches the edge", sub2d(0, (4, 0), (6, 4), id, b, None), GL_NO_ERROR);
        check(f, "empty box", sub2d(0, (0, 0), (0, 0), id, b, None), GL_NO_ERROR);
        /* outside the level */
        check(f, "past the right edge", sub2d(0, (8, 0), (4, 4), id, b, None), GL_INVALID_VALUE);
        check(f, "past the bottom edge", sub2d(0, (0, 8), (4, 4), id, b, None), GL_INVALID_VALUE);
        check(f, "negative x", sub2d(0, (-4, 0), (4, 4), id, b, None), GL_INVALID_VALUE);
        check(f, "negative y", sub2d(0, (0, -4), (4, 4), id, b, None), GL_INVALID_VALUE);
        check(f, "negative width", sub2d(0, (0, 0), (-4, 4), id, b, Some(0)), GL_INVALID_VALUE);
        check(f, "negative height", sub2d(0, (0, 0), (4, -4), id, b, Some(0)), GL_INVALID_VALUE);
    });
}

/// ES 3.0.6 3.8.6: format must match the level's internal format, else
/// GL_INVALID_OPERATION (dEQP compressedtexsubimage2d_invalid_format); an
/// uncompressed format is no compressed format at all, GL_INVALID_ENUM.
#[test]
fn sub2d_format_mismatch() {
    each_sub(|f| {
        texture(f, 8, 8);
        let b = f.block;
        for other in FORMATS.iter().filter(|o| o.sub_image && o.id != f.id) {
            /* the box and its size are right for `f`, so only the format is wrong */
            let got = sub2d(0, (0, 0), (4, 4), other.id, b, Some(b as GLsizei));
            check(f, &format!("sub-image as {}", other.name), got, GL_INVALID_OPERATION);
        }
        check(f, "an uncompressed format", sub2d(0, (0, 0), (4, 4), GL_RGBA, b, Some(b as GLsizei)), GL_INVALID_ENUM);
        /* a level nobody specified holds no compressed image */
        check(f, "level 1 never specified", sub2d(1, (0, 0), (4, 4), f.id, b, None), GL_INVALID_OPERATION);
        /* nor does an uncompressed texture */
        bind_new(GL_TEXTURE_2D);
        // SAFETY: an RGBA8 8x8 level from null.
        unsafe {
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null())
        };
        assert_eq!(error(), GL_NO_ERROR);
        check(f, "into an RGBA8 texture", sub2d(0, (0, 0), (4, 4), f.id, b, None), GL_INVALID_OPERATION);
    });
}

/// ES 3.0.6 3.8.6: a negative level is GL_INVALID_VALUE, and so is a level
/// above the largest (dEQP compressedtexsubimage2d_neg_level, _max_level).
#[test]
fn sub2d_level_range() {
    each_sub(|f| {
        texture(f, 8, 8);
        let (b, id) = (f.block, f.id);
        check(f, "level -1", sub2d(-1, (0, 0), (4, 4), id, b, None), GL_INVALID_VALUE);
        check(f, "first level past the chain", sub2d(level_past_max(), (0, 0), (4, 4), id, b, None), GL_INVALID_VALUE);
        check(f, "level 64", sub2d(64, (0, 0), (4, 4), id, b, None), GL_INVALID_VALUE);
    });
}

/// A sub-image is checked against the size of the level it names: the
/// level 1 of an 8x8 texture is 4x4.
#[test]
fn sub2d_checks_the_level_it_names() {
    each_sub(|f| {
        texture(f, 8, 8);
        assert_eq!(image2d(GL_TEXTURE_2D, 1, f, 4, 4), GL_NO_ERROR);
        let (b, id) = (f.block, f.id);
        check(f, "whole level 1", sub2d(1, (0, 0), (4, 4), id, b, None), GL_NO_ERROR);
        check(f, "x 4 of level 1", sub2d(1, (4, 0), (4, 4), id, b, None), GL_INVALID_VALUE);
    });
}

/// OES_compressed_ETC1_RGB8_texture: "CompressedTexSubImage2D ... with
/// GL_ETC1_RGB8_OES generates GL_INVALID_OPERATION" (extension spec, errors
/// of the new tokens), and the ES 3.0 spec lists ETC1 in none of its
/// compressed-format rules. The GL core refuses it only where ETC1 is
/// decoded at upload (`etc1_decoded_at_upload`, texture.rs's unit test at
/// "OES_compressed_ETC1_RGB8_texture: no CompressedTexSubImage2D"); where the
/// texture unit samples ETC1, which every build here does (this one too),
/// `compressed_format_info` maps the name to the RGB8_ETC2 class and the
/// sub-image is written with no error. Not fixed here (src/gl/texture.rs is
/// held by other items); run with `-- --ignored` to see it.
#[ignore = "OES_compressed_ETC1_RGB8_texture: CompressedTexSubImage2D with ETC1 is GL_INVALID_OPERATION; the GL core accepts it where ETC1 is sampled, not decoded"]
#[test]
fn etc1_has_no_sub_image() {
    each(|f| {
        if f.sub_image {
            return;
        }
        texture(f, 8, 8);
        check(f, "TexSubImage2D of an ETC1 level", sub2d(0, (0, 0), (4, 4), f.id, f.block, None), GL_INVALID_OPERATION);
    });
}

/// The table itself: every row's block size is what ES 3.0 table 3.17 says,
/// so a wrong row cannot hide behind the size checks above.
#[test]
fn table_block_sizes() {
    for f in FORMATS {
        let sixteen = matches!(f.id, GL_COMPRESSED_RGBA8_ETC2_EAC | GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC);
        assert_eq!(f.block, if sixteen { 16 } else { 8 }, "{}", f.name);
    }
}
