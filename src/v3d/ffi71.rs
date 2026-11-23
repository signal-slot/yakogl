// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! FFI to the V3D 7.1 (Raspberry Pi 5, V3D 7.1.10) job, texture and TFU
//! builders in `driver/src/v3d/`: `v3d_job71.h`, `v3d_tex71.h` and
//! `v3d_tfu71.h`, and the version-neutral types of `v3d_frame.h`,
//! `v3d_cl.h` and `v3d_tex.h` they take.
//!
//! The 7.1 twin of `ffi.rs` (its style, its pins). It mirrors the structs
//! field for field in the header's order, the constants as `u32`, the
//! functions as `extern "C"`. Drift between a header and this file would
//! corrupt the control lists silently, so both sides are pinned to the same
//! numbers: the `const _` asserts at the end check the Rust layout, and
//! `src/v3d/ffi71_layout_check.c` (compiled against the REAL headers with
//! `-DV3D_CL_VER=71`) checks the same sizes, offsets and constants. The
//! numbers are LP64 numbers (every type fixed-width, pointers 8 bytes): the
//! aarch64 ones, which the Pi 5's Zephyr image is. `tests/v3d71_ffi.rs`
//! compiles both.
//!
//! Self-contained on purpose: nothing from the crate is imported, only
//! `core`, so a test may include this file with `#[path]`. The includer
//! allows `dead_code` (the table that calls these is a later item). The
//! program, attribute, viewport, uniform and tiling types of `v3d_frame.h`
//! and the control list of `v3d_cl.h` are the same C structs `ffi.rs`
//! mirrors; they are written again here (a `V3d*` type of this file is not
//! the `ffi.rs` one) so that this file stands alone.
//!
//! Not mirrored (the table does not call them, or `ffi.rs` already does the
//! same call): the layout functions of `v3d_tex.h` (`v3d_tex_layout` and
//! friends; `V3dTexLayout` is here because `v3d_tex71_state_from_layout`
//! takes it), `v3d_frame_tiling_gen` (the job fills its own tiling) and
//! the packet-level `v3d_cl_*` helpers.

use core::mem::{offset_of, size_of};

/* ---- v3d_frame.h (the version-neutral types the 7.1 builder takes) ------- */

/// `struct v3d_uniform`: one uniform stream word, `(contents, data)`.
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
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dWpos {
    pub y_scale: f32,
    pub y_offset: f32,
}

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
    pub flat_shade_flags_hi: u32,
    pub noperspective_flags_hi: u32,
    pub centroid_flags_hi: u32,
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

/// `struct v3d_vertex_attr`: one attribute record's array.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dVertexAttr {
    pub address: u32,
    pub stride: u32,
    pub components: u32,
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

/// `struct v3d_cl` (`v3d_cl.h`): a control list being written into
/// CPU-mapped GPU memory.
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
    /// `v3d_cl_init`'s four field stores.
    pub fn new(buf: *mut u8, gpu: u32, size: u32) -> V3dCl {
        V3dCl { buf, gpu, size, used: 0, overflow: false, bad_field: false }
    }
}

/* Mesa's enum quniform_contents (v3d_frame.h) and our own gl_FragCoord /
 * gl_PointCoord contents. */
pub const V3D_QUNIFORM_CONSTANT: u32 = 0;
pub const V3D_QUNIFORM_UNIFORM: u32 = 1;
pub const V3D_QUNIFORM_VIEWPORT_X_SCALE: u32 = 2;
pub const V3D_QUNIFORM_VIEWPORT_Y_SCALE: u32 = 3;
pub const V3D_QUNIFORM_VIEWPORT_Z_OFFSET: u32 = 4;
pub const V3D_QUNIFORM_VIEWPORT_Z_SCALE: u32 = 5;
pub const V3D_QUNIFORM_FB_WPOS_Y_SCALE: u32 = 0x10000;
pub const V3D_QUNIFORM_FB_WPOS_Y_OFFSET: u32 = 0x10001;
pub const V3D_QUNIFORM_FB_PNTC_Y_SCALE: u32 = 0x10002;
pub const V3D_QUNIFORM_FB_PNTC_Y_OFFSET: u32 = 0x10003;

/* "Compare Function", "Stencil Op", "Blend Factor", "Blend Mode", "Primitive",
 * index types (v3d_cl.h, v3d_frame.h): the same numbers at 4.2 and 7.1. */
