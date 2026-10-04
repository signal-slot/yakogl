// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The driver render service's types (`pvr_render.h` in the powervr-bxs-am62p
//! repository): the private contract between the PowerVR backend and the
//! OSAL. `OsalOps::submit` takes a [`PvrRenderKick`]; `OsalOps::query`
//! (`OSAL_QUERY_RENDER_STATIC`) fills a [`PvrRenderStatic`]. The OSAL forwards
//! both to the driver and never interprets the registers.

/* enum pvr_render_heap */
pub const PVR_RENDER_HEAP_GENERAL: usize = 0;
pub const PVR_RENDER_HEAP_PDS: usize = 1;
pub const PVR_RENDER_HEAP_USC: usize = 2;

/// PDS-heap offset of the static EOT (pixel-event) PDS code the firmware runs.
pub const PVR_RENDER_EOT_PDS_CODE_OFF: u32 = 0x80;

/// Colour attachments one render may write. The hardware allows eight; the
/// end-of-tile programs captured for this driver cover three, which is what a
/// Quick 3D G-buffer pass asks for.
pub const PVR_RENDER_MAX_RT: usize = 3;

/// Tile buffers: one per colour attachment that does not fit the pixel's four
/// output registers. Three RGBA16F attachments (a Quick 3D G-buffer) spill one.
pub const PVR_RENDER_TILE_BUFFERS: usize = 2;

/// One tile's worth of spilled outputs: `tile_size_x` (16) * `tile_size_y`
/// (16) * `max_partitions` (16) * `usc_min_output_registers_per_pix` (2)
/// dwords, times the one cluster this core has (Mesa
/// `pvr_get_tile_buffer_size_per_core`). The driver allocates the buffers; the
/// backend only hands their addresses to the shader.
pub const PVR_TILE_BUFFER_BYTES: u32 = 8192 * 4;

/// `struct pvr_render_static`.
///
/// The `*_n` arrays are indexed by the colour-attachment count (1..=3): one
/// end-of-tile program per count, each emitting that many surfaces. The `_tb_`
/// pair is the same for a render whose LAST attachment comes from a tile
/// buffer (the shader stored it there because a pixel has only four output
/// registers, and the program loads it back before emitting it).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct PvrRenderStatic {
    pub eot_doutu_src0: u64,
    pub event_pixel_pds_info: u32,
    pub eot_doutu_src0_n: [u64; PVR_RENDER_MAX_RT + 1],
    pub event_pixel_pds_info_n: [u32; PVR_RENDER_MAX_RT + 1],
    pub eot_doutu_src0_tb_n: [u64; PVR_RENDER_MAX_RT + 1],
    pub event_pixel_pds_info_tb_n: [u32; PVR_RENDER_MAX_RT + 1],
    /// Tile buffers the fragment shader may store outputs into; their
    /// addresses go to the shared registers the shader reads them from.
    pub tile_buffer_gpu: [u64; PVR_RENDER_TILE_BUFFERS],
    pub border_gpu: u64,
    pub heap_base: [u64; 3],
}

/// `struct pvr_render_geom_regs`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct PvrRenderGeomRegs {
    pub vdm_ctrl_stream_base: u64,
    pub ppp_ctrl: u32,
    pub te_psg: u32,
}

/// `struct pvr_render_frag_regs`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct PvrRenderFragRegs {
    pub usc_pixel_output_ctrl: u32,
    pub usc_clear_register: [u32; 4],
    pub isp_bgobjdepth: u32,
    pub isp_bgobjvals: u32,
    pub isp_aa: u32,
    pub isp_ctl: u32,
    pub event_pixel_pds_info: u32,
    pub event_pixel_pds_data: u32,
    /// PBE REG_WORD0/1 per colour attachment (the firmware's 3D registers hold
    /// `pbe_word[8][3]`; this driver fills the first two words of the first
    /// [`PVR_RENDER_MAX_RT`]). `num_rt` 0 means one.
    pub pbe_word: [[u64; 2]; PVR_RENDER_MAX_RT],
    pub num_rt: u32,
    pub pds_bgnd: [u64; 3],
    /// CR_ISP_SCISSOR_BASE: the scissor table (2 dwords per entry).
    pub isp_scissor_base: u64,
    pub isp_dbias_base: u64,
    pub isp_zlsctl: u64,
    pub isp_zload_store_base: u64,
    pub isp_stencil_load_store_base: u64,
    pub cmd_flags: u32,
}

/// `struct pvr_render_kick`: one GEOM kick + one FRAG kick on one HWRT dataset.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct PvrRenderKick {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub geom: PvrRenderGeomRegs,
    pub frag: PvrRenderFragRegs,
}
