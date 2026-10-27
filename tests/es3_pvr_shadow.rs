// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Shadow samplers on the PowerVR online back end, against
//! docs/es3/pvr-2darray.md section 7: every shadow look-up form of GLSL ES
//! 3.00 that dEQP-GLES3 uses (`texture.shadow.*` and
//! `shaders.texture_functions.*.sampler{2d,2darray,cube}shadow_*`) compiles,
//! in both stages; the depth is sampled as the plain look-up samples it and
//! compared in the shader against the sampler meta's op; and for a 2D
//! sampler and a native 2D array the compare per texel (PCF) that a LINEAR
//! filter asks for -- the bilinear footprint gathered raw (pco's `tg4`:
//! `smp` with `sbmode` rawdata, one channel, the level replaced by 0, the
//! layer's address for an array), each tap compared, the four results
//! weighted by `fract(coord * size - 0.5)` -- taken when sampler meta dword
//! 13 bit 0 says so. The programs run in the byte model against the IR over
//! every compare op, the fixed-point clamp, no compare, and the per-texel
//! bit on and off.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_shadow
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::shaderc::online::ir::IoSlot;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::srcs;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The native-array switch set for the closure, and back at its default
/// after it, panic or not.
fn with_native_array<T>(on: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_array(i32::from(on));
    f()
}

/// One shader through the online PowerVR chain: the record (its code kept
/// by `store::begin_keep` for the closure) or the refusal.
fn compile_with<T>(stage: Stage, src: &str, f: impl FnOnce(&ShcShader, &[u8]) -> T) -> Result<T, String> {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a)
        } else {
            compile_vertex(&l.module, &l.reflection, a)
        }
        .map_err(|e| format!("chain: {e:?}"))?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        Ok(f(&rec, &image))
    })
    .unwrap()
}

/// The `smp` fields of every sample in a program, by name, in program order,
/// with the data-block temporary each reads.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 11] =
        ["fcnorm", "dmn", "chan", "lodm", "pplod", "proj", "nncoords", "soo", "tao", "sbmode", "sno"];
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if ops_for(r, ph).any(|o| o.name == "smp") {
                let mut f: BTreeMap<&str, u32> = NAMES.iter().map(|n| (*n, r.get(n).unwrap_or(0))).collect();
                let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
                f.insert("data", lo[1].map_or(u32::MAX, |o| o.index));
                out.push(f);
            }
        }
        off += g.total;
    }
    out
}

fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

const DECLS: &str = "uniform vec4 c;\nuniform vec2 g;\nuniform float l;\n";

fn fragment(sampler: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n{DECLS}\
         out vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(sampler: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp {sampler} s;\n{DECLS}\
         in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; {body} }}\n"
    )
}

/// Every shadow look-up form dEQP-GLES3 shaders.texture_functions and
/// texture.shadow use, per sampler type (the ES 3.00 built-in table,
/// section 8.8): (sampler, expression, fragment only).
const FORMS: &[(&str, &str, bool)] = &[
    ("sampler2DShadow", "texture(s, c.xyz)", false),
    ("sampler2DShadow", "texture(s, c.xyz, l)", true),
    ("sampler2DShadow", "textureProj(s, c)", false),
    ("sampler2DShadow", "textureProj(s, c, l)", true),
    ("sampler2DShadow", "textureLod(s, c.xyz, l)", false),
    ("sampler2DShadow", "textureOffset(s, c.xyz, ivec2(3, -2))", false),
    ("sampler2DShadow", "textureOffset(s, c.xyz, ivec2(3, -2), l)", true),
    ("sampler2DShadow", "textureProjOffset(s, c, ivec2(1, 1))", false),
    ("sampler2DShadow", "textureProjOffset(s, c, ivec2(1, 1), l)", true),
    ("sampler2DShadow", "textureLodOffset(s, c.xyz, l, ivec2(-8, 7))", false),
    ("sampler2DShadow", "textureProjLod(s, c, l)", false),
    ("sampler2DShadow", "textureProjLodOffset(s, c, l, ivec2(2, -1))", false),
    ("sampler2DShadow", "textureGrad(s, c.xyz, g, g.yx)", false),
    ("sampler2DShadow", "textureGradOffset(s, c.xyz, g, g.yx, ivec2(1, 0))", false),
    ("sampler2DShadow", "textureProjGrad(s, c, g, g.yx)", false),
    ("sampler2DShadow", "textureProjGradOffset(s, c, g, g.yx, ivec2(0, 1))", false),
    ("sampler2DArrayShadow", "texture(s, c)", false),
    ("sampler2DArrayShadow", "textureGrad(s, c, g, g.yx)", false),
    ("sampler2DArrayShadow", "textureGradOffset(s, c, g, g.yx, ivec2(1, -1))", false),
    ("samplerCubeShadow", "texture(s, c)", false),
    ("samplerCubeShadow", "texture(s, c, l)", true),
];

