// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `textureSize` on the PowerVR online back end, as dEQP-GLES3
//! `shaders.texture_functions.texturesize.*` asks it (VK-GL-CTS 93bca01,
//! es3fShaderTextureFunctionTests.cpp `TextureSizeCase`): dEQP's own
//! shader for each of its 19 sampler kinds, compiled by the chain and run in
//! the byte model over the descriptor `src/pvr/backend.rs` writes for the
//! texture of each row of dEQP's table, must light the pixel white for GL's
//! size of the level (ES 3.0.6 3.8.11: each side halved `lod` times, at
//! least 1; an array's third component is its layer count) and black for
//! any other.
//!
//! Where the sizes come from: a 2D sampler reads the image word (width,
//! height, the base level of a twiddled word) as pco's
//! usclib_tex_state_size does; a stacked array or cube divides the stacked
//! image's height by the layer count (glsl2usc's pvr_arrSize, pvr_cubeSize),
//! a stacked 3D texture reads its grid of slices (pvr_volSize) and halves
//! what it finds; the native array and the native 3D texture
//! (`pvr_native_array`, `pvr_native_3d`) read the layer or depth count from
//! the twiddled word's `depth` field.
//!
//! Not covered here, and still failing the 38 board cases: dEQP's three rows
//! with GL_TEXTURE_BASE_LEVEL 1, which the draw does not apply
//! (src/gl/draw.rs `mip_levels_needed`: "the draw does not apply
//! GL_TEXTURE_BASE_LEVEL"). The last test pins what the compiler does with
//! a base level the descriptor carries, so that the GL core's part can
//! land on its own.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_texsize
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use std::collections::BTreeMap;
use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_2D_ARRAY_TWIDDLED,
    GLES_TEXTYPE_3D, GLES_TEXTYPE_3D_TWIDDLED, GLES_TEXTYPE_CUBE, GLES_WRAP_CLAMP_TO_EDGE,
};
use yakogl::pvr::backend::texture_descriptor;
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::SHC_UNIFORM_NO_BLOCK;

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the native switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The native array and native 3D switches set for the closure and back at
/// their defaults after it, panic or not.
fn with_native<T>(array: bool, vol: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_array(i32::from(array));
    yakogl::shaderc::pvr_set_native_3d(i32::from(vol));
    f()
}

/// One uniform row of a record: its name, where it sits in the block
/// (`None` for a sampler) and its dwords.
struct Row {
    name: String,
    dword_off: Option<usize>,
    dwords: usize,
}

/// A compiled program: its bytes and its uniform rows.
struct Prog {
    image: Vec<u8>,
    rows: Vec<Row>,
}

/// One shader through the online PowerVR chain (under `with_native`, which
/// holds `COMPILING`), or the refusal.
fn compile(stage: Stage, src: &str) -> Result<Prog, String> {
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
        let rows = rows
            .iter()
            .map(|u| Row {
                name: u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect(),
                dword_off: (u.dword_off != SHC_UNIFORM_NO_BLOCK).then_some(u.dword_off as usize),
                dwords: u.dwords as usize,
            })
            .collect();
        Ok(Prog { image: code.iter().flat_map(|w| w.to_le_bytes()).collect(), rows })
    })
    .unwrap()
}

