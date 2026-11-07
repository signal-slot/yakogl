// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core's CPU mipmap filter (glGenerateMipmap, src/gl/texture.rs) reads
//! each source row once with wide loads. Its bytes must be those of the filter
//! it replaced, which did four single-byte loads a 2x2 box: this test keeps a
//! copy of that filter (`reference_chain`, the per-channel `(sum + n / 2) / n`
//! loop of origin/main d254d172) and compares the whole storage of the
//! texture, every level and every pad byte, with the real call's, over odd
//! and 1-wide sizes, each format the filter takes, on two level layouts.
//! It also checks that `gl_mip_cpu_counters` advances by the source bytes of
//! the levels and one call.
//!
//! Not in a `report-es2` build: the NPOT sizes need GL_OES_texture_npot.
//!
//! Run: `cargo test --no-default-features --features host-test,shaderc-online
//! --test gl_mip_cpu_filter_wide`.

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, gles_texfmt_bytes, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, CAP_MIP_LEVELS};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glGetError, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glGenerateMipmap, glTexImage2D, gl_mip_cpu_counters,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back ends that make one */

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

const fn backend_table(name: &'static core::ffi::CStr, native_layered: bool, mip_level_align: u32) -> YakoglBackend {
    YakoglBackend {
        name: name.as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align,
        compressed_blocks_in_rows: false,
        native_layered_textures: native_layered,
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
    }
}

static PACKED: YakoglBackend = backend_table(c"mip-wide-packed", true, 0);
static ALIGNED: YakoglBackend = backend_table(c"mip-wide-aligned", true, 64);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context(b: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: both tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &PACKED;
            assert!(!context::gl_context_create(8, 8).is_null());
        });
        backend::g_backend = b;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// xorshift64*, a fixed seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u8 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
    }
}

/// The filter as origin/main d254d172 ran it on a one-layer 8-bit texture
/// (`csize == 1`, not sRGB): copied, with the setup reduced to the arguments.
fn reference_chain(texels: &mut [u8], t: &context::GlTexture, bpp: usize) {
    let comps = bpp;
    let csize = bpp / comps;
    let layers = t.layers.max(1) as usize;
    let (mut w, mut fh) = (t.width as usize, t.height as usize);
    for l in 1..(t.chain_levels as usize).min(CAP_MIP_LEVELS) {
        let src = t.level_off[l - 1] as usize;
        let dst = t.level_off[l] as usize;
        let (dw, dfh) = (if w > 1 { w / 2 } else { 1 }, if fh > 1 { fh / 2 } else { 1 });
        let (sx, sy) = (if w > 1 { 2usize } else { 1 }, if fh > 1 { 2usize } else { 1 });
        let (sp, dp) = (t.level_pitch_at(l as u32 - 1) as usize, t.level_pitch_at(l as u32) as usize);
        for y in 0..dfh * layers {
            let sy0 = (y / dfh) * fh + (y % dfh) * sy;
            for x in 0..dw {
                for ch in 0..comps {
                    let d = dst + (y * dp + x) * bpp + ch * csize;
                    let box_src = |j: usize, i: usize| src + ((sy0 + j) * sp + x * sx + i) * bpp + ch * csize;
                    let mut sum: u32 = 0;
                    for j in 0..sy {
                        for i in 0..sx {
                            sum += texels[box_src(j, i)] as u32;
                        }
                    }
                    let n = (sx * sy) as u32;
                    texels[d] = ((sum + n / 2) / n) as u8;
                }
            }
        }
        w = dw;
        fh = dfh;
    }
}

/// Source bytes the filter reads for the chain of `t`.
fn chain_source_bytes(t: &context::GlTexture, bpp: usize) -> u64 {
    let (mut w, mut fh) = (t.width as usize, t.height as usize);
    let mut total = 0;
    for _ in 1..(t.chain_levels as usize).min(CAP_MIP_LEVELS) {
        let (dw, dfh) = ((w / 2).max(1), (fh / 2).max(1));
        let (sx, sy) = (if w > 1 { 2 } else { 1 }, if fh > 1 { 2 } else { 1 });
        total += sx * sy * dw * dfh * bpp;
        w = dw;
        fh = dfh;
    }
    total as u64
}

const SIZES: [(u32, u32); 11] =
    [(1, 1), (1, 7), (7, 1), (2, 2), (3, 5), (8, 8), (75, 37), (130, 3), (257, 255), (64, 300), (800, 600)];

const FORMATS: [(&str, GLenum, usize); 5] = [
    ("RGBA8", GL_RGBA, 4),
    ("RGB8", GL_RGB, 3),
    ("LA8", GL_LUMINANCE_ALPHA, 2),
    ("L8", GL_LUMINANCE, 1),
    ("A8", GL_ALPHA, 1),
];

fn run(b: &'static YakoglBackend) {
    let _g = context(b);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut filtered_seen = 0;
    for (fname, fmt, per_texel) in FORMATS {
        for (w, h) in SIZES {
            let img: Vec<u8> = (0..w as usize * h as usize * per_texel).map(|_| rng.next()).collect();
            let mut name = 0;
            // SAFETY: plain GL calls; `img` holds w*h texels of the format.
            unsafe {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    fmt as GLint,
                    w as GLsizei,
                    h as GLsizei,
                    0,
                    fmt,
                    GL_UNSIGNED_BYTE,
                    img.as_ptr() as *const c_void,
                );
                assert_eq!(glGetError(), GL_NO_ERROR, "{fname} {w}x{h} upload");
            }
            let c = context::current().expect("a current context");
            let slot = names::slot_of(c, Kind::Texture, name).expect("a texture name");
            let c0 = gl_mip_cpu_counters();
            // SAFETY: plain GL call.
            unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
            // SAFETY: plain GL call.
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{fname} {w}x{h} glGenerateMipmap");
            let c1 = gl_mip_cpu_counters();
            let t = c.texture_object(slot).expect("a texture object");
            // SAFETY: as above.
            let after = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
            let bpp = gles_texfmt_bytes(t.texfmt) as usize;
            /* the storage may be laid out anew by the call (a chain made room
             * for): the reference starts from level 0 alone in a zeroed copy
             * of the storage, as the real filter finds the chain's levels
             * 1.. zeroed */
            let mut expect = vec![0u8; after.len()];
            let l0 = t.level_off[0] as usize;
            let l0_len = h as usize * t.level_pitch_at(0) as usize * bpp;
            expect[l0..l0 + l0_len].copy_from_slice(&after[l0..l0 + l0_len]);
            if c1.2 - c0.2 == 1 {
                reference_chain(&mut expect, t, bpp);
                filtered_seen += 1;
                assert_eq!(c1.1 - c0.1, chain_source_bytes(t, bpp), "{fname} {w}x{h}: source bytes counted");
                assert!(c1.0 >= c0.0);
            } else {
                assert_eq!((c1.1, c1.2), (c0.1, c0.2), "{fname} {w}x{h}: an unfiltered texture counts nothing");
            }
            assert!(expect == after, "{fname} {w}x{h}: the chain differs from the old filter's");
            // SAFETY: a name made above.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
    assert!(filtered_seen >= SIZES.len(), "the CPU filter ran for at least RGBA8 at every size");
}

#[test]
fn wide_filter_matches_the_old_filter_on_packed_levels() {
    run(&PACKED);
}

#[test]
fn wide_filter_matches_the_old_filter_on_aligned_levels() {
    run(&ALIGNED);
}
