/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The V3D 7.1 clear-only frame (frame71.h). The sequence is
 * docs/notes/v3d71-clear-frame.md section 1 (Mesa 26.2.1's v3dX(start_binning),
 * v3dX(bcl_epilogue), v3dX(emit_rcl), emit_render_layer and
 * v3d_rcl_emit_generic_per_tile_list at V3D_VERSION 71), with the 4.2 frame
 * builder's own choices kept (driver/src/v3d/v3d_frame.c): a raster store of
 * render target 0, early Z disabled. Bit positions are the packet's, counted
 * from bit 0 of the opcode byte, as Mesa's unpack functions name them.
 */
#include <string.h>

#include "frame71.h"

/* Opcodes and lengths: v3d_packet.xml, the 7.1 column (the note, 1.1 to 1.3). */
#define OP_FLUSH                      4
#define OP_START_TILE_BINNING         6
#define OP_END_OF_RENDERING           13
#define OP_RETURN_FROM_SUB_LIST       18
#define OP_FLUSH_VCD_CACHE            19
#define OP_GENERIC_TILE_LIST          20
#define OP_BRANCH_TO_IMPLICIT_TILE    21
#define OP_SUPERTILE_COORDINATES      23
#define OP_CLEAR_RENDER_TARGETS       25
#define OP_END_OF_LOADS               26
#define OP_END_OF_TILE_MARKER         27
#define OP_STORE_TILE_BUFFER_GENERAL  29
#define OP_SET_INSTANCEID             54
#define OP_PRIM_LIST_FORMAT           56
#define OP_OCCLUSION_QUERY_COUNTER    92
#define OP_NUMBER_OF_LAYERS           119
#define OP_TILE_BINNING_MODE_CFG      120
#define OP_TILE_RENDERING_MODE_CFG    121 /* sub-id 0 common, 1 Z/S clear, 2 RT part 1 */
#define OP_SUPERTILE_CFG              122
#define OP_TILE_LIST_SET_BASE         123
#define OP_TILE_COORDINATES           124
#define OP_TILE_COORDINATES_IMPLICIT  125
#define OP_TILE_LIST_INITIAL_BLOCK    126

#define SUB_COMMON          0
#define SUB_ZS_CLEAR_VALUES 1
#define SUB_RT_PART1        2

#define TILE_ALLOC_BLOCK_64B  0
#define TILE_ALLOC_BLOCK_128B 1
#define STORE_BUFFER_RT0      0
#define STORE_BUFFER_NONE     8
#define MEMORY_FORMAT_RASTER  0
#define OUTPUT_FORMAT_RGBA8   27 /* Output Image Format rgba8 */
#define INTERNAL_BPP_32       0
#define RT_TYPE_CLAMP_8       8  /* Render Target Type Clamp "8" (X:154-175) */
#define INTERNAL_DEPTH_32F    0  /* Internal Depth Type depth_32f */
#define PRIM_LIST_TRIANGLES   2

void cl71_init(struct cl71 *cl, void *buf, uint32_t gpu, uint32_t cap)
{
	cl->p = buf;
	cl->gpu = gpu;
	cl->len = 0;
	cl->cap = cap;
	cl->ok = true;
}

/* One packet being written: its bytes, zeroed, and whether every field fit. */
struct pkt {
	uint8_t *b;
	bool ok;
};

static struct pkt begin(struct cl71 *cl, uint8_t opcode, uint32_t len)
{
	static uint8_t sink[16]; /* where a packet that does not fit is written */
	struct pkt k = { sink, true };

	if (cl->ok && len <= cl->cap - cl->len) {
		k.b = cl->p + cl->len;
		cl->len += len;
	} else {
		cl->ok = false;
	}
	memset(k.b, 0, len);
	k.b[0] = opcode;
	return k;
}

static void end(struct cl71 *cl, const struct pkt *k)
{
	if (!k->ok)
		cl->ok = false;
}

/* `v` into bits [start, start + width) of the packet, little-endian. */
static void put(struct pkt *k, uint32_t start, uint32_t width, uint64_t v)
{
	if (width < 64 && (v >> width) != 0) {
		k->ok = false;
		return;
	}
	for (uint32_t i = 0; i < width; i++) {
		if (v & ((uint64_t)1 << i))
			k->b[(start + i) / 8] |= (uint8_t)(1u << ((start + i) % 8));
	}
}

static uint32_t log2_tile(uint32_t edge)
{
	return edge == 64 ? 3 : edge == 32 ? 2 : edge == 16 ? 1 : 0;
}