pub const V3D_COMPARE_FUNC_NEVER: u32 = 0;
pub const V3D_COMPARE_FUNC_LESS: u32 = 1;
pub const V3D_COMPARE_FUNC_EQUAL: u32 = 2;
pub const V3D_COMPARE_FUNC_LEQUAL: u32 = 3;
pub const V3D_COMPARE_FUNC_GREATER: u32 = 4;
pub const V3D_COMPARE_FUNC_NOTEQUAL: u32 = 5;
pub const V3D_COMPARE_FUNC_GEQUAL: u32 = 6;
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
pub const V3D_BLEND_MODE_ADD: u32 = 0;
pub const V3D_BLEND_MODE_SUB: u32 = 1;
pub const V3D_BLEND_MODE_RSUB: u32 = 2;
pub const V3D_BLEND_MODE_MIN: u32 = 3;
pub const V3D_BLEND_MODE_MAX: u32 = 4;
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

/* ---- v3d_job71.h ------------------------------------------------------------ */

/// `V3D_JOB71_MAX_RTS` (v3d_job71.h:67): `devinfo->max_render_targets` of a
/// 7.x device.
pub const V3D_JOB71_MAX_RTS: usize = 8;
/// `V3D_JOB71_MAX_SIZE` (v3d_job71.h:68): pixels, either side.
pub const V3D_JOB71_MAX_SIZE: u32 = 7680;
/// `V3D_QUNIFORM_TMU_CONFIG_P0` / `_P1` (v3d_job71.h:78-79): the TMU
/// configuration words of the sampler binding in bits 24..31 of `data`:
/// `V3dJob71Tmu::tex_state` (P0) or `sampler` (P1) ORed with the low 24 bits
/// (Mesa's `write_tmu_p0` / `write_tmu_p1`).
pub const V3D_QUNIFORM_TMU_CONFIG_P0: u32 = 40;
pub const V3D_QUNIFORM_TMU_CONFIG_P1: u32 = 41;
/// `V3D_QUNIFORM_TEXTURE_*` (v3d_job71.h:82-85): a texture's base level size,
/// depth and layer count at the sampler binding `data`.
pub const V3D_QUNIFORM_TEXTURE_WIDTH: u32 = 44;
pub const V3D_QUNIFORM_TEXTURE_HEIGHT: u32 = 45;
pub const V3D_QUNIFORM_TEXTURE_DEPTH: u32 = 46;
pub const V3D_QUNIFORM_TEXTURE_ARRAY_SIZE: u32 = 47;
/// `V3D_JOB71_ZS_DEPTH` / `_STENCIL` (v3d_job71.h:256-257): the aspect bits of
/// `V3dJob71Clear::zs_load_aspects` / `zs_store_aspects`.
pub const V3D_JOB71_ZS_DEPTH: u8 = 1;
pub const V3D_JOB71_ZS_STENCIL: u8 = 2;

/// `v3d_job71_draw`'s results (v3d_job71.h:276-278).
pub const V3D_JOB71_OK: i32 = 0;
pub const V3D_JOB71_FULL: i32 = 1;
pub const V3D_JOB71_REFUSED: i32 = 2;

/* "Memory Format", "Internal Type" (depth) and the depth/stencil "Output
 * Image Format" values the 7.1 job reads (v3d_packet_v71_pack.h:121-126,
 * :154-156, :189-231). The colour output formats are `format71.rs` /
 * `rt71.rs`'s rows. */
pub const V3D_MEMORY_FORMAT_RASTER: u32 = 0;
pub const V3D_MEMORY_FORMAT_LINEARTILE: u32 = 1;
pub const V3D_MEMORY_FORMAT_UB_LINEAR_1_UIF_BLOCK_WIDE: u32 = 2;
pub const V3D_MEMORY_FORMAT_UB_LINEAR_2_UIF_BLOCKS_WIDE: u32 = 3;
pub const V3D_MEMORY_FORMAT_UIF_NO_XOR: u32 = 4;
pub const V3D_MEMORY_FORMAT_UIF_XOR: u32 = 5;
pub const V3D_INTERNAL_TYPE_DEPTH_32F: u32 = 0;
pub const V3D_INTERNAL_TYPE_DEPTH_24: u32 = 1;
pub const V3D_INTERNAL_TYPE_DEPTH_16: u32 = 2;
pub const V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8: u32 = 0;
pub const V3D_OUTPUT_IMAGE_FORMAT_BGR565: u32 = 7;
pub const V3D_OUTPUT_IMAGE_FORMAT_RGBA8: u32 = 27;
pub const V3D_OUTPUT_IMAGE_FORMAT_D32F: u32 = 40;
pub const V3D_OUTPUT_IMAGE_FORMAT_D16: u32 = 42;
pub const V3D_OUTPUT_IMAGE_FORMAT_D24S8: u32 = 43;
pub const V3D_OUTPUT_IMAGE_FORMAT_S8: u32 = 44;

