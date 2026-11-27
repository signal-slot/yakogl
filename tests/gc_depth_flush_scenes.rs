// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): dtpu knob mode 4 (`DEPTH_TPU_D24X8_FLUSH`,
//! src/vivante/depth_sample.rs) puts its texture-cache flush block only in
//! front of draws that sample a depth view, in glmark2's three scenes
//! (docs/notes/gc-depth-sample-wire.md (a).10). The mode is set explicitly.
//! Each scene runs its frames through the real GL core and the Vivante back
//! end over the stand-in OSAL of `common/gc_scenes.rs`; every submitted
//! stream is decoded into register writes, front-end stalls and draws (the
//! harness of tests/gc_depth_view_flush_mode.rs, copied). A *flush block* is
//! the seven events `emit_view_flush` writes (FLUSH_CACHE DEPTH; SEMAPHORE
//! FE-PE; STALL FE-PE; FLUSH_CACHE TEXTURE; FLUSH_CACHE TEXTUREVS; SEMAPHORE
//! RA-PE; STALL_TOKEN RA-PE).
//!
//! Pinned, for each of the six frames of each scene in mode 4:
//!
//! * shadow and refract: as many flush blocks as draws whose slot 0
//!   descriptor is D24X8 (a depth view), at least one; each block lies in
//!   the window of such a draw (after the previous draw, before this one);
//!   every such draw's window holds exactly one block (one depth write a
//!   frame, so one block per sampling draw) and no other window holds one;
//! * terrain: no flush block and no depth-view draw;
//! * all three: the CPU depth resolve (`depth_resolve` of the table) runs
//!   0 times, so no resolve sits behind those draws.
//!
//! The runs are in one `#[test]` in a fixed order: the back end keeps state
//! across runs of one process.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_flush_scenes
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

#[path = "common/gc_scenes.rs"]
mod gc_scenes;

use core::ffi::c_void;
use gc_scenes::*;
use std::sync::{Mutex, OnceLock};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown};
use yakogl::vivante::depth_sample::{
    gc_debug_set_depth_tpu, DEPTH_TPU_D24X8_FLUSH, DEPTH_TPU_OFF, GC_TEX_FORMAT_D24X8,
};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// One decoded stream item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ev {
    /// A register write.
    State(u32, u32),
    /// The front end's own STALL word (token).
    FeStall(u32),
    /// A draw.
    Draw,
    /// Slot 0's descriptor address was written; the descriptor's CONFIG0 and
    /// SAMP_CTRL0 as they stood in memory at the submit (None: no live
    /// allocation holds the address).
    Slot0(Option<u32>),
}

/// The decoded kicks of the frame being recorded.
static CAPTURE: Mutex<Vec<Vec<Ev>>> = Mutex::new(Vec::new());
/// The stand-in's allocations as the wrapper saw them: (gpu, cpu, size).
static MEMS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

/// The base table's entries, to hand each call on to.
fn base() -> &'static OsalOps {
    &OSAL
}

unsafe extern "C" fn w_gpu_alloc(heap: u64, size: usize, flags: u32, out: *mut OsalGpuMem) -> i32 {
    // SAFETY: handed on as it came.
    let rc = unsafe { (base().gpu_alloc.unwrap())(heap, size, flags, out) };
    if rc == 0 {
        // SAFETY: the base filled the descriptor.
        let m = unsafe { *out };
        MEMS.lock().unwrap_or_else(|e| e.into_inner()).push((m.gpu, m.cpu as usize, m.size));
    }
    rc
}

unsafe extern "C" fn w_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's live descriptor.
    let gpu = unsafe { (*mem).gpu };
    MEMS.lock().unwrap_or_else(|e| e.into_inner()).retain(|m| m.0 != gpu);
    // SAFETY: handed on as it came.
    unsafe { (base().gpu_free.unwrap())(mem) }
}

/// The dword at GPU address `gpu`, if a live allocation holds it.
fn read_dword(gpu: u64) -> Option<u32> {
    let mems = MEMS.lock().unwrap_or_else(|e| e.into_inner());
    mems.iter().find(|m| gpu >= m.0 && gpu + 4 <= m.0 + m.2 as u64).map(|m| {
        // SAFETY: inside a live allocation of the stand-in (8-aligned boxed
        // [u64]); gpu is 4-aligned.
        unsafe { ((m.1 + (gpu - m.0) as usize) as *const u32).read() }
    })
}

