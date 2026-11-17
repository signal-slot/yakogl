// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D refusal of the uniform words 46 and 47 and how narrow it is (plan
//! #380, issue #1261; images S32b `texturesize.sampler3d_*` /
//! `sampler2darray_*` and S32a's 3D `textureGrad` cases of
//! tools/cts/pi4-gles3-tree.txt).
//!
//! A program that needs a 3D texture's depth (`QUNIFORM_TEXTURE_DEPTH`, 46) or
//! a 2D array's layer count (`QUNIFORM_TEXTURE_ARRAY_SIZE`, 47) carries that
//! word in its uniform stream; `write_uniforms` in driver/src/v3d/v3d_job.c
//! has no case for it and its `default:` is `bad_field`. The draw is refused
//! with, verbatim,
//!
//! ```text
//! the job builder refused the draw (a uniform, program or primitive it does not write)
//! ```
//!
//! (`Attempt::Unsupported` after `v3d_job_draw` in src/v3d/backend.rs,
//! -ENOTSUP, which the GL core maps to GL_INVALID_OPERATION). The C half is not
//! linked into a host test, so the nearest host-visible points are pinned: the
//! text in src/v3d/backend.rs, the C switch's lack of a case, and the
//! compiled stream. tests/es3_texsize3d_v3d.rs pins the same cause from the
//! stream's side; this file adds the narrowness: a plain 3D `texture()`, a 2D
//! `textureSize` and a 2D `textureGrad` emit no such word and so are not
//! refused for it.
//!
//! The tests whose names end `_flip_when_words_46_47_are_written` are the ones
//! whoever makes the job builder write words 46/47 (#321's driver/src/v3d/**)
//! must flip; the control test stays.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_tex_words_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::{QUNIFORM_TEXTURE_ARRAY_SIZE, QUNIFORM_TEXTURE_DEPTH};
use yakogl::shaderc::online::{lower, sema};

/// The refusal text, verbatim (src/v3d/backend.rs).
const REFUSED: &str = "the job builder refused the draw (a uniform, program or primitive it does not write)";

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec3 a_texCoord;\n\
                  out highp vec3 v_texCoord;\n\
                  void main() {\n  gl_Position = a_position;\n  v_texCoord = a_texCoord;\n}\n";

/// The contents of the fragment uniform stream of `VS` + a fragment shader
/// reading `sampler` `s` in `body`.
fn fragment_contents(sampler: &str, body: &str) -> Vec<u32> {
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
    p.fs.code.uniforms.iter().map(|u| u.contents).collect()
}

fn needs_46_47(c: &[u32]) -> bool {
    c.contains(&QUNIFORM_TEXTURE_DEPTH) || c.contains(&QUNIFORM_TEXTURE_ARRAY_SIZE)
}

/// What the C switch over the stream writes: the part of v3d_job.c from the
/// first texture-size case to the store.
fn job_builder_switch() -> &'static str {
    let src = include_str!("../driver/src/v3d/v3d_job.c");
    let rest = &src[src.find("case V3D_QUNIFORM_TEXTURE_WIDTH:").expect("the width case")..];
    &rest[..rest.find("put_u32(p + i * 4, v);").expect("the store")]
}

#[test]
fn the_refusal_text_is_pinned() {
    let backend = include_str!("../src/v3d/backend.rs");
    let at = backend.find(REFUSED).expect("the refusal text moved");
    let before: String = backend[..at].split_whitespace().collect();
    assert!(
        before.ends_with("Err(Attempt::Unsupported(\""),
        "the text is an Unsupported refusal (-ENOTSUP, GL_INVALID_OPERATION)"
    );
}

#[test]
fn texture_size_of_a_3d_sampler_needs_word_46_flip_when_words_46_47_are_written() {
    let c = fragment_contents("sampler3D", "o = vec4(vec3(textureSize(s, 0)), 1.0);");
    assert!(c.contains(&QUNIFORM_TEXTURE_DEPTH), "{c:?}");
    let sw = job_builder_switch();
    assert!(!sw.contains("V3D_QUNIFORM_TEXTURE_DEPTH") && !sw.contains("case 46:"), "flip: 46 is written");
    assert!(sw.contains("default:\n\t\t\tcl->bad_field = true;"), "a word without a case is refused");
}

#[test]
fn texture_size_of_a_2d_array_needs_word_47_flip_when_words_46_47_are_written() {
    let c = fragment_contents("sampler2DArray", "o = vec4(vec3(textureSize(s, 0)), 1.0);");
    assert!(c.contains(&QUNIFORM_TEXTURE_ARRAY_SIZE), "{c:?}");
    let sw = job_builder_switch();
    assert!(!sw.contains("V3D_QUNIFORM_TEXTURE_ARRAY_SIZE") && !sw.contains("case 47:"), "flip: 47 is written");
}

#[test]
fn texture_grad_of_a_3d_sampler_needs_word_46_flip_when_words_46_47_are_written() {
    let c = fragment_contents("sampler3D", "o = textureGrad(s, v_texCoord, vec3(0.1), vec3(0.2));");
    assert!(c.contains(&QUNIFORM_TEXTURE_DEPTH), "{c:?}");
    assert!(!job_builder_switch().contains("case 46:"), "flip: 46 is written");
}

/// The control: the refusal is narrow. These programs name no word the job
/// builder lacks, so the draw is not refused for it.
#[test]
fn controls_without_words_46_47_are_not_refused_that_way() {
    let cases = [
        ("sampler3D", "o = texture(s, v_texCoord);"),
        ("sampler3D", "o = texelFetch(s, ivec3(1, 2, 0), 0);"),
        ("sampler2D", "o = vec4(vec2(textureSize(s, 0)), 0.0, 1.0);"),
        ("sampler2D", "o = textureGrad(s, v_texCoord.xy, vec2(0.1), vec2(0.2));"),
        ("sampler2DArray", "o = texture(s, v_texCoord);"),
    ];
    for (sampler, body) in cases {
        let c = fragment_contents(sampler, body);
        assert!(!needs_46_47(&c), "{sampler} `{body}` carries word 46/47: {c:?}");
    }
}
