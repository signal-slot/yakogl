// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The harness of `gc_scene_kicks.rs`: glmark2's terrain, shadow and refract
//! frames driven through the real GL core and the Vivante back end over a
//! stand-in OSAL, with the per-frame counts (see that file's header for what
//! each count is and where each GL call sequence comes from).
//!
//! Shared the way `deqp_style.rs` is: `#[path = "common/gc_scenes.rs"] mod
//! gc_scenes;` in a test that carries the same `#![cfg(all(feature =
//! "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature
//! = "report-es2"))]`, or one that sets the knob (`gc_scene_kicks_tiled.rs`,
//! `test-knobs` in place of `report-es2`). [`run_with`] takes [`Hooks`]: `after_init` runs once
//! after `gc_init` and the context are made (before the scene's setup),
//! `each_frame(n)` before each frame's canvas clear (n from 1) and
//! `after_frame(n)` after its swap, for a test that sets a back-end knob.
//! [`stream_tally`] counts the RS jobs and early-depth draws of the streams
//! submitted (`gc_scene_kicks_tiled_rt.rs`).

#![allow(dead_code)]

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
use yakogl::vivante::regs::{
    GC_OP_DRAW_INSTANCED, GC_OP_LOAD_STATE, GC_OP_STALL, GC_PE_DEPTH_CONFIG, GC_PE_DEPTH_EARLY_Z,
    GC_PE_DEPTH_SUPER_TILED, GC_RS_CLEAR_CONTROL, GC_RS_CLEAR_CONTROL_MODE_ENABLED1, GC_RS_CONFIG, GC_RS_DEST_TILED,
    GC_RS_KICKER, GC_RS_SOURCE_TILED,
};

/// One frame's counts (see the file's comment for what each one is).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub kicks: u32,
    pub waits_other: u32,
    pub finishes_other: u32,
    pub tiled_copies: u32,
    pub resolves: u32,
    /// Not a ceiling: the swap's own waits, printed for the record.
    pub waits_swap: u32,
}

impl Counts {
    /// `self` field by field at most `ceil` (the swap's waits are not held).
    pub fn within(&self, ceil: &Counts) -> Vec<&'static str> {
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
    pub fn max_of(frames: &[Counts]) -> Counts {
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

/// What the kicks' command streams hold, read by the stand-in's `submit`
/// (`gc_scene_kicks_tiled.rs` pins them; `gc_scene_kicks.rs` ignores them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCounts {
    /// RS kicks that write a TILED destination from a linear source: the
    /// tiled-target tile-in and the RS tiled copies of vertex-stage maps.
    pub rs_tile_in: u32,
    /// RS kicks that read a TILED source into a linear destination: a tiled
    /// shadow's copy back.
    pub rs_copy_back: u32,
    /// Every other RS kick: fills (`rs_clear.rs`) and linear copies.
    pub rs_other: u32,
    /// Draw commands.
    pub draws: u32,
    /// Draws whose PE depth config says SUPER_TILED: a draw into a tiled
    /// shadow (a linear target keeps its depth linear).
    pub draws_tiled_shadow: u32,
    /// Draws whose PE depth config says EARLY_Z.
    pub draws_early_z: u32,
}

impl StreamCounts {
    fn minus(&self, o: &StreamCounts) -> StreamCounts {
        StreamCounts {
            rs_tile_in: self.rs_tile_in - o.rs_tile_in,
            rs_copy_back: self.rs_copy_back - o.rs_copy_back,
            rs_other: self.rs_other - o.rs_other,
            draws: self.draws - o.draws,
            draws_tiled_shadow: self.draws_tiled_shadow - o.draws_tiled_shadow,
            draws_early_z: self.draws_early_z - o.draws_early_z,
        }
    }

