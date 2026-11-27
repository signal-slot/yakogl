// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `shaders.texture_functions` shaders that the PowerVR online
//! compiler once refused (class A: offsets on stacked and 3D textures, class
//! C: textureGrad / textureProjGrad on cube, 3D and 2D-array samplers, class
//! D: the remaining offset and gradient forms) compile now (commits
//! f9744f26 and 92b93c23). This is the compile-only pin: each representative
//! shader goes through the online chain in a fragment and a vertex stage,
//! with the layout switches (native array, native 3D) off and on, and must
//! not be refused. Linking and drawing are `es3_pvr_texfn`'s.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_texfn_compile
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/// The compiler keeps one arena and the switches are process-wide.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One shader through the online PowerVR chain, or the refusal.
fn compile(stage: Stage, native: bool, src: &str) -> Result<(), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    yakogl::shaderc::pvr_set_native_array(i32::from(native));
    yakogl::shaderc::pvr_set_native_3d(i32::from(native));
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a).map(|_| ())
        } else {
            compile_vertex(&l.module, &l.reflection, a).map(|_| ())
        }
        .map_err(|e| format!("chain: {e:?}"))
    })
    .unwrap()
}

/// dEQP's shape: one sampler `s`, one look-up into the output (fragment) or
/// a varying (vertex), the plain shader of the other stage not needed here.
fn frag(sampler: &str, e: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n\
         in vec4 v;\nout vec4 o;\nvoid main() {{ o = vec4({e}); }}\n"
    )
}

fn vert(sampler: &str, e: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n\
         in vec4 a;\nout vec4 r;\nvoid main() {{ r = vec4({e}); gl_Position = a; }}\n"
    )
}

/// (sampler, expression) per class. `v` / `a` are the vec4 input.
const CASES: &[(&str, &str, &str)] = &[
    // class A: offsets on 2D arrays and 3D textures
    ("A", "sampler2DArray", "textureOffset(s, v.xyz, ivec2(1, -2))"),
    ("A", "sampler2DArray", "textureLodOffset(s, v.xyz, 1.0, ivec2(-3, 2))"),
    ("A", "sampler3D", "textureOffset(s, v.xyz, ivec3(1, -2, 3))"),
    ("A", "sampler3D", "textureLodOffset(s, v.xyz, 1.0, ivec3(2, 1, -1))"),
    ("A", "sampler3D", "textureProjOffset(s, v, ivec3(1, 2, 3))"),
    ("A", "sampler3D", "textureProjLodOffset(s, v, 1.0, ivec3(1, 2, 3))"),
    ("A", "sampler2DArray", "vec4(texelFetchOffset(s, ivec3(v.xyz), 0, ivec2(1, 2)))"),
    ("A", "sampler3D", "vec4(texelFetchOffset(s, ivec3(v.xyz), 0, ivec3(1, 2, 3)))"),
    ("A", "sampler2D", "vec4(texelFetchOffset(s, ivec2(v.xy), 0, ivec2(1, 2)))"),
    // class C: gradients on cube, 3D and 2D array
    ("C", "samplerCube", "textureGrad(s, v.xyz, v.xyz, v.zyx)"),
    ("C", "sampler3D", "textureGrad(s, v.xyz, v.xyz, v.zyx)"),
    ("C", "sampler2DArray", "textureGrad(s, v.xyz, v.xy, v.yx)"),
    ("C", "sampler3D", "textureProjGrad(s, v, v.xyz, v.zyx)"),
    ("C", "sampler2D", "textureProjGrad(s, v.xyz, v.xy, v.yx)"),
    ("C", "isampler3D", "vec4(textureGrad(s, v.xyz, v.xyz, v.zyx))"),
    ("C", "usamplerCube", "vec4(textureGrad(s, v.xyz, v.xyz, v.zyx))"),
    // class D: gradients with offsets
    ("D", "sampler2DArray", "textureGradOffset(s, v.xyz, v.xy, v.yx, ivec2(1, -1))"),
    ("D", "sampler3D", "textureGradOffset(s, v.xyz, v.xyz, v.zyx, ivec3(1, -1, 2))"),
    ("D", "sampler2D", "textureGradOffset(s, v.xy, v.xy, v.yx, ivec2(1, -1))"),
    ("D", "sampler2D", "textureProjGradOffset(s, v.xyz, v.xy, v.yx, ivec2(1, -1))"),
    ("D", "sampler3D", "textureProjGradOffset(s, v, v.xyz, v.zyx, ivec3(1, -1, 2))"),
    ("D", "sampler2DArrayShadow", "vec4(textureGradOffset(s, v, v.xy, v.yx, ivec2(1, -1)))"),
    ("D", "sampler2DShadow", "vec4(textureProjGradOffset(s, v, v.xy, v.yx, ivec2(1, -1)))"),
];

fn sweep(stage: Stage, class: &str) -> Vec<String> {
    let mut refused = Vec::new();
    for &(c, sampler, e) in CASES.iter().filter(|c| c.0 == class) {
        for native in [false, true] {
            let src = if stage == Stage::Fragment {
                frag(sampler, e)
            } else {
                vert(sampler, &e.replace("v.", "a.").replace("(s, v,", "(s, a,").replace("v)", "a)"))
            };
            if let Err(m) = compile(stage, native, &src) {
                refused.push(format!("class {c} {sampler} `{e}` native={native}: {m}"));
            }
        }
    }
    refused
}

fn check(class: &str) {
    for stage in [Stage::Fragment, Stage::Vertex] {
        let r = sweep(stage, class);
        assert!(r.is_empty(), "{stage:?} refused:\n{}", r.join("\n"));
    }
}

#[test]
fn class_a_offsets_compile() {
    check("A");
}

#[test]
fn class_c_gradients_compile() {
    check("C");
}

#[test]
fn class_d_gradient_offsets_compile() {
    check("D");
}
