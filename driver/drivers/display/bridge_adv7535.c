/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * bridge_adv7535.c -- the ADV7535 DSI-to-HDMI bridge (yakogl,adv7535) as a
 * device on its I2C bus: registration, the I2C address map, the optional GPIO
 * lines and a mutex. Every register access is Rust (driver/rust/src/ip/
 * adv7535.rs, from Linux's adv7511_drv.c and adv7533.c, through
 * src/ffi/mp25_display.rs and include/yakogl_mp25_display.h).
 *
 * The device has no Zephyr API table; the chain owner calls
 * include/yakogl_drv/adv7535.h in the order that header documents (start only
 * after the DSI host streams video). All I2C goes through Zephyr's I2C API on
 * the node's bus, the controller being yakogl,stm32mp2-i2c on the DK.
 *
 * The 5 V supply, the reset line's pin mux, the I2C controller's clocks and
 * pins are the board layer's; init does not touch the chip.
 */
#define DT_DRV_COMPAT yakogl_adv7535

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <yakogl_mp25_display.h>
#include <yakogl_drv/adv7535.h>
#include <yakogl_drv/yd_env.h>

LOG_MODULE_REGISTER(adv7535, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

/*
 * env->i2c_write / env->i2c_write_read take a bus id; yd_env_init_devs() makes
 * it the index into the struct i2c_dt_spec array. The ids of the ABI are
 * indices of cfg->map, so each id selects the 7-bit address of its map:
 *
 *   YD_MP25_ADV7535_BUS_MAIN    0  the node's reg            (DK 0x3d)
 *   YD_MP25_ADV7535_BUS_DSI_CEC 1  cec-reg, DSI receiver     (DK 0x3c)
 *   YD_MP25_ADV7535_BUS_EDID    2  edid-reg                  (DK 0x3f)
 *   YD_MP25_ADV7535_BUS_PACKET  3  packet-reg                (DK 0x38)
 */
#define ADV7535_N_BUS 4
BUILD_ASSERT(YD_MP25_ADV7535_BUS_MAIN == 0 && YD_MP25_ADV7535_BUS_DSI_CEC == 1 &&
		     YD_MP25_ADV7535_BUS_EDID == 2 && YD_MP25_ADV7535_BUS_PACKET == 3,
	     "YD_MP25_ADV7535_BUS_* ids are not the indices of adv7535_cfg.map");

struct adv7535_cfg {
	struct i2c_dt_spec map[ADV7535_N_BUS]; /* indexed by YD_MP25_ADV7535_BUS_* */
	struct gpio_dt_spec reset;
	struct gpio_dt_spec pd;
	struct gpio_dt_spec hpd;
	struct yd_mp25_adv7535_config yd;
};

struct adv7535_data {
	struct yd_mp25_adv7535_data yd;
	struct yd_env env;
	struct yd_env_devs devs;
	struct k_mutex lock;
};

/* dev->api of every instance: how the API calls recognise one. */
static const int adv7535_api;

static bool is_adv(const struct device *dev)
{
	return dev != NULL && dev->api == &adv7535_api && device_is_ready(dev);
}

int adv7535_init(const struct device *dev)
{
	const struct adv7535_cfg *c;
	struct adv7535_data *d;
	int rc;

	if (!is_adv(dev)) {
		return -ENODEV;
	}
	c = dev->config;
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	/* reset-gpios: released (the line is active low; Linux does not drive
	 * it at all, ST's DK leaves it as the bootloader had it). */
	if (c->reset.port != NULL) {
		gpio_pin_configure_dt(&c->reset, GPIO_OUTPUT_INACTIVE);
	}
	/* pd-gpios: adv7511_drv.c requests it asserted, waits 5 ms and releases. */
	if (c->pd.port != NULL) {
		gpio_pin_configure_dt(&c->pd, GPIO_OUTPUT_ACTIVE);
		k_msleep(5);
		gpio_pin_set_dt(&c->pd, 0);
	}
	if (c->hpd.port != NULL) {
		gpio_pin_configure_dt(&c->hpd, GPIO_INPUT);
	}
	rc = yd_mp25_adv7535_init(&c->yd, &d->yd, &d->env);
	k_mutex_unlock(&d->lock);
	if (rc < 0) {
		LOG_ERR("ADV7535 init failed (%d)", rc);
	}
	return rc;
}

int adv7535_hpd(const struct device *dev)
{
	const struct adv7535_cfg *c;
	struct adv7535_data *d;
	int rc;

	if (!is_adv(dev)) {
		return -ENODEV;
	}
	c = dev->config;
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	if (c->hpd.port != NULL) {
		rc = gpio_pin_get_dt(&c->hpd);
	} else {
		rc = yd_mp25_adv7535_hpd(&d->yd, &d->env);
	}
	k_mutex_unlock(&d->lock);
	return rc;
}

int adv7535_read_edid(const struct device *dev, uint8_t *buf)
{
	struct adv7535_data *d;
	int rc;

	if (!is_adv(dev) || buf == NULL) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_mp25_adv7535_read_edid(&d->yd, &d->env, 0, buf);
	k_mutex_unlock(&d->lock);
	return rc;
}

int adv7535_choose_mode(const struct device *dev, const uint8_t *edid, size_t len,
			struct yd_mp25_mode *mode, struct yd_mp25_hdmi_sink *sink)
{
	if (!is_adv(dev) || mode == NULL || sink == NULL) {
		return -ENODEV;
	}
	/* Pure: no bus access, no lock. */
	return yd_mp25_choose_mode(&((const struct adv7535_cfg *)dev->config)->yd, edid, len, mode,
				   sink);
}

int adv7535_start(const struct device *dev, const struct yd_mp25_mode *mode,
		  const struct yd_mp25_hdmi_sink *sink)
{
	struct adv7535_data *d;
	int rc;

	if (!is_adv(dev) || mode == NULL || sink == NULL) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_mp25_adv7535_start(&((const struct adv7535_cfg *)dev->config)->yd, &d->yd, &d->env,
				   mode, sink);
	k_mutex_unlock(&d->lock);
	return rc;
}

int adv7535_stop(const struct device *dev)
{
	struct adv7535_data *d;
	int rc;

	if (!is_adv(dev)) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_mp25_adv7535_stop(&d->yd, &d->env);
	k_mutex_unlock(&d->lock);
	return rc;
}

const struct yd_mp25_adv7535_data *adv7535_data(const struct device *dev)
{
	return is_adv(dev) ? &((struct adv7535_data *)dev->data)->yd : NULL;
}

static int adv7535_dev_init(const struct device *dev)
{
	const struct adv7535_cfg *c = dev->config;
	struct adv7535_data *d = dev->data;
	const struct gpio_dt_spec *lines[] = {&c->reset, &c->pd, &c->hpd};

	if (!i2c_is_ready_dt(&c->map[YD_MP25_ADV7535_BUS_MAIN])) {
		LOG_ERR("I2C bus not ready");
		return -ENODEV;
	}
	for (size_t i = 0; i < ARRAY_SIZE(lines); i++) {
		if (lines[i]->port != NULL && !gpio_is_ready_dt(lines[i])) {
			LOG_ERR("gpio %zu not ready", i);
			return -ENODEV;
		}
	}
	k_mutex_init(&d->lock);
	/* The gpio array stays empty: the Rust side drives no line itself. */
	d->devs = (struct yd_env_devs){.i2c = c->map, .n_i2c = ADV7535_N_BUS};
	yd_env_init_devs(&d->env, &d->devs);
	return 0;
}

#define ADV7535_BUS(inst, a7)                                                                    \
	{.bus = DEVICE_DT_GET(DT_INST_BUS(inst)), .addr = (a7)}

#define ADV7535_DEFINE(inst)                                                                       \
	BUILD_ASSERT(DT_INST_PROP(inst, adi_dsi_lanes) >= 2,                                       \
		     "the ADV7535's DSI receiver takes 2 to 4 lanes");                             \
	static const struct adv7535_cfg adv7535_cfg_##inst = {                                     \
		.map = {                                                                           \
			[YD_MP25_ADV7535_BUS_MAIN] = I2C_DT_SPEC_INST_GET(inst),                   \
			[YD_MP25_ADV7535_BUS_DSI_CEC] = ADV7535_BUS(inst, DT_INST_PROP(inst, cec_reg)), \
			[YD_MP25_ADV7535_BUS_EDID] = ADV7535_BUS(inst, DT_INST_PROP(inst, edid_reg)),   \
			[YD_MP25_ADV7535_BUS_PACKET] =                                             \
				ADV7535_BUS(inst, DT_INST_PROP(inst, packet_reg)),                 \
		},                                                                                 \
		.reset = GPIO_DT_SPEC_INST_GET_OR(inst, reset_gpios, {0}),                         \
		.pd = GPIO_DT_SPEC_INST_GET_OR(inst, pd_gpios, {0}),                               \
		.hpd = GPIO_DT_SPEC_INST_GET_OR(inst, hpd_gpios, {0}),                             \
		.yd = {                                                                            \
			.lanes = DT_INST_PROP(inst, adi_dsi_lanes),                                \
			.flags = DT_INST_PROP(inst, adi_disable_timing_generator)                  \
					 ? YD_MP25_ADV7535_NO_TIMING_GEN : 0u,                     \
			.edid_addr = DT_INST_PROP(inst, edid_reg),                                 \
			.packet_addr = DT_INST_PROP(inst, packet_reg),                             \
			.cec_addr = DT_INST_PROP(inst, cec_reg),                                   \
		},                                                                                 \
	};                                                                                         \
	static struct adv7535_data adv7535_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, adv7535_dev_init, NULL, &adv7535_data_##inst,                  \
			      &adv7535_cfg_##inst, POST_KERNEL,                                    \
			      CONFIG_YAKOGL_DRV_ADV7535_INIT_PRIORITY, &adv7535_api);

DT_INST_FOREACH_STATUS_OKAY(ADV7535_DEFINE)
