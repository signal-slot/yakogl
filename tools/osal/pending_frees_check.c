/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pending_frees_check.c - host check of src/osal/pending_frees.h, the queue
 * of deferred GPU frees that zephyr.c, zephyr_gc.c
 * (gc_frees.h) and zephyr_v3d.c (v3d_frees.h) share.
 *
 *   cc -std=gnu11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/osal/pending_frees_check.c \
 *      -o build/osal-check/pending_frees_check && build/osal-check/pending_frees_check
 *
 * The shapes (a queue of 4):
 * 1. Drain order: a drain gives back every completed entry oldest first and
 *    keeps the others in their order.
 * 2. A full queue: pf_push waits for the lowest fence held (also when it is
 *    not the first entry), drains, and holds the new block; `waits` counts
 *    it and the high-water mark is the capacity.
 * 3. A failed wait: a timeout refuses with -ETIMEDOUT, an error with its own
 *    code; nothing is given back or dropped, and the refusal is counted with
 *    its KiB.
 * 4. The oldest kick not submitted yet: refused with -EBUSY without a wait.
 * 5. A wait that reports a fence below the one asked for: refused, not held
 *    past the capacity.
 * 6. A wait that drains the queue itself (zephyr.c's wait does): the push
 *    still holds the block once.
 * 7. No wait (the default build: PF_WAIT is NULL without
 *    YAKOGL_PENDING_FREES_WAIT=1): a full queue refuses at once with
 *    -ENOSPC, counted as such.
 * 8. The counters' line.
 */
#include <stdbool.h>
#include <stdio.h>
#include <string.h>

#define PF_PAYLOAD  unsigned int
#define PF_CAPACITY 4
#include "pending_frees.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("pending_frees_check: FAIL %s\n", what);
	}
}

/* what release() gave back, in order */
static unsigned int released[16], nreleased;

static void release(const unsigned int *p)
{
	if (nreleased < 16)
		released[nreleased] = *p;
	nreleased++;
}

/* the wait stub: what it was asked, what it answers */
static struct pf_queue q;
static uint32_t asked, answer_done;
static int answer_rc, nwaits;
static bool wait_drains;

static int wait_stub(uint32_t fence, uint32_t *completed)
{
	nwaits++;
	asked = fence;
	if (answer_rc == 0) {
		*completed = answer_done;
		if (wait_drains)
			pf_drain(&q, answer_done, release);
	}
	return answer_rc;
}

static void reset(void)
{
	memset(&q, 0, sizeof(q));
	memset(released, 0, sizeof(released));
	nreleased = 0;
	asked = 0;
	answer_done = 0;
	answer_rc = 0;
	nwaits = 0;
	wait_drains = false;
}

/* hold ids[i] with fences[i], `submitted` = 9: never a wait (the queue is not full) */
static void fill(const unsigned int *ids, const uint32_t *fences, unsigned int n)
{
	for (unsigned int i = 0; i < n; i++)
		pf_push(&q, &ids[i], 1, fences[i], 9, wait_stub, release);
}

