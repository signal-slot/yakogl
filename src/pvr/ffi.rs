// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! FFI to the driver's OS-free command generators.
//!
//! The PPP/VDM builders and the PDS program generators STAY IN C in the driver
//! repository (powervr-bxs-am62p/src: `pvr_vdm_ppp_gen.[ch]`,
//! `pvr_pds_render_gen.[ch]`, `pvr_pds_pixel_event_gen.[ch]`): they are the
//! silicon-proven encoders the driver's own self-test render uses, and the
//! build links them into the library. This file mirrors ONLY what
//! `backend.rs` uses of those headers: structs field for field in the
//! header's order, constants as `u32`, functions as `extern "C"`.
//!
//! Drift between the driver headers and this file would corrupt the command
//! stream silently, so both sides are pinned to the same numbers: the
//! `const _` asserts below check the Rust layout, and
//! `src/pvr/ffi_layout_check.c` (compiled by the C side of the build against
//! the REAL driver headers) checks the same sizes, offsets, constant values
//! and function prototypes. Every mirrored type is built from fixed-width
//! integers, so the numbers are identical on the x86-64 host and on aarch64.
//! When a driver header changes, update the struct here and BOTH sets of
//! asserts.

use core::mem::{offset_of, size_of};

/* ---- csbgen enum values (rogue/ppp.xml, rogue/vdm.xml) --------------------- */

/// Worst-case dword count of one PPP state block (incl. the terminate block).
pub const PVR_PPP_BLOCK_MAX_DWORDS: u32 = 40;

pub const PVR_TA_OBJTYPE_TRIANGLE: u32 = 0;
pub const PVR_TA_OBJTYPE_LINE: u32 = 1;
/// Points (Mesa pvr_ta_objtype).
pub const PVR_TA_OBJTYPE_SPRITE_01UV: u32 = 4;

/* The backend hands `GlesCmp` to the TA unchanged (it is declared in
 * TA_CMPMODE order), so the whole enum is pinned, not just ALWAYS. */
pub const PVR_TA_CMPMODE_NEVER: u32 = 0;
pub const PVR_TA_CMPMODE_LESS: u32 = 1;
pub const PVR_TA_CMPMODE_EQUAL: u32 = 2;
pub const PVR_TA_CMPMODE_LESS_OR_EQUAL: u32 = 3;
pub const PVR_TA_CMPMODE_GREATER: u32 = 4;
pub const PVR_TA_CMPMODE_NOT_EQUAL: u32 = 5;
pub const PVR_TA_CMPMODE_GREATER_OR_EQUAL: u32 = 6;
pub const PVR_TA_CMPMODE_ALWAYS: u32 = 7;

/* Likewise `GlesStencilOp` is the ISPB op encoding, passed through. */
pub const PVR_TA_STENCILOP_KEEP: u32 = 0;
pub const PVR_TA_STENCILOP_ZERO: u32 = 1;
pub const PVR_TA_STENCILOP_REPLACE: u32 = 2;
pub const PVR_TA_STENCILOP_INCR_SAT: u32 = 3;
pub const PVR_TA_STENCILOP_DECR_SAT: u32 = 4;
pub const PVR_TA_STENCILOP_INVERT: u32 = 5;
pub const PVR_TA_STENCILOP_INCR: u32 = 6;
pub const PVR_TA_STENCILOP_DECR: u32 = 7;

pub const PVR_TA_CULLMODE_NONE: u32 = 0;
pub const PVR_TA_CULLMODE_CW: u32 = 1;
pub const PVR_TA_CULLMODE_CCW: u32 = 2;

/// NO_FRONT_OR_REAR.
pub const PVR_TA_CLIP_MODE_NONE: u32 = 0;
pub const PVR_TA_CLIP_MODE_FRONT_REAR: u32 = 1;
/// The third mode the driver header names. Its name distinguishes it from
/// FRONT_REAR by clipping against *depth*, which is what clipping after the
/// viewport z transform would be called -- so it is the first thing to try if
/// the hardware turns out to clip `0 <= z <= w` before that transform, where
/// GL wants `-w <= z <= w`. Nothing has selected it yet; `gl_test` case 62
/// asks the silicon which of the two is GL's.
pub const PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH: u32 = 2;

pub const PVR_TA_FLATSHADE_VERTEX_0: u32 = 1;

