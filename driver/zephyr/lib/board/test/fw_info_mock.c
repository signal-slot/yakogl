/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host mock of rpi5_scanout/fw_info.h: the request byte for byte and without
 * the serial and MAC tags, good and bad answers, the model truncation, the
 * revision decode (warranty is bit 25 of a new-style code) and the greppable
 * line. Built by tools/v3d/check71.d/fw_info.sh.
 */
#include <stdio.h>
#include <string.h>

#include "fw_info.h"

static int n_checks, n_failed;

#define CHECK(cond, what)                                                     \
	do {                                                                  \
		n_checks++;                                                   \
		if (!(cond)) {                                                \
			n_failed++;                                           \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, what); \
		}                                                             \
	} while (0)

/* answer both tags: the model as len bytes (1..16, NUL included when the
 * caller passes it) and the revision code */
static void answer(uint32_t *b, const char *model, uint32_t len, uint32_t rev)
{
	fw_info_request(b, FW_WORDS);
	b[1] = FW_CODE_SUCCESS;
	b[4] = FW_TAG_RESPONSE | len;
	for (uint32_t i = 0; i < len; i++) {
		uint32_t *w = &b[5u + i / 4u];
		uint32_t sh = 8u * (i % 4u);

		*w = (*w & ~(0xffu << sh)) | ((uint32_t)(uint8_t)model[i] << sh);
	}
	b[11] = FW_TAG_RESPONSE | 4u;
	b[12] = rev;
}

