// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `gpu-v3d71` build's table (plan item feat71): the feature picks the
//! V3D 7.1 table `yakogl_backend_v3d71` (src/v3d/table71.rs) as the V3D
//! table `g_backend` links (src/backend.rs, `v3d_table`), not the 4.2 one,
//! and the skeleton's hooks refuse every draw and clear by name until
//! table71-draw records them (docs/notes/v3d71-table71.md).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table
//! ```
//!
//! A host-test build starts with `g_backend` null (a test installs its
//! table), so the pick is checked on `yakogl::backend::v3d_table`, the one
//! name both cfg arms of `g_backend` use. The refusal is checked in the log
//! the library writes through the OSAL (`osal::log_str`), and through the
//! GL core's own entries (`yakogl::backend::draw` and friends) with the
//! table installed.
#![cfg(feature = "gpu-v3d71")]

use std::ffi::{c_char, CStr};
use std::sync::{Mutex, MutexGuard};

use yakogl::backend::{self, GlesDraw, YakoglBackend, GLES_HEAP_DATA, GLES_HEAP_SHADER};
use yakogl::osal::{osal_set_ops, OsalOps};
use yakogl::v3d::table71::{self, yakogl_backend_v3d71, REFUSAL_PREFIX, REF_NO_RECORDING};

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOCK: Mutex<()> = Mutex::new(());

/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

fn osal() -> &'static OsalOps {
    // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
    let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
    Box::leak(Box::new(OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: None,
        gpu_free: None,
        mmu_map: None,
        map_phys: None,
        gpu_write: None,
        gpu_read: None,
        cache_flush: None,
        cache_invd: None,
        submit: None,
        wait: None,
        completed: None,
        reg_read: None,
        reg_write: None,
        query: None,
        log: Some(log),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    }))
}

