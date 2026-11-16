/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Raspberry Pi 5's screen over the firmware's property mailbox (route 2
 * of driver/docs/rpi5-display.md): what the firmware reports about a
 * framebuffer before we ask for one, and a known pattern written to and read
 * back from the framebuffer rpi_fb_alloc() (rpi_mbox.c) gets. The Pi 4's
 * calls are used unchanged; the BCM2712 differs only in the mailbox's base
 * and the bus view (no 0xc0000000 alias), which rpi_mbox.c already takes from
 * CONFIG_SOC_BCM2712. Nothing here ran on a board yet.
 */
#ifndef RPI5_SCREEN_H
#define RPI5_SCREEN_H

#include <stdint.h>
#include "rpi_mbox.h"

/* The GET tags asked before an allocation (raspberrypi-firmware.h), one
 * request each so that an unknown tag costs one answer, not the others. */
enum rpi5_get {
	RPI5_GET_PHYSICAL, /* 0x00040003: width, height */
	RPI5_GET_VIRTUAL,  /* 0x00040004: width, height */
	RPI5_GET_DEPTH,    /* 0x00040005 */
	RPI5_GET_ORDER,    /* 0x00040006: 0 BGR, 1 RGB */
	RPI5_GET_PITCH,    /* 0x00040008 */
	RPI5_GET_OFFSET,   /* 0x00040009: x, y */
	RPI5_GET_COUNT
};

struct rpi5_fw_fb {
	/* per tag: the request's return code (0, -EIO, -ETIMEDOUT, ...), the
	 * tag's response code word with bit 31 (answered) kept only when the
	 * request succeeded, and the answer's words */
	int rc[RPI5_GET_COUNT];
	uint32_t code[RPI5_GET_COUNT];
	uint32_t val[RPI5_GET_COUNT][2];
};

/* Ask every GET tag. Returns how many the firmware answered. */
int rpi5_screen_query(struct rpi5_fw_fb *s);

/* 1 when the firmware answered the physical size with both sides non-zero:
 * a framebuffer is already set up (or its mode is). */
int rpi5_screen_query_has_fb(const struct rpi5_fw_fb *s);

/* The pixel word for (r, g, b) in the firmware's pixel order (0 BGR, 1 RGB):
 * RGB puts R in the first byte of the pixel, BGR puts B there. */
uint32_t rpi5_pixel(uint32_t order, uint8_t r, uint8_t g, uint8_t b);

/* The pattern's word at (x, y) of a w x h area: eight vertical colour bars
 * and, in the bottom eighth, a horizontal ramp of the red channel, so a
 * wrong order, pitch or width shows in the bytes. */
uint32_t rpi5_pattern_at(uint32_t order, uint32_t w, uint32_t h, uint32_t x, uint32_t y);

/* Write rows [y0, y1) of the framebuffer with the pattern (the pattern's own
 * y runs from 0 at row y0). */
void rpi5_pattern_fill(uint8_t *fb, uint32_t pitch, uint32_t order, uint32_t w, uint32_t y0, uint32_t y1);

/* Read the same rows back and compare. Returns the number of wrong pixels;
 * *first_bad (may be NULL) is the pixel index (row * w + x, rows from y0) of
 * the first one, or UINT32_MAX. */
uint32_t rpi5_pattern_verify(const uint8_t *fb, uint32_t pitch, uint32_t order, uint32_t w, uint32_t y0,
			     uint32_t y1, uint32_t *first_bad);

#endif
