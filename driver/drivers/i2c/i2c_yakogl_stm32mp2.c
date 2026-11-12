/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * i2c_yakogl_stm32mp2.c -- an STM32MP25 I2C controller as a polled Zephyr I2C
 * master (yakogl,stm32mp2-i2c). Registration, the register mapping and a
 * mutex; every register access is Rust (driver/rust/src/ip/stm32_i2c.rs,
 * through src/ffi/mp25_i2c.rs and include/yakogl_mp25_i2c.h).
 *
 * The clock gate, the reset, the pins and the RIF setting of the controller
 * are not this driver's: the board layer (the display chain's owner) sets
 * them before the first use, and the A35 port has no pinctrl, so the node
 * carries no pinctrl-0. The kernel clock rate is the devicetree's
 * kernel-clock-frequency, as the shim has no clock driver.
 *
 * Transfers are polled. The ABI takes one write, or a write then a read; a
 * Zephyr message list is cut into such transactions at each I2C_MSG_STOP, and
 * a transaction of any other shape (a read alone, three messages, a write
 * followed by a write without a restart) fails with -ENOTSUP. 10-bit
 * addresses are not supported.
 */
#define DT_DRV_COMPAT yakogl_stm32mp2_i2c

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/device_mmio.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <yakogl_mp25_i2c.h>
#include <yakogl_drv/yd_env.h>

