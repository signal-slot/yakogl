// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The level contents of a mipmap chain at glmark2 terrain's sizes, pinned.
//!
//! The PowerVR backend's `generate_mipmap` (src/pvr/backend.rs
//! `pvr_generate_mipmap`) draws levels 1.. with the GPU, one kick per level.
//! The host build cannot run that: its stand-in GPU draws nothing (see
//! tests/mipmap_offer_guard.rs), so no texel of a kick's level can be read
//! here. What this file pins is the CPU reference filter those levels must
//! equal: the GL core's glGenerateMipmap on a back end that offers no chain
//! (`YakoglBackend::generate_mipmap` None), which is also the answer the
//! GL core gives when the PowerVR backend refuses a chain or hands back a
//! tail (`Prefix(n)`, `Backend::mip_cpu_tail`). A change to how the AM62P
//! builds chains (fewer kicks, levels skipped or built lazily) has to leave
//! every level equal to these hashes; the board compares the same texels.
//!
//! Sizes: glmark2 terrain (scene-terrain.cpp at the pinned 22c527cb) renders
//! RGBA8 maps into 512x512 mipmapped textures (the 10-level chain of
//! tools/hostharness/glmark2/FINDINGS.md row 22; 9 kicks on the PowerVR) and
//! 256x256 height and normal maps; 1024x1024, 800x600 (the scene's window,
//! non power of two) and 512x256 (non square) cover a larger, an NPOT and a
//! non-square case. The glmark2 sources were not fetched for this file: the
//! sizes are the ones the repository's own findings name.
//!
//! Level 0 is a fixed ramp (`texel`); every level is read back through a
//! framebuffer (glFramebufferTexture2D at the level, glReadPixels) and its
//! FNV-1a 64 hash compared with `PINNED`. Levels of 4 texels or fewer are
//! also compared byte for byte (`SMALL`).
//!
//! How the pinned values were generated: once, from the code at d83e25b5,
//! by running this test with both tables empty (it then prints them instead
//! of asserting, `-- --nocapture`) and pasting the output between the
//! PINNED markers. Not in a
//! `report-es2` build (level attachments are ES 3.0).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_mip_chain_pin
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glCheckFramebufferStatus, glDeleteFramebuffers, glFramebufferTexture2D, glGenFramebuffers,
    glReadPixels,
};
use yakogl::gl::state::glGetError;
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glGenerateMipmap, glTexImage2D, glTexParameteri,
};
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

/// A back end that offers no chain: the GL core filters on the CPU.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"pvr-mip-chain-pin".as_ptr() as *const c_char,
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
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    g
}

/// Texel (x, y) of level 0: every channel a different ramp.
fn texel(x: u32, y: u32) -> [u8; 4] {
    [(x * 3 + 29) as u8, (y * 5 + 7) as u8, ((x ^ y) * 11 + 1) as u8, (255 - ((x + y) & 63)) as u8]
}

fn level0(w: u32, h: u32) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).flat_map(move |x| texel(x, y))).collect()
}

fn fnv1a64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| (h ^ u64::from(c)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// Every level of a `w` x `h` chain after glGenerateMipmap: (width, height,
/// bytes).
fn chain(w: u32, h: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = Vec::new();
    let (mut tex, mut fb) = (0, 0);
    let px = level0(w, h);
    // SAFETY: room for one name each and for the level's RGBA8 pixels at every call.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            px.as_ptr() as *const c_void,
        );
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(err(), GL_NO_ERROR, "{w}x{h}: glGenerateMipmap");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let (mut lw, mut lh, mut l) = (w, h, 0);
        loop {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, l);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{w}x{h} level {l}");
            let mut out = vec![0xa5u8; (lw * lh * 4) as usize];
            glReadPixels(
                0,
                0,
                lw as GLsizei,
                lh as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                out.as_mut_ptr() as *mut c_void,
            );
            assert_eq!(err(), GL_NO_ERROR, "{w}x{h}: read level {l}");
            levels.push((lw, lh, out));
            if lw == 1 && lh == 1 {
                break;
            }
            (lw, lh, l) = ((lw / 2).max(1), (lh / 2).max(1), l + 1);
        }
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glBindTexture(GL_TEXTURE_2D, 0);
        glDeleteTextures(1, &tex);
    }
    levels
}

