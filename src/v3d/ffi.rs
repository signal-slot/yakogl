// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! FFI to the V3D job builder in `driver/src/v3d/` (`v3d_job.h`, and the
//! program and frame types of `v3d_frame.h` it takes).
//!
//! The builder stays in C for the reason `docs/online-shaderc/v3d-backend.md`
//! ("M6") gives: its packets are checked byte for byte against Mesa's packers
//! (`tools/v3d/cl_golden.c`) and whole jobs are decoded by Mesa's own CLE
//! (`tools/v3d/job_check.c`, which also holds a one-draw job to the M5 frame
//! that drew on the board). A Rust port would lose both checks. This file
//! mirrors ONLY what `backend.rs` uses of those headers: the structs field for
//! field in the header's order, the constants as `u32`, the functions as
//! `extern "C"`.
//!
//! Drift between the headers and this file would corrupt the control lists
//! silently, so both sides are pinned to the same numbers: the `const _`
//! asserts below check the Rust layout, and `src/v3d/ffi_layout_check.c`
//! (compiled against the REAL headers by `tools/check.sh` and by the image
//! build) checks the same sizes, offsets and constants. The numbers were
//! measured with the host C compiler (LP64, every type fixed-width), so they
//! are the aarch64 numbers too. When a header changes, update the struct
//! here and BOTH sets of asserts.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};

/* ---- v3d_frame.h ----------------------------------------------------------- */

/// Mesa's `enum quniform_contents`, the values the builder writes.
pub const V3D_QUNIFORM_CONSTANT: u32 = 0;
pub const V3D_QUNIFORM_UNIFORM: u32 = 1;
pub const V3D_QUNIFORM_VIEWPORT_X_SCALE: u32 = 2;
pub const V3D_QUNIFORM_VIEWPORT_Y_SCALE: u32 = 3;
pub const V3D_QUNIFORM_VIEWPORT_Z_OFFSET: u32 = 4;
pub const V3D_QUNIFORM_VIEWPORT_Z_SCALE: u32 = 5;

/// `struct v3d_uniform`: one uniform stream word, `(contents, data)`. The
/// same layout as [`crate::shaderc::ShcV3dUniform`], which the backend hands
/// over by pointer (pinned below).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V3dUniform {
    pub contents: u32,
    pub data: u32,
}

/// `struct v3d_viewport`: gallium's pipe_viewport_state.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dViewport {
    pub scale: [f32; 3],
    pub translate: [f32; 3],
}

/// `struct v3d_wpos`: gl_FragCoord.y = FYCD * y_scale + y_offset.
/// (-1, the target's height) for a window, (1, 0) for an FBO; all zero is
/// absent, and a program that reads it is refused by the uniform writer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dWpos {
    pub y_scale: f32,
    pub y_offset: f32,
}

/// Our own uniform contents codes for gl_FragCoord.y, outside Mesa's enum
/// (`v3d_frame.h`); the compiler's twins are `select.rs`'s
/// `QUNIFORM_FB_WPOS_Y_*`. Their `data` is 0.
pub const V3D_QUNIFORM_FB_WPOS_Y_SCALE: u32 = 0x10000;
pub const V3D_QUNIFORM_FB_WPOS_Y_OFFSET: u32 = 0x10001;
/// gl_PointCoord.y's transform (`v3d_frame.h`): y = y_raw * scale + offset,
/// (-1, 1) for a window and (1, 0) for an FBO, from the same [`V3dWpos`].
/// Their `data` is 0.
pub const V3D_QUNIFORM_FB_PNTC_Y_SCALE: u32 = 0x10002;
pub const V3D_QUNIFORM_FB_PNTC_Y_OFFSET: u32 = 0x10003;

/// `struct v3d_fs_prog`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dFsProg {
    pub code: u32,
    pub threads: u32,
    pub single_seg: bool,
    pub num_inputs: u32,
    pub flat_shade_flags: u32,
    pub noperspective_flags: u32,
    pub centroid_flags: u32,
    pub writes_z: bool,
    pub disable_ez: bool,
    pub uses_center_w: bool,
    pub lock_scoreboard_on_first_thrsw: bool,
    pub uses_implicit_point_line_varyings: bool,
    pub uses_pid: bool,
    pub uniforms: *const V3dUniform,
    pub num_uniforms: u32,
}

