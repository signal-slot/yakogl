// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Explicit gradients on the PowerVR online back end beyond the 2D image
//! (dEQP-GLES3 shaders.texture_functions: textureGrad, textureGradOffset,
//! textureProjGrad and textureProjGradOffset) on a cube, a 2D array and a 3D
//! texture, each kept as the TPU's own (`pvr_native_cube`,
//! `pvr_native_array`, `pvr_native_3d` on) and as the stacked image (off),
//! in a vertex and a fragment shader, every return type.
//!
//! As Mesa's pco builds them (pco_nir_tex.c:274-283, txd): `lodm`
//! gradients, d/dx then d/dy of each coordinate in the data block after the
//! coordinates. The TPU's own cube and 3D texture take the three axes as
//! they are given, a native array the two of its layer. A stacked image is
//! a 2D one, so its two pairs are the given gradients carried into its
//! coordinates: an array's v divided by the layer count, the grid of a 3D
//! texture's u and v by its columns and rows (on each of the two slice
//! samples; d/dw has no word), and a cube's through the face projection
//! (the quotient rule on `sc / |ma|`, then 0.5, and v / 6 for the stack).
//! The byte model (tests/usc_exec) runs each program and returns the words
//! it found; they are checked against the values worked out here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_texgrad
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// How a sampler is kept: the three switches the look-up arms read.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    Stacked,
    Native,
}

/// The three switches for the closure, and back at their defaults after
/// it, panic or not.
fn with_layout<T>(layout: Layout, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    let on = i32::from(layout == Layout::Native);
    yakogl::shaderc::pvr_set_native_cube(on);
    yakogl::shaderc::pvr_set_native_array(on);
    yakogl::shaderc::pvr_set_native_3d(on);
    f()
}

/// The uniforms of every shader here: the coordinate `c`, the projective
/// one `q`, the gradients `dx` and `dy`, and the stacked image's layer
/// count, by name and component.
fn value(name: &str, k: u32, c: [f32; 3]) -> u32 {
    let x = match name {
        "c" => c[k as usize % 3],
        "q" => [c[0] * 2.0, c[1] * 2.0, c[2] * 2.0, 2.0][k as usize % 4],
        "dx" => [0.01, 0.02, -0.03][k as usize % 3],
        "dy" => [-0.004, 0.05, 0.007][k as usize % 3],
        n if n.starts_with("pvr_layers_") => LAYERS,
        _ => 0.0,
    };
    x.to_bits()
}

/// The layer count the stacked array and the grid are given: a grid of 2
/// columns and 2 rows (`ceil(sqrt(4 - 0.5))`, `ceil(4 / 2)`).
const LAYERS: f32 = 4.0;

/// What one compiled program does: its `smp`s' fields and, run once in the
/// byte model, the gradient words of each gradient sample.
#[derive(Debug)]
struct Prog {
    smps: Vec<BTreeMap<&'static str, u32>>,
    grads: Vec<Vec<f32>>,
}

/// One shader through the online PowerVR chain and the byte model, or the
/// refusal.
fn run(stage: Stage, layout: Layout, src: &str, c: [f32; 3]) -> Result<Prog, String> {
    with_layout(layout, || {
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
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let name_of = |n: &[core::ffi::c_char]| -> String {
                n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
            };
            /* the shared file as the driver fills it: one 20-dword
             * descriptor per sampler row, then the uniform block by name */
            let no_block = yakogl::shaderc::online::reflect::NO_BLOCK;
            let mut inp = usc_exec::sentinel_inputs(1.0e30);
            let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                rows.iter().filter(|u| u.dword_off == no_block).collect();
            samplers.sort_by_key(|u| u.binding);
            for (i, u) in samplers.iter().enumerate() {
                let marker = usc_exec::named(7, &name_of(&u.name), 99);
                inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(marker));
                inp.shared[20 * i + 4..20 * i + 7].copy_from_slice(&usc_exec::tex_meta(marker));
            }
            let base = 20 * samplers.len();
            for u in rows.iter().filter(|u| u.dword_off != no_block) {
                for k in 0..u.dwords as u32 {
                    inp.shared[base + u.dword_off as usize + k as usize] = value(&name_of(&u.name), k, c);
                }
            }
            let _ = usc_exec::take_gradients();
            usc_exec::run_binary(&image, &inp, true).map_err(|e| format!("byte model: {e}"))?;
            let grads =
                usc_exec::take_gradients().into_iter().map(|g| g.into_iter().map(f32::from_bits).collect()).collect();
            Ok(Prog { smps: smps(&image), grads })
        })
        .unwrap()
    })
}