/// dEQP's table (`TextureSizeCase::iterate`, `testSizes`): the texture's
/// size, the level asked, GL_TEXTURE_BASE_LEVEL, and the size it expects (a
/// 2D array's third component is replaced by the layer count when the
/// uniform is set).
const TEST_SIZES: &[([u32; 3], u32, u32, [u32; 3])] = &[
    ([1, 2, 1], 1, 0, [1, 1, 1]),
    ([1, 2, 1], 0, 0, [1, 2, 1]),
    ([1, 3, 2], 0, 0, [1, 3, 2]),
    ([1, 3, 2], 1, 0, [1, 1, 1]),
    ([100, 31, 18], 0, 0, [100, 31, 18]),
    ([100, 31, 18], 1, 0, [50, 15, 9]),
    ([100, 31, 18], 2, 0, [25, 7, 4]),
    ([100, 31, 18], 3, 0, [12, 3, 2]),
    ([100, 31, 18], 4, 0, [6, 1, 1]),
    ([100, 31, 18], 5, 0, [3, 1, 1]),
    ([100, 31, 18], 6, 0, [1, 1, 1]),
    ([100, 128, 32], 0, 0, [100, 128, 32]),
    ([100, 128, 32], 1, 0, [50, 64, 16]),
    ([100, 128, 32], 2, 0, [25, 32, 8]),
    ([100, 128, 32], 3, 0, [12, 16, 4]),
    ([100, 128, 32], 4, 0, [6, 8, 2]),
    ([100, 128, 32], 5, 0, [3, 4, 1]),
    ([100, 128, 32], 6, 0, [1, 2, 1]),
    ([100, 128, 32], 7, 0, [1, 1, 1]),
    ([128, 64, 32], 0, 0, [128, 64, 32]),
    ([128, 64, 32], 1, 0, [64, 32, 16]),
    ([128, 64, 32], 2, 0, [32, 16, 8]),
    ([128, 64, 32], 3, 0, [16, 8, 4]),
    ([128, 64, 32], 4, 0, [8, 4, 2]),
    ([128, 64, 32], 5, 0, [4, 2, 1]),
    ([128, 64, 32], 6, 0, [2, 1, 1]),
    ([128, 64, 32], 7, 0, [1, 1, 1]),
    ([1, 1, 1], 0, 0, [1, 1, 1]),
    ([64, 64, 64], 0, 0, [64, 64, 64]),
    ([64, 64, 64], 1, 0, [32, 32, 32]),
    ([64, 64, 64], 2, 0, [16, 16, 16]),
    ([64, 64, 64], 3, 0, [8, 8, 8]),
    ([64, 64, 64], 4, 0, [4, 4, 4]),
    ([100, 31, 18], 3, 1, [6, 1, 1]),
    ([128, 64, 32], 3, 1, [8, 4, 2]),
    ([64, 64, 64], 1, 1, [16, 16, 16]),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    D2,
    D2Array,
    D3,
    Cube,
}

/// dEQP's 19 sampler kinds (the case name less `_vertex` / `_fragment`, the
/// GLSL type, the texture it binds).
const SAMPLERS: &[(&str, &str, Kind)] = &[
    ("sampler2d_fixed", "sampler2D", Kind::D2),
    ("sampler2d_float", "sampler2D", Kind::D2),
    ("isampler2d", "isampler2D", Kind::D2),
    ("usampler2d", "usampler2D", Kind::D2),
    ("sampler2dshadow", "sampler2DShadow", Kind::D2),
    ("sampler3d_fixed", "sampler3D", Kind::D3),
    ("sampler3d_float", "sampler3D", Kind::D3),
    ("isampler3d", "isampler3D", Kind::D3),
    ("usampler3d", "usampler3D", Kind::D3),
    ("samplercube_fixed", "samplerCube", Kind::Cube),
    ("samplercube_float", "samplerCube", Kind::Cube),
    ("isamplercube", "isamplerCube", Kind::Cube),
    ("usamplercube", "usamplerCube", Kind::Cube),
    ("samplercubeshadow", "samplerCubeShadow", Kind::Cube),
    ("sampler2darray_fixed", "sampler2DArray", Kind::D2Array),
    ("sampler2darray_float", "sampler2DArray", Kind::D2Array),
    ("isampler2darray", "isampler2DArray", Kind::D2Array),
    ("usampler2darray", "usampler2DArray", Kind::D2Array),
    ("sampler2darrayshadow", "sampler2DArrayShadow", Kind::D2Array),
];

/// `textureSize`'s components for the kind (dEQP's `m_has3DSize`).
fn components(kind: Kind) -> usize {
    if matches!(kind, Kind::D3 | Kind::D2Array) {
        3
    } else {
        2
    }
}

/// dEQP's fragment shader for the fragment case (`genFragmentShader`): the
/// size against `u_texSize`, white for equal and opaque black otherwise.
fn fragment_source(sampler: &str, kind: Kind) -> String {
    format!(
        "#version 300 es\n\
         layout(location = 0) out mediump vec4 o_color;\n\
         uniform highp {sampler} u_sampler;\n\
         uniform highp ivec{n} u_texSize;\n\
         uniform highp int u_lod;\n\
         void main()\n{{\n\
         \x20   o_color = (textureSize(u_sampler, u_lod) == u_texSize ? vec4(1.0, 1.0, 1.0, 1.0) : vec4(0.0, 0.0, 0.0, 1.0));\n\
         }}\n",
        n = components(kind)
    )
}

