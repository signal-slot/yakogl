/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fw_info: which board this is, as the firmware answers it. Header only: the
 * property mailbox request asking GET_BOARD_MODEL and GET_BOARD_REVISION in
 * one buffer, the check of the answer and the decode of the 32-bit board
 * revision code. A Zephyr kernel= image has no vcgencmd; every record of the
 * Pi 5 display session names its board with this line, next to the
 * get_throttled line of throttled.h.
 *
 * The tags (rpi:include/soc/bcm2835/raspberrypi-firmware.h at rpi-6.12.y
 * 5bd564aa, the source table of driver/docs/rpi5-display.md):
 *   GET_BOARD_MODEL    0x00010001, :45, a string in a 16-byte value buffer
 *   GET_BOARD_REVISION 0x00010002, :46, the revision code in 4 bytes
 * The request never asks GET_BOARD_MAC_ADDRESS 0x00010003 (:47) or
 * GET_BOARD_SERIAL 0x00010004 (:48): they name this one board, and records
 * are published; identifiers, like addresses and credentials, stay in the
 * environment (CLAUDE.md). The mock checks that neither id appears in the
 * request.
 *
 * The buffer layout is the one rpi4/rpi_mbox.c builds for a multi-tag
 * request (rpi_fb_alloc, tag_add :235-244): size in bytes, request code,
 * then per tag the tag id, the value buffer bytes, the tag request code and
 * the value words, then the 0 end tag. Two tags: 14 words, model first. The
 * caller sends it (16-byte aligned, below 4 GiB) with its own transport, as
 * in throttled.h; value words are native little-endian, as rpi_mbox.c's
 * memcpy leaves them. Response code 0x80000000 and the tag response bit 31:
 * raspberrypi-firmware.h:15-16 and facts section 9.
 *
 * The revision code layout is Raspberry Pi documentation, "Raspberry Pi
 * revision codes" of raspberrypi.com/documentation/computers/raspberry-pi.html
 * (read 2026-11-15), new style NOQuuuWuFMMMCCCCPPPPTTTTTTTTRRRR:
 *   0-3   R revision    4-11 T type      12-15 P processor (4: BCM2712)
 *   16-19 C manufacturer (0: Sony UK, 5: Stadium)
 *   20-22 M memory size (0 256 MB, 1 512 MB, 2 1 GB, 3 2 GB, 4 4 GB,
 *       5 8 GB, 6 16 GB, 7 other)
 *   23    F new-style flag (1: the code is new style)
 *   24    unused (older Raspberry Pi documentation put the warranty flag of
 *       the old-style codes here; a new-style code leaves it unused, and the
 *       firmware of a BCM2712 answers new-style codes only - unverified)
 *   25    W warranty void (1: voided by overclocking)
 *   26-28 unused
 *   29    Q OTP read disallowed        30 O OTP program disallowed
 *   31    N overvoltage disallowed
 * So the warranty flag of the new-style codes this board answers is bit 25,
 * not bit 24. The board revision of a Pi 5 8 GB, warranty voided, is
 * 0x2d04170: new style, type 0x17 (Pi 5), processor 4, manufacturer 0,
 * memory 5.
 *
 * C11, no Zephyr includes; only stdint.h, stdio.h and static inline.
 */
#ifndef YAKOGL_RPI5_SCANOUT_FW_INFO_H
#define YAKOGL_RPI5_SCANOUT_FW_INFO_H

#include <stdint.h>
#include <stdio.h>

#define FW_TAG_BOARD_MODEL 0x00010001u /* raspberrypi-firmware.h:45 */
#define FW_TAG_BOARD_REVISION 0x00010002u /* :46 */
#define FW_TAG_BOARD_MAC 0x00010003u /* :47; never sent in a request */
#define FW_TAG_BOARD_SERIAL 0x00010004u /* :48; never sent in a request */
#define FW_CODE_REQUEST 0x00000000u /* raspberrypi-firmware.h:15 */
#define FW_CODE_SUCCESS 0x80000000u /* :16 */
#define FW_TAG_RESPONSE 0x80000000u /* tag code bit 31: the tag answered */

#define FW_MODEL_BYTES 16u /* GET_BOARD_MODEL value buffer bytes */
#define FW_WORDS 14u /* buffer size in 32-bit words */

/* results of fw_info_check(), all negative */
#define FW_ERR_ARG (-1) /* NULL pointer or too few words */
#define FW_ERR_SIZE (-2) /* word 0 is not the request's size */
#define FW_ERR_CODE (-3) /* word 1 is not 0x80000000 */
#define FW_ERR_MODEL_TAG (-4) /* word 2 is not the model tag */
#define FW_ERR_MODEL_RESP (-5) /* model tag response bit missing */
#define FW_ERR_MODEL_LEN (-6) /* model response length 0 or over 16 */
#define FW_ERR_REV_TAG (-7) /* word 9 is not the revision tag */
#define FW_ERR_REV_RESP (-8) /* revision tag response bit missing */
#define FW_ERR_REV_LEN (-9) /* revision response length below 4 */

