// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `textureSize` on the PowerVR's online compiler (dEQP-GLES3
//! shaders.texture_functions.texturesize, 38 cases): every sampler kind --
//! 2D, cube, 2D array and 3D, float, integer and shadow -- in the vertex and
//! the fragment stage, with the TPU's own cube, array and volume on and
//! off, run in the byte model over the descriptor words
//! `pvr::backend::texture_descriptor` writes for the bind the draw makes for
//! that program (the stacked image, or the twiddled copy the record asks
//! for): at levels 0 to 7 of a 100x31x18 texture, and at each row of dEQP's
//! own table (`TextureSizeCase::iterate`, es3fShaderTextureFunctionTests.cpp:
//! six sizes from 1x2x1 to 128x64x32 at the levels it checks). The expected
//! size is GLSL ES 3.00 8.8's: each dimension halved `lod` times and at
//! least 1, the layer count of an array unshifted. A program that reads a
//! copy's word says so in the record (`native_cube` bits 0, 1, 2), so the
//! draw binds that copy; a stacked one says nothing. No program samples.
//!
//! Not here: dEQP's three "with lod base" rows set `GL_TEXTURE_BASE_LEVEL`
//! 1 and expect level `lod + 1`'s size. The compiler adds the base level
//! the image word carries (`tex_size_2d`), but no PowerVR bind writes one
//! (`src/gl/draw.rs` `mip_levels_needed`: the draw does not apply the base
//! level), so those rows fail on a board until the bind does.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_texsize
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use yakogl::backend::{
    GlesTextureBind, GLES_TEXFMT_RGBA8, GLES_TEXTYPE_2D, GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_2D_ARRAY_TWIDDLED,
    GLES_TEXTYPE_3D, GLES_TEXTYPE_3D_TWIDDLED, GLES_TEXTYPE_CUBE, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
    GLES_WRAP_CLAMP_TO_EDGE,
};
use yakogl::shaderc::online::powervr::chain::{compile_fragment, compile_vertex};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::SHC_UNIFORM_NO_BLOCK;

mod usc_exec;

/// The compiler keeps its output in one arena that every compile resets,
/// and the switches are process-wide: one compile at a time.
static COMPILING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The three switches set for the closure, and back at their defaults
/// after it, panic or not.
fn with_native<T>(cube: bool, array: bool, vol: bool, f: impl FnOnce() -> T) -> T {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
            yakogl::shaderc::pvr_set_native_array(yakogl::shaderc::PVR_NATIVE_ARRAY_DEFAULT);
            yakogl::shaderc::pvr_set_native_3d(yakogl::shaderc::PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_cube(i32::from(cube));
    yakogl::shaderc::pvr_set_native_array(i32::from(array));
    yakogl::shaderc::pvr_set_native_3d(i32::from(vol));
    f()
}

/// A texture's level-0 extent: `w x h`, `n` layers or slices (a cube's
/// faces are `w x w`), and how the bind describes its chain: `levels`
/// levels, filtered through them (`mip`, dEQP's cases filter NEAREST over
/// `lod + 1` levels) or not.
#[derive(Clone, Copy)]
struct Size {
    w: u32,
    h: u32,
    n: u32,
    levels: u8,
    mip: bool,
}

/// The texture of the all-levels sweep: 100x31, 18 layers or slices,
/// seven levels (100 -> 1), a mipmap filter so the stacked binds describe
/// the chain.
const SWEEP: Size = Size { w: 100, h: 31, n: 18, levels: 7, mip: true };

/// The slice grid the GL core tiles an `n`-slice 3D texture in (texture.rs
/// grid_cols_for: the smallest c with c * c >= depth; rows ceil(n / c)),
/// which the compiler recomputes from the layer count (`vol_grid`).
fn grid(n: u32) -> (u32, u32) {
    let cols = (1..).find(|c| c * c >= n).unwrap();
    (cols, n.div_ceil(cols))
}

/// A bind of kind `type_`, the image `w x h` as the backend is told it.
fn bind(type_: u8, w: u32, h: u32, layers: u16, sz: Size) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_,
        layers,
        base_gpu: 0x80_0000_9000,
        width: w,
        height: h,
        stride_texels: w,
        bytes: w * h * 4 * u32::from(layers.max(1)),
        linear_filter: u8::from(sz.mip),
        min_linear: u8::from(sz.mip),
        mip_filter: u8::from(sz.mip),
        mip_levels: sz.levels,
        wrap_s: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_t: GLES_WRAP_CLAMP_TO_EDGE,
        wrap_r: GLES_WRAP_CLAMP_TO_EDGE,
        ..GlesTextureBind::ZERO
    }
}

