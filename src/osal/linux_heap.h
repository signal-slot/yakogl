/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * linux_heap.h - the fake device-VA allocator behind src/osal/linux.c's
 * gpu_alloc/gpu_free: a finite byte range per heap, first fit over a free
 * list of freed ranges, coalescing a freed range with a free neighbour on
 * either side. Plain C, so the host checks it directly
 * (tools/osal/linux_heap_check.c), the way v3d_granules.h and gc_frees.h are
 * checked.
 *
 * Before this, linux.c only bumped a per-heap offset with no limit and no
 * reuse, a code review finding ("host fake VA: no overflow or reuse").
 * A bump that never stops can wrap a `size_t` for a size near SIZE_MAX, and
 * with no reuse a long-running host session (the no_std Slint harness this
 * OSAL exists for, tools/hostharness) can never give an address back, so its
 * fake address space is only ever spent, never reclaimed, however many
 * glDeleteTextures/glDeleteBuffers calls it makes.
 *
 * Reuse is a free list, not a reset entry: a reset only helps a test that
 * can start over, and this OSAL is meant to run a real application's call
 * stream for a whole session (linux.c's own header comment), where nothing
 * ever "resets".
 */
#ifndef LINUX_HEAP_H
#define LINUX_HEAP_H

#include <stdint.h>

/* Free ranges kept per heap. A freed range that cannot coalesce into an
 * existing entry once this is full is dropped instead of overwriting one
 * (linux_heap_free's `leaked`): safe (nothing is corrupted, and the range
 * stays marked used forever, so nothing else can be handed out over it), but
 * that byte range never comes back. Sized well past what this OSAL's own
 * users need at once (a host session's live GL objects), so `leaked` staying
 * zero is the expected case, not a tuned limit. */
#define LINUX_HEAP_MAX_FREE 256

struct linux_heap {
	uint64_t range;   /* the heap's whole byte range: [0, range) */
	uint64_t bump;    /* the top of what has ever been carved from `range` */
	uint64_t used;    /* bytes live now */
	uint64_t high;    /* the most `used` has ever been ("the high-water mark") */
	uint64_t leaked;  /* bytes freed while the free list was full: never reused */
	struct {
		uint64_t off, size;
	} free_[LINUX_HEAP_MAX_FREE];
	unsigned nfree;   /* free_[0 .. nfree), kept sorted by `off`, no two adjacent
			   * (adjacent free ranges are always coalesced into one) */
};

static inline void linux_heap_init(struct linux_heap *h, uint64_t range)
{
	h->range = range;
	h->bump = 0;
	h->used = 0;
	h->high = 0;
	h->leaked = 0;
	h->nfree = 0;
}

/* First fit over the free list; when nothing there is large enough, carve
 * `size` fresh bytes from the bump. -1 when neither has room: the heap's
 * finite range is exhausted (the caller reports -ENOMEM), never a wrap. */
static inline int64_t linux_heap_alloc(struct linux_heap *h, uint64_t size)
{
	unsigned i;
	uint64_t off;

	if (size == 0 || size > h->range)
		return -1;
	for (i = 0; i < h->nfree; i++) {
		if (h->free_[i].size < size)
			continue;
		off = h->free_[i].off;
		if (h->free_[i].size == size) {
			for (unsigned j = i; j + 1 < h->nfree; j++)
				h->free_[j] = h->free_[j + 1];
			h->nfree--;
		} else {
			h->free_[i].off += size;
			h->free_[i].size -= size;
		}
		h->used += size;
		if (h->used > h->high)
			h->high = h->used;
		return (int64_t)off;
	}
	if (size > h->range - h->bump)
		return -1;
	off = h->bump;
	h->bump += size;
	h->used += size;
	if (h->used > h->high)
		h->high = h->used;
	return (int64_t)off;
}

/* Give `[off, off + size)` back: coalesced into a free neighbour on either
 * side (or both), else a fresh entry -- kept sorted by `off` so the
 * neighbour test is a lookup, not a scan. When the freed range (or, after
 * coalescing, the merged range) reaches all the way up to `bump`, it is
 * folded back into the bump instead of staying a free-list entry, cascading
 * to any entry that now also reaches the new bump: without this, a hole at
 * the very top of the heap and the untouched space above it are two
 * separate free ranges that happen to be adjacent, and a request bigger
 * than either alone but no bigger than both together would be refused even
 * though the heap has the room, contiguously (linux_heap_check.c's "no
 * overlap" model is a whole-range first fit, and is exactly this case: it
 * would place such a request at the hole; without the fold-back this
 * allocator placed it past the untouched space instead, unable to see that
 * the two were really one free run). A caller frees only a range it was
 * given by linux_heap_alloc and never twice, the same contract gpu_free
 * already has (linux.c never double-frees a `struct osal_gpu_mem`); this does
 * not itself detect a double free or a size that does not match the
 * allocation, the way an address is not re-checked against a live map. */
static inline void linux_heap_free(struct linux_heap *h, uint64_t off, uint64_t size)
{
	unsigned i, ins;

	if (size == 0)
		return;
	h->used -= size;
	if (off + size == h->bump) {
		h->bump = off;
		while (h->nfree && h->free_[h->nfree - 1].off + h->free_[h->nfree - 1].size == h->bump) {
			h->bump = h->free_[h->nfree - 1].off;
			h->nfree--;
		}
		return;
	}
	for (ins = 0; ins < h->nfree && h->free_[ins].off < off; ins++)
		;
	if (ins > 0 && h->free_[ins - 1].off + h->free_[ins - 1].size == off) {
		h->free_[ins - 1].size += size;
		if (ins < h->nfree && h->free_[ins - 1].off + h->free_[ins - 1].size == h->free_[ins].off) {
			h->free_[ins - 1].size += h->free_[ins].size;
			for (i = ins; i + 1 < h->nfree; i++)
				h->free_[i] = h->free_[i + 1];
			h->nfree--;
		}
	} else if (ins < h->nfree && off + size == h->free_[ins].off) {
		h->free_[ins].off = off;
		h->free_[ins].size += size;
	} else if (h->nfree >= LINUX_HEAP_MAX_FREE) {
		h->leaked += size;
		return;
	} else {
		for (i = h->nfree; i > ins; i--)
			h->free_[i] = h->free_[i - 1];
		h->free_[ins].off = off;
		h->free_[ins].size = size;
		h->nfree++;
	}
	/* the merge above may have produced (or left) a topmost entry that now
	 * reaches `bump`; fold it in too, the same way, cascading. */
	while (h->nfree && h->free_[h->nfree - 1].off + h->free_[h->nfree - 1].size == h->bump) {
		h->bump = h->free_[h->nfree - 1].off;
		h->nfree--;
	}
}

#endif
