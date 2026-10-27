// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The frame builder's uniform block for a fragment shader that loads it
//! (`LD_PKA`, a uniform array element read at a run-time index: gl_test
//! case 122 `uniidxi`, #1385 (b)) against the board: Mesa 25.0.7's draw of
//! case 122 on the NanoPC-T6 (tools/mali/board-dump/uniidxi-2026-10-20.txt,
//! cut from 0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/
//! mesa-uniidxi.pandecode), built again with the case's uniform values and
//! the record our back end makes of its fragment shader
//! (tools/mali/testdata/uniidx_i.expect.json: the FAU of the blend
//! constants, `fidx`, 0.5 and the handle; the block of `fcols` and `fidx`).
//!
//! What is compared: the UBO table the fragment resource table's entry 0
//! names, the Buffer descriptor of UBO 0, the block's 36 bytes, the
//! fragment FAU and the fragment shader program descriptor. Every byte
//! where ours may differ from the dump is listed with its cause, the dump
//! is checked to hold what the cause says, and every other byte is equal.

use super::arena::Arena;
use super::cs::{Instr, Stream};
use super::frame::*;
use super::genxml::v10::*;
use super::trace;
use std::collections::BTreeMap;

const DUMP: &str = include_str!("../../tools/mali/board-dump/uniidxi-2026-10-20.txt");

/* the trace's addresses: the descriptor buffer, the render target (the
 * window, drawn flipped), the texels and the heap */
const DESC: u64 = 0x7fff_fffc_f000;
const TARGET: u64 = 0x7fff_fffe_4000;
const TEXELS: u64 = 0x7fff_fffe_a000;
const SAMPLES: u64 = 0x7fff_ffff_f000;
const HEAP_DESC: u64 = 0x7fff_ffff_b000;
const GEOMETRY: u64 = 0x7fff_fffe_b000;
const SPDS: u64 = 0x7fff_ffff_d000;

/// The case's uniform storage: `vec4 fcols[4]` (dwords 0-15) then `int
/// fidx` (dword 16), one dword a component, as the GL core keeps it.
fn uniforms() -> Vec<u32> {
    let fcols: [[f32; 4]; 4] = [[0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 0.0, 1.0], [0.2, 0.4, 0.6, 1.0], [0.0, 1.0, 0.0, 1.0]];
    let mut v: Vec<u32> = fcols.iter().flatten().map(|f| f.to_bits()).collect();
    v.push(2);
    v
}

/// The block's rows as the record carries them (fp16 pairs an element, the
/// int one word): 36 bytes.
fn rows() -> Vec<UboRow> {
    let mut r: Vec<UboRow> = (0..4).map(|e| UboRow { dword: 4 * e, word: 2 * e, comps: 4, halves: true }).collect();
    r.push(UboRow { dword: 16, word: 8, comps: 1, halves: false });
    r
}

/// The fragment record's push words (uniidx_i.expect.json's FAU: the blend
/// constants, `fidx`, the promoted 0.5 and the texture handle).
const PUSH: [PushWord; 7] = [
    PushWord::Sysval { sysval: 0, component: 0 },
    PushWord::Sysval { sysval: 0, component: 1 },
    PushWord::Sysval { sysval: 0, component: 2 },
    PushWord::Sysval { sysval: 0, component: 3 },
    PushWord::Uniform { dword: 16 },
    PushWord::Constant(0x3f00_0000),
    PushWord::Constant(0x2000_1800),
];
const SYSVALS: [Sysval; 1] = [Sysval::BlendConstants];

