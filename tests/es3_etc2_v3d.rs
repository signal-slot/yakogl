// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ten ES 3.0 ETC2/EAC formats on the V3D 4.2 (plan #354, item
//! v3d-etc2): what the GL core stores for each name on the V3D build and
//! what the TMU is handed for it.
//!
//! The V3D samples four of them as blocks (the GL core's ETC2 classes,
//! `V3D_SAMPLED_TEXFMTS` since this item: before it every ETC2 draw was
//! refused as "a texel format the V3D does not sample") and the GL core
//! decodes the other six at upload (docs/notes/etc2-eac.md) into R16F,
//! RGBA16F, RGBA8 and SRGB8_ALPHA8 texels. Pinned here, per name: the class
//! the upload stores, that the mask holds it, that the binding the draw would
//! hand over is not refused, and its TMU texture type, written out from
//! Mesa's V3D 4.2 format table (v3dx_format_table.c:159-168 for the blocks,
//! the R16F / RGBA16F / RGBA8 rows for the decoded ones), not from the
//! helpers; and the storage size of a block binding. The draw itself, the
//! record the job builder is given, is `v3d::backend_tests`
//! `all_ten_etc2_eac_formats_are_drawn_with_their_tmu_type`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::glGetError;
use yakogl::gl::texture::{glBindTexture, glCompressedTexImage2D, glGenTextures};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::texture;

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
    name: c"es3-etc2-v3d".as_ptr() as *const c_char,
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

/* ---- the ten names -------------------------------------------------------- */

/// (name, bytes per 4x4 block, class stored, V3D texture type, native block
/// class). Types are Mesa's `TEXTURE_DATA_FORMAT_*` (v3d_packet.xml): 32
/// RGB8_ETC2, 38 RGBA8_ETC2_EAC, 16 R16F, 18 RGBA16F, 4 RGBA8.
const TEN: [(GLenum, usize, u8, u32, bool); 10] = [
    (GL_COMPRESSED_RGB8_ETC2, 8, GLES_TEXFMT_ETC2_RGB, 32, true),
    (GL_COMPRESSED_SRGB8_ETC2, 8, GLES_TEXFMT_ETC2_SRGB_RGB, 32, true),
    (GL_COMPRESSED_RGBA8_ETC2_EAC, 16, GLES_TEXFMT_ETC2_RGBA, 38, true),
    (GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, 16, GLES_TEXFMT_ETC2_SRGB_RGBA, 38, true),
    (GL_COMPRESSED_R11_EAC, 8, GLES_TEXFMT_R16F, 16, false),
    (GL_COMPRESSED_SIGNED_R11_EAC, 8, GLES_TEXFMT_R16F, 16, false),
    (GL_COMPRESSED_RG11_EAC, 16, GLES_TEXFMT_RGBA16F, 18, false),
    (GL_COMPRESSED_SIGNED_RG11_EAC, 16, GLES_TEXFMT_RGBA16F, 18, false),
    (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8, GLES_TEXFMT_RGBA8, 4, false),
    (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8, GLES_TEXFMT_SRGB8_A8, 4, false),
];

fn error() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// The binding a draw of `t` hands the back end, as `gl::draw` builds it for
/// a 2D texture of one level.
fn bind_of(t: &context::GlTexture) -> GlesTextureBind {
    GlesTextureBind {
        width: t.width,
        height: t.height,
        stride_texels: t.level_pitch(t.width),
        layers: 1,
        mip_levels: 1,
        format: t.texfmt,
        ..GlesTextureBind::ZERO
    }
}

fn upload_8x8(name: GLenum, bb: usize) -> GLuint {
    let mut tex = 0;
    let data: Vec<u8> = (0..4 * bb).map(|i| (i * 29 + 3) as u8).collect();
    // SAFETY: one name of room; `data` is the 8x8 image's imageSize.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glCompressedTexImage2D(GL_TEXTURE_2D, 0, name, 8, 8, 0, data.len() as GLsizei, data.as_ptr() as *const c_void);
    }
    assert_eq!(error(), GL_NO_ERROR, "upload 0x{name:x}");
    tex
}

#[test]
fn the_mask_holds_the_four_block_classes_and_the_decode_targets() {
    for f in [
        GLES_TEXFMT_ETC2_RGB,
        GLES_TEXFMT_ETC2_RGBA,
        GLES_TEXFMT_ETC2_SRGB_RGB,
        GLES_TEXFMT_ETC2_SRGB_RGBA,
        GLES_TEXFMT_R16F,
        GLES_TEXFMT_RGBA16F,
        GLES_TEXFMT_RGBA8,
        GLES_TEXFMT_SRGB8_A8,
    ] {
        assert_ne!(V3D_SAMPLED_TEXFMTS & gles_texfmt_bit(f), 0, "class {f}");
    }
    /* class 3, the punch-through block class, is no class the GL core stores */
    assert_eq!(V3D_SAMPLED_TEXFMTS & gles_texfmt_bit(GLES_TEXFMT_ETC2_RGB_PT), 0);
}

#[test]
fn each_name_is_stored_in_a_class_the_tmu_samples_with_its_type() {
    let _g = context();
    for (name, bb, class, type_, native) in TEN {
        let tex = upload_8x8(name, bb);
        let t = texture_object(tex);
        assert_eq!(t.texfmt, class, "0x{name:x}: the stored class");
        assert_eq!(t.block_bytes != 0, native, "0x{name:x}: blocks or decoded texels");
        let b = bind_of(t);
        assert_eq!(texture::refusal(&b), None, "0x{name:x}: not refused");
        let (got, _) = texture::tmu_format(&b);
        assert_eq!(got, type_, "0x{name:x}: the TMU texture type");
        if native {
            /* four 8-byte or 16-byte blocks, tiled into Mesa's UIF tree of 4x4 blocks */
            assert_eq!(texture::gl_bytes(&b) as usize, 4 * bb, "0x{name:x}: the GL core's block storage");
            assert!(
                texture::layout_compressed(8, 8, 1, 4, 4, bb as u32).is_some(),
                "0x{name:x}: Mesa's layout of the blocks"
            );
        }
    }
}

#[test]
fn a_compressed_3d_binding_stays_refused_and_only_the_native_classes_have_blocks() {
    let _g = context();
    let tex = upload_8x8(GL_COMPRESSED_RGBA8_ETC2_EAC, 16);
    let b = GlesTextureBind { type_: GLES_TEXTYPE_3D, layers: 2, ..bind_of(texture_object(tex)) };
    assert_eq!(texture::refusal(&b), Some("a 3D texture of a compressed format"));
    /* the decoded six are plain texels: the 3D rules are the uncompressed ones */
    for (name, bb, _, _, native) in TEN {
        if !native {
            let t = upload_8x8(name, bb);
            assert_eq!(texture_object(t).block_bytes, 0, "0x{name:x}");
        }
    }
}
