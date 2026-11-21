// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's UBO 0 for a uniform array, from hand-made block
//! rows (the compiler does not emit them yet, `lower-uniarray`): the bytes
//! Mesa 26.2.1 holds for the dEQP-GLES2 uniform-array shapes, as the
//! references pin them (docs/notes/mali-deqp-uniarray.md sections 5 and 7,
//! docs/notes/mali-uniform-arrays.md "The upload").
//!
//! - A plain `mediump` array is fp16, each element rounded up to whole
//!   dwords: float 1 dword, vec2 1, vec3 2, vec4 2 (the loads of
//!   `deqp_uniarray_{float,vec2,vec3,vec4}_const_{fs,vs}`: `16` at 0/4/8/12,
//!   `32x4` at 0, `16x3` at 0/8/16/24, `32x4` at 0 and 16); the GL storage
//!   keeps the packed fp32 dwords (1/2/3/4 an element), the frame converts.
//! - An array that is a struct member is fp32 at 1/2/3/4 dwords an element
//!   (`deqp_uniarray_struct_member_*`: `s.a` 0, `s.b[0..2]` 4-12, `s.c` 16,
//!   range 20; `deqp_uniarray_struct`: range 80, `32x4` loads).
//! - An array that does not start the block starts at the next free word
//!   (Mesa's `IADD_IMM #0x10` after four int index words).
//! - glUniform*v of fewer elements than the array leaves the storage tail
//!   zero (dEQP's `assign_partial`), so the block's tail is zero.
//!
//! Feature set: host-test,gpu-mali,shaderc-online,no-shader-table.

use super::arena::Arena;
use super::cs::Stream;
use super::frame::*;
use super::genxml::v10::*;

const DESC: u64 = 0x7fff_fffc_f000;
const TARGET: u64 = 0x7fff_fffe_4000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
const SPDS: u64 = 0x7fff_ffff_d000;

/// The fp16 bits of the integers 0 to 16 (IEEE binary16, written out).
const HALF: [u32; 17] = [
    0x0000, 0x3c00, 0x4000, 0x4200, 0x4400, 0x4500, 0x4600, 0x4700, 0x4800, 0x4880, 0x4900, 0x4980, 0x4a00, 0x4a80,
    0x4b00, 0x4b80, 0x4c00,
];

fn bits(v: u32) -> u32 {
    (v as f32).to_bits()
}

fn meta(stage: ShaderStage, ubo: &[UboRow]) -> ShaderMeta<'_> {
    let fragment = stage == ShaderStage::FRAGMENT;
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        preload: if fragment { 0x3000 } else { 0x3800 },
        sysvals: &[],
        push: &[],
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs: u32::from(fragment),
        samples_texture: false,
        requires_helper_threads: false,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo,
    }
}

/// A draw whose `stage` holds `ubo` rows over `uniforms` (the other stage
/// has no block and no storage), built into a frame: the block's bytes, or
/// the builder's refusal.
fn block_of(stage: ShaderStage, ubo: &[UboRow], uniforms: &[u32]) -> Result<Vec<u8>, FrameError> {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let layout;
    {
        let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: SAMPLES,
            tiler_max_levels: 8,
            tiler_bin_budget: (2 << 20) - 64,
            tile_budget: TileBudget::G610,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", SPDS, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let fragment = stage == ShaderStage::FRAGMENT;
        let (vs_ubo, fs_ubo): (&[UboRow], &[UboRow]) = if fragment { (&[], ubo) } else { (ubo, &[]) };
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX, vs_ubo)).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT, fs_ubo)).unwrap();
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 192 },
            offset: 0,
            stride: 32,
            components: 4,
            instance_divisor: 0,
        }];
        let none: &[u32] = &[];
        let draw = DrawCall {
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
            flip_y: true,
            blend_color: [0.0; 4],
            blend: None,
            polygon_offset: None,
            sample_mask: 0xf,
            vs_uniforms: if fragment { none } else { uniforms },
            fs_uniforms: if fragment { uniforms } else { none },
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            cull: Cull::None,
            textures: &[],
            vs_texture: None,
            occlusion_query: None,
            blend_shader: None,
            alpha_to_coverage: false,
        };
        let target =
            Target { gpu: TARGET, width: 64, height: 64, row_stride: 256, format: TargetFormat::Rgba8888, samples: 1 };
        let mut cs = Stream::new(&mut words, 92);
        let mut open = begin_frame(&mut desc, &mut cs).unwrap();
        add_draw(&mut open, &mut desc, &mut cs, &ctx, &target, false, &draw)?;
        layout = open.layout;
    }
    let ubos = if stage == ShaderStage::FRAGMENT { layout.fs_ubos } else { layout.vs_ubos };
    let table = ubos.expect("a UBO table");
    let at = |gpu: u64, len: usize| &desc_m[(gpu - DESC) as usize..(gpu - DESC) as usize + len];
    let buffer = at(table.gpu, Buffer::LENGTH);
    let size = u32::from_le_bytes(buffer[4..8].try_into().unwrap()) as usize;
    let address = u64::from_le_bytes(buffer[8..16].try_into().unwrap());
    Ok(at(address, size).to_vec())
}

