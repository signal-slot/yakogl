/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * bridge_adv7535.c -- the ADV7535 DSI-to-HDMI bridge (yakogl,adv7535) as a
 * device on its I2C bus. A compiling shell: the config is filled from the
 * devicetree and nothing is programmed; the device has no API table. The
 * follow-up item adds the FFI calls over ip::adv7535: the ADV7533-mode
 * power-up and DSI receiver setup through the main/cec/edid/packet maps, the
 * mode set, and the HPD and EDID reads, all over the I2C device.
 */
#define DT_DRV_COMPAT yakogl_adv7535

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/logging/log.h>
#include <errno.h>

LOG_MODULE_REGISTER(adv7535, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

struct adv7535_cfg {
	struct i2c_dt_spec bus; /* the main map: the i2c bus and reg */
	uint8_t edid_addr;
	uint8_t cec_addr;
	uint8_t packet_addr;
	uint8_t dsi_lanes;
	bool disable_timing_generator;
	struct gpio_dt_spec hpd;
	struct gpio_dt_spec pd;
};

struct adv7535_data {
	struct k_mutex lock;
};

static int adv7535_init(const struct device *dev)
{
	struct adv7535_data *d = dev->data;

	k_mutex_init(&d->lock);
	return 0;
}

#define ADV7535_DEFINE(inst)                                                                       \
	static const struct adv7535_cfg adv7535_cfg_##inst = {                                     \
		.bus = I2C_DT_SPEC_INST_GET(inst),                                                 \
		.edid_addr = DT_INST_PROP(inst, edid_reg),                                         \
		.cec_addr = DT_INST_PROP(inst, cec_reg),                                           \
		.packet_addr = DT_INST_PROP(inst, packet_reg),                                     \
		.dsi_lanes = DT_INST_PROP(inst, adi_dsi_lanes),                                    \
		.disable_timing_generator = DT_INST_PROP(inst, adi_disable_timing_generator),      \
		.hpd = GPIO_DT_SPEC_INST_GET_OR(inst, hpd_gpios, {0}),                             \
		.pd = GPIO_DT_SPEC_INST_GET_OR(inst, pd_gpios, {0}),                               \
	};                                                                                         \
	static struct adv7535_data adv7535_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, adv7535_init, NULL, &adv7535_data_##inst, &adv7535_cfg_##inst, \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_ADV7535_INIT_PRIORITY, NULL);

DT_INST_FOREACH_STATUS_OKAY(ADV7535_DEFINE)
