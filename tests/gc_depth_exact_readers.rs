// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the readers that need the exact 24-bit depth stay on
//! the CPU resolve when the dtpu knob (`gc_debug_set_depth_tpu`) is non-zero
//! (src/vivante/depth_sample.rs `exact_depth_reader`, `view_binding`;
//! docs/notes/gc-depth-sample-wire.md). glmark2's shadow scene (a depth
//! texture rendered into an FBO and sampled in the window) on the real
//! Vivante back end over a stand-in OSAL (the helpers of
//! gc_depth_sample_lifetime.rs, copied).
//!
//! * `mode_0_resolves_on_the_cpu_with_and_without_compare`: mode 0, a plain
//!   sample and one with GL_TEXTURE_COMPARE_MODE set each run the CPU resolve
//!   once and read the exact reference depth (`unorm24_depth` of the word).
//! * `mode_1_plain_sample_uses_the_depth_view`: mode 1, no compare mode: the
//!   draw is not refused, no CPU resolve runs, and the surface is the view's.
//! * `mode_1_compare_binding_is_refused_and_plain_is_not`: `binding` against
//!   the live cache: every compare function LESS..NOTEQUAL is -ENOTSUP in
//!   modes 1 and 2, NEVER, ALWAYS and no compare mode take the view.
//! * `mode_1_compare_draw_keeps_the_cpu_resolve` (#[ignore]): the draw itself.
//!   Missing change: src/gl (#397) must ask `samples_rendered_depth` with the
//!   binding's compare (and sampler object's) and call
//!   `exact_depth_reader`; until then the GL core leaves the texels stale and
//!   the draw is refused with -ENOTSUP.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_exact_readers
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use core::ffi::c_void;
use gc_scenes::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{GlesTextureBind, GLES_TEXFMT_R32F, GLES_WRAP_CLAMP_TO_EDGE};
use yakogl::ds_retention::unorm24_depth;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::glBindFramebuffer;
use yakogl::gl::state::{glClear, glColorMask, glCullFace, glEnable, glGetError, glViewport};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;
use yakogl::vivante::backend::{gc_init, gc_shutdown};
use yakogl::vivante::depth_sample::{binding, gc_debug_set_depth_tpu};
use yakogl::vivante::ds_cache;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL ---------------------------------------------------------------- */

#[derive(Default)]
struct Plat {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    /// The last fence handed out and the last one completed.
    fence: i64,
    done: i64,
    /// Every free: (gpu base, the completed fence at that moment, inside shutdown).
    frees: Vec<(u64, i64, bool)>,
    shutting_down: bool,
    faults: Vec<String>,
}

static PLAT: Mutex<Option<Plat>> = Mutex::new(None);

