// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): what glmark2's terrain and refract textures ask of the
//! render pool, in the order the GL core asks it (item pvr-pool-npot).
//!
//! The pool replay in the drivers crate (driver/rust/src/gpu/pvr/pool/
//! tests/replay.rs, `NPOT_800X600`, `NPOT_1600X1200`, the `Regrow` op) sizes
//! terrain's mipmapped 800x600 targets and refract's mipmapped 1600x1200
//! colour texture with the numbers pinned here. That crate cannot call the
//! GL core, so this file runs the real path -- `glTexImage2D` then
//! `glGenerateMipmap` on a texture with a mipmap filter, which is
//! glmark2's `BaseRenderer::create_texture` + `update_texture_parameters`
//! (scene-terrain/base-renderer.cpp) and `DistanceRenderTarget::setup` +
//! the per-frame `glGenerateMipmap` of scene-refract.cpp -- on a context
//! whose backend has the PowerVR's layout flags, and records every
//! `gpu_alloc` and `gpu_free` the core makes.
//!
//! The PowerVR's layout (src/gl/texture.rs `pot_chain_layout`, Mesa pvr's
//! rule): a non-power-of-two chain is stored as the chain of its
//! power-of-two extent, so an 800x600 RGBA8 chain is the 1024x1024 one,
//! 4 * (1024^2 + 512^2 + ... + 1) = 5,592,404 bytes, and a 1600x1200 one is
//! the 2048x2048 chain, 22,369,620 bytes. The layout is chosen by two
//! backend flags (`mip_level_align` 0, `native_layered_textures` false);
//! the stand-in here carries the PowerVR's, and a test compares them with
//! `yakogl_backend_pvr`'s so the numbers follow the real table.
//!
//! The order matters to the replay: `glTexImage2D` with no data allocates
//! level 0 alone (a NPOT texture has one level until its chain is asked
//! for); the first `glGenerateMipmap` lays the chain out
//! (`grow_npot_chain` -> `relayout_keeping_levels` -> `texture_alloc_laid_out`),
//! which releases level 0's storage FIRST (its texels are kept on the host
//! heap meanwhile) and then allocates the chain. So the pool sees
//! alloc(level 0), free(level 0), alloc(chain), never both at once.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_pool_npot
//! ```
//!
//! No draw is made, so no shader compiler is needed.

#![cfg(all(feature = "host-test", feature = "gpu-powervr"))]

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::glGetError;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the stand-in back end: the PowerVR's layout flags, nothing drawn ------ */

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

/// The PowerVR's flags (src/pvr/backend.rs `yakogl_backend_pvr`): packed mip
/// levels (`mip_level_align` 0), the TPU walks no layers itself
/// (`native_layered_textures` false). `the_stand_in_has_the_powervrs_layout_
/// flags` checks them against the real table.
static PVR_SHAPE: YakoglBackend = YakoglBackend {
    name: c"pvr pool npot stand-in".as_ptr() as *const c_char,
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
};

/* ---- the OSAL: host memory, every pool request recorded ------------------- */

/// One request of the GL core to the GPU pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Alloc(usize),
    Free(usize),
}

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

fn events() -> Vec<Event> {
    core::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Pointer-valued device addresses: `gpu` is the CPU address.
unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(Event::Alloc(size));
    0
}
unsafe extern "C" fn o_gpu_free(m: *mut OsalGpuMem) -> i32 {
    // SAFETY: the core passes its own live record.
    let size = unsafe { (*m).size };
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(Event::Free(size));
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    lock: None,
    unlock: None,
    thread_id: None,
};

static SERIAL: Mutex<()> = Mutex::new(());

