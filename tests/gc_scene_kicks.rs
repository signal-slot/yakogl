// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): ceilings on what one warm frame of glmark2's three slow
//! scenes, terrain, shadow and refract, costs the CPU in the Vivante back end
//! (#1204, #1106, plan #328): kicks, CPU waits for the GPU outside the swap,
//! `finish()` calls outside the swap, CPU tiled copies and CPU depth
//! resolves. Each ceiling is the count at the commit that wrote this file;
//! a change that raises one fails here. A fix that lowers one lowers its
//! ceiling in the same commit. The comment at each ceiling names the planned
//! fix that lowers it, and to what.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL, as `gc_glmark2_mip_offer.rs` does, driven with the GL calls
//! glmark2 (`22c527cb`, `$GLMARK2_SRC`) makes in a frame: `canvas_.clear()`,
//! the scene's `draw()`, then the swap. The models are small and the shaders
//! short (the counts do not depend on either), but every target, attachment,
//! clear, draw, texture binding by stage, blend and `glGenerateMipmap` of a
//! frame is glmark2's, in its order:
//!
//! * terrain: `scene-terrain.cpp:332` (the height/normal chain, then the
//!   terrain chain: terrain, bloom_h, bloom_v, overlay, tilt_h, tilt_v),
//!   `scene-terrain/base-renderer.cpp`, `texture-renderer.cpp:53`,
//!   `terrain-renderer.cpp:43`, `overlay-renderer.cpp:72`;
//!   `docs/notes/gc-slow-terrain.md`, "The 52 kicks".
//! * shadow: `scene-shadow.cpp:404` (`DepthRenderTarget::enable`, the horse,
//!   `disable`, the ground sampling the depth texture, the lit horse);
//!   `docs/notes/gc-slow-shadow-refract.md`, "shadow".
//! * refract: `scene-refract.cpp:394` (`DistanceRenderTarget` 1600x1200
//!   colour and depth, the bunny culled front, `glGenerateMipmap` of the
//!   colour, the bunny into the window with the depth, colour and image
//!   textures); the same note, "refract".
//!
//! The swap is the board's (`src/gl/egl.rs:2396-2413`, a double-buffered
//! window): `flush`, then `finish_previous`. The host harness's window has no
//! present, so its swap is a `finish()`; that difference is the harness's.
//!
//! What is counted, a frame, with what:
//! * kicks: the stand-in's `submit` calls (`frame_submit`);
//! * waits outside the swap: the stand-in's `wait` calls made outside the
//!   swap (`frame_open` on a slot still pending, `finish`, `wait_kick`);
//! * `finish()` calls outside the swap: the back end's `gc_finishes`;
//! * CPU tiled copies: `cache_invd` calls outside the depth resolve: the
//!   CPU tiled copy's read of the rendered map (`tiled_copy.rs` `refresh`)
//!   is the only CPU read of GPU memory in these frames outside the resolve
//!   (the notes' catch-all over `cache_invd`);
//! * CPU depth resolves: calls of the back end table's `depth_resolve`, the
//!   one entry wrapped by this test (its copy of `yakogl_backend_vivante`).
//!
//! Two GPU models, as the harness's GC cost mode has
//! (`docs/notes/gc-slow-host-counts.md`, "Run"): `Lag`, where a kick
//! completes only when the CPU waits for it or for a later one (every check
//! of a pending kick is a wait: the most the board pays), and `Done`, where
//! every kick completes when submitted. Waits are read from `Lag`. The RS
//! path of the tiled copy depends on the model (the same note, "After
//! tiled-copy-rs"), so the tiled copies have a ceiling in each.
//!
//! ```text
//! The library reports ES 2.0, as glmark2's board images
//! (`CONFIG_YAKOGL_REPORT_ES2`, default y) and the host harness
//! (`-DYAKOGL_REPORT_ES2=ON`) build it: under an ES 3.0 report shadow's and
//! refract's unsized `GL_DEPTH_COMPONENT` textures are no attachment
//! (`GlTexture::unsized_depth`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_scene_kicks -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::YakoglBackend;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glRenderbufferStorage,
};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glGetUniformLocation, glLinkProgram,
    glUniform1i, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{
    glBlendFunc, glClear, glClearColor, glClearDepthf, glColorMask, glCullFace, glDepthMask, glDisable, glEnable,
    glGetError, glPixelStorei, glViewport,
};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_finishes, gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the ceilings ------------------------------------------------------------------- */

