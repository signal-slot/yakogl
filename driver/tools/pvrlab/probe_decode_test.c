// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* Host selftest of gl_test_probe_decode.inc (case 244's depth-dump decode and
 * case 243's outcome classification). Build and run:
 *
 *   gcc -std=c99 -Wall -Werror -O2 driver/tools/pvrlab/probe_decode_test.c -o $TMPDIR/pdt && $TMPDIR/pdt
 *
 * Prints "probe_decode_test: N ok, 0 failed" and exits 0 when all pass.
 */
#include <stdio.h>
#include <string.h>

#include "gl_test_probe_decode.inc"

static int ok, failed;

#define CHECK(cond)                                                          \
	do {                                                                 \
		if (cond) {                                                  \
			ok++;                                                \
		} else {                                                     \
			failed++;                                            \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); \
		}                                                            \
	} while (0)

#define CLEAR 0xffffff00u /* depth 1.0, stencil 0 */

static uint32_t surf[PD_N];
static struct pd_work work;

/* Fill the surface with the ramp laid out by `map`; `jitter` moves each z24
 * by up to that many units (alternating sign). */
static void fill(enum pd_map map, uint32_t jitter)
{
	uint32_t x, y;

	for (x = 0; x < PD_N; x++)
		surf[x] = CLEAR;
	for (y = 0; y < PD_H; y++)
		for (x = 0; x < PD_W; x++) {
			uint32_t z = pd_expect24(x, y);
			uint32_t j = (x + y) % (jitter + 1);

			z = ((x + y) & 1) ? z + j : z - j;
			surf[pd_map_offset(map, x, y, PD_STRIDE)] = z << 8 | 0x07;
		}
}

static void check_offs(const struct pd_result *r, enum pd_map map)
{
	uint32_t i;

	for (i = 0; i < PD_NREF; i++)
		CHECK(r->offs[i] == pd_map_offset(map, pd_ref_xy[i][0], pd_ref_xy[i][1], PD_STRIDE));
}

