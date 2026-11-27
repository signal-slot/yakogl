// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): dtpu knob mode 3 (`DEPTH_TPU_RSCOPY`,
//! src/vivante/depth_sample.rs), outcome (b).1 of
//! docs/notes/gc-depth-sample-wire.md: a draw that samples a rendered depth
//! texture records a resolve-engine copy of the row-linear D24S8 surface into
//! a SUPER_TILED buffer of `depth_copy::CopyPool` in its own stream, before the
//! draw, and samples that buffer as D24X8; no CPU resolve runs.
//!
//! The frame is glmark2 shadow's at a 1024x1024 depth texture, on the harness
//! of `gc_scene_kicks.rs` (`tests/common/gc_scenes.rs`), as
//! `gc_depth_sample_default.rs` drives it: a depth pass into an FBO whose only
//! attachment is the depth texture, then the ground pass that samples it. No
//! GPU runs, so the test writes the surface's D24S8 words itself between the
//! two passes. The stand-in OSAL's `submit` is wrapped to keep every stream's
//! RS jobs and draws, and a snapshot of each texture descriptor the stream
//! names, and each counted frame is checked:
//!
//! * no CPU depth resolve (the table's `depth_resolve` is not called);
//! * one RS job whose source is the surface and destination the pool buffer
//!   of the texture (`depth_sample::copy_live`), its registers at the kick
//!   the ones `copy_rs_writes` pins (tests/gc_depth_copy_rs.rs), and it comes
//!   before the draw that samples the buffer, in the same stream;
//! * that draw's slot-0 descriptor is `copy_sampler_words` of the buffer:
//!   D24X8, ADDRESSING TILED, HALIGN SUPER_TILED, the surface's stride, LOD 0
//!   at the buffer;
//! * the copy run through `rs_interp` from the stream's registers, then read
//!   through the descriptor's words as D24X8 (bits 31..8 over 2^24 - 1), is
//!   the reference depth of every texel, bit for bit;
//! * the buffer is stamped with the sampling draw's kick; after the last
//!   frame `gc_attachment_gone` retires it, and its memory is freed only once
//!   that kick completed (at once when it had, `Gpu::Done`).
//!
//! `a_copy_without_memory_falls_back_to_the_cpu_resolve`: with the buffer's
//! allocation refused, mode 3 answers no and the frame takes the CPU resolve
//! (one a frame) with no RS copy. What the hardware does with the copy is
//! the board's (the note's (b).1 "Board check").
//!
//! The knob is put back to its value before, even when a run panics. The
//! surface is addressed without a guard (a build without
//! `vivante-depth-guard`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_copy_knob
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use core::ffi::c_void;
use gc_scenes::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use yakogl::backend::{GlesTextureBind, YakoglBackend, GLES_TEXTYPE_2D};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{glBindFramebuffer, glDeleteFramebuffers, glFramebufferTexture2D};
use yakogl::gl::state::{glClear, glColorMask, glViewport};
use yakogl::gl::texture::glDeleteTextures;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_attachment_gone, gc_finish, gc_init, gc_kick_id, gc_kick_pending, gc_shutdown};
use yakogl::vivante::depth_sample::{
    copies_held, copy_live, copy_rs_writes, copy_sampler_words, gc_debug_set_depth_tpu, mode, CopySamplerWords,
    DepthView, DEPTH_TPU_RSCOPY,
};
use yakogl::vivante::ds_cache;
use yakogl::vivante::kick::GcKick;
use yakogl::vivante::layout::Layout;
use yakogl::vivante::regs::*;
use yakogl::vivante::rs_interp::{Memory, RsInterp};

const W: u32 = 1024;
const H: u32 = 1024;
const MAX_D: f64 = 16_777_215.0;

/// gpu address -> (cpu address, size) of every live allocation of the run.
static ALLOCS: Mutex<BTreeMap<u64, (usize, usize)>> = Mutex::new(BTreeMap::new());
/// The gpu addresses of every allocation freed.
static FREED: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());
/// Refuse an allocation of this many bytes (0: none).
static REFUSE_BYTES: Mutex<usize> = Mutex::new(0);
/// CPU depth resolves of the run.
static RESOLVES: Mutex<u32> = Mutex::new(0);
/// What the streams of the current frame held, in submit order.
static ITEMS: Mutex<Vec<Item>> = Mutex::new(Vec::new());
/// What the frame's sampling draw left: (id, surface, pool buffer, kick).
static SAMPLED: Mutex<Option<(u32, DepthView, u64, u32)>> = Mutex::new(None);
/// The scene's depth texture and FBO, deleted after the last frame.
static SCENE: Mutex<Option<(GLuint, GLuint)>> = Mutex::new(None);
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CHECKED: Mutex<usize> = Mutex::new(0);