/// Decode one stream (the register walk of `gc_scenes::tally_stream`).
fn decode(words: &[u32]) -> Vec<Ev> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for (k, &v) in words.iter().skip(i + 1).take(count).enumerate() {
                    let reg = base + 4 * k as u32;
                    if reg == nte_descriptor_addr(0) {
                        out.push(Ev::Slot0(read_dword(v as u64 + GC_TEXDESC_CONFIG0 as u64)));
                    }
                    out.push(Ev::State(reg, v));
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                out.push(Ev::Draw);
                i += 4;
            }
            GC_OP_STALL => {
                out.push(Ev::FeStall(words.get(i + 1).copied().unwrap_or(0)));
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

unsafe extern "C" fn w_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<GcKick>() {
        // SAFETY: the back end passes a live GcKick of that size.
        let k = unsafe { *(kick as *const GcKick) };
        if k.magic == GC_KICK_MAGIC {
            let mems = MEMS.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(m) = mems.iter().find(|m| k.stream_gpu >= m.0 && k.stream_gpu < m.0 + m.2 as u64) {
                let at = (k.stream_gpu - m.0) as usize;
                let n = (k.stream_bytes as usize).min(m.2 - at) / 4;
                // SAFETY: inside the live allocation, read as dwords.
                let words = unsafe { core::slice::from_raw_parts((m.1 + at) as *const u32, n) }.to_vec();
                let evs = decode(&words);
                CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).push(evs);
            }
        }
    }
    // SAFETY: handed on as it came.
    unsafe { (base().submit.unwrap())(kick, len) }
}

/// The base table with the three wrapped entries, made once.
fn wrapped() -> *const OsalOps {
    static T: OnceLock<usize> = OnceLock::new();
    *T.get_or_init(|| {
        // SAFETY: plain data (function pointers); the copy is a second table.
        let mut t: OsalOps = unsafe { core::ptr::read(&OSAL) };
        t.gpu_alloc = Some(w_gpu_alloc);
        t.gpu_free = Some(w_gpu_free);
        t.submit = Some(w_submit);
        Box::leak(Box::new(t)) as *const OsalOps as usize
    }) as *const OsalOps
}

/// Run `scene` with the depth-view knob in `mode` and return the decoded
/// kicks of every frame. The wrapper is installed before `gc_init` (so it
/// sees every allocation), the knob is set and put back inside the run, and
/// every run starts from clean captures; `SERIAL` is held throughout.
fn frames(mode: i32, scene: fn() -> Box<dyn FnMut()>) -> Vec<(Vec<Vec<Ev>>, u32)> {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| *p = Platform::default());
    MEMS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: SERIAL is held; the tables are statics or leaked.
    let ctx = unsafe {
        osal_set_ops(wrapped());
        yakogl::backend::g_backend = counted_table();
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(CANVAS_W, CANVAS_H);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    gc_debug_set_depth_tpu(mode);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        step("setup");
        // SAFETY: plain GL calls (glmark2's Scene::setup state).
        unsafe {
            use yakogl::gl::consts::*;
            use yakogl::gl::state::*;
            glViewport(0, 0, CANVAS_W as i32, CANVAS_H as i32);
            glEnable(GL_DEPTH_TEST);
            glEnable(GL_CULL_FACE);
            glCullFace(GL_BACK);
        }
        let mut frame = scene();
        no_error("the scene's setup");
        let mut all = Vec::new();
        for n in 1..=FRAMES {
            CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).clear();
            let resolves_before = snapshot().resolves;
            canvas_clear();
            frame();
            swap();
            no_error(&format!("frame {n}"));
            let resolves = snapshot().resolves - resolves_before;
            all.push((std::mem::take(&mut *CAPTURE.lock().unwrap_or_else(|e| e.into_inner())), resolves));
        }
        all
    }));
    gc_debug_set_depth_tpu(DEPTH_TPU_OFF);
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    r.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/// The windows of one kick: (start of the window, index of its draw, slot-0
/// CONFIG0 written in it).
fn windows(evs: &[Ev]) -> Vec<(usize, usize, Option<u32>)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut cfg = None;
    for (i, e) in evs.iter().enumerate() {
        match e {
            Ev::Slot0(c) => cfg = *c,
            Ev::Draw => {
                out.push((start, i, cfg.take()));
                start = i + 1;
            }
            _ => {}
        }
    }
    out
}

const DESC_FORMAT_MASK: u32 = 0x1f << 13;

