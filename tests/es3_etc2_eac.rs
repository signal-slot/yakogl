// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's six ETC2/EAC formats the GL core decodes at upload
//! (docs/notes/etc2-eac.md; plan #354): `GL_COMPRESSED_R11_EAC`,
//! `SIGNED_R11_EAC`, `RG11_EAC`, `SIGNED_RG11_EAC` and the two
//! `PUNCHTHROUGH_ALPHA1` formats, through the GL API on a back end that
//! samples the classes they decode into (R16F, RGBA16F, RGBA8, SRGB8_A8).
//!
//! Pinned here, on the stored texels (read from the texture object's memory,
//! compared with `gl::etc2`'s block decode, itself tested against Mesa's in
//! its own module):
//! - the format list: ETC1 and ES 3.0's ten, 11 names (under report-es2 the
//!   file compiles to nothing; the list of three is tests/es3_caps.rs's);
//! - 2D images of every format, partial edge blocks (a size that is not a
//!   multiple of 4), a mip chain, cube faces, a 2D array's layers;
//! - `glCompressedTexSubImage2D/3D` of a block at an offset;
//! - a PIXEL_UNPACK_BUFFER source, whatever the pixel-store state (the
//!   decode reads blocks, not texels);
//! - the rules of a compressed texture: the sub-image of another format,
//!   `glTexSubImage2D`, `glCopyTexSubImage2D`, `glGenerateMipmap` and a
//!   framebuffer attachment are refused, a respecification with another
//!   format lifts them, a 3D target is GL_INVALID_OPERATION, a wrong
//!   imageSize GL_INVALID_VALUE.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_eac
//! ```
#![cfg(all(
    feature = "host-test",
    not(feature = "report-es2"),
    any(feature = "gpu-v3d", feature = "gpu-powervr", feature = "gpu-vivante")
))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3_formats::f32_to_half;
use yakogl::gl::etc2;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glFramebufferTexture2D, glGenFramebuffers,
};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
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

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time. Unpack alignment 1
/// and no error pending on entry.
fn gl() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the guard serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the formats ----------------------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind11 {
    R11,
    SignedR11,
    Rg11,
    SignedRg11,
    Punch,
    SrgbPunch,
}

const FORMATS: [(GLenum, Kind11, usize); 6] = [
    (GL_COMPRESSED_R11_EAC, Kind11::R11, 8),
    (GL_COMPRESSED_SIGNED_R11_EAC, Kind11::SignedR11, 8),
    (GL_COMPRESSED_RG11_EAC, Kind11::Rg11, 16),
    (GL_COMPRESSED_SIGNED_RG11_EAC, Kind11::SignedRg11, 16),
    (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, Kind11::Punch, 8),
    (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, Kind11::SrgbPunch, 8),
];

/// Deterministic block bytes: block `i` of a run, from a xorshift of `seed`.
fn blocks(n: usize, bb: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..n * bb)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// The texel (x, y) of the w x h image `data` of `kind` as the GL core
/// stores it: the half floats (`f32_to_half` of the spec's value) of the
/// channels, or the RGBA8 bytes, from `etc2`'s decode of the block.
fn expected(kind: Kind11, w: usize, data: &[u8], x: usize, y: usize) -> Vec<u8> {
    let bw = w.div_ceil(4);
    let bb = if matches!(kind, Kind11::Rg11 | Kind11::SignedRg11) { 16 } else { 8 };
    let blk = &data[((y / 4) * bw + x / 4) * bb..][..bb];
    let i = (y % 4) * 4 + x % 4;
    let half = |v: f32| f32_to_half(v).to_ne_bytes();
    match kind {
        Kind11::R11 | Kind11::Rg11 => (0..bb / 8)
            .flat_map(|c| {
                half(
                    f32::from(etc2::decode_eac11_unsigned_block(blk[c * 8..c * 8 + 8].try_into().unwrap())[i])
                        / 65535.0,
                )
            })
            .collect(),
        Kind11::SignedR11 | Kind11::SignedRg11 => (0..bb / 8)
            .flat_map(|c| {
                half(
                    (f32::from(etc2::decode_eac11_signed_block(blk[c * 8..c * 8 + 8].try_into().unwrap())[i])
                        / 32767.0)
                        .max(-1.0),
                )
            })
            .collect(),
        Kind11::Punch | Kind11::SrgbPunch => etc2::decode_rgb_block(blk.try_into().unwrap(), true)[i].to_vec(),
    }
}

