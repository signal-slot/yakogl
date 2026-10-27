/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * linux_heap_check.c - host check of src/osal/linux_heap.h, the fake
 * device-VA allocator behind src/osal/linux.c's gpu_alloc/gpu_free.
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/osal/linux_heap_check.c \
 *      -o build/osal-check/linux_heap_check && build/osal-check/linux_heap_check
 *
 * What this does NOT cover: linux.c itself (the page rounding in
 * l_gpu_alloc/l_gpu_free, the per-heap-base learning in heap_index, the
 * l_shutdown report). No `cargo test` reaches linux.c either -- the Rust
 * host tests run against the mock OSAL in src/testutil.rs, never against the
 * C OSAL files. linux.c is exercised by tools/hostharness (run.sh links it
 * straight into the recording/differential programs described in
 * tools/hostharness/README.md) and by a `cmake -DYAKOGL_OSAL=linux` build;
 * neither runs from this repository's gate, so this header is the only part
 * of R-25's fix `tools/check.sh` verifies.
 *
 * The shapes:
 * 1. Exhaustion: the whole range in one block, then a refusal, not a wrap; a
 *    block larger than the range is refused; a block that would cross the
 *    end after partial use is refused, and a smaller one that fits is not.
 * 2. Reuse: a freed block is found again by first fit, exactly at its own
 *    offset, and a hole too small is passed to the tail (the bump); freeing
 *    part of what was just reused narrows the entry rather than doubling it.
 * 3. Coalescing: two freed neighbours (either free order) serve one block of
 *    their combined size, at the lower one's offset; three in a row collapse
 *    to a single entry.
 * 4. A free list that overflows: enough non-adjacent single-byte frees to
 *    pass LINUX_HEAP_MAX_FREE leaves the rest permanently leaked (`leaked`
 *    counts them, `used` still only the live bytes, `high` the peak before
 *    any of them freed) -- computed exactly, not just bounded, because the
 *    sequence is deterministic.
 * 5. A random alloc/free sequence: every linux_heap_alloc() equals a
 *    brute-force first-fit model over a byte-exact "used" shadow of the
 *    whole range (this allocator's free-list-then-bump split is exactly
 *    that model, address order preserved), no live block ever overlaps
 *    another, and the used/high counters match the shadow's.
 */
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "linux_heap.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("linux_heap_check: FAIL %s\n", what);
	}
}

/* 1. exhaustion: a refusal, not a wrap */
static void exhaustion(void)
{
	struct linux_heap h;

	linux_heap_init(&h, 100);
	check(linux_heap_alloc(&h, 100) == 0, "the whole range in one block");
	check(linux_heap_alloc(&h, 1) == -1, "a full heap refuses, even 1 byte");
	linux_heap_init(&h, 100);
	check(linux_heap_alloc(&h, 101) == -1, "a block larger than the range is refused");
	linux_heap_init(&h, 100);
	check(linux_heap_alloc(&h, 90) == 0, "90 of 100 bytes, from the bump");
	check(linux_heap_alloc(&h, 11) == -1, "11 more would cross the end: refused");
	check(linux_heap_alloc(&h, 10) == 90, "10 more exactly fills the range");
	check(linux_heap_alloc(&h, 1) == -1, "now full: refused");
}

/* 2. reuse: a freed block is found again, exactly */
static void reuse(void)
{
	struct linux_heap h;

	linux_heap_init(&h, 1000);
	int64_t a = linux_heap_alloc(&h, 10);
	int64_t b = linux_heap_alloc(&h, 10);
	int64_t c = linux_heap_alloc(&h, 10);
	check(a == 0 && b == 10 && c == 20, "three consecutive blocks from the bump");
	linux_heap_free(&h, (uint64_t)b, 10);
	check(linux_heap_alloc(&h, 10) == b, "a freed block is found again by first fit");
	check(h.bump == 30, "reuse never moves the bump");
	linux_heap_free(&h, (uint64_t)b, 10);
	check(linux_heap_alloc(&h, 11) == 30, "11 does not fit the 10-byte hole: taken from the bump");
	check(linux_heap_alloc(&h, 4) == b, "a smaller reuse takes from the front of the hole (still free from above)");
	check(h.nfree == 1 && h.free_[0].off == (uint64_t)b + 4 && h.free_[0].size == 6,
	      "the remaining 6 bytes of the hole stay a free entry, narrowed");
}

