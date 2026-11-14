/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Framebuffer query and allocate classifier for the VideoCore property
 * mailbox on the BCM2712 (Raspberry Pi 5). Header-only, no Zephyr includes,
 * no calls into rpi_mbox.c: it builds the word buffers a caller sends and
 * parses the answers, so the U3 question "is there a firmware framebuffer,
 * and does FRAMEBUFFER_ALLOCATE still answer" has a host-testable verdict.
 *
 * A buffer is the property interface's: size in bytes, request code, tags
 * (id, value bytes, request/response code, values), end tag 0
 * (drivers/firmware/raspberrypi.c:100-118 in Linux; tag ids from the
 * RPI_FIRMWARE_FRAMEBUFFER_* names in include/soc/bcm2835/
 * raspberrypi-firmware.h). The allocate list is word for word the one
 * driver/zephyr/lib/board/rpi4/rpi_mbox.c builds in rpi_fb_alloc(), which has
 * no virtual-offset tag. GET_DISPLAY_SETTINGS and GET_NUM_DISPLAYS are not
 * here: the Linux header was not at hand to cite them.
 *
 * To send a list with rpi_mbox_request_timed(), pass the words from index 2
 * up to (not including) the end tag: tags = &w[2], words = n - 3.
 */
#ifndef FB_QUERY_H
#define FB_QUERY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define FBQ_TAG_ALLOCATE 0x00040001u
#define FBQ_TAG_GET_PHYS 0x00040003u /* width, height */
#define FBQ_TAG_GET_VIRT 0x00040004u /* width, height */
#define FBQ_TAG_GET_DEPTH 0x00040005u
#define FBQ_TAG_GET_ORDER 0x00040006u
#define FBQ_TAG_GET_PITCH 0x00040008u
#define FBQ_TAG_GET_VOFF 0x00040009u /* x, y */
#define FBQ_TAG_SET_PHYS 0x00048003u
#define FBQ_TAG_SET_VIRT 0x00048004u
#define FBQ_TAG_SET_DEPTH 0x00048005u
#define FBQ_TAG_SET_ORDER 0x00048006u

#define FBQ_STATUS_REQUEST 0x00000000u
#define FBQ_STATUS_SUCCESS 0x80000000u
#define FBQ_TAG_ANSWERED 0x80000000u

/* words of the two lists, header and end tag included */
#define FBQ_QUERY_WORDS 30u
#define FBQ_ALLOC_WORDS 30u

enum fbq_class {
	FBQ_MALFORMED = 0,
	FBQ_PRESENT,
	FBQ_ABSENT,
	FBQ_ALLOC_GRANTED,
	FBQ_ALLOC_REFUSED,
	FBQ_TAG_UNKNOWN,
};

/* bits of fbq_result.seen (tag in the buffer) and .answered (bit 31 set) */
#define FBQ_A_PHYS (1u << 0)
#define FBQ_A_VIRT (1u << 1)
#define FBQ_A_DEPTH (1u << 2)
#define FBQ_A_ORDER (1u << 3)
#define FBQ_A_PITCH (1u << 4)
#define FBQ_A_VOFF (1u << 5)
#define FBQ_A_ALLOC (1u << 6)

struct fbq_result {
	enum fbq_class cls;
	uint32_t seen;
	uint32_t answered;
	uint32_t phys_w, phys_h; /* the physical size answered (the granted size) */
	uint32_t virt_w, virt_h;
	uint32_t depth;
	uint32_t order;
	uint32_t pitch;
	uint32_t voff_x, voff_y;
	uint32_t base; /* ALLOCATE: bus address, 0 when refused */
	uint32_t size;
};

static inline uint32_t fbq_put(uint32_t *w, uint32_t n, uint32_t tag, uint32_t nval,
			       const uint32_t *vals)
{
	w[n++] = tag;
	w[n++] = nval * 4;
	w[n++] = 0;
	for (uint32_t i = 0; i < nval; i++)
		w[n++] = vals ? vals[i] : 0;
	return n;
}

/* The query list: reads only. Writes FBQ_QUERY_WORDS words and returns the
 * count. */
static inline uint32_t fbq_build_query(uint32_t *w)
{
	uint32_t n = 2;
	n = fbq_put(w, n, FBQ_TAG_GET_PHYS, 2, NULL);
	n = fbq_put(w, n, FBQ_TAG_GET_VIRT, 2, NULL);
	n = fbq_put(w, n, FBQ_TAG_GET_DEPTH, 1, NULL);
	n = fbq_put(w, n, FBQ_TAG_GET_ORDER, 1, NULL);
	n = fbq_put(w, n, FBQ_TAG_GET_PITCH, 1, NULL);
	n = fbq_put(w, n, FBQ_TAG_GET_VOFF, 2, NULL);
	w[n++] = 0;
	w[0] = n * 4;
	w[1] = FBQ_STATUS_REQUEST;
	return n;
}

/* The allocate list rpi_fb_alloc() builds: physical and virtual size, depth
 * 32, pixel order 1 (RGB), ALLOCATE with alignment 4096, GET_PITCH. Writes
 * FBQ_ALLOC_WORDS words and returns the count. */