/* PINNED_BEGIN */
/// (width, height, per-level FNV-1a 64 of the bytes, level 0 first).
const PINNED: &[(u32, u32, &[u64])] = &[
    (
        1024,
        1024,
        &[
            0xc426f8287a6aa325,
            0x1b9c69c8e15f8325,
            0x77766b1267afb325,
            0xfb85fb198b574325,
            0x89ef9d6cc1af9725,
            0x374080f5cb53d125,
            0x9df71f2b3c59b325,
            0x6ed49917a55409a5,
            0x073b00795de90825,
            0xc1a41c828ccf6b65,
            0x9d869aad2b3b2d55,
        ],
    ),
    (
        512,
        512,
        &[
            0x8f8535a41bb44325,
            0x676f3512dc317b25,
            0x64abd0e6fd558725,
            0x7d79ec42367f6b25,
            0x30761b565eb25c25,
            0x668adba94b0baea5,
            0x622a5a7ef5f39225,
            0x69e0dc81a9aecd45,
            0xc1a41c828ccf6b65,
            0x9d869aad2b3b2d55,
        ],
    ),
    (
        256,
        256,
        &[
            0x52cf6abafd46ab25,
            0xd12e8b0f063df925,
            0x49ff3b8c85564d25,
            0x29736a68a94611a5,
            0xde529605cc20eea5,
            0xe3e6993be48238c5,
            0xbaf31b8129cdf645,
            0x77e8e176dfd2e6fd,
            0x9d869aad2b3b2d55,
        ],
    ),
    (
        800,
        600,
        &[
            0xf1174fe1261664e5,
            0x57b75c26461993c5,
            0x6364f77f03633cc5,
            0xd89d217d237535b5,
            0xbc89110dd7df6fed,
            0xeae8c1b563ccf5d0,
            0xd941106ba5ad3055,
            0xb18f7032551e49f5,
            0xb0c2451196568a05,
            0x9d869aad2b3b2d55,
        ],
    ),
    (
        512,
        256,
        &[
            0x7ee0dc734feb3325,
            0x24eddef10029cf25,
            0x000898c78ebbd525,
            0x8724be117450c725,
            0x591c8901d24e48a5,
            0x74f8f9fda68a5b65,
            0xd5964a0b79db19a5,
            0xc5f92985fe6c2e15,
            0x186b072087d61ac5,
            0x9d869aad2b3b2d55,
        ],
    ),
];
/// (width, height, the bytes of every level of 4 texels or fewer).
const SMALL: &[(u32, u32, &[&[u8]])] = &[
    (
        1024,
        1024,
        &[&[128, 128, 128, 224, 128, 128, 128, 224, 128, 128, 128, 224, 128, 128, 128, 224], &[128, 128, 128, 224]],
    ),
    (
        512,
        512,
        &[&[128, 128, 128, 224, 128, 128, 128, 224, 128, 128, 128, 224, 128, 128, 128, 224], &[128, 128, 128, 224]],
    ),
    (
        256,
        256,
        &[&[116, 115, 122, 224, 140, 115, 134, 224, 116, 141, 134, 224, 140, 141, 122, 224], &[128, 128, 128, 224]],
    ),
    (800, 600, &[&[128, 128, 128, 224]]),
    (512, 256, &[&[128, 128, 128, 224, 128, 128, 128, 224], &[128, 128, 128, 224]]),
];
/* PINNED_END */

const SIZES: [(u32, u32); 5] = [(1024, 1024), (512, 512), (256, 256), (800, 600), (512, 256)];

#[test]
fn every_level_of_terrains_chains_is_what_was_pinned() {
    let _g = context();
    if PINNED.is_empty() {
        println!("const PINNED: &[(u32, u32, &[u64])] = &[");
        for (w, h) in SIZES {
            let hs: Vec<String> = chain(w, h).iter().map(|(_, _, b)| format!("{:#018x}", fnv1a64(b))).collect();
            println!("    ({w}, {h}, &[{}]),", hs.join(", "));
        }
        println!("];\nconst SMALL: &[(u32, u32, &[&[u8]])] = &[");
        for (w, h) in SIZES {
            let sm: Vec<String> =
                chain(w, h).iter().filter(|(a, b, _)| a * b <= 4).map(|(_, _, b)| format!("&{b:?}")).collect();
            println!("    ({w}, {h}, &[{}]),", sm.join(", "));
        }
        println!("];");
        return;
    }
    assert_eq!(PINNED.len(), SIZES.len());
    assert_eq!(SMALL.len(), SIZES.len());
    for (i, &(w, h)) in SIZES.iter().enumerate() {
        let (pw, ph, hashes) = PINNED[i];
        assert_eq!((w, h), (pw, ph));
        let levels = chain(w, h);
        assert_eq!(levels.len(), hashes.len(), "{w}x{h}: the chain's length");
        assert_eq!(levels[0].2, level0(w, h), "{w}x{h}: level 0 is untouched");
        for (l, ((lw, lh, b), &want)) in levels.iter().zip(hashes).enumerate() {
            assert_eq!(fnv1a64(b), want, "{w}x{h} level {l} ({lw}x{lh})");
        }
        let small = SMALL[i].2;
        let got: Vec<&Vec<u8>> = levels.iter().filter(|(a, b, _)| a * b <= 4).map(|(_, _, b)| b).collect();
        assert_eq!(got.len(), small.len(), "{w}x{h}: the small levels");
        for (g, s) in got.iter().zip(small) {
            assert_eq!(&g[..], *s, "{w}x{h}: a small level's bytes");
        }
    }
}
