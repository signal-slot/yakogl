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
 * and three made-up runs of the crash kexec, PD_GPU handed over on with the
 * GPU as panthor runs it (masks 0x00020103 / 0xffffffff / 0x0000ffff, the
 * firmware's global interrupt and an MMU fault pending, MCU enabled):
 *
 *   t6_z1_test <probe> crash  the take-over and Z1 run through (the record
 *                             t6_gpu prints before its PASS line)
 *   t6_z1_test <probe> mask   JOB_INT_MASK reads back set: stop 14, refusal 9
 *   t6_z1_test <probe> hard   the MCU does not halt, then neither reset
 *                             completes: stop 14, refusal 11
 *
 * driver/tools/kexec-zephyr/t6-verdict-test.sh builds it with the host cc and
 * compares its lines with those in the Z1 stop fixtures (testdata/).
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "t6_z1.h"

/* The crash route: PD_GPU on (Armbian's words with the GPU active), the
 * PLL on, the take-over as far as `mode` lets it go. */
static void take_over(struct yd_t6_gpu_up *o, const char *mode)
{
	const uint32_t pd_on[6] = { 0x0000fffe, 0x2455ffff, 0x000083ce,
				    0x000a9ffe, 0x000a9ffe, 0xfad50003 };
	const uint32_t masks[3] = { 0x00020103, 0xffffffff, 0x0000ffff };
	const uint32_t raw[3] = { 0x00000000, 0x80000000, 0x00000001 };
	const uint32_t cleared[3] = { 0x00000000, 0x80000000, 0x00000000 };

	memcpy(o->pd_before, pd_on, sizeof(pd_on));
	memcpy(o->pd_after, pd_on, sizeof(pd_on));
	o->pll_on[0] = 1;
	o->pll_on[2] = 0x00004000;
	o->pd_was_on = 1;
	o->take_over_ran = 1;
	memcpy(o->take_over_masks, masks, sizeof(masks));
	memcpy(o->take_over_raw, raw, sizeof(raw));
	o->take_over_mcu[0] = 1;
	if (strcmp(mode, "mask") == 0) {
		o->stop = YD_T6_GPU_STOP_TAKE_OVER;
		o->refusal = YD_MALI_IRQ_MASK_STUCK;
		o->detail[0] = 0xffffffff;
		o->detail[1] = 0x1008;
		return;
	}
	memcpy(o->take_over_raw_cleared, cleared, sizeof(cleared));
	o->take_over_steps = YD_T6_TAKE_OVER_MASKED;
	if (strcmp(mode, "hard") == 0) {
		o->take_over_mcu[1] = 100001;
		o->take_over_mcu[2] = 1;
		o->take_over_mcu[3] = 1;
		o->take_over_steps = YD_T6_TAKE_OVER_HALT_WAITED;
		o->take_over_hard = 1;
		o->stop = YD_T6_GPU_STOP_TAKE_OVER;
		o->refusal = YD_MALI_HARD_RESET_TIMEOUT;
		return;
	}
	o->take_over_mcu[1] = 3;
	o->take_over_steps = YD_T6_TAKE_OVER_RESET;
	o->take_over_reset[0] = 0x00000000;
	o->take_over_reset[2] = 1;
	o->stop = YD_T6_GPU_STOP_NONE;
	for (int b = 0; b < 4; b++) {
		o->qos[b][0] = 0x80000000;
	}
	o->qos_policy = YD_T6_QOS_LINUX_ZERO;
	const uint32_t ident[16] = { 0xa8670005, 0x07120306, 0x00000809, 0x00000301,
				     0x00002830, 0x000000ff, 0x040a0412, 0x00050005, 0,
				     1, 0, 1, 0, 0x00000001, 0, 0 };
	memcpy(o->ident, ident, sizeof(ident));
	memcpy(o->ident_after_reset, ident, sizeof(ident));
	o->reset[0] = 0x00000100;
	o->reset[2] = 1;
	o->l2_mask[0] = 1;
	o->l2_ready[0] = 1;
	o->l2_waited_us[1] = 2;
}

int main(int argc, char **argv)
{
	struct yd_t6_gpu_up o;

	const char *const modes[] = { "pll", "pd", "crash", "mask", "hard" };
	bool known = false;

	for (size_t i = 0; argc == 3 && i < sizeof(modes) / sizeof(modes[0]); i++) {
		known = known || strcmp(argv[2], modes[i]) == 0;
	}
	if (!known) {
		fprintf(stderr, "usage: %s <probe> {pll|pd|crash|mask|hard}\n", argv[0]);
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
	if (strcmp(argv[2], "crash") == 0 || strcmp(argv[2], "mask") == 0 ||
	    strcmp(argv[2], "hard") == 0) {
		take_over(&o, argv[2]);
		if (strcmp(argv[2], "crash") == 0) {
			t6_z1_print_record(argv[1], &o, false);
			return 0;
		}
	} else if (strcmp(argv[2], "pll") == 0) {
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