/// One frame's counts (see the file's comment for what each one is).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    kicks: u32,
    waits_other: u32,
    finishes_other: u32,
    tiled_copies: u32,
    resolves: u32,
    /// Not a ceiling: the swap's own waits, printed for the record.
    waits_swap: u32,
}

impl Counts {
    /// `self` field by field at most `ceil` (the swap's waits are not held).
    fn within(&self, ceil: &Counts) -> Vec<&'static str> {
        let mut over = Vec::new();
        let rows = [
            ("kicks", self.kicks, ceil.kicks),
            ("waits outside the swap", self.waits_other, ceil.waits_other),
            ("finish() outside the swap", self.finishes_other, ceil.finishes_other),
            ("CPU tiled copies", self.tiled_copies, ceil.tiled_copies),
            ("CPU depth resolves", self.resolves, ceil.resolves),
        ];
        for (name, got, max) in rows {
            if got > max {
                over.push(name);
            }
        }
        over
    }

    /// The largest of each count over `frames`.
    fn max_of(frames: &[Counts]) -> Counts {
        frames.iter().fold(Counts::default(), |m, f| Counts {
            kicks: m.kicks.max(f.kicks),
            waits_other: m.waits_other.max(f.waits_other),
            finishes_other: m.finishes_other.max(f.finishes_other),
            tiled_copies: m.tiled_copies.max(f.tiled_copies),
            resolves: m.resolves.max(f.resolves),
            waits_swap: m.waits_swap.max(f.waits_swap),
        })
    }
}

/// terrain, `Lag`. Kicks 14: the canvas clear, the height map, the normal
/// map, the terrain pass, bloom_h, bloom_v, the overlay, tilt_h and tilt_v
/// (9 passes: every render-target change ends the kick), and the 5 GPU
/// mipmap chains, one kick each (`mipmap.rs`: the frame open is submitted
/// first, then the chain's levels share one stream). mip-one-kick took the
/// chains from 43 one-level kicks to 5 (52 -> 14; gc-slow-host-counts.md
/// predicted about 9, from the stream diff's count, which puts each chain in
/// its pass's kick). gc-frames-ring (gc-kick-continue.md C1-C3: a target
/// change does not end the kick) merges the passes between two chains: the
/// canvas clear, height, normal and terrain passes become one kick, 14 -> 11.
/// C5 (the chain stays in the open kick) then takes it to 1, submitted at
/// the swap.
/// Waits outside the swap 14: every `frame_open` (one a kick) reuses one of
/// the 4 stream slots (`GC_FRAMES`) whose kick the lag model still holds.
/// They fall with the kicks: at 2 kicks a frame or fewer the slot reused is
/// the previous frame's, waited for at the swap, so gc-frames-ring with C5
/// takes them to 0 (the harness's lag count was 48.2 before mip-one-kick).
/// No `finish()` and no CPU tiled copy: tiled-copy-rs put the RS copy of the
/// two vertex-stage maps in the stream (finish 1 -> 0, copies 2 -> 0). No
/// depth resolve: terrain's depth is a renderbuffer, never sampled.
const TERRAIN_LAG: Counts =
    Counts { kicks: 14, waits_other: 14, finishes_other: 0, tiled_copies: 0, resolves: 0, waits_swap: 0 };
/// terrain, `Done`: the same kicks, no waits. One of the two maps takes the
/// CPU tiled copy: its producer kick has completed, so `tiled_copy.rs`
/// `refresh` finds nothing pending to tile behind and copies on the CPU
/// (gc-slow-host-counts.md, "After tiled-copy-rs"; the board lies between
/// the two models). gc-frames-ring keeps both maps' producers in the kick
/// being recorded when the terrain draw reads them, which takes it to 0.
const TERRAIN_DONE: Counts =
    Counts { kicks: 14, waits_other: 0, finishes_other: 0, tiled_copies: 1, resolves: 0, waits_swap: 0 };

