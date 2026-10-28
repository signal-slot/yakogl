// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's points against the board: the draws Mesa 25.0.7
//! made on the NanoPC-T6 for points.c's modes (tools/mali/board-dump/
//! points-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/2026-10-19-points/
//! mesa).
//!
//! What is compared, per mode: the registers the RUN_IDVS reads that the
//! mode moves (r56 the primitive flags, d60 the primitive size, r33 the
//! vertex count), the RUN_IDVS flags override (the draw mode), and d16, the
//! vertex Shader Program descriptor, with the binary it names. The arenas
//! stand at Mesa's addresses, so the descriptors and binaries do too: the
//! vertex binary at 0x7fffffffc000 (256 bytes, Mesa 25.0.7's no_psiz copy at
//! 0x80), its points descriptor at 0x7fffffffd000 and its triangle
//! descriptor at 0x7fffffffd020.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const POINTS: &str = include_str!("../../tools/mali/board-dump/points-2026-10-19.txt");
const POINTCOORD: &str = include_str!("../../tools/mali/board-dump/pointcoord-2026-10-19.txt");
const DESC: u64 = 0x7fff_fffd_0000;

const SPDS: u64 = 0x7fff_ffff_d000;
const CODE: u64 = 0x7fff_ffff_c000;
/// Mesa 25.0.7's `no_psiz_offset` for points.c's const shader (its
/// triangle descriptor's binary, 0x7fffffffc080).
const NO_PSIZ: u32 = 0x80;

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

fn meta(stage: ShaderStage, no_psiz_offset: Option<u32>) -> ShaderMeta<'static> {
    ShaderMeta {
        requires_helper_threads: false,
        samples_texture: false,
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: 0,
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset,
        colour_outputs: u32::from(stage == ShaderStage::FRAGMENT),
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo: &[],
    }
}

/// points.c's frame, cleared blue with one draw in `primitive` of a vertex
/// shader that writes gl_PointSize (`psiz`) or not: the stream's words and
/// the descriptor arena's bytes.
fn build(primitive: Topology, vertex_count: u32, psiz: bool) -> (Vec<u64>, Vec<u8>) {
    let (words, spds, _) = build_on(primitive, vertex_count, psiz, false);
    (words, spds)
}

/// [`build`] into a bottom-up target or a top-down one (`flip_y`), with the
/// frame's framebuffer parameters.
fn build_on(
    primitive: Topology,
    vertex_count: u32,
    psiz: bool,
    flip_y: bool,
) -> (Vec<u64>, Vec<u8>, FramebufferParameters) {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let n = {
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
        let mut spds = Arena::new("spds", SPDS, &mut spd_m).unwrap();
        /* a position program and its no_psiz copy, each 128 bytes */
        let vs_bin = [0u8; 256];
        let vs_meta = meta(ShaderStage::VERTEX, psiz.then_some(NO_PSIZ));
        let vs = upload_program(&mut code, &mut spds, &vs_bin, vs_meta).unwrap();
        let fs = upload_program(&mut code, &mut spds, &[0u8; 128], meta(ShaderStage::FRAGMENT, None)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 64 },
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
            clear_rgba: Some([0.0, 0.0, 1.0, 1.0]),
            draw: Some(DrawCall {
                textures: &[],
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw: false,
                cull: Cull::None,
                vs: &vs,
                fs: &fs,
                primitive,
                first_vertex: 0,
                vertex_count,
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport { x: 0, y: 0, width: 64, height: 64, near: 0.0, far: 1.0 },
                scissor: None,
                line_width: 1.0,
                flip_y,
                blend_color: [0.0; 4],
                blend: None,
                polygon_offset: None,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
            }),
        };
        let mut cs = Stream::new(&mut words, 92);
        let f = build_frame(&mut desc, &mut cs, &ctx, &input).unwrap();
        (cs.words().len(), ((f.fbd_pointer & !63) - DESC) as usize)
    };
    let (n, fbd) = n;
    let params = FramebufferParameters::unpack(&desc_m[fbd..fbd + 64].try_into().unwrap()).unwrap();
    (words[..n].to_vec(), spd_m, params)
}

/// The `Binary:` the excerpt's Position shader descriptor of `source`
/// names, and the descriptor's address.
fn mesa_position_shader(source: &str) -> (u64, u64) {
    let mut block = None;
    let (mut at, mut binary) = (None, None);
    let hex = |s: &str| u64::from_str_radix(s.trim().trim_start_matches("0x").trim_end_matches(':'), 16).unwrap();
    for line in POINTS.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            block = h.split(" lines ").next().map(str::to_string);
            continue;
        }
        if block.as_deref() != Some(source) {
            continue;
        }
        if let Some(a) = line.trim().strip_prefix("Position shader Shader @") {
            at = Some(hex(a));
        }
        if let Some(b) = line.trim().strip_prefix("Binary: ") {
            binary = Some(hex(b));
        }
    }
    (at.expect("a Position shader descriptor"), binary.expect("its binary"))
}

