// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the lifetime of the D24S8 surface a depth texture is
//! sampled from in place (`gc_debug_set_depth_tpu(1)`, src/vivante/
//! depth_sample.rs; `ds_cache::depth_view(id, guard, kick)`; docs/notes/
//! gc-depth-sample-wire.md (a).2 `note_sampled` / `used_by`), and what the CPU
//! readers and the refusals do around it. Everything goes through the GL entry
//! points of glmark2's shadow (a GL_DEPTH_COMPONENT / GL_UNSIGNED_INT 2D
//! texture, one level, NEAREST, CLAMP_TO_EDGE, the only attachment of an FBO)
//! on the real Vivante back end over a stand-in OSAL of this file: host memory,
//! kicks that complete when something waits for them (or a later one), and a
//! record of every `gpu_free` with the fence that had completed at that moment.
//!
//! * `deleting_the_texture_and_fbo_keeps_the_surface_until_the_sampling_kick_completed`:
//!   the depth pass, the draw that samples the texture (mode 1: no CPU
//!   resolve), then glDeleteTextures and glDeleteFramebuffers while that draw
//!   is still in the open frame. The surface's memory is not freed until the
//!   kick holding the draw completed (a free while it ran would be the GPU
//!   reading freed memory), and it is freed (not leaked) by the time the
//!   back end is shut down.
//! * `a_cpu_writer_after_a_sampling_draw_still_resolves_the_exact_depth`: after
//!   a sampling draw the texels are stale; a CPU access of the texture
//!   (glTexSubImage2D, which settles the texels first) runs the CPU resolve,
//!   once, and the texels around the write are the rule of gc_depth_resolve.rs
//!   (`(word >> 8) / (2^24 - 1)`) for every word of the surface, the sampling
//!   draw having left the surface as it was.
//!   GLES has no glReadPixels of depth, and glBlitFramebuffer depth reads
//!   the plane, not the texture's texels; the texels are read here from the
//!   stand-in's allocation of the texture's R32F storage.
//! * `a_depth_that_is_only_a_clear_value_is_resolved_not_sampled_stale`: a
//!   depth clear with no draw holds a value, not a live plane
//!   (`depth_view` is None): the knob's answer is no, the draw keeps the CPU
//!   resolve, and the sampled texels are the clear value.
//! * `a_texture_larger_than_the_surface_is_refused_by_name`: `depth_sample::
//!   binding` against the real cache: a texture wider or taller than the live
//!   surface, and an id with no live plane, are -ENOTSUP, and a respecified
//!   (larger) texture has no live plane any more, so the answer is no and its
//!   draw takes the CPU path.
//! * `knob_off_resolves_once_a_frame`: mode 0, shadow and refract resolve
//!   once per warm frame (the count of gc_scene_kicks_depth_tpu.rs), and the
//!   knob's answer is no throughout.
//!
//! * `a_new_context_inherits_no_render_target_marks` (#[ignore]): a defect of
//!   the GL core, described at the test.
//!
//! What it cannot show: that the texture unit reads the surface (the board's
//! case 222), and the memory reuse of an evicted cache slot (`victim` skips a
//! surface a pending kick uses; ds_cache.rs unit tests).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_sample_lifetime
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
use yakogl::gl::framebuffer::{glBindFramebuffer, glDeleteFramebuffers};
use yakogl::gl::state::{glClear, glClearDepthf, glGetError};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;
use yakogl::vivante::backend::{gc_init, gc_shutdown};
use yakogl::vivante::depth_sample::{binding, gc_debug_set_depth_tpu, gc_depth_sampled};
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

/// What a case leaves behind: the frees (after the case's own, with shutdown's marked) and faults.
struct Left {
    frees: Vec<(u64, i64, bool)>,
}

/// Run `case` on the real Vivante back end and GL core over the stand-in,
/// the knob at `mode`; the back end is shut down afterwards and the frees returned.
fn rig(mode: i32, case: impl FnOnce()) -> Left {
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
    let (frees, faults) = plat(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a);
        }
        (core::mem::take(&mut p.frees), core::mem::take(&mut p.faults))
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    Left { frees }
}

use yakogl::gl::state::{glColorMask, glCullFace, glEnable, glViewport};

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
        self.unbind();
    }

    /// A depth clear alone: no draw, so the plane holds a value.
    fn clear_only(&self, value: f32) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fbo);
            glViewport(0, 0, self.w as GLsizei, self.h as GLsizei);
            glColorMask(0, 0, 0, 0);
            glClearDepthf(value);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        self.unbind();
    }

    fn unbind(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glColorMask(1, 1, 1, 1);
        }
    }

    /// `GroundRenderer::draw`: the depth texture sampled into the window.
    fn ground(&self) {
        bind(GL_TEXTURE0, self.tex);
        draw(self.ground_prog, self.quad, GL_TRIANGLE_STRIP, 4);
        bind(GL_TEXTURE0, 0);
    }

    /// A CPU write of texel (0, 0): `settle_texture` first, which resolves.
    fn write_one_texel(&self) {
        // SAFETY: plain GL calls; one texel of GL_UNSIGNED_INT depth.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, self.tex);
            let texel: u32 = 0;
            glTexSubImage2D(
                GL_TEXTURE_2D,
                0,
                0,
                0,
                1,
                1,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                &texel as *const u32 as *const c_void,
            );
            glBindTexture(GL_TEXTURE_2D, 0);
        }
    }

    fn delete(&self) {
        // SAFETY: plain GL calls with names this scene made.
        unsafe {
            glDeleteTextures(1, &self.tex);
            glDeleteFramebuffers(1, &self.fbo);
        }
    }
}