fn le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The rows of `n` elements of `comps` components of a plain mediump array,
/// the first at storage dword `dword` and block word `word`: fp16, each
/// element `ceil(comps / 2)` words.
fn fp16_rows(n: usize, comps: usize, dword: usize, word: usize) -> Vec<UboRow> {
    (0..n)
        .map(|e| UboRow { dword: dword + e * comps, word: word + e * comps.div_ceil(2), comps, halves: true })
        .collect()
}

/// The fp16 words Mesa holds for `n` elements of `comps` components valued
/// 1, 2, 3, ... in order: each element's halves low component first, the
/// odd last half zero.
fn fp16_words(n: usize, comps: usize) -> Vec<u32> {
    let mut out = Vec::new();
    for e in 0..n {
        let h = |c: usize| if c < comps { HALF[1 + e * comps + c] } else { 0 };
        for k in 0..comps.div_ceil(2) {
            out.push(h(2 * k) | h(2 * k + 1) << 16);
        }
    }
    out
}

/// `n` elements of `comps` components valued 1, 2, 3, ..., packed.
fn array_storage(n: usize, comps: usize) -> Vec<u32> {
    (0..n * comps).map(|i| bits(1 + i as u32)).collect()
}

/// `uniform mediump {float,vec2,vec3,vec4} u[4]` first in the block, in
/// both stages: 4, 4, 8 and 8 bytes an element, as the references load
/// them (`deqp_uniarray_*_const_*`).
#[test]
fn a_mediump_array_is_fp16_at_mesas_strides() {
    for stage in [ShaderStage::FRAGMENT, ShaderStage::VERTEX] {
        for (comps, stride) in [(1, 4), (2, 4), (3, 8), (4, 8)] {
            let got = block_of(stage, &fp16_rows(4, comps, 0, 0), &array_storage(4, comps)).expect("the draw builds");
            assert_eq!(got.len(), 4 * stride, "{stage:?} comps {comps}: the block's range");
            assert_eq!(got, le(&fp16_words(4, comps)), "{stage:?} comps {comps}");
        }
    }
}

/// Spelled out for the shapes with an odd component count: a vec3 array
/// (the odd half is zero) and a float array (one half in a dword).
#[test]
fn the_odd_half_of_an_element_is_zero() {
    let got = block_of(ShaderStage::FRAGMENT, &fp16_rows(4, 3, 0, 0), &array_storage(4, 3)).unwrap();
    let want = [
        0x4000 << 16 | 0x3c00,
        0x4200,
        0x4500 << 16 | 0x4400,
        0x4600,
        0x4800 << 16 | 0x4700,
        0x4880,
        0x4980 << 16 | 0x4900,
        0x4a00,
    ];
    assert_eq!(got, le(&want));
    let got = block_of(ShaderStage::FRAGMENT, &fp16_rows(4, 1, 0, 0), &array_storage(4, 1)).unwrap();
    assert_eq!(got, le(&[0x3c00, 0x4000, 0x4200, 0x4400]));
}

/// The array behind four int index words (`uniarray_highp`'s shape and the
/// note's `#0x10`): the ints are words 0-3, the array starts at word 4,
/// byte 16, whatever its element type.
#[test]
fn an_array_behind_four_ints_starts_at_byte_16() {
    for stage in [ShaderStage::FRAGMENT, ShaderStage::VERTEX] {
        for comps in 1..=4 {
            let mut rows: Vec<UboRow> = (0..4).map(|i| UboRow { dword: i, word: i, comps: 1, halves: false }).collect();
            rows.extend(fp16_rows(4, comps, 4, 4));
            let mut storage = vec![0, 1, 2, 3];
            storage.extend(array_storage(4, comps));
            let got = block_of(stage, &rows, &storage).expect("the draw builds");
            let mut want = vec![0, 1, 2, 3];
            want.extend(fp16_words(4, comps));
            assert_eq!(got, le(&want), "{stage:?} comps {comps}");
            assert_eq!(got.len(), 16 + 4 * if comps <= 2 { 4 } else { 8 });
        }
    }
}

