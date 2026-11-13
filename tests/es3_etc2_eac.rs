// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ten OpenGL ES 3.0 ETC2/EAC compressed formats through the GL API
//! (item etc2-eac; design in docs/notes/etc2-eac.md).
//!
//! The GL core stores four of them as blocks the GPU decodes
//! (GL_COMPRESSED_RGB8_ETC2, RGBA8_ETC2_EAC and, where the back end samples
//! them, the two sRGB ones) and decodes the other six on the CPU at upload
//! (`gl::etc2`, checked against Mesa's decoder in its own tests):
//! R11_EAC, SIGNED_R11_EAC, RG11_EAC, SIGNED_RG11_EAC into R16F / RG16F, the two
//! punch-through formats into RGBA8 / SRGB8_ALPHA8. Pinned here, on every GPU
//! build:
//!
//! - GL_NUM_COMPRESSED_TEXTURE_FORMATS and GL_COMPRESSED_TEXTURE_FORMATS list
//!   the ten (and ETC1) under ES 3.0; under `report-es2` the list is the
//!   three of ES 2.0 and the six names are GL_INVALID_ENUM;
//! - every name takes glCompressedTexImage2D (levels, cube faces),
//!   glCompressedTexSubImage2D, a 2D array through glCompressedTexImage3D /
//!   glCompressedTexSubImage3D, and a PIXEL_UNPACK_BUFFER offset, with the
//!   errors of ES 3.0 3.8.6 (imageSize, whole blocks, no 3D texture);
//! - a decode leaves the application's unpack state as it was (the texels
//!   themselves are checked in src/gl/texture.rs's tests, which can read the storage).
//!
//! What the picture looks like (filtering, the 11-bit precision against dEQP's
//! thresholds) is the board's and the host dEQP's to say, not this file's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-powervr --test es3_etc2_eac
//! cargo test --no-default-features --features host-test,gpu-vivante,report-es2 --test es3_etc2_eac
//! ```

#![cfg(all(feature = "host-test", not(feature = "gpu-mali")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
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
    // SAFETY: a plain GL call.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    guard
}

fn error() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/* ---- the ten formats ------------------------------------------------------- */

const ETC1: GLenum = GL_ETC1_RGB8_OES;

/// (name, bytes a 4x4 block) of the ten ES 3.0 formats.
const TEN: [(GLenum, usize); 10] = [
    (GL_COMPRESSED_RGB8_ETC2, 8),
    (GL_COMPRESSED_SRGB8_ETC2, 8),
    (GL_COMPRESSED_RGBA8_ETC2_EAC, 16),
    (GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC, 16),
    (GL_COMPRESSED_R11_EAC, 8),
    (GL_COMPRESSED_SIGNED_R11_EAC, 8),
    (GL_COMPRESSED_RG11_EAC, 16),
    (GL_COMPRESSED_SIGNED_RG11_EAC, 16),
    (GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8),
    (GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, 8),
];

/// The six the GL core decodes at upload on every build.
const SIX: [GLenum; 6] = [
    GL_COMPRESSED_R11_EAC,
    GL_COMPRESSED_SIGNED_R11_EAC,
    GL_COMPRESSED_RG11_EAC,
    GL_COMPRESSED_SIGNED_RG11_EAC,
    GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
    GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
];

fn list() -> Vec<GLenum> {
    let mut n = 0;
    let mut v = [0 as GLint; 16];
    // SAFETY: room for every value the widest pname writes.
    unsafe {
        glGetIntegerv(GL_NUM_COMPRESSED_TEXTURE_FORMATS, &mut n);
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, v.as_mut_ptr());
    }
    assert_eq!(error(), GL_NO_ERROR);
    v[..n as usize].iter().map(|&f| f as GLenum).collect()
}

/// `n` blocks of `bb` bytes, each block's bytes its own.
fn blocks(n: usize, bb: usize) -> Vec<u8> {
    (0..n * bb).map(|i| (0x21 + (i / bb) * 37 + (i % bb) * 11) as u8).collect()
}