/* 3. coalescing: freed neighbours merge, in either free order */
static void coalescing(void)
{
	struct linux_heap h;

	linux_heap_init(&h, 1000);
	int64_t a = linux_heap_alloc(&h, 10), b = linux_heap_alloc(&h, 10);
	(void)linux_heap_alloc(&h, 10);
	linux_heap_free(&h, (uint64_t)a, 10);
	linux_heap_free(&h, (uint64_t)b, 10);
	check(h.nfree == 1 && h.free_[0].off == 0 && h.free_[0].size == 20,
	      "freeing b after a coalesces them (a, then its right neighbour)");
	check(linux_heap_alloc(&h, 20) == a, "the merged 20-byte block is found at a's offset");

	linux_heap_init(&h, 1000);
	a = linux_heap_alloc(&h, 10);
	b = linux_heap_alloc(&h, 10);
	(void)linux_heap_alloc(&h, 10);
	linux_heap_free(&h, (uint64_t)b, 10);
	linux_heap_free(&h, (uint64_t)a, 10);
	check(h.nfree == 1 && h.free_[0].off == 0 && h.free_[0].size == 20,
	      "freeing a after b coalesces them too (b, then its left neighbour)");

	linux_heap_init(&h, 1000);
	a = linux_heap_alloc(&h, 10);
	b = linux_heap_alloc(&h, 10);
	int64_t c = linux_heap_alloc(&h, 10);
	int64_t d = linux_heap_alloc(&h, 10);
	linux_heap_free(&h, (uint64_t)a, 10);
	linux_heap_free(&h, (uint64_t)c, 10);
	linux_heap_free(&h, (uint64_t)b, 10);
	check(h.nfree == 1 && h.free_[0].off == 0 && h.free_[0].size == 30,
	      "the middle free (b) joins both neighbours into one 30-byte entry");
	(void)d;
}

/* 4. the free list overflows: deterministic, so the leaked count is exact,
 * not merely bounded. Odd offsets stay live, so every even-offset free is
 * an isolated 1-byte fragment (no coalescing) -- the free list fills after
 * exactly LINUX_HEAP_MAX_FREE of them, and every one after that is leaked. */
static void free_list_overflow(void)
{
	struct linux_heap h;
	const unsigned live_and_freed = 600; /* 300 kept live (odd offsets), 300 offered to free() */
	const unsigned freed = live_and_freed / 2;
	const unsigned expect_leaked = freed > LINUX_HEAP_MAX_FREE ? freed - LINUX_HEAP_MAX_FREE : 0;
	unsigned bad_fill = 0, bad_reuse = 0;

	linux_heap_init(&h, 1000);
	for (unsigned i = 0; i < live_and_freed; i++)
		if (linux_heap_alloc(&h, 1) != (int64_t)i)
			bad_fill++;
	check(bad_fill == 0, "every byte comes from the bump in order");
	check(h.used == live_and_freed && h.high == live_and_freed, "every allocated byte is live and is the peak");
	/* every freed offset is even; its neighbours (odd, on either side) stay
	 * live, so none of these 300 frees can coalesce with another, or (the
	 * top one, 598) with the bump: exactly what fragments the free list. */
	for (unsigned off = 0; off < live_and_freed; off += 2)
		linux_heap_free(&h, off, 1);
	check(h.nfree == LINUX_HEAP_MAX_FREE, "the free list fills to its cap on isolated 1-byte frees");
	check(h.leaked == expect_leaked, "every fragment past the cap is leaked, not silently dropped uncounted");
	check(h.used == live_and_freed - freed, "used counts only the still-live (odd-offset) bytes");
	check(h.high == live_and_freed, "the high-water mark stays at the earlier peak");
	check(linux_heap_alloc(&h, 1) == 0, "first fit reuses the lowest stored free entry (offset 0)");
	for (unsigned i = 1; i < LINUX_HEAP_MAX_FREE; i++)
		if (linux_heap_alloc(&h, 1) != (int64_t)(2 * i))
			bad_reuse++;
	check(bad_reuse == 0, "reuse walks the stored entries in address order");
	check(h.nfree == 0, "every stored free entry has been reused");
	check(linux_heap_alloc(&h, 1) == (int64_t)live_and_freed,
	      "past the reused entries, allocation resumes from the bump -- never from a leaked byte");
}

