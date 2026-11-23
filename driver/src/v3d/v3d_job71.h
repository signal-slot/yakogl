/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2014-2017 Broadcom
 * Ported from Mesa 26.2.1 src/gallium/drivers/v3d/ (v3dx_draw.c, v3dx_emit.c,
 * v3dx_rcl.c, v3d_context.c), the version 71 paths (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * V3D 7.1 job builder: the binning and render control lists of one job (one
 * frame) with any number of draws, for the BCM2712 D0 (V3D 7.1.10, the
 * Raspberry Pi 5). The 7.1 counterpart of v3d_job.h (4.2), built with
 * -DV3D_CL_VER=71 and linked with v3d_frame.c built the same way (the tile
 * size, the tile allocation sizes and the gl_FragCoord words come from
 * there). At 4.2 this header declares nothing and v3d_job71.c is empty, so a
 * build may list them for both generations.
 *
 * What differs from the 4.2 builder, each as Mesa 26.2.1's gallium driver
 * does it for V3D_VERSION 71 (docs/notes/v3d71-clear-frame.md,
 * docs/notes/v3d71-triangle-frame.md, driver/docs/rpi5-v3d71-facts.md):
 *  - the shader state record is the D0's "GL Shader State Record Draw Index"
 *    (32 bytes, v3dx_draw.c emit_shader_state_record with
 *    v3d_device_has_draw_index): this builder is for 7.1.10 and later only;
 *  - there are no default attribute values (no default-attribute buffer):
 *    the GFXH-930 dummy attribute record of a program that reads no
 *    attribute points at the indirect buffer's first byte, as Mesa's does;
 *  - up to 8 colour render targets (V3D_JOB71_MAX_RTS), each configured by
 *    TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1..3 with its own TLB base and
 *    row stride and one internal "type and clamping" value; the tile size is
 *    chosen from the bytes per pixel of all of them together;
 *  - image sides up to V3D_JOB71_MAX_SIZE (7680) pixels;
 *  - the tile buffer is cleared by CLEAR_RENDER_TARGETS (no flags) after the
 *    stores of every tile and in the first of the two dummy tiles of the
 *    GFXH-1742 workaround; the store's "clear buffer being stored" bit is
 *    never set;
 *  - NUMBER_OF_LAYERS (1) opens the binning list; the tile size is written
 *    into TILE_BINNING_MODE_CFG and TILE_RENDERING_MODE_CFG_COMMON;
 *  - the clipper's XY scale is in 1/64 pixel, and CFG_BITS carries the Z
 *    clipping mode (-1..1) instead of early-Z bits.
 *
 * Refused (V3D_JOB71_REFUSED, or v3d_job71_begin false), never drawn wrong:
 * 4x multisampling (no field: not yet), a viewport whose |Z scale| is below
 * 0.01 (Mesa writes CLIPPER_Z_SCALE_AND_OFFSET_NO_GUARDBAND there, a packet
 * v3d_cl_packets.h does not have yet), a TMU configuration uniform
 * (QUNIFORM_TMU_CONFIG_P0/P1) whose sampler binding is not given or whose
 * record address and payload share a bit, VPM
 * "separate segments" (no 7.1 record flag), a program reading the base
 * instance, transform feedback, primitive restart, a base vertex, early Z
 * (off: legal, only slower).
 *
 * No OS calls: the caller owns the memory and the addresses. The structs
 * below are the FFI layout (src/v3d/ffi71.rs mirrors them): fixed-size
 * fields only, no bit fields.
 */
#ifndef V3D_JOB71_H
#define V3D_JOB71_H

#include <stdint.h>
#include <stdbool.h>

#include "v3d_cl.h"
#include "v3d_frame.h" /* the program, attribute, viewport, uniform and tiling types */

#if V3D_CL_VER == 71

#define V3D_JOB71_MAX_RTS  8    /* devinfo->max_render_targets of a 7.x device */
#define V3D_JOB71_MAX_SIZE 7680 /* pixels, either side */

/* Mesa's enum quniform_contents values the 7.1 builder reads besides
 * v3d_frame.h's (the same numbers as v3d_job.h, which a 7.1 build cannot
 * include): the TMU configuration words P0 and P1 of the sampler binding in
 * bits 24..31 of `data` (struct v3d_job71_tmu's tex_state or sampler ORed
 * with the low 24 bits, as v3d_uniforms.c write_tmu_p0 / write_tmu_p1), and
 * a texture's base level size, depth and layer count at sampler binding
 * `data`. */
#ifndef V3D_QUNIFORM_TMU_CONFIG_P0
#define V3D_QUNIFORM_TMU_CONFIG_P0 40
#define V3D_QUNIFORM_TMU_CONFIG_P1 41
#endif
#ifndef V3D_QUNIFORM_TEXTURE_WIDTH
#define V3D_QUNIFORM_TEXTURE_WIDTH      44
#define V3D_QUNIFORM_TEXTURE_HEIGHT     45
#define V3D_QUNIFORM_TEXTURE_DEPTH      46
#define V3D_QUNIFORM_TEXTURE_ARRAY_SIZE 47
#endif

/* One colour render target. */
struct v3d_job71_rt {
	/* GPU address of the image (the level and layer drawn to). 0: no
	 * buffer (a GL_NONE draw buffer): the RT is configured unused (stride
	 * 1, as Mesa does for a NULL cbuf, v3dx_rcl.c emit_rcl), never loaded
	 * or stored, and takes no TLB space. */
	uint32_t address;
	uint32_t format;        /* V3D_OUTPUT_IMAGE_FORMAT_*: a row of rt71_internal (v3d_job71.c) */
	uint32_t memory_format; /* V3D_MEMORY_FORMAT_* (RASTER .. UIF_XOR) */
	/* Raster: the row stride in bytes. UIF: the padded height in UIF blocks
	 * (v3dx_rcl.c v3d_surface_get_height_in_ub_or_stride). Other tilings: 0. */
	uint32_t height_in_ub_or_stride;
	bool r_b_swap;      /* load and store swap R and B (v3d_format_needs_tlb_rb_swap) */
	bool force_alpha_1; /* the load reads alpha as 1 (util_format_has_alpha1) */
	uint8_t pad[2];
};

/* A depth/stencil image in memory. */
struct v3d_job71_zs {
	uint32_t address;       /* GPU address; 0: none */
	/* V3D_OUTPUT_IMAGE_FORMAT_D24S8, _D32F or _D16 for the depth buffer
	 * (v3d_get_rt_format of the zsbuf), _S8 or _D24S8 for a separate
	 * stencil buffer */
	uint32_t format;
	uint32_t memory_format; /* V3D_MEMORY_FORMAT_* */
	uint32_t height_in_ub;  /* UIF: the padded height in UIF blocks; raster: the stride */
};

/* The job's framebuffer. */
struct v3d_job71_target {
	uint32_t width, height; /* pixels, 1 .. V3D_JOB71_MAX_SIZE */
	uint32_t tile_alloc;    /* GPU address, 64-byte aligned */
	/* Colour render targets 0 .. rts - 1, rts 0 .. V3D_JOB71_MAX_RTS. 0:
	 * no colour buffer (a depth-only framebuffer): render target 0 is
	 * configured unused, as Mesa does for nr_cbufs 0. */
	uint32_t rts;
	struct v3d_job71_rt rt[V3D_JOB71_MAX_RTS];
	/* The tile buffer's depth type, V3D_INTERNAL_TYPE_DEPTH_32F (0, also
	 * Mesa's value with no zsbuf), _24 or _16: the zsbuf's internal type
	 * (v3dx_rcl.c emit_rcl). */
	uint32_t depth_type;
	/* The depth (and, when its format is D24S8 and `s` has no address, the
	 * stencil) buffer, and a separate stencil buffer (`s`; Mesa's
	 * separate_stencil of a Z32F_S8X24 zsbuf). With `s`, `zs` is loaded and
	 * stored as buffer Z and `s` as buffer STENCIL; without, `zs` is
	 * ZSTENCIL when D24S8 and Z otherwise (v3dx_rcl.c
	 * zs_buffer_from_pipe_bits), or the aspects v3d_job71_clear's
	 * zs_load_aspects / zs_store_aspects name. */
	struct v3d_job71_zs zs, s;
};

/* gallium pipe_stencil_state for one face, ops already in V3D's encoding. */
struct v3d_job71_stencil_face {
	uint32_t func;                         /* V3D_COMPARE_FUNC_* */
	uint32_t sfail_op, zfail_op, zpass_op; /* V3D_STENCIL_OP_* */
	uint32_t ref, value_mask, write_mask;  /* 8 bits each */
};

/* The state of one draw (gallium terms), field for field v3d_job.h's struct
 * v3d_job_state so that one Rust translation can fill both. */
struct v3d_job71_state {
	struct v3d_viewport vp;
	/* pipe_scissor_state in the target's own rows (Y = 0 at the top): max is
	 * exclusive. Only when `scissor` is set. */
	bool scissor;
	uint32_t scissor_minx, scissor_miny, scissor_maxx, scissor_maxy;
	/* pipe_rasterizer_state */
	bool front_ccw;
	bool cull_front, cull_back;
	bool offset_tri;
	float offset_units, offset_scale, offset_clamp;
	float line_width;
	/* pipe_depth_stencil_alpha_state */
	bool depth_enabled, depth_writemask;
	uint32_t depth_func;                   /* V3D_COMPARE_FUNC_* */
	bool stencil_enabled, stencil_back_enabled;
	struct v3d_job71_stencil_face stencil_front, stencil_back;
	/* pipe_blend_state, render target 0, for every RT (no independent blend) */
	bool blend_enabled;
	uint32_t rgb_func, rgb_src, rgb_dst;   /* V3D_BLEND_MODE_*, V3D_BLEND_FACTOR_* */
	uint32_t alpha_func, alpha_src, alpha_dst;
	uint32_t colormask;                    /* PIPE_MASK_RGBA: bit 0 R .. bit 3 A, set = written */
	uint16_t blend_color_f16[4];           /* r, g, b, a as IEEE halves */
	/* gl_FragCoord.y's transform (v3d_frame.h v3d_wpos_gl); zeroed: absent */
	struct v3d_wpos wpos;
	/* SAMPLE_STATE's mask in a 4x job; not read (no 4x at 7.1 yet: 0xf) */
	uint32_t sample_mask;
	/* OCCLUSION_QUERY_COUNTER's address, 0 for none; 4-byte aligned */
	uint32_t oq_addr;
};

/* One sampler binding of a stage (v3d_job.h struct v3d_job_tmu, the same
 * layout). The TMU_CONFIG_P0/P1 uniforms read the record addresses, the
 * TEXTURE_* uniforms the sizes; a field no uniform of the stream asks for
 * is not read (its 0 is refused only when one does). */
struct v3d_job71_tmu {
	uint32_t unit;
	/* GPU address of the 7.1 TEXTURE_SHADER_STATE record (v3d_tex71.h,
	 * V3D_TEX71_RECORD_ALIGN): P0's address; its low 4 bits must be 0 */
	uint32_t tex_state;
	/* GPU address of the SAMPLER_STATE record, the variant the view needs
	 * (write_tmu_p1's sampler_state_offset[variant]): P1's address; its low
	 * 3 bits must be 0 */
	uint32_t sampler;
	uint32_t width;
	uint32_t height;
	uint32_t depth;
	uint32_t array_size;
};

/* One draw, field for field v3d_job.h's struct v3d_job_draw. */
struct v3d_job71_draw {
	const struct v3d_job71_state *st;
	const struct v3d_fs_prog *fs;
	const struct v3d_vs_prog *vs, *cs;
	struct v3d_uniform_storage vs_storage, fs_storage;
	const struct v3d_vertex_attr *attrs; /* record i is the programs' vattr_sizes[i] */
	uint32_t num_attrs;
	uint32_t prim;                       /* V3D_PRIM_* */
	uint32_t count;                      /* vertices, or indices when indexed */
	uint32_t first;                      /* arrays: the first vertex */
	bool indexed;
	uint32_t index_type;                 /* V3D_INDEX_TYPE_* */
	uint32_t index_buffer, index_buffer_bytes; /* INDEX_BUFFER_SETUP */
	uint32_t index_offset;               /* bytes from index_buffer to the first index */
	const struct v3d_job71_tmu *tmu;     /* the FS stream's sampler bindings */
	uint32_t num_tmu;
	const struct v3d_job71_tmu *vs_tmu;  /* the VS and CS streams' */
	uint32_t num_vs_tmu;
	bool vs_writes_psiz;                 /* the record's Point size in shaded vertex data */
	uint32_t instances;                  /* 0 or 1: one; more: the instanced packets */
};

/* What the render list does with the tile buffer around the draws. */
struct v3d_job71_clear {
	/* bit n: load RT n from its image (it was not cleared); an RT without
	 * an address is never loaded */
	uint32_t load_rts;
	/* The clear colour of RT n as the tile buffer holds it, packed per
	 * internal type as v3d_tlb_clear packs job->clear_color (one word at 32
	 * bpp, two at 64, four at 128); RENDER_TARGET_PART1 takes word 0,
	 * PART2 word 1 and the low byte of word 2, PART3 the rest
	 * (v3dx_rcl.c emit_rcl, V3D_VERSION 71). A loaded RT's is not read
	 * (0 is written). */
	uint32_t color[V3D_JOB71_MAX_RTS][4];
	float z;           /* the Z/S the tile buffer starts from: the clear */
	uint32_t stencil;  /* values, or GL's defaults (1.0, 0) */
	/* the target's zs / s buffers: load before the draws, store after */
	bool zs_load, zs_store, s_load, s_store;
	/* the job clears depth/stencil (glClear's DEPTH or STENCIL bit): with
	 * neither loaded nor stored, the rendering mode's Early Depth/Stencil
	 * Clear is set (v3dx_rcl.c emit_rcl, job->early_zs_clear) */
	bool zs_cleared;
	/* What `zs_load` and `zs_store` move of the `zs` buffer, V3D_JOB71_ZS_*
	 * bits (Mesa's job->load / job->store PIPE_CLEAR_DEPTH and _STENCIL
	 * bits, v3dx_rcl.c zs_buffer_from_pipe_bits): DEPTH | STENCIL is buffer
	 * ZSTENCIL, DEPTH alone Z, STENCIL alone STENCIL, each with the image's
	 * own format (a D24S8 image moved as Z or as STENCIL keeps
	 * Output/Input Image Format D24S8). 0: every aspect `zs` holds
	 * (ZSTENCIL for a D24S8 without `s`, Z otherwise), the behaviour before
	 * these fields. A STENCIL bit on a `zs` that holds no stencil (not
	 * D24S8, or with a separate `s`) or a bit above STENCIL makes
	 * v3d_job71_end refuse the job. Not read when the load or store is not
	 * asked for. */
	uint8_t zs_load_aspects, zs_store_aspects;
	uint8_t pad[1];
};

#define V3D_JOB71_ZS_DEPTH   1u
#define V3D_JOB71_ZS_STENCIL 2u

struct v3d_job71 {
	struct v3d_cl *bcl, *rcl, *ind;
	struct v3d_job71_target t;
	struct v3d_tiling tiling; /* v3d_frame_tiling_gen(71, ...) of the target */
	uint32_t draws;
	bool broken;              /* a step failed: the job must not be submitted */
};

/* Where the job put what one draw wrote, for a checker (GPU addresses). */
struct v3d_job71_draw_layout {
	uint32_t fs_uniforms, vs_uniforms, cs_uniforms;
	uint32_t record, attr_records;
	struct V3D71_GL_SHADER_STATE_RECORD_DRAW_INDEX rec;
};

/* v3d_job71_draw's results. A draw that is not recorded leaves nothing
 * behind: the lists are rolled back to where they stood. */
#define V3D_JOB71_OK      0
#define V3D_JOB71_FULL    1 /* a list is full: end this job, draw into a new one */
#define V3D_JOB71_REFUSED 2 /* the draw asks for something this builder does not write */

/* Begin a job: the BCL prologue (v3dx_draw.c start_binning). False (and the
 * job is broken) for a target this builder refuses or a list overflow. */
bool v3d_job71_begin(struct v3d_job71 *j, const struct v3d_job71_target *t, struct v3d_cl *bcl,
		     struct v3d_cl *rcl, struct v3d_cl *ind);

/* Record one draw. `layout` may be NULL. */
int v3d_job71_draw(struct v3d_job71 *j, const struct v3d_job71_draw *d,
		   struct v3d_job71_draw_layout *layout);

/* End the job: the BCL epilogue, the generic tile list (in `ind`) and the
 * RCL. `tile_list_start`/`end` may be NULL. False when the job is broken, a
 * buffer to load or store is not given, a Z/S aspect is not one the buffer
 * holds, or a list overflowed; nothing may then be submitted. */
bool v3d_job71_end(struct v3d_job71 *j, const struct v3d_job71_clear *c, uint32_t *tile_list_start,
		   uint32_t *tile_list_end);

/* The submit's tile allocation size (qms) and tile state size for the job's
 * tiling and draws (v3d_frame_alloc_sizes), the former rounded up to
 * `page_size` (the kernel's buffer granule: 16 KiB on the Pi 5's kernel). */
void v3d_job71_alloc_sizes(const struct v3d_job71 *j, uint32_t page_size, uint32_t *tile_alloc_size,
			   uint32_t *tile_state_size);

#endif /* V3D_CL_VER == 71 */

#endif
