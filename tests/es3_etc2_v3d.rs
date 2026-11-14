// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D samples the ETC2 block formats the GL core stores
//! (`V3D_SAMPLED_TEXFMTS` lists `GLES_TEXFMT_ETC2_RGB`, `_RGBA`, `_SRGB_RGB`,
//! `_SRGB_RGBA` and the unused `_RGB_PT`): the GL core's storage of an ETC2
//! upload, through the GL API with the V3D's layout flags, is what the V3D
//! back end reads, and the back end's draw refuses none of it. Until the
//! switch-on a draw that sampled any of them was refused as "a texel format
//! the V3D does not sample" (`texture::refusal`), the OSAL-side tiling and
//! `layout_compressed` of 333a19f7 notwithstanding.
//!
//! Pinned:
//!
//! * the stand-in back end has the V3D's layout flags (blocks in raster rows,
//!   no level or row alignment, native layers); that the real table has them
//!   is pinned by `v3d::backend::backend_tests::the_v3d_table_reads_blocks_in_raster_rows`
//!   (this binary links no job builder, so it cannot name the table);
//! * for each of the four names a texture of 10x6 with its chain, a cube map
//!   face and a 3-layer array: the GL core's level offsets are
//!   `v3d::texture::gl_level_blocks`' (the offsets `texture::tile` reads the
//!   grid of blocks from), the storage is `gl_bytes`' size, and block (bx, by)
//!   of a level is at `by * pitch + bx` blocks (raster rows, no padding);
//! * the binding a draw of it makes is a class the V3D samples, with a TMU
//!   row, and the sampled set refuses nothing of it (`texture::refusal` is
//!   None, the level offsets fit the storage); a 3D texture of the format is
//!   refused by name.
//!
//! The draw itself, with the tiling and the record, is
//! `v3d::backend::backend_tests::an_etc2_texture_is_drawn_and_its_blocks_are_tiled_as_texels`
//! (the job builder is a fake there); the TMU's decode is the board's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_v3d
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-v3d",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, gles_texfmt_bit, GlesDraw, GlesHeap, YakoglBackend, GLES_TEXFMT_ETC2_RGB, GLES_TEXFMT_ETC2_RGBA,
    GLES_TEXFMT_ETC2_SRGB_RGB, GLES_TEXFMT_ETC2_SRGB_RGBA, GLES_TEXTYPE_3D, V3D_SAMPLED_TEXFMTS,
};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glCompressedTexImage2D, glCompressedTexImage3D, glGenTextures};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::texture;

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

/// The V3D's layout flags (`yakogl_backend_v3d`'s), a draw that does nothing.
static V3D_LIKE: YakoglBackend = YakoglBackend {
    name: c"es3-etc2-v3d".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: true,
    native_layered_textures: true,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: true,
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
    samples_texel_copies: true,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: true,
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
            backend::g_backend = &V3D_LIKE;
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

fn texture_object(name: GLuint) -> &'static context::GlTexture {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
    c.texture_object(slot).expect("a texture object")
}

/// (GL name, class, block bytes) of the four ETC2 names the GL core stores.
const FORMATS: [(GLenum, u8, u32); 4] = [
    (GL_COMPRESSED_RGB8_ETC2, GLES_TEXFMT_ETC2_RGB, 8),
    (GL_COMPRESSED_SRGB8_ETC2, GLES_TEXFMT_ETC2_SRGB_RGB, 8),
    (GL_COMPRESSED_RGBA8_ETC2_EAC, GLES_TEXFMT_ETC2_RGBA, 16),
    (GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, GLES_TEXFMT_ETC2_SRGB_RGBA, 16),
];

/// The bytes of block (bx, by) of level `l` of layer `z`: distinct for each.
fn block(bb: u32, z: u32, l: u32, bx: u32, by: u32) -> Vec<u8> {
    (0..bb).map(|i| (z * 53 + l * 29 + bx * 11 + by * 5 + i * 3 + 1) as u8).collect()
}

fn level_blocks(bb: u32, w: u32, h: u32, z: u32, layers: u32, l: u32) -> Vec<u8> {
    let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
    let mut d = Vec::new();
    for _ in 0..layers.min(1) {
        for by in 0..lh.div_ceil(4) {
            for bx in 0..lw.div_ceil(4) {
                d.extend(block(bb, z, l, bx, by));
            }
        }
    }
    d
}

fn upload_2d(format: GLenum, bb: u32, w: u32, h: u32, target: GLenum, name: GLuint) {
    let levels = w.max(h).ilog2() + 1;
    // SAFETY: each `data` is the imageSize named.
    unsafe {
        glBindTexture(if target == GL_TEXTURE_2D { GL_TEXTURE_2D } else { GL_TEXTURE_CUBE_MAP }, name);
        for l in 0..levels {
            let data = level_blocks(bb, w, h, 0, 1, l);
            glCompressedTexImage2D(
                target,
                l as GLint,
                format,
                (w >> l).max(1) as GLsizei,
                (h >> l).max(1) as GLsizei,
                0,
                data.len() as GLsizei,
                data.as_ptr().cast(),
            );
            assert_eq!(glGetError(), GL_NO_ERROR, "level {l}");
        }
    }
}

