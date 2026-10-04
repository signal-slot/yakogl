/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_frees_check.c - host check of src/osal/v3d_frees.h, the V3D OSAL's
 * rule for giving freed GPU memory back (zephyr_v3d.c uses it; no host build
 * reaches that file).
 *
 *   cc -std=c11 -O1 -Wall -Wextra -Werror -Isrc/osal tools/v3d/osal_frees_check.c \
 *      -o build/v3d-check/osal_frees_check && build/v3d-check/osal_frees_check
 *
 * The shapes:
 * 1. T4's (cube.512x512_l8 on the Pi 4): textures deleted and allocated again
 *    with no job open (their draws refused). Every free comes back at once.
 *    Before v3d_frees.h, every free waited for the next job, and 27 deleted
 *    textures (25.3 MB) stayed held.
 * 2. A free while the backend records a job: held until that job (the next
 *    submitted) has completed, then given back by drain.
 * 3. The backend closes the job: drain gives back what it held.
 * 4. A full queue whose oldest job is still being recorded: no wait can
 *    complete it, so the free is refused (-EBUSY) and counted, not lost
 *    silently.
 * 6. A job in flight (the asynchronous submit: completed == submitted - 1),
 *    no job open: a free waits for the job in flight (fence = submitted) and
 *    comes back when that fence completes, not before. Under the old rule
 *    (no job open: at once) it came back while the GPU could still read it.
 * 7. A job in flight and one open: held for the one being recorded
 *    (submitted + 1); the in-flight job completing does not release it.
 * 8. The in-flight job already completed (completed == submitted) and no job
 *    open: at once, as with a synchronous OSAL.
 * 9. A full queue whose oldest job was submitted: the wait for it
 *    (pending_frees.h) gives the queue back and the new free is held.
 * 10. A glmark2 scene's teardown (v3d_frees_poll): the scene's last job has
 *    finished on the GPU, but the OSAL has not looked (completed ==
 *    submitted - 1). Without the poll the free is held for that job; with
 *    it the poll finds the job done and the free comes back at once.
 * 11. Before an allocation: frees held for a job the GPU has finished come
 *    back when the poll looks, so the allocation sees them free.
 * 12. The poll on a job still running: nothing changes, the free is held
 *    for it as in 6.
 */
#include <stdio.h>
#include <string.h>

#include "v3d_frees.h"

static int checks, failures;

static void check(bool ok, const char *what)
{
	checks++;
	if (!ok) {
		failures++;
		printf("osal_frees_check: FAIL %s\n", what);
	}
}

/* the allocator: which granules are free again */
#define GRANULES 4096
static bool released[GRANULES];
static uint32_t nreleased;

static void release(const struct v3d_frees_run *r)
{
	for (uint32_t i = r->granule; i < r->granule + r->n && i < GRANULES; i++)
		released[i] = true;
	nreleased += r->n;
}

/* the OSAL's wait, as pending_frees.h calls it: job `fence` completes */
static int nwaits;

static int wait_ok(uint32_t fence, uint32_t *completed)
{
	nwaits++;
	*completed = fence;
	return 0;
}

/* the OSAL's poll (v3d_frees_poll): what FRDONE says now, counted */
static uint32_t poll_answer;
static int npolls;

static uint32_t poll_job(void)
{
	npolls++;
	return poll_answer;
}

static void reset(struct v3d_frees *f)
{
	memset(f, 0, sizeof(*f));
	memset(released, 0, sizeof(released));
	nreleased = 0;
	nwaits = 0;
	npolls = 0;
}

