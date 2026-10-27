/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_render_pool.h - the GC7000 render pool: a bump allocator with a small
 * free list, as static functions over an explicit state so that a host test
 * can exercise the shipped code rather than a copy of it.
 *
 * Its PowerVR twin was src/pvr_render_pool.h, extracted for the same reason
 * (now Rust, driver/rust/src/gpu/pvr/pool.rs, tested there):
 * a pool test that tests a transcription of the pool is worth very little.
 *
 * The carveout is identity-mapped and permanently mapped in the GPU MMU, so
 * one offset describes a block in the CPU, physical and GPU address spaces at
 * once and a reused block needs no MMU work. There is exactly one address
 * space here, which is why this pool needs no CPU/VA dual-adjacency test
 * before coalescing (the PowerVR's does, because its VA is reclaimed and
 * re-pointed onto different physical pages).
 *
 * The invariant the host test asserts, and the one that catches real defects:
 *
 *     live + free + dropped == bump - base
 *
 * `dropped` is memory the pool can no longer hand out: alignment padding
 * skipped when splitting a free block or when bumping, and blocks lost when
 * the list is full. A *refused* double free is not dropped -- those bytes are
 * still on the list from the first free -- so it has its own counter and stays
 * out of this one, or the leak metric lies.
 */
#ifndef GC_RENDER_POOL_H
#define GC_RENDER_POOL_H

#include <stddef.h>
#include <stdint.h>   /* SIZE_MAX */
#include <string.h>

#ifndef GC_POOL_ROUND_UP
#define GC_POOL_ROUND_UP(x, a) (((x) + (a) - 1) & ~((a) - 1))
#endif

/* How many free blocks the list can hold. 128 was chosen on the assumption that
 * a UI reuses a stable handful of sizes, and nothing had measured it. The host
 * test did (driver/tools/gclab/pooltest, 200k mixed ops), and the assumption is
 * only half right: the number needed does NOT grow with how many allocations
 * are live -- 256 live or 8192, the peak is the same -- but it does grow with
 * how many distinct sizes and alignments the workload uses, because that is
 * what decides how many holes coalescing cannot close.
 *
 *   workload                             peak free blocks    dropped
 *   15 sizes x 6 alignments (3 seeds)       215 / 313 / 222   3.3 %
 *   continuous 4..256 KiB   (3 seeds)       836 / 911 / 971   1.2 %
 *
 * The second row is deliberately harsher than any real renderer -- every
 * allocation a different size -- and it still peaks under a thousand. 1024
 * covers both with headroom and costs 16 KiB of BSS, which is 0.02 % of the
 * 64 MiB it manages. Scanning it costs about 1.6 us per operation on the host
 * against 0.65 us at 128; both disappear behind the memset that every
 * allocation already does.
 *
 * Overriding it is for the host test, which sweeps it; the driver ships the
 * default. */
#ifndef GC_POOL_FREE_SLOTS
#define GC_POOL_FREE_SLOTS 1024
#endif

struct gc_pool_free_block {
	size_t off;    /* offset into the carveout (already aligned) */
	size_t size;   /* bytes; 0 = empty slot */
};

struct gc_pool {
	size_t base;    /* first usable offset */
	size_t limit;   /* one past the last usable offset */
	size_t bump;    /* everything in [base, bump) has been handed out once */
	size_t last;    /* offset of the newest bump allocation, for the one-block
			 * undo; SIZE_MAX when there is nothing to undo */
	size_t live;    /* bytes currently allocated */
	size_t dropped; /* bytes the pool can never hand out again */
	unsigned double_frees; /* frees refused because the range was already free */
	struct gc_pool_free_block free[GC_POOL_FREE_SLOTS];
};

static inline void gc_pool_init(struct gc_pool *p, size_t base, size_t limit)
{
	memset(p, 0, sizeof(*p));
	p->base = base;
	p->limit = limit;
	p->bump = base;
	p->last = SIZE_MAX;
}

/* Best fit, so a small allocation does not eat a large block. Returns the
 * offset, or SIZE_MAX when nothing on the list fits. */
static inline size_t gc_pool_free_take(struct gc_pool *p, size_t size, size_t align)
{
	struct gc_pool_free_block *best = NULL;
	size_t off;

	for (unsigned int i = 0; i < GC_POOL_FREE_SLOTS; i++) {
		struct gc_pool_free_block *b = &p->free[i];

		if (b->size == 0) {
			continue;
		}
		/* The block has to hold the allocation after the alignment
		 * padding its own start may need. */
		off = GC_POOL_ROUND_UP(b->off, align);
		if (off - b->off + size > b->size) {
			continue;
		}
		if (best == NULL || b->size < best->size) {
			best = b;
		}
	}
	if (best == NULL) {
		return SIZE_MAX;
	}
	off = GC_POOL_ROUND_UP(best->off, align);
	/* Whatever the alignment skipped is dropped with the block: it is at
	 * most `align` bytes and chasing it would need a second slot. The tail
	 * after the allocation is kept. */
	p->dropped += off - best->off;
	best->size -= (off - best->off) + size;
	best->off = off + size;
	return off;
}

