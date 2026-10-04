/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The VideoCore firmware's property mailbox on the BCM2711 (Raspberry Pi 4):
 * the one channel through which the ARM asks the firmware for a framebuffer
 * and switches the V3D clock.
 *
 * Facts from Linux v6.17 (cited, not transcribed): the mailbox registers
 * (drivers/mailbox/bcm2835-mailbox.c:32-50, base bcm283x.dtsi:100), the
 * property buffer format and channel 8 (drivers/firmware/raspberrypi.c:23,
 * 100-131) and the tag numbers (include/soc/bcm2835/raspberrypi-firmware.h).
 */
#ifndef RPI_MBOX_H
#define RPI_MBOX_H

#include <stdint.h>
#include <stdbool.h>

#define RPI_FW_CLK_V3D 5 /* raspberrypi-firmware.h:144, bcm2711-rpi.dtsi:96 */

/* Map the mailbox. Returns 0 or a negative errno. */
int rpi_mbox_init(void);

/*
 * Send one property tag and wait for the answer. `val` holds `words` request
 * words and receives as many response words. Returns 0, -EIO if the firmware
 * answered with an error or did not mark the tag as answered, or -ETIMEDOUT.
 */
int rpi_mbox_tag(uint32_t tag, uint32_t *val, uint32_t words);

/* The CPU mapping rpi_fb_alloc gives the framebuffer, named for the logs. */
#define RPI_FB_MAPPING "normal NC"

struct rpi_fb {
	uint8_t *cpu;          /* CPU mapping, Normal non-cacheable */
	uint32_t phys;         /* ARM physical address */
	uint32_t width, height, pitch, size;
	/* SET_PIXEL_ORDER's answer as the firmware gave it: the tag's response
	 * code word (bit 31: answered, bits 30..0: value bytes) and the order it
	 * set (0 BGR, 1 RGB). Interpreted by the reader (the board layer's
	 * yd_rpi4_window_format), not here. */
	uint32_t order_code, order;
};

/* Ask the firmware for a 32 bpp RGB framebuffer on the HDMI output. */
int rpi_fb_alloc(uint32_t width, uint32_t height, struct rpi_fb *fb);

/*
 * One request of caller-built tags, timed. `tags` holds `words` words: each
 * tag as tag id, value buffer bytes, request code 0, then its value buffer
 * (the layout of raspberrypi.c:100-118 without the request header and the end
 * tag, which this adds). On return `tags` holds the firmware's answer; the
 * caller checks each tag's response bit (bit 31 of its code word). The
 * transaction is split in cycles of k_cycle_get_64: `write` (the buffer's
 * flush, the wait for room and the write), `poll` (until the firmware's
 * answer is in MAIL0: the firmware's own time), `inval` (the buffer's
 * invalidate). Returns 0, -EIO for an error answer, -EINVAL, or -ETIMEDOUT.
 */
struct rpi_mbox_timing {
	uint64_t write, poll, inval;
};
int rpi_mbox_request_timed(uint32_t *tags, uint32_t words, struct rpi_mbox_timing *t);

/*
 * The flip: a framebuffer twice as tall as the screen, shown half at a time
 * by moving the firmware's virtual offset (driver/samples/zephyr/v3d_fb_flip
 * measured it on the Pi 4, 2026-09-25: the firmware grants 1920x1440, a pan
 * costs about 90 us, SET_VSYNC waits about 32 ms, and a pan list that repeats
 * the size tags resets the offset to 0,0 and does not pan).
 *
 * rpi_fb_alloc_virtual: one request (physical w x h, virtual w x virt_h,
 * depth 32, pixel order RGB, virtual offset 0,0, allocate 4096, pitch); every
 * answer is checked. 0 and `fb` as rpi_fb_alloc's (fb->height the physical
 * height; `fb->size` covers the virtual height), or -ERANGE when the firmware
 * granted another physical or virtual size or depth (`got` then holds the
 * granted physical w, h and virtual w, h), -EIO when a tag was not answered
 * or no buffer was allocated, -ENOMEM when the mapping failed.
 */
int rpi_fb_alloc_virtual(uint32_t w, uint32_t h, uint32_t virt_h, struct rpi_fb *fb, uint32_t got[4]);

/* SET_VIRTUAL_OFFSET 0,y alone (never with the size tags). 0 when the
 * firmware answered; *echo_y (may be NULL) is the row it answered with, which
 * is the row now shown when it equals y. `t` (may be NULL) receives the
 * transaction's split. */
int rpi_fb_pan(uint32_t y, uint32_t *echo_y, struct rpi_mbox_timing *t);

/* GET_VIRTUAL_OFFSET: the offset the firmware holds now. */
int rpi_fb_get_offset(uint32_t *x, uint32_t *y);

/* SET_VSYNC: returns at the display's next vsync (about 32 ms apart on the
 * Pi 4 as measured). `t` (may be NULL) receives the split. */
int rpi_fb_wait_vsync(struct rpi_mbox_timing *t);

/* What the flip calls cost since boot, in k_cycle_get_64 cycles: pans (and
 * those that failed or whose answer named another row) and vsync waits,
 * their summed and longest transactions. A report prints differences. */
struct rpi_fb_counters {
	uint32_t pans, pans_failed, pans_not_echoed;
	uint64_t pan_cycles, pan_max_cycles;
	uint32_t vsyncs, vsyncs_failed;
	uint64_t vsync_cycles, vsync_max_cycles;
};
void rpi_fb_counters_get(struct rpi_fb_counters *out);

int rpi_clock_state(uint32_t id, bool on);
int rpi_clock_rate(uint32_t id, uint32_t *hz);     /* current rate */
int rpi_clock_max_rate(uint32_t id, uint32_t *hz);
int rpi_clock_set_rate(uint32_t id, uint32_t hz);

#endif
