// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's fixed-function blending against the board: the
//! draws Mesa 25.0.7 made on the NanoPC-T6 for ten blend modes into a 64x64
//! RGBA8 FBO (tools/mali/board-dump/blend-2026-10-19.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-19-blend/mesa).
//!
//! What is compared, per mode: the draw's blend descriptor byte for byte
//! (load destination, the constant, the equation word, the mode and the
//! conversion) and DCD0 (r57; forward pixel kill is off while the blend
//! reads the destination).
//!
//! Blend shaders: case 8's constant colour
//! (tools/mali/board-dump/blendconst-2026-10-20.txt) and case 190's GL_MIN
//! and ONE_MINUS_DST_ALPHA columns (tools/mali/board-dump/
//! blendshader-2026-10-24.txt), the descriptors byte for byte; the words the
//! compiler builds for case 211 and 190 against
//! tools/mali/testdata/blend_case*.words.
//!
//! RGBA16F targets (case 91 hdrblend): the blend unit on Mesa 26.2.1's
//! float rules, against the drm-shim's hdrcolor draw
//! (tools/mali/testdata/fsout_91_hdrcolor.trace and .expect.json, an RGBA8
//! target) with every differing byte named (docs/notes/mali-blend-shaders.md
//! section 8).

use super::arena::Arena;
use super::blend::{Base, Blending, Channel, Factor, Func};
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const BLEND: &str = include_str!("../../tools/mali/board-dump/blend-2026-10-19.txt");
const BLENDCONST: &str = include_str!("../../tools/mali/board-dump/blendconst-2026-10-20.txt");
const DESC: u64 = 0x7fff_fffd_0000;

/// The registers the one RUN_IDVS of a stream reads, and its flags
/// override.
fn regs_at_idvs(words: &[u64]) -> (BTreeMap<u32, u32>, u32) {
    let mut r = BTreeMap::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(i)) => return (r, i.flags_override),
            _ => {}
        }
    }
    panic!("no RUN_IDVS in the stream")
}

fn meta(stage: ShaderStage) -> ShaderMeta<'static> {
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: 0,
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs: u32::from(stage == ShaderStage::FRAGMENT),
        samples_texture: false,
        requires_helper_threads: false,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// blend.c's frame: cleared, the quad over the left half with `blend`;
/// the descriptor arena's bytes and the RUN_IDVS registers.
fn build(blend: Option<Blending>, blend_color: [f32; 4]) -> Result<(Vec<u8>, BTreeMap<u32, u32>), FrameError> {
    build_with(blend, blend_color, None)
}

/// [`build`] with a blend shader at `blend_shader`.
fn build_with(
    blend: Option<Blending>,
    blend_color: [f32; 4],
    blend_shader: Option<u64>,
) -> Result<(Vec<u8>, BTreeMap<u32, u32>), FrameError> {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let n;
    {
        let mut ctx_a = Arena::new("context", 0x7fff_ffff_b000, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: 0x7fff_fffe_b000,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: 0x7fff_ffff_f000,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 48 },
            offset: 0,
            stride: 8,
            components: 2,
            instance_divisor: 0,
        }];
        let input = FrameInput {
            zs: None,
            target: Target {
                gpu: 0x7fff_fffe_4000,
                width: 64,
                height: 64,
                row_stride: 256,
                format: TargetFormat::Rgba8888,
                samples: 1,
            },
            clear_rgba: Some([0.25, 0.5, 0.75, 0.5]),
            draw: Some(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                vs: &vs,
                fs: &fs,
                primitive: Topology::Triangles,
                first_vertex: 0,
                vertex_count: 6,
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
                scissor: None,
                line_width: 1.0,
                flip_y: false,
                blend_color,
                blend,
                polygon_offset: None,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
                textures: &[],
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader,
            }),
        };
        let mut cs = Stream::new(&mut words, 92);
        build_frame(&mut desc, &mut cs, &ctx, &input)?;
        n = cs.words().len();
    }
    let (regs, _) = regs_at_idvs(&words[..n]);
    Ok((desc_m, regs))
}

const fn f(base: Base, invert: bool) -> Factor {
    Factor { base, invert }
}

/// blend.c's modes: their equations as the GL core hands them, and the
/// blend colour.
fn mode(name: &str) -> (Blending, [f32; 4]) {
    use Base::*;
    let ch = |func, src, dst| Channel { func, src, dst };
    let same = |c: Channel| Blending { rgb: c, alpha: c };
    let one = Factor::ONE;
    let zero = Factor::ZERO;
    let none = [0.0; 4];
    match name {
        "over" => (same(ch(Func::Add, Factor::SRC_ALPHA, Factor::INV_SRC_ALPHA)), none),
        "add" => (same(ch(Func::Add, one, one)), none),
        "premul" => (same(ch(Func::Add, one, Factor::INV_SRC_ALPHA)), none),
        "replace" => (same(ch(Func::Add, one, zero)), none),
        "sub" => (same(ch(Func::Subtract, one, one)), none),
        "revsub" => (same(ch(Func::ReverseSubtract, one, one)), none),
        "mul" => (same(ch(Func::Add, f(DstColor, false), zero)), none),
        "twosd" => (same(ch(Func::Add, f(DstColor, false), f(SrcColor, false))), none),
        "const" => (same(ch(Func::Add, f(ConstColor, false), f(ConstColor, true))), [0.5; 4]),
        "sep" => (
            Blending { rgb: ch(Func::Add, Factor::SRC_ALPHA, Factor::INV_SRC_ALPHA), alpha: ch(Func::Add, one, zero) },
            none,
        ),
        _ => unreachable!("not a blend.c mode"),
    }
}

/// The board's blend descriptor of a later draw: on 16 bytes, its count in
/// the low 4 bits (`0x7FFFFFFD08B1`).
fn d50_16(r: &BTreeMap<u32, u32>) -> u64 {
    (u64::from(r[&50]) | u64::from(r[&51]) << 32) & !15
}

