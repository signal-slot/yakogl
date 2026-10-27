// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's face culling against the board: gl_test case 4
//! (cull), the draws Mesa 25.0.7 made on the NanoPC-T6, six triangle draws
//! (four into a 64x48 window, two into an FBO) and one GL_LINES draw with
//! culling on (tools/mali/board-dump/cull-2026-10-24.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs5).
//!
//! What is compared: the whole of DCD Flags 0 (r57), the word the CS loads
//! for the draw. Bit 16 is Front Face CCW, bit 17 Cull Front Face, bit 18
//! Cull Back Face. The rest of the word is the flat-colour triangle's and
//! does not move between the draws. A points or lines draw leaves bits 17
//! and 18 clear however the culling is set (`polygon &&` in pan_csf.c:1549).

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;

const CULL: &str = include_str!("../../tools/mali/board-dump/cull-2026-10-24.txt");
const W: u32 = 64;
const H: u32 = 48;
const FRONT_FACE_CCW: u32 = 1 << 16;
const CULL_FRONT_FACE: u32 = 1 << 17;
const CULL_BACK_FACE: u32 = 1 << 18;

/// r57 as the one RUN_IDVS of a stream reads it.
fn dcd0_of(words: &[u64]) -> u32 {
    let mut r57 = None;
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) if m.destination == 57 => r57 = Some(m.immediate),
            Ok(Instr::RunIdvs(_)) => return r57.expect("no MOVE32 r57 before RUN_IDVS"),
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

/// A frame with one draw of `primitive` into a top-down target when
/// `flip_y`, glFrontFace(GL_CW) when `front_cw`, culling `cull`: DCD0.
fn build(flip_y: bool, front_cw: bool, cull: Cull, primitive: Topology) -> u32 {
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
        let mut desc = Arena::new("desc", 0x7fff_fffd_0000, &mut desc_m).unwrap();
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
                width: W,
                height: H,
                row_stride: 4 * W,
                format: TargetFormat::Rgba8888,
                samples: 1,
            },
            clear_rgba: Some([0.0, 0.0, 1.0, 1.0]),
            draw: Some(DrawCall {
                zs: DepthStencilState::OFF,
                colour_mask: 0xf,
                front_cw,
                cull,
                texture: None,
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
                vs: &vs,
                fs: &fs,
                primitive,
                first_vertex: 0,
                vertex_count: 3,
                instance_count: 1,
                indices: None,
                attributes: &attributes,
                viewport: Viewport { x: 0, y: 0, width: W as i32, height: H as i32, near: 0.0, far: 1.0 },
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
        build_frame(&mut desc, &mut cs, &ctx, &input).expect("case 4's frame builds");
        n = cs.words().len();
    }
    dcd0_of(&words[..n])
}

/// The dump's DCD0 of frame `n` (source cull<n>).
fn mesa(n: u32) -> u32 {
    let words: Vec<u64> = trace::cs_words(CULL, &format!("cull{n}")).iter().map(|w| w.word).collect();
    dcd0_of(&words)
}

/// The six triangle draws of case 4: the dump's words, from the draw state
/// GL_TEST sets (GlesDraw.cull is the winding of the faces culled:
/// GL_BACK + CCW front is CW, GL_FRONT + CCW is CCW, GL_BACK + CW is CCW,
/// GL_FRONT + CW is CW).
#[test]
fn case_4_s_six_draws_are_the_board_s_dcd_flags() {
    use Cull::*;
    let draws = [
        // (frame, window, glFrontFace CW, culled winding, r57)
        (1, true, false, Cw, 0x521AB),
        (2, true, false, Ccw, 0x321AB),
        (3, true, true, Ccw, 0x421AB),
        (4, true, true, Cw, 0x221AB),
        (5, false, false, Cw, 0x421AB),
        (6, false, false, Ccw, 0x221AB),
    ];
    for (frame, flip_y, front_cw, cull, want) in draws {
        assert_eq!(mesa(frame), want, "frame {frame}: the dump's r57");
        assert_eq!(build(flip_y, front_cw, cull, Topology::Triangles), want, "frame {frame}: our r57");
    }
}

/// Strips and fans are polygons too (`u_reduced_prim`).
#[test]
fn strips_and_fans_cull_as_lists_do() {
    for prim in [Topology::TriangleStrip, Topology::TriangleFan] {
        assert_eq!(build(true, false, Cull::Cw, prim), 0x521AB, "{prim:?}");
        assert_eq!(build(true, false, Cull::Ccw, prim), 0x321AB, "{prim:?}");
    }
}

/// Lines and points are drawn even when every face is culled: bits 17 and
/// 18 stay clear. Frame 7 of the dump is a GL_LINES draw with GL_BACK
/// culling on (r57 0x121AB, a window's no-cull word).
#[test]
fn points_and_lines_keep_the_cull_bits_clear() {
    assert_eq!(mesa(7), 0x121AB);
    for prim in [Topology::Lines, Topology::LineStrip, Topology::LineLoop, Topology::Points] {
        for cull in [Cull::Cw, Cull::Ccw] {
            let w = build(true, false, cull, prim);
            assert_eq!(w & (CULL_FRONT_FACE | CULL_BACK_FACE), 0, "{prim:?} {cull:?}");
            assert_eq!(w, build(true, false, Cull::None, prim), "{prim:?} {cull:?}: the no-cull word");
        }
    }
    assert_eq!(build(true, false, Cull::Cw, Topology::Lines), 0x121AB);
}

/// Without culling the word is what it was before: the front face bit only
/// (a window CCW 0x121AB, an FBO CCW 0x21AB, flip's dump).
#[test]
fn no_cull_words_are_unchanged() {
    for (flip_y, front_cw, want) in [(true, false, 0x121AB), (false, false, 0x21AB), (true, true, 0x21AB)] {
        let w = build(flip_y, front_cw, Cull::None, Topology::Triangles);
        assert_eq!(w, want, "flip_y={flip_y} front_cw={front_cw}");
        assert_eq!(w & (CULL_FRONT_FACE | CULL_BACK_FACE), 0);
        assert_eq!(w & FRONT_FACE_CCW != 0, flip_y != front_cw);
    }
}