/* VDMCTRL_INDEX_LIST0 primitive topology (vdm.xml:117-130). */
pub const PVR_VDM_TOPOLOGY_POINT_LIST: u32 = 0;
pub const PVR_VDM_TOPOLOGY_LINE_LIST: u32 = 1;
/// vdm.xml PRIMITIVE_TOPOLOGY (2 = LINE_LIST_ADJ).
pub const PVR_VDM_TOPOLOGY_LINE_STRIP: u32 = 3;
pub const PVR_VDM_TOPOLOGY_LINE_LOOP: u32 = 5;
pub const PVR_VDM_TOPOLOGY_TRI_LIST: u32 = 6;
pub const PVR_VDM_TOPOLOGY_TRI_STRIP: u32 = 9;
pub const PVR_VDM_TOPOLOGY_TRI_FAN: u32 = 11;

/* enum pvr_ppp_passtype: TA_STATE_ISPA.passtype [24:26] (rogue/ppp.xml:89-97).
 * PUNCH_THROUGH is required for a fragment shader that kills pixels
 * (discard/texkill) so the ISP resolves tag/visibility feedback; emitting such
 * a shader as TRANSLUCENT deadlocks the ISP (3D DM HWR). */
pub const PVR_PPP_PASSTYPE_OPAQUE: u32 = 0;
pub const PVR_PPP_PASSTYPE_TRANSLUCENT: u32 = 1;
pub const PVR_PPP_PASSTYPE_PUNCH_THROUGH: u32 = 2;

pub const PVR_PDS_VERTEX_MAX_STREAMS: u32 = 16;
/// The most USC temporaries a DOUTU word declares (six bits of four-temp
/// blocks); every PDS program generator refuses more.
pub const PVR_PDS_DOUTU_MAX_TEMPS: u32 = 252;
/// `pvr_pds_render_vertex_program_ids`: no vertex / instance id register.
pub const PVR_PDS_NO_ID_REG: u32 = 0xff;

/* enum pvr_pds_coeff_interp */
/// Perspective-correct (GOURAUD, perspective=1).
pub const PVR_PDS_COEFF_SMOOTH: u32 = 0;
/// Linear in screen space (GOURAUD, perspective=0).
pub const PVR_PDS_COEFF_NPC: u32 = 1;
/// FLAT_VERTEX0.
pub const PVR_PDS_COEFF_FLAT: u32 = 2;

pub const PVR_PDS_COEFF_MAX_VARYINGS: u32 = 16;
pub const PVR_PDS_DMA_MAX_KICKS: u32 = 8;

/* ---- pvr_vdm_ppp_gen.h ------------------------------------------------------ */

/// `struct pvr_ppp_frag_inputs`: the fragment-PDS bindings of the PPP
/// TA_STATE_PDS_* block (all offsets PDS-heap relative, in bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrPppFragInputs {
    /// The fragment kick-USC PDS program (DATA start; the code follows it)
    /// -> TA_STATE_PDS_SHADERBASE.
    pub pixel_program_code_offset: u32,
    /// The texture-state / uniform DMA PDS code -> TA_STATE_PDS_TEXUNICODEBASE.
    pub texture_program_code_offset: u32,
    /// ... and its DATA segment -> TA_STATE_PDS_TEXTUREDATABASE.
    pub texture_program_data_offset: u32,
    pub texture_program_data_size_bytes: u32,
    pub texture_program_temps: u32,
    /// Shared-register dwords the fragment USC reads (TA_STATE_PDS_SIZEINFO2).
    pub shared_reg_count: u32,
    /// TA_STATE_ISPCTL.upass; 0 for bring-up.
    pub isp_userpass: u32,
    /// `PVR_PPP_PASSTYPE_*` (ISPA.passtype).
    pub passtype: u8,
    /// The coefficient-loading (DOUTI) program -> TA_STATE_PDS_VARYINGBASE;
    /// 0 = the dummy program at PDS offset 0 (no varying iterated).
    pub varying_program_code_offset: u32,
}

impl PvrPppFragInputs {
    pub const ZERO: PvrPppFragInputs = PvrPppFragInputs {
        pixel_program_code_offset: 0,
        texture_program_code_offset: 0,
        texture_program_data_offset: 0,
        texture_program_data_size_bytes: 0,
        texture_program_temps: 0,
        shared_reg_count: 0,
        isp_userpass: 0,
        passtype: 0,
        varying_program_code_offset: 0,
    };
}