const MODES: [&str; 10] = ["over", "add", "premul", "replace", "sub", "revsub", "mul", "twosd", "const", "sep"];

fn d50(r: &BTreeMap<u32, u32>) -> u64 {
    (u64::from(r[&50]) | u64::from(r[&51]) << 32) & !63
}

/// Each mode's blend descriptor and DCD0 are the board's.
#[test]
fn each_blend_mode_is_the_board_s() {
    for name in MODES {
        let source = format!("mesa-{name}");
        let words: Vec<u64> = trace::cs_words(BLEND, &source).iter().map(|w| w.word).collect();
        let (theirs, _) = regs_at_idvs(&words);
        let m = trace::memory_of(BLEND, &source);
        let (blending, colour) = mode(name);
        let (desc, ours) = build(Some(blending), colour).expect("blend.c's frame builds");
        let at = (d50(&ours) - DESC) as usize;
        assert_eq!(
            desc[at..at + Blend::LENGTH],
            trace::bytes(&m, d50(&theirs), Blend::LENGTH)[..],
            "{name}: the blend descriptor"
        );
        assert_eq!(ours[&57], theirs[&57], "{name}: DCD0");
    }
    /* the dump holds what the test reads: "over" loads the destination
     * (byte 0 bit 0) with forward pixel kill off (r57 0x21AA), "replace" is
     * opaque (0x21AB); the constant 0.5 is 0x7F00 (the board's packing) */
    let row = |name: &str| {
        let (theirs, _) =
            regs_at_idvs(&trace::cs_words(BLEND, &format!("mesa-{name}")).iter().map(|w| w.word).collect::<Vec<_>>());
        (trace::bytes(&trace::memory_of(BLEND, &format!("mesa-{name}")), d50(&theirs), 4), theirs[&57])
    };
    assert_eq!(row("over"), (vec![1, 2, 0, 0], 0x21aa));
    assert_eq!(row("replace"), (vec![0, 2, 0, 0], 0x21ab));
    assert_eq!(row("const").0, vec![1, 2, 0, 0x7f]);
}

/// What Mesa runs in a blend shader is refused before any word: min, and
/// two different constants read.
#[test]
fn an_equation_mesa_runs_in_a_blend_shader_is_refused_by_name() {
    let min = Blending {
        rgb: Channel { func: Func::Min, src: Factor::ONE, dst: Factor::ONE },
        alpha: Channel { func: Func::Add, src: Factor::ONE, dst: Factor::ZERO },
    };
    let (c, _) = mode("const");
    for (b, colour) in [(min, [0.0; 4]), (c, [0.5, 0.25, 0.5, 0.5])] {
        assert_eq!(build(Some(b), colour).err(), Some(FrameError::NotBuilt(NOT_BUILT_BLEND_SHADER)));
    }
    assert!(build(Some(c), [0.5, 0.5, 0.5, 0.25]).is_ok(), "alpha is not read: one constant");
}

/// gl_test case 8's fifth frame (glBlendColor 0.25, 0.5, 0.75, 1.0 with
/// CONSTANT_COLOR, ONE_MINUS_CONSTANT_COLOR) against the board's Mesa
/// (tools/mali/board-dump/blendconst-2026-10-20.txt): its blend descriptor
/// to the byte, mode Shader with the blend shader's address in word 3 and
/// neither an equation nor a constant, and DCD0 the fixed-function blended
/// draw's. The board's blend shader is at `0x7FFFFFFC7000`, in the
/// fragment program's 4 GiB window (`0x7FFFFFFFC100`); the frame here puts
/// it there too. Its bytes are 25.0.7's (19 words); ours are 26.2.1's
/// (tests.rs `the_blend_shaders_are_mesa_s`).
#[test]
fn a_constant_colour_blend_runs_in_its_blend_shader_as_on_the_board() {
    const PC: u64 = 0x7fff_fffc_7000;
    let (c, _) = mode("const");
    let colour = [0.25, 0.5, 0.75, 1.0];
    let (desc, ours) = build_with(Some(c), colour, Some(PC)).expect("the blended draw builds");
    let words = |src: &str| trace::cs_words(BLENDCONST, src).iter().map(|w| w.word).collect::<Vec<_>>();
    let (f5, _) = regs_at_idvs(&words("mesa-blend-f5"));
    let (f1, _) = regs_at_idvs(&words("mesa-blend-f1"));
    /* the board's descriptor is on 16 bytes, its count in the low 4 bits
     * (`0x7FFFFFFD07B1`) */
    let f5_blend = (u64::from(f5[&50]) | u64::from(f5[&51]) << 32) & !15;
    let theirs = trace::bytes(&trace::memory_of(BLENDCONST, "mesa-blend-f5"), f5_blend, Blend::LENGTH);
    let at = (d50(&ours) - DESC) as usize;
    assert_eq!(desc[at..at + Blend::LENGTH], theirs[..], "the blend descriptor");
    /* word 0: load destination (bit 0) and enable (bit 9), no constant;
     * word 1 (the equation) and word 2 (mode Shader, 0) zero; word 3 the
     * address's low 32 bits */
    assert_eq!(theirs, [1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x70, 0xfc, 0xff]);
    /* the program the descriptor names is in the dump, at its address */
    let code = trace::bytes(&trace::memory_of(BLENDCONST, "mesa-blend-f5"), PC, 8);
    assert_eq!(code, [0xc0, 0x00, 0xd0, 0x0e, 0x00, 0xc2, 0x10, 0x01], "25.0.7's first word");
    /* DCD0: the board's blended draws write one value in frames 1 and 5
     * (a pbuffer, `0x121AA`); ours is the fixed-function blended draw's of
     * this FBO frame (`0x21AA`, blend.c's) */
    assert_eq!((f5[&57], f1[&57]), (0x121aa, 0x121aa));
    let (_, over) = build(Some(mode("over").0), [0.0; 4]).expect("over builds");
    assert_eq!(ours[&57], over[&57]);
    assert_eq!(ours[&57], 0x21aa);
}

