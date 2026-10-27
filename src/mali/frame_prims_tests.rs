// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's primitives against the board: the draws Mesa 25.0.7
//! made on the NanoPC-T6 for one red draw in each GL mode other than
//! GL_TRIANGLES into a 64x64 RGBA8 FBO (tools/mali/board-dump/
//! prims-2026-10-19.txt, cut from 0dce654a8:tools/mali/board-ref/
//! 2026-10-19-primitives/mesa).
//!
//! What is compared, per mode: the registers the RUN_IDVS reads that the
//! mode moves in Mesa (r56 the tiler's primitive flags, d60 the primitive
//! size, r33 the vertex count) and the RUN_IDVS flags override, which
//! carries the draw mode. The DCD flags (r57, r58) do not move with the
//! mode in the dump, and do not in ours. The draw's other blocks are the
//! triangle frame's, which `frame_tests.rs` compares with the L1 trace.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const PRIMS: &str = include_str!("../../tools/mali/board-dump/prims-2026-10-19.txt");

/// Each mode prims.c draws: the dump's source name, the topology and the
/// vertex count.
const MODES: [(&str, Topology, u32); 5] = [
    ("lines", Topology::Lines, 4),
    ("line_strip", Topology::LineStrip, 3),
    ("line_loop", Topology::LineLoop, 3),
    ("tri_strip", Topology::TriangleStrip, 4),
    ("tri_fan", Topology::TriangleFan, 4),
];

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

/// prims.c's frame, cleared blue with one draw in `primitive`: the
/// stream's words, or the builder's refusal.
fn build(primitive: Topology, vertex_count: u32) -> Result<Vec<u64>, FrameError> {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
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
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: Cull::None,
            texture: None,
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
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
            flip_y: false,
            blend_color: [0.0; 4],
            blend: None,
            polygon_offset: None,
            sample_mask: 0xf,
            vs_uniforms: &[],
            fs_uniforms: &[],
        }),
    };
    let mut cs = Stream::new(&mut words, 92);
    build_frame(&mut desc, &mut cs, &ctx, &input)?;
    let n = cs.words().len();
    Ok(words[..n].to_vec())
}

/// Each mode's draw reads the board's primitive flags, primitive size,
/// vertex count and draw mode; the DCD flags do not move with the mode.
#[test]
fn each_primitive_s_draw_is_the_board_s() {
    let (triangles, _) = regs_at_idvs(&build(Topology::Triangles, 6).unwrap());
    let mut mesa_dcd = None;
    for (source, primitive, count) in MODES {
        let mesa_words: Vec<u64> = trace::cs_words(PRIMS, &format!("mesa-{source}")).iter().map(|w| w.word).collect();
        let (mesa, mesa_over) = regs_at_idvs(&mesa_words);
        let (ours, over) = regs_at_idvs(&build(primitive, count).unwrap());
        for r in [56, 60, 61, 33] {
            assert_eq!(ours.get(&r), mesa.get(&r), "{source}: r{r}");
        }
        assert_eq!(over, mesa_over, "{source}: the RUN_IDVS flags override");
        /* r57/r58 are the same in every mode of the dump, and ours are the
         * triangle draw's */
        let dcd = (mesa[&57], mesa[&58]);
        assert_eq!(*mesa_dcd.get_or_insert(dcd), dcd, "{source}: the dump's DCD flags moved");
        assert_eq!((ours[&57], ours[&58]), (triangles[&57], triangles[&58]), "{source}: our DCD flags moved");
    }
    /* the dump holds what the test reads: line modes do not allow rotating
     * primitives (bit 15 of r56), strips and fans do; line width 1.0 */
    for (source, r56, over) in [
        ("lines", 0x430000, 2),
        ("line_strip", 0x430000, 4),
        ("line_loop", 0x430000, 6),
        ("tri_strip", 0x438000, 0xa),
        ("tri_fan", 0x438000, 0xc),
    ] {
        let w = trace::cs_words(PRIMS, &format!("mesa-{source}"));
        assert!(w.iter().any(|w| w.mnemonic == format!("MOVE32 r56, #{r56:#X}").replace("0X", "0x")), "{source}");
        assert!(w.iter().any(|w| w.mnemonic == "MOVE d60, #0x3F800000"), "{source}");
        assert!(w
            .iter()
            .any(|w| w.mnemonic.starts_with("RUN_IDVS") && w.mnemonic.ends_with(&format!("r0, #{over:x}"))));
    }
}

/// prims.c's points draw is points.c's const mode (the same program and
/// calls): the frame_points_tests.rs proof covers it. The excerpt here
/// holds the same registers.
#[test]
fn prims_c_points_is_points_c_const() {
    let points = include_str!("../../tools/mali/board-dump/points-2026-10-19.txt");
    let regs = |ex: &str, src: &str| regs_at_idvs(&trace::cs_words(ex, src).iter().map(|w| w.word).collect::<Vec<_>>());
    let ((a, ao), (b, bo)) = (regs(PRIMS, "mesa-points"), regs(points, "mesa-const"));
    for r in [56, 60, 61, 33] {
        assert_eq!(a.get(&r), b.get(&r), "r{r}");
    }
    assert_eq!(ao, bo);
}
