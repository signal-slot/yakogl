// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 compressed formats the GL core decodes at upload (plan #354,
//! `src/gl/etc2.rs`, `src/gl/texture.rs` `DECODED_ETC2`): R11_EAC,
//! SIGNED_R11_EAC, RG11_EAC, SIGNED_RG11_EAC and the two punch-through ETC2
//! formats are accepted by glCompressedTexImage2D / 3D and
//! glCompressedTexSubImage2D / 3D, decoded into R16F, RG16F, RGBA8 and
//! SRGB8_ALPHA8 texels the back ends sample as they sample any texture of
//! those classes, and listed by GL_COMPRESSED_TEXTURE_FORMATS beside the
//! formats the back end stores as blocks (ES 3.0 requires all ten). The
//! block decoder is checked against Mesa's own in `src/gl/etc2.rs`;
//! here the API: what lands in the storage for hand-made blocks, mip levels,
//! cube faces and array layers, sub-images, the pixel-store state, the PBO
//! path and the errors.
//!
//! The storage is read through the texture object (`GlTexture::level_off`,
//! `level_pitch_at`), so the tests hold for each GPU build.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_eac
//! ```
#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    GlesHeap, YakoglBackend, GLES_TEXFMT_R16F, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8, GLES_TEXFMT_SRGB8_A8,
};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glDeleteBuffers, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlTexture};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glGetIntegerv, glPixelStorei};
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

/* ---- reading the storage back ---------------------------------------------- */

fn texture_object(name: GLuint) -> &'static GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

fn half(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) & 1, i32::from((h >> 10) & 0x1f), f32::from(h & 0x3ff));
    let v = if e == 0 { m * 2f32.powi(-24) } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) };
    if s == 1 {
        -v
    } else {
        v
    }
}

/// The channels of texel (x, y) of `level` as the class holds them: floats for
/// the half classes, bytes for the 8-bit ones. `row` is the texel's row in the
/// level's rows (a cube face's or an array layer's rows are stacked under
/// each other).
fn texel(name: GLuint, level: usize, x: u32, row: u32) -> Vec<f32> {
    let t = texture_object(name);
    let bpp = if t.bpp != 0 { usize::from(t.bpp) } else { 4 };
    let pitch = t.level_pitch_at(level as u32) as usize;
    let at = t.level_off[level] as usize + (row as usize * pitch + x as usize) * bpp;
    // SAFETY: the texel lies inside the level the test uploaded.
    let b = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), bpp) };
    match t.texfmt {
        GLES_TEXFMT_R16F => vec![half(u16::from_le_bytes([b[0], b[1]]))],
        GLES_TEXFMT_RGBA16F => (0..4).map(|i| half(u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]))).collect(),
        GLES_TEXFMT_RGBA8 | GLES_TEXFMT_SRGB8_A8 => b.iter().map(|&v| f32::from(v)).collect(),
        other => panic!("unexpected texel class {other}"),
    }
}

/// What a class holding R16F / RG16F texels returns for the channels the
/// format has: (r) or (r, g), whatever the class's width.
fn channels(v: &[f32], n: usize) -> Vec<f32> {
    v[..n].to_vec()
}

/* ---- blocks made by hand -------------------------------------------------- */

const TABLE0: [i32; 8] = [-3, -6, -9, -15, 2, 5, 8, 14];

/// An EAC block with every pixel index `idx`.
fn eac(base: u8, mult: u8, table: u8, idx: u8) -> [u8; 8] {
    let bits = u64::from(idx) * 0x2492_4924_9249; /* sixteen 3-bit fields */
    let b = bits.to_be_bytes();
    [base, (mult << 4) | table, b[2], b[3], b[4], b[5], b[6], b[7]]
}

/// The 11-bit codeword of such a block, from the format's formula.
fn eac_value(base: i32, mult: i32, idx: usize, signed: bool) -> i32 {
    let m = if mult == 0 { TABLE0[idx] } else { TABLE0[idx] * mult * 8 };
    if signed {
        (base.max(-127) * 8 + m).clamp(-1023, 1023)
    } else {
        (base * 8 + 4 + m).clamp(0, 2047)
    }
}

