/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_frees.h - when the V3D OSAL (zephyr_v3d.c) may give freed GPU memory
 * back to its allocator. Plain C with no Zephyr in it, so the host checks it
 * (tools/v3d/osal_frees_check.c).
 *
 * A job may still be running when submit() returns: the OSAL starts a job's
 * render and returns, and finishes the job (FRDONE, the TMU clean, its fence
 * completed) in wait(), in completed(), or at the next submit(), at most one
 * job in flight. A freed block may be read by the job being recorded and by
 * the job in flight. The backend says when it opens and closes a job
 * (OSAL_QUERY_V3D_JOB_OPEN). The fence a free waits for is the
 * newest job that may read it, as in zephyr_gc.c (gc_frees.h):
 * - a job open: the job being recorded, the next one submitted
 *   (fence = submitted + 1);
 * - no job open: the last job submitted (fence = submitted), which may be
 *   in flight.
 * A free whose fence has already completed goes back at once; the others are
 * held and given back by drain, which runs whenever a fence completes.
 * With every job completed inside submit() (completed == submitted, as
 * before the asynchronous submit), this is the rule it replaces: at once with
 * no job open, held for submitted + 1 with one open.
 *
 * Before the at-once case, every free was held for the next job, so an
 * application that deleted textures and allocated again without drawing
 * never got the memory back (T4's cube.512x512_l8 OOM on the Pi 4: 25.3 MB
 * held by 27 deleted textures whose draws were refused).
 *
 * The held frees sit in the queue every deferring OSAL shares
 * (pending_frees.h): a full queue waits for its oldest job (opt-in) or
 * refuses the free, counted, never a silent leak.
 */
#ifndef V3D_FREES_H
#define V3D_FREES_H

#include <stdbool.h>
#include <stdint.h>

/* The queue's capacity. What bounds it: static RAM on the Zephyr image (an
 * entry is 16 bytes, 4 KiB for the queue) against how often a burst of
 * deletes between two jobs has to wait for the oldest job. */
#define V3D_FREES_MAX 256

/* granules [granule, granule + n) */
struct v3d_frees_run {
	uint32_t granule, n;
};

#define PF_PAYLOAD  struct v3d_frees_run
#define PF_CAPACITY V3D_FREES_MAX
#include "pending_frees.h"

struct v3d_frees {
	struct pf_queue q; /* the held frees; its counters cover a full queue */
	bool job_open;     /* the backend is recording a job */
};

/* Gives granules [r->granule, r->granule + r->n) back to the allocator. */
typedef pf_release_fn v3d_frees_release_fn;

/* Give back every held free whose job has completed. */
static inline void v3d_frees_drain(struct v3d_frees *f, uint32_t completed, v3d_frees_release_fn release)
{
	pf_drain(&f->q, completed, release);
}

/* The newest completed fence after looking at the job in flight without
 * blocking (zephyr_v3d.c: finish_inflight(true), FRDONE read once). FRDONE
 * has no interrupt, so the OSAL learns that a job completed only when it
 * looks: in wait(), completed() or the next submit. Between a glmark2
 * scene's teardown and the next scene's setup nothing looks, and on the Pi
 * 4 the torn-down scene's frees stayed held (31.8 MB after terrain), the
 * next setup allocated around them and refract never found 7.7 MB in one
 * piece (tools/cts/RESULTS.md, the memory lines). */
typedef uint32_t (*v3d_frees_poll_fn)(void);

/* Look at the job in flight (`poll`) and give back every held free whose
 * job has completed. Returns the newest completed fence, for the free that
 * follows. For before a free is fenced and before an allocation is placed
 * while frees are held, so neither sees a completion the GPU has already
 * reached as still pending. */
static inline uint32_t v3d_frees_poll(struct v3d_frees *f, v3d_frees_poll_fn poll, v3d_frees_release_fn release)
{
	const uint32_t completed = poll();

	v3d_frees_drain(f, completed, release);
	return completed;
}

/* Free [granule, granule + n), `kib` KiB: held until the newest job that may
 * read it has completed (submitted + 1 with a job open, else submitted), and
 * given back at once when that job already has. On a full queue,
 * pending_frees.h's rule: `wait` (PF_WAIT of the OSAL's wait, NULL without
 * the opt-in) for the oldest job, or refuse. Returns 0, or the refusal's
 * negative code (counted in f->q). */
static inline int v3d_frees_free(struct v3d_frees *f, uint32_t granule, uint32_t n, uint32_t kib,
				 uint32_t submitted, uint32_t completed, pf_wait_fn wait,
				 v3d_frees_release_fn release)
{
	const struct v3d_frees_run r = { .granule = granule, .n = n };

	v3d_frees_drain(f, completed, release);
	const uint32_t fence = f->job_open ? submitted + 1 : submitted;
	if (fence <= completed) {
		release(&r);
		return 0;
	}
	return pf_push(&f->q, &r, kib, fence, submitted, wait, release);
}

/* The free runs of the allocator's granule map (bit set = used) over
 * [first, n): the longest run in granules and how many runs there are.
 * A large allocation fails when the longest run is shorter than it even
 * though enough granules are free in total: fragmentation (DEMO4, 2026-09-25:
 * a 5.5 MB tiled copy failed 634 times with 11.6 MB of the carve-out free).
 * Either pointer may be NULL. */
static inline void v3d_free_runs(const uint32_t *map, uint32_t first, uint32_t n, uint32_t *longest,
				 uint32_t *runs)
{
	uint32_t best = 0, cur = 0, count = 0;
	for (uint32_t g = first; g < n; g++) {
		if (map[g / 32] & (1u << (g % 32))) {
			cur = 0;
			continue;
		}
		if (cur++ == 0)
			count++;
		if (cur > best)
			best = cur;
	}
	if (longest)
		*longest = best;
	if (runs)
		*runs = count;
}

#endif
