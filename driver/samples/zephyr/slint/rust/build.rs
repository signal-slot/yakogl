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
//! SAFEUI_LAYOUT_DIR names the generated layout for the panel (ui/1024x600
//! in the SafeUI tree, the same on every board), put on the Slint include
//! path so that `import ... from "safeui-layout.slint"` resolves.
fn main() {
    let ui = std::env::var("SLINT_UI_FILE").expect("SLINT_UI_FILE must name cluster.slint (CMake passes it)");
    let layout =
        std::env::var("SAFEUI_LAYOUT_DIR").expect("SAFEUI_LAYOUT_DIR must name the generated layout for this panel");
    println!("cargo:rerun-if-env-changed=SLINT_UI_FILE");
    println!("cargo:rerun-if-env-changed=SAFEUI_LAYOUT_DIR");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![layout.into()])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(&ui, config).expect("Slint compile failed");
}