fn meta(stage: ShaderStage, ubo: &'static [UboRow]) -> ShaderMeta<'static> {
    let fragment = stage == ShaderStage::FRAGMENT;
    ShaderMeta {
        stage,
        register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
        /* the trace's fragment shader: r60 and r61 ("R48-R63: 0x3000") */
        preload: if fragment { 0x3000 } else { 0x3800 },
        sysvals: if fragment { &SYSVALS } else { &[] },
        push: if fragment { &PUSH } else { &[] },
        varyings: 0,
        varying_bytes: 0,
        secondary: None,
        no_psiz_offset: None,
        colour_outputs: u32::from(fragment),
        samples_texture: fragment,
        /* "Requires helper threads: true": an implicit-LOD look-up */
        requires_helper_threads: fragment,
        can_discard: false,
        writes_depth: false,
        writes_stencil: false,
        writes_coverage: false,
        writes_global: false,
        ubo,
    }
}

/// The case's 1x1 opaque black texture, nearest, repeat (the GL defaults).
fn texture() -> SampledTexture {
    let mut level = [TextureLevel::default(); MAX_TEXTURE_LEVELS];
    level[0] = TextureLevel { gpu: TEXELS, row_stride: 64 };
    SampledTexture {
        width: 1,
        height: 1,
        level,
        levels: 1,
        faces: 1,
        format: Format::RGBA8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0,
        clump: ClumpFormat::RAW32,
        block_bytes: 0,
        swizzle: SWIZZLE_RGBA,
        min_nearest: true,
        mag_nearest: true,
        mipmap: MipmapMode::NONE,
        wrap_s: WrapMode::REPEAT,
        wrap_t: WrapMode::REPEAT,
    }
}

struct Built {
    desc: Vec<u8>,
    spds: Vec<u8>,
    words: Vec<u64>,
    layout: Layout,
    fs_spd: u64,
}

/// The case's draw: six vertices of a quad over the window with the
/// uniforms above, into a target at the trace's addresses. `ubo` is the
/// fragment record's block; `fs_uniforms` the draw's storage.
fn try_build(ubo: &'static [UboRow], fs_uniforms: &[u32]) -> Result<Built, FrameError> {
    let (mut desc_m, mut code_m, mut spd_m, mut ctx_m) =
        (vec![0u8; 0x4000], vec![0u8; 0x800], vec![0u8; 0x100], vec![0u8; 0x200]);
    let mut words = vec![0u64; 512];
    let (layout, fs_spd);
    {
        let mut ctx_a = Arena::new("context", HEAP_DESC, &mut ctx_m).unwrap();
        let heap = tiler_heap_descriptor(&mut ctx_a, 0x8000_00a0_0000, 2 << 20).unwrap();
        let ctx = Context {
            heap: heap.gpu,
            geometry_buffer: GEOMETRY,
            geometry_buffer_size: GEOMETRY_BUFFER_BYTES,
            sample_positions: SAMPLES,
            tiler_max_levels: 8,
        };
        let mut desc = Arena::new("desc", DESC, &mut desc_m).unwrap();
        let mut code = Arena::new("code", 0x7fff_ffff_c000, &mut code_m).unwrap();
        let mut spds = Arena::new("spds", SPDS, &mut spd_m).unwrap();
        let bin = [0u8; 128];
        let vs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::VERTEX, &[])).unwrap();
        let fs = upload_program(&mut code, &mut spds, &bin, meta(ShaderStage::FRAGMENT, ubo)).unwrap();
        fs_spd = fs.spd;
        let attributes = [VertexAttribute {
            buffer: VertexBuffer { gpu: 0x7fff_ffec_c000, size: 192 },
            offset: 0,
            stride: 32,
            components: 4,
            instance_divisor: 0,
        }];
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
            vs_uniforms: &[],
            fs_uniforms,
            zs: DepthStencilState::OFF,
            colour_mask: 0xf,
            front_cw: false,
            texture: Some(texture()),
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
        end_frame(&open, &mut desc, &mut cs, &ctx, &target, [0.0, 0.0, 1.0, 1.0], None).unwrap();
        let n = cs.words().len();
        words.truncate(n);
    }
    Ok(Built { desc: desc_m, spds: spd_m, words, layout, fs_spd })
}

/// A Buffer descriptor's `Size` (bits 32-63) and `Address` (64-127).
fn buffer_fields(b: &[u8]) -> (u32, u64) {
    (u32::from_le_bytes(b[4..8].try_into().unwrap()), u64::from_le_bytes(b[8..16].try_into().unwrap()))
}