/// Every form compiles in the fragment stage, and every one without a bias
/// in the vertex stage, with the native array on (the array forms sample
/// by layer address); the 2D and cube forms compile with it off too.
#[test]
fn every_shadow_form_of_deqp_compiles() {
    for (sampler, e, fragment_only) in FORMS {
        let fs = fragment(sampler, &format!("o = vec4({e});"));
        with_native_array(true, || {
            compile_with(Stage::Fragment, &fs, |_, _| ()).unwrap_or_else(|m| panic!("{sampler} {e}: {m}"));
        });
        if !*fragment_only {
            let vs = vertex(sampler, &format!("v = vec4({e});"));
            with_native_array(true, || {
                compile_with(Stage::Vertex, &vs, |_, _| ()).unwrap_or_else(|m| panic!("vertex {sampler} {e}: {m}"));
            });
        }
        if *sampler != "sampler2DArrayShadow" {
            with_native_array(false, || {
                compile_with(Stage::Fragment, &fs, |_, _| ()).unwrap_or_else(|m| panic!("off: {sampler} {e}: {m}"));
            });
        }
    }
    /* textureSize on a shadow sampler: the plain textureSize */
    for sampler in ["sampler2DShadow", "sampler2DArrayShadow", "samplerCubeShadow"] {
        let fs = fragment(sampler, "o = vec4(vec2(textureSize(s, 1).xy), 0.0, 1.0);");
        with_native_array(true, || compile_with(Stage::Fragment, &fs, |_, _| ()).unwrap_or_else(|m| panic!("{m}")));
    }
}

/// A 2D shadow look-up is two samples: the plain one (the look-up's own
/// level mode, offset and gradients) and the gather -- `sbmode` rawdata,
/// one channel, `lodm` replace with `pplod`, the same `soo`, no `tao` --
/// reading its own data block; on a native 2D array both carry `tao`; a
/// cube shadow look-up has the plain sample alone (no cube gather).
#[test]
fn a_shadow_look_up_is_the_plain_sample_and_a_one_channel_gather() {
    let rawdata = enum_value("sbmode", "rawdata");
    let cases: &[(&str, &str, &str, u32, u32)] = &[
        ("sampler2DShadow", "texture(s, c.xyz)", "normal", 0, 0),
        ("sampler2DShadow", "textureLod(s, c.xyz, l)", "replace", 1, 0),
        ("sampler2DShadow", "textureOffset(s, c.xyz, ivec2(3, -2))", "normal", 0, 1),
        ("sampler2DShadow", "textureProjGradOffset(s, c, g, g.yx, ivec2(0, 1))", "gradients", 0, 1),
        ("sampler2DArrayShadow", "texture(s, c)", "bias", 1, 0),
        ("sampler2DArrayShadow", "textureGradOffset(s, c, g, g.yx, ivec2(1, -1))", "gradients", 0, 1),
    ];
    for (sampler, e, plain_lodm, plain_pplod, soo) in cases {
        let fs = fragment(sampler, &format!("o = vec4({e});"));
        let fields = with_native_array(true, || {
            compile_with(Stage::Fragment, &fs, |_, image| smps(image)).unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(fields.len(), 2, "{sampler} {e}: the plain sample and the gather");
        let (plain, gather) = (&fields[0], &fields[1]);
        let tao = u32::from(*sampler == "sampler2DArrayShadow");
        assert_eq!(
            (plain["sbmode"], plain["chan"], plain["lodm"], plain["pplod"], plain["soo"], plain["tao"], plain["proj"]),
            (0, 4, enum_value("lodm", plain_lodm), *plain_pplod, *soo, tao, 0),
            "{sampler} {e}: the plain sample"
        );
        assert_eq!(
            (gather["sbmode"], gather["chan"], gather["lodm"], gather["pplod"], gather["soo"], gather["tao"]),
            (rawdata, 1, enum_value("lodm", "replace"), 1, *soo, tao),
            "{sampler} {e}: the gather"
        );
        assert_eq!(
            (gather["fcnorm"], gather["dmn"], gather["proj"], gather["nncoords"]),
            (1, enum_value("dmn", "2d"), 0, 0)
        );
        assert_eq!(plain["data"], gather["data"], "both read the one data block");
    }
    let fs = fragment("samplerCubeShadow", "o = vec4(texture(s, c));");
    let fields = with_native_array(true, || compile_with(Stage::Fragment, &fs, |_, image| smps(image)).unwrap());
    assert_eq!(fields.len(), 1, "a cube shadow look-up: the plain sample alone");
    assert_eq!((fields[0]["sbmode"], fields[0]["dmn"]), (0, enum_value("dmn", "3d")));
    /* the stacked array (switch off) keeps the one filtered sample */
    let fs = fragment("sampler2DArrayShadow", "o = vec4(texture(s, c));")
        .replace("uniform highp", "uniform float pvr_layers_s;\nuniform highp");
    let fields = with_native_array(false, || compile_with(Stage::Fragment, &fs, |_, image| smps(image)).unwrap());
    assert_eq!(fields.len(), 1, "the stacked array shadow look-up: no gather");
}

type InputFn<'a> = dyn Fn(&str, u32) -> u32 + 'a;

fn sampler_marker(name: &str) -> u32 {
    usc_exec::named(7, name, 99)
}

/// Our program for a fragment shader with the one sampler `s` against the
/// IR, once per input set, with the sampler meta words `meta` (dwords 12
/// and 13 of the descriptor) and the native array on for both sides; the
/// packed pixels, each asserted equal to the IR's.
fn run_against_ir(src: &str, meta: [u32; 2], inputs: &[&InputFn<'_>]) -> Vec<u32> {
    with_native_array(true, || {
        usc_exec::set_smp_meta(meta);
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("refused: {e:?}"));
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(rec.num_varyings, 0, "the harness places no varyings");
            let name_of = |n: &[core::ffi::c_char]| -> String {
                n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
            };
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            let mut out = Vec::new();
            for val in inputs {
                let mut inp = usc_exec::sentinel_inputs(1.0e30);
                let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                    rows.iter().filter(|u| u.dword_off == no_block).collect();
                samplers.sort_by_key(|u| u.binding);
                for (i, u) in samplers.iter().enumerate() {
                    let marker = sampler_marker(&name_of(&u.name));
                    inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(marker));
                    inp.shared[20 * i + 4..20 * i + 7].copy_from_slice(&usc_exec::tex_meta(marker));
                    inp.shared[20 * i + 12..20 * i + 14].copy_from_slice(&meta);
                }
                let base = 20 * samplers.len();
                for u in rows.iter().filter(|u| u.dword_off != no_block) {
                    for k in 0..u.dwords as u32 {
                        inp.shared[base + u.dword_off as usize + k as usize] = val(&name_of(&u.name), k);
                    }
                }
                let ours = *usc_exec::run_binary(&image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
                let ir_in = usc_exec::ir::IrInputs {
                    input: &|_, _| 0,
                    uniform: &|d| {
                        let u =
                            l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                        u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                    },
                    marker: &|binding| {
                        let s = l.reflection.uniforms.iter().find(|u| u.binding == binding && u.dword_off == no_block);
                        sampler_marker(s.map_or("s", |u| u.name))
                    },
                };
                let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
                let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
                let mut px = 0u32;
                for (c, lane) in lanes.iter().enumerate() {
                    let x = f32::from_bits(lane.expect("stored").0);
                    px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
                }
                assert_eq!(ours, px, "input {}: ours {ours:08x}, the IR {px:08x}", out.len());
                out.push(ours);
            }
            out
        })
        .unwrap()
    })
}