/// `struct v3d_job71_rt` (v3d_job71.h:89): one colour render target.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJob71Rt {
    /// GPU address of the image; 0: no buffer (a GL_NONE draw buffer).
    pub address: u32,
    /// `V3D_OUTPUT_IMAGE_FORMAT_*`.
    pub format: u32,
    /// `V3D_MEMORY_FORMAT_*`.
    pub memory_format: u32,
    /// Raster: the row stride in bytes. UIF: the padded height in UIF blocks.
    pub height_in_ub_or_stride: u32,
    pub r_b_swap: bool,
    pub force_alpha_1: bool,
    pub pad: [u8; 2],
}

/// `struct v3d_job71_zs` (v3d_job71.h:106): a depth/stencil image in memory.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJob71Zs {
    pub address: u32,
    pub format: u32,
    pub memory_format: u32,
    pub height_in_ub: u32,
}

/// `struct v3d_job71_target` (v3d_job71.h:117): the job's framebuffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJob71Target {
    pub width: u32,
    pub height: u32,
    pub tile_alloc: u32,
    /// Colour render targets `0 .. rts`, `rts` 0 ..= [`V3D_JOB71_MAX_RTS`].
    pub rts: u32,
    pub rt: [V3dJob71Rt; V3D_JOB71_MAX_RTS],
    /// `V3D_INTERNAL_TYPE_DEPTH_*`.
    pub depth_type: u32,
    pub zs: V3dJob71Zs,
    pub s: V3dJob71Zs,
}

/// `struct v3d_job71_stencil_face` (v3d_job71.h:140).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dJob71StencilFace {
    pub func: u32,
    pub sfail_op: u32,
    pub zfail_op: u32,
    pub zpass_op: u32,
    pub ref_: u32,
    pub value_mask: u32,
    pub write_mask: u32,
}

/// `struct v3d_job71_state` (v3d_job71.h:148): one draw's state, field for
/// field `ffi.rs`'s `V3dJobState`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dJob71State {
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
    pub stencil_front: V3dJob71StencilFace,
    pub stencil_back: V3dJob71StencilFace,
    pub blend_enabled: bool,
    pub rgb_func: u32,
    pub rgb_src: u32,
    pub rgb_dst: u32,
    pub alpha_func: u32,
    pub alpha_src: u32,
    pub alpha_dst: u32,
    pub colormask: u32,
    pub blend_color_f16: [u16; 4],
    pub wpos: V3dWpos,
    /// Not read at 7.1 yet (no 4x): 0xf.
    pub sample_mask: u32,
    pub oq_addr: u32,
}

/// `struct v3d_job71_tmu` (v3d_job71.h:183): one sampler binding: the
/// record addresses for the TMU configuration words (`tex_state` low 4 bits
/// and `sampler` low 3 bits 0), the sizes for the `TEXTURE_*` words.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dJob71Tmu {
    pub unit: u32,
    pub tex_state: u32,
    pub sampler: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub array_size: u32,
}

/// `struct v3d_job71_draw` (v3d_job71.h:199).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dJob71Draw {
    pub st: *const V3dJob71State,
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
    pub tmu: *const V3dJob71Tmu,
    pub num_tmu: u32,
    pub vs_tmu: *const V3dJob71Tmu,
    pub num_vs_tmu: u32,
    pub vs_writes_psiz: bool,
    pub instances: u32,
}

/// `struct v3d_job71_clear` (v3d_job71.h:222): what the render list does with
/// the tile buffer around the draws.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dJob71Clear {
    /// Bit n: load RT n from its image.
    pub load_rts: u32,
    /// RT n's clear colour as the tile buffer holds it, packed per internal
    /// type.
    pub color: [[u32; 4]; V3D_JOB71_MAX_RTS],
    pub z: f32,
    pub stencil: u32,
    pub zs_load: bool,
    pub zs_store: bool,
    pub s_load: bool,
    pub s_store: bool,
    pub zs_cleared: bool,
    /// `V3D_JOB71_ZS_*` bits; 0: every aspect `zs` holds.
    pub zs_load_aspects: u8,
    pub zs_store_aspects: u8,
    pub pad: [u8; 1],
}

/// `struct v3d_job71` (v3d_job71.h:259): the builder's state; the caller
/// zeroes it and `v3d_job71_begin` fills it.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dJob71 {
    pub bcl: *mut V3dCl,
    pub rcl: *mut V3dCl,
    pub ind: *mut V3dCl,
    pub t: V3dJob71Target,
    pub tiling: V3dTiling,
    pub draws: u32,
    pub broken: bool,
}

