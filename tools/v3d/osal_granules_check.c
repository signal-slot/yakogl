/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_granules_check.c - host check of src/osal/v3d_granules.h, the V3D
 * OSAL's carve-out allocator (zephyr_v3d.c uses it; no host build reaches
 * that file).
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/v3d/osal_granules_check.c \
 *      -o build/v3d-check/osal_granules_check && build/v3d-check/osal_granules_check
 *
 * The shapes:
 * 1. Alignment: a block starts on a multiple of `align` at or past `first`,
 *    also when `first` is not a multiple of it.
 * 2. Alloc, free, coalesce: a freed block is found again, two freed
 *    neighbours serve one block of both sizes, a hole too small is passed.
 * 3. Exhaustion: a full map, a block larger than the range and a block that
 *    would cross the end are refused (-1), never wrapped to the front.
 * 4. The used count and its high-water mark: marking twice counts once.
 * 5. The counter: every call counted, refusals included; its reads and the
 *    most in one call. A used front of 3,200 granules costs at most one read
 *    a word (<= 102 reads), and once the hint has moved past it, <= 2; a
 *    free below the hint is found again. The granule-at-a-time search read
 *    3,201 here on every call (DRAWPROF-HOOK: 7,666-10,775 on the board).
 * 6. A random alloc/free sequence: every find() equals a first-fit model
 *    that tests one granule at a time (so placement, and with it the
 *    fragmentation, is what the OSAL had), every block is aligned, inside
 *    [first, n) and overlaps no live block, and the used count matches.
 * With small blocks from the top (v3d_granules_top_below, opt-in):
 * 7. Placement: a small block is the highest aligned free one, a large one
 *    first fit; `first` not a multiple of the alignment; a full map, a
 *    block larger than the range and a range with no aligned room refuse
 *    (-1); the top hint costs one read a word and at most 2 once past, and
 *    a free above it is found again.
 * 8. The random sequence of 6 with top_below 16: every place() equals a
 *    granule-at-a-time last fit (small) or first fit (large), refuses
 *    exactly when first fit would, and keeps the invariants of 6.
 * 9. The STRESS1D replay (stress1d_replay.h): with the knob off it
 *    reproduces the record's per-step shape (RP_FIDELITY) and its numbers
 *    are pinned; with blocks below 1 MiB from the top its free runs and
 *    longest run are pinned, the bytes used are the same at every step, no
 *    allocation is refused, and the find cost (a call and the most in one)
 *    is not above first fit's.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "v3d_granules.h"
#include "stress1d_replay.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("osal_granules_check: FAIL %s\n", what);
	}
}

#define N 4096
static uint32_t map[N / 32];

/* The model: first fit, one granule at a time, as zephyr_v3d.c's find()
 * was before v3d_granules.h. */
static int64_t model_find(const struct v3d_granules *m, uint32_t need, uint32_t align)
{
	for (uint64_t g = (m->first + align - 1) / align * align; g + need <= m->n; g += align) {
		uint32_t i = 0;
		while (i < need && !v3d_granule_used(m, (uint32_t)g + i))
			i++;
		if (i == need)
			return (int64_t)g;
	}
	return -1;
}

/* The model for small blocks from the top: last fit, one granule at a
 * time. */
static int64_t model_find_top(const struct v3d_granules *m, uint32_t need, uint32_t align)
{
	if (need > m->n)
		return -1;
	for (int64_t g = (int64_t)(m->n - need) / align * align; g >= (int64_t)m->first; g -= align) {
		uint32_t i = 0;
		while (i < need && !v3d_granule_used(m, (uint32_t)g + i))
			i++;
		if (i == need)
			return g;
	}
	return -1;
}

static int64_t alloc(struct v3d_granules *m, uint32_t need, uint32_t align)
{
	int64_t g = v3d_granules_place(m, need, align);
	if (g >= 0)
		v3d_granules_mark(m, (uint32_t)g, need, true);
	return g;
}

/* 6: the live blocks, and which one owns each granule (0 = none) */
#define LIVE 512
static struct {
	uint32_t g, n;
} live[LIVE];
static unsigned nlive;
static uint16_t owner[N];

static uint32_t rnd_state = 12345;
static uint32_t rnd(void)
{
	rnd_state = rnd_state * 1103515245u + 12345u;
	return rnd_state >> 8;
}