/// `struct v3d_vs_prog` (the vertex and the coordinate shader).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dVsProg {
    pub code: u32,
    pub threads: u32,
    pub single_seg: bool,
    pub vpm_input_size: u32,
    pub vpm_output_size: u32,
    pub separate_segments: bool,
    pub vcm_cache_size: u32,
    pub uses_vid: bool,
    pub uses_iid: bool,
    pub uses_biid: bool,
    pub vattr_sizes: [u8; 16],
    pub uniforms: *const V3dUniform,
    pub num_uniforms: u32,
}

/// `struct v3d_vertex_attr`: one attribute record's array (float only).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dVertexAttr {
    pub address: u32,
    pub stride: u32,
    pub components: u32,
    /// 0: per vertex; n: one value per n instances (the record's Instance
    /// Divisor).
    pub divisor: u32,
}

/// `V3D_MAX_ATTRS`.
pub const V3D_MAX_ATTRS: usize = 16;

/// `struct v3d_uniform_storage`: a stage's GL uniform block.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dUniformStorage {
    pub words: *const u32,
    pub count: u32,
}

impl V3dUniformStorage {
    pub const NONE: V3dUniformStorage = V3dUniformStorage { words: core::ptr::null(), count: 0 };
}

/// `struct v3d_tiling`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dTiling {
    pub tile_w: u32,
    pub tile_h: u32,
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub supertile_w: u32,
    pub supertile_h: u32,
    pub frame_w_supertiles: u32,
    pub frame_h_supertiles: u32,
}

/* ---- v3d_cl.h --------------------------------------------------------------- */

/// `struct v3d_cl`: a control list being written into CPU-mapped GPU memory.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dCl {
    pub buf: *mut u8,
    pub gpu: u32,
    pub size: u32,
    pub used: u32,
    pub overflow: bool,
    pub bad_field: bool,
}

impl V3dCl {
    pub const ZERO: V3dCl =
        V3dCl { buf: core::ptr::null_mut(), gpu: 0, size: 0, used: 0, overflow: false, bad_field: false };

    /// `v3d_cl_init` (a static inline in C, so written again here: its four
    /// field stores, pinned by the layout asserts).
    pub fn new(buf: *mut u8, gpu: u32, size: u32) -> V3dCl {
        V3dCl { buf, gpu, size, used: 0, overflow: false, bad_field: false }
    }
}

/* "Compare Function", "Stencil Op", "Blend Factor", "Blend Mode", "Primitive",
 * index types (v3d_cl.h, v3d_job.h). */
pub const V3D_COMPARE_FUNC_ALWAYS: u32 = 7;
pub const V3D_STENCIL_OP_ZERO: u32 = 0;
pub const V3D_STENCIL_OP_KEEP: u32 = 1;
pub const V3D_STENCIL_OP_REPLACE: u32 = 2;
pub const V3D_STENCIL_OP_INCR: u32 = 3;
pub const V3D_STENCIL_OP_DECR: u32 = 4;
pub const V3D_STENCIL_OP_INVERT: u32 = 5;
pub const V3D_STENCIL_OP_INCWRAP: u32 = 6;
pub const V3D_STENCIL_OP_DECWRAP: u32 = 7;
pub const V3D_BLEND_FACTOR_ZERO: u32 = 0;
pub const V3D_BLEND_FACTOR_ONE: u32 = 1;
pub const V3D_BLEND_FACTOR_SRC_COLOR: u32 = 2;
pub const V3D_BLEND_FACTOR_INV_SRC_COLOR: u32 = 3;
pub const V3D_BLEND_FACTOR_DST_COLOR: u32 = 4;
pub const V3D_BLEND_FACTOR_INV_DST_COLOR: u32 = 5;
pub const V3D_BLEND_FACTOR_SRC_ALPHA: u32 = 6;
pub const V3D_BLEND_FACTOR_INV_SRC_ALPHA: u32 = 7;
pub const V3D_BLEND_FACTOR_DST_ALPHA: u32 = 8;
pub const V3D_BLEND_FACTOR_INV_DST_ALPHA: u32 = 9;
pub const V3D_BLEND_FACTOR_CONST_COLOR: u32 = 10;
pub const V3D_BLEND_FACTOR_INV_CONST_COLOR: u32 = 11;
pub const V3D_BLEND_FACTOR_CONST_ALPHA: u32 = 12;
pub const V3D_BLEND_FACTOR_INV_CONST_ALPHA: u32 = 13;
pub const V3D_BLEND_FACTOR_SRC_ALPHA_SATURATE: u32 = 14;
pub const V3D_PRIM_POINTS: u32 = 0;
pub const V3D_PRIM_LINES: u32 = 1;
pub const V3D_PRIM_LINE_LOOP: u32 = 2;
pub const V3D_PRIM_LINE_STRIP: u32 = 3;
pub const V3D_PRIM_TRIANGLES: u32 = 4;
pub const V3D_PRIM_TRIANGLE_STRIP: u32 = 5;
pub const V3D_PRIM_TRIANGLE_FAN: u32 = 6;
pub const V3D_INDEX_TYPE_8: u32 = 0;
pub const V3D_INDEX_TYPE_16: u32 = 1;
pub const V3D_INDEX_TYPE_32: u32 = 2;