impl V3dJob71 {
    pub const ZERO: V3dJob71 = V3dJob71 {
        bcl: core::ptr::null_mut(),
        rcl: core::ptr::null_mut(),
        ind: core::ptr::null_mut(),
        t: V3dJob71Target {
            width: 0,
            height: 0,
            tile_alloc: 0,
            rts: 0,
            rt: [V3dJob71Rt {
                address: 0,
                format: 0,
                memory_format: 0,
                height_in_ub_or_stride: 0,
                r_b_swap: false,
                force_alpha_1: false,
                pad: [0; 2],
            }; V3D_JOB71_MAX_RTS],
            depth_type: 0,
            zs: V3dJob71Zs { address: 0, format: 0, memory_format: 0, height_in_ub: 0 },
            s: V3dJob71Zs { address: 0, format: 0, memory_format: 0, height_in_ub: 0 },
        },
        tiling: V3dTiling {
            tile_w: 0,
            tile_h: 0,
            tiles_x: 0,
            tiles_y: 0,
            supertile_w: 0,
            supertile_h: 0,
            frame_w_supertiles: 0,
            frame_h_supertiles: 0,
        },
        draws: 0,
        broken: false,
    };
}

/// `struct v3d_job71_draw_layout` (v3d_job71.h:268): where the job put what
/// one draw wrote (GPU addresses), for a checker. `rec` is the 96 bytes of
/// `struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX` (Mesa's unpacked record,
/// 24 words), not read from Rust; the C check pins its offset and the size.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct V3dJob71DrawLayout {
    pub fs_uniforms: u32,
    pub vs_uniforms: u32,
    pub cs_uniforms: u32,
    pub record: u32,
    pub attr_records: u32,
    pub rec: [u32; 24],
}

impl V3dJob71DrawLayout {
    pub const ZERO: V3dJob71DrawLayout =
        V3dJob71DrawLayout { fs_uniforms: 0, vs_uniforms: 0, cs_uniforms: 0, record: 0, attr_records: 0, rec: [0; 24] };
}

extern "C" {
    /// v3d_job71.h:282. False (and the job is broken) for a target the builder
    /// refuses or a list overflow.
    pub fn v3d_job71_begin(
        j: *mut V3dJob71,
        t: *const V3dJob71Target,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool;
    /// v3d_job71.h:286. `layout` may be null. `V3D_JOB71_OK` / `_FULL` /
    /// `_REFUSED`; a draw that is not recorded leaves nothing behind.
    pub fn v3d_job71_draw(j: *mut V3dJob71, d: *const V3dJob71Draw, layout: *mut V3dJob71DrawLayout) -> i32;
    /// v3d_job71.h:293. `tile_list_start` / `end` may be null. False when the
    /// job is broken, a buffer to load or store is not given, a Z/S aspect is
    /// not one the buffer holds, or a list overflowed.
    pub fn v3d_job71_end(
        j: *mut V3dJob71,
        c: *const V3dJob71Clear,
        tile_list_start: *mut u32,
        tile_list_end: *mut u32,
    ) -> bool;
    /// v3d_job71.h:299: the submit's tile allocation size (rounded up to
    /// `page_size`, 16 KiB on the Pi 5's kernel) and tile state size.
    pub fn v3d_job71_alloc_sizes(
        j: *const V3dJob71,
        page_size: u32,
        tile_alloc_size: *mut u32,
        tile_state_size: *mut u32,
    );
}

/* ---- v3d_tex71.h (and the layout type of v3d_tex.h) ------------------------- */

/// `V3D_TEX_MAX_LEVELS` (v3d_tex.h).
pub const V3D_TEX_MAX_LEVELS: usize = 15;

/// `struct v3d_tex_slice` (v3d_tex.h), what `V3dTexLayout` holds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTexSlice {
    pub offset: u32,
    pub stride: u32,
    pub padded_height: u32,
    pub size: u32,
    pub ub_pad: u32,
    pub tiling: u32,
}

/// `struct v3d_tex_layout` (v3d_tex.h): the argument of
/// [`v3d_tex71_state_from_layout`].
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

