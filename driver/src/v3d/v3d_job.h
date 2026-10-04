/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2014-2017 Broadcom
 * Ported from Mesa 26.2.1 src/gallium/drivers/v3d/ (v3dx_draw.c, v3dx_emit.c,
 * v3dx_rcl.c, v3d_job.c) (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * V3D 4.2 job builder: the binning and render control lists of one job (one
 * frame of one render target) with any number of draws, for the GL core's
 * backend (src/v3d/, docs/online-shaderc/v3d-backend.md "M6").
 *
 * v3d_frame.c builds the M4/M5 frames (one clear, one triangle) and stays as
 * it is; this is its general form. Each step names the Mesa 26.2.1 function it
 * follows (gallium v3d, V3D_VERSION 42): v3d_job_begin is start_binning,
 * v3d_job_draw is v3d_draw_vbo (v3dX(emit_state) with every dirty bit set,
 * or with the packet skip below its dirty groups only,
 * v3d_emit_gl_shader_state, the primitive), v3d_job_end is bcl_epilogue and
 * v3dX(emit_rcl) with v3d_rcl_emit_generic_per_tile_list. The packets are
 * v3d_cl.h, checked byte for byte against Mesa by tools/v3d/cl_golden.c.
 *
 * The state a draw takes is gallium's (pipe_rasterizer/zsa/blend state after
 * st/mesa): the GL-to-gallium translation (the y flip of a window surface,
 * glFrontFace, the scissor box) is the caller's, in src/v3d/, where it is
 * host-tested.
 *
 * What this cut does not do, refused rather than drawn wrong: 4x
 * multisampling with more than one render target, early Z (off: legal, only
 * slower), transform feedback, a base vertex, primitive restart. (Several
 * render targets, depth/stencil kept in memory, instancing and 4x
 * multisampling of one render target were added since the first cut; see
 * each field below.)
 *
 * No OS calls: the caller owns the memory and the addresses.
 */
#ifndef V3D_JOB_H
#define V3D_JOB_H

#include <stdint.h>
#include <stdbool.h>

#include "v3d_cl.h"
#include "v3d_frame.h" /* the program, attribute, viewport and uniform types */

/* The render target: raster RGBA8888, one sample, top row first in memory. */
struct v3d_job_target {
	uint32_t width, height;  /* pixels */
	/* GPU address of the first row. 0: no colour buffer (a depth/stencil-
	 * only framebuffer, Mesa's job with nr_cbufs 0): RT 0 is configured, so
	 * the TLB has room for a fragment's colour, but it is never loaded or
	 * stored; a tile list that then stores nothing stores buffer NONE
	 * (v3dx_rcl.c:315-321). Not with msaa. */
	uint32_t address;
	uint32_t stride;         /* bytes per row */
	uint32_t tile_alloc;     /* GPU address, 64-byte aligned */
	/* The target is 16-bit B5G6R5 (GL's RGB565: R in the high bits): the
	 * colour load and store use output image format bgr565, Mesa's rt format
	 * for PIPE_FORMAT_B5G6R5_UNORM (v3dx_format_table.c:81), with no R/B
	 * swap (v3d_formats.c:176). The tile buffer stays 8-bit RGBA
	 * (v3dx_format_table.c:225-234). False: RGBA8. */
	bool rgb565;
	/* Render targets 1..3 (multiple render targets, docs/online-shaderc/
	 * v3d-backend.md): rts is how many the job configures, 0 and 1 both
	 * meaning RT 0 alone (the fields above). Each extra one is RGBA8 raster
	 * at rt[n - 1]; an address of 0 is a GL_NONE draw buffer: the RT is
	 * configured, so a program's write to it has TLB space, but it is never
	 * loaded or stored, so the write is dropped. */
	uint32_t rts;
	struct v3d_job_rt {
		uint32_t address;  /* GPU address of the first row; 0: none */
		uint32_t stride;   /* bytes per row */
	} rt[3];
	/* 4x multisampling (docs/online-shaderc/v3d-backend.md "Multisampling"):
	 * the tile buffer holds four samples a pixel (Mesa's job->msaa:
	 * TILE_BINNING_MODE_CFG and the rendering mode's Multisample Mode (4x),
	 * v3dx_draw.c:92, v3dx_rcl.c:763-764), so the tiles are smaller
	 * (v3d_frame_tiling_ms). RT 0 is stored resolved into `address` (store
	 * general, decimate 4X, v3dx_rcl.c:159-166): the window, glReadPixels
	 * and a texture read see the resolved image after every job. Its
	 * samples are kept between jobs in `ms_address` when not 0: RGBA8, the
	 * layout Mesa gives a multisampled resource (single-level UIF at twice
	 * the width and height, v3d_resource.c:612-642), stored and loaded with
	 * decimate ALL_SAMPLES (v3dx_rcl.c:104-107, :157-158). A job that loads
	 * RT 0 at 4x needs it; the Z/S buffers of a 4x job (v3d_job_clear) are
	 * 4x ones, loaded and stored with ALL_SAMPLES too. One RT only at 4x. */
	bool msaa;
	uint32_t ms_address;         /* GPU address, 4 KiB aligned; 0: samples not kept */
	uint32_t ms_memory_format;   /* V3D_MEMORY_FORMAT_UIF_* */
	uint32_t ms_height_in_ub;    /* the padded height in UIF blocks */
};