/// Bytes a texel of the stored class: RG11 stores two halves in RGBA16F's
/// first two channels (the rest zero, alpha one: not compared past the
/// decoded channels).
fn compared_bytes(kind: Kind11) -> usize {
    match kind {
        Kind11::R11 | Kind11::SignedR11 => 2,
        Kind11::Rg11 | Kind11::SignedRg11 => 4,
        Kind11::Punch | Kind11::SrgbPunch => 4,
    }
}

fn new_tex(target: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: plain GL calls, a live out pointer.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The stored bytes of texel (x, y) of layer `layer` of level `level`.
fn texel(name: GLuint, level: u32, layer: u32, x: u32, y: u32, n: usize) -> Vec<u8> {
    let t = texture_object(name);
    let (lw, lh) = (u32::max(t.width >> level, 1), u32::max(t.height >> level, 1));
    let pitch = if t.pot_chain { t.level_pitch_at(level) } else { t.level_pitch(lw) };
    let at = t.level_off[level as usize] as usize + (((layer * lh + y) * pitch + x) as usize) * usize::from(t.bpp);
    // SAFETY: inside the level's storage, which the GL core allocated.
    unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), n) }.to_vec()
}

fn image(target: GLenum, level: GLint, format: GLenum, w: i32, h: i32, data: &[u8]) {
    // SAFETY: `data` is the image imageSize names.
    unsafe {
        glCompressedTexImage2D(target, level, format, w, h, 0, data.len() as GLsizei, data.as_ptr() as *const c_void)
    };
}

/// Every texel of the level against the decode of `data`.
fn check_level(name: GLuint, level: u32, kind: Kind11, w: usize, h: usize, data: &[u8], layer: u32) {
    for y in 0..h {
        for x in 0..w {
            assert_eq!(
                texel(name, level, layer, x as u32, y as u32, compared_bytes(kind)),
                expected(kind, w, data, x, y),
                "{kind:?} {w}x{h} level {level} layer {layer} texel ({x}, {y})"
            );
        }
    }
}

/* ---- the tests ------------------------------------------------------------- */

#[test]
fn the_format_list_is_etc1_and_es3s_ten() {
    let _g = gl();
    let mut n = 0;
    let mut list = [0 as GLint; 16];
    // SAFETY: plain GL calls; room for the widest list.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr());
    }
    assert_eq!(error(), GL_NO_ERROR);
    let want = [
        GL_ETC1_RGB8_OES,
        GL_COMPRESSED_RGB8_ETC2,
        GL_COMPRESSED_RGBA8_ETC2_EAC,
        GL_COMPRESSED_SRGB8_ETC2,
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
        GL_COMPRESSED_R11_EAC,
        GL_COMPRESSED_SIGNED_R11_EAC,
        GL_COMPRESSED_RG11_EAC,
        GL_COMPRESSED_SIGNED_RG11_EAC,
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
    ];
    assert_eq!(n as usize, 11);
    assert_eq!(list[..11], want.map(|f| f as GLint));
    assert!(list[11..].iter().all(|&v| v == 0), "nothing past the list: {list:?}");
}

