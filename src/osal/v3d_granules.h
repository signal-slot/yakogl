/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_granules.h - the V3D OSAL's (zephyr_v3d.c) carve-out allocator: a map
 * of fixed-size granules, one bit each (set = used), and a first-fit search
 * over it. Plain C with no Zephyr in it, so the host checks it
 * (tools/v3d/osal_granules_check.c), as v3d_frees.h is checked.
 *
 * The map covers granules [0, n); the allocator hands out [first, n) (the
 * page table and the scratch page sit below `first`). A block starts on a
 * multiple of `align` granules. Nothing fits: -1, never a wrap.
 *
 * find() counts its map reads (one per 32-granule word read) and its calls,
 * so a run states what the search cost. It was one read per granule, from
 * `first` on every call: DRAWPROF-HOOK (2026-09-26) counted 7,666-10,775 a
 * call, gpu_alloc 66-94 us of the upload hook's 106-146 us. The search is
 * still first fit (the same block for the same map, so the same
 * fragmentation); it reads a word at a time and starts at `hint`, below
 * which every granule is used.
 *
 * Small blocks from the top (opt-in, v3d_granules_top_below; off = 0 =
 * first fit for every block, the placement above): a block of fewer
 * granules than `top_below` is placed by last fit, the highest aligned
 * block that is free, and larger ones by first fit from the bottom. The
 * small, short-lived blocks (tiled copies of glyph runs and uploads, vertex
 * buffers) then gather at the top and leave the middle of the carve-out in
 * one run for a large block. STRESS1D (2026-09-26) ended its glyph and
 * upload steps with 309 free runs and the longest 4.6 MB of 9.6 MB free:
 * a 5.5 MB hole that the evicted 1920x720 tiled copy left was filled by
 * glyph blocks from the bottom. The alignment, the capacity and the
 * refusal do not change: the last-fit search covers every aligned
 * position in [first, n) as first fit does, so it returns -1 exactly when
 * first fit would.
 */
#ifndef V3D_GRANULES_H
#define V3D_GRANULES_H

#include <stdbool.h>
#include <stdint.h>

struct v3d_granules {
	uint32_t *map;         /* n bits, set = used; n / 32 words */
	uint32_t first, n;     /* the allocator's granules: [first, n) */
	uint32_t used, high;   /* granules marked used now, and the most at once */
	uint32_t hint;         /* every granule in [first, hint) is used */
	uint32_t top_below;    /* blocks of fewer granules come from the top; 0 = off */
	uint32_t hint_top;     /* every granule in [hint_top, n) is used */
	uint32_t finds;        /* find() calls */
	uint64_t reads;        /* map reads find() made, all calls */
	uint32_t reads_max;    /* the most map reads one find() made */
};

/* An empty map over [first, n): `map` holds n / 32 words (n a multiple of
 * 32). The counters start at zero; every block by first fit (top_below 0). */
static inline void v3d_granules_init(struct v3d_granules *m, uint32_t *map, uint32_t first, uint32_t n)
{
	m->map = map;
	m->first = first;
	m->n = n;
	for (uint32_t w = 0; w < n / 32; w++)
		map[w] = 0;
	m->used = m->high = 0;
	m->hint = first;
	m->top_below = 0;
	m->hint_top = n;
	m->finds = 0;
	m->reads = 0;
	m->reads_max = 0;
}

static inline bool v3d_granule_used(const struct v3d_granules *m, uint32_t g)
{
	return m->map[g / 32] & (1u << (g % 32));
}

/* The bits of word w that fall in [g, end). */
static inline uint32_t v3d_granules_mask(uint32_t w, uint32_t g, uint32_t end)
{
	uint32_t lo = g > w * 32 ? g - w * 32 : 0;
	uint32_t hi = end < w * 32 + 32 ? end - w * 32 : 32;
	uint32_t m = hi == 32 ? ~0u : (1u << hi) - 1;
	return m & ~((1u << lo) - 1);
}

/* Mark [g, g + n) used (on) or free, a word at a time; `used` counts the
 * granules that changed, so marking twice counts once. A free below the
 * hint moves the hint down to it. */
static inline void v3d_granules_mark(struct v3d_granules *m, uint32_t g, uint32_t n, bool on)
{
	const uint32_t end = g + n;
	for (uint32_t w = g / 32; n && w <= (end - 1) / 32; w++) {
		uint32_t bits = v3d_granules_mask(w, g, end);
		if (on) {
			m->used += (uint32_t)__builtin_popcount(bits & ~m->map[w]);
			m->map[w] |= bits;
		} else {
			m->used -= (uint32_t)__builtin_popcount(bits & m->map[w]);
			m->map[w] &= ~bits;
		}
	}
	if (n && !on && g < m->hint)
		m->hint = g < m->first ? m->first : g;
	if (n && !on && end > m->hint_top)
		m->hint_top = end > m->n ? m->n : end;
	if (m->used > m->high)
		m->high = m->used;
}

