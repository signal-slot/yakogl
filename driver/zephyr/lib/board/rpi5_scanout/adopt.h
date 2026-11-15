/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Route 1 of the Pi 5 display (driver/docs/rpi5-v3d71-facts.md section 10):
 * may the plane the firmware's HVS6 already scans out be adopted as the
 * screen? A pure decision on what a probe decoded from the display list:
 * no register access, no allocation, header-only C11.
 *
 * The caller passes the plane (bus address, format, pitch, width, height),
 * the DRAM range, the range the running image occupies, and the bus-to-ARM
 * translation (bus_addr.h is another item's; here it is a function pointer).
 *
 * Mapping of the output onto driver/rust/include/yakogl_drivers.h:
 *   out.phys, width, height, pitch, rows -> struct yd_rpi4_fb, same names
 *     (rows = height: one screen, as src/board/rpi_fb.rs adopt_scanout).
 *   out.size is pitch * height, the figure yd_rpi_allocate_ok() takes.
 *   out.order is the pixel-order tag's value that yd_rpi4_window_format()
 *     takes (0 BGR: B first in memory, 1 RGB). yd_rpi_adopt_scanout() takes
 *     YD_RPI_SCANOUT_FMT_* instead, with the opposite numbers (RGBX 0,
 *     BGRX 1): use rpi5_adopt_scanout_fmt().
 *   RPI5_ADOPT_ORDER_NONE (RGB565) has no 32 bpp window: do not hand it to
 *     yd_rpi_adopt_scanout().
 */
#ifndef RPI5_SCANOUT_ADOPT_H
#define RPI5_SCANOUT_ADOPT_H

#include <stddef.h>
#include <stdint.h>

enum rpi5_adopt_format {
	RPI5_ADOPT_FMT_UNKNOWN = 0,
	RPI5_ADOPT_FMT_XRGB8888, /* DRM name: B, G, R, X in memory */
	RPI5_ADOPT_FMT_ARGB8888, /* B, G, R, A in memory; alpha is ignored */
	RPI5_ADOPT_FMT_RGB565,   /* 16 bpp */
	RPI5_ADOPT_FMT_XBGR8888, /* R, G, B, X in memory */
	RPI5_ADOPT_FMT_ABGR8888  /* R, G, B, A in memory */
};

/* The plane as decoded from the HVS6 display list. */
struct rpi5_adopt_plane {
	uint64_t bus_addr; /* as the display list holds it */
	enum rpi5_adopt_format format;
	uint32_t pitch; /* bytes per row */
	uint32_t w;
	uint32_t h;
};

/* Bus to ARM physical: 1 and *arm, 0 when the address is not translatable. */
typedef int (*rpi5_adopt_bus_to_arm)(void *ctx, uint64_t bus, uint64_t *arm);

struct rpi5_adopt_env {
	uint64_t dram_base; /* ARM physical DRAM [base, base + size) */
	uint64_t dram_size;
	uint64_t reserved_base; /* the running image; size 0: none */
	uint64_t reserved_size;
	rpi5_adopt_bus_to_arm bus_to_arm; /* required */
	void *ctx;
};

enum rpi5_adopt_verdict {
	RPI5_ADOPT = 0,
	RPI5_ADOPT_REJECT
};

#define RPI5_ADOPT_ORDER_BGR 0u /* the mailbox pixel-order tag's value */
#define RPI5_ADOPT_ORDER_RGB 1u
#define RPI5_ADOPT_ORDER_NONE 2u

/* Fields of struct yd_rpi4_fb, then the rest. */
struct rpi5_adopt_out {
	uint64_t phys; /* ARM physical */
	uint32_t width;
	uint32_t height;
	uint32_t pitch;
	uint32_t rows; /* = height */
	uint64_t size; /* pitch * height */
	uint32_t bpp;  /* bytes per pixel: 4 or 2 */
	uint32_t order; /* RPI5_ADOPT_ORDER_* */
};

struct rpi5_adopt_result {
	enum rpi5_adopt_verdict verdict;
	const char *reason; /* short, never NULL */
};

static inline uint32_t rpi5_adopt_bpp(enum rpi5_adopt_format f)
{
	switch (f) {
	case RPI5_ADOPT_FMT_XRGB8888:
	case RPI5_ADOPT_FMT_ARGB8888:
	case RPI5_ADOPT_FMT_XBGR8888:
	case RPI5_ADOPT_FMT_ABGR8888:
		return 4;
	case RPI5_ADOPT_FMT_RGB565:
		return 2;
	default:
		return 0;
	}
}

/* The value yd_rpi_adopt_scanout() takes (YD_RPI_SCANOUT_FMT_RGBX 0 / BGRX 1)
 * for an order, or UINT32_MAX for RPI5_ADOPT_ORDER_NONE. */
static inline uint32_t rpi5_adopt_scanout_fmt(uint32_t order)
{
	return order == RPI5_ADOPT_ORDER_RGB ? 0u : order == RPI5_ADOPT_ORDER_BGR ? 1u : UINT32_MAX;
}

static inline struct rpi5_adopt_result rpi5_adopt__rej(const char *why)
{
	struct rpi5_adopt_result r = { RPI5_ADOPT_REJECT, why };
	return r;
}

/* out is written on RPI5_ADOPT only. */
static inline struct rpi5_adopt_result rpi5_adopt_check(const struct rpi5_adopt_plane *p,
							const struct rpi5_adopt_env *env,
							struct rpi5_adopt_out *out)
{
	struct rpi5_adopt_result ok = { RPI5_ADOPT, "adopt" };
	uint32_t bpp;
	uint64_t arm = 0, size, end, dram_end;

	if (!p || !env || !out || !env->bus_to_arm)
		return rpi5_adopt__rej("bad argument");
	bpp = rpi5_adopt_bpp(p->format);
	if (bpp == 0)
		return rpi5_adopt__rej("unknown format");
	if (p->w == 0 || p->h == 0)
		return rpi5_adopt__rej("zero size");
	if ((uint64_t)p->pitch < (uint64_t)p->w * bpp)
		return rpi5_adopt__rej("pitch below w*bpp");
	if (p->pitch % bpp != 0)
		return rpi5_adopt__rej("pitch not whole pixels");
	if (p->bus_addr == 0)
		return rpi5_adopt__rej("no address");
	if (!env->bus_to_arm(env->ctx, p->bus_addr, &arm))
		return rpi5_adopt__rej("bus address not translatable");

	size = (uint64_t)p->pitch * p->h; /* both 32 bit: no overflow */
	if (arm + size < arm)
		return rpi5_adopt__rej("buffer outside DRAM");
	end = arm + size;
	dram_end = env->dram_base + env->dram_size;
	if (dram_end < env->dram_base || arm < env->dram_base || end > dram_end)
		return rpi5_adopt__rej("buffer outside DRAM");
	if (env->reserved_size != 0) {
		uint64_t rend = env->reserved_base + env->reserved_size;

		if (rend < env->reserved_base)
			rend = UINT64_MAX;
		if (arm < rend && env->reserved_base < end)
			return rpi5_adopt__rej("buffer overlaps the image");
	}

	out->phys = arm;
	out->width = p->w;
	out->height = p->h;
	out->pitch = p->pitch;
	out->rows = p->h;
	out->size = size;
	out->bpp = bpp;
	switch (p->format) {
	case RPI5_ADOPT_FMT_XBGR8888:
	case RPI5_ADOPT_FMT_ABGR8888:
		out->order = RPI5_ADOPT_ORDER_RGB;
		break;
	case RPI5_ADOPT_FMT_RGB565:
		out->order = RPI5_ADOPT_ORDER_NONE;
		break;
	default:
		out->order = RPI5_ADOPT_ORDER_BGR;
		break;
	}
	return ok;
}

#endif