/// `V3D_TEX71_STATE_BYTES` (v3d_tex71.h:74-75): TEXTURE_SHADER_STATE at 7.1.
pub const V3D_TEX71_STATE_BYTES: usize = 24;
/// `V3D_TEX71_SAMPLER_BYTES` (v3d_tex71.h:75): SAMPLER_STATE at 7.1.
pub const V3D_TEX71_SAMPLER_BYTES: usize = 24;
/// `V3D_TEX71_RECORD_ALIGN` (v3d_tex71.h:76).
pub const V3D_TEX71_RECORD_ALIGN: u32 = 32;
/// Transfer func (v3d_tex71.h:79-80).
pub const V3D_TEX71_TRANSFER_NONE: u32 = 0;
pub const V3D_TEX71_TRANSFER_SRGB: u32 = 1;
/// Wrap modes and border colour modes (v3d_tex71.h:130-138).
pub const V3D_TEX71_WRAP_REPEAT: u32 = 0;
pub const V3D_TEX71_WRAP_CLAMP: u32 = 1;
pub const V3D_TEX71_WRAP_MIRROR: u32 = 2;
pub const V3D_TEX71_WRAP_BORDER: u32 = 3;
pub const V3D_TEX71_WRAP_MIRROR_ONCE: u32 = 4;
pub const V3D_TEX71_BORDER_0000: u32 = 0;
pub const V3D_TEX71_BORDER_0001: u32 = 1;
pub const V3D_TEX71_BORDER_1111: u32 = 2;
pub const V3D_TEX71_BORDER_FOLLOWS: u32 = 7;

/// `struct v3d_tex71_state` (v3d_tex71.h:82): the TEXTURE_SHADER_STATE
/// fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTex71State {
    pub base: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub base_level: u32,
    pub max_level: u32,
    /// `V3D_TEXTURE_DATA_FORMAT_*` (v3d_tex.h).
    pub type_: u32,
    /// R, G, B, A: `V3D_SWIZZLE_*` (v3d_tex.h).
    pub swizzle: [u8; 4],
    /// `V3D_TEX71_TRANSFER_*`.
    pub transfer_func: u32,
    pub r_b_swap: bool,
    pub reverse: bool,
    pub level_0_is_strictly_uif: bool,
    pub level_0_xor_enable: bool,
    pub extended: bool,
    pub level_0_ub_pad: u32,
    pub array_stride_64: u32,
}

/// `struct v3d_tex71_sampler` (v3d_tex71.h:140): the SAMPLER_STATE fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3dTex71Sampler {
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
    pub compare_func: u32,
    pub max_anisotropy: u32,
    pub border_color_mode: u32,
    pub border_color: [u32; 4],
}

/// `struct v3d_tex71_p2` (v3d_tex71.h:167): TMU configuration parameter 2.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dTex71P2 {
    pub op: u32,
    pub offset_s: i32,
    pub offset_t: i32,
    pub offset_r: i32,
    pub gather_mode: bool,
    pub gather_component: u32,
    pub coefficient_mode: bool,
    pub sample_number: u32,
    pub disable_autolod: bool,
    pub offset_format_8: bool,
    pub lod_query: bool,
    pub dim_query: bool,
    pub write_conversion: bool,
}

extern "C" {
    /// v3d_tex71.h:113: the layout-derived fields of a 2D, cube or array
    /// texture (`array_stride_64` = the mip tree's stride / 64).
    pub fn v3d_tex71_state_from_layout(s: *mut V3dTex71State, l: *const V3dTexLayout);
    /// v3d_tex71.h:116: the same for a 3D texture.
    pub fn v3d_tex71_state_from_layout_3d(s: *mut V3dTex71State, l: *const V3dTexLayout);
    /// v3d_tex71.h:124: false, `out` untouched, for a record the TMU would
    /// read as something else (nothing is masked to fit). `out` is
    /// [`V3D_TEX71_STATE_BYTES`] bytes.
    pub fn v3d_tex71_pack_state(out: *mut u8, s: *const V3dTex71State) -> bool;
    /// v3d_tex71.h:162: false, `out` untouched, for a mode or value outside
    /// its enum or field. `out` is [`V3D_TEX71_SAMPLER_BYTES`] bytes.
    pub fn v3d_tex71_pack_sampler(out: *mut u8, s: *const V3dTex71Sampler) -> bool;
    /// v3d_tex71.h:179: TMU parameter 2; false, `*out` untouched, for a value
    /// that does not fit its field.
    pub fn v3d_tex71_p2(p: *const V3dTex71P2, out: *mut u32) -> bool;
}

/* ---- v3d_tfu71.h ------------------------------------------------------------ */

/// `enum v3d_tfu71_tiling` (v3d_tfu71.h:33-40), Mesa's `v3d_tiling_mode`
/// order. A C enum is `int`-sized; the fields below are `u32` because every
/// value is small and non-negative.
pub const V3D_TFU71_TILING_RASTER: u32 = 0;
pub const V3D_TFU71_TILING_LINEARTILE: u32 = 1;
pub const V3D_TFU71_TILING_UBLINEAR_1_COLUMN: u32 = 2;
pub const V3D_TFU71_TILING_UBLINEAR_2_COLUMN: u32 = 3;
pub const V3D_TFU71_TILING_UIF_NO_XOR: u32 = 4;
pub const V3D_TFU71_TILING_UIF_XOR: u32 = 5;

