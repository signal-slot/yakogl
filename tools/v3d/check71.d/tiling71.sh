#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tiling71: driver/src/v3d/v3d_frame.c's tile size and tile counts at V3D 7.1
# (v3d_frame_tiling_gen, ver 71) against Mesa's own v3d_choose_tile_size
# (third_party/mesa-26.2.1/src/broadcom/common/v3d_util.c:122, the 7.x branch
# at :148-175) linked from the reference build, for 1/2/4/8 render targets of
# 32/64/128 bpp (and mixed sets), MSAA off and on, and sizes on and next to
# every tile boundary. The tile counts are ceil(size / tile) as in Mesa's
# v3d_job_set_tile_size. The same table is compared at 4.2
# (v3d_frame_tiling_bpp) so the 4.2 path is shown unchanged.
# Reference: tools/v3d/mesa_ref.sh (build/v3d-check/mesa), or MESA_V3D_TREE
# with build-v3d as the v3d rows of tools/check.sh. Convention:
# tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=tiling71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

tree=${MESA_V3D_TREE:-}
mt=third_party/mesa-26.2.1
ref=build/v3d-check/mesa
inc=(-D_GNU_SOURCE -DHAVE_ENDIAN_H -I/usr/include/libdrm)
if [ -z "$tree" ]; then
  tools/v3d/mesa_ref.sh >"$dir/mesa_ref.log" 2>&1 || {
    tail -20 "$dir/mesa_ref.log"
    exit 1
  }
  inc+=(-I"$mt/src" -I"$mt/include" -I"$mt/src/broadcom" -I"$ref/gen")
  libs=("$ref/libmesa_ref.a" -lexpat -lz -lm -lpthread)
elif [ -d "$tree/build-v3d" ]; then
  mb=$tree/build-v3d
  inc+=(-I"$tree/src" -I"$tree/include" -I"$tree/src/broadcom" -I"$mb/src/broadcom" -I"$mb/src")
  libs=("$mb/src/broadcom/libbroadcom_v3d.a" "$mb/src/broadcom/cle/libbroadcom_cle.a"
    "$mb/src/broadcom/libv3d_neon.a" "$mb/src/util/libmesa_util.a" -lexpat -lz -lzstd -lm -lpthread)
else
  echo "MESA_V3D_TREE='$tree' has no build-v3d (unset it for the vendored reference build)"
  echo "gap: MESA_V3D_TREE"
  exit 77
fi

cat >"$dir/tiling71_check.c" <<'EOF'
/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include "common/v3d_device_info.h"
#include "common/v3d_util.h"
#include "v3d_frame.h"

static uint32_t cdiv(uint32_t a, uint32_t b)
{
	return (a + b - 1) / b;
}

struct set {
	uint32_t n, tot, maxc; /* render targets, bytes per pixel of all, largest bpp class */
};

int main(void)
{
	struct v3d_device_info d42 = { .ver = 42, .has_accumulators = true };
	struct v3d_device_info d71 = { .ver = 71, .has_accumulators = false };
	/* bytes per pixel of one render target: 32, 64, 128 bpp */
	static const uint32_t bytes[3] = { 4, 8, 16 };
	static const uint32_t rtn[4] = { 1, 2, 4, 8 };
	/* sizes: every tile edge (8, 16, 32, 64 and their multiples), +-1 */
	static const uint32_t sz[] = { 1,   7,   8,   9,   15,  16,  17,   31,   32,   33,   63,
				       64,  65,  127, 128, 129, 191, 192,  193,  255,  256,  257,
				       512, 513, 640, 641, 1024, 1025, 1919, 1920, 1921, 4096 };
	const int nsz = sizeof sz / sizeof sz[0];
	int total = 0, bad = 0;
	struct set sets[32];
	int ns = 0;

	for (int r = 0; r < 4; r++)
		for (int b = 0; b < 3; b++)
			sets[ns++] = (struct set){ rtn[r], rtn[r] * bytes[b], b };
	/* mixed formats */
	sets[ns++] = (struct set){ 2, 4 + 16, 2 };
	sets[ns++] = (struct set){ 4, 4 + 4 + 8 + 16, 2 };
	sets[ns++] = (struct set){ 8, 4 * 4 + 2 * 8 + 2 * 16, 2 };
	sets[ns++] = (struct set){ 3, 4 + 4 + 8, 1 };

	for (int s = 0; s < ns; s++)
		for (int ms = 0; ms < 2; ms++)
			for (int ver = 0; ver < 2; ver++) {
				const struct set *e = &sets[s];
				/* 4.2: Mesa asserts past the 8x8 entry and steps 3 for more than
				 * 4 RTs where v3d_frame_tiling_bpp (unchanged) stops at 2: only
				 * the sets both agree on are compared */
				if (!ver && (e->n > 4 || (e->n > 2 ? 2 : e->n > 1) + 2 * ms + (int)e->maxc > 6))
					continue;
				uint32_t tw, th;
				v3d_choose_tile_size(ver ? &d71 : &d42, e->n, e->maxc, e->tot, ms,
						     false, &tw, &th);
				for (int i = 0; i < nsz; i++)
					for (int j = 0; j < nsz; j++) {
						struct v3d_tiling t;
						v3d_frame_tiling_gen(ver ? 71 : 42, sz[i], sz[j], e->n, ms,
								     e->maxc, e->tot, &t);
						int ok = t.tile_w == tw && t.tile_h == th &&
							 t.tiles_x == cdiv(sz[i], tw) &&
							 t.tiles_y == cdiv(sz[j], th);
						if (!ver) {
							struct v3d_tiling u;
							v3d_frame_tiling_bpp(sz[i], sz[j], e->n, ms, e->maxc, &u);
							ok = ok && !memcmp(&t, &u, sizeof t);
						}
						total++;
						if (!ok && ++bad < 20)
							printf("FAIL: v%d %u RTs %u B %s %ux%u: ours %ux%u, Mesa %ux%u\n",
							       ver ? 71 : 42, e->n, e->tot, ms ? "msaa" : "",
							       sz[i], sz[j], t.tile_w, t.tile_h, tw, th);
					}
			}
	printf("tiling71: %d of %d equal, %d failed\n", total - bad, total, bad);
	return bad != 0;
}
EOF
cc -O1 -std=gnu11 "${inc[@]}" -Idriver/src/v3d -Idriver/src/v3d/generated \
  "$dir/tiling71_check.c" driver/src/v3d/v3d_frame.c "${libs[@]}" -o "$dir/tiling71_check" \
  >"$dir/build.log" 2>&1 || {
  tail -20 "$dir/build.log"
  exit 1
}
"$dir/tiling71_check"
