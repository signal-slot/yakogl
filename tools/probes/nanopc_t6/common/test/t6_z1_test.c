/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The host side of Z1's record (../t6_z1.c): prints the stop record a probe
 * prints when Z1's power-on stops, for one of two made-up stops on the
 * hand-over of 2026-11-11 (driver/docs/nanopc-t6-mali.md, "Measured,
 * 2026-11-11"): CLKSEL_CON(158) 0x00001f60 (NPLL), NPLL powered down,
 * BUCK1_ON_VSEL 0x30. Not board output.
 *
 *   t6_z1_test <probe> pll   NPLL does not lock: stop 4, refusal 1, before
 *                            the hand-over report
 *   t6_z1_test <probe> pd    PD_GPU's idle acknowledge never clears: stop 5,
 *                            refusal 3, detail 0x000a9fff (the record of
 *                            2026-11-11 13:13)
 *
 * driver/tools/kexec-zephyr/t6-verdict-test.sh builds it with the host cc and
 * compares its lines with those in the Z1 stop fixtures (testdata/).
 */
#include <stdio.h>
#include <string.h>

#include "t6_z1.h"

int main(int argc, char **argv)
{
	struct yd_t6_gpu_up o;

	if (argc != 3 || (strcmp(argv[2], "pll") != 0 && strcmp(argv[2], "pd") != 0)) {
		fprintf(stderr, "usage: %s <probe> {pll|pd}\n", argv[0]);
		return 2;
	}
	memset(&o, 0, sizeof(o));
	/* the hand-over */
	const uint32_t pd_before[6] = { 0x0000ffff, 0x2457ffff, 0x0000e3cf,
					0x000affff, 0x000affff, 0xfad40001 };
	const uint32_t clock[8] = { 0x00001f60, 0x00000000, 0x000000d6, 0x00000545,
				    0x000001a9, 0x00002083, 0x00000000, 0x00004000 };
	const uint32_t spi_found[8] = { 0x00002c01, 0, 0, 0, 0xc6, 0x0000000c, 0, 0x00110002 };

	memcpy(o.pd_before, pd_before, sizeof(pd_before));
	memcpy(o.clock, clock, sizeof(clock));
	memcpy(o.spi_found, spi_found, sizeof(spi_found));
	o.gpu_src_hz[0] = 850000000;
	o.rk806[0] = 0x0f;
	o.rk806[1] = 0xe0;
	o.rk806[2] = 0xa4;
	o.rk806[3] = 0x30;
	o.vdd_gpu_uv = 800000;
	/* 850 MHz is an OPP rate: the clock is kept */
	o.clock_kept = 1;
	o.clock_cpll_hz[0] = 1500000000;
	o.clock_sel[0] = 0x00001f60;
	o.clock_set_hz[0] = 850000000;
	if (strcmp(argv[2], "pll") == 0) {
		o.stop = YD_T6_GPU_STOP_CLOCK;
		o.refusal = YD_T6_GPU_CLOCK_PLL_NO_LOCK;
		o.detail[0] = 0x00004000;
	} else {
		o.pll_on[0] = 0;
		o.pll_on[1] = 10;
		o.pll_on[2] = 0x0000a400;
		o.pd_opened = 0xd2;
		o.pd_waited_us[0] = 6;
		o.pd_waited_us[1] = 1000;
		const uint32_t pd_after[6] = { 0x00000000, 0x2457ffff, 0x0000a3cf,
					       0x000a9fff, 0x000a9fff, 0xfad50001 };
		memcpy(o.pd_after, pd_after, sizeof(pd_after));
		o.stop = YD_T6_GPU_STOP_PD;
		o.refusal = 3;
		o.detail[0] = 0x000a9fff;
	}
	t6_z1_print_stop(argv[1], &o);
	return 0;
}
