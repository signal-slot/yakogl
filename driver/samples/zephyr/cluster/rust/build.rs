// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Compiles the agl-slint-cluster UI at build time, as gl_slint's build.rs
//! does (EmbedForSoftwareRenderer: glyphs pre-rendered to bitmaps, images
//! decoded; the no_std target has neither a font stack nor an image decoder).
//!
//! SLINT_UI_FILE is ../ui/zephyr-cluster.slint, which imports the cluster's
//! `ui/app-window.slint` through the Slint include path
//! AGL_SLINT_CLUSTER_UI_DIR (the cluster's `ui/` directory; the clone is not
//! part of this repository).
//!
//! The fonts are named, not taken from the host's defaults (the host's
//! default sans-serif was Noto Sans CJK KR on the first compile):
//! - SLINT_DEFAULT_FONT: NotoSans-Regular.ttf;
//! - SLINT_FONT_PATH: NotoSans-Light.ttf and NotoSans-ExtraLight.ttf, the
//!   faces for the UI's weights 300 and 200;
//! - SLINT_FONT_SIZES: every pixel size the UI uses. The cluster passes most
//!   sizes through component properties, which the compiler cannot read, and
//!   the FemtoVG path scales the nearest smaller embedded size up (the 82.5 px
//!   speed digits would otherwise be 17 px glyphs five times enlarged).
//! CMake sets all of them; each is declared rerun-if-env-changed, because
//! cargo reuses the old output otherwise and a font change would not show.
fn main() {
    for var in
        ["SLINT_UI_FILE", "AGL_SLINT_CLUSTER_UI_DIR", "SLINT_DEFAULT_FONT", "SLINT_FONT_PATH", "SLINT_FONT_SIZES"]
    {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let need = |var: &str| std::env::var(var).unwrap_or_else(|_| panic!("{var} must be set (CMakeLists.txt sets it)"));
    let ui = need("SLINT_UI_FILE");
    let cluster_ui = need("AGL_SLINT_CLUSTER_UI_DIR");
    for var in ["SLINT_DEFAULT_FONT", "SLINT_FONT_PATH", "SLINT_FONT_SIZES"] {
        need(var);
    }
    println!("cargo:rerun-if-changed={cluster_ui}");
    let config = slint_build::CompilerConfiguration::new()
        .with_include_paths(vec![cluster_ui.into()])
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer);
    slint_build::compile_with_config(&ui, config).expect("Slint compile failed");
}
