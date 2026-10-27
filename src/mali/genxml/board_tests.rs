// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Descriptors Mesa wrote on the board, packed again from their decoded
//! values: the bytes must be the board's.
//!
//! The source is tools/mali/board-dump/tri-2026-10-18.txt, an unchanged
//! excerpt of a `PAN_MESA_DEBUG=dump,trace,sync` run of one GLES2 triangle
//! on the NanoPC-T6 (Mesa 25.0.7 on Armbian; the excerpt's header has the
//! run). Each case below names the decoded block it transcribes (`@<va>`, the
//! excerpt's line range); the raw bytes are read from the excerpt's
//! hexdump, never typed in. Mesa 25.0.7 is the board's version, not the
//! 26.2.1 the packers are generated from: a field the two versions lay out
//! differently would show here as a difference, and none does for these
//! eight descriptors.
//!
//! A field that shares bits with another (pandecode prints every member of
//! a union from the same bits) is set once: the member the descriptor's
//! kind uses, the other left zero. The position shader is a vertex shader,
//! so its bit 8 is `Vertex warp limit` (Half), not the fragment-only
//! `Fragment coverage bitmask type` that pandecode prints beside it; its
//! preload is the `R48-R63` mask, whose bits pandecode also prints as the
//! one-register flags.

use super::v10::*;
use crate::mali::trace::{bytes as board_bytes, memory};

const EXCERPT: &str = include_str!("../../../tools/mali/board-dump/tri-2026-10-18.txt");
const LINEAR_EXCERPT: &str = include_str!("../../../tools/mali/board-dump/tri-linear-2026-10-18.txt");

