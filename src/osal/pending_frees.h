/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pending_frees.h - the queue of GPU memory that an OSAL was told to free
 * while a kick that may still read it has not completed. Every OSAL that
 * defers frees uses it: zephyr.c (PowerVR), zephyr_gc.c (through
 * gc_frees.h) and zephyr_v3d.c (through
 * v3d_frees.h). linux.c frees at once (every submit completes inside
 * submit()), and baremetal.c never gives memory back (a bump allocator), so
 * neither has a queue.
 *
 * Each entry holds the fence of the newest kick that may read the block. A
 * drain gives back, oldest first, every entry whose fence has completed.
 *
 * A full queue (a code review finding). Before this, a free into a full
 * queue cleared the descriptor and kept the memory for good: an application
 * that deleted many objects between two fences leaked GPU memory without
 * end. Now pf_push:
 *  1. waits for the oldest (lowest) fence in the queue with the OSAL's own
 *     wait, bounded by the timeout that OSAL's submit-wait uses, drains, and
 *     enqueues (counted in `waits`);
 *  2. refuses the free when that wait fails (-ETIMEDOUT or the wait's own
 *     negative code), or when the oldest fence is a kick not submitted yet
 *     (-EBUSY): no wait can complete a kick that is still being recorded,
 *     and the OSAL cannot submit it (the backend owns the recording).
 * A refused block stays allocated for good: a kick may still read it, so
 * giving it back would be worse than losing it. It is counted (`refused`,
 * `refused_kib`), the OSAL logs the first refusal, and the OSAL returns the
 * negative code from gpu_free. The GL object is deleted either way.
 *
 * The wait is opt-in (a hardware-facing change ships opt-in until the
 * regression set has run with it): an OSAL passes its wait as
 * PF_WAIT(fn), which is NULL unless the build defines
 * YAKOGL_PENDING_FREES_WAIT=1. With no wait, step 1 is skipped and a full
 * queue refuses at once (-ENOSPC), counted the same way: the leak the queue
 * had before, now counted, logged and returned instead of silent.
 *
 * Plain C with no OS in it, so the host checks it
 * (tools/osal/pending_frees_check.c).
 *
 * Before including, define:
 *   PF_PAYLOAD  - the type of what `release` needs to give a block back;
 *   PF_CAPACITY - the queue's capacity (one named constant per OSAL).
 * One instance per translation unit.
 */
#ifndef PENDING_FREES_H
#define PENDING_FREES_H

#include <errno.h>
#include <stdint.h>
#include <stdio.h>

#if !defined(PF_PAYLOAD) || !defined(PF_CAPACITY)
#error "define PF_PAYLOAD and PF_CAPACITY before including pending_frees.h"
#endif

/* The full-queue wait's opt-in (the top of this file). */
#ifndef YAKOGL_PENDING_FREES_WAIT
#define YAKOGL_PENDING_FREES_WAIT 0
#endif
#define PF_WAIT(fn) (YAKOGL_PENDING_FREES_WAIT ? (fn) : (pf_wait_fn)0)

struct pf_entry {
	PF_PAYLOAD p;
	uint32_t fence; /* given back once a fence >= this has completed */
	uint32_t kib;   /* the block's size, rounded up, for the counters */
};

struct pf_queue {
	struct pf_entry e[PF_CAPACITY]; /* oldest first */
	unsigned int n;
	unsigned int high_water;      /* most entries held at once */
	uint32_t waits;               /* full-queue waits taken */
	uint32_t refused;             /* frees refused (memory kept for good) */
	uint32_t refused_unsubmitted; /* of those: the oldest kick was not submitted */
	uint32_t refused_no_wait;     /* of those: full, and the wait not opted in */
	uint32_t refused_kib;         /* memory those frees kept */
};

/* Gives one block back to the allocator. */
typedef void (*pf_release_fn)(const PF_PAYLOAD *p);

/* Waits for `fence`, bounded by the OSAL's submit-wait timeout. Returns 0
 * when it completed and sets *completed to the newest completed fence
 * (>= fence); >0 on a timeout; <0 on an error. */
typedef int (*pf_wait_fn)(uint32_t fence, uint32_t *completed);

/* Give back, oldest first, every entry whose fence has completed. Fences
 * never wrap in an image's life, so a plain comparison is right. Returns how
 * many entries went back. */
static inline unsigned int pf_drain(struct pf_queue *q, uint32_t completed, pf_release_fn release)
{
	unsigned int kept = 0, n = q->n;

	for (unsigned int i = 0; i < n; i++) {
		if (q->e[i].fence <= completed)
			release(&q->e[i].p);
		else
			q->e[kept++] = q->e[i];
	}
	q->n = kept;
	return n - kept;
}

static inline int pf_refuse(struct pf_queue *q, uint32_t kib, int rc)
{
	q->refused++;
	q->refused_kib += kib;
	if (rc == -EBUSY)
		q->refused_unsubmitted++;
	if (rc == -ENOSPC)
		q->refused_no_wait++;
	return rc;
}

/* Hold `p` (a block of `kib` KiB) until `fence` has completed. `submitted`
 * is the newest fence submitted. On a full queue: wait for the oldest fence
 * and drain first (see the top of this file); `wait` NULL (PF_WAIT without
 * the opt-in) refuses instead. Returns 0 when the block is held, else the
 * negative code of the refusal. */
static inline int pf_push(struct pf_queue *q, const PF_PAYLOAD *p, uint32_t kib, uint32_t fence,
			  uint32_t submitted, pf_wait_fn wait, pf_release_fn release)
{
	if (q->n >= PF_CAPACITY) {
		uint32_t oldest = q->e[0].fence, done = 0;
		int rc;

		for (unsigned int i = 1; i < q->n; i++)
			if (q->e[i].fence < oldest)
				oldest = q->e[i].fence;
		if (oldest > submitted)
			return pf_refuse(q, kib, -EBUSY);
		if (!wait)
			return pf_refuse(q, kib, -ENOSPC);
		q->waits++;
		rc = wait(oldest, &done);
		if (rc != 0)
			return pf_refuse(q, kib, rc < 0 ? rc : -ETIMEDOUT);
		/* the wait may have drained already (an OSAL's wait drains) */
		pf_drain(q, done, release);
		if (q->n >= PF_CAPACITY) /* a wait that said done below `oldest` */
			return pf_refuse(q, kib, -ETIMEDOUT);
	}
	q->e[q->n].p = *p;
	q->e[q->n].fence = fence;
	q->e[q->n].kib = kib;
	q->n++;
	if (q->n > q->high_water)
		q->high_water = q->n;
	return 0;
}

/* The counters as one line, for the OSAL's close report and its first
 * refusal: "pending frees: high-water H/CAP, waits W (opt-in O), refused R
 * (U unsubmitted, N no wait, K KiB kept)". */
static inline int pf_format(const struct pf_queue *q, char *buf, size_t len)
{
	return snprintf(buf, len,
			"pending frees: high-water %u/%u, waits %u (opt-in %d), refused %u (%u unsubmitted, %u no wait, "
			"%u KiB kept)",
			q->high_water, (unsigned int)PF_CAPACITY, (unsigned int)q->waits, YAKOGL_PENDING_FREES_WAIT,
			(unsigned int)q->refused, (unsigned int)q->refused_unsubmitted, (unsigned int)q->refused_no_wait,
			(unsigned int)q->refused_kib);
}

#endif