static void random_sequence(uint32_t top_below)
{
	struct v3d_granules m;
	const uint32_t first = 37;
	v3d_granules_init(&m, map, first, N);
	v3d_granules_top_below(&m, top_below);
	unsigned refusal_differs = 0;
	memset(owner, 0, sizeof(owner));
	nlive = 0;
	unsigned mismatch = 0, outside = 0, misaligned = 0, overlap = 0, miscount = 0, refused = 0, placed = 0;
	uint32_t shadow_used = 0;
	for (int step = 0; step < 200000; step++) {
		if (nlive < LIVE && (nlive == 0 || rnd() % 100 < 55)) {
			/* sizes as the OSAL sees them: many small, some page-sized
			 * and larger (align 16 granules = a 4 KiB page) */
			uint32_t r = rnd() % 100;
			uint32_t need = r < 70 ? 1 + rnd() % 15 : r < 95 ? 16 + rnd() % 200 : 256 + rnd() % 800;
			uint32_t align = need >= 16 ? 16 : 1;
			int64_t first_fit = model_find(&m, need, align);
			int64_t want = need < top_below ? model_find_top(&m, need, align) : first_fit;
			int64_t g = alloc(&m, need, align);
			if (g != want)
				mismatch++;
			if ((g < 0) != (first_fit < 0))
				refusal_differs++;
			if (g < 0) {
				refused++;
				continue;
			}
			placed++;
			if ((uint32_t)g < first || (uint64_t)g + need > N)
				outside++;
			if (g % align)
				misaligned++;
			for (uint32_t i = 0; i < need; i++) {
				if (owner[g + i])
					overlap++;
				owner[g + i] = (uint16_t)(nlive + 1);
			}
			live[nlive].g = (uint32_t)g;
			live[nlive].n = need;
			nlive++;
			shadow_used += need;
		} else {
			unsigned k = rnd() % nlive;
			v3d_granules_mark(&m, live[k].g, live[k].n, false);
			for (uint32_t i = 0; i < live[k].n; i++)
				owner[live[k].g + i] = 0;
			shadow_used -= live[k].n;
			live[k] = live[--nlive];
			/* the moved block's owner tag */
			if (k < nlive)
				for (uint32_t i = 0; i < live[k].n; i++)
					owner[live[k].g + i] = (uint16_t)(k + 1);
		}
		if (m.used != shadow_used)
			miscount++;
	}
	if (mismatch || outside || misaligned || overlap || miscount || refusal_differs)
		printf("osal_granules_check: random (top_below %u): %u placed, %u refused; %u differ from the model, "
		       "%u outside, %u misaligned, %u overlapping granules, %u miscounts, %u refusals first fit "
		       "would not make\n",
		       top_below, placed, refused, mismatch, outside, misaligned, overlap, miscount, refusal_differs);
	if (!top_below) {
		check(placed > 10000 && refused > 100, "random: the sequence both places and runs out");
		check(mismatch == 0, "random: every find() is the first fit the model finds");
		check(outside == 0 && misaligned == 0, "random: every block aligned and inside [first, n)");
		check(overlap == 0, "random: no block overlaps a live one");
		check(miscount == 0, "random: the used count is the live granules'");
		return;
	}
	check(placed > 10000 && refused > 100, "random, small from the top: the sequence both places and runs out");
	check(mismatch == 0, "random, small from the top: every place() is the last fit (small) or first fit the model finds");
	check(refusal_differs == 0, "random, small from the top: -1 exactly when first fit refuses");
	check(outside == 0 && misaligned == 0, "random, small from the top: every block aligned and inside [first, n)");
	check(overlap == 0 && miscount == 0, "random, small from the top: no overlap, the used count is the live granules'");
}

