// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `samplers.{single,multi}_tex_3d.diff_*` (14 Fail rows on the
//! Pi 4, docs/notes/v3d-notpass-rest.md G6) on the V3D host path.
//!
//! The board log that shows the rows (tools/cts/log/2026-10-31-pi4-gles3-a)
//! predates 8ae0a44b ("2D-array and 3D look-ups ... pinned against Mesa"),
//! which made the online compiler accept `sampler3D`; the refusal the log
//! names, `NotYet("a texture look-up other than 2D or cube")`, is gone. So:
//!
//! * the shaders dEQP builds for these cases (written here after the case
//!   description: a vertex shader passing a vec3 coordinate, a fragment
//!   shader reading one or two `sampler3D` with `texture()`, plus the
//!   `textureLod()` and `textureProj()` forms) compile and link through
//!   `SHC_ONLINE`;
//! * their TMU programming is Mesa's v3d_tex.c sequence for a 3D texture:
//!   P0 and P1 (`wrtmuc`), T on `tmut`, R on `tmur`, S on `tmus` (the
//!   retiring write), the level on `tmuslod`, never `tmui` (the array layer)
//!   or `tmuscm` (the cube);
//! * the sampler-object state the cases vary (`diff_*`: wrap S/T/R, min/mag
//!   filter) reaches the V3D sampler record, and a 3D binding is not refused.
//!
//! What this file cannot show: the GL-core side (`GL_TEXTURE_3D` upload,
//! sampler-object precedence) and the draw itself need the C half of the
//! driver (`v3d_tex_state_from_layout`, `v3d_tex_layout`), which a plain
//! `cargo test` does not link; tests/depth_resolve_v3d.rs dlopens it. The
//! pixels are the board's (#320). Cause of the 14 rows as far as the host
//! shows it: the stale compiler refusal, fixed by 8ae0a44b; a Pi 4 rerun
//! decides whether anything else is left.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_samplers3d_v3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::c_char;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::listing::write_instr;
use yakogl::shaderc::online::{lower, sema};
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};
use yakogl::v3d::ffi::*;
use yakogl::v3d::texture;

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

fn compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 1024];
    let text = format!("{src}\0");
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, text.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(out)
    } else {
        Err(text_of(&log))
    }
}

/// The disassembly of both stages of the program `vs` + `fs`, from the V3D
/// code generator (the instructions `SHC_ONLINE` encodes into the record).
fn listings(vs: &str, fs: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut mem = vec![0u8; 16 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(vs).map_err(|_| "arena")?;
    let fs = a.alloc_str(fs).map_err(|_| "arena")?;
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| format!("vertex: {}", e.msg))?;
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs).map_err(|e| format!("fragment: {}", e.msg))?;
    let p = compile_program(&v.module, &f.module, Prim::Triangles, Target { vpm_size: 65536 }, &a)
        .map_err(|e| format!("program: {e:?}"))?;
    let text = |code: &[yakogl::shaderc::online::v3d::isa::Instr]| -> Vec<String> {
        code.iter()
            .map(|i| {
                let mut t = String::new();
                write_instr(&mut t, i).unwrap();
                t
            })
            .collect()
    };
    Ok((text(p.vs.code.instrs), text(p.fs.code.instrs)))
}

/// glCompileShader on both stages, then glLinkProgram; the two listings.
fn build(vs: &str, fs: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::shaderc::online::backend::forget_compiled();
    let vso = compile(SHC_STAGE_VERTEX, vs).map_err(|m| format!("vertex: {m}"))?;
    let mut fso = compile(SHC_STAGE_FRAGMENT, fs).map_err(|m| format!("fragment: {m}"))?;
    let link = yakogl::shaderc::online::SHC_ONLINE.link.expect("the online back end links");
    let mut log = [0 as c_char; 1024];
    // SAFETY: two distinct compiled shaders and a log of the size announced.
    let rc = unsafe { link(&vso, &mut fso, log.as_mut_ptr(), log.len()) };
    if rc != 0 {
        return Err(format!("link: {}", text_of(&log)));
    }
    listings(vs, fs)
}

/// dEQP's vertex shader of the samplers cases: a position and a texture
/// coordinate (a vec3 for a 3D texture) passed on.
const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec3 a_texCoord;\n\
                  out highp vec3 v_texCoord;\n\
                  void main() {\n  gl_Position = a_position;\n  v_texCoord = a_texCoord;\n}\n";

