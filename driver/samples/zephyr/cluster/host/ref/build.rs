// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host reference compiles the same UI as the image (../../rust/build.rs):
//! EmbedForSoftwareRenderer, so the images are decoded (and the SVGs
//! rasterised) at compile time as on the board, and the same font files and
//! sizes, set by run.sh.
fn main() {
    for var in
        ["SLINT_UI_FILE", "AGL_SLINT_CLUSTER_UI_DIR", "SLINT_DEFAULT_FONT", "SLINT_FONT_PATH", "SLINT_FONT_SIZES"]
    {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let need = |var: &str| std::env::var(var).unwrap_or_else(|_| panic!("{var} must be set (run.sh sets it)"));
    let ui = need("SLINT_UI_FILE");
    let cluster_ui = need("AGL_SLINT_CLUSTER_UI_DIR");
    println!("cargo:rerun-if-changed={cluster_ui}");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![cluster_ui.into()])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(&ui, config).expect("Slint compile failed");
}
