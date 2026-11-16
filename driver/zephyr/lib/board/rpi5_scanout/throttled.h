/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * GET_THROTTLED through the property mailbox, header only: the request for
 * tag 0x00030046 written into a caller buffer, the check of the answer and the
 * decode of the 32-bit value. A Zephyr kernel= image has no vcgencmd; every
 * record of the Pi 5 display session carries this value (weak supply).
 *
 * The buffer layout is the one rpi4/rpi_mbox.c rpi_mbox_tag() builds (and
 * raspberrypi.c:100-118 in rpi-6.12.y): size in bytes, request code, tag, value
 * buffer bytes, tag request code, the value, the 0 end tag. Seven words.
 * The caller sends it (16-byte aligned, below 4 GiB) with its own transport.
 *
 * Tag id: RPI_FIRMWARE_GET_THROTTLED = 0x00030046, include/soc/bcm2835/
 * raspberrypi-firmware.h:78 in rpi-6.12.y (the line before GET_CLOCK_MEASURED
 * 0x00030047 at :79, which driver/docs/rpi5-v3d71-facts.md section 9 cites).
 * The enum entry was read in the rpi-6.12.y file; :78 is counted by hand.
 * Response code 0x80000000 (raspberrypi-firmware.h:16) and the tag's response
 * bit 31, low 31 bits = the value length (facts section 9).
 *
 * Bits of the value (vcgencmd get_throttled), Raspberry Pi documentation,
 * "vcgencmd" section of raspberrypi.com/documentation/computers/os.html,
 * command get_throttled:
 *   0 under-voltage detected      16 under-voltage has occurred
 *   1 ARM frequency capped        17 ARM frequency capping has occurred
 *   2 currently throttled         18 throttling has occurred
 *   3 soft temperature limit      19 soft temperature limit has occurred
 * Other bits are kept in `raw` and not named.
 *
 * C11, no Zephyr includes; only uint32_t, snprintf and static inline.
 */
#ifndef YAKOGL_RPI5_SCANOUT_THROTTLED_H
#define YAKOGL_RPI5_SCANOUT_THROTTLED_H

#include <stdint.h>
#include <stdio.h>

#define THR_TAG_GET_THROTTLED 0x00030046u
#define THR_CODE_REQUEST      0x00000000u
#define THR_CODE_SUCCESS      0x80000000u
#define THR_TAG_RESPONSE      0x80000000u
#define THR_WORDS             7u /* buffer size in 32-bit words */

#define THR_UNDER_VOLTAGE   (1u << 0)
#define THR_FREQ_CAPPED     (1u << 1)
#define THR_THROTTLED       (1u << 2)
#define THR_SOFT_TEMP_LIMIT (1u << 3)
#define THR_SEEN_SHIFT     16

/* results of thr_check(), all negative */
#define THR_ERR_ARG    (-1) /* NULL pointer or too few words */
#define THR_ERR_SIZE   (-2) /* word 0 is not the request's size */
#define THR_ERR_CODE   (-3) /* word 1 is not 0x80000000 */
#define THR_ERR_TAG    (-4) /* word 2 is not the tag */
#define THR_ERR_RESP   (-5) /* tag response bit 31 missing */
#define THR_ERR_LENGTH (-6) /* response length below 4 bytes */

struct thr_flags {
	uint32_t raw;
	int uv, cap, thr, soft;                     /* bits 0-3 */
	int uv_seen, cap_seen, thr_seen, soft_seen; /* bits 16-19 */
};

/* Write the request into buf[0..words). Returns the words used (THR_WORDS),
 * 0 if buf is NULL or words is too small. */
static inline uint32_t thr_request(uint32_t *buf, uint32_t words)
{
	if (!buf || words < THR_WORDS)
		return 0;
	buf[0] = THR_WORDS * 4u;
	buf[1] = THR_CODE_REQUEST;
	buf[2] = THR_TAG_GET_THROTTLED;
	buf[3] = 4u; /* value buffer bytes */
	buf[4] = 0u; /* tag request code */
	buf[5] = 0u; /* the value */
	buf[6] = 0u; /* end tag */
	return THR_WORDS;
}

static inline void thr_decode(uint32_t v, struct thr_flags *f)
{
	f->raw = v;
	f->uv = (v & THR_UNDER_VOLTAGE) != 0;
	f->cap = (v & THR_FREQ_CAPPED) != 0;
	f->thr = (v & THR_THROTTLED) != 0;
	f->soft = (v & THR_SOFT_TEMP_LIMIT) != 0;
	f->uv_seen = (v & (THR_UNDER_VOLTAGE << THR_SEEN_SHIFT)) != 0;
	f->cap_seen = (v & (THR_FREQ_CAPPED << THR_SEEN_SHIFT)) != 0;
	f->thr_seen = (v & (THR_THROTTLED << THR_SEEN_SHIFT)) != 0;
	f->soft_seen = (v & (THR_SOFT_TEMP_LIMIT << THR_SEEN_SHIFT)) != 0;
}

/* Check the answer in buf (the request buffer after the firmware wrote it) and
 * decode the value. Returns 0, or a THR_ERR_*; *f is only written on 0. */
static inline int thr_check(const uint32_t *buf, uint32_t words, struct thr_flags *f)
{
	if (!buf || !f || words < THR_WORDS)
		return THR_ERR_ARG;
	if (buf[0] != THR_WORDS * 4u)
		return THR_ERR_SIZE;
	if (buf[1] != THR_CODE_SUCCESS)
		return THR_ERR_CODE;
	if (buf[2] != THR_TAG_GET_THROTTLED)
		return THR_ERR_TAG;
	if (!(buf[4] & THR_TAG_RESPONSE))
		return THR_ERR_RESP;
	if ((buf[4] & ~THR_TAG_RESPONSE) < 4u)
		return THR_ERR_LENGTH;
	thr_decode(buf[5], f);
	return 0;
}

/* The greppable line; returns snprintf's value. */
static inline int thr_format(char *out, unsigned long n, const struct thr_flags *f)
{
	return snprintf(out, n,
			"throttled=0x%08x uv=%d cap=%d thr=%d soft=%d uv_seen=%d cap_seen=%d "
			"thr_seen=%d soft_seen=%d",
			(unsigned)f->raw, f->uv, f->cap, f->thr, f->soft, f->uv_seen,
			f->cap_seen, f->thr_seen, f->soft_seen);
}

#endif /* YAKOGL_RPI5_SCANOUT_THROTTLED_H */