/// The words of a tools/mali/testdata `*.words` file: one 64-bit word a
/// line, in hex.
fn testdata_words(text: &str) -> Vec<u64> {
    text.lines().map(|l| u64::from_str_radix(l.trim(), 16).expect("a hex word")).collect()
}

/// The blend shaders the frame compiles for case 211's constant colour at
/// 4x and case 190's columns 3 and 4 at 1x (`shader_equation`, then
/// src/shaderc/online/mali/blend.rs) are the drm-shim's (Mesa 26.2.1), word
/// for word, each under the key it was recorded with
/// (docs/notes/mali-blend-shaders.md section 5).
#[test]
fn the_reference_blend_shaders_are_the_drm_shim_s_words() {
    use super::blend::shader_equation;
    use crate::shaderc::online::{mali::blend::shader, store};
    let (c, _) = mode("const");
    let rgba8 = TargetFormat::Rgba8888;
    let files = [
        (c, 4, include_str!("../../tools/mali/testdata/blend_case211.words")),
        (case190(3), 1, include_str!("../../tools/mali/testdata/blend_case190_col3.words")),
        (case190(4), 1, include_str!("../../tools/mali/testdata/blend_case190_col4.words")),
    ];
    for (b, samples, text) in files {
        let eq = shader_equation(&b, 0xf, samples, rgba8).expect("a built key");
        let mut words = store::with_scratch(|sc| {
            let mut out = [0u64; 64];
            shader(&mut out, sc, eq, 0xed000).map(|s| s.code.to_vec()).expect("the blend shader compiles")
        })
        .expect("the scratch arena is free");
        while words.last() == Some(&0) {
            words.pop();
        }
        assert_eq!(words, testdata_words(text), "{b:?} at {samples}x");
        /* the 1x program ends in its BRANCHZI.end, a 4x one too */
        assert_eq!(words.last(), Some(&0x782f_c100_0000_7070));
    }
    assert_eq!(testdata_words(files[0].2).len(), 22);
    assert_eq!(testdata_words(files[1].2).len(), 16);
    assert_eq!(testdata_words(files[2].2).len(), 17);
    /* the key: the sample count, the mask and the format are in it; a MIN
     * channel's factors are not read (st/mesa's ONE, ONE match too) */
    let one = shader_equation(&c, 0xf, 1, rgba8).expect("1x: case 8's");
    assert_ne!(Some(one), shader_equation(&c, 0xf, 4, rgba8), "the sample count");
    assert!(shader_equation(&case190(3), 0xf, 4, rgba8).is_none(), "column 3 at 4x");
    assert!(shader_equation(&case190(3), 0x7, 1, rgba8).is_none(), "column 3 masked");
    assert!(shader_equation(&case190(4), 0xf, 1, TargetFormat::Rgb565).is_none(), "column 4 into RGB565");
    let mut min_one = case190(3);
    (min_one.rgb.src, min_one.rgb.dst) = (Factor::ONE, Factor::ONE);
    assert_eq!(shader_equation(&min_one, 0xf, 1, rgba8), shader_equation(&case190(3), 0xf, 1, rgba8));
    let mut max = case190(3);
    max.rgb.func = Func::Max;
    assert!(shader_equation(&max, 0xf, 1, rgba8).is_none(), "MAX has no reference");
}

const BLENDSHADER: &str = include_str!("../../tools/mali/board-dump/blendshader-2026-10-24.txt");

/// gl_test case 190's columns as the GL core hands them
/// (driver/tools/pvrlab/gl_test.c `case_blendonce`): 1 and 2 SRC_ALPHA,
/// ONE_MINUS_SRC_ALPHA (2 red-masked); 3 GL_MIN on the colour with those
/// factors, ADD on alpha; 4 ONE_MINUS_DST_ALPHA, ONE_MINUS_SRC_ALPHA on the
/// colour, ONE, ZERO on alpha.
fn case190(column: u32) -> Blending {
    let over = Channel { func: Func::Add, src: Factor::SRC_ALPHA, dst: Factor::INV_SRC_ALPHA };
    match column {
        1 | 2 => Blending { rgb: over, alpha: over },
        3 => Blending { rgb: Channel { func: Func::Min, ..over }, alpha: over },
        4 => Blending {
            rgb: Channel { func: Func::Add, src: f(Base::DstAlpha, true), dst: Factor::INV_SRC_ALPHA },
            alpha: Channel { func: Func::Add, src: Factor::ONE, dst: Factor::ZERO },
        },
        _ => unreachable!("case 190 has four columns"),
    }
}

