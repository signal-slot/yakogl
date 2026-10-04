/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_clear - milestone 4 on the i.MX8MP GPU: fill a linear XRGB8888 surface
 * in the carveout with a colour, then read the pixels back on the CPU.
 *
 * The clear goes through the resolve engine, which this core uses because it
 * has no BLT engine. That engine touches none of the 3D state, so this is the
 * first picture the GPU can be asked for that depends on nothing but memory,
 * the MMU and the submit path -- if this passes and gc_triangle does not, the
 * fault is in the 3D pipe.
 *
 * The surface is poisoned with a pattern first and then cleared twice with
 * different colours, so neither "the memory happened to be zero" nor "the
 * first clear was the only one that landed" can pass.
 *
 * Boot it from Torizon with tools/kexec-zephyr:
 *   kexec_zephyr zephyr.bin 0x42000000
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <gc.h>
#include <gc_render.h>
#include <gc_draw.h>

#include "gc_sample.h"

LOG_MODULE_REGISTER(gc_clear, LOG_LEVEL_INF);

#define W  CONFIG_GC_CLEAR_WIDTH
#define H  CONFIG_GC_CLEAR_HEIGHT

static struct gc_target s_target;

/* Every corner and the middle. A clear that only covered part of the surface
 * -- the wrong window size, the wrong stride -- fails at a corner. */
static bool check_all(uint32_t want)
{
	static const struct { uint32_t x, y; const char *name; } probe[] = {
		{ 0, 0, "top left" },
		{ W - 1, 0, "top right" },
		{ 0, H - 1, "bottom left" },
		{ W - 1, H - 1, "bottom right" },
		{ W / 2, H / 2, "middle" },
	};
	bool ok = true;

	for (size_t i = 0; i < ARRAY_SIZE(probe); i++) {
		uint32_t got = gc_target_pixel(&s_target, probe[i].x, probe[i].y);

		LOG_INF("  %-13s (%3u,%3u) = 0x%06x", probe[i].name, probe[i].x, probe[i].y,
			got);
		if (got != want) {
			LOG_ERR("  %s: want 0x%06x, got 0x%06x", probe[i].name, want, got);
			ok = false;
		}
	}
	return ok;
}

static bool clear_and_check(uint32_t colour)
{
	uint32_t fence = 0;
	int rc = gc_draw_clear(&s_target, colour, &fence);

	if (rc) {
		LOG_ERR("gc_draw_clear(0x%06x): %d", colour, rc);
		return false;
	}
	rc = gc_render_wait(fence, 500);
	if (rc) {
		LOG_ERR("fence %u: %d", fence, rc);
		return false;
	}
	LOG_INF("cleared to 0x%06x (fence %u):", colour, fence);
	return check_all(colour);
}

static const char *run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo bo;
	int rc;

	if (!device_is_ready(gpu)) {
		return "the GPU device did not initialise";
	}
	rc = gc_render_init(gpu);
	if (rc) {
		LOG_ERR("gc_render_init: %d", rc);
		return "the render service did not come up";
	}

	/* The stride is the width rounded up to the pixel engine's 64-byte
	 * alignment, which for a 32-bit format means 16 pixels. */
	s_target.width = W;
	s_target.height = H;
	s_target.stride = ROUND_UP(W * 4u, 64u);
	rc = gc_render_alloc((size_t)s_target.stride * H, 64, &bo);
	if (rc) {
		return "no carveout memory for the render target";
	}
	s_target.gpu = bo.gpu;
	s_target.cpu = bo.cpu;
	LOG_INF("target %ux%u stride %u at GPU 0x%08x (CPU %p)", W, H, s_target.stride,
		s_target.gpu, s_target.cpu);

	/* Poison it, so "the allocator zeroed it" cannot look like a pass. */
	for (uint32_t y = 0; y < H; y++) {
		uint32_t *row = (uint32_t *)((uint8_t *)s_target.cpu + y * s_target.stride);

		for (uint32_t x = 0; x < W; x++) {
			row[x] = 0x00a55a3c;
		}
	}
	gc_render_cache_flush(s_target.cpu, (size_t)s_target.stride * H);

	gc_sample_service_wdog();
	if (!clear_and_check(gc_rgb(0x20, 0x80, 0xc0))) {
		return "the first clear did not fill the surface";
	}
	gc_sample_service_wdog();
	if (!clear_and_check(gc_rgb(0xff, 0x00, 0x40))) {
		return "the second clear did not replace the first";
	}
	return NULL;
}

int main(void)
{
	const char *reason;

	gc_sample_service_wdog();
	reason = run();
	gc_sample_verdict("gc_clear", reason == NULL, reason);
	if (reason != NULL) {
		gc_render_stall_dump("gc_clear failed");
	}
	gc_sample_finish();
	return 0;
}