/// The `smp` fields of every sample in a program, by name, in program order.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 5] = ["dmn", "lodm", "pplod", "soo", "tao"];
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if ops_for(r, ph).any(|o| o.name == "smp") {
                out.push(NAMES.iter().map(|n| (*n, r.get(n).unwrap_or(0))).collect());
            }
        }
        off += g.total;
    }
    out
}

/// An `smp` field's enumerated value by name, from the ISA tables.
fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

/// `stage`'s shader around the sampler `decl` and the look-up `e`.
fn shader(stage: Stage, sampler: &str, e: &str) -> String {
    let head = "#version 300 es\nprecision highp float;\nprecision highp int;\n";
    let uniforms =
        format!("uniform highp {sampler} s;\nuniform vec3 c;\nuniform vec4 q;\nuniform vec3 dx;\nuniform vec3 dy;\n");
    if stage == Stage::Fragment {
        format!("{head}{uniforms}out vec4 o;\nvoid main() {{ o = vec4({e}); }}\n")
    } else {
        format!("{head}{uniforms}in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; v = vec4({e}); }}\n")
    }
}

/// The look-up's texture kind.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Cube,
    Array,
    Vol,
}

/// Every gradient form ES 3.00 has on a cube, a 2D array and a 3D texture
/// (GLSL ES 3.00 8.8), with each return type: (sampler, look-up, kind).
const FORMS: &[(&str, &str, Kind)] = &[
    ("samplerCube", "textureGrad(s, c, dx, dy)", Kind::Cube),
    ("isamplerCube", "textureGrad(s, c, dx, dy)", Kind::Cube),
    ("usamplerCube", "textureGrad(s, c, dx, dy)", Kind::Cube),
    ("sampler2DArray", "textureGrad(s, c, dx.xy, dy.xy)", Kind::Array),
    ("isampler2DArray", "textureGrad(s, c, dx.xy, dy.xy)", Kind::Array),
    ("usampler2DArray", "textureGradOffset(s, c, dx.xy, dy.xy, ivec2(1, -2))", Kind::Array),
    ("sampler2DArray", "textureGradOffset(s, c, dx.xy, dy.xy, ivec2(-3, 4))", Kind::Array),
    ("sampler3D", "textureGrad(s, c, dx, dy)", Kind::Vol),
    ("sampler3D", "textureGradOffset(s, c, dx, dy, ivec3(1, -2, 3))", Kind::Vol),
    ("sampler3D", "textureProjGrad(s, q, dx, dy)", Kind::Vol),
    ("sampler3D", "textureProjGradOffset(s, q, dx, dy, ivec3(-1, 2, 1))", Kind::Vol),
    ("isampler3D", "textureGrad(s, c, dx, dy)", Kind::Vol),
    ("usampler3D", "textureProjGradOffset(s, q, dx, dy, ivec3(2, 0, -1))", Kind::Vol),
];

/// The shadow forms: their depth is sampled as the plain look-up's (the
/// arms above), then compared in the shader.
const SHADOW: &[(&str, &str)] = &[
    ("samplerCubeShadow", "textureGrad(s, q, dx, dy)"),
    ("sampler2DArrayShadow", "textureGrad(s, q, dx.xy, dy.xy)"),
    ("sampler2DArrayShadow", "textureGradOffset(s, q, dx.xy, dy.xy, ivec2(2, 1))"),
];