static inline uint32_t fbq_build_alloc(uint32_t *w, uint32_t width, uint32_t height)
{
	const uint32_t size[2] = { width, height };
	const uint32_t depth = 32, order = 1, align[2] = { 4096, 0 };
	uint32_t n = 2;
	n = fbq_put(w, n, FBQ_TAG_SET_PHYS, 2, size);
	n = fbq_put(w, n, FBQ_TAG_SET_VIRT, 2, size);
	n = fbq_put(w, n, FBQ_TAG_SET_DEPTH, 1, &depth);
	n = fbq_put(w, n, FBQ_TAG_SET_ORDER, 1, &order);
	n = fbq_put(w, n, FBQ_TAG_ALLOCATE, 2, align);
	n = fbq_put(w, n, FBQ_TAG_GET_PITCH, 1, NULL);
	w[n++] = 0;
	w[0] = n * 4;
	w[1] = FBQ_STATUS_REQUEST;
	return n;
}

/* Parse the buffer `w` of `nwords` words (header included) after the
 * firmware answered, either list. A buffer with an ALLOCATE tag is judged as
 * an allocate answer, any other as a query answer. */
static inline struct fbq_result fbq_parse(const uint32_t *w, size_t nwords)
{
	struct fbq_result r;
	memset(&r, 0, sizeof(r));
	r.cls = FBQ_MALFORMED;
	if (!w || nwords < 3 || w[0] % 4 || w[0] < 12 || w[0] / 4 > nwords)
		return r;
	if (w[1] != FBQ_STATUS_SUCCESS)
		return r;
	size_t total = w[0] / 4, i = 2;
	bool ended = false;
	while (i < total) {
		uint32_t id = w[i];
		if (id == 0) {
			ended = true;
			break;
		}
		if (i + 3 > total || w[i + 1] % 4)
			return r;
		uint32_t nv = w[i + 1] / 4;
		if (nv > total - (i + 3))
			return r;
		const uint32_t *v = &w[i + 3];
		bool ans = (w[i + 2] & FBQ_TAG_ANSWERED) != 0;
		uint32_t bit = 0, need = 0;
		switch (id & ~0x8000u) { /* a SET tag has the GET id's field */
		case FBQ_TAG_GET_PHYS:
			bit = FBQ_A_PHYS, need = 2;
			break;
		case FBQ_TAG_GET_VIRT:
			bit = FBQ_A_VIRT, need = 2;
			break;
		case FBQ_TAG_GET_DEPTH:
			bit = FBQ_A_DEPTH, need = 1;
			break;
		case FBQ_TAG_GET_ORDER:
			bit = FBQ_A_ORDER, need = 1;
			break;
		case FBQ_TAG_GET_PITCH:
			bit = FBQ_A_PITCH, need = 1;
			break;
		case FBQ_TAG_GET_VOFF:
			bit = FBQ_A_VOFF, need = 2;
			break;
		case FBQ_TAG_ALLOCATE:
			bit = FBQ_A_ALLOC, need = 2;
			break;
		default:
			break; /* a tag this parser has no field for */
		}
		if (bit) {
			if (nv < need)
				return r;
			r.seen |= bit;
			if (ans) {
				r.answered |= bit;
				switch (bit) {
				case FBQ_A_PHYS:
					r.phys_w = v[0], r.phys_h = v[1];
					break;
				case FBQ_A_VIRT:
					r.virt_w = v[0], r.virt_h = v[1];
					break;
				case FBQ_A_DEPTH:
					r.depth = v[0];
					break;
				case FBQ_A_ORDER:
					r.order = v[0];
					break;
				case FBQ_A_PITCH:
					r.pitch = v[0];
					break;
				case FBQ_A_VOFF:
					r.voff_x = v[0], r.voff_y = v[1];
					break;
				default: /* FBQ_A_ALLOC */
					r.base = v[0], r.size = v[1];
					break;
				}
			}
		}
		i += 3 + nv;
	}
	if (!ended || !r.seen)
		return r;

	if (r.seen & FBQ_A_ALLOC) {
		if (!(r.answered & FBQ_A_ALLOC))
			r.cls = FBQ_TAG_UNKNOWN;
		else if (r.base == 0 || r.size == 0)
			r.cls = FBQ_ALLOC_REFUSED;
		else
			r.cls = FBQ_ALLOC_GRANTED;
		return r;
	}
	if (!(r.answered & (FBQ_A_PHYS | FBQ_A_VIRT)))
		r.cls = FBQ_TAG_UNKNOWN;
	else if ((r.phys_w && r.phys_h) || (r.virt_w && r.virt_h))
		r.cls = FBQ_PRESENT;
	else
		r.cls = FBQ_ABSENT;
	return r;
}

/* One short ASCII token for the probe's verdict lines. */
static inline const char *fbq_class_name(enum fbq_class c)
{
	switch (c) {
	case FBQ_PRESENT:
		return "present";
	case FBQ_ABSENT:
		return "absent";
	case FBQ_ALLOC_GRANTED:
		return "granted";
	case FBQ_ALLOC_REFUSED:
		return "refused";
	case FBQ_TAG_UNKNOWN:
		return "unknown";
	default:
		return "malformed";
	}
}

#endif /* FB_QUERY_H */
