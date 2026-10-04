/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_frees_check.c - host check of src/osal/gc_frees.h, the GC7000 Zephyr
 * OSAL's rule for giving freed GPU memory back (zephyr_gc.c uses it; no host
 * build reaches that file).
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/gc/osal_frees_check.c \
 *      -o build/gc-check/osal_frees_check && build/gc-check/osal_frees_check
 *
 * The shapes:
 * 1. dEQP texture.size on the i.MX8MP: the previous case's textures are
 *    deleted with no frame open and every submitted kick completed. They go
 *    back at once. Before gc_frees.h they waited for a kick nothing would
 *    submit until the next draw, and 32 MiB stayed held.
 * 2. A free while a frame is recorded: held until that frame (the next kick
 *    submitted) has completed, and not before.
 * 3. A free with no frame open while the last kick is still running: held
 *    until that kick, not the one after.
 * 4. Before the backend has said anything, a frame is assumed open (the old
 *    rule), so an OSAL whose backend never reports is no worse than before.
 * 5. A full queue whose oldest kick is still being recorded: no wait can
 *    complete it, so the free is refused (-EBUSY) and counted, and nothing is
 *    given back early.
 * 6. A full queue whose oldest kick was submitted: the wait for it
 *    (pending_frees.h) gives the queue back, counted as drained, and the new
 *    block is held.
 */
#include <stdio.h>
#include <string.h>

#include "gc_frees.h"

/* the OSAL's wait, as pending_frees.h calls it: kick `fence` completes */
static int nwaits;

static int wait_ok(uint32_t fence, uint32_t *completed)
{
	nwaits++;
	*completed = fence;
	return 0;
}

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("osal_frees_check: FAIL %s\n", what);
	}
}

/* the pool: which blocks came back, and how many bytes */
static uint32_t nreleased;
static size_t released_bytes;
static uint32_t last_gpu;

static void release(const struct gc_frees_block *b)
{
	nreleased++;
	released_bytes += b->size;
	last_gpu = b->gpu;
}

static void reset(struct gc_frees *f)
{
	gc_frees_init(f);
	nreleased = 0;
	nwaits = 0;
	released_bytes = 0;
	last_gpu = 0;
}

static struct gc_frees_block blk(uint32_t gpu, size_t size)
{
	static char cpu;

	return (struct gc_frees_block){ .cpu = &cpu, .gpu = gpu, .phys = gpu, .size = size };
}

int main(void)
{
	static struct gc_frees f;
	const int planned = 28;

	/* 1. texture.size: nothing recorded, every kick done */
	reset(&f);
	f.frame_open = false;
	for (uint32_t i = 0; i < 4; i++) {
		struct gc_frees_block b = blk(0x1000 * (i + 1), 8u << 20);

		check(gc_frees_free(&f, &b, 5, 5, wait_ok, release) == 0, "1: free accepted");
	}
	check(nreleased == 4 && released_bytes == 32u << 20, "1: all 32 MiB back at once");
	check(f.q.n == 0 && f.held_bytes == 0, "1: nothing held");
	check(f.freed_at_once == 4 && f.drained == 0, "1: counted as freed at once");

	/* 2. a frame open: held for the next kick */
	reset(&f);
	f.frame_open = true;
	{
		struct gc_frees_block b = blk(0x2000, 4096);

		check(gc_frees_free(&f, &b, 5, 5, wait_ok, release) == 0, "2: free accepted");
	}
	check(nreleased == 0 && f.q.n == 1, "2: held while the frame is recorded");
	gc_frees_drain(&f, 5, release);
	check(nreleased == 0, "2: not back at the kick before the frame");
	f.frame_open = false; /* the backend submitted it: kick 6 */
	gc_frees_drain(&f, 5, release);
	check(nreleased == 0, "2: not back while kick 6 runs");
	gc_frees_drain(&f, 6, release);
	check(nreleased == 1 && last_gpu == 0x2000 && f.q.n == 0, "2: back once kick 6 completed");
	check(f.drained == 1 && f.freed_at_once == 0, "2: counted as drained");
	check(f.held_peak_bytes == 4096 && f.held_bytes == 0, "2: high-water kept, held now 0");

	/* 3. no frame open, the last kick still running */
	reset(&f);
	f.frame_open = false;
	{
		struct gc_frees_block b = blk(0x3000, 4096);

		check(gc_frees_free(&f, &b, 7, 6, wait_ok, release) == 0, "3: free accepted");
	}
	check(nreleased == 0 && f.q.e[0].fence == 7, "3: held for the running kick 7, not 8");
	gc_frees_drain(&f, 7, release);
	check(nreleased == 1 && f.q.n == 0, "3: back once kick 7 completed");

	/* 4. nothing reported yet: the old rule */
	reset(&f);
	check(f.frame_open, "4: a frame is assumed open until the backend says");
	{
		struct gc_frees_block b = blk(0x4000, 4096);

		check(gc_frees_free(&f, &b, 5, 5, wait_ok, release) == 0, "4: free accepted");
	}
	check(nreleased == 0 && f.q.e[0].fence == 6, "4: held for the next kick, as before");

	/* 5. a full list leaks, never frees early */
	reset(&f);
	for (uint32_t i = 0; i < GC_FREES_MAX; i++) {
		struct gc_frees_block b = blk(0x10000 + i, 4096);

		gc_frees_free(&f, &b, 5, 5, wait_ok, release);
	}
	check(f.q.n == GC_FREES_MAX && nreleased == 0, "5: the list is full, nothing back");
	{
		struct gc_frees_block b = blk(0x20000, 8192);

		check(gc_frees_free(&f, &b, 5, 5, wait_ok, release) == -EBUSY && nwaits == 0,
		      "5: the next free is refused: its oldest kick is not submitted");
	}
	check(f.q.refused == 1 && f.q.refused_unsubmitted == 1 && f.q.refused_kib == 8 && nreleased == 0,
	      "5: refused and counted, not given back");
	check(f.held_peak_bytes == (size_t)GC_FREES_MAX * 4096, "5: high-water is the full list");
	gc_frees_drain(&f, 6, release);
	check(nreleased == GC_FREES_MAX && f.held_bytes == 0, "5: all back once kick 6 completed");

	/* 6. a full queue, its oldest kick submitted: wait, drain, hold */
	reset(&f);
	for (uint32_t i = 0; i < GC_FREES_MAX; i++) {
		struct gc_frees_block b = blk(0x10000 + i, 4096);

		gc_frees_free(&f, &b, 5, 5, wait_ok, release); /* held for kick 6 */
	}
	{
		struct gc_frees_block b = blk(0x30000, 4096);

		/* kick 6 submitted and running, the next frame open: fence 7 */
		check(gc_frees_free(&f, &b, 6, 5, wait_ok, release) == 0 && nwaits == 1,
		      "6: the free waits for kick 6 and is held");
	}
	check(nreleased == GC_FREES_MAX && f.drained == GC_FREES_MAX, "6: kick 6's blocks back, counted as drained");
	check(f.q.n == 1 && f.q.e[0].fence == 7 && f.held_bytes == 4096 && f.q.waits == 1 && f.q.refused == 0,
	      "6: the new block held for kick 7, one wait counted");

	printf("osal_frees_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return failures || checks != planned;
}