/// A fresh context on the PowerVR shape, for the length of `case`; the
/// event log starts empty at `case` (the window's own buffers are not in it).
fn on_powervr(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &PVR_SHAPE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    events();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

/* ---- the sizes, from the layout rule --------------------------------------- */

const BPP: usize = 4;

/// The bytes of a w x h RGBA8 chain on the PowerVR: the chain of the
/// power-of-two extent down to 1x1 (Mesa pvr `pvr_image_plane_setup_mip_levels`:
/// "as if a full mip chain until 1x1x1 were present").
fn pot_chain_bytes(w: usize, h: usize) -> usize {
    let (mut pw, mut ph) = (w.next_power_of_two(), h.next_power_of_two());
    let mut bytes = 0;
    loop {
        bytes += pw * ph * BPP;
        if pw == 1 && ph == 1 {
            break;
        }
        pw = (pw / 2).max(1);
        ph = (ph / 2).max(1);
    }
    bytes
}

/// What the replay pins (replay.rs `NPOT_800X600`, `NPOT_1600X1200`).
const NPOT_800X600: usize = 5_592_404;
const NPOT_1600X1200: usize = 22_369_620;

/* ---- glmark2's calls ------------------------------------------------------- */

/// glmark2 `BaseRenderer::create_texture` + `update_texture_parameters` with
/// `setup_texture(GL_LINEAR_MIPMAP_LINEAR, GL_LINEAR, ...)` (terrain's
/// `terrain_renderer`, `tilt_h_renderer`, `specular_map_renderer`, the bloom
/// renderers): an RGBA texture of `w x h` with no data, the mipmap filter,
/// then `update_mipmap`'s `glGenerateMipmap`.
fn terrain_target(w: GLsizei, h: GLsizei) -> GLuint {
    let mut tex: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    no_error(&format!("terrain's {w}x{h} target"));
    tex
}

/* ---- the tests --------------------------------------------------------------- */

/// The stand-in's layout flags are the PowerVR table's: the chain layout the
/// core picks here (`pot_chain_layout`, the power-of-two extent's chain) is
/// the one the AM62P gets.
#[test]
fn the_stand_in_has_the_powervrs_layout_flags() {
    let pvr = &yakogl::pvr::backend::yakogl_backend_pvr;
    assert_eq!((PVR_SHAPE.mip_level_align, PVR_SHAPE.native_layered_textures), (0, false));
    assert_eq!(PVR_SHAPE.mip_level_align, pvr.mip_level_align, "mip_level_align");
    assert_eq!(PVR_SHAPE.native_layered_textures, pvr.native_layered_textures, "native_layered_textures");
}

/// The replay's constants follow from the rule.
#[test]
fn the_replays_chain_sizes_follow_from_the_power_of_two_extent() {
    assert_eq!(pot_chain_bytes(800, 600), NPOT_800X600, "the 1024x1024 chain");
    assert_eq!(pot_chain_bytes(1600, 1200), NPOT_1600X1200, "the 2048x2048 chain");
    assert_eq!(pot_chain_bytes(512, 512), 1_398_100);
    assert_eq!(pot_chain_bytes(256, 256), 349_524);
}

/// Terrain's mipmapped 800x600 target (`terrain_renderer`, `tilt_h_renderer`
/// at glmark2's default options): the pool is asked for level 0 (800 x 600
/// x 4 = 1,920,000 bytes), then that storage is freed and the 1024x1024
/// chain (5,592,404 bytes) is allocated, in that order. Before pvr-npot-mips
/// the texture stayed at level 0.
#[test]
fn terrains_800x600_target_frees_level_0_then_takes_the_chain() {
    on_powervr(|| {
        terrain_target(800, 600);
        assert_eq!(
            events(),
            [Event::Alloc(800 * 600 * BPP), Event::Free(800 * 600 * BPP), Event::Alloc(NPOT_800X600)],
            "level 0, then the chain in its place"
        );
        /* the next frame's update_mipmap filters in place: nothing asked */
        // SAFETY: a plain GL call.
        unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
        no_error("the second glGenerateMipmap");
        assert_eq!(events(), [], "the chain is kept across frames");
    });
}

/// Terrain's power-of-two targets ask for their whole chain at
/// `glTexImage2D`, as they always did: a mipmapped 512x512 (`specular_map_
/// renderer`) is 1,398,100 bytes, a 256x256 (the bloom renderers) 349,524,
/// and `glGenerateMipmap` asks for nothing more.
#[test]
fn terrains_power_of_two_targets_are_their_chain_from_the_start() {
    on_powervr(|| {
        terrain_target(512, 512);
        assert_eq!(events(), [Event::Alloc(1_398_100)], "512x512");
        terrain_target(256, 256);
        assert_eq!(events(), [Event::Alloc(349_524)], "256x256");
    });
}

/// Refract's `DistanceRenderTarget::setup`: a 1600x1200 depth texture
/// (NEAREST, one level: 7,680,000 bytes) and a 1600x1200 RGBA colour texture
/// with LINEAR_MIPMAP_LINEAR (level 0: 7,680,000 bytes). The frame's
/// `glGenerateMipmap` of the colour texture (scene-refract.cpp, after the
/// depth pass) frees level 0 and takes the 2048x2048 chain, 22,369,620
/// bytes: 2.9 times level 0, 21,846 KiB in one block. The depth texture is
/// never mipmapped and keeps its one level.
#[test]
fn refracts_colour_texture_frees_level_0_then_takes_the_chain_at_its_first_frame() {
    on_powervr(|| {
        let mut tex: [GLuint; 2] = [0; 2];
        let level0 = 1600 * 1200 * BPP;
        // SAFETY: two names; plain GL calls in scene-refract.cpp's order.
        unsafe {
            glGenTextures(2, tex.as_mut_ptr());
            glBindTexture(GL_TEXTURE_2D, tex[0]);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT as GLint,
                1600,
                1200,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                core::ptr::null(),
            );
        }
        no_error("the depth texture");
        assert_eq!(events(), [Event::Alloc(level0)], "the depth texture: one level");
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex[1]);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                1600,
                1200,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            glBindTexture(GL_TEXTURE_2D, 0);
        }
        no_error("the colour texture");
        assert_eq!(events(), [Event::Alloc(level0)], "the colour texture: level 0 at set-up");
        /* the first frame, after the depth pass */
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex[1]);
            glGenerateMipmap(GL_TEXTURE_2D);
            glBindTexture(GL_TEXTURE_2D, 0);
        }
        no_error("glGenerateMipmap of the colour texture");
        assert_eq!(
            events(),
            [Event::Free(level0), Event::Alloc(NPOT_1600X1200)],
            "level 0 freed, then the 2048x2048 chain"
        );
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex[1]);
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        no_error("the second frame's glGenerateMipmap");
        assert_eq!(events(), [], "the chain is kept across frames");
    });
}
