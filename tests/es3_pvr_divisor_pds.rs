// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: a per-instance stream can carry a GL divisor
//! (docs/notes/instanceid-per-draw.md, fix P3): the REAL generator
//! (driver/src/pvr_pds_render_gen.c) sets Mesa's stream `multiplier`/`shift`
//! as pvr_pipeline_pds.c:846-864 computes them. tests/es3_pvr_divisor_pds.c
//! checks that divisor 0 and 1 give the old words, a power of two is one
//! shift, 3 and 0xffff get Mesa's multiplier word, and the multiply and shift
//! equal integer division.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_divisor_pds
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

#[test]
fn the_vertex_program_divides_the_instance_number_per_stream() {
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
    let dir = std::env::temp_dir().join(format!("yakogl-divisor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("div_pds");
    let cc = std::process::Command::new("cc")
        .args(["-std=c11", "-O1"])
        .args(dirs.iter().map(|d| format!("-I{root}/{d}")))
        .arg(format!("{root}/tests/es3_pvr_divisor_pds.c"))
        .arg(format!("{root}/{drv}/src/pvr_pds_render_gen.c"))
        .arg(format!("{root}/{drv}/compat/pvr_mesa_pds.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    let out = std::process::Command::new(&exe).output().expect("div_pds");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}");
    // Mesa: shift = ceil(log2 d), multiplier = (2^(shift+31) + d - 1) / d.
    // Divisor 0 and 1 are today's words; a power of two is one SFTLP32 (one
    // code word, no multiplier); the others are two LIMM, a MAD and two
    // SFTLP64 (five code words); the data size never changes (the multiplier
    // takes the constant after the stride).
    let want = [
        "d=0 mult=0x0 dgrow=0 cgrow=0 same=1 div_ok=1",
        "d=1 mult=0x0 dgrow=0 cgrow=0 same=1 div_ok=1",
        "d=2 mult=0x0 dgrow=0 cgrow=1 same=0 div_ok=1",
        "d=3 mult=0xaaaaaaab dgrow=0 cgrow=5 same=0 div_ok=1",
        "d=4 mult=0x0 dgrow=0 cgrow=1 same=0 div_ok=1",
        "d=65535 mult=0x80008001 dgrow=0 cgrow=5 same=0 div_ok=1",
        "not_per_instance_ignored=1",
    ];
    let got: Vec<&str> = text.lines().collect();
    assert_eq!(got, want, "{text}");
}