/* 7: small blocks from the top */
static void top_placement(void)
{
	struct v3d_granules m;

	v3d_granules_init(&m, map, 5, N);
	v3d_granules_top_below(&m, 64);
	check(v3d_granules_place(&m, 1, 1) == N - 1, "top: a small block is the highest granule");
	check(v3d_granules_place(&m, 16, 16) == N - 16, "top: aligned, the highest multiple of 16 that fits");
	v3d_granules_mark(&m, N - 1, 1, true);
	check(v3d_granules_place(&m, 16, 16) == N - 32, "top: a used granule at n - 1 moves it down to n - 32");
	check(v3d_granules_place(&m, 15, 1) == N - 16, "top: 15 fit below the used granule");
	check(v3d_granules_place(&m, 64, 16) == 16, "top: a block of top_below granules is first fit");
	check(v3d_granules_place(&m, 0, 1) == v3d_granules_find(&m, 0, 1), "top: an empty block is first fit's");

	/* `first` not a multiple of the alignment, everything else used */
	v3d_granules_init(&m, map, 5, N);
	v3d_granules_top_below(&m, 64);
	v3d_granules_mark(&m, 21, N - 21, true);
	check(v3d_granules_place(&m, 16, 16) == -1, "top: no multiple of 16 with 16 free granules in [5, 21)");
	check(v3d_granules_place(&m, 11, 1) == 10, "top: 11 at the top of [5, 21)");
	check(v3d_granules_place(&m, 16, 1) == 5, "top: 16 from `first` exactly");
	check(v3d_granules_place(&m, 17, 1) == -1, "top: 17 do not fit in 16 free");

	/* exhaustion */
	v3d_granules_init(&m, map, 7, N);
	v3d_granules_top_below(&m, 64);
	v3d_granules_mark(&m, 7, N - 7, true);
	check(v3d_granules_place(&m, 1, 1) == -1, "top: a full map refuses");
	v3d_granules_init(&m, map, 7, N);
	v3d_granules_top_below(&m, N + 1);
	check(v3d_granules_place(&m, N, 1) == -1 && v3d_granules_place(&m, N - 7, 1) == 7,
	      "top: a block larger than the range refuses, the whole range fits");

	/* the top hint and the counter */
	v3d_granules_init(&m, map, 0, N);
	v3d_granules_top_below(&m, 64);
	v3d_granules_mark(&m, N - 3200, 3200, true);
	uint64_t r0 = m.reads;
	check(v3d_granules_place(&m, 1, 1) == N - 3201 && m.reads - r0 <= 3200 / 32 + 2,
	      "top: a used top of 3,200 granules: one read a word");
	v3d_granules_mark(&m, N - 3201, 1, true);
	r0 = m.reads;
	check(v3d_granules_place(&m, 1, 1) == N - 3202 && m.reads - r0 <= 2, "top: past the hint, at most 2 reads");
	v3d_granules_mark(&m, N - 5, 1, false);
	check(v3d_granules_place(&m, 1, 1) == N - 5, "top: a free above the hint is found again");
}

/* 9: the STRESS1D replay. RP_FIDELITY: the record's per-step lines (the
 * next step's start line; after step 14 the t=70s line) and what the
 * replay with first fit gives, steps 10-14. */
static const uint32_t record_runs[5] = { 26, 106, 307, 309, 310 };
static const uint32_t record_longest[5] = { 16744448, 12689408, 5199104, 4579328, 3694592 };
static const uint32_t record_used[3] = { 32988416, 36822016, 39590144 }; /* the t=50/55/60s lines */
#define RECORD_HIGH 44240896u
static const uint32_t replay_off_runs[5] = { 27, 106, 308, 309, 309 };
static const uint32_t replay_off_longest[5] = { 16748544, 12688384, 5185536, 4390912, 3862528 };
static const uint32_t replay_top_runs[5] = { 25, 107, 341, 343, 343 };
static const uint32_t replay_top_longest[5] = { 16748544, 12685312, 7249920, 6717440, 6189056 };
#define REPLAY_TOP_BELOW (1024u * 1024)

static void replay(void)
{
	static struct rp_result off, top;
	stress1d_replay(0, true, &off);
	stress1d_replay(REPLAY_TOP_BELOW, true, &top);

	/* the instrument: first fit reproduces the record's shape */
	unsigned shape = 0, pinned_off = 0, pinned_top = 0, same_bytes = 0;
	for (int i = 0; i < 5; i++) {
		const struct rp_step *a = &off.step[9 + i], *b = &top.step[9 + i];
		uint32_t dr = a->runs > record_runs[i] ? a->runs - record_runs[i] : record_runs[i] - a->runs;
		uint32_t dl = a->longest > record_longest[i] ? a->longest - record_longest[i]
							      : record_longest[i] - a->longest;
		shape += dr > 3 || dl > record_longest[i] / 16; /* runs within 3, the longest within 6.25% */
		pinned_off += a->runs != replay_off_runs[i] || a->longest != replay_off_longest[i];
		pinned_top += b->runs != replay_top_runs[i] || b->longest != replay_top_longest[i];
	}
	for (int i = 0; i < RP_STEPS; i++)
		same_bytes += off.step[i].used != top.step[i].used || off.step[i].high != top.step[i].high;
	unsigned used_exact = 0;
	for (int i = 0; i < 3; i++)
		used_exact += off.step[9 + i].used != record_used[i];
	check(off.refused_alloc == 0 && off.queue_full == 0 && top.refused_alloc == 0 && top.queue_full == 0,
	      "replay: nothing refused, the replay's free queue never full");
	check(used_exact == 0 && off.step[11].high == RECORD_HIGH,
	      "replay, first fit: bytes used at steps 10-12 and the high-water mark are the record's");
	check(shape == 0, "replay, first fit: free runs within 3 and the longest within 6.25% of the record, steps 10-14");
	check(pinned_off == 0, "replay, first fit: the pinned free runs and longest runs, steps 10-14");
	check(pinned_top == 0, "replay, small from the top: the pinned free runs and longest runs, steps 10-14");
	check(top.step[13].longest >= off.step[13].longest * 3 / 2,
	      "replay, small from the top: the longest run after step 14 at least 1.5 times first fit's");
	check(same_bytes == 0, "replay, small from the top: the same bytes used and high-water mark at every step");
	check(top.finds == off.finds && top.reads <= off.reads && top.reads_max <= off.reads_max,
	      "replay, small from the top: map reads a call and at most in one not above first fit's");
}

