/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Zephyr side of struct yd_env (driver/rust/include/yakogl_drivers.h),
 * for the shims in driver/drivers/: yd_env_init() fills one with k_busy_wait,
 * k_msleep, the yakogl_drivers log module, and -ENOTSUP stubs for I2C and
 * GPIO; yd_env_init_devs() adds the I2C buses and GPIO lines of a device
 * (bus / line N of the Rust side = element N of the arrays).
 */
#ifndef YAKOGL_DRV_YD_ENV_H
#define YAKOGL_DRV_YD_ENV_H

#include <stddef.h>
#include <yakogl_drivers.h>

void yd_env_init(struct yd_env *env, void *ctx);

#if defined(CONFIG_I2C) && defined(CONFIG_GPIO)
#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/i2c.h>

struct yd_env_devs {
	const struct i2c_dt_spec *i2c;
	size_t n_i2c;
	const struct gpio_dt_spec *gpio; /* .port NULL: the line does not exist */
	size_t n_gpio;
};

/* `devs` must outlive `env` (it becomes its ctx). */
void yd_env_init_devs(struct yd_env *env, const struct yd_env_devs *devs);
#endif

#endif /* YAKOGL_DRV_YD_ENV_H */