int main(void)
{
	uint32_t b[FW_WORDS + 2];
	struct fw_info i, z;
	struct fw_board_rev r;
	char out[200];

	/* request, byte for byte */
	memset(b, 0xa5, sizeof(b));
	CHECK(fw_info_request(b, FW_WORDS + 2) == FW_WORDS, "request words");
	static const uint32_t want[FW_WORDS] = {
		56, 0, 0x00010001, 16, 0, 0, 0, 0, 0, 0x00010002, 4, 0, 0, 0
	};
	CHECK(memcmp(b, want, sizeof(want)) == 0, "request bytes");
	CHECK(b[FW_WORDS] == 0xa5a5a5a5u && b[FW_WORDS + 1] == 0xa5a5a5a5u,
	      "request writes only 14 words");
	CHECK(fw_info_request(b, FW_WORDS - 1) == 0, "request: buffer too small");
	CHECK(fw_info_request(NULL, FW_WORDS) == 0, "request: NULL");

	/* the per-board ids are never in the request */
	fw_info_request(b, FW_WORDS);
	for (unsigned k = 0; k < FW_WORDS; k++)
		CHECK(b[k] != FW_TAG_BOARD_SERIAL, "no serial tag in the request");
	for (unsigned k = 0; k < FW_WORDS; k++)
		CHECK(b[k] != FW_TAG_BOARD_MAC, "no MAC tag in the request");

	/* a good answer: the Pi 5 8 GB, warranty voided */
	memset(&z, 0x77, sizeof(z));
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	CHECK(fw_info_check(b, FW_WORDS, &i) == 0, "good answer");
	CHECK(strcmp(i.model, "Raspberry Pi 5") == 0, "model");
	CHECK(i.rev.code == 0x2d04170, "code");
	CHECK(i.rev.new_style == 1 && i.rev.warranty == 1, "new style, warranty");
	CHECK(i.rev.type == 0x17 && i.rev.processor == 4, "type, processor");
	CHECK(i.rev.manufacturer == 0 && i.rev.mem == 5 && i.rev.revision == 0,
	      "manuf, mem, revision");
	CHECK(i.rev.otp_read == 0 && i.rev.otp_program == 0 &&
		      i.rev.overvoltage == 0,
	      "bits 29-31 clear");

	/* a one-word answer */
	answer(b, "X", 2, 0);
	CHECK(fw_info_check(b, FW_WORDS, &i) == 0 && strcmp(i.model, "X") == 0,
	      "model one char");
	CHECK(i.rev.code == 0, "code 0");

	/* model bounds: 16 bytes without a NUL truncate, never overrun */
	answer(b, "Raspberry Pi 400", 16, 0);
	CHECK(fw_info_check(b, FW_WORDS, &i) == 0, "model 16 bytes, no NUL");
	CHECK(strlen(i.model) == 16 && i.model[16] == 0, "model truncated at 16");
	answer(b, "Raspberry Pi Model B+", 16, 0);
	CHECK(fw_info_check(b, FW_WORDS, &i) == 0 &&
		      strcmp(i.model, "Raspberry Pi Mod") == 0,
	      "long model truncated to the buffer");

	/* model length: 0 and over 16 are refused, 16 taken */
	answer(b, "Raspberry Pi 5", 15, 0);
	b[4] = FW_TAG_RESPONSE;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_MODEL_LEN, "model length 0");
	b[4] = FW_TAG_RESPONSE | 17u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_MODEL_LEN, "model length 17");
	b[4] = FW_TAG_RESPONSE | 16u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == 0, "model length 16 accepted");

	/* revision length: below 4 refused, more taken (as throttled.h) */
	answer(b, "X", 2, 0x2d04170);
	b[11] = FW_TAG_RESPONSE | 3u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_REV_LEN, "rev length 3");
	b[11] = FW_TAG_RESPONSE | 8u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == 0, "rev length 8 accepted");

	/* bad answers; *out untouched */
	memset(&z, 0x77, sizeof(z));
	memset(&i, 0x77, sizeof(i));
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[0] = 52;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_SIZE, "wrong size");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[1] = FW_CODE_REQUEST;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_CODE, "request code left");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[1] = 0x80000001u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_CODE, "error code");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[2] = FW_TAG_BOARD_REVISION;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_MODEL_TAG, "model tag");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[4] = 15u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_MODEL_RESP,
	      "model response bit missing");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[9] = FW_TAG_BOARD_MODEL;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_REV_TAG, "rev tag");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	b[11] = 4u;
	CHECK(fw_info_check(b, FW_WORDS, &i) == FW_ERR_REV_RESP,
	      "rev response bit missing");
	CHECK(fw_info_check(b, FW_WORDS - 1, &i) == FW_ERR_ARG, "too few words");
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	CHECK(fw_info_check(NULL, FW_WORDS, &i) == FW_ERR_ARG, "NULL buffer");
	CHECK(fw_info_check(b, FW_WORDS, NULL) == FW_ERR_ARG, "NULL out");
	CHECK(memcmp(&i, &z, sizeof(z)) == 0, "out untouched after errors");

	/* decodes */
	fw_board_rev_decode(0, &r);
	CHECK(r.code == 0 && r.revision == 0 && r.type == 0 && r.processor == 0,
	      "zero code");
	CHECK(r.manufacturer == 0 && r.mem == 0 && r.new_style == 0 &&
		      r.warranty == 0 && r.otp_read == 0 && r.otp_program == 0 &&
		      r.overvoltage == 0,
	      "zero code, flags");
	fw_board_rev_decode(0xd04170, &r); /* the same board, warranty intact */
	CHECK(r.new_style == 1 && r.warranty == 0 && r.type == 0x17 &&
		      r.processor == 4 && r.mem == 5,
	      "0xd04170");
	fw_board_rev_decode(0x2d04170, &r); /* warranty bit 25 */
	CHECK(r.warranty == 1 && r.new_style == 1, "0x2d04170 warranty");
	CHECK(r.type == 0x17 && r.processor == 4 && r.mem == 5,
	      "0x2d04170 fields");
	fw_board_rev_decode(0x1d04170, &r); /* bit 24 only: unused, not warranty */
	CHECK(r.warranty == 0, "0x1d04170 bit 24 is not the warranty flag");
	CHECK(r.new_style == 1, "0x1d04170 new style");
	fw_board_rev_decode(0xffffffffu, &r);
	CHECK(r.revision == 15 && r.type == 255 && r.processor == 15 &&
		      r.manufacturer == 15 && r.mem == 7,
	      "all ones, fields");
	CHECK(r.new_style == 1 && r.warranty == 1 && r.otp_read == 1 &&
		      r.otp_program == 1 && r.overvoltage == 1,
	      "all ones, flags");

	/* the line, and snprintf truncation safety */
	answer(b, "Raspberry Pi 5", 15, 0x2d04170);
	fw_info_check(b, FW_WORDS, &i);
	int n = fw_info_format(out, sizeof(out), &i);
	static const char *line =
		"model=Raspberry Pi 5 rev=0x02d04170 new=1 warranty=1 type=0x17 "
		"proc=4 manuf=0 mem=5 overvolt=0";
	CHECK(n == (int)strlen(line) && strcmp(out, line) == 0, line);
	memset(out, 'x', sizeof(out));
	CHECK(fw_info_format(out, 10, &i) > 10 && out[9] == 0 && out[10] == 'x',
	      "line truncates");

	printf("fw_info_mock: %d of %d checks, %d failed\n", n_checks - n_failed,
	       n_checks, n_failed);
	return n_failed ? 1 : 0;
}