void frame71_tiling(const struct frame71 *f, struct v3d_tiling *t)
{
	v3d_frame_tiling_gen(71, f->width, f->height, 1, false, 0, 4, t);
}

static void op_only(struct cl71 *cl, uint8_t opcode)
{
	struct pkt k = begin(cl, opcode, 1);
	end(cl, &k);
}

static void store_general(struct cl71 *cl, uint32_t buffer, uint32_t format, uint32_t stride, uint32_t address)
{
	struct pkt k = begin(cl, OP_STORE_TILE_BUFFER_GENERAL, 13);
	put(&k, 8, 4, buffer);
	put(&k, 12, 3, MEMORY_FORMAT_RASTER);
	/* flip Y (15), dither (16..17), decimate sample 0 (18..19): 0 */
	put(&k, 20, 6, format);
	/* clear buffer being stored (26), channel reverse (27), R/B swap (28): 0 */
	put(&k, 36, 20, stride); /* height in UB or stride: the byte stride for raster */
	/* height (56..71): 0, as Mesa's raster store */
	put(&k, 72, 32, address);
	end(cl, &k);
}

/* v3dX(start_binning) and the FLUSH of v3dX(bcl_epilogue): 19 bytes. */
static void emit_bcl(const struct frame71 *f, const struct v3d_tiling *t, struct cl71 *bcl)
{
	struct pkt k = begin(bcl, OP_NUMBER_OF_LAYERS, 2);
	put(&k, 8, 8, 1 - 1);
	end(bcl, &k);

	k = begin(bcl, OP_TILE_BINNING_MODE_CFG, 9);
	put(&k, 10, 2, TILE_ALLOC_BLOCK_128B); /* initial block size */
	put(&k, 12, 2, TILE_ALLOC_BLOCK_64B);  /* block size */
	put(&k, 16, 3, log2_tile(t->tile_w));
	put(&k, 19, 3, log2_tile(t->tile_h));
	put(&k, 40, 16, f->width - 1);
	put(&k, 56, 16, f->height - 1);
	end(bcl, &k);

	op_only(bcl, OP_FLUSH_VCD_CACHE);
	k = begin(bcl, OP_OCCLUSION_QUERY_COUNTER, 5); /* address 0: no query */
	end(bcl, &k);
	op_only(bcl, OP_START_TILE_BINNING);
	/* no draws: FLUSH caps every tile list with a return */
	op_only(bcl, OP_FLUSH);
}

/* v3d_rcl_emit_generic_per_tile_list: 27 bytes. */
static void emit_tile_list(const struct frame71 *f, struct cl71 *tl)
{
	op_only(tl, OP_TILE_COORDINATES_IMPLICIT);
	op_only(tl, OP_END_OF_LOADS); /* no loads: the whole target is cleared */

	struct pkt k = begin(tl, OP_PRIM_LIST_FORMAT, 2);
	put(&k, 8, 6, PRIM_LIST_TRIANGLES);
	end(tl, &k);
	k = begin(tl, OP_SET_INSTANCEID, 5); /* instance 0 */
	end(tl, &k);
	k = begin(tl, OP_BRANCH_TO_IMPLICIT_TILE, 2); /* tile list set 0 */
	end(tl, &k);

	store_general(tl, STORE_BUFFER_RT0, OUTPUT_FORMAT_RGBA8, f->stride, f->target);
	op_only(tl, OP_CLEAR_RENDER_TARGETS); /* job->clear_tlb: ready for the next tile */
	op_only(tl, OP_END_OF_TILE_MARKER);
	op_only(tl, OP_RETURN_FROM_SUB_LIST);
}

