/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dlist_alloc: placement of two (front/back) display lists in BCM2712 HVS6
 * display-list memory, clear of the lists the firmware still scans out, and
 * the swap between them. Pure arithmetic, no register access: the caller
 * takes the live spans from a snapshot and publishes the returned pointer.
 *
 * Facts and what is not confirmed:
 *  - The firmware keeps the first HVS_BOOTLOADER_DLIST_END = 32 words of list
 *    memory ("never overwrite the setup from the bootloader"): facts file
 *    section 10, vc4_hvs.c:1570-1583, vc4_regs.h:1056. DLA_BOOTLOADER_END.
 *  - The size of the display-list memory in words: UNVERIFIED (the facts file
 *    does not give it); the caller passes it.
 *  - The alignment a list needs (one word assumed, DLA_MIN_ALIGN): UNVERIFIED.
 *  - The channel's list pointer holds the list's start as a word offset into
 *    list memory: UNVERIFIED. dla_ptr() is the one place to change.
 *
 * Words, not bytes, throughout. C11, no libc beyond stdint/stddef.
 */
#ifndef RPI5_SCANOUT_DLIST_ALLOC_H
#define RPI5_SCANOUT_DLIST_ALLOC_H

#include <stddef.h>
#include <stdint.h>

#define DLA_BOOTLOADER_END 32u /* words kept by the firmware, from 0 */
#define DLA_MIN_ALIGN 1u       /* unverified */
#define DLA_MAX_LIVE 32u

enum dla_result {
	DLA_OK = 0,
	DLA_ERR_ARG = -1,   /* zero length, bad alignment, null pointer, too many spans */
	DLA_ERR_SPAN = -2,  /* a live span lies outside the memory */
	DLA_ERR_NOROOM = -3 /* no two disjoint free slots */
};

struct dla_span {
	uint32_t start;
	uint32_t len;
};

struct dla_slots {
	uint32_t start[2];
	uint32_t len;
	int front; /* slot index the hardware scans (0 or 1) */
};

/* The value the channel's list pointer takes for a list at word offset start. */
static inline uint32_t dla_ptr(uint32_t start)
{
	return start; /* unverified: word offset */
}

static inline uint64_t dla_up(uint64_t v, uint32_t align)
{
	return (v + align - 1) / align * align;
}

/* First aligned start >= the kept words where [s, s+len) meets no span; -1 if none. */
static inline int64_t dla_first_fit(uint32_t mem, uint32_t len, uint32_t align,
				    const struct dla_span *sp, size_t n)
{
	uint64_t s = dla_up(DLA_BOOTLOADER_END, align);
	int moved;

	do {
		moved = 0;
		if (s + len > mem)
			return -1;
		for (size_t i = 0; i < n; i++) {
			uint64_t a = sp[i].start, b = a + sp[i].len;

			if (sp[i].len && s < b && a < s + len) {
				s = dla_up(b, align);
				moved = 1;
			}
		}
	} while (moved);
	return (int64_t)s;
}

/*
 * Two slots of len words, each aligned to align (a power of two), disjoint
 * from each other, from words [0, DLA_BOOTLOADER_END) and from every live
 * span, inside mem words. On refusal out is untouched.
 */
static inline int dlist_place(uint32_t mem, uint32_t len, uint32_t align,
			      const struct dla_span *live, size_t nlive,
			      struct dla_slots *out)
{
	struct dla_span tmp[DLA_MAX_LIVE + 1];
	int64_t a, b;

	if (!out || !len || !align || (align & (align - 1)) || (nlive && !live) ||
	    nlive > DLA_MAX_LIVE)
		return DLA_ERR_ARG;
	if (align < DLA_MIN_ALIGN)
		align = DLA_MIN_ALIGN;
	for (size_t i = 0; i < nlive; i++) {
		if ((uint64_t)live[i].start + live[i].len > mem)
			return DLA_ERR_SPAN;
		tmp[i] = live[i];
	}
	a = dla_first_fit(mem, len, align, tmp, nlive);
	if (a < 0)
		return DLA_ERR_NOROOM;
	tmp[nlive].start = (uint32_t)a;
	tmp[nlive].len = len;
	b = dla_first_fit(mem, len, align, tmp, nlive + 1);
	if (b < 0)
		return DLA_ERR_NOROOM;
	out->start[0] = (uint32_t)a;
	out->start[1] = (uint32_t)b;
	out->len = len;
	out->front = 0;
	return DLA_OK;
}

/* Slot the caller writes next: the one the hardware is not scanning. */
static inline int dlist_back(const struct dla_slots *s)
{
	return 1 - s->front;
}

/*
 * Publish the back slot (already written): *ptr is the pointer value to put in
 * the channel, the back slot becomes the front, and the slot to write next
 * (the old front) is returned. The old front must not be rewritten before the
 * hardware has moved to the new list (one frame; the caller waits). Returns
 * -1 on a null argument or a bad front index.
 */
static inline int dlist_swap(struct dla_slots *s, uint32_t *ptr)
{
	if (!s || !ptr || (s->front != 0 && s->front != 1))
		return -1;
	s->front = dlist_back(s);
	*ptr = dla_ptr(s->start[s->front]);
	return dlist_back(s);
}

#endif
