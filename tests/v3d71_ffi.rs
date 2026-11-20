// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `src/v3d/ffi71.rs` (the Rust mirror of the V3D 7.1 C builder headers
//! `v3d_job71.h`, `v3d_tex71.h`, `v3d_tfu71.h`) and its C twin
//! `src/v3d/ffi71_layout_check.c`.
//!
//! Two sub-checks:
//!  - including `ffi71.rs` compiles its `const _` size and offset asserts;
//!  - the C twin is compiled against the REAL headers with the host C compiler
//!    (`$CC` or `cc`, `-fsyntax-only -std=c11 -Wall -Wextra -Werror
//!    -DV3D_CL_VER=71`, the include directories `tools/v3d/check71.d/job71.sh`
//!    uses): a header that moved fails its `_Static_assert`. Without a C
//!    compiler that sub-check prints a skip line and passes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_ffi
//! ```
#![allow(dead_code)]

#[path = "../src/v3d/ffi71.rs"]
mod ffi71;

use ffi71::*;
use std::process::Command;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// The host C compiler, or None when there is none.
fn find_cc() -> Option<String> {
    let cands = [std::env::var("CC").unwrap_or_default(), "cc".to_string()];
    for c in cands {
        if c.is_empty() {
            continue;
        }
        let mut parts = c.split_whitespace();
        let Some(prog) = parts.next() else { continue };
        if Command::new(prog).arg("--version").output().is_ok() {
            return Some(c);
        }
    }
    None
}

#[test]
fn rust_side_layout_is_compiled() {
    // The const asserts are the check; spot-check that the file's own
    // numbers are the ones this test names.
    assert_eq!(core::mem::size_of::<V3dJob71>(), 280);
    assert_eq!(core::mem::size_of::<V3dJob71Target>(), 212);
    assert_eq!(core::mem::size_of::<V3dJob71Clear>(), 148);
    assert_eq!(core::mem::size_of::<V3dTex71State>(), 52);
    assert_eq!(core::mem::size_of::<V3dTfu71Job>(), 48);
    assert_eq!(V3D_JOB71_MAX_RTS, 8);
    assert_eq!(V3D_JOB71_MAX_SIZE, 7680);
    let j = V3dJob71::ZERO;
    assert!(j.bcl.is_null() && !j.broken && j.t.rt.len() == V3D_JOB71_MAX_RTS);
    let l = V3dJob71DrawLayout::ZERO;
    assert_eq!(l.rec.len() * 4 + 20, 116);
    let cl = V3dCl::new(core::ptr::null_mut(), 0x1000, 64);
    assert_eq!((cl.gpu, cl.size, cl.used, cl.overflow), (0x1000, 64, 0, false));
}

#[test]
fn c_twin_compiles_against_the_headers() {
    let Some(cc) = find_cc() else {
        println!("skip: no C compiler (set CC or install cc): ffi71_layout_check.c not compiled");
        return;
    };
    let mesa = format!("{ROOT}/third_party/mesa-26.2.1");
    let mut parts = cc.split_whitespace();
    let prog = parts.next().unwrap();
    let out = Command::new(prog)
        .args(parts)
        .args(["-fsyntax-only", "-std=c11", "-Wall", "-Wextra", "-Werror", "-D_GNU_SOURCE", "-DHAVE_ENDIAN_H"])
        .arg("-DV3D_CL_VER=71")
        .arg(format!("-I{mesa}/src"))
        .arg(format!("-I{mesa}/include"))
        .arg(format!("-I{mesa}/src/broadcom"))
        .arg(format!("-I{ROOT}/driver/src/v3d"))
        .arg(format!("-I{ROOT}/driver/src/v3d/generated"))
        .arg(format!("{ROOT}/src/v3d/ffi71_layout_check.c"))
        .output()
        .expect("run the C compiler");
    assert!(
        out.status.success(),
        "ffi71_layout_check.c failed to compile against the 7.1 headers (a header moved: update ffi71.rs and both sets of asserts):\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    println!("compiled ffi71_layout_check.c with {cc}");
}

#[test]
fn c_twin_pins_every_rust_struct() {
    // Every `#[repr(C)] pub struct V3dJob71*`, `V3dTex71*`, `V3dTfu71*` of
    // ffi71.rs has a size pin in the C twin (the Rust side pins itself).
    let c = std::fs::read_to_string(format!("{ROOT}/src/v3d/ffi71_layout_check.c")).unwrap();
    for t in [
        "v3d_job71_rt",
        "v3d_job71_zs",
        "v3d_job71_target",
        "v3d_job71_stencil_face",
        "v3d_job71_state",
        "v3d_job71_tmu",
        "v3d_job71_draw",
        "v3d_job71_clear",
        "v3d_job71",
        "v3d_job71_draw_layout",
        "v3d_tex71_state",
        "v3d_tex71_sampler",
        "v3d_tex71_p2",
        "v3d_tfu71_job",
        "v3d_tfu71_surface",
        "v3d_tfu71_args",
    ] {
        assert!(c.contains(&format!("SZ({t},")), "no SZ pin for struct {t} in ffi71_layout_check.c");
    }
}