/// The registers each RUN_IDVS of a stream reads (every MOVE32 and MOVE48
/// before it, per register).
fn regs_at_idvs(words: &[u64]) -> Vec<BTreeMap<u32, u32>> {
    let mut r = BTreeMap::new();
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => out.push(r.clone()),
            _ => {}
        }
    }
    out
}

/// The block's bytes as Mesa's storage has them: fp16 pairs, then fidx.
fn block_bytes() -> Vec<u8> {
    let u = uniforms();
    let mut b = Vec::new();
    for e in 0..4 {
        for k in 0..2 {
            let w = u32::from(f32_to_f16(u[4 * e + 2 * k])) | u32::from(f32_to_f16(u[4 * e + 2 * k + 1])) << 16;
            b.extend(w.to_le_bytes());
        }
    }
    b.extend(2u32.to_le_bytes());
    b
}

static ROWS: std::sync::LazyLock<Vec<UboRow>> = std::sync::LazyLock::new(rows);

/// Case 122's fragment descriptors against the board's: the UBO 0 Buffer
/// (36 bytes), the block's bytes (equal), the UBO table (one entry more:
/// 26.2.1's blend-constant sysval block, which 25.0.7's shader did not
/// have), the FAU (25.0.7 pushed `fidx` alone; 26.2.1 pushes the blend
/// constants first and promotes 0.5 and the handle, the shim's words) and
/// the fragment SPD (the binary's address).
#[test]
fn case_122_s_uniform_block_is_the_board_s() {
    let b = try_build(&ROWS, &uniforms()).expect("the draw builds");
    let m = trace::memory_of(DUMP, "uniidxi");
    let ours = |gpu: u64, len: usize| b.desc[(gpu - DESC) as usize..(gpu - DESC) as usize + len].to_vec();
    let mesa_regs = regs_at_idvs(&trace::cs_words(DUMP, "uniidxi").iter().map(|w| w.word).collect::<Vec<_>>());
    let our_regs = regs_at_idvs(&b.words);
    assert_eq!((mesa_regs.len(), our_regs.len()), (1, 1), "one RUN_IDVS each");
    let reg64 = |r: &BTreeMap<u32, u32>, n: u32| u64::from(r[&n]) | u64::from(r[&(n + 1)]) << 32;
    let (mr, or) = (&mesa_regs[0], &our_regs[0]);

    /* Mesa's blocks, from the fragment SRT (d4), FAU (d12) and SPD (d20) registers */
    let entry = |t: &[u8]| Resource::unpack(t[..16].try_into().unwrap()).unwrap();
    let m_table = trace::bytes(&m, reg64(mr, 4) & !63, Resource::LENGTH);
    let m_ubos = entry(&m_table);
    let m_buffer = trace::bytes(&m, m_ubos.address, Buffer::LENGTH);
    let m_ubo0 = buffer_fields(&m_buffer);
    let m_block = trace::bytes(&m, m_ubo0.1, m_ubo0.0 as usize);
    let m_fau_reg = reg64(mr, 12);
    let m_fau = trace::bytes(&m, m_fau_reg & 0xff_ffff_ffff_ffff, 8 * (m_fau_reg >> 56) as usize);
    let m_spd = trace::bytes(&m, reg64(mr, 20), ShaderProgram::LENGTH);

    /* ours */
    let o_table = ours(b.layout.fs_resources.expect("a fragment resource table").gpu, Resource::LENGTH);
    let o_ubos = entry(&o_table);
    let o_ubos_block = b.layout.fs_ubos.expect("a UBO table");
    assert_eq!(o_ubos.address, o_ubos_block.gpu);
    let o_buffer = ours(o_ubos.address, Buffer::LENGTH);
    let o_ubo0 = buffer_fields(&o_buffer);
    let o_block = ours(o_ubo0.1, o_ubo0.0 as usize);
    let o_fau_reg = reg64(or, 12);
    let o_fau = ours(o_fau_reg & 0xff_ffff_ffff_ffff, 8 * (o_fau_reg >> 56) as usize);
    let o_spd = b.spds[(b.fs_spd - SPDS) as usize..(b.fs_spd - SPDS) as usize + ShaderProgram::LENGTH].to_vec();

    /* the UBO 0 Buffer: 36 bytes, Mesa's at its own address */
    assert_eq!((m_ubo0.0, o_ubo0.0), (36, 36), "the exact packed bytes of fcols and fidx");
    let mut want = m_buffer.clone();
    want[8..16].copy_from_slice(&o_ubo0.1.to_le_bytes());
    assert_eq!(o_buffer, want, "the UBO 0 Buffer descriptor, but for the block's address");
    /* the block: equal, and what the case's values make */
    assert_eq!(o_block, m_block, "the block's bytes");
    assert_eq!(o_block, block_bytes());
    /* the UBO table: Mesa 25.0.7's one entry (its shader had no sysval
     * block), ours two: the block, then 26.2.1's blend-constant sysvals */
    assert_eq!((m_ubos.size, o_ubos.size), (0x20, 0x40), "the UBO table's descriptors");
    assert!(m_ubos.contains_descriptors && o_ubos.contains_descriptors);
    let o_sysval = buffer_fields(&ours(o_ubos.address + 32, Buffer::LENGTH));
    assert_eq!(o_sysval, (16, b.layout.fs_sysvals.expect("sysvals").gpu));
    /* the FAU: 25.0.7's `fidx` and a pad word; 26.2.1's blend constants,
     * `fidx`, 0.5 and the handle (the shim's words, uniidx_i.expect.json),
     * padded to a slot */
    let words = |b: &[u8]| b.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect::<Vec<_>>();
    assert_eq!(words(&m_fau), [2, 0]);
    assert_eq!(words(&o_fau), [0, 0, 0, 0, 2, 0x3f00_0000, 0x2000_1800, 0]);
    assert_eq!(words(&m_fau)[0], words(&o_fau)[4], "fidx, the one word both push");
    /* the fragment SPD: the binary's address alone */
    let mut want = m_spd.clone();
    want[8..16].copy_from_slice(&o_spd[8..16]);
    assert_eq!(o_spd, want, "the fragment SPD, but for the binary");
    assert_eq!(m_spd[8..16], 0x7fff_ffff_c080u64.to_le_bytes(), "the dump's binary");
}