    /// The largest of each count over `frames`.
    pub fn max_of(frames: &[StreamCounts]) -> StreamCounts {
        frames.iter().fold(StreamCounts::default(), |m, f| StreamCounts {
            rs_tile_in: m.rs_tile_in.max(f.rs_tile_in),
            rs_copy_back: m.rs_copy_back.max(f.rs_copy_back),
            rs_other: m.rs_other.max(f.rs_other),
            draws: m.draws.max(f.draws),
            draws_tiled_shadow: m.draws_tiled_shadow.max(f.draws_tiled_shadow),
            draws_early_z: m.draws_early_z.max(f.draws_early_z),
        })
    }
}

/// Walk one kick's stream (the `LOAD_STATE` / draw / stall encoding of
/// `cmd.rs`) and add what it holds to `c`.
fn count_stream(words: &[u32], c: &mut StreamCounts) {
    let (mut config, mut clear, mut depth) = (0u32, 0u32, 0u32);
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    let Some(&v) = words.get(i + 1 + k) else { break };
                    match base + 4 * k as u32 {
                        GC_RS_CONFIG => config = v,
                        GC_RS_CLEAR_CONTROL => clear = v,
                        GC_PE_DEPTH_CONFIG => depth = v,
                        GC_RS_KICKER => {
                            let fill = clear & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 != 0;
                            let (src, dst) = (config & GC_RS_SOURCE_TILED != 0, config & GC_RS_DEST_TILED != 0);
                            match (fill, src, dst) {
                                (false, false, true) => c.rs_tile_in += 1,
                                (false, true, false) => c.rs_copy_back += 1,
                                _ => c.rs_other += 1,
                            }
                        }
                        _ => {}
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                c.draws += 1;
                c.draws_tiled_shadow += u32::from(depth & GC_PE_DEPTH_SUPER_TILED != 0);
                c.draws_early_z += u32::from(depth & GC_PE_DEPTH_EARLY_Z != 0);
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
}

/// Frames drawn, and the first one counted: frame 1 allocates the targets'
/// surfaces and copies, frame 2 is the first whose swap has a previous frame.
pub const FRAMES: usize = 6;
pub const FIRST_COUNTED: usize = 3;

/* ---- the stand-in OSAL: host memory, kicks, waits and CPU reads counted -------------- */

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gpu {
    /// A kick completes when the CPU waits for it or for a later one.
    #[default]
    Lag,
    /// Every kick completes when it is submitted.
    Done,
}

#[derive(Default)]
pub struct Platform {
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
    stream: StreamCounts,
    /// The test's step now, and what the stand-in saw in each one: `kick`
    /// and `wait` (outside the swap) by step, for the record.
    step: &'static str,
    events: Vec<String>,
    faults: Vec<String>,
    /// What the submitted streams asked of the resolve engine and the pixel
    /// engine ([`stream_tally`]), and the two registers it follows across
    /// kicks (the GPU keeps its state from one kick to the next).
    tally: StreamTally,
    rs_config: u32,
    pe_depth_config: u32,
}

/// The resolve-engine jobs and the draws of the submitted streams, read off
/// their words at `submit` (`LOAD_STATE`, `DRAW_INSTANCED`): what a test of
/// the tiled shadows (`gc_tiled_rt`) counts besides [`Counts`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamTally {
    /// `RS_KICKER` writes.
    pub rs_kicks: u32,
    /// RS jobs whose source is TILED (`RS_CONFIG` `SOURCE_TILED`): a shadow's
    /// copy back.
    pub rs_from_tiled: u32,
    /// RS jobs whose destination is TILED (`DEST_TILED`): a shadow's tile-in,
    /// or a texture's RS tiled copy (`tiled_copy.rs`).
    pub rs_to_tiled: u32,
    /// Draws, and those with `PE_DEPTH_CONFIG` `EARLY_Z` set.
    pub draws: u32,
    pub early_z_draws: u32,
}

impl StreamTally {
    /// `self - before`, count by count.
    pub fn since(&self, before: &StreamTally) -> StreamTally {
        StreamTally {
            rs_kicks: self.rs_kicks - before.rs_kicks,
            rs_from_tiled: self.rs_from_tiled - before.rs_from_tiled,
            rs_to_tiled: self.rs_to_tiled - before.rs_to_tiled,
            draws: self.draws - before.draws,
            early_z_draws: self.early_z_draws - before.early_z_draws,
        }
    }
}

/// The tally of every stream submitted since the run began.
pub fn stream_tally() -> StreamTally {
    with_platform(|p| p.tally)
}

/// Walk one submitted stream into the tally (the register walk of
/// `backend_tests.rs` `walk`).
fn tally_stream(p: &mut Platform, words: &[u32]) {
    use yakogl::vivante::regs::*;
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for (k, &v) in words.iter().skip(i + 1).take(count).enumerate() {
                    match base + 4 * k as u32 {
                        GC_RS_CONFIG => p.rs_config = v,
                        GC_PE_DEPTH_CONFIG => p.pe_depth_config = v,
                        GC_RS_KICKER => {
                            p.tally.rs_kicks += 1;
                            p.tally.rs_from_tiled += u32::from(p.rs_config & GC_RS_SOURCE_TILED != 0);
                            p.tally.rs_to_tiled += u32::from(p.rs_config & GC_RS_DEST_TILED != 0);
                        }
                        _ => {}
                    }
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                p.tally.draws += 1;
                p.tally.early_z_draws += u32::from(p.pe_depth_config & GC_PE_DEPTH_EARLY_Z != 0);
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1, /* NOP and the pad words */
        }
    }
}