/// shadow, `Lag`. Kicks 3: the canvas clear (K1), the 1600x1200 depth pass
/// (K2, submitted when the ground's draw resolves the depth it samples), the
/// window's ground and lit horse (K3, at the swap). gc-frames-ring takes them
/// to 2 (K1 and K2 share a stream); depth-sample-gc with C9 to 1.
/// Waits outside the swap 1: the depth resolve's `wait_kick` for K2
/// (`texture.rs` `resolve_depth`; resolve-own-kick made it that kick alone,
/// where it was a `finish()` of every slot: finish 1 -> 0; the harness's
/// lag count was 1.7).
/// depth-sample-gc (the texture unit samples the D24S8 surface, no CPU
/// resolve) takes it to 0.
/// One CPU depth resolve (1600x1200, 7.6 MB read): depth-sample-gc takes it
/// to 0; resolve-wide makes it cheaper, not fewer.
const SHADOW_LAG: Counts =
    Counts { kicks: 3, waits_other: 1, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const SHADOW_DONE: Counts =
    Counts { kicks: 3, waits_other: 0, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };

/// refract, `Lag`. Kicks 4: the canvas clear (K1), the 1600x1200 pass (K2),
/// its 11-level GPU chain in one kick of its own (mip-one-kick: 10 one-level
/// kicks, 13 -> 4; gc-slow-host-counts.md predicted about 3), the window's
/// bunny (at the swap). gc-frames-ring takes them to 3 (K1 shares K2's
/// stream), C5 to 2 (the chain stays in that stream), depth-sample-gc with C9
/// to 1.
/// Waits outside the swap 2: the chain's `frame_open` on a slot the lag model
/// still holds (mip-one-kick took the chain's from 8 to 1; 0 at 2 kicks a
/// frame, gc-frames-ring with C5) and the depth resolve's `wait_kick` for K2
/// (depth-sample-gc takes it to 0). No `finish()` (resolve-own-kick, as in
/// shadow). One CPU depth resolve: depth-sample-gc takes it to 0.
const REFRACT_LAG: Counts =
    Counts { kicks: 4, waits_other: 2, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };
const REFRACT_DONE: Counts =
    Counts { kicks: 4, waits_other: 0, finishes_other: 0, tiled_copies: 0, resolves: 1, waits_swap: 0 };

/// Frames drawn, and the first one counted: frame 1 allocates the targets'
/// surfaces and copies, frame 2 is the first whose swap has a previous frame.
const FRAMES: usize = 6;
const FIRST_COUNTED: usize = 3;

/* ---- the stand-in OSAL: host memory, kicks, waits and CPU reads counted -------------- */

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gpu {
    /// A kick completes when the CPU waits for it or for a later one.
    #[default]
    Lag,
    /// Every kick completes when it is submitted.
    Done,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    gpu: Gpu,
    /// The last fence handed out, and the last one completed (`Lag`).
    fence: i64,
    done: i64,
    in_swap: bool,
    in_resolve: bool,
    kicks: u32,
    waits_other: u32,
    waits_swap: u32,
    cpu_reads: u32,
    resolves: u32,
    /// The test's step now, and what the stand-in saw in each one: `kick`
    /// and `wait` (outside the swap) by step, for the record.
    step: &'static str,
    events: Vec<String>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
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
    with_platform(|p| {
        /* leaked, not dropped: a static of the library may still name the block */
        match p.allocs.remove(&m.gpu) {
            Some(b) => core::mem::forget(b),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache_flush(_cpu: *mut c_void, _n: usize) {}

/// A CPU read of GPU memory: counted when it is not the depth resolve's.
unsafe extern "C" fn p_cache_invd(_cpu: *mut c_void, _n: usize) {
    with_platform(|p| {
        if !p.in_resolve {
            p.cpu_reads += 1;
        }
    });
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        p.kicks += 1;
        p.fence += 1;
        let e = format!("kick@{}", p.step);
        p.events.push(e);
        if p.gpu == Gpu::Done {
            p.done = p.fence;
        }
        p.fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        if fence > p.fence {
            p.faults.push(format!("wait for fence {fence}, which was never handed out (last {})", p.fence));
        }
        if p.in_swap {
            p.waits_swap += 1;
        } else {
            p.waits_other += 1;
            let e = format!("wait@{}", p.step);
            p.events.push(e);
        }
        p.done = p.done.max(fence.min(p.fence));
    });
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.done)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache_flush),
    cache_invd: Some(p_cache_invd),
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

/* ---- the back end's table, its depth resolve counted ---------------------------------- */

/// `yakogl_backend_vivante`'s own depth resolve, counted, with the CPU reads
/// it makes kept apart from the tiled copies'.
unsafe extern "C" fn counted_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    with_platform(|p| {
        p.resolves += 1;
        p.in_resolve = true;
    });
    let resolve = yakogl_backend_vivante.depth_resolve.expect("the GC7000 back end resolves depth");
    // SAFETY: the GL core's arguments, handed on as they came.
    let rc = unsafe { resolve(id, dst, w, h, pitch) };
    with_platform(|p| p.in_resolve = false);
    rc
}

/// A copy of the Vivante table with [`counted_depth_resolve`] in it, made
/// once and kept for the process.
fn counted_table() -> *const YakoglBackend {
    static TABLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *TABLE.get_or_init(|| {
        // SAFETY: the table is plain data (pointers, function pointers and
        // integers, no Drop); the copy is a second table of the same entries.
        let mut t: YakoglBackend = unsafe { core::ptr::read(&yakogl_backend_vivante) };
        t.depth_resolve = Some(counted_depth_resolve);
        Box::leak(Box::new(t)) as *const YakoglBackend as usize
    }) as *const YakoglBackend
}

/* ---- the rig -------------------------------------------------------------------------- */

const CANVAS_W: u32 = 800;
const CANVAS_H: u32 = 600;

/// Run `scene` (its setup, then `frame` FRAMES times with the canvas clear
/// before and the swap after) on the real back end under `gpu`, and return
/// the counts of every frame from FIRST_COUNTED on, and the last frame's
/// kicks and waits by step.
fn run(gpu: Gpu, scene: impl FnOnce() -> Box<dyn FnMut()>) -> (Vec<Counts>, Vec<String>) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| *p = Platform { gpu, ..Platform::default() });
    // SAFETY: SERIAL is held; the tables are statics or leaked.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = counted_table();
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(CANVAS_W, CANVAS_H);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        step("setup");
        // SAFETY: plain GL calls (glmark2's Scene::setup state).
        unsafe {
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glEnable(GL_DEPTH_TEST);
            glEnable(GL_CULL_FACE);
            glCullFace(GL_BACK);
        }
        let mut frame = scene();
        no_error("the scene's setup");
        let mut counted = Vec::new();
        for n in 1..=FRAMES {
            let before = snapshot();
            with_platform(|p| p.events.clear());
            canvas_clear();
            frame();
            swap();
            no_error(&format!("frame {n}"));
            let after = snapshot();
            if n >= FIRST_COUNTED {
                counted.push(Counts {
                    kicks: after.kicks - before.kicks,
                    waits_other: after.waits_other - before.waits_other,
                    finishes_other: after.finishes_other - before.finishes_other,
                    tiled_copies: after.tiled_copies - before.tiled_copies,
                    resolves: after.resolves - before.resolves,
                    waits_swap: after.waits_swap - before.waits_swap,
                });
            }
        }
        (counted, with_platform(|p| core::mem::take(&mut p.events)))
    }));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a);
        }
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

