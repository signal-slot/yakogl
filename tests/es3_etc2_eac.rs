// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0's ten ETC2/EAC formats through the GL API on the host, per GPU
//! build (plan #354; design docs/notes/etc2-eac.md).
//!
//! - GL_NUM_COMPRESSED_TEXTURE_FORMATS / GL_COMPRESSED_TEXTURE_FORMATS: in
//!   an ES 3.0 report on the PowerVR, the GC7000UL and the V3D, ETC1 and the
//!   ten (11); under `report-es2` ETC1, RGB8_ETC2 and RGBA8_ETC2_EAC (3),
//!   unchanged (dEQP-GLES3 implementation_limits.{num_,}compressed_texture_formats
//!   and state_query.integers.compressed_texture_formats_*).
//! - The six without a block class (R11/RG11 EAC, signed or not, and the two
//!   punch-through ETC2 formats) are taken by glCompressedTexImage2D/3D and
//!   decoded at upload into a class the back end samples
//!   (src/gl/texture.rs `DECODED_FORMATS`): R16F, RGBA16F, RGBA8 and
//!   SRGB8_A8. Every stored texel is checked against the decoders of
//!   src/gl/etc2.rs (which are checked against Mesa's there), converted as
//!   the design says (an 11-bit value's 16-bit widening over 65535 or 32767,
//!   rounded to the nearest half).
//! - A decoded texture is a compressed one: glCompressedTexSubImage2D of its
//!   format decodes into it, glTexSubImage2D is GL_INVALID_OPERATION.
//!
//! What a GPU samples from these classes is the back ends' (R16F, RGBA16F,
//! RGBA8 and SRGB8_A8 are sampled by all three today); no board ran this.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_eac
//! ```

#![cfg(feature = "host-test")]
// the ES 2.0 report runs one test of the file; the helpers of the rest are idle there
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::es3_formats::f32_to_half;
use yakogl::gl::etc2;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::texture::{
    glBindTexture, glCompressedTexImage2D, glCompressedTexSubImage2D, glGenTextures, glTexSubImage2D,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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

/* Both tables are the fields this file needs over all-zero ones (every field
 * an Option, a bool, a number or a pointer): a field another change adds to
 * either table cannot break this file's build (CLAUDE.md, the #787 lesson). */

// SAFETY (both): all-zero is a valid OsalOps / YakoglBackend.
static OSAL: OsalOps = OsalOps {
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    ..unsafe { core::mem::zeroed() }
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-etc2-eac".as_ptr() as *const c_char,
    init: Some(b_init),
    heap_base: Some(b_heap_base),
    max_instance_divisor: 1,
    ..unsafe { core::mem::zeroed() }
};

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> std::sync::MutexGuard<'static, ()> {
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

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// Does this build store the six (an ES 3.0 report on a back end that
/// samples R16F, RGBA16F, RGBA8 and SRGB8_A8: all but the Mali's)?
const DECODES: bool = !cfg!(feature = "report-es2") && !cfg!(feature = "gpu-mali");

const TEN: [GLenum; 10] = [
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

#[test]
fn the_list_is_etc1_and_the_ten_under_es3_and_three_under_es2() {
    let _g = context();
    let mut n = 0;
    let mut list = [0 as GLint; 32];
    // SAFETY: room for one value and for far more than the list.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, list.as_mut_ptr());
    }
    assert_eq!(error(), GL_NO_ERROR);
    let list: Vec<GLenum> = list[..n as usize].iter().map(|&f| f as GLenum).collect();
    if DECODES {
        assert_eq!(n, 11, "{list:x?}");
        assert!(list.contains(&GL_ETC1_RGB8_OES));
        for f in TEN {
            assert!(list.contains(&f), "0x{f:x} in {list:x?}");
        }
    } else {
        assert_eq!(list, [GL_ETC1_RGB8_OES, GL_COMPRESSED_RGB8_ETC2, GL_COMPRESSED_RGBA8_ETC2_EAC]);
    }
}

/* ---- the six decoded at upload -------------------------------------------- */

/// Distinct 8-byte blocks: EAC bases, multipliers and tables, ETC2 modes
/// with the opaque bit set and clear.
const BLOCKS: [[u8; 8]; 4] = [
    [0x80, 0x5a, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc],
    [0x00, 0xf3, 0xff, 0x00, 0xaa, 0x55, 0x0f, 0xf0],
    [0xff, 0x1d, 0x24, 0x92, 0x49, 0x24, 0x92, 0x49],
    [0x41, 0x7a, 0xe2, 0x02, 0x11, 0x22, 0x44, 0x88],
];

/// (format, bytes a block, bytes a stored texel)
const SIX: [(GLenum, usize, usize); 6] = [
    (GL_COMPRESSED_R11_EAC, 8, 2),
    (GL_COMPRESSED_SIGNED_R11_EAC, 8, 2),
    (GL_COMPRESSED_RG11_EAC, 16, 8),
    (GL_COMPRESSED_SIGNED_RG11_EAC, 16, 8),
    (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8, 4),
    (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8, 4),
];

/// `n` blocks of `bb` bytes from [`BLOCKS`].
fn blocks(n: usize, bb: usize) -> Vec<u8> {
    (0..n * bb / 8).flat_map(|i| BLOCKS[i % 4]).collect()
}

/// Texel `i` of `block` of `f` as its class stores it.
fn want(f: GLenum, block: &[u8], i: usize) -> Vec<u8> {
    let half = |v: f32| f32_to_half(v).to_ne_bytes().to_vec();
    let eac = |b: &[u8], signed: bool| {
        let k: [u8; 8] = b[..8].try_into().unwrap();
        if signed {
            (f32::from(etc2::decode_eac11_signed_block(&k)[i]) / 32767.0).max(-1.0)
        } else {
            f32::from(etc2::decode_eac11_unsigned_block(&k)[i]) / 65535.0
        }
    };
    match f {
        GL_COMPRESSED_R11_EAC => half(eac(block, false)),
        GL_COMPRESSED_SIGNED_R11_EAC => half(eac(block, true)),
        GL_COMPRESSED_RG11_EAC | GL_COMPRESSED_SIGNED_RG11_EAC => {
            let s = f == GL_COMPRESSED_SIGNED_RG11_EAC;
            [half(eac(&block[..8], s)), half(eac(&block[8..], s)), half(0.0), half(1.0)].concat()
        }
        _ => etc2::decode_rgb_block(&block[..8].try_into().unwrap(), true)[i].to_vec(),
    }
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// Every texel of level 0 (w x h) of 2D texture `name` against the decode
/// of its block in `data`.
fn assert_level0(name: GLuint, f: GLenum, bb: usize, bpp: usize, w: u32, h: u32, data: &[u8]) {
    let t = texture_object(name);
    let pitch = t.level_pitch(w) as usize;
    let base = t.mem.cpu as *const u8;
    let bw = w.div_ceil(4) as usize;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let block = &data[((y / 4) * bw + x / 4) * bb..][..bb];
            let o = t.level_off[0] as usize + (y * pitch + x) * bpp;
            // SAFETY: texel (x, y) of level 0 lies inside the texture's storage.
            let got = unsafe { core::slice::from_raw_parts(base.add(o), bpp) };
            assert_eq!(got, want(f, block, (y % 4) * 4 + x % 4), "0x{f:x} texel ({x}, {y})");
        }
    }
}