/// (block, expected unsigned codeword) of the four texels of an 8x8 image.
fn r11_blocks(signed: bool) -> Vec<([u8; 8], i32)> {
    let specs: [(i32, i32, usize); 4] = if signed {
        [(0x40, 2, 1), (-128, 15, 3), (0x7f, 15, 7), (0x10, 0, 2)]
    } else {
        [(128, 1, 0), (255, 15, 7), (0, 1, 3), (100, 0, 4)]
    };
    specs
        .iter()
        .map(|&(base, mult, idx)| (eac(base as u8, mult as u8, 0, idx as u8), eac_value(base, mult, idx, signed)))
        .collect()
}

fn near(got: f32, want: f32, what: &str) {
    /* the half nearest the 11-bit value is within 2^-12 of it */
    assert!((got - want).abs() <= 1.0 / 4096.0, "{what}: {got} vs {want}");
}

/* ---- uploads -------------------------------------------------------------- */

fn new_texture(target: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
    }
    name
}

fn upload2d(target: GLenum, level: GLint, format: GLenum, w: GLsizei, h: GLsizei, data: &[u8]) -> GLenum {
    // SAFETY: `data` holds the imageSize it is given.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glCompressedTexImage2D(target, level, format, w, h, 0, data.len() as GLsizei, data.as_ptr().cast());
        glGetError()
    }
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

const ALL_SIX: [GLenum; 6] = [
    GL_COMPRESSED_R11_EAC,
    GL_COMPRESSED_SIGNED_R11_EAC,
    GL_COMPRESSED_RG11_EAC,
    GL_COMPRESSED_SIGNED_RG11_EAC,
    GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
    GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
];

fn block_bytes(format: GLenum) -> usize {
    if matches!(format, GL_COMPRESSED_RG11_EAC | GL_COMPRESSED_SIGNED_RG11_EAC) {
        16
    } else {
        8
    }
}

/* ---- the tests ------------------------------------------------------------ */

/// ES 3.0 lists the ten formats it requires, ETC1 first: what dEQP's
/// implementation_limits.{compressed,num_compressed}_texture_formats and
/// state_query.integers.compressed_texture_formats_* ask.
#[test]
fn the_ten_es3_formats_are_listed() {
    let _g = context();
    let mut n = 0;
    let mut list = [0 as GLint; 16];
    // SAFETY: room for the widest list.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR);
    let list = &list[..n as usize];
    for f in ALL_SIX
        .into_iter()
        .chain([GL_COMPRESSED_RGB8_ETC2, GL_COMPRESSED_SRGB8_ETC2, GL_COMPRESSED_RGBA8_ETC2_EAC])
        .chain([GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC])
    {
        assert!(list.contains(&(f as GLint)), "{f:#x} missing from {list:x?}");
    }
    assert_eq!(list[0], GL_ETC1_RGB8_OES as GLint);
    assert_eq!(list.len(), 11);
}

/// R11 EAC and SIGNED_R11_EAC: the four blocks of an 8x8 image, every texel
/// of a block the 11-bit value the format's formula gives, as a half.
#[test]
fn r11_eac_decodes_into_halves() {
    let _g = context();
    for (format, signed) in [(GL_COMPRESSED_R11_EAC, false), (GL_COMPRESSED_SIGNED_R11_EAC, true)] {
        let name = new_texture(GL_TEXTURE_2D);
        let blocks = r11_blocks(signed);
        let data: Vec<u8> = blocks.iter().flat_map(|(b, _)| *b).collect();
        assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &data), GL_NO_ERROR, "{format:#x}");
        let div = if signed { 1023.0 } else { 2047.0 };
        for (i, (_, v)) in blocks.iter().enumerate() {
            let (bx, by) = ((i % 2) as u32 * 4, (i / 2) as u32 * 4);
            for (x, y) in [(0, 0), (3, 0), (1, 2), (3, 3)] {
                let got = channels(&texel(name, 0, bx + x, by + y), 1);
                near(got[0], *v as f32 / div, &format!("{format:#x} block {i} texel ({x}, {y})"));
            }
        }
    }
}

