// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 2D array (`pvr_native_array`, `gl::native_array`),
//! against docs/es3/pvr-2darray.md: the texture-state words the back end
//! writes for the twiddled layer-major copy (section 3), the `smp` each GLSL
//! form compiles to (section 4.1's table: `tao`, `dmn` 2d, the level mode,
//! `pplod`, `nncoords`, `soo`), the data block's order (coordinates, the
//! level word, the layer address low then high, the lookup word), the
//! record's flag, and the layer arithmetic (section 5: `floor(z + 0.5)`
//! clamped to the image's depth, the 64-bit carry) by running our bytes in
//! the byte model against the IR.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_2darray
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_2D_ARRAY_TWIDDLED, GLES_WRAP_REPEAT,
};
use yakogl::shaderc::online::ir::IoSlot;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::operands::{dsts, srcs};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The switch on for the closure, and back at its default after it, panic
/// or not.
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

/// The `smp` fields of every sample in a program, by name, in program order.
fn smps(image: &[u8]) -> Vec<BTreeMap<&'static str, u32>> {
    const NAMES: [&str; 11] =
        ["fcnorm", "dmn", "chan", "lodm", "pplod", "proj", "nncoords", "soo", "tao", "sbmode", "sno"];
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

/// The data block of the first `smp`: for each of its temporaries (from the
/// one `smp` names as its data operand), the last write before the sample --
/// `Some(v)` for an immediate move of `v`, `None` for a computed value.
fn data_block(image: &[u8]) -> Vec<Option<u32>> {
    let mut off = 0usize;
    /* (temp, Some(imm) | None) in program order until the smp */
    let mut writes: Vec<(u32, Option<u32>)> = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let names: Vec<&str> = ops_for(r, ph).map(|o| o.name).collect();
            if names.contains(&"smp") {
                let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
                let data = lo[1].expect("the smp's data operand");
                let mut block = Vec::new();
                for k in 0..16u32 {
                    let Some((_, w)) = writes.iter().rev().find(|(t, _)| *t == data.index + k) else { break };
                    block.push(*w);
                }
                return block;
            }
            let imm = if names.contains(&"bbyp0bm_imm32") { r.get("imm32") } else { None };
            let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
            for o in d.into_iter().flatten() {
                /* temp is bank 1 (operands.rs) */
                if o.bank == 1 {
                    writes.push((o.index, imm));
                }
            }
        }
        off += g.total;
    }
    panic!("no smp in the program")
}

fn fragment(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\nuniform vec2 g;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nout vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\nuniform vec2 g;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nin vec4 a;\nout vec4 v;\n\
         void main() {{ gl_Position = a; {body} }}\n"
    )
}

/// Section 4.1's table, one row per form: (the look-up, lodm, pplod,
/// nncoords, soo). `lodm`: 1 replace, 2 bias, 3 gradients (pco_isa.py).
const FORMS: &[(&str, &str, u32, u32, u32)] = &[
    ("texture(s, c)", "bias", 1, 0, 0),
    ("texture(s, c, l)", "bias", 1, 0, 0),
    ("textureLod(s, c, l)", "replace", 1, 0, 0),
    ("textureOffset(s, c, ivec2(3, -2))", "bias", 1, 0, 1),
    ("textureOffset(s, c, ivec2(3, -2), l)", "bias", 1, 0, 1),
    ("textureLodOffset(s, c, l, ivec2(-8, 7))", "replace", 1, 0, 1),
    ("texelFetch(s, p, il)", "replace", 1, 1, 0),
    ("texelFetchOffset(s, p, il, ivec2(1, 1))", "replace", 1, 1, 0),
    ("textureGrad(s, c, g, g.yx)", "gradients", 0, 0, 0),
    ("textureGradOffset(s, c, g, g.yx, ivec2(2, 2))", "gradients", 0, 0, 1),
];

