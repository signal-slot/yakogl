// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ETC2 2D arrays on the PowerVR's native array path (`gl::native_array`,
//! docs/es3/pvr-2darray.md section 6): a compressed array is copyable, its
//! copy is layer-major with every layer on a 4 KiB boundary and the levels
//! inside a layer at the TPU's block-grid offsets (Mesa
//! `pvr_image_plane_setup_mip_levels` over the power-of-two extent, in 4x4
//! blocks; `pvr_image.c:173-196`), the blocks of a (layer, level) twiddled;
//! the back end describes the copy as a compressed TEXTYPE_2D with the layer
//! count and the chain; and a `sampler2DArray` program compiles to the
//! native form whatever the texture's format.
//!
//! The GL half uploads through the API, distinct blocks to every (layer,
//! level), and reads the copy `native_array::copy` made, on a stand-in back
//! end whose OSAL maps a GPU address to host memory (tests/es3_tex_levels.rs'
//! set-up); the copy's bytes are compared with offsets computed here from
//! Mesa's rules, independently of `native_array.rs`. No GPU runs.
//!
//! The GL core stores ETC2 RGB8, RGBA8 (ETC2_EAC) and their sRGB forms as
//! blocks. The EAC R11/RG11 and the punch-through formats it decodes at
//! upload into an uncompressed array (tests/es3_etc2_eac.rs): there are no
//! blocks of those to copy.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_etc_array
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA,
    GLES_TEXTYPE_2D_ARRAY_TWIDDLED, GLES_WRAP_REPEAT,
};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::native_array::{self, LAYER_ALIGN};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glCompressedTexImage3D, glDeleteTextures, glGenTextures};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::powervr::chain::compile_fragment;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

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

/// The PowerVR's shape: no native layers (the GL core stacks a compressed
/// array's layers within each level), twiddled block grids.
static PVR_LIKE: YakoglBackend = YakoglBackend {
    name: c"es3-pvr-etc-array".as_ptr() as *const c_char,
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
/// in blocks, halved per level, at least one (`pvr_image.c:83-92` and the
/// level setup over the physical extent).
fn grid(w: u32, h: u32, l: u32) -> (u32, u32) {
    let g = |s: u32| (s.div_ceil(4).next_power_of_two() >> l).max(1);
    (g(w), g(h))
}

/// The levels of the TPU's chain: down to 1x1 of the power-of-two extent.
fn tpu_levels(w: u32, h: u32) -> u32 {
    w.next_power_of_two().max(h.next_power_of_two()).ilog2() + 1
}

/// (the byte offset of each level inside a layer, the layer's bytes, 4 KiB
/// aligned: `pvr_image.c:195-196`).
fn layer_layout(w: u32, h: u32, bb: u32) -> (Vec<u32>, u32) {
    let (mut at, mut off) = (0, Vec::new());
    for l in 0..tpu_levels(w, h) {
        off.push(at);
        let (gw, gh) = grid(w, h, l);
        at += gw * gh * bb;
    }
    (off, at.next_multiple_of(4096))
}

/* ---- the upload -------------------------------------------------------- */

/// Block `(bx, by)` of level `l`, layer `k`: a tag no other block shares.
fn block(bb: usize, k: u32, l: u32, bx: u32, by: u32) -> Vec<u8> {
    (0..bb).map(|i| (k * 64 + l * 8 + bx * 3 + by * 5 + i as u32 * 17 + 1) as u8 ^ (i as u8)).collect()
}

/// An array of `w x h`, `layers` layers and the GL core's whole chain, each
/// level through glCompressedTexImage3D; returns the texture name.
fn upload(internalformat: GLenum, bb: usize, w: u32, h: u32, layers: u32) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D_ARRAY, name);
    }
    let levels = w.max(h).ilog2() + 1;
    for l in 0..levels {
        let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
        let (bw, bh) = (lw.div_ceil(4), lh.div_ceil(4));
        let mut data = Vec::new();
        for k in 0..layers {
            for by in 0..bh {
                for bx in 0..bw {
                    data.extend(block(bb, k, l, bx, by));
                }
            }
        }
        // SAFETY: `data` holds bw x bh x layers blocks, imageSize bytes.
        unsafe {
            glCompressedTexImage3D(
                GL_TEXTURE_2D_ARRAY,
                l as GLint,
                internalformat,
                lw as GLsizei,
                lh as GLsizei,
                layers as GLsizei,
                0,
                data.len() as GLsizei,
                data.as_ptr() as *const c_void,
            )
        };
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "level {l} {lw}x{lh}x{layers}");
    }
    name
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// `native_array::copy` of the array at the full chain (a mipmapped draw),
/// compared block by block with Mesa's layout.
fn check_copy(internalformat: GLenum, bb: usize, w: u32, h: u32, layers: u32) {
    let name = upload(internalformat, bb, w, h, layers);
    let t = texture_object(name);
    assert_eq!(usize::from(t.block_bytes), bb, "the GL core holds the blocks");
    assert_eq!((t.grid_cols, u32::from(t.layers)), (0, layers), "layers stacked, not a 3D grid");
    let (off, layer) = layer_layout(w, h, bb as u32);
    assert_eq!(layer % LAYER_ALIGN, 0);
    let (gpu, bytes) =
        native_array::copy(t, 0, false).unwrap_or_else(|| panic!("{internalformat:#x} {w}x{h}x{layers}: copyable"));
    assert_eq!(bytes, layer * layers, "one 4 KiB-aligned layer per layer");
    // SAFETY: the mock OSAL maps a GPU address to its host memory, `bytes` long.
    let s = unsafe { core::slice::from_raw_parts(gpu as *const u8, bytes as usize) };
    for k in 0..layers {
        for l in 0..(w.max(h).ilog2() + 1) {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            let (gw, gh) = grid(w, h, l);
            for by in 0..lh.div_ceil(4) {
                for bx in 0..lw.div_ceil(4) {
                    let d = (k * layer + off[l as usize] + twiddle(bx, by, gw, gh) * bb as u32) as usize;
                    assert_eq!(
                        &s[d..d + bb],
                        &block(bb, k, l, bx, by)[..],
                        "{internalformat:#x} {w}x{h}: layer {k} level {l} block ({bx}, {by}) at {d:#x}"
                    );
                }
            }
        }
    }
    /* the copy is kept for the same texels */
    let n = native_array::copies();
    assert_eq!(native_array::copy(t, 0, false).map(|c| c.0), Some(gpu));
    assert_eq!(native_array::copies(), n, "not made again");
    /* a draw sampling one level: level 0 alone, the layer still aligned */
    let (_, one) = native_array::copy(t, 0, true).expect("a one-level copy");
    assert_eq!(one, (grid(w, h, 0).0 * grid(w, h, 0).1 * bb as u32).next_multiple_of(LAYER_ALIGN) * layers);
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &name) };
}

