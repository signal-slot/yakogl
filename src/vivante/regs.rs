// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2012-2026 Wladimir J. van der Laan
// SPDX-FileCopyrightText: 2012-2026 Christian Gmeiner
// SPDX-FileCopyrightText: 2012-2026 Lucas Stach
// SPDX-FileCopyrightText: 2012-2026 Russell King
// Register addresses and encodings derived from the etna_viv rnndb
// headers as shipped in Mesa 26.2.1 src/etnaviv/hw/ (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The Vivante GC7000 registers and command words this backend writes, for
//! the i.MX8MP's GC7000UL (model 0x7000, revision 0x6204, HALTI5).
//!
//! The GPU has no firmware: the host builds a command stream in memory and
//! the front end fetches it. A command is one of a handful of opcodes in the
//! top five bits of its first word; LOAD_STATE carries N consecutive register
//! values. Every command occupies an even number of words.
//!
//! Only the registers the backend programs are here, in address order, with
//! the fields it uses. The addresses and encodings were read from Mesa's
//! etnaviv register descriptions (the generated headers under
//! `src/etnaviv/hw`, from rnndb) and its driver; nothing here is copied from
//! a vendor header. What each one is for is in VIVANTE_NOTES.md.
#![allow(dead_code)]

/// `LOAD_STATE <reg>, <count>` [, fixed-point values]. The register address
/// is in words, hence the `>> 2`.
pub const fn load_state(reg: u32, count: u32, fixp: bool) -> u32 {
    GC_OP_LOAD_STATE | if fixp { 0x0400_0000 } else { 0 } | (count << 16) | (reg >> 2)
}

/// DRAW_INSTANCED: three words plus a pad.
pub const fn draw_instanced_hdr(indexed: bool, prim: u32, instances_lo: u32) -> u32 {
    GC_OP_DRAW_INSTANCED | if indexed { 0x0010_0000 } else { 0 } | (prim << 16) | (instances_lo & 0xffff)
}

pub const fn draw_instanced_count(instances_hi: u32, vertices: u32) -> u32 {
    (instances_hi << 24) | (vertices & 0x00ff_ffff)
}

/// `STALL <from>, <to>`: hold the front end until `to` has drained.
pub const fn stall_token(from: u32, to: u32) -> u32 {
    from | (to << 8)
}

/// The output mode of render target `rt` (PS_CONTROL_EXT).
pub const fn ps_output_mode(rt: u32, x: u32) -> u32 {
    (x & 0x7) << (4 * rt)
}

/* Render targets 1..7 (MRT): the 8-RT bank, which a HALTI2+ core uses
 * (Mesa etnaviv: num_rts = 8, etnaviv_emit.c); index rt - 1. State names
 * and addresses from state_3d.xml.h. */
/// RT `rt`'s address for pixel pipe `pipe` (one pipe on this core).
pub const fn pe_rt_addr_8_pipe(rt: u32, pipe: u32) -> u32 {
    0x00014800 + 0x20 * (rt - 1) + 4 * pipe
}
/// RT `rt`'s stride, format and tiling.
pub const fn pe_rt_config_8(rt: u32) -> u32 {
    0x00014900 + 4 * (rt - 1)
}
pub const fn rt_config_stride(x: u32) -> u32 {
    x & 0xffff
}
pub const fn rt_config_format(x: u32) -> u32 {
    (x & 0x3f) << 20
}
/// RT `rt`'s colour mask and overwrite hint.
pub const fn pe_halti5_rt_colormask(rt: u32) -> u32 {
    0x00014920 + 4 * (rt - 1)
}
pub const fn pe_halti5_rt_colormask_components(x: u32) -> u32 {
    (x & 0xf) << 4
}
pub const GC_PE_HALTI5_RT_COLORMASK_OVERWRITE: u32 = 0x00000100;
/// RT `rt`'s blend (PE_ALPHA_CONFIG's layout).
pub const fn pe_halti5_rt_alpha_config(rt: u32) -> u32 {
    0x00014960 + 4 * (rt - 1)
}
/// The unorm saturate of render target `rt` (PS_CONTROL), RT0's bit shifted.
pub const fn ps_control_saturate_rt(rt: u32) -> u32 {
    GC_PS_CONTROL_SATURATE_RT0 << rt
}

// gc_regs.h - the Vivante GC7000 registers and command words this backend
// writes, for the i.MX8MP's GC7000UL (model 0x7000, revision 0x6204, HALTI5).
// The GPU has no firmware: the host builds a command stream in memory and the
// front end fetches it. A command is one of a handful of opcodes in the top
// five bits of its first word; LOAD_STATE carries N consecutive register
// values. Every command occupies an even number of words.
// Only the registers the backend programs are listed, in address order, with
// the fields it uses. The addresses and encodings were read from Mesa's
// etnaviv register descriptions (the generated headers under
// src/etnaviv/hw, from rnndb) and its driver; nothing here is copied from a
// vendor header. What
// each one is for is in VIVANTE_NOTES.md and the pre-board map
// tools/cts/log/investigations/imx8mp-draw-state.md.

// ---- command stream ----------------------------------------------------

pub const GC_OP_LOAD_STATE: u32 = 0x08000000;
pub const GC_OP_NOP: u32 = 0x18000000;
pub const GC_OP_DRAW_INSTANCED: u32 = 0x60000000;
pub const GC_OP_STALL: u32 = 0x48000000;

// LOAD_STATE <reg>, <count> [, fixed-point values]. The register address is
// in words, hence the >> 2.

// DRAW_INSTANCED: three words plus a pad.

// A submit ends by writing GL_EVENT (below): the interrupt that reports the
// event id is the fence. Linking the stream into a ring is the OSAL's job.

// STALL <from>, <to>: hold the front end until `to` has drained.
pub const GC_SYNC_FE: u32 = 0x01;
pub const GC_SYNC_RA: u32 = 0x05;
pub const GC_SYNC_PE: u32 = 0x07;

// primitive types (DRAW_INSTANCED)
pub const GC_PRIM_POINTS: u32 = 1;
pub const GC_PRIM_LINES: u32 = 2;
pub const GC_PRIM_LINE_STRIP: u32 = 3;
pub const GC_PRIM_TRIANGLES: u32 = 4;
pub const GC_PRIM_TRIANGLE_STRIP: u32 = 5;
pub const GC_PRIM_TRIANGLE_FAN: u32 = 6;
pub const GC_PRIM_LINE_LOOP: u32 = 7;

// ---- front end: vertex and index streams --------------------------------