static void test_case244(void)
{
	static const uint32_t want[4] = { PD_ROW_LINEAR_CODE, PD_TILED4_CODE,
					  PD_SUPER_DRM_CODE, PD_SUPER_INTERLEAVED_CODE };
	struct pd_result r;
	uint32_t m, w[3], b[3], c[3], i;

	/* the helper values */
	{
		/* CRC-32 of "12345678" */
		uint32_t s8[2] = { 0x34333231u, 0x38373635u };

		CHECK(pd_crc32(s8, 2) == 0x9ae0daafu);
	}
	CHECK(pd_expect24(0, 0) == 66560u); /* 32.5 / 8192 of 2^24 - 1 = 66559.996, rounded */
	CHECK(pd_map_offset(PD_TILED4, 4, 0, PD_STRIDE) == 16);
	CHECK(pd_map_offset(PD_TILED4, 0, 4, PD_STRIDE) == 64 * 4);
	CHECK(pd_map_offset(PD_SUPER_DRM, 4, 0, PD_STRIDE) == 16);
	CHECK(pd_map_offset(PD_SUPER_DRM, 8, 0, PD_STRIDE) == 128);
	CHECK(pd_map_offset(PD_SUPER_INTERLEAVED, 8, 0, PD_STRIDE) == 64);

	for (m = 0; m < 4; m++) {
		fill((enum pd_map)m, 0);
		pd_decode(surf, CLEAR, &work, &r);
		CHECK(r.code == want[m]);
		CHECK(r.matched == PD_N);
		check_offs(&r, (enum pd_map)m);
		CHECK(r.first_idx == 0); /* pixel (0,0) is dword 0 in every map */

		/* within 1024 units */
		fill((enum pd_map)m, 1000);
		pd_decode(surf, CLEAR, &work, &r);
		CHECK(r.code == want[m]);
		check_offs(&r, (enum pd_map)m);
	}

	/* the maps are four different bijections of 0..4095 */
	for (m = 1; m < 4; m++) {
		uint32_t x, y, diff = 0;

		for (y = 0; y < 64; y++)
			for (x = 0; x < 64; x++)
				diff += pd_map_offset((enum pd_map)m, x, y, PD_STRIDE) !=
					pd_map_offset(PD_ROW_LINEAR, x, y, PD_STRIDE);
		CHECK(diff > 0);
	}

	/* NO_WRITE: all clear */
	for (i = 0; i < PD_N; i++)
		surf[i] = CLEAR;
	pd_decode(surf, CLEAR, &work, &r);
	CHECK(r.code == PD_NO_WRITE);
	CHECK(r.matched == 0);
	CHECK(r.first_idx == PD_NONE);
	CHECK(r.offs[0] == PD_NONE);

	/* PARTIAL: some pixels never written, then one id twice */
	fill(PD_ROW_LINEAR, 0);
	for (i = 100; i < 110; i++)
		surf[i] = CLEAR;
	pd_decode(surf, CLEAR, &work, &r);
	CHECK(r.code == PD_PARTIAL);
	CHECK(r.matched == PD_N - 10);
	fill(PD_TILED4, 0);
	surf[5] = surf[0]; /* id 0 twice */
	pd_decode(surf, CLEAR, &work, &r);
	CHECK(r.code == PD_PARTIAL);
	CHECK(r.matched == PD_N - 2); /* id 0 twice, and the id formerly at 5 gone */
	/* a value matching no pixel is not counted */
	fill(PD_ROW_LINEAR, 0);
	surf[7] = 0xf0000000u; /* z24 0xf00000, above the ramp's top */
	pd_decode(surf, CLEAR, &work, &r);
	CHECK(r.code == PD_PARTIAL);
	CHECK(r.matched == PD_N - 1);

	/* OTHER: each id once, at none of the four maps (row-linear reversed) */
	fill(PD_ROW_LINEAR, 0);
	for (i = 0; i < PD_N / 2; i++) {
		uint32_t t = surf[i];

		surf[i] = surf[PD_N - 1 - i];
		surf[PD_N - 1 - i] = t;
	}
	pd_decode(surf, CLEAR, &work, &r);
	CHECK(r.code == PD_OTHER);
	CHECK(r.matched == PD_N);
	CHECK(r.offs[1] == PD_N - 1 - 2); /* the 12 offsets let the host derive the map */
	CHECK(r.offs[11] == PD_N - 1 - (63 + 64 * 63));

	/* page words, mode 2 SUPER_DRM */
	fill(PD_SUPER_DRM, 0);
	pd_decode(surf, CLEAR, &work, &r);
	pd_pack_a(&r, 2, 0, w);
	CHECK(w[0] == (PD_SUPER_DRM_CODE | 2u << 4 | PD_N << 16));
	CHECK((w[1] & 0xffff) == pd_map_offset(PD_SUPER_DRM, 1, 0, PD_STRIDE));
	CHECK((w[1] >> 16) == pd_map_offset(PD_SUPER_DRM, 2, 0, PD_STRIDE));
	CHECK((w[2] & 0xffff) == pd_map_offset(PD_SUPER_DRM, 4, 0, PD_STRIDE));
	CHECK((w[2] >> 16) == pd_map_offset(PD_SUPER_DRM, 8, 0, PD_STRIDE));
	pd_pack_b(&r, b);
	CHECK(b[0] == pd_crc32(surf, PD_N));
	CHECK((b[1] & 0xffff) == pd_map_offset(PD_SUPER_DRM, 0, 1, PD_STRIDE));
	CHECK((b[1] >> 16) == pd_map_offset(PD_SUPER_DRM, 0, 4, PD_STRIDE));
	CHECK((b[2] >> 16) == 0); /* first dword written is dword 0 */
	CHECK((b[2] & 0xffff) == (pd_expect24(0, 0) & 0xff));
	pd_pack_a(&r, 0, 3, w); /* timeouts: HANG */
	CHECK((w[0] & 0xf) == PD_HANG && (w[0] >> 8 & 0xff) == 3);
	{
		uint32_t o[6];

		pd_pack_offsets(&r, o);
		uint32_t k, h;

		CHECK((o[5] >> 16) == pd_map_offset(PD_SUPER_DRM, 63, 63, PD_STRIDE));
		CHECK(o[0] == w[1] && o[1] == w[2] && o[2] == b[1]);
		pd_pack_c(&r, c);
		CHECK(o[3] == c[0] && o[4] == c[1] && o[5] == c[2]);
		for (k = 0; k < PD_NREF; k++) {
			h = k & 1 ? o[k / 2] >> 16 : o[k / 2] & 0xffffu;
			CHECK(h == pd_map_offset(PD_SUPER_DRM, pd_ref_xy[k][0],
						 pd_ref_xy[k][1], PD_STRIDE));
		}
		CHECK(PD_TAG_C == PD_TAG_B + 4 && PD_TAG_B == PD_TAG_A + 4);
	}
	/* page B of an all-clear surface */
	for (i = 0; i < PD_N; i++)
		surf[i] = CLEAR;
	pd_decode(surf, CLEAR, &work, &r);
	pd_pack_b(&r, b);
	CHECK((b[2] >> 16) == 0xffff);
	pd_pack_c(&r, c);
	CHECK(c[0] == 0xffffffffu && c[1] == 0xffffffffu && c[2] == 0xffffffffu);
}