/// Case 190's columns 3 and 4 against the board's Mesa
/// (tools/mali/board-dump/blendshader-2026-10-24.txt, the third and fourth
/// draws of 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs4 `mesa-blendonce`):
/// each blend descriptor byte for byte with the frame given Mesa's shader
/// address, mode Shader, the destination loaded, neither an equation (the
/// trace's `Color Mask: 0`) nor a constant; DCD0 the blended draw's. Columns
/// 1 and 2 are fixed function in both (the second through its colour mask,
/// [`each_blend_mode_is_the_board_s`] builds those equations). The words
/// the board's Mesa 25.0.7 ran at those addresses differ from the drm-shim's
/// 26.2.1 the frame uploads (register naming and order; both 16 and 17
/// words): [`the_reference_blend_shaders_are_the_drm_shim_s_words`].
#[test]
fn case_190_s_blend_shader_columns_are_the_board_s() {
    for (column, source, pc) in
        [(3, "mesa-blendonce-c3", 0x7fff_ffec_7000u64), (4, "mesa-blendonce-c4", 0x7fff_ffec_6000)]
    {
        let words: Vec<u64> = trace::cs_words(BLENDSHADER, source).iter().map(|w| w.word).collect();
        let (theirs, _) = regs_at_idvs(&words);
        let m = trace::memory_of(BLENDSHADER, source);
        let their_blend = trace::bytes(&m, d50_16(&theirs), Blend::LENGTH);
        let (desc, ours) = build_with(Some(case190(column)), [0.0; 4], Some(pc)).expect("the column builds");
        let at = (d50(&ours) - DESC) as usize;
        assert_eq!(desc[at..at + Blend::LENGTH], their_blend[..], "column {column}: the blend descriptor");
        let mut word3 = [0u8; 4];
        word3.copy_from_slice(&their_blend[12..16]);
        assert_eq!(u64::from(u32::from_le_bytes(word3)), pc & 0xffff_ffff, "column {column}: Mesa's PC");
        /* DCD0: the pbuffer's front face (bit 16, `front_face_ccw`, set in
         * a window target) is the one difference with this FBO frame */
        assert_eq!(theirs[&57], 0x121aa, "column {column}: the board's DCD0");
        assert_eq!(ours[&57], theirs[&57] & !(1 << 16), "column {column}: DCD0");
        /* the shader Mesa ran is in the excerpt, where the descriptor
         * points: 16 and 17 words, ending in BRANCHZI.end */
        let n = if column == 3 { 16 } else { 17 };
        assert_eq!(trace::bytes(&m, pc + 8 * (n - 1), 8), 0x782f_c100_0000_7070u64.to_le_bytes(), "column {column}");
    }
    /* the dump holds what the test reads: load destination and enable,
     * mode Shader at 0xFFEC7000 (`mesa-blendonce.pandecode:1538-1557`) */
    let (theirs, _) =
        regs_at_idvs(&trace::cs_words(BLENDSHADER, "mesa-blendonce-c3").iter().map(|w| w.word).collect::<Vec<_>>());
    assert_eq!(
        trace::bytes(&trace::memory_of(BLENDSHADER, "mesa-blendonce-c3"), d50_16(&theirs), Blend::LENGTH),
        [1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x70, 0xec, 0xff]
    );
}

/// A blend shader given where none is needed, misaligned or outside the
/// fragment program's 4 GiB window is the caller's defect.
#[test]
fn a_blend_shader_out_of_place_is_refused() {
    let (c, _) = mode("const");
    let distinct = [0.25, 0.5, 0.75, 1.0];
    let invalid = Some(FrameError::Invalid(INVALID_BLEND_SHADER));
    assert_eq!(build_with(Some(c), [0.5; 4], Some(0x7fff_fffc_7000)).err(), invalid, "one constant: fixed function");
    assert_eq!(build_with(None, distinct, Some(0x7fff_fffc_7000)).err(), invalid, "blending off");
    assert_eq!(build_with(Some(c), distinct, Some(0x7fff_fffc_7008)).err(), invalid, "not on 16 bytes");
    assert_eq!(build_with(Some(c), distinct, Some(0x7ffe_fffc_7000)).err(), invalid, "another window");
}

/* ---- case 91 hdrblend: blends and masks into an RGBA16F target ---------- */

/// Mesa 26.2.1 on the drm-shim drawing case 91's colour program with its
/// left half's equation (`drm_shim_ref.sh run ... hdrquad.vert
/// hdrcolor.frag fsout_91_hdrcolor --blend-func ONE,ONE_MINUS_SRC_ALPHA`,
/// docs/notes/mali-fragment-outputs.md): one quad into a 64x64 RGBA8
/// target, `gl_ref.py` having no RGBA16F one. Its program words
/// (`.expect.json`) are the compiler's; the frame here uploads them.
const HDRCOLOR: &str = include_str!("../../tools/mali/testdata/fsout_91_hdrcolor.trace");
const HDRCOLOR_EXPECT: &str = include_str!("../../tools/mali/testdata/fsout_91_hdrcolor.expect.json");

/// The words of region `region` of a tools/mali/testdata `*.expect.json`.
fn region_words(json: &str, region: &str) -> Vec<u64> {
    let at = json.find(&format!("\"{region}\"")).unwrap_or_else(|| panic!("no region {region}"));
    let rest = &json[at..];
    let list = &rest[rest.find("\"words\":").expect("a words array")..];
    let list = &list[list.find('[').unwrap() + 1..list.find(']').unwrap()];
    list.split(',').map(|w| u64::from_str_radix(w.trim().trim_matches('"'), 16).expect("a hex word")).collect()
}

fn le_bytes(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// One block pandecode decoded: its `Key: value` lines, each key the path
/// of the headers above it (`Equation/RGB/A`).
type Fields = BTreeMap<String, String>;

/// The fields of the first block whose header line is `header`.
fn fields(trace: &str, header: &str) -> Fields {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let lines: Vec<&str> = trace.lines().collect();
    let at = lines.iter().position(|l| l.trim() == header).unwrap_or_else(|| panic!("no {header}"));
    let base = indent(lines[at]);
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut f = Fields::new();
    for l in lines[at + 1..].iter().take_while(|l| indent(l) > base && !l.trim().is_empty()) {
        let s = l.trim();
        while path.last().is_some_and(|(i, _)| *i >= indent(l)) {
            path.pop();
        }
        if let Some(h) = s.strip_suffix(':') {
            path.push((indent(l), h.to_string()));
            continue;
        }
        let (k, v) = s.split_once(": ").unwrap_or_else(|| panic!("{s}: not a field"));
        let key = path.iter().map(|(_, h)| h.as_str()).chain([k]).collect::<Vec<_>>().join("/");
        assert!(f.insert(key, v.to_string()).is_none(), "{s} twice");
    }
    f
}

fn take(f: &mut Fields, k: &str) -> String {
    f.remove(k).unwrap_or_else(|| panic!("no {k}"))
}

fn flag(f: &mut Fields, k: &str) -> bool {
    match take(f, k).as_str() {
        "true" => true,
        "false" => false,
        v => panic!("{k}: {v}"),
    }
}

fn num(f: &mut Fields, k: &str) -> u32 {
    let v = take(f, k);
    match v.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => v.parse(),
    }
    .unwrap_or_else(|_| panic!("{k}: {v} not a number"))
}

