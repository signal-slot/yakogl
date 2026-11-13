// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- the decode of case 244 (dlayout) and the outcome of case 243 (tsprobe) --
 * Included by gl_test.c (or by probe_decode_test.c on the host). Plain C99:
 * no GL calls, no allocation, only static functions over caller-provided
 * memory. The C twin of docs/notes/gc-ts-probe.md, "Part (ii)" (case 244) and
 * "Part (iii)" (case 243), and of the Rust that follows the same note
 * (src/vivante/layout.rs, ts.rs): the host test verifies what the board
 * decodes.
 *
 * Case 244. The draw is a 64x64 depth ramp z(x, y) = (x + 64 y) / 8192, the
 * window depth at the pixel centre ((x + 0.5) + 64 (y + 0.5)) / 8192 rounded to
 * 24 bits (pd_expect24). A D24S8 dword d holds z24 = d >> 8; it belongs to the
 * pixel whose expected value is nearest, within 1024 units (pd_decode).
 * Offsets are in dwords of the 16 KiB surface.
 *
 * Case 243. The readback struct (tsp_readback) is filled by the case from its
 * K0..K3 reads; tsp_classify gives the outcome code, tsp_pack_* the page words.
 */
#include <stdint.h>

#if defined(__GNUC__)
#define PD_FN static __attribute__((unused))
#else
#define PD_FN static
#endif

/* ---- address maps (src/vivante/layout.rs: offset_in, tile_index, super_index)
 * One pixel is one dword (D24S8 and A8R8G8B8, bpp = 4). TILE = 4 pixels along
 * a tile, SUPER_TILE = 64 pixels along a super-tile. */
#define PD_TILE 4u
#define PD_SUPER_TILE 64u

enum pd_map {
	PD_ROW_LINEAR = 0,        /* y * stride + x * 4, stride = align(64, 64) * 4 */
	PD_TILED4 = 1,            /* layout::offset(Tiled, ..) */
	PD_SUPER_DRM = 2,         /* layout::offset_in(SuperTiled, Drm, ..) */
	PD_SUPER_INTERLEAVED = 3, /* layout::offset_in(SuperTiled, Interleaved, ..) */
};

/* layout::tile_index: row-major in the 4x4 tile. */
PD_FN uint32_t pd_tile_index(uint32_t x, uint32_t y)
{
	return (y & 3u) * PD_TILE + (x & 3u);
}

/* layout::super_index: the pixel's index in its 64x64 super-tile. */
PD_FN uint32_t pd_super_index(int interleaved, uint32_t x, uint32_t y)
{
	uint32_t i;

	x &= 63u;
	y &= 63u;
	i = pd_tile_index(x, y);
	if (!interleaved) {
		/* Drm: x2 | y2 y3 | x3 x4 x5 | y4 y5 */
		return i | ((x & 0x04u) << 2) | ((y & 0x0cu) << 3) |
		       ((x & 0x38u) << 4) | ((y & 0x30u) << 6);
	}
	/* Interleaved: x2 y2 x3 y3 x4 y4 x5 y5 above the tile */
	return i | ((x & 0x04u) << 2) | ((y & 0x04u) << 3) | ((x & 0x08u) << 3) |
	       ((y & 0x08u) << 4) | ((x & 0x10u) << 4) | ((y & 0x10u) << 5) |
	       ((x & 0x20u) << 5) | ((y & 0x20u) << 6);
}

/* The dword offset of pixel (x, y); `stride` is the padded row in bytes
 * (align(64, 64) * 4 = 256 for the 64x64 case). The byte formulas of
 * layout::offset_in with bpp = 4, divided by 4:
 *   Tiled:      (y & ~3) * stride + bpp * ((x & ~3) << 2) + bpp * tile_index
 *   SuperTiled: (y & ~63) * stride + bpp * ((x & ~63) << 6) + bpp * super_index */
