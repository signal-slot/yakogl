// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The draws `vertex_attributes` refused as `NOT_BUILT_ATTRIBUTE_RECORDS`
//! (src/mali/backend.rs), built as Mesa 26.2.1 builds them:
//!
//! - a location the vertex shader reads that the GL core staged no
//!   attribute for: Mesa gives every read input a vertex element, the
//!   current value's when no array is enabled (`st_setup_current`,
//!   mesa/state_tracker/st_atom_array.cpp:371-385: four floats,
//!   `_mesa_draw_current_attrib`, at stride 0 from a user buffer of their
//!   own). An attribute never given a value holds GL's initial (0, 0, 0, 1).
//!   Its ATTRIBUTE is `panfrost_pack_attribute`'s
//!   (gallium/drivers/panfrost/pan_cmdstream.c:4084-4117): table
//!   ATTRIBUTE_BUFFER, frequency VERTEX, 1D with offset enable, format
//!   R32G32B32A32_FLOAT (RGBA32F RGBA, pan_format.c:454), offset 0, stride
//!   0; its Buffer is `panfrost_emit_vertex_buffers`'s (:938-971), Simple,
//!   the 16 bytes;
//! - a location whose array is disabled, the current value the GL core
//!   staged at stride 0 with the input's component count (two floats' worth
//!   of RG32F for a vec2): the same element, RGBA32F at stride 0 from 16
//!   bytes holding the staged floats, the components the core staged no
//!   bytes for GL's (0, 0, 0, 1) (`st_setup_current` uploads the whole
//!   four-float `_mesa_draw_current_attrib`); arrays stay as they are;
//! - a record below the highest that no location names: the table is
//!   indexed by record (Mesa's `attributes_read_count` entries,
//!   pan_cmdstream.c:2522-2548), so the slot takes the same element and no
//!   code fetches it.
//!
//! Two locations naming one record is no shape the compiler makes (a
//! record is the count of read locations below, Mesa's
//! `util_bitcount64(inputs_read & BITFIELD_MASK(loc))`): it is refused as
//! `INVALID_ATTRIBUTE_RECORDS`. More than 16 records stay
//! `NOT_BUILT_ATTRIBUTES`.
//!
//! The descriptor words are checked against v10.xml's layout (Mesa 26.2.1
//! src/panfrost/genxml/v10.xml:947-965 Attribute, :1056-1060 Buffer), not
//! through our own unpacker.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --lib mali::frame_attrrec_tests
//! ```

use super::arena::{Arena, Pool};
use super::backend::tests::{triangle, VERTICES};
use super::backend::{vertex_attributes, INITIAL_CURRENT_VALUE, INVALID_ATTRIBUTE_RECORDS};
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use crate::backend::{GlesAttrib, GlesDraw, GLES_ATTRIBF_FLOAT32};

/// The data pool's and the descriptor arena's bases.
const DATA: u64 = 0x7fff_fff0_0000;
const DESC: u64 = 0x7fff_fffd_0000;
/// Where the draw's one staged attribute lives on the GPU (`triangle`
/// stages it from the CPU; the pool copies it to DATA first).
const TARGET: u64 = 0x7fff_fffe_4000;

const NONE: VertexAttribute = VertexAttribute {
    buffer: VertexBuffer { gpu: 0, size: 0 },
    offset: 0,
    stride: 0,
    components: 0,
    instance_divisor: 0,
};