pub const GC_FE_INDEX_STREAM_BASE_ADDR: u32 = 0x00000644;
pub const GC_FE_INDEX_STREAM_CONTROL: u32 = 0x00000648;
pub const GC_INDEX_TYPE_U8: u32 = 0x00000000;
pub const GC_INDEX_TYPE_U16: u32 = 0x00000001;
pub const GC_INDEX_TYPE_U32: u32 = 0x00000002;
pub const GC_FE_PRIMITIVE_RESTART_INDEX: u32 = 0x00000674;
// gl_VertexID / gl_InstanceID: which register the front end writes them to
pub const GC_FE_HALTI5_ID_CONFIG: u32 = 0x000007c4;
pub const GC_ID_CONFIG_VERTEX_ID_ENABLE: u32 = 0x00000001;
pub const GC_ID_CONFIG_INSTANCE_ID_ENABLE: u32 = 0x00000002;
pub const fn id_config_vertex_id_reg(x: u32) -> u32 {
    (x & 0xff) << 8
}
pub const fn id_config_instance_id_reg(x: u32) -> u32 {
    (x & 0xff) << 16
}
pub const GC_FE_HALTI5_UNK007D8: u32 = 0x000007d8;

// HALTI2+ vertex streams: one base address, stride and instance divisor each
pub const fn nfe_vertex_streams_base_addr(i: u32) -> u32 {
    0x00014600 + 4 * i
}
pub const fn nfe_vertex_streams_control(i: u32) -> u32 {
    0x00014640 + 4 * i
}
pub const fn vertex_stream_stride(x: u32) -> u32 {
    x & 0xff
}
pub const fn nfe_vertex_streams_vertex_divisor(i: u32) -> u32 {
    0x00014680 + 4 * i
}

// HALTI5 vertex elements: format in CONFIG0, extent in CONFIG1
pub const fn nfe_generic_attrib_config0(i: u32) -> u32 {
    0x00017800 + 4 * i
}
pub const fn attrib_type(x: u32) -> u32 {
    x & 0xf
}
pub const fn attrib_endian(x: u32) -> u32 {
    (x & 0x3) << 4
}
pub const fn attrib_stream(x: u32) -> u32 {
    (x & 0xf) << 8
}
/// components, 4 encodes as 0
pub const fn attrib_num(x: u32) -> u32 {
    (x & 0x3) << 12
}
pub const fn attrib_normalize(x: u32) -> u32 {
    (x & 0x3) << 14
}
pub const GC_NORMALIZE_OFF: u32 = 0;
pub const GC_NORMALIZE_SIGN_EXTEND: u32 = 1;
pub const GC_NORMALIZE_ON: u32 = 2;
pub const fn attrib_start(x: u32) -> u32 {
    (x & 0xff) << 16
}
pub const fn nfe_generic_attrib_scale(i: u32) -> u32 {
    0x00017a00 + 4 * i
}
pub const fn nfe_generic_attrib_config1(i: u32) -> u32 {
    0x00017a80 + 4 * i
}
pub const fn attrib_end(x: u32) -> u32 {
    x & 0xff
}
pub const GC_ATTRIB_NONCONSECUTIVE: u32 = 0x00000800;
/// the whole array, cleared at reset
pub const GC_NFE_GENERIC_ATTRIB_COUNT: usize = 32;

// vertex element data types
pub const GC_DATA_TYPE_BYTE: u32 = 0x0;
pub const GC_DATA_TYPE_UNSIGNED_BYTE: u32 = 0x1;
pub const GC_DATA_TYPE_SHORT: u32 = 0x2;
pub const GC_DATA_TYPE_UNSIGNED_SHORT: u32 = 0x3;
pub const GC_DATA_TYPE_FLOAT: u32 = 0x8;
pub const GC_DATA_TYPE_FIXED: u32 = 0xb;

// ---- vertex shader ------------------------------------------------------

pub const GC_VS_OUTPUT_COUNT: u32 = 0x00000804;
pub const GC_VS_INPUT_COUNT: u32 = 0x00000808;
pub const fn vs_input_count_count(x: u32) -> u32 {
    x & 0x1f
}
pub const fn vs_input_count_unk8(x: u32) -> u32 {
    (x & 0x1f) << 8
}
pub const GC_VS_INPUT_COUNT_ID_ENABLE: u32 = 0x80000000;
pub const GC_VS_TEMP_REGISTER_CONTROL: u32 = 0x0000080c;
pub const fn temp_register_num(x: u32) -> u32 {
    x & 0x3f
}
pub const GC_VS_LOAD_BALANCING: u32 = 0x00000830;
/// in vec4 registers
pub const GC_VS_UNIFORM_BASE: u32 = 0x00000864;
pub const GC_VS_INST_ADDR: u32 = 0x0000086c;
pub const GC_SH_ICACHE_CONTROL: u32 = 0x00000868;
pub const GC_SH_ICACHE_ENABLE: u32 = 0x00000001;
pub const GC_VS_HALTI5_OUTPUT_COUNT: u32 = 0x00000870;
pub const GC_VS_NEWRANGE_LOW: u32 = 0x00000874;
pub const GC_PS_NEWRANGE_LOW: u32 = 0x0000087c;
pub const GC_VS_HALTI1_UNK00884: u32 = 0x00000884;
pub const GC_VS_ICACHE_PREFETCH: u32 = 0x0000088c;
pub const GC_VS_HALTI5_UNK008A0: u32 = 0x000008a0;
pub const GC_VS_SAMPLER_BASE: u32 = 0x000008a8;
pub const GC_VS_ICACHE_INVALIDATE: u32 = 0x000008b0;
pub const GC_VS_ICACHE_INVALIDATE_ALL: u32 = 0x0000001f;
pub const GC_VS_HALTI5_RANGE_HIGH: u32 = 0x000008bc;
// the vertex shader's input registers, four per word
pub const fn vs_halti5_input(i: u32) -> u32 {
    0x000008c0 + 4 * i
}
pub const GC_VS_HALTI5_INPUT_LEN: usize = 8;
// the vertex shader's output registers, four per word: position first, then
// one per varying, then point size
pub const fn vs_halti5_output(i: u32) -> u32 {
    0x000008e0 + 4 * i
}
pub const GC_VS_HALTI5_OUTPUT_LEN: usize = 8;
pub const GC_VS_ICACHE_COUNT: u32 = 0x00015604;
pub const GC_SH_CONFIG: u32 = 0x00015600;
pub const GC_SH_CONFIG_RTNE_ROUNDING: u32 = 0x00000002;

// ---- primitive assembly -------------------------------------------------