/// RG11 EAC: red from the first 8 bytes of a block, green from the next 8.
#[test]
fn rg11_eac_decodes_two_planes() {
    let _g = context();
    for (format, signed) in [(GL_COMPRESSED_RG11_EAC, false), (GL_COMPRESSED_SIGNED_RG11_EAC, true)] {
        let name = new_texture(GL_TEXTURE_2D);
        let blocks = r11_blocks(signed);
        /* red: block i; green: block 3 - i */
        let data: Vec<u8> = (0..4).flat_map(|i| [blocks[i].0, blocks[3 - i].0].concat()).collect();
        assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &data), GL_NO_ERROR, "{format:#x}");
        let div = if signed { 1023.0 } else { 2047.0 };
        for i in 0..4 {
            let (bx, by) = ((i % 2) as u32 * 4, (i / 2) as u32 * 4);
            let got = channels(&texel(name, 0, bx + 2, by + 1), 2);
            near(got[0], blocks[i].1 as f32 / div, &format!("{format:#x} red {i}"));
            near(got[1], blocks[3 - i].1 as f32 / div, &format!("{format:#x} green {i}"));
        }
    }
}

/// The punch-through formats: an opaque differential block decodes as ETC2
/// RGB with alpha 255; a block with the opaque bit clear makes pixel index 2
/// (0, 0, 0, 0). The sRGB format holds the same bytes (in the sRGB class).
#[test]
fn punchthrough_decodes_into_rgba8() {
    let _g = context();
    /* differential mode, R = G = B = 16 (0x84 = 132 once extended), table 0
     * (+2 -> 134 at index 0), opaque bit set; then the same with the bit
     * clear (modifier 0 at index 0: 132) and index 2 everywhere (transparent) */
    let opaque = [0x80, 0x80, 0x80, 0x02, 0, 0, 0, 0];
    let clear_idx0 = [0x80, 0x80, 0x80, 0x00, 0, 0, 0, 0];
    let clear_idx2 = [0x80, 0x80, 0x80, 0x00, 0xff, 0xff, 0, 0];
    let mut data = Vec::new();
    for b in [opaque, clear_idx0, clear_idx2, opaque] {
        data.extend(b);
    }
    for format in [GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2] {
        let name = new_texture(GL_TEXTURE_2D);
        assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &data), GL_NO_ERROR, "{format:#x}");
        let t = texture_object(name);
        let want_class = if format == GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2 {
            GLES_TEXFMT_SRGB8_A8
        } else {
            GLES_TEXFMT_RGBA8
        };
        assert_eq!(t.texfmt, want_class, "{format:#x}: the class");
        assert_eq!(texel(name, 0, 1, 1), [134.0, 134.0, 134.0, 255.0], "{format:#x}: opaque block");
        assert_eq!(texel(name, 0, 5, 2), [132.0, 132.0, 132.0, 255.0], "{format:#x}: clear bit, index 0");
        assert_eq!(texel(name, 0, 2, 6), [0.0, 0.0, 0.0, 0.0], "{format:#x}: clear bit, index 2");
        assert_eq!(texel(name, 0, 7, 7), [134.0, 134.0, 134.0, 255.0], "{format:#x}: the last block");
    }
}