/// One thing a submitted stream did.
#[derive(Clone, Debug)]
enum Item {
    /// The start of a stream (a kick).
    Kick,
    /// An RS job: every register's value at its RS_KICKER.
    Rs(BTreeMap<u32, u32>),
    /// A draw, with the slot-0 descriptor written for it (since the draw
    /// before), snapshotted at submit.
    Draw(Option<(u32, Vec<u32>)>),
}

fn problem(s: String) {
    PROBLEMS.lock().unwrap().push(s);
}

/// The live allocation holding `gpu` .. `gpu + n`, as host memory.
fn host(gpu: u64, n: usize) -> Option<&'static mut [u8]> {
    let allocs = ALLOCS.lock().unwrap();
    let (&base, &(cpu, size)) = allocs.range(..=gpu).next_back()?;
    let off = (gpu - base) as usize;
    (off + n <= size).then(|| {
        // SAFETY: host memory the stand-in OSAL keeps (leaked at the run's end), inside the allocation.
        unsafe { core::slice::from_raw_parts_mut((cpu as *mut u8).add(off), n) }
    })
}

unsafe extern "C" fn rec_gpu_alloc(heap_base: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    let refuse = *REFUSE_BYTES.lock().unwrap();
    if refuse != 0 && size == refuse {
        return -12;
    }
    let rc = unsafe { (OSAL.gpu_alloc.expect("the harness allocates"))(heap_base, size, flags, out) };
    if rc == 0 {
        let m = unsafe { *out };
        ALLOCS.lock().unwrap().insert(m.gpu, (m.cpu as usize, m.size));
    }
    rc
}

unsafe extern "C" fn rec_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    let gpu = unsafe { (*mem).gpu };
    if ALLOCS.lock().unwrap().remove(&gpu).is_some() {
        FREED.lock().unwrap().insert(gpu);
    }
    unsafe { (OSAL.gpu_free.expect("the harness frees"))(mem) }
}

unsafe extern "C" fn rec_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    *RESOLVES.lock().unwrap() += 1;
    let counted = unsafe { (*counted_table()).depth_resolve.expect("counted") };
    unsafe { counted(id, dst, w, h, pitch) }
}

/// Walk a stream (the `LOAD_STATE` / draw / stall encoding of `cmd.rs`, as
/// `gc_scenes.rs` walks it) into [`Item`]s.
fn walk(words: &[u32]) -> Vec<Item> {
    let mut out = vec![Item::Kick];
    let mut regs = BTreeMap::new();
    let mut slot0 = None;
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for (k, &v) in words.iter().skip(i + 1).take(count).enumerate() {
                    let r = base + 4 * k as u32;
                    regs.insert(r, v);
                    if r == GC_RS_KICKER {
                        out.push(Item::Rs(regs.clone()));
                    }
                    if r == nte_descriptor_addr(0) {
                        let desc = host(u64::from(v), GC_TEXDESC_BYTES)
                            .map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect::<Vec<u32>>());
                        slot0 = desc.map(|d| (v, d));
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(Item::Draw(slot0.take()));
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

unsafe extern "C" fn rec_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<GcKick>() {
        // SAFETY: the back end passes a live GcKick of that size.
        let k = unsafe { *(kick as *const GcKick) };
        match host(k.stream_gpu, k.stream_bytes as usize) {
            Some(b) => {
                let words: Vec<u32> = b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
                let items = walk(&words);
                ITEMS.lock().unwrap().extend(items);
            }
            None => problem(format!("a stream at {:#x} outside every allocation", k.stream_gpu)),
        }
    }
    unsafe { (OSAL.submit.expect("the harness submits"))(kick, len) }
}

/// The reference word of pixel (x, y): depth in bits 31..8, stencil in 7..0,
/// depth 0 and 2^24 - 1 at two corners.
fn word(x: u32, y: u32) -> u32 {
    let d = match (x, y) {
        (0, 0) => 0,
        (x, y) if (x, y) == (W - 1, H - 1) => 0xff_ffff,
        _ => (y * W + x).wrapping_mul(2_654_435_761).rotate_left(11) & 0xff_ffff,
    };
    d << 8 | ((x ^ y) & 0xff)
}

/// The id the depth texture's surface is kept under (bit 30: a texture).
fn find_view() -> Option<(u32, DepthView)> {
    (0x4000_0001u32..0x4000_0400)
        .find_map(|id| ds_cache::depth_view(id, 0, 0).map(|(gpu, stride, rows)| (id, DepthView { gpu, stride, rows })))
}

fn surface_words(v: &DepthView) -> &'static mut [u32] {
    let b = host(v.gpu, v.stride as usize * v.rows as usize).expect("the surface is an allocation of the run");
    // SAFETY: whole, aligned words of the stand-in's u64-backed allocation.
    unsafe { core::slice::from_raw_parts_mut(b.as_mut_ptr() as *mut u32, b.len() / 4) }
}

