/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Host mock for rpi5_scanout/infoframe.h. */
#include <stdio.h>
#include <string.h>

#include "infoframe.h"

static int n, bad;
#define CHECK(c)                                                                \
	do {                                                                    \
		n++;                                                            \
		if (!(c)) {                                                     \
			bad++;                                                  \
			printf("FAIL %s:%d %s\n", __FILE__, __LINE__, #c);      \
		}                                                               \
	} while (0)

static unsigned sum(const uint8_t *p, size_t l)
{
	unsigned s = 0;

	while (l--) {
		s += *p++;
	}
	return s & 0xff;
}

int main(void)
{
	static const unsigned vics[] = {1, 4, 16};
	uint8_t f[AVI_IF_BYTES], g[AVI_IF_BYTES];
	uint32_t w[AVI_IF_WORDS];
	unsigned v, q, a, i;

	for (v = 0; v < 3; v++) {
		for (q = AVI_Q_LIMITED; q <= AVI_Q_FULL; q++) {
			CHECK(avi_infoframe_build(vics[v], AVI_CS_RGB, (enum avi_quant)q,
						  AVI_ASPECT_16_9, f) == 0);
			CHECK(f[0] == 0x82 && f[1] == 2 && f[2] == 13);
			CHECK(sum(f, sizeof f) == 0);
			CHECK(f[7] == vics[v]);
			CHECK(((f[6] >> 2) & 3) == q);
			CHECK((f[4] >> 6) == 0);
			CHECK(((f[5] >> 4) & 3) == 2);
			CHECK(avi_infoframe_check(f, sizeof f) == 0);
			for (i = 8; i < AVI_IF_BYTES; i++) {
				CHECK(f[i] == 0);
			}
		}
	}
	/* known vector: VIC 16, RGB, full, 16:9 */
	CHECK(avi_infoframe_build(16, AVI_CS_RGB, AVI_Q_FULL, AVI_ASPECT_16_9, f) == 0);
	CHECK(f[3] == (uint8_t)(0x100 - (0x82 + 2 + 13 + 0x20 + 0x08 + 16)));
	CHECK(f[3] == 0x37);
	/* aspects */
	for (a = AVI_ASPECT_NONE; a <= AVI_ASPECT_16_9; a++) {
		CHECK(avi_infoframe_build(1, AVI_CS_RGB, AVI_Q_DEFAULT, (enum avi_aspect)a, f) == 0);
		CHECK(((f[5] >> 4) & 3) == a && avi_infoframe_check(f, sizeof f) == 0);
	}
	/* bad arguments */
	CHECK(avi_infoframe_build(0, AVI_CS_RGB, AVI_Q_FULL, AVI_ASPECT_NONE, f) < 0);
	CHECK(avi_infoframe_build(128, AVI_CS_RGB, AVI_Q_FULL, AVI_ASPECT_NONE, f) < 0);
	CHECK(avi_infoframe_build(4, AVI_CS_RGB, (enum avi_quant)3, AVI_ASPECT_NONE, f) < 0);
	CHECK(avi_infoframe_build(4, AVI_CS_RGB, AVI_Q_FULL, (enum avi_aspect)3, f) < 0);
	CHECK(avi_infoframe_build(4, (enum avi_colourspace)1, AVI_Q_FULL, AVI_ASPECT_NONE, f) < 0);
	CHECK(avi_infoframe_build(4, AVI_CS_RGB, AVI_Q_FULL, AVI_ASPECT_NONE, NULL) < 0);
	/* check: bad lengths, corrupted bytes, wrong length field */
	CHECK(avi_infoframe_build(4, AVI_CS_RGB, AVI_Q_LIMITED, AVI_ASPECT_16_9, f) == 0);
	CHECK(avi_infoframe_check(f, 16) < 0);
	CHECK(avi_infoframe_check(f, 0) < 0);
	CHECK(avi_infoframe_check(f, 18) < 0);
	CHECK(avi_infoframe_check(NULL, AVI_IF_BYTES) < 0);
	for (i = 0; i < AVI_IF_BYTES; i++) {
		memcpy(g, f, sizeof g);
		g[i] ^= 0x10;
		CHECK(avi_infoframe_check(g, sizeof g) < 0);
	}
	memcpy(g, f, sizeof g);
	g[2] = 12;
	g[3] = (uint8_t)(g[3] + 1);
	CHECK(sum(g, sizeof g) == 0);
	CHECK(avi_infoframe_check(g, sizeof g) < 0);
	/* packed words */
	avi_infoframe_pack(f, w);
	CHECK(w[0] == (0x82u | 2u << 8 | 13u << 16 | (uint32_t)f[3] << 24));
	CHECK(w[1] == ((uint32_t)f[4] | (uint32_t)f[5] << 8 | (uint32_t)f[6] << 16 |
		       (uint32_t)f[7] << 24));
	CHECK(w[4] == f[16]);
	for (i = 0; i < AVI_IF_BYTES; i++) {
		g[i] = (uint8_t)(w[i / 4] >> (8 * (i % 4)));
	}
	CHECK(memcmp(g, f, sizeof g) == 0);
	CHECK(avi_infoframe_check(g, sizeof g) == 0);

	if (bad) {
		printf("infoframe_mock: %d of %d checks, %d failed\n", n - bad, n, bad);
		return 1;
	}
	printf("infoframe_mock: %d of %d checks, 0 failed\n", n, n);
	return 0;
}