/// The CS word a trace's last `MOVE32 r<n>,` line sets.
fn cs_word32(trace: &str, mnemonic: &str) -> u32 {
    let l = trace.lines().rev().find(|l| l.contains(mnemonic)).unwrap_or_else(|| panic!("no {mnemonic}"));
    let b: Vec<u8> = l.split_whitespace().take(4).map(|h| u8::from_str_radix(h, 16).expect("a printed byte")).collect();
    u32::from_le_bytes(b.try_into().expect("four bytes"))
}
/// Mesa's `Blend RT 0` of the hdrcolor trace, its decoded fields packed
/// again with our packer: the blend unit's ONE, ONE_MINUS_SRC_ALPHA into
/// RGBA8 (`RGBA8 TB RGBA`).
fn hdrcolor_blend() -> Blend {
    let mut f = fields(HDRCOLOR, "Blend RT 0:");
    let f = &mut f;
    let mut func = |p: &str| {
        let a = match take(f, &format!("Equation/{p}/A")).as_str() {
            "Src" => BlendOperandA::SRC,
            "Dest" => BlendOperandA::DEST,
            "Zero" => BlendOperandA::ZERO,
            v => panic!("A {v}"),
        };
        let b = match take(f, &format!("Equation/{p}/B")).as_str() {
            "Src" => BlendOperandB::SRC,
            "Dest" => BlendOperandB::DEST,
            v => panic!("B {v}"),
        };
        let c = match take(f, &format!("Equation/{p}/C")).as_str() {
            "Zero" => BlendOperandC::ZERO,
            "Src Alpha" => BlendOperandC::SRC_ALPHA,
            v => panic!("C {v}"),
        };
        BlendFunction {
            a,
            negate_a: flag(f, &format!("Equation/{p}/Negate A")),
            b,
            negate_b: flag(f, &format!("Equation/{p}/Negate B")),
            c,
            invert_c: flag(f, &format!("Equation/{p}/Invert C")),
        }
    };
    let (rgb, alpha) = (func("RGB"), func("Alpha"));
    assert_eq!(take(f, "Internal/Mode"), "Fixed-Function");
    assert_eq!(take(f, "Internal/Fixed-Function/Conversion/Format (v7)"), "RGBA8 TB RGBA");
    let b = Blend {
        load_destination: flag(f, "Load Destination"),
        alpha_to_one: flag(f, "Alpha To One"),
        enable: flag(f, "Enable"),
        srgb: flag(f, "sRGB"),
        round_to_fb_precision: flag(f, "Round to FB precision"),
        blend_constant: num(f, "Blend Constant"),
        equation: BlendEquation { rgb, alpha, color_mask: num(f, "Equation/Color Mask") },
        internal: InternalBlend {
            mode: BlendMode::FIXED_FUNCTION,
            fixed_function: BlendFixedFunction {
                num_comps: num(f, "Internal/Fixed-Function/Num Comps"),
                rt: num(f, "Internal/Fixed-Function/RT"),
                conversion: InternalConversion {
                    memory_format: Format::RGBA8_TB.0 << 12 | RgbComponentOrder::RGBA.0,
                    raw: flag(f, "Internal/Fixed-Function/Conversion/Raw"),
                },
            },
            ..Default::default()
        },
    };
    /* `Shader/PC` overlays the conversion: the same top word */
    let pc = num(f, "Internal/Shader/PC");
    assert!(f.is_empty(), "blend fields not read: {f:?}");
    assert_eq!(pc, b.internal.fixed_function.conversion.memory_format);
    b
}

/// One draw of a case 91 frame: its blending, colour mask and scissor.
#[derive(Clone, Copy)]
struct Draw91 {
    blend: Option<Blending>,
    mask: u8,
    scissor: Option<[i32; 4]>,
}

/// What a case 91 frame built: the descriptor arena, each RUN_IDVS's
/// registers, the frame, and the code arena with its fragment program.
struct Built91 {
    desc: Vec<u8>,
    regs: Vec<BTreeMap<u32, u32>>,
    frame: Frame,
    code: Vec<u8>,
    fs_binary: u64,
}

const CODE: u64 = 0x7fff_ffff_c000;

impl Built91 {
    fn at(&self, gpu: u64, len: usize) -> &[u8] {
        let o = (gpu - DESC) as usize;
        &self.desc[o..o + len]
    }
    fn blend(&self, draw: usize) -> &[u8] {
        self.at(d50(&self.regs[draw]), Blend::LENGTH)
    }
    fn rt(&self) -> &[u8] {
        /* no depth/stencil attachment: no ZS/CRC extension before it */
        self.at(self.frame.layout.fbd.expect("an FBD").gpu + 128, RgbRenderTarget::LENGTH)
    }
    fn params(&self) -> FramebufferParameters {
        let fbd = self.frame.layout.fbd.expect("an FBD");
        FramebufferParameters::unpack(self.at(fbd.gpu, 64).try_into().unwrap()).unwrap()
    }
}