pub static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
pub static SERIAL: Mutex<()> = Mutex::new(());

pub fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
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
        let stream = p.allocs.range(..=k.stream_gpu).next_back().and_then(|(&base, a)| {
            let (at, n) = ((k.stream_gpu - base) as usize, k.stream_bytes as usize);
            // SAFETY: the stand-in's own allocation, read as u32 words inside
            // its size (the back end's stream lies in one allocation).
            (at + n <= a.len() * 8).then(|| unsafe {
                core::slice::from_raw_parts((a.as_ptr() as *const u8).add(at) as *const u32, n / 4).to_vec()
            })
        });
        match stream {
            Some(words) => tally_stream(p, &words),
            None => p.faults.push(format!("submit of a stream at 0x{:x} outside every allocation", k.stream_gpu)),
        }
        p.kicks += 1;
        p.fence += 1;
        if let Some((base, a)) = p.allocs.range(..=k.stream_gpu).next_back() {
            let off = (k.stream_gpu - base) as usize;
            // SAFETY: a boxed [u64], read as bytes.
            let bytes = unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u8, a.len() * 8) };
            let end = (off + k.stream_bytes as usize).min(bytes.len());
            let words: Vec<u32> =
                bytes[off.min(end)..end].as_chunks::<4>().0.iter().map(|b| u32::from_ne_bytes(*b)).collect();
            let mut s = p.stream;
            count_stream(&words, &mut s);
            p.stream = s;
        }
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

