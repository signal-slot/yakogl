// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `gl_FragDepth` on the PowerVR online back end (the 10 `fbo.depth` clamp
//! cases of dEQP-GLES3): the written depth is clamped to [0, 1]
//! (ES 3.00 section 7.2) by an `fadd` with `sat` set, handed to the ISP by
//! `depthf` from that result, and the record says so in its own field,
//! `ShcShader::writes_depth`, which makes the draw's pass type
//! DEPTH_FEEDBACK. The sampler flavours stay in `native_cube`, and the tests
//! below pin that a depth write never changes that byte and a native sample
//! never sets `writes_depth`. Refused, by name: a depth written together
//! in a variant that rewrites the outputs as colour (several sRGB
//! attachments, an integer render target). A depth written with a discard
//! (`depthf` under the kill test's p0, after `alphaf`), beside several colour
//! outputs and in a blend variant builds.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_fragdepth
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_fragment_for};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::{dsts, srcs};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

/// The compiler keeps its output in one arena that every compile resets, and
/// the native 3D switch is process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the chain recorded for one fragment shader.
#[derive(Debug)]
struct Rec {
    native_cube: u8,
    writes_depth: u8,
    image: Vec<u8>,
}

/// One fragment shader through the chain, or the refusal.
fn compile(src: &str) -> Result<Rec, String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    compile_at_rest(src)
}

/// `compile` for callers that already hold `COMPILING` (under
/// `with_native_3d`, below).
fn compile_at_rest(src: &str) -> Result<Rec, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment(&l.module, &l.reflection, a).map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        Ok(Rec {
            native_cube: rec.native_cube,
            writes_depth: rec.writes_depth,
            image: code.iter().flat_map(|w| w.to_le_bytes()).collect(),
        })
    })
    .unwrap()
}

/// The native 3D switch on or off for the closure, and back at its default
/// after it, panic or not. Holds `COMPILING`, so call `compile_at_rest`
/// inside, not `compile`.
fn with_native_3d<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_3d(i32::from(on));
    f()
}

fn fragment(body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nuniform vec4 c;\nuniform float d;\nin vec2 v;\n\
         layout(location = 0) out vec4 o;\nvoid main() {{ o = c; {body} }}\n"
    )
}

/// A sampler3D sample that is used, and a `gl_FragDepth` write or not.
fn frag_3d(body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp sampler3D s;\n\
         uniform vec4 c;\nuniform vec3 p;\nuniform float d;\nlayout(location = 0) out vec4 o;\n\
         void main() {{ o = c * texture(s, p); {body} }}\n"
    )
}

/// The program's operations, in order: (name, `sat`, the first source's
/// temporary, the destination's temporary).
fn ops(image: &[u8]) -> Vec<(&'static str, u32, Option<u32>, Option<u32>)> {
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let Some(op) = ops_for(r, ph).next() else { continue };
            let s0 = g.lower.as_ref().and_then(|l| srcs(l)[0]).map(|o| o.index);
            let d0 = g.dst.as_ref().and_then(|d| dsts(d)[0]).map(|o| o.index);
            out.push((op.name, r.get("sat").unwrap_or(0), s0, d0));
        }
        off += g.total;
    }
    out
}

/// A program that writes depth clamps it with an `fadd` with `sat` set and
/// hands that result to `depthf`; the record says it writes depth.
#[test]
fn written_depth_is_clamped_and_fed_back() {
    let bodies =
        ["gl_FragDepth = d;", "gl_FragDepth = (v.x + v.y) * 2.0 - 0.5;", "gl_FragDepth = 2.0;", "gl_FragDepth = -1.0;"];
    for body in bodies {
        let src = fragment(body);
        let r = compile(&src).unwrap_or_else(|e| panic!("{body}: {e}"));
        assert_eq!(r.writes_depth, 1, "{body}: the record names the depth output");
        assert_eq!(r.native_cube, 0, "{body}: no sampler, no sampler flavour");
        let p = ops(&r.image);
        let at = p.iter().position(|o| o.0 == "depthf").unwrap_or_else(|| panic!("{body}: no depthf in {p:?}"));
        assert_eq!(p.iter().filter(|o| o.0 == "depthf").count(), 1, "{body}");
        let src_reg = p[at].2.unwrap_or_else(|| panic!("{body}: depthf reads no register"));
        assert!(
            p[..at].iter().any(|o| o.0 == "fadd" && o.1 == 1 && o.3 == Some(src_reg)),
            "{body}: depthf's temporary t{src_reg} is not the result of a saturating fadd in {p:?}"
        );
    }
}

/// A program that writes no depth carries neither the instruction nor the
/// flag.
#[test]
fn no_depth_write_no_depthf() {
    let r = compile(&fragment("")).expect("compiles");
    assert_eq!(r.writes_depth, 0);
    assert!(ops(&r.image).iter().all(|o| o.0 != "depthf"));
}

