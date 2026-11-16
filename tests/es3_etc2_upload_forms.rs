// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! All ten ES 3.0 ETC2/EAC compressed names through every compressed upload
//! path of the GL core (item etc2-store of plan #354):
//! * glCompressedTexImage2D into GL_TEXTURE_2D at level 0 and at a smaller
//!   level, with a size that is not a multiple of four (5x3, then 2x1);
//! * glCompressedTexSubImage2D on a 4-aligned rectangle;
//! * glCompressedTexImage3D / glCompressedTexSubImage3D into a 2D array of
//!   two layers;
//! * a source in a PIXEL_UNPACK_BUFFER at a nonzero offset (image and
//!   sub-image);
//! * glCompressedTexImage3D into GL_TEXTURE_3D is GL_INVALID_OPERATION
//!   (ES 3.0.6 3.8.6).
//!
//! The four names the GL core keeps as blocks (RGB8_ETC2, RGBA8_ETC2_EAC and
//! the two sRGB ones) are checked byte for byte in the stored block grid; the
//! six it decodes at upload (docs/notes/etc2-eac.md) are checked texel for
//! texel against `yakogl::gl::etc2`, in the class the design gives them. The
//! sizes keep the block grid one block high (or wide) so the storage order of
//! the blocks (linear or twiddled, per back end) is the same.
//!
//! What it cannot show: what a GPU samples (tests/es3_etc2_v3d.rs and the
//! board runs do), nor dEQP's pictures.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_upload_forms
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_upload_forms
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_upload_forms
//! ```
#![cfg(all(
    feature = "host-test",
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d"),
    not(feature = "report-es2")
))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlTexture};
use yakogl::gl::es3_formats::f32_to_half;
use yakogl::gl::etc2;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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

/// A stand-in back end; the build's GPU mask (`backend::sampled_texfmts`)
/// decides which classes the GL core stores.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-etc2-upload-forms".as_ptr() as *const c_char,
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
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the ten formats ------------------------------------------------------ */

/// How the GL core keeps a name, and what a stored texel must be.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fmt {
    /// Kept as blocks of this many bytes (RGB8_ETC2 and its three kin).
    Blocks(usize),
    R11,
    R11S,
    Rg11,
    Rg11S,
    Pt,
    PtSrgb,
}

const TEN: [(&str, GLenum, Fmt); 10] = [
    ("RGB8_ETC2", GL_COMPRESSED_RGB8_ETC2, Fmt::Blocks(8)),
    ("SRGB8_ETC2", GL_COMPRESSED_SRGB8_ETC2, Fmt::Blocks(8)),
    ("RGBA8_ETC2_EAC", GL_COMPRESSED_RGBA8_ETC2_EAC, Fmt::Blocks(16)),
    ("SRGB8_ALPHA8_ETC2_EAC", GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, Fmt::Blocks(16)),
    ("R11_EAC", GL_COMPRESSED_R11_EAC, Fmt::R11),
    ("SIGNED_R11_EAC", GL_COMPRESSED_SIGNED_R11_EAC, Fmt::R11S),
    ("RG11_EAC", GL_COMPRESSED_RG11_EAC, Fmt::Rg11),
    ("SIGNED_RG11_EAC", GL_COMPRESSED_SIGNED_RG11_EAC, Fmt::Rg11S),
    ("RGB8_PUNCHTHROUGH_ALPHA1_ETC2", GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, Fmt::Pt),
    ("SRGB8_PUNCHTHROUGH_ALPHA1_ETC2", GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, Fmt::PtSrgb),
];