/// The running totals (`finishes_other` is the back end's `finish()` count:
/// the board's swap takes `finish_previous`, which is not one).
fn snapshot() -> Counts {
    let finishes = gc_finishes();
    with_platform(|p| Counts {
        kicks: p.kicks,
        waits_other: p.waits_other,
        finishes_other: finishes,
        tiled_copies: p.cpu_reads,
        resolves: p.resolves,
        waits_swap: p.waits_swap,
    })
}

/// `CanvasGeneric::clear` (canvas-generic.cpp:95) on the window.
fn canvas_clear() {
    step("canvas clear");
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
}

/// The board's swap of a double-buffered window (egl.rs:2396-2413).
fn swap() {
    step("swap");
    with_platform(|p| p.in_swap = true);
    assert_eq!(yakogl::backend::flush(), 0, "the swap's flush");
    assert_eq!(yakogl::backend::finish_previous(), 0, "the swap's finish_previous");
    with_platform(|p| p.in_swap = false);
}

/// The step of the frame the GL calls from here on belong to.
fn step(name: &'static str) {
    with_platform(|p| p.step = name);
}

/// Check `frames` against `ceil` and print them, and the last frame's kicks
/// and waits by step, for the record.
fn check(scene: &str, gpu: Gpu, (frames, events): &(Vec<Counts>, Vec<String>), ceil: &Counts) {
    let max = Counts::max_of(frames);
    eprintln!("{scene} {gpu:?}: a frame at most {max:?}; frames {FIRST_COUNTED}..={FRAMES}: {frames:?}");
    eprintln!("{scene} {gpu:?}: frame {FRAMES}: {}", events.join(", "));
    let over = max.within(ceil);
    assert!(over.is_empty(), "{scene} ({gpu:?}): {over:?} above the ceiling: a frame {max:?}, ceiling {ceil:?}");
}