/// 16.16 fixed point
pub const GC_PA_VIEWPORT_SCALE_X: u32 = 0x00000a00;
pub const GC_PA_VIEWPORT_SCALE_Y: u32 = 0x00000a04;
/// float
pub const GC_PA_VIEWPORT_SCALE_Z: u32 = 0x00000a08;
/// 16.16 fixed point
pub const GC_PA_VIEWPORT_OFFSET_X: u32 = 0x00000a0c;
pub const GC_PA_VIEWPORT_OFFSET_Y: u32 = 0x00000a10;
/// float
pub const GC_PA_VIEWPORT_OFFSET_Z: u32 = 0x00000a14;
/// float, half the width
pub const GC_PA_LINE_WIDTH: u32 = 0x00000a18;
/// float, half the size
pub const GC_PA_POINT_SIZE: u32 = 0x00000a1c;
pub const GC_PA_SYSTEM_MODE: u32 = 0x00000a28;
pub const GC_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST: u32 = 0x00000001;
pub const GC_PA_SYSTEM_MODE_HALF_PIXEL_CENTER: u32 = 0x00000010;
pub const GC_PA_W_CLIP_LIMIT: u32 = 0x00000a2c;
pub const GC_PA_ATTRIBUTE_ELEMENT_COUNT: u32 = 0x00000a30;
pub const fn pa_attribute_element_count_count(x: u32) -> u32 {
    (x & 0xff) << 8
}
pub const GC_PA_CONFIG: u32 = 0x00000a34;
pub const GC_PA_CONFIG_POINT_SIZE_ENABLE: u32 = 0x00000004;
pub const GC_PA_CONFIG_POINT_SPRITE_ENABLE: u32 = 0x00000010;
pub const GC_PA_CONFIG_CULL_FACE_OFF: u32 = 0x00000000;
pub const GC_PA_CONFIG_CULL_FACE_CW: u32 = 0x00000100;
pub const GC_PA_CONFIG_CULL_FACE_CCW: u32 = 0x00000200;
pub const GC_PA_CONFIG_FILL_SOLID: u32 = 0x00002000;
pub const GC_PA_CONFIG_SHADE_MODEL_FLAT: u32 = 0x00000000;
pub const GC_PA_CONFIG_SHADE_MODEL_SMOOTH: u32 = 0x00010000;
pub const GC_PA_CONFIG_WIDE_LINE: u32 = 0x00400000;
pub const GC_PA_WIDE_LINE_WIDTH0: u32 = 0x00000a38;
pub const GC_PA_WIDE_LINE_WIDTH1: u32 = 0x00000a3c;
pub const GC_PA_VIEWPORT_UNK00A80: u32 = 0x00000a80;
pub const GC_PA_VIEWPORT_UNK00A84: u32 = 0x00000a84;
pub const GC_PA_FLAGS: u32 = 0x00000a88;
pub const GC_PA_ZFARCLIPPING: u32 = 0x00000a8c;
// four bits per varying: how many components it has
pub const fn pa_varying_num_components(i: u32) -> u32 {
    0x00000a90 + 4 * i
}
pub const GC_PA_VS_OUTPUT_COUNT: u32 = 0x00000aa8;

// ---- setup engine: scissor, clip, depth bias ----------------------------

/// 16.16 fixed point
pub const GC_SE_SCISSOR_LEFT: u32 = 0x00000c00;
pub const GC_SE_SCISSOR_TOP: u32 = 0x00000c04;
pub const GC_SE_SCISSOR_RIGHT: u32 = 0x00000c08;
pub const GC_SE_SCISSOR_BOTTOM: u32 = 0x00000c0c;
/// float: polygon offset factor
pub const GC_SE_DEPTH_SCALE: u32 = 0x00000c10;
/// float: polygon offset units
pub const GC_SE_DEPTH_BIAS: u32 = 0x00000c14;
pub const GC_SE_CONFIG: u32 = 0x00000c18;
/// 16.16 fixed point
pub const GC_SE_CLIP_RIGHT: u32 = 0x00000c20;
pub const GC_SE_CLIP_BOTTOM: u32 = 0x00000c24;
// Mesa rounds the scissor and clip rectangles outwards by these amounts, and
// the two are not the same number: the clip gets 0xffff, the scissor gets
// 0x1119 across and 0x1111 down (ETNA_SE_SCISSOR_MARGIN_RIGHT / _BOTTOM in
// etnaviv_internal.h). They look arbitrary because they are -- they came from
// the hardware, not from arithmetic.
//
// Both scissor margins used to be 0xffff here, copied from the clip. That is
// almost a whole pixel of extra fraction: on a 64-wide target it put the
// scissor's right edge at 64.99998 instead of 64.067, so a fragment at x = 64
// passed a test meant to stop at 64.
pub const GC_SE_SCISSOR_MARGIN_RIGHT: u32 = 0x0000_1119;
pub const GC_SE_SCISSOR_MARGIN_BOTTOM: u32 = 0x0000_1111;
pub const GC_SE_CLIP_MARGIN: u32 = 0x0000ffff;

// ---- rasteriser ---------------------------------------------------------

pub const GC_RA_CONTROL: u32 = 0x00000e00;
pub const GC_RA_CONTROL_UNK0: u32 = 0x00000001;
pub const GC_RA_CONTROL_LAST_VARYING_2X: u32 = 0x00000002;
pub const GC_RA_EARLY_DEPTH: u32 = 0x00000e08;
pub const GC_RA_EARLY_DEPTH_TEST_ENABLE: u32 = 0x00000001;
pub const GC_RA_EARLY_DEPTH_FORWARD_Z: u32 = 0x01000000;
pub const GC_RA_EARLY_DEPTH_FORWARD_W: u32 = 0x04000000;
pub const GC_RA_EARLY_DEPTH_WRITE_DISABLE: u32 = 0x10000000;
/// always set; blob value
pub const GC_RA_EARLY_DEPTH_BASE: u32 = 0x00000030;
pub const GC_RA_UNK00E0C: u32 = 0x00000e0c;
pub const GC_RA_HDEPTH_CONTROL: u32 = 0x00000e20;

// ---- fragment shader ----------------------------------------------------