impl Fmt {
    /// Bytes per 4x4 block (ES 3.0 table 3.17).
    fn block(self) -> usize {
        match self {
            Fmt::Blocks(b) => b,
            Fmt::Rg11 | Fmt::Rg11S => 16,
            _ => 8,
        }
    }
    /// Bytes per stored texel of a decoded name.
    fn bpp(self) -> usize {
        match self {
            Fmt::Blocks(_) => 0,
            Fmt::R11 | Fmt::R11S => 2,
            Fmt::Rg11 | Fmt::Rg11S => 8,
            Fmt::Pt | Fmt::PtSrgb => 4,
        }
    }
    /// The texel (x, y) of the w x h image `blocks`, as the class stores it.
    fn texel(self, w: usize, h: usize, blocks: &[u8], x: usize, y: usize) -> Vec<u8> {
        let half = |v: f32| f32_to_half(v).to_le_bytes();
        let n = w * h;
        match self {
            Fmt::Blocks(_) => unreachable!("kept as blocks"),
            Fmt::R11 => {
                let mut o = vec![0u16; n];
                assert!(etc2::decode_eac11_unsigned_image(w, h, 1, blocks, &mut o));
                half(f32::from(o[y * w + x]) / 65535.0).to_vec()
            }
            Fmt::R11S => {
                let mut o = vec![0i16; n];
                assert!(etc2::decode_eac11_signed_image(w, h, 1, blocks, &mut o));
                half((f32::from(o[y * w + x]) / 32767.0).max(-1.0)).to_vec()
            }
            Fmt::Rg11 => {
                let mut o = vec![0u16; n * 2];
                assert!(etc2::decode_eac11_unsigned_image(w, h, 2, blocks, &mut o));
                let v = |c: usize| half(f32::from(o[(y * w + x) * 2 + c]) / 65535.0);
                [v(0), v(1), [0, 0], half(1.0)].concat()
            }
            Fmt::Rg11S => {
                let mut o = vec![0i16; n * 2];
                assert!(etc2::decode_eac11_signed_image(w, h, 2, blocks, &mut o));
                let v = |c: usize| half((f32::from(o[(y * w + x) * 2 + c]) / 32767.0).max(-1.0));
                [v(0), v(1), [0, 0], half(1.0)].concat()
            }
            Fmt::Pt | Fmt::PtSrgb => {
                let mut o = vec![0u8; n * 4];
                assert!(etc2::decode_rgb_image(w, h, blocks, true, &mut o));
                o[(y * w + x) * 4..(y * w + x) * 4 + 4].to_vec()
            }
        }
    }
}

/// `n` bytes from a seed: every bit pattern a block can hold turns up.
fn bytes(seed: u32, n: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(0x9e37_79b9) | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 11) as u8
        })
        .collect()
}

/// The blocks of a w x h image.
fn image(f: Fmt, w: usize, h: usize, seed: u32) -> Vec<u8> {
    bytes(seed, w.div_ceil(4) * h.div_ceil(4) * f.block())
}

fn texture_object(name: GLuint) -> &'static GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The stored bytes at `at` of the level storage of `name`.
fn storage(name: GLuint, at: usize, n: usize) -> Vec<u8> {
    let t = texture_object(name);
    // SAFETY: the mock OSAL maps the storage to host memory; callers stay inside the level.
    unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), n) }.to_vec()
}

/// The stored decoded texel (x, y) of image `layer` of `level`, as the GL
/// core keeps it: layers stacked under one another in every level, rows at
/// the level's pitch.
fn stored(name: GLuint, level: u32, layer: u32, x: usize, y: usize, bpp: usize) -> Vec<u8> {
    let t = texture_object(name);
    let (lw, lh) = (((t.width >> level).max(1)) as usize, ((t.height >> level).max(1)) as usize);
    let pitch = if t.pot_chain { t.level_pitch_at(level) } else { t.level_pitch(lw as u32) } as usize;
    assert_eq!(usize::from(t.bpp), bpp, "texel size of the class");
    storage(name, t.level_off[level as usize] as usize + (((layer as usize * lh + y) * pitch) + x) * bpp, bpp)
}

/// Where the sub-image is on the texture: (level, layer, x, y), 4-aligned x, y.
type Place = (u32, u32, usize, usize);

/// The w x h `blocks` image sits at `(level, layer, x0, y0)` of `name`: every
/// block (kept) or texel (decoded) is the given one.
fn check_image(name: GLuint, f: Fmt, (level, layer, x0, y0): Place, (w, h): (usize, usize), blocks: &[u8], what: &str) {
    if let Fmt::Blocks(bb) = f {
        let t = texture_object(name);
        assert_eq!(usize::from(t.block_bytes), bb, "{what}: block size of the class");
        let (lw, lh) = ((t.width >> level).max(1) as usize, (t.height >> level).max(1) as usize);
        let (gw, gh) = (lw.div_ceil(4), lh.div_ceil(4));
        assert!(gw == 1 || gh == 1, "{what}: the test keeps the grid one block high or wide");
        let base = t.level_off[level as usize] as usize + layer as usize * gw * gh * bb;
        let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
        for by in 0..bh {
            for bx in 0..bw {
                let idx = (y0 / 4 + by) * gw + x0 / 4 + bx;
                assert_eq!(
                    storage(name, base + idx * bb, bb),
                    blocks[(by * bw + bx) * bb..][..bb],
                    "{what}: {f:?} level {level} layer {layer} block ({bx}, {by})"
                );
            }
        }
        return;
    }
    for y in 0..h {
        for x in 0..w {
            assert_eq!(
                stored(name, level, layer, x0 + x, y0 + y, f.bpp()),
                f.texel(w, h, blocks, x, y),
                "{what}: {f:?} level {level} layer {layer} texel ({}, {})",
                x0 + x,
                y0 + y
            );
        }
    }
}