#[test]
fn the_mask_holds_the_four_classes_the_gl_core_stores() {
    for (_, class, bb) in FORMATS {
        assert_ne!(V3D_SAMPLED_TEXFMTS & gles_texfmt_bit(class), 0, "class {class}");
        assert!(texture::etc2_of(class).is_some());
        let b = yakogl::backend::GlesTextureBind {
            width: 8,
            height: 8,
            layers: 1,
            mip_levels: 1,
            format: class,
            ..yakogl::backend::GlesTextureBind::ZERO
        };
        assert_eq!(texture::block_bytes(&b), Some(bb));
        assert_eq!(texture::refusal(&b), None, "class {class}");
    }
    let vol = yakogl::backend::GlesTextureBind {
        width: 8,
        height: 8,
        layers: 4,
        type_: GLES_TEXTYPE_3D,
        mip_levels: 1,
        format: GLES_TEXFMT_ETC2_RGB,
        ..yakogl::backend::GlesTextureBind::ZERO
    };
    assert_eq!(texture::refusal(&vol), Some("a 3D texture of a compressed format"));
}

/// The GL core's 2D storage is the grid of blocks `gl_level_blocks` reads,
/// and its binding is accepted.
#[test]
fn a_2d_texture_is_stored_as_the_back_end_reads_it() {
    let _g = context();
    for (format, class, bb) in FORMATS {
        let mut name = 0;
        // SAFETY: room for one name.
        unsafe { glGenTextures(1, &mut name) };
        upload_2d(format, bb, 10, 6, GL_TEXTURE_2D, name);
        let t = texture_object(name);
        assert_eq!((t.texfmt, t.block_bytes as u32), (class, bb), "format {format:#x}");
        let levels = 4; /* 10x6: 10x6, 5x3, 2x1, 1x1 */
        assert_eq!(t.chain_levels, levels);
        for l in 0..levels as u32 {
            let (off, gw, gh) = texture::gl_level_blocks(10, 6, 1, bb, l);
            assert_eq!(t.level_off[l as usize], off, "format {format:#x} level {l}");
            let level_data = level_blocks(bb, 10, 6, 0, 1, l);
            assert_eq!(level_data.len() as u32, gw * gh * bb);
            // SAFETY: the level lies inside the storage (gl_level_blocks).
            let stored =
                unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(off as usize), level_data.len()) };
            assert_eq!(stored, &level_data[..], "format {format:#x} level {l}: raster rows of blocks");
        }
        /* the binding a draw of it makes: its class, its size and its levels */
        let b = yakogl::backend::GlesTextureBind {
            width: t.width,
            height: t.height,
            stride_texels: t.width,
            layers: 1,
            mip_levels: levels,
            mip_filter: 1,
            format: t.texfmt,
            ..yakogl::backend::GlesTextureBind::ZERO
        };
        assert_eq!(texture::refusal(&b), None, "format {format:#x}");
        assert!(
            texture::gl_bytes(&b) as usize <= t.mem.size,
            "format {format:#x}: the sampled levels lie inside the storage"
        );
    }
    assert_eq!(error(), GL_NO_ERROR);
}

/// A cube map's faces and a 2D array's layers: each level holds its layers
/// one after the other, each a grid of blocks (`gl_level_blocks` with the
/// layer count).
#[test]
fn a_cube_and_an_array_stack_their_layers_in_each_level() {
    let _g = context();
    for (format, class, bb) in FORMATS {
        /* a 2D array of 3 layers, 8x8, three levels uploaded */
        let mut name = 0;
        // SAFETY: room for one name.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D_ARRAY, name);
        }
        for l in 0..4u32 {
            let mut data = Vec::new();
            for z in 0..3 {
                data.extend(level_blocks(bb, 8, 8, z, 1, l));
            }
            // SAFETY: `data` is the imageSize named.
            unsafe {
                glCompressedTexImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    l as GLint,
                    format,
                    (8 >> l).max(1) as GLsizei,
                    (8 >> l).max(1) as GLsizei,
                    3,
                    0,
                    data.len() as GLsizei,
                    data.as_ptr().cast(),
                )
            };
            assert_eq!(error(), GL_NO_ERROR, "format {format:#x} level {l}");
        }
        let t = texture_object(name);
        assert_eq!((t.texfmt, t.layers, t.block_bytes as u32), (class, 3, bb));
        for l in 0..4u32 {
            let (off, gw, gh) = texture::gl_level_blocks(8, 8, 3, bb, l);
            assert_eq!(t.level_off[l as usize], off, "format {format:#x} array level {l}");
            for z in 0..3u32 {
                let want = level_blocks(bb, 8, 8, z, 1, l);
                let at = off + z * gw * gh * bb;
                // SAFETY: inside the storage (gl_level_blocks).
                let stored =
                    unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at as usize), want.len()) };
                assert_eq!(stored, &want[..], "format {format:#x} layer {z} level {l}");
            }
        }
        /* a cube: six faces of 8x8, level 0 only */
        let mut cube = 0;
        // SAFETY: room for one name.
        unsafe { glGenTextures(1, &mut cube) };
        for face in 0..6u32 {
            // SAFETY: as above.
            unsafe {
                glBindTexture(GL_TEXTURE_CUBE_MAP, cube);
                let data = level_blocks(bb, 8, 8, face, 1, 0);
                glCompressedTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                    0,
                    format,
                    8,
                    8,
                    0,
                    data.len() as GLsizei,
                    data.as_ptr().cast(),
                );
            }
            assert_eq!(error(), GL_NO_ERROR, "format {format:#x} face {face}");
        }
        let t = texture_object(cube);
        let (off, gw, gh) = texture::gl_level_blocks(8, 8, 6, bb, 0);
        assert_eq!((t.level_off[0], t.layers), (off, 6));
        for face in 0..6u32 {
            let want = level_blocks(bb, 8, 8, face, 1, 0);
            let at = off + face * gw * gh * bb;
            // SAFETY: inside the storage (gl_level_blocks).
            let stored = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at as usize), want.len()) };
            assert_eq!(stored, &want[..], "format {format:#x} face {face}");
        }
    }
}
