/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_triangle - milestone 5 on the i.MX8MP GPU: clear a surface, then draw
 * one smooth-shaded triangle into it through the 3D pipe, and check the
 * pixels on the CPU.
 *
 * The three vertices are red, green and blue, so the read-back can tell four
 * different failures apart:
 *
 *   - nothing drew          -> every probe still has the clear colour;
 *   - the triangle drew but flat-shaded -> the three vertex probes agree;
 *   - the geometry is wrong -> the corner probe is no longer the clear
 *                              colour, or the vertex probes are;
 *   - the colours are swapped -> the dominant channel is the wrong one,
 *                              which means the pixel format, not the render.
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

LOG_MODULE_REGISTER(gc_triangle, LOG_LEVEL_INF);

#define W  CONFIG_GC_TRIANGLE_WIDTH
#define H  CONFIG_GC_TRIANGLE_HEIGHT

#define CLEAR_COLOUR  gc_rgb(0x00, 0x00, 0x40)   /* dark blue */

static struct gc_target s_target;

/* Clip space, w = 1. The viewport puts clip y = +1 on row 0, so v0 is the
 * top vertex in memory as well as on a screen. */
static const struct gc_vertex s_tri[3] = {
	{ { 0.0f,  0.8f, 0.0f, 1.0f }, { 1.0f, 0.0f, 0.0f, 1.0f } },   /* red,   top */
	{ {-0.8f, -0.8f, 0.0f, 1.0f }, { 0.0f, 1.0f, 0.0f, 1.0f } },   /* green, bottom left */
	{ { 0.8f, -0.8f, 0.0f, 1.0f }, { 0.0f, 0.0f, 1.0f, 1.0f } },   /* blue,  bottom right */
};

/* Clip space to pixel, the same mapping the viewport registers program. */
static uint32_t px_of(float x) { return (uint32_t)((x + 1.0f) * 0.5f * (float)W); }
static uint32_t py_of(float y) { return (uint32_t)((1.0f - y) * 0.5f * (float)H); }

static inline uint8_t red(uint32_t p)   { return (p >> 16) & 0xff; }
static inline uint8_t green(uint32_t p) { return (p >> 8) & 0xff; }
static inline uint8_t blue(uint32_t p)  { return p & 0xff; }

/* A point 80 per cent of the way from the centroid to vertex `i`: well
 * inside the triangle, and dominated by that vertex's colour. Barycentric
 * weight there is about 0.87, so the matching channel should read near 0xdd
 * and the other two near 0x11. */
static void probe_point(int i, uint32_t *x, uint32_t *y)
{
	float cx = (s_tri[0].pos[0] + s_tri[1].pos[0] + s_tri[2].pos[0]) / 3.0f;
	float cy = (s_tri[0].pos[1] + s_tri[1].pos[1] + s_tri[2].pos[1]) / 3.0f;

	*x = px_of(cx + 0.8f * (s_tri[i].pos[0] - cx));
	*y = py_of(cy + 0.8f * (s_tri[i].pos[1] - cy));
}

static bool check_vertex_colours(void)
{
	static const char *const name[3] = { "red (top)", "green (left)", "blue (right)" };
	bool ok = true;

	for (int i = 0; i < 3; i++) {
		uint32_t x, y, p;
		uint8_t c[3];

		probe_point(i, &x, &y);
		p = gc_target_pixel(&s_target, x, y);
		c[0] = red(p);
		c[1] = green(p);
		c[2] = blue(p);
		LOG_INF("  near %-13s (%3u,%3u) = 0x%06x", name[i], x, y, p);

		if (c[i] < 0x80) {
			LOG_ERR("  near %s: the %s channel is only 0x%02x", name[i], name[i],
				c[i]);
			ok = false;
		}
		for (int j = 0; j < 3; j++) {
			if (j != i && c[j] >= c[i]) {
				LOG_ERR("  near %s: channel %d (0x%02x) is not below channel "
					"%d (0x%02x)", name[i], j, c[j], i, c[i]);
				ok = false;
			}
		}
	}
	return ok;
}

static const char *run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo bo;
	uint32_t fence = 0, corner, mid;
	int rc;

	if (!device_is_ready(gpu)) {
		return "the GPU device did not initialise";
	}
	rc = gc_render_init(gpu);
	if (rc) {
		LOG_ERR("gc_render_init: %d", rc);
		return "the render service did not come up";
	}

	s_target.width = W;
	s_target.height = H;
	s_target.stride = ROUND_UP(W * 4u, 64u);
	rc = gc_render_alloc((size_t)s_target.stride * H, 64, &bo);
	if (rc) {
		return "no carveout memory for the render target";
	}
	s_target.gpu = bo.gpu;
	s_target.cpu = bo.cpu;
	LOG_INF("target %ux%u stride %u at GPU 0x%08x", W, H, s_target.stride, s_target.gpu);

	gc_sample_service_wdog();
	rc = gc_draw_clear(&s_target, CLEAR_COLOUR, &fence);
	if (rc == 0) {
		rc = gc_render_wait(fence, 500);
	}
	if (rc) {
		LOG_ERR("clear: %d", rc);
		return "the clear failed (run gc_clear first)";
	}
	if (gc_target_pixel(&s_target, 1, 1) != CLEAR_COLOUR) {
		return "the clear did not take effect";
	}

	gc_sample_service_wdog();
	rc = gc_draw_triangle(&s_target, s_tri, &fence);
	if (rc) {
		LOG_ERR("gc_draw_triangle: %d", rc);
		return "the draw could not be submitted";
	}
	rc = gc_render_wait(fence, 1000);
	if (rc) {
		LOG_ERR("fence %u: %d", fence, rc);
		return rc == -ETIMEDOUT ? "the draw never completed" : "the GPU faulted";
	}

	/* A corner is outside the triangle and must still be the clear
	 * colour: the proof that the draw was bounded and not a full-surface
	 * overwrite. */
	corner = gc_target_pixel(&s_target, 1, 1);
	LOG_INF("  corner        (  1,  1) = 0x%06x (clear is 0x%06x)", corner, CLEAR_COLOUR);
	if (corner != CLEAR_COLOUR) {
		return "the triangle covered a corner that is outside it";
	}

	if (!check_vertex_colours()) {
		return "the vertex colours did not interpolate";
	}

	/* The centroid has all three vertices about equally, so every channel
	 * has to be present and none may dominate. A flat-shaded triangle
	 * fails here even when the three vertex probes look plausible. */
	mid = gc_target_pixel(&s_target, px_of(0.0f), py_of(-0.2667f));
	LOG_INF("  centroid              = 0x%06x", mid);
	if (red(mid) < 0x20 || green(mid) < 0x20 || blue(mid) < 0x20 ||
	    red(mid) > 0xe0 || green(mid) > 0xe0 || blue(mid) > 0xe0) {
		return "the centroid is not a blend of all three vertex colours";
	}
	return NULL;
}

int main(void)
{
	const char *reason;

	gc_sample_service_wdog();
	reason = run();
	gc_sample_verdict("gc_triangle", reason == NULL, reason);
	if (reason != NULL) {
		gc_render_stall_dump("gc_triangle failed");
	}
	gc_sample_finish();
	return 0;
}
