// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A draw whose vertex shader reads no attribute (#11328): dEQP-GLES2's
//! smallest pair, `void main (void) { gl_Position = vec4(0.0); }` with
//! `gl_FragColor = vec4(1.0)`, which its negative_api, state_query and
//! shader_api cases draw (docs/es2-cts-failures.md M.1 rows 7, 10, 11).
//!
//! Mesa's choice (26.2.1): such a shader's vertex elements are empty, so
//! `panfrost_emit_resources` (gallium/drivers/panfrost/pan_cmdstream.h:
//! 296-301) makes the vertex stage's Attribute table with
//! `num_elements` 0 and its Attribute Buffer table with
//! `util_last_bit(vb_mask)` 0, and `pan_make_resource_table`
//! (panfrost/lib/pan_encoder.h:268-279) returns on a count of 0: both
//! Resource entries stay the zero the table was cleared to. The draw's
//! counts are the draw's (`csf_launch_draw`, pan_csf.c: INDEX_COUNT the
//! vertex count, INSTANCE_COUNT the instance count), whatever the
//! attributes. The builder builds the same: no Attribute and no Buffer
//! array, entries 1 and 2 of the vertex resource table zero, the counts as
//! given; the other entries as for a draw with an attribute.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --lib mali::frame_noattr_tests
//! ```

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;

/// The descriptor arena's base.
const DESC: u64 = 0x7fff_fffd_0000;
/// The vertex resource table's entries (`PAN_TABLE_ATTRIBUTE`,
/// `PAN_TABLE_ATTRIBUTE_BUFFER`, `PAN_TABLE_SAMPLER`; frame.rs TABLE_*).
const T_ATTRIBUTE: usize = 1;
const T_ATTRIBUTE_BUFFER: usize = 2;
const T_SAMPLER: usize = 3;
/// The tables a resource table holds (frame.rs RESOURCE_TABLES).
const TABLES: usize = 7;
/// INDEX_COUNT and INSTANCE_COUNT (frame.rs `reg`).
const R_INDEX_COUNT: u32 = 33;
const R_INSTANCE_COUNT: u32 = 34;

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

/// One triangle with `attributes`: the frame, the stream's words and the
/// descriptor arena's bytes, or the builder's refusal.
fn build(attributes: &[VertexAttribute]) -> Result<(Frame, Vec<u64>, Vec<u8>), FrameError> {
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
        tiler_bin_budget: (2 << 20) - 64,
        tile_budget: TileBudget::G610,
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
            width: 8,
            height: 8,
            row_stride: 32,
            format: TargetFormat::Rgba8888,
            samples: 1,
        },
        clear_rgba: Some([0.5, 0.25, 0.75, 0.4]),
        draw: Some(DrawCall {
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: Cull::None,
            vs: &vs,
            fs: &fs,
            primitive: Topology::Triangles,
            first_vertex: 0,
            vertex_count: 3,
            instance_count: 1,
            indices: None,
            attributes,
            viewport: Viewport { x: 0, y: 0, width: 8, height: 8, near: 0.0, far: 1.0 },
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
            vs_textures: &[],
            occlusion_query: None,
            alpha_to_coverage: false,
            blend_shader: None,
        }),
    };
    let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
    let mut cs = Stream::new(&mut words, 92);
    let frame = build_frame(&mut desc, &mut cs, &ctx, &input)?;
    let n = cs.words().len();
    Ok((frame, words[..n].to_vec(), desc_m))
}

/// The vertex resource table's entries, unpacked.
fn vs_table(frame: &Frame, desc: &[u8]) -> Vec<Resource> {
    let t = frame.layout.vs_resources.expect("the draw has a vertex resource table");
    let at = (t.gpu - DESC) as usize;
    (0..TABLES)
        .map(|i| {
            let o = at + i * Resource::LENGTH;
            Resource::unpack(desc[o..o + Resource::LENGTH].try_into().unwrap()).unwrap()
        })
        .collect()
}