/// A sampler dimension: its GLSL sampler kinds, the lanes `textureSize`
/// gives, the bind the draw makes with the switch off (the stacked image,
/// draw.rs: width times the grid's columns, height times its rows or the
/// layer count) and on (the twiddled copy, one face's, layer's or the
/// volume's extent), the record bit the copy needs, and whether the third
/// lane halves with the level (a volume's depth) or not (an array's count).
struct Dim {
    kinds: &'static [&'static str],
    lanes: usize,
    stacked: GlesTextureBind,
    native: Option<(GlesTextureBind, u8)>,
    depth_halves: bool,
}

fn dims(sz: Size) -> [Dim; 4] {
    let Size { w, h, n, .. } = sz;
    let (cols, rows) = grid(n);
    [
        Dim {
            kinds: &["sampler2D", "isampler2D", "usampler2D", "sampler2DShadow"],
            lanes: 2,
            stacked: bind(GLES_TEXTYPE_2D, w, h, 0, sz),
            native: None,
            depth_halves: false,
        },
        Dim {
            kinds: &["samplerCube", "isamplerCube", "usamplerCube", "samplerCubeShadow"],
            lanes: 2,
            /* a cube's faces are w x w, six of them one above the other */
            stacked: bind(GLES_TEXTYPE_CUBE, w, w * 6, 6, sz),
            native: Some((bind(GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, w, w, 6, sz), 1)),
            depth_halves: false,
        },
        Dim {
            kinds: &["sampler2DArray", "isampler2DArray", "usampler2DArray", "sampler2DArrayShadow"],
            lanes: 3,
            stacked: bind(GLES_TEXTYPE_2D_ARRAY, w, h * n, n as u16, sz),
            native: Some((bind(GLES_TEXTYPE_2D_ARRAY_TWIDDLED, w, h, n as u16, sz), 2)),
            depth_halves: false,
        },
        Dim {
            kinds: &["sampler3D", "isampler3D", "usampler3D"],
            lanes: 3,
            stacked: bind(GLES_TEXTYPE_3D, w * cols, h * rows, n as u16, sz),
            native: Some((bind(GLES_TEXTYPE_3D_TWIDDLED, w, h, n as u16, sz), 4)),
            depth_halves: true,
        },
    ]
}

/// GLSL ES 3.00 8.8: the size at level `lod`.
fn expected(d: &Dim, sz: Size, lod: u32) -> Vec<u32> {
    let at = |x: u32| (x >> lod).max(1);
    let face = d.kinds[0].contains("Cube");
    let mut v = vec![at(sz.w), at(if face { sz.w } else { sz.h })];
    if d.lanes == 3 {
        v.push(if d.depth_halves { at(sz.n) } else { sz.n });
    }
    v
}

/// The shader of one case: `s` of the kind, the size at the uniform level
/// `il`. A vertex shader writes the lanes to `v` as they are (read back
/// exactly as float bits); a fragment shader writes one lane `c` as its
/// low and high byte, exact through an RGBA8 pixel, or, for `None`, every
/// lane as one byte (a size below 256, as all of dEQP's are).
fn source(stage: Stage, kind: &str, lanes: usize, c: Option<char>) -> String {
    let head = "#version 300 es\nprecision highp float;\nprecision highp int;\n";
    let decl = format!("uniform highp {kind} s;\nuniform int il;\n");
    let (ivec, pad) = if lanes == 3 { ("ivec3", "vec3(t), 255.0") } else { ("ivec2", "vec2(t), 0.0, 255.0") };
    match (stage, c) {
        (Stage::Vertex, _) => {
            let pad = if lanes == 3 { "vec3(t), 0.0" } else { "vec2(t), 0.0, 0.0" };
            format!(
                "{head}{decl}in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; {ivec} t = textureSize(s, il); v = vec4({pad}); }}\n"
            )
        }
        (_, Some(c)) => format!(
            "{head}{decl}out vec4 o;\nvoid main() {{ int q = textureSize(s, il).{c}; int hi = q / 256; \
             o = vec4(float(q - hi * 256), float(hi), 0.0, 1.0) / 255.0; }}\n"
        ),
        (_, None) => {
            format!(
                "{head}{decl}out vec4 o;\nvoid main() {{ {ivec} t = textureSize(s, il); o = vec4({pad}) / 255.0; }}\n"
            )
        }
    }
}