/// The texture binding the GL core makes for the depth texture (its size
/// and type: all `copy_sampler_words` reads).
fn texture() -> GlesTextureBind {
    GlesTextureBind { width: W, height: H, type_: GLES_TEXTYPE_2D, layers: 1, mip_levels: 1, ..GlesTextureBind::ZERO }
}

/// glmark2 shadow's frame: the depth pass, the test's words in the surface,
/// the ground pass that samples the texture.
fn shadow_frame() -> Box<dyn FnMut()> {
    let depth_tex = depth_texture(W, H);
    let fbo = framebuffer(None, Depth::Texture(depth_tex), W, H);
    *SCENE.lock().unwrap() = Some((depth_tex, fbo));
    let depth_prog = program(MODEL_VS, FLAT_FS, &[]);
    let ground_prog = program(QUAD_VS, GROUND_FS, &[("ShadowMap", 0)]);
    let horse = model_vbo();
    let ground = quad_vbo();
    Box::new(move || {
        step("depth pass");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth_tex, 0);
            glViewport(0, 0, W as GLsizei, H as GLsizei);
            glColorMask(0, 0, 0, 0);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        draw(depth_prog, horse, GL_TRIANGLES, 96);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glColorMask(1, 1, 1, 1);
        }
        let Some((id, view)) = find_view() else {
            return problem("no live depth surface after the depth pass".into());
        };
        assert!(view.covers(W, H), "the surface covers the texture");
        assert_eq!((view.stride, view.rows), (W * 4, H), "a 1024x1024 surface needs no padding");
        let words = surface_words(&view);
        let stride_w = (view.stride / 4) as usize;
        for y in 0..H {
            for x in 0..W {
                words[y as usize * stride_w + x as usize] = word(x, y);
            }
        }
        step("ground");
        bind(GL_TEXTURE0, depth_tex);
        draw(ground_prog, ground, GL_TRIANGLE_STRIP, 4);
        bind(GL_TEXTURE0, 0);
        /* the open kick holds the draw; the buffer is stamped with it */
        let kick = gc_kick_id();
        *SAMPLED.lock().unwrap() = copy_live(id).map(|(buf, _, read_by)| {
            if read_by != kick {
                problem(format!("the copy buffer is stamped with kick {read_by}, the draw is in {kick}"));
            }
            (id, view, buf, kick)
        });
        if find_view().is_none() {
            problem("the surface is gone after the sampling draw".into());
        }
    })
}

/// Delete the scene's depth texture and FBO: a context destroyed with them
/// alive leaves their render-into marks to the next run's context (the GL
/// core defect `gc_depth_sample_lifetime.rs`'s ignored
/// `a_new_context_inherits_no_render_target_marks` describes), which then
/// skips a depth-sample answer in a later frame.
fn delete_scene() {
    if let Some((tex, fbo)) = SCENE.lock().unwrap().take() {
        // SAFETY: plain GL calls on the scene's own objects.
        unsafe {
            glDeleteTextures(1, &tex);
            glDeleteFramebuffers(1, &fbo);
        }
    }
}

/// Read one RS register of a job, by name.
fn reg(job: &BTreeMap<u32, u32>, r: u32) -> u32 {
    job.get(&r).copied().unwrap_or_else(|| panic!("RS register {r:#x} never written"))
}

