#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# clear71: the lists rpi5_clear submits on the Raspberry Pi 5
# (tools/probes/rpi_5/rpi5_clear/src/frame71.c, which writes its 7.1 packets
# itself) against the same frame packed with Mesa's own 7.1 packers
# (v3d_packet_v71_pack.h), byte for byte: the binning list, the render list and
# the generic tile list, for target sizes on and next to the tile boundaries,
# several clear colours, strides and addresses. The tile size is the one
# v3d_frame_tiling_gen gives at 71 (checked against Mesa by tiling71.sh). The
# 64x64 frame must also be 19 / 96 / 27 bytes, the counts of
# docs/notes/v3d71-clear-frame.md section 1 (Mesa's own dump, shim71/clear.clif),
# and a frame the builder must refuse is refused.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE with
# build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=clear71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
probe=tools/probes/rpi_5/rpi5_clear/src
inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H)
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$mb/src/broadcom" -I"$mb/src")
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi

# Mesa's packers, alone in their file (their value macros, NONE, Z, ..., would
# meet the 4.2 ones of v3d_frame.h otherwise).
cat >"$dir/clear71_mesa.c" <<'EOF'
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include <stdint.h>
#include <string.h>

#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#include "cle/v3d_packet_v71_pack.h"

#include "clear71_ref.h"

struct cl {
	uint8_t *base, *p;
};

