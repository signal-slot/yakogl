/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * bridge_sn65dsi8x.c -- the TI SN65DSI83/84 DSI-to-LVDS bridge as a Zephyr
 * device on I2C: registration only. The register logic, the sequence and
 * the clock arithmetic are Rust (driver/rust/src/ip/sn65dsi8x.rs, written
 * from TI SLLSEC2H / SLLSEC1I); this file resolves the devicetree, owns the
 * GPIO configuration, serialises calls and, for a node that starts itself,
 * attaches the DSI host first.
 *
 * Binding: dts/bindings/display/yakogl,sn65dsi8x.yaml. API:
 * include/safeui/sn65dsi8x.h.
 */
#define DT_DRV_COMPAT yakogl_sn65dsi8x

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <safeui/sn65dsi8x.h>
#include <safeui/yd_env.h>

LOG_MODULE_REGISTER(sn65dsi8x, CONFIG_SAFEUI_SN65DSI8X_LOG_LEVEL);

enum { LINE_EN, LINE_RESET, N_LINES }; /* the Rust side's env lines */

struct sn_cfg {
	struct i2c_dt_spec i2c;
	struct gpio_dt_spec gpio[N_LINES];
	struct gpio_dt_spec backlight;
	const struct device *dsi;        /* NULL: started by its display controller */
	struct yd_sn65dsi8x_config yd;
	struct yd_sn65dsi8x_mode mode;   /* self-starting nodes only */
};

struct sn_data {
	struct yd_sn65dsi8x_data yd;
	struct yd_env env;
	struct yd_env_devs devs;
	struct k_mutex lock;
};

/* dev->api of every instance: how the API calls recognise one. */
static const int sn_api;

static bool is_sn(const struct device *dev)
{
	return dev != NULL && dev->api == &sn_api && device_is_ready(dev);
}

int sn65dsi8x_claim_enable(const struct device *dev, bool asserted)
{
	const struct sn_cfg *c;

	if (!is_sn(dev)) {
		return -ENODEV;
	}
	c = dev->config;
	if (c->gpio[LINE_EN].port == NULL) {
		return 0;
	}
	return gpio_pin_configure_dt(&c->gpio[LINE_EN],
				     asserted ? GPIO_OUTPUT_ACTIVE : GPIO_OUTPUT_INACTIVE);
}

int sn65dsi8x_start(const struct device *dev, const struct yd_sn65dsi8x_mode *mode,
		    uint32_t lane_rate_hz)
{
	struct sn_data *d;
	int rc;

	if (!is_sn(dev) || mode == NULL) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_sn65dsi8x_start(&((const struct sn_cfg *)dev->config)->yd, &d->yd, &d->env, mode,
				lane_rate_hz);
	k_mutex_unlock(&d->lock);
	return rc;
}

int sn65dsi8x_check_link(const struct device *dev)
{
	struct sn_data *d;
	int rc;

	if (!is_sn(dev)) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_sn65dsi8x_check_link(&d->yd, &d->env);
	k_mutex_unlock(&d->lock);
	return rc;
}

int sn65dsi8x_stop(const struct device *dev)
{
	struct sn_data *d;
	int rc;

	if (!is_sn(dev)) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_sn65dsi8x_stop(&((const struct sn_cfg *)dev->config)->yd, &d->yd, &d->env);
	k_mutex_unlock(&d->lock);
	return rc;
}