/// A depth written by a shader that also discards: the kill test, `alphaf`
/// and `depthf` (both under p0) and one `wdf`, in that order, then the write.
#[test]
fn depth_with_discard_feeds_back_after_the_kill_test() {
    let r = compile(&fragment("if (v.x < 0.5) discard; gl_FragDepth = d;")).expect("compiles");
    assert_eq!(r.writes_depth, 1);
    let p = ops(&r.image);
    let pos = |n: &str| p.iter().position(|o| o.0 == n).unwrap_or_else(|| panic!("no {n} in {p:?}"));
    assert_eq!(p.iter().filter(|o| o.0 == "depthf").count(), 1, "{p:?}");
    assert_eq!(p.iter().filter(|o| o.0 == "alphaf").count(), 1, "{p:?}");
    assert!(pos("alphaf") < pos("depthf"), "{p:?}");
    let wdf = p.iter().rposition(|o| o.0 == "wdf").expect("wdf");
    assert!(pos("depthf") < wdf, "{p:?}");
    // the discard-only program is unchanged by the depth work
    let k = compile(&fragment("if (v.x < 0.5) discard;")).expect("compiles");
    assert_eq!(k.writes_depth, 0);
    assert!(ops(&k.image).iter().all(|o| o.0 != "depthf"));
}

/// A depth written beside several colour outputs.
#[test]
fn depth_with_several_outputs_builds() {
    let src = "#version 300 es\nprecision highp float;\nuniform vec4 c;\nuniform float d;\nin vec2 v;\n\
               layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
               void main() { o0 = c; o1 = c + 1.0; BODY }\n";
    let body = "gl_FragDepth = d;";
    let r = compile(&src.replace("BODY", body)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r.writes_depth, 1);
    let p = ops(&r.image);
    assert_eq!(p.iter().filter(|o| o.0 == "depthf").count(), 1, "{p:?}");
    let at = p.iter().position(|o| o.0 == "depthf").unwrap();
    let src_reg = p[at].2.expect("depthf reads a register");
    assert!(p[..at].iter().any(|o| o.0 == "fadd" && o.1 == 1 && o.3 == Some(src_reg)), "{p:?}");
}

/// `compile` under a blend key: Ok or the refusal.
fn compile_for(src: &str, key: u32) -> Result<(), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        compile_fragment_for(&l.module, &l.reflection, a, key).map(|_| ()).map_err(|e| format!("chain: {e:?}"))
    })
    .unwrap()
}

/// Several sRGB attachments encode every output as colour and reset the key:
/// a depth written there is refused by name, not encoded with the sRGB curve.
#[test]
fn depth_with_several_srgb_attachments_is_refused() {
    let src = fragment("gl_FragDepth = d;");
    let e = compile_for(&src, yakogl::shaderc::SHC_BLEND_KEY_SRGB_ENCODED).expect_err("refused");
    assert!(e.contains("gl_FragDepth with several sRGB attachments"), "{e}");
    // the same shader without a depth write still compiles under the key
    compile_for(&fragment(""), yakogl::shaderc::SHC_BLEND_KEY_SRGB_ENCODED).expect("sRGB variant compiles");
}

/// An integer render target's output (`shc_blend_key_int_rt`), and a
/// `gl_FragDepth` write or not.
fn fragment_int(body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp int;\nprecision highp float;\n\
         uniform highp uvec4 u_v;\nuniform float d;\n\
         layout(location = 0) out highp uvec4 o;\nvoid main() {{ o = u_v; {body} }}\n"
    )
}

/// An integer render target packs every output as an integer colour and
/// resets the key: a depth written there is refused by name, not packed as
/// an integer colour either.
#[test]
fn depth_with_an_integer_render_target_is_refused() {
    let rt = yakogl::shaderc::ShcIntRt { channels: 4, bits: 32, signed: false };
    let key = yakogl::shaderc::shc_blend_key_int_rt(rt);
    let e = compile_for(&fragment_int("gl_FragDepth = d;"), key).expect_err("refused");
    assert!(e.contains("gl_FragDepth with an integer render target"), "{e}");
    // the same shader without a depth write still compiles under the key
    compile_for(&fragment_int(""), key).expect("integer variant compiles");
}