/// `struct pvr_ppp_state`: the per-draw render state of the PPP block.
/// Zero-initialise, then set what differs (C `int` flags are `i32`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrPppState {
    /// `PVR_TA_OBJTYPE_*` (default TRIANGLE).
    pub objtype: u32,
    /// ISPA.pointlinewidth: 4.4 fixed point minus 1 (15 = 1 px, 255 = 16 px).
    pub point_line_width: u32,
    /// SIZEINFO2.pds_tri_merge_disable: required for lines and points.
    pub tri_merge_disable: i32,
    /// `PVR_TA_CMPMODE_*`. Zero is NOT a valid GL state: "no depth" is
    /// ALWAYS + depth_write 0.
    pub depth_cmp: u32,
    pub depth_write: i32,
    /// ISPCTL.tagwritedisable: depth/stencil only, no colour.
    pub color_write_disable: i32,
    /* ISPB stencil for the front face (both faces unless two_sided) */
    pub stencil_cmp: u32,
    pub stencil_ref: u32,
    pub stencil_sop_fail: u32,
    pub stencil_sop_dfail: u32,
    pub stencil_sop_pass: u32,
    pub stencil_cmp_mask: u32,
    pub stencil_write_mask: u32,
    /// ISPCTL.two_sided + a second ISPA/ISPB pair: the front words apply to
    /// triangles that are clockwise in the TA's winding, the back words to
    /// counter-clockwise ones.
    pub two_sided: i32,
    /// ISPCTL.dbenable + a STATE_ISPDBSC word naming the entry of the frag
    /// kick's ISP_DBIAS_BASE table. Triangles only.
    pub dbias_enable: i32,
    pub dbias_index: u32,
    /// Pixel scissor (GL scissor test): ISPCTL.scenable + the scindex half of
    /// the STATE_ISPDBSC word, naming a 2-dword entry of the frag kick's
    /// ISP_SCISSOR_BASE table (IPF_SCISSOR_WORD_0/1). Used with the region
    /// clip, which only drops whole tiles.
    pub scissor_enable: i32,
    pub scissor_index: u32,
    pub stencil_back_cmp: u32,
    pub stencil_back_ref: u32,
    pub stencil_back_sop_fail: u32,
    pub stencil_back_sop_dfail: u32,
    pub stencil_back_sop_pass: u32,
    pub stencil_back_cmp_mask: u32,
    pub stencil_back_write_mask: u32,
    /// TA_REGION_CLIP (tile granular, inclusive tile indices); 0 = whole screen.
    pub clip_enable: i32,
    pub clip_left: u32,
    pub clip_right: u32,
    pub clip_top: u32,
    pub clip_bottom: u32,
    /// Hardware viewport transform: 6 f32 bit patterns (x_center, x_scale,
    /// y_center, y_scale, z_center, z_scale); 0 = not emitted.
    pub viewport_enable: i32,
    pub viewport: [u32; 6],
    /// VS output vector size in dwords (TA_OUTPUT_SEL).
    pub vtxsize: u32,
    /// 1/w plane present (the FS uses w).
    pub rhw_pres: i32,
    /// OUTPUT_SEL: the VS emits a point size (sprites).
    pub psprite_size_pres: i32,
    /* TA_STATE_VARYING0: varying f32 component counts */
    pub f32_linear: u32,
    pub f32_flat: u32,
    pub f32_npc: u32,
    /* TA_STATE_PDS_SIZEINFO1 varying sizes (units: usc 16 dw, pds 4 dw) */
    pub usc_varyingsize: u32,
    pub pds_varyingsize: u32,
    /* TA_STATE_PPP_CTRL */
    /// 1 = screen-space input.
    pub pretransform: i32,
    pub cullmode: u32,
    pub clip_mode: u32,
    pub drawclippededges: i32,
    pub wclampen: i32,
    /// `PVR_TA_FLATSHADE_*` or 0.
    pub flatshade_vtx: u32,
    /// OUTPUT_SEL tsp_unclamped_z_pres: the FS iterates the Z plane.
    pub z_pres: i32,
    /// ISPCTL.vistest: the ISP counts the object's passing samples into
    /// dword `vis_reg` (14 bits) of the frag kick's ISP_OCLQRY_BASE buffer.
    pub vis_test: i32,
    pub vis_reg: u32,
}

impl PvrPppState {
    pub const ZERO: PvrPppState = PvrPppState {
        objtype: 0,
        point_line_width: 0,
        tri_merge_disable: 0,
        depth_cmp: 0,
        depth_write: 0,
        color_write_disable: 0,
        stencil_cmp: 0,
        stencil_ref: 0,
        stencil_sop_fail: 0,
        stencil_sop_dfail: 0,
        stencil_sop_pass: 0,
        stencil_cmp_mask: 0,
        stencil_write_mask: 0,
        two_sided: 0,
        dbias_enable: 0,
        dbias_index: 0,
        scissor_enable: 0,
        scissor_index: 0,
        stencil_back_cmp: 0,
        stencil_back_ref: 0,
        stencil_back_sop_fail: 0,
        stencil_back_sop_dfail: 0,
        stencil_back_sop_pass: 0,
        stencil_back_cmp_mask: 0,
        stencil_back_write_mask: 0,
        clip_enable: 0,
        clip_left: 0,
        clip_right: 0,
        clip_top: 0,
        clip_bottom: 0,
        viewport_enable: 0,
        viewport: [0; 6],
        vtxsize: 0,
        rhw_pres: 0,
        psprite_size_pres: 0,
        f32_linear: 0,
        f32_flat: 0,
        f32_npc: 0,
        usc_varyingsize: 0,
        pds_varyingsize: 0,
        pretransform: 0,
        cullmode: 0,
        clip_mode: 0,
        drawclippededges: 0,
        wclampen: 0,
        flatshade_vtx: 0,
        z_pres: 0,
        vis_test: 0,
        vis_reg: 0,
    };
}