uint32_t sn65dsi8x_health(const struct device *dev)
{
	struct sn_data *d;
	uint32_t h;

	if (!is_sn(dev)) {
		return 0;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	h = yd_sn65dsi8x_health(&d->env);
	k_mutex_unlock(&d->lock);
	return h;
}

int sn65dsi8x_irq_stat(const struct device *dev)
{
	struct sn_data *d;
	int rc;

	if (!is_sn(dev)) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_sn65dsi8x_irq_stat(&d->env);
	k_mutex_unlock(&d->lock);
	return rc;
}

const struct yd_sn65dsi8x_data *sn65dsi8x_data(const struct device *dev)
{
	return is_sn(dev) ? &((struct sn_data *)dev->data)->yd : NULL;
}

/* A node with a dsi-host: attach the host (its clock lane goes to HS, which
 * the data sheet wants before EN rises), then bring the bridge up, then
 * check the link; the video source (the AM62P DSS) is already running. */
static int sn_self_start(const struct device *dev)
{
	const struct sn_cfg *c = dev->config;
	struct sn_data *d = dev->data;
	const struct yd_sn65dsi8x_mode *m = &c->mode;
	int rc;

#ifdef CONFIG_MIPI_DSI
	struct mipi_dsi_device mdev = {
		.data_lanes = (uint8_t)c->yd.lanes,
		.pixfmt = MIPI_DSI_PIXFMT_RGB888,
		.mode_flags = MIPI_DSI_MODE_VIDEO | MIPI_DSI_MODE_VIDEO_BURST,
		.timings = {
			.hactive = m->hactive, .hfp = m->hfp, .hbp = m->hbp, .hsync = m->hsync,
			.vactive = m->vactive, .vfp = m->vfp, .vbp = m->vbp, .vsync = m->vsync,
		},
	};

	if (!device_is_ready(c->dsi)) {
		LOG_ERR("DSI host %s not ready", c->dsi->name);
		return -ENODEV;
	}
	rc = mipi_dsi_attach(c->dsi, 0, &mdev);
	if (rc) {
		LOG_ERR("DSI attach failed (%d)", rc);
		return rc;
	}
#else
	return -ENOTSUP;
#endif
	rc = yd_sn65dsi8x_start(&c->yd, &d->yd, &d->env, m, 0);
	if (rc) {
		return rc;
	}
	rc = yd_sn65dsi8x_check_link(&d->yd, &d->env);
	if (rc < 0) {
		return rc;
	}
	if (c->backlight.port != NULL) {
		gpio_pin_set_dt(&c->backlight, 1);
	}
	LOG_INF("SN65DSI8x up: lvds range %u, dsi range %u, div %u, fmt 0x%02x, IRQ_STAT 0x%02x",
		d->yd.lvds_clk_range, d->yd.dsi_clk_range, d->yd.dsi_clk_div, d->yd.lvds_fmt,
		d->yd.irq_stat);
	return 0;
}

static int sn_init(const struct device *dev)
{
	const struct sn_cfg *c = dev->config;
	struct sn_data *d = dev->data;

	if (!i2c_is_ready_dt(&c->i2c)) {
		LOG_ERR("I2C bus not ready");
		return -ENODEV;
	}
	for (int i = 0; i < N_LINES; i++) {
		if (c->gpio[i].port != NULL && !gpio_is_ready_dt(&c->gpio[i])) {
			LOG_ERR("gpio %d not ready", i);
			return -ENODEV;
		}
	}
	k_mutex_init(&d->lock);
	d->devs = (struct yd_env_devs){
		.i2c = &c->i2c, .n_i2c = 1, .gpio = c->gpio, .n_gpio = N_LINES,
	};
	yd_env_init_devs(&d->env, &d->devs);

	if (c->dsi == NULL) {
		/* Its controller decides: nothing is touched here, not even the
		 * EN pin (sn65dsi8x_claim_enable). */
		return 0;
	}
	/* Starting itself: everything from a known state. */
	if (c->gpio[LINE_RESET].port != NULL) {
		gpio_pin_configure_dt(&c->gpio[LINE_RESET], GPIO_OUTPUT_ACTIVE); /* hold in reset */
	}
	if (c->gpio[LINE_EN].port != NULL) {
		gpio_pin_configure_dt(&c->gpio[LINE_EN], GPIO_OUTPUT_INACTIVE);
	}
	if (c->backlight.port != NULL) {
		gpio_pin_configure_dt(&c->backlight, GPIO_OUTPUT_INACTIVE);
	}
	return sn_self_start(dev);
}

#define SN_FLAG(inst, prop, flag) (DT_INST_PROP(inst, prop) ? (flag) : 0u)

#define SN_MODE(inst)                                                                          \
	{                                                                                      \
		.pixel_clock_hz = DT_INST_PROP_OR(inst, pixel_clock_hz, 0),                    \
		.hactive = DT_INST_PROP_OR(inst, width, 0),                                    \
		.vactive = DT_INST_PROP_OR(inst, height, 0),                                   \
		.hsync = DT_INST_PROP_OR(inst, hsync_len, 0),                                  \
		.vsync = DT_INST_PROP_OR(inst, vsync_len, 0),                                  \
		.hbp = DT_INST_PROP_OR(inst, hback_porch, 0),                                  \
		.vbp = DT_INST_PROP_OR(inst, vback_porch, 0),                                  \
		.hfp = DT_INST_PROP_OR(inst, hfront_porch, 0),                                 \
		.vfp = DT_INST_PROP_OR(inst, vfront_porch, 0),                                 \
	}

#define SN_SELF_START(inst) DT_INST_NODE_HAS_PROP(inst, dsi_host)

#define SN_DEFINE(inst)                                                                        \
	BUILD_ASSERT(!SN_SELF_START(inst) || DT_INST_NODE_HAS_PROP(inst, pixel_clock_hz),      \
		     "a yakogl,sn65dsi8x with a dsi-host needs its mode (pixel-clock-hz...)");  \
	BUILD_ASSERT(DT_INST_PROP(inst, lvds_termination_ohms) == 100 ||                       \
		     DT_INST_PROP(inst, lvds_termination_ohms) == 200,                         \
		     "lvds-termination-ohms is 100 or 200");                                   \
	static const struct sn_cfg sn_cfg_##inst = {                                           \
		.i2c = I2C_DT_SPEC_INST_GET(inst),                                             \
		.gpio = {                                                                      \
			[LINE_EN] = GPIO_DT_SPEC_INST_GET_OR(inst, enable_gpios, {0}),         \
			[LINE_RESET] = GPIO_DT_SPEC_INST_GET_OR(inst, reset_gpios, {0}),       \
		},                                                                             \
		.backlight = GPIO_DT_SPEC_INST_GET_OR(inst, backlight_gpios, {0}),             \
		.dsi = COND_CODE_1(SN_SELF_START(inst),                                        \
				   (DEVICE_DT_GET(DT_INST_PHANDLE(inst, dsi_host))), (NULL)),  \
		.yd = {                                                                        \
			.lanes = DT_INST_PROP(inst, data_lanes),                               \
			.flags = SN_FLAG(inst, lvds_24bpp, YD_SN65DSI8X_LVDS_24BPP) |          \
				 SN_FLAG(inst, lvds_jeida, YD_SN65DSI8X_FORMAT1) |             \
				 (DT_INST_PROP(inst, lvds_termination_ohms) == 100             \
					  ? YD_SN65DSI8X_TERM_100_OHM : 0u) |                  \
				 SN_FLAG(inst, hsync_active_low, YD_SN65DSI8X_HS_NEG) |        \
				 SN_FLAG(inst, vsync_active_low, YD_SN65DSI8X_VS_NEG) |        \
				 SN_FLAG(inst, de_active_low, YD_SN65DSI8X_DE_NEG) |           \
				 (DT_INST_NODE_HAS_PROP(inst, enable_gpios)                    \
					  ? YD_SN65DSI8X_HAS_EN : 0u) |                        \
				 (DT_INST_NODE_HAS_PROP(inst, reset_gpios)                     \
					  ? YD_SN65DSI8X_HAS_RESET : 0u),                      \
		},                                                                             \
		.mode = SN_MODE(inst),                                                         \
	};                                                                                     \
	static struct sn_data sn_data_##inst;                                                  \
	COND_CODE_1(SN_SELF_START(inst),                                                       \
		(DEVICE_DT_INST_DEFINE(inst, sn_init, NULL, &sn_data_##inst, &sn_cfg_##inst,   \
				       POST_KERNEL,                                            \
				       CONFIG_SAFEUI_SN65DSI8X_SELF_START_INIT_PRIORITY,       \
				       &sn_api)),                                              \
		(DEVICE_DT_INST_DEFINE(inst, sn_init, NULL, &sn_data_##inst, &sn_cfg_##inst,   \
				       POST_KERNEL, CONFIG_SAFEUI_SN65DSI8X_INIT_PRIORITY,     \
				       &sn_api)))

DT_INST_FOREACH_STATUS_OKAY(SN_DEFINE)
