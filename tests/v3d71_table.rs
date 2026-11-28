// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The `gpu-v3d71` build's table (plan items feat71 and table71-draw): the
//! feature picks the V3D 7.1 table `yakogl_backend_v3d71`
//! (src/v3d/table71.rs) as the V3D table `g_backend` links (src/backend.rs,
//! `v3d_table`), not the 4.2 one; its constant answers are the design
//! note's (docs/notes/v3d71-table71.md section 1); its init checks the VPM
//! and the hub identity and takes the job arenas from the OSAL; a draw it
//! cannot record is refused by name; and with nothing recorded the job
//! hooks have nothing to submit. The jobs it records are
//! tests/v3d71_table_draw.rs's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d71,shaderc-online,no-shader-table --test v3d71_table
//! ```
//!
//! A host-test build starts with `g_backend` null (a test installs its
//! table), so the pick is checked on `yakogl::backend::v3d_table`, the one
//! name both cfg arms of `g_backend` use. The OSAL is the mock of
//! tests/v3d71_table_rig/mod.rs; a refusal is checked in the log the library
//! writes through it, and through the GL core's own entries
//! (`yakogl::backend::draw` and friends) with the table installed. No test
//! here builds a job, so none needs the C compiler.
#![cfg(feature = "gpu-v3d71")]

mod v3d71_table_rig;

use std::ffi::CStr;

use v3d71_table_rig::{log_lines, logs_with, osal_counts, rig};
use yakogl::backend::{self, GlesDraw, YakoglBackend, GLES_HEAP_DATA, GLES_HEAP_SHADER};
use yakogl::v3d::table71::{self, yakogl_backend_v3d71, REFUSAL_PREFIX};

/// glue71's name for a GlesDraw with neither a colour target nor a
/// depth/stencil buffer (`GlesDraw::ZERO` with a count or a clear).
const REF_NO_TARGET: &str = "glue71: V3D 7.1: a draw with no colour target and no depth/stencil buffer";

fn refusal_line() -> String {
    format!("{REFUSAL_PREFIX}{REF_NO_TARGET}")
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
/// hooks the first cut leaves to the GL core. P10 (src/v3d/table71tex.rs)
/// set the three texture hooks and samples copies made at draw time.
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
    /* the 4.2 table's 0xffff since backend71 draws instanced arrays (inst71) */
    assert_eq!(t.max_instance_divisor, 0xffff);
    assert!(t.samples_texel_copies);
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
        ("texture_released", t.texture_released.is_some()),
        ("generate_mipmap", t.generate_mipmap.is_some()),
        ("return_mipmap_chain", t.return_mipmap_chain.is_some()),
    ] {
        assert!(set, "{hook} must be set");
    }
    for (hook, set) in [
        ("surface_swapped", t.surface_swapped.is_some()),
        ("texture_uploaded", t.texture_uploaded.is_some()),
        ("attachment_gone", t.attachment_gone.is_some()),
        ("depth_resolve", t.depth_resolve.is_some()),
        ("stencil_resolve", t.stencil_resolve.is_some()),
        ("ds_write", t.ds_write.is_some()),
        ("query_begin", t.query_begin.is_some()),
        ("query_end", t.query_end.is_some()),
        ("query_result", t.query_result.is_some()),
    ] {
        assert!(!set, "{hook} is None in the first cut");
    }
}

/// Init takes the job arenas from the OSAL and says what it found; shutdown
/// gives every one of them back.
#[test]
fn init_checks_the_identity_and_takes_the_job_arenas() {
    let _g = rig();
    assert_eq!(backend::init(), 0, "{:?}", log_lines());
    assert!(table71::inited());
    let lines = logs_with("yakogl/v3d71: V3D 7.1.10; job arenas for 256 draws (the default)");
    assert_eq!(lines.len(), 1, "{:?}", log_lines());
    /* the binning list, the render list, the indirect buffer, the data
     * arena, the texture record arena */
    let sizes: Vec<usize> = v3d71_table_rig::live_allocs().iter().map(|a| a.1).collect();
    let bcl = (table71::BCL_FIXED + 256 * table71::BCL_BYTES_PER_DRAW) as usize;
    let ind = (table71::IND_FIXED + 256 * table71::IND_BYTES_PER_DRAW) as usize;
    assert_eq!(sizes, [bcl, table71::RCL_BYTES, ind, table71::DATA_BYTES, table71::RECS_BYTES]);
    assert_eq!(backend::init(), 0, "a second init is the first");
    assert_eq!(v3d71_table_rig::live_allocs().len(), 5);
    backend::shutdown();
    assert!(!table71::inited());
    assert!(v3d71_table_rig::live_allocs().is_empty(), "shutdown frees the arenas");
}