/// `struct pvr_vdm_vs_state`: vertex pipeline state for VDMCTRL_VDM_STATE0/2/3/4/5.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PvrVdmVsState {
    /// PDS-heap byte offset of the vertex PDS DATA (16-aligned).
    pub vs_pds_data_offset: u32,
    /// ... and CODE.
    pub vs_pds_code_offset: u32,
    pub vs_pds_data_size_bytes: u32,
    pub vs_pds_temps: u32,
    pub vs_output_size_bytes: u32,
    /// VS input vertex size in bytes: every vtxin register the vertex PDS
    /// program DMAs into the unified store (highest register + 1, times 4);
    /// VDM_STATE5.vs_usc_unified_size is sized from it. 0 = the output size.
    pub vs_input_size_bytes: u32,
    /// VDM_STATE0.flatshade_control (0 = VERTEX_0).
    pub flatshade_control: u32,
}

/// `struct pvr_vdm_pds_state`: a PDS program the VDM runs on a state update to
/// fill the vertex shader's common store (vertex uniforms via DOUTD).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PvrVdmPdsState {
    pub pds_data_offset: u32,
    pub pds_code_offset: u32,
    pub pds_data_size_bytes: u32,
    /// PDS temps the program uses (dwords).
    pub pds_temps: u32,
    /// Shared registers the DMA fills (bytes).
    pub usc_common_size_bytes: u32,
}

/// `struct pvr_vdm_field_refusal`: the size field a checked emitter refused
/// (it wrote nothing): its name, the value in the field's units, the largest
/// value that fits and the unit in bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrVdmFieldRefusal {
    /// A static NUL-terminated name such as `VDM_STATE5.vs_pds_data_size`,
    /// or null when nothing was refused.
    pub field: *const core::ffi::c_char,
    pub value: u64,
    pub max: u32,
    pub unit_bytes: u32,
}

impl PvrVdmFieldRefusal {
    pub const ZERO: PvrVdmFieldRefusal =
        PvrVdmFieldRefusal { field: core::ptr::null(), value: 0, max: 0, unit_bytes: 0 };
}

/// `PVR_VDM_FIELD_REFUSED`: a checked emitter's return when a size field
/// does not fit its bits.
pub const PVR_VDM_FIELD_REFUSED: i32 = -1;

/* ---- pvr_pds_render_gen.h --------------------------------------------------- */

/// `struct pvr_pds_render_vertex_info`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PvrPdsRenderVertexInfo {
    pub data_size_dw: u32,
    pub code_size_dw: u32,
    pub temps_used: u32,
    pub vdm_pds_data_size_units: u32,
    pub vdm_pds_temp_size_units: u32,
    pub vdm_usc_unified_size_units: u32,
}

/// `struct pvr_pds_render_kick_usc_info`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PvrPdsRenderKickUscInfo {
    /// Always 8 (ROGUE_PDS_FIXED_PIXEL_SHADER_DATA_SIZE).
    pub data_size_dw: u32,
    /// Always 1 (single DOUTU).
    pub code_size_dw: u32,
    pub doutu_offset: u32,
}

/// `struct pvr_pds_render_color_upload_info`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PvrPdsRenderColorUploadInfo {
    pub data_size_dw: u32,
    pub code_size_dw: u32,
    pub temps_used: u32,
}

/// `struct pvr_pds_render_stream`: one DMA stream of the vertex-fetch program.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrPdsVertexStream {
    /// GPU VA of the first vertex of this attribute.
    pub address: u64,
    /// Bytes between vertices.
    pub stride: u32,
    /// Bytes DMAed per vertex (rounded up to dwords).
    pub dma_size: u32,
    /// First vtxin (vi) register.
    pub dst_reg: u32,
    /// 1: one element per instance (GL divisor 1), indexed by the instance
    /// number (IR1) instead of the vertex index (IR0).
    pub per_instance: u32,
}

impl PvrPdsVertexStream {
    pub const ZERO: PvrPdsVertexStream =
        PvrPdsVertexStream { address: 0, stride: 0, dma_size: 0, dst_reg: 0, per_instance: 0 };
}