/// The checks of one frame whose streams are [`ITEMS`] (mode 3).
fn check_copy_frame(n: usize) {
    let items = core::mem::take(&mut *ITEMS.lock().unwrap());
    let Some((_, view, buf, _)) = *SAMPLED.lock().unwrap() else {
        return problem(format!("frame {n}: the sampling draw left no copy buffer"));
    };
    let samples_buf =
        |it: &Item| matches!(it, Item::Draw(Some((_, d))) if u64::from(d[texdesc_lod_addr(0) as usize / 4]) == buf);
    let is_copy = |it: &Item| {
        matches!(it, Item::Rs(j) if j.get(&rs_pipe_dest_addr(0)) == Some(&(buf as u32))
            && j.get(&rs_pipe_source_addr(0)) == Some(&(view.gpu as u32)))
    };
    let copies: Vec<usize> = items.iter().enumerate().filter(|(_, it)| is_copy(it)).map(|(i, _)| i).collect();
    let draws: Vec<usize> = items.iter().enumerate().filter(|(_, it)| samples_buf(it)).map(|(i, _)| i).collect();
    if copies.len() != 1 || draws.len() != 1 {
        return problem(format!(
            "frame {n}: {} RS copies into the buffer and {} draws sampling it",
            copies.len(),
            draws.len()
        ));
    }
    let (c, d) = (copies[0], draws[0]);
    if c > d || items[c..d].iter().any(|it| matches!(it, Item::Kick)) {
        return problem(format!(
            "frame {n}: the RS copy (item {c}) is not before the sampling draw (item {d}) in its stream"
        ));
    }

    /* the copy's registers at its kick are the pinned ones */
    let Item::Rs(job) = &items[c] else { unreachable!() };
    let want = copy_rs_writes(&view, buf).expect("the copy is possible");
    let kicker = want.iter().position(|(r, _)| *r == GC_RS_KICKER).expect("one kick");
    for &(r, v) in &want[..kicker] {
        if r != GC_GL_FLUSH_CACHE && reg(job, r) != v {
            return problem(format!("frame {n}: RS register {r:#x} is {:#x}, the copy's is {v:#x}", reg(job, r)));
        }
    }
    let a8 = rs_source_format(GC_RS_FORMAT_A8R8G8B8) | rs_dest_format(GC_RS_FORMAT_A8R8G8B8);
    assert_eq!(reg(job, GC_RS_CONFIG), a8 | GC_RS_DEST_TILED, "A8R8G8B8 both sides, DEST_TILED only");
    assert_eq!(
        reg(job, GC_RS_DEST_STRIDE),
        (view.stride << 2) | GC_RS_DEST_STRIDE_TILING | GC_RS_DEST_STRIDE_SUPER_TILED_NEW,
        "a super-tiled destination of the surface's stride"
    );

    /* the descriptor the sampling draw reads */
    let Item::Draw(Some((_, desc))) = &items[d] else { unreachable!() };
    let at = |r: u32| desc[r as usize / 4];
    let got = CopySamplerWords {
        config0: at(GC_TEXDESC_CONFIG0),
        config1: at(GC_TEXDESC_CONFIG1),
        config2: at(GC_TEXDESC_CONFIG2),
        linear_stride: at(GC_TEXDESC_LINEAR_STRIDE),
        slice: at(GC_TEXDESC_SLICE),
        size: at(GC_TEXDESC_SIZE),
        lod0: u64::from(at(texdesc_lod_addr(0))),
    };
    let want_words = copy_sampler_words(&texture(), &view, buf).expect("the words of the copy");
    if got != want_words {
        return problem(format!("frame {n}: descriptor {got:x?}, the copy's {want_words:x?}"));
    }
    assert_eq!(got.layout(), Some(Layout::SuperTiled), "the descriptor addresses a super-tiled level");

    /* the copy run from the stream's registers, then every texel read
     * through the descriptor as D24X8 */
    let writes: Vec<(u32, u32)> =
        want.iter().enumerate().map(|(i, &(r, v))| if i <= kicker { (r, reg(job, r)) } else { (r, v) }).collect();
    let bytes = (view.stride * view.rows) as usize;
    let lin = host(view.gpu, bytes).expect("the surface").to_vec();
    let mut mem = Memory::new();
    mem.map(view.gpu as u32, lin);
    mem.map(buf as u32, vec![0xcd; bytes]);
    let mut rs = RsInterp::new();
    rs.run(&writes, &mut mem);
    assert_eq!(rs.jobs.len(), 1, "one RS job");
    assert_eq!(rs.jobs[0].dst.layout, Layout::SuperTiled);
    let mut checked = 0;
    for y in 0..H {
        for x in 0..W {
            let a = got.texel_addr(x, y).unwrap();
            let t = u32::from_le_bytes(mem.read(a, 4).try_into().unwrap());
            let (depth, want_d) = ((t >> 8) as f64 / MAX_D, (word(x, y) >> 8) as f64 / MAX_D);
            if depth.to_bits() != want_d.to_bits() {
                return problem(format!("frame {n}: ({x},{y}) samples {depth}, reference {want_d}"));
            }
            checked += 1;
        }
    }
    *CHECKED.lock().unwrap() += checked;
}