fn plat<R>(f: impl FnOnce(&mut Plat) -> R) -> R {
    let mut g = PLAT.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Plat::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    plat(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    plat(|p| {
        /* leaked, not dropped: a static of the library may still name the block */
        match p.allocs.remove(&m.gpu) {
            Some(b) => {
                core::mem::forget(b);
                let at = (m.gpu, p.done, p.shutting_down);
                p.frees.push(at);
            }
            None if !m.impl_.is_null() => p.faults.push(format!("gpu_free of 0x{:x}: no live allocation", m.gpu)),
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    plat(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    plat(|p| {
        if fence > p.fence {
            p.faults.push(format!("wait for fence {fence}, never handed out (last {})", p.fence));
        }
        p.done = p.done.max(fence.min(p.fence));
    });
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    plat(|p| p.done)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static STANDIN: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// The knob put back to 0, whatever the case did.
struct KnobOff;
impl Drop for KnobOff {
    fn drop(&mut self) {
        gc_debug_set_depth_tpu(0);
    }
}

/// Run `case` on the real Vivante back end and GL core over the stand-in,
/// the knob at `mode`; the back end is shut down afterwards.
fn rig(mode: i32, case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let _off = KnobOff;
    plat(|p| *p = Plat::default());
    // SAFETY: LOCK is held; the tables are statics or leaked.
    let ctx = unsafe {
        osal_set_ops(&STANDIN);
        yakogl::backend::g_backend = counted_table();
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(CANVAS_W, CANVAS_H);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    gc_debug_set_depth_tpu(mode);
    // SAFETY: plain GL calls.
    unsafe {
        glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
        glEnable(GL_DEPTH_TEST);
        glEnable(GL_CULL_FACE);
        glCullFace(GL_BACK);
    }
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    plat(|p| p.shutting_down = true);
    gc_shutdown();
    let faults = plat(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- the scene ------------------------------------------------------------------------ */

/// A size that no other allocation of the back end has, 64-aligned inside the
/// surface in neither axis: the surface is 128 x 128 words.
const W: u32 = 100;
const H: u32 = 70;

struct Shadow {
    tex: GLuint,
    fbo: GLuint,
    w: u32,
    h: u32,
    depth_prog: GLuint,
    ground_prog: GLuint,
    model: GLuint,
    quad: GLuint,
}

fn shadow_scene(w: u32, h: u32) -> Shadow {
    let tex = depth_texture(w, h);
    let fbo = framebuffer(None, Depth::Texture(tex), w, h);
    Shadow {
        tex,
        fbo,
        w,
        h,
        depth_prog: program(MODEL_VS, FLAT_FS, &[]),
        ground_prog: program(QUAD_VS, GROUND_FS, &[("ShadowMap", 0)]),
        model: model_vbo(),
        quad: quad_vbo(),
    }
}

impl Shadow {
    /// `DepthRenderTarget::enable`, the clear and the model's draw, `disable`.
    fn depth_pass(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fbo);
            glViewport(0, 0, self.w as GLsizei, self.h as GLsizei);
            glColorMask(0, 0, 0, 0);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        draw(self.depth_prog, self.model, GL_TRIANGLES, 96);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glColorMask(1, 1, 1, 1);
        }
    }

    /// GL_TEXTURE_COMPARE_MODE / FUNC of the texture (`None`: GL_NONE).
    fn compare(&self, func: Option<GLenum>) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, self.tex);
            match func {
                Some(f) => {
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_FUNC, f as GLint);
                }
                None => glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_COMPARE_MODE, GL_NONE as GLint),
            }
            glBindTexture(GL_TEXTURE_2D, 0);
        }
    }

    /// `GroundRenderer::draw`: the depth texture sampled into the window.
    fn ground(&self) {
        bind(GL_TEXTURE0, self.tex);
        draw(self.ground_prog, self.quad, GL_TRIANGLE_STRIP, 4);
        bind(GL_TEXTURE0, 0);
    }

    fn delete(&self) {
        // SAFETY: plain GL calls with names this scene made.
        unsafe {
            glDeleteTextures(1, &self.tex);
            yakogl::gl::framebuffer::glDeleteFramebuffers(1, &self.fbo);
        }
    }
}

/// The only live depth texture's (id, allocation base, stride, rows, guard).
fn the_surface() -> (u32, u64, u32, u32, usize) {
    let ids: Vec<u32> = (0x4000_0001..0x4000_0400).filter(|&id| ds_cache::depth_view(id, 0, 0).is_some()).collect();
    assert_eq!(ids.len(), 1, "one live depth surface: {ids:?}");
    let (gpu, stride, rows) = ds_cache::depth_view(ids[0], 0, 0).unwrap();
    let size = plat(|p| p.allocs.get(&gpu).map(|a| a.len() * 8)).expect("the surface's allocation");
    let guard = (size - stride as usize * rows as usize) / 2;
    (ids[0], gpu, stride, rows, guard)
}

fn frame_start() {
    canvas_clear();
}

fn resolves() -> u32 {
    snapshot().resolves
}

/// The words of the surface at `base` as the stand-in GPU would have rendered them.
fn fill_surface(base: u64, guard: usize, stride: u32, rows: u32, word: impl Fn(u32, u32) -> u32) {
    plat(|p| {
        let mem = p.allocs.get_mut(&base).expect("the surface's allocation");
        // SAFETY: the allocation holds guard + stride * rows + guard bytes; guard is word-aligned.
        let px = unsafe {
            core::slice::from_raw_parts_mut(
                (mem.as_mut_ptr() as *mut u8).add(guard) as *mut u32,
                (stride / 4 * rows) as usize,
            )
        };
        for y in 0..rows {
            for x in 0..stride / 4 {
                px[(y * (stride / 4) + x) as usize] = word(x, y);
            }
        }
    });
}

/// A distinct depth and a stencil in every word of the surface.
fn pattern(x: u32, y: u32) -> u32 {
    let d = (x.wrapping_mul(0x9e37).wrapping_add(y.wrapping_mul(0x1_0f3b)).wrapping_add(0x1234)) & 0xff_ffff;
    d << 8 | 0xa5
}

/// The floats of the one allocation of exactly `w` x `h` R32F texels, if any.
fn storage_of(w: u32, h: u32) -> Option<Vec<f32>> {
    plat(|p| {
        let mut of: Vec<_> = p.allocs.values().filter(|a| a.len() * 8 == (w * h * 4) as usize).collect();
        assert!(of.len() <= 1, "one allocation of the texture's size");
        of.pop().map(|a| {
            // SAFETY: a boxed [u64] read as f32.
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const f32, (w * h) as usize).to_vec() }
        })
    })
}