/* One decoded board revision code. The flag fields are the raw bits. */
struct fw_board_rev {
	uint32_t code;
	unsigned revision; /* bits 0-3 */
	unsigned type; /* bits 4-11 */
	unsigned processor; /* bits 12-15 */
	unsigned manufacturer; /* bits 16-19 */
	unsigned mem; /* bits 20-22 */
	int new_style; /* bit 23, 1: new-style code */
	int warranty; /* bit 25, 1: warranty voided */
	int otp_read; /* bit 29, 1: OTP reading disallowed */
	int otp_program; /* bit 30, 1: OTP programming disallowed */
	int overvoltage; /* bit 31, 1: overvoltage disallowed */
};

/* The answer: the board model (NUL-terminated, 16 chars at the very most,
 * truncated when the firmware leaves no NUL in its buffer) and the decoded
 * revision code. */
struct fw_info {
	char model[FW_MODEL_BYTES + 1];
	struct fw_board_rev rev;
};

/* Write the request into buf[0..words). Returns the words used (FW_WORDS),
 * 0 if buf is NULL or words is too small. */
static inline uint32_t fw_info_request(uint32_t *buf, uint32_t words)
{
	if (!buf || words < FW_WORDS)
		return 0;
	buf[0] = FW_WORDS * 4u;
	buf[1] = FW_CODE_REQUEST;
	buf[2] = FW_TAG_BOARD_MODEL;
	buf[3] = FW_MODEL_BYTES;
	buf[4] = 0u; /* tag request code */
	buf[5] = 0u; /* the 16-byte string buffer */
	buf[6] = 0u;
	buf[7] = 0u;
	buf[8] = 0u;
	buf[9] = FW_TAG_BOARD_REVISION;
	buf[10] = 4u;
	buf[11] = 0u; /* tag request code */
	buf[12] = 0u; /* the value */
	buf[13] = 0u; /* end tag */
	return FW_WORDS;
}

static inline void fw_board_rev_decode(uint32_t code, struct fw_board_rev *r)
{
	r->code = code;
	r->revision = code & 0xfu;
	r->type = (code >> 4) & 0xffu;
	r->processor = (code >> 12) & 0xfu;
	r->manufacturer = (code >> 16) & 0xfu;
	r->mem = (code >> 20) & 7u;
	r->new_style = (code >> 23) & 1u;
	r->warranty = (code >> 25) & 1u;
	r->otp_read = (code >> 29) & 1u;
	r->otp_program = (code >> 30) & 1u;
	r->overvoltage = (code >> 31) & 1u;
}

/* Check the answer in buf (the request buffer after the firmware wrote it)
 * and decode it into *out. Returns 0, or a FW_ERR_*; *out is only written
 * on 0. */
static inline int fw_info_check(const uint32_t *buf, uint32_t words,
				struct fw_info *out)
{
	struct fw_info f;
	uint32_t len;

	if (!buf || !out || words < FW_WORDS)
		return FW_ERR_ARG;
	if (buf[0] != FW_WORDS * 4u)
		return FW_ERR_SIZE;
	if (buf[1] != FW_CODE_SUCCESS)
		return FW_ERR_CODE;
	if (buf[2] != FW_TAG_BOARD_MODEL)
		return FW_ERR_MODEL_TAG;
	if (!(buf[4] & FW_TAG_RESPONSE))
		return FW_ERR_MODEL_RESP;
	len = buf[4] & ~FW_TAG_RESPONSE;
	if (len == 0u || len > FW_MODEL_BYTES)
		return FW_ERR_MODEL_LEN;
	for (unsigned i = 0; i < FW_MODEL_BYTES; i++) {
		uint8_t c = (i < len) ?
			(uint8_t)((buf[5u + i / 4u] >> (8u * (i % 4u))) & 0xffu) :
			0u;
		/* truncate, never run past model[16]: the firmware may fill
		 * all 16 bytes without a NUL */
		f.model[i] = (char)c;
	}
	f.model[FW_MODEL_BYTES] = 0;
	if (buf[9] != FW_TAG_BOARD_REVISION)
		return FW_ERR_REV_TAG;
	if (!(buf[11] & FW_TAG_RESPONSE))
		return FW_ERR_REV_RESP;
	if ((buf[11] & ~FW_TAG_RESPONSE) < 4u)
		return FW_ERR_REV_LEN;
	fw_board_rev_decode(buf[12], &f.rev);
	*out = f;
	return 0;
}

/* The greppable line; returns snprintf's value. */
static inline int fw_info_format(char *out, unsigned long n,
				 const struct fw_info *i)
{
	return snprintf(out, n,
			"model=%s rev=0x%08x new=%d warranty=%d type=0x%02x "
			"proc=%u manuf=%u mem=%u overvolt=%d",
			i->model, (unsigned)i->rev.code, i->rev.new_style,
			i->rev.warranty, i->rev.type, i->rev.processor,
			i->rev.manufacturer, i->rev.mem, i->rev.overvoltage);
}

#endif /* YAKOGL_RPI5_SCANOUT_FW_INFO_H */
