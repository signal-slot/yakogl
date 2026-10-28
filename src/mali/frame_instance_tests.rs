// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's per-instance vertex attributes against the board:
//! gl_test case 52 (es3split) as Mesa 25.0.7 drew it on the NanoPC-T6
//! (tools/mali/board-dump/instsplit-2026-10-24.txt, cut from
//! 0dce654a8:tools/mali/board-ref/2026-10-24-mesa-refs5/mesa/mesa-instsplit.pandecode;
//! docs/notes/mali-frame-formats.md sections 3 and 6).
//!
//! The draw: `glDrawElementsInstanced(GL_TRIANGLES, 294, GL_UNSIGNED_SHORT,
//! 0, n)`, n = 8 (inst1) and 40 (inst2); attribute 0 a vec4 per vertex
//! (stride 16, the 8x8 grid at 7ffffffea000, 1024 bytes), attributes 1 and 2
//! a vec2 at offset 0 and a vec4 at offset 8 of one instance buffer
//! (7ffffffde000, 40 records of 24 bytes), divisor 1 each.
//!
//! What is compared, per draw: the three Attribute descriptors and the three
//! Buffer descriptors byte for byte (built with the dump's buffer
//! addresses), and the registers the draw moves for its counts (r33 the
//! index count, r34 INSTANCE_COUNT, r35 the index offset, r36 the vertex
//! offset, r39 the indices' bytes).

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const INST: &str = include_str!("../../tools/mali/board-dump/instsplit-2026-10-24.txt");

/// The dump's descriptor buffer: the three Attributes, then the three
/// Buffers, 32 bytes each.
const ATTRIBUTES: u64 = 0x7fff_fffc_e160;
const BUFFERS: u64 = 0x7fff_fffc_e1c0;
/// INSTANCE_COUNT (`frame.rs` reg::INSTANCE_COUNT).
const R_INSTANCE_COUNT: u32 = 34;
const GRID: u64 = 0x7fff_fffe_a000;
const INSTANCES: u64 = 0x7fff_fffd_e000;

/// The registers the one RUN_IDVS of a stream reads.
fn regs_at_idvs(words: &[u64]) -> BTreeMap<u32, u32> {
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
            Ok(Instr::RunIdvs(_)) => return r,
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

/// Case 52's attributes as the backend hands them: the grid per vertex, the
/// instance buffer's two arrays with divisor `divisor`.
fn attributes(divisor: u32) -> [VertexAttribute; 3] {
    let instances = VertexBuffer { gpu: INSTANCES, size: 40 * 24 };
    [
        VertexAttribute {
            buffer: VertexBuffer { gpu: GRID, size: 8 * 8 * 16 },
            offset: 0,
            stride: 16,
            components: 4,
            instance_divisor: 0,
        },
        VertexAttribute { buffer: instances, offset: 0, stride: 24, components: 2, instance_divisor: divisor },
        VertexAttribute { buffer: instances, offset: 8, stride: 24, components: 4, instance_divisor: divisor },
    ]
}

/// Case 52's draw of `instances` instances: the stream's words and the
/// descriptor arena's bytes, or the builder's refusal.
fn build(instances: u32, attributes: &[VertexAttribute]) -> Result<(Vec<u64>, Vec<u8>), FrameError> {
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
    let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
    let mut spds = Arena::new("spds", 0x7fff_ffff_d000, &mut spd_m).unwrap();
    let bin = [0u8; 128];
    let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX)).unwrap();
    let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT)).unwrap();
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
        clear_rgba: Some([0.0, 0.0, 0.0, 1.0]),
        draw: Some(DrawCall {
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: Cull::None,
            vs: &vs,
            fs: &fs,
            primitive: Topology::Triangles,
            first_vertex: 0,
            vertex_count: 294,
            instance_count: instances,
            indices: Some(Indices { gpu: 0x7fff_fffd_f000 }),
            attributes,
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
            textures: &[],
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        }),
    };
    let mut desc = Arena::new("desc", 0x7fff_fffd_0000, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    build_frame(&mut desc, &mut cs, &ctx, &input)?;
    let n = cs.words().len();
    Ok((words[..n].to_vec(), desc_m))
}