/// The tests take turns: one OSAL, one `g_backend`, a fresh log epoch (so
/// the once-per-epoch refusal line is logged again) and an empty log each.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: statics; the lock serialises every test of the module.
    unsafe {
        osal_set_ops(osal());
        backend::g_backend = &yakogl_backend_v3d71;
    }
    yakogl::osal::new_log_epoch();
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn log_lines() -> Vec<String> {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn refusal_line() -> String {
    format!("{REFUSAL_PREFIX}{REF_NO_RECORDING}")
}

/// `gpu-v3d71` picks the 7.1 table, and the table is the Pi 5's by name.
#[test]
fn the_7_1_table_is_the_v3d_table_of_the_build() {
    let picked: &YakoglBackend = &backend::v3d_table;
    assert!(core::ptr::eq(picked, &yakogl_backend_v3d71), "gpu-v3d71 must pick yakogl_backend_v3d71");
    /* not the 4.2 table, checked by its name: naming the 4.2 static here
     * would link the 4.2 C job builder into this test (gate71's subject) */
    // SAFETY: `name` points at a static C string.
    let name = unsafe { CStr::from_ptr(picked.name) };
    assert_eq!(name.to_str().unwrap(), "Broadcom V3D 7.1 (Raspberry Pi 5)");
    assert_eq!(name, table71::NAME);
    assert!(!name.to_str().unwrap().contains("4.2"));
}

/// The constant answers of docs/notes/v3d71-table71.md section 1, and the
/// hooks the first cut leaves to the GL core.
#[test]
fn the_tables_answers_are_the_design_notes() {
    let t = &yakogl_backend_v3d71;
    assert_eq!(t.native_attrib_formats, backend::gles_attribf_bit(backend::GLES_ATTRIBF_FLOAT32));
    assert!(!t.rt_rgba8888_is_bgra);
    assert_eq!(t.mip_level_align, 0);
    assert!(t.compressed_blocks_in_rows);
    assert!(t.native_layered_textures);
    assert!(!t.front_facing_shader_variant);
    assert!(t.etc1_decoded_at_upload);
    assert_eq!(t.max_instance_divisor, 1);
    assert!(!t.samples_texel_copies);
    assert_eq!(t.linear_row_align, 0);
    assert!(t.uniforms_host_only);
    assert_eq!(t.max_samples, 0);
    for (hook, set) in [
        ("init", t.init.is_some()),
        ("shutdown", t.shutdown.is_some()),
        ("draw", t.draw.is_some()),
        ("flush", t.flush.is_some()),
        ("finish", t.finish.is_some()),
        ("finish_previous", t.finish_previous.is_some()),
        ("kick_id", t.kick_id.is_some()),
        ("kick_pending", t.kick_pending.is_some()),
        ("heap_base", t.heap_base.is_some()),
        ("counters", t.counters.is_some()),
        ("wait_kick", t.wait_kick.is_some()),
    ] {
        assert!(set, "{hook} must be set");
    }
    for (hook, set) in [
        ("texture_released", t.texture_released.is_some()),
        ("surface_swapped", t.surface_swapped.is_some()),
        ("texture_uploaded", t.texture_uploaded.is_some()),
        ("attachment_gone", t.attachment_gone.is_some()),
        ("depth_resolve", t.depth_resolve.is_some()),
        ("stencil_resolve", t.stencil_resolve.is_some()),
        ("ds_write", t.ds_write.is_some()),
        ("query_begin", t.query_begin.is_some()),
        ("query_end", t.query_end.is_some()),
        ("query_result", t.query_result.is_some()),
        ("generate_mipmap", t.generate_mipmap.is_some()),
        ("return_mipmap_chain", t.return_mipmap_chain.is_some()),
    ] {
        assert!(!set, "{hook} is None in the first cut");
    }
}

/// A draw and a clear-only draw through the GL core's entry are refused by
/// name with -ENOTSUP: one log line per epoch, every refusal counted.
#[test]
fn draws_and_clears_are_refused_by_name() {
    let _g = rig();
    assert_eq!(backend::init(), 0, "{:?}", log_lines());
    assert!(table71::inited());
    let before = table71::refused_draws();

    let mut draw = GlesDraw::ZERO;
    draw.index_count = 3;
    let mut clear = GlesDraw::ZERO;
    clear.do_clear = true;
    let enotsup = -yakogl::util::ENOTSUP;
    assert_eq!(backend::draw(&draw), enotsup);
    assert_eq!(backend::draw(&clear), enotsup);
    assert_eq!(table71::refused_draws(), before + 2);

    let lines = log_lines();
    let named: Vec<&String> = lines.iter().filter(|l| **l == refusal_line()).collect();
    assert_eq!(named.len(), 1, "one named refusal line per epoch: {lines:?}");
    assert!(REF_NO_RECORDING.starts_with("V3D 7.1: "), "the refusal names the 7.1 table");

    /* a new epoch logs it again */
    yakogl::osal::new_log_epoch();
    assert_eq!(backend::draw(&draw), enotsup);
    assert_eq!(log_lines().iter().filter(|l| **l == refusal_line()).count(), 2);
    backend::shutdown();
    assert!(!table71::inited());
}

/// With nothing ever recorded, the job hooks succeed and report no kick,
/// and nothing is logged for them.
#[test]
fn the_job_hooks_have_nothing_to_do() {
    let _g = rig();
    let t = &yakogl_backend_v3d71;
    // SAFETY: the table's hooks, called with their documented arguments.
    unsafe {
        assert_eq!(t.init.unwrap()(), 0);
        assert_eq!(t.flush.unwrap()(), 0);
        assert_eq!(t.finish.unwrap()(), 0);
        assert_eq!(t.finish_previous.unwrap()(), 0);
        assert_eq!(t.kick_id.unwrap()(), 0);
        assert!(!t.kick_pending.unwrap()(0));
        assert!(!t.kick_pending.unwrap()(1));
        assert_eq!(t.wait_kick.unwrap()(0), 0);
        assert_eq!(t.heap_base.unwrap()(GLES_HEAP_DATA), 0);
        assert_eq!(t.heap_base.unwrap()(GLES_HEAP_SHADER), 0);
        let (mut kicks, mut waits) = (7u32, 7u32);
        t.counters.unwrap()(&mut kicks, &mut waits);
        assert_eq!((kicks, waits), (0, 0));
        t.counters.unwrap()(core::ptr::null_mut(), core::ptr::null_mut());
        t.shutdown.unwrap()();
    }
    assert!(log_lines().is_empty(), "{:?}", log_lines());
}