pub const GC_PS_OUTPUT_REG: u32 = 0x00001004;
pub const GC_PS_INPUT_COUNT: u32 = 0x00001008;
pub const fn ps_input_count_count(x: u32) -> u32 {
    x & 0x1f
}
pub const fn ps_input_count_unk8(x: u32) -> u32 {
    (x & 0x1f) << 8
}
pub const GC_PS_TEMP_REGISTER_CONTROL: u32 = 0x0000100c;
pub const GC_PS_CONTROL: u32 = 0x00001010;
pub const GC_PS_CONTROL_SATURATE_RT0: u32 = 0x00000002;
pub const fn ps_control_rt_count(x: u32) -> u32 {
    (x & 0x7) << 8
}
/// in vec4 registers
pub const GC_PS_UNIFORM_BASE: u32 = 0x00001024;
pub const GC_PS_INST_ADDR: u32 = 0x00001028;
pub const GC_PS_CONTROL_EXT: u32 = 0x00001030;
pub const GC_PS_OUTPUT_MODE_NORMAL: u32 = 0;
/// A render target whose components are 32 bits wide. `COLOR_OUTPUT_MODE_UIF32`
/// in the rnndb header (`hw/state_3d.xml.h:131`).
pub const GC_PS_OUTPUT_MODE_UIF32: u32 = 2;
pub const GC_PS_HALTI3_UNK0103C: u32 = 0x0000103c;
pub const GC_PS_ICACHE_PREFETCH: u32 = 0x00001048;
pub const GC_PS_MSAA_CONFIG: u32 = 0x00001054;
/// alpha-to-coverage, and the bit that stops it writing
pub const GC_PS_MSAA_ALPHA_TO_COVERAGE: u32 = 0x00001000;
pub const GC_PS_MSAA_ALPHA_TO_COVERAGE_MASK: u32 = 0x00008000;
/* The sample mask and sample coverage fields and their write masks
 * (etnaviv state_3d.xml.h: VIVS_PS_MSAA_CONFIG_*). A `_MASK` bit set leaves
 * its field as it was; cleared, the field is written. */
pub const GC_PS_MSAA_SAMPLE_MASK_FIELD: u32 = 0x0000000f;
pub const GC_PS_MSAA_SAMPLE_MASK_MASK: u32 = 0x00000080;
pub const GC_PS_MSAA_SAMPLE_MASK_ENABLE: u32 = 0x00000100;
pub const GC_PS_MSAA_SAMPLE_MASK_ENABLE_MASK: u32 = 0x00000800;
pub const GC_PS_MSAA_SAMPLE_COVERAGE_INVERT: u32 = 0x00010000;
pub const GC_PS_MSAA_SAMPLE_COVERAGE_INVERT_MASK: u32 = 0x00080000;
pub const GC_PS_MSAA_SAMPLE_COVERAGE_VALUE_FIELD: u32 = 0x01f00000;
pub const GC_PS_MSAA_SAMPLE_COVERAGE_VALUE_MASK: u32 = 0x08000000;
pub const GC_PS_MSAA_SAMPLE_COVERAGE: u32 = 0x10000000;
pub const GC_PS_MSAA_SAMPLE_COVERAGE_MASK: u32 = 0x80000000;
pub const GC_PS_SAMPLER_BASE: u32 = 0x00001058;
pub const fn ps_varying_num_components(i: u32) -> u32 {
    0x00001080 + 4 * i
}
pub const GC_PS_HALTI5_RANGE_HIGH: u32 = 0x00001090;
pub const GC_PS_ICACHE_COUNT: u32 = 0x00001094;

// ---- pixel engine -------------------------------------------------------