/// `struct pvr_pds_coeff_varying`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrPdsCoeffVarying {
    /// 1..4 f32 components.
    pub components: u32,
    /// `PVR_PDS_COEFF_*`.
    pub interp: u32,
    /// Dword offset of the varying in the VS output vector.
    pub vtxout_off: u32,
}

impl PvrPdsCoeffVarying {
    pub const ZERO: PvrPdsCoeffVarying = PvrPdsCoeffVarying { components: 0, interp: 0, vtxout_off: 0 };
}

/// `struct pvr_pds_coeff_info`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PvrPdsCoeffInfo {
    pub data_size_dw: u32,
    pub code_size_dw: u32,
    pub coeff_dwords: u32,
    /// TA_STATE_PDS_SIZEINFO1.pds_varyingsize (units of 4 dw).
    pub pds_varyingsize: u32,
    /// TA_STATE_PDS_SIZEINFO1.usc_varyingsize (units of 16 dw).
    pub usc_varyingsize: u32,
}

/// `struct pvr_pds_dma_kick`: DMA `dwords` from `address` into the USC common
/// store at attribute offset `dest_offset`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PvrPdsDmaKick {
    pub address: u64,
    pub dwords: u32,
    pub dest_offset: u32,
}

impl PvrPdsDmaKick {
    pub const ZERO: PvrPdsDmaKick = PvrPdsDmaKick { address: 0, dwords: 0, dest_offset: 0 };
}