int main(void)
{
	struct v3d_granules m;

	/* 1. alignment, with `first` not a multiple of it */
	v3d_granules_init(&m, map, 5, N);
	check(v3d_granules_find(&m, 1, 1) == 5, "align 1: the first granule is `first`");
	check(v3d_granules_find(&m, 16, 16) == 16, "align 16: rounded up past `first`");
	v3d_granules_mark(&m, 16, 1, true);
	check(v3d_granules_find(&m, 16, 16) == 32, "align 16: a used granule at 16 moves it to 32");
	check(v3d_granules_find(&m, 11, 1) == 5, "align 1: the 11 granules below 16 fit");
	check(v3d_granules_find(&m, 12, 1) == 17, "align 1: 12 do not fit below 16");

	/* 2. alloc, free, coalesce */
	v3d_granules_init(&m, map, 0, N);
	int64_t a = alloc(&m, 10, 1), b = alloc(&m, 10, 1), c = alloc(&m, 10, 1);
	check(a == 0 && b == 10 && c == 20, "consecutive blocks");
	v3d_granules_mark(&m, (uint32_t)b, 10, false);
	check(v3d_granules_find(&m, 10, 1) == b, "a freed block is found again");
	check(v3d_granules_find(&m, 11, 1) == 30, "a hole too small is passed");
	v3d_granules_mark(&m, (uint32_t)a, 10, false);
	check(v3d_granules_find(&m, 20, 1) == 0, "two freed neighbours serve one block");

	/* 3. exhaustion: a refusal, not a wrap */
	v3d_granules_init(&m, map, 7, N);
	check(alloc(&m, N - 7, 1) == 7, "the whole range in one block");
	check(v3d_granules_find(&m, 1, 1) == -1, "a full map refuses");
	v3d_granules_mark(&m, N - 3, 3, false);
	check(v3d_granules_find(&m, 4, 1) == -1, "a block that would cross the end is refused");
	check(v3d_granules_find(&m, 3, 1) == N - 3, "the last 3 granules fit 3");
	check(v3d_granules_find(&m, 1, 16) == -1, "no multiple of 16 in the last 3 granules");
	v3d_granules_init(&m, map, 7, N);
	check(v3d_granules_find(&m, N, 1) == -1, "a block larger than the range is refused");

	/* 4. the used count and its high-water mark */
	v3d_granules_init(&m, map, 0, N);
	v3d_granules_mark(&m, 100, 50, true);
	v3d_granules_mark(&m, 120, 50, true);
	check(m.used == 70 && m.high == 70, "overlapping marks count each granule once");
	v3d_granules_mark(&m, 100, 70, false);
	check(m.used == 0 && m.high == 70, "freed: used 0, the high-water mark stays");

	/* 5. the counter */
	v3d_granules_init(&m, map, 0, N);
	(void)v3d_granules_find(&m, 1, 1);
	check(m.finds == 1 && m.reads >= 1 && m.reads_max == m.reads, "one call counted, with its reads");
	v3d_granules_mark(&m, 0, N, true);
	(void)v3d_granules_find(&m, 1, 1);
	check(m.finds == 2 && m.reads_max >= 1, "a refusal is counted too");
	v3d_granules_init(&m, map, 0, N);
	v3d_granules_mark(&m, 0, 3200, true);
	uint64_t r0 = m.reads;
	check(v3d_granules_find(&m, 1, 1) == 3200 && m.reads - r0 <= 3200 / 32 + 2,
	      "a used front of 3,200 granules: one read a word");
	v3d_granules_mark(&m, 3200, 1, true);
	r0 = m.reads;
	check(v3d_granules_find(&m, 1, 1) == 3201 && m.reads - r0 <= 2, "past the hint: at most 2 reads");
	v3d_granules_mark(&m, 5, 1, false);
	check(v3d_granules_find(&m, 1, 1) == 5, "a free below the hint is found again");

	/* 6. random */
	random_sequence(0);

	/* 7, 8: small blocks from the top */
	top_placement();
	random_sequence(16);

	/* 9. the STRESS1D replay */
	replay();

	const int planned = 27 + 15 + 5 + 8;
	printf("osal_granules_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