/* v3dX(emit_rcl) and emit_render_layer, layer 0: 96 bytes for one tile. */
static void emit_rcl(const struct frame71 *f, const struct v3d_tiling *t, uint32_t tl_start,
		     uint32_t tl_end, struct cl71 *rcl)
{
	struct pkt k = begin(rcl, OP_TILE_RENDERING_MODE_CFG, 9);
	put(&k, 8, 3, SUB_COMMON);
	put(&k, 12, 4, 1 - 1); /* one render target */
	put(&k, 16, 16, f->width);
	put(&k, 32, 16, f->height);
	/* 4x (50), double buffer (51), depth-buffer disable (52; Mesa leaves it
	 * 0), early Z direction LT/LE (53): 0 */
	put(&k, 54, 1, 1); /* early Z disable: no depth buffer */
	put(&k, 55, 4, INTERNAL_DEPTH_32F);
	/* early depth/stencil clear (59): 0 */
	put(&k, 60, 3, log2_tile(t->tile_w));
	put(&k, 63, 3, log2_tile(t->tile_h));
	end(rcl, &k);

	/* render target 0: TLB base 0, stride tile_w * words / 2 (one word at
	 * 32 bpp; v3d_util.c v3d_compute_rt_row_row_stride_128_bits) */
	k = begin(rcl, OP_TILE_RENDERING_MODE_CFG, 9);
	put(&k, 8, 3, SUB_RT_PART1);
	put(&k, 11, 3, 0);  /* render target number */
	put(&k, 15, 11, 0); /* base address */
	put(&k, 26, 7, t->tile_w / 2 - 1);
	put(&k, 33, 2, INTERNAL_BPP_32);
	put(&k, 35, 5, RT_TYPE_CLAMP_8);
	put(&k, 40, 32, f->clear_rgba8);
	end(rcl, &k);

	k = begin(rcl, OP_TILE_RENDERING_MODE_CFG, 9);
	put(&k, 8, 4, SUB_ZS_CLEAR_VALUES);
	/* stencil 0 (16..23), Z 0.0f (24..55) */
	end(rcl, &k);

	k = begin(rcl, OP_TILE_LIST_INITIAL_BLOCK, 2);
	put(&k, 8, 2, TILE_ALLOC_BLOCK_128B);
	put(&k, 10, 1, 1); /* auto-chained tile lists */
	end(rcl, &k);

	/* the set number shares the low address bits: 64-byte alignment */
	k = begin(rcl, OP_TILE_LIST_SET_BASE, 5);
	if (f->tile_alloc & 63)
		k.ok = false;
	put(&k, 8, 32, f->tile_alloc); /* tile list set 0 */
	end(rcl, &k);

	k = begin(rcl, OP_SUPERTILE_CFG, 9);
	put(&k, 8, 8, t->supertile_w - 1);
	put(&k, 16, 8, t->supertile_h - 1);
	put(&k, 24, 8, t->frame_w_supertiles);
	put(&k, 32, 8, t->frame_h_supertiles);
	put(&k, 40, 12, t->tiles_x);
	put(&k, 52, 12, t->tiles_y);
	/* multicore (64), raster order (68): 0; one bin tile list (69..71) */
	end(rcl, &k);

	/* the initial clear and GFXH-1742's two dummy tiles, the clear only in
	 * the first (no double buffer) */
	for (int i = 0; i < 2; i++) {
		k = begin(rcl, OP_TILE_COORDINATES, 4); /* column 0, row 0 */
		end(rcl, &k);
		op_only(rcl, OP_END_OF_LOADS);
		store_general(rcl, STORE_BUFFER_NONE, 0, 0, 0);
		if (i == 0)
			op_only(rcl, OP_CLEAR_RENDER_TARGETS);
		op_only(rcl, OP_END_OF_TILE_MARKER);
	}
	op_only(rcl, OP_FLUSH_VCD_CACHE);

	k = begin(rcl, OP_GENERIC_TILE_LIST, 9);
	put(&k, 8, 32, tl_start);
	put(&k, 40, 32, tl_end);
	end(rcl, &k);

	/* every supertile covering the frame (the draw area of a clear) */
	uint32_t stw = t->tile_w * t->supertile_w, sth = t->tile_h * t->supertile_h;
	for (uint32_t y = 0; y <= (f->height - 1) / sth; y++) {
		for (uint32_t x = 0; x <= (f->width - 1) / stw; x++) {
			k = begin(rcl, OP_SUPERTILE_COORDINATES, 3);
			put(&k, 8, 8, x);
			put(&k, 16, 8, y);
			end(rcl, &k);
		}
	}
	op_only(rcl, OP_END_OF_RENDERING);
}

bool frame71_build_clear(const struct frame71 *f, struct cl71 *bcl, struct cl71 *rcl, struct cl71 *tl)
{
	/* the binning packet's 16-bit minus-one size; the common packet's
	 * 16-bit size without the minus one */
	if (f->width == 0 || f->height == 0 || f->width > 0xffff || f->height > 0xffff)
		return false;

	struct v3d_tiling t;
	frame71_tiling(f, &t);

	emit_bcl(f, &t, bcl);
	uint32_t tl_start = cl71_end(tl);
	emit_tile_list(f, tl);
	uint32_t tl_end = cl71_end(tl);
	emit_rcl(f, &t, tl_start, tl_end, rcl);
	return bcl->ok && rcl->ok && tl->ok;
}