extern "C" {
    /* pvr_vdm_ppp_gen.h. Each emitter writes at `dw` and returns the number
     * of dwords written. */

    /// Emit the full TA_STATE_* PPP state block; `out` holds at least
    /// `PVR_PPP_BLOCK_MAX_DWORDS`. Returns the dword count (also `*out_count`),
    /// or [`PVR_VDM_FIELD_REFUSED`] when a size field does not fit (nothing
    /// written, `*out_count` 0, `why` filled).
    pub fn pvr_ppp_build_block(
        input: *const PvrPppFragInputs,
        st: *const PvrPppState,
        out: *mut u32,
        out_count: *mut u32,
        why: *mut PvrVdmFieldRefusal,
    ) -> i32;
    /// The 3-word PPP terminate block for a w x h render target: 3, or
    /// [`PVR_VDM_FIELD_REFUSED`] when the last 32-px block does not fit
    /// clip_right/clip_bottom (nothing written, `why` filled).
    pub fn pvr_ppp_build_terminate_block(width: u32, height: u32, out: *mut u32, why: *mut PvrVdmFieldRefusal) -> i32;
    /// One 2-dword entry of the frag kick's ISP_SCISSOR_BASE table
    /// (`IPF_SCISSOR_WORD_0/1`, Mesa csbgen rogue/ipf.xml:150-166): 2, or
    /// [`PVR_VDM_FIELD_REFUSED`] when a coordinate does not fit its 16-bit
    /// field (nothing written, `why` filled).
    pub fn pvr_isp_build_scissor_word(
        xmin: u32,
        xmax: u32,
        ymin: u32,
        ymax: u32,
        out: *mut u32,
        why: *mut PvrVdmFieldRefusal,
    ) -> i32;
    /// 2 dwords.
    /// ... or [`PVR_VDM_FIELD_REFUSED`] (nothing written, `why` filled).
    pub fn pvr_vdm_emit_ppp_state(
        dw: *mut u32,
        ppp_dev_addr: u64,
        word_count: u32,
        why: *mut PvrVdmFieldRefusal,
    ) -> i32;
    /// 3 dwords.
    /// ... or [`PVR_VDM_FIELD_REFUSED`] (nothing written, `why` filled).
    pub fn pvr_vdm_emit_pds_state(dw: *mut u32, ps: *const PvrVdmPdsState, why: *mut PvrVdmFieldRefusal) -> i32;
    /// 5 dwords.
    /// ... or [`PVR_VDM_FIELD_REFUSED`] (nothing written, `why` filled).
    pub fn pvr_vdm_emit_vdm_state(dw: *mut u32, vs: *const PvrVdmVsState, why: *mut PvrVdmFieldRefusal) -> i32;
    /// `index_buf_dev_addr != 0`: indexed u16 draw (3 dwords); 0: non-indexed (2).
    /// ... or [`PVR_VDM_FIELD_REFUSED`] (nothing written, `why` filled).
    pub fn pvr_vdm_emit_index_list(
        dw: *mut u32,
        topology: u32,
        index_buf_dev_addr: u64,
        index_count: u32,
        why: *mut PvrVdmFieldRefusal,
    ) -> i32;
    /// As above for `instance_count` instances (> 1 adds INDEX_LIST3 = count - 1:
    /// one dword more).
    pub fn pvr_vdm_emit_index_list_instanced(
        dw: *mut u32,
        topology: u32,
        index_buf_dev_addr: u64,
        index_count: u32,
        instance_count: u32,
        why: *mut PvrVdmFieldRefusal,
    ) -> i32;
    /// 1 dword.
    pub fn pvr_vdm_emit_terminate(dw: *mut u32) -> u32;

    /* pvr_pds_render_gen.h. All return 0 on success, non-zero on bad input;
     * data/code are little-endian u32 arrays. */

    /// Fragment kick-USC PDS program: `data_out` >= 8 dwords, `code_out` >= 1.
    pub fn pvr_pds_render_kick_usc_program(
        fs_usc_exec_offset: u64,
        fs_temps: u32,
        sample_rate: u32,
        data_out: *mut u32,
        code_out: *mut u32,
        info: *mut PvrPdsRenderKickUscInfo,
    ) -> i32;
    /// Single-kick colour upload: `data_out` >= 3 dwords, `code_out` >= 2.
    pub fn pvr_pds_render_color_upload_program(
        color_buf_addr: u64,
        dma_dword_count: u32,
        dest_offset: u32,
        has_slc_mcu_cache_controls: i32,
        data_out: *mut u32,
        code_out: *mut u32,
        info: *mut PvrPdsRenderColorUploadInfo,
    ) -> i32;
    /// Vertex-fetch program, one DMA per stream, also writing the vertex
    /// index (IR0) and/or the instance index (IR1) into the vtxin registers
    /// of a shader that reads gl_VertexID / gl_InstanceID
    /// (`PVR_PDS_NO_ID_REG`: not written): `data_out` >= 8 * streams (+ 8
    /// with an id) dwords, `code_out` >= streams + 2 (+ 2 with an id).
    pub fn pvr_pds_render_vertex_program_ids(
        vs_usc_exec_offset: u64,
        vs_temps: u32,
        streams: *const PvrPdsVertexStream,
        num_streams: u32,
        has_slc_mcu_cache_controls: i32,
        vertex_id_reg: u32,
        instance_id_reg: u32,
        data_out: *mut u32,
        code_out: *mut u32,
        info: *mut PvrPdsRenderVertexInfo,
    ) -> i32;
    /// Coefficient (DOUTI) program: `data_out` >= 2 * (varyings + 2) + 2
    /// dwords (padded to 4), `code_out` >= varyings + 2 (W and the point
    /// coordinate ahead of the varyings).
    pub fn pvr_pds_render_coeff_program(
        uses_w: i32,
        uses_z: i32,
        uses_pntc: i32,
        varyings: *const PvrPdsCoeffVarying,
        num_varyings: u32,
        data_out: *mut u32,
        code_out: *mut u32,
        info: *mut PvrPdsCoeffInfo,
    ) -> i32;
    /// Common-store DMA program with N kicks: `data_out` >= 3 * kicks dwords,
    /// `code_out` >= kicks + 1.
    pub fn pvr_pds_render_dma_program(
        kicks: *const PvrPdsDmaKick,
        num_kicks: u32,
        has_slc_mcu_cache_controls: i32,
        data_out: *mut u32,
        code_out: *mut u32,
        info: *mut PvrPdsRenderColorUploadInfo,
    ) -> i32;

    /* pvr_pds_pixel_event_gen.h */

    /// Pixel-event (EOT) program: `emit_words` 2 * num_rt dwords, `data_out`
    /// >= 3 * num_rt + 1 dwords, `code_out` >= num_rt + 1.
    pub fn pvr_pds_generate_pixel_event_tq(
        doutu_src0: u64,
        emit_words: *const u32,
        num_rt: u32,
        has_slc_mcu_cache_controls: i32,
        data_out: *mut u32,
        code_out: *mut u32,
        data_size_dw: *mut u32,
        code_size_dw: *mut u32,
    ) -> i32;
}

/* ---- layout pins (the same numbers as ffi_layout_check.c) ------------------ */

const _: () = assert!(size_of::<PvrPppFragInputs>() == 36);
const _: () = assert!(offset_of!(PvrPppFragInputs, pixel_program_code_offset) == 0);
const _: () = assert!(offset_of!(PvrPppFragInputs, texture_program_code_offset) == 4);
const _: () = assert!(offset_of!(PvrPppFragInputs, texture_program_data_offset) == 8);
const _: () = assert!(offset_of!(PvrPppFragInputs, texture_program_data_size_bytes) == 12);
const _: () = assert!(offset_of!(PvrPppFragInputs, texture_program_temps) == 16);
const _: () = assert!(offset_of!(PvrPppFragInputs, shared_reg_count) == 20);
const _: () = assert!(offset_of!(PvrPppFragInputs, isp_userpass) == 24);
const _: () = assert!(offset_of!(PvrPppFragInputs, passtype) == 28);
const _: () = assert!(offset_of!(PvrPppFragInputs, varying_program_code_offset) == 32);