/* ---- GL plumbing ---------------------------------------------------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// A linked program, attribute 0 `position`, each sampler named in
/// `samplers` on its texture unit.
fn program(vs: &str, fs: &str, samplers: &[(&str, GLint)]) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"position".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {vs:?} {fs:?}");
        glUseProgram(p);
        for (name, unit) in samplers {
            let name = CString::new(*name).unwrap();
            let at = glGetUniformLocation(p, name.as_ptr());
            assert!(at >= 0, "sampler {name:?}");
            glUniform1i(at, *unit);
        }
        glUseProgram(0);
        no_error("the program");
        p
    }
}

const QUAD_VS: &str = "attribute vec2 position;\nvarying vec2 v;\n\
    void main() { v = position * 0.5 + 0.5; gl_Position = vec4(position, 0.0, 1.0); }\n";
const FLAT_FS: &str = "precision mediump float;\nvarying vec2 v;\nvoid main() { gl_FragColor = vec4(v, 0.5, 1.0); }\n";
const TEX_FS: &str = "precision mediump float;\nuniform sampler2D t;\nvarying vec2 v;\n\
    void main() { gl_FragColor = texture2D(t, v); }\n";
/// A blur: taps of the input on each side (`terrain-blur.frag`).
const BLUR_FS: &str = "precision mediump float;\nuniform sampler2D t;\nvarying vec2 v;\n\
    void main() { gl_FragColor = 0.25 * (texture2D(t, v - vec2(0.01, 0.0)) + 2.0 * texture2D(t, v)\n\
    + texture2D(t, v + vec2(0.01, 0.0))); }\n";
/// `terrain.vert`: the displacement (height) and normal maps sampled in the
/// vertex stage.
const TERRAIN_VS: &str = "attribute vec2 position;\nuniform sampler2D tDisplacement;\nuniform sampler2D tNormal;\n\
    varying vec2 v;\nvarying vec3 n;\n\
    void main() { v = position * 0.5 + 0.5; float h = texture2D(tDisplacement, v).x;\n\
    n = texture2D(tNormal, v).xyz; gl_Position = vec4(position, h * 0.5, 1.0); }\n";
/// `terrain.frag`: detail, both diffuse maps, the displacement and specular maps.
const TERRAIN_FS: &str = "precision mediump float;\nuniform sampler2D tDiffuse1;\nuniform sampler2D tDiffuse2;\n\
    uniform sampler2D tDetail;\nuniform sampler2D tSpecular;\nuniform sampler2D tDisplacement;\n\
    varying vec2 v;\nvarying vec3 n;\n\
    void main() { vec4 d = mix(texture2D(tDiffuse1, v), texture2D(tDiffuse2, v), texture2D(tDisplacement, v).x);\n\
    gl_FragColor = d * texture2D(tDetail, v) + vec4(texture2D(tSpecular, v).xyz * n, 0.0); }\n";
/// A model drawn with a transform (the horse's and the bunny's programs).
const MODEL_VS: &str = "attribute vec2 position;\nvarying vec2 v;\n\
    void main() { v = position * 0.5 + 0.5; gl_Position = vec4(position * 0.8, 0.25, 1.0); }\n";
/// shadow's ground: the depth texture sampled (`shadow.frag`).
const GROUND_FS: &str = "precision mediump float;\nuniform sampler2D ShadowMap;\nvarying vec2 v;\n\
    void main() { float d = texture2D(ShadowMap, v).x; gl_FragColor = vec4(vec3(d < 0.5 ? 0.3 : 1.0), 1.0); }\n";
/// refract's bunny in the window: the distance (depth), the mipmapped colour
/// and the image (`refract.frag`).
const REFRACT_FS: &str = "precision mediump float;\nuniform sampler2D DistanceMap;\nuniform sampler2D NormalMap;\n\
    uniform sampler2D ImageMap;\nvarying vec2 v;\n\
    void main() { float d = texture2D(DistanceMap, v).x;\n\
    gl_FragColor = mix(texture2D(NormalMap, v * d), texture2D(ImageMap, v), 0.5); }\n";

/// A full-viewport quad, glmark2's `Mesh` quad in a VBO (`render_vbo`).
fn quad_vbo() -> GLuint {
    static QUAD: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
    vbo(&QUAD)
}

/// A small model: a fan of triangles in a VBO (the counts do not depend on
/// the horse's 7,172 or the bunny's 69,666 triangles).
fn model_vbo() -> GLuint {
    let mut v = Vec::new();
    for i in 0..32 {
        let a0 = i as f32 / 32.0 * core::f32::consts::TAU;
        let a1 = (i + 1) as f32 / 32.0 * core::f32::consts::TAU;
        v.extend_from_slice(&[0.0, 0.0, a0.cos(), a0.sin(), a1.cos(), a1.sin()]);
    }
    vbo(&v)
}

fn vbo(data: &[f32]) -> GLuint {
    let mut b: GLuint = 0;
    // SAFETY: plain GL calls; `data` lives across glBufferData.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(data) as GLsizeiptr,
            data.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glBindBuffer(GL_ARRAY_BUFFER, 0);
    }
    b
}

/// `Mesh::render_vbo` of `buf` (2D positions), `vertices` of them, with
/// `prog` in use.
fn draw(prog: GLuint, buf: GLuint, mode: GLenum, vertices: GLsizei) {
    // SAFETY: plain GL calls; the attribute reads the bound VBO.
    unsafe {
        glUseProgram(prog);
        glBindBuffer(GL_ARRAY_BUFFER, buf);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, core::ptr::null());
        glDrawArrays(mode, 0, vertices);
        glDisableVertexAttribArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glUseProgram(0);
    }
}

fn bind(unit: GLenum, tex: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(unit);
        glBindTexture(GL_TEXTURE_2D, tex);
    }
}

/// glTexImage2D of an RGBA8 `w` x `h` texture with `min` filtering, clamped;
/// `pixels` None is a render target (`BaseRenderer::create_texture`).
fn texture(w: u32, h: u32, min: GLenum, pixels: Option<&[u8]>) -> GLuint {
    let mut tex: GLuint = 0;
    // SAFETY: plain GL calls; `pixels` holds the w x h texels when given.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            pixels.map_or(core::ptr::null(), |p| p.as_ptr() as *const c_void),
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glBindTexture(GL_TEXTURE_2D, 0);
    }
    no_error("glTexImage2D");
    tex
}

/// A GL_DEPTH_COMPONENT / GL_UNSIGNED_INT texture (shadow's and refract's
/// depth targets), nearest, clamped.
fn depth_texture(w: u32, h: u32) -> GLuint {
    let mut tex: GLuint = 0;
    // SAFETY: plain GL calls; no pixels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH_COMPONENT as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
    }
    no_error("the depth texture");
    tex
}

/// A framebuffer with `colour` (a texture, or none) and `depth`: a 16-bit
/// renderbuffer of `w` x `h` (`BaseRenderer::create_fbo`), a depth texture,
/// or none. The window stays bound afterwards.
fn framebuffer(colour: Option<GLuint>, depth: Depth, w: u32, h: u32) -> GLuint {
    let mut fb: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if let Some(c) = colour {
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, c, 0);
        }
        match depth {
            Depth::None => {}
            Depth::Renderbuffer16 => {
                let mut rb: GLuint = 0;
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, w as GLsizei, h as GLsizei);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            }
            Depth::Texture(t) => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, t, 0),
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "framebuffer {w}x{h}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    no_error("the framebuffer");
    fb
}

#[derive(Clone, Copy)]
enum Depth {
    None,
    Renderbuffer16,
    Texture(GLuint),
}

fn generate_mipmap(tex: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
}

/// An image texture as `Texture::load` makes it (texture.cpp:95): uploaded,
/// mipmapped once at load when its filter asks for it.
fn image(w: u32, h: u32, mipmapped: bool) -> GLuint {
    let px = vec![0x7fu8; (w * h * 4) as usize];
    let min = if mipmapped { GL_LINEAR_MIPMAP_LINEAR } else { GL_LINEAR };
    let t = texture(w, h, min, Some(&px));
    if mipmapped {
        generate_mipmap(t);
        // SAFETY: a plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_2D, 0) };
    }
    t
}

/* ---- terrain -------------------------------------------------------------------------- */