#[test]
fn every_format_decodes_a_2d_image() {
    let _g = gl();
    for (format, kind, bb) in FORMATS {
        let name = new_tex(GL_TEXTURE_2D);
        let data = blocks(4 * 2, bb, u64::from(format));
        image(GL_TEXTURE_2D, 0, format, 16, 8, &data);
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}");
        let t = texture_object(name);
        assert_eq!((t.block_bytes, t.decoded_from), (0, format as u16), "format {format:#x}: texels, not blocks");
        check_level(name, 0, kind, 16, 8, &data, 0);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn the_decoded_class_is_the_one_every_back_end_samples() {
    let _g = gl();
    let class = |f| {
        let name = new_tex(GL_TEXTURE_2D);
        let (bb, _) = if f == GL_COMPRESSED_RG11_EAC || f == GL_COMPRESSED_SIGNED_RG11_EAC { (16, 0) } else { (8, 0) };
        image(GL_TEXTURE_2D, 0, f, 4, 4, &blocks(1, bb, 3));
        assert_eq!(error(), GL_NO_ERROR);
        let t = texture_object(name);
        let c = (t.texfmt, t.bpp);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
        c
    };
    use backend::{GLES_TEXFMT_R16F, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8, GLES_TEXFMT_SRGB8_A8};
    assert_eq!(class(GL_COMPRESSED_R11_EAC), (GLES_TEXFMT_R16F, 2));
    assert_eq!(class(GL_COMPRESSED_SIGNED_R11_EAC), (GLES_TEXFMT_R16F, 2));
    assert_eq!(class(GL_COMPRESSED_RG11_EAC), (GLES_TEXFMT_RGBA16F, 8));
    assert_eq!(class(GL_COMPRESSED_SIGNED_RG11_EAC), (GLES_TEXFMT_RGBA16F, 8));
    assert_eq!(class(GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2), (GLES_TEXFMT_RGBA8, 4));
    assert_eq!(class(GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2), (GLES_TEXFMT_SRGB8_A8, 4));
}

/// A size that is not a multiple of 4: the edge blocks decode whole and keep
/// the texels inside the level (dEQP's texture.wrap NPOT cases).
#[test]
fn a_partial_edge_block_is_cut_at_the_levels_edge() {
    let _g = gl();
    for (format, kind, bb) in FORMATS {
        let name = new_tex(GL_TEXTURE_2D);
        let (w, h) = (13usize, 9usize);
        let data = blocks(4 * 3, bb, 77 + u64::from(format));
        image(GL_TEXTURE_2D, 0, format, w as i32, h as i32, &data);
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}");
        check_level(name, 0, kind, w, h, &data, 0);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn a_mip_chain_decodes_level_by_level() {
    let _g = gl();
    for (format, kind, bb) in FORMATS {
        let name = new_tex(GL_TEXTURE_2D);
        let sizes = [16usize, 8, 4, 2, 1];
        let datas: Vec<Vec<u8>> = sizes
            .iter()
            .map(|&s| blocks(s.div_ceil(4) * s.div_ceil(4), bb, 5 + s as u64 + u64::from(format)))
            .collect();
        for (l, (&s, d)) in sizes.iter().zip(&datas).enumerate() {
            image(GL_TEXTURE_2D, l as GLint, format, s as i32, s as i32, d);
            assert_eq!(error(), GL_NO_ERROR, "format {format:#x} level {l}");
        }
        for (l, (&s, d)) in sizes.iter().zip(&datas).enumerate() {
            check_level(name, l as u32, kind, s, s, d, 0);
        }
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
    }
}

/// As the native formats: a level that does not fit the chain level 0 laid
/// out is recorded, not an error, and a level uploaded before level 0 is the
/// uncompressed path's (dEQP's texture.mipmap and wrap cases upload in any order).
#[test]
fn a_level_that_does_not_fit_is_recorded_without_an_error() {
    let _g = gl();
    for (format, kind, bb) in FORMATS {
        let name = new_tex(GL_TEXTURE_2D);
        let l0 = blocks(16, bb, 1);
        image(GL_TEXTURE_2D, 0, format, 16, 16, &l0);
        image(GL_TEXTURE_2D, 1, format, 4, 4, &blocks(1, bb, 2)); /* 8 x 8 fits, 4 x 4 does not */
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}: a level of the wrong size");
        check_level(name, 0, kind, 16, 16, &l0, 0);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };

        let name = new_tex(GL_TEXTURE_2D);
        let l1 = blocks(4, bb, 3);
        image(GL_TEXTURE_2D, 1, format, 8, 8, &l1);
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}: level 1 first");
        check_level(name, 1, kind, 8, 8, &l1, 0);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn cube_faces_decode_one_call_each() {
    let _g = gl();
    let (format, kind, bb) = FORMATS[2];
    let name = new_tex(GL_TEXTURE_CUBE_MAP);
    let datas: Vec<Vec<u8>> = (0..6).map(|f| blocks(4, bb, 100 + f)).collect();
    for (f, d) in datas.iter().enumerate() {
        image(GL_TEXTURE_CUBE_MAP_POSITIVE_X + f as u32, 0, format, 8, 8, d);
        assert_eq!(error(), GL_NO_ERROR, "face {f}");
    }
    for (f, d) in datas.iter().enumerate() {
        check_level(name, 0, kind, 8, 8, d, f as u32);
    }
    // SAFETY: a name made above.
    unsafe { glDeleteTextures(1, &name) };
}

#[test]
fn a_2d_array_decodes_each_layer_and_a_sub_image_replaces_a_block() {
    let _g = gl();
    for (format, kind, bb) in FORMATS {
        let name = new_tex(GL_TEXTURE_2D_ARRAY);
        let layers = 3usize;
        let data = blocks(4 * layers, bb, 9 + u64::from(format));
        // SAFETY: `data` holds the 3 images of 4 blocks imageSize names.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                0,
                format,
                8,
                8,
                layers as GLsizei,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}");
        let image_bytes = 4 * bb;
        for z in 0..layers {
            check_level(name, 0, kind, 8, 8, &data[z * image_bytes..][..image_bytes], z as u32);
        }
        /* layer 1's block (1, 0) replaced */
        let new = blocks(1, bb, 4242);
        // SAFETY: one block.
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
                format,
                bb as GLsizei,
                new.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}: sub-image");
        let mut want = data[image_bytes..][..image_bytes].to_vec();
        want[bb..2 * bb].copy_from_slice(&new);
        check_level(name, 0, kind, 8, 8, &want, 1);
        check_level(name, 0, kind, 8, 8, &data[..image_bytes], 0);
        check_level(name, 0, kind, 8, 8, &data[2 * image_bytes..][..image_bytes], 2);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn a_2d_sub_image_replaces_blocks_at_an_offset_and_a_cube_face_takes_one() {
    let _g = gl();
    for (format, kind, bb) in FORMATS {
        let name = new_tex(GL_TEXTURE_2D);
        let data = blocks(4 * 4, bb, 31 + u64::from(format));
        image(GL_TEXTURE_2D, 0, format, 16, 16, &data);
        /* the 2x2 blocks at (4, 8) */
        let new = blocks(4, bb, 99);
        // SAFETY: four blocks.
        unsafe {
            glCompressedTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                4,
                8,
                8,
                8,
                format,
                new.len() as GLsizei,
                new.as_ptr() as *const c_void,
            )
        };
        assert_eq!(error(), GL_NO_ERROR, "format {format:#x}");
        let mut want = data.clone();
        for by in 0..2 {
            for bx in 0..2 {
                let dst = ((2 + by) * 4 + 1 + bx) * bb;
                want[dst..dst + bb].copy_from_slice(&new[(by * 2 + bx) * bb..][..bb]);
            }
        }
        check_level(name, 0, kind, 16, 16, &want, 0);
        // SAFETY: a name made above.
        unsafe { glDeleteTextures(1, &name) };
    }
}

