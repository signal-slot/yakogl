// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Compiles the cluster UI (../ui/cluster.slint) at build time. slint-build runs
//! the Slint compiler on the host and writes generated Rust into OUT_DIR, pulled in
//! by `slint::include_modules!()` in src/lib.rs.
//!
//! `EmbedForSoftwareRenderer` pre-renders every font face and size the .slint files
//! use into bitmap glyphs (registered by the generated code through
//! `Renderer::register_bitmap_font`) and decodes the images into raw textures. The
//! no_std target has neither a font stack nor an image decoder, so both come from
//! the compiler, the same way Slint runs on MCUs.
//!
//! SAFEUI_LAYOUT_DIR names the generated layout for the panel this image is for
//! (ui/<panel>/ in the SafeUI repository), put on the Slint include path so that
//! `import ... from "safeui-layout.slint"` resolves. The safety layer the safe
//! core draws over these frames is built from the same directory, which is what
//! keeps the two layers on top of each other.
fn main() {
    // CMake passes the absolute path (the manifest is generated into the build
    // directory, so a relative path would not find the sources).
    let ui = std::env::var("SLINT_UI_FILE").unwrap_or_else(|_| {
        format!("{}/../ui/cluster.slint", std::env::var("CARGO_MANIFEST_DIR").unwrap())
    });
    let layout = std::env::var("SAFEUI_LAYOUT_DIR")
        .expect("SAFEUI_LAYOUT_DIR must name the generated layout for this panel");
    println!("cargo:rerun-if-env-changed=SLINT_UI_FILE");
    println!("cargo:rerun-if-env-changed=SAFEUI_LAYOUT_DIR");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![layout.into()])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(&ui, config).expect("Slint compile failed");
}
