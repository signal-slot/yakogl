/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* The Pi 5's mailbox screen (see rpi5_screen.h). */
#include <errno.h>
#include <string.h>

#include "rpi5_screen.h"

#define TAG_ANSWERED 0x80000000u

static const struct {
	uint32_t tag;
	uint32_t words;
} gets[RPI5_GET_COUNT] = {
	[RPI5_GET_PHYSICAL] = { 0x00040003, 2 }, [RPI5_GET_VIRTUAL] = { 0x00040004, 2 },
	[RPI5_GET_DEPTH] = { 0x00040005, 1 },    [RPI5_GET_ORDER] = { 0x00040006, 1 },
	[RPI5_GET_PITCH] = { 0x00040008, 1 },    [RPI5_GET_OFFSET] = { 0x00040009, 2 },
};

int rpi5_screen_query(struct rpi5_fw_fb *s)
{
	int answered = 0;

	memset(s, 0, sizeof(*s));
	for (int i = 0; i < RPI5_GET_COUNT; i++) {
		uint32_t t[5] = { gets[i].tag, gets[i].words * 4, 0, 0, 0 };

		s->rc[i] = rpi_mbox_request_timed(t, 3 + gets[i].words, NULL);
		s->code[i] = t[2];
		s->val[i][0] = t[3];
		s->val[i][1] = gets[i].words > 1 ? t[4] : 0;
		if ((t[2] & TAG_ANSWERED) != 0 && s->rc[i] == 0) {
			answered++;
		} else {
			s->code[i] &= ~TAG_ANSWERED;
		}
	}
	return answered;
}

int rpi5_screen_query_has_fb(const struct rpi5_fw_fb *s)
{
	return (s->code[RPI5_GET_PHYSICAL] & TAG_ANSWERED) != 0 && s->rc[RPI5_GET_PHYSICAL] == 0 &&
	       s->val[RPI5_GET_PHYSICAL][0] != 0 && s->val[RPI5_GET_PHYSICAL][1] != 0;
}

uint32_t rpi5_pixel(uint32_t order, uint8_t r, uint8_t g, uint8_t b)
{
	if (order == 1u) {
		return (uint32_t)b << 16 | (uint32_t)g << 8 | r;
	}
	return (uint32_t)r << 16 | (uint32_t)g << 8 | b;
}

uint32_t rpi5_pattern_at(uint32_t order, uint32_t w, uint32_t h, uint32_t x, uint32_t y)
{
	static const uint8_t bars[8][3] = {
		{ 255, 255, 255 }, { 255, 255, 0 }, { 0, 255, 255 }, { 0, 255, 0 },
		{ 255, 0, 255 },   { 255, 0, 0 },   { 0, 0, 255 },   { 0, 0, 0 },
	};

	if (w == 0u || h == 0u) {
		return 0;
	}
	if (y >= h - h / 8u) {
		return rpi5_pixel(order, (uint8_t)((uint64_t)x * 255u / (w > 1u ? w - 1u : 1u)), 0, 0);
	}
	const uint8_t *c = bars[(uint64_t)x * 8u / w];

	return rpi5_pixel(order, c[0], c[1], c[2]);
}

void rpi5_pattern_fill(uint8_t *fb, uint32_t pitch, uint32_t order, uint32_t w, uint32_t y0, uint32_t y1)
{
	uint32_t h = y1 - y0;

	for (uint32_t y = y0; y < y1; y++) {
		uint32_t *p = (uint32_t *)(fb + (size_t)y * pitch);

		for (uint32_t x = 0; x < w; x++) {
			p[x] = rpi5_pattern_at(order, w, h, x, y - y0);
		}
	}
}

uint32_t rpi5_pattern_verify(const uint8_t *fb, uint32_t pitch, uint32_t order, uint32_t w, uint32_t y0,
			     uint32_t y1, uint32_t *first_bad)
{
	uint32_t h = y1 - y0, bad = 0;

	if (first_bad) {
		*first_bad = UINT32_MAX;
	}
	for (uint32_t y = y0; y < y1; y++) {
		const uint32_t *p = (const uint32_t *)(fb + (size_t)y * pitch);

		for (uint32_t x = 0; x < w; x++) {
			if (p[x] != rpi5_pattern_at(order, w, h, x, y - y0)) {
				if (bad == 0u && first_bad) {
					*first_bad = (y - y0) * w + x;
				}
				bad++;
			}
		}
	}
	return bad;
}