fn is_view(cfg: Option<u32>) -> bool {
    cfg.is_some_and(|c| c & DESC_FORMAT_MASK == texdesc_format(GC_TEX_FORMAT_D24X8))
}

/// The seven events of `emit_view_flush`.
fn block() -> [Ev; 7] {
    let fe_pe = stall_token(GC_SYNC_FE, GC_SYNC_PE);
    let ra_pe = stall_token(GC_SYNC_RA, GC_SYNC_PE);
    [
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_DEPTH),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, fe_pe),
        Ev::FeStall(fe_pe),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTURE),
        Ev::State(GC_GL_FLUSH_CACHE, GC_FLUSH_TEXTUREVS),
        Ev::State(GC_GL_SEMAPHORE_TOKEN, ra_pe),
        Ev::State(GC_GL_STALL_TOKEN, ra_pe),
    ]
}

/// Where the flush blocks of one kick start.
fn blocks(evs: &[Ev]) -> Vec<usize> {
    let b = block();
    (0..evs.len()).filter(|&i| evs[i..].starts_with(&b)).collect()
}

/// One frame's flush blocks as (kick, block start), and its sampling draws
/// as (kick, window start, draw index).
type Found = (Vec<(usize, usize)>, Vec<(usize, usize, usize)>);

fn found(kicks: &[Vec<Ev>]) -> Found {
    let mut bl = Vec::new();
    let mut sd = Vec::new();
    for (k, evs) in kicks.iter().enumerate() {
        bl.extend(blocks(evs).into_iter().map(|s| (k, s)));
        sd.extend(windows(evs).into_iter().filter(|w| is_view(w.2)).map(|(s, d, _)| (k, s, d)));
    }
    (bl, sd)
}

/// One frame's flush blocks as (kick, block start), and its sampling draws
/// Frames of one scene in mode 4: the flush blocks and the depth-view draws
/// agree, window by window; `views` says whether the scene samples a depth
/// view at all.
fn check(name: &str, all: &[(Vec<Vec<Ev>>, u32)], views: bool) {
    assert!(all.len() >= 2, "{name}: need two frames");
    for (f, (kicks, resolves)) in all.iter().enumerate() {
        let at = format!("{name} frame {}", f + 1);
        assert_eq!(*resolves, 0, "{at}: {resolves} CPU depth resolves, want none");
        let mut sampling = 0usize;
        let mut flushed = 0usize;
        for (k, evs) in kicks.iter().enumerate() {
            let at_blocks = blocks(evs);
            for (start, draw, cfg) in windows(evs) {
                let here: Vec<usize> = at_blocks.iter().copied().filter(|&b| start <= b && b < draw).collect();
                if is_view(cfg) {
                    sampling += 1;
                    assert_eq!(here.len(), 1, "{at} kick {k}: the depth-view draw at {draw} has {} blocks", here.len());
                    flushed += 1;
                } else {
                    assert!(here.is_empty(), "{at} kick {k}: a flush block at {here:?} before the draw at {draw}, which samples no depth view");
                }
            }
            /* every block is in some draw's window */
            let inside = at_blocks.iter().filter(|&&b| windows(evs).iter().any(|w| w.0 <= b && b < w.1)).count();
            assert_eq!(inside, at_blocks.len(), "{at} kick {k}: a flush block outside every draw window");
        }
        let (bl, sd) = found(kicks);
        assert_eq!(bl.len(), sd.len(), "{at}: {} blocks, {} depth-view draws", bl.len(), sd.len());
        assert_eq!(flushed, sampling);
        if views {
            assert!(sampling >= 1, "{at}: no draw samples the depth view");
        } else {
            assert!(bl.is_empty() && sd.is_empty(), "{at}: blocks {bl:?}, depth-view draws {sd:?}");
        }
    }
}

#[test]
fn mode4_flushes_only_in_front_of_depth_view_draws() {
    for (name, scene, views) in
        [("shadow", shadow as fn() -> Box<dyn FnMut()>, true), ("refract", refract, true), ("terrain", terrain, false)]
    {
        let all = frames(DEPTH_TPU_D24X8_FLUSH, scene);
        assert_eq!(all.len(), FRAMES);
        check(name, &all, views);
    }
    /* the knob is back off */
    let off = frames(DEPTH_TPU_OFF, shadow);
    assert!(off.iter().all(|(k, _)| found(k).0.is_empty()), "mode 0 wrote a flush block");
}
