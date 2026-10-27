// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's native 3D texture (`pvr_native_3d`, `gl::native_vol`),
//! against docs/notes/pvr-3d-native.md: a sampler3D compiles to one `smp`
//! with `dmn` 3d over the three coordinates (the TPU picks the level,
//! filters across slices and wraps every axis), the record says native 3D
//! (`native_cube` bit 2), `textureSize` reads the image word, and with the
//! switch off the grid of slices (two 2D samples mixed, level 0) is what
//! comes out, as before.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_3d
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::ShcShader;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The switch on or off for the closure, and back at its default after it,
/// panic or not.
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

fn fragment(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\nuniform vec3 g;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nout vec4 o;\nvoid main() {{ {body} }}\n"
    )
}

fn vertex(decl: &str, body: &str) -> String {
    format!(
        "#version 300 es\nprecision highp float;\nprecision highp int;\n{decl}\nuniform vec3 c;\n\
         uniform float l;\nuniform ivec3 p;\nuniform int il;\nin vec4 a;\nout vec4 v;\n\
         void main() {{ gl_Position = a; {body} }}\n"
    )
}

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

/// One row per form of a sampler3D look-up: (the look-up, lodm, pplod,
/// nncoords). The level word is in the data block for a bias or a replaced
/// level (`pplod`), absent for the implicit one.
const FORMS: &[(&str, &str, u32, u32)] = &[
    ("texture(s, c)", "normal", 0, 0),
    ("texture(s, c, l)", "bias", 1, 0),
    ("textureLod(s, c, l)", "replace", 1, 0),
    ("textureProj(s, vec4(c, 2.0))", "normal", 0, 0),
    ("texelFetch(s, p, il)", "replace", 1, 1),
];

/// Every form compiles, in a fragment shader, to one `smp` with `dmn` 3d,
/// no `tao`, four channels, `fcnorm` for a float sampler and the row's level
/// mode; the record says native 3D (bit 2) and neither cube nor array.
#[test]
fn every_form_samples_the_tpus_3d_texture() {
    for (e, mode, pplod, nncoords) in FORMS {
        let src = fragment("uniform highp sampler3D s;", &format!("o = {e};"));
        let (flag, fields) = with_native_3d(true, || {
            compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image)))
                .unwrap_or_else(|m| panic!("{e}: {m}"))
        });
        assert_eq!(flag, 4, "{e}: the record says native 3D");
        assert_eq!(fields.len(), 1, "{e}: one sample, not two slices mixed");
        let want: BTreeMap<&str, u32> = [
            ("fcnorm", 1),
            ("dmn", enum_value("dmn", "3d")),
            ("chan", 4),
            ("lodm", enum_value("lodm", mode)),
            ("pplod", *pplod),
            ("proj", 0),
            ("nncoords", *nncoords),
            ("soo", 0),
            ("tao", 0),
            ("sbmode", 0),
            ("sno", 0),
        ]
        .into_iter()
        .collect();
        assert_eq!(fields[0], want, "{e}");
    }
}

/// A vertex shader samples the same way (textureLod is its only form).
#[test]
fn a_vertex_shader_samples_the_volume_too() {
    let src = vertex("uniform highp sampler3D s;", "v = textureLod(s, c, l);");
    let (flag, fields) = with_native_3d(true, || {
        compile_with(Stage::Vertex, &src, |rec, image| (rec.native_cube, smps(image))).unwrap()
    });
    assert_eq!(flag, 4);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["dmn"], enum_value("dmn", "3d"));
    assert_eq!(fields[0]["lodm"], enum_value("lodm", "replace"));
}

/// With the switch off the volume stays the grid of slices: the two slices
/// around the depth, sampled as 2D images and mixed -- two `smp`, `dmn` 2d,
/// the record clear.
#[test]
fn off_the_volume_stays_the_grid_of_slices() {
    let src = fragment("uniform highp sampler3D s;", "o = texture(s, c);");
    let (flag, fields) = with_native_3d(false, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image))).unwrap()
    });
    assert_eq!(flag, 0);
    assert_eq!(fields.len(), 2, "two slices");
    assert!(fields.iter().all(|f| f["dmn"] == enum_value("dmn", "2d")));
}

/// A dead volume look-up takes its sample and its third coordinate with it:
/// the program compiles and the record says nothing. A program with a cube,
/// an array-free volume and a native cube says both bits (5).
#[test]
fn a_dead_sample_sets_nothing_and_a_cube_beside_a_volume_sets_both_bits() {
    let dead = fragment("uniform highp sampler3D s;", "vec4 t = texture(s, c); o = vec4(c, 1.0);");
    let flag = with_native_3d(true, || compile_with(Stage::Fragment, &dead, |rec, _| rec.native_cube).unwrap());
    assert_eq!(flag, 0, "a dead look-up: compiled, nothing native left");
    let both =
        fragment("uniform highp sampler3D s;\nuniform highp samplerCube k;", "o = texture(s, c) + texture(k, c);");
    let flag = with_native_3d(true, || {
        yakogl::shaderc::pvr_set_native_cube(1);
        let f = compile_with(Stage::Fragment, &both, |rec, _| rec.native_cube);
        yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        f.unwrap()
    });
    assert_eq!(flag, 5, "a native cube (bit 0) and a native volume (bit 2) in one program");
}

/// `textureSize` of a native volume reads the image word: no `smp`, and the
/// program compiles with a level argument. The word is the twiddled copy's
/// (a grid's is a stride image with no depth), so the record asks for that
/// copy (bit 2) even though nothing is sampled.
#[test]
fn texture_size_reads_the_image_word_and_samples_nothing() {
    let src = fragment("uniform highp sampler3D s;", "o = vec4(textureSize(s, il), 1.0);");
    let (flag, n) = with_native_3d(true, || {
        compile_with(Stage::Fragment, &src, |rec, image| (rec.native_cube, smps(image).len())).unwrap()
    });
    assert_eq!((flag, n), (4, 0));
}