/// Where `needle` starts in `hay`, at a 32-byte boundary (a descriptor's
/// alignment), if it does.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    (0..hay.len().saturating_sub(needle.len()) + 1).step_by(32).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Both draws' Attribute and Buffer descriptors are the board's, byte for
/// byte, and so are the counts the draw moves (INSTANCE_COUNT 8 and 40).
#[test]
fn per_instance_attributes_are_the_board_s() {
    for (draw, instances) in [("inst1", 8), ("inst2", 40)] {
        let m = trace::memory_of(INST, draw);
        let (words, desc) = build(instances, &attributes(1)).unwrap();
        let attrs = trace::bytes(&m, ATTRIBUTES, 3 * Attribute::LENGTH);
        let bufs = trace::bytes(&m, BUFFERS, 3 * Buffer::LENGTH);
        assert!(find(&desc, &attrs).is_some(), "{draw}: the three Attributes {attrs:02x?}");
        assert!(find(&desc, &bufs).is_some(), "{draw}: the three Buffers {bufs:02x?}");
        let theirs: Vec<u64> = trace::cs_words(INST, draw).iter().map(|w| w.word).collect();
        let (theirs, ours) = (regs_at_idvs(&theirs), regs_at_idvs(&words));
        assert_eq!(theirs[&R_INSTANCE_COUNT], instances, "{draw}: the dump's r34");
        for r in [33, R_INSTANCE_COUNT, 35, 36, 39] {
            assert_eq!(ours.get(&r), theirs.get(&r), "{draw}: r{r}");
        }
    }
}

/// What the dump settled, read off its words (docs/notes/mali-frame-formats.md
/// section 6): the per-instance pair are `1D POT Divisor` with `Frequency:
/// Instance`, `Offset enable` false and `Offset` 0, each its own Buffer
/// index; `icol`'s offset 8 is in its Buffer (base + 8, 960 - 8 bytes).
#[test]
fn the_dump_folds_an_instance_offset_into_the_buffer() {
    let m = trace::memory_of(INST, "inst1");
    let word = |va: u64| u32::from_le_bytes(trace::bytes(&m, va, 4).try_into().unwrap());
    for i in 0..3u64 {
        let a = ATTRIBUTES + 32 * i;
        let (attribute_type, offset_enable) = ((word(a) >> 4) & 0xf, (word(a) >> 8) & 1);
        let frequency = (word(a + 4) >> 6) & 3;
        let per_instance = i != 0;
        assert_eq!(
            AttributeType(attribute_type),
            if per_instance { AttributeType::_1D_POT_DIVISOR } else { AttributeType::_1D },
            "attribute {i}: its type"
        );
        assert_eq!(offset_enable, u32::from(!per_instance), "attribute {i}: Offset enable");
        assert_eq!(frequency, u32::from(per_instance), "attribute {i}: Frequency");
        assert_eq!(word(a + 4) >> 24, 0, "attribute {i}: Divisor R and E");
        assert_eq!((word(a + 8), word(a + 12)), (0, i as u32), "attribute {i}: Offset and Buffer index");
        assert_eq!(word(a + 20), 0, "attribute {i}: Divisor D");
    }
    let buffer = |i: u64| {
        let b = BUFFERS + 32 * i;
        (u64::from(word(b + 8)) | u64::from(word(b + 12)) << 32, word(b + 4))
    };
    assert_eq!(buffer(0), (GRID, 1024));
    assert_eq!(buffer(1), (INSTANCES, 960));
    assert_eq!(buffer(2), (INSTANCES + 8, 952));
}

/// A divisor other than 1 is refused by name, before any word: the dump
/// shows divisor 1 only.
#[test]
fn a_divisor_other_than_one_is_refused() {
    assert_eq!(build(8, &attributes(2)), Err(FrameError::NotBuilt(NOT_BUILT_INSTANCE_DIVISOR)));
    assert!(build(8, &attributes(1)).is_ok());
}
