/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dsi_yakogl_stm32.c -- the STM32MP25 MIPI DSI host (DesignWare host plus ST's
 * D-PHY wrapper) as a Zephyr MIPI-DSI device (yakogl,stm32-dsi):
 * registration, the register mapping, the Zephyr-to-ABI translation and a
 * mutex. Every register access is Rust (driver/rust/src/ip/dw_mipi_dsi.rs and
 * ip/stm32_dsi_phy.rs, through src/ffi/mp25_display.rs and
 * include/yakogl_mp25_display.h).
 *
 * attach() is the whole bring-up, in the order of that header: probe the
 * block, attach the device (lanes, format, mode flags), enable (PHY PLL, host
 * registers from the device's timings, command mode, then video mode with the
 * clock lane in HS). The chain owner therefore calls it with the LTDC already
 * scanning, and starts the ADV7535 (adv7535_start()) only afterwards.
 * detach() disables the host (command mode, PLL and DSIEN off, host and PHY
 * in reset).
 *
 * transfer() returns -ENOTSUP: video mode only; the ADV7535 behind the host is
 * programmed over I2C, and the host's command FIFOs are not driven.
 *
 * Not this driver's: the block's clocks (CK_BUS_DSI, CK_KER_DSIPHY =
 * pll-ref-frequency), reset, SCMI requests, RIF settings and pins (the board
 * layer's); init maps the registers and reads nothing.
 *
 * The pixel clock and sync polarities are not in struct mipi_dsi_device:
 * stm32_dsi_set_mode() (include/yakogl_drv/stm32_dsi.h) gives them, else the
 * node's pixel-clock-frequency and active-high syncs apply.
 */
#define DT_DRV_COMPAT yakogl_stm32_dsi

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/device_mmio.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <yakogl_mp25_display.h>
#include <yakogl_drv/stm32_dsi.h>
#include <yakogl_drv/yd_env.h>

LOG_MODULE_REGISTER(stm32_dsi, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

struct mp2_dsi_cfg {
	uintptr_t phys;
	size_t size;
	struct yd_mp25_dsi_config yd; /* base is filled in data->yd_cfg at init */
	bool bridge_is_adv7535;       /* the bridge phandle is a yakogl,adv7535 node */
};

struct mp2_dsi_data {
	struct yd_mp25_dsi_config yd_cfg; /* the cfg with the mapped base */
	struct yd_mp25_dsi_data yd;
	struct yd_env env;
	struct k_mutex lock;
	uint32_t pixel_clock_hz;          /* node's pixel-clock-frequency, or set_mode's */
	uint32_t sync_flags;              /* YD_MP25_MODE_* polarities */
};

/* Zephyr's MIPI_DSI_PIXFMT_* (dt-bindings/mipi_dsi/mipi_dsi.h) to the ABI's
 * YD_MP25_DSI_FMT_*. The numbers happen to agree today; the switch does not
 * rely on it. */
static int mp2_dsi_format(uint32_t pixfmt, uint32_t *fmt)
{
	switch (pixfmt) {
	case MIPI_DSI_PIXFMT_RGB888:
		*fmt = YD_MP25_DSI_FMT_RGB888;
		return 0;
	case MIPI_DSI_PIXFMT_RGB666:
		*fmt = YD_MP25_DSI_FMT_RGB666;
		return 0;
	case MIPI_DSI_PIXFMT_RGB666_PACKED:
		*fmt = YD_MP25_DSI_FMT_RGB666_PACKED;
		return 0;
	case MIPI_DSI_PIXFMT_RGB565:
		*fmt = YD_MP25_DSI_FMT_RGB565;
		return 0;
	default:
		return -EINVAL;
	}
}

/* Zephyr's MIPI_DSI_MODE_* / MIPI_DSI_CLOCK_* to the ABI's YD_MP25_DSI_MODE_*
 * (Linux's bits). Bit 9 is the one that differs in meaning: Zephyr's
 * MIPI_DSI_MODE_EOT_PACKET sends an EoT packet, Linux's
 * MIPI_DSI_MODE_NO_EOT_PACKET (the ABI's) suppresses it, so the ABI bit is
 * set when Zephyr's is clear. Modes the DesignWare host takes no input from
 * (AUTO_VERT, HFP, HBP, HSA, VSYNC_FLUSH, LPM) are not translated. Video mode
 * is required (-ENOTSUP). */
static int mp2_dsi_mode_flags(uint32_t zflags, uint32_t *flags)
{
	uint32_t f = 0;

	if ((zflags & MIPI_DSI_MODE_VIDEO) == 0u) {
		return -ENOTSUP;
	}
	f |= YD_MP25_DSI_MODE_VIDEO;
	if (zflags & MIPI_DSI_MODE_VIDEO_BURST) {
		f |= YD_MP25_DSI_MODE_VIDEO_BURST;
	}
	if (zflags & MIPI_DSI_MODE_VIDEO_SYNC_PULSE) {
		f |= YD_MP25_DSI_MODE_VIDEO_SYNC_PULSE;
	}
	if (zflags & MIPI_DSI_MODE_VIDEO_HSE) {
		f |= YD_MP25_DSI_MODE_VIDEO_HSE;
	}
	if (zflags & MIPI_DSI_CLOCK_NON_CONTINUOUS) {
		f |= YD_MP25_DSI_CLOCK_NON_CONTINUOUS;
	}
	if ((zflags & MIPI_DSI_MODE_EOT_PACKET) == 0u) {
		f |= YD_MP25_DSI_MODE_NO_EOT_PACKET;
	}
	*flags = f;
	return 0;
}

static int mp2_dsi_attach(const struct device *dev, uint8_t channel,
			  const struct mipi_dsi_device *mdev)
{
	const struct mp2_dsi_cfg *c = dev->config;
	struct mp2_dsi_data *d = dev->data;
	struct yd_mp25_mode mode;
	uint32_t fmt;
	uint32_t flags;
	int rc;

	ARG_UNUSED(channel);
	if (mdev == NULL) {
		return -EINVAL;
	}
	rc = mp2_dsi_format(mdev->pixfmt, &fmt);
	if (rc) {
		LOG_ERR("attach: pixel format %u not supported", mdev->pixfmt);
		return rc;
	}
	rc = mp2_dsi_mode_flags(mdev->mode_flags, &flags);
	if (rc) {
		LOG_ERR("attach: video mode required (flags 0x%x)", mdev->mode_flags);
		return rc;
	}
	if (c->bridge_is_adv7535) {
		/* What adv7533.c asks of the host: video, sync pulses, no EoT, HSE. */
		flags |= YD_MP25_DSI_MODE_ADV7535;
	}
	if (d->pixel_clock_hz == 0u) {
		LOG_ERR("attach: no pixel clock (pixel-clock-frequency or stm32_dsi_set_mode)");
		return -EINVAL;
	}
	mode = (struct yd_mp25_mode){
		.pixel_clock_hz = d->pixel_clock_hz,
		.hactive = mdev->timings.hactive,
		.hfront = mdev->timings.hfp,
		.hsync = mdev->timings.hsync,
		.hback = mdev->timings.hbp,
		.vactive = mdev->timings.vactive,
		.vfront = mdev->timings.vfp,
		.vsync = mdev->timings.vsync,
		.vback = mdev->timings.vbp,
		.flags = d->sync_flags,
	};
	d->yd_cfg.pixel_clock_hz = d->pixel_clock_hz;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_mp25_dsi_probe(&d->yd_cfg, &d->env);
	if (rc < 0) {
		LOG_ERR("DSI host does not answer (%d): clock, reset or RIF", rc);
		goto out;
	}
	rc = yd_mp25_dsi_attach(&d->yd_cfg, &d->yd, mdev->data_lanes, fmt, flags);
	if (rc < 0) {
		goto out;
	}
	rc = yd_mp25_dsi_enable(&d->yd_cfg, &d->yd, &d->env, &mode);
	if (rc < 0) {
		LOG_ERR("DSI enable failed (%d) at step %u (errno %d)", rc, d->yd.failed_step,
			d->yd.failed_errno);
		yd_mp25_dsi_disable(&d->yd_cfg, &d->yd);
	}
out:
	k_mutex_unlock(&d->lock);
	return rc;
}

/* -ENOTSUP: video mode only (see the file comment). */
static ssize_t mp2_dsi_transfer(const struct device *dev, uint8_t channel,
				struct mipi_dsi_msg *msg)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(channel);
	ARG_UNUSED(msg);
	return -ENOTSUP;
}

static int mp2_dsi_detach(const struct device *dev, uint8_t channel,
			  const struct mipi_dsi_device *mdev)
{
	struct mp2_dsi_data *d = dev->data;
	int rc;

	ARG_UNUSED(channel);
	ARG_UNUSED(mdev);
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_mp25_dsi_disable(&d->yd_cfg, &d->yd);
	k_mutex_unlock(&d->lock);
	return rc;
}

static DEVICE_API(mipi_dsi, mp2_dsi_api) = {
	.attach = mp2_dsi_attach,
	.transfer = mp2_dsi_transfer,
	.detach = mp2_dsi_detach,
};

/* dev->api of every instance is mp2_dsi_api: how the extra calls recognise one. */
static bool is_dsi(const struct device *dev)
{
	return dev != NULL && dev->api == &mp2_dsi_api && device_is_ready(dev);
}

int stm32_dsi_set_mode(const struct device *dev, uint32_t pixel_clock_hz, uint32_t mode_flags)
{
	struct mp2_dsi_data *d;

	if (!is_dsi(dev)) {
		return -ENODEV;
	}
	if (pixel_clock_hz == 0u) {
		return -EINVAL;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	if (d->yd.attached) {
		k_mutex_unlock(&d->lock);
		return -EBUSY;
	}
	d->pixel_clock_hz = pixel_clock_hz;
	d->sync_flags = mode_flags & (YD_MP25_MODE_HSYNC_LOW | YD_MP25_MODE_VSYNC_LOW |
				      YD_MP25_MODE_DE_LOW);
	k_mutex_unlock(&d->lock);
	return 0;
}

const struct yd_mp25_dsi_data *stm32_dsi_data(const struct device *dev)
{
	return is_dsi(dev) ? &((struct mp2_dsi_data *)dev->data)->yd : NULL;
}

static int mp2_dsi_init(const struct device *dev)
{
	const struct mp2_dsi_cfg *c = dev->config;
	struct mp2_dsi_data *d = dev->data;

	k_mutex_init(&d->lock);
	d->yd_cfg = c->yd;
	device_map(&d->yd_cfg.base, c->phys, c->size, K_MEM_CACHE_NONE);
	yd_env_init(&d->env, (void *)dev);
	d->pixel_clock_hz = c->yd.pixel_clock_hz;
	return 0;
}

#define MP2_DSI_BRIDGE_IS_ADV7535(inst)                                                            \
	COND_CODE_1(DT_INST_NODE_HAS_PROP(inst, bridge),                                           \
		    (DT_NODE_HAS_COMPAT(DT_INST_PHANDLE(inst, bridge), yakogl_adv7535)), (false))

#define MP2_DSI_DEFINE(inst)                                                                       \
	static const struct mp2_dsi_cfg mp2_dsi_cfg_##inst = {                                     \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.yd = {                                                                            \
			.lanes = DT_INST_PROP(inst, data_lanes),                                   \
			.pll_ref_hz = DT_INST_PROP(inst, pll_ref_frequency),                       \
			.lane_rate_hz = DT_INST_PROP(inst, burst_clock_frequency),                 \
			.pixel_clock_hz = DT_INST_PROP(inst, pixel_clock_frequency),               \
		},                                                                                 \
		.bridge_is_adv7535 = MP2_DSI_BRIDGE_IS_ADV7535(inst),                              \
	};                                                                                         \
	static struct mp2_dsi_data mp2_dsi_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, mp2_dsi_init, NULL, &mp2_dsi_data_##inst, &mp2_dsi_cfg_##inst, \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_DSI_STM32_INIT_PRIORITY,              \
			      &mp2_dsi_api);

DT_INST_FOREACH_STATUS_OKAY(MP2_DSI_DEFINE)
