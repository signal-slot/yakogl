// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: the vertex PDS program can start gl_InstanceID at a per-draw base
//! (docs/notes/instanceid-per-draw.md, fix P1): the REAL generator
//! `pvr_pds_render_vertex_program_ids_base` (driver/src/pvr_pds_render_gen.c)
//! sets Mesa's `instance_id_modifier`. tests/es3_pvr_instanceid_pds.c checks
//! that base 0 gives the old words, that a base emits the constant and grows
//! the program, and that the base is ignored without an instance register.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_instanceid_pds
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

#[test]
fn the_vertex_program_adds_the_per_draw_instance_base() {
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
    let dir = std::env::temp_dir().join(format!("yakogl-instanceid-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("inst_pds");
    let cc = std::process::Command::new("cc")
        .args(["-std=c11", "-O1"])
        .args(dirs.iter().map(|d| format!("-I{root}/{d}")))
        .arg(format!("{root}/tests/es3_pvr_instanceid_pds.c"))
        .arg(format!("{root}/{drv}/src/pvr_pds_render_gen.c"))
        .arg(format!("{root}/{drv}/compat/pvr_mesa_pds.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    let out = std::process::Command::new(&exe).output().expect("inst_pds");
    let _ = std::fs::remove_dir_all(&dir);
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(out.status.success(), "{line}");
    let (head, tail) = line.split_once(" grow=").expect(&line);
    assert_eq!(head, "base0_same=1 modifier_in_data=1", "{line}");
    let (grow, rest) = tail.split_once(' ').expect(&line);
    assert_eq!(rest, "no_reg_ignored=1", "{line}");
    let (data, code) = grow.split_once(',').expect(&line);
    let (data, code): (u32, u32) = (data.parse().unwrap(), code.parse().unwrap());
    assert!((0..=2).contains(&data), "data grows by {data}");
    assert_eq!(code, 1, "code grows by one ADD32");
}
