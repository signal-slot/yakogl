// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): the two-plane split of a 128-bit look-up on every
//! sampler kind and look-up form the compiler reaches
//! (docs/notes/gc-128bit-texture-sampling.md, docs/notes/gc-128bit-swizzle.md).
//!
//! tests/es3_wide128_gc.rs pins the entry on a plain `sampler2D` /
//! `usampler2D` `texture()` call. This file drives the same public
//! `yakogl::shaderc::{wide128_sensitivity, wide128_planes, wide128_variant}`
//! on the rest, and for each form checks that the sensitivity names the
//! binding and that the keyed variant has two look-ups of the same opcode
//! (the second on the companion slot) where the plain shader has one:
//!
//! - sampler kinds: `sampler3D`, `samplerCube`, `sampler2DArray`,
//!   `isampler2D`, `isampler3D`, `usamplerCube`, `usampler2DArray`;
//! - forms on `sampler2D`: bias, `textureLod`, `texelFetch`, `textureProj`
//!   (3 and 4 components) in the fragment stage, and `texture`,
//!   `textureLod` and `texelFetch` in the vertex stage;
//! - two keyed bindings in one shader, and one keyed beside one that is not;
//! - `textureSize` alone, which does not make the binding sensitive.
//!
//! Finding pinned as today's behaviour: `textureOffset`,
//! `texelFetchOffset` and `textureGrad` on a `sampler2D` are refused by the
//! plain compile (the code generator does not select them yet), so no
//! shader exists to key. The refusal text is pinned; when the selector
//! learns them these cases move to the split check.
//!
//! What it cannot see: the GPU, the descriptors, and the GL core's draw. The
//! join's arithmetic is read only as look-up and word counts.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_wide128_join_targets_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use std::sync::Mutex;
use yakogl::shaderc::online::backend::{forget_compiled, SHC_ONLINE};
use yakogl::shaderc::online::vivante::decode::decode_instr;
use yakogl::shaderc::{
    release_shader, wide128_planes, wide128_sensitivity, wide128_variant, LodKey, ShcShader, Wide128Planes,
    SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX,
};

/// Held across every compile and every read of its code: the plane key is
/// process-wide, and `forget_compiled` resets the arena the code lives in.
static COMPILING: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    forget_compiled();
    g
}

fn text(log: &[core::ffi::c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
}

/// `src` through the online compiler: the shader, or the refusal's log.
fn try_compile(stage: u32, src: &str) -> Result<ShcShader, String> {
    let src = std::ffi::CString::new(src).unwrap();
    let mut sh = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    let compile = SHC_ONLINE.compile.expect("the online back end compiles");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, src.as_ptr(), &mut sh, log.as_mut_ptr(), log.len()) };
    if rc == 0 {
        Ok(sh)
    } else {
        Err(text(&log))
    }
}

fn compile(stage: u32, src: &str) -> ShcShader {
    try_compile(stage, src).unwrap_or_else(|e| panic!("{e}"))
}

/// `sh` under `planes` (and no sampler-state key), or the refusal's log.
fn variant(sh: &ShcShader, planes: Wide128Planes) -> Result<ShcShader, String> {
    let mut out = ShcShader::ZERO;
    let mut log = [0 as core::ffi::c_char; 512];
    match wide128_variant(sh, planes, LodKey::NONE, &mut out, &mut log) {
        0 => Ok(out),
        rc => {
            assert!(rc < 0, "{rc}");
            Err(text(&log))
        }
    }
}

/// The opcode and `TEX_ID` of every look-up of `sh`, in program order.
fn units(sh: &ShcShader) -> Vec<(&'static str, u32)> {
    sh.usc_words()
        .chunks(4)
        .map(|c| decode_instr(&c.try_into().unwrap()).expect("decodes"))
        .filter(|i| i.op().name.starts_with("tex") || i.op().name == "txf")
        .map(|i| (i.op().name, i.get("TEX_ID").expect("a look-up names its unit")))
        .collect()
}

/// A fragment shader with the varyings `uv`, `uv3` and `uv4` declared.
fn fs(decl: &str, out: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nin vec2 uv;\nin vec3 uv3;\nin vec4 uv4;\n{decl}\n\
         out highp {out} o;\nvoid main() {{ {body} }}\n"
    )
}

fn vs(decl: &str, out: &str, body: &str) -> String {
    format!(
        "#version 300 es\n{decl}\nin vec4 p;\nflat out highp {out} v;\n\
         void main() {{ {body} gl_Position = p; }}\n"
    )
}