pub const GC_PE_DEPTH_CONFIG: u32 = 0x00001400;
pub const GC_PE_DEPTH_MODE_NONE: u32 = 0x00000000;
pub const GC_PE_DEPTH_MODE_Z: u32 = 0x00000001;
pub const GC_PE_DEPTH_FORMAT_D16: u32 = 0x00000000;
pub const GC_PE_DEPTH_FORMAT_D24S8: u32 = 0x00000010;
pub const fn pe_depth_func(x: u32) -> u32 {
    (x & 0x7) << 8
}
pub const GC_PE_DEPTH_WRITE_ENABLE: u32 = 0x00001000;
pub const GC_PE_DEPTH_EARLY_Z: u32 = 0x00010000;
pub const GC_PE_DEPTH_UNK18: u32 = 0x00040000;
pub const GC_PE_DEPTH_DISABLE_ZS: u32 = 0x01000000;
// The depth surface is super-tiled, which is what Mesa describes for every
// depth buffer it makes: etna_layout_multiple gives ETNA_LAYOUT_SUPER_TILED a
// 64x64 padding, and ensure_depth() already allocates to exactly that.
pub const GC_PE_DEPTH_SUPER_TILED: u32 = 0x04000000;
/// float
pub const GC_PE_DEPTH_NEAR: u32 = 0x00001404;
/// float
pub const GC_PE_DEPTH_FAR: u32 = 0x00001408;
/// float: 2^bits - 1
pub const GC_PE_DEPTH_NORMALIZE: u32 = 0x0000140c;
pub const GC_PE_DEPTH_STRIDE: u32 = 0x00001414;
pub const GC_PE_STENCIL_OP: u32 = 0x00001418;
pub const fn pe_stencil_func_front(x: u32) -> u32 {
    x & 0x7
}
pub const fn pe_stencil_pass_front(x: u32) -> u32 {
    (x & 0x7) << 4
}
pub const fn pe_stencil_fail_front(x: u32) -> u32 {
    (x & 0x7) << 8
}
pub const fn pe_stencil_depth_fail_front(x: u32) -> u32 {
    (x & 0x7) << 12
}
pub const fn pe_stencil_func_back(x: u32) -> u32 {
    (x & 0x7) << 16
}
pub const fn pe_stencil_pass_back(x: u32) -> u32 {
    (x & 0x7) << 20
}
pub const fn pe_stencil_fail_back(x: u32) -> u32 {
    (x & 0x7) << 24
}
pub const fn pe_stencil_depth_fail_back(x: u32) -> u32 {
    (x & 0x7) << 28
}
pub const GC_PE_STENCIL_CONFIG: u32 = 0x0000141c;
pub const GC_PE_STENCIL_MODE_DISABLED: u32 = 0x00000000;
pub const GC_PE_STENCIL_MODE_ONE_SIDED: u32 = 0x00000001;
pub const GC_PE_STENCIL_MODE_TWO_SIDED: u32 = 0x00000002;
pub const fn pe_stencil_ref_front(x: u32) -> u32 {
    (x & 0xff) << 8
}
pub const fn pe_stencil_mask_front(x: u32) -> u32 {
    (x & 0xff) << 16
}
pub const fn pe_stencil_write_mask_front(x: u32) -> u32 {
    (x & 0xff) << 24
}
pub const GC_PE_ALPHA_OP: u32 = 0x00001420;
pub const GC_PE_ALPHA_BLEND_COLOR: u32 = 0x00001424;
pub const fn pe_blend_color_b(x: u32) -> u32 {
    x & 0xff
}
pub const fn pe_blend_color_g(x: u32) -> u32 {
    (x & 0xff) << 8
}
pub const fn pe_blend_color_r(x: u32) -> u32 {
    (x & 0xff) << 16
}
pub const fn pe_blend_color_a(x: u32) -> u32 {
    (x & 0xff) << 24
}
pub const GC_PE_ALPHA_CONFIG: u32 = 0x00001428;
pub const GC_PE_ALPHA_BLEND_ENABLE_COLOR: u32 = 0x00000001;
pub const fn pe_alpha_src_func_color(x: u32) -> u32 {
    (x & 0xf) << 4
}
pub const fn pe_alpha_dst_func_color(x: u32) -> u32 {
    (x & 0xf) << 8
}
pub const fn pe_alpha_eq_color(x: u32) -> u32 {
    (x & 0x7) << 12
}
pub const GC_PE_ALPHA_BLEND_SEPARATE_ALPHA: u32 = 0x00010000;
pub const fn pe_alpha_src_func_alpha(x: u32) -> u32 {
    (x & 0xf) << 20
}
pub const fn pe_alpha_dst_func_alpha(x: u32) -> u32 {
    (x & 0xf) << 24
}
pub const fn pe_alpha_eq_alpha(x: u32) -> u32 {
    (x & 0x7) << 28
}
pub const GC_PE_COLOR_FORMAT: u32 = 0x0000142c;
pub const fn pe_color_format_format(x: u32) -> u32 {
    x & 0xf
}
pub const fn pe_color_format_components(x: u32) -> u32 {
    (x & 0xf) << 8
}
pub const GC_PE_COLOR_FORMAT_OVERWRITE: u32 = 0x00010000;
/// A supertiled colour surface (etnaviv state_3d.xml.h: VIVS_PE_COLOR_FORMAT_SUPER_TILED).
pub const GC_PE_COLOR_FORMAT_SUPER_TILED: u32 = 0x0010_0000;
/// The same on a CACHE128B256BPERLINE core (VIVS_PE_COLOR_FORMAT_SUPER_TILED_NEW).
pub const GC_PE_COLOR_FORMAT_SUPER_TILED_NEW: u32 = 0x0000_2000;
pub const fn pe_color_format_format_ext(x: u32) -> u32 {
    (x & 0x7f) << 24
}
/// The write mask of the 4-bit FORMAT field (bit 4): set, FORMAT keeps its
/// value and FORMAT_EXT is the format. etnaviv_state.c:282-283 writes
/// `FORMAT_EXT(fmt) | FORMAT_MASK` for every format from R16F up
/// (state_3d.xml.h: VIVS_PE_COLOR_FORMAT_FORMAT_MASK 0x00000010).
pub const GC_PE_COLOR_FORMAT_FORMAT_MASK: u32 = 0x0000_0010;
/// The write mask of FORMAT_EXT itself (bit 31, VIVS_PE_COLOR_FORMAT_FORMAT_EXT_MASK).
/// Never set: with it the PE keeps its old FORMAT_EXT. This backend set it in
/// place of bit 4 until gl_test 44, so every float target was drawn in
/// whatever format the PE held (#6).
pub const GC_PE_COLOR_FORMAT_FORMAT_EXT_MASK: u32 = 0x8000_0000;
pub const GC_PE_COLOR_STRIDE: u32 = 0x00001434;
pub const GC_PE_HDEPTH_CONTROL: u32 = 0x00001454;
pub const fn pe_pipe_color_addr(i: u32) -> u32 {
    0x00001460 + 4 * i
}
pub const fn pe_pipe_depth_addr(i: u32) -> u32 {
    0x00001480 + 4 * i
}
pub const GC_PE_STENCIL_CONFIG_EXT: u32 = 0x000014a0;
pub const fn pe_stencil_ref_back(x: u32) -> u32 {
    x & 0xff
}
pub const GC_PE_LOGIC_OP: u32 = 0x000014a4;
pub const fn pe_logic_op_op(x: u32) -> u32 {
    x & 0xf
}
pub const fn pe_logic_op_single_buffer(x: u32) -> u32 {
    (x & 0x3) << 8
}
/// Bits 26:24, unnamed; Mesa sets 5 for a 4x target on a SMALL_MSAA core
/// (etnaviv_state.c). Bit 27 is its write mask, left clear.
pub const fn pe_logic_op_unk24(x: u32) -> u32 {
    (x & 0x7) << 24
}
/// always set; blob value
pub const GC_PE_LOGIC_OP_BASE: u32 = 0x000e4000;
pub const GC_LOGIC_OP_COPY: u32 = 0x0000000c;
pub const fn pe_dither(i: u32) -> u32 {
    0x000014a8 + 4 * i
}
/// two halves: B, G
pub const GC_PE_ALPHA_COLOR_EXT0: u32 = 0x000014b0;
/// two halves: R, A
pub const GC_PE_ALPHA_COLOR_EXT1: u32 = 0x000014b4;
pub const GC_PE_STENCIL_CONFIG_EXT2: u32 = 0x000014b8;
pub const fn pe_stencil_mask_back(x: u32) -> u32 {
    x & 0xff
}
pub const fn pe_stencil_write_mask_back(x: u32) -> u32 {
    (x & 0xff) << 8
}
pub const GC_PE_MEM_CONFIG: u32 = 0x000014bc;
pub const GC_PE_HALTI4_UNK014C0: u32 = 0x000014c0;

// colour surface formats (PE_COLOR_FORMAT_FORMAT)
pub const GC_PE_FORMAT_R5G6B5: u32 = 0x4;
pub const GC_PE_FORMAT_X8R8G8B8: u32 = 0x5;
pub const GC_PE_FORMAT_A8R8G8B8: u32 = 0x6;
// the extended set (PE_COLOR_FORMAT_FORMAT_EXT with FORMAT_MASK)
pub const GC_PE_FORMAT_R16F: u32 = 0x11;
pub const GC_PE_FORMAT_A16B16G16R16F: u32 = 0x13;
pub const GC_PE_FORMAT_R32F: u32 = 0x14;

// depth formats (PE_DEPTH_CONFIG_DEPTH_FORMAT): D24S8 is the only one with
// stencil, and the one this backend allocates.
pub const GC_DEPTH_BITS_D24S8: u32 = 24;

// blend factors (PE_ALPHA_CONFIG)
pub const GC_BLEND_ZERO: u32 = 0x0;
pub const GC_BLEND_ONE: u32 = 0x1;
pub const GC_BLEND_SRC_COLOR: u32 = 0x2;
pub const GC_BLEND_ONE_MINUS_SRC_COLOR: u32 = 0x3;
pub const GC_BLEND_SRC_ALPHA: u32 = 0x4;
pub const GC_BLEND_ONE_MINUS_SRC_ALPHA: u32 = 0x5;
pub const GC_BLEND_DST_ALPHA: u32 = 0x6;
pub const GC_BLEND_ONE_MINUS_DST_ALPHA: u32 = 0x7;
pub const GC_BLEND_DST_COLOR: u32 = 0x8;
pub const GC_BLEND_ONE_MINUS_DST_COLOR: u32 = 0x9;
pub const GC_BLEND_SRC_ALPHA_SATURATE: u32 = 0xa;
pub const GC_BLEND_CONSTANT_ALPHA: u32 = 0xb;
pub const GC_BLEND_ONE_MINUS_CONSTANT_ALPHA: u32 = 0xc;
pub const GC_BLEND_CONSTANT_COLOR: u32 = 0xd;
pub const GC_BLEND_ONE_MINUS_CONSTANT_COLOR: u32 = 0xe;
// blend equations: the GL order (add, subtract, reverse subtract, min, max)
pub const GC_BLEND_EQ_ADD: u32 = 0x0;

