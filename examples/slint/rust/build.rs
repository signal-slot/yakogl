// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Compiles the cluster UI (SLINT_UI_FILE, the one cluster.slint) at build
//! time; slint-build writes the generated Rust into OUT_DIR, pulled in by
//! `slint::include_modules!()` in src/lib.rs. The same steps as
//! gl_slint/rust/build.rs, which goes when gl_slint does (one-method.md,
//! section 5, step 1).
//!
//! `EmbedForSoftwareRenderer` pre-renders every font face and size the .slint
//! files use into bitmap glyphs and decodes the images into raw textures: the
//! no_std target has neither a font stack nor an image decoder.
//!
//! CLUSTER_LAYOUT_DIR names the generated layout for the panel (ui/1024x600
//! in the SafeUI tree, the same on every board), put on the Slint include
//! path so that `import ... from "cluster-layout.slint"` resolves.
/// The build inputs the image reports ("slint-build: ..." at boot, src/main.c):
/// the line build-inputs.cmake wrote (the Slint, femtovg and glow trees and the
/// lock) and the rustc that compiles this crate. It goes into the image as the
/// compile-time string `SLINT_BUILD_LINE` (src/lib.rs). The rendering does not
/// read it.
fn build_line() -> String {
    let file = std::env::var("SLINT_BUILD_INPUTS_FILE")
        .expect("SLINT_BUILD_INPUTS_FILE must name the file build-inputs.cmake writes (CMake passes it)");
    println!("cargo:rerun-if-env-changed=SLINT_BUILD_INPUTS_FILE");
    println!("cargo:rerun-if-changed={file}");
    let inputs = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("cannot read {file}: {e}"));
    // `rustc -vV`: "rustc 1.97.0-nightly (hash date)" is its first line.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let out = std::process::Command::new(&rustc).arg("-vV").output().unwrap_or_else(|e| panic!("cannot run {rustc}: {e}"));
    let version = String::from_utf8_lossy(&out.stdout);
    let first = version.lines().next().unwrap_or("").trim();
    let rustc_id = first.strip_prefix("rustc ").unwrap_or("unknown");
    format!("{} rustc {rustc_id}", inputs.trim())
}

fn main() {
    println!("cargo:rustc-env=SLINT_BUILD_LINE={}", build_line());
    let ui = std::env::var("SLINT_UI_FILE").expect("SLINT_UI_FILE must name cluster.slint (CMake passes it)");
    let layout =
        std::env::var("CLUSTER_LAYOUT_DIR").expect("CLUSTER_LAYOUT_DIR must name the generated layout for this panel");
    println!("cargo:rerun-if-env-changed=SLINT_UI_FILE");
    println!("cargo:rerun-if-env-changed=CLUSTER_LAYOUT_DIR");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![layout.into()])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(&ui, config).expect("Slint compile failed");
}