/* ---- v3d_job.h -------------------------------------------------------------- */

/// `struct v3d_job_target`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJobTarget {
    pub width: u32,
    pub height: u32,
    pub address: u32,
    pub stride: u32,
    pub tile_alloc: u32,
    /// The target is 16-bit B5G6R5 (GL's RGB565): loaded and stored as
    /// bgr565; false is RGBA8.
    pub rgb565: bool,
    /// Render targets the job configures, 0 and 1 both meaning RT 0 alone;
    /// RT n > 0 is RGBA8 raster at `rt[n - 1]` (address 0: a GL_NONE draw
    /// buffer, configured but never loaded or stored).
    pub rts: u32,
    pub rt: [V3dJobRt; 3],
    /// 4x multisampling: RT 0 is stored resolved into `address`, and its
    /// samples kept in `ms_address` (UIF RGBA8 at 2w x 2h; 0: not kept),
    /// loaded and stored with all samples. One RT only.
    pub msaa: bool,
    pub ms_address: u32,
    pub ms_memory_format: u32,
    pub ms_height_in_ub: u32,
}

/// `struct v3d_job_rt`: one of render targets 1..3.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJobRt {
    pub address: u32,
    pub stride: u32,
}

/// `struct v3d_job_stencil_face`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJobStencilFace {
    pub func: u32,
    pub sfail_op: u32,
    pub zfail_op: u32,
    pub zpass_op: u32,
    pub ref_: u32,
    pub value_mask: u32,
    pub write_mask: u32,
}

/// `struct v3d_job_state`: one draw's state in gallium's terms.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dJobState {
    pub vp: V3dViewport,
    pub scissor: bool,
    pub scissor_minx: u32,
    pub scissor_miny: u32,
    pub scissor_maxx: u32,
    pub scissor_maxy: u32,
    pub front_ccw: bool,
    pub cull_front: bool,
    pub cull_back: bool,
    pub offset_tri: bool,
    pub offset_units: f32,
    pub offset_scale: f32,
    pub offset_clamp: f32,
    pub line_width: f32,
    pub depth_enabled: bool,
    pub depth_writemask: bool,
    pub depth_func: u32,
    pub stencil_enabled: bool,
    pub stencil_back_enabled: bool,
    pub stencil_front: V3dJobStencilFace,
    pub stencil_back: V3dJobStencilFace,
    pub blend_enabled: bool,
    pub rgb_func: u32,
    pub rgb_src: u32,
    pub rgb_dst: u32,
    pub alpha_func: u32,
    pub alpha_src: u32,
    pub alpha_dst: u32,
    pub colormask: u32,
    pub blend_color_f16: [u16; 4],
    /// gl_FragCoord.y's transform ([`crate::v3d::state::wpos`]).
    pub wpos: V3dWpos,
    /// SAMPLE_STATE's mask in a 4x job (a single-sampled one writes 0xf).
    pub sample_mask: u32,
}

/// `struct v3d_job_draw_layout`: where one draw's uniform streams went (GPU
/// addresses), which the MMU-fault record reads back. Only the three leading
/// fields are named; the rest (the attribute records' address and the
/// shader record as written) is room the C fills and Rust does not read.
/// src/v3d/ffi_layout_check.c pins the offsets and the size (116 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct V3dJobDrawLayout {
    pub fs_uniforms: u32,
    pub vs_uniforms: u32,
    pub cs_uniforms: u32,
    pub rest: [u32; 26],
}

impl V3dJobDrawLayout {
    pub const ZERO: V3dJobDrawLayout =
        V3dJobDrawLayout { fs_uniforms: 0, vs_uniforms: 0, cs_uniforms: 0, rest: [0; 26] };
}