fn gen_bind(target: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

fn delete(name: GLuint) {
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

fn upload2d(level: GLint, id: GLenum, (w, h): (usize, usize), blocks: &[u8]) {
    // SAFETY: `blocks` is the image, imageSize bytes.
    unsafe {
        glCompressedTexImage2D(
            GL_TEXTURE_2D,
            level,
            id,
            w as GLsizei,
            h as GLsizei,
            0,
            blocks.len() as GLsizei,
            blocks.as_ptr() as *const c_void,
        )
    };
}

fn sub2d(id: GLenum, level: GLint, (x, y): (usize, usize), (w, h): (usize, usize), blocks: &[u8]) {
    // SAFETY: `blocks` is the image, imageSize bytes.
    unsafe {
        glCompressedTexSubImage2D(
            GL_TEXTURE_2D,
            level,
            x as GLint,
            y as GLint,
            w as GLsizei,
            h as GLsizei,
            id,
            blocks.len() as GLsizei,
            blocks.as_ptr() as *const c_void,
        )
    };
}

/// A buffer in GL_PIXEL_UNPACK_BUFFER holding 8 junk bytes, then `blocks`.
fn bind_pbo(blocks: &[u8]) {
    let mut buf = 0;
    let mut padded = vec![0xeeu8; 8];
    padded.extend_from_slice(blocks);
    // SAFETY: the buffer is filled from `padded`, then read at offset 8.
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
        glBufferData(GL_PIXEL_UNPACK_BUFFER, padded.len() as isize, padded.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
}

fn unbind_pbo() {
    // SAFETY: plain GL call.
    unsafe { glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0) };
}

/// Run `body` for each of the ten names, the context held.
fn each(body: impl Fn(&str, GLenum, Fmt)) {
    let _g = context();
    for (name, id, f) in TEN {
        body(name, id, f);
        assert_eq!(error(), GL_NO_ERROR, "{name}: an error left over");
    }
}

/* ---- the tests ------------------------------------------------------------ */

#[test]
fn image_2d_at_level_0_and_a_smaller_level_off_the_block_grid() {
    each(|n, id, f| {
        let name = gen_bind(GL_TEXTURE_2D);
        /* 5x3 is two blocks wide, one high; level 1 is 2x1, one block */
        let l0 = image(f, 5, 3, 11);
        upload2d(0, id, (5, 3), &l0);
        assert_eq!(error(), GL_NO_ERROR, "{n}: 5x3 level 0");
        let l1 = image(f, 2, 1, 12);
        upload2d(1, id, (2, 1), &l1);
        assert_eq!(error(), GL_NO_ERROR, "{n}: 2x1 level 1");
        check_image(name, f, (0, 0, 0, 0), (5, 3), &l0, "level 0");
        check_image(name, f, (1, 0, 0, 0), (2, 1), &l1, "level 1");
        /* and a size on the grid, with a level below it */
        let name2 = gen_bind(GL_TEXTURE_2D);
        let a = image(f, 8, 4, 13);
        upload2d(0, id, (8, 4), &a);
        let b = image(f, 4, 2, 14);
        upload2d(1, id, (4, 2), &b);
        assert_eq!(error(), GL_NO_ERROR, "{n}: 8x4 and 4x2");
        check_image(name2, f, (0, 0, 0, 0), (8, 4), &a, "8x4");
        check_image(name2, f, (1, 0, 0, 0), (4, 2), &b, "4x2");
        delete(name);
        delete(name2);
    });
}

#[test]
fn sub_image_2d_on_a_4_aligned_rectangle() {
    each(|n, id, f| {
        let name = gen_bind(GL_TEXTURE_2D);
        let whole = image(f, 16, 4, 21);
        upload2d(0, id, (16, 4), &whole);
        let patch = image(f, 8, 4, 22);
        sub2d(id, 0, (4, 0), (8, 4), &patch);
        assert_eq!(error(), GL_NO_ERROR, "{n}: sub-image (4,0) 8x4");
        check_image(name, f, (0, 0, 4, 0), (8, 4), &patch, "patched");
        /* the blocks around it are the whole upload's */
        let bb = f.block();
        let left = &whole[..bb];
        let right = &whole[bb * 3..];
        check_image(name, f, (0, 0, 0, 0), (4, 4), left, "left of the patch");
        check_image(name, f, (0, 0, 12, 0), (4, 4), right, "right of the patch");
        /* a smaller level takes a sub-image too */
        let l1 = image(f, 8, 2, 23);
        upload2d(1, id, (8, 2), &l1);
        let p1 = image(f, 4, 2, 24);
        sub2d(id, 1, (4, 0), (4, 2), &p1);
        assert_eq!(error(), GL_NO_ERROR, "{n}: level 1 sub-image");
        check_image(name, f, (1, 0, 4, 0), (4, 2), &p1, "level 1 patched");
        check_image(name, f, (1, 0, 0, 0), (4, 2), &l1[..bb], "level 1 left");
        delete(name);
    });
}

#[test]
fn a_2d_array_of_two_layers_takes_image_3d_and_sub_image_3d() {
    each(|n, id, f| {
        let name = gen_bind(GL_TEXTURE_2D_ARRAY);
        let layers = [image(f, 8, 4, 31), image(f, 8, 4, 32)];
        let data = layers.concat();
        // SAFETY: `data` holds two images, imageSize bytes.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                id,
                8,
                4,
                2,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{n}: TexImage3D 8x4x2");
        for (l, blocks) in layers.iter().enumerate() {
            check_image(name, f, (0, l as u32, 0, 0), (8, 4), blocks, "array image");
        }
        /* layer 1, right half, replaced */
        let patch = image(f, 4, 4, 33);
        // SAFETY: `patch` is the box, imageSize bytes.
        unsafe {
            glCompressedTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                4,
                0,
                1,
                4,
                4,
                1,
                id,
                patch.len() as GLsizei,
                patch.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{n}: TexSubImage3D layer 1");
        check_image(name, f, (0, 1, 4, 0), (4, 4), &patch, "layer 1 patched");
        check_image(name, f, (0, 1, 0, 0), (4, 4), &layers[1][..f.block()], "layer 1 left kept");
        check_image(name, f, (0, 0, 0, 0), (8, 4), &layers[0], "layer 0 untouched");
        /* both layers at once */
        let both = [image(f, 4, 4, 34), image(f, 4, 4, 35)].concat();
        // SAFETY: `both` is two boxes, imageSize bytes.
        unsafe {
            glCompressedTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                0,
                0,
                0,
                4,
                4,
                2,
                id,
                both.len() as GLsizei,
                both.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{n}: TexSubImage3D both layers");
        let bb = both.len() / 2;
        check_image(name, f, (0, 0, 0, 0), (4, 4), &both[..bb], "both, layer 0");
        check_image(name, f, (0, 1, 0, 0), (4, 4), &both[bb..], "both, layer 1");
        delete(name);
    });
}

/// Two layers of `(w, h)` at level 0 and of `(w >> 1, h >> 1)` at level 1,
/// uploaded and checked.
fn array_with_a_smaller_level(n: &str, id: GLenum, f: Fmt, (w, h): (usize, usize)) {
    let name = gen_bind(GL_TEXTURE_2D_ARRAY);
    let (w1, h1) = ((w >> 1).max(1), (h >> 1).max(1));
    let l0 = [image(f, w, h, 41), image(f, w, h, 42)].concat();
    let l1 = [image(f, w1, h1, 43), image(f, w1, h1, 44)].concat();
    for (level, (w, h), data) in [(0, (w, h), &l0), (1, (w1, h1), &l1)] {
        // SAFETY: `data` holds two images, imageSize bytes.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                level,
                id,
                w as GLsizei,
                h as GLsizei,
                2,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{n}: {w}x{h}x2 level {level}");
    }
    let (a, b) = (l0.len() / 2, l1.len() / 2);
    check_image(name, f, (0, 0, 0, 0), (w, h), &l0[..a], "layer 0, level 0");
    check_image(name, f, (0, 1, 0, 0), (w, h), &l0[a..], "layer 1, level 0");
    check_image(name, f, (1, 0, 0, 0), (w1, h1), &l1[..b], "layer 0, level 1");
    check_image(name, f, (1, 1, 0, 0), (w1, h1), &l1[b..], "layer 1, level 1");
    delete(name);
}

#[test]
fn a_2d_array_with_a_smaller_level() {
    each(|n, id, f| array_with_a_smaller_level(n, id, f, (8, 4)));
}

/// ES 3.0 allows a mip level 1 of a 5x3 array like any other level. The
/// four names kept as blocks store it; the six decoded ones do not: the
/// array is one level (`chain_levels` 1, as an uncompressed non-power-of-two
/// array), the level 1 call is accepted and its texels go nowhere. A 5x3
/// 2D texture (`image_2d_at_level_0_and_a_smaller_level_off_the_block_grid`)
/// does store level 1.
#[test]
fn a_2d_array_off_the_block_grid_with_a_smaller_level() {
    each(|n, id, f| array_with_a_smaller_level(n, id, f, (5, 3)));
}

#[test]
fn a_pixel_unpack_buffer_source_at_a_nonzero_offset() {
    each(|n, id, f| {
        let name = gen_bind(GL_TEXTURE_2D);
        let blocks = image(f, 8, 4, 51);
        bind_pbo(&blocks);
        // SAFETY: with a buffer bound the pointer is an offset (8) into it.
        unsafe { glCompressedTexImage2D(GL_TEXTURE_2D, 0, id, 8, 4, 0, blocks.len() as GLsizei, 8 as *const c_void) };
        assert_eq!(error(), GL_NO_ERROR, "{n}: TexImage2D from the buffer");
        unbind_pbo();
        check_image(name, f, (0, 0, 0, 0), (8, 4), &blocks, "image from the buffer");
        /* a sub-image from the buffer, over the right half */
        let patch = image(f, 4, 4, 52);
        bind_pbo(&patch);
        // SAFETY: as above.
        unsafe {
            glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 0, 4, 4, id, patch.len() as GLsizei, 8 as *const c_void)
        };
        assert_eq!(error(), GL_NO_ERROR, "{n}: TexSubImage2D from the buffer");
        unbind_pbo();
        check_image(name, f, (0, 0, 4, 0), (4, 4), &patch, "sub-image from the buffer");
        check_image(name, f, (0, 0, 0, 0), (4, 4), &blocks[..f.block()], "left half kept");
        delete(name);
        /* an array layer from the buffer */
        let name = gen_bind(GL_TEXTURE_2D_ARRAY);
        let layers = [image(f, 4, 4, 53), image(f, 4, 4, 54)].concat();
        bind_pbo(&layers);
        // SAFETY: as above.
        unsafe {
            glCompressedTexImage3D(GL_TEXTURE_2D_ARRAY, 0, id, 4, 4, 2, 0, layers.len() as GLsizei, 8 as *const c_void)
        };
        assert_eq!(error(), GL_NO_ERROR, "{n}: TexImage3D from the buffer");
        unbind_pbo();
        let half = layers.len() / 2;
        check_image(name, f, (0, 0, 0, 0), (4, 4), &layers[..half], "array layer 0 from the buffer");
        check_image(name, f, (0, 1, 0, 0), (4, 4), &layers[half..], "array layer 1 from the buffer");
        delete(name);
    });
}

#[test]
fn texture_3d_is_invalid_operation() {
    each(|n, id, f| {
        gen_bind(GL_TEXTURE_3D);
        let data = vec![0u8; f.block() * 2];
        // SAFETY: `data` holds the imageSize.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_3D,
                0,
                id,
                4,
                4,
                1,
                0,
                f.block() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_INVALID_OPERATION, "{n}: TexImage3D into GL_TEXTURE_3D");
        // SAFETY: as above.
        unsafe {
            glCompressedTexSubImage3D(
                GL_TEXTURE_3D,
                0,
                0,
                0,
                0,
                4,
                4,
                1,
                id,
                f.block() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_INVALID_OPERATION, "{n}: TexSubImage3D into GL_TEXTURE_3D");
    });
}