/* one synthetic readback per outcome code */
static struct tsp_readback base(void)
{
	struct tsp_readback r;

	memset(&r, 0, sizeof r);
	r.arm = 1;
	r.steps_done = 3;
	r.k0_ok = 1;
	r.t_first_bad = TSP_NOT_FOUND;
	r.first_idx = TSP_NOT_FOUND;
	return r;
}

static void test_case243(void)
{
	struct tsp_readback r;
	uint32_t w[3], x;

	r = base();
	r.timeouts = 1;
	r.guard_changed = 2; /* HANG wins over every line below */
	CHECK(tsp_classify(&r) == TSP_HANG);

	r = base();
	r.guard_changed = 1;
	r.k0_ok = 0; /* GUARD wins over NO_CONTROL */
	CHECK(tsp_classify(&r) == TSP_GUARD);
	r = base();
	r.s_changed = 1;
	CHECK(tsp_classify(&r) == TSP_GUARD);

	r = base();
	r.k0_ok = 0;
	r.t_first_bad = 0;
	CHECK(tsp_classify(&r) == TSP_NO_CONTROL);

	r = base();
	r.t_first_bad = 17;
	r.n_clear = TSP_N;
	CHECK(tsp_classify(&r) == TSP_FILL_MISS);

	r = base();
	r.n_clear = TSP_N;
	CHECK(tsp_classify(&r) == TSP_YES);

	r = base();
	r.n_detile = TSP_N;
	CHECK(tsp_classify(&r) == TSP_IGNORED);

	r = base();
	r.n_dead = TSP_N;
	CHECK(tsp_classify(&r) == TSP_NO_WRITE);

	r = base();
	r.n_clear = 2048;
	r.n_detile = 2048;
	CHECK(tsp_classify(&r) == TSP_MIXED);
	r = base();
	CHECK(tsp_classify(&r) == TSP_MIXED);

	/* page words */
	r = base();
	r.n_clear = TSP_N;
	r.crc = 0x12345678u;
	r.guard_changed = 0;
	r.t_first_bad = TSP_NOT_FOUND;
	tsp_pack_p(&r, w);
	CHECK(w[0] == (TSP_YES | 1u << 4 | 3u << 8));
	CHECK(w[1] == 0x12345678u);
	CHECK(w[2] == (TSP_N << 16));
	tsp_pack_x(&r, w);
	CHECK(w[0] == 0xffffffffu);
	CHECK(w[1] == TSP_C);
	CHECK(w[2] == 0xffff0000u);
	r = base();
	r.n_clear = 100;
	r.n_detile = 200;
	r.timeouts = 2;
	r.first_idx = 33;
	r.first_val = 0xa5000021u;
	r.t_first_bad = 5;
	r.guard_changed = 9;
	tsp_pack_p(&r, w);
	CHECK(w[0] == (TSP_HANG | 1u << 4 | 3u << 8 | 2u << 16));
	CHECK(w[2] == (100u << 16 | 200u));
	tsp_pack_x(&r, w);
	CHECK(w[0] == (33u << 16 | 0x0021u));
	CHECK(w[2] == (5u << 16 | 9u));
	r.k3_map = 0x55;
	r.k3_nibble_order = 1;
	r.k3_crc = 0xdeadc0deu;
	r.k3_n_clear = 2048;
	r.k3_n_detile = 2048;
	tsp_pack_k3(&r, w);
	CHECK(w[0] == (0x55u | 1u << 8));
	CHECK(w[1] == 0xdeadc0deu);
	CHECK(w[2] == (2048u << 16 | 2048u));

	/* detile(B) and K3's expected map (gc-ts-probe.md, "What K3 expects") */
	CHECK(tsp_detile_expect(PD_TILED4, 0) == 0xa5000000u);
	CHECK(tsp_detile_expect(PD_TILED4, 4) == 0xa5000010u);
	CHECK(tsp_detile_expect(PD_TILED4, 64) == 0xa5000004u);
	CHECK(tsp_k3_expected_map(PD_TILED4, 128, 1) == 0x55u);
	CHECK(tsp_k3_expected_map(PD_TILED4, 128, 0) == 0xaau);
	CHECK(tsp_k3_expected_map(PD_TILED4, 256, 1) == 0x33u);
	CHECK(tsp_k3_expected_map(PD_TILED4, 256, 0) == 0xccu);
	x = tsp_k3_expected_map(PD_SUPER_DRM, 128, 1);
	CHECK(x == 0xffu); /* each (8k, 0) starts a 128-dword group: always an even granule */
}

int main(void)
{
	test_case244();
	test_case243();
	printf("probe_decode_test: %d ok, %d failed\n", ok, failed);
	return failed ? 1 : 0;
}