fn fs(body: &str, samplers: usize) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp sampler3D;\n");
    for k in 0..samplers {
        s += &format!("uniform sampler3D u_sampler{k};\n");
    }
    s + &format!(
        "in highp vec3 v_texCoord;\nlayout(location = 0) out highp vec4 o_color;\nvoid main() {{\n  {body}\n}}\n"
    )
}

fn count(ops: &[String], needle: &str) -> usize {
    ops.iter().filter(|o| o.contains(needle)).count()
}

fn at(ops: &[String], needle: &str) -> usize {
    ops.iter().position(|o| o.contains(needle)).unwrap_or_else(|| panic!("no `{needle}` in {ops:#?}"))
}

/// Mesa's TMU sequence of a non-array 3D look-up (v3d_tex.c
/// v3d_vir_emit_tex): P0 and P1 as `wrtmuc`, T, R, then S retires it.
fn assert_3d_sequence(what: &str, ops: &[String], retire: &str) {
    for w in ["tmut", "tmur", retire] {
        assert!(count(ops, w) >= 1, "{what}: no `{w}`: {ops:#?}");
    }
    assert!(at(ops, "tmut") < at(ops, retire) && at(ops, "tmur") < at(ops, retire), "{what}: S last: {ops:#?}");
    assert_eq!(count(ops, "tmui"), 0, "{what}: a 3D look-up has no array layer: {ops:#?}");
    assert_eq!(count(ops, "tmuscm"), 0, "{what}: a 3D look-up is not a cube: {ops:#?}");
    assert!(count(ops, "wrtmuc") >= 2, "{what}: P0 and P1: {ops:#?}");
    assert!(count(ops, "ldtmu") >= 1, "{what}: the result is read back: {ops:#?}");
}

/// samplers.single_tex_3d.*: one sampler3D, `texture()`.
#[test]
fn single_tex_3d_texture_compiles_with_mesas_tmu_sequence() {
    let (_, f) = build(VS, &fs("o_color = texture(u_sampler0, v_texCoord);", 1)).unwrap_or_else(|e| panic!("{e}"));
    assert_3d_sequence("texture", &f, "tmus,");
    assert_eq!(count(&f, "tmuslod"), 0, "an implicit level has no TMUSLOD: {f:#?}");
}

/// samplers.multi_tex_3d.*: two sampler3D units in one shader.
#[test]
fn multi_tex_3d_two_samplers_compile() {
    let body = "o_color = texture(u_sampler0, v_texCoord) * 0.5 + texture(u_sampler1, v_texCoord.zyx) * 0.5;";
    let (_, f) = build(VS, &fs(body, 2)).unwrap_or_else(|e| panic!("{e}"));
    assert_3d_sequence("multi", &f, "tmus,");
    assert_eq!(count(&f, "tmur"), 2, "an R write per look-up: {f:#?}");
    assert_eq!(count(&f, "tmut"), 2, "a T write per look-up: {f:#?}");
}

