/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The LCDIF half of display_internal.h over the Rust LCDIFv3
 * (CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST; built instead of imx8mp_lcdif.c):
 * configure / enable / disable go to the display node's `lcdif`
 * (yakogl,imx8mp-lcdif, driver/drivers/display/display_yakogl_imx8mp_lcdif.c),
 * which owns the LCDIF's devicetree facts (INV_PXCK, panic thresholds,
 * interrupt); the reads and the flip of a running layer stay on the display
 * node's own "lcdif" mapping, so imx8mp_display.c's adopt path and health word
 * do not depend on that device's state. Register logic:
 * driver/rust/src/ip/lcdifv3.rs. driver/docs/imx8mp-display-cold.md.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <safeui/imx8mp_lcdif.h>
#include <safeui/yd_env.h>

#include "display_internal.h"

LOG_MODULE_DECLARE(imx8mp_disp, CONFIG_SAFEUI_DISPLAY_IMX8MP_LOG_LEVEL);

#define DISP_NODE DT_COMPAT_GET_ANY_STATUS_OKAY(safeui_imx8mp_display)

BUILD_ASSERT(DT_NODE_HAS_PROP(DISP_NODE, lcdif),
	     "CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST: the display node needs lcdif = <&...>");
BUILD_ASSERT(DT_NODE_HAS_COMPAT(DT_PHANDLE(DISP_NODE, lcdif), yakogl_imx8mp_lcdif),
	     "lcdif: a yakogl,imx8mp-lcdif node");
BUILD_ASSERT(DT_REG_ADDR(DT_PHANDLE(DISP_NODE, lcdif)) == DT_REG_ADDR_BY_NAME(DISP_NODE, lcdif),
	     "lcdif: the same LCDIF as the display node's \"lcdif\" register range");
BUILD_ASSERT(!DT_PROP(DT_PHANDLE(DISP_NODE, lcdif), adopt_only),
	     "lcdif: the chain owner configures it; adopt-only is for chains it does not own");

#define LCDIF_DEV DEVICE_DT_GET(DT_PHANDLE(DISP_NODE, lcdif))

bool lcdif_is_running(mm_reg_t lcdif)
{
	return yd_lcdif_is_running(lcdif);
}

uint32_t lcdif_layer_control(mm_reg_t lcdif)
{
	return yd_lcdif_layer_control(lcdif);
}

bool lcdif_read_scanout(mm_reg_t lcdif, uint32_t *w, uint32_t *h, uint32_t *pitch,
			uint32_t *bytes_pp, uint64_t *phys)
{
	struct yd_lcdif_scanout s;

	if (!yd_lcdif_read_scanout(lcdif, &s)) {
		return false;
	}
	*w = s.width;
	*h = s.height;
	*pitch = s.pitch;
	*bytes_pp = s.bytes_pp;
	*phys = s.phys;
	return true;
}

int lcdif_set_fb(mm_reg_t lcdif, uint64_t fb_phys)
{
	struct yd_env env;

	yd_env_init(&env, NULL);
	return yd_lcdif_flip(lcdif, &env, fb_phys);
}

int lcdif_configure(struct display_ctx *c, uint64_t fb_phys, uint32_t pitch)
{
	const struct display_mode *m = &c->mode;
	const struct yd_lcdif_mode mode = {
		.pixel_clock_hz = c->info.pixel_clock_hz,
		.hactive = m->hactive, .hfront = m->hfront, .hsync = m->hsync, .hback = m->hback,
		.vactive = m->vactive, .vfront = m->vfront, .vsync = m->vsync, .vback = m->vback,
	};

	if (!device_is_ready(LCDIF_DEV)) {
		return -ENODEV;
	}
	return yakogl_imx8mp_lcdif_configure(LCDIF_DEV, &mode, fb_phys, pitch);
}

int lcdif_enable(struct display_ctx *c)
{
	ARG_UNUSED(c);
	return yakogl_imx8mp_lcdif_enable(LCDIF_DEV);
}

void lcdif_disable(struct display_ctx *c)
{
	struct yd_lcdif_data d = {.base = c->lcdif, .on = 1};
	struct yd_env env;

	if (yakogl_imx8mp_lcdif_data(LCDIF_DEV)->on) {
		(void)yakogl_imx8mp_lcdif_disable(LCDIF_DEV);
		return;
	}
	/* A layer the device did not start (the facade adopted the chain the
	 * previous kernel left, FORCE_COLD tears it down): stop it on the
	 * display node's mapping, as imx8mp_lcdif.c did -- a cold stop (layer
	 * off at a frame boundary, display off), not an adopted hand-back. */
	LOG_DBG("LCDIF: stopping a layer this driver did not start");
	yd_env_init(&env, NULL);
	(void)yd_lcdif_release(&d, &env);
}