fn upload(f: GLenum, w: GLsizei, h: GLsizei, data: &[u8]) -> GLuint {
    let mut name = 0;
    // SAFETY: plain GL calls; `data` holds imageSize bytes.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, w, h, 0, data.len() as GLsizei, data.as_ptr() as *const c_void);
    }
    name
}

#[test]
#[cfg(not(feature = "report-es2"))]
fn the_six_are_decoded_into_the_class_each_samples_as() {
    let _g = context();
    error();
    for (f, bb, bpp) in SIX {
        /* 7x6: 2x2 blocks, the right and bottom ones cut */
        let data = blocks(4, bb);
        let name = upload(f, 7, 6, &data);
        if !DECODES {
            assert_eq!(error(), GL_INVALID_ENUM, "0x{f:x}");
            continue;
        }
        assert_eq!(error(), GL_NO_ERROR, "0x{f:x}");
        let t = texture_object(name);
        assert_eq!((t.block_bytes, t.width, t.height), (0, 7, 6), "0x{f:x}: texels, not blocks");
        assert_ne!(t.decoded_from, 0, "0x{f:x}: the texture remembers its format");
        assert_eq!(yakogl::backend::gles_texfmt_bytes(t.texfmt) as usize, bpp, "0x{f:x}");
        assert_level0(name, f, bb, bpp, 7, 6, &data);
    }
}

#[test]
#[cfg(not(feature = "report-es2"))]
fn a_decoded_texture_takes_compressed_sub_images_of_its_format_only() {
    let _g = context();
    if !DECODES {
        return;
    }
    error();
    let (f, bb, bpp) = SIX[3]; /* SIGNED_RG11 */
    let mut data = blocks(4, bb);
    let name = upload(f, 8, 8, &data);
    assert_eq!(error(), GL_NO_ERROR);
    /* block (1, 0) gets its own bytes inverted: another block */
    let b: Vec<u8> = data[bb..2 * bb].iter().map(|x| !x).collect();
    let halves = [0u8; 4 * 4 * 8];
    // SAFETY: one block, and a 4x4 RGBA half-float image.
    unsafe {
        glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 0, 4, 4, f, bb as GLsizei, b.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "a sub-image of its format");
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, GL_RGBA, GL_HALF_FLOAT, halves.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "glTexSubImage2D of a compressed texture");
    }
    data[bb..2 * bb].copy_from_slice(&b);
    assert_level0(name, f, bb, bpp, 8, 8, &data);
}

#[test]
#[cfg(feature = "report-es2")]
fn an_es2_report_refuses_the_six() {
    let _g = context();
    error();
    for (f, bb, _) in SIX {
        upload(f, 4, 4, &blocks(1, bb));
        assert_eq!(error(), GL_INVALID_ENUM, "0x{f:x}");
    }
}