/// The ids of the depth textures that hold a live plane in the cache.
fn live_texture_ids() -> Vec<u32> {
    (0x4000_0001..0x4000_0400).filter(|&id| ds_cache::depth_view(id, 0, 0).is_some()).collect()
}

/// The only live depth texture's (id, allocation base, stride, rows, guard).
fn the_surface() -> (u32, u64, u32, u32, usize) {
    let ids = live_texture_ids();
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

/// The pixel words of the surface at `base`, as the stand-in GPU would have
/// rendered them: `word(x, y)` D24S8 for every texel of the surface's `rows`.
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

/* ---- case 1: delete before the sampling kick completed ---------------------------------- */

#[test]
fn deleting_the_texture_and_fbo_keeps_the_surface_until_the_sampling_kick_completed() {
    let mut watched: Option<(u64, i64)> = None;
    let left = rig(1, || {
        let s = shadow_scene(W, H);
        frame_start();
        s.depth_pass();
        let r0 = resolves();
        s.ground();
        assert_eq!(resolves(), r0, "mode 1: the sampling draw resolves nothing");
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
        let (id, base, _, _, _) = the_surface();
        assert!(gc_depth_sampled(id, W, H));
        /* the draw is in the open frame, whose kick is the next one handed out */
        let sampling_kick = plat(|p| p.fence) + 1;
        s.delete();
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
        assert!(plat(|p| p.frees.iter().all(|f| f.0 != base)), "the surface is freed with the draw still recorded");
        assert_eq!(yakogl::backend::flush(), 0);
        assert_eq!(plat(|p| (p.fence, p.done < sampling_kick)), (sampling_kick, true), "the kick is in flight");
        assert!(plat(|p| p.frees.iter().all(|f| f.0 != base)), "freed while the sampling kick runs");
        watched = Some((base, sampling_kick));
        /* the frames after: the kick completes (the swap waits for it), and the
         * surface may go from then on */
        for _ in 0..3 {
            frame_start();
            swap();
        }
        assert!(plat(|p| p.done >= sampling_kick), "the swaps completed the kick");
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
    });
    let (base, kick) = watched.unwrap();
    let freed: Vec<_> = left.frees.iter().filter(|f| f.0 == base).collect();
    assert_eq!(freed.len(), 1, "the surface is freed exactly once, not leaked: {freed:?}");
    let (_, done_at_free, in_shutdown) = *freed[0];
    assert!(
        in_shutdown || done_at_free >= kick,
        "the surface was freed with fence {done_at_free} complete, the sampling kick being {kick}"
    );
}

/* ---- case 2: a CPU reader after a sampling draw resolves the exact depth ----------------- */

#[test]
fn a_cpu_writer_after_a_sampling_draw_still_resolves_the_exact_depth() {
    rig(1, || {
        let s = shadow_scene(W, H);
        frame_start();
        s.depth_pass();
        let (_, base, stride, rows, guard) = the_surface();
        /* the stand-in GPU's render of the depth pass, in the surface's words */
        fill_surface(base, guard, stride, rows, pattern);
        let r0 = resolves();
        s.ground();
        assert_eq!(resolves(), r0, "mode 1: sampled in place, no resolve");
        /* the texture's storage, if any yet, has not been written by a resolve */
        let stale = storage_of(W, H);
        s.write_one_texel();
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "glTexSubImage2D");
        assert_eq!(resolves() - r0, 1, "the CPU access ran the resolve, once");
        let texels = storage_of(W, H).expect("the texture's storage");
        assert_ne!(stale.as_deref(), Some(&texels[..]), "the storage was written by the resolve");
        for y in 0..H {
            for x in 0..W {
                if (x, y) == (0, 0) {
                    continue; /* the glTexSubImage2D's own texel */
                }
                let got = texels[(y * W + x) as usize];
                let want = unorm24_depth(pattern(x, y) >> 8);
                assert_eq!(got.to_bits(), want.to_bits(), "texel ({x}, {y}): {got} vs {want}");
            }
        }
        frame_start();
        swap();
        s.delete(); /* see `a_new_context_inherits_no_render_target_marks` */
    });
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

/* ---- case 3: a value clear, and a texture larger than the surface ------------------------ */

#[test]
fn a_depth_that_is_only_a_clear_value_is_resolved_not_sampled_stale() {
    rig(1, || {
        let s = shadow_scene(W, H);
        frame_start();
        s.clear_only(0.25);
        assert!(live_texture_ids().is_empty(), "a clear alone leaves no live plane");
        let r0 = resolves();
        s.ground();
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "the draw is not refused");
        assert_eq!(resolves() - r0, 1, "answered no: the CPU resolve ran");
        let texels = storage_of(W, H).expect("the texture's storage");
        let want = unorm24_depth((0.25f64 * 16_777_215.0).round() as u32);
        assert!(
            texels.iter().all(|t| (*t - want).abs() <= 1e-6),
            "the clear value, not stale texels: {:?}",
            &texels[..4]
        );
        frame_start();
        swap();
        s.delete();
    });
}

