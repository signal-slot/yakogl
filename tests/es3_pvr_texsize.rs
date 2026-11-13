// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `textureSize` on the PowerVR's online compiler (dEQP-GLES3
//! shaders.texture_functions.texturesize, 38 cases): every sampler kind --
//! 2D, cube, 2D array and 3D, float, integer and shadow -- in the vertex and
//! the fragment stage, with the TPU's own cube, array and volume on and
//! off, run in the byte model over the descriptor words
//! `pvr::backend::texture_descriptor` writes for the bind the draw makes for
//! that program (the stacked image, or the twiddled copy the record asks
//! for), at levels 0 to 7 of a 100x31x18 texture. The expected size is GLSL
//! ES 3.00 8.8's: each dimension halved `lod` times and at least 1, the
//! layer count of an array unshifted. A program that reads a copy's word
//! says so in the record (`native_cube` bits 0, 1, 2), so the draw binds
//! that copy; a stacked one says nothing. No program samples.
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

/// The texture of every case: 100x31, 18 layers or slices, seven levels
/// (100 -> 1), a mipmap filter so the stacked binds describe the chain.
const W: u32 = 100;
const H: u32 = 31;
const N: u32 = 18;
const LEVELS: u8 = 7;

/// The slice grid the GL core tiles an 18-slice 3D texture in
/// (texture.rs grid_cols_for: the smallest c with c * c >= depth, 5; rows
/// ceil(18 / 5) = 4), which the compiler recomputes from the layer count.
const GRID: (u32, u32) = (5, 4);

