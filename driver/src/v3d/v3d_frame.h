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
 * V3D 4.2 frame builder: the binning and render control lists of one frame.
 *
 * Milestones M3/M4 of the bring-up
 * (tools/cts/log/board-sessions/pi4-v3d-bringup.md): a frame with no draws that
 * clears one RGBA8888 render target through the tile buffer (a TLB clear) and
 * stores it to a raster buffer. M5: the same clear followed by one
 * non-indexed triangle draw with the three QPU programs of a GL program
 * (v3dx_draw.c v3d_draw_vbo, v3dx_emit.c v3dX(emit_state),
 * v3d_emit_gl_shader_state, v3d_uniforms.c v3d_write_uniforms). The lists follow Mesa's gallium v3d driver for
 * version 42 packet for packet (v3dx_draw.c start_binning, v3dx_job.c
 * bcl_epilogue, v3dx_rcl.c emit_rcl / emit_render_layer /
 * v3d_rcl_emit_generic_per_tile_list), so a V3D_DEBUG=clif dump of Mesa doing
 * the same clear on the board is the byte reference.
 *
 * No OS calls: the caller owns the memory and the addresses.
 */
#ifndef V3D_FRAME_H
#define V3D_FRAME_H

#include <stdint.h>
#include <stdbool.h>

#include "v3d_cl.h"

/* Tile geometry of a frame (v3d_util.c v3d_choose_tile_size, the 4.x branch). */
struct v3d_tiling {
	uint32_t tile_w, tile_h;       /* pixels */
	uint32_t tiles_x, tiles_y;
	uint32_t supertile_w, supertile_h; /* tiles */
	uint32_t frame_w_supertiles, frame_h_supertiles;
};

/* One render target, 1 RT, no MSAA, no double buffer, internal bpp 32. */
void v3d_frame_tiling(uint32_t width, uint32_t height, struct v3d_tiling *t);
/* The same for `rts` render targets (1..4) of internal bpp 32. */
void v3d_frame_tiling_rts(uint32_t width, uint32_t height, uint32_t rts, struct v3d_tiling *t);
/* The same with 4x multisampling when `msaa`. */
void v3d_frame_tiling_ms(uint32_t width, uint32_t height, uint32_t rts, bool msaa, struct v3d_tiling *t);

/* Tile allocation and tile state sizes for a frame (v3d_util.c
 * v3d_tile_alloc_sizes, one layer). `draws` is the number of draw calls. */
void v3d_frame_alloc_sizes(const struct v3d_tiling *t, uint32_t draws, uint32_t page_size,
			   uint32_t *tile_alloc_size, uint32_t *tile_state_size);

struct v3d_clear_frame {
	uint32_t width, height;
	uint32_t clear_rgba8;      /* R in bits 0..7, A in 24..31 (R8G8B8A8_UNORM) */
	uint32_t target;           /* GPU address of the raster RGBA8888 buffer */
	uint32_t stride;           /* its row stride in bytes */
	uint32_t tile_alloc;       /* GPU address, 64-byte aligned */
};

/*
 * Write the three lists of a clear frame: the BCL, the RCL, and the generic
 * per-tile list the RCL points at (Mesa keeps that one in its "indirect"
 * buffer). Returns false if any list overflowed or a field was out of range;
 * nothing may then be submitted.
 */
bool v3d_frame_build_clear(const struct v3d_clear_frame *f, struct v3d_cl *bcl, struct v3d_cl *rcl,
			   struct v3d_cl *tile_list);

/* --- M5: one triangle draw --- */

/* Mesa's enum quniform_contents (compiler/v3d_compiler.h), the values the
 * frame builder can write. Every other value is refused. */
#define V3D_QUNIFORM_CONSTANT          0
#define V3D_QUNIFORM_UNIFORM           1 /* the stage's GL uniform storage, word `data` */
#define V3D_QUNIFORM_VIEWPORT_X_SCALE  2
#define V3D_QUNIFORM_VIEWPORT_Y_SCALE  3
#define V3D_QUNIFORM_VIEWPORT_Z_OFFSET 4
#define V3D_QUNIFORM_VIEWPORT_Z_SCALE  5