PD_FN uint32_t pd_map_offset(enum pd_map map, uint32_t x, uint32_t y, uint32_t stride)
{
	uint32_t sd = stride / 4u; /* stride in dwords */

	switch (map) {
	case PD_TILED4:
		return (y & ~3u) * sd + ((x & ~3u) << 2) + pd_tile_index(x, y);
	case PD_SUPER_DRM:
		return (y & ~63u) * sd + ((x & ~63u) << 6) + pd_super_index(0, x, y);
	case PD_SUPER_INTERLEAVED:
		return (y & ~63u) * sd + ((x & ~63u) << 6) + pd_super_index(1, x, y);
	case PD_ROW_LINEAR:
	default:
		return y * sd + x;
	}
}

/* ---- case 244: the ramp and its decode ---- */
#define PD_W 64u
#define PD_H 64u
#define PD_N (PD_W * PD_H)    /* 4096 pixels and dwords */
#define PD_STRIDE (PD_W * 4u) /* align(64, 64) * 4 */
#define PD_TOL 1024u          /* match radius in units of 2^-24 */
#define PD_NREF 12u
#define PD_NONE 0xffffu       /* a reference id or first dword that was not found */

/* The tags of the pages: page A of mode m is 0xc0 + m, page B 0xc4 + m,
 * page C 0xc8 + m. */
#define PD_TAG_A 0xc0u
#define PD_TAG_B 0xc4u
#define PD_TAG_C 0xc8u

/* The codes, the low nibble of page A's w1 (gc-ts-probe.md). */
#define PD_HANG 0xfu
#define PD_NO_WRITE 1u
#define PD_ROW_LINEAR_CODE 2u
#define PD_TILED4_CODE 3u
#define PD_SUPER_DRM_CODE 4u
#define PD_SUPER_INTERLEAVED_CODE 5u
#define PD_OTHER 6u
#define PD_PARTIAL 7u

/* The 12 reference pixels, in the page's order. */
static const uint8_t pd_ref_xy[PD_NREF][2] = {
	{1, 0}, {2, 0}, {4, 0}, {8, 0}, {0, 1}, {0, 4},
	{0, 8}, {16, 0}, {0, 16}, {32, 0}, {0, 32}, {63, 63},
};

/* The 24-bit window depth of pixel (x, y): ((x + .5) + 64 (y + .5)) / 8192 of
 * 2^24 - 1, rounded; i.e. (2 id + 65) * 0xffffff / 16384 with id = x + 64 y. */
PD_FN uint32_t pd_expect24(uint32_t x, uint32_t y)
{
	uint64_t id = x + 64u * y;

	return (uint32_t)(((2u * id + 65u) * 0xffffffu + 8192u) >> 14);
}

/* Scratch of pd_decode, 12 KiB, caller-provided (a static in the case). */
struct pd_work {
	uint16_t off[PD_N]; /* the first dword offset an id was found at */
	uint8_t cnt[PD_N];  /* how many dwords matched the id (saturating) */
};

struct pd_result {
	uint32_t code;          /* PD_* above */
	uint32_t matched;       /* ids found exactly once, of 4096 */
	uint32_t offs[PD_NREF]; /* dword offsets of the 12 reference pixels, PD_NONE if not found */
	uint32_t crc;           /* CRC-32 of the 16 KiB surface */
	uint32_t first_idx;     /* index of the first dword that is not the clear value, PD_NONE if none */
	uint32_t first_z;       /* its z byte: the low byte of z24, (d >> 8) & 0xff */
};

/* CRC-32 (IEEE, reflected, 0xedb88320) of `n` dwords, little-endian bytes. */
PD_FN uint32_t pd_crc32(const uint32_t *p, uint32_t n)
{
	uint32_t crc = 0xffffffffu, i, b, k;

	for (i = 0; i < n; i++)
		for (b = 0; b < 4; b++) {
			crc ^= (p[i] >> (8u * b)) & 0xffu;
			for (k = 0; k < 8; k++)
				crc = (crc >> 1) ^ (0xedb88320u & (0u - (crc & 1u)));
		}
	return ~crc;
}