fn depth_bind(id: u32, w: u32, h: u32) -> GlesTextureBind {
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
        ..GlesTextureBind::ZERO
    }
}

#[test]
fn a_texture_larger_than_the_surface_is_refused_by_name() {
    rig(1, || {
        let s = shadow_scene(W, H);
        frame_start();
        s.depth_pass();
        s.ground();
        let (id, _, stride, rows, _) = the_surface();
        let (sw, sh) = (stride / 4, rows);
        assert!(gc_depth_sampled(id, W, H) && gc_depth_sampled(id, sw, sh), "the whole surface fits");
        assert!(binding(&depth_bind(id, W, H), 0).is_ok());
        for (w, h) in [(sw + 1, sh), (sw, sh + 1), (W + 100, H + 100), (0, H)] {
            assert!(!gc_depth_sampled(id, w, h), "{w}x{h}");
            assert_eq!(binding(&depth_bind(id, w, h), 0).err(), Some(-ENOTSUP), "{w}x{h} is refused");
        }
        assert_eq!(binding(&depth_bind(id + 1, W, H), 0).err(), Some(-ENOTSUP), "an id with no live plane");
        assert_eq!(binding(&depth_bind(0, W, H), 0).err(), Some(-ENOTSUP), "no id");

        /* the texture specified again, larger: the surface's contents are gone;
         * no live plane is left to sample, the answer is no and the draw
         * takes the texels (a fresh, cleared storage), not the old surface */
        frame_start();
        swap();
        // SAFETY: plain GL calls; no pixels.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, s.tex);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_DEPTH_COMPONENT24 as GLint,
                (sw + 64) as GLsizei,
                (sh + 64) as GLsizei,
                0,
                GL_DEPTH_COMPONENT,
                GL_UNSIGNED_INT,
                core::ptr::null(),
            );
            glBindTexture(GL_TEXTURE_2D, 0);
        }
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
        assert!(live_texture_ids().is_empty(), "the old surface's plane is gone");
        assert!(!gc_depth_sampled(id, sw + 64, sh + 64));
        frame_start();
        s.ground();
        assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "the draw samples the new texels");
        swap();
        s.delete();
    });
}

/// Defect (src/gl, #397's): `gl_context_destroy` does not clear the render-into
/// marks (`framebuffer::TARGET_USE`, `DEPTH_TARGET`; only a texture's delete
/// does, `clear_target_mark`). A texture left undeleted by a destroyed context
/// leaves its slot marked as rendered into by a kick that no longer exists, so
/// the next context's texture in that slot looks like a render target: a
/// depth clear alone, sampled, finds a non-zero `texture_target_use`, skips
/// the CPU resolve that would give the clear value (0 resolves, not 1) and the
/// draw samples cleared-to-zero texels. Fix: `clear_target_mark` for every
/// slot of the context in `gl_context_destroy` (or at its create).
#[test]
#[ignore = "gl_context_destroy leaves framebuffer::TARGET_USE marks: the next context's texture in the slot looks rendered into"]
fn a_new_context_inherits_no_render_target_marks() {
    rig(1, || {
        let s = shadow_scene(W, H);
        frame_start();
        s.depth_pass();
        s.ground();
        s.write_one_texel(); /* the texels resolved: the texture's mark says so */
        swap();
        /* not deleted: the context goes with its texture's mark */
    });
    rig(1, || {
        let s = shadow_scene(W, H);
        frame_start();
        s.clear_only(0.25);
        let r0 = resolves();
        s.ground();
        assert_eq!(resolves() - r0, 1, "the clear value is resolved into the texels");
        s.delete();
    });
}

/* ---- case 4: the knob at 0 resolves once a frame ---------------------------------------- */

#[test]
fn knob_off_resolves_once_a_frame() {
    for (name, scene) in [("shadow", shadow as fn() -> Box<dyn FnMut()>), ("refract", refract)] {
        let (counts, _) = run_with(
            Gpu::Done,
            scene,
            Hooks {
                after_init: Some(Box::new(|| {
                    assert_eq!(gc_debug_set_depth_tpu(0), 0, "the knob starts at 0");
                })),
                ..Hooks::default()
            },
        );
        assert!(!counts.is_empty());
        for (n, c) in counts.iter().enumerate() {
            assert_eq!(c.resolves, 1, "{name} frame {}: one CPU depth resolve a frame: {counts:?}", FIRST_COUNTED + n);
        }
    }
    /* and the answer is no with it off, for any id */
    assert!(!gc_depth_sampled(0x4000_0001, W, H));
}