const _: () = assert!(size_of::<PvrPppState>() == 216);
const _: () = assert!(offset_of!(PvrPppState, objtype) == 0);
const _: () = assert!(offset_of!(PvrPppState, point_line_width) == 4);
const _: () = assert!(offset_of!(PvrPppState, tri_merge_disable) == 8);
const _: () = assert!(offset_of!(PvrPppState, depth_cmp) == 12);
const _: () = assert!(offset_of!(PvrPppState, depth_write) == 16);
const _: () = assert!(offset_of!(PvrPppState, color_write_disable) == 20);
const _: () = assert!(offset_of!(PvrPppState, stencil_cmp) == 24);
const _: () = assert!(offset_of!(PvrPppState, stencil_ref) == 28);
const _: () = assert!(offset_of!(PvrPppState, stencil_sop_fail) == 32);
const _: () = assert!(offset_of!(PvrPppState, stencil_sop_dfail) == 36);
const _: () = assert!(offset_of!(PvrPppState, stencil_sop_pass) == 40);
const _: () = assert!(offset_of!(PvrPppState, stencil_cmp_mask) == 44);
const _: () = assert!(offset_of!(PvrPppState, stencil_write_mask) == 48);
const _: () = assert!(offset_of!(PvrPppState, two_sided) == 52);
const _: () = assert!(offset_of!(PvrPppState, dbias_enable) == 56);
const _: () = assert!(offset_of!(PvrPppState, dbias_index) == 60);
const _: () = assert!(offset_of!(PvrPppState, scissor_enable) == 64);
const _: () = assert!(offset_of!(PvrPppState, scissor_index) == 68);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_cmp) == 72);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_ref) == 76);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_sop_fail) == 80);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_sop_dfail) == 84);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_sop_pass) == 88);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_cmp_mask) == 92);
const _: () = assert!(offset_of!(PvrPppState, stencil_back_write_mask) == 96);
const _: () = assert!(offset_of!(PvrPppState, clip_enable) == 100);
const _: () = assert!(offset_of!(PvrPppState, clip_left) == 104);
const _: () = assert!(offset_of!(PvrPppState, clip_right) == 108);
const _: () = assert!(offset_of!(PvrPppState, clip_top) == 112);
const _: () = assert!(offset_of!(PvrPppState, clip_bottom) == 116);
const _: () = assert!(offset_of!(PvrPppState, viewport_enable) == 120);
const _: () = assert!(offset_of!(PvrPppState, viewport) == 124);
const _: () = assert!(offset_of!(PvrPppState, vtxsize) == 148);
const _: () = assert!(offset_of!(PvrPppState, rhw_pres) == 152);
const _: () = assert!(offset_of!(PvrPppState, psprite_size_pres) == 156);
const _: () = assert!(offset_of!(PvrPppState, f32_linear) == 160);
const _: () = assert!(offset_of!(PvrPppState, f32_flat) == 164);
const _: () = assert!(offset_of!(PvrPppState, f32_npc) == 168);
const _: () = assert!(offset_of!(PvrPppState, usc_varyingsize) == 172);
const _: () = assert!(offset_of!(PvrPppState, pds_varyingsize) == 176);
const _: () = assert!(offset_of!(PvrPppState, pretransform) == 180);
const _: () = assert!(offset_of!(PvrPppState, cullmode) == 184);
const _: () = assert!(offset_of!(PvrPppState, clip_mode) == 188);
const _: () = assert!(offset_of!(PvrPppState, drawclippededges) == 192);
const _: () = assert!(offset_of!(PvrPppState, wclampen) == 196);
const _: () = assert!(offset_of!(PvrPppState, flatshade_vtx) == 200);
const _: () = assert!(offset_of!(PvrPppState, z_pres) == 204);
const _: () = assert!(offset_of!(PvrPppState, vis_test) == 208);
const _: () = assert!(offset_of!(PvrPppState, vis_reg) == 212);

const _: () = assert!(size_of::<PvrVdmVsState>() == 28);
const _: () = assert!(offset_of!(PvrVdmVsState, vs_pds_data_offset) == 0);
const _: () = assert!(offset_of!(PvrVdmVsState, vs_pds_code_offset) == 4);
const _: () = assert!(offset_of!(PvrVdmVsState, vs_pds_data_size_bytes) == 8);
const _: () = assert!(offset_of!(PvrVdmVsState, vs_pds_temps) == 12);
const _: () = assert!(offset_of!(PvrVdmVsState, vs_output_size_bytes) == 16);
const _: () = assert!(offset_of!(PvrVdmVsState, vs_input_size_bytes) == 20);
const _: () = assert!(offset_of!(PvrVdmVsState, flatshade_control) == 24);