#[test]
fn layer_offsets_by_hand_are_what_the_tpu_walks() {
    /* the numbers the helpers above must give, spelled out: 16x8 is a 4x2
     * block grid, levels of 8, 2, 1, 1, 1 blocks of 8 bytes */
    let (off, layer) = layer_layout(16, 8, 8);
    assert_eq!(off, [0, 64, 80, 88, 96]);
    assert_eq!(layer, 4096);
    /* 40x24 (NPOT): 10x6 blocks live in a 16x8 grid, and the TPU's chain
     * runs 16x8, 8x4, 4x2, 2x1, 1x1, 1x1, 1x1 blocks (seven levels,
     * from the 64-texel extent; GL's chain of 40 has six): 173 blocks of 16
     * bytes, 2768 bytes, one 4 KiB layer */
    assert_eq!(grid(40, 24, 0), (16, 8));
    let (off, layer) = layer_layout(40, 24, 16);
    assert_eq!(off, [0, 2048, 2560, 2688, 2720, 2736, 2752]);
    assert_eq!(layer, 4096);
}

#[test]
fn an_etc2_rgb8_array_with_a_chain_is_copied_layer_major() {
    let _g = context();
    check_copy(GL_COMPRESSED_RGB8_ETC2, 8, 16, 8, 3);
    check_copy(GL_COMPRESSED_RGB8_ETC2, 8, 64, 64, 3);
    /* a NPOT layer sits in its power-of-two grid */
    check_copy(GL_COMPRESSED_RGB8_ETC2, 8, 20, 12, 3);
}

#[test]
fn an_etc2_eac_rgba8_array_with_a_chain_is_copied_layer_major() {
    let _g = context();
    check_copy(GL_COMPRESSED_RGBA8_ETC2_EAC, 16, 16, 8, 3);
    check_copy(GL_COMPRESSED_RGBA8_ETC2_EAC, 16, 32, 32, 3);
}