/* Decode a 64x64 D24S8 surface (4096 dwords) drawn with the ramp over a
 * `clear` dword. The first of ROW_LINEAR, TILED4, SUPER_DRM, SUPER_INTERLEAVED
 * under which every id sits at its map's offset gives the code; OTHER when
 * each id is found once but at none of them; PARTIAL when an id is missing or
 * found twice; NO_WRITE when every dword is `clear`. (HANG is the case's
 * own, from its fence timeouts: pd_pack_a.) */
PD_FN void pd_decode(const uint32_t *surf, uint32_t clear, struct pd_work *w,
		     struct pd_result *r)
{
	uint32_t i, id, m, c, lo, hi, best, bestd, d, z;
	uint32_t written = 0;

	for (i = 0; i < PD_N; i++) {
		w->off[i] = 0;
		w->cnt[i] = 0;
	}
	r->matched = 0;
	r->first_idx = PD_NONE;
	r->first_z = 0;
	r->crc = pd_crc32(surf, PD_N);

	for (i = 0; i < PD_N; i++) {
		if (surf[i] == clear)
			continue;
		if (!written) {
			written = 1;
			r->first_idx = i;
			r->first_z = (surf[i] >> 8) & 0xffu;
		}
		z = surf[i] >> 8;
		/* expect24(id) is about 2048 * (id + 32.5): search around it */
		c = (z + 1024u) / 2048u;
		lo = c > 35u ? c - 35u : 0u;
		hi = c + 30u < PD_N - 1u ? c + 30u : PD_N - 1u;
		best = PD_N;
		bestd = PD_TOL + 1u;
		for (id = lo; id <= hi; id++) {
			uint32_t e = pd_expect24(id & 63u, id >> 6);

			d = z > e ? z - e : e - z;
			if (d < bestd) {
				bestd = d;
				best = id;
			}
		}
		if (best == PD_N)
			continue;
		if (w->cnt[best] == 0)
			w->off[best] = (uint16_t)i;
		if (w->cnt[best] < 255)
			w->cnt[best]++;
	}

	for (id = 0; id < PD_N; id++)
		if (w->cnt[id] == 1)
			r->matched++;
	for (i = 0; i < PD_NREF; i++) {
		id = pd_ref_xy[i][0] + 64u * pd_ref_xy[i][1];
		r->offs[i] = w->cnt[id] ? w->off[id] : PD_NONE;
	}

	if (!written) {
		r->code = PD_NO_WRITE;
		return;
	}
	if (r->matched < PD_N) {
		r->code = PD_PARTIAL;
		return;
	}
	for (m = 0; m < 4; m++) {
		for (id = 0; id < PD_N; id++)
			if (w->off[id] != pd_map_offset((enum pd_map)m, id & 63u, id >> 6, PD_STRIDE))
				break;
		if (id == PD_N) {
			r->code = PD_ROW_LINEAR_CODE + m;
			return;
		}
	}
	r->code = PD_OTHER;
}

/* Two 16-bit values in a word, `lo` in the low half. */
PD_FN uint32_t pd_pack16(uint32_t lo, uint32_t hi)
{
	return (lo & 0xffffu) | ((hi & 0xffffu) << 16);
}

/* The 12 reference offsets as 6 words (offs[2k] low, offs[2k + 1] high), in
 * the page order. gc-ts-probe.md spreads them over three pages a mode, as o[0]
 * = A w2, o[1] = A w3, o[2] = B w2, o[3..5] = C w1..w3 (page A and B spend
 * one word on the code and on the CRC, so they have 3 free words between
 * them; page C has 3). pd_pack_a, pd_pack_b and pd_pack_c carry them. */
PD_FN void pd_pack_offsets(const struct pd_result *r, uint32_t w[6])
{
	uint32_t k;

	for (k = 0; k < 6; k++)
		w[k] = pd_pack16(r->offs[2 * k], r->offs[2 * k + 1]);
}

/* Page A of mode `mode` (0..3), tag PD_TAG_A + mode: w1 = code | mode << 4 |
 * fence timeouts << 8 | matched << 16 (any timeout makes the code HANG),
 * w2 and w3 the first two offset words. */