/// Does the program sample (an `smp` anywhere)?
fn samples(image: &[u8]) -> bool {
    let mut off = 0usize;
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if ops_for(r, ph).any(|o| o.name == "smp") {
                return true;
            }
        }
        off += g.total;
    }
    false
}

/// One program through the chain and the byte model over descriptor
/// `desc` of the one sampler `s`, at level `lod`, the stacked layer count
/// `layers` in the hidden uniform when the program has it: (the record's
/// `native_cube`, whether it samples, the output dwords -- a fragment
/// program's pixel at 0, a vertex program's outputs by dword).
fn run(
    stage: Stage,
    src: &str,
    desc: &[u32],
    lod: u32,
    layers: u32,
) -> (u8, bool, std::collections::BTreeMap<u32, u32>) {
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, stage, text).unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
        let rec = if stage == Stage::Fragment {
            compile_fragment(&l.module, &l.reflection, a)
        } else {
            compile_vertex(&l.module, &l.reflection, a)
        }
        .unwrap_or_else(|e| panic!("refused: {e:?}\n{src}"));
        // SAFETY: the record's code and uniform rows live in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let samplers = rows.iter().filter(|u| u.dword_off == SHC_UNIFORM_NO_BLOCK).count();
        assert_eq!(samplers, 1, "the one sampler keeps its row: {src}");
        let mut inp = usc_exec::sentinel_inputs(1.0e30);
        /* the shared file as the driver fills it: the sampler's descriptor,
         * then the uniform block by name */
        inp.shared[..desc.len()].copy_from_slice(desc);
        let base = desc.len();
        for u in rows.iter().filter(|u| u.dword_off != SHC_UNIFORM_NO_BLOCK) {
            let value = match name_of(&u.name).as_str() {
                "il" => lod,
                "pvr_layers_s" => (layers as f32).to_bits(),
                _ => 0,
            };
            for k in 0..u.dwords as usize {
                inp.shared[base + u.dword_off as usize + k] = value;
            }
        }
        let out = usc_exec::run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("the byte model: {e}\n{src}"));
        (rec.native_cube, samples(&image), out)
    })
    .unwrap()
}

/// `textureSize` of program `kind` over `desc` at `lod` in `stage`: the
/// lanes, and the record's flag. A fragment program per lane when `wide`
/// (two bytes a lane), else one program for every lane (a byte each).
fn size_of(stage: Stage, kind: &str, lanes: usize, desc: &[u32], lod: u32, layers: u32, wide: bool) -> (Vec<u32>, u8) {
    match stage {
        Stage::Vertex => {
            let src = source(stage, kind, lanes, None);
            let (flag, smp, out) = run(stage, &src, desc, lod, layers);
            assert!(!smp, "{kind}: textureSize samples nothing");
            /* gl_Position is outputs 0..3; v follows */
            ((4..4 + lanes as u32).map(|k| f32::from_bits(*out.get(&k).expect("an output")) as u32).collect(), flag)
        }
        _ if wide => {
            let mut v = Vec::new();
            let mut flag = 0;
            for c in "xyz".chars().take(lanes) {
                let src = source(stage, kind, lanes, Some(c));
                let (f, smp, out) = run(stage, &src, desc, lod, layers);
                assert!(!smp, "{kind}: textureSize samples nothing");
                let px = *out.get(&0).expect("a pixel");
                v.push((px & 0xff) + ((px >> 8) & 0xff) * 256);
                flag = f;
            }
            (v, flag)
        }
        _ => {
            let src = source(stage, kind, lanes, None);
            let (flag, smp, out) = run(stage, &src, desc, lod, layers);
            assert!(!smp, "{kind}: textureSize samples nothing");
            let px = *out.get(&0).expect("a pixel");
            ((0..lanes as u32).map(|k| (px >> (8 * k)) & 0xff).collect(), flag)
        }
    }
}