/// Each points.c mode's draw reads the board's primitive flags, primitive
/// size, vertex count, draw mode and vertex descriptor, and that
/// descriptor names the board's binary: a points draw of a shader that
/// writes gl_PointSize the point size array (r56 0x839000, d60 0) and the
/// program at offset 0; its triangle strip the no_psiz copy; a points draw
/// of a shader that does not write it the fixed size 1.0.
#[test]
fn each_points_mode_s_draw_is_the_board_s() {
    let modes = [
        ("const", Topology::Points, 3, true),
        ("attr", Topology::Points, 3, true),
        ("uniform", Topology::Points, 3, true),
        ("tri_psiz", Topology::TriangleStrip, 4, true),
        ("nopsiz", Topology::Points, 3, false),
    ];
    for (mode, primitive, count, psiz) in modes {
        let source = format!("mesa-{mode}");
        let mesa_words: Vec<u64> = trace::cs_words(POINTS, &source).iter().map(|w| w.word).collect();
        let (mesa, mesa_over) = regs_at_idvs(&mesa_words);
        let (words, spds) = build(primitive, count, psiz);
        let (ours, over) = regs_at_idvs(&words);
        for r in [56, 60, 61, 33, 16, 17] {
            assert_eq!(ours.get(&r), mesa.get(&r), "{mode}: r{r}");
        }
        assert_eq!(over, mesa_over, "{mode}: the RUN_IDVS flags override");
        /* the descriptor d16 names: Mesa's, and its binary Mesa's */
        let (at, binary) = mesa_position_shader(&source);
        let d16 = u64::from(ours[&16]) | u64::from(ours[&17]) << 32;
        assert_eq!(d16, at, "{mode}: the vertex descriptor");
        let off = (d16 - SPDS) as usize;
        let ours_binary = u64::from_le_bytes(spds[off + 8..off + 16].try_into().unwrap());
        assert_eq!(ours_binary, binary, "{mode}: the vertex descriptor's binary");
    }
    /* the excerpt holds what the test reads */
    let (r56, _) = regs_at_idvs(&trace::cs_words(POINTS, "mesa-const").iter().map(|w| w.word).collect::<Vec<_>>());
    assert_eq!((r56[&56], r56[&60], r56[&61]), (0x839000, 0, 0), "const: r56, d60");
    assert_eq!(mesa_position_shader("mesa-tri_psiz"), (SPDS + 0x20, CODE + u64::from(NO_PSIZ)));
}

/// A no_psiz offset the binary does not hold, or one not on a program's
/// 128-byte boundary, or on a fragment shader, is a record defect.
#[test]
fn a_no_psiz_offset_outside_the_position_program_is_invalid() {
    let (mut code_m, mut spd_m) = (vec![0u8; 0x800], vec![0u8; 0x100]);
    let mut code = Arena::new("code", CODE, &mut code_m).unwrap();
    let mut spds = Arena::new("spds", SPDS, &mut spd_m).unwrap();
    let bin = [0u8; 256];
    for (stage, at) in
        [(ShaderStage::VERTEX, 0), (ShaderStage::VERTEX, 64), (ShaderStage::VERTEX, 256), (ShaderStage::FRAGMENT, 128)]
    {
        let r = upload_program(&mut code, &mut spds, &bin, meta(stage, Some(at)));
        assert_eq!(r.map(|_| ()), Err(FrameError::Invalid(INVALID_NO_PSIZ)), "{stage:?} at {at}");
    }
}

/// The framebuffer descriptor of gl_test case 83's draw on the board: the
/// point sprite origin is high y into an FBO and low y into a pbuffer
/// (Mesa's FBD at 0x7ffffffd4000, the excerpt's first 64 bytes; the two
/// differ in that bit only). Ours sets it from the target's orientation, a
/// points draw only.
#[test]
fn the_point_sprite_origin_is_the_board_s() {
    let mem = |src: &str| {
        let m = trace::memory_of(POINTCOORD, src);
        FramebufferParameters::unpack(&trace::bytes(&m, 0x7fff_fffd_4000, 64).try_into().unwrap()).unwrap()
    };
    let (fbo, pb) = (mem("mesa-fbo"), mem("mesa-pb"));
    assert_eq!((fbo.point_sprite_coord_origin_max_y, pb.point_sprite_coord_origin_max_y), (true, false));
    assert_eq!(FramebufferParameters { point_sprite_coord_origin_max_y: false, ..fbo }.pack(), pb.pack());
    /* an FBO is bottom-up, a pbuffer of Mesa's top-down (ours: flip_y) */
    for (flip_y, want) in [(false, fbo), (true, pb)] {
        let (_, _, ours) = build_on(Topology::Points, 1, true, flip_y);
        assert_eq!(ours.point_sprite_coord_origin_max_y, want.point_sprite_coord_origin_max_y, "flip_y {flip_y}");
    }
    /* no points draw: the batch's tristate stays unset */
    let (_, _, tri) = build_on(Topology::TriangleStrip, 4, true, false);
    assert!(!tri.point_sprite_coord_origin_max_y);
}