int main(void)
{
	/* 1. drain order */
	reset();
	fill((const unsigned int[]){ 10, 11, 12, 13 }, (const uint32_t[]){ 3, 1, 4, 2 }, 4);
	check(pf_drain(&q, 2, release) == 2 && nreleased == 2 && released[0] == 11 && released[1] == 13,
	      "drain: the completed entries go back oldest first");
	check(q.n == 2 && q.e[0].p == 10 && q.e[1].p == 12, "drain: the others keep their order");

	/* 2. a full queue: wait for the lowest fence, drain, hold */
	reset();
	fill((const unsigned int[]){ 20, 21, 22, 23 }, (const uint32_t[]){ 5, 4, 5, 6 }, 4);
	answer_done = 4;
	{
		const unsigned int id = 24;
		int rc = pf_push(&q, &id, 8, 7, 4, wait_stub, release); /* oldest == submitted */

		check(rc == 0 && nwaits == 1 && asked == 4, "full: waits for the lowest fence held (not the first entry)");
		check(nreleased == 1 && released[0] == 21, "full: the wait's drain gives back what completed");
		check(q.n == 4 && q.e[3].p == 24 && q.e[3].fence == 7 && q.e[3].kib == 8, "full: the new block is held");
		check(q.waits == 1 && q.refused == 0 && q.high_water == 4, "full: one wait counted, high-water 4");
	}

	/* 3. a failed wait: timeout, then an error */
	reset();
	fill((const unsigned int[]){ 30, 31, 32, 33 }, (const uint32_t[]){ 2, 3, 4, 5 }, 4);
	answer_rc = 1;
	{
		const unsigned int id = 34;
		int rc = pf_push(&q, &id, 64, 6, 5, wait_stub, release);

		check(rc == -ETIMEDOUT && nwaits == 1 && asked == 2, "timeout: refused with -ETIMEDOUT after one wait");
		check(q.n == 4 && nreleased == 0 && q.e[0].p == 30, "timeout: nothing given back, nothing dropped");
		answer_rc = -EIO;
		rc = pf_push(&q, &id, 16, 6, 5, wait_stub, release);
		check(rc == -EIO, "error: refused with the wait's own code");
		check(q.waits == 2 && q.refused == 2 && q.refused_unsubmitted == 0 && q.refused_kib == 80,
		      "failed waits: counted, with the KiB kept");
	}

	/* 4. the oldest kick is still being recorded */
	reset();
	fill((const unsigned int[]){ 40, 41, 42, 43 }, (const uint32_t[]){ 10, 10, 10, 10 }, 4);
	{
		const unsigned int id = 44;
		int rc = pf_push(&q, &id, 4, 10, 9, wait_stub, release);

		check(rc == -EBUSY && nwaits == 0, "unsubmitted: refused with -EBUSY, no wait");
		check(q.refused == 1 && q.refused_unsubmitted == 1 && q.refused_kib == 4 && q.waits == 0,
		      "unsubmitted: counted as such");
	}

	/* 5. a wait that reports less than asked */
	reset();
	fill((const unsigned int[]){ 50, 51, 52, 53 }, (const uint32_t[]){ 3, 3, 4, 4 }, 4);
	answer_done = 2;
	{
		const unsigned int id = 54;
		int rc = pf_push(&q, &id, 1, 5, 4, wait_stub, release);

		check(rc == -ETIMEDOUT && q.n == 4 && q.refused == 1, "short wait: refused, never past the capacity");
	}

	/* 6. a wait that drains itself */
	reset();
	fill((const unsigned int[]){ 60, 61, 62, 63 }, (const uint32_t[]){ 1, 2, 3, 4 }, 4);
	answer_done = 2;
	wait_drains = true;
	{
		const unsigned int id = 64;
		int rc = pf_push(&q, &id, 1, 5, 4, wait_stub, release);

		check(rc == 0 && nreleased == 2 && q.n == 3 && q.e[2].p == 64,
		      "a draining wait: each block given back once, the new one held once");
	}

	/* 7. no wait: the default build */
	reset();
	check(PF_WAIT(wait_stub) == NULL, "default build: PF_WAIT is NULL (the wait is opt-in)");
	fill((const unsigned int[]){ 70, 71, 72, 73 }, (const uint32_t[]){ 1, 2, 3, 4 }, 4);
	{
		const unsigned int id = 74;
		int rc = pf_push(&q, &id, 32, 5, 4, PF_WAIT(wait_stub), release);

		check(rc == -ENOSPC && nwaits == 0 && q.n == 4 && nreleased == 0,
		      "no wait: a full queue refuses at once with -ENOSPC");
		check(q.refused == 1 && q.refused_no_wait == 1 && q.refused_kib == 32 && q.waits == 0,
		      "no wait: counted as such");
	}

	/* 8. the counters' line */
	{
		char line[192];

		q.waits = 3;
		q.refused = 2;
		q.refused_unsubmitted = 1;
		q.refused_no_wait = 1;
		q.refused_kib = 1536;
		pf_format(&q, line, sizeof(line));
		check(strcmp(line, "pending frees: high-water 4/4, waits 3 (opt-in 0), refused 2 (1 unsubmitted, 1 no wait, "
				   "1536 KiB kept)") == 0,
		      "the counters' line");
	}

	const int planned = 18;
	printf("pending_frees_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
