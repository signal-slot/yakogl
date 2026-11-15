// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
// Host test of rpi5_scanout/hdmi_rates.h. Build: see
// tools/v3d/check71.d/hdmi_rates.sh.
#include <stdint.h>
#include <stdio.h>

#include "hdmi_rates.h"

static int n, bad;
#define CHECK(c)                                                                    \
	do {                                                                        \
		n++;                                                                \
		if (!(c)) {                                                         \
			bad++;                                                      \
			printf("FAIL line %d: %s\n", __LINE__, #c);                 \
		}                                                                   \
	} while (0)

#define M 1000000ULL

int main(void)
{
	struct hdmi_rates r;

	/* 640x480 at 25.175 MHz: HSM floor, BVB 75 */
	r = hdmi_rates_for_mode(25175000, 8, 0, 640);
	CHECK(r.tmds_hz == 25175000ULL);
	CHECK(r.hsm_min_hz == 120 * M);
	CHECK(r.bvb_hz == 75 * M);
	CHECK(r.verdict == HDMI_RATES_OK);

	/* 720p */
	r = hdmi_rates_for_mode(74250000, 8, 0, 1280);
	CHECK(r.hsm_min_hz == 120 * M);
	CHECK(r.bvb_hz == 75 * M);
	CHECK(r.verdict == HDMI_RATES_OK);

	/* exactly 148.5 MHz: still 75 (150 is "above") */
	r = hdmi_rates_for_mode(148500000, 8, 0, 1920);
	CHECK(r.hsm_min_hz == 149985000ULL);
	CHECK(r.bvb_hz == 75 * M);
	r = hdmi_rates_for_mode(148500001, 8, 0, 1920);
	CHECK(r.bvb_hz == 150 * M);

	/* exactly 297 MHz: 150; above: 300 */
	r = hdmi_rates_for_mode(297000000, 8, 0, 3840);
	CHECK(r.hsm_min_hz == 299970000ULL);
	CHECK(r.bvb_hz == 150 * M);
	CHECK(r.verdict == HDMI_RATES_OK);
	r = hdmi_rates_for_mode(297000001, 8, 0, 3840);
	CHECK(r.bvb_hz == 300 * M);

	/* 594 MHz (4k60): HDMI 2.0, core must be >= 550 */
	r = hdmi_rates_for_mode(594 * M, 8, 0, 3840);
	CHECK(r.tmds_hz == 594 * M);
	CHECK(r.hsm_min_hz == 599940000ULL);
	CHECK(r.bvb_hz == 300 * M);
	CHECK(r.core_min_hz == 550 * M);
	CHECK(r.verdict == HDMI_RATES_NEEDS_HDMI20);
	r = hdmi_rates_for_mode(594 * M, 8, 550 * M, 3840);
	CHECK(r.verdict == HDMI_RATES_NEEDS_HDMI20);
	r = hdmi_rates_for_mode(594 * M, 8, 550 * M - 1, 3840);
	CHECK(r.verdict == (HDMI_RATES_NEEDS_HDMI20 | HDMI_RATES_CORE_TOO_LOW));
	/* a low core clock does not matter for a 1080p mode */
	r = hdmi_rates_for_mode(148500000, 8, 200 * M, 1920);
	CHECK(r.verdict == HDMI_RATES_OK);

	/* 4096 wide needs 600 MHz */
	r = hdmi_rates_for_mode(297 * M, 8, 599 * M, 4096);
	CHECK(r.core_min_hz == 600 * M);
	CHECK(r.verdict == HDMI_RATES_CORE_TOO_LOW);

	/* 650 MHz: over the 600 MHz limit; exactly 600 is fine */
	r = hdmi_rates_for_mode(650 * M, 8, 0, 0);
	CHECK(r.verdict & HDMI_RATES_OVER_MAX_PIXEL);
	CHECK(r.hsm_min_hz == 656500000ULL);
	r = hdmi_rates_for_mode(600 * M, 8, 0, 0);
	CHECK(!(r.verdict & HDMI_RATES_OVER_MAX_PIXEL));

	/* deep colour: 148.5 MHz at 10 bpc is a 185.625 MHz TMDS rate */
	r = hdmi_rates_for_mode(148500000, 10, 0, 1920);
	CHECK(r.tmds_hz == 185625000ULL);
	CHECK(r.hsm_min_hz == 187481250ULL);
	CHECK(r.bvb_hz == 75 * M); /* BVB follows the pixel clock */
	/* 297 MHz at 12 bpc crosses 340 MHz TMDS */
	r = hdmi_rates_for_mode(297 * M, 12, 0, 3840);
	CHECK(r.tmds_hz == 445500000ULL);
	CHECK(r.verdict == HDMI_RATES_NEEDS_HDMI20);

	/* bad arguments */
	CHECK(hdmi_rates_for_mode(0, 8, 0, 0).verdict == HDMI_RATES_BAD_ARGS);
	CHECK(hdmi_rates_for_mode(74250000, 9, 0, 0).verdict == HDMI_RATES_BAD_ARGS);

	if (bad)
		printf("hdmi_rates_mock: %d of %d checks, %d failed\n", n - bad, n, bad);
	else
		printf("hdmi_rates_mock: %d of %d checks, 0 failed\n", n, n);
	return bad ? 1 : 0;
}