/// The knob's value before the run set it.
static PREV_KNOB: Mutex<Option<i32>> = Mutex::new(None);

fn restore_knob() {
    if let Some(prev) = PREV_KNOB.lock().unwrap().take() {
        gc_debug_set_depth_tpu(prev);
    }
}

/// The knob and the statics above are process-wide: one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn reset() {
    ALLOCS.lock().unwrap().clear();
    FREED.lock().unwrap().clear();
    ITEMS.lock().unwrap().clear();
    PROBLEMS.lock().unwrap().clear();
    *RESOLVES.lock().unwrap() = 0;
    *SAMPLED.lock().unwrap() = None;
    *CHECKED.lock().unwrap() = 0;
}

/// The hook that sets mode 3 and installs the recording table and OSAL.
fn install() -> Box<dyn FnOnce()> {
    Box::new(|| {
        *PREV_KNOB.lock().unwrap() = Some(gc_debug_set_depth_tpu(DEPTH_TPU_RSCOPY));
        // SAFETY: SERIAL is held by the harness; the table is leaked, the OSAL copy too.
        unsafe {
            let mut t: YakoglBackend = core::ptr::read(counted_table());
            t.depth_resolve = Some(rec_depth_resolve);
            yakogl::backend::g_backend = Box::leak(Box::new(t));
            let mut o: OsalOps = core::ptr::read(&OSAL);
            o.gpu_alloc = Some(rec_gpu_alloc);
            o.gpu_free = Some(rec_gpu_free);
            o.submit = Some(rec_submit);
            osal_set_ops(Box::leak(Box::new(o)));
            /* the streams and the arenas gc_init made before this hook are
             * not in ALLOCS: made again under the recording OSAL (the
             * context holds no back-end memory yet) */
            gc_shutdown();
            assert_eq!(gc_init(), 0, "gc_init under the recording OSAL");
        }
        assert_eq!(mode(), DEPTH_TPU_RSCOPY, "the knob took");
    })
}

/// After the last frame: the buffer is retired by `gc_attachment_gone` and
/// its memory freed only once the sampling kick completed.
fn check_release() {
    let Some((id, _, buf, kick)) = *SAMPLED.lock().unwrap() else {
        return problem("no copy buffer to release".into());
    };
    let base = {
        let allocs = ALLOCS.lock().unwrap();
        match allocs.range(..=buf).next_back() {
            Some((&b, &(_, size))) if buf < b + size as u64 => b,
            _ => return problem(format!("the copy buffer {buf:#x} is no live allocation")),
        }
    };
    let pending = gc_kick_pending(kick);
    gc_attachment_gone(id);
    if copy_live(id).is_some() {
        problem("the buffer is still live after gc_attachment_gone".into());
    }
    let freed = FREED.lock().unwrap().contains(&base);
    match pending {
        true if freed || copies_held() != 1 => {
            problem(format!("freed while kick {kick} was pending ({} held)", copies_held()))
        }
        false if !freed || copies_held() != 0 => {
            problem(format!("kick {kick} had completed and the buffer was kept ({} held)", copies_held()))
        }
        _ => {}
    }
    /* every kick completes: the retired buffer goes */
    assert_eq!(gc_finish(), 0);
    if !FREED.lock().unwrap().contains(&base) || copies_held() != 0 {
        problem(format!("after gc_finish the buffer is not freed ({} held)", copies_held()));
    }
}