/* gallium pipe_stencil_state for one face, ops already in V3D's encoding. */
struct v3d_job_stencil_face {
	uint32_t func;                         /* V3D_COMPARE_FUNC_* */
	uint32_t sfail_op, zfail_op, zpass_op; /* V3D_STENCIL_OP_* */
	uint32_t ref, value_mask, write_mask;  /* 8 bits each */
};

/* The state of one draw (gallium terms). */
struct v3d_job_state {
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
	struct v3d_job_stencil_face stencil_front, stencil_back;
	/* pipe_blend_state, render target 0 */
	bool blend_enabled;
	uint32_t rgb_func, rgb_src, rgb_dst;   /* V3D_BLEND_MODE_*, V3D_BLEND_FACTOR_* */
	uint32_t alpha_func, alpha_src, alpha_dst;
	uint32_t colormask;                    /* PIPE_MASK_RGBA: bit 0 R .. bit 3 A, set = written */
	uint16_t blend_color_f16[4];           /* r, g, b, a as IEEE halves */
	/* gl_FragCoord.y's transform (v3d_frame.h, v3d_wpos_gl): (-1, the
	 * target's height) for a window, (1, 0) for an FBO. Zeroed, it is
	 * absent, and a program that reads it is refused. */
	struct v3d_wpos wpos;
	/* SAMPLE_STATE's mask in a 4x job (bit n: sample n is written): the
	 * sample mask gallium's state tracker makes of glSampleCoverage
	 * (st_atom_msaa.c) and Mesa sets only for a multisampled job
	 * (v3dx_emit.c:728-736: mask = job->msaa ? sample_mask : 0xf). A
	 * single-sampled job writes 0xf whatever this holds; a 4x job writes
	 * this, so a 4x caller sets it (0xf: every sample). */
	uint32_t sample_mask;
};

/* Primitive codes: Mesa's "Primitive" enum (V3D_PRIM_POINTS ..
 * V3D_PRIM_TRIANGLE_FAN, v3d_packet_v42_pack.h), the GL modes one for one. */

struct v3d_job_draw {
	const struct v3d_job_state *st;
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
	/* The textures the programs sample, per stage: the P0/P1 uniforms of a
	 * binding take its records' addresses (M6 textures). The unit in a
	 * P0/P1's data (bits 24..31) is the stage's sampler binding, a numbering
	 * of its own per stage: the FS stream reads `tmu`, the VS and CS
	 * streams read `vs_tmu`. */
	const struct v3d_job_tmu *tmu;
	uint32_t num_tmu;
	const struct v3d_job_tmu *vs_tmu;
	uint32_t num_vs_tmu;
	/* The VS writes the point size into its VPM output (its prog_data's
	 * writes_psiz): the record's Point size in shaded vertex data. Mesa's VS
	 * writes it exactly when the draw is points (vs_key per_vertex_point_size,
	 * v3d_program.c), and sets the bit for those draws (v3dx_draw.c:556-558):
	 * the same rule, taken from the program instead of the primitive. */
	bool vs_writes_psiz;
	/* Instances of the draw: 0 or 1 is one (VERTEX_ARRAY_PRIMS /
	 * INDEXED_PRIM_LIST), more is VERTEX_ARRAY_INSTANCED_PRIMS /
	 * INDEXED_INSTANCED_PRIM_LIST (v3dx_draw.c:1322-1366): the hardware walks
	 * the instances, gl_InstanceID and the attrs' divisors with them. */
	uint32_t instances;
};

/* Mesa's enum quniform_contents (v3d_compiler.h:181-264): the V3D 4.x TMU
 * configuration words. The data is unit << 24 | the compiler's p0/p1 bits
 * (v3d_unit_data_create, v3d_compiler.h:371-385). */
#define V3D_QUNIFORM_TMU_CONFIG_P0 40
#define V3D_QUNIFORM_TMU_CONFIG_P1 41

/* One sampler binding of a stage: the GPU addresses of its texture shader state
 * and its sampler state (driver/src/v3d/v3d_tex.h), each aligned to
 * V3D_TEX_RECORD_ALIGN (32): the uniform ORs the compiler's bits into the
 * low ones (v3d_uniforms.c write_tmu_p0/p1). */