/* 5. random: every alloc equals a brute-force first-fit model, no overlap */
#define RANGE 4096
static uint8_t model_used[RANGE];

static int64_t model_find(uint64_t need)
{
	uint64_t run = 0;

	if (need == 0 || need > RANGE)
		return -1;
	for (uint64_t i = 0; i < RANGE; i++) {
		if (!model_used[i]) {
			if (++run == need)
				return (int64_t)(i + 1 - need);
		} else {
			run = 0;
		}
	}
	return -1;
}

#define LIVE 400
static struct {
	uint64_t off, size;
} live[LIVE];
static unsigned nlive;

static uint32_t rnd_state = 987654321u;
static uint32_t rnd(void)
{
	rnd_state = rnd_state * 1103515245u + 12345u;
	return rnd_state >> 8;
}

static void random_sequence(void)
{
	struct linux_heap h;

	linux_heap_init(&h, RANGE);
	memset(model_used, 0, sizeof(model_used));
	nlive = 0;
	unsigned mismatch = 0, overlap = 0, placed = 0, refused = 0, miscount = 0;
	uint64_t shadow_used = 0, shadow_high = 0;

	for (int step = 0; step < 100000; step++) {
		if (nlive < LIVE && (nlive == 0 || rnd() % 100 < 55)) {
			uint64_t need = 1 + rnd() % 64;
			int64_t want = model_find(need);
			int64_t g = linux_heap_alloc(&h, need);
			if (g != want) {
				mismatch++;
				continue;
			}
			if (g < 0) {
				refused++;
				continue;
			}
			placed++;
			for (uint64_t i = 0; i < need; i++) {
				if (model_used[(uint64_t)g + i])
					overlap++;
				model_used[(uint64_t)g + i] = 1;
			}
			live[nlive].off = (uint64_t)g;
			live[nlive].size = need;
			nlive++;
			shadow_used += need;
		} else {
			unsigned k = rnd() % nlive;
			linux_heap_free(&h, live[k].off, live[k].size);
			for (uint64_t i = 0; i < live[k].size; i++)
				model_used[live[k].off + i] = 0;
			shadow_used -= live[k].size;
			live[k] = live[--nlive];
		}
		if (shadow_used > shadow_high)
			shadow_high = shadow_used;
		if (h.used != shadow_used)
			miscount++;
	}
	if (mismatch || overlap || miscount)
		printf("linux_heap_check: random: %u placed, %u refused; %u differ from the model, "
		       "%u overlapping bytes, %u miscounts\n",
		       placed, refused, mismatch, overlap, miscount);
	check(placed > 5000 && refused > 0, "random: the sequence both places and, sometimes, runs out");
	check(mismatch == 0, "random: every alloc is exactly the brute-force first-fit model's answer");
	check(overlap == 0, "random: no live block ever overlaps another");
	check(miscount == 0, "random: h.used tracks the live bytes exactly, every step");
	check(h.high == shadow_high, "random: h.high is the shadow's true peak");
}

int main(void)
{
	exhaustion();
	reuse();
	coalescing();
	free_list_overflow();
	random_sequence();

	const int planned = 32;
	printf("linux_heap_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