type Units = Vec<(&'static str, u32)>;

/// Binding 0's look-up of `src` under a key putting its second plane at
/// companion slot 5: the plain look-ups, the split ones, and the plain and
/// split code sizes in words.
fn split0(stage: u32, src: &str) -> (Units, Units, usize, usize) {
    let sh = compile(stage, src);
    let sense = wide128_sensitivity(&sh);
    assert_eq!(sense, 1, "the look-up makes binding 0 sensitive");
    let plain = units(&sh);
    let v = variant(&sh, wide128_planes(sense, &[Some(5)])).expect("the keyed variant compiles");
    let split = units(&v);
    let words = (sh.usc_words().len(), v.usc_words().len());
    assert_eq!(units(&sh), plain, "the program's own shader is untouched");
    release_shader(&v);
    release_shader(&sh);
    (plain, split, words.0, words.1)
}

/// `split` is the one look-up of `plain` followed by its twin on `slot`.
fn assert_twinned(what: &str, plain: &Units, split: &Units, slot: u32) {
    assert_eq!(plain.len(), 1, "{what}: one look-up in the plain shader: {plain:?}");
    assert_eq!(split.len(), 2, "{what}: two look-ups in the keyed one: {split:?}");
    assert_eq!(split[0], plain[0], "{what}: the R,G plane on the binding's own unit");
    assert_eq!(split[1], (plain[0].0, slot), "{what}: the B,A plane, same opcode, on the companion slot");
}

/// Every sampler kind and fragment look-up form the selector takes: the
/// binding is sensitive and the look-up is twinned on the companion slot,
/// with more code than the plain shader.
#[test]
fn every_selected_form_splits_onto_the_companion_slot() {
    let _g = serial();
    /* name, declaration, output type, body, the look-up's opcode */
    let forms = [
        ("sampler3D", "uniform highp sampler3D s;", "vec4", "o = texture(s, uv3);", "texld"),
        ("samplerCube", "uniform highp samplerCube s;", "vec4", "o = texture(s, uv3);", "texld"),
        ("sampler2DArray", "uniform highp sampler2DArray s;", "vec4", "o = texture(s, uv3);", "texld"),
        ("isampler2D", "uniform highp isampler2D s;", "ivec4", "o = texture(s, uv);", "texld"),
        ("isampler3D", "uniform highp isampler3D s;", "ivec4", "o = texture(s, uv3);", "texld"),
        ("usamplerCube", "uniform highp usamplerCube s;", "uvec4", "o = texture(s, uv3);", "texld"),
        ("usampler2DArray", "uniform highp usampler2DArray s;", "uvec4", "o = texture(s, uv3);", "texld"),
        ("bias", "uniform highp sampler2D s;", "vec4", "o = texture(s, uv, 0.5);", "texldb"),
        ("textureLod", "uniform highp sampler2D s;", "vec4", "o = textureLod(s, uv, 1.0);", "texldl"),
        ("texelFetch", "uniform highp sampler2D s;", "vec4", "o = texelFetch(s, ivec2(uv), 0);", "txf"),
        ("textureProj3", "uniform highp sampler2D s;", "vec4", "o = textureProj(s, uv3);", "texld"),
        ("textureProj4", "uniform highp sampler2D s;", "vec4", "o = textureProj(s, uv4);", "texld"),
    ];
    for (name, decl, out, body, op) in forms {
        let (plain, split, w0, w1) = split0(SHC_STAGE_FRAGMENT, &fs(decl, out, body));
        assert_eq!(plain[0].0, op, "{name}");
        assert_twinned(name, &plain, &split, 5);
        assert!(w1 > w0, "{name}: the join costs instructions ({w0} -> {w1})");
    }
}

/// The vertex stage: a look-up of any form there is an explicit-level one
/// (`texture` is `texldl`), and the companion slot is numbered as the
/// stage numbers its own units (the binding's unit plus the slot).
#[test]
fn the_vertex_stage_splits_every_form() {
    let _g = serial();
    let forms = [
        ("textureLod", "uniform highp sampler2D s;", "v = textureLod(s, p.xy, 0.0);", "texldl"),
        ("texture", "uniform highp sampler2D s;", "v = texture(s, p.xy);", "texldl"),
        ("texelFetch", "uniform highp sampler2D s;", "v = texelFetch(s, ivec2(p.xy), 0);", "txf"),
        ("sampler3D", "uniform highp sampler3D s;", "v = textureLod(s, p.xyz, 0.0);", "texldl"),
    ];
    for (name, decl, body, op) in forms {
        let (plain, split, w0, w1) = split0(SHC_STAGE_VERTEX, &vs(decl, "vec4", body));
        assert_eq!(plain[0].0, op, "{name}");
        assert_twinned(name, &plain, &split, plain[0].1 + 5);
        assert!(w1 > w0, "{name}: ({w0} -> {w1})");
    }
}

/// The forms the plain compile refuses today, so no shader exists to key.
/// Pinned as it is (a finding: an offset, a gradient or `texelFetchOffset`
/// on a binding that may be a 128-bit texture is not compilable on the
/// GC7000UL at all, with or without the key).
#[test]
fn offset_and_gradient_forms_are_refused_by_the_plain_compile() {
    let _g = serial();
    let refusal = "online shaderc: the code generator refused this fragment shader: Select(Unsupported(\"an ES 3.00 \
                   texture look-up (a shadow sampler, an integer result, a gradient, an offset or a cube fetch) is \
                   not selected yet\"))";
    for (name, body) in [
        ("textureOffset", "o = textureOffset(s, uv, ivec2(1, 2));"),
        ("texelFetchOffset", "o = texelFetchOffset(s, ivec2(uv), 0, ivec2(1, 2));"),
        ("textureGrad", "o = textureGrad(s, uv, dFdx(uv), dFdy(uv));"),
    ] {
        let src = fs("uniform highp sampler2D s;", "vec4", body);
        match try_compile(SHC_STAGE_FRAGMENT, &src) {
            Ok(sh) => {
                release_shader(&sh);
                panic!("{name} compiles now: move it to the split check");
            }
            Err(log) => assert_eq!(log, refusal, "{name}"),
        }
    }
}

/// Two keyed bindings in one shader: each look-up gets its own twin on its
/// own companion slot; keying only one leaves the other single.
#[test]
fn two_keyed_bindings_each_get_their_own_twin() {
    let _g = serial();
    let sh = compile(
        SHC_STAGE_FRAGMENT,
        &fs(
            "uniform highp sampler2D a;\nuniform highp usampler3D b;",
            "vec4",
            "o = texture(a, uv) + vec4(texture(b, uv3));",
        ),
    );
    let sense = wide128_sensitivity(&sh);
    assert_eq!(sense, 0b11);
    let mut plain = units(&sh);
    plain.sort();
    assert_eq!(plain, [("texld", 0), ("texld", 1)]);

    let both = variant(&sh, wide128_planes(sense, &[Some(6), Some(7)])).expect("both keyed");
    let order = units(&both);
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(sorted, [("texld", 0), ("texld", 1), ("texld", 6), ("texld", 7)], "{order:?}");
    /* the twin of each look-up follows it, on its own binding's slot */
    for (i, (_, id)) in order.iter().enumerate() {
        if *id < 2 {
            assert_eq!(order[i + 1], ("texld", id + 6), "{order:?}");
        }
    }
    release_shader(&both);

    for (b, slot) in [(0usize, 6u8), (1, 7)] {
        let mut companion = [None, None];
        companion[b] = Some(slot);
        let one = variant(&sh, wide128_planes(sense, &companion)).expect("one keyed");
        let mut got = units(&one);
        got.sort();
        let mut want = vec![("texld", 0), ("texld", 1), ("texld", slot as u32)];
        want.sort();
        assert_eq!(got, want, "only binding {b} keyed");
        release_shader(&one);
    }
    release_shader(&sh);
}

/// `textureSize` alone reads the primary unit's size and no texel: the
/// binding is not sensitive, a key naming it is empty, and the variant is
/// the plain code. Beside a look-up of another binding only that one is.
#[test]
fn texture_size_alone_is_not_sensitive() {
    let _g = serial();
    for (name, decl, body) in [
        ("sampler2D", "uniform highp sampler2D s;", "o = vec4(vec2(textureSize(s, 0)), 0.0, 1.0);"),
        ("sampler3D", "uniform highp sampler3D s;", "o = vec4(vec3(textureSize(s, 0)), 1.0);"),
        ("samplerCube", "uniform highp samplerCube s;", "o = vec4(vec2(textureSize(s, 0)), 0.0, 1.0);"),
    ] {
        let sh = compile(SHC_STAGE_FRAGMENT, &fs(decl, "vec4", body));
        assert_eq!(wide128_sensitivity(&sh), 0, "{name}");
        assert!(units(&sh).is_empty(), "{name}: no look-up");
        let key = wide128_planes(wide128_sensitivity(&sh), &[Some(5)]);
        assert!(key.is_empty(), "{name}");
        let v = variant(&sh, key).expect("a copy");
        assert_eq!(v.usc_words(), sh.usc_words(), "{name}");
        release_shader(&v);
        release_shader(&sh);
    }
    let sh = compile(
        SHC_STAGE_FRAGMENT,
        &fs(
            "uniform highp sampler2D a;\nuniform highp sampler2D b;",
            "vec4",
            "o = texture(a, uv) + vec4(vec2(textureSize(b, 0)), 0.0, 0.0);",
        ),
    );
    assert_eq!(wide128_sensitivity(&sh), 0b01, "the look-up's binding only, not the textureSize's");
    release_shader(&sh);
}