/// Every kind of `sz` at `lod` in `stage`, the switches all on or all off:
/// the size is GLSL's and the record asks for the copy it read, or nothing.
fn check_at(stage: Stage, native: bool, sz: Size, lod: u32, wide: bool) {
    for d in dims(sz) {
        let (bind, want_flag) = match (&d.native, native) {
            (Some((b, flag)), true) => (b, *flag),
            _ => (&d.stacked, 0),
        };
        let desc = yakogl::pvr::backend::texture_descriptor(bind);
        for kind in d.kinds {
            let (got, flag) =
                with_native(native, native, native, || size_of(stage, kind, d.lanes, &desc, lod, sz.n, wide));
            assert_eq!(
                got,
                expected(&d, sz, lod),
                "{stage:?} {kind} native {native}: textureSize(s, {lod}) of {}x{}x{}",
                sz.w,
                sz.h,
                sz.n
            );
            assert_eq!(flag, want_flag, "{stage:?} {kind} native {native}: the record's copy bits");
        }
    }
}

fn check(stage: Stage, native: bool) {
    for lod in 0..8u32 {
        check_at(stage, native, SWEEP, lod, true);
    }
}

/// With every switch off: the stacked image's word, the whole stack's
/// height divided by the face, layer or row count (at least 1: a 100-high
/// face at level 7 is 4 stacked rows, not 0), a volume's slice grid, each
/// dimension halved by the level (the slice count too).
#[test]
fn the_fragment_stage_sizes_every_stacked_kind_at_every_level() {
    check(Stage::Fragment, false);
}

#[test]
fn the_vertex_stage_sizes_every_stacked_kind_at_every_level() {
    check(Stage::Vertex, false);
}

/// With the TPU's own cube, array and volume on: the copy's word (one
/// face's or layer's size, the depth field), and the record asks for the
/// copy (bits 0, 1, 2) although nothing samples; a 2D sampler is unchanged.
#[test]
fn the_fragment_stage_sizes_every_native_kind_at_every_level() {
    check(Stage::Fragment, true);
}

#[test]
fn the_vertex_stage_sizes_every_native_kind_at_every_level() {
    check(Stage::Vertex, true);
}

/// dEQP's table (`TextureSizeCase::iterate`), the rows with base level 0:
/// (width, height, depth or layers, lod, the expected size). The expected
/// column is dEQP's, so the rule the sweep tests use is shown to be its;
/// a cube's rows are the square sizes only, a 2D or array row whose level
/// has no texel in either dimension is skipped, as dEQP skips them.
const DEQP_TABLE: [([u32; 3], u32, [u32; 3]); 33] = [
    ([1, 2, 1], 1, [1, 1, 1]),
    ([1, 2, 1], 0, [1, 2, 1]),
    ([1, 3, 2], 0, [1, 3, 2]),
    ([1, 3, 2], 1, [1, 1, 1]),
    ([100, 31, 18], 0, [100, 31, 18]),
    ([100, 31, 18], 1, [50, 15, 9]),
    ([100, 31, 18], 2, [25, 7, 4]),
    ([100, 31, 18], 3, [12, 3, 2]),
    ([100, 31, 18], 4, [6, 1, 1]),
    ([100, 31, 18], 5, [3, 1, 1]),
    ([100, 31, 18], 6, [1, 1, 1]),
    ([100, 128, 32], 0, [100, 128, 32]),
    ([100, 128, 32], 1, [50, 64, 16]),
    ([100, 128, 32], 2, [25, 32, 8]),
    ([100, 128, 32], 3, [12, 16, 4]),
    ([100, 128, 32], 4, [6, 8, 2]),
    ([100, 128, 32], 5, [3, 4, 1]),
    ([100, 128, 32], 6, [1, 2, 1]),
    ([100, 128, 32], 7, [1, 1, 1]),
    ([128, 64, 32], 0, [128, 64, 32]),
    ([128, 64, 32], 1, [64, 32, 16]),
    ([128, 64, 32], 2, [32, 16, 8]),
    ([128, 64, 32], 3, [16, 8, 4]),
    ([128, 64, 32], 4, [8, 4, 2]),
    ([128, 64, 32], 5, [4, 2, 1]),
    ([128, 64, 32], 6, [2, 1, 1]),
    ([128, 64, 32], 7, [1, 1, 1]),
    ([1, 1, 1], 0, [1, 1, 1]),
    ([64, 64, 64], 0, [64, 64, 64]),
    ([64, 64, 64], 1, [32, 32, 32]),
    ([64, 64, 64], 2, [16, 16, 16]),
    ([64, 64, 64], 3, [8, 8, 8]),
    ([64, 64, 64], 4, [4, 4, 4]),
];