/* The first granule in [g, end) whose bit is `used`, or end; a map read per
 * word. */
static inline uint32_t v3d_granules_next(const struct v3d_granules *m, uint32_t g, uint32_t end, bool used,
					 uint32_t *reads)
{
	while (g < end) {
		uint32_t w = g / 32;
		uint32_t bits = used ? m->map[w] : ~m->map[w];
		(*reads)++;
		bits &= ~0u << (g % 32);
		if (bits) {
			uint32_t f = w * 32 + (uint32_t)__builtin_ctz(bits);
			return f < end ? f : end;
		}
		g = w * 32 + 32;
	}
	return end;
}

static inline void v3d_granules_count(struct v3d_granules *m, uint32_t reads)
{
	m->finds++;
	m->reads += reads;
	if (reads > m->reads_max)
		m->reads_max = reads;
}

/* First fit: the lowest granule g >= first, a multiple of `align`, with
 * [g, g + need) free and g + need <= n; -1 when there is none. From the
 * hint: the first free granule (the hint moves up to it), rounded up to
 * `align`; the first used granule in [g, g + need) ends the candidate, and
 * the search goes on from the next free granule past it. A candidate
 * skipped this way starts on a used granule or covers one, so the block is
 * the one a granule-by-granule first fit returns. */
static inline int64_t v3d_granules_find(struct v3d_granules *m, uint32_t need, uint32_t align)
{
	uint32_t reads = 0;
	uint32_t f = v3d_granules_next(m, m->hint > m->first ? m->hint : m->first, m->n, false, &reads);
	m->hint = f;
	while (f < m->n) {
		uint32_t g = (f + align - 1) / align * align;
		if (need > m->n || g > m->n - need)
			break;
		uint32_t u = v3d_granules_next(m, g, g + need, true, &reads);
		if (u == g + need) {
			v3d_granules_count(m, reads);
			return g;
		}
		f = v3d_granules_next(m, u + 1, m->n, false, &reads);
	}
	v3d_granules_count(m, reads);
	return -1;
}

/* The end (index + 1) of the last granule in [lo, g) whose bit is `used`,
 * or lo when there is none; a map read per word, from g down. */
static inline uint32_t v3d_granules_prev(const struct v3d_granules *m, uint32_t lo, uint32_t g, bool used,
					 uint32_t *reads)
{
	while (g > lo) {
		uint32_t w = (g - 1) / 32;
		uint32_t bits = used ? m->map[w] : ~m->map[w];
		uint32_t top = (g - 1) % 32;
		(*reads)++;
		bits &= top == 31 ? ~0u : (1u << (top + 1)) - 1;
		if (bits) {
			uint32_t f = w * 32 + 31 - (uint32_t)__builtin_clz(bits);
			return f >= lo ? f + 1 : lo;
		}
		g = w * 32;
	}
	return lo;
}

/* Last fit: the highest granule g >= first, a multiple of `align`, with
 * [g, g + need) free and g + need <= n; -1 when there is none. From
 * hint_top: the end e of the last free granule below it (the hint moves
 * down to e), the highest aligned g with g + need <= e; the last used
 * granule in [g, g + need) ends the candidate, and the search goes on
 * below it. A candidate skipped this way covers a used granule, so the
 * block is the one a granule-by-granule last fit returns. */
static inline int64_t v3d_granules_find_top(struct v3d_granules *m, uint32_t need, uint32_t align)
{
	uint32_t reads = 0;
	uint32_t e = v3d_granules_prev(m, m->first, m->hint_top < m->n ? m->hint_top : m->n, false, &reads);
	m->hint_top = e;
	while (e > m->first && need <= e - m->first) {
		uint32_t g = (e - need) / align * align;
		if (g < m->first)
			break;
		uint32_t u = v3d_granules_prev(m, g, g + need, true, &reads);
		if (u == g) {
			v3d_granules_count(m, reads);
			return g;
		}
		/* u - 1 is used: the next candidate ends below it */
		e = v3d_granules_prev(m, m->first, u - 1, false, &reads);
	}
	v3d_granules_count(m, reads);
	return -1;
}

/* Blocks of fewer than `top_below` granules come from the top (last fit),
 * the others by first fit; 0 (the default) = first fit for all. */
static inline void v3d_granules_top_below(struct v3d_granules *m, uint32_t granules)
{
	m->top_below = granules;
}

/* The allocator's search: first fit, or last fit for a small block with
 * top_below set (the header's comment). */
static inline int64_t v3d_granules_place(struct v3d_granules *m, uint32_t need, uint32_t align)
{
	if (need && need < m->top_below)
		return v3d_granules_find_top(m, need, align);
	return v3d_granules_find(m, need, align);
}

#endif