pub static OSAL: OsalOps = OsalOps {
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
pub fn counted_table() -> *const YakoglBackend {
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

pub const CANVAS_W: u32 = 800;
pub const CANVAS_H: u32 = 600;

/// Run `scene` (its setup, then `frame` FRAMES times with the canvas clear
/// before and the swap after) on the real back end under `gpu`, and return
/// the counts of every frame from FIRST_COUNTED on, and the last frame's
/// kicks and waits by step.
pub fn run(gpu: Gpu, scene: impl FnOnce() -> Box<dyn FnMut()>) -> (Vec<Counts>, Vec<String>) {
    run_with(gpu, scene, Hooks::default())
}

/// What a test may do around the frames: `after_init` once after `gc_init`
/// and the context exist (before the scene's setup), `each_frame(n)` before
/// frame `n`'s canvas clear (n from 1), `after_frame(n)` after its swap.
#[derive(Default)]
pub struct Hooks<'a> {
    pub after_init: Option<Box<dyn FnOnce() + 'a>>,
    pub each_frame: Option<Box<dyn FnMut(usize) + 'a>>,
    pub after_frame: Option<Box<dyn FnMut(usize) + 'a>>,
}

/// [`run`] with [`Hooks`].
pub fn run_with(gpu: Gpu, scene: impl FnOnce() -> Box<dyn FnMut()>, hooks: Hooks<'_>) -> (Vec<Counts>, Vec<String>) {
    let (counts, events, _) = run_full(gpu, scene, hooks);
    (counts, events)
}

/// [`run_with`] with the counted frames' [`StreamCounts`] as well.
pub fn run_full(
    gpu: Gpu,
    scene: impl FnOnce() -> Box<dyn FnMut()>,
    mut hooks: Hooks<'_>,
) -> (Vec<Counts>, Vec<String>, Vec<StreamCounts>) {
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
        if let Some(f) = hooks.after_init.take() {
            f();
        }
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
        let mut streams = Vec::new();
        for n in 1..=FRAMES {
            if let Some(f) = hooks.each_frame.as_mut() {
                f(n);
            }
            let before = snapshot();
            let stream_before = with_platform(|p| p.stream);
            with_platform(|p| p.events.clear());
            canvas_clear();
            frame();
            swap();
            no_error(&format!("frame {n}"));
            if let Some(f) = hooks.after_frame.as_mut() {
                f(n);
            }
            let after = snapshot();
            if n >= FIRST_COUNTED {
                streams.push(with_platform(|p| p.stream).minus(&stream_before));
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
        (counted, with_platform(|p| core::mem::take(&mut p.events)), streams)
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
pub fn snapshot() -> Counts {
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
pub fn canvas_clear() {
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
pub fn swap() {
    step("swap");
    with_platform(|p| p.in_swap = true);
    assert_eq!(yakogl::backend::flush(), 0, "the swap's flush");
    assert_eq!(yakogl::backend::finish_previous(), 0, "the swap's finish_previous");
    with_platform(|p| p.in_swap = false);
}

/// The step of the frame the GL calls from here on belong to.
pub fn step(name: &'static str) {
    with_platform(|p| p.step = name);
}

/* ---- GL plumbing ---------------------------------------------------------------------- */

pub fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

pub fn shader(ty: GLenum, text: &str) -> GLuint {
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
pub fn program(vs: &str, fs: &str, samplers: &[(&str, GLint)]) -> GLuint {
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

pub const QUAD_VS: &str = "attribute vec2 position;\nvarying vec2 v;\n\
    void main() { v = position * 0.5 + 0.5; gl_Position = vec4(position, 0.0, 1.0); }\n";
pub const FLAT_FS: &str =
    "precision mediump float;\nvarying vec2 v;\nvoid main() { gl_FragColor = vec4(v, 0.5, 1.0); }\n";
pub const TEX_FS: &str = "precision mediump float;\nuniform sampler2D t;\nvarying vec2 v;\n\
    void main() { gl_FragColor = texture2D(t, v); }\n";
/// A blur: taps of the input on each side (`terrain-blur.frag`).
pub const BLUR_FS: &str = "precision mediump float;\nuniform sampler2D t;\nvarying vec2 v;\n\
    void main() { gl_FragColor = 0.25 * (texture2D(t, v - vec2(0.01, 0.0)) + 2.0 * texture2D(t, v)\n\
    + texture2D(t, v + vec2(0.01, 0.0))); }\n";
/// `terrain.vert`: the displacement (height) and normal maps sampled in the
/// vertex stage.
pub const TERRAIN_VS: &str = "attribute vec2 position;\nuniform sampler2D tDisplacement;\nuniform sampler2D tNormal;\n\
    varying vec2 v;\nvarying vec3 n;\n\
    void main() { v = position * 0.5 + 0.5; float h = texture2D(tDisplacement, v).x;\n\
    n = texture2D(tNormal, v).xyz; gl_Position = vec4(position, h * 0.5, 1.0); }\n";
/// `terrain.frag`: detail, both diffuse maps, the displacement and specular maps.
pub const TERRAIN_FS: &str = "precision mediump float;\nuniform sampler2D tDiffuse1;\nuniform sampler2D tDiffuse2;\n\
    uniform sampler2D tDetail;\nuniform sampler2D tSpecular;\nuniform sampler2D tDisplacement;\n\
    varying vec2 v;\nvarying vec3 n;\n\
    void main() { vec4 d = mix(texture2D(tDiffuse1, v), texture2D(tDiffuse2, v), texture2D(tDisplacement, v).x);\n\
    gl_FragColor = d * texture2D(tDetail, v) + vec4(texture2D(tSpecular, v).xyz * n, 0.0); }\n";
/// A model drawn with a transform (the horse's and the bunny's programs).
pub const MODEL_VS: &str = "attribute vec2 position;\nvarying vec2 v;\n\
    void main() { v = position * 0.5 + 0.5; gl_Position = vec4(position * 0.8, 0.25, 1.0); }\n";
/// shadow's ground: the depth texture sampled (`shadow.frag`).
pub const GROUND_FS: &str = "precision mediump float;\nuniform sampler2D ShadowMap;\nvarying vec2 v;\n\
    void main() { float d = texture2D(ShadowMap, v).x; gl_FragColor = vec4(vec3(d < 0.5 ? 0.3 : 1.0), 1.0); }\n";
/// refract's bunny in the window: the distance (depth), the mipmapped colour
/// and the image (`refract.frag`).
pub const REFRACT_FS: &str = "precision mediump float;\nuniform sampler2D DistanceMap;\nuniform sampler2D NormalMap;\n\
    uniform sampler2D ImageMap;\nvarying vec2 v;\n\
    void main() { float d = texture2D(DistanceMap, v).x;\n\
    gl_FragColor = mix(texture2D(NormalMap, v * d), texture2D(ImageMap, v), 0.5); }\n";

/// A full-viewport quad, glmark2's `Mesh` quad in a VBO (`render_vbo`).
pub fn quad_vbo() -> GLuint {
    static QUAD: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
    vbo(&QUAD)
}

/// A small model: a fan of triangles in a VBO (the counts do not depend on
/// the horse's 7,172 or the bunny's 69,666 triangles).
pub fn model_vbo() -> GLuint {
    let mut v = Vec::new();
    for i in 0..32 {
        let a0 = i as f32 / 32.0 * core::f32::consts::TAU;
        let a1 = (i + 1) as f32 / 32.0 * core::f32::consts::TAU;
        v.extend_from_slice(&[0.0, 0.0, a0.cos(), a0.sin(), a1.cos(), a1.sin()]);
    }
    vbo(&v)
}

pub fn vbo(data: &[f32]) -> GLuint {
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
pub fn draw(prog: GLuint, buf: GLuint, mode: GLenum, vertices: GLsizei) {
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

pub fn bind(unit: GLenum, tex: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(unit);
        glBindTexture(GL_TEXTURE_2D, tex);
    }
}

/// glTexImage2D of an RGBA8 `w` x `h` texture with `min` filtering, clamped;
/// `pixels` None is a render target (`BaseRenderer::create_texture`).
pub fn texture(w: u32, h: u32, min: GLenum, pixels: Option<&[u8]>) -> GLuint {
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

/// The depth textures' internal format. Under `report-es2` glmark2's own
/// unsized `GL_DEPTH_COMPONENT`; under the ES 3.0 report (a build without
/// `report-es2`) an unsized depth texture is no attachment
/// (`GlTexture::unsized_depth`), so the sized 24-bit one stands in for it.
fn depth_internalformat() -> GLenum {
    if cfg!(feature = "report-es2") {
        GL_DEPTH_COMPONENT
    } else {
        GL_DEPTH_COMPONENT24
    }
}

/// A GL_DEPTH_COMPONENT / GL_UNSIGNED_INT texture (shadow's and refract's
/// depth targets), nearest, clamped.
pub fn depth_texture(w: u32, h: u32) -> GLuint {
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
            depth_internalformat() as GLint,
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
pub fn framebuffer(colour: Option<GLuint>, depth: Depth, w: u32, h: u32) -> GLuint {
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
pub enum Depth {
    None,
    Renderbuffer16,
    Texture(GLuint),
}

pub fn generate_mipmap(tex: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
}

/// An image texture as `Texture::load` makes it (texture.cpp:95): uploaded,
/// mipmapped once at load when its filter asks for it.
pub fn image(w: u32, h: u32, mipmapped: bool) -> GLuint {
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
pub struct Renderer {
    fbo: GLuint,
    tex: GLuint,
    w: u32,
    h: u32,
    depth: bool,
    mipmapped: bool,
}

impl Renderer {
    pub fn offscreen(w: u32, h: u32, depth: bool, mipmapped: bool) -> Renderer {
        let min = if mipmapped { GL_LINEAR_MIPMAP_LINEAR } else { GL_LINEAR };
        let tex = texture(w, h, min, None);
        let fbo = framebuffer(Some(tex), if depth { Depth::Renderbuffer16 } else { Depth::None }, w, h);
        let r = Renderer { fbo, tex, w, h, depth, mipmapped };
        /* setup_texture's update_texture_parameters -> update_mipmap */
        r.update_mipmap();
        r
    }

    pub fn onscreen() -> Renderer {
        Renderer { fbo: 0, tex: 0, w: CANVAS_W, h: CANVAS_H, depth: true, mipmapped: false }
    }

    /// `BaseRenderer::make_current` (base-renderer.cpp:65).
    pub fn make_current(&self) {
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
    pub fn update_mipmap(&self) {
        if self.tex != 0 && self.mipmapped {
            generate_mipmap(self.tex);
        }
    }

    /// `TextureRenderer::render` (texture-renderer.cpp:53): clear, the input
    /// on unit 0, the quad, the mipmap.
    pub fn render_texture(&self, prog: GLuint, quad: GLuint, input: GLuint) {
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
pub fn terrain() -> Box<dyn FnMut()> {
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
pub fn shadow() -> Box<dyn FnMut()> {
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
pub fn refract() -> Box<dyn FnMut()> {
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