/// An `smp` field's enumerated value by name, from the ISA tables
/// (pco_isa.py): `lodm` normal/bias/replace/gradients, `dmn` 1d/2d/3d.
fn enum_value(field: &str, name: &str) -> u32 {
    yakogl::shaderc::online::powervr::isa_tables::ENUMS
        .iter()
        .find(|e| e.name == field)
        .and_then(|e| e.elems.iter().find(|(_, n)| *n == name))
        .map(|(v, _)| *v)
        .unwrap_or_else(|| panic!("{field} has no value {name}"))
}

fn lodm(name: &str) -> u32 {
    enum_value("lodm", name)
}

/// `dmn` 2d: an array is sampled as a 2D image (to_pco_dim, pco_nir_tex.c:106-130).
fn dmn_2d() -> u32 {
    enum_value("dmn", "2d")
}

/// Every form of section 4.1 compiles, in a fragment shader, to one `smp`
/// with `tao`, `dmn` 2d, four channels, `fcnorm` for a float sampler and
/// the row's level mode, `pplod`, `nncoords` and `soo`; the record says
/// native array (bit 1) and not native cube.
#[test]
fn every_form_samples_with_the_address_override_as_the_table_says() {
    for (e, mode, pplod, nncoords, soo) in FORMS {
        let src = fragment("uniform highp sampler2DArray s;", &format!("o = {e};"));
        let (flag, fields) = with_native_array(true, || {
            compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image)))
                .unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(flag, 2, "{e}: the record says native array, not native cube");
        assert_eq!(fields.len(), 1, "{e}: one sample");
        let f = &fields[0];
        let want: BTreeMap<&str, u32> = [
            ("fcnorm", 1),
            ("dmn", dmn_2d()),
            ("chan", 4),
            ("lodm", lodm(mode)),
            ("pplod", *pplod),
            ("proj", 0),
            ("nncoords", *nncoords),
            ("soo", *soo),
            ("tao", 1),
            ("sbmode", 0),
            ("sno", 0),
        ]
        .into_iter()
        .collect();
        assert_eq!(f, &want, "{e}");
    }
}