/// A set of reference values around the model's texels (the model's depth
/// at a texel is k/1000 for some k): the eight compare ops, the fixed-point
/// clamp bit (a reference outside [0, 1]) and no compare.
fn references() -> Vec<f32> {
    vec![0.0, 0.25, 0.5, 0.75, 1.0, -0.5, 1.5]
}

/// Section 7.1: the filtered depth compared against the reference by the
/// sampler's op -- every op, the fixed-point clamp (bit 3: a reference of
/// 1.5 compares as 1.0), no compare (bit 4: the depth itself) -- on a 2D
/// sampler, with the per-texel bit off; the plain forms with a level, a
/// projection and an offset agree too.
#[test]
fn the_filtered_depth_is_compared_by_the_samplers_op() {
    let forms = [
        "texture(s, c.xyz)",
        "textureLod(s, c.xyz, l)",
        "textureProj(s, c)",
        "textureOffset(s, c.xyz, ivec2(3, -2))",
        "textureProjLodOffset(s, c, l, ivec2(2, -1))",
    ];
    for e in forms {
        let src = fragment("sampler2DShadow", &format!("o = vec4({e});"));
        for op in [0u32, 1, 2, 3, 4, 5, 6, 7, 1 | 8, 4 | 8, 16] {
            let cases: Vec<Box<InputFn<'_>>> = references()
                .into_iter()
                .map(|r| -> Box<InputFn<'_>> {
                    Box::new(move |n: &str, k: u32| match n {
                        "c" => [0.31f32, 0.77, r, 1.0][k as usize].to_bits(),
                        "l" => 1.0f32.to_bits(),
                        _ => 0,
                    })
                })
                .collect();
            let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
            let px = run_against_ir(&src, [op, 0], &refs);
            if op == 16 {
                let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
                assert_eq!(distinct.len(), 1, "{e} op {op}: no compare, the depth whatever the reference: {px:08x?}");
            }
        }
    }
}