#[test]
fn texture_lod_and_proj_forms_compile() {
    // (name, body, the retiring write)
    let forms: [(&str, &str, &str); 4] = [
        ("textureLod", "o_color = textureLod(u_sampler0, v_texCoord, 1.0);", "tmuslod"),
        ("textureProj", "o_color = textureProj(u_sampler0, vec4(v_texCoord, 2.0));", "tmus,"),
        ("texture bias", "o_color = texture(u_sampler0, v_texCoord, -0.5);", "tmus,"),
        ("textureProjLod", "o_color = textureProjLod(u_sampler0, vec4(v_texCoord, 2.0), 1.0);", "tmuslod"),
    ];
    for (name, body, retire) in forms {
        let (_, f) = build(VS, &fs(body, 1)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_3d_sequence(name, &f, retire);
    }
}

#[test]
fn vertex_stage_look_up_is_at_level_zero() {
    // texture() in a vertex shader has no derivatives: Mesa's
    // lower_invalid_implicit_lod makes it a TMUSLOD look-up at level 0
    let vs = "#version 300 es\nuniform highp sampler3D u_sampler0;\nin highp vec4 a_position;\n\
              in highp vec3 a_texCoord;\nout highp vec4 v_color;\n\
              void main() {\n  gl_Position = a_position;\n  v_color = texture(u_sampler0, a_texCoord);\n}\n";
    let fs = "#version 300 es\nprecision highp float;\nin highp vec4 v_color;\nout highp vec4 o;\n\
              void main() { o = v_color; }\n";
    let (v, _) = build(vs, fs).unwrap_or_else(|e| panic!("{e}"));
    assert_3d_sequence("vertex texture", &v, "tmuslod");
    let vs_lod = vs.replace("texture(u_sampler0, a_texCoord)", "textureLod(u_sampler0, a_texCoord, 1.0)");
    let (v, _) = build(&vs_lod, fs).unwrap_or_else(|e| panic!("{e}"));
    assert_3d_sequence("vertex textureLod", &v, "tmuslod");
}

/// texelFetch, textureSize and textureOffset of a 3D sampler are the same
/// group's neighbours (texture.filtering.3d, texturesize).
#[test]
fn neighbours_of_the_group_compile() {
    let bodies = [
        "o_color = vec4(texelFetch(u_sampler0, ivec3(v_texCoord), 0));",
        "o_color = vec4(vec3(textureSize(u_sampler0, 1)), 1.0);",
        "o_color = textureOffset(u_sampler0, v_texCoord, ivec3(1, -2, 3));",
    ];
    for body in bodies {
        build(VS, &fs(body, 1)).unwrap_or_else(|e| panic!("{body}: {e}"));
    }
}

// ---- the descriptor side that needs no C half ----

fn bind3d() -> GlesTextureBind {
    GlesTextureBind {
        type_: GLES_TEXTYPE_3D,
        format: GLES_TEXFMT_RGBA8,
        layers: 4,
        width: 4,
        height: 4,
        stride_texels: 4,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    }
}

#[test]
fn a_3d_binding_is_sampled_not_refused() {
    let b = bind3d();
    assert_eq!(texture::refusal(&b), None);
    assert_eq!(texture::layers(&b), 4, "Image Depth is level 0's depth");
    // a 3D binding without slices is not a texture
    let flat = GlesTextureBind { layers: 0, ..b };
    assert!(texture::refusal(&flat).is_some());
}

/// The cases' `diff_*` axes: each sampler-object state reaches the V3D
/// sampler record, wrap R included.
#[test]
fn sampler_object_state_reaches_the_v3d_sampler() {
    let b = GlesTextureBind {
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_MIRRORED_REPEAT,
        wrap_r: GLES_WRAP_CLAMP_TO_EDGE,
        linear_filter: 0,
        min_linear: 1,
        mip_filter: 0,
        ..bind3d()
    };
    let s = texture::sampler(&b);
    assert_eq!((s.wrap_s, s.wrap_t, s.wrap_r), (V3D_WRAP_MODE_CLAMP, V3D_WRAP_MODE_MIRROR, V3D_WRAP_MODE_CLAMP));
    assert!(s.mag_nearest && !s.min_nearest && s.mip_none);
    let d = texture::sampler(&bind3d());
    assert_eq!((d.wrap_s, d.wrap_t, d.wrap_r), (V3D_WRAP_MODE_REPEAT, V3D_WRAP_MODE_REPEAT, V3D_WRAP_MODE_REPEAT));
    // two bindings of one texture under different sampler objects differ
    // only in the sampler record
    assert_ne!(s, d);
    assert_eq!(texture::tmu_format(&b), texture::tmu_format(&bind3d()));
}

/// Mesa's v3d_setup_slices depth rule (v3d_resource.c:609-639), which the
/// slice count of every mip level of a 3D texture follows.
#[test]
fn level_depths_follow_mesas_slice_rule() {
    assert_eq!(texture::level_depth(6, 0), 6);
    assert_eq!(texture::level_depth(6, 1), 4);
    assert_eq!(texture::level_depth(6, 2), 2);
    assert_eq!(texture::level_depth(6, 3), 1);
    assert_eq!(texture::level_depth(1, 1), 1);
}

/// Pinned expectation, owners `src/gl` (sampler objects) and `src/v3d`
/// texture.rs (3D layout): the GL path through a V3D host back end is not
/// reachable from this harness without the dlopen rig of
/// tests/depth_resolve_v3d.rs.
#[test]
#[ignore = "needs the dlopen rig of tests/depth_resolve_v3d.rs; owner src/gl (sampler objects), src/v3d texture.rs (3D layout)"]
fn gl_sampler_object_wins_over_texture_state_for_a_3d_unit() {
    unimplemented!(
        "glSamplerParameteri(WRAP_R, CLAMP) on a unit holding a REPEAT 3D texture reaches V3dSampler.wrap_r"
    );
}
