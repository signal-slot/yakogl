// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000 command-stream decode harness of the depth-view flush tests
//! (`gc_depth_view_flushes.rs`, `gc_depth_view_flush_mode.rs`,
//! `gc_depth_flush_scenes.rs`): a wrapper over the stand-in OSAL of
//! `gc_scenes.rs` that decodes every submitted stream, at its submit, into
//! register writes, front-end stalls, draws and slot 0's descriptor CONFIG0.
//! A test declares both modules (`#[path = "common/gc_scenes.rs"] mod
//! gc_scenes;` and `#[path = "common/gc_stream.rs"] mod gc_stream;`) and
//! needs the `test-knobs` feature, as the depth-view knob does.

#![allow(dead_code)]

use crate::gc_scenes::*;
use core::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown};
use yakogl::vivante::depth_sample::{gc_debug_set_depth_tpu, DEPTH_TPU_OFF};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// One decoded stream item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
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
pub static CAPTURE: Mutex<Vec<Vec<Ev>>> = Mutex::new(Vec::new());
/// The stand-in's allocations as the wrapper saw them: (gpu, cpu, size).
pub static MEMS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

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
pub fn decode(words: &[u32]) -> Vec<Ev> {
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

/// Run `scene` with the depth-view knob in `mode` and return, for each of the
/// `FRAMES` frames, its decoded kicks and what `around` returned. `around`
/// is handed the frame's body (clear, draw, swap) and calls it once, so a
/// test can read a counter on both sides. The wrapper is installed before
/// `gc_init` (so it sees every allocation), the knob is set and put back
/// inside the run, and every run starts from clean captures; `SERIAL` is
/// held throughout.
pub fn frames_with<R>(
    mode: i32,
    scene: fn() -> Box<dyn FnMut()>,
    mut around: impl FnMut(&mut dyn FnMut()) -> R,
) -> Vec<(Vec<Vec<Ev>>, R)> {
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
            let extra = around(&mut || {
                canvas_clear();
                frame();
                swap();
            });
            no_error(&format!("frame {n}"));
            all.push((std::mem::take(&mut *CAPTURE.lock().unwrap_or_else(|e| e.into_inner())), extra));
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