/// Section 7.2: with sampler meta dword 13 bit 0 the compare is per texel
/// of the gather's footprint, bilinearly weighted -- the result is a
/// fraction when the reference falls between the four depths -- on a 2D
/// sampler and on a native 2D array (the gather carries the layer address),
/// with and without an offset and under a projection; and the IR agrees.
#[test]
fn the_per_texel_compare_weights_the_four_taps() {
    let marker = sampler_marker("s");
    let (u, v) = (0.3137f32, 0.7712f32);
    for (sampler, e, off) in [
        ("sampler2DShadow", "texture(s, c.xyz)", [0i32, 0i32]),
        ("sampler2DShadow", "textureOffset(s, c.xyz, ivec2(3, -2))", [3, -2]),
        ("sampler2DShadow", "textureProj(s, c)", [0, 0]),
        ("sampler2DShadow", "textureGrad(s, c.xyz, g, g.yx)", [0, 0]),
        ("sampler2DArrayShadow", "texture(s, c)", [0, 0]),
        ("sampler2DArrayShadow", "textureGradOffset(s, c, g, g.yx, ivec2(1, -1))", [1, -1]),
    ] {
        let src = fragment(sampler, &format!("o = vec4({e});"));
        let array = sampler == "sampler2DArrayShadow";
        /* a reference among the footprint's four depths (the model's
         * texels of the layer the look-up reads, the offset applied), so
         * that some taps pass and some fail */
        let layered = if array { usc_exec::marker_at_layer(marker, 2) } else { marker };
        let taps = usc_exec::gather_model(layered, u.to_bits(), v.to_bits(), off).map(f32::from_bits);
        let mut sorted = taps;
        sorted.sort_by(f32::total_cmp);
        let reference = (sorted[1] + sorted[2]) * 0.5;
        assert!(sorted[1] < sorted[2], "{sampler} {e}: the footprint's depths differ: {taps:?}");
        let coord = move |n: &str, k: u32| match n {
            /* an array: (u, v, layer 2, reference); 2D: (u, v, reference,
             * q = 1 for the projective form) */
            "c" if array => [u, v, 2.0, reference][k as usize].to_bits(),
            "c" => [u, v, reference, 1.0][k as usize].to_bits(),
            "g" => [0.01f32, 0.02][k as usize].to_bits(),
            _ => 0,
        };
        for op in [1u32, 4, 2, 7] {
            let on = run_against_ir(&src, [op, 1], &[&coord])[0] & 0xff;
            let off = run_against_ir(&src, [op, 0], &[&coord])[0] & 0xff;
            if op == 1 || op == 4 {
                assert!(on != 0 && on != 255, "{sampler} {e} op {op}: a fraction of the taps pass, got {on}");
                assert!(off == 0 || off == 255, "{sampler} {e} op {op}: the filtered compare is 0 or 1, got {off}");
            }
            if op == 7 {
                assert_eq!((on, off), (255, 255), "always");
            }
        }
        /* no compare: the depth itself, whatever the bit */
        assert_eq!(run_against_ir(&src, [16, 1], &[&coord]), run_against_ir(&src, [16, 0], &[&coord]));
    }
}

/// The weights are `fract(coord * size - 0.5)` per axis: at a coordinate
/// exactly on a texel centre the fractions are 0 and the result is tap
/// (i0, j0)'s compare alone.
#[test]
fn a_texel_centre_takes_the_first_tap_alone() {
    let marker = sampler_marker("s");
    let [w, h] = usc_exec::tex_size_2d(usc_exec::tex_state(marker), 0);
    /* a texel centre of the size the program reads from the state words */
    let (u, v) = ((3.0 + 0.5) / w as f32, (5.0 + 0.5) / h as f32);
    let weights = usc_exec::gather_weights(u, v, w, h);
    assert!(weights[0].abs() < 1e-4 && weights[1].abs() < 1e-4, "{weights:?}");
    let taps = usc_exec::gather_model(marker, u.to_bits(), v.to_bits(), [0, 0]).map(f32::from_bits);
    let src = fragment("sampler2DShadow", "o = vec4(texture(s, c.xyz));");
    for (reference, want) in [(taps[0] - 0.0005, 255u32), (taps[0] + 0.0005, 0)] {
        let coord = move |n: &str, k: u32| match n {
            "c" => [u, v, reference, 1.0][k as usize].to_bits(),
            _ => 0,
        };
        /* op 1: pass when reference < depth */
        let px = run_against_ir(&src, [1, 1], &[&coord])[0] & 0xff;
        assert!(px.abs_diff(want) <= 1, "reference {reference}: {px}, wanted about {want}");
    }
}