#[test]
fn a_one_layer_array_is_copyable_too() {
    let _g = context();
    check_copy(GL_COMPRESSED_RGB8_ETC2, 8, 8, 8, 1);
}

/* ---- the descriptor ---------------------------------------------------- */

fn desc(t: &GlesTextureBind) -> [u64; 2] {
    let w = yakogl::pvr::backend::texture_descriptor(t);
    [w[0] as u64 | (w[1] as u64) << 32, w[2] as u64 | (w[3] as u64) << 32]
}

/// The copy of a compressed array is a TEXTYPE_2D in the ETC format's word
/// 0, one layer's size, the layer count in the depth field, the chain's
/// levels, the TPU decoding the blocks (compression mode 1, no alpha_msb),
/// and one layer's bytes in meta dword 4.
#[test]
fn the_descriptor_of_an_etc_array_copy_is_a_compressed_twiddled_2d() {
    let base = 0x80_1234_5000u64;
    for (format, rgba) in [(GLES_TEXFMT_ETC2_RGB, false), (GLES_TEXFMT_ETC2_RGBA, true)] {
        let t = GlesTextureBind {
            format,
            type_: GLES_TEXTYPE_2D_ARRAY_TWIDDLED,
            layers: 3,
            base_gpu: base,
            width: 16,
            height: 8,
            stride_texels: 16,
            bytes: 3 * 4096,
            mip_filter: 2,
            mip_levels: 5,
            linear_filter: 1,
            min_linear: 1,
            wrap_s: GLES_WRAP_REPEAT,
            wrap_t: GLES_WRAP_REPEAT,
            ..GlesTextureBind::ZERO
        };
        let w = yakogl::pvr::backend::texture_descriptor(&t);
        let [iw0, iw1] = desc(&t);
        assert_eq!(iw0 & 7, 1, "TEXTYPE_2D");
        assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (15, 7), "one layer's size");
        assert_eq!(iw1 & 0xf, 5, "16x8: 16x8, 8x4, 4x2, 2x1, 1x1");
        assert_eq!((iw1 >> 4) & 0x7ff, 2, "depth = layers - 1");
        assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
        assert_eq!((iw1 >> 54) & 7, 1, "compression mode TPU");
        assert_eq!((iw1 >> 59) & 1, 0, "no alpha_msb");
        assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, base >> 2, "texaddr");
        assert_eq!(iw1 >> 60, 0, "baselevel 0");
        assert_eq!(w[4], 4096, "one layer's bytes");
        assert_eq!((w[5], w[6]), (0x1234_5000, 0x80), "the base, low dword first");
        /* the format word differs by the ETC class, and is the one a 2D ETC
         * texture of the same class gets */
        let flat = desc(&GlesTextureBind { type_: backend::GLES_TEXTYPE_2D, layers: 0, ..t });
        assert_eq!(iw0 & !7 & 0xffff_ffff, flat[0] & !7 & 0xffff_ffff, "rgba={rgba}: the 2D texture's format bits");
        /* one level sampled: a one-level image */
        let [_, one] = desc(&GlesTextureBind { mip_filter: 0, ..t });
        assert_eq!((one & 0xf, (one >> 15) & 1), (1, 0));
    }
}

/* ---- the program ------------------------------------------------------- */

static COMPILING: Mutex<()> = Mutex::new(());

/// A `sampler2DArray` program is native whatever the texture bound to it
/// will be: the record says so (bit 1 of `native_cube`) with the switch on,
/// and says nothing with it off, which keeps the stacked image.
#[test]
fn a_sampler2darray_program_compiles_native_for_an_etc_array() {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    let src = "#version 300 es\nprecision highp float;\nuniform highp sampler2DArray s;\nuniform vec3 c;\n\
               uniform float l;\nin vec2 uv;\nout vec4 o;\n\
               void main() { o = texture(s, c) + textureLod(s, c, l) + texelFetch(s, ivec3(uv, 1), 0); }\n";
    for (on, flag) in [(true, 2u8), (false, 0)] {
        yakogl::shaderc::pvr_set_native_array(i32::from(on));
        let got = store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
            compile_fragment(&l.module, &l.reflection, a).map(|r| r.native_cube).map_err(|e| format!("chain: {e:?}"))
        })
        .unwrap();
        yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
        assert_eq!(got.unwrap_or_else(|m| panic!("on={on}: {m}")) & 2, flag, "on={on}");
    }
}