#[test]
fn the_unpack_buffer_is_read_whatever_the_pixel_store_state() {
    let _g = gl();
    let (format, kind, bb) = FORMATS[3];
    let data = blocks(4, bb, 8);
    let (mut buf, name) = (0, new_tex(GL_TEXTURE_2D));
    // SAFETY: plain GL calls; `data` is the buffer's contents.
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
        glBufferData(GL_PIXEL_UNPACK_BUFFER, data.len() as isize, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        /* none of these may move the blocks */
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 100);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 3);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, 5);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, 8, 8, 0, data.len() as GLsizei, core::ptr::null());
    }
    assert_eq!(error(), GL_NO_ERROR);
    check_level(name, 0, kind, 8, 8, &data, 0);
    let mut state = [0; 4];
    // SAFETY: plain GL calls with live out pointers.
    unsafe {
        glGetIntegerv(GL_UNPACK_ROW_LENGTH, &mut state[0]);
        glGetIntegerv(GL_UNPACK_SKIP_PIXELS, &mut state[1]);
        glGetIntegerv(GL_UNPACK_SKIP_ROWS, &mut state[2]);
        glGetIntegerv(GL_UNPACK_ALIGNMENT, &mut state[3]);
    }
    assert_eq!(state, [100, 3, 5, 8], "the caller's pixel-store state is back");
    let mut bound = 0;
    // SAFETY: a live out pointer.
    unsafe { glGetIntegerv(GL_PIXEL_UNPACK_BUFFER_BINDING, &mut bound) };
    assert_eq!(bound as u32, buf, "and the unpack buffer is still bound");
    /* a source past the buffer's end is the buffer rule's error, no texels */
    // SAFETY: an offset the buffer does not hold.
    unsafe { glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, 8, 8, 0, data.len() as GLsizei, 8 as *const c_void) };
    assert_eq!(error(), GL_INVALID_OPERATION);
    // SAFETY: restore the state; delete the names made above.
    unsafe {
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
        glPixelStorei(GL_UNPACK_SKIP_ROWS, 0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        glDeleteBuffers(1, &buf);
        glDeleteTextures(1, &name);
    }
}