/// The imageSize rule of every name, in ES 3.0 and as before; a partial block
/// at the edge is a whole block.
#[test]
fn image_size_and_shape_errors_are_the_formats_rules() {
    let _g = context();
    for format in ALL_SIX {
        let bb = block_bytes(format);
        new_texture(GL_TEXTURE_2D);
        let _ = upload2d(GL_TEXTURE_2D, 0, format, 5, 3, &vec![0; bb * 2]);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(
            upload2d(GL_TEXTURE_2D, 0, format, 5, 3, &vec![0; bb]),
            GL_INVALID_VALUE,
            "{format:#x}: 5x3 is 2 blocks"
        );
        assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &vec![0; bb * 4]), GL_NO_ERROR, "{format:#x}: 8x8");
        /* a level that is not the chain's size is recorded, not an error */
        assert_eq!(upload2d(GL_TEXTURE_2D, 1, format, 4, 4, &vec![0; bb]), GL_NO_ERROR, "{format:#x}: level 1");
        assert_eq!(upload2d(GL_TEXTURE_2D, 3, format, 1, 1, &vec![0; bb]), GL_NO_ERROR, "{format:#x}: level 3");
        // SAFETY: the failing calls read nothing.
        unsafe {
            glCompressedTexImage2D(GL_TEXTURE_3D, 0, format, 4, 4, 0, bb as GLsizei, core::ptr::null());
            assert_eq!(err(), GL_INVALID_ENUM, "{format:#x}: TEXTURE_3D through the 2D entry point");
            glCompressedTexImage3D(GL_TEXTURE_3D, 0, format, 4, 4, 1, 0, bb as GLsizei, core::ptr::null());
            assert_eq!(err(), GL_INVALID_OPERATION, "{format:#x}: no 3D form");
        }
    }
}

/// A full chain, 16x16 down to 1x1, with cube faces and a 2D array, each
/// level's blocks distinct: every texel lands in its level, face and layer.
#[test]
fn mip_levels_cube_faces_and_array_layers_land_where_they_belong() {
    let _g = context();
    let format = GL_COMPRESSED_R11_EAC;
    /* texel value of (level, layer): block base codeword l * 16 + layer */
    let block_of = |level: u32, layer: u32| eac((level * 16 + layer + 1) as u8, 1, 0, 3);
    let value_of = |level: u32, layer: u32| eac_value((level * 16 + layer + 1) as i32, 1, 3, false) as f32 / 2047.0;

    /* a 2D chain */
    let name = new_texture(GL_TEXTURE_2D);
    for level in 0..5u32 {
        let side: u32 = 16 >> level;
        let blocks = side.div_ceil(4).pow(2);
        let data: Vec<u8> = (0..blocks).flat_map(|_| block_of(level, 0)).collect();
        assert_eq!(
            upload2d(GL_TEXTURE_2D, level as GLint, format, side as GLsizei, side as GLsizei, &data),
            GL_NO_ERROR
        );
    }
    for level in 0..5u32 {
        let side = 16 >> level;
        near(texel(name, level as usize, side - 1, side - 1)[0], value_of(level, 0), &format!("2D level {level}"));
    }

    /* a cube: six faces of 8x8 and 4x4 */
    let name = new_texture(GL_TEXTURE_CUBE_MAP);
    for face in 0..6u32 {
        for (level, side) in [(0u32, 8u32), (1, 4)] {
            let data: Vec<u8> = (0..side.div_ceil(4).pow(2)).flat_map(|_| block_of(level, face)).collect();
            let target = GL_TEXTURE_CUBE_MAP_POSITIVE_X + face;
            assert_eq!(upload2d(target, level as GLint, format, side as GLsizei, side as GLsizei, &data), GL_NO_ERROR);
        }
    }
    for face in 0..6u32 {
        near(texel(name, 0, 3, face * 8 + 7)[0], value_of(0, face), &format!("face {face} level 0"));
        near(texel(name, 1, 0, face * 4)[0], value_of(1, face), &format!("face {face} level 1"));
    }

    /* an array of three 8x8 layers, two levels */
    let name = new_texture(GL_TEXTURE_2D_ARRAY);
    for (level, side) in [(0u32, 8u32), (1, 4)] {
        let blocks = side.div_ceil(4).pow(2);
        let data: Vec<u8> = (0..3).flat_map(|z| (0..blocks).flat_map(move |_| block_of(level, z))).collect();
        // SAFETY: `data` holds the 3 images imageSize names.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                level as GLint,
                format,
                side as GLsizei,
                side as GLsizei,
                3,
                0,
                data.len() as GLsizei,
                data.as_ptr().cast(),
            )
        };
        assert_eq!(err(), GL_NO_ERROR, "array level {level}");
    }
    let t = texture_object(name);
    assert_eq!(t.layers, 3);
    for z in 0..3u32 {
        near(texel(name, 0, 5, z * 8 + 6)[0], value_of(0, z), &format!("layer {z} level 0"));
        near(texel(name, 1, 2, z * 4 + 1)[0], value_of(1, z), &format!("layer {z} level 1"));
    }
}