/// One of terrain's renderers (`BaseRenderer`): its target (a framebuffer
/// and texture, or the window), its size, whether it has depth, and whether
/// its texture is mipmapped (`update_mipmap`).
#[derive(Clone, Copy)]
struct Renderer {
    fbo: GLuint,
    tex: GLuint,
    w: u32,
    h: u32,
    depth: bool,
    mipmapped: bool,
}

impl Renderer {
    fn offscreen(w: u32, h: u32, depth: bool, mipmapped: bool) -> Renderer {
        let min = if mipmapped { GL_LINEAR_MIPMAP_LINEAR } else { GL_LINEAR };
        let tex = texture(w, h, min, None);
        let fbo = framebuffer(Some(tex), if depth { Depth::Renderbuffer16 } else { Depth::None }, w, h);
        let r = Renderer { fbo, tex, w, h, depth, mipmapped };
        /* setup_texture's update_texture_parameters -> update_mipmap */
        r.update_mipmap();
        r
    }

    fn onscreen() -> Renderer {
        Renderer { fbo: 0, tex: 0, w: CANVAS_W, h: CANVAS_H, depth: true, mipmapped: false }
    }

    /// `BaseRenderer::make_current` (base-renderer.cpp:65).
    fn make_current(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, self.fbo);
            glViewport(0, 0, self.w as GLsizei, self.h as GLsizei);
            if self.depth {
                glEnable(GL_DEPTH_TEST);
                glDepthMask(1);
            } else {
                glDisable(GL_DEPTH_TEST);
                glDepthMask(0);
            }
        }
    }

    /// `BaseRenderer::update_mipmap` (base-renderer.cpp:84).
    fn update_mipmap(&self) {
        if self.tex != 0 && self.mipmapped {
            generate_mipmap(self.tex);
        }
    }

    /// `TextureRenderer::render` (texture-renderer.cpp:53): clear, the input
    /// on unit 0, the quad, the mipmap.
    fn render_texture(&self, prog: GLuint, quad: GLuint, input: GLuint) {
        self.make_current();
        // SAFETY: a plain GL call.
        unsafe { glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT) };
        bind(GL_TEXTURE0, input);
        draw(prog, quad, GL_TRIANGLE_STRIP, 4);
        if self.mipmapped {
            step("its chain");
        }
        self.update_mipmap();
    }
}

