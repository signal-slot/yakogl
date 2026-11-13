// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR build's 2D sampling of the four ETC2 names the GL core
//! stores: GL_COMPRESSED_RGB8_ETC2, RGBA8_ETC2_EAC, SRGB8_ETC2 and
//! SRGB8_ALPHA8_ETC2_EAC (`compressed_format_info`). tests/es3_pvr_etc_array.rs
//! has the arrays; this is the plain 2D texture: 4x4, 8x8 and a 6x6 whose
//! edge blocks reach past the texture, each with two mip levels.
//!
//! What the host shows, per name and size:
//!  * the storage the GL core makes on a twiddled back end: the class
//!    (GLES_TEXFMT_ETC2_*), 8 or 16 byte blocks, level 0's block grid in the
//!    power-of-two grid the TPU walks, level 1 at the packed offset after it
//!    in the grid halved (Mesa `pvr_image_plane_setup_mip_levels`,
//!    pvr_image.c:173-196), every block at its twiddled index;
//!  * the TPU texture state `texture_descriptor` emits for that texture, word
//!    by word against Mesa's pvr_arch_formats.c:158-163 (`FORMAT_COMPRESSED`:
//!    ETC2_RGB8 and ETC2_SRGB8 are TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB,
//!    ETC2_RGBA8 and ETC2_SRGBA8 are ..._ETC2A_RGBA; the sRGB pair differs by
//!    TEXSTATE_IMAGE_WORD0.gamma only, pvr_arch_tex_state.c:207-226), with
//!    the TPU decoding the blocks (compression mode TPU, no alpha_msb), the
//!    API size, the level count and mipmaps_present, the image address, and
//!    the RGB formats' alpha forced to ONE by their swizzle;
//!  * the texels the TPU would be handed: the stored blocks, taken back out
//!    of their twiddled places, decode with src/gl/etc2.rs to the same RGBA8
//!    as the blocks uploaded, edge blocks included (the texels past the
//!    texture's edge are decoded and never read).
//!
//! What the host cannot show: there is no TPU interpreter, so nothing here
//! decodes ETC2 the way the hardware does, filters, applies the gamma decode
//! (the sRGB names decode to the same bytes as the linear ones here, the
//! TPU's gamma bit is the whole difference), clamps or wraps at an edge
//! block, or walks the levels by LOD; the descriptor is checked to be what
//! Mesa describes, not to be what the silicon accepts. That is the board's
//! texture.wrap / texture.format ETC2 cases (#313). The bind is built here
//! the way draw.rs does for a compressed texture (level 0's block grid as
//! `bytes`), because `texture_bind_2d` is crate-private.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_etc2_pvr
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA,
    GLES_TEXFMT_ETC2_SRGB_RGB, GLES_TEXFMT_ETC2_SRGB_RGBA, GLES_TEXTYPE_2D, GLES_WRAP_CLAMP_TO_EDGE,
};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::etc2;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glCompressedTexImage2D, glDeleteTextures, glGenTextures, glTexParameteri};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::csbgen::{ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA, ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB};

/* ---- a context on the host: the smallest OSAL and back end that make one */

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

/// The PowerVR's shape for a compressed 2D texture: twiddled block grids,
/// no mip level alignment.
static PVR_LIKE: YakoglBackend = YakoglBackend {
    name: c"es3-etc2-pvr".as_ptr() as *const c_char,
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
};

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the table is a static; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &PVR_LIKE;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/* ---- the four names ---------------------------------------------------- */

struct Case {
    name: &'static str,
    internalformat: GLenum,
    /// The GL core's class for it, and Mesa's TEXSTATE_FORMAT_COMPRESSED.
    class: u8,
    texformat: u64,
    /// Bytes per 4x4 block.
    bb: usize,
    srgb: bool,
}

