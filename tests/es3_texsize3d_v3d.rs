// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D uniform words of a 3D depth and a 2D array's layer count
//! (`QUNIFORM_TEXTURE_DEPTH` 46, `QUNIFORM_TEXTURE_ARRAY_SIZE` 47), which the
//! online compiler emits and the C job builder does not write (plan #380,
//! host check of 2026-11-17 with tools/cts/host-deqp-run.sh, GPU=v3d,
//! MODULE=gles3, HOST-ONLY).
//!
//! The host dEQP run of `shaders.texture_functions.*` at 6d2e7d0d had every
//! case begin and no `NotYet(...)` refusal of the compiler left, but the draws
//! of `texturegrad.*3d_*` and `texturegradoffset.*3d_*` (16 cases) and of
//! `texturesize.sampler2darray_fixed_fragment` ended in
//!
//! ```text
//! yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or primitive it does not write)
//! yakogl: glDrawArrays was not recorded (-95); the frame is missing it
//! ```
//!
//! (`Attempt::Unsupported`, `-ENOTSUP`, GL error 0x502). The cause is what
//! this file pins, in the two halves it can reach without the C half linked:
//!
//! * the uniform stream of a 3D `textureSize`, a 2D array's `textureSize` and
//!   a 3D `textureGrad` (the gradient lanes are scaled by level 0's size,
//!   `TexSizeWord` lane 2) holds a word of contents 46 or 47, whose `data` is
//!   the sampler binding, as Mesa's v3d_uniforms.c would write it from the
//!   texture's depth or layer count;
//! * `driver/src/v3d/v3d_job.c`'s switch over the stream has cases for the
//!   width and the height (`V3D_QUNIFORM_TEXTURE_WIDTH` / `_HEIGHT`, from
//!   `struct v3d_job_tmu`'s `width` and `height`) and, since the fix of #1261,
//!   for 46 and 47 (`depth` and `array_size`, filled by `texture_units` in
//!   src/v3d/backend.rs); its `default:` is still `bad_field`, the refusal
//!   above, for a word it does not write. tools/v3d/job_check.c checks the
//!   values written. Before the fix every dEQP case whose shader reads a 3D
//!   depth or a 2D array's layer count drew nothing on the Pi 4;
//!   `the_job_builder_has_cases_for_depth_and_array_size` is the pin.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_texsize3d_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{
    QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH, QUNIFORM_TEXTURE_HEIGHT, QUNIFORM_TEXTURE_WIDTH,
};
use yakogl::shaderc::online::{lower, sema};

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec3 a_texCoord;\n\
                  out highp vec3 v_texCoord;\n\
                  void main() {\n  gl_Position = a_position;\n  v_texCoord = a_texCoord;\n}\n";

/// The fragment uniform stream, `(contents, data)`, of `VS` + a fragment
/// shader reading `sampler` `s` in `body`.
fn fragment_stream(sampler: &str, body: &str) -> Vec<(u32, u32)> {
    let fs = format!(
        "#version 300 es\nprecision highp float;\nprecision highp {sampler};\nuniform {sampler} s;\n\
         in highp vec3 v_texCoord;\nlayout(location = 0) out highp vec4 o;\nvoid main() {{ {body} }}\n"
    );
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).unwrap();
    let fs = a.alloc_str(&fs).unwrap();
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).unwrap_or_else(|e| panic!("vertex: {}", e.msg));
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).unwrap_or_else(|e| panic!("fragment: {}", e.msg));
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .unwrap_or_else(|e| panic!("program: {e:?}"));
    p.fs.code.uniforms.iter().map(|u| (u.contents, u.data)).collect()
}

fn contents(stream: &[(u32, u32)]) -> Vec<u32> {
    stream.iter().map(|&(c, _)| c).collect()
}

#[test]
fn a_3d_texture_size_reads_width_height_and_depth() {
    let s = fragment_stream("sampler3D", "o = vec4(vec3(textureSize(s, 1)), 1.0);");
    assert_eq!(
        s,
        [(QUNIFORM_TEXTURE_WIDTH, 0), (QUNIFORM_TEXTURE_HEIGHT, 0), (QUNIFORM_TEXTURE_DEPTH, 0)],
        "the sampler binding is `data`: {s:?}"
    );
}

#[test]
fn a_2d_array_texture_size_reads_width_height_and_layer_count() {
    let s = fragment_stream("sampler2DArray", "o = vec4(vec3(textureSize(s, 1)), 1.0);");
    assert_eq!(
        s,
        [(QUNIFORM_TEXTURE_WIDTH, 0), (QUNIFORM_TEXTURE_HEIGHT, 0), (QUNIFORM_TEXTURE_ARRAY_SIZE, 0)],
        "the sampler binding is `data`: {s:?}"
    );
}

#[test]
fn a_3d_texture_grad_scales_its_gradients_by_the_depth_too() {
    let s = fragment_stream("sampler3D", "o = textureGrad(s, v_texCoord, vec3(0.1), vec3(0.2));");
    assert!(contents(&s).contains(&QUNIFORM_TEXTURE_DEPTH), "no depth word: {s:?}");
}

#[test]
fn a_2d_texture_grad_reads_no_depth_or_layer_count() {
    let s = fragment_stream("sampler2D", "o = textureGrad(s, v_texCoord.xy, vec2(0.1), vec2(0.2));");
    let c = contents(&s);
    assert!(!c.contains(&QUNIFORM_TEXTURE_DEPTH) && !c.contains(&QUNIFORM_TEXTURE_ARRAY_SIZE), "{s:?}");
}

/// What the C job builder writes of the stream (driver/src/v3d/v3d_job.c).
#[test]
fn the_job_builder_has_cases_for_depth_and_array_size() {
    let src = include_str!("../driver/src/v3d/v3d_job.c");
    let sw = src.find("case V3D_QUNIFORM_TEXTURE_WIDTH:").expect("the width case");
    let rest = &src[sw..];
    let end = rest.find("put_u32(p + i * 4, v);").expect("the end of the uniform switch");
    let sw = &rest[..end];
    assert!(sw.contains("case V3D_QUNIFORM_TEXTURE_HEIGHT:"), "the height case");
    for case in ["case V3D_QUNIFORM_TEXTURE_DEPTH:", "case V3D_QUNIFORM_TEXTURE_ARRAY_SIZE:"] {
        assert!(sw.contains(case), "the job builder lacks `{case}`");
    }
    assert!(
        sw.contains("default:\n\t\t\tcl->bad_field = true;\n\t\t\treturn false;"),
        "a word no case writes is refused (bad_field): {sw}"
    );
    let header = include_str!("../driver/src/v3d/v3d_job.h");
    assert!(header.contains("V3D_QUNIFORM_TEXTURE_DEPTH      46"), "word 46");
    assert!(header.contains("V3D_QUNIFORM_TEXTURE_ARRAY_SIZE 47"), "word 47");
    let backend = include_str!("../src/v3d/backend.rs");
    assert!(
        backend.contains("the job builder refused the draw (a uniform, program or primitive it does not write)"),
        "the refusal text moved"
    );
}