// stencil operations (PE_STENCIL_OP)
pub const GC_STENCIL_OP_KEEP: u32 = 0x0;
pub const GC_STENCIL_OP_ZERO: u32 = 0x1;
pub const GC_STENCIL_OP_REPLACE: u32 = 0x2;
pub const GC_STENCIL_OP_INCR: u32 = 0x3;
pub const GC_STENCIL_OP_DECR: u32 = 0x4;
pub const GC_STENCIL_OP_INVERT: u32 = 0x5;
pub const GC_STENCIL_OP_INCR_WRAP: u32 = 0x6;
pub const GC_STENCIL_OP_DECR_WRAP: u32 = 0x7;

// ---- tile status (not used: no fast clear, no compression) --------------

pub const GC_TS_FLUSH_CACHE: u32 = 0x00001650;
pub const GC_TS_FLUSH: u32 = 0x00000001;
pub const GC_TS_MEM_CONFIG: u32 = 0x00001654;

// ---- global -------------------------------------------------------------

pub const GC_GL_PIPE_SELECT: u32 = 0x00003800;
pub const GC_PIPE_3D: u32 = 0x00000000;
pub const GC_GL_EVENT: u32 = 0x00003804;
pub const GC_GL_SEMAPHORE_TOKEN: u32 = 0x00003808;
/// unused: the FE stalls itself
pub const GC_GL_STALL_TOKEN: u32 = 0x00003c00;
pub const GC_GL_FLUSH_CACHE: u32 = 0x0000380c;
pub const GC_FLUSH_DEPTH: u32 = 0x00000001;
pub const GC_FLUSH_COLOR: u32 = 0x00000002;
pub const GC_FLUSH_TEXTURE: u32 = 0x00000004;
pub const GC_FLUSH_TEXTUREVS: u32 = 0x00000010;
pub const GC_FLUSH_SHADER_L1: u32 = 0x00000020;
/// the two descriptor-cache bits
pub const GC_FLUSH_DESCRIPTOR: u32 = 0x00003000;
pub const GC_GL_MULTI_SAMPLE_CONFIG: u32 = 0x00003818;
pub const GC_MSAA_SAMPLES_NONE: u32 = 0x00000000;
pub const GC_MSAA_SAMPLES_4X: u32 = 0x00000002;
pub const fn msaa_enables(x: u32) -> u32 {
    (x & 0xf) << 4
}
pub const GC_GL_VARYING_TOTAL_COMPONENTS: u32 = 0x0000381c;
/// Where the pixel engine stores its running sample count (64 bits) when
/// `GC_GL_OCCLUSION_QUERY_CONTROL` is written: Mesa's state.xml.h
/// VIVS_GL_OCCLUSION_QUERY_ADDR (vendored at driver/src/vivante/hw/state.xml.h),
/// written by etnaviv_query_acc_occlusion.c's resume.
pub const GC_GL_OCCLUSION_QUERY_ADDR: u32 = 0x00003824;
/// Writing it makes the pixel engine store the count at the address above
/// (VIVS_GL_OCCLUSION_QUERY_CONTROL, etnaviv's suspend).
pub const GC_GL_OCCLUSION_QUERY_CONTROL: u32 = 0x00003830;
/// What etnaviv writes there: "the value used by the blob - but any random
/// value will work" (etnaviv_query_acc_occlusion.c, occlusion_suspend).
pub const GC_OQ_CONTROL_VALUE: u32 = 0x01df5e76;
pub const GC_GL_API_MODE: u32 = 0x0000384c;
pub const GC_API_MODE_OPENGL: u32 = 0x00000000;
pub const GC_GL_BUG_FIXES: u32 = 0x00003860;
pub const GC_GL_HALTI5_SH_SPECIALS: u32 = 0x00003888;
pub const GC_SH_SPECIALS_BASE: u32 = 0x7f7f0000;
pub const fn sh_specials_vs_psize_out(x: u32) -> u32 {
    x & 0x7f
}
pub const fn sh_specials_ps_pcoord_in(x: u32) -> u32 {
    (x & 0x7f) << 8
}
pub const GC_SH_SPECIALS_NO_PCOORD: u32 = 0x7f;
// two bits per varying component, sixteen components per word: what the
// rasteriser puts there (Mesa's state.xml.h VIVS_GL_VARYING_COMPONENT_USE,
// ..._USE2, ..._USE3; etnaviv_emit.c writes all four)
pub const GC_GL_VARYING_COMPONENT_USE0: u32 = 0x00003828;
pub const GC_GL_VARYING_COMPONENT_USE1: u32 = 0x0000382c;
pub const GC_GL_VARYING_COMPONENT_USE2: u32 = 0x00003838;
pub const GC_GL_VARYING_COMPONENT_USE3: u32 = 0x00003854;
pub const GC_VARYING_COMPONENT_USE_GENERIC: u32 = 0;
pub const GC_VARYING_COMPONENT_USE_POINTCOORD_X: u32 = 2;
pub const GC_VARYING_COMPONENT_USE_POINTCOORD_Y: u32 = 3;
// four bits per varying component: its interpolation mode
pub const fn gl_halti5_shader_attributes(i: u32) -> u32 {
    0x000038c0 + 4 * i
}
pub const GC_INTERP_SMOOTH: u32 = 0;
pub const GC_INTERP_NONPERSPECTIVE: u32 = 1;
pub const GC_INTERP_FLAT: u32 = 2;

// ---- texture descriptors (HALTI5) ---------------------------------------

