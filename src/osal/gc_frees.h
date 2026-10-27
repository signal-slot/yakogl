/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_frees.h - when the GC7000's Zephyr OSAL (zephyr_gc.c) may give freed GPU
 * memory back to the driver's pool. Plain C with no Zephyr in it, so the host
 * checks it (tools/gc/osal_frees_check.c).
 *
 * Submits are asynchronous here: a submitted kick may still be running. So a
 * freed block is held until the last kick that may read it has completed:
 * - while the backend records a frame (OSAL_QUERY_GC_FRAME_OPEN 1), that is
 *   the frame being recorded, the next kick submitted (fence submitted + 1);
 * - with no frame open, it is the last kick submitted (fence submitted),
 *   and a block whose kick has already completed goes back at once.
 * A held block goes back through `release` once its fence has completed:
 * straight to the pool, since it has waited for its own kick
 * (gc_render_free_retired).
 *
 * Before this, every free waited for submitted + 1, also while nothing was
 * recorded. When nothing records, the backend submits nothing, so that kick
 * did not exist until the next draw: dEQP texture.size's next case uploads
 * before it draws, and the i.MX8MP ran out of its 64 MiB carveout with
 * 32 MiB held that way. The OSAL then handed each block to gc_render_free(),
 * which held it for one kick more (8 MiB more). tools/cts/RESULTS.md,
 * "What the 43 MiB are". The V3D has the same rule (v3d_frees.h, T4).
 *
 * The held blocks sit in the queue every deferring OSAL shares
 * (pending_frees.h): a full queue waits for its oldest kick (opt-in) or
 * refuses the free, counted, never a silent leak.
 */
#ifndef GC_FREES_H
#define GC_FREES_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* The queue's capacity. What bounds it: static RAM on the Zephyr image (an
 * entry is 40 bytes, 5 KiB for the queue) against how often a burst of
 * deletes between two kicks has to wait for the oldest kick. */
#define GC_FREES_MAX 128

struct gc_frees_block {
	void *cpu;
	uint32_t gpu;
	uint64_t phys;
	size_t size;
};

#define PF_PAYLOAD  struct gc_frees_block
#define PF_CAPACITY GC_FREES_MAX
#include "pending_frees.h"

struct gc_frees {
	struct pf_queue q; /* the held blocks; its counters cover a full queue */
	bool frame_open;   /* the backend is recording a frame */
	/* counters: so a zero is told apart from "never ran" */
	uint32_t freed_at_once;   /* frees whose kick had already completed */
	uint32_t drained;         /* held frees given back by a drain */
	size_t held_bytes;        /* held now */
	size_t held_peak_bytes;   /* high-water of held_bytes */
};

/* Gives one block back to the pool. */
typedef pf_release_fn gc_frees_release_fn;

/* Until the backend says, a frame is assumed open: every free is held for
 * the next kick, the behaviour before OSAL_QUERY_GC_FRAME_OPEN. */
static inline void gc_frees_init(struct gc_frees *f)
{
	*f = (struct gc_frees){ .frame_open = true };
}

/* held_bytes from the queue (at most GC_FREES_MAX entries), and its peak */
static inline void gc_frees_count(struct gc_frees *f)
{
	size_t held = 0;

	for (unsigned int i = 0; i < f->q.n; i++)
		held += f->q.e[i].p.size;
	f->held_bytes = held;
	if (held > f->held_peak_bytes)
		f->held_peak_bytes = held;
}

/* Give back every held block whose kick has completed. Fences never wrap in
 * an image's life (gc_render.h), so a plain comparison is right. */
static inline void gc_frees_drain(struct gc_frees *f, uint32_t completed, gc_frees_release_fn release)
{
	f->drained += pf_drain(&f->q, completed, release);
	gc_frees_count(f);
}

/* Free `b`: at once when the last kick that may read it has completed, else
 * held for it. On a full queue, pending_frees.h's rule: `wait` (PF_WAIT of
 * the OSAL's wait, NULL without the opt-in) for the oldest kick, or refuse.
 * Returns 0, or the refusal's negative code (counted in f->q). */
static inline int gc_frees_free(struct gc_frees *f, const struct gc_frees_block *b, uint32_t submitted,
				uint32_t completed, pf_wait_fn wait, gc_frees_release_fn release)
{
	uint32_t fence = f->frame_open ? submitted + 1 : submitted;
	unsigned int before;
	int rc;

	gc_frees_drain(f, completed, release);
	if (fence <= completed) {
		release(b);
		f->freed_at_once++;
		return 0;
	}
	before = f->q.n;
	rc = pf_push(&f->q, b, (uint32_t)((b->size + 1023) / 1024), fence, submitted, wait, release);
	/* what a full-queue wait gave back counts as drained */
	f->drained += before + (rc == 0) - f->q.n;
	gc_frees_count(f);
	return rc;
}

#endif
