/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The DSIM half of display_internal.h over the Rust Samsung DSIM
 * (CONFIG_SAFEUI_DSIM_RUST; built instead of imx8mp_dsim.c): the display
 * node's `dsi` (yakogl,samsung-dsim,
 * driver/drivers/mipi_dsi/dsi_yakogl_samsung.c) through Zephyr's MIPI-DSI
 * API plus the driver-specific start of include/safeui/samsung_dsim.h.
 *
 * The order stays imx8mp_display.c's: dsim_enable() is mipi_dsi_attach(),
 * which returns with the clock lane in HS and the data lanes held; the
 * bridge is programmed; the LCDIF is enabled; dsim_video_start() releases the
 * data lanes (risk R5). Register logic: driver/rust/src/ip/samsung_dsim.rs.
 * driver/docs/imx8mp-display-cold.md.
 */
#include <zephyr/kernel.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <safeui/samsung_dsim.h>

#include "display_internal.h"

LOG_MODULE_DECLARE(imx8mp_disp, CONFIG_SAFEUI_DISPLAY_IMX8MP_LOG_LEVEL);

#define DISP_NODE DT_COMPAT_GET_ANY_STATUS_OKAY(safeui_imx8mp_display)
#define DSI_NODE  DT_PHANDLE(DISP_NODE, dsi)

BUILD_ASSERT(DT_NODE_HAS_PROP(DISP_NODE, dsi),
	     "CONFIG_SAFEUI_DSIM_RUST: the display node needs dsi = <&...>");
BUILD_ASSERT(DT_NODE_HAS_COMPAT(DSI_NODE, yakogl_samsung_dsim), "dsi: a yakogl,samsung-dsim node");
BUILD_ASSERT(DT_REG_ADDR(DSI_NODE) == DT_REG_ADDR_BY_NAME(DISP_NODE, dsim),
	     "dsi: the same DSIM as the display node's \"dsim\" register range");
BUILD_ASSERT(DT_PROP(DSI_NODE, pll_ref_frequency) == DT_PROP(DISP_NODE, pll_ref_frequency),
	     "dsi: pll-ref-frequency must equal the display node's (it programs the root)");
BUILD_ASSERT(DT_PROP(DSI_NODE, burst_clock_frequency) == DT_PROP(DISP_NODE, burst_clock_frequency),
	     "dsi: burst-clock-frequency must equal the display node's");
BUILD_ASSERT(DT_PROP(DSI_NODE, data_lanes) == DT_PROP(DISP_NODE, dsi_lanes),
	     "dsi: data-lanes must equal the display node's dsi-lanes");

#define DSI_DEV DEVICE_DT_GET(DSI_NODE)

static struct mipi_dsi_device mdev_of(const struct display_ctx *c)
{
	const struct display_mode *m = &c->mode;
	struct mipi_dsi_device d = {
		.data_lanes = c->lanes,
		.pixfmt = MIPI_DSI_PIXFMT_RGB888,
		.mode_flags = MIPI_DSI_MODE_VIDEO | MIPI_DSI_MODE_VIDEO_BURST,
	};

	d.timings.hactive = m->hactive;
	d.timings.hfp = m->hfront;
	d.timings.hbp = m->hback;
	d.timings.hsync = m->hsync;
	d.timings.vactive = m->vactive;
	d.timings.vfp = m->vfront;
	d.timings.vbp = m->vback;
	d.timings.vsync = m->vsync;
	return d;
}

int dsim_enable(struct display_ctx *c)
{
	struct mipi_dsi_device mdev = mdev_of(c);
	const struct yd_samsung_dsim_data *d;
	int rc;

	if (!device_is_ready(DSI_DEV)) {
		return -ENODEV;
	}
	rc = yakogl_samsung_dsim_set_pixel_clock(DSI_DEV, c->info.pixel_clock_hz);
	if (rc == 0) {
		rc = mipi_dsi_attach(DSI_DEV, 0, &mdev);
	}
	d = yakogl_samsung_dsim_data(DSI_DEV);
	if (rc) {
		LOG_ERR("DSIM: attach failed in step %u (%d)", d->step, rc);
		return rc;
	}
	c->info.pll_p = (uint8_t)d->pll_p;
	c->info.pll_m = (uint16_t)d->pll_m;
	c->info.pll_s = (uint8_t)d->pll_s;
	c->info.dsi_hs_clock_hz = d->lane_rate_hz;
	c->info.dsi_byte_clock_hz = d->byte_hz;
	c->info.dsi_esc_clock_hz = d->esc_hz;
	return 0;
}

void dsim_video_start(struct display_ctx *c)
{
	ARG_UNUSED(c);
	(void)yakogl_samsung_dsim_video_start(DSI_DEV);
}

void dsim_disable(struct display_ctx *c)
{
	struct mipi_dsi_device mdev = mdev_of(c);

	(void)mipi_dsi_detach(DSI_DEV, 0, &mdev);
}

bool dsim_pll_is_stable(const struct display_ctx *c)
{
	ARG_UNUSED(c);
	return (yakogl_samsung_dsim_health(DSI_DEV) & YD_SAMSUNG_DSIM_H_PLL_STABLE) != 0u;
}

bool dsim_hs_clock_is_ready(const struct display_ctx *c)
{
	ARG_UNUSED(c);
	return (yakogl_samsung_dsim_health(DSI_DEV) & YD_SAMSUNG_DSIM_H_HS_CLOCK) != 0u;
}
