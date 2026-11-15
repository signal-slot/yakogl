/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host mock of rpi5_scanout/throttled.h: the request byte for byte, good and
 * bad answers, the decode of the flag bits and the greppable line.
 * Built by tools/v3d/check71.d/throttled.sh.
 */
#include <stdio.h>
#include <string.h>

#include "throttled.h"

static int n_checks, n_failed;

#define CHECK(cond, what)                                                          \
	do {                                                                       \
		n_checks++;                                                        \
		if (!(cond)) {                                                     \
			n_failed++;                                                \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, what);      \
		}                                                                  \
	} while (0)

/* a good answer carrying v */
static void answer(uint32_t *b, uint32_t v)
{
	thr_request(b, THR_WORDS);
	b[1] = 0x80000000u;
	b[4] = 0x80000004u;
	b[5] = v;
}

static void decode_case(uint32_t v, const int e[8], const char *line)
{
	struct thr_flags f;
	char out[160];

	thr_decode(v, &f);
	CHECK(f.raw == v, "raw");
	CHECK(f.uv == e[0] && f.cap == e[1] && f.thr == e[2] && f.soft == e[3], "bits 0-3");
	CHECK(f.uv_seen == e[4] && f.cap_seen == e[5] && f.thr_seen == e[6] &&
		      f.soft_seen == e[7],
	      "bits 16-19");
	int n = thr_format(out, sizeof(out), &f);
	CHECK(n == (int)strlen(line) && strcmp(out, line) == 0, line);
}

int main(void)
{
	uint32_t b[THR_WORDS + 2];
	struct thr_flags f, z;
	char out[160];

	/* request, byte for byte */
	memset(b, 0xa5, sizeof(b));
	CHECK(thr_request(b, THR_WORDS + 2) == 7, "request words");
	static const uint32_t want[7] = { 28, 0, 0x00030046, 4, 0, 0, 0 };
	CHECK(memcmp(b, want, sizeof(want)) == 0, "request bytes");
	CHECK(b[7] == 0xa5a5a5a5u && b[8] == 0xa5a5a5a5u, "request writes only 7 words");
	CHECK(thr_request(b, 6) == 0, "request: buffer too small");
	CHECK(thr_request(NULL, 7) == 0, "request: NULL");

	/* good answers */
	answer(b, 0x50005);
	CHECK(thr_check(b, THR_WORDS, &f) == 0 && f.raw == 0x50005, "good answer");
	answer(b, 0);
	CHECK(thr_check(b, THR_WORDS, &f) == 0 && f.raw == 0, "good answer, value 0");
	answer(b, 1);
	b[4] = 0x80000008u;
	CHECK(thr_check(b, THR_WORDS, &f) == 0, "longer response length accepted");

	/* bad answers; *f untouched */
	memset(&f, 0x77, sizeof(f));
	memset(&z, 0x77, sizeof(z));
	answer(b, 5);
	b[1] = 0x80000001u;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_CODE, "error code");
	answer(b, 5);
	b[1] = 0;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_CODE, "request code left");
	answer(b, 5);
	b[4] = 0x00000004u;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_RESP, "response bit missing");
	answer(b, 5);
	b[4] = 0x80000003u;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_LENGTH, "truncated length 3");
	b[4] = 0x80000000u;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_LENGTH, "length 0");
	answer(b, 5);
	b[2] = 0x00030047;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_TAG, "wrong tag");
	answer(b, 5);
	b[0] = 24;
	CHECK(thr_check(b, THR_WORDS, &f) == THR_ERR_SIZE, "wrong size");
	answer(b, 5);
	CHECK(thr_check(b, 6, &f) == THR_ERR_ARG, "too few words");
	CHECK(thr_check(NULL, 7, &f) == THR_ERR_ARG, "NULL buffer");
	CHECK(thr_check(b, 7, NULL) == THR_ERR_ARG, "NULL flags");
	CHECK(memcmp(&z, &f, sizeof(z)) == 0, "flags untouched after errors");

	/* decodes and lines */
	static const int e0[8] = { 0, 0, 0, 0, 0, 0, 0, 0 };
	static const int e1[8] = { 1, 0, 1, 0, 1, 0, 1, 0 };
	static const int e2[8] = { 0, 0, 0, 1, 0, 0, 0, 1 };
	static const int e3[8] = { 1, 1, 1, 1, 1, 1, 1, 1 };
	decode_case(0x0, e0,
		    "throttled=0x00000000 uv=0 cap=0 thr=0 soft=0 uv_seen=0 cap_seen=0 "
		    "thr_seen=0 soft_seen=0");
	decode_case(0x50005, e1,
		    "throttled=0x00050005 uv=1 cap=0 thr=1 soft=0 uv_seen=1 cap_seen=0 "
		    "thr_seen=1 soft_seen=0");
	decode_case(0x80008, e2,
		    "throttled=0x00080008 uv=0 cap=0 thr=0 soft=1 uv_seen=0 cap_seen=0 "
		    "thr_seen=0 soft_seen=1");
	decode_case(0xffffffffu, e3,
		    "throttled=0xffffffff uv=1 cap=1 thr=1 soft=1 uv_seen=1 cap_seen=1 "
		    "thr_seen=1 soft_seen=1");

	/* snprintf truncation is safe */
	thr_decode(0x50005, &f);
	memset(out, 'x', sizeof(out));
	CHECK(thr_format(out, 10, &f) > 10 && out[9] == 0 && out[10] == 'x', "line truncates");

	printf("throttled_mock: %d of %d checks, %d failed\n", n_checks - n_failed, n_checks,
	       n_failed);
	return n_failed ? 1 : 0;
}