struct v3d_job_tmu {
	uint32_t unit;
	uint32_t tex_state;
	uint32_t sampler;
};

/* What the render list does with the tile buffer before the draws. */
struct v3d_job_clear {
	bool load_rt0;       /* load the target's colour (it was not cleared) */
	uint32_t rgba8;      /* the colour clear, R in bits 0..7 (when !load_rt0) */
	float z;             /* the Z/S the tile buffer starts from: the clear */
	uint32_t stencil;    /* values, or GL's defaults (1.0, 0) */
	/* Depth and stencil kept in memory between jobs (M6 step C): a D24S8
	 * buffer the TLB loads at the start of every tile and stores at its
	 * end, as Mesa's v3d does for a zsbuf (v3dx_rcl.c load_general /
	 * store_general, ZSTENCIL: the V3D 4.2 loses the clear of one of Z and
	 * S when only the other is loaded, v3dx_draw.c:1676-1689). zs_address 0
	 * is no buffer: neither is emitted. */
	uint32_t zs_address;        /* GPU address, 4 KiB aligned */
	uint32_t zs_memory_format;  /* V3D_MEMORY_FORMAT_* (the layout's tiling) */
	uint32_t zs_height_in_ub;   /* UIF: the padded height in UIF blocks; else 0 */
	bool zs_load, zs_store;
	/* The target's format has no alpha (GL_RGB, GL_RGB565, stored as RGBA8):
	 * the colour load reads its alpha as 1 (load_general's Force alpha 1,
	 * as Mesa's v3d does for a format whose alpha swizzle is 1,
	 * v3dx_rcl.c:99), and the draws do not write alpha (the job state's
	 * colormask), so what is stored keeps alpha 1. */
	bool force_alpha_1;
	/* The target stores B,G,R,X (an XRGB8888 surface or EGLImage): the
	 * colour load and store swap R and B (Mesa's r_b_swap for a format whose
	 * first swizzle is Z, v3d_formats.c v3d_format_needs_tlb_rb_swap,
	 * v3dx_rcl.c:98 and :150); the tile buffer holds R,G,B,A throughout. */
	bool r_b_swap;
	/* Depth and stencil kept apart (an FBO whose depth and stencil
	 * renderbuffers differ, docs/online-shaderc/v3d-backend.md "Z and
	 * stencil apart"): s_address not 0 makes zs_* the depth's buffer,
	 * loaded and stored as buffer Z, and s_* the stencil's, loaded and
	 * stored as buffer STENCIL; both D24S8 in memory, as Mesa loads or
	 * stores one half of a packed zsbuf (v3dx_rcl.c:173-184
	 * zs_buffer_from_pipe_bits, :220-234 and :288-304 for the order:
	 * stencil then Z on load, Z then stencil on store). s_address 0: one
	 * buffer, ZSTENCIL, as above. zs_address 0 with s_address set: the
	 * stencil alone (a stencil-only framebuffer), nothing of Z. */
	uint32_t s_address;
	uint32_t s_memory_format;
	uint32_t s_height_in_ub;
	bool s_load, s_store;
	/* Render targets 1..3, as load_rt0 / rgba8 are for RT 0 (the target's
	 * rt[n - 1]; ignored past its rts). */
	bool load_rt[3];
	uint32_t rgba8_rt[3];
	/* The stencil's own buffer (s_address) is S8, 1 byte a pixel: loaded
	 * and stored with image format S8, as Mesa loads and stores the
	 * separate stencil of a Z32F_S8X24 zsbuf (v3dx_rcl.c:92-93 load, :139-140
	 * store); false: D24S8. */
	bool s_s8;
};

/*
 * Packet skip (v3d_job_pack_skip, which the Zephyr OSAL sets from
 * CONFIG_YAKOGL_V3D_PACK_SKIP; on by default since batch 94's board
 * evidence, off on host builds where the Kconfig symbol is not compiled
 * in).
 *
 * The state packets of the BCL are sticky: a register keeps what the last
 * packet in the list set until another packet of its kind comes. Mesa relies
 * on that: v3dX(emit_state) (v3dx_emit.c:229-736) emits each state packet
 * only when its dirty flag is set, and the flags are all set when a job
 * starts (v3d_job.c:436, v3d->dirty = ~0) and cleared after each draw
 * (v3dx_draw.c:1269). With the knob on, this builder does the same with
 * values instead of flags: the state packets of a draw fall into groups
 * (V3D_PACK_*), each packed from a few inputs, and a group is packed and
 * written only when it was not written yet in this job or its inputs differ
 * from the ones it was last written with. POINT_SIZE (the 4.2 binner FIFO
 * workaround, v3dx_emit.c:422-437) and the draw's own packets (VCM_CACHE_SIZE,
 * GL_SHADER_STATE, INDEX_BUFFER_SETUP, the primitive) are written on every
 * draw. A draw that is not recorded leaves the groups' last inputs as they
 * were, as it leaves the lists. With the knob off every group is written on
 * every draw, as before, and nothing is compared.
 */
