/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * pwm_yakogl_imx.c -- the i.MX PWM (i.MX 8M Plus PWM1-4) as a Zephyr PWM
 * device: registration only. The register logic is Rust
 * (driver/rust/src/ip/imx_pwm.rs and soc/imx8mp/ccm.rs, in
 * libyakogl_drivers.a); this file maps the PWM and CCM register ranges,
 * applies the pad's pinctrl, fills the devicetree configuration and
 * serialises calls with a spinlock.
 *
 * Binding: dts/bindings/pwm/yakogl,imx-pwm.yaml. Zephyr's own pwm_imx.c
 * (fsl,imx27-pwm) serves the same IP on the i.MX6/7 M4 only; see
 * docs/source-form.md, group E.
 */
#define DT_DRV_COMPAT yakogl_imx_pwm

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/pinctrl.h>
#include <zephyr/drivers/pwm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/spinlock.h>

#include <yakogl_drivers.h>
#include <safeui/imx_pwm.h>
#include <safeui/yd_env.h>

struct pwm_yk_config {
	const struct pinctrl_dev_config *pincfg;
	uintptr_t pwm_phys;
	size_t pwm_size;
	uintptr_t ccm_phys;
	size_t ccm_size;
	struct yd_imx_pwm_config yd;
};

struct pwm_yk_data {
	struct yd_imx_pwm_data yd;
	struct yd_env env;
	struct k_spinlock lock;
};

static int pwm_yk_set_cycles(const struct device *dev, uint32_t channel, uint32_t period,
			     uint32_t pulse, pwm_flags_t flags)
{
	const struct pwm_yk_config *cfg = dev->config;
	struct pwm_yk_data *d = dev->data;
	k_spinlock_key_t key = k_spin_lock(&d->lock);
	int rc = yd_imx_pwm_set_cycles(&cfg->yd, &d->yd, &d->env, channel, period, pulse, flags);

	k_spin_unlock(&d->lock, key);
	return rc;
}

static int pwm_yk_get_cycles_per_sec(const struct device *dev, uint32_t channel, uint64_t *cycles)
{
	const struct pwm_yk_config *cfg = dev->config;
	struct pwm_yk_data *d = dev->data;

	return yd_imx_pwm_get_cycles_per_sec(&cfg->yd, &d->yd, channel, cycles);
}

static DEVICE_API(pwm, pwm_yk_api) = {
	.set_cycles = pwm_yk_set_cycles,
	.get_cycles_per_sec = pwm_yk_get_cycles_per_sec,
};

bool yakogl_imx_pwm_is_enabled(const struct device *dev)
{
	if (dev == NULL || dev->api != &pwm_yk_api || !device_is_ready(dev)) {
		return false;
	}
	return yd_imx_pwm_is_enabled(&((struct pwm_yk_data *)dev->data)->yd);
}

static int pwm_yk_init(const struct device *dev)
{
	const struct pwm_yk_config *cfg = dev->config;
	struct pwm_yk_data *d = dev->data;
	int rc;

	rc = pinctrl_apply_state(cfg->pincfg, PINCTRL_STATE_DEFAULT);
	if (rc < 0) {
		return rc;
	}
	device_map(&d->yd.pwm_base, cfg->pwm_phys, cfg->pwm_size, K_MEM_CACHE_NONE);
	device_map(&d->yd.ccm_base, cfg->ccm_phys, cfg->ccm_size, K_MEM_CACHE_NONE);
	yd_env_init(&d->env, (void *)dev);
	return yd_imx_pwm_init(&cfg->yd, &d->yd, &d->env);
}

#define PWM_YK_DEFINE(inst)                                                                    \
	PINCTRL_DT_INST_DEFINE(inst);                                                          \
	static const struct pwm_yk_config pwm_yk_cfg_##inst = {                                \
		.pincfg = PINCTRL_DT_INST_DEV_CONFIG_GET(inst),                                \
		.pwm_phys = DT_INST_REG_ADDR_BY_NAME(inst, pwm),                               \
		.pwm_size = DT_INST_REG_SIZE_BY_NAME(inst, pwm),                               \
		.ccm_phys = DT_INST_REG_ADDR_BY_NAME(inst, ccm),                               \
		.ccm_size = DT_INST_REG_SIZE_BY_NAME(inst, ccm),                               \
		.yd = {                                                                        \
			.ccm_root = DT_INST_PROP(inst, ccm_root),                              \
			.ccm_gate = DT_INST_PROP(inst, ccm_gate),                              \
		},                                                                             \
	};                                                                                     \
	static struct pwm_yk_data pwm_yk_data_##inst;                                          \
	DEVICE_DT_INST_DEFINE(inst, pwm_yk_init, NULL, &pwm_yk_data_##inst, &pwm_yk_cfg_##inst, \
			      POST_KERNEL, CONFIG_PWM_INIT_PRIORITY, &pwm_yk_api);

DT_INST_FOREACH_STATUS_OKAY(PWM_YK_DEFINE)