/// pvr_arch_formats.c:158-163.
const CASES: [Case; 4] = [
    Case {
        name: "RGB8_ETC2",
        internalformat: GL_COMPRESSED_RGB8_ETC2,
        class: GLES_TEXFMT_ETC2_RGB,
        texformat: ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB as u64,
        bb: 8,
        srgb: false,
    },
    Case {
        name: "SRGB8_ETC2",
        internalformat: GL_COMPRESSED_SRGB8_ETC2,
        class: GLES_TEXFMT_ETC2_SRGB_RGB,
        texformat: ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2_RGB as u64,
        bb: 8,
        srgb: true,
    },
    Case {
        name: "RGBA8_ETC2_EAC",
        internalformat: GL_COMPRESSED_RGBA8_ETC2_EAC,
        class: GLES_TEXFMT_ETC2_RGBA,
        texformat: ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA as u64,
        bb: 16,
        srgb: false,
    },
    Case {
        name: "SRGB8_ALPHA8_ETC2_EAC",
        internalformat: GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
        class: GLES_TEXFMT_ETC2_SRGB_RGBA,
        texformat: ROGUE_TEXSTATE_FORMAT_COMPRESSED_ETC2A_RGBA as u64,
        bb: 16,
        srgb: true,
    },
];

/// 4x4 and 8x8 (whole blocks), 6x6 (2x2 blocks, the right and bottom ones
/// half outside).
const SIZES: [u32; 3] = [4, 8, 6];

/* ---- Mesa's layout, computed here -------------------------------------- */

/// Block (bx, by) of a gw x gh grid in twiddled order: the two coordinates'
/// low bits interleaved (y the lower), up to the smaller side; what is left
/// of the larger coordinate is linear above them (`pvr_image.c`'s morton).
fn twiddle(bx: u32, by: u32, gw: u32, gh: u32) -> u32 {
    let bits = gw.min(gh).ilog2();
    let mut out = 0;
    for i in 0..bits {
        out |= ((by >> i) & 1) << (2 * i);
        out |= ((bx >> i) & 1) << (2 * i + 1);
    }
    let rest = if gw > gh { bx } else { by } >> bits;
    out | (rest << (2 * bits))
}

/// The block grid of level `l` of a `w x h` image: the power-of-two extent
/// in blocks, halved per level, at least one (`pvr_image.c:83-92`).
fn grid(w: u32, h: u32, l: u32) -> (u32, u32) {
    let g = |s: u32| (s.div_ceil(4).next_power_of_two() >> l).max(1);
    (g(w), g(h))
}

/// Level `l`'s byte offset in a chain packed level after level.
fn level_offset(w: u32, h: u32, bb: usize, l: u32) -> usize {
    (0..l)
        .map(|k| {
            let (gw, gh) = grid(w, h, k);
            (gw * gh) as usize * bb
        })
        .sum()
}

/* ---- the upload -------------------------------------------------------- */

/// `n` blocks of `bb` bytes, a different run for each seed: xorshift bytes,
/// every block distinct.
fn blocks(seed: u32, n: usize, bb: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(0x9E37_79B9) | 1;
    (0..n * bb)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 11) as u8
        })
        .collect()
}

fn dims(size: u32, l: u32) -> (u32, u32, u32, u32) {
    let s = (size >> l).max(1);
    (s, s, s.div_ceil(4), s.div_ceil(4))
}

