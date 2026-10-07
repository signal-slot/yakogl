// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The one triangle sample's shader texts (driver/samples/zephyr/triangle,
//! check/src/shaders.rs) through this compiler, for the GPU of the build.
//!
//! The gate that a sample hands glShaderSource GLSL ES 1.00 this compiler
//! builds: table membership is not (92a360b8 drew from `#version 450` texts
//! that both offline tables held, and the Pi 4, whose only back end is this
//! compiler, refused them at `#version`).
use super::codegen::{self, CompileError};

use super::store;

#[path = "../../../driver/samples/zephyr/triangle/check/src/shaders.rs"]
mod triangle_shaders;
use triangle_shaders::{FS_SRC, VS_SRC};

/// The text without its NUL, as glShaderSource's NUL-terminated copy reaches
/// the compiler.
fn text(src: &'static str) -> &'static [u8] {
    src.strip_suffix('\0').expect("NUL-terminated").as_bytes()
}

/// The V3D compiles the pair as one program at glLinkProgram.
#[cfg(feature = "gpu-v3d")]
#[test]
fn the_triangle_sample_program_builds() {
    let _k = store::test_keep();
    let r = store::with_scratch(|s| {
        codegen::link_sources(text(VS_SRC), None, crate::shaderc::LodKey::NONE, text(FS_SRC), &[], s, store::keep())
            .map(|(sh, _)| sh.usc_dw)
    })
    .expect("the scratch region is free");
    assert!(matches!(r, Ok(n) if n > 0), "{r:?}");
}

/// One stage through `codegen::compile`.
#[cfg(not(feature = "gpu-v3d"))]
fn compile(stage: u32, src: &'static str) -> Result<u32, CompileError> {
    let _k = store::test_keep();
    store::with_scratch(|s| codegen::compile(stage, text(src), 0, s, store::keep()).map(|sh| sh.usc_dw))
        .expect("the scratch region is free")
}

/// The GC7000 and the PowerVR compile both stages. (On the GC7000 a pure
/// copy may legitimately be zero instructions, SHADERC_NOTES, so Ok is the
/// gate there; the PowerVR builds code for both.)
#[cfg(not(feature = "gpu-v3d"))]
#[test]
fn the_triangle_sample_shaders_build() {
    let vs = compile(crate::shaderc::SHC_STAGE_VERTEX, VS_SRC);
    let fs = compile(crate::shaderc::SHC_STAGE_FRAGMENT, FS_SRC);
    assert!(vs.is_ok() && fs.is_ok(), "vs {vs:?}, fs {fs:?}");
    #[cfg(not(feature = "gpu-vivante"))]
    assert!(matches!((vs, fs), (Ok(v), Ok(f)) if v > 0 && f > 0), "the PowerVR has code for both stages");
}

/// The gate can fail: the `#version 450` texts 92a360b8 drew from are
/// refused at `#version` by the front end, on every GPU of this compiler.
#[test]
fn the_desktop_glsl_texts_are_refused() {
    static VS_450: &str = concat!(include_str!("../../../tools/shaders/passthrough.vert"), "\0");
    let _k = store::test_keep();
    let r = store::with_scratch(|s| {
        codegen::compile(crate::shaderc::SHC_STAGE_VERTEX, text(VS_450), 0, s, store::keep()).map(|sh| sh.usc_dw)
    })
    .expect("the scratch region is free");
    assert!(matches!(r, Err(CompileError::Rejected { msg, .. }) if msg.contains("desktop GLSL")), "{r:?}");
}
