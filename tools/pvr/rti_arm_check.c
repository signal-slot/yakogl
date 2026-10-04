/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host check of the AM62P RTI arming arithmetic
 * (driver/zephyr/lib/board/am62p/rti_arm.h, used by rti_arm.c).
 *
 *   cc -std=c11 -Wall -Wextra -Werror tools/pvr/rti_arm_check.c -o rti_arm_check
 *
 * Every expected value is worked out in its comment and was recomputed
 * independently (python3) before it was written here; none is taken from
 * the code under test. The rates are the four parents of TISCI clock 125:0
 * that the board's Linux clock debugfs reported (2026-09-29).
 * Prints "rti_arm_check: N checks run (planned N), 0 failed" on success.
 */
#include <stdio.h>

#include "../../driver/zephyr/lib/board/am62p/rti_arm.h"

#define PLANNED 18

static int run, failed;

static void check(int cond, const char *what)
{
	run++;
	if (!cond) {
		failed++;
		printf("FAIL  %s\n", what);
	}
}

int main(void)
{
	/* 32768 Hz (clk:125:2, the parent Linux uses): 60 * 32768 = 1966080;
	 * >> 13 = 240; preload 239 = 0xef; expiry 240 * 8192 / 32768 = 60.000 s. */
	check(rti_arm_rate_known(32768), "32768 Hz is a reported rate");
	check(rti_arm_preload(32768, 60) == 0xef, "32768 Hz, 60 s: preload 0xef");
	check(rti_arm_expiry_ms(0xef, 32768) == 60000, "preload 0xef at 32768 Hz: 60000 ms");
	/* Linux's own preload on this board was 0xf0 (its driver omits the -1):
	 * 241 * 8192 / 32768 = 60.25 s. The formula here never exceeds 60 s. */
	check(rti_arm_expiry_ms(0xf0, 32768) == 60250, "Linux's 0xf0 at 32768 Hz: 60250 ms");

	/* 32552 Hz (clk:125:4): 60 * 32552 = 1953120; >> 13 = 238
	 * (238 * 8192 = 1949696 <= 1953120 < 1957888); preload 237 = 0xed;
	 * expiry 1949696 * 1000 / 32552 = 59894 ms (rounded down). */
	check(rti_arm_rate_known(32552), "32552 Hz is a reported rate");
	check(rti_arm_preload(32552, 60) == 0xed, "32552 Hz, 60 s: preload 0xed");
	check(rti_arm_expiry_ms(0xed, 32552) == 59894, "preload 0xed at 32552 Hz: 59894 ms");

	/* 25 MHz (clk:125:1; the value Zephyr's k3-am62-main.dtsi gives, the
	 * likely reset default): 60 * 25e6 >> 13 = 183105 > 4096: refused. The
	 * longest possible expiry, preload 0xfff: 4096 * 8192 / 25e6 = 1342 ms. */
	check(rti_arm_rate_known(25000000), "25 MHz is a reported rate");
	check(rti_arm_preload(25000000, 60) == -1, "25 MHz, 60 s: refused (183105 units > 4096)");
	check(rti_arm_expiry_ms(0xfff, 25000000) == 1342, "25 MHz, preload 0xfff: 1342 ms");
	check(rti_arm_preload(25000000, 1) == 0xbea,
	      "25 MHz, 1 s: 25e6 >> 13 = 3051, preload 3050 = 0xbea (fits)");

	/* 12.5 MHz (clk:125:3): 60 * 12.5e6 >> 13 = 91552 > 4096: refused. */
	check(rti_arm_rate_known(12500000), "12.5 MHz is a reported rate");
	check(rti_arm_preload(12500000, 60) == -1, "12.5 MHz, 60 s: refused");

	/* Rates the board did not report are not known: 32000 (a nominal
	 * "32 kHz"), 0 and 24 MHz. */
	check(!rti_arm_rate_known(32000), "32000 Hz is not a reported rate");
	check(!rti_arm_rate_known(0), "0 Hz is not a reported rate");
	check(!rti_arm_rate_known(24000000), "24 MHz is not a reported rate");

	/* Edges of the 12-bit field: 0 units is refused (no preload of -1);
	 * exactly 4096 units fits as 0xfff, 4097 does not. */
	check(rti_arm_preload(4096, 1) == -1, "4096 Hz, 1 s: 0 units, refused");
	check(rti_arm_preload(4096ull * 8192, 1) == 0xfff && rti_arm_preload(4097ull * 8192, 1) == -1,
	      "4096 units fit as 0xfff, 4097 do not");

	printf("rti_arm_check: %d checks run (planned %d), %d failed\n", run, PLANNED, failed);
	return (failed || run != PLANNED) ? 1 : 0;
}