LOG_MODULE_REGISTER(stm32mp2_i2c, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

BUILD_ASSERT(sizeof(struct i2c_msg) == sizeof(struct yd_mp25_i2c_msg),
	     "i2c_msg layout differs from yd_mp25_i2c_msg");
BUILD_ASSERT(I2C_MSG_WRITE == YD_MP25_I2C_MSG_WRITE && I2C_MSG_READ == YD_MP25_I2C_MSG_READ &&
		     I2C_MSG_STOP == YD_MP25_I2C_MSG_STOP &&
		     I2C_MSG_RESTART == YD_MP25_I2C_MSG_RESTART,
	     "I2C_MSG_* flags differ from YD_MP25_I2C_MSG_*");

struct mp2_i2c_cfg {
	uintptr_t phys;
	size_t size;
	struct yd_mp25_i2c_config yd; /* base is filled in data->yd_cfg at init */
};

struct mp2_i2c_data {
	struct yd_mp25_i2c_config yd_cfg; /* the cfg with the mapped base, and the speed in force */
	struct yd_mp25_i2c_data yd;
	struct yd_env env;
	struct k_mutex lock;
};

static int mp2_i2c_speed_hz(uint32_t dev_config, uint32_t *hz)
{
	switch (I2C_SPEED_GET(dev_config)) {
	case I2C_SPEED_STANDARD:
		*hz = 100000;
		return 0;
	case I2C_SPEED_FAST:
		*hz = 400000;
		return 0;
	case I2C_SPEED_FAST_PLUS:
		*hz = 1000000;
		return 0;
	default:
		return -ENOTSUP;
	}
}

static int mp2_i2c_configure(const struct device *dev, uint32_t dev_config)
{
	struct mp2_i2c_data *d = dev->data;
	uint32_t hz;
	int rc;

	if ((dev_config & I2C_MODE_CONTROLLER) == 0U || (dev_config & I2C_ADDR_10_BITS) != 0U) {
		return -ENOTSUP;
	}
	rc = mp2_i2c_speed_hz(dev_config, &hz);
	if (rc != 0) {
		return rc;
	}

	k_mutex_lock(&d->lock, K_FOREVER);
	if (hz != d->yd_cfg.speed_hz) {
		struct yd_mp25_i2c_config next = d->yd_cfg;

		next.speed_hz = hz;
		rc = yd_mp25_i2c_init(&next, &d->yd, &d->env);
		if (rc == 0) {
			d->yd_cfg = next;
		}
	}
	k_mutex_unlock(&d->lock);
	return rc;
}

static int mp2_i2c_get_config(const struct device *dev, uint32_t *dev_config)
{
	struct mp2_i2c_data *d = dev->data;

	switch (d->yd_cfg.speed_hz) {
	case 100000:
		*dev_config = I2C_MODE_CONTROLLER | I2C_SPEED_SET(I2C_SPEED_STANDARD);
		return 0;
	case 400000:
		*dev_config = I2C_MODE_CONTROLLER | I2C_SPEED_SET(I2C_SPEED_FAST);
		return 0;
	case 1000000:
		*dev_config = I2C_MODE_CONTROLLER | I2C_SPEED_SET(I2C_SPEED_FAST_PLUS);
		return 0;
	default:
		return -ENOTSUP;
	}
}

static int mp2_i2c_transfer(const struct device *dev, struct i2c_msg *msgs, uint8_t num_msgs,
			    uint16_t addr)
{
	struct mp2_i2c_data *d = dev->data;
	uint8_t i = 0;
	int rc = 0;

	if (num_msgs == 0U) {
		return 0;
	}
	if (msgs == NULL) {
		return -EINVAL;
	}

	k_mutex_lock(&d->lock, K_FOREVER);
	while (i < num_msgs && rc == 0) {
		/* One transaction: up to and including the message that stops the bus. */
		uint8_t n = 1;

		while ((msgs[i + n - 1].flags & I2C_MSG_STOP) == 0U && i + n < num_msgs) {
			n++;
		}
		rc = yd_mp25_i2c_transfer(&d->yd_cfg, (struct yd_mp25_i2c_msg *)&msgs[i], n, addr);
		i += n;
	}
	k_mutex_unlock(&d->lock);

	if (rc != 0) {
		LOG_DBG("transfer to 0x%02x: %d", addr, rc);
	}
	return rc;
}

static DEVICE_API(i2c, mp2_i2c_api) = {
	.configure = mp2_i2c_configure,
	.get_config = mp2_i2c_get_config,
	.transfer = mp2_i2c_transfer,
};

static int mp2_i2c_init(const struct device *dev)
{
	const struct mp2_i2c_cfg *c = dev->config;
	struct mp2_i2c_data *d = dev->data;
	int rc;

	k_mutex_init(&d->lock);
	yd_env_init(&d->env, (void *)dev);
	d->yd_cfg = c->yd;
	device_map(&d->yd_cfg.base, c->phys, c->size, K_MEM_CACHE_NONE);
	rc = yd_mp25_i2c_init(&d->yd_cfg, &d->yd, &d->env);
	if (rc != 0) {
		LOG_ERR("%s: init: %d", dev->name, rc);
	}
	return rc;
}

#define MP2_I2C_DEFINE(inst)                                                                       \
	static const struct mp2_i2c_cfg mp2_i2c_cfg_##inst = {                                     \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.yd =                                                                              \
			{                                                                          \
				.kernel_clock_hz = DT_INST_PROP(inst, kernel_clock_frequency),     \
				.speed_hz = DT_INST_PROP_OR(inst, clock_frequency, 100000),        \
				.rise_ns = DT_INST_PROP_OR(inst, i2c_scl_rising_time_ns, 0),       \
				.fall_ns = DT_INST_PROP_OR(inst, i2c_scl_falling_time_ns, 0),      \
				.dnf_ns = DT_INST_PROP_OR(inst, i2c_digital_filter_width_ns, 0),   \
				.flags = DT_INST_PROP(inst, i2c_analog_filter)                     \
						 ? YD_MP25_I2C_ANALOG_FILTER                       \
						 : 0,                                              \
			},                                                                         \
	};                                                                                         \
	static struct mp2_i2c_data mp2_i2c_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, mp2_i2c_init, NULL, &mp2_i2c_data_##inst,                      \
			      &mp2_i2c_cfg_##inst, POST_KERNEL,                                    \
			      CONFIG_YAKOGL_DRV_I2C_STM32MP2_INIT_PRIORITY, &mp2_i2c_api);

DT_INST_FOREACH_STATUS_OKAY(MP2_I2C_DEFINE)