/// A frame of `draws` with Mesa's hdrquad/hdrcolor programs
/// (fsout_91_hdrcolor.expect.json) into a `w` x `h` target of `format`,
/// cleared to `clear`.
fn build_91(format: TargetFormat, w: u32, h: u32, clear: [f32; 4], draws: &[Draw91]) -> Result<Built91, FrameError> {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 1024];
    let (n, frame, fs_binary);
    {
        let mut ctx_a = Arena::new("context", 0x7fff_ffff_b000, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: 0x7fff_fffe_b000,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: 0x7fff_ffff_f000,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", CODE, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
        let vs_bin = le_bytes(&region_words(HDRCOLOR_EXPECT, "vs.position"));
        let fs_bin = le_bytes(&region_words(HDRCOLOR_EXPECT, "fs.fragment"));
        /* the programs' `preload` (0x3800.. and 0x3000..: r48-r63 in the
         * top 16 bits), 32 work registers */
        let vs_meta = ShaderMeta { preload: 0x3800, ..meta(ShaderStage::VERTEX) };
        let fs_meta = ShaderMeta { preload: 0x3000, ..meta(ShaderStage::FRAGMENT) };
        let vs = upload_program(&mut code, &mut spds, &vs_bin, vs_meta).unwrap();
        let fs = upload_program(&mut code, &mut spds, &fs_bin, fs_meta).unwrap();
        fs_binary = fs.binary;
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 96 },
            offset: 0,
            stride: 16,
            components: 4,
            instance_divisor: 0,
        }];
        let t =
            Target { gpu: 0x7fff_ffe0_0000, width: w, height: h, row_stride: w * format.bytes(), format, samples: 1 };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs)?;
        for d in draws {
            let draw = DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: d.mask,
                front_cw: false,
                cull: Cull::None,
                vs: &vs,
                fs: &fs,
                primitive: Topology::Triangles,
                first_vertex: 0,
                vertex_count: 6,
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport { x: 0, y: 0, width: w as i32, height: h as i32, near: 0.0, far: 1.0 },
                scissor: d.scissor,
                line_width: 1.0,
                flip_y: false,
                blend_color: [0.0; 4],
                blend: d.blend,
                polygon_offset: None,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
                textures: &[],
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
            };
            add_draw(&mut open, &mut desc, &mut cs, &ctx, &t, false, &draw)?;
        }
        frame = end_frame(&open, &mut desc, &mut cs, &ctx, &t, clear, None)?;
        n = cs.words().len();
    }
    let mut regs = Vec::new();
    let mut r = BTreeMap::new();
    for &w in &words[..n] {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => regs.push(r.clone()),
            _ => {}
        }
    }
    Ok(Built91 { desc: desc_m, regs, frame, code: code_m, fs_binary })
}

/// Case 91's left half (Quick 3D's premultiplied SourceOver) and right
/// half (the plain over).
fn premul() -> Blending {
    mode("premul").0
}

fn over() -> Blending {
    mode("over").0
}

/// `MALI_BLEND_RGBA16F` (pan_format.c:37): the conversion of a float target.
const RGBA16F_BLEND: u32 = Format::RGBA16F.0 << 12 | RgbComponentOrder::RGBA.0;
/// The hdrcolor draw as Mesa drew it, into RGBA8: its blend descriptor is
/// Mesa's `Blend RT 0` byte for byte (load destination, enable, no
/// constant, `A Src, B Dest, C Src Alpha inverted` on both groups, mask 15,
/// fixed function, 4 components, RT 0, `RGBA8 TB RGBA`, not raw) and DCD0
/// and DCD1 Mesa's (r57 0x21AA: forward pixel kill off for a draw reading
/// the destination; r58 0x1FFFF). The fragment program the frame runs is
/// Mesa's words, which read the descriptor (`BLEND ... blend_descriptor_0.w0`,
/// `BRANCHZI ... blend_descriptor_0.w1`).
#[test]
fn case_91_s_left_equation_is_mesa_s_into_rgba8() {
    let draw = Draw91 { blend: Some(premul()), mask: 0xf, scissor: None };
    let b = build_91(TargetFormat::Rgba8888, 64, 64, [0.0, 0.0, 1.0, 1.0], &[draw]).expect("hdrcolor's frame builds");
    assert_eq!(b.blend(0), hdrcolor_blend().pack().unwrap(), "Mesa's blend descriptor");
    assert_eq!(b.regs[0][&57], cs_word32(HDRCOLOR, "MOVE32 r57,"), "DCD0");
    assert_eq!(b.regs[0][&57], 0x21aa);
    assert_eq!(b.regs[0][&58], cs_word32(HDRCOLOR, "MOVE32 r58,"), "DCD1");
    let fs_words = region_words(HDRCOLOR_EXPECT, "fs.fragment");
    let fs = le_bytes(&fs_words);
    let o = (b.fs_binary - CODE) as usize;
    assert_eq!(b.code[o..o + fs.len()], fs[..], "Mesa's fragment program");
    /* the program that reads the descriptor: BLEND.slot0.v4.f16.end */
    assert_eq!(fs_words[4], 0x787f_4004_333c_00f0);
}

/// The same draw into RGBA16F: Mesa 26.2.1's descriptor for a float
/// target is the RGBA8 one but for the conversion (`fixed_function_float`
/// holds for ONE, ONE_MINUS_SRC_ALPHA; the equation word is the same,
/// `to_mali_function` not reading `is_float`; `pan_dithered_format_from_pipe_format`
/// of R16G16B16A16_FLOAT is MALI_BLEND_RGBA16F): the differing bytes are 13
/// and 14 only, the memory format 0xED000 (RGBA8 TB) become 0xDF000
/// (RGBA16F) in word 3, the top word the program's BRANCHZI reads.
#[test]
fn case_91_s_left_equation_into_rgba16f_differs_from_mesa_s_rgba8_in_the_conversion() {
    let draw = Draw91 { blend: Some(premul()), mask: 0xf, scissor: None };
    let b = build_91(TargetFormat::Rgba16f, 64, 64, [0.0, 0.0, 1.0, 1.0], &[draw]).expect("the RGBA16F frame builds");
    let mut want = hdrcolor_blend();
    want.internal.fixed_function.conversion.memory_format = RGBA16F_BLEND;
    assert_eq!(b.blend(0), want.pack().unwrap());
    let mesa = hdrcolor_blend().pack().unwrap();
    let differ: Vec<usize> = (0..Blend::LENGTH).filter(|&i| b.blend(0)[i] != mesa[i]).collect();
    assert_eq!(differ, [13, 14], "the conversion's format only");
    assert_eq!((mesa[13], mesa[14]), (0xd0, 0x0e));
    assert_eq!((b.blend(0)[13], b.blend(0)[14]), (0xf0, 0x0d));
    assert_eq!(b.regs[0][&57], cs_word32(HDRCOLOR, "MOVE32 r57,"), "DCD0: Mesa's");
    assert_eq!(b.regs[0][&58], cs_word32(HDRCOLOR, "MOVE32 r58,"), "DCD1: Mesa's");
}