/// glCompressedTexSubImage2D of the same format replaces a block; another
/// format, an offset inside a block and an uncompressed glTexSubImage2D are
/// errors; the level keeps what it had.
#[test]
fn sub_images_write_whole_blocks_of_the_same_format_only() {
    let _g = context();
    let format = GL_COMPRESSED_RG11_EAC;
    let name = new_texture(GL_TEXTURE_2D);
    let (a, b) = (r11_blocks(false), r11_blocks(false));
    let data: Vec<u8> = (0..4).flat_map(|i| [a[i].0, b[i].0].concat()).collect();
    assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &data), GL_NO_ERROR);
    let before = texel(name, 0, 5, 5);
    /* block (1, 1) <- red from block 2, green from block 1 */
    let sub = [a[2].0, a[1].0].concat();
    // SAFETY: `sub` is the one block imageSize names.
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 4, 4, format, 16, sub.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR);
    }
    let after = texel(name, 0, 5, 5);
    assert_ne!(before, after);
    near(after[0], a[2].1 as f32 / 2047.0, "red of the new block");
    near(after[1], a[1].1 as f32 / 2047.0, "green of the new block");
    near(texel(name, 0, 1, 1)[0], a[0].1 as f32 / 2047.0, "block (0, 0) kept");
    // SAFETY: the failing calls read at most one block.
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_COMPRESSED_SIGNED_RG11_EAC, 16, sub.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_OPERATION, "another format");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 1, 0, 4, 4, format, 16, sub.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_OPERATION, "an offset inside a block");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 8, 4, format, 32, sub.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_VALUE, "past the level's edge");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, format, 15, sub.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_VALUE, "imageSize");
        let texels = [0u16; 4];
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RG, GL_HALF_FLOAT, texels.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_OPERATION, "uncompressed data into a compressed level");
    }
    near(texel(name, 0, 1, 1)[0], a[0].1 as f32 / 2047.0, "nothing written by the failures");
}

/// The pixel-store state belongs to glTexImage2D and its kin: the decode
/// ignores it for its own rows (a row length or alignment set by the
/// application does not shear the decoded image) and leaves it as it was.
#[test]
fn the_unpack_state_neither_shears_the_decode_nor_changes() {
    let _g = context();
    let format = GL_COMPRESSED_R11_EAC;
    let name = new_texture(GL_TEXTURE_2D);
    let blocks = r11_blocks(false);
    let data: Vec<u8> = blocks.iter().flat_map(|(b, _)| *b).collect();
    // SAFETY: plain state calls.
    unsafe {
        glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 100);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 3);
    }
    assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &data), GL_NO_ERROR);
    let (mut align, mut row, mut skip) = (0, 0, 0);
    // SAFETY: room for one value each.
    unsafe {
        glGetIntegerv(GL_UNPACK_ALIGNMENT, &mut align);
        glGetIntegerv(GL_UNPACK_ROW_LENGTH, &mut row);
        glGetIntegerv(GL_UNPACK_SKIP_PIXELS, &mut skip);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
        glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
        glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
    }
    assert_eq!((align, row, skip), (8, 100, 3), "the state is as it was");
    for (i, (_, v)) in blocks.iter().enumerate() {
        let (bx, by) = ((i % 2) as u32 * 4, (i / 2) as u32 * 4);
        near(texel(name, 0, bx + 3, by + 3)[0], *v as f32 / 2047.0, &format!("block {i}"));
    }
}