fn new_texture(target: GLenum) -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
    }
    t
}

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;
    use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};

    #[test]
    fn the_ten_formats_are_listed() {
        let _g = context();
        let l = list();
        for (f, _) in TEN {
            assert_eq!(l.iter().filter(|&&x| x == f).count(), 1, "{f:#x} once in the list {l:x?}");
        }
        assert!(l.contains(&ETC1));
        assert_eq!(l.len(), 11, "ETC1 and the ten: {l:x?}");
        for f in SIX {
            assert!(l.contains(&f));
        }
    }

    #[test]
    fn every_format_takes_a_chain_a_sub_image_and_a_cube() {
        let _g = context();
        for (f, bb) in TEN {
            let _t = new_texture(GL_TEXTURE_2D);
            // SAFETY: each call's data holds the imageSize it names.
            unsafe {
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 8, 8, 0, 4 * bb as GLsizei, blocks(4, bb).as_ptr().cast());
                glCompressedTexImage2D(GL_TEXTURE_2D, 1, f, 4, 4, 0, bb as GLsizei, blocks(1, bb).as_ptr().cast());
                glCompressedTexImage2D(GL_TEXTURE_2D, 2, f, 2, 2, 0, bb as GLsizei, blocks(1, bb).as_ptr().cast());
                glCompressedTexImage2D(GL_TEXTURE_2D, 3, f, 1, 1, 0, bb as GLsizei, blocks(1, bb).as_ptr().cast());
            }
            assert_eq!(error(), GL_NO_ERROR, "{f:#x}: the chain");
            // SAFETY: as above.
            unsafe {
                glCompressedTexSubImage2D(
                    GL_TEXTURE_2D,
                    0,
                    4,
                    4,
                    4,
                    4,
                    f,
                    bb as GLsizei,
                    blocks(1, bb).as_ptr().cast(),
                );
            }
            assert_eq!(error(), GL_NO_ERROR, "{f:#x}: a sub-image of one block");
            let _c = new_texture(GL_TEXTURE_CUBE_MAP);
            for face in 0..6 {
                // SAFETY: as above.
                unsafe {
                    glCompressedTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                        0,
                        f,
                        4,
                        4,
                        0,
                        bb as GLsizei,
                        blocks(1, bb).as_ptr().cast(),
                    );
                }
            }
            assert_eq!(error(), GL_NO_ERROR, "{f:#x}: the six faces");
        }
    }

    #[test]
    fn a_2d_array_takes_every_format_and_a_3d_texture_none() {
        let _g = context();
        for (f, bb) in TEN {
            let _t = new_texture(GL_TEXTURE_2D_ARRAY);
            // SAFETY: each call's data holds the imageSize it names.
            unsafe {
                glCompressedTexImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    0,
                    f,
                    4,
                    4,
                    3,
                    0,
                    3 * bb as GLsizei,
                    blocks(3, bb).as_ptr().cast(),
                );
                assert_eq!(error(), GL_NO_ERROR, "{f:#x}: three layers");
                glCompressedTexSubImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    0,
                    0,
                    0,
                    1,
                    4,
                    4,
                    2,
                    f,
                    2 * bb as GLsizei,
                    blocks(2, bb).as_ptr().cast(),
                );
                assert_eq!(error(), GL_NO_ERROR, "{f:#x}: two layers of it");
                let _v = new_texture(GL_TEXTURE_3D);
                glCompressedTexImage3D(GL_TEXTURE_3D, 0, f, 4, 4, 1, 0, bb as GLsizei, blocks(1, bb).as_ptr().cast());
                assert_eq!(error(), GL_INVALID_OPERATION, "{f:#x}: no 3D form (ES 3.0 3.8.6)");
            }
        }
    }

    #[test]
    fn the_errors_of_the_compressed_calls_are_the_same_for_every_format() {
        let _g = context();
        for (f, bb) in TEN {
            let _t = new_texture(GL_TEXTURE_2D);
            let d = blocks(4, bb);
            // SAFETY: the failing calls read nothing; the good ones hold their imageSize.
            unsafe {
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 8, 8, 0, 4 * bb as GLsizei - 1, d.as_ptr().cast());
                assert_eq!(error(), GL_INVALID_VALUE, "{f:#x}: imageSize");
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 8, 8, 1, 4 * bb as GLsizei, d.as_ptr().cast());
                assert_eq!(error(), GL_INVALID_VALUE, "{f:#x}: border");
                glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, f, bb as GLsizei, d.as_ptr().cast());
                assert_eq!(error(), GL_INVALID_OPERATION, "{f:#x}: no image to replace");
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 8, 8, 0, 4 * bb as GLsizei, d.as_ptr().cast());
                assert_eq!(error(), GL_NO_ERROR);
                glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 2, 0, 4, 4, f, bb as GLsizei, d.as_ptr().cast());
                assert_eq!(error(), GL_INVALID_OPERATION, "{f:#x}: not on a block");
                glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 4, 4, 8, 8, f, 4 * bb as GLsizei, d.as_ptr().cast());
                assert_eq!(error(), GL_INVALID_VALUE, "{f:#x}: outside the level");
                glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, f, 2 * bb as GLsizei, d.as_ptr().cast());
                assert_eq!(error(), GL_INVALID_VALUE, "{f:#x}: sub-image imageSize");
            }
        }
    }

    /// With a PIXEL_UNPACK_BUFFER bound, `data` is an offset into it.
    #[test]
    fn a_pixel_unpack_buffer_offset_is_the_blocks_address() {
        let _g = context();
        for (f, bb) in TEN {
            let mut buf = 0;
            let mut data = vec![0xeeu8; 16];
            data.extend(blocks(1, bb));
            let _t = new_texture(GL_TEXTURE_2D);
            // SAFETY: plain GL calls; the buffer holds 16 bytes of padding and the block.
            unsafe {
                glGenBuffers(1, &mut buf);
                glBindBuffer(GL_PIXEL_UNPACK_BUFFER, buf);
                glBufferData(GL_PIXEL_UNPACK_BUFFER, data.len() as isize, data.as_ptr().cast(), GL_STATIC_DRAW);
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 4, 4, 0, bb as GLsizei, 16 as *const c_void);
                assert_eq!(error(), GL_NO_ERROR, "{f:#x}: the offset");
                glCompressedTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 4, 4, f, bb as GLsizei, 16 as *const c_void);
                assert_eq!(error(), GL_NO_ERROR, "{f:#x}: a sub-image at the offset");
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 4, 4, 0, bb as GLsizei, 64 as *const c_void);
                assert_eq!(error(), GL_INVALID_OPERATION, "{f:#x}: past the buffer's end");
                glBindBuffer(GL_PIXEL_UNPACK_BUFFER, 0);
            }
        }
    }

    /// The application's unpack state is neither read for the blocks nor
    /// changed by the decode (the decoded texels go through the uncompressed
    /// path under the default state, and the application's is put back); the
    /// texels themselves are checked in src/gl/texture.rs's tests, which can
    /// read the storage.
    #[test]
    fn a_decoded_upload_leaves_the_unpack_state_alone() {
        let _g = context();
        for f in SIX {
            let bb = TEN.iter().find(|t| t.0 == f).unwrap().1;
            let _t = new_texture(GL_TEXTURE_2D);
            let mut v = 0;
            // SAFETY: plain GL calls; the data holds the imageSize it names.
            unsafe {
                glPixelStorei(GL_UNPACK_ALIGNMENT, 8);
                glPixelStorei(GL_UNPACK_ROW_LENGTH, 99);
                glPixelStorei(GL_UNPACK_SKIP_PIXELS, 5);
                glPixelStorei(GL_UNPACK_SKIP_ROWS, 3);
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 8, 4, 0, 2 * bb as GLsizei, blocks(2, bb).as_ptr().cast());
                assert_eq!(error(), GL_NO_ERROR, "{f:#x}");
                for (pname, want) in [
                    (GL_UNPACK_ALIGNMENT, 8),
                    (GL_UNPACK_ROW_LENGTH, 99),
                    (GL_UNPACK_SKIP_PIXELS, 5),
                    (GL_UNPACK_SKIP_ROWS, 3),
                ] {
                    glGetIntegerv(pname, &mut v);
                    assert_eq!(v, want, "{f:#x}: pname {pname:#x} is kept");
                }
                glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
                glPixelStorei(GL_UNPACK_ROW_LENGTH, 0);
                glPixelStorei(GL_UNPACK_SKIP_PIXELS, 0);
                glPixelStorei(GL_UNPACK_SKIP_ROWS, 0);
            }
        }
    }
}

#[cfg(feature = "report-es2")]
mod es2 {
    use super::*;

    /// An ES 2.0 report lists ETC1, RGB8_ETC2 and RGBA8_ETC2_EAC and takes none
    /// of the six (ES 2.0 has no EAC, no punch-through, no sRGB compressed).
    #[test]
    fn the_list_is_unchanged_and_the_six_are_refused() {
        let _g = context();
        assert_eq!(list(), [ETC1, GL_COMPRESSED_RGB8_ETC2, GL_COMPRESSED_RGBA8_ETC2_EAC]);
        for f in SIX {
            let _t = new_texture(GL_TEXTURE_2D);
            let bb = TEN.iter().find(|t| t.0 == f).unwrap().1;
            // SAFETY: the data holds one block.
            unsafe {
                glCompressedTexImage2D(GL_TEXTURE_2D, 0, f, 4, 4, 0, bb as GLsizei, blocks(1, bb).as_ptr().cast())
            };
            assert_eq!(error(), GL_INVALID_ENUM, "{f:#x}");
        }
    }
}