/// dEQP's vertex shader for the vertex case (`genVertexShader`): the same
/// comparison into `v_color`, the position passed through.
fn vertex_source(sampler: &str, kind: Kind) -> String {
    format!(
        "#version 300 es\n\
         in highp vec4 a_position;\n\
         out mediump vec4 v_color;\n\
         uniform highp {sampler} u_sampler;\n\
         uniform highp ivec{n} u_texSize;\n\
         uniform highp int u_lod;\n\
         void main()\n{{\n\
         \x20   v_color = (textureSize(u_sampler, u_lod) == u_texSize ? vec4(1.0, 1.0, 1.0, 1.0) : vec4(0.0, 0.0, 0.0, 1.0));\n\
         \x20   gl_Position = a_position;\n\
         }}\n",
        n = components(kind)
    )
}

/// The grid a 3D texture's slices are tiled in (src/gl/texture.rs
/// `grid_cols_for`): the smallest `c` with `c * c >= depth`.
fn grid_cols(depth: u32) -> u32 {
    (1u32..).find(|c| c * c >= depth).unwrap()
}

/// The texture the GL core binds for a `w x h x d` texture of `levels`
/// levels of the kind, filtered NEAREST as dEQP's is (src/gl/draw.rs: a
/// stacked array or cube is one tall image, a stacked 3D texture a grid of
/// `grid_cols` columns; a native array or volume one layer's size with the
/// count in `layers`).
fn bind(kind: Kind, native: bool, [w, h, d]: [u32; 3], levels: u32) -> GlesTextureBind {
    let (type_, width, height, layers) = match (kind, native) {
        (Kind::D2, _) => (GLES_TEXTYPE_2D, w, h, 1),
        (Kind::Cube, _) => (GLES_TEXTYPE_CUBE, w, h * 6, 6),
        (Kind::D2Array, false) => (GLES_TEXTYPE_2D_ARRAY, w, h * d, d),
        (Kind::D2Array, true) => (GLES_TEXTYPE_2D_ARRAY_TWIDDLED, w, h, d),
        (Kind::D3, false) => {
            let cols = grid_cols(d);
            (GLES_TEXTYPE_3D, w * cols, h * d.div_ceil(cols), d)
        }
        (Kind::D3, true) => (GLES_TEXTYPE_3D_TWIDDLED, w, h, d),
    };
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_,
        layers: layers as u16,
        base_gpu: 0x80_0000_9000,
        width,
        height,
        stride_texels: width,
        bytes: width * height * 4 * if native { layers } else { 1 },
        linear_filter: 0,
        min_linear: 0,
        mip_filter: 0,
        mip_levels: levels as u8,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_r: GLES_WRAP_CLAMP_TO_EDGE,
        ..GlesTextureBind::ZERO
    }
}

/// GL's `textureSize(s, lod)` of the kind's `w x h x d` texture, or `None`
/// for a row dEQP skips (a cube that is not square; a 2D size whose level
/// is past both sides' last, `is2DLodValid`).
fn expected(kind: Kind, [w, h, d]: [u32; 3], lod: u32) -> Option<Vec<u32>> {
    let shr = |x: u32| x.checked_shr(lod).unwrap_or(0);
    let at = |x: u32| shr(x).max(1);
    let lod_valid = shr(w) != 0 || shr(h) != 0;
    match kind {
        Kind::D2 => lod_valid.then(|| vec![at(w), at(h)]),
        Kind::Cube => (w == h).then(|| vec![at(w), at(h)]),
        Kind::D2Array => lod_valid.then(|| vec![at(w), at(h), d]),
        Kind::D3 => Some(vec![at(w), at(h), at(d)]),
    }
}

const WHITE: u32 = 0xffff_ffff;
const BLACK: u32 = 0xff00_0000;

/// The program's outputs over descriptor `desc` (shared dwords 0..20, where
/// the chain puts the one sampler's descriptor) and the uniforms `u_lod`,
/// `u_texSize` and the stacked `pvr_layers_u_sampler` by name after it.
fn run(p: &Prog, desc: &[u32; 20], layers: u32, lod: u32, tex_size: &[u32]) -> BTreeMap<u32, u32> {
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    inp.shared[..20].copy_from_slice(desc);
    let base = 20;
    for r in &p.rows {
        let Some(off) = r.dword_off else { continue };
        for k in 0..r.dwords {
            inp.shared[base + off + k] = match r.name.as_str() {
                "u_lod" => lod,
                "u_texSize" => tex_size[k],
                "pvr_layers_u_sampler" => (layers as f32).to_bits(),
                other => panic!("a uniform row this test does not set: {other}"),
            };
        }
    }
    usc_exec::run_binary(&p.image, &inp, true).unwrap_or_else(|e| panic!("the byte model refused: {e}"))
}