struct v3d_uniform {
	uint32_t contents; /* V3D_QUNIFORM_* */
	uint32_t data;
};

/* gallium's pipe_viewport_state: window = ndc * scale + translate. For GL,
 * v3d_viewport_gl() computes it as st/mesa does. */
struct v3d_viewport {
	float scale[3], translate[3];
};

/*
 * st_update_viewport + _mesa_get_viewport_xform (st_atom_viewport.c:55-70,
 * main/viewport.c:535-561) for glViewport(x, y, w, h), glDepthRange(n, f),
 * GL_LOWER_LEFT / GL_NEGATIVE_ONE_TO_ONE. `y0_top` is st/mesa's
 * fb_orientation == Y_0_TOP, true for a window-system framebuffer whose first
 * row in memory is the top of the picture; `fb_height` is that buffer's height.
 */
void v3d_viewport_gl(float x, float y, float w, float h, float n, float f, bool y0_top,
		     uint32_t fb_height, struct v3d_viewport *vp);

/*
 * gl_FragCoord.y (docs/online-shaderc/v3d-backend.md, M6 "gl_FragCoord").
 * The V3D reports an upper-left origin (v3d_screen.c:313-316); GL's is lower
 * left for a window. Mesa's st/mesa passes STATE_FB_WPOS_Y_TRANSFORM as a GL
 * state uniform (prog_statevars.c:736-751); we have no state uniforms, so
 * the two words are contents codes of our own, far outside Mesa's enum
 * quniform_contents. The compiler (src/shaderc/online/v3d/select.rs,
 * QUNIFORM_FB_WPOS_Y_*) emits y = FYCD * scale + offset. Their `data` is 0;
 * any other data is refused.
 */
#define V3D_QUNIFORM_FB_WPOS_Y_SCALE  0x10000u
#define V3D_QUNIFORM_FB_WPOS_Y_OFFSET 0x10001u

/*
 * gl_PointCoord.y the same way: Mesa passes STATE_FB_PNTC_Y_TRANSFORM
 * (prog_statevars.c:754-763) and nir_lower_pntc_ytransform.c:76-81 emits
 * y = y_raw * scale + offset. ES has no GL_POINT_SPRITE_COORD_ORIGIN, so the
 * origin is always upper left and only the draw buffer's flip decides:
 * (-1, 1) for a window, (1, 0) for an FBO. The V3D has no sprite-origin bit
 * of its own (v3d_packet.xml), so these two words are the whole flip. They
 * come from the same struct v3d_wpos (its scale says window or FBO); `data`
 * is 0.
 */
#define V3D_QUNIFORM_FB_PNTC_Y_SCALE  0x10002u
#define V3D_QUNIFORM_FB_PNTC_Y_OFFSET 0x10003u

/* The two words: (-1, height) for a window whose first row in memory is the
 * top of the picture (y0_top, as for the viewport), (1, 0) for an FBO. All
 * zero is absent: a draw that reads them is refused, never drawn with 0. */
struct v3d_wpos {
	float y_scale, y_offset;
};

void v3d_wpos_gl(bool y0_top, uint32_t fb_height, struct v3d_wpos *w);

/* The word of a QUNIFORM_FB_WPOS_Y_* or QUNIFORM_FB_PNTC_Y_* entry for a
 * target `fb_height` rows high, shared by both uniform writers. False, and
 * *out untouched, when `data` is not 0, when the scale is not exactly 1 or
 * -1, or when the offset is not the one that scale means (0, or
 * fb_height). */
bool v3d_wpos_uniform(const struct v3d_wpos *w, uint32_t fb_height, uint32_t contents,
		      uint32_t data, uint32_t *out);

/* The fragment shader's prog_data, as tools/v3d/v3d_shc prints it. */
struct v3d_fs_prog {
	uint32_t code;       /* GPU address of the QPU code, 8-byte aligned */
	uint32_t threads;
	bool single_seg;
	uint32_t num_inputs;
	/* word 0 of each flags array: M5 supports num_inputs <= 24 and all zero */
	uint32_t flat_shade_flags, noperspective_flags, centroid_flags;
	bool writes_z, disable_ez, uses_center_w, lock_scoreboard_on_first_thrsw;
	bool uses_implicit_point_line_varyings, uses_pid;
	const struct v3d_uniform *uniforms;
	uint32_t num_uniforms;
};