/// A block row past the draw's uniform storage, or rows that do not fill
/// the block from word 0 up, are refused before any descriptor is written.
#[test]
fn a_block_row_outside_the_storage_is_refused() {
    /* a row past the 17 dwords (the pushed dword 16 itself is inside) */
    static PAST: [UboRow; 1] = [UboRow { dword: 14, word: 0, comps: 4, halves: true }];
    match try_build(&PAST, &uniforms()) {
        Err(FrameError::NotBuilt(s)) => assert_eq!(s, NOT_BUILT_UBO_BLOCK),
        other => panic!("{:?}", other.map(|_| ())),
    }
    static GAPPED: [UboRow; 1] = [UboRow { dword: 0, word: 1, comps: 4, halves: true }];
    match try_build(&GAPPED, &uniforms()) {
        Err(FrameError::NotBuilt(s)) => assert_eq!(s, NOT_BUILT_UBO_BLOCK),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

/// A fragment shader without a block keeps the one-entry table: the sysval
/// block at entry 0, as every frame before this change built it.
#[test]
fn a_shader_without_a_block_keeps_the_sysval_table() {
    let b = try_build(&[], &uniforms()).expect("the draw builds");
    let t = &b.desc[(b.layout.fs_resources.unwrap().gpu - DESC) as usize..][..Resource::LENGTH];
    let e = Resource::unpack(t.try_into().unwrap()).unwrap();
    assert_eq!(e.size, 0x20);
    let s = buffer_fields(&b.desc[(e.address - DESC) as usize..][..Buffer::LENGTH]);
    assert_eq!(s, (16, b.layout.fs_sysvals.unwrap().gpu));
}
