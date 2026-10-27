/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * yd_zephyr.c -- what libyakogl_drivers.a (driver/rust) needs from Zephyr:
 * the struct yd_env services every Rust entry point takes, and the panic
 * hook of the Rust panic handler. Shared by every shim in driver/drivers/.
 */
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <zephyr/logging/log_ctrl.h>
#include <errno.h>
#include <string.h>

#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

LOG_MODULE_REGISTER(yakogl_drivers, CONFIG_YAKOGL_DRV_DISPLAY_RUST_LOG_LEVEL);

static void yd_busy_wait_us(void *ctx, uint32_t us)
{
	ARG_UNUSED(ctx);
	k_busy_wait(us);
}

static void yd_sleep_ms(void *ctx, uint32_t ms)
{
	ARG_UNUSED(ctx);
	k_msleep((int32_t)ms);
}

/* The Rust side hands a line that is not NUL-terminated; the logger may defer
 * the formatting, so the text goes in as a "%s" of a terminated copy, which
 * the logger copies itself. Lines are at most 160 bytes (env::LINE_MAX). */
static void yd_log(void *ctx, uint32_t level, const uint8_t *buf, size_t len)
{
	char line[161];

	ARG_UNUSED(ctx);
	if (len >= sizeof(line)) {
		len = sizeof(line) - 1;
	}
	memcpy(line, buf, len);
	line[len] = '\0';
	switch (level) {
	case 1:
		LOG_ERR("%s", line);
		break;
	case 2:
		LOG_WRN("%s", line);
		break;
	case 3:
		LOG_INF("%s", line);
		break;
	default:
		LOG_DBG("%s", line);
		break;
	}
}

static int yd_no_i2c_write(void *ctx, uint32_t bus, const uint8_t *buf, size_t len)
{
	ARG_UNUSED(ctx);
	ARG_UNUSED(bus);
	ARG_UNUSED(buf);
	ARG_UNUSED(len);
	return -ENOTSUP;
}

static int yd_no_i2c_write_read(void *ctx, uint32_t bus, const uint8_t *wbuf, size_t wlen,
				uint8_t *rbuf, size_t rlen)
{
	ARG_UNUSED(ctx);
	ARG_UNUSED(bus);
	ARG_UNUSED(wbuf);
	ARG_UNUSED(wlen);
	ARG_UNUSED(rbuf);
	ARG_UNUSED(rlen);
	return -ENOTSUP;
}

static int yd_no_gpio_set(void *ctx, uint32_t line, uint32_t value)
{
	ARG_UNUSED(ctx);
	ARG_UNUSED(line);
	ARG_UNUSED(value);
	return -ENOTSUP;
}

void yd_env_init(struct yd_env *env, void *ctx)
{
	*env = (struct yd_env){
		.ctx = ctx,
		.busy_wait_us = yd_busy_wait_us,
		.sleep_ms = yd_sleep_ms,
		.log = yd_log,
		.i2c_write = yd_no_i2c_write,
		.i2c_write_read = yd_no_i2c_write_read,
		.gpio_set = yd_no_gpio_set,
	};
}

#if defined(CONFIG_I2C) && defined(CONFIG_GPIO)

/* The env's I2C buses and GPIO lines are indices into the shim's own
 * yd_env_devs. An index outside it, or a GPIO the devicetree did not give
 * (port NULL), is -ENOTSUP. */
static int yd_devs_i2c_write(void *ctx, uint32_t bus, const uint8_t *buf, size_t len)
{
	const struct yd_env_devs *d = ctx;

	if (bus >= d->n_i2c) {
		return -ENOTSUP;
	}
	return i2c_write_dt(&d->i2c[bus], buf, len);
}

static int yd_devs_i2c_write_read(void *ctx, uint32_t bus, const uint8_t *wbuf, size_t wlen,
				  uint8_t *rbuf, size_t rlen)
{
	const struct yd_env_devs *d = ctx;

	if (bus >= d->n_i2c) {
		return -ENOTSUP;
	}
	return i2c_write_read_dt(&d->i2c[bus], wbuf, wlen, rbuf, rlen);
}

static int yd_devs_gpio_set(void *ctx, uint32_t line, uint32_t value)
{
	const struct yd_env_devs *d = ctx;

	if (line >= d->n_gpio || d->gpio[line].port == NULL) {
		return -ENOTSUP;
	}
	return gpio_pin_set_dt(&d->gpio[line], value != 0u);
}

void yd_env_init_devs(struct yd_env *env, const struct yd_env_devs *devs)
{
	yd_env_init(env, (void *)devs);
	env->i2c_write = yd_devs_i2c_write;
	env->i2c_write_read = yd_devs_i2c_write_read;
	env->gpio_set = yd_devs_gpio_set;
}

#endif /* CONFIG_I2C && CONFIG_GPIO */

FUNC_NORETURN void yd_panic(const uint8_t *file, size_t file_len, uint32_t line)
{
	char f[96];

	if (file_len >= sizeof(f)) {
		file_len = sizeof(f) - 1;
	}
	memcpy(f, file, file_len);
	f[file_len] = '\0';
	LOG_ERR("Rust panic at %s:%u", f, line);
	LOG_PANIC();
	k_panic();
	CODE_UNREACHABLE;
}