/// The platform's draw budget sizes the arenas.
#[test]
fn the_platforms_draw_budget_sizes_the_arenas() {
    let _g = rig();
    v3d71_table_rig::set_draws_per_job(16);
    assert_eq!(backend::init(), 0);
    assert_eq!(logs_with("job arenas for 16 draws (the platform's budget)").len(), 1, "{:?}", log_lines());
    let sizes: Vec<usize> = v3d71_table_rig::live_allocs().iter().map(|a| a.1).collect();
    assert_eq!(sizes[0], (table71::BCL_FIXED + 16 * table71::BCL_BYTES_PER_DRAW) as usize);
}

/// Only V3D 7.1 with IPREV >= 10 (BCM2712 D0) starts: anything else is
/// ident71's named refusal, and so are an OSAL without the identity query
/// and a VPM the programs are not compiled for.
#[test]
fn another_part_does_not_start_and_says_why() {
    for (ident, vpm, want) in [
        /* the Pi 4's hub: HUB_IDENT1 0x000e1124 (TVER 4, REV 2) */
        (Some([0x000e_1124, 0]), 65536, "V3D 4.2"),
        /* a 7.1 with IPREV 5 */
        (Some([0x0008_1117, 0x0002_0510]), 65536, "7.1.10 (BCM2712 D0) only"),
        (None, 65536, "the OSAL does not give the hub identity"),
        (Some(v3d71_table_rig::PI5_IDENT), 16384, "the VPM is Some(16384) bytes, the programs are compiled for 65536"),
    ] {
        let _g = rig();
        v3d71_table_rig::set_ident(ident);
        v3d71_table_rig::set_vpm(vpm);
        assert!(backend::init() < 0, "{ident:x?} {vpm}");
        assert!(!table71::inited());
        let lines = logs_with("not starting");
        assert_eq!(lines.len(), 1, "{:?}", log_lines());
        assert!(lines[0].starts_with("yakogl/v3d71: ") && lines[0].contains(want), "{lines:?}");
        assert!(v3d71_table_rig::live_allocs().is_empty(), "nothing is kept");
        /* and no draw is taken */
        let mut d = GlesDraw::ZERO;
        d.do_clear = true;
        assert_eq!(backend::draw(&d), -yakogl::util::ENODEV);
    }
}

/// A draw and a clear-only draw the table cannot record are refused by name
/// with -ENOTSUP: one log line per cause and epoch, every refusal counted.
#[test]
fn a_draw_without_a_target_is_refused_by_name() {
    let _g = rig();
    assert_eq!(backend::init(), 0, "{:?}", log_lines());
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

    /* a new epoch logs it again */
    yakogl::osal::new_log_epoch();
    assert_eq!(backend::draw(&draw), enotsup);
    assert_eq!(logs_with(REF_NO_TARGET).len(), 2);
    /* nothing was queued: a flush submits nothing */
    assert_eq!(backend::flush(), 0);
    assert_eq!(osal_counts(), (0, 0));
}

/// A product build has no recording path yet and says so by name.
#[test]
fn the_product_builds_refusal_names_the_host_test_only_modules() {
    assert!(table71::REF_NO_RECORDING.starts_with("V3D 7.1: "));
    assert!(table71::REF_NO_RECORDING.contains("host-test"));
}

/// With nothing recorded, the job hooks succeed and submit nothing; the
/// kick being recorded is the first one, pending until it has run
/// (src/backend.rs, `YakoglBackend::kick_id` and `kick_pending`).
#[test]
fn the_job_hooks_have_nothing_to_do() {
    let _g = rig();
    let t = &yakogl_backend_v3d71;
    // SAFETY: the table's hooks, called with their documented arguments.
    unsafe {
        /* before init: no kick, nothing pending */
        assert_eq!(t.kick_id.unwrap()(), 0);
        assert!(!t.kick_pending.unwrap()(1));
        assert_eq!(t.init.unwrap()(), 0);
        assert_eq!(t.flush.unwrap()(), 0);
        assert_eq!(t.finish.unwrap()(), 0);
        assert_eq!(t.finish_previous.unwrap()(), 0);
        assert_eq!(t.kick_id.unwrap()(), 1, "the next frame's kick");
        assert!(!t.kick_pending.unwrap()(0), "0 is no kick");
        assert!(t.kick_pending.unwrap()(1), "not submitted yet");
        assert_eq!(t.wait_kick.unwrap()(0), 0);
        assert_eq!(t.wait_kick.unwrap()(1), 0, "an empty frame: nothing to submit");
        assert_eq!(t.heap_base.unwrap()(GLES_HEAP_DATA), 0);
        assert_eq!(t.heap_base.unwrap()(GLES_HEAP_SHADER), 0);
        let (mut kicks, mut waits) = (7u32, 7u32);
        t.counters.unwrap()(&mut kicks, &mut waits);
        assert_eq!((kicks, waits), (0, 0));
        t.counters.unwrap()(core::ptr::null_mut(), core::ptr::null_mut());
        t.shutdown.unwrap()();
        assert_eq!(t.flush.unwrap()(), -yakogl::util::ENODEV, "after shutdown");
    }
    assert_eq!(osal_counts(), (0, 0), "no submit, no wait");
    assert!(logs_with("refused").is_empty(), "{:?}", log_lines());
}