PD_FN void pd_pack_a(const struct pd_result *r, uint32_t mode, uint32_t timeouts,
		     uint32_t w[3])
{
	uint32_t o[6];

	pd_pack_offsets(r, o);
	w[0] = ((timeouts ? PD_HANG : r->code) & 0xfu) | ((mode & 0xfu) << 4) |
	       ((timeouts > 255u ? 255u : timeouts) << 8) | (r->matched << 16);
	w[1] = o[0];
	w[2] = o[1];
}

/* Page B of mode `mode`, tag PD_TAG_B + mode: w1 = CRC-32 of the surface,
 * w2 = the third offset word (o[2]: (0,1) and (0,4)), w3 = first non-clear dword index << 16 | its z
 * byte (index 0xffff and byte 0 when the surface is all clear). */
PD_FN void pd_pack_b(const struct pd_result *r, uint32_t w[3])
{
	uint32_t o[6];

	pd_pack_offsets(r, o);
	w[0] = r->crc;
	w[1] = o[2];
	w[2] = pd_pack16(r->first_z, r->first_idx);
}

/* Page C of mode `mode`, tag PD_TAG_C + mode: w1..w3 = o[3..5], the offsets
 * of (0,8) (16,0) | (0,16) (32,0) | (0,32) (63,63). */
PD_FN void pd_pack_c(const struct pd_result *r, uint32_t w[3])
{
	uint32_t o[6];

	pd_pack_offsets(r, o);
	w[0] = o[3];
	w[1] = o[4];
	w[2] = o[5];
}

/* ---- case 243: the outcome of a tile-status probe arm ----
 * The case's reads of one arm, reduced to numbers (gc-ts-probe.md, "Expected
 * readback and the outcome code"). B(i) = 0xa5000000 | i is S's start
 * pattern (i = dword index in the 16 KiB surface, see tsp_detile_expect),
 * C the clear word 0xff4080bf, 4096 dwords in D. */
#define TSP_N 4096u
#define TSP_NOT_FOUND 0xffffu /* "no such index" in the first-index fields */
#define TSP_DEAD 0xdeadbeefu  /* D's start value */
#define TSP_C 0xff4080bfu     /* Mesa's clear word for (0.25, 0.5, 0.75, 1.0) */

#define TSP_TAG_P 0xd0u /* page 1 of arm a: 0xd0 + a */
#define TSP_TAG_K3 0xd4u /* K3 page of arm a: 0xd4 + a */
#define TSP_TAG_X 0xd8u /* detail page of arm a: 0xd8 + a */

/* The outcome codes, the low nibble of page w1. */
#define TSP_HANG 0xfu
#define TSP_GUARD 1u
#define TSP_NO_CONTROL 2u
#define TSP_FILL_MISS 3u
#define TSP_YES 4u
#define TSP_IGNORED 5u
#define TSP_NO_WRITE 6u
#define TSP_MIXED 7u

struct tsp_readback {
	uint32_t arm;           /* 0..3 */
	uint32_t steps_done;    /* K0..K3 steps that ran */
	uint32_t timeouts;      /* fence timeouts in any step */
	uint32_t guard_changed; /* guard dwords that changed */
	uint32_t s_changed;     /* nonzero: S changed in K1 or K2 */
	uint32_t k0_ok;         /* nonzero: K0's D == detile(B) */
	uint32_t t_first_bad;   /* first index of T[0..64] != 0x11111111 after K1, TSP_NOT_FOUND if none */
	uint32_t n_clear;       /* K2: count(D == C), of 4096 */
	uint32_t n_detile;      /* K2: count(D == detile(B)), of 4096 */
	uint32_t n_dead;        /* K2: count(D == 0xdeadbeef), of 4096 */
	uint32_t crc;           /* CRC-32 of K2's D (or the last D read) */
	uint32_t first_idx;     /* K2: first dword of D that is neither C nor detile(B), TSP_NOT_FOUND if none */
	uint32_t first_val;     /* the value read there */
	/* K3, present only when K2 was YES */
	uint32_t k3_present;
	uint32_t k3_map;        /* row-0 map: bit k = dword 8k of row 0 of D is C */
	uint32_t k3_nibble_order; /* bit 0: low nibble first; bit 1: Interleaved matched */
	uint32_t k3_crc;
	uint32_t k3_n_clear;
	uint32_t k3_n_detile;
};