/// Each row of dEQP's table, as dEQP binds it (NEAREST, `lod + 1` levels),
/// in both stages with the switches off and on: the answer is dEQP's
/// expected size (an array's third lane its layer count), through one
/// fragment program per kind (every lane a byte: no dEQP size reaches 256).
fn deqp_rows(stage: Stage, native: bool) {
    for ([w, h, n], lod, want) in DEQP_TABLE {
        let sz = Size { w, h, n, levels: (lod + 1) as u8, mip: false };
        let at = |x: u32| (x >> lod).max(1);
        assert_eq!([at(w), at(h), at(n)], want, "the rule is dEQP's expected column for {w}x{h}x{n} at {lod}");
        let square = w == h;
        let valid_2d = (w >> lod) != 0 || (h >> lod) != 0;
        for d in dims(sz) {
            let cube = d.kinds[0].contains("Cube");
            if (cube && !square) || (!cube && !d.depth_halves && !valid_2d) {
                continue;
            }
            let (bind, want_flag) = match (&d.native, native) {
                (Some((b, flag)), true) => (b, *flag),
                _ => (&d.stacked, 0),
            };
            let desc = yakogl::pvr::backend::texture_descriptor(bind);
            for kind in d.kinds {
                let (got, flag) =
                    with_native(native, native, native, || size_of(stage, kind, d.lanes, &desc, lod, n, false));
                let mut exp = vec![want[0], if cube { want[0] } else { want[1] }];
                if d.lanes == 3 {
                    exp.push(if d.depth_halves { want[2] } else { n });
                }
                assert_eq!(got, exp, "{stage:?} {kind} native {native}: textureSize(s, {lod}) of {w}x{h}x{n}");
                assert_eq!(flag, want_flag, "{stage:?} {kind} native {native}: the record's copy bits");
            }
        }
    }
}

#[test]
fn deqps_table_in_the_fragment_stage_stacked() {
    deqp_rows(Stage::Fragment, false);
}

#[test]
fn deqps_table_in_the_fragment_stage_native() {
    deqp_rows(Stage::Fragment, true);
}

#[test]
fn deqps_table_in_the_vertex_stage_stacked() {
    deqp_rows(Stage::Vertex, false);
}

#[test]
fn deqps_table_in_the_vertex_stage_native() {
    deqp_rows(Stage::Vertex, true);
}

/// A native cube that is also sampled: one program, the sample's `smp` and
/// the size from the same TEXTYPE_CUBE word (one face's extent, not a
/// sixth of it): the record asks for the copy once (bit 0).
#[test]
fn a_sampled_native_cube_sizes_one_face() {
    let src = "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp samplerCube s;\n\
               uniform int il;\nuniform vec3 c;\nout vec4 o;\nvoid main() { int q = textureSize(s, il).y; int hi = q / 256; \
               o = vec4(float(q - hi * 256), float(hi), 0.0, 1.0) / 255.0 + 0.0 * texture(s, c); }\n";
    let desc =
        yakogl::pvr::backend::texture_descriptor(&bind(GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, SWEEP.w, SWEEP.w, 6, SWEEP));
    for lod in [0u32, 3] {
        let (flag, smp, out) = with_native(true, false, false, || run(Stage::Fragment, src, &desc, lod, 6));
        assert!(smp, "the cube is sampled");
        assert_eq!(flag, 1, "the copy, once");
        let px = *out.get(&0).expect("a pixel");
        assert_eq!((px & 0xff) + ((px >> 8) & 0xff) * 256, (SWEEP.w >> lod).max(1), "the face's height at level {lod}");
    }
}