/// The stacked cube's (u, v) gradient for the direction `c` and its
/// derivative `d`, from ES 3.0 table 3.21 (the major axis, `sc`, `tc` and
/// `ma`) and the quotient rule on `s = 0.5 * sc / |ma| + 0.5`, then v / 6
/// for the stack of six faces.
fn cube_grad(c: [f32; 3], d: [f32; 3]) -> [f32; 2] {
    let [x, y, z] = c;
    /* (sc, tc, ma) as components of c, by index and sign */
    let (sc, tc, ma): ((usize, f32), (usize, f32), (usize, f32)) = if x.abs() >= y.abs() && x.abs() >= z.abs() {
        if x > 0.0 {
            ((2, -1.0), (1, -1.0), (0, 1.0))
        } else {
            ((2, 1.0), (1, -1.0), (0, -1.0))
        }
    } else if y.abs() >= z.abs() {
        if y > 0.0 {
            ((0, 1.0), (2, 1.0), (1, 1.0))
        } else {
            ((0, 1.0), (2, -1.0), (1, -1.0))
        }
    } else if z > 0.0 {
        ((0, 1.0), (1, -1.0), (2, 1.0))
    } else {
        ((0, -1.0), (1, -1.0), (2, -1.0))
    };
    let m = c[ma.0] * ma.1;
    let dm = d[ma.0] * ma.1;
    let ds = 0.5 * (d[sc.0] * sc.1 - c[sc.0] * sc.1 / m * dm) / m;
    let dt = 0.5 * (d[tc.0] * tc.1 - c[tc.0] * tc.1 / m * dm) / m;
    [ds, dt / 6.0]
}

/// The gradient words a sample of `kind` in `layout` takes, d/dx then d/dy
/// per coordinate, at the direction or coordinate `c`.
fn want(kind: Kind, layout: Layout, c: [f32; 3]) -> Vec<f32> {
    let dx: Vec<f32> = (0..3).map(|k| f32::from_bits(value("dx", k, c))).collect();
    let dy: Vec<f32> = (0..3).map(|k| f32::from_bits(value("dy", k, c))).collect();
    let pairs: Vec<[f32; 2]> = match (kind, layout) {
        (Kind::Cube | Kind::Vol, Layout::Native) => (0..3).map(|k| [dx[k], dy[k]]).collect(),
        (Kind::Array, Layout::Native) => (0..2).map(|k| [dx[k], dy[k]]).collect(),
        (Kind::Array, Layout::Stacked) => vec![[dx[0], dy[0]], [dx[1] / LAYERS, dy[1] / LAYERS]],
        /* 2 columns, 2 rows */
        (Kind::Vol, Layout::Stacked) => vec![[dx[0] / 2.0, dy[0] / 2.0], [dx[1] / 2.0, dy[1] / 2.0]],
        (Kind::Cube, Layout::Stacked) => {
            let gx = cube_grad(c, [dx[0], dx[1], dx[2]]);
            let gy = cube_grad(c, [dy[0], dy[1], dy[2]]);
            vec![[gx[0], gy[0]], [gx[1], gy[1]]]
        }
    };
    pairs.into_iter().flatten().collect()
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-5 * y.abs().max(1e-3))
}

/// The directions of the cube test: one per face, none on a tie.
const DIRECTIONS: [[f32; 3]; 6] =
    [[0.9, -0.3, 0.2], [-0.9, 0.4, -0.2], [0.3, 0.8, -0.5], [0.3, -0.8, 0.5], [-0.2, 0.4, 0.7], [0.6, -0.1, -0.7]];