/// `vertex_attributes` for `records` over the board's triangle (one vec2
/// staged at `vi_reg` 0): the attributes, or the refusal, and the data
/// pool's bytes.
fn attributes(d: &GlesDraw, records: &[(u8, u8)]) -> (Result<Vec<VertexAttribute>, FrameError>, Vec<u8>) {
    let mut mem = vec![0u8; 0x1000];
    let mut out = [NONE; 16];
    let r = {
        let mut data = Pool::new("data", DATA, &mut mem).unwrap();
        vertex_attributes(d, records, &mut data, &mut out).map(|n| out[..n].to_vec())
    };
    (r, mem)
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

/// One triangle with `attributes`: the frame, whether the stream runs the
/// draw, and the descriptor arena's bytes.
fn build(attributes: &[VertexAttribute]) -> (Frame, bool, Vec<u8>) {
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
        target: Target { gpu: TARGET, width: 8, height: 8, row_stride: 32, format: TargetFormat::Rgba8888, samples: 1 },
        clear_rgba: Some([0.0, 0.0, 1.0, 1.0]),
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
    let frame = build_frame(&mut desc, &mut cs, &ctx, &input).expect("the draw is built");
    let n = cs.words().len();
    let runs = words[..n].iter().any(|&w| matches!(Instr::decode(w), Ok(Instr::RunIdvs(_))));
    (frame, runs, desc_m)
}

/// The eight 32-bit words of descriptor `i` of the array at `block`.
fn words8(desc: &[u8], block: u64, i: usize) -> [u32; 8] {
    let at = (block - DESC) as usize + 32 * i;
    core::array::from_fn(|w| u32::from_le_bytes(desc[at + 4 * w..at + 4 * w + 4].try_into().unwrap()))
}

/* v10.xml's values: Descriptor Type Attribute 5, Buffer 9; Attribute Type
 * 1D 1; Buffer Type Simple 1; Attribute Frequency Vertex 0; table
 * ATTRIBUTE_BUFFER 2 (PAN_TABLE_ATTRIBUTE_BUFFER); Pixel Format RGBA32F
 * 0xbf and RG32F 0xaf, component order RGBA 0 and RGB1 0x10 */
const ATTR_RGBA32F_1D: u32 = 5 | 1 << 4 | 1 << 8 | ((0xbf << 12) << 10);
const ATTR_RG32F_1D: u32 = 5 | 1 << 4 | 1 << 8 | ((0xaf << 12 | 0x10) << 10);
const BUFFER_SIMPLE: u32 = 9 | 1 << 4;

/// The current-value element's Attribute and Buffer words, for slot `i` of
/// a buffer at `gpu`.
fn current_value_words(i: u32, gpu: u64) -> ([u32; 8], [u32; 8]) {
    ([ATTR_RGBA32F_1D, 2, 0, i, 0, 0, 0, 0], [BUFFER_SIMPLE, 16, gpu as u32, (gpu >> 32) as u32, 0, 0, 0, 0])
}

/// The hand-written formats above against our genxml (Mesa's v10.xml:230,
/// :244, :295, :301: RG32F 175, RGBA32F 191, RGBA 0, RGB1 16).
#[test]
fn the_hand_written_formats_are_the_genxml_s() {
    assert_eq!(Format::RGBA32F.0 << 12 | RgbComponentOrder::RGBA.0, 0xbf << 12);
    assert_eq!(Format::RG32F.0 << 12 | RgbComponentOrder::RGB1.0, 0xaf << 12 | 0x10);
}

#[test]
fn a_location_the_core_did_not_stage_reads_the_initial_current_value() {
    let d = triangle(TARGET, &VERTICES);
    /* location 0, staged, is record 0; location 3, not staged, record 1 */
    let (got, mem) = attributes(&d, &[(0, 0), (3, 1)]);
    let got = got.expect("the draw's attributes are built");
    assert_eq!(got.len(), 2);
    let (a, c) = (got[0], got[1]);
    assert_eq!((a.components, a.stride, a.buffer.size), (2, 8, 48), "the staged vec2");
    assert_eq!((c.components, c.stride, c.offset, c.instance_divisor, c.buffer.size), (4, 0, 0, 0, 16));
    let at = (c.buffer.gpu - DATA) as usize;
    let value: Vec<f32> = mem[at..at + 16].as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect();
    assert_eq!(value, INITIAL_CURRENT_VALUE, "GL's initial current value, four floats");

    let (frame, runs, desc) = build(&got);
    assert!(runs, "the draw runs");
    let (attrs, bufs) = (frame.layout.attributes.unwrap(), frame.layout.buffers.unwrap());
    assert_eq!(words8(&desc, attrs.gpu, 0), [ATTR_RG32F_1D, 2, 0, 0, 8, 0, 0, 0], "the staged attribute");
    let (want_a, want_b) = current_value_words(1, c.buffer.gpu);
    assert_eq!(words8(&desc, attrs.gpu, 1), want_a, "the current-value Attribute");
    assert_eq!(words8(&desc, bufs.gpu, 1), want_b, "the current-value Buffer");
}

#[test]
fn a_record_no_location_names_takes_the_current_value_element() {
    let d = triangle(TARGET, &VERTICES);
    /* records 0 and 2, record 1 named by no location; two unstaged
     * locations share the one upload */
    let (got, _) = attributes(&d, &[(0, 0), (5, 2), (6, 3)]);
    let got = got.expect("the draw's attributes are built");
    assert_eq!(got.len(), 4, "one past the highest record");
    for i in 1..4 {
        assert_eq!((got[i].components, got[i].stride, got[i].buffer.size), (4, 0, 16), "slot {i}");
        assert_eq!(got[i].buffer.gpu, got[1].buffer.gpu, "slot {i}: one upload for the draw");
    }
    let (frame, runs, desc) = build(&got);
    assert!(runs);
    let (attrs, bufs) = (frame.layout.attributes.unwrap(), frame.layout.buffers.unwrap());
    for i in 1..4 {
        let (want_a, want_b) = current_value_words(i as u32, got[1].buffer.gpu);
        assert_eq!(words8(&desc, attrs.gpu, i), want_a, "Attribute {i}");
        assert_eq!(words8(&desc, bufs.gpu, i), want_b, "Buffer {i}");
    }
}

#[test]
fn a_staged_draw_is_unchanged() {
    let d = triangle(TARGET, &VERTICES);
    let (got, mem) = attributes(&d, &[(0, 0)]);
    let got = got.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].buffer.gpu, got[0].buffer.size, got[0].stride, got[0].components), (DATA, 48, 8, 2));
    assert!(mem[48..].iter().all(|&b| b == 0), "no current value is uploaded");
}

