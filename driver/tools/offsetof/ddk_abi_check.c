/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * ddk_abi_check.c -- proves every firmware-interface number the PowerVR
 * driver uses against the vendored TI/Img DDK 25.2 headers
 * (third_party/ti-img-rogue-ddk-25.2) compiled with the AM62P build
 * configuration the firmware matches (config/am62p_linux_release).
 *
 * The numbers are in ddk_abi_pins.h; each becomes a _Static_assert here, so
 * a header or configuration that moves any of them does not compile. The
 * one table that cannot be a static assertion, the alignment-check array the
 * host hands the firmware at boot (RGXFW_ALIGN_CHECKS_INIT_KM, which the
 * firmware compares against its own and refuses to boot on a difference),
 * is compared when the program runs.
 *
 * Build and run (tools/check.sh does this):
 *   cc -std=gnu11 -Wall -Wextra -Werror $(driver/tools/offsetof/ddk-cflags.sh) \
 *      driver/tools/offsetof/ddk_abi_check.c -o ddk_abi_check && ./ddk_abi_check
 * It prints "ddk_abi_check: N checks run (planned N), 0 failed".
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#include "pvr_ddk.h" /* driver/compat: the driver's own include of the DDK headers */

#define PIN(value, ddk) _Static_assert((ddk) == (value), "DDK: " #ddk " != " #value);
#define MISMATCH(ddk_value, ddk, fork_value, fork, why) PIN(ddk_value, ddk)
#include "ddk_abi_pins.h"
#undef PIN
#undef MISMATCH

enum {
#define PIN(value, ddk) +1
#define MISMATCH(ddk_value, ddk, fork_value, fork, why) +1
	N_PINS = 0
#include "ddk_abi_pins.h"
#undef PIN
#undef MISMATCH
};

/* The alignment-check array pvr_fw_boot.c hands the firmware, as the board
 * runs it (sysinit.sAlignChecks). */
static const uint32_t pinned_align_checks[] = {
	880, 560, 568, 576, 584, 632, 636, 160,
	0, 4, 12, 16, 36, 40, 912, 0,
	456, 456, 96, 80, 4, 12, 0, 4, 8,
};
static const IMG_UINT32 ddk_align_checks[] = { RGXFW_ALIGN_CHECKS_INIT_KM };
_Static_assert(sizeof(pinned_align_checks) == sizeof(ddk_align_checks),
	       "RGXFW_ALIGN_CHECKS_INIT_KM has a different number of entries");
#define N_ALIGN (sizeof(pinned_align_checks) / sizeof(pinned_align_checks[0]))

int main(void)
{
	unsigned failed = 0;

	for (size_t i = 0; i < N_ALIGN; i++) {
		if (pinned_align_checks[i] != ddk_align_checks[i]) {
			printf("align check %zu: driver %u, DDK %u\n", i,
			       pinned_align_checks[i], ddk_align_checks[i]);
			failed++;
		}
	}
	printf("ddk_abi_check: %u checks run (planned %u), %u failed\n",
	       (unsigned)(N_PINS + N_ALIGN), (unsigned)(N_PINS + N_ALIGN), failed);
	return failed ? 1 : 0;
}
