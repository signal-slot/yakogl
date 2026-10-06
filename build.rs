// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The C the crate compiles itself, per GPU backend:
//!
//! * `gpu-v3d`: Mesa 26.2.1's QPU packer
//!   (`third_party/mesa-26.2.1/src/broadcom/qpu/{qpu_pack.c,qpu_instr.c}`,
//!   unmodified) and its adapter `src/shaderc/online/v3d/qpu_mesa.c`
//!   (docs/source-form.md, F1).
//! * `gpu-powervr`: the backend's csbgen packers.
//!   src/pvr/backend.rs packs its texture, PBE and fragment-register words
//!   with Mesa 26.2.1's csbgen packers (driver/src/pvr/generated/csbgen/rogue/,
//!   Mesa's generator over the vendored XML). Those are static inline C
//!   functions; bindgen's `--wrap-static-fns` made non-static wrappers for the
//!   ones the backend calls (driver/src/pvr/generated/csbgen_wrap.c, committed
//!   with its Rust side src/pvr/csbgen.rs by `yakogl-tools csbgen-bindings`),
//!   and this script compiles them. Nothing here runs bindgen: no build needs
//!   libclang.
//!
//! A build compiles only its own backend's C, so no image carries another
//! GPU's. The GC7000 build compiles no C here.
//!
//! One rule for every target: the host tests, the Zephyr archive and a
//! bare-metal one all go through this script, and the C build systems do not
//! compile these files a second time. The compiler is the `cc` crate's
//! choice, which the C builds steer through its environment variables
//! (`CC_<target>`, `CFLAGS_<target>`, `AR_<target>`; capi/yakogl_capi.cmake
//! passes the Zephyr toolchain's, baremetal.mk its own). On a target with no
//! operating system (`target_os = "none"`) the files are compiled
//! freestanding, with the few C library declarations they include taken from
//! a `freestanding/` directory of the backend's (`src/shaderc/online/v3d/`,
//! `driver/compat/`; the definitions are the platform's: Zephyr's libc, or the
//! bare-metal application's), and, when nothing names a compiler, with clang
//! (`--target` from the Rust triple), since a host has no `aarch64-none-elf-gcc`
//! as a rule. NDEBUG as Mesa's release builds (V3D: on a target with no
//! operating system; PowerVR: on every target, so a packer shifts a value into
//! its field without asserting its width, which is what the hand-written words
//! it replaced did).

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    mesa_reference();
    precompiled_table();
    if env::var_os("CARGO_FEATURE_GPU_V3D").is_some() {
        v3d_qpu();
    }
    if env::var_os("CARGO_FEATURE_GPU_POWERVR").is_some() {
        pvr_csbgen();
    }
}

/// `cfg(mesa_reference)`: the Mesa reference table (`tests/reference/`) is
/// compiled into this build. Two builds take it: one that asks for it as its
/// shader table (`mesa-reference-table`, the measurement images), and the host
/// tests (`host-test`), which compare our compiler with it, unless they ask
/// for no table (`no-shader-table`). No other build compiles it, so no other
/// product build can link a Mesa-built row (tools/check.sh, "no Mesa
/// reference in the artefact").
fn mesa_reference() {
    println!("cargo::rustc-check-cfg=cfg(mesa_reference)");
    let on = |f: &str| env::var_os(format!("CARGO_FEATURE_{f}")).is_some();
    if on("MESA_REFERENCE_TABLE") || (on("HOST_TEST") && !on("NO_SHADER_TABLE")) {
        println!("cargo:rustc-cfg=mesa_reference");
    }
}