/* The vertex or coordinate shader's prog_data (v3d_vs_prog_data). */
struct v3d_vs_prog {
	uint32_t code;
	uint32_t threads;
	bool single_seg;
	uint32_t vpm_input_size, vpm_output_size;
	bool separate_segments;
	uint32_t vcm_cache_size;
	bool uses_vid, uses_iid, uses_biid;
	uint8_t vattr_sizes[16]; /* per vertex attribute record */
	const struct v3d_uniform *uniforms;
	uint32_t num_uniforms;
};

/* One vertex element (pipe_vertex_element + its vertex buffer). M5 accepts
 * 32-bit float components only (v3dx_state.c v3d_vertex_state_create). */
struct v3d_vertex_attr {
	uint32_t address;       /* GPU address of the first vertex's value */
	uint32_t stride;        /* bytes */
	uint32_t components;    /* 1..4 */
	/* 0: one value per vertex; n: one per n instances (the attribute
	 * record's 16-bit Instance Divisor, v3d_packet.xml:1493; Mesa
	 * v3dx_state.c:388-389). Only an instanced draw (v3d_job_draw's
	 * instances > 1) walks instances. */
	uint32_t divisor;
};

#define V3D_MAX_ATTRS 16

/*
 * A stage's GL uniform storage: what gallium's constbuf 0 user buffer is in
 * Mesa. QUNIFORM_UNIFORM's data is a 32-bit word index into it
 * (v3d_uniforms.c:251-252, gallium_uniforms[data]). The VS and the CS read the
 * vertex shader's storage. Zeroed, it is absent, and a QUNIFORM_UNIFORM is
 * refused; so is an index at or past `count`, never read past the end.
 */
struct v3d_uniform_storage {
	const uint32_t *words;
	uint32_t count;
};

struct v3d_triangle_frame {
	struct v3d_clear_frame clear;   /* target, size, clear colour, tile alloc */
	struct v3d_uniform_storage vs_storage, fs_storage;
	struct v3d_viewport vp;
	bool front_ccw;                 /* gallium rasterizer front_ccw */
	const struct v3d_fs_prog *fs;
	const struct v3d_vs_prog *vs, *cs;
	const struct v3d_vertex_attr *attrs; /* record i is vattr_sizes[i] */
	uint32_t num_attrs;
	uint32_t first_vertex, vertex_count; /* GL_TRIANGLES, a multiple of 3 */
	struct v3d_wpos wpos;           /* zeroed: absent (v3d_wpos_uniform) */
};

/* Where the builder put what it wrote into the indirect list (GPU addresses),
 * so that a checker can find the records again. */
struct v3d_triangle_layout {
	uint32_t defaults;                     /* default attribute values */
	uint32_t fs_uniforms, vs_uniforms, cs_uniforms;
	uint32_t record;                       /* GL_SHADER_STATE_RECORD */
	uint32_t attr_records;                 /* the first attribute record */
	uint32_t tile_list_start, tile_list_end;
	struct V3D42_GL_SHADER_STATE_RECORD rec; /* the record's values */
	struct V3D42_GL_SHADER_STATE_ATTRIBUTE_RECORD attr[V3D_MAX_ATTRS];
};

/*
 * Write the lists of a clear-then-triangle frame. `indirect` takes what Mesa
 * keeps in the job's indirect buffer, in Mesa's order: the uniform streams
 * (FS, VS, CS), the shader state record and the attribute records, then the
 * generic per-tile list. The default attribute values (Mesa: a separate
 * state_uploader buffer) are written first into `indirect` as well.
 * Returns false, and nothing may be submitted, when any list overflowed, a
 * field did not fit, or the program asks for something M5 does not write
 * (a uniform contents other than the ones above, non-zero varying flags,
 * a non-float attribute, a vertex count that is not a whole triangle list).
 * `layout` may be NULL.
 */
bool v3d_frame_build_triangle(const struct v3d_triangle_frame *f, struct v3d_cl *bcl,
			      struct v3d_cl *rcl, struct v3d_cl *indirect,
			      struct v3d_triangle_layout *layout);

#endif