/// Field positions and codes (v3d_tfu71.h:43-53).
pub const V3D_TFU71_MAX_DIM: u32 = 16384;
pub const V3D_TFU71_ICFG_OTYPE_SHIFT: u32 = 16;
pub const V3D_TFU71_ICFG_IFORMAT_SHIFT: u32 = 23;
pub const V3D_TFU71_ICFG_FORMAT_RASTER: u32 = 0;
pub const V3D_TFU71_ICFG_FORMAT_LINEARTILE: u32 = 3;
pub const V3D_TFU71_IOC_DIMTW: u32 = 1 << 0;
pub const V3D_TFU71_IOC_NUMMM_SHIFT: u32 = 4;
pub const V3D_TFU71_IOC_FORMAT_SHIFT: u32 = 12;
pub const V3D_TFU71_IOC_FORMAT_RASTER: u32 = 0;
pub const V3D_TFU71_IOC_FORMAT_LINEARTILE: u32 = 3;
pub const V3D_TFU71_IOC_STRIDE_SHIFT: u32 = 16;
/// Texture data types of the cases checked first (v3d_tfu71.h:59-61).
pub const V3D_TFU71_TTYPE_R8: u32 = 0;
pub const V3D_TFU71_TTYPE_RGBA8: u32 = 4;
pub const V3D_TFU71_TTYPE_RGB565: u32 = 6;

/// `struct v3d_tfu71_job` (v3d_tfu71.h:68): the register values of one TFU
/// job, written in the order IIA, IIS, ICA, IUA, IOA, IOC, IOS, COEF0..3,
/// then ICFG (which starts the job).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dTfu71Job {
    pub icfg: u32,
    pub iia: u32,
    pub iis: u32,
    pub ica: u32,
    pub iua: u32,
    pub ioa: u32,
    pub ios: u32,
    pub coef: [u32; 4],
    pub ioc: u32,
}

/// `struct v3d_tfu71_surface` (v3d_tfu71.h:77): one side of the conversion.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dTfu71Surface {
    pub addr: u32,
    /// `enum v3d_tfu71_tiling` (`V3D_TFU71_TILING_*`).
    pub tiling: u32,
    pub stride_or_padded_height: u32,
}

/// `struct v3d_tfu71_args` (v3d_tfu71.h:87): the job's inputs.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct V3dTfu71Args {
    pub ttype: u32,
    pub cpp: u32,
    pub width: u32,
    pub height: u32,
    pub num_mm: u32,
    pub src: V3dTfu71Surface,
    pub dst: V3dTfu71Surface,
}

extern "C" {
    /// v3d_tfu71.h:97: may the TFU convert `ttype`, and generate mipmaps of
    /// it?
    pub fn v3d_tfu71_supports(ttype: u32, for_mipmap: bool) -> bool;
    /// v3d_tfu71.h:108: pack the job; false, `j` untouched, where Mesa's
    /// v3dX(tfu) declines or a value would not fit.
    pub fn v3d_tfu71_pack(a: *const V3dTfu71Args, j: *mut V3dTfu71Job) -> bool;
}

/* ---- the pins: the numbers `ffi71_layout_check.c` asserts against the headers */

const _: () = assert!(size_of::<V3dUniform>() == 8);
const _: () = assert!(size_of::<V3dViewport>() == 24);
const _: () = assert!(size_of::<V3dWpos>() == 8);
const _: () = assert!(size_of::<V3dUniformStorage>() == 16);
const _: () = assert!(size_of::<V3dVertexAttr>() == 16);
const _: () = assert!(size_of::<V3dTiling>() == 32);
const _: () = assert!(size_of::<V3dFsProg>() == 64);
const _: () = assert!(size_of::<V3dVsProg>() == 64);
const _: () = assert!(size_of::<V3dCl>() == 24);
const _: () = assert!(offset_of!(V3dCl, gpu) == 8);
const _: () = assert!(offset_of!(V3dCl, used) == 16);
const _: () = assert!(offset_of!(V3dCl, overflow) == 20);
const _: () = assert!(offset_of!(V3dCl, bad_field) == 21);