/// A bind of kind `type_`, the image `w x h` as the backend is told it.
fn bind(type_: u8, w: u32, h: u32, layers: u16) -> GlesTextureBind {
    GlesTextureBind {
        format: GLES_TEXFMT_RGBA8,
        type_,
        layers,
        base_gpu: 0x80_0000_9000,
        width: w,
        height: h,
        stride_texels: w,
        bytes: w * h * 4 * u32::from(layers.max(1)),
        linear_filter: 1,
        min_linear: 1,
        mip_filter: 1,
        mip_levels: LEVELS,
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

fn dims() -> [Dim; 4] {
    [
        Dim {
            kinds: &["sampler2D", "isampler2D", "usampler2D", "sampler2DShadow"],
            lanes: 2,
            stacked: bind(GLES_TEXTYPE_2D, W, H, 0),
            native: None,
            depth_halves: false,
        },
        Dim {
            kinds: &["samplerCube", "isamplerCube", "usamplerCube", "samplerCubeShadow"],
            lanes: 2,
            /* a cube's faces are W x W, six of them one above the other */
            stacked: bind(GLES_TEXTYPE_CUBE, W, W * 6, 6),
            native: Some((bind(GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, W, W, 6), 1)),
            depth_halves: false,
        },
        Dim {
            kinds: &["sampler2DArray", "isampler2DArray", "usampler2DArray", "sampler2DArrayShadow"],
            lanes: 3,
            stacked: bind(GLES_TEXTYPE_2D_ARRAY, W, H * N, N as u16),
            native: Some((bind(GLES_TEXTYPE_2D_ARRAY_TWIDDLED, W, H, N as u16), 2)),
            depth_halves: false,
        },
        Dim {
            kinds: &["sampler3D", "isampler3D", "usampler3D"],
            lanes: 3,
            stacked: bind(GLES_TEXTYPE_3D, W * GRID.0, H * GRID.1, N as u16),
            native: Some((bind(GLES_TEXTYPE_3D_TWIDDLED, W, H, N as u16), 4)),
            depth_halves: true,
        },
    ]
}

/// GLSL ES 3.00 8.8: the size at level `lod`.
fn expected(d: &Dim, lod: u32) -> Vec<u32> {
    let at = |x: u32| (x >> lod).max(1);
    let face = d.kinds[0].contains("Cube");
    let mut v = vec![at(W), at(if face { W } else { H })];
    if d.lanes == 3 {
        v.push(if d.depth_halves { at(N) } else { N });
    }
    v
}

/// The shader of one case: `s` of the kind, the size at the uniform level
/// `il`. A vertex shader writes the lanes to `v` as they are (read back
/// exactly as float bits); a fragment shader writes one lane `c` as its
/// low and high byte, exact through an RGBA8 pixel.
fn source(stage: Stage, kind: &str, lanes: usize, c: Option<char>) -> String {
    let head = "#version 300 es\nprecision highp float;\nprecision highp int;\n";
    let decl = format!("uniform highp {kind} s;\nuniform int il;\n");
    match stage {
        Stage::Vertex => {
            let (ivec, pad) = if lanes == 3 { ("ivec3", "vec3(t), 0.0") } else { ("ivec2", "vec2(t), 0.0, 0.0") };
            format!(
                "{head}{decl}in vec4 a;\nout vec4 v;\nvoid main() {{ gl_Position = a; {ivec} t = textureSize(s, il); v = vec4({pad}); }}\n"
            )
        }
        _ => {
            let c = c.expect("the lane");
            format!(
                "{head}{decl}out vec4 o;\nvoid main() {{ int q = textureSize(s, il).{c}; int hi = q / 256; \
                 o = vec4(float(q - hi * 256), float(hi), 0.0, 1.0) / 255.0; }}\n"
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
/// lanes, and the record's flag.
fn size_of(stage: Stage, kind: &str, lanes: usize, desc: &[u32], lod: u32, layers: u32) -> (Vec<u32>, u8) {
    match stage {
        Stage::Vertex => {
            let src = source(stage, kind, lanes, None);
            let (flag, smp, out) = run(stage, &src, desc, lod, layers);
            assert!(!smp, "{kind}: textureSize samples nothing");
            /* gl_Position is outputs 0..3; v follows */
            ((4..4 + lanes as u32).map(|k| f32::from_bits(*out.get(&k).expect("an output")) as u32).collect(), flag)
        }
        _ => {
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
    }
}

fn check(stage: Stage, native: bool) {
    for d in dims() {
        let (bind, want_flag) = match (&d.native, native) {
            (Some((b, flag)), true) => (b, *flag),
            _ => (&d.stacked, 0),
        };
        let desc = yakogl::pvr::backend::texture_descriptor(bind);
        for kind in d.kinds {
            for lod in 0..8u32 {
                let (got, flag) = with_native(native, native, native, || size_of(stage, kind, d.lanes, &desc, lod, N));
                assert_eq!(got, expected(&d, lod), "{stage:?} {kind} native {native}: textureSize(s, {lod})");
                assert_eq!(flag, want_flag, "{stage:?} {kind} native {native}: the record's copy bits");
            }
        }
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

/// A native cube that is also sampled: one program, the sample's `smp` and
/// the size from the same TEXTYPE_CUBE word (one face's extent, not a
/// sixth of it): the record asks for the copy once (bit 0).
#[test]
fn a_sampled_native_cube_sizes_one_face() {
    let src = "#version 300 es\nprecision highp float;\nprecision highp int;\nuniform highp samplerCube s;\n\
               uniform int il;\nuniform vec3 c;\nout vec4 o;\nvoid main() { int q = textureSize(s, il).y; int hi = q / 256; \
               o = vec4(float(q - hi * 256), float(hi), 0.0, 1.0) / 255.0 + 0.0 * texture(s, c); }\n";
    let desc = yakogl::pvr::backend::texture_descriptor(&bind(GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, W, W, 6));
    for lod in [0u32, 3] {
        let (flag, smp, out) = with_native(true, false, false, || run(Stage::Fragment, src, &desc, lod, 6));
        assert!(smp, "the cube is sampled");
        assert_eq!(flag, 1, "the copy, once");
        let px = *out.get(&0).expect("a pixel");
        assert_eq!((px & 0xff) + ((px >> 8) & 0xff) * 256, (W >> lod).max(1), "the face's height at level {lod}");
    }
}
