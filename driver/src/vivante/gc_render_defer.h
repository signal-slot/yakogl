/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_render_defer.h - hold a freed block until the kick that referenced it has
 * retired, as static functions over an explicit state so the host test can
 * exercise the shipped code rather than a copy of it (the twin of
 * gc_render_pool.h, for the same reason).
 *
 * Why it exists. GL lets an application delete or re-specify a buffer while a
 * draw that reads it is still in flight; the implementation owns the lifetime,
 * not the caller. Freeing on the spot returns the block to the pool, the next
 * allocation is handed the same memory, and the GPU reads what the new owner
 * wrote. Measured on the i.MX8MP as gl_test case 20 after case 40: the ETC2
 * half drew the background, and both knobs that stopped immediate reuse at the
 * time (pool_no_reuse, and a pool_defer_free that this header replaced) made it
 * pass while scrubbing the freed memory did not change the wrong value -- which
 * is what said the block was live to the GPU rather than merely stale.
 *
 * Deferral is the path now rather than an option, so pool_defer_free no longer
 * exists: the live module parameter is `defer_off`, with the opposite sense,
 * turning deferral off to reproduce the defect on demand.
 *
 * The key is the referencing kick's own sequence, not "the next kick": a block
 * freed while kick N is being recorded is safe once `completed >= N`, and
 * waiting for anything later would hold memory longer than its own work needs.
 *
 * What happens when the queue is full is the whole design. The obvious
 * fallback -- free it immediately, just this once -- reintroduces exactly the
 * defect the queue exists to prevent, and does so under memory pressure, which
 * is when it is least likely to be noticed. It is the same shape as the pool's
 * old "the list is full, drop the block that just arrived": under pressure, do
 * the unsafe thing quietly. So this queue never frees early. The caller drains
 * and retries, and if that cannot make room it must wait or give up the block;
 * the one thing it may not do is hand it back to the pool.
 */
#ifndef GC_RENDER_DEFER_H
#define GC_RENDER_DEFER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

/* Sized by measurement, not by guess. pooltest's churn workload (allocate,
 * submit, free inside the same frame, N frames in flight) reports the peak
 * occupancy over 200k operations:
 *
 *   frames in flight:   1    2    3    4    6    8   16
 *   peak, seed A:      42   42   44   42   46   42   75
 *   peak, seed B:      55   64   63   64   64   64   77
 *
 * It barely depends on the number of frames in flight, which is the useful
 * part: the queue holds roughly the frees issued between two retirements, and
 * that is set by the allocation pattern rather than by pipeline depth. 256 is
 * a little over three times the worst of those, and the cost of the headroom
 * is 6 KiB of BSS.
 *
 * Being wrong here is not fatal by design: past the bound the caller waits,
 * and failing that leaks the block loudly. It never frees early. */
#ifndef GC_DEFER_SLOTS
#define GC_DEFER_SLOTS 256
#endif

/* How long a free waits for the oldest kick when the queue is full. Bounded so
 * that a wedged GPU cannot turn a free into a hang; on expiry the caller leaks
 * the block loudly rather than freeing it early. */
#ifndef GC_DEFER_WAIT_MS
#define GC_DEFER_WAIT_MS 2000u
#endif

struct gc_defer_block {
	size_t   off;    /* offset into the carveout, as the pool sees it */
	size_t   size;
	uint32_t fence;  /* retire this block once completed >= fence */
};

struct gc_defer {
	struct gc_defer_block q[GC_DEFER_SLOTS];
	unsigned n;
	unsigned peak;      /* high-water occupancy: what sizes the bound */
	unsigned refused;   /* pushes the queue could not take (never freed early) */
};

static inline void gc_defer_init(struct gc_defer *d)
{
	memset(d, 0, sizeof(*d));
}

/* Returns false when the queue is full. The caller must NOT free the block on
 * a false: drain and retry, wait, or leak it -- never hand it to the pool. */
static inline bool gc_defer_push(struct gc_defer *d, size_t off, size_t size, uint32_t fence)
{
	if (d->n >= GC_DEFER_SLOTS) {
		d->refused++;
		return false;
	}
	d->q[d->n].off = off;
	d->q[d->n].size = size;
	d->q[d->n].fence = fence;
	d->n++;
	if (d->n > d->peak) {
		d->peak = d->n;
	}
	return true;
}

/* Move every block whose kick has retired into `out` (up to `max`) and return
 * how many. Order is preserved, so blocks come back in submission order and
 * nothing waits longer than its own kick.
 *
 * The comparison is signed on the difference so that a fence counter which
 * wraps still orders correctly. */
static inline unsigned gc_defer_drain(struct gc_defer *d, uint32_t completed,
				      struct gc_defer_block *out, unsigned max)
{
	unsigned kept = 0, got = 0;

	for (unsigned i = 0; i < d->n; i++) {
		if (got < max && (int32_t)(completed - d->q[i].fence) >= 0) {
			out[got++] = d->q[i];
		} else {
			d->q[kept++] = d->q[i];
		}
	}
	d->n = kept;
	return got;
}

/* The fence the caller has to wait for to make room, or false when empty. */
static inline bool gc_defer_oldest(const struct gc_defer *d, uint32_t *fence)
{
	if (d->n == 0) {
		return false;
	}
	*fence = d->q[0].fence;
	for (unsigned i = 1; i < d->n; i++) {
		if ((int32_t)(d->q[i].fence - *fence) < 0) {
			*fence = d->q[i].fence;
		}
	}
	return true;
}

#endif /* GC_RENDER_DEFER_H */