/// The sampler flavours (`native_cube`) and the depth output
/// (`writes_depth`) are two fields, and no combination makes one change the
/// other. Both matter outside the record: the draw picks its pass from
/// `writes_depth` only (`src/pvr/backend.rs`) and binds the twiddled 3D
/// sampler copy from `native_cube` bit 2 only (`src/gl/draw.rs`), so a wrong
/// value here is wrong behaviour there.
#[test]
fn sampler_flavours_and_depth_output_are_independent() {
    /* native 3D off, sampler3D and gl_FragDepth together: the native 3D bit
     * (4) stays clear -- it would make draw.rs bind the twiddled copy, the
     * test of tests/es3_pvr_3d.rs -- while the depth output is named and
     * depthf feeds it. */
    let off = with_native_3d(false, || compile_at_rest(&frag_3d(""))).unwrap();
    let r = with_native_3d(false, || compile_at_rest(&frag_3d("gl_FragDepth = d;"))).unwrap();
    assert_eq!(r.native_cube & 4, 0, "no native 3D sample with native 3D off");
    assert_eq!(r.native_cube, off.native_cube, "a depth write leaves the sampler flavours alone");
    assert_eq!(r.writes_depth, 1, "the depth output is named");
    assert!(ops(&r.image).iter().any(|o| o.0 == "depthf"));
    /* native 3D on, sampler3D without any depth: the 3D bit set, no depth
     * output, no depthf -- such a program never gets DEPTH_FEEDBACK. */
    let on = with_native_3d(true, || compile_at_rest(&frag_3d(""))).unwrap();
    assert_eq!(on.native_cube & 4, 4, "the 3D sample is native");
    assert_eq!(on.writes_depth, 0, "no depth written, no depth output");
    assert!(ops(&on.image).iter().all(|o| o.0 != "depthf"));
    /* native 3D on with both: the sampler flavours as without the depth
     * write, the clamp and depthf as always. */
    let r = with_native_3d(true, || compile_at_rest(&frag_3d("gl_FragDepth = d;"))).unwrap();
    assert_eq!(r.native_cube, on.native_cube);
    assert_eq!(r.writes_depth, 1);
    assert_eq!(ops(&r.image).iter().filter(|o| o.0 == "depthf").count(), 1);
}

/// A blend variant (glBlendFunc SRC_ALPHA, ONE_MINUS_SRC_ALPHA) of a shader
/// that writes depth, with and without a discard, builds: the blend works on
/// the colour and the feedback is independent of it.
#[test]
fn depth_in_a_blend_variant_builds() {
    use yakogl::shaderc::{shc_blend_key, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA};
    let key = shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, 0, 0);
    for body in ["gl_FragDepth = d;", "if (v.x < 0.5) discard; gl_FragDepth = d;"] {
        compile_for(&fragment(body), key).unwrap_or_else(|e| panic!("{body}: {e}"));
    }
}

/// The eight `shaders.fragdepth` programs that write `gl_FragDepth`
/// (es3fFragDepthTests.cpp), as dEQP sends them: (name, declarations, body).
const DEQP: [(&str, &str, &str); 8] = [
    ("const", "", "    gl_FragDepth = 0.5;\n"),
    ("uniform", "uniform highp float uf_half;\n", "    gl_FragDepth = uf_half;\n"),
    ("dynamic", "in highp vec2 v_coord;\n", "    gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n"),
    ("fragcoord_z", "", "    gl_FragDepth = gl_FragCoord.z;\n"),
    (
        "uniform_conditional_write",
        "uniform bool ub_true;\nin highp vec2 v_coord;\n",
        "    if (ub_true)\n        gl_FragDepth = (v_coord.x+v_coord.y)*0.5;\n",
    ),
    (
        "dynamic_conditional_write",
        "uniform bool ub_true;\nin highp vec2 v_coord;\n",
        "    mediump float d = (v_coord.x+v_coord.y)*0.5f;\n    if (v_coord.y < 0.5)\n        gl_FragDepth = d;\n    else\n        gl_FragDepth = 1.0 - d;\n",
    ),
    (
        "uniform_loop_write",
        "uniform int ui_two;\nuniform highp float uf_fourth;\nin highp vec2 v_coord;\n",
        "    gl_FragDepth = 0.0;\n    for (int i = 0; i < ui_two; i++)\n        gl_FragDepth += uf_fourth;\n",
    ),
    ("write_in_function", "in highp vec2 v_coord;\n", "    myfunc(v_coord);\n"),
];

fn deqp_source(name: &str, decl: &str, body: &str) -> String {
    let func = if name == "write_in_function" {
        "void myfunc (highp vec2 coord)\n{\n    gl_FragDepth = (coord.x+coord.y)*0.5;\n}\n"
    } else {
        ""
    };
    format!(
        "#version 300 es\nuniform highp vec4 u_color;\n{decl}layout(location = 0) out mediump vec4 o_color;\n\
         {func}void main (void)\n{{\n    o_color = u_color;\n{body}}}\n"
    )
}

/// Every dEQP fragdepth program that writes depth compiles and feeds back
/// exactly one depth.
#[test]
fn deqp_fragdepth_programs_compile() {
    let mut bad = Vec::new();
    for (name, decl, body) in DEQP {
        match compile(&deqp_source(name, decl, body)) {
            Ok(r) => {
                assert_eq!(r.writes_depth, 1, "{name}");
                assert_eq!(ops(&r.image).iter().filter(|o| o.0 == "depthf").count(), 1, "{name}");
            }
            Err(e) => bad.push(format!("{name}: {e}")),
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
