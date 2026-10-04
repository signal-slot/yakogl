/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pins_check.c -- the Rust kernel side's layout numbers
 * (driver/rust/src/gpu/pvr/fwif.rs PINS) against the vendored TI/Img DDK
 * 25.2 headers with the AM62P configuration. `km_check pins` writes them as
 * rust_pins.h (PIN(value, expression) lines); each becomes a _Static_assert
 * here, the way driver/tools/offsetof/ddk_abi_check.c proves the C driver's.
 */
#include <stddef.h>
#include <stdio.h>

#include "pvr_ddk.h"

#define PIN(value, ddk) _Static_assert((unsigned long long)(ddk) == (value), "DDK: " #ddk " != " #value);
#define ALIGN(i, value)
#include "rust_pins.h"
#undef PIN
#undef ALIGN

enum {
#define PIN(value, ddk) +1
#define ALIGN(i, value)
	N_PINS = 0
#include "rust_pins.h"
#undef PIN
#undef ALIGN
};

/* The alignment-check table (RGXFW_ALIGN_CHECKS_INIT_KM) cannot be a static
 * assertion: compared here, element by element, and in number. */
static const IMG_UINT32 ddk_align[] = { RGXFW_ALIGN_CHECKS_INIT_KM };
static const struct { unsigned i; unsigned v; } rust_align[] = {
#define PIN(value, ddk)
#define ALIGN(i, value) { i, value },
#include "rust_pins.h"
#undef PIN
#undef ALIGN
};

int main(void)
{
	const unsigned na = sizeof(rust_align) / sizeof(rust_align[0]);
	const unsigned nd = sizeof(ddk_align) / sizeof(ddk_align[0]);
	unsigned failed = 0;

	if (na != nd) {
		printf("align checks: Rust has %u entries, the DDK %u\n", na, nd);
		failed++;
	}
	for (unsigned k = 0; k < na && k < nd; k++) {
		if (rust_align[k].i != k || rust_align[k].v != ddk_align[k]) {
			printf("align check %u: Rust %u, DDK %u\n", k, rust_align[k].v, ddk_align[k]);
			failed++;
		}
	}
	printf("pins_check: %u checks run (planned %u), %u failed\n", (unsigned)N_PINS + na + 1,
	       (unsigned)N_PINS + na + 1, failed);
	return failed ? 1 : 0;
}