/// `SceneTerrain`'s renderers at setup (scene-terrain.cpp:59-172), and its
/// frame (`draw`, :332).
fn terrain() -> Box<dyn FnMut()> {
    let quad = quad_vbo();
    let grid = vbo(&[-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0]);
    let noise = program(QUAD_VS, FLAT_FS, &[]);
    let copy = program(QUAD_VS, TEX_FS, &[("t", 0)]);
    let blur = program(QUAD_VS, BLUR_FS, &[("t", 0)]);
    let terrain_prog = program(
        TERRAIN_VS,
        TERRAIN_FS,
        &[("tDiffuse1", 0), ("tDiffuse2", 1), ("tDetail", 2), ("tNormal", 3), ("tSpecular", 4), ("tDisplacement", 5)],
    );

    let height_map = Renderer::offscreen(256, 256, false, false);
    let normal_map = Renderer::offscreen(256, 256, false, false);
    let specular_map = Renderer::offscreen(512, 512, false, true);
    let terrain = Renderer::offscreen(CANVAS_W, CANVAS_H, true, true);
    let bloom_h = Renderer::offscreen(256, 256, false, true);
    let bloom_v = Renderer::offscreen(256, 256, false, true);
    let tilt_h = Renderer::offscreen(CANVAS_W, CANVAS_H, false, true);
    let tilt_v = Renderer::onscreen();
    /* TerrainRenderer::init_textures: the grass, rock and detail images */
    let diffuse1 = image(512, 512, true);
    let diffuse2 = image(512, 512, true);
    let detail = image(512, 512, true);
    /* the specular map: the luminance of diffuse1, rendered at setup */
    specular_map.render_texture(copy, quad, diffuse1);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
    }

    Box::new(move || {
        /* height_normal_chain: the noise, then the normal map from it */
        step("height map");
        height_map.render_texture(noise, quad, 0);
        step("normal map");
        normal_map.render_texture(copy, quad, height_map.tex);
        /* terrain_chain: TerrainRenderer::render (terrain-renderer.cpp:43) */
        step("terrain");
        terrain.make_current();
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.825, 0.7425, 0.61875, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        /* bind_textures (:144-160) */
        bind(GL_TEXTURE0, diffuse1);
        bind(GL_TEXTURE1, diffuse2);
        bind(GL_TEXTURE2, detail);
        bind(GL_TEXTURE3, normal_map.tex);
        bind(GL_TEXTURE4, specular_map.tex);
        bind(GL_TEXTURE5, height_map.tex);
        draw(terrain_prog, grid, GL_TRIANGLES, 6);
        step("terrain chain");
        terrain.update_mipmap();
        /* the bloom chain: bloom_h, bloom_v, the overlay */
        step("bloom_h");
        bloom_h.render_texture(blur, quad, terrain.tex);
        step("bloom_v");
        bloom_v.render_texture(blur, quad, bloom_h.tex);
        /* OverlayRenderer::render (overlay-renderer.cpp:72): into the
         * terrain's target, blended, then its mipmap again */
        step("overlay");
        terrain.make_current();
        bind(GL_TEXTURE0, bloom_v.tex);
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE);
        }
        draw(copy, quad, GL_TRIANGLE_STRIP, 4);
        // SAFETY: a plain GL call.
        unsafe { glDisable(GL_BLEND) };
        step("overlay chain");
        terrain.update_mipmap();
        /* the tilt-shift chain: tilt_h from the terrain's texture (the
         * overlay's), tilt_v into the window */
        step("tilt_h");
        tilt_h.render_texture(blur, quad, terrain.tex);
        step("tilt_v");
        tilt_v.render_texture(blur, quad, tilt_h.tex);
    })
}