fn depth_bind(id: u32, w: u32, h: u32, compare_op: u8) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_R32F,
        width: w,
        height: h,
        stride_texels: w,
        bytes: w * h * 4,
        mip_levels: 1,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        no_alpha: 1,
        depth_luminance: 1,
        depth_id: id,
        compare_op,
        ..GlesTextureBind::ZERO
    }
}

/// The depth pass, the stand-in GPU's render of it, then the sampling draw
/// with the texture's compare set as `func`; returns (resolves the draw ran,
/// GL error, texels of the texture's storage).
fn sample(func: Option<GLenum>) -> (u32, GLenum, Option<Vec<f32>>) {
    let s = shadow_scene(W, H);
    frame_start();
    s.depth_pass();
    let (_, base, stride, rows, guard) = the_surface();
    fill_surface(base, guard, stride, rows, pattern);
    s.compare(func);
    let r0 = resolves();
    s.ground();
    // SAFETY: plain GL call.
    let err = unsafe { glGetError() };
    let ran = resolves() - r0;
    let texels = storage_of(W, H);
    frame_start();
    swap();
    s.delete();
    (ran, err, texels)
}

fn exact(texels: &Option<Vec<f32>>) {
    let texels = texels.as_ref().expect("the texture's storage");
    for y in 0..H {
        for x in 0..W {
            let got = texels[(y * W + x) as usize];
            let want = unorm24_depth(pattern(x, y) >> 8);
            assert_eq!(got.to_bits(), want.to_bits(), "texel ({x}, {y}): {got} vs {want}");
        }
    }
}

#[test]
fn mode_0_resolves_on_the_cpu_with_and_without_compare() {
    for func in [None, Some(GL_LESS), Some(GL_LEQUAL), Some(GL_NOTEQUAL)] {
        rig(0, || {
            let (ran, err, texels) = sample(func);
            assert_eq!(err, GL_NO_ERROR, "{func:?}");
            assert_eq!(ran, 1, "mode 0 {func:?}: the CPU resolve ran");
            exact(&texels);
        });
    }
}

#[test]
fn mode_1_plain_sample_uses_the_depth_view() {
    rig(1, || {
        let (ran, err, _) = sample(None);
        assert_eq!(err, GL_NO_ERROR, "the draw is not refused");
        assert_eq!(ran, 0, "mode 1, no compare mode: the view is sampled, no CPU resolve");
    });
}

#[test]
fn mode_1_compare_binding_is_refused_and_plain_is_not() {
    for mode in [1, 2] {
        rig(mode, || {
            let s = shadow_scene(W, H);
            frame_start();
            s.depth_pass();
            let (id, ..) = the_surface();
            // compare_op: function less NEVER (bit 0 less, 1 equal, 2 greater), 8 fixed point, 16 no compare
            for op in [1u8, 2, 3, 4, 5, 6] {
                assert_eq!(binding(&depth_bind(id, W, H, op | 8), 0).err(), Some(-ENOTSUP), "mode {mode} op {op}");
            }
            for op in [0u8, 7, 16, 16 | 8, 8] {
                assert!(binding(&depth_bind(id, W, H, op), 0).is_ok(), "mode {mode} op {op}");
            }
            frame_start();
            swap();
            s.delete();
        });
    }
}

#[test]
#[ignore = "src/gl (#397) must ask samples_rendered_depth with the binding's compare and call exact_depth_reader; today the compare draw is refused"]
fn mode_1_compare_draw_keeps_the_cpu_resolve() {
    for func in [GL_LESS, GL_LEQUAL, GL_NOTEQUAL] {
        rig(1, || {
            let (ran, err, texels) = sample(Some(func));
            assert_eq!(err, GL_NO_ERROR, "{func:#x}: the draw is not refused");
            assert_eq!(ran, 1, "{func:#x}: the CPU resolve ran");
            exact(&texels);
        });
    }
}
