// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The six ES 3.0 ETC2/EAC formats the GL core decodes at upload
//! (docs/notes/etc2-eac.md): GL_COMPRESSED_R11_EAC, SIGNED_R11_EAC,
//! RG11_EAC, SIGNED_RG11_EAC, RGB8_PUNCHTHROUGH_ALPHA1_ETC2 and
//! SRGB8_PUNCHTHROUGH_ALPHA1_ETC2.
//!
//! Through the API, on a stand-in back end (the OSAL maps a GPU address to
//! host memory; no GPU runs), the test pins:
//! * each name is accepted by glCompressedTexImage2D, a texel of the stored
//!   level is the decoder's value (`yakogl::gl::etc2`, itself checked against
//!   Mesa's outputs and dEQP's reference) in the class the design gives it
//!   (R16F, RGBA16F, RGBA8, SRGB8_ALPHA8), for a size that is not a multiple
//!   of four, every level of a chain, the faces of a cube map, the layers of
//!   a 2D array, and a source in a PIXEL_UNPACK_BUFFER;
//! * glCompressedTexSubImage2D / 3D with the same name decodes into the
//!   texels, with another name or off the block grid it is an error;
//! * the texels are not an image: glTexSubImage2D, glCopyTexSubImage2D and
//!   glGenerateMipmap refuse them, and a framebuffer cannot render to them;
//!   a new level 0 (glTexImage2D) makes the texture an ordinary one;
//! * GL_NUM_COMPRESSED_TEXTURE_FORMATS / GL_COMPRESSED_TEXTURE_FORMATS list
//!   ETC1 and the ten.
//!
//! What it cannot show: that a GPU samples the classes within dEQP's
//! thresholds (a board does; tests/es3_decoded_class_v3d.rs runs the V3D's
//! look-up on the interpreter), and dEQP's pictures.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_eac
//! ```
#![cfg(all(feature = "host-test", any(feature = "gpu-powervr", feature = "gpu-vivante"), not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlTexture};
use yakogl::gl::es3_formats::f32_to_half;
use yakogl::gl::etc2;
use yakogl::gl::framebuffer::{glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
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

/// A back end of the build's GPU mask (`backend::sampled_texfmts`): the
/// classes the decode lands in are sampled on the PowerVR and the GC7000.
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

/* ---- the six formats ---------------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind6 {
    R11,
    R11S,
    Rg11,
    Rg11S,
    Pt,
    PtSrgb,
}

const SIX: [(GLenum, Kind6); 6] = [
    (GL_COMPRESSED_R11_EAC, Kind6::R11),
    (GL_COMPRESSED_SIGNED_R11_EAC, Kind6::R11S),
    (GL_COMPRESSED_RG11_EAC, Kind6::Rg11),
    (GL_COMPRESSED_SIGNED_RG11_EAC, Kind6::Rg11S),
    (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, Kind6::Pt),
    (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, Kind6::PtSrgb),
];

impl Kind6 {
    /// Bytes per 4x4 block, and per stored texel.
    fn block(self) -> usize {
        match self {
            Kind6::Rg11 | Kind6::Rg11S => 16,
            _ => 8,
        }
    }
    fn bpp(self) -> usize {
        match self {
            Kind6::R11 | Kind6::R11S => 2,
            Kind6::Rg11 | Kind6::Rg11S => 8,
            Kind6::Pt | Kind6::PtSrgb => 4,
        }
    }
    /// The texel (x, y) of the w x h image `blocks`, as the class stores it.
    fn texel(self, w: usize, h: usize, blocks: &[u8], x: usize, y: usize) -> Vec<u8> {
        let half = |v: f32| f32_to_half(v).to_le_bytes();
        let n = w * h;
        match self {
            Kind6::R11 => {
                let mut o = vec![0u16; n];
                assert!(etc2::decode_eac11_unsigned_image(w, h, 1, blocks, &mut o));
                half(f32::from(o[y * w + x]) / 65535.0).to_vec()
            }
            Kind6::R11S => {
                let mut o = vec![0i16; n];
                assert!(etc2::decode_eac11_signed_image(w, h, 1, blocks, &mut o));
                half((f32::from(o[y * w + x]) / 32767.0).max(-1.0)).to_vec()
            }
            Kind6::Rg11 => {
                let mut o = vec![0u16; n * 2];
                assert!(etc2::decode_eac11_unsigned_image(w, h, 2, blocks, &mut o));
                let v = |c: usize| half(f32::from(o[(y * w + x) * 2 + c]) / 65535.0);
                [v(0), v(1), [0, 0], half(1.0)].concat()
            }
            Kind6::Rg11S => {
                let mut o = vec![0i16; n * 2];
                assert!(etc2::decode_eac11_signed_image(w, h, 2, blocks, &mut o));
                let v = |c: usize| half((f32::from(o[(y * w + x) * 2 + c]) / 32767.0).max(-1.0));
                [v(0), v(1), [0, 0], half(1.0)].concat()
            }
            Kind6::Pt | Kind6::PtSrgb => {
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
fn image(k: Kind6, w: usize, h: usize, seed: u32) -> Vec<u8> {
    bytes(seed, w.div_ceil(4) * h.div_ceil(4) * k.block())
}

fn texture_object(name: GLuint) -> &'static GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The stored texel (x, y) of image `layer` (a cube face, an array layer) of
/// `level`, `bpp` bytes, as the GL core keeps it: layers stacked under one
/// another in every level, rows at the level's pitch.
fn stored(name: GLuint, level: u32, layer: u32, x: usize, y: usize, bpp: usize) -> Vec<u8> {
    let t = texture_object(name);
    let (lw, lh) = (((t.width >> level).max(1)) as usize, ((t.height >> level).max(1)) as usize);
    let pitch = if t.pot_chain { t.level_pitch_at(level) } else { t.level_pitch(lw as u32) } as usize;
    assert_eq!(usize::from(t.bpp), bpp, "texel size of the class");
    let at = t.level_off[level as usize] as usize + (((layer as usize * lh + y) * pitch) + x) * bpp;
    // SAFETY: the mock OSAL maps the storage to host memory; the texel lies inside the level.
    unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), bpp) }.to_vec()
}

/// Every texel of a w x h image at (`level`, `layer`) is the decoder's.
#[allow(clippy::too_many_arguments)]
fn check_image(name: GLuint, k: Kind6, level: u32, layer: u32, w: usize, h: usize, blocks: &[u8], what: &str) {
    for y in 0..h {
        for x in 0..w {
            assert_eq!(
                stored(name, level, layer, x, y, k.bpp()),
                k.texel(w, h, blocks, x, y),
                "{what}: {k:?} level {level} layer {layer} texel ({x}, {y})"
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

fn upload2d(target: GLenum, level: GLint, f: GLenum, w: usize, h: usize, blocks: &[u8]) {
    // SAFETY: `blocks` is the image, imageSize bytes.
    unsafe {
        glCompressedTexImage2D(
            target,
            level,
            f,
            w as GLsizei,
            h as GLsizei,
            0,
            blocks.len() as GLsizei,
            blocks.as_ptr() as *const c_void,
        )
    };
}

fn sub2d(f: GLenum, level: GLint, (x, y): (usize, usize), (w, h): (usize, usize), blocks: &[u8]) {
    // SAFETY: `blocks` is the image, imageSize bytes.
    unsafe {
        glCompressedTexSubImage2D(
            GL_TEXTURE_2D,
            level,
            x as GLint,
            y as GLint,
            w as GLsizei,
            h as GLsizei,
            f,
            blocks.len() as GLsizei,
            blocks.as_ptr() as *const c_void,
        )
    };
}

/* ---- the tests ----------------------------------------------------------- */

#[test]
fn each_format_is_stored_as_the_decoders_texels_in_its_class() {
    let _g = context();
    for (i, (f, k)) in SIX.into_iter().enumerate() {
        for (w, h) in [(8, 8), (16, 4), (4, 4)] {
            let name = gen_bind(GL_TEXTURE_2D);
            let blocks = image(k, w, h, 7 + i as u32);
            upload2d(GL_TEXTURE_2D, 0, f, w, h, &blocks);
            assert_eq!(error(), GL_NO_ERROR, "{k:?} {w}x{h}");
            check_image(name, k, 0, 0, w, h, &blocks, "level 0");
            let t = texture_object(name);
            assert_eq!(t.decoded_from, f as u16, "the texture records its name");
            let class = match k {
                Kind6::R11 | Kind6::R11S => backend::GLES_TEXFMT_R16F,
                Kind6::Rg11 | Kind6::Rg11S => backend::GLES_TEXFMT_RGBA16F,
                Kind6::Pt => backend::GLES_TEXFMT_RGBA8,
                Kind6::PtSrgb => backend::GLES_TEXFMT_SRGB8_A8,
            };
            assert_eq!((t.texfmt, t.block_bytes), (class, 0), "{k:?}: the class, not a block layout");
            // SAFETY: a name this test made.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
}

#[test]
fn a_size_off_the_block_grid_cuts_the_edge_blocks() {
    let _g = context();
    for (f, k) in SIX {
        let (w, h) = (10, 6);
        let name = gen_bind(GL_TEXTURE_2D);
        let blocks = image(k, w, h, 31);
        upload2d(GL_TEXTURE_2D, 0, f, w, h, &blocks);
        assert_eq!(error(), GL_NO_ERROR, "{k:?}");
        check_image(name, k, 0, 0, w, h, &blocks, "10x6");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn every_level_of_a_chain_is_decoded() {
    let _g = context();
    for (f, k) in SIX {
        let name = gen_bind(GL_TEXTURE_2D);
        let mut all = Vec::new();
        for l in 0..4u32 {
            let s = 16usize >> l;
            let blocks = image(k, s, s, 100 + l);
            upload2d(GL_TEXTURE_2D, l as GLint, f, s, s, &blocks);
            assert_eq!(error(), GL_NO_ERROR, "{k:?} level {l}");
            all.push((s, blocks));
        }
        for (l, (s, blocks)) in all.iter().enumerate() {
            check_image(name, k, l as u32, 0, *s, *s, blocks, "chain");
        }
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn the_six_faces_of_a_cube_map_are_decoded() {
    let _g = context();
    for (f, k) in SIX {
        let name = gen_bind(GL_TEXTURE_CUBE_MAP);
        let faces: Vec<Vec<u8>> = (0..6).map(|i| image(k, 8, 8, 200 + i)).collect();
        for (i, blocks) in faces.iter().enumerate() {
            upload2d(GL_TEXTURE_CUBE_MAP_POSITIVE_X + i as GLenum, 0, f, 8, 8, blocks);
            assert_eq!(error(), GL_NO_ERROR, "{k:?} face {i}");
        }
        for (i, blocks) in faces.iter().enumerate() {
            check_image(name, k, 0, i as u32, 8, 8, blocks, "cube");
        }
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn the_layers_of_a_2d_array_are_decoded_and_a_layer_is_replaced_by_the_sub_image_call() {
    let _g = context();
    for (f, k) in SIX {
        let name = gen_bind(GL_TEXTURE_2D_ARRAY);
        let layers: Vec<Vec<u8>> = (0..3).map(|i| image(k, 8, 8, 300 + i)).collect();
        let data = layers.concat();
        // SAFETY: `data` holds three images, imageSize bytes.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                f,
                8,
                8,
                3,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{k:?}");
        for (i, blocks) in layers.iter().enumerate() {
            check_image(name, k, 0, i as u32, 8, 8, blocks, "array");
        }
        assert_eq!(texture_object(name).decoded_from, f as u16);
        /* layer 1 again, whole */
        let again = image(k, 8, 8, 999);
        // SAFETY: one image, imageSize bytes.
        unsafe {
            glCompressedTexSubImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                0,
                0,
                1,
                8,
                8,
                1,
                f,
                again.len() as GLsizei,
                again.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "{k:?} sub image 3D");
        check_image(name, k, 0, 0, 8, 8, &layers[0], "layer 0 kept");
        check_image(name, k, 0, 1, 8, 8, &again, "layer 1 replaced");
        check_image(name, k, 0, 2, 8, 8, &layers[2], "layer 2 kept");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn a_sub_image_of_the_same_name_decodes_into_the_texels() {
    let _g = context();
    for (f, k) in SIX {
        let name = gen_bind(GL_TEXTURE_2D);
        let base = image(k, 16, 16, 400);
        upload2d(GL_TEXTURE_2D, 0, f, 16, 16, &base);
        /* the block at (4, 8), 8 wide and 4 high: two blocks */
        let patch = image(k, 8, 4, 401);
        sub2d(f, 0, (4, 8), (8, 4), &patch);
        assert_eq!(error(), GL_NO_ERROR, "{k:?}");
        for y in 0..16 {
            for x in 0..16 {
                let inside = (4..12).contains(&x) && (8..12).contains(&y);
                let want = if inside { k.texel(8, 4, &patch, x - 4, y - 8) } else { k.texel(16, 16, &base, x, y) };
                assert_eq!(stored(name, 0, 0, x, y, k.bpp()), want, "{k:?} texel ({x}, {y})");
            }
        }
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn a_sub_image_is_checked_as_for_any_compressed_format() {
    let _g = context();
    let (f, k) = SIX[0]; /* R11_EAC */
    let name = gen_bind(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, f, 16, 16, &image(k, 16, 16, 1));
    assert_eq!(error(), GL_NO_ERROR);
    let one = image(k, 4, 4, 2);
    /* another name than the texture's: GL_INVALID_OPERATION (3.8.6) */
    sub2d(GL_COMPRESSED_SIGNED_R11_EAC, 0, (0, 0), (4, 4), &one);
    assert_eq!(error(), GL_INVALID_OPERATION, "SIGNED_R11 into R11 texels");
    sub2d(GL_COMPRESSED_RGBA8_ETC2_EAC, 0, (0, 0), (4, 4), &image(Kind6::Rg11, 4, 4, 2));
    assert_eq!(error(), GL_INVALID_OPERATION, "a native name into decoded texels");
    /* off the block grid */
    sub2d(f, 0, (2, 0), (4, 4), &one);
    assert_eq!(error(), GL_INVALID_OPERATION, "xoffset 2");
    sub2d(f, 0, (0, 0), (6, 4), &image(k, 6, 4, 3));
    assert_eq!(error(), GL_INVALID_OPERATION, "width 6 short of the level's edge");
    /* outside the level, a wrong imageSize, a level never specified */
    sub2d(f, 0, (16, 0), (4, 4), &one);
    assert_eq!(error(), GL_INVALID_VALUE, "outside");
    // SAFETY: imageSize 7 is a size error caught before the data is read.
    unsafe { glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, f, 7, one.as_ptr() as *const c_void) };
    assert_eq!(error(), GL_INVALID_VALUE, "imageSize");
    sub2d(f, 1, (0, 0), (4, 4), &one);
    assert_eq!(error(), GL_INVALID_OPERATION, "level 1 is not specified");
    /* a texture that was not decoded from the name: ordinary R16F */
    let plain = gen_bind(GL_TEXTURE_2D);
    // SAFETY: no pixels.
    unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_R16F as GLint, 16, 16, 0, GL_RED, GL_HALF_FLOAT, core::ptr::null()) };
    assert_eq!(error(), GL_NO_ERROR);
    sub2d(f, 0, (0, 0), (4, 4), &one);
    assert_eq!(error(), GL_INVALID_OPERATION, "an R16F texture is not R11_EAC");
    // SAFETY: names this test made.
    unsafe { glDeleteTextures(1, &name) };
    unsafe { glDeleteTextures(1, &plain) };
}

#[test]
fn the_texels_are_not_an_image_of_their_class() {
    let _g = context();
    let (f, k) = (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, Kind6::Pt);
    let name = gen_bind(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, f, 8, 8, &image(k, 8, 8, 5));
    assert_eq!(error(), GL_NO_ERROR);
    /* glTexSubImage2D of a compressed texture */
    let px = [0u8; 16];
    // SAFETY: four RGBA texels.
    unsafe { glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void) };
    assert_eq!(error(), GL_INVALID_OPERATION, "glTexSubImage2D");
    // SAFETY: the read framebuffer is the default surface.
    unsafe { glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 2, 2) };
    assert_eq!(error(), GL_INVALID_OPERATION, "glCopyTexSubImage2D");
    // SAFETY: a bound texture.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(error(), GL_INVALID_OPERATION, "glGenerateMipmap");
    /* not colour-renderable */
    let mut fb = 0;
    // SAFETY: room for one name; the texture is the level 0 image.
    let status = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
        let s = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        s
    };
    assert_ne!(status, GL_FRAMEBUFFER_COMPLETE, "a decoded compressed texture is no render target");
    while error() != GL_NO_ERROR {}
    /* the same RGBA8 image from glTexImage2D is one of them (the rule above
     * is about the name, not the class) */
    let plain = gen_bind(GL_TEXTURE_2D);
    // SAFETY: no pixels.
    unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null()) };
    // SAFETY: a bound texture.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    assert_eq!(error(), GL_NO_ERROR, "an ordinary RGBA8 texture mipmaps");
    // SAFETY: names this test made.
    unsafe { glDeleteTextures(1, &name) };
    unsafe { glDeleteTextures(1, &plain) };
}

#[test]
fn a_new_level_0_makes_the_texture_an_ordinary_one_again() {
    let _g = context();
    let (f, k) = SIX[2]; /* RG11_EAC, RGBA16F */
    let name = gen_bind(GL_TEXTURE_2D);
    upload2d(GL_TEXTURE_2D, 0, f, 8, 8, &image(k, 8, 8, 6));
    assert_eq!(texture_object(name).decoded_from, f as u16);
    // SAFETY: no pixels.
    unsafe { glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA16F as GLint, 8, 8, 0, GL_RGBA, GL_HALF_FLOAT, core::ptr::null()) };
    assert_eq!(error(), GL_NO_ERROR);
    assert_eq!(texture_object(name).decoded_from, 0);
    sub2d(f, 0, (0, 0), (4, 4), &image(k, 4, 4, 7));
    assert_eq!(error(), GL_INVALID_OPERATION, "no longer RG11_EAC texels");
    /* and another compressed name takes over */
    upload2d(GL_TEXTURE_2D, 0, GL_COMPRESSED_R11_EAC, 8, 8, &image(Kind6::R11, 8, 8, 8));
    assert_eq!(texture_object(name).decoded_from, GL_COMPRESSED_R11_EAC as u16);
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

#[test]
fn a_source_in_a_pixel_unpack_buffer_decodes_the_same() {
    let _g = context();
    for (f, k) in SIX {
        let blocks = image(k, 8, 8, 500);
        let mut buf = 0;
        // SAFETY: the buffer is filled with the blocks, then read at offset 8.
        unsafe {
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
            let mut padded = vec![0xeeu8; 8];
            padded.extend_from_slice(&blocks);
            glBufferData(
                GL_PIXEL_UNPACK_BUFFER,
                padded.len() as isize,
                padded.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );
        }
        let name = gen_bind(GL_TEXTURE_2D);
        // SAFETY: with a buffer bound the pointer is an offset (8) into it.
        unsafe { glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 8, 8, 0, blocks.len() as GLsizei, 8 as *const c_void) };
        assert_eq!(error(), GL_NO_ERROR, "{k:?}");
        check_image(name, k, 0, 0, 8, 8, &blocks, "from the buffer");
        // SAFETY: names this test made.
        unsafe {
            glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            glDeleteTextures(1, &name);
        }
    }
}

#[test]
fn the_argument_errors_of_a_compressed_image_hold_for_the_six() {
    let _g = context();
    for (f, k) in SIX {
        let name = gen_bind(GL_TEXTURE_2D);
        let blocks = image(k, 8, 8, 9);
        // SAFETY: a size error is found before the data is read.
        unsafe {
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                f,
                8,
                8,
                0,
                blocks.len() as GLsizei - 1,
                blocks.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_INVALID_VALUE, "{k:?}: imageSize");
        // SAFETY: as above.
        unsafe {
            glCompressedTexImage2D(
                GL_TEXTURE_2D,
                0,
                f,
                8,
                8,
                1,
                blocks.len() as GLsizei,
                blocks.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_INVALID_VALUE, "{k:?}: border");
        /* no 3D form (ES 3.0 3.8.6) */
        let vol = gen_bind(GL_TEXTURE_3D);
        // SAFETY: as above.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_3D,
                0,
                f,
                4,
                4,
                1,
                0,
                k.block() as GLsizei,
                blocks.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_INVALID_OPERATION, "{k:?}: 3D");
        // SAFETY: names this test made.
        unsafe {
            glDeleteTextures(1, &name);
            glDeleteTextures(1, &vol);
        }
    }
}

#[test]
fn the_format_list_holds_etc1_and_the_ten() {
    let _g = context();
    let (mut n, mut list) = (0, [0 as GLint; 16]);
    // SAFETY: room for the 16 values the list may hold.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr());
    }
    assert_eq!(error(), GL_NO_ERROR);
    let list = &list[..n as usize];
    assert_eq!(n, 11, "ETC1 and the ten");
    for f in [
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
    ] {
        assert_eq!(list.iter().filter(|&&x| x as GLenum == f).count(), 1, "0x{f:x} once in the list");
    }
}
