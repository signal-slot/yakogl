// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's depth bias (glPolygonOffset) against the board: the
//! depth/stencil descriptors Mesa 25.0.7 made on the NanoPC-T6 for a blue
//! quad and then a green one on the same plane with
//! GL_POLYGON_OFFSET_FILL, in seven offsets (tools/mali/board-dump/
//! polyoffset-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/
//! 2026-10-19-polyoffset/mesa).
//!
//! What is compared, byte for byte: the offset draw's descriptor (the bias
//! on, the factor as GL gives it, the units doubled for the D24S8 buffer)
//! and the first draw's (no bias, factor and units 0).

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const POFF: &str = include_str!("../../tools/mali/board-dump/polyoffset-2026-10-19.txt");
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

/// poff.c's offset draw: the depth test LESS with writes into a D24S8
/// attachment, `polygon_offset` on; the descriptor arena's bytes and the
/// RUN_IDVS registers.
fn build(polygon_offset: Option<[f32; 2]>) -> (Vec<u8>, BTreeMap<u32, u32>) {
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
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 72 },
            offset: 0,
            stride: 12,
            components: 3,
            instance_divisor: 0,
        }];
        let input = FrameInput {
            zs: Some(ZsAttachment { gpu: 0x7fff_fffe_0000, row_stride: 256, clear_depth: 1.0, clear_stencil: 0 }),
            target: Target {
                gpu: 0x7fff_fffe_4000,
                width: 64,
                height: 64,
                row_stride: 256,
                format: TargetFormat::Rgba8888,
                samples: 1,
            },
            clear_rgba: Some([0.0, 0.0, 0.0, 1.0]),
            draw: Some(DrawCall {
                zs: DepthStencilState { depth_func: Func::LESS, depth_write: true, ..DepthStencilState::OFF },
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
                blend_color: [0.0; 4],
                blend: None,
                polygon_offset,
                sample_mask: 0xf,
                vs_uniforms: &[],
                fs_uniforms: &[],
                texture: None,
                vs_texture: None,
                occlusion_query: None,
                alpha_to_coverage: false,
                blend_shader: None,
            }),
        };
        let mut cs = Stream::new(&mut words, 92);
        build_frame(&mut desc, &mut cs, &ctx, &input).expect("poff.c's frame builds");
        n = cs.words().len();
    }
    let (regs, _) = regs_at_idvs(&words[..n]);
    (desc_m, regs)
}

/// Each mode's glPolygonOffset(factor, units).
const MODES: [(&str, f32, f32); 7] = [
    ("flat_0_m4", 0.0, -4.0),
    ("flat_0_4", 0.0, 4.0),
    ("flat_0_m1", 0.0, -1.0),
    ("flat_m1_0", -1.0, 0.0),
    ("tilt_m1_0", -1.0, 0.0),
    ("tilt_1_0", 1.0, 0.0),
    ("tilt_m0p5_m1", -0.5, -1.0),
];

/// The two d52 moves of a source's frame: the first draw's, the offset
/// draw's.
fn zsds(source: &str) -> Vec<u64> {
    trace::cs_words(POFF, source)
        .iter()
        .filter_map(|w| match Instr::decode(w.word) {
            Ok(Instr::Move48(m)) if m.destination == 52 => Some(m.immediate),
            _ => None,
        })
        .collect()
}

/// The offset draw's depth/stencil descriptor is the board's in each mode,
/// and a draw with no offset is the first draw's.
#[test]
fn each_polygon_offset_is_the_board_s_depth_bias() {
    let ours_at = |b: &(Vec<u8>, BTreeMap<u32, u32>)| {
        let at = ((u64::from(b.1[&52]) | u64::from(b.1[&53]) << 32) - DESC) as usize;
        b.0[at..at + DepthStencil::LENGTH].to_vec()
    };
    for (name, factor, units) in MODES {
        let source = format!("mesa-{name}");
        let m = trace::memory_of(POFF, &source);
        let at = zsds(&source);
        assert_eq!(at.len(), 2, "{name}: two draws");
        let theirs = |va: u64| trace::bytes(&m, va, DepthStencil::LENGTH);
        assert_eq!(ours_at(&build(Some([factor, units]))), theirs(at[1]), "{name}: the offset draw");
        assert_eq!(ours_at(&build(None)), theirs(at[0]), "{name}: the draw before it");
    }
    /* the dump holds what the test reads: units doubled (GL -4 as -8.0,
     * bytes 20-23), the factor as it is (bytes 24-27), the bias bit (byte
     * 19 bit 4) */
    let m = trace::memory_of(POFF, "mesa-flat_0_m4");
    let b = trace::bytes(&m, zsds("mesa-flat_0_m4")[1], DepthStencil::LENGTH);
    assert_eq!(f32::from_le_bytes(b[20..24].try_into().unwrap()), -8.0);
    assert_eq!(b[19] & 0x10, 0x10);
    let m = trace::memory_of(POFF, "mesa-tilt_m0p5_m1");
    let b = trace::bytes(&m, zsds("mesa-tilt_m0p5_m1")[1], DepthStencil::LENGTH);
    assert_eq!(f32::from_le_bytes(b[24..28].try_into().unwrap()), -0.5);
}
