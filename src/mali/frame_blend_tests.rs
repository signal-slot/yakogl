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
                texture: None,
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
