// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: the render layer at four colour targets (RC14,
//! docs/es3-cts-failures.md; plan item pvr-render). `PVR_RENDER_MAX_RT` is 4
//! in driver/src/pvr_render.h, src/osal/render.rs and
//! driver/rust/src/gpu/pvr/render.rs; the static end-of-tile PDS program
//! the firmware runs for every render moves a fourth PBE state only with the
//! driver's `pvr_render_eot_four_rt` (off by default).
//!
//! tests/pvr_render_rt4.c makes pvr_render.c's calls with the REAL
//! generators (driver/src/pvr_pds_pixel_event_gen.c,
//! driver/src/pvr_render_regs.c over Mesa 26.2.1) and pins: off, the
//! three-pair program and info word every 1, 2 and 3 target render ran with
//! (byte for byte the committed words); on, the four-pair program, whose
//! fourth DOUTW moves the fourth target's PBE state to the shared registers
//! the four-target end-of-tile program reads; and the four-target render's
//! DATA segment. The kick side (the fourth PBE pair in the 3D command only
//! for four targets, the 1..3 target commands unchanged, the HWRT data's
//! RTA size still three) is driver/rust/src/gpu/pvr/render.rs's
//! `rc14_the_fourth_pbe_pair_is_written_for_four_targets_only` and
//! `with_the_knob_the_hwrt_data_carries_the_partial_render_accounting`
//! (`cargo test -p yakogl-drivers --all-features gpu::pvr::render`). The
//! back end stays at three targets (src/pvr/backend.rs `PVR_BACKEND_MAX_RT`)
//! until the pvr-backend item.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_render_rt4
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use yakogl::osal::render::{PVR_RENDER_EOT_MAX_RT, PVR_RENDER_EOT_PDS_PAIRS, PVR_RENDER_MAX_RT};

#[test]
fn the_rust_mirror_has_four_targets_and_a_three_pair_default() {
    assert_eq!(PVR_RENDER_MAX_RT, 4);
    assert_eq!(PVR_RENDER_EOT_MAX_RT, PVR_RENDER_MAX_RT);
    assert_eq!(PVR_RENDER_EOT_PDS_PAIRS, 3);
}

#[test]
fn the_end_of_tile_pds_program_moves_a_fourth_state_only_when_asked() {
    let root = env!("CARGO_MANIFEST_DIR");
    let drv = "driver";
    let mesa = "third_party/mesa-26.2.1";
    let imagination = format!("{mesa}/src/imagination");
    let dirs = [
        format!("{drv}/src"),
        format!("{drv}/compat"),
        format!("{drv}/src/pvr/generated"),
        format!("{mesa}/include"),
        format!("{mesa}/src"),
        imagination.clone(),
        format!("{imagination}/include"),
        format!("{imagination}/common"),
        format!("{imagination}/vulkan"),
        format!("{imagination}/vulkan/pds"),
        format!("{imagination}/vulkan/pds/pvr_pds_programs"),
    ];
    let dir = std::env::temp_dir().join(format!("yakogl-pvr-render-rt4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("pvr_render_rt4");
    let cc = std::process::Command::new("cc")
        .args(["-std=c11", "-O1", "-Wall", "-Wextra", "-Werror"])
        .args(dirs.iter().map(|d| format!("-I{root}/{d}")))
        .arg(format!("{root}/tests/pvr_render_rt4.c"))
        .arg(format!("{root}/{drv}/src/pvr_render_regs.c"))
        .arg(format!("{root}/{drv}/src/pvr_pds_pixel_event_gen.c"))
        .arg(format!("{root}/{drv}/compat/pvr_mesa_pds.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    let out = std::process::Command::new(&exe).output().expect("pvr_render_rt4");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}");
    let last = text.lines().last().unwrap_or("");
    let (run, rest) =
        last.strip_prefix("pvr_render_rt4: ").and_then(|l| l.split_once(" checks run (planned ")).expect(&text);
    assert_eq!(rest, format!("{run}), 0 failed"), "{text}");
}