pub const GC_NTE_DESCRIPTOR_CONTROL: u32 = 0x00014c40;
pub const GC_NTE_DESCRIPTOR_ENABLE: u32 = 0x00000001;
pub const GC_NTE_DESCRIPTOR_FLUSH: u32 = 0x00014c44;
pub const GC_NTE_DESCRIPTOR_INVALIDATE: u32 = 0x00014c48;
pub const GC_NTE_INVALIDATE_UNK29: u32 = 0x20000000;
pub const fn nte_invalidate_idx(x: u32) -> u32 {
    x & 0x3f
}
pub const fn nte_descriptor_addr(i: u32) -> u32 {
    0x00015c00 + 4 * i
}
pub const fn nte_descriptor_tx_ctrl(i: u32) -> u32 {
    0x00015e00 + 4 * i
}
pub const fn nte_descriptor_samp_ctrl0(i: u32) -> u32 {
    0x00016c00 + 4 * i
}
pub const fn samp_uwrap(x: u32) -> u32 {
    x & 0x7
}
pub const fn samp_vwrap(x: u32) -> u32 {
    (x & 0x7) << 3
}
pub const fn samp_wwrap(x: u32) -> u32 {
    (x & 0x7) << 6
}
pub const fn samp_min(x: u32) -> u32 {
    (x & 0x3) << 9
}
pub const fn samp_mip(x: u32) -> u32 {
    (x & 0x3) << 11
}
pub const fn samp_mag(x: u32) -> u32 {
    (x & 0x3) << 13
}
pub const GC_SAMP_CTRL0_UNK21: u32 = 0x00200000;
pub const GC_SAMP_INT_FILTER: u32 = 0x00800000;
pub const fn nte_descriptor_samp_ctrl1(i: u32) -> u32 {
    0x00016e00 + 4 * i
}
pub const GC_SAMP_CTRL1_UNK1: u32 = 0x00000002;
/// The sampler decodes sRGB red, green and blue to linear before it filters
/// (`VIVS_NTE_DESCRIPTOR_SAMP_CTRL1_SRGB`, Mesa 26.2.1
/// src/etnaviv/hw/state_3d.xml.h:1984; set for an sRGB view,
/// etnaviv_texture_desc.c:246-247).
pub const GC_SAMP_CTRL1_SRGB: u32 = 0x00000004;
pub const fn nte_descriptor_samp_lod_minmax(i: u32) -> u32 {
    0x00017000 + 4 * i
}
// The LOD range is 5.8 fixed point; 0xfff is the largest it holds, which is
// what GL_TEXTURE_MAX_LOD's default of 1000 saturates to.
pub const GC_SAMP_LOD_FP58_MAX: u32 = 0xfff;
pub const fn samp_lod_max(x: u32) -> u32 {
    x & 0xffff
}
pub const fn samp_lod_min(x: u32) -> u32 {
    (x & 0xffff) << 16
}
pub const fn nte_descriptor_samp_lod_bias(i: u32) -> u32 {
    0x00017200 + 4 * i
}
pub const fn nte_descriptor_samp_anisotropy(i: u32) -> u32 {
    0x00017400 + 4 * i
}

// filters (sampler MIN/MAG/MIP)
pub const GC_TEX_FILTER_NONE: u32 = 0;
pub const GC_TEX_FILTER_NEAREST: u32 = 1;
pub const GC_TEX_FILTER_LINEAR: u32 = 2;
// wrap modes
pub const GC_TEX_WRAP_REPEAT: u32 = 0;
pub const GC_TEX_WRAP_MIRROR_REPEAT: u32 = 1;
pub const GC_TEX_WRAP_CLAMP_TO_EDGE: u32 = 2;

// One texture descriptor: 256 bytes in GPU memory, addressed by
// NTE_DESCRIPTOR_ADDR. Only the words this backend writes are named.
pub const GC_TEXDESC_BYTES: usize = 256;
/// 16 mip addresses, the first at the descriptor's offset 0
pub const fn texdesc_lod_addr(lod: u32) -> u32 {
    4 * lod
}
// Each of those levels is fetched from a 64-byte boundary; Mesa aligns its own
// levels to ETNA_PE_ALIGNMENT, which is the same 64, for the same reason.
pub const GC_TEXDESC_LOD_ALIGN: usize = 64;
pub const GC_TEXDESC_CONFIG0: u32 = 0x40;
pub const fn texdesc_type(x: u32) -> u32 {
    x & 0x7
}
pub const fn texdesc_format(x: u32) -> u32 {
    (x & 0x1f) << 13
}
pub const fn texdesc_addressing_mode(x: u32) -> u32 {
    (x & 0x3) << 20
}
pub const GC_TEX_ADDRESSING_TILED: u32 = 0;
pub const GC_TEX_ADDRESSING_LINEAR: u32 = 3;
pub const GC_TEX_TYPE_2D: u32 = 2;
pub const GC_TEX_TYPE_3D: u32 = 3;
pub const GC_TEX_TYPE_CUBE_MAP: u32 = 5;
pub const GC_TEXDESC_SIZE: u32 = 0x44;
pub const fn texdesc_size_width(x: u32) -> u32 {
    x & 0xffff
}
pub const fn texdesc_size_height(x: u32) -> u32 {
    (x & 0xffff) << 16
}
pub const GC_TEXDESC_LINEAR_STRIDE: u32 = 0x48;
pub const GC_TEXDESC_CONFIG1: u32 = 0x4c;
pub const fn texdesc_format_ext(x: u32) -> u32 {
    x & 0x3f
}
pub const fn texdesc_swizzle_r(x: u32) -> u32 {
    (x & 0x7) << 8
}
pub const fn texdesc_swizzle_g(x: u32) -> u32 {
    (x & 0x7) << 12
}
pub const fn texdesc_swizzle_b(x: u32) -> u32 {
    (x & 0x7) << 16
}
pub const fn texdesc_swizzle_a(x: u32) -> u32 {
    (x & 0x7) << 20
}
pub const fn texdesc_halign(x: u32) -> u32 {
    (x & 0x7) << 26
}
/// CONFIG1: the layers are an array, not the slices of a volume.
pub const GC_TEXDESC_TEXTURE_ARRAY: u32 = 0x0100_0000;
pub const GC_TEXDESC_CONFIG2: u32 = 0x6c;
/// always set; blob value
pub const GC_TEXDESC_CONFIG2_BASE: u32 = 0x00030000;
pub const GC_TEXDESC_BASELOD: u32 = 0x68;
pub const fn texdesc_baselod_base(x: u32) -> u32 {
    x & 0xf
}
pub const fn texdesc_baselod_max(x: u32) -> u32 {
    (x & 0xf) << 8
}
pub const GC_TEXDESC_LOG_SIZE_EXT: u32 = 0x74;
pub const fn texdesc_log_width(x: u32) -> u32 {
    x & 0xffff
}
pub const fn texdesc_log_height(x: u32) -> u32 {
    (x & 0xffff) << 16
}
pub const GC_TEXDESC_VOLUME: u32 = 0x78;
pub const GC_TEXDESC_SLICE: u32 = 0x7c;
pub const GC_TEXDESC_3D_CONFIG: u32 = 0x84;
pub const fn texdesc_3d_depth(x: u32) -> u32 {
    x & 0x3fff
}

// texel formats (TEXDESC_CONFIG0_FORMAT)
/* one and two bytes a texel (Mesa etnaviv_format.c: A8_UNORM, L8_UNORM,
 * L8A8_UNORM; common_3d.xml.h TEXTURE_FORMAT_*), sampled as (0,0,0,A),
 * (L,L,L,1) and (L,L,L,A) with no swizzle */