/// With PIXEL_UNPACK_BUFFER bound the data is an offset into the buffer
/// (ES 3.0 3.7.1), for the image and for a sub-image; an offset that runs
/// past the buffer is GL_INVALID_OPERATION.
#[test]
fn the_pbo_path_reads_the_buffer() {
    let _g = context();
    let format = GL_COMPRESSED_RG11_EAC;
    let blocks = r11_blocks(false);
    let image: Vec<u8> = (0..4).flat_map(|i| [blocks[i].0, blocks[3 - i].0].concat()).collect();
    let mut buf = 0;
    // SAFETY: the buffer is given the bytes it is told of.
    unsafe {
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
        let mut all = vec![0u8; 16];
        all.extend(&image);
        glBufferData(GL_PIXEL_UNPACK_BUFFER, all.len() as isize, all.as_ptr().cast(), GL_STATIC_DRAW);
        assert_eq!(err(), GL_NO_ERROR);
        let name = new_texture(GL_TEXTURE_2D);
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, 8, 8, 0, 64, 16 as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "an offset of 16 into the buffer");
        near(texel(name, 0, 2, 1)[0], blocks[0].1 as f32 / 2047.0, "red of block 0");
        near(texel(name, 0, 6, 6)[1], blocks[0].1 as f32 / 2047.0, "green of block 3");
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, format, 16, 32 as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "a sub-image from offset 32");
        near(texel(name, 0, 0, 0)[0], blocks[1].1 as f32 / 2047.0, "red of the replaced block (block 1 of the image)");
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, 8, 8, 0, 64, 40 as *const c_void);
        assert_eq!(err(), GL_INVALID_OPERATION, "64 bytes from offset 40 of an 80-byte buffer");
        glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
        glDeleteBuffers(1, &buf);
    }
}

/// An immutable texture takes no new image: the call is GL_INVALID_OPERATION
/// and the six names are no glTexStorage2D formats here (they are
/// compressed formats, which this GL stores through glCompressedTexImage*).
#[test]
fn an_immutable_texture_refuses_a_new_image() {
    let _g = context();
    let name = new_texture(GL_TEXTURE_2D);
    // SAFETY: plain GL calls; the data is the one block imageSize names.
    unsafe {
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, 8, 8);
        assert_eq!(err(), GL_NO_ERROR);
        for format in ALL_SIX {
            let block = vec![0u8; block_bytes(format)];
            glCompressedTexImage2D(GL_TEXTURE_2D, 0, format, 4, 4, 0, block.len() as GLsizei, block.as_ptr().cast());
            assert_eq!(err(), GL_INVALID_OPERATION, "{format:#x} onto glTexStorage2D's texture");
        }
    }
    let _ = name;
}

/// What a decoded image is not: no generated chain, no copy destination, and
/// a respecification with another format leaves nothing of it (the format's
/// sub-image is then GL_INVALID_OPERATION).
#[test]
fn a_decoded_texture_is_compressed_to_the_application() {
    let _g = context();
    let format = GL_COMPRESSED_R11_EAC;
    let name = new_texture(GL_TEXTURE_2D);
    let blocks = r11_blocks(false);
    let data: Vec<u8> = blocks.iter().flat_map(|(b, _)| *b).collect();
    assert_eq!(upload2d(GL_TEXTURE_2D, 0, format, 8, 8, &data), GL_NO_ERROR);
    // SAFETY: the failing calls read nothing; the last texels are 8x8 RGBA8.
    unsafe {
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(err(), GL_INVALID_OPERATION, "glGenerateMipmap of a compressed level");
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 1, 1);
        assert_eq!(err(), GL_INVALID_OPERATION, "glCopyTexSubImage2D into a compressed level");
        let rgba = [0u8; 8 * 8 * 4];
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8 as GLint, 8, 8, 0, GL_RGBA, GL_UNSIGNED_BYTE, rgba.as_ptr().cast());
        assert_eq!(err(), GL_NO_ERROR);
        let block = blocks[0].0;
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, format, 8, block.as_ptr().cast());
        assert_eq!(err(), GL_INVALID_OPERATION, "the level is RGBA8 now");
    }
    assert_eq!(texture_object(name).etc2_decoded, 0);
}