/// Every gradient form compiles in both stages and both layouts; every
/// `smp` it makes has `lodm` gradients and no `pplod`, the TPU's own cube
/// and 3D texture sample `dmn` 3d, a native array with `tao`, a stacked
/// image as a 2D one, and an offset sets `soo`. The grid samples a float
/// texture's two slices, an integer one's nearest slice.
#[test]
fn every_gradient_form_samples_with_lodm_gradients() {
    let gradients = enum_value("lodm", "gradients");
    let (d2, d3) = (enum_value("dmn", "2d"), enum_value("dmn", "3d"));
    for (sampler, e, kind) in FORMS {
        for layout in [Layout::Stacked, Layout::Native] {
            for stage in [Stage::Fragment, Stage::Vertex] {
                let what = format!("{e} on {sampler}, {layout:?}, {stage:?}");
                let p = run(stage, layout, &shader(stage, sampler, e), DIRECTIONS[0])
                    .unwrap_or_else(|m| panic!("{what}: {m}"));
                let n = if *kind == Kind::Vol && layout == Layout::Stacked && !sampler.starts_with(['i', 'u']) {
                    2
                } else {
                    1
                };
                assert_eq!(p.smps.len(), n, "{what}: {:?}", p.smps);
                let dmn = if layout == Layout::Native && *kind != Kind::Array { d3 } else { d2 };
                let tao = u32::from(layout == Layout::Native && *kind == Kind::Array);
                let soo = u32::from(e.contains("Offset"));
                for s in &p.smps {
                    assert_eq!(
                        (s["lodm"], s["pplod"], s["dmn"], s["tao"], s["soo"]),
                        (gradients, 0, dmn, tao, soo),
                        "{what}: {s:?}"
                    );
                }
            }
        }
    }
}

/// The gradient words each form's samples take, read by the byte model:
/// the given gradients on the TPU's own cube, array and 3D texture, and on
/// a stacked image the gradients in its coordinates -- a stacked array's v
/// divided by the layer count, the grid's u and v by its columns and rows
/// (on both slices), a stacked cube's through the projection onto each of
/// the six faces. In both stages. A projective look-up's gradients are not
/// divided (GLSL ES 3.00 8.8: they are already projected).
#[test]
fn the_gradient_words_are_the_gradients_in_the_sampled_image() {
    for (sampler, e, kind) in FORMS {
        let directions: &[[f32; 3]] = if *kind == Kind::Cube { &DIRECTIONS } else { &DIRECTIONS[2..3] };
        for &c in directions {
            for layout in [Layout::Stacked, Layout::Native] {
                for stage in [Stage::Fragment, Stage::Vertex] {
                    let what = format!("{e} on {sampler} at {c:?}, {layout:?}, {stage:?}");
                    let p = run(stage, layout, &shader(stage, sampler, e), c).unwrap_or_else(|m| panic!("{what}: {m}"));
                    let w = want(*kind, layout, c);
                    assert!(!p.grads.is_empty(), "{what}: no gradient sample ran");
                    for g in &p.grads {
                        assert!(close(g, &w), "{what}: words {g:?}, want {w:?}");
                    }
                }
            }
        }
    }
}

/// The shadow forms compile in both stages and both layouts, their depth
/// sampled with `lodm` gradients.
#[test]
fn the_shadow_gradient_forms_sample_the_depth_with_gradients() {
    let gradients = enum_value("lodm", "gradients");
    for (sampler, e) in SHADOW {
        for layout in [Layout::Stacked, Layout::Native] {
            for stage in [Stage::Fragment, Stage::Vertex] {
                let what = format!("{e} on {sampler}, {layout:?}, {stage:?}");
                let src = shader(stage, sampler, e);
                let smps = with_layout(layout, || {
                    store::with_scratch(|a| {
                        let _keep = store::begin_keep().expect("keep");
                        let text = a.alloc_str(&src).unwrap();
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
                        Ok::<_, String>(smps(&image))
                    })
                    .unwrap()
                })
                .unwrap_or_else(|m| panic!("{what}: {m}"));
                assert!(smps.iter().any(|s| s["lodm"] == gradients), "{what}: {smps:?}");
            }
        }
    }
}
