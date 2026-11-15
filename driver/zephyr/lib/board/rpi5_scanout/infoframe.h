/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * CEA-861 AVI InfoFrame builder for the Pi 5 HDMI mode (route 3 after DVI).
 * Header-only C11, pure functions: no register is written and no HDMI RAM
 * packet register offset appears here.
 *
 * Layout source (public specification, no driver code used):
 *   CEA-861-D/E "AVI InfoFrame" and HDMI 1.4 section 8.2.3, as reproduced in
 *   the open CTA/HDMI summaries. 17 bytes on the wire:
 *     [0] type 0x82   [1] version 2   [2] length 13   [3] checksum
 *     [4] PB1: Y1Y0 (7:6) 0 = RGB, A0 (4), B1B0 (3:2), S1S0 (1:0)
 *     [5] PB2: C1C0 (7:6), M1M0 (5:4) 1 = 4:3, 2 = 16:9, R3..R0 (3:0)
 *     [6] PB3: ITC (7), EC2..0 (6:4), Q1Q0 (3:2) 0 default, 1 limited,
 *              2 full, SC1SC0 (1:0)
 *     [7] PB4: VIC (7 bits)
 *     [8] PB5: YQ1YQ0 (7:6), CN1CN0 (5:4), PR3..0 (3:0)
 *     [9..16] PB6..PB13: bar info, zero here
 *   The checksum byte makes the sum of all 17 bytes 0 mod 256.
 *
 * The packed form is the bytes in order, little-endian into 32-bit words
 * (byte 0 in bits 7:0 of word 0); 17 bytes make 5 words, the last one
 * zero-padded. Which register takes each word is not decided here.
 */
#ifndef YAKOGL_RPI5_SCANOUT_INFOFRAME_H
#define YAKOGL_RPI5_SCANOUT_INFOFRAME_H

#include <stddef.h>
#include <stdint.h>

#define AVI_IF_TYPE 0x82u
#define AVI_IF_VERSION 2u
#define AVI_IF_LEN 13u
#define AVI_IF_BYTES (4u + AVI_IF_LEN)
#define AVI_IF_WORDS 5u

enum avi_colourspace { AVI_CS_RGB = 0 };
enum avi_quant { AVI_Q_DEFAULT = 0, AVI_Q_LIMITED = 1, AVI_Q_FULL = 2 };
enum avi_aspect { AVI_ASPECT_NONE = 0, AVI_ASPECT_4_3 = 1, AVI_ASPECT_16_9 = 2 };

/* 0 on success, -1 on a bad argument (VIC 0 or > 127, unknown enum value). */
static inline int avi_infoframe_build(unsigned vic, enum avi_colourspace cs,
				      enum avi_quant q, enum avi_aspect aspect,
				      uint8_t out[AVI_IF_BYTES])
{
	unsigned i;
	uint8_t sum = 0;

	if (out == NULL || vic == 0 || vic > 127 || cs != AVI_CS_RGB ||
	    (unsigned)q > AVI_Q_FULL || (unsigned)aspect > AVI_ASPECT_16_9) {
		return -1;
	}
	for (i = 0; i < AVI_IF_BYTES; i++) {
		out[i] = 0;
	}
	out[0] = AVI_IF_TYPE;
	out[1] = AVI_IF_VERSION;
	out[2] = AVI_IF_LEN;
	out[4] = (uint8_t)((unsigned)cs << 6);
	out[5] = (uint8_t)((unsigned)aspect << 4);
	out[6] = (uint8_t)((unsigned)q << 2);
	out[7] = (uint8_t)vic;
	for (i = 0; i < AVI_IF_BYTES; i++) {
		sum = (uint8_t)(sum + out[i]);
	}
	out[3] = (uint8_t)(0x100u - sum);
	return 0;
}

/* 0 when the bytes are a well-formed AVI InfoFrame, -1 otherwise. */
static inline int avi_infoframe_check(const uint8_t *p, size_t len)
{
	uint8_t sum = 0;
	size_t i;

	if (p == NULL || len != AVI_IF_BYTES || p[0] != AVI_IF_TYPE ||
	    p[1] != AVI_IF_VERSION || p[2] != AVI_IF_LEN) {
		return -1;
	}
	for (i = 0; i < len; i++) {
		sum = (uint8_t)(sum + p[i]);
	}
	return sum == 0 ? 0 : -1;
}

/* Pack the 17 bytes into 5 little-endian 32-bit words (tail zero-padded). */
static inline void avi_infoframe_pack(const uint8_t in[AVI_IF_BYTES],
				      uint32_t w[AVI_IF_WORDS])
{
	unsigned i;

	for (i = 0; i < AVI_IF_WORDS; i++) {
		w[i] = 0;
	}
	for (i = 0; i < AVI_IF_BYTES; i++) {
		w[i / 4] |= (uint32_t)in[i] << (8 * (i % 4));
	}
}

#endif /* YAKOGL_RPI5_SCANOUT_INFOFRAME_H */