/// `precompiled-shaders`: the table yakogl-precompile wrote, named by
/// YAKOGL_PRECOMPILED_TABLE, compiled in as `shaderc::online::precompiled::
/// table` through `$OUT_DIR/precompiled_table.rs` (one `#[path]` module line:
/// the table is a module file with inner attributes, which `include!` cannot
/// take). Rebuilt when the variable or the file changes.
fn precompiled_table() {
    println!("cargo:rerun-if-env-changed=YAKOGL_PRECOMPILED_TABLE");
    if env::var_os("CARGO_FEATURE_PRECOMPILED_SHADERS").is_none() {
        return;
    }
    let Some(path) = env::var_os("YAKOGL_PRECOMPILED_TABLE") else {
        panic!(
            "feature `precompiled-shaders` needs YAKOGL_PRECOMPILED_TABLE: the table yakogl-precompile wrote \
             (cargo run -p yakogl-precompile --features gpu-<gpu> -- -o <file>; tools/README.md)"
        );
    };
    let path = PathBuf::from(path);
    let abs = path.canonicalize().unwrap_or_else(|e| panic!("YAKOGL_PRECOMPILED_TABLE={}: {e}", path.display()));
    let text = abs.to_str().unwrap_or_else(|| panic!("YAKOGL_PRECOMPILED_TABLE={}: not UTF-8", abs.display()));
    assert!(!text.contains('"'), "YAKOGL_PRECOMPILED_TABLE={text}: a path with a quote");
    println!("cargo:rerun-if-changed={text}");
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("precompiled_table.rs");
    std::fs::write(&out, format!("#[path = \"{text}\"]\nmod table;\n"))
        .unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

fn root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
}

/// Freestanding flags and the default compiler for a target with no
/// operating system; nothing on a hosted one.
fn freestanding(b: &mut cc::Build, decls: &Path) {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "none" {
        return;
    }
    b.include(decls).flag("-ffreestanding").flag("-fno-stack-protector");
    let target = env::var("TARGET").expect("cargo sets TARGET");
    let named = [format!("CC_{target}"), format!("CC_{}", target.replace('-', "_")), "TARGET_CC".to_string()]
        .iter()
        .any(|v| env::var_os(v).is_some());
    if !named {
        b.compiler("clang");
    }
}

fn v3d_qpu() {
    let root = root();
    let mesa = root.join("third_party/mesa-26.2.1");
    let adapter = root.join("src/shaderc/online/v3d");
    let files = [
        mesa.join("src/broadcom/qpu/qpu_pack.c"),
        mesa.join("src/broadcom/qpu/qpu_instr.c"),
        adapter.join("qpu_mesa.c"),
    ];
    for dep in files.iter().cloned().chain([
        mesa.join("src/broadcom/qpu/qpu_instr.h"),
        mesa.join("src/broadcom/common/v3d_device_info.h"),
        mesa.join("src/util"),
        mesa.join("include"),
        adapter.join("freestanding"),
    ]) {
        println!("cargo:rerun-if-changed={}", dep.display());
    }

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let mut b = cc::Build::new();
    b.files(&files)
        .include(mesa.join("src"))
        .include(mesa.join("include"))
        // What Mesa's meson.build finds on every compiler we use; without
        // them util/bitscan.h declares ffs()/ffsll() as library calls.
        .define("HAVE___BUILTIN_FFS", None)
        .define("HAVE___BUILTIN_FFSLL", None)
        .std("gnu11")
        .warnings(true)
        .extra_warnings(false)
        // Mesa's own flag for these files (src/broadcom/qpu/meson.build,
        // no_override_init_args): qpu_instr.c's designated initialisers
        // override on purpose. gcc and clang both take this spelling.
        .flag("-Wno-override-init");
    if os == "none" {
        b.define("NDEBUG", None);
    }
    freestanding(&mut b, &adapter.join("freestanding"));
    b.compile("yakogl_v3d_qpu");
}

fn pvr_csbgen() {
    let root = root();
    let mesa = root.join("third_party/mesa-26.2.1");
    let compat = root.join("driver/compat");
    let generated = root.join("driver/src/pvr/generated");
    let wrap = generated.join("csbgen_wrap.c");
    for dep in [
        wrap.clone(),
        generated.join("csbgen"),
        compat.join("pvr_csbgen_rs.h"),
        compat.join("pvr_mesa.h"),
        compat.join("freestanding"),
        mesa.join("src/imagination/csbgen/pvr_packet_helpers.h"),
        mesa.join("src/imagination/include/pvr_types.h"),
    ] {
        println!("cargo:rerun-if-changed={}", dep.display());
    }

    let mut b = cc::Build::new();
    b.file(&wrap)
        .include(&compat)
        .include(&generated)
        .include(mesa.join("src/imagination"))
        .include(mesa.join("src"))
        .include(mesa.join("include"))
        .define("NDEBUG", None)
        .std("gnu11")
        .warnings(true)
        .extra_warnings(false);
    freestanding(&mut b, &compat.join("freestanding"));
    b.compile("yakogl_pvr_csbgen");
}