/* The first of these that holds (HANG first). */
PD_FN uint32_t tsp_classify(const struct tsp_readback *r)
{
	if (r->timeouts)
		return TSP_HANG;
	if (r->guard_changed || r->s_changed)
		return TSP_GUARD;
	if (!r->k0_ok)
		return TSP_NO_CONTROL;
	if (r->t_first_bad != TSP_NOT_FOUND)
		return TSP_FILL_MISS;
	if (r->n_clear == TSP_N)
		return TSP_YES;
	if (r->n_detile == TSP_N)
		return TSP_IGNORED;
	if (r->n_dead == TSP_N)
		return TSP_NO_WRITE;
	return TSP_MIXED;
}

/* detile(B) of dword `lin` (y * 64 + x) of a 64x64 surface: S's start pattern
 * B(i) = 0xa5000000 | i at the dword the layout `map` puts the pixel. TILED
 * arms use PD_TILED4, arm 3 PD_SUPER_DRM or PD_SUPER_INTERLEAVED. */
PD_FN uint32_t tsp_detile_expect(enum pd_map map, uint32_t lin)
{
	return 0xa5000000u | pd_map_offset(map, lin & 63u, lin >> 6, PD_STRIDE);
}

/* K3's expected row-0 map: T is 0x01 bytes (low nibble 1 = cleared). With the
 * TS granule `granule_bytes` (128 or 256) the pixel (8k, 0) lies in granule
 * pd_map_offset * 4 / granule_bytes; nibble j of T covers granule j, the low
 * nibble first when `low_first` (granule parity 0 cleared), else the high
 * one (parity 1). Bit k of the result: that pixel is cleared. */
PD_FN uint32_t tsp_k3_expected_map(enum pd_map map, uint32_t granule_bytes, int low_first)
{
	uint32_t k, bits = 0;

	for (k = 0; k < 8; k++) {
		uint32_t g = pd_map_offset(map, 8u * k, 0, PD_STRIDE) * 4u / granule_bytes;

		if ((g & 1u) == (low_first ? 0u : 1u))
			bits |= 1u << k;
	}
	return bits;
}

/* The pages of arm `a` (the tag is TSP_TAG_P / TSP_TAG_K3 / TSP_TAG_X + arm).
 * Page 1: w1 = code | arm << 4 | steps done << 8 | timeouts << 16, w2 = CRC-32
 * of K2's D, w3 = count(D == C) << 16 | count(D == detile(B)). */
PD_FN void tsp_pack_p(const struct tsp_readback *r, uint32_t w[3])
{
	w[0] = (tsp_classify(r) & 0xfu) | ((r->arm & 0xfu) << 4) |
	       ((r->steps_done & 0xffu) << 8) | ((r->timeouts & 0xffffu) << 16);
	w[1] = r->crc;
	w[2] = pd_pack16(r->n_detile, r->n_clear);
}

/* K3 page (written only when `k3_present`): w1 = row-0 map | nibble_order
 * << 8, w2 = CRC-32 of K3's D, w3 = count(D == C) << 16 | count(D == detile). */
PD_FN void tsp_pack_k3(const struct tsp_readback *r, uint32_t w[3])
{
	w[0] = (r->k3_map & 0xffu) | ((r->k3_nibble_order & 0xffu) << 8);
	w[1] = r->k3_crc;
	w[2] = pd_pack16(r->k3_n_detile, r->k3_n_clear);
}

/* Detail page: w1 = first dword index of K2's D that is neither C nor
 * detile(B) << 16 | low half of the value read (0xffffffff when none), w2 =
 * C, w3 = first bad T index << 16 | count of changed guard dwords (low 16). */
PD_FN void tsp_pack_x(const struct tsp_readback *r, uint32_t w[3])
{
	w[0] = r->first_idx == TSP_NOT_FOUND ? 0xffffffffu
					     : pd_pack16(r->first_val, r->first_idx);
	w[1] = TSP_C;
	w[2] = pd_pack16(r->guard_changed, r->t_first_bad);
}
