/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * AM62P DSS (DSS7 "K3" generation) driver for the R5F platform firmware.
 *
 * One video port drives the panel (VP1 -> DSI on DSS1).  Two planes sit in
 * its overlay manager:
 *   - layer 0: "vid"   - the application frame handed over by the A53 (RGB565)
 *   - layer 1: "vidl1" - the safety layer owned by this core (ARGB8888)
 * The hardware alpha blender composites them, so the A53 can stop or hand
 * over garbage without affecting the safety layer.
 */
#ifndef YAKOGL_DRV_AM62P_DSS_H_
#define YAKOGL_DRV_AM62P_DSS_H_

#include <zephyr/device.h>
#include <zephyr/kernel.h>
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

/* DSS pixel format codes (DISPC_VID_ATTRIBUTES.FORMAT) */
#define AM62P_DSS_FMT_RGB565    0x3
#define AM62P_DSS_FMT_ARGB8888  0x7
#define AM62P_DSS_FMT_XRGB8888  0x27

struct am62p_dss_plane_cfg {
	uint32_t seq;          /* frame sequence number, reported back as "shown" */
	uintptr_t pa;           /* physical address of the first pixel */
	uint32_t width;
	uint32_t height;
	uint32_t stride_bytes;
	uint32_t format;        /* AM62P_DSS_FMT_* */
};

struct am62p_dss_stats {
	uint32_t vsyncs;
	uint32_t app_frames;
	uint32_t underflows;
	uint32_t sync_lost;
	uint32_t go_timeouts;
	uint32_t go_waits;     /* commits that rode on an already pending GO */
	uint32_t safety_flips; /* safety layer buffer flips */
	bool app_plane_enabled;
};

/* Buffer of the safety layer (ARGB8888, width*height), written by this core. */
/* Present the buffer last returned by am62p_dss_safety_buffer() at the next vsync;
 * afterwards am62p_dss_safety_buffer() returns the other buffer.
 */
int am62p_dss_safety_flip(const struct device *dev);
/* The safety layer's pixels are premultiplied by their alpha (Slint's software
 * renderer writes them that way); default: straight alpha. */
int am62p_dss_safety_premultiplied(const struct device *dev, bool on);
/* false while the last flip has not been latched yet, i.e. the buffer
 * am62p_dss_safety_buffer() returns is still on screen: do not draw into it.
 */
bool am62p_dss_safety_back_free(const struct device *dev);
void *am62p_dss_safety_buffer(const struct device *dev, uint32_t *width, uint32_t *height,
			      uint32_t *stride_bytes);
/* Point the application plane at a new buffer (takes effect at the next vsync). */
int am62p_dss_set_app_buffer(const struct device *dev, const struct am62p_dss_plane_cfg *cfg);
/* Which of the n buffers at pa[] the application plane may still scan: bit k
 * for pa[k] (n <= 32). The buffer it scans and the one waiting for the next
 * vsync are busy; one replaced before its vsync is not. Too large is
 * possible, too small never. Never waits. Call it from the thread that calls
 * am62p_dss_set_app_buffer. */
uint32_t am62p_dss_app_busy(const struct device *dev, const uint64_t *pa, size_t n);
int am62p_dss_enable_app_plane(const struct device *dev, bool enable);
/* Start / stop the video port. */
int am62p_dss_output_enable(const struct device *dev, bool enable);
int am62p_dss_wait_vsync(const struct device *dev, k_timeout_t timeout);

/* Called from the DSS interrupt on every vertical sync of the output port. */
/* seq of the application frame currently scanned out (exact: updated when the
 * hardware latched the commit at a vsync) */
uint32_t am62p_dss_app_seq_shown(const struct device *dev);
typedef void (*am62p_dss_vsync_cb_t)(const struct device *dev, void *user);
void am62p_dss_set_vsync_callback(const struct device *dev, am62p_dss_vsync_cb_t cb, void *user);
void am62p_dss_get_stats(const struct device *dev, struct am62p_dss_stats *out);

#endif