/// `struct v3d_job_draw`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dJobDraw {
    pub st: *const V3dJobState,
    pub fs: *const V3dFsProg,
    pub vs: *const V3dVsProg,
    pub cs: *const V3dVsProg,
    pub vs_storage: V3dUniformStorage,
    pub fs_storage: V3dUniformStorage,
    pub attrs: *const V3dVertexAttr,
    pub num_attrs: u32,
    pub prim: u32,
    pub count: u32,
    pub first: u32,
    pub indexed: bool,
    pub index_type: u32,
    pub index_buffer: u32,
    pub index_buffer_bytes: u32,
    pub index_offset: u32,
    /// The FS stream's sampler bindings (`struct v3d_job_tmu`), or null.
    pub tmu: *const V3dJobTmu,
    pub num_tmu: u32,
    /// The VS and CS streams' sampler bindings, or null.
    pub vs_tmu: *const V3dJobTmu,
    pub num_vs_tmu: u32,
    /// The VS writes the point size into its VPM output: the record's
    /// Point size in shaded vertex data (Mesa sets both for a points draw).
    pub vs_writes_psiz: bool,
    /// Instances of the draw (0 or 1: one).
    pub instances: u32,
}

/// `struct v3d_job_tmu`: one texture unit's record addresses.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dJobTmu {
    pub unit: u32,
    pub tex_state: u32,
    pub sampler: u32,
}

pub const V3D_QUNIFORM_TMU_CONFIG_P0: u32 = 40;
pub const V3D_QUNIFORM_TMU_CONFIG_P1: u32 = 41;

/* ---- driver/src/v3d/v3d_tex.h ------------------------------------------ */

pub const V3D_TEX_MAX_LEVELS: usize = 15;
pub const V3D_TEX_STATE_BYTES: usize = 24;
pub const V3D_SAMPLER_STATE_BYTES: usize = 24;
pub const V3D_TEX_RECORD_ALIGN: u32 = 32;
pub const V3D_TEXTURE_DATA_FORMAT_R8: u32 = 0;
pub const V3D_TEXTURE_DATA_FORMAT_RG8: u32 = 2;
pub const V3D_TEXTURE_DATA_FORMAT_RGBA8: u32 = 4;
pub const V3D_TEXTURE_DATA_FORMAT_R32F: u32 = 29;
pub const V3D_SWIZZLE_ZERO: u8 = 0;
pub const V3D_SWIZZLE_ONE: u8 = 1;
pub const V3D_SWIZZLE_RED: u8 = 2;
pub const V3D_SWIZZLE_GREEN: u8 = 3;
pub const V3D_SWIZZLE_BLUE: u8 = 4;
pub const V3D_SWIZZLE_ALPHA: u8 = 5;
pub const V3D_WRAP_MODE_REPEAT: u32 = 0;
pub const V3D_WRAP_MODE_CLAMP: u32 = 1;
pub const V3D_WRAP_MODE_MIRROR: u32 = 2;
pub const V3D_BORDER_COLOR_0000: u32 = 0;

/// `enum v3d_tex_tiling`: Mesa's v3d_tiling_mode order, which is also the
/// "Memory Format" numbering of the load/store packets (Mesa passes a
/// slice's tiling straight into them, v3dx_rcl.c:89, :151).
pub const V3D_TEX_TILING_LINEARTILE: u32 = 1;
pub const V3D_TEX_TILING_UIF_NO_XOR: u32 = 4;
pub const V3D_TEX_TILING_UIF_XOR: u32 = 5;

/// `struct v3d_tex_slice`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTexSlice {
    pub offset: u32,
    pub stride: u32,
    pub padded_height: u32,
    pub size: u32,
    pub ub_pad: u32,
    /// `enum v3d_tex_tiling`.
    pub tiling: u32,
}

/// `struct v3d_tex_layout`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTexLayout {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub cpp: u32,
    pub size: u32,
    pub slices: [V3dTexSlice; V3D_TEX_MAX_LEVELS],
}

/// `struct v3d_tex_tfu_job`: one TFU job's registers (v3d_tex_tfu_pack).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTexTfuJob {
    pub icfg: u32,
    pub iia: u32,
    pub iis: u32,
    pub ica: u32,
    pub iua: u32,
    pub ioa: u32,
    pub ios: u32,
    pub coef0: u32,
}