#[test]
fn mesa_s_descriptors_from_the_board_pack_to_the_board_s_bytes() {
    let mem = memory(EXCERPT);
    let cases: [(&str, u64, Vec<u8>); 8] = [
        /* lines 85-92: Buffer @7ffffffd31e0 */
        (
            "Buffer @7ffffffd31e0",
            0x7fff_fffd_31e0,
            Buffer { size: 32, address: 0x7fff_fffd_31c0, ..Default::default() }.pack().expect("packs").to_vec(),
        ),
        /* lines 116-123: Buffer @7ffffffd3180 */
        (
            "Buffer @7ffffffd3180",
            0x7fff_fffd_3180,
            Buffer { size: 1_048_576, address: 0x7fff_ffec_b000, ..Default::default() }.pack().expect("packs").to_vec(),
        ),
        /* lines 97-111: Attribute @7ffffffd3160 (Format (v7): RG32F RGB1 is
         * Format RG32F 175 << 12 | RGB1 16) */
        (
            "Attribute @7ffffffd3160",
            0x7fff_fffd_3160,
            Attribute {
                attribute_type: AttributeType::_1D,
                offset_enable: true,
                format: (175 << 12) | 16,
                table: 2,
                frequency: AttributeFrequency::VERTEX,
                stride: 8,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
        ),
        /* lines 128-153: Sampler @7ffffffd31a0 */
        (
            "Sampler @7ffffffd31a0",
            0x7fff_fffd_31a0,
            Sampler {
                round_to_nearest_even: true,
                seamless_cube_map: true,
                normalized_coordinates: true,
                clamp_integer_array_indices: true,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
        ),
        /* lines 317-339: Position shader Shader @7fffffffd020 */
        (
            "Shader Program @7fffffffd020",
            0x7fff_ffff_d020,
            ShaderProgram {
                stage: ShaderStage::VERTEX,
                vertex_warp_limit: WarpLimit::HALF,
                register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
                preload: Preload { r48_r63: 0x3800, ..Default::default() },
                binary: 0x7fff_ffff_c000,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
        ),
        /* lines 352-374: Fragment shader Shader @7fffffffd040 */
        (
            "Shader Program @7fffffffd040",
            0x7fff_ffff_d040,
            ShaderProgram {
                stage: ShaderStage::FRAGMENT,
                fragment_coverage_bitmask_type: CoverageBitmaskType::GL,
                register_allocation: ShaderRegisterAllocation::_32_PER_THREAD,
                preload: Preload { r48_r63: 0x3000, ..Default::default() },
                binary: 0x7fff_ffff_c080,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
        ),
        /* lines 388-395: Position Local Storage @7ffffffd3100: every field
         * at Mesa's default (WLS Instances 2147483648, NO_WORKGROUP_MEM) */
        ("Local Storage @7ffffffd3100", 0x7fff_fffd_3100, LocalStorage::default().pack().expect("packs").to_vec()),
        /* lines 427-450: Tiler Context @7ffffffd32c0 */
        (
            "Tiler Context @7ffffffd32c0",
            0x7fff_fffd_32c0,
            TilerContext {
                polygon_list: 0x2_8000_00bf_ff00,
                hierarchy_mask: 254,
                fb_width: 64,
                fb_height: 64,
                layer_count: 1,
                heap: 0x7fff_ffff_b000,
                geometry_buffer_size: 65536,
                geometry_buffer: 0x7fff_fffe_b000,
                private_state: TilerState { word0: 1, ..Default::default() },
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
        ),
    ];
    let mut differ = Vec::new();
    for (name, va, ours) in &cases {
        let board = board_bytes(&mem, *va, ours.len());
        if &board != ours {
            differ.push(format!("{name}:\n  board {board:02x?}\n  ours  {ours:02x?}"));
        }
    }
    eprintln!("mali board descriptors: {} run (planned 8), {} differing", cases.len(), differ.len());
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}

/// The excerpt parser on a known row: the shader descriptor's first word,
/// and a row inside a `*` run (0x300 of the 7ffffffd3000 buffer, all zero
/// between the printed 0x2f0 and 0x320 rows).
#[test]
fn the_excerpt_reads_back_as_printed() {
    let mem = memory(EXCERPT);
    assert_eq!(board_bytes(&mem, 0x7fff_ffff_d020, 4), [0x38, 0x01, 0x00, 0x80]);
    assert_eq!(board_bytes(&mem, 0x7fff_fffd_3300, 16), [0u8; 16]);
    assert_eq!(board_bytes(&mem, 0x7fff_fffd_3320, 1), [0x01]);
}

/// A case: its name, where the board's bytes are in the dump's memory (or
/// `None`), the packed bytes, and the command-stream word to compare with.
type Case<'a> = (&'a str, Option<u64>, Vec<u8>, Vec<u8>);

/// The 32-bit immediate of the excerpt's `MOVE32 r<reg>` command-stream
/// instruction, from its raw bytes (the first four, little-endian), after
/// checking them against the value the disassembly prints beside them.
fn move32(excerpt: &str, reg: u8) -> [u8; 4] {
    let tail = format!("MOVE32 r{reg}, #0x");
    let line = excerpt.lines().find(|l| l.contains(&tail)).unwrap_or_else(|| panic!("no MOVE32 r{reg} in the excerpt"));
    let (hex, text) = line.split_once("MOVE32").expect("a MOVE32 line");
    let bytes: Vec<u8> = hex.split_whitespace().map(|h| u8::from_str_radix(h, 16).expect("a hex byte")).collect();
    assert_eq!(bytes.len(), 8, "{line}");
    assert_eq!(bytes[6], reg, "the instruction's destination register byte: {line}");
    let printed = u32::from_str_radix(text.rsplit("#0x").next().expect("an immediate"), 16).expect("hex");
    let raw = [bytes[0], bytes[1], bytes[2], bytes[3]];
    assert_eq!(u32::from_le_bytes(raw), printed, "bytes against the disassembly: {line}");
    raw
}

/// The linear-layout run of the same triangle (tri-linear-2026-10-18.txt):
/// the descriptors the first excerpt does not have, packed from the values
/// Mesa 25.0.7 printed, against the board's bytes.
///
/// The trace is Mesa 25.0.7 and the packers come from Mesa 26.2.1's v10.xml;
/// a field whose meaning moved between the two would show as a difference,
/// and none does for these descriptors. Two fields were renamed, at the same
/// bits, and the trace pins only one of them:
///   - DCD Flags 0 bit 13: "Single-sampled lines" (25.0.7) is "Aligned
///     line-ends" (26.2.1). The board sets it (0x21AB has bit 13); the
///     packed word equals the board's with `aligned_line_ends`, so the bit
///     is the same, and what the name means for the hardware is not settled
///     by this trace.
///   - Primitive flags bit 14: "Primitive Index Writeback" (25.0.7) is
///     "Primitive Index Override" (26.2.1). The board has it clear, so the
///     trace says nothing about the bit; it is not asserted to be the same.
///
/// The design page's open question (which choices are 25.0.7 and which
/// 26.2.1) is left open: a trace from a 26.2.1 build settles it.
///
/// There is no ZS/CRC extension in this trace ("Has ZS CRC Extension:
/// false"), so none is packed here; the Framebuffer Parameters carry that
/// flag clear. The Blend's internal half is read twice by pandecode, as the
/// shader PC and as the fixed-function half; the Opaque mode uses the
/// fixed-function half, and the 0xED000 it prints as a PC is Memory Format
/// 237 (RGBA8 TB) << 12.
#[test]
fn the_linear_run_s_framebuffer_render_target_blend_depth_stencil_and_dcd_words_pack_to_the_board_s_bytes() {
    assert!(LINEAR_EXCERPT.contains("Mesa 25.0.7"), "the trace's Mesa version is recorded in the excerpt");
    let mem = memory(LINEAR_EXCERPT);
    let word = |reg: u8| move32(LINEAR_EXCERPT, reg).to_vec();
    /* the draw's Blend and Depth/Stencil pointers (r50, r52) and the
     * flags words (r56, r57, r58) are in the instruction stream: the
     * descriptors' addresses come from there, the low 6 bits of the Blend's
     * being its count (1) */
    let blend_va = 0x7fff_fffd_4140;
    let ds_va = 0x7fff_fffd_4120;
    assert!(LINEAR_EXCERPT.contains("MOVE d50, #0x7FFFFFFD4141"));
    assert!(LINEAR_EXCERPT.contains("MOVE d52, #0x7FFFFFFD4120"));
    let equation =
        BlendFunction { a: BlendOperandA::SRC, b: BlendOperandB::SRC, c: BlendOperandC::ZERO, ..Default::default() };
    let cases: [Case; 7] = [
        (
            "Framebuffer Parameters @7ffffffd4000",
            Some(0x7fff_fffd_4000),
            FramebufferParameters {
                pre_frame_0: PrePostFrameShaderMode::NEVER,
                pre_frame_1: PrePostFrameShaderMode::NEVER,
                post_frame: PrePostFrameShaderMode::NEVER,
                downscale_geometry: Downscale::NONE,
                sample_locations: 0x7fff_ffff_f000,
                width: 64,
                height: 64,
                bound_max_x: 63,
                bound_max_y: 63,
                point_sprite_coord_origin_max_y: false,
                sample_count: 1,
                sample_pattern: SamplePattern::SINGLE_SAMPLED,
                tie_break_rule: TieBreakRule::MINUS_180_IN_0_OUT,
                effective_tile_size: 1024,
                render_target_count: 1,
                color_buffer_allocation: 4096,
                z_internal_format: ZInternalFormat::D24,
                tiler: 0x7fff_fffd_42c0,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
            Vec::new(),
        ),
        (
            "RGB Render Target @7ffffffd4080",
            Some(0x7fff_fffd_4080),
            RgbRenderTarget {
                writeback_mode: WritebackMode::COMPAT,
                internal_format: ColorBufferInternalFormat::R8G8B8A8,
                write_enable: true,
                writeback_format: ColorFormat::R8G8B8A8,
                writeback_block_format: BlockFormat::LINEAR,
                writeback_msaa: Msaa::SINGLE,
                dithering_enable: true,
                swizzle: 1672,
                clean_tile_write_enable: true,
                writeback_buffer: RtBuffer { base: 0x7fff_fffe_4000, row_stride: 256, surface_stride: 0 },
                clear: RtClear {
                    color_0: 4_294_901_760,
                    color_1: 4_294_901_760,
                    color_2: 4_294_901_760,
                    color_3: 4_294_901_760,
                },
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
            Vec::new(),
        ),
        (
            "Blend @7ffffffd4140",
            Some(blend_va),
            Blend {
                enable: true,
                equation: BlendEquation { rgb: equation, alpha: equation, color_mask: 15 },
                internal: InternalBlend {
                    mode: BlendMode::OPAQUE,
                    fixed_function: BlendFixedFunction {
                        num_comps: 4,
                        rt: 0,
                        conversion: InternalConversion { memory_format: 237 << 12, raw: false },
                    },
                    ..Default::default()
                },
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
            Vec::new(),
        ),
        (
            "Depth/Stencil @7ffffffd4120",
            Some(ds_va),
            DepthStencil {
                type_: DescriptorType::DEPTH_STENCIL,
                front_compare_function: Func::NEVER,
                front_stencil_fail: StencilOp::KEEP,
                front_depth_fail: StencilOp::KEEP,
                front_depth_pass: StencilOp::KEEP,
                back_compare_function: Func::NEVER,
                back_stencil_fail: StencilOp::KEEP,
                back_depth_fail: StencilOp::KEEP,
                back_depth_pass: StencilOp::KEEP,
                depth_cull_enable: true,
                depth_clamp_mode: DepthClampMode::_0_1,
                depth_source: DepthSource::FIXED_FUNCTION,
                depth_function: Func::ALWAYS,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
            Vec::new(),
        ),
        (
            "Primitive flags (r56)",
            None,
            PrimitiveFlags {
                /* r56 has draw mode None: the draw mode is the Flags
                 * Override of RUN_IDVS (checked below) */
                draw_mode: DrawMode::NONE,
                index_type: IndexType::NONE,
                point_size_array_format: PointSizeArrayFormat::NONE,
                allow_rotating_primitives: true,
                low_depth_cull: true,
                high_depth_cull: true,
                position_fifo_format: FifoFormat::BASIC,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
            word(56),
        ),
        (
            "DCD Flags 0 (r57)",
            None,
            DcdFlags0 {
                allow_forward_pixel_to_kill: true,
                allow_forward_pixel_to_be_killed: true,
                pixel_kill_operation: PixelKill::WEAK_EARLY,
                zs_update_operation: PixelKill::WEAK_EARLY,
                overdraw_alpha0: true,
                overdraw_alpha1: true,
                /* "Single-sampled lines" in 25.0.7 */
                aligned_line_ends: true,
                occlusion_query: OcclusionMode::DISABLED,
                ..Default::default()
            }
            .pack()
            .expect("packs")
            .to_vec(),
            word(57),
        ),
        (
            "DCD Flags 1 (r58)",
            None,
            DcdFlags1 { sample_mask: 65535, render_target_mask: 1 }.pack().expect("packs").to_vec(),
            word(58),
        ),
    ];
    let mut differ = Vec::new();
    /* RUN_IDVS's Flags Override (its first 32 bits, the "#8" the
     * disassembly prints) is the draw mode, Triangles = 8, in the bits of
     * the Primitive Flags' draw mode; pandecode prints the two merged */
    let idvs = LINEAR_EXCERPT.lines().find(|l| l.contains("RUN_IDVS.")).expect("a RUN_IDVS line");
    let idvs_bytes: Vec<u8> = idvs
        .split("RUN_IDVS")
        .next()
        .expect("bytes")
        .split_whitespace()
        .map(|h| u8::from_str_radix(h, 16).expect("hex"))
        .collect();
    assert_eq!(idvs_bytes.len(), 8, "{idvs}");
    assert_eq!(u32::from(idvs_bytes[7]), CsOpcode::RUN_IDVS.0, "the opcode byte of {idvs}");
    let flags_override = u32::from_le_bytes([idvs_bytes[0], idvs_bytes[1], idvs_bytes[2], idvs_bytes[3]]);
    assert_eq!(flags_override, DrawMode::TRIANGLES.0, "the Flags Override is the draw mode");
    let merged = u32::from_le_bytes(move32(LINEAR_EXCERPT, 56)) | flags_override;
    let triangles = PrimitiveFlags {
        draw_mode: DrawMode::TRIANGLES,
        allow_rotating_primitives: true,
        low_depth_cull: true,
        high_depth_cull: true,
        position_fifo_format: FifoFormat::BASIC,
        ..Default::default()
    };
    assert_eq!(
        triangles.pack().expect("packs"),
        merged.to_le_bytes(),
        "r56 with the override, as pandecode printed it"
    );
    for (name, va, ours, raw) in &cases {
        let board = match va {
            Some(va) => board_bytes(&mem, *va, ours.len()),
            None => raw.clone(),
        };
        if &board != ours {
            differ.push(format!("{name}:\n  board {board:02x?}\n  ours  {ours:02x?}"));
        }
    }
    eprintln!(
        "mali board descriptors, linear run (Mesa 25.0.7 trace, 26.2.1 packers): {} run (planned 7), {} differing",
        cases.len(),
        differ.len()
    );
    assert!(differ.is_empty(), "{}", differ.join("\n"));
}