int main(void)
{
	static struct v3d_frees f;
	uint32_t submitted = 5, completed = 5; /* a synchronous OSAL */

	/* 1. T4: 27 textures deleted with no job open, then an allocation */
	reset(&f);
	for (uint32_t t = 0; t < 27; t++)
		v3d_frees_free(&f, t * 100, 90, 90, submitted, completed, wait_ok, release);
	check(nreleased == 27 * 90 && f.q.n == 0, "no job open: 27 deleted textures come back at once");
	check(released[26 * 100 + 89], "no job open: the last texture's last granule is free");

	/* 2. a free while a job is recorded: held for it */
	reset(&f);
	f.job_open = true;
	v3d_frees_free(&f, 10, 4, 4, submitted, completed, wait_ok, release);
	check(nreleased == 0 && f.q.n == 1, "a job open: the free is held");
	check(f.q.e[0].fence == submitted + 1, "a job open: held for the job being recorded");
	v3d_frees_drain(&f, completed, release);
	check(nreleased == 0, "a job open: a drain before the job runs gives nothing back");

	/* 3. the job is submitted and completes, the backend closes it */
	submitted++;
	completed++;
	f.job_open = false;
	v3d_frees_drain(&f, completed, release);
	check(nreleased == 4 && f.q.n == 0 && released[13], "the job ran: its held free comes back");

	/* 4. a full list: leaked and counted */
	reset(&f);
	f.job_open = true;
	for (uint32_t i = 0; i < V3D_FREES_MAX; i++)
		v3d_frees_free(&f, i, 1, 1, submitted, completed, wait_ok, release);
	check(v3d_frees_free(&f, 3000, 7, 7, submitted, completed, wait_ok, release) == -EBUSY && nwaits == 0 &&
		      f.q.refused_unsubmitted == 1 && f.q.refused_kib == 7 && nreleased == 0,
	      "a full queue, its oldest job open: refused and counted");

	/* 5. the free runs: all free, a hole between used blocks, runs at both
	 * ends, the first granule respected, and fragmentation (enough free in
	 * total, no run long enough) */
	{
		static uint32_t map[GRANULES / 32];
		uint32_t longest, runs;
		memset(map, 0, sizeof(map));
		v3d_free_runs(map, 16, GRANULES, &longest, &runs);
		check(longest == GRANULES - 16 && runs == 1, "free runs: an empty map is one run past `first`");
		for (uint32_t g = 100; g < 200; g++)
			map[g / 32] |= 1u << (g % 32);
		for (uint32_t g = 250; g < GRANULES; g++)
			map[g / 32] |= 1u << (g % 32);
		v3d_free_runs(map, 16, GRANULES, &longest, &runs);
		check(longest == 84 && runs == 2, "free runs: [16,100) and [200,250): the longest 84, two runs");
		/* every other 8 granules used: 2048 free in total, none longer than 8 */
		memset(map, 0, sizeof(map));
		for (uint32_t g = 0; g < GRANULES; g++)
			if ((g / 8) % 2)
				map[g / 32] |= 1u << (g % 32);
		v3d_free_runs(map, 0, GRANULES, &longest, &runs);
		check(longest == 8 && runs == GRANULES / 16, "free runs: fragmented, 2048 free but no run over 8");
	}

	/* 6. a job in flight, none open: held for the job in flight */
	reset(&f);
	submitted = 9;
	completed = 8; /* job 9's render started, not yet observed done */
	v3d_frees_free(&f, 40, 3, 3, submitted, completed, wait_ok, release);
	check(nreleased == 0 && f.q.n == 1 && f.q.e[0].fence == 9,
	      "in flight, none open: held for the job in flight (fence submitted)");
	v3d_frees_drain(&f, completed, release);
	check(nreleased == 0, "in flight: a drain before it completes gives nothing back");
	completed = 9;
	v3d_frees_drain(&f, completed, release);
	check(nreleased == 3 && f.q.n == 0 && released[42], "in flight: it completed, the free comes back");

	/* 7. a job in flight and one open: held for the one being recorded */
	reset(&f);
	submitted = 9;
	completed = 8;
	f.job_open = true;
	v3d_frees_free(&f, 50, 2, 2, submitted, completed, wait_ok, release);
	check(f.q.n == 1 && f.q.e[0].fence == 10, "in flight and one open: held for submitted + 1");
	completed = 9;
	v3d_frees_drain(&f, completed, release);
	check(nreleased == 0 && f.q.n == 1, "in flight and one open: the in-flight job completing keeps it");
	submitted = 10;
	completed = 10;
	v3d_frees_drain(&f, completed, release);
	check(nreleased == 2 && f.q.n == 0, "in flight and one open: the recorded job completed, it comes back");

	/* 8. nothing in flight, none open: at once */
	reset(&f);
	submitted = 10;
	completed = 10;
	v3d_frees_free(&f, 60, 5, 5, submitted, completed, wait_ok, release);
	check(nreleased == 5 && f.q.n == 0, "nothing in flight, none open: at once");

	/* 9. a full queue, its oldest job submitted: wait, drain, hold */
	reset(&f);
	submitted = 10;
	completed = 10;
	f.job_open = true;
	for (uint32_t i = 0; i < V3D_FREES_MAX; i++)
		v3d_frees_free(&f, i, 1, 1, submitted, completed, wait_ok, release); /* held for job 11 */
	submitted = 11; /* job 11 in flight, the next one open */
	check(v3d_frees_free(&f, 3000, 7, 2, submitted, completed, wait_ok, release) == 0 && nwaits == 1 &&
		      nreleased == V3D_FREES_MAX,
	      "a full queue, its oldest job submitted: one wait gives the queue back");
	check(f.q.n == 1 && f.q.e[0].fence == 12 && f.q.waits == 1 && f.q.refused == 0 && f.q.high_water == V3D_FREES_MAX,
	      "a full queue: the new free held for job 12, the wait and high-water counted");

	/* 10. teardown after the GPU finished, the OSAL not yet looking */
	reset(&f);
	submitted = 10;
	completed = 9;
	v3d_frees_free(&f, 70, 4, 4, submitted, completed, wait_ok, release);
	check(nreleased == 0 && f.q.n == 1, "unlooked completion, no poll: the free is held for the job in flight");
	reset(&f);
	poll_answer = 10; /* FRDONE is up */
	completed = v3d_frees_poll(&f, poll_job, release);
	v3d_frees_free(&f, 70, 4, 4, submitted, completed, wait_ok, release);
	check(npolls == 1 && completed == 10 && nreleased == 4 && f.q.n == 0,
	      "unlooked completion, polled first: the free comes back at once");

	/* 11. before an allocation: the held frees of a finished job come back */
	reset(&f);
	completed = 9;
	for (uint32_t i = 0; i < 5; i++)
		v3d_frees_free(&f, 100 + 10 * i, 10, 10, submitted, completed, wait_ok, release);
	poll_answer = 10;
	(void)v3d_frees_poll(&f, poll_job, release);
	check(nreleased == 50 && f.q.n == 0, "before an allocation: the poll gives back what the finished job held");

	/* 12. the poll on a job still running changes nothing */
	reset(&f);
	poll_answer = 9; /* FRDONE not up */
	completed = v3d_frees_poll(&f, poll_job, release);
	v3d_frees_free(&f, 80, 2, 2, submitted, completed, wait_ok, release);
	check(completed == 9 && nreleased == 0 && f.q.n == 1 && f.q.e[0].fence == 10,
	      "a job still running: the free stays held for it");

	const int planned = 23;
	printf("osal_frees_check: %d checks run (planned %d), %d failed\n", checks, planned, failures);
	return (failures == 0 && checks == planned) ? 0 : 1;
}