/* Returns the offset, or SIZE_MAX when the request does not fit. */
static inline size_t gc_pool_alloc(struct gc_pool *p, size_t size, size_t align)
{
	size_t start;

	if (size == 0) {
		return SIZE_MAX;
	}
	size = GC_POOL_ROUND_UP(size, align);
	start = gc_pool_free_take(p, size, align);
	if (start == SIZE_MAX) {
		start = GC_POOL_ROUND_UP(p->bump, align);
		if (start + size > p->limit) {
			return SIZE_MAX;
		}
		p->dropped += start - p->bump;   /* padding before the bump allocation */
		p->last = start;
		p->bump = start + size;
	} else {
		/* A reused block is not the top of the bump, so the one-block
		 * undo must not think it is. */
		p->last = SIZE_MAX;
	}
	p->live += size;
	return start;
}

static inline void gc_pool_free(struct gc_pool *p, size_t off, size_t size)
{
	struct gc_pool_free_block *empty = NULL;

	if (size == 0 || off < p->base || off + size > p->bump) {
		return;
	}
	/* A range that is already free is a double free: two frees would put
	 * two overlapping blocks in the list and the next two allocations would
	 * be handed the same memory. A duplicate satisfies neither merge test
	 * below (append wants b->off + b->size == off, prepend wants
	 * off + size == b->off), so without this pass it would quietly take an
	 * empty slot. It must be its own pass: the duplicate can sit in any
	 * slot while a merge would commit at the first adjacent one. */
	for (unsigned int i = 0; i < GC_POOL_FREE_SLOTS; i++) {
		const struct gc_pool_free_block *b = &p->free[i];

		if (b->size != 0 && off < b->off + b->size && b->off < off + size) {
			p->double_frees++;
			return;   /* NOT counted as dropped: still free from the first */
		}
	}
	p->live -= size;
	if (off == p->last && off + size == p->bump) {
		p->bump = off;   /* the top of the bump: just give it back */
		p->last = SIZE_MAX;
		return;
	}
	for (unsigned int i = 0; i < GC_POOL_FREE_SLOTS; i++) {
		struct gc_pool_free_block *b = &p->free[i];

		if (b->size == 0) {
			if (empty == NULL) {
				empty = b;
			}
			continue;
		}
		if (b->off + b->size == off) {
			b->size += size;       /* append to the block before it */
			return;
		}
		if (off + size == b->off) {
			b->off = off;          /* prepend to the block after it */
			b->size += size;
			return;
		}
	}
	if (empty == NULL) {
		/* The list is full. Dropping the block that just arrived is the
		 * obvious thing and the wrong one: it makes the loss depend on
		 * arrival order rather than on usefulness, and once the list
		 * fills, every later free is lost the same way. Measured on the
		 * mixed workload at the old 128 slots, that cost 91 % of all
		 * freed memory -- and, worse, reuse stopped, so the bump ran
		 * away to 65 MiB where a working list needs 9.6 MiB.
		 *
		 * Keep the largest blocks instead. A big free block can satisfy
		 * any request a small one can; the reverse is false, so when
		 * something has to go, the smallest listed block is the one
		 * worth least. The incoming block is dropped only when it is
		 * the smallest of the lot.
		 *
		 * This bounds the damage rather than removing it: the list is
		 * still finite, and a workload that outgrows it still loses the
		 * tail. What it removes is the cliff. */
		struct gc_pool_free_block *worst = &p->free[0];

		for (unsigned int i = 1; i < GC_POOL_FREE_SLOTS; i++) {
			if (p->free[i].size < worst->size) {
				worst = &p->free[i];
			}
		}
		if (worst->size < size) {
			p->dropped += worst->size;   /* evicted, not the newcomer */
			worst->off = off;
			worst->size = size;
		} else {
			p->dropped += size;
		}
		return;
	}
	empty->off = off;
	empty->size = size;
}

/* Put the pool back as a freshly initialised one. Safe only when nothing it
 * handed out is still referenced. */
static inline void gc_pool_reset(struct gc_pool *p)
{
	gc_pool_init(p, p->base, p->limit);
}

#endif /* GC_RENDER_POOL_H */