/// The TFU job types by texel size (v3d_tex.h `V3D_TFU_TTYPE_*`): a byte
/// copy, as Mesa's blit rewrites the format (v3dx_tfu.c:63-73).
pub const V3D_TFU_TTYPE_R8: u32 = 0;
pub const V3D_TFU_TTYPE_R16F: u32 = 16;
pub const V3D_TFU_TTYPE_R32F: u32 = 29;

/// `struct v3d_tex_state`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTexState {
    pub base: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub base_level: u32,
    pub max_level: u32,
    pub type_: u32,
    pub swizzle: [u8; 4],
    pub srgb: bool,
    pub level_0_is_strictly_uif: bool,
    pub level_0_xor_enable: bool,
    pub extended: bool,
    pub level_0_ub_pad: u32,
    pub array_stride_64: u32,
}

/// `struct v3d_sampler`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dSampler {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub wrap_r: u32,
    pub min_nearest: bool,
    pub mag_nearest: bool,
    pub mip_nearest: bool,
    pub mip_none: bool,
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
    pub border_color_mode: u32,
}

/// `struct v3d_job_clear`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dJobClear {
    pub load_rt0: bool,
    pub rgba8: u32,
    pub z: f32,
    pub stencil: u32,
    /// The depth/stencil buffer kept between jobs (M6 step C): its GPU
    /// address (0: none), tiling and UIF height; whether the job loads it
    /// at every tile's start and stores it at every tile's end.
    pub zs_address: u32,
    pub zs_memory_format: u32,
    pub zs_height_in_ub: u32,
    pub zs_load: bool,
    pub zs_store: bool,
    /// The target has no alpha: the colour load reads alpha as 1 (Mesa's
    /// v3dx_rcl.c:99) and the draws leave alpha unwritten.
    pub force_alpha_1: bool,
    /// The target stores B,G,R,X: the colour load and store swap R and B
    /// (Mesa's r_b_swap, v3dx_rcl.c:98, :150).
    pub r_b_swap: bool,
    /// Depth and stencil apart: `s_address` not 0 makes `zs_*` the depth's
    /// buffer (loaded and stored as buffer Z) and `s_*` the stencil's
    /// (buffer STENCIL), D24S8 in memory unless `s_s8`; 0 is one buffer,
    /// ZSTENCIL. `zs_address` 0 with `s_address` set: the stencil alone.
    pub s_address: u32,
    pub s_memory_format: u32,
    pub s_height_in_ub: u32,
    pub s_load: bool,
    pub s_store: bool,
    /// Render targets 1..3, as `load_rt0` / `rgba8` are for RT 0.
    pub load_rt: [bool; 3],
    pub rgba8_rt: [u32; 3],
    /// The stencil's own buffer is S8 (1 byte a pixel): its load and store
    /// use image format S8, as Mesa's separate stencil (v3dx_rcl.c:93,
    /// :140); else D24S8.
    pub s_s8: bool,
}

/// `V3D_PACK_GROUPS` and `V3D_PACK_KEY_MAX` (v3d_job.h "Packet skip").
pub const V3D_PACK_GROUPS: usize = 11;
pub const V3D_PACK_KEY_MAX: usize = 16;

/// `struct v3d_job_pack_counts`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJobPackCounts {
    pub emitted: u32,
    pub skipped: u32,
    pub compared: u32,
    pub mismatched: u32,
}

/// `struct v3d_job_pack`: the packet skip's record of what a job's BCL
/// holds. Only the C job builder reads or writes it; `v3d_job_begin` sets it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dJobPack {
    pub on: bool,
    pub valid: u32,
    pub key: [[u32; V3D_PACK_KEY_MAX]; V3D_PACK_GROUPS],
    pub n: V3dJobPackCounts,
}

impl V3dJobPack {
    pub const ZERO: V3dJobPack = V3dJobPack {
        on: false,
        valid: 0,
        key: [[0; V3D_PACK_KEY_MAX]; V3D_PACK_GROUPS],
        n: V3dJobPackCounts { emitted: 0, skipped: 0, compared: 0, mismatched: 0 },
    };
}

/// `struct v3d_job`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dJob {
    pub bcl: *mut V3dCl,
    pub rcl: *mut V3dCl,
    pub ind: *mut V3dCl,
    pub t: V3dJobTarget,
    pub draws: u32,
    pub defaults: u32,
    pub broken: bool,
    pub pack: V3dJobPack,
}

