/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * i2c_yakogl_stm32mp2.c -- an STM32MP25 I2C controller as a polled Zephyr I2C
 * master (yakogl,stm32mp2-i2c). A compiling shell: the config is filled from
 * the devicetree and every call returns -ENOSYS. The follow-up item adds the
 * FFI calls over ip::stm32_i2c: configure (TIMINGR computed from
 * kernel-clock-frequency and the bus speed) and a polled transfer of an
 * i2c_msg list.
 */
#define DT_DRV_COMPAT yakogl_stm32mp2_i2c

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/logging/log.h>
#include <errno.h>

LOG_MODULE_REGISTER(stm32mp2_i2c, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

struct mp2_i2c_cfg {
	uintptr_t phys;
	size_t size;
	uint32_t kernel_clock_hz;
	uint32_t bus_hz;
};

struct mp2_i2c_data {
	struct k_mutex lock;
};

static int mp2_i2c_configure(const struct device *dev, uint32_t dev_config)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(dev_config);
	return -ENOSYS;
}

static int mp2_i2c_transfer(const struct device *dev, struct i2c_msg *msgs, uint8_t num_msgs,
			    uint16_t addr)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(msgs);
	ARG_UNUSED(num_msgs);
	ARG_UNUSED(addr);
	return -ENOSYS;
}

static DEVICE_API(i2c, mp2_i2c_api) = {
	.configure = mp2_i2c_configure,
	.transfer = mp2_i2c_transfer,
};

static int mp2_i2c_init(const struct device *dev)
{
	struct mp2_i2c_data *d = dev->data;

	k_mutex_init(&d->lock);
	return 0;
}

#define MP2_I2C_DEFINE(inst)                                                                       \
	static const struct mp2_i2c_cfg mp2_i2c_cfg_##inst = {                                     \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.kernel_clock_hz = DT_INST_PROP(inst, kernel_clock_frequency),                     \
		.bus_hz = DT_INST_PROP_OR(inst, clock_frequency, 100000),                          \
	};                                                                                         \
	static struct mp2_i2c_data mp2_i2c_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, mp2_i2c_init, NULL, &mp2_i2c_data_##inst,                      \
			      &mp2_i2c_cfg_##inst, POST_KERNEL,                                    \
			      CONFIG_YAKOGL_DRV_I2C_STM32MP2_INIT_PRIORITY, &mp2_i2c_api);

DT_INST_FOREACH_STATUS_OKAY(MP2_I2C_DEFINE)