const _: () = assert!(size_of::<PvrVdmPdsState>() == 20);
const _: () = assert!(offset_of!(PvrVdmPdsState, pds_data_offset) == 0);
const _: () = assert!(offset_of!(PvrVdmPdsState, pds_code_offset) == 4);
const _: () = assert!(offset_of!(PvrVdmPdsState, pds_data_size_bytes) == 8);
const _: () = assert!(offset_of!(PvrVdmPdsState, pds_temps) == 12);
const _: () = assert!(offset_of!(PvrVdmPdsState, usc_common_size_bytes) == 16);

const _: () = assert!(size_of::<PvrVdmFieldRefusal>() == 24);
const _: () = assert!(offset_of!(PvrVdmFieldRefusal, field) == 0);
const _: () = assert!(offset_of!(PvrVdmFieldRefusal, value) == 8);
const _: () = assert!(offset_of!(PvrVdmFieldRefusal, max) == 16);
const _: () = assert!(offset_of!(PvrVdmFieldRefusal, unit_bytes) == 20);

const _: () = assert!(size_of::<PvrPdsRenderVertexInfo>() == 24);
const _: () = assert!(offset_of!(PvrPdsRenderVertexInfo, data_size_dw) == 0);
const _: () = assert!(offset_of!(PvrPdsRenderVertexInfo, code_size_dw) == 4);
const _: () = assert!(offset_of!(PvrPdsRenderVertexInfo, temps_used) == 8);
const _: () = assert!(offset_of!(PvrPdsRenderVertexInfo, vdm_pds_data_size_units) == 12);
const _: () = assert!(offset_of!(PvrPdsRenderVertexInfo, vdm_pds_temp_size_units) == 16);
const _: () = assert!(offset_of!(PvrPdsRenderVertexInfo, vdm_usc_unified_size_units) == 20);

const _: () = assert!(size_of::<PvrPdsRenderKickUscInfo>() == 12);
const _: () = assert!(offset_of!(PvrPdsRenderKickUscInfo, data_size_dw) == 0);
const _: () = assert!(offset_of!(PvrPdsRenderKickUscInfo, code_size_dw) == 4);
const _: () = assert!(offset_of!(PvrPdsRenderKickUscInfo, doutu_offset) == 8);

const _: () = assert!(size_of::<PvrPdsRenderColorUploadInfo>() == 12);
const _: () = assert!(offset_of!(PvrPdsRenderColorUploadInfo, data_size_dw) == 0);
const _: () = assert!(offset_of!(PvrPdsRenderColorUploadInfo, code_size_dw) == 4);
const _: () = assert!(offset_of!(PvrPdsRenderColorUploadInfo, temps_used) == 8);

const _: () = assert!(size_of::<PvrPdsVertexStream>() == 24);
const _: () = assert!(offset_of!(PvrPdsVertexStream, address) == 0);
const _: () = assert!(offset_of!(PvrPdsVertexStream, stride) == 8);
const _: () = assert!(offset_of!(PvrPdsVertexStream, dma_size) == 12);
const _: () = assert!(offset_of!(PvrPdsVertexStream, dst_reg) == 16);
const _: () = assert!(offset_of!(PvrPdsVertexStream, per_instance) == 20);

const _: () = assert!(size_of::<PvrPdsCoeffVarying>() == 12);
const _: () = assert!(offset_of!(PvrPdsCoeffVarying, components) == 0);
const _: () = assert!(offset_of!(PvrPdsCoeffVarying, interp) == 4);
const _: () = assert!(offset_of!(PvrPdsCoeffVarying, vtxout_off) == 8);

const _: () = assert!(size_of::<PvrPdsCoeffInfo>() == 20);
const _: () = assert!(offset_of!(PvrPdsCoeffInfo, data_size_dw) == 0);
const _: () = assert!(offset_of!(PvrPdsCoeffInfo, code_size_dw) == 4);
const _: () = assert!(offset_of!(PvrPdsCoeffInfo, coeff_dwords) == 8);
const _: () = assert!(offset_of!(PvrPdsCoeffInfo, pds_varyingsize) == 12);
const _: () = assert!(offset_of!(PvrPdsCoeffInfo, usc_varyingsize) == 16);

const _: () = assert!(size_of::<PvrPdsDmaKick>() == 16);
const _: () = assert!(offset_of!(PvrPdsDmaKick, address) == 0);
const _: () = assert!(offset_of!(PvrPdsDmaKick, dwords) == 8);
const _: () = assert!(offset_of!(PvrPdsDmaKick, dest_offset) == 12);