#[test]
fn two_locations_of_one_record_and_more_than_sixteen_are_refused() {
    let d = triangle(TARGET, &VERTICES);
    match attributes(&d, &[(0, 0), (1, 0)]).0 {
        Err(FrameError::Invalid(why)) => assert_eq!(why, INVALID_ATTRIBUTE_RECORDS),
        r => panic!("two locations of record 0: {:?}", r.map(|v| v.len())),
    }
    match attributes(&d, &[(0, 0), (1, 16)]).0 {
        Err(FrameError::NotBuilt(why)) => assert_eq!(why, NOT_BUILT_ATTRIBUTES),
        r => panic!("record 16: {:?}", r.map(|v| v.len())),
    }
}

/// `d` with a second attribute at `vi_reg` 1: a disabled vec2 array's
/// current value as the core stages it, stride 0 and `bytes` its staging.
fn with_constant(mut d: GlesDraw, bytes: &[f32]) -> GlesDraw {
    d.num_attribs = 2;
    d.attribs[1] = GlesAttrib {
        vi_reg: 1,
        components: 2,
        format: GLES_ATTRIBF_FLOAT32,
        stride: 0,
        cpu: bytes.as_ptr().cast(),
        cpu_bytes: 4 * bytes.len() as u32,
        ..GlesAttrib::ZERO
    };
    d
}

/// The current value the frame uploaded for `a`, as floats.
fn value_of(mem: &[u8], a: &VertexAttribute) -> Vec<f32> {
    let at = (a.buffer.gpu - DATA) as usize;
    mem[at..at + 16].as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect()
}

#[test]
fn a_staged_vec2_current_value_is_mesas_rgba32f_element() {
    /* the core's staging at the tip: the whole current value, 16 bytes,
     * labelled with the vec2's two components */
    static WHOLE: [f32; 4] = [0.5, 0.25, 0.0, 1.0];
    let d = with_constant(triangle(TARGET, &VERTICES), &WHOLE);
    let (got, mem) = attributes(&d, &[(0, 0), (1, 1)]);
    let got = got.expect("the draw's attributes are built");
    assert_eq!(got.len(), 2);
    let c = got[1];
    assert_eq!((c.components, c.stride, c.offset, c.instance_divisor, c.buffer.size), (4, 0, 0, 0, 16));
    assert_eq!(value_of(&mem, &c), WHOLE, "the staged value, four floats");

    let (frame, runs, desc) = build(&got);
    assert!(runs, "the draw runs");
    let (attrs, bufs) = (frame.layout.attributes.unwrap(), frame.layout.buffers.unwrap());
    assert_eq!(words8(&desc, attrs.gpu, 0), [ATTR_RG32F_1D, 2, 0, 0, 8, 0, 0, 0], "the staged array is unchanged");
    let (want_a, want_b) = current_value_words(1, c.buffer.gpu);
    assert_eq!(words8(&desc, attrs.gpu, 1), want_a, "Mesa's RGBA32F Attribute (word 0 0x2fc00115), stride 0");
    assert_eq!(want_a[0], 0x2fc0_0115);
    assert_eq!(words8(&desc, bufs.gpu, 1), want_b, "Mesa's Simple Buffer of 16 bytes");
}

#[test]
fn a_shorter_staged_current_value_is_padded_with_0_0_0_1() {
    /* two floats only: the missing z and w are GL's 0 and 1 */
    static TWO: [f32; 2] = [0.5, 0.25];
    let d = with_constant(triangle(TARGET, &VERTICES), &TWO);
    let (got, mem) = attributes(&d, &[(0, 0), (1, 1)]);
    let got = got.expect("the draw's attributes are built");
    let c = got[1];
    assert_eq!((c.components, c.stride, c.buffer.size), (4, 0, 16));
    assert_eq!(value_of(&mem, &c), [0.5, 0.25, 0.0, 1.0]);
    let (frame, _, desc) = build(&got);
    let attrs = frame.layout.attributes.unwrap();
    assert_eq!(words8(&desc, attrs.gpu, 1), current_value_words(1, c.buffer.gpu).0);
}

#[test]
fn a_per_instance_array_at_stride_0_is_not_a_current_value() {
    static ONE: [f32; 2] = [0.5, 0.25];
    let mut d = with_constant(triangle(TARGET, &VERTICES), &ONE);
    d.attribs[1].per_instance = 1;
    d.attribs[1].divisor = 1;
    let (got, _) = attributes(&d, &[(0, 0), (1, 1)]);
    let c = got.expect("the draw's attributes are built")[1];
    assert_eq!((c.components, c.stride, c.buffer.size, c.instance_divisor), (2, 0, 8, 1), "the array as staged");
}