/// An array that is a struct member stays fp32, one dword a component, no
/// alignment: `struct { float a[2]; vec4 b[2]; }` is 2 + 8 dwords, one row
/// of whole words an element, equal to the storage word for word
/// (`deqp_uniarray_struct`, `deqp_uniarray_struct_member_*`).
#[test]
fn a_struct_member_array_is_fp32_at_one_to_four_dwords() {
    for stage in [ShaderStage::FRAGMENT, ShaderStage::VERTEX] {
        let rows = [
            UboRow { dword: 0, word: 0, comps: 1, halves: false },
            UboRow { dword: 1, word: 1, comps: 1, halves: false },
            UboRow { dword: 2, word: 2, comps: 4, halves: false },
            UboRow { dword: 6, word: 6, comps: 4, halves: false },
        ];
        let storage = array_storage(10, 1);
        let got = block_of(stage, &rows, &storage).expect("the draw builds");
        assert_eq!(got, le(&storage), "{stage:?}");
        /* a vec3 array member: 3 dwords an element, 4 elements, 12 words */
        let rows: Vec<UboRow> = (0..4).map(|e| UboRow { dword: 3 * e, word: 3 * e, comps: 3, halves: false }).collect();
        let storage = array_storage(4, 3);
        assert_eq!(block_of(stage, &rows, &storage).unwrap(), le(&storage), "{stage:?} vec3");
    }
}

/// glUniform2fv of 2 elements into `vec2 u[4]` (and the other types): the
/// GL core leaves storage dwords of the untouched elements zero, and the
/// block holds zero halves there, the block's range still the whole array.
#[test]
fn a_partial_assignment_leaves_the_tail_zero() {
    for (comps, set) in [(1, 2), (2, 2), (3, 1), (4, 3)] {
        let mut storage = array_storage(4, comps);
        for d in &mut storage[set * comps..] {
            *d = 0;
        }
        let got = block_of(ShaderStage::FRAGMENT, &fp16_rows(4, comps, 0, 0), &storage).unwrap();
        let all = fp16_words(4, comps);
        let per = comps.div_ceil(2);
        let mut want = all[..set * per].to_vec();
        want.resize(all.len(), 0);
        assert_eq!(got, le(&want), "comps {comps}, {set} of 4 set");
        assert!(got[set * per * 4..].iter().all(|&b| b == 0), "comps {comps}: the tail");
    }
    /* an fp32 struct member's array too */
    let rows: Vec<UboRow> = (0..2).map(|e| UboRow { dword: 4 * e, word: 4 * e, comps: 4, halves: false }).collect();
    let mut storage = array_storage(2, 4);
    storage[4..].fill(0);
    let got = block_of(ShaderStage::VERTEX, &rows, &storage).unwrap();
    assert_eq!(got, le(&storage));
}

/// Rows naming dwords past the storage keep their refusal, whatever the
/// stride: an array longer than what the draw hands over, a row that does
/// not start where the last ended, and a block row of either stage.
#[test]
fn rows_past_the_storage_are_refused() {
    let is_refused = |r: Result<Vec<u8>, FrameError>, why: &str| match r {
        Err(FrameError::NotBuilt(s)) => assert_eq!(s, NOT_BUILT_UBO_BLOCK),
        other => panic!("{why}: {:?}", other),
    };
    for stage in [ShaderStage::FRAGMENT, ShaderStage::VERTEX] {
        /* 4 vec3 elements over 11 dwords: the last element's dwords 9-11 */
        is_refused(block_of(stage, &fp16_rows(4, 3, 0, 0), &array_storage(4, 3)[..11]), "vec3 past 11 dwords");
        /* a fp32 vec4 element at dword 8 of 10 */
        let rows = [UboRow { dword: 8, word: 0, comps: 4, halves: false }];
        is_refused(block_of(stage, &rows, &array_storage(10, 1)), "vec4 at dword 8 of 10");
        /* no storage at all */
        is_refused(block_of(stage, &fp16_rows(1, 1, 0, 0), &[]), "an empty storage");
        /* a gap in the block (an element placed one word late) */
        let mut rows = fp16_rows(2, 2, 0, 0);
        rows[1].word += 1;
        is_refused(block_of(stage, &rows, &array_storage(2, 2)), "a gap");
    }
}