/// The pixel of the fragment program.
fn pixel(p: &Prog, desc: &[u32; 20], layers: u32, lod: u32, tex_size: &[u32]) -> u32 {
    *run(p, desc, layers, lod, tex_size).get(&0).expect("a pixel")
}

/// Every row of dEQP's table with base level 0, for every sampler kind, in
/// the stage: white for GL's size and black for a size one texel wider. The
/// native array and native 3D arms when `native`. Returns the rows checked.
fn check_table(stage: Stage, native: bool) -> usize {
    let mut checked = 0;
    for &(case, sampler, kind) in SAMPLERS {
        if native && !matches!(kind, Kind::D2Array | Kind::D3) {
            continue;
        }
        let src = if stage == Stage::Fragment { fragment_source(sampler, kind) } else { vertex_source(sampler, kind) };
        let p = with_native(native, native, || compile(stage, &src)).unwrap_or_else(|m| panic!("{case}: {m}"));
        for &(size, lod, base, _) in TEST_SIZES {
            if base != 0 {
                continue;
            }
            let Some(want) = expected(kind, size, lod) else { continue };
            let desc = texture_descriptor(&bind(kind, native, size, lod + 1));
            let mut wrong = want.clone();
            wrong[0] += 1;
            let colour = |tex_size: &[u32]| -> u32 {
                let out = run(&p, &desc, size[2], lod, tex_size);
                if stage == Stage::Fragment {
                    *out.get(&0).expect("a pixel")
                } else {
                    /* v_color's four dwords: the ones that are a colour's
                     * components (the position is the sentinel passed
                     * through), packed as the fragment's pixel */
                    let lanes: Vec<f32> =
                        out.values().map(|v| f32::from_bits(*v)).filter(|x| *x == 0.0 || *x == 1.0).collect();
                    assert_eq!(lanes.len(), 4, "{case}: v_color's four components in the outputs: {out:x?}");
                    lanes.iter().enumerate().fold(0u32, |px, (c, x)| px | ((*x as u32) * 255) << (8 * c))
                }
            };
            assert_eq!(colour(&want), WHITE, "{case} ({kind:?}, native {native}): {size:?} level {lod} is {want:?}");
            assert_eq!(
                colour(&wrong),
                BLACK,
                "{case} ({kind:?}, native {native}): {size:?} level {lod} is not {wrong:?}"
            );
            checked += 1;
        }
    }
    checked
}

/// The fragment case of every kind over the stacked textures (`pvr_native_array`
/// and `pvr_native_3d` off): dEQP's 33 base-level-0 rows, less the ones it
/// skips, each white for GL's size and black for another.
#[test]
fn every_row_of_deqps_table_lights_the_fragment_white_for_gls_size() {
    let n = check_table(Stage::Fragment, false);
    /* 33 base-level-0 rows, every one a valid 2D level: 5 kinds of 2D, 4 of
     * 3D and 5 of array see them all, the 5 cube kinds the 6 square ones
     * (1^3 and 64^3 at levels 0 to 4) */
    assert_eq!(n, 5 * 33 + 4 * 33 + 5 * 6 + 5 * 33, "rows checked");
}

/// The same over the native array and the native volume: the layer or depth
/// count from the twiddled word, one layer's size from its first pair.
#[test]
fn every_row_of_deqps_table_lights_the_fragment_white_with_the_native_arms() {
    let n = check_table(Stage::Fragment, true);
    assert_eq!(n, 4 * 33 + 5 * 33, "rows checked");
}

/// The vertex case: the same sizes through `v_color`.
#[test]
fn every_row_of_deqps_table_colours_the_vertex_white_for_gls_size() {
    let n = check_table(Stage::Vertex, false);
    assert_eq!(n, 5 * 33 + 4 * 33 + 5 * 6 + 5 * 33, "rows checked");
}

