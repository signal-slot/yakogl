// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's indexed draws against the board: the draws Mesa
//! 25.0.7 made on the NanoPC-T6 for one red glDrawElements in five modes
//! into a 64x64 RGBA8 FBO (tools/mali/board-dump/indexed-2026-10-19.txt,
//! cut from 0dce654a8:tools/mali/board-ref/2026-10-19-indexed/mesa).
//!
//! What is compared, per mode: the registers an indexed draw moves (d54
//! the index buffer, r39 its bytes, r33 the index count, r35 the index
//! offset, r36 the vertex offset, r56 the tiler flags) and the RUN_IDVS
//! flags override (the draw mode and the index type), each built with the
//! index address Mesa's draw read (its pool copy of client indices, or the
//! element buffer at its offset). One difference is named: Mesa reads u8
//! indices as they are (override index type U8, r39 = count), and the GL
//! core widens them to u16 before the backend sees them
//! (`GlesDraw::index_size`), so the builder's u8 draw is a u16 draw of the
//! same indices.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const IDX: &str = include_str!("../../tools/mali/board-dump/indexed-2026-10-19.txt");

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

/// idx.c's frame, cleared blue with one draw of `vertex_count` vertices,
/// or indices with `indices`: the stream's words, or the builder's refusal.
fn build(
    primitive: Topology,
    first_vertex: u32,
    vertex_count: u32,
    indices: Option<Indices>,
) -> Result<Vec<u64>, FrameError> {
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
            vs: &vs,
            fs: &fs,
            primitive,
            first_vertex,
            vertex_count,
            instance_count: 1,
            indices,
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
            texture: None,
            vs_texture: None,
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        }),
    };
    let mut cs = Stream::new(&mut words, 92);
    build_frame(&mut desc, &mut cs, &ctx, &input)?;
    let n = cs.words().len();
    Ok(words[..n].to_vec())
}

/// The dump's registers at the draw and its flags override.
fn mesa(mode: &str) -> (BTreeMap<u32, u32>, u32) {
    let words: Vec<u64> = trace::cs_words(IDX, &format!("mesa-{mode}")).iter().map(|w| w.word).collect();
    regs_at_idvs(&words)
}

fn d54(r: &BTreeMap<u32, u32>) -> u64 {
    u64::from(r[&54]) | u64::from(r[&55]) << 32
}

/// Each u16 mode's indexed draw reads the board's index buffer, its bytes,
/// the count, the offsets, the tiler flags and the override.
#[test]
fn an_indexed_draw_is_the_board_s() {
    for (mode, primitive, count) in [
        ("u16_client", Topology::Triangles, 6),
        ("u16_ibo", Topology::Triangles, 6),
        ("u16_offset", Topology::Triangles, 6),
        ("strip_ibo", Topology::TriangleStrip, 4),
    ] {
        let (theirs, their_over) = mesa(mode);
        let at = d54(&theirs);
        let (ours, over) = regs_at_idvs(&build(primitive, 0, count, Some(Indices { gpu: at })).unwrap());
        assert_eq!(d54(&ours), at, "{mode}: d54 (the index buffer)");
        for r in [39, 33, 35, 36, 56] {
            assert_eq!(ours.get(&r), theirs.get(&r), "{mode}: r{r}");
        }
        assert_eq!(over, their_over, "{mode}: the flags override");
    }
    /* the dump holds what the test reads: u16 triangles' override #208
     * (index type U16 in bits 8-9), r39 = 2 * count, an element buffer
     * read at its offset (d54 ends in 6 for u16_offset) */
    let w = |m: &str| trace::cs_words(IDX, &format!("mesa-{m}"));
    assert!(w("u16_ibo").iter().any(|w| w.mnemonic.ends_with("r0, #208")));
    assert!(w("u16_ibo").iter().any(|w| w.mnemonic == "MOVE32 r39, #0xC"));
    assert_eq!(d54(&mesa("u16_offset").0) - d54(&mesa("u16_ibo").0), 6);
}

/// u8 indices: the one named difference. Mesa reads them as U8 (override
/// #108, r39 = 6); the core hands the backend the same indices widened to
/// u16, so ours is the u16 draw (#208, r39 = 12), the rest the same.
#[test]
fn u8_indices_are_drawn_as_the_core_widened_them() {
    let (theirs, their_over) = mesa("u8_ibo");
    let (ours, over) = regs_at_idvs(&build(Topology::Triangles, 0, 6, Some(Indices { gpu: d54(&theirs) })).unwrap());
    let index_type = |o: u32| (o >> 8) & 3;
    assert_eq!((index_type(their_over), theirs[&39]), (IndexType::UINT8.0, 6), "the dump");
    assert_eq!((index_type(over), ours[&39]), (IndexType::UINT16.0, 12), "ours");
    assert_eq!(over & !(3 << 8), their_over & !(3 << 8), "the override apart from the index type");
    for r in [33, 35, 36, 56] {
        assert_eq!(ours.get(&r), theirs.get(&r), "r{r}");
    }
}

/// An index address that is not 2-byte aligned and an indexed draw with a
/// first vertex are the caller's defects, refused before any word.
#[test]
fn an_odd_index_address_or_an_index_bias_is_invalid() {
    let at = Some(Indices { gpu: 0x7fff_ffec_0100 });
    assert_eq!(
        build(Topology::Triangles, 0, 6, at.map(|i| Indices { gpu: i.gpu + 1 })),
        Err(FrameError::Invalid(INVALID_INDEX_ALIGN))
    );
    assert_eq!(build(Topology::Triangles, 3, 6, at), Err(FrameError::Invalid(INVALID_INDEXED_FIRST_VERTEX)));
    assert!(build(Topology::Triangles, 0, 6, at).is_ok());
}