/// The data block follows pco_emit_nir_smp's order with the address words
/// after the level word: u, v, level (a 0.0 bias when the shader gives
/// none -- a move from a register the allocator already holds zero in, so
/// it shows as computed here; that it IS zero, the IR agreement below
/// shows, a bias word of anything else reading another level of the
/// model -- else the computed level or bias), address low, address high
/// (both computed), then the lookup word (an immediate: u in bits 0..5, v in
/// 6..11, two's complement) -- and with gradients, the two gradient pairs
/// between the coordinates and the address.
#[test]
fn the_data_block_puts_the_address_after_the_level_word() {
    let cases: &[(&str, Vec<Option<u32>>)] = &[
        ("texture(s, c)", vec![None, None, None, None, None]),
        ("texture(s, c, l)", vec![None, None, None, None, None]),
        ("textureLod(s, c, l)", vec![None, None, None, None, None]),
        ("textureOffset(s, c, ivec2(3, -2))", vec![None, None, None, None, None, Some(3 | (0x3e << 6))]),
        ("textureLodOffset(s, c, l, ivec2(-8, 7))", vec![None, None, None, None, None, Some(0x38 | (7 << 6))]),
        ("textureGrad(s, c, g, g.yx)", vec![None, None, None, None, None, None, None, None]),
        (
            "textureGradOffset(s, c, g, g.yx, ivec2(2, 2))",
            vec![None; 8].into_iter().chain([Some(2 | (2 << 6))]).collect(),
        ),
    ];
    for (e, want) in cases {
        let src = fragment("uniform highp sampler2DArray s;", &format!("o = {e};"));
        let block = with_native_array(true, || {
            compile_with(Stage::Fragment, &src, |_, image| data_block(image)).unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        /* every word of the block is written before the sample (the chain
         * zeroes the rest; a zero is a move from a register the allocator
         * keeps zero in, so it shows as computed), and the lookup word is
         * the immediate at the place the order puts it -- after u, v, the
         * level, the two address words (and the gradient pairs) */
        assert_eq!(block.len(), 16, "{e}: the whole block is written");
        for (k, w) in want.iter().enumerate() {
            if w.is_some() {
                assert_eq!(block[k], *w, "{e}: word {k} of the data block");
            }
        }
        assert!(
            block[want.len()..].iter().all(|w| w.is_none()),
            "{e}: no lookup word past the block: {:?}",
            &block[want.len()..]
        );
    }
}

/// An integer sampler's look-up is the same `smp` without `fcnorm`
/// (pco_nir_tex.c:236), on an array as on a 2D image.
#[test]
fn an_integer_array_sampler_samples_without_fcnorm() {
    for (decl, e) in [
        ("uniform highp isampler2DArray s;", "vec4(texture(s, c))"),
        ("uniform highp usampler2DArray s;", "vec4(texelFetch(s, p, il))"),
        ("uniform highp isampler2DArray s;", "vec4(textureGrad(s, c, g, g))"),
    ] {
        let src = fragment(decl, &format!("o = {e};"));
        let fields = with_native_array(true, || {
            compile_with(Stage::Fragment, &src, |_, image| smps(image)).unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!((fields.len(), fields[0]["fcnorm"], fields[0]["tao"], fields[0]["dmn"]), (1, 0, 1, dmn_2d()), "{e}");
    }
}

/// The stacked images (a cube, a 3D texture and a 2D array kept as one tall
/// 2D image) are read by `isampler*` / `usampler*` as well: every `smp` the
/// look-up emits hands back the texel's integers (`fcnorm` clear), and a 3D
/// look-up reads its nearest slice with one sample, not the two a float one
/// mixes.
#[test]
fn the_stacked_integer_samplers_sample_without_fcnorm() {
    let layers = "uniform float pvr_layers_s;\n";
    for (decl, e) in [
        ("uniform highp isamplerCube s;".to_string(), "vec4(texture(s, c))"),
        ("uniform highp usamplerCube s;".to_string(), "vec4(textureLod(s, c, l))"),
        (format!("{layers}uniform highp isampler3D s;"), "vec4(texture(s, c))"),
        (format!("{layers}uniform highp usampler3D s;"), "vec4(texelFetch(s, p, il))"),
        (format!("{layers}uniform highp isampler2DArray s;"), "vec4(texture(s, c))"),
        (format!("{layers}uniform highp usampler2DArray s;"), "vec4(texelFetch(s, p, il))"),
    ] {
        let src = fragment(&decl, &format!("o = {e};"));
        let fields = with_native_array(false, || {
            compile_with(Stage::Fragment, &src, |_, image| smps(image)).unwrap_or_else(|m| panic!("{decl} {e}: {m}"))
        });
        assert_eq!(fields.len(), 1, "{decl} {e}: one sample");
        assert_eq!(fields[0]["fcnorm"], 0, "{decl} {e}: no fcnorm");
    }
}

/// A vertex shader has no derivative: its forms carry a level
/// (`textureLod`, `texelFetch`, the gradients, and `texture`, which the
/// front end gives level 0) and compile to the same `smp` through the vertex
/// chain; the record says native array.
#[test]
fn the_vertex_stage_samples_an_array_the_same_way() {
    for (e, mode, nncoords) in [
        ("textureLod(s, c, l)", "replace", 0),
        ("texture(s, c)", "replace", 0),
        ("texelFetch(s, p, il)", "replace", 1),
        ("textureGrad(s, c, g, g.yx)", "gradients", 0),
        ("textureLodOffset(s, c, l, ivec2(1, -1))", "replace", 0),
    ] {
        let src = vertex("uniform highp sampler2DArray s;", &format!("v = {e};"));
        let (flag, fields) = with_native_array(true, || {
            compile_with(Stage::Vertex, &src, |rec, image| (rec.native_cube, smps(image)))
                .unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(flag, 2, "{e}: the record says native array");
        assert_eq!(fields.len(), 1, "{e}");
        assert_eq!(
            (fields[0]["tao"], fields[0]["dmn"], fields[0]["lodm"], fields[0]["nncoords"]),
            (1, dmn_2d(), lodm(mode), nncoords),
            "{e}"
        );
    }
}

/// With the switch off the same sources keep the stacked image: no `tao`,
/// a flag of 0, and the forms the stacked image cannot take are refused as
/// before (an offset, explicit gradients, an integer sampler).
#[test]
fn off_the_stacked_image_stays_and_the_record_says_nothing() {
    let stacked = ["texture(s, c)", "texture(s, c, l)", "textureLod(s, c, l)", "texelFetch(s, p, il)"];
    for e in stacked {
        let src = fragment("uniform float pvr_layers_s;\nuniform highp sampler2DArray s;", &format!("o = {e};"));
        let (flag, fields) = with_native_array(false, || {
            compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image)))
                .unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(flag, 0, "{e}: off, the stacked array");
        assert_eq!(fields[0]["tao"], 0, "{e}: off, no address override");
    }
    for (e, _, _, _, _) in FORMS.iter().filter(|f| !stacked.contains(&f.0)) {
        let src = fragment("uniform float pvr_layers_s;\nuniform highp sampler2DArray s;", &format!("o = {e};"));
        let got = with_native_array(false, || compile_with(Stage::Fragment, &src, |_, _| ()));
        assert!(got.is_err(), "{e}: off, the stacked image refuses it as before");
    }
}

/// A dead array look-up takes its sample, its address and its gradients
/// with it: the program compiles and the record says nothing (the native
/// cube's rule, tests/usc_limits.rs a_dead_cube_sample_takes_its_z_with_it).
/// A program with a native cube and a native array says both (3).
#[test]
fn a_dead_array_sample_takes_its_address_with_it_and_a_cube_beside_it_sets_both_bits() {
    let dead = fragment("uniform highp sampler2DArray s;", "vec4 t = textureGrad(s, c, g, g); o = vec4(c, 1.0);");
    let flag = with_native_array(true, || compile_with(Stage::Fragment, &dead, |rec, _| rec.native_cube).unwrap());
    assert_eq!(flag, 0, "a dead array look-up: compiled, nothing native left");
    let both =
        fragment("uniform highp sampler2DArray s;\nuniform highp samplerCube k;", "o = texture(s, c) + texture(k, c);");
    let flag = with_native_array(true, || compile_with(Stage::Fragment, &both, |rec, _| rec.native_cube).unwrap());
    assert_eq!(flag, 3, "a native cube and a native array in one program");
}

/// `textureSize` of a native array: one layer's size at the level and the
/// layer count from the image word's `depth` field plus one (section 3.5),
/// no `smp` and no `pvr_layers_` uniform read -- the program compiles
/// without that uniform declared and reads descriptor dword 2.
#[test]
fn texture_size_reads_the_layer_count_from_the_image_word() {
    let src = fragment("uniform highp sampler2DArray s;", "o = vec4(vec3(textureSize(s, il)) / 2048.0, 1.0);");
    let (flag, n) = with_native_array(true, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image).len())).unwrap()
    });
    assert_eq!((flag, n), (0, 0), "no sample, and textureSize alone binds no copy");
    /* its value: the byte model against the IR (which reads the same
     * descriptor words) at levels 0 and 1 */
    let level0 = |_n: &str, _c: u32| 0;
    let level1 = |n: &str, _c: u32| if n == "il" { 1 } else { 0 };
    let px = run_against_ir(&src, &[&level0, &level1]);
    let depth = usc_exec::tex_depth(sampler_marker("s"));
    let count = (((depth + 1) as f32 / 2048.0).clamp(0.0, 1.0) * 255.0).round() as u32;
    assert_eq!((px[0] >> 16) & 0xff, count, "the layer count, from the depth field");
    assert_eq!((px[1] >> 16) & 0xff, count, "unshifted by the level");
}

/// The marker of sampler `s` in the model (usc_exec::tex_state is derived
/// from it, as usc_limits.rs `sampler_marker` names it).
fn sampler_marker(name: &str) -> u32 {
    usc_exec::named(7, name, 99)
}

type InputFn<'a> = dyn Fn(&str, u32) -> u32 + 'a;

/// Our program for a fragment shader with the one sampler `s` against the
/// IR, once per input set (a uniform name and component to bits), with
/// the native array on for both the compile and the IR; the packed pixels,
/// each asserted equal to the IR's. The shader must read no varying.
fn run_against_ir(src: &str, inputs: &[&InputFn<'_>]) -> Vec<u32> {
    with_native_array(true, || {
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
                /* the shared file as the driver fills it: one 20-dword
                 * descriptor per sampler row (image words, then the meta a
                 * native array reads), then the uniform block by name */
                let mut samplers: Vec<&yakogl::shaderc::ShcUniform> =
                    rows.iter().filter(|u| u.dword_off == no_block).collect();
                samplers.sort_by_key(|u| u.binding);
                for (i, u) in samplers.iter().enumerate() {
                    let marker = sampler_marker(&name_of(&u.name));
                    inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(marker));
                    inp.shared[20 * i + 4..20 * i + 7].copy_from_slice(&usc_exec::tex_meta(marker));
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

/// Section 5 and section 10's test 6: the layer is `floor(z + 0.5)` clamped
/// to `0 ..= depth` (2.5 rounds to 3, not Mesa's even 2; -1 and 1e9 clamp),
/// the address `base + layer * layer_size` with the carry into the high
/// dword (the model's base is a few layers below the 32-bit wrap:
/// usc_exec::tex_meta), read back from the `tao` words by the byte model,
/// and the program agrees with the IR at every layer; a `texelFetch` takes
/// the integer itself, clamped.
#[test]
fn the_layer_is_floor_of_z_plus_half_clamped_to_the_depth_with_a_carry() {
    let depth = usc_exec::tex_depth(sampler_marker("s"));
    assert!(depth >= 4, "the model's depth for `s` is {depth}: pick another sampler name");
    let zs: Vec<f32> = vec![-1.0, 0.0, 0.49, 0.5, 1.5, 2.5, depth as f32, depth as f32 + 1.0, 1.0e9];
    let want: Vec<u32> = zs.iter().map(|z| ((z + 0.5).floor().max(0.0).min(depth as f32)) as u32).collect();
    assert_eq!(want[5], 3, "2.5 rounds up");
    let src = fragment("uniform highp sampler2DArray s;", "o = texture(s, c);");
    let cases: Vec<Box<InputFn<'_>>> = zs
        .iter()
        .map(|z| -> Box<InputFn<'_>> {
            let z = *z;
            Box::new(move |n: &str, k: u32| match n {
                "c" => [0.3f32, 0.8, z][k as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let _ = usc_exec::take_layers();
    let px = run_against_ir(&src, &refs);
    assert_eq!(usc_exec::take_layers(), want, "the layer each sample addressed");
    assert!(px[1] != px[4] && px[4] != px[5], "the layers read different texels: {px:08x?}");

    /* the integer form: texelFetch's layer is the integer, clamped */
    let src = fragment("uniform highp sampler2DArray s;", "o = texelFetch(s, p, il);");
    let ps = [-3i32, 0, 2, depth as i32, depth as i32 + 5];
    let cases: Vec<Box<InputFn<'_>>> = ps
        .iter()
        .map(|z| -> Box<InputFn<'_>> {
            let z = *z;
            Box::new(move |n: &str, k: u32| match n {
                "p" => [5i32, 9, z][k as usize] as u32,
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let _ = usc_exec::take_layers();
    run_against_ir(&src, &refs);
    assert_eq!(usc_exec::take_layers(), vec![0, 0, 2, depth, depth]);
}

/// The other forms agree with the IR too: an explicit level and a bias, an
/// offset (the lookup word), a level with an offset, explicit gradients (the
/// model's level 0) and texelFetchOffset, at a layer that carries.
#[test]
fn every_form_agrees_with_the_ir() {
    let forms = [
        "texture(s, c, l)",
        "textureLod(s, c, l)",
        "textureOffset(s, c, ivec2(3, -2))",
        "textureLodOffset(s, c, l, ivec2(-8, 7))",
        "textureGrad(s, c, g, g.yx)",
        "textureGradOffset(s, c, g, g.yx, ivec2(2, 2))",
        "texelFetchOffset(s, p, il, ivec2(1, 1))",
        "0.5 * (texture(s, c) + textureLod(s, c.yxz, l))",
    ];
    for e in forms {
        let src = fragment("uniform highp sampler2DArray s;", &format!("o = {e};"));
        let cases: Vec<Box<InputFn<'_>>> = [(1.2f32, 0.0f32), (2.6, 1.0), (3.0, 2.0)]
            .into_iter()
            .map(|(z, l)| -> Box<InputFn<'_>> {
                Box::new(move |n: &str, k: u32| match n {
                    "c" => [0.3f32, 0.8, z][k as usize].to_bits(),
                    "g" => [0.01f32, 0.02][k as usize].to_bits(),
                    "l" => l.to_bits(),
                    "p" => [5i32, 9, z as i32][k as usize] as u32,
                    "il" => l as u32,
                    _ => 0,
                })
            })
            .collect();
        let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
        let _ = usc_exec::take_layers();
        let px = run_against_ir(&src, &refs);
        /* z 1.2 -> 1, 2.6 -> 3 and 3.0 -> 3 rounded; 1, 2, 3 as integers */
        let layers = usc_exec::take_layers();
        assert!(layers.iter().all(|l| [1, 2, 3].contains(l)) && layers.contains(&3), "{e}: layers {layers:?}");
        assert!(px[0] != px[1] || px[1] != px[2], "{e}: every input read the same: {px:08x?}");
    }
}

/// A loop keeps the sample's data block honest: the address words of an
/// array sample inside a loop body are written on every iteration, and the
/// block's later words are zeroed again where an earlier sample wrote data
/// (chain.rs's loop zero-reset counts the two address words).
#[test]
fn an_array_sample_in_a_loop_agrees_with_the_ir() {
    let src = fragment(
        "uniform highp sampler2DArray s;\nuniform int n;",
        "vec4 acc = vec4(0.0); for (int i = 0; i < n; i++) { acc += 0.25 * textureOffset(s, vec3(c.xy, float(i)), ivec2(1, 0)); } o = acc;",
    );
    let cases: Vec<Box<InputFn<'_>>> = [1u32, 3]
        .into_iter()
        .map(|n| -> Box<InputFn<'_>> {
            Box::new(move |name: &str, k: u32| match name {
                "c" => [0.3f32, 0.8, 0.0][k as usize].to_bits(),
                "n" => n,
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let _ = usc_exec::take_layers();
    let px = run_against_ir(&src, &refs);
    /* the byte model executes a masked-off iteration too (the execution
     * mask hides its result), so one layer past n appears per run */
    let layers = usc_exec::take_layers();
    assert!(layers.starts_with(&[0]) && layers.ends_with(&[0, 1, 2, 3]), "{layers:?}");
    assert_ne!(px[0], px[1]);
}

/// Section 3's state words for the twiddled layer-major copy, through the
/// back end's descriptor: TEXTYPE_2D of one layer's size, `depth` of
/// `layers - 1`, the chain's level count and `mipmaps_present`, one layer's
/// bytes in meta dword 4, the base in meta dwords 5 and 6 (low first, the
/// heap's high dword 0x80); and the stacked bind of the same array is the
/// stride image it always was.
#[test]
fn the_descriptor_of_the_copy_is_a_twiddled_2d_with_the_layer_count_and_the_base() {
    let base = 0x80_1234_5000u64;
    let t = GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_: GLES_TEXTYPE_2D_ARRAY_TWIDDLED,
        layers: 5,
        base_gpu: base,
        width: 32,
        height: 8,
        stride_texels: 32,
        bytes: 5 * 4096,
        mip_filter: 2,
        mip_levels: 6,
        linear_filter: 1,
        min_linear: 1,
        wrap_s: GLES_WRAP_REPEAT,
        wrap_t: GLES_WRAP_REPEAT,
        ..GlesTextureBind::ZERO
    };
    let w = yakogl::pvr::backend::texture_descriptor(&t);
    let iw0 = w[0] as u64 | (w[1] as u64) << 32;
    let iw1 = w[2] as u64 | (w[3] as u64) << 32;
    assert_eq!(iw0 & 7, 1, "TEXTYPE_2D");
    assert_eq!(((iw0 >> 34) & 0x3fff, (iw0 >> 48) & 0x3fff), (31, 7), "one layer's size");
    assert_eq!(iw1 & 0xf, 6, "32x8: 32x8, 16x4, 8x2, 4x1, 2x1, 1x1");
    assert_eq!((iw1 >> 4) & 0x7ff, 4, "depth = layers - 1");
    assert_eq!((iw1 >> 15) & 1, 1, "mipmaps_present");
    assert_eq!((iw1 >> 16) & 0x3F_FFFF_FFFF, base >> 2, "texaddr");
    assert_eq!(iw1 >> 60, 0, "baselevel 0");
    assert_eq!(w[4], 4096, "one layer's bytes");
    assert_eq!((w[5], w[6]), (0x1234_5000, 0x80), "the base, low dword first");
    /* the sampler: the LOD range covers the written levels (maxlod u4.6,
     * SAMPLER_WORD0 bits 23..32 as src/pvr/backend.rs's tests read it) */
    assert_eq!(((w[8] >> 23) & 0x1ff) | ((w[9] & 1) << 9), 5 * 64, "maxlod over six levels");
    let stacked = yakogl::pvr::backend::texture_descriptor(&GlesTextureBind {
        type_: GLES_TEXTYPE_2D_ARRAY,
        height: 40,
        bytes: 32 * 40 * 4,
        ..t
    });
    assert_eq!(stacked[0] & 7, 4, "the stacked array: a STRIDE image");
    assert_eq!((stacked[5], stacked[6]), (0, 0), "no base in its meta");
}

/// The sampler names a record lists as uniform rows.
fn sampler_rows(rec: &ShcShader) -> Vec<String> {
    // SAFETY: the record's `num_uniforms` rows live in the arena for this call.
    let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, usize::from(rec.num_uniforms)) };
    rows.iter()
        .filter(|u| u.dword_off == yakogl::shaderc::SHC_UNIFORM_NO_BLOCK)
        .map(|u| {
            let bytes: Vec<u8> = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            String::from_utf8(bytes).unwrap()
        })
        .collect()
}

/// A vertex shader that reads a sampler only through `textureSize` still
/// lists its row: without it `glGetUniformLocation` is -1 and the unit
/// cannot be bound, so `textureSize` reads another unit's size.
#[test]
fn a_vertex_texture_size_alone_keeps_the_sampler_row() {
    for decl in ["uniform highp sampler2D s;", "uniform highp sampler2DArray s;"] {
        for body in ["v = vec4(textureSize(s, il).xyxy);", "v = vec4(textureSize(s, 0).xyxy);"] {
            let src = vertex(decl, body);
            let rows =
                with_native_array(false, || compile_with(Stage::Vertex, &src, |rec, _| sampler_rows(rec)).unwrap());
            assert_eq!(rows, ["s"], "{decl}, {body}: the textureSize-only sampler keeps its row");
        }
    }
}