/// A `size x size` texture of two levels, each through glCompressedTexImage2D
/// in raster block order; returns the name and the blocks sent per level.
fn upload(c: &Case, size: u32, levels: u32) -> (GLuint, Vec<Vec<u8>>) {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
    }
    let mut sent = Vec::new();
    for l in 0..levels {
        let (lw, lh, bw, bh) = dims(size, l);
        let data = blocks(size * 16 + l + c.bb as u32, (bw * bh) as usize, c.bb);
        // SAFETY: `data` holds bw x bh blocks, imageSize bytes.
        unsafe {
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                c.internalformat,
                lw as GLsizei,
                lh as GLsizei,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{} {size}x{size} level {l}", c.name);
        sent.push(data);
    }
    // SAFETY: the texture is bound.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
    }
    (name, sent)
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The blocks of `level` out of the texture's storage, back in raster order.
fn stored_raster(t: &context::GlTexture, size: u32, bb: usize, level: u32) -> Vec<u8> {
    let (gw, gh) = grid(size, size, level);
    let (_, _, bw, bh) = dims(size, level);
    let off = level_offset(size, size, bb, level);
    assert_eq!(t.level_off[level as usize] as usize, off, "level {level}'s offset (packed, no alignment)");
    let total = level_offset(size, size, bb, level + 1);
    // SAFETY: the mock OSAL maps a GPU address to host memory; the storage
    // holds the chain, `total` bytes of it up to this level's end.
    let s = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, total) };
    let mut out = Vec::new();
    for by in 0..bh {
        for bx in 0..bw {
            let d = off + twiddle(bx, by, gw, gh) as usize * bb;
            out.extend_from_slice(&s[d..d + bb]);
        }
    }
    out
}

fn decode(c: &Case, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let ok = if c.bb == 8 {
        etc2::decode_rgb_image(w as usize, h as usize, data, false, &mut out)
    } else {
        etc2::decode_rgba_image(w as usize, h as usize, data, &mut out)
    };
    assert!(ok, "{} {w}x{h}: the decoder takes the blocks", c.name);
    out
}