/* cl_emit as Mesa's v3dx_cl.h has it, for 7.1 only */
#define cl_emit(cl, T, nm)                                                                         \
	for (struct V3D71_##T nm = { V3D71_##T##_header }, *_once = (void *)1; _once;              \
	     _once = NULL, V3D71_##T##_pack(NULL, (cl)->p, &nm), (cl)->p += V3D71_##T##_length)

static uint32_t log2_tile(uint32_t e)
{
	return e == 64 ? 3 : e == 32 ? 2 : e == 16 ? 1 : 0;
}

static void store(struct cl *cl, unsigned buffer, unsigned out_fmt, unsigned stride, uint32_t addr)
{
	cl_emit(cl, STORE_TILE_BUFFER_GENERAL, s) {
		s.buffer_to_store = buffer;
		s.memory_format = 0; /* raster */
		s.output_image_format = out_fmt;
		s.decimate_mode = 0;
		s.height_in_ub_or_stride = stride;
		s.address = addr;
	}
}

/* v3dx_draw.c start_binning, v3dx_job.c bcl_epilogue, v3dx_rcl.c emit_rcl,
 * emit_render_layer and v3d_rcl_emit_generic_per_tile_list at 71, for one
 * raster RGBA8 target cleared and stored (the 4.2 builder's choices). */
void ref_build(const struct ref_frame *f, uint8_t *bcl, uint8_t *rcl, uint8_t *tl, uint32_t tl_gpu,
	       uint32_t len[3])
{
	struct cl b = { bcl, bcl }, r = { rcl, rcl }, t = { tl, tl };
	uint32_t lw = log2_tile(f->tile_w), lh = log2_tile(f->tile_h);

	cl_emit(&b, NUMBER_OF_LAYERS, n) { n.number_of_layers = 1; }
	cl_emit(&b, TILE_BINNING_MODE_CFG, c) {
		c.tile_allocation_initial_block_size = TILE_ALLOCATION_INITIAL_BLOCK_SIZE_128B;
		c.tile_allocation_block_size = TILE_ALLOCATION_BLOCK_SIZE_64B;
		c.log2_tile_width = lw;
		c.log2_tile_height = lh;
		c.width_in_pixels = f->width;
		c.height_in_pixels = f->height;
	}
	cl_emit(&b, FLUSH_VCD_CACHE, c) { (void)c; }
	cl_emit(&b, OCCLUSION_QUERY_COUNTER, c) { c.address = 0; }
	cl_emit(&b, START_TILE_BINNING, s) { (void)s; }
	cl_emit(&b, FLUSH, x) { (void)x; }

	cl_emit(&t, TILE_COORDINATES_IMPLICIT, c) { (void)c; }
	cl_emit(&t, END_OF_LOADS, e) { (void)e; }
	cl_emit(&t, PRIM_LIST_FORMAT, p) { p.primitive_type = LIST_TRIANGLES; }
	cl_emit(&t, SET_INSTANCEID, i) { i.instance_id = 0; }
	cl_emit(&t, BRANCH_TO_IMPLICIT_TILE_LIST, br) { br.tile_list_set_number = 0; }
	store(&t, RENDER_TARGET_0, V3D_OUTPUT_IMAGE_FORMAT_RGBA8, f->stride, f->target);
	cl_emit(&t, CLEAR_RENDER_TARGETS, c) { (void)c; }
	cl_emit(&t, END_OF_TILE_MARKER, e) { (void)e; }
	cl_emit(&t, RETURN_FROM_SUB_LIST, x) { (void)x; }
	uint32_t tl_end = tl_gpu + (uint32_t)(t.p - t.base);

	cl_emit(&r, TILE_RENDERING_MODE_CFG_COMMON, c) {
		c.number_of_render_targets = 1;
		c.image_width_pixels = f->width;
		c.image_height_pixels = f->height;
		c.early_z_disable = true;
		c.internal_depth_type = V3D_INTERNAL_TYPE_DEPTH_32F;
		c.log2_tile_width = lw;
		c.log2_tile_height = lh;
	}
	cl_emit(&r, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1, p) {
		p.render_target_number = 0;
		p.base_address = 0;
		p.stride = f->tile_w * 1 / 2; /* v3d_compute_rt_row_row_stride_128_bits, 32 bpp */
		p.internal_bpp = V3D_INTERNAL_BPP_32;
		p.internal_type_and_clamping = V3D_RENDER_TARGET_TYPE_CLAMP_8;
		p.clear_color_low_bits = f->clear;
	}
	cl_emit(&r, TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES, z) {
		z.stencil_clear_value = 0;
		z.z_clear_value = 0.0f;
	}
	cl_emit(&r, TILE_LIST_INITIAL_BLOCK_SIZE, i) {
		i.size_of_first_block_in_chained_tile_lists = TILE_ALLOCATION_BLOCK_SIZE_128B;
		i.use_auto_chained_tile_lists = true;
	}
	cl_emit(&r, MULTICORE_RENDERING_TILE_LIST_SET_BASE, s) {
		s.tile_list_set_number = 0;
		s.address = f->tile_alloc;
	}
	cl_emit(&r, MULTICORE_RENDERING_SUPERTILE_CFG, s) {
		s.supertile_width_in_tiles = f->supertile_w;
		s.supertile_height_in_tiles = f->supertile_h;
		s.total_frame_width_in_supertiles = f->frame_w_supertiles;
		s.total_frame_height_in_supertiles = f->frame_h_supertiles;
		s.total_frame_width_in_tiles = f->tiles_x;
		s.total_frame_height_in_tiles = f->tiles_y;
		s.number_of_bin_tile_lists = 1;
	}
	for (int i = 0; i < 2; i++) {
		cl_emit(&r, TILE_COORDINATES, c) { (void)c; }
		cl_emit(&r, END_OF_LOADS, e) { (void)e; }
		store(&r, NONE, 0, 0, 0);
		if (i == 0)
			cl_emit(&r, CLEAR_RENDER_TARGETS, c) { (void)c; }
		cl_emit(&r, END_OF_TILE_MARKER, e) { (void)e; }
	}
	cl_emit(&r, FLUSH_VCD_CACHE, c) { (void)c; }
	cl_emit(&r, START_ADDRESS_OF_GENERIC_TILE_LIST, g) {
		g.start = tl_gpu;
		g.end = tl_end;
	}
	uint32_t stw = f->tile_w * f->supertile_w, sth = f->tile_h * f->supertile_h;
	for (uint32_t y = 0; y <= (f->height - 1) / sth; y++)
		for (uint32_t x = 0; x <= (f->width - 1) / stw; x++)
			cl_emit(&r, SUPERTILE_COORDINATES, c) {
				c.column_number_in_supertiles = x;
				c.row_number_in_supertiles = y;
			}
	cl_emit(&r, END_OF_RENDERING, e) { (void)e; }

	len[0] = (uint32_t)(b.p - b.base);
	len[1] = (uint32_t)(r.p - r.base);
	len[2] = (uint32_t)(t.p - t.base);
}
EOF

cat >"$dir/clear71_ref.h" <<'EOF'
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include <stdint.h>
struct ref_frame {
	uint32_t width, height, clear, target, stride, tile_alloc;
	uint32_t tile_w, tile_h, tiles_x, tiles_y, supertile_w, supertile_h;
	uint32_t frame_w_supertiles, frame_h_supertiles;
};
void ref_build(const struct ref_frame *f, uint8_t *bcl, uint8_t *rcl, uint8_t *tl, uint32_t tl_gpu,
	       uint32_t len[3]);
EOF

cat >"$dir/clear71_check.c" <<'EOF'
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include <stdio.h>
#include <string.h>

#include "frame71.h"
#include "clear71_ref.h"

#define CAP 65536
static uint8_t ours[3][CAP], mesa[3][CAP];
static const char *const names[3] = { "binning", "render", "tile list" };

static int total, bad;

static void fail(const char *what)
{
	total++;
	if (++bad < 20)
		printf("FAIL: %s\n", what);
}

static void pass(void)
{
	total++;
}

/* Both builders on one frame; returns the lengths ours wrote. */
static void one(uint32_t w, uint32_t h, uint32_t clear, uint32_t target, uint32_t stride,
		uint32_t tile_alloc, uint32_t len_out[3])
{
	const uint32_t gpu[3] = { 0x00101000u, 0x00111000u, 0x00121000u };
	struct frame71 f = { w, h, clear, target, stride, tile_alloc };
	struct cl71 cl[3];
	struct v3d_tiling t;
	char what[160];

	memset(ours, 0xa5, sizeof ours);
	memset(mesa, 0xa5, sizeof mesa);
	for (int i = 0; i < 3; i++)
		cl71_init(&cl[i], ours[i], gpu[i], CAP);
	if (!frame71_build_clear(&f, &cl[0], &cl[1], &cl[2])) {
		snprintf(what, sizeof what, "%ux%u: frame71_build_clear refused a frame Mesa packs", w, h);
		fail(what);
		return;
	}
	frame71_tiling(&f, &t);
	struct ref_frame r = { w,         h,         clear,         target,        stride,
			       tile_alloc, t.tile_w,  t.tile_h,      t.tiles_x,     t.tiles_y,
			       t.supertile_w, t.supertile_h, t.frame_w_supertiles, t.frame_h_supertiles };
	uint32_t len[3];
	ref_build(&r, mesa[0], mesa[1], mesa[2], gpu[2], len);
	for (int i = 0; i < 3; i++) {
		len_out[i] = cl[i].len;
		if (cl[i].len != len[i] || memcmp(ours[i], mesa[i], len[i]) != 0) {
			uint32_t at = 0;
			while (at < len[i] && at < cl[i].len && ours[i][at] == mesa[i][at])
				at++;
			snprintf(what, sizeof what,
				 "%ux%u clear %08x: %s list, ours %u bytes, Mesa %u, first difference at byte %u",
				 w, h, clear, names[i], cl[i].len, len[i], at);
			fail(what);
		} else {
			pass();
		}
	}
}

static void refused(const char *why, uint32_t w, uint32_t h, uint32_t tile_alloc, uint32_t cap)
{
	struct frame71 f = { w, h, 0xff00ff00u, 0x00200000u, w * 4, tile_alloc };
	struct cl71 cl[3];
	for (int i = 0; i < 3; i++)
		cl71_init(&cl[i], ours[i], 0x00101000u + 0x10000u * (uint32_t)i, cap);
	if (frame71_build_clear(&f, &cl[0], &cl[1], &cl[2])) {
		char what[120];
		snprintf(what, sizeof what, "%s: not refused", why);
		fail(what);
	} else {
		pass();
	}
}

int main(void)
{
	static const uint32_t sz[] = { 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 255, 256, 257,
				       720, 1080, 1280, 1919, 1920, 1921, 4096 };
	static const uint32_t colours[] = { 0xff00ff00u, 0xffff0000u, 0x00000000u, 0xffffffffu, 0x80402010u };
	const int n = sizeof sz / sizeof sz[0];
	uint32_t len[3];

	for (int i = 0; i < n; i++)
		for (int j = 0; j < n; j++)
			one(sz[i], sz[j], colours[(i + j) % 5], 0x00200200u + 64u * (uint32_t)i,
			    sz[i] * 4 + 64u * (uint32_t)j, 0x00400000u + 64u * (uint32_t)j, len);

	/* the note's frame (Mesa's own dump): 64x64, 19 / 96 / 27 bytes */
	one(64, 64, 0xffff0000u, 0x00003000u, 256, 0x00009000u, len);
	if (len[0] == 19 && len[1] == 96 && len[2] == 27) {
		pass();
	} else {
		char what[100];
		snprintf(what, sizeof what, "64x64: %u / %u / %u bytes, the note says 19 / 96 / 27", len[0], len[1],
			 len[2]);
		fail(what);
	}
	/* rpi5_clear's own frame: 128x64, two tiles side by side */
	one(128, 64, 0xff00ff00u, 0x00100000u + 0x1000u + 512u, 512, 0x00110000u, len);

	refused("zero width", 0, 64, 0x00400000u, CAP);
	refused("zero height", 64, 0, 0x00400000u, CAP);
	refused("width 65536", 65536, 64, 0x00400000u, CAP);
	refused("tile allocation not 64-byte aligned", 64, 64, 0x00400020u, CAP);
	refused("lists of 20 bytes", 64, 64, 0x00400000u, 20);

	printf("clear71: %d of %d equal, %d failed\n", total - bad, total, bad);
	return bad != 0;
}
EOF

cc -O1 -std=gnu11 -Wall -Werror "${inc[@]}" -I"$dir" -c "$dir/clear71_mesa.c" -o "$dir/clear71_mesa.o" \
  >"$dir/build.log" 2>&1 &&
  cc -O1 -std=gnu11 -Wall -Wextra -Werror "${inc[@]}" -I"$dir" -I"$probe" -Idriver/src/v3d \
    -Idriver/src/v3d/generated "$dir/clear71_check.c" "$probe/frame71.c" driver/src/v3d/v3d_frame.c \
    "$dir/clear71_mesa.o" -lm -o "$dir/clear71_check" >>"$dir/build.log" 2>&1 || {
  tail -30 "$dir/build.log"
  exit 1
}
"$dir/clear71_check"