/// The rules of a compressed texture hold for a decoded one.
#[test]
fn a_decoded_texture_is_a_compressed_texture_to_the_api() {
    let _g = gl();
    let (format, _, bb) = FORMATS[0];
    let name = new_tex(GL_TEXTURE_2D);
    let data = blocks(4, bb, 6);
    image(GL_TEXTURE_2D, 0, format, 8, 8, &data);
    assert_eq!(error(), GL_NO_ERROR);
    let px = [0u8; 64];
    // SAFETY: plain GL calls; the buffers are what the sizes name.
    unsafe {
        /* another compressed format */
        glCompressedTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            0,
            0,
            4,
            4,
            GL_COMPRESSED_SIGNED_R11_EAC,
            8,
            data.as_ptr() as *const c_void,
        );
        assert_eq!(error(), GL_INVALID_OPERATION, "another compressed format");
        glCompressedTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            0,
            0,
            4,
            4,
            GL_COMPRESSED_RGB8_ETC2,
            8,
            data.as_ptr() as *const c_void,
        );
        assert_eq!(error(), GL_INVALID_OPERATION, "a native compressed format");
        /* an offset inside a block, a wrong size */
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 4, 4, format, 8, data.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_OPERATION, "an offset inside a block");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, format, 16, data.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_VALUE, "imageSize");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 8, 8, format, 32, data.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_VALUE, "outside the level");
        /* an uncompressed call on a compressed texture */
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RED, GL_HALF_FLOAT, px.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_OPERATION, "glTexSubImage2D");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 2, 2);
        assert_ne!(error(), GL_NO_ERROR, "glCopyTexSubImage2D");
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(error(), GL_INVALID_OPERATION, "glGenerateMipmap");
        /* not an attachment */
        let mut fbo = 0;
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
        assert_ne!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "an attachment");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        while glGetError() != GL_NO_ERROR {}
    }
    /* the texels are as the decode left them */
    check_level(name, 0, Kind11::R11, 8, 8, &data, 0);
    /* a respecification with another format lifts the rules */
    // SAFETY: `px` holds 2 x 2 RGBA8 texels.
    unsafe {
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        assert_eq!(error(), GL_NO_ERROR);
        assert_eq!(texture_object(name).decoded_from, 0);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 2, 2, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
        assert_eq!(error(), GL_NO_ERROR, "an ordinary texture again");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, format, 8, data.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_OPERATION, "no compressed image left to replace");
        glDeleteTextures(1, &name);
    }
}

#[test]
fn the_argument_errors_are_the_compressed_formats() {
    let _g = gl();
    let (format, _, bb) = FORMATS[2];
    let data = blocks(4, bb, 2);
    let name = new_tex(GL_TEXTURE_2D);
    // SAFETY: plain GL calls; the failing ones read nothing.
    unsafe {
        glCompressedTexImage2D(
            GL_TEXTURE_2D,
            0,
            format,
            8,
            8,
            0,
            (data.len() - 1) as GLsizei,
            data.as_ptr() as *const c_void,
        );
        assert_eq!(error(), GL_INVALID_VALUE, "imageSize");
        glCompressedTexImage2D(
            GL_TEXTURE_2D,
            0,
            format,
            8,
            8,
            1,
            data.len() as GLsizei,
            data.as_ptr() as *const c_void,
        );
        assert_eq!(error(), GL_INVALID_VALUE, "border");
        glCompressedTexImage3D(GL_TEXTURE_3D, 0, format, 8, 8, 1, 0, 64, data.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_OPERATION, "a 3D target");
        glCompressedTexImage2D(GL_TEXTURE_2D_ARRAY, 0, format, 8, 8, 0, 64, data.as_ptr() as *const c_void);
        assert_eq!(error(), GL_INVALID_ENUM, "a 3D target through the 2D call");
        /* a level 0x0 is legal and holds nothing */
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, 0, 0, 0, 0, core::ptr::null());
        assert_eq!(error(), GL_NO_ERROR, "an empty image");
        glDeleteTextures(1, &name);
    }
}
