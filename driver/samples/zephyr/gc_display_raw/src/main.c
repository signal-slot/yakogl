/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_display_raw - the display driver by itself, nothing else in the way.
 * A framebuffer of our own, a colour ramp in it, imx8mp_display_on(), and
 * the verdict in the hold time as every gc sample does it.
 */
#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/mm.h>
#include "gc_sample.h"
#include "imx8mp_display.h"

#define W 1280u
#define H 800u
#define PITCH (W * 4u)

static uint8_t fb[PITCH * H] __aligned(4096) __noinit;

int main(void)
{
	int rc;

	gc_sample_service_wdog();
	for (uint32_t y = 0; y < H; y++) {
		uint32_t *row = (uint32_t *)(fb + (size_t)y * PITCH);

		for (uint32_t x = 0; x < W; x++) {
			row[x] = 0xff000000u | ((x * 255u / W) << 16) | ((y * 255u / H) << 8) | 0x40u;
		}
	}
	sys_cache_data_flush_range(fb, sizeof(fb));
	rc = imx8mp_display_on(k_mem_phys_addr(fb), PITCH);
	gc_sample_verdict("gc_display_raw", rc == 0, rc == 0 ? NULL : "imx8mp_display_on failed");
	/* Whatever happened, say which stages are alive: the return time is
	 * 70 s + 3 s * the health bits (imx8mp_display.h), read after a
	 * second so the bridge and PLLs have had time to settle. */
	k_sleep(K_SECONDS(1));
	gc_sample_hold_code(imx8mp_display_health());
	gc_sample_finish();
	return 0;
}