impl V3dJob {
    pub const ZERO: V3dJob = V3dJob {
        bcl: core::ptr::null_mut(),
        rcl: core::ptr::null_mut(),
        ind: core::ptr::null_mut(),
        t: V3dJobTarget {
            width: 0,
            height: 0,
            address: 0,
            stride: 0,
            tile_alloc: 0,
            rgb565: false,
            rts: 0,
            rt: [V3dJobRt { address: 0, stride: 0 }; 3],
            msaa: false,
            ms_address: 0,
            ms_memory_format: 0,
            ms_height_in_ub: 0,
        },
        draws: 0,
        defaults: 0,
        broken: false,
        pack: V3dJobPack::ZERO,
    };
}

/// `v3d_job_draw`'s results.
pub const V3D_JOB_OK: i32 = 0;
pub const V3D_JOB_FULL: i32 = 1;
pub const V3D_JOB_REFUSED: i32 = 2;

extern "C" {
    pub fn v3d_job_begin(
        j: *mut V3dJob,
        t: *const V3dJobTarget,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool;
    /// `layout` is a `struct v3d_job_draw_layout *` ([`V3dJobDrawLayout`])
    /// or null.
    pub fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32;
    pub fn v3d_job_end(
        j: *mut V3dJob,
        c: *const V3dJobClear,
        tile_list_start: *mut u32,
        tile_list_end: *mut u32,
    ) -> bool;
    pub fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling);
    pub fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling);
    pub fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling);
    pub fn v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool;
    pub fn v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool;
    pub fn v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, src_stride: u32);
    /// Pack the TFU job that tiles level 0 of `l` from a raster source
    /// (`src_gpu`, rows `src_stride` bytes apart) into the image at
    /// `dst_gpu`; false for what the TFU does not take (v3d_tex.h).
    pub fn v3d_tex_tfu_pack(
        l: *const V3dTexLayout,
        ttype: u32,
        src_gpu: u32,
        src_stride: u32,
        dst_gpu: u32,
        j: *mut V3dTexTfuJob,
    ) -> bool;
    /// The bytes from one layer's mip tree to the next (Mesa's
    /// cube_map_stride, v3d_resource.c:757-764).
    pub fn v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32;
    /// The bytes of `layers` mip trees; false past 32 bits or for 0 layers.
    pub fn v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool;
    /// Tile a w x h box of `level` at (x, y) from `src` (rows `src_stride`
    /// bytes apart); false, writing nothing, for a box outside the level or
    /// a stride shorter than one of its rows.
    #[allow(clippy::too_many_arguments)]
    pub fn v3d_tex_store_box(
        image: *mut u8,
        l: *const V3dTexLayout,
        level: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        src: *const u8,
        src_stride: u32,
    ) -> bool;
    /// The box back out of the tiled image into raster rows (Mesa's
    /// v3d_load_tiled_image); the same refusals as [`v3d_tex_store_box`].
    pub fn v3d_tex_load_box(
        image: *const u8,
        l: *const V3dTexLayout,
        level: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        dst: *mut u8,
        dst_stride: u32,
    ) -> bool;
    pub fn v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout);
    /// False, writing nothing, for a field that does not fit or a type no
    /// supported format has (v3d-tex, 6320fb26).
    pub fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool;
    pub fn v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool;
    pub fn v3d_frame_alloc_sizes(
        t: *const V3dTiling,
        draws: u32,
        page_size: u32,
        tile_alloc_size: *mut u32,
        tile_state_size: *mut u32,
    );
}

/* ---- the pins: the numbers `ffi_layout_check.c` asserts against the headers */