/// Case 91's frame (gl_test.c `case_hdrblend`): an 8x8 RGBA16F target
/// cleared to (2.0, 0.5, 0.25, 1.0), the left half drawn with ONE,
/// ONE_MINUS_SRC_ALPHA and the right with SRC_ALPHA, ONE_MINUS_SRC_ALPHA,
/// each scissored. It builds; each descriptor is Mesa's fixed-function
/// one with the RGBA16F conversion (the left the drm-shim's hdrcolor
/// words, the right the board's `over` of blend-2026-10-19, Mesa 25.0.7
/// into RGBA8, whose equation word 26.2.1 packs alike); the render target
/// is Mesa 26.2.1's for R16G16B16A16_FLOAT (`Internal Format` RGBA16F
/// 0x12, the float writeback 0x13), the clear raw halves, 8 bytes a sample.
#[test]
fn case_91_s_frame_builds_with_mesa_s_float_blend_words() {
    let left = Draw91 { blend: Some(premul()), mask: 0xf, scissor: Some([0, 0, 4, 8]) };
    let right = Draw91 { blend: Some(over()), mask: 0xf, scissor: Some([4, 0, 4, 8]) };
    let b = build_91(TargetFormat::Rgba16f, 8, 8, [2.0, 0.5, 0.25, 1.0], &[left, right]).expect("case 91 builds");
    assert_eq!(b.regs.len(), 2, "two draws");

    let mut want = hdrcolor_blend();
    want.internal.fixed_function.conversion.memory_format = RGBA16F_BLEND;
    let want = want.pack().unwrap();
    assert_eq!(b.blend(0), want, "the left half");
    /* the board's `over`, its conversion RGBA16F's */
    let (theirs, _) = regs_at_idvs(&trace::cs_words(BLEND, "mesa-over").iter().map(|w| w.word).collect::<Vec<_>>());
    let board = trace::bytes(&trace::memory_of(BLEND, "mesa-over"), d50(&theirs), Blend::LENGTH);
    assert_eq!((board[13], board[14]), (0xd0, 0x0e), "the board's RGBA8 TB");
    let mut right_want = board.clone();
    right_want[12..16].copy_from_slice(&want[12..16]);
    assert_eq!(b.blend(1), &right_want[..], "the right half");
    for r in &b.regs {
        assert_eq!(r[&57], 0x21aa, "DCD0: the destination read");
    }

    /* the render target against hdrcolor's RGBA8 one, field by field;
     * what differs is named */
    let mut f = fields(HDRCOLOR, "AFBC RGB Color Render Target 0:");
    assert_eq!(take(&mut f, "Writeback Mode"), "Compat");
    assert_eq!(take(&mut f, "Internal Buffer Offset"), "0");
    /* the named difference: the tile buffer's format, R8G8B8A8 (1) there,
     * RGBA16F (0x12, `BFMT_FLOAT`) here */
    assert_eq!(take(&mut f, "Internal Format"), "R8G8B8A8");
    /* the writeback: 0x13 in the five bits both names share, R8G8B8A8 as a
     * Color Format and RGBA16F as a Float Color Format; Mesa's RGBA16F
     * target (`MALI_FLOAT_COLOR_FORMAT_RGBA16F`) writes the same bits */
    assert_eq!(take(&mut f, "Writeback Format"), "R8G8B8A8");
    assert_eq!(take(&mut f, "Writeback Float Format"), "RGBA16F");
    assert_eq!(ColorFormat::R8G8B8A8.0, FloatColorFormat::RGBA16F.0);
    assert_eq!(take(&mut f, "Writeback MSAA"), "Single");
    assert_eq!(take(&mut f, "Swizzle"), "1672 (RGBA)");
    let want_rt = RgbRenderTarget {
        writeback_mode: WritebackMode::COMPAT,
        internal_buffer_offset: 0,
        yuv_enable: flag(&mut f, "YUV Enable"),
        dithered_clear: flag(&mut f, "Dithered Clear"),
        internal_format: ColorBufferInternalFormat::RGBA16F,
        write_enable: flag(&mut f, "Write Enable"),
        writeback_float_format: FloatColorFormat::RGBA16F,
        writeback_msaa: Msaa::SINGLE,
        srgb: flag(&mut f, "sRGB"),
        dithering_enable: flag(&mut f, "Dithering Enable"),
        swizzle: 1672,
        clean_tile_write_enable: flag(&mut f, "Clean Tile Write Enable"),
        /* the named difference: raw halves (`pan_pack_raw`), R lowest:
         * 2.0 0x4000, 0.5 0x3800, 0.25 0x3400, 1.0 0x3C00, twice */
        clear: RtClear { color_0: 0x3800_4000, color_1: 0x3c00_3400, color_2: 0x3800_4000, color_3: 0x3c00_3400 },
        /* the named difference: linear, the target's memory, where Mesa's
         * is AFBC */
        writeback_block_format: BlockFormat::LINEAR,
        writeback_buffer: RtBuffer { base: 0x7fff_ffe0_0000, row_stride: 64, surface_stride: 0 },
        ..Default::default()
    };
    assert_eq!(b.rt(), want_rt.pack().unwrap(), "the render target");
    for c in 0..4 {
        take(&mut f, &format!("Clear/Color {c}"));
    }
    assert_eq!(take(&mut f, "Writeback Block Format"), "AFBC");
    for k in [
        "YUV Transform",
        "Split block",
        "Wide block",
        "Reverse issue order",
        "Front buffer",
        "Alpha hint",
        "Compression mode",
        "Header",
        "Row stride",
        "Body offset",
    ] {
        take(&mut f, k);
    }
    assert!(f.is_empty(), "render target fields not read: {f:?}");
    /* 8 bytes a sample (`pan_format_tib_size` of a float format, RAW64's
     * too): a 1024-pixel tile, 8 KiB of colour buffer */
    let p = b.params();
    assert_eq!((p.effective_tile_size, p.color_buffer_allocation), (1024, 8192));
}

