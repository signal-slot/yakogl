/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dsi_yakogl_samsung.c -- the i.MX 8M Plus MIPI DSI host (Samsung DSIM core)
 * and its D-PHY as a Zephyr MIPI-DSI device: registration, the register
 * mapping and a mutex. Every register access is Rust
 * (driver/rust/src/ip/samsung_dsim.rs and ip/dphy_timing.rs, from IMX8MPRM
 * Rev. 3 13.6 / 13.7 and the MIPI D-PHY v1.2 minimums, through
 * src/ffi/imx8mp_dsi.rs).
 *
 * attach() leaves the clock lane in HS and the data lanes held in stop state;
 * yakogl_samsung_dsim_video_start() (include/safeui/samsung_dsim.h) releases
 * them -- Zephyr's MIPI-DSI API has no "start video" step (risk R5,
 * decided in driver/docs/imx8mp-display-cold.md). transfer() is not
 * supported: video mode only, and the SN65DSI84 behind it is on I2C.
 *
 * The block's power domain, clocks, resets and PLL reference are the chain
 * owner's (yakogl,imx8mp-media-pd); init maps and reads nothing.
 */
#define DT_DRV_COMPAT yakogl_samsung_dsim

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/device_mmio.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <safeui/samsung_dsim.h>
#include <safeui/yd_env.h>

LOG_MODULE_REGISTER(samsung_dsim, CONFIG_SAFEUI_DISPLAY_RUST_LOG_LEVEL);

struct dsim_cfg {
	uintptr_t phys;
	size_t size;
	struct yd_samsung_dsim_config yd;
	uint32_t pixel_clock_hz; /* pixel-clock-frequency, until set */
};

struct dsim_data {
	struct yd_samsung_dsim_data yd;
	struct yd_env env;
	struct k_mutex lock;
	uint32_t pixel_clock_hz;
};

int yakogl_samsung_dsim_set_pixel_clock(const struct device *dev, uint32_t hz)
{
	struct dsim_data *d = dev->data;

	if (hz == 0u) {
		return -EINVAL;
	}
	d->pixel_clock_hz = hz;
	return 0;
}

int yakogl_samsung_dsim_video_start(const struct device *dev)
{
	struct dsim_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_samsung_dsim_video_start(&d->yd);
	k_mutex_unlock(&d->lock);
	return rc;
}

uint32_t yakogl_samsung_dsim_health(const struct device *dev)
{
	return yd_samsung_dsim_health(&((const struct dsim_data *)dev->data)->yd);
}

const struct yd_samsung_dsim_data *yakogl_samsung_dsim_data(const struct device *dev)
{
	return &((const struct dsim_data *)dev->data)->yd;
}

static int dsim_attach(const struct device *dev, uint8_t channel, const struct mipi_dsi_device *mdev)
{
	const struct dsim_cfg *c = dev->config;
	struct dsim_data *d = dev->data;
	const struct yd_samsung_dsim_mode mode = {
		.pixel_clock_hz = d->pixel_clock_hz,
		.hactive = mdev->timings.hactive,
		.hfront = mdev->timings.hfp,
		.hsync = mdev->timings.hsync,
		.hback = mdev->timings.hbp,
		.vactive = mdev->timings.vactive,
		.vfront = mdev->timings.vfp,
		.vsync = mdev->timings.vsync,
		.vback = mdev->timings.vbp,
		.lanes = mdev->data_lanes,
	};
	struct yd_samsung_dsim_config cfg = c->yd;
	int rc;

	ARG_UNUSED(channel);
	if (mdev->pixfmt != MIPI_DSI_PIXFMT_RGB888 || (mdev->mode_flags & MIPI_DSI_MODE_VIDEO) == 0u) {
		return -ENOTSUP;
	}
	if (d->pixel_clock_hz == 0u) {
		LOG_ERR("attach: no pixel clock (pixel-clock-frequency or "
			"yakogl_samsung_dsim_set_pixel_clock)");
		return -EINVAL;
	}
	if (mdev->mode_flags & MIPI_DSI_MODE_VIDEO_BURST) {
		cfg.flags |= YD_SAMSUNG_DSIM_BURST;
	}
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_samsung_dsim_attach(&cfg, &d->yd, &d->env, &mode);
	k_mutex_unlock(&d->lock);
	return rc;
}

static ssize_t dsim_transfer(const struct device *dev, uint8_t channel, struct mipi_dsi_msg *msg)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(channel);
	ARG_UNUSED(msg);
	return -ENOTSUP;
}

static int dsim_detach(const struct device *dev, uint8_t channel, const struct mipi_dsi_device *mdev)
{
	struct dsim_data *d = dev->data;

	ARG_UNUSED(channel);
	ARG_UNUSED(mdev);
	k_mutex_lock(&d->lock, K_FOREVER);
	yd_samsung_dsim_detach(&d->yd);
	k_mutex_unlock(&d->lock);
	return 0;
}

static DEVICE_API(mipi_dsi, dsim_api) = {
	.attach = dsim_attach,
	.transfer = dsim_transfer,
	.detach = dsim_detach,
};

static int dsim_init(const struct device *dev)
{
	const struct dsim_cfg *c = dev->config;
	struct dsim_data *d = dev->data;

	k_mutex_init(&d->lock);
	device_map(&d->yd.base, c->phys, c->size, K_MEM_CACHE_NONE);
	yd_env_init(&d->env, (void *)dev);
	d->pixel_clock_hz = c->pixel_clock_hz;
	return 0;
}

#define DSIM_DEFINE(inst)                                                                          \
	static const struct dsim_cfg dsim_cfg_##inst = {                                           \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.yd = {                                                                            \
			.pll_ref_hz = DT_INST_PROP(inst, pll_ref_frequency),                       \
			.lane_rate_hz = DT_INST_PROP(inst, burst_clock_frequency),                 \
			.esc_max_hz = DT_INST_PROP(inst, esc_clock_frequency),                     \
			.lanes = DT_INST_PROP(inst, data_lanes),                                   \
			.flags = DT_INST_PROP(inst, burst_mode) ? YD_SAMSUNG_DSIM_BURST : 0u,      \
		},                                                                                 \
		.pixel_clock_hz = DT_INST_PROP(inst, pixel_clock_frequency),                       \
	};                                                                                         \
	static struct dsim_data dsim_data_##inst;                                                  \
	DEVICE_DT_INST_DEFINE(inst, dsim_init, NULL, &dsim_data_##inst, &dsim_cfg_##inst,           \
			      POST_KERNEL, CONFIG_SAFEUI_DSIM_RUST_INIT_PRIORITY, &dsim_api);

DT_INST_FOREACH_STATUS_OKAY(DSIM_DEFINE)