/// The value the stream last moves into `reg` before its RUN_IDVS.
fn reg_at_idvs(words: &[u64], reg: u32) -> Option<u32> {
    let mut v = None;
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) if m.destination == reg => v = Some(m.immediate),
            Ok(Instr::Move48(m)) if m.destination == reg => v = Some(m.immediate as u32),
            Ok(Instr::RunIdvs(_)) => return v,
            _ => {}
        }
    }
    panic!("no RUN_IDVS in the stream")
}

/// An entry `pan_make_resource_table` did not write: all 16 bytes zero. A
/// written entry has `Contains descriptors` set, v10.xml's default (its
/// Resource struct, line 2073).
const ZERO: Resource = Resource { address: 0, contains_descriptors: false, size: 0 };

#[test]
fn a_draw_with_no_attribute_is_built() {
    let (frame, words, _) = build(&[]).expect("a draw with no vertex attribute is built");
    assert!(frame.layout.attributes.is_none(), "no Attribute array (Mesa uploads none for num_elements 0)");
    assert!(frame.layout.buffers.is_none(), "no Buffer array (Mesa's vb_mask is empty)");
    assert!(words.iter().any(|&w| matches!(Instr::decode(w), Ok(Instr::RunIdvs(_)))), "the draw runs");
}

#[test]
fn its_attribute_entries_are_zero_as_mesa_s() {
    let (frame, _, desc) = build(&[]).unwrap();
    let t = vs_table(&frame, &desc);
    assert_eq!(t[T_ATTRIBUTE], ZERO, "the Attribute table's Resource entry");
    assert_eq!(t[T_ATTRIBUTE_BUFFER], ZERO, "the Attribute Buffer table's Resource entry");
    /* at least the workaround sampler, as with attributes
     * (pan_cmdstream.h:286-287) */
    let s = frame.layout.vs_sampler.unwrap();
    assert_eq!(t[T_SAMPLER], Resource { address: s.gpu, contains_descriptors: true, size: 32 });
}

#[test]
fn its_counts_are_the_draw_s() {
    let (_, words, _) = build(&[]).unwrap();
    assert_eq!(reg_at_idvs(&words, R_INDEX_COUNT), Some(3), "INDEX_COUNT, the vertex count");
    assert_eq!(reg_at_idvs(&words, R_INSTANCE_COUNT), Some(1), "INSTANCE_COUNT");
}

#[test]
fn one_attribute_still_fills_both_entries() {
    let one = [VertexAttribute {
        buffer: VertexBuffer { gpu: 0x7fff_fffe_a000, size: 48 },
        offset: 0,
        stride: 16,
        components: 4,
        instance_divisor: 0,
    }];
    let (frame, _, desc) = build(&one).unwrap();
    let t = vs_table(&frame, &desc);
    let (a, b) = (frame.layout.attributes.unwrap(), frame.layout.buffers.unwrap());
    assert_eq!(t[T_ATTRIBUTE], Resource { address: a.gpu, contains_descriptors: true, size: 32 });
    assert_eq!(t[T_ATTRIBUTE_BUFFER], Resource { address: b.gpu, contains_descriptors: true, size: 32 });
}

#[test]
fn more_than_sixteen_and_other_widths_are_still_refused() {
    let a = VertexAttribute {
        buffer: VertexBuffer { gpu: 0x7fff_fffe_a000, size: 48 },
        offset: 0,
        stride: 16,
        components: 4,
        instance_divisor: 0,
    };
    for bad in [&[a; 17][..], &[VertexAttribute { components: 5, ..a }], &[VertexAttribute { components: 0, ..a }]] {
        match build(bad) {
            Err(FrameError::NotBuilt(why)) => assert_eq!(why, NOT_BUILT_ATTRIBUTES),
            r => panic!("{} attributes: {:?}", bad.len(), r.map(|_| ())),
        }
    }
}