/* ---- shadow --------------------------------------------------------------------------- */

/// `ShadowPrivate::setup` and `draw` (scene-shadow.cpp:404).
fn shadow() -> Box<dyn FnMut()> {
    let (w, h) = (CANVAS_W * 2, CANVAS_H * 2);
    let depth_tex = depth_texture(w, h);
    let fbo = framebuffer(None, Depth::Texture(depth_tex), w, h);
    let depth_prog = program(MODEL_VS, FLAT_FS, &[]);
    let ground_prog = program(QUAD_VS, GROUND_FS, &[("ShadowMap", 0)]);
    let lit_prog = program(MODEL_VS, FLAT_FS, &[]);
    let horse = model_vbo();
    let ground = quad_vbo();
    Box::new(move || {
        /* DepthRenderTarget::enable (:144) */
        step("depth pass");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth_tex, 0);
            glViewport(0, 0, w as GLsizei, h as GLsizei);
            glColorMask(0, 0, 0, 0);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        draw(depth_prog, horse, GL_TRIANGLES, 96);
        /* DepthRenderTarget::disable (:153) */
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glColorMask(1, 1, 1, 1);
        }
        /* GroundRenderer::draw (:252): the depth texture sampled */
        step("ground");
        bind(GL_TEXTURE0, depth_tex);
        draw(ground_prog, ground, GL_TRIANGLE_STRIP, 4);
        bind(GL_TEXTURE0, 0);
        /* the lit horse */
        step("horse");
        draw(lit_prog, horse, GL_TRIANGLES, 96);
    })
}

/* ---- refract -------------------------------------------------------------------------- */

/// `RefractPrivate::setup` and `draw` (scene-refract.cpp:394).
fn refract() -> Box<dyn FnMut()> {
    let (w, h) = (CANVAS_W * 2, CANVAS_H * 2);
    /* DistanceRenderTarget::setup (:165): DEPTH, then COLOR, mipmapped */
    let depth_tex = depth_texture(w, h);
    let colour_tex = texture(w, h, GL_LINEAR_MIPMAP_LINEAR, None);
    let fbo = framebuffer(Some(colour_tex), Depth::Texture(depth_tex), w, h);
    /* the image (Texture::load, GL_LINEAR, no mipmap) */
    let image_tex = image(512, 512, false);
    let distance_prog = program(MODEL_VS, FLAT_FS, &[]);
    let refract_prog = program(MODEL_VS, REFRACT_FS, &[("DistanceMap", 0), ("NormalMap", 1), ("ImageMap", 2)]);
    let bunny = model_vbo();
    Box::new(move || {
        /* DistanceRenderTarget::enable (:244) */
        step("distance pass");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth_tex, 0);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, colour_tex, 0);
            glViewport(0, 0, w as GLsizei, h as GLsizei);
            glClear(GL_DEPTH_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
            glCullFace(GL_FRONT);
        }
        draw(distance_prog, bunny, GL_TRIANGLES, 96);
        /* DistanceRenderTarget::disable (:258) */
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS_W as GLsizei, CANVAS_H as GLsizei);
            glCullFace(GL_BACK);
        }
        /* :428-431 */
        step("its chain");
        generate_mipmap(colour_tex);
        // SAFETY: a plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_2D, 0) };
        /* :445-452, then the bunny into the window */
        step("bunny");
        bind(GL_TEXTURE0, depth_tex);
        bind(GL_TEXTURE1, colour_tex);
        bind(GL_TEXTURE2, image_tex);
        draw(refract_prog, bunny, GL_TRIANGLES, 96);
    })
}

/* ---- the tests ------------------------------------------------------------------------ */

#[test]
fn terrain_a_frame_within_its_ceilings() {
    check("terrain", Gpu::Lag, &run(Gpu::Lag, terrain), &TERRAIN_LAG);
    check("terrain", Gpu::Done, &run(Gpu::Done, terrain), &TERRAIN_DONE);
}

#[test]
fn shadow_a_frame_within_its_ceilings() {
    check("shadow", Gpu::Lag, &run(Gpu::Lag, shadow), &SHADOW_LAG);
    check("shadow", Gpu::Done, &run(Gpu::Done, shadow), &SHADOW_DONE);
}

#[test]
fn refract_a_frame_within_its_ceilings() {
    check("refract", Gpu::Lag, &run(Gpu::Lag, refract), &REFRACT_LAG);
    check("refract", Gpu::Done, &run(Gpu::Done, refract), &REFRACT_DONE);
}