#[test]
fn the_storage_is_twiddled_blocks_in_mesas_chain() {
    let _g = context();
    for c in &CASES {
        for size in SIZES {
            let (name, sent) = upload(c, size, 2);
            let t = texture_object(name);
            assert_eq!(t.texfmt, c.class, "{} {size}: the class", c.name);
            assert_eq!(usize::from(t.block_bytes), c.bb, "{} {size}: block bytes", c.name);
            assert_eq!((t.width, t.height, t.levels), (size, size, 2), "{} {size}: size and levels", c.name);
            let (gw, gh) = grid(size, size, 0);
            assert_eq!((t.block_w, t.block_h), (gw, gh), "{} {size}: level 0 in the power-of-two grid", c.name);
            for l in 0..2 {
                let raster = stored_raster(t, size, c.bb, l);
                assert_eq!(raster, sent[l as usize], "{} {size} level {l}: blocks at their twiddled places", c.name);
            }
            // SAFETY: a name this test made.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
}

#[test]
fn the_stored_blocks_decode_to_what_was_uploaded_edge_blocks_included() {
    let _g = context();
    for c in &CASES {
        for size in SIZES {
            let (name, sent) = upload(c, size, 2);
            let t = texture_object(name);
            for l in 0..2 {
                let (lw, lh, _, _) = dims(size, l);
                let from_storage = decode(c, lw, lh, &stored_raster(t, size, c.bb, l));
                let from_upload = decode(c, lw, lh, &sent[l as usize]);
                assert_eq!(from_storage, from_upload, "{} {size} level {l}: texels", c.name);
                /* the decode is not a blank: distinct random blocks give more
                 * than one colour in the visible texels */
                assert!(
                    from_upload.chunks(4).any(|p| p != &from_upload[..4]) || lw * lh == 1,
                    "{} {size} level {l}: decoded something",
                    c.name
                );
            }
            // SAFETY: a name this test made.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
}

/* ---- the TPU texture state --------------------------------------------- */

const BASE: u64 = 0x80_0000_1000;

/// The bind a draw of `sampler2D` over `t` makes (draw.rs: `format`,
/// `tb.bytes` as level 0's block grid, `sampling_fields`), at a base address
/// the descriptor can hold.
fn bind(t: &context::GlTexture, mip_filter: u8) -> GlesTextureBind {
    GlesTextureBind {
        format: t.texfmt,
        type_: GLES_TEXTYPE_2D,
        layers: 1,
        base_gpu: BASE,
        width: t.width,
        height: t.height,
        stride_texels: t.width,
        bytes: t.block_w * t.block_h * u32::from(t.block_bytes),
        linear_filter: 1,
        min_linear: 1,
        mip_filter,
        mip_levels: t.levels,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        ..GlesTextureBind::ZERO
    }
}

fn words(t: &GlesTextureBind) -> [u32; 16] {
    let w = yakogl::pvr::backend::texture_descriptor(t);
    core::array::from_fn(|i| w[i])
}

/// TEXSTATE_SWIZ: SRCCHAN_0..3 are 0..3, SRC_ONE 4.
fn swiz(s: [u64; 4]) -> u64 {
    s[0] << 14 | s[1] << 11 | s[2] << 8 | s[3] << 5
}

#[test]
fn the_image_words_are_mesas_compressed_2d_with_the_tpu_decoding() {
    let _g = context();
    for c in &CASES {
        for size in SIZES {
            let (name, _) = upload(c, size, 2);
            let t = texture_object(name);
            for (mip_filter, levels) in [(2u8, 2u64), (0, 1)] {
                let w = words(&bind(t, mip_filter));
                let (w0, w1) = (w[0] as u64 | (w[1] as u64) << 32, w[2] as u64 | (w[3] as u64) << 32);
                /* RGB formats read alpha as ONE (swizzle), RGBA formats R,G,B,A */
                let sw = if c.bb == 8 { swiz([0, 1, 2, 4]) } else { swiz([0, 1, 2, 3]) };
                let want0 = c.texformat << 27
                    | sw
                    | 1 /* TEXTYPE_2D */
                    | u64::from(c.srgb) << 3 /* gamma */
                    | u64::from(size - 1) << 34
                    | u64::from(size - 1) << 48;
                assert_eq!(w0, want0, "{} {size}: IMAGE_WORD0 ({levels} level(s))", c.name);
                let want1 = levels /* num_mip_levels */
                    | u64::from(levels > 1) << 15 /* mipmaps_present */
                    | (BASE >> 2) << 16 /* texaddr */
                    | 1 << 54 /* compression mode TPU */;
                assert_eq!(w1, want1, "{} {size}: IMAGE_WORD1 ({levels} level(s))", c.name);
                assert_eq!(w[4], t.block_w * t.block_h * c.bb as u32, "{} {size}: meta size", c.name);
            }
            // SAFETY: a name this test made.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
}

/// The sRGB name is its linear name's descriptor with the gamma bit and
/// nothing else (the formats share one TEXSTATE_FORMAT_COMPRESSED), and the
/// sampler words do not depend on the format at all.
#[test]
fn the_srgb_names_differ_from_the_linear_ones_by_the_gamma_bit_only() {
    let _g = context();
    for size in SIZES {
        let mut by_name = Vec::new();
        for c in &CASES {
            let (name, _) = upload(c, size, 2);
            by_name.push(words(&bind(texture_object(name), 2)));
            // SAFETY: a name this test made.
            unsafe { glDeleteTextures(1, &name) };
        }
        let [rgb, srgb, rgba, srgba] = [by_name[0], by_name[1], by_name[2], by_name[3]];
        assert_eq!(srgb[0], rgb[0] | 1 << 3, "{size}: RGB8 pair, word 0");
        assert_eq!(srgba[0], rgba[0] | 1 << 3, "{size}: RGBA8 pair, word 0");
        assert_eq!(srgb[1..], rgb[1..], "{size}: RGB8 pair, the rest");
        assert_eq!(srgba[1..], rgba[1..], "{size}: RGBA8 pair, the rest");
        assert_ne!(rgb[0], rgba[0], "{size}: the two block formats are two TPU formats");
        for d in [&srgb, &rgba, &srgba] {
            assert_eq!(d[8..12], rgb[8..12], "{size}: sampler words, format independent");
            assert_eq!(d[7], rgb[7], "{size}: pack info, one for the compressed formats");
        }
    }
}