/// The sizes that went wrong before, by name, so that a regression names
/// the row: a stacked 3D texture's level above 0 (the helper answered level
/// 0's size at every level), and a stacked array's layer height once the
/// stacked image's height at the level divides to 0 (100x31x18 at levels 5
/// and 6: 17 / 18 and 8 / 18 are 0, GL says 1).
#[test]
fn the_stacked_3d_level_and_the_short_stacked_layer_read_gls_sizes() {
    let src = fragment_source("sampler3D", Kind::D3);
    let p = with_native(false, false, || compile(Stage::Fragment, &src)).unwrap();
    let desc = texture_descriptor(&bind(Kind::D3, false, [64, 64, 64], 7));
    for (lod, want) in [(1, [32, 32, 32]), (4, [4, 4, 4]), (6, [1, 1, 1]), (9, [1, 1, 1])] {
        assert_eq!(pixel(&p, &desc, 64, lod, &want), WHITE, "64^3 at level {lod} is {want:?}");
        assert_eq!(pixel(&p, &desc, 64, lod, &[64, 64, 64]), BLACK, "64^3 at level {lod} is not 64^3");
    }
    let desc = texture_descriptor(&bind(Kind::D3, false, [100, 31, 18], 7));
    for (lod, want) in [(0, [100, 31, 18]), (3, [12, 3, 2]), (5, [3, 1, 1]), (6, [1, 1, 1])] {
        assert_eq!(pixel(&p, &desc, 18, lod, &want), WHITE, "100x31x18 at level {lod} is {want:?}");
    }
    let src = fragment_source("sampler2DArray", Kind::D2Array);
    let p = with_native(false, false, || compile(Stage::Fragment, &src)).unwrap();
    let desc = texture_descriptor(&bind(Kind::D2Array, false, [100, 31, 18], 7));
    for (lod, want) in [(4, [6, 1, 18]), (5, [3, 1, 18]), (6, [1, 1, 18])] {
        assert_eq!(pixel(&p, &desc, 18, lod, &want), WHITE, "100x31x18 layers at level {lod} are {want:?}");
        assert_eq!(pixel(&p, &desc, 18, lod, &[want[0], 0, 18]), BLACK, "never a height of 0");
    }
}

/// A base level the descriptor carries (the twiddled IMAGE_WORD1's
/// `baselevel`, bits 60..63: dword 3 bits 28..31) is added to the level, as
/// pco's usclib_tex_state_size adds it, so `textureSize(s, lod)` is level
/// `base + lod`'s size -- GL's answer, whose levels count from the base
/// level. A STRIDE image (textype 4) has no such field (the bits are its
/// level count) and adds nothing: its base level is the GL core's to apply
/// by binding the base level's image, which the draw does not do yet
/// (dEQP's three base-level rows).
#[test]
fn a_base_level_in_a_twiddled_word_moves_the_level_and_a_stride_word_has_none() {
    let src = fragment_source("sampler2DArray", Kind::D2Array);
    let p = with_native(true, false, || compile(Stage::Fragment, &src)).unwrap();
    let mut desc = texture_descriptor(&bind(Kind::D2Array, true, [128, 64, 32], 4));
    assert_eq!(desc[0] & 7, 1, "a native array is a TEXTYPE_2D twiddled image");
    assert_eq!(desc[3] >> 28, 0, "the backend writes base level 0");
    assert_eq!(pixel(&p, &desc, 32, 3, &[16, 8, 32]), WHITE, "base 0: level 3 of 128x64");
    desc[3] |= 1 << 28;
    assert_eq!(pixel(&p, &desc, 32, 3, &[8, 4, 32]), WHITE, "base 1: level 3 is level 4's size");
    assert_eq!(pixel(&p, &desc, 32, 0, &[64, 32, 32]), WHITE, "base 1: level 0 is level 1's size");
    assert_eq!(pixel(&p, &desc, 32, 3, &[16, 8, 32]), BLACK, "base 1: not level 3's size");

    let src = fragment_source("sampler2D", Kind::D2);
    let p = with_native(false, false, || compile(Stage::Fragment, &src)).unwrap();
    /* a mipmap filter, so that the word carries the chain's level count (a
     * NEAREST sampler's word says one level) */
    let desc = texture_descriptor(&GlesTextureBind { mip_filter: 1, ..bind(Kind::D2, false, [128, 64, 1], 4) });
    assert_eq!(desc[0] & 7, 4, "a linear 2D texture is a STRIDE image");
    assert_eq!(desc[3] >> 28, 4, "whose bits 60..63 are its level count");
    assert_eq!(pixel(&p, &desc, 1, 3, &[16, 8]), WHITE, "a STRIDE word adds no base level");
    assert_eq!(pixel(&p, &desc, 1, 3, &[1, 1]), BLACK, "not level 7's size");
}