/// A frame whose RGBA16F draws all write every channel opaquely keeps the
/// board's Mesa 25.0.7 RAW64 words (case 54's, frame_formats_tests.rs);
/// one blended or colour-masked draw makes the frame's tile buffer
/// RGBA16F, whichever draw it is.
#[test]
fn only_a_frame_that_reads_its_rgba16f_target_keeps_it_as_rgba16f() {
    /* `Internal Format`: bits 26..31 of word 0 */
    let internal = |b: &Built91| ColorBufferInternalFormat(u32::from_le_bytes(b.rt()[0..4].try_into().unwrap()) >> 26);
    let opaque = Draw91 { blend: None, mask: 0xf, scissor: None };
    let replace = Draw91 { blend: Some(mode("replace").0), ..opaque };
    let blended = Draw91 { blend: Some(over()), ..opaque };
    let masked = Draw91 { mask: 0x7, ..opaque };
    let clear = [0.0, 0.0, 1.0, 1.0];
    for (draws, want) in [
        (&[opaque][..], ColorBufferInternalFormat::RAW64),
        (&[opaque, replace][..], ColorBufferInternalFormat::RAW64),
        (&[opaque, blended][..], ColorBufferInternalFormat::RGBA16F),
        (&[blended, opaque][..], ColorBufferInternalFormat::RGBA16F),
        (&[masked][..], ColorBufferInternalFormat::RGBA16F),
    ] {
        let b = build_91(TargetFormat::Rgba16f, 8, 8, clear, draws).expect("builds");
        assert_eq!(internal(&b), want, "{} draws", draws.len());
    }
    /* an RGBA8 target is R8G8B8A8 whatever the draws */
    let b = build_91(TargetFormat::Rgba8888, 8, 8, clear, &[blended]).unwrap();
    assert_eq!(internal(&b), ColorBufferInternalFormat::R8G8B8A8);
}

/// A colour-masked draw into RGBA16F (blending off, mask R, G, B): fixed
/// function (`pan_blend_can_fixed_function` holds with blending off), the
/// destination loaded, `replace` under the mask, the RGBA16F conversion.
#[test]
fn a_colour_masked_draw_into_rgba16f_runs_in_the_blend_unit() {
    let masked = Draw91 { blend: None, mask: 0x7, scissor: None };
    let b = build_91(TargetFormat::Rgba16f, 8, 8, [0.0; 4], &[masked]).expect("the masked draw builds");
    let replace =
        BlendFunction { a: BlendOperandA::SRC, b: BlendOperandB::SRC, c: BlendOperandC::ZERO, ..Default::default() };
    let want = Blend {
        load_destination: true,
        enable: true,
        equation: BlendEquation { rgb: replace, alpha: replace, color_mask: 0x7 },
        internal: InternalBlend {
            mode: BlendMode::FIXED_FUNCTION,
            fixed_function: BlendFixedFunction {
                num_comps: 4,
                rt: 0,
                conversion: InternalConversion { memory_format: RGBA16F_BLEND, raw: false },
            },
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(b.blend(0), want.pack().unwrap());
    assert_eq!(b.regs[0][&57], 0x21aa, "DCD0: the destination read");
}

/// The equations the blend unit does not run on floats are refused by
/// name before any word, whatever the mask: a destination-colour factor,
/// the 2src form, a constant, min; with no channel written there is
/// nothing to blend. blend.c's other modes into RGBA16F build.
#[test]
fn an_rgba16f_blend_that_needs_a_blend_shader_is_refused_by_name() {
    let min = Blending {
        rgb: Channel { func: Func::Min, src: Factor::ONE, dst: Factor::ONE },
        alpha: Channel { func: Func::Add, src: Factor::ONE, dst: Factor::ZERO },
    };
    let refused = Some(FrameError::NotBuilt(NOT_BUILT_FLOAT_BLEND));
    for b in [mode("mul").0, mode("twosd").0, mode("const").0, min] {
        for mask in [0xf, 0x7] {
            let d = [Draw91 { blend: Some(b), mask, scissor: None }];
            assert_eq!(build_91(TargetFormat::Rgba16f, 8, 8, [0.0; 4], &d).err(), refused, "{b:?} mask {mask:#x}");
        }
        let off = [Draw91 { blend: Some(b), mask: 0, scissor: None }];
        assert!(build_91(TargetFormat::Rgba16f, 8, 8, [0.0; 4], &off).is_ok(), "{b:?}: no channel written");
    }
    for name in ["over", "add", "premul", "replace", "sub", "revsub", "sep"] {
        let d = [Draw91 { blend: Some(mode(name).0), mask: 0xf, scissor: None }];
        assert!(build_91(TargetFormat::Rgba16f, 8, 8, [0.0; 4], &d).is_ok(), "{name}");
    }
}
