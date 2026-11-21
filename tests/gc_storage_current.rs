// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the Vivante arm of `backend::storage_current`
//! (`src/vivante/storage.rs`, docs/notes/gc-tiled-wire.md W10 groundwork).
//!
//! - With no shadow (every run today: nothing marks a lazy entry), the call
//!   is a no-op: no finish, no state change, no backend initialised.
//! - A shadow in `State::RsNewer` (W10's lazy entry) yields exactly one RS
//!   job, of the tiled to linear shape `tiled_rt::untile` emits (the W4
//!   frame-end write-back): SOURCE_TILED A8R8G8B8 to A8R8G8B8, the source
//!   stride the shadow's row stride times 4, the destination the storage at
//!   its pitch. Every other state yields none.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_storage_current
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use yakogl::backend::{storage_current, GlesTextureBind};
use yakogl::vivante::backend::gc_finishes;
use yakogl::vivante::cmd::GcCmd;
use yakogl::vivante::regs::*;
use yakogl::vivante::storage::record_resolve;
use yakogl::vivante::tiled_rt::{self, Spec};
use yakogl::vivante::wide_target::{self, State};

/// Every LOAD_STATE write of `words`, `(register, value)` in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                out.push((base + 4 * k as u32, words[i + 1 + k]));
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

/// The registers of each RS kick: (config, src, src stride, dst, dst
/// stride, window, clear control).
fn rs_jobs(w: &[(u32, u32)]) -> Vec<[u32; 7]> {
    let mut cur = [0u32; 7];
    let mut jobs = Vec::new();
    for &(r, v) in w {
        match r {
            GC_RS_CONFIG => cur[0] = v,
            GC_RS_SOURCE_STRIDE => cur[2] = v,
            GC_RS_DEST_STRIDE => cur[4] = v,
            GC_RS_WINDOW_SIZE => cur[5] = v,
            GC_RS_CLEAR_CONTROL => cur[6] = v,
            GC_RS_KICKER => jobs.push(cur),
            r if r == rs_pipe_source_addr(0) => cur[1] = v,
            r if r == rs_pipe_dest_addr(0) => cur[3] = v,
            _ => {}
        }
    }
    jobs
}

/// `state` recorded through `record_resolve` into a fresh stream: whether a
/// job was recorded, and the stream's words.
fn record(state: State, spec: &Spec, shadow: u64) -> (bool, Vec<u32>) {
    let mut buf = vec![0u32; 256];
    let mut c = GcCmd { words: buf.as_mut_ptr(), used: 0, cap: buf.len() as u32, overflow: false };
    let rec = record_resolve(&mut c, state, spec, shadow);
    assert!(!c.overflow);
    let used = c.used as usize;
    buf.truncate(used);
    (rec, buf)
}

#[test]
fn storage_current_with_no_shadow_is_a_no_op() {
    let gpu = 0x4800_0000u64;
    let before = gc_finishes();
    assert_eq!(wide_target::state_of(gpu), None);
    assert_eq!(wide_target::rs_newer(gpu), None);
    let b = GlesTextureBind { base_gpu: gpu, width: 256, height: 256, stride_texels: 256, ..GlesTextureBind::ZERO };
    storage_current(&b);
    assert_eq!(gc_finishes(), before, "no shadow: no finish");
    assert_eq!(wide_target::state_of(gpu), None);
    /* nothing marks a lazy entry today, at any address */
    assert!(!wide_target::mark_rs_newer(gpu, 1));
    assert_eq!(wide_target::rs_newer(gpu), None);
}

#[test]
fn a_shadow_marked_newer_yields_one_tiled_to_linear_rs_job() {
    /* terrain's 256x256 RGBA8 height map, the size W10 is for */
    let spec = Spec { gpu: 0x4800_0000, w: 256, h: 256, pitch: 1024 };
    let shadow = 0x4900_0000u64;
    let (rec, words) = record(State::RsNewer(3), &spec, shadow);
    assert!(rec);
    let jobs = rs_jobs(&writes(&words));
    assert_eq!(jobs.len(), 1, "exactly one RS job");
    let fmt = GC_RS_FORMAT_A8R8G8B8;
    assert_eq!(
        jobs[0],
        [
            rs_source_format(fmt) | rs_dest_format(fmt) | GC_RS_SOURCE_TILED,
            shadow as u32,
            tiled_rt::stride(spec.w) << 2,
            spec.gpu as u32,
            spec.pitch,
            rs_window_width(spec.w) | rs_window_height(spec.h),
            0,
        ]
    );
    assert_eq!(jobs[0][0] & GC_RS_DEST_TILED, 0, "the destination is the linear storage");
    assert_eq!(tiled_rt::stride(256), 1024);
}

#[test]
fn no_other_state_yields_an_rs_job() {
    let spec = Spec { gpu: 0x4800_0000, w: 256, h: 256, pitch: 1024 };
    for s in [State::StorageNewer, State::InStep, State::ShadowNewer(3)] {
        let (rec, words) = record(s, &spec, 0x4900_0000);
        assert!(!rec, "{s:?}");
        assert!(words.is_empty(), "{s:?}: nothing recorded");
    }
}