const _: () = assert!(size_of::<V3dJob71Rt>() == 20);
const _: () = assert!(offset_of!(V3dJob71Rt, format) == 4);
const _: () = assert!(offset_of!(V3dJob71Rt, memory_format) == 8);
const _: () = assert!(offset_of!(V3dJob71Rt, height_in_ub_or_stride) == 12);
const _: () = assert!(offset_of!(V3dJob71Rt, r_b_swap) == 16);
const _: () = assert!(offset_of!(V3dJob71Rt, force_alpha_1) == 17);
const _: () = assert!(offset_of!(V3dJob71Rt, pad) == 18);
const _: () = assert!(size_of::<V3dJob71Zs>() == 16);
const _: () = assert!(offset_of!(V3dJob71Zs, format) == 4);
const _: () = assert!(offset_of!(V3dJob71Zs, height_in_ub) == 12);
const _: () = assert!(size_of::<V3dJob71Target>() == 212);
const _: () = assert!(offset_of!(V3dJob71Target, tile_alloc) == 8);
const _: () = assert!(offset_of!(V3dJob71Target, rts) == 12);
const _: () = assert!(offset_of!(V3dJob71Target, rt) == 16);
const _: () = assert!(offset_of!(V3dJob71Target, depth_type) == 176);
const _: () = assert!(offset_of!(V3dJob71Target, zs) == 180);
const _: () = assert!(offset_of!(V3dJob71Target, s) == 196);
const _: () = assert!(size_of::<V3dJob71StencilFace>() == 28);
const _: () = assert!(size_of::<V3dJob71State>() == 188);
const _: () = assert!(offset_of!(V3dJob71State, scissor) == 24);
const _: () = assert!(offset_of!(V3dJob71State, scissor_maxy) == 40);
const _: () = assert!(offset_of!(V3dJob71State, front_ccw) == 44);
const _: () = assert!(offset_of!(V3dJob71State, offset_tri) == 47);
const _: () = assert!(offset_of!(V3dJob71State, offset_units) == 48);
const _: () = assert!(offset_of!(V3dJob71State, line_width) == 60);
const _: () = assert!(offset_of!(V3dJob71State, depth_enabled) == 64);
const _: () = assert!(offset_of!(V3dJob71State, depth_func) == 68);
const _: () = assert!(offset_of!(V3dJob71State, stencil_enabled) == 72);
const _: () = assert!(offset_of!(V3dJob71State, stencil_front) == 76);
const _: () = assert!(offset_of!(V3dJob71State, stencil_back) == 104);
const _: () = assert!(offset_of!(V3dJob71State, blend_enabled) == 132);
const _: () = assert!(offset_of!(V3dJob71State, rgb_func) == 136);
const _: () = assert!(offset_of!(V3dJob71State, colormask) == 160);
const _: () = assert!(offset_of!(V3dJob71State, blend_color_f16) == 164);
const _: () = assert!(offset_of!(V3dJob71State, wpos) == 172);
const _: () = assert!(offset_of!(V3dJob71State, sample_mask) == 180);
const _: () = assert!(offset_of!(V3dJob71State, oq_addr) == 184);
const _: () = assert!(size_of::<V3dJob71Tmu>() == 28);
const _: () = assert!(offset_of!(V3dJob71Tmu, width) == 12);
const _: () = assert!(offset_of!(V3dJob71Tmu, array_size) == 24);
const _: () = assert!(size_of::<V3dJob71Draw>() == 152);
const _: () = assert!(offset_of!(V3dJob71Draw, fs) == 8);
const _: () = assert!(offset_of!(V3dJob71Draw, vs_storage) == 32);
const _: () = assert!(offset_of!(V3dJob71Draw, fs_storage) == 48);
const _: () = assert!(offset_of!(V3dJob71Draw, attrs) == 64);
const _: () = assert!(offset_of!(V3dJob71Draw, num_attrs) == 72);
const _: () = assert!(offset_of!(V3dJob71Draw, prim) == 76);
const _: () = assert!(offset_of!(V3dJob71Draw, indexed) == 88);
const _: () = assert!(offset_of!(V3dJob71Draw, index_type) == 92);
const _: () = assert!(offset_of!(V3dJob71Draw, index_offset) == 104);
const _: () = assert!(offset_of!(V3dJob71Draw, tmu) == 112);
const _: () = assert!(offset_of!(V3dJob71Draw, num_tmu) == 120);
const _: () = assert!(offset_of!(V3dJob71Draw, vs_tmu) == 128);
const _: () = assert!(offset_of!(V3dJob71Draw, num_vs_tmu) == 136);
const _: () = assert!(offset_of!(V3dJob71Draw, vs_writes_psiz) == 140);
const _: () = assert!(offset_of!(V3dJob71Draw, instances) == 144);
const _: () = assert!(size_of::<V3dJob71Clear>() == 148);
const _: () = assert!(offset_of!(V3dJob71Clear, color) == 4);
const _: () = assert!(offset_of!(V3dJob71Clear, z) == 132);
const _: () = assert!(offset_of!(V3dJob71Clear, stencil) == 136);
const _: () = assert!(offset_of!(V3dJob71Clear, zs_load) == 140);
const _: () = assert!(offset_of!(V3dJob71Clear, zs_cleared) == 144);
const _: () = assert!(offset_of!(V3dJob71Clear, zs_load_aspects) == 145);
const _: () = assert!(offset_of!(V3dJob71Clear, pad) == 147);
const _: () = assert!(size_of::<V3dJob71>() == 280);
const _: () = assert!(offset_of!(V3dJob71, t) == 24);
const _: () = assert!(offset_of!(V3dJob71, tiling) == 236);
const _: () = assert!(offset_of!(V3dJob71, draws) == 268);
const _: () = assert!(offset_of!(V3dJob71, broken) == 272);
const _: () = assert!(size_of::<V3dJob71DrawLayout>() == 116);
const _: () = assert!(offset_of!(V3dJob71DrawLayout, record) == 12);
const _: () = assert!(offset_of!(V3dJob71DrawLayout, attr_records) == 16);
const _: () = assert!(offset_of!(V3dJob71DrawLayout, rec) == 20);
const _: () = assert!(core::mem::align_of::<V3dJob71DrawLayout>() == 4);