const _: () = assert!(size_of::<V3dUniform>() == 8);
const _: () = assert!(size_of::<V3dUniform>() == size_of::<crate::shaderc::ShcV3dUniform>());
const _: () = assert!(offset_of!(V3dUniform, data) == offset_of!(crate::shaderc::ShcV3dUniform, data));
const _: () = assert!(size_of::<V3dViewport>() == 24);
const _: () = assert!(size_of::<V3dUniformStorage>() == 16);
const _: () = assert!(size_of::<V3dVertexAttr>() == 16);
const _: () = assert!(offset_of!(V3dVertexAttr, divisor) == 12);
const _: () = assert!(size_of::<V3dTiling>() == 32);
const _: () = assert!(size_of::<V3dFsProg>() == 56);
const _: () = assert!(offset_of!(V3dFsProg, threads) == 4);
const _: () = assert!(offset_of!(V3dFsProg, single_seg) == 8);
const _: () = assert!(offset_of!(V3dFsProg, num_inputs) == 12);
const _: () = assert!(offset_of!(V3dFsProg, flat_shade_flags) == 16);
const _: () = assert!(offset_of!(V3dFsProg, centroid_flags) == 24);
const _: () = assert!(offset_of!(V3dFsProg, writes_z) == 28);
const _: () = assert!(offset_of!(V3dFsProg, uses_pid) == 33);
const _: () = assert!(offset_of!(V3dFsProg, uniforms) == 40);
const _: () = assert!(offset_of!(V3dFsProg, num_uniforms) == 48);
const _: () = assert!(size_of::<V3dVsProg>() == 64);
const _: () = assert!(offset_of!(V3dVsProg, single_seg) == 8);
const _: () = assert!(offset_of!(V3dVsProg, vpm_input_size) == 12);
const _: () = assert!(offset_of!(V3dVsProg, separate_segments) == 20);
const _: () = assert!(offset_of!(V3dVsProg, vcm_cache_size) == 24);
const _: () = assert!(offset_of!(V3dVsProg, uses_biid) == 30);
const _: () = assert!(offset_of!(V3dVsProg, vattr_sizes) == 31);
const _: () = assert!(offset_of!(V3dVsProg, uniforms) == 48);
const _: () = assert!(offset_of!(V3dVsProg, num_uniforms) == 56);
const _: () = assert!(size_of::<V3dCl>() == 24);
const _: () = assert!(offset_of!(V3dCl, gpu) == 8);
const _: () = assert!(offset_of!(V3dCl, used) == 16);
const _: () = assert!(offset_of!(V3dCl, overflow) == 20);
const _: () = assert!(offset_of!(V3dCl, bad_field) == 21);
const _: () = assert!(size_of::<V3dJobTarget>() == 68);
const _: () = assert!(offset_of!(V3dJobTarget, rgb565) == 20);
const _: () = assert!(offset_of!(V3dJobTarget, rts) == 24);
const _: () = assert!(offset_of!(V3dJobTarget, rt) == 28);
const _: () = assert!(offset_of!(V3dJobTarget, msaa) == 52);
const _: () = assert!(offset_of!(V3dJobTarget, ms_address) == 56);
const _: () = assert!(offset_of!(V3dJobTarget, ms_memory_format) == 60);
const _: () = assert!(offset_of!(V3dJobTarget, ms_height_in_ub) == 64);
const _: () = assert!(size_of::<V3dJobStencilFace>() == 28);
const _: () = assert!(size_of::<V3dJobState>() == 184);
const _: () = assert!(offset_of!(V3dJobState, scissor) == 24);
const _: () = assert!(offset_of!(V3dJobState, scissor_maxy) == 40);
const _: () = assert!(offset_of!(V3dJobState, front_ccw) == 44);
const _: () = assert!(offset_of!(V3dJobState, offset_tri) == 47);
const _: () = assert!(offset_of!(V3dJobState, offset_units) == 48);
const _: () = assert!(offset_of!(V3dJobState, line_width) == 60);
const _: () = assert!(offset_of!(V3dJobState, depth_enabled) == 64);
const _: () = assert!(offset_of!(V3dJobState, depth_func) == 68);
const _: () = assert!(offset_of!(V3dJobState, stencil_enabled) == 72);
const _: () = assert!(offset_of!(V3dJobState, stencil_front) == 76);
const _: () = assert!(offset_of!(V3dJobState, stencil_back) == 104);
const _: () = assert!(offset_of!(V3dJobState, blend_enabled) == 132);
const _: () = assert!(offset_of!(V3dJobState, rgb_func) == 136);
const _: () = assert!(offset_of!(V3dJobState, colormask) == 160);
const _: () = assert!(offset_of!(V3dJobState, blend_color_f16) == 164);
const _: () = assert!(offset_of!(V3dJobState, wpos) == 172);
const _: () = assert!(size_of::<V3dWpos>() == 8);
const _: () = assert!(size_of::<V3dJobDraw>() == 152);
const _: () = assert!(offset_of!(V3dJobDraw, instances) == 144);
const _: () = assert!(offset_of!(V3dJobDraw, vs_tmu) == 128);
const _: () = assert!(offset_of!(V3dJobDraw, num_vs_tmu) == 136);
const _: () = assert!(offset_of!(V3dJobDraw, vs_writes_psiz) == 140);
const _: () = assert!(offset_of!(V3dJobDraw, tmu) == 112);
const _: () = assert!(offset_of!(V3dJobDraw, num_tmu) == 120);
const _: () = assert!(size_of::<V3dJobTmu>() == 12);
const _: () = assert!(size_of::<V3dTexSlice>() == 24);
const _: () = assert!(size_of::<V3dTexLayout>() == 20 + 24 * V3D_TEX_MAX_LEVELS);
const _: () = assert!(offset_of!(V3dTexLayout, slices) == 20);
const _: () = assert!(size_of::<V3dTexTfuJob>() == 32);
const _: () = assert!(offset_of!(V3dTexTfuJob, ios) == 24);
const _: () = assert!(size_of::<V3dTexState>() == 44);
const _: () = assert!(offset_of!(V3dTexState, swizzle) == 28);
const _: () = assert!(offset_of!(V3dTexState, srgb) == 32);
const _: () = assert!(offset_of!(V3dTexState, extended) == 35);
const _: () = assert!(offset_of!(V3dTexState, level_0_ub_pad) == 36);
const _: () = assert!(offset_of!(V3dTexState, array_stride_64) == 40);
const _: () = assert!(size_of::<V3dSampler>() == 32);
const _: () = assert!(offset_of!(V3dSampler, min_nearest) == 12);
const _: () = assert!(offset_of!(V3dSampler, mip_none) == 15);
const _: () = assert!(offset_of!(V3dSampler, min_lod) == 16);
const _: () = assert!(offset_of!(V3dSampler, border_color_mode) == 28);
const _: () = assert!(offset_of!(V3dJobDraw, vs_storage) == 32);
const _: () = assert!(offset_of!(V3dJobDraw, fs_storage) == 48);
const _: () = assert!(offset_of!(V3dJobDraw, attrs) == 64);
const _: () = assert!(offset_of!(V3dJobDraw, num_attrs) == 72);
const _: () = assert!(offset_of!(V3dJobDraw, prim) == 76);
const _: () = assert!(offset_of!(V3dJobDraw, indexed) == 88);
const _: () = assert!(offset_of!(V3dJobDraw, index_type) == 92);
const _: () = assert!(offset_of!(V3dJobDraw, index_offset) == 104);
const _: () = assert!(size_of::<V3dJobClear>() == 68);
const _: () = assert!(offset_of!(V3dJobClear, s_s8) == 64);
const _: () = assert!(offset_of!(V3dJobClear, load_rt) == 46);
const _: () = assert!(offset_of!(V3dJobClear, rgba8_rt) == 52);
const _: () = assert!(offset_of!(V3dJobClear, rgba8) == 4);
const _: () = assert!(offset_of!(V3dJobClear, z) == 8);
const _: () = assert!(offset_of!(V3dJobClear, zs_address) == 16);
const _: () = assert!(offset_of!(V3dJobClear, zs_height_in_ub) == 24);
const _: () = assert!(offset_of!(V3dJobClear, zs_load) == 28);
const _: () = assert!(offset_of!(V3dJobClear, zs_store) == 29);
const _: () = assert!(offset_of!(V3dJobClear, force_alpha_1) == 30);
const _: () = assert!(offset_of!(V3dJobClear, r_b_swap) == 31);
const _: () = assert!(offset_of!(V3dJobClear, s_address) == 32);
const _: () = assert!(offset_of!(V3dJobClear, s_height_in_ub) == 40);
const _: () = assert!(offset_of!(V3dJobClear, s_load) == 44);
const _: () = assert!(offset_of!(V3dJobClear, s_store) == 45);
const _: () = assert!(size_of::<V3dJob>() == 832);
const _: () = assert!(offset_of!(V3dJob, pack) == 104);
const _: () = assert!(size_of::<V3dJobPack>() == 728);
const _: () = assert!(offset_of!(V3dJobPack, valid) == 4);
const _: () = assert!(offset_of!(V3dJobPack, key) == 8);
const _: () = assert!(offset_of!(V3dJobPack, n) == 712);
const _: () = assert!(offset_of!(V3dJob, t) == 24);
const _: () = assert!(offset_of!(V3dJob, draws) == 92);
const _: () = assert!(offset_of!(V3dJob, broken) == 100);