enum {
	V3D_PACK_CLIP,         /* CLIP_WINDOW */
	V3D_PACK_CFG,          /* CFG_BITS */
	V3D_PACK_DEPTH_OFFSET, /* DEPTH_OFFSET, when offset_tri */
	V3D_PACK_LINE,         /* LINE_WIDTH */
	V3D_PACK_VIEWPORT,     /* CLIPPER_XY, CLIPPER_Z, CLIPPER_Z_MIN_MAX, VIEWPORT_OFFSET */
	V3D_PACK_BLEND,        /* BLEND_ENABLES, and BLEND_CFG when blending */
	V3D_PACK_COLOR_MASK,   /* COLOR_WRITE_MASKS */
	V3D_PACK_BLEND_COLOR,  /* BLEND_CONSTANT_COLOR */
	V3D_PACK_STENCIL,      /* STENCIL_CFG (one or two), when stencil_enabled */
	V3D_PACK_VARYINGS,     /* FLAT_SHADE, NON_PERSPECTIVE and CENTROID flags */
	V3D_PACK_FIXED,        /* TRANSFORM_FEEDBACK_SPECS, OCCLUSION_QUERY_COUNTER, SAMPLE_STATE */
	V3D_PACK_GROUPS
};

/* The most input words one group has (V3D_PACK_STENCIL: 15). */
#define V3D_PACK_KEY_MAX 16

/* Packet counts. emitted: the BCL packets the draws wrote (state and draw
 * packets); skipped: the state packets they left out; compared: the groups
 * compared with their last written inputs; mismatched: those that differed
 * (and were written). */
struct v3d_job_pack_counts {
	uint32_t emitted, skipped, compared, mismatched;
};

struct v3d_job_pack {
	bool on;              /* v3d_job_pack_skip as the job began */
	uint32_t valid;       /* bit g: group g was written in this job */
	uint32_t key[V3D_PACK_GROUPS][V3D_PACK_KEY_MAX]; /* group g's inputs, as last written */
	struct v3d_job_pack_counts n; /* this job's draws */
};

/* The knob. v3d_job_begin reads it, so one job is built one way. */
extern bool v3d_job_pack_skip;

/* The counts of every job ended since start (v3d_job_end adds each one's),
 * and how many jobs that is. Either pointer may be NULL. */
void v3d_job_pack_totals(struct v3d_job_pack_counts *out, uint32_t *jobs);

struct v3d_job {
	struct v3d_cl *bcl, *rcl, *ind;
	struct v3d_job_target t;
	uint32_t draws;
	uint32_t defaults;    /* GPU address of the default attribute values */
	bool broken;          /* a step failed: the job must not be submitted */
	struct v3d_job_pack pack;
};

/* Where the job put what one draw wrote, for a checker (GPU addresses). */
struct v3d_job_draw_layout {
	uint32_t fs_uniforms, vs_uniforms, cs_uniforms;
	uint32_t record, attr_records;
	struct V3D42_GL_SHADER_STATE_RECORD rec;
};

/* Begin a job: the BCL prologue and the default attribute values. Returns
 * false (and the job is broken) when a list overflowed or the target is
 * empty. */
bool v3d_job_begin(struct v3d_job *j, const struct v3d_job_target *t, struct v3d_cl *bcl,
		   struct v3d_cl *rcl, struct v3d_cl *ind);

/* v3d_job_draw's results. A draw that is not recorded leaves nothing behind:
 * the lists are rolled back to where they stood, so the job's earlier draws
 * are intact and it can still be ended and submitted. */
#define V3D_JOB_OK      0
#define V3D_JOB_FULL    1  /* a list is full: end this job, draw into a new one */
#define V3D_JOB_REFUSED 2  /* the draw asks for something this builder does not write */

/* Record one draw. `layout` may be NULL. */
int v3d_job_draw(struct v3d_job *j, const struct v3d_job_draw *d, struct v3d_job_draw_layout *layout);

/* End the job: the BCL epilogue, the generic tile list (in `ind`) and the
 * RCL. `tile_list_start`/`end` may be NULL. Returns false when the job is
 * broken or a list overflowed; nothing may then be submitted. */
bool v3d_job_end(struct v3d_job *j, const struct v3d_job_clear *c, uint32_t *tile_list_start,
		 uint32_t *tile_list_end);

#endif
