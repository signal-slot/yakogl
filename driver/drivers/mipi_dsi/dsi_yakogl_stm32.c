/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dsi_yakogl_stm32.c -- the STM32MP25 MIPI DSI host (DesignWare host plus ST's
 * D-PHY wrapper) as a Zephyr MIPI-DSI device (yakogl,stm32-dsi). A compiling
 * shell: the config is filled from the devicetree and the API returns
 * -ENOSYS. The follow-up item adds the FFI calls over ip::dw_mipi_dsi and
 * ip::stm32_dsi_phy: the PLL and lane setup, the video-mode packet and timing
 * configuration at attach, and the detach.
 */
#define DT_DRV_COMPAT yakogl_stm32_dsi

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/logging/log.h>
#include <errno.h>

LOG_MODULE_REGISTER(stm32_dsi, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

struct mp2_dsi_cfg {
	uintptr_t phys;
	size_t size;
	uint8_t lanes;
	uint32_t pll_ref_hz;
	uint32_t lane_rate_hz;
	uint32_t pixel_clock_hz;
	const struct device *bridge; /* NULL: none */
};

struct mp2_dsi_data {
	struct k_mutex lock;
};

static int mp2_dsi_attach(const struct device *dev, uint8_t channel,
			  const struct mipi_dsi_device *mdev)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(channel);
	ARG_UNUSED(mdev);
	return -ENOSYS;
}

static ssize_t mp2_dsi_transfer(const struct device *dev, uint8_t channel,
				struct mipi_dsi_msg *msg)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(channel);
	ARG_UNUSED(msg);
	return -ENOSYS;
}

static int mp2_dsi_detach(const struct device *dev, uint8_t channel,
			  const struct mipi_dsi_device *mdev)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(channel);
	ARG_UNUSED(mdev);
	return -ENOSYS;
}

static DEVICE_API(mipi_dsi, mp2_dsi_api) = {
	.attach = mp2_dsi_attach,
	.transfer = mp2_dsi_transfer,
	.detach = mp2_dsi_detach,
};

static int mp2_dsi_init(const struct device *dev)
{
	struct mp2_dsi_data *d = dev->data;

	k_mutex_init(&d->lock);
	return 0;
}

#define MP2_DSI_BRIDGE(inst)                                                                       \
	COND_CODE_1(DT_INST_NODE_HAS_PROP(inst, bridge),                                           \
		    (DEVICE_DT_GET(DT_INST_PHANDLE(inst, bridge))), (NULL))

#define MP2_DSI_DEFINE(inst)                                                                       \
	static const struct mp2_dsi_cfg mp2_dsi_cfg_##inst = {                                     \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.lanes = DT_INST_PROP(inst, data_lanes),                                           \
		.pll_ref_hz = DT_INST_PROP(inst, pll_ref_frequency),                               \
		.lane_rate_hz = DT_INST_PROP(inst, burst_clock_frequency),                         \
		.pixel_clock_hz = DT_INST_PROP(inst, pixel_clock_frequency),                       \
		.bridge = MP2_DSI_BRIDGE(inst),                                                    \
	};                                                                                         \
	static struct mp2_dsi_data mp2_dsi_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, mp2_dsi_init, NULL, &mp2_dsi_data_##inst, &mp2_dsi_cfg_##inst, \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_DSI_STM32_INIT_PRIORITY,              \
			      &mp2_dsi_api);

DT_INST_FOREACH_STATUS_OKAY(MP2_DSI_DEFINE)
