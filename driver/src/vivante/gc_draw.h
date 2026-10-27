/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2012-2019 Etnaviv Project
 * Ported from Mesa 26.2.1 src/gallium/drivers/etnaviv/ (etnaviv_context.c,
 * etnaviv_emit.c, etnaviv_state.c, etnaviv_rs.c, etnaviv_screen.c) (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * gc_draw.h -- milestones 4 and 5: clear a surface, and draw one Gouraud
 * triangle into it.
 *
 * This is a bring-up rig, not a GL backend. It builds two fixed command
 * streams for one fixed surface format (linear XRGB8888) so that the first
 * picture on the i.MX8MP GPU depends on as little as possible. The real
 * per-draw translation is milestone 6, in YakoGL's Vivante backend.
 */
#ifndef GC_DRAW_H
#define GC_DRAW_H

#include <stdint.h>
#include "gc_render.h"

/*
 * A render target: linear, XRGB8888, one 32-bit pixel per dword in memory as
 * 0x00RRGGBB (little endian, bytes B G R X). That is Mesa's
 * B8G8R8X8_UNORM, the PE's X8R8G8B8.
 *
 * Constraints, all checked:
 *   - `gpu` 64-byte aligned (the pixel engine's alignment),
 *   - `stride` a multiple of 64 bytes,
 *   - `width` a multiple of 16 for gc_draw_clear(): the resolve engine
 *     refuses anything else.
 */
struct gc_target {
	uint32_t gpu;
	uint32_t width;
	uint32_t height;
	uint32_t stride;   /* bytes */
	void    *cpu;      /* CPU view for read-back, or NULL */
};

/* One vertex: clip-space position and a colour, both as four floats, which is
 * exactly what the pass-through vertex shader forwards. */
struct gc_vertex {
	float pos[4];
	float color[4];
};

/* Pack a colour into the target's pixel format. */
static inline uint32_t gc_rgb(uint8_t r, uint8_t g, uint8_t b)
{
	return ((uint32_t)r << 16) | ((uint32_t)g << 8) | b;
}

/* Fill the whole target with `argb` using the resolve engine. Submits and
 * returns the fence; the caller waits. */
int gc_draw_clear(const struct gc_target *t, uint32_t argb, uint32_t *fence_out);

/* Draw one triangle with the pass-through shaders, smooth shaded. Submits and
 * returns the fence. */
int gc_draw_triangle(const struct gc_target *t, const struct gc_vertex v[3],
		     uint32_t *fence_out);

/* Read one pixel back, invalidating the CPU cache first when the target is
 * cached. Returns 0x00RRGGBB; the X byte is masked off. */
uint32_t gc_target_pixel(const struct gc_target *t, uint32_t x, uint32_t y);

#endif /* GC_DRAW_H */