const _: () = assert!(size_of::<V3dTexSlice>() == 24);
const _: () = assert!(size_of::<V3dTexLayout>() == 20 + 24 * V3D_TEX_MAX_LEVELS);
const _: () = assert!(offset_of!(V3dTexLayout, slices) == 20);
const _: () = assert!(size_of::<V3dTex71State>() == 52);
const _: () = assert!(offset_of!(V3dTex71State, width) == 4);
const _: () = assert!(offset_of!(V3dTex71State, type_) == 24);
const _: () = assert!(offset_of!(V3dTex71State, swizzle) == 28);
const _: () = assert!(offset_of!(V3dTex71State, transfer_func) == 32);
const _: () = assert!(offset_of!(V3dTex71State, r_b_swap) == 36);
const _: () = assert!(offset_of!(V3dTex71State, reverse) == 37);
const _: () = assert!(offset_of!(V3dTex71State, level_0_is_strictly_uif) == 38);
const _: () = assert!(offset_of!(V3dTex71State, level_0_ub_pad) == 44);
const _: () = assert!(offset_of!(V3dTex71State, array_stride_64) == 48);
const _: () = assert!(size_of::<V3dTex71Sampler>() == 56);
const _: () = assert!(offset_of!(V3dTex71Sampler, min_nearest) == 12);
const _: () = assert!(offset_of!(V3dTex71Sampler, min_lod) == 16);
const _: () = assert!(offset_of!(V3dTex71Sampler, compare_func) == 28);
const _: () = assert!(offset_of!(V3dTex71Sampler, max_anisotropy) == 32);
const _: () = assert!(offset_of!(V3dTex71Sampler, border_color_mode) == 36);
const _: () = assert!(offset_of!(V3dTex71Sampler, border_color) == 40);
const _: () = assert!(size_of::<V3dTex71P2>() == 40);
const _: () = assert!(offset_of!(V3dTex71P2, offset_s) == 4);
const _: () = assert!(offset_of!(V3dTex71P2, gather_mode) == 16);
const _: () = assert!(offset_of!(V3dTex71P2, gather_component) == 20);
const _: () = assert!(offset_of!(V3dTex71P2, coefficient_mode) == 24);
const _: () = assert!(offset_of!(V3dTex71P2, sample_number) == 28);
const _: () = assert!(offset_of!(V3dTex71P2, disable_autolod) == 32);
const _: () = assert!(offset_of!(V3dTex71P2, lod_query) == 34);

const _: () = assert!(size_of::<V3dTfu71Job>() == 48);
const _: () = assert!(offset_of!(V3dTfu71Job, coef) == 28);
const _: () = assert!(offset_of!(V3dTfu71Job, ioc) == 44);
const _: () = assert!(size_of::<V3dTfu71Surface>() == 12);
const _: () = assert!(offset_of!(V3dTfu71Surface, tiling) == 4);
const _: () = assert!(offset_of!(V3dTfu71Surface, stride_or_padded_height) == 8);
const _: () = assert!(size_of::<V3dTfu71Args>() == 44);
const _: () = assert!(offset_of!(V3dTfu71Args, cpp) == 4);
const _: () = assert!(offset_of!(V3dTfu71Args, width) == 8);
const _: () = assert!(offset_of!(V3dTfu71Args, num_mm) == 16);
const _: () = assert!(offset_of!(V3dTfu71Args, src) == 20);
const _: () = assert!(offset_of!(V3dTfu71Args, dst) == 32);