fn run_copy(gpu: Gpu) -> (Vec<Counts>, bool) {
    reset();
    let before = mode();
    let mut held_while_pending = false;
    let hooks = Hooks {
        after_init: Some(install()),
        each_frame: Some(Box::new(|_| ITEMS.lock().unwrap().clear())),
        after_frame: Some(Box::new(|n| {
            if n >= FIRST_COUNTED {
                check_copy_frame(n);
            }
            if n == FRAMES {
                if let Some((_, _, _, kick)) = *SAMPLED.lock().unwrap() {
                    held_while_pending = gc_kick_pending(kick);
                }
                check_release();
                delete_scene();
            }
        })),
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_with(gpu, shadow_frame, hooks)));
    restore_knob();
    assert_eq!(mode(), before, "the knob is put back");
    let (counts, _) = run.unwrap_or_else(|e| std::panic::resume_unwind(e));
    let problems = PROBLEMS.lock().unwrap().clone();
    assert!(problems.is_empty(), "{gpu:?}: {problems:?}");
    (counts, held_while_pending)
}

/// Mode 3 on glmark2 shadow's 1024x1024 depth texture: the RS copy before
/// the sampling draw, no CPU resolve, the descriptor at the super-tiled
/// buffer, the reference depth through its words, and the buffer kept until
/// its kick completed.
#[test]
fn mode_3_copies_the_depth_on_the_gpu_and_samples_the_copy() {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut saw_pending = false;
    for gpu in [Gpu::Lag, Gpu::Done] {
        let (counts, pending) = run_copy(gpu);
        saw_pending |= pending;
        let resolves: Vec<u32> = counts.iter().map(|c| c.resolves).collect();
        assert!(resolves.iter().all(|&r| r == 0), "{gpu:?}: CPU depth resolves per warm frame {resolves:?}");
        assert_eq!(*RESOLVES.lock().unwrap(), 0, "{gpu:?}: no CPU depth resolve in the whole run");
        let checked = *CHECKED.lock().unwrap();
        assert_eq!(
            checked,
            (W * H) as usize * (FRAMES - FIRST_COUNTED + 1),
            "{gpu:?}: every texel of every counted frame"
        );
        assert_eq!(copies_held(), 0, "{gpu:?}: no buffer left after shutdown");
    }
    assert!(saw_pending, "the lagging GPU left the sampling kick pending at the release check");
}

/// No memory for the copy: mode 3 answers no and the draw takes the CPU
/// resolve, one a frame, with no RS copy into a buffer.
#[test]
fn a_copy_without_memory_falls_back_to_the_cpu_resolve() {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset();
    /* CopyPool::buffer asks for copy_bytes plus the alignment room */
    *REFUSE_BYTES.lock().unwrap() = (W * 4 * H) as usize + 63;
    let before = mode();
    let hooks = Hooks {
        after_init: Some(install()),
        after_frame: Some(Box::new(|n| {
            let items = core::mem::take(&mut *ITEMS.lock().unwrap());
            if SAMPLED.lock().unwrap().is_some() {
                problem(format!("frame {n}: a copy buffer exists"));
            }
            if items
                .iter()
                .any(|it| matches!(it, Item::Rs(j) if j.get(&GC_RS_CONFIG).is_some_and(|c| c & GC_RS_DEST_TILED != 0)))
            {
                problem(format!("frame {n}: an RS job into a tiled destination"));
            }
            if n == FRAMES {
                delete_scene();
            }
        })),
        ..Hooks::default()
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_with(Gpu::Lag, shadow_frame, hooks)));
    *REFUSE_BYTES.lock().unwrap() = 0;
    restore_knob();
    assert_eq!(mode(), before, "the knob is put back");
    let (counts, _) = run.unwrap_or_else(|e| std::panic::resume_unwind(e));
    let problems = PROBLEMS.lock().unwrap().clone();
    assert!(problems.is_empty(), "{problems:?}");
    let resolves: Vec<u32> = counts.iter().map(|c| c.resolves).collect();
    assert!(resolves.iter().all(|&r| r == 1), "one CPU depth resolve per warm frame, got {resolves:?}");
    assert_eq!(copies_held(), 0);
}