pub const GC_TEX_FORMAT_A8: u32 = 0x01;
pub const GC_TEX_FORMAT_L8: u32 = 0x02;
pub const GC_TEX_FORMAT_A8L8: u32 = 0x04;
pub const GC_TEX_FORMAT_A8R8G8B8: u32 = 0x07;
pub const GC_TEX_FORMAT_X8R8G8B8: u32 = 0x08;
pub const GC_TEX_FORMAT_A8B8G8R8: u32 = 0x09;
pub const GC_TEX_FORMAT_X8B8G8R8: u32 = 0x0a;
pub const GC_TEX_FORMAT_R5G6B5: u32 = 0x0b;
pub const GC_TEX_FORMAT_ETC1: u32 = 0x1e;
// the extended set (TEXDESC_CONFIG1_FORMAT_EXT, with CONFIG0 format 0)
pub const GC_TEX_FORMAT_EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2: u32 = 0x01;
pub const GC_TEX_FORMAT_EXT_RGBA8_ETC2_EAC: u32 = 0x02;
pub const GC_TEX_FORMAT_EXT_R16F: u32 = 0x07;
pub const GC_TEX_FORMAT_EXT_A16B16G16R16F: u32 = 0x09;
pub const GC_TEX_FORMAT_EXT_R32F: u32 = 0x0a;
// swizzles
pub const GC_SWIZZLE_R: u32 = 0;
pub const GC_SWIZZLE_G: u32 = 1;
pub const GC_SWIZZLE_B: u32 = 2;
pub const GC_SWIZZLE_A: u32 = 3;
pub const GC_SWIZZLE_0: u32 = 4;
pub const GC_SWIZZLE_1: u32 = 5;

// ---- the unified constant register file (HALTI5) ------------------------

/// the vertex stage's window
pub const GC_SH_HALTI5_UNIFORMS_VS: u32 = 0x00034000;
/// the fragment stage's window
pub const GC_SH_HALTI5_UNIFORMS_PS: u32 = 0x00036000;
/// vec4 registers, both stages
pub const GC_NUM_CONSTANTS: u32 = 320;

/* ---- multisampling: the sample positions the rasteriser needs ----------- *
 *
 * The values are the blob's, by way of Mesa; nothing is known about the
 * encoding beyond that 4x wants these sixteen words. */
pub const GC_RA_MULTISAMPLE_UNK00E04: u32 = 0x00000e04;
pub const fn ra_multisample_unk00e10(i: u32) -> u32 {
    0x00000e10 + 4 * i
}
pub const fn ra_centroid_table(i: u32) -> u32 {
    0x00000e40 + 4 * i
}

/* ---- the resolve engine ------------------------------------------------- *
 *
 * A separate block from the 3D pipe, driven by the same command stream: load
 * its registers, write GC_RS_KICKER, and it copies a rectangle. With
 * DOWNSAMPLE_X and DOWNSAMPLE_Y it averages each 2x2 group as it goes, which
 * is how a 4x multisampled surface becomes a single-sampled one here.
 *
 * The width must be a multiple of 16. Mesa calls abort() otherwise, with the
 * comment that the hardware scribbles over memory or hangs. */
pub const GC_RS_KICKER: u32 = 0x00001600;
/// any write kicks; the blob's
pub const GC_RS_KICKER_VALUE: u32 = 0xbeeb_beeb;
pub const GC_RS_CONFIG: u32 = 0x00001604;
pub const fn rs_source_format(x: u32) -> u32 {
    x & 0x1f
}
pub const GC_RS_DOWNSAMPLE_X: u32 = 0x00000020;
pub const GC_RS_DOWNSAMPLE_Y: u32 = 0x00000040;
pub const GC_RS_SOURCE_TILED: u32 = 0x00000080;
pub const fn rs_dest_format(x: u32) -> u32 {
    (x & 0x1f) << 8
}
pub const GC_RS_DEST_TILED: u32 = 0x00004000;
pub const GC_RS_SWAP_RB: u32 = 0x2000_0000;
/// Copy with the rows reversed (etnaviv state_3d.xml.h: VIVS_RS_CONFIG_FLIP).
/// Wired but never set by etnaviv; used only by the flip study.
pub const GC_RS_CONFIG_FLIP: u32 = 0x4000_0000;
pub const GC_RS_FLIP: u32 = 0x4000_0000;
pub const GC_RS_SOURCE_ADDR: u32 = 0x00001608;
pub const GC_RS_SOURCE_STRIDE: u32 = 0x0000160c;
/// A tiled source (etnaviv state_3d.xml.h: VIVS_RS_SOURCE_STRIDE_TILING).
pub const GC_RS_SOURCE_STRIDE_TILING: u32 = 0x8000_0000;
/// A supertiled source on a CACHE128B256BPERLINE core (VIVS_RS_SOURCE_STRIDE_SUPER_TILED_NEW).
pub const GC_RS_SOURCE_STRIDE_SUPER_TILED_NEW: u32 = 0x0800_0000;
pub const GC_RS_DEST_ADDR: u32 = 0x00001610;
pub const GC_RS_DEST_STRIDE: u32 = 0x00001614;
pub const GC_RS_WINDOW_SIZE: u32 = 0x00001620;
pub const fn rs_window_width(x: u32) -> u32 {
    x & 0xffff
}
pub const fn rs_window_height(x: u32) -> u32 {
    (x & 0xffff) << 16
}
pub const fn rs_dither(i: u32) -> u32 {
    0x00001630 + 4 * i
}
pub const GC_RS_CLEAR_CONTROL: u32 = 0x0000163c;
pub const fn rs_fill_value(i: u32) -> u32 {
    0x00001640 + 4 * i
}
pub const GC_RS_EXTRA_CONFIG: u32 = 0x000016a0;
pub const fn rs_extra_aa(x: u32) -> u32 {
    x & 0x3
}
pub const GC_RS_SINGLE_BUFFER: u32 = 0x000016b8;
pub const GC_RS_SINGLE_BUFFER_ENABLE: u32 = 0x00000001;
pub const fn rs_pipe_source_addr(i: u32) -> u32 {
    0x000016c0 + 4 * i
}
pub const fn rs_pipe_dest_addr(i: u32) -> u32 {
    0x000016e0 + 4 * i
}
pub const fn rs_pipe_offset(i: u32) -> u32 {
    0x00001700 + 4 * i
}

/// RS surface formats; the same names the pixel engine uses, different codes.
pub const GC_RS_FORMAT_A4R4G4B4: u32 = 0x00000001;
pub const GC_RS_FORMAT_R5G6B5: u32 = 0x00000004;
pub const GC_RS_FORMAT_X8R8G8B8: u32 = 0x00000005;
pub const GC_RS_FORMAT_A8R8G8B8: u32 = 0x00000006;
