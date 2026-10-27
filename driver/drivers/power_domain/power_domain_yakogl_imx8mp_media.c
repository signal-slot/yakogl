/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * power_domain_yakogl_imx8mp_media.c -- the i.MX 8M Plus display power
 * domains as Zephyr devices: registration only. The register logic is Rust
 * (driver/rust/src/soc/imx8mp/{media,gpc,ccm,anatop,media_blk}.rs, written
 * from IMX8MPRM Rev. 3, in libyakogl_drivers.a).
 *
 *   yakogl,imx8mp-media-blk-ctrl  maps MEDIA_BLK_CTRL, GPC, CCM, CCM_ANALOG,
 *                                 holds the shared state and its lock, and
 *                                 opens the early clocks at PRE_KERNEL_1
 *   yakogl,imx8mp-media-pd        one per domain ("lcdif1", "mipi-dsi1"), a
 *                                 power-domain device: resume = on,
 *                                 suspend = off (PM_DEVICE_DT_INST_DEFINE,
 *                                 pm_device_driver_init)
 *
 * Bindings: dts/bindings/power-domain/yakogl,imx8mp-media-{blk-ctrl,pd}.yaml.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/init.h>
#include <zephyr/logging/log.h>
#include <zephyr/pm/device.h>
#include <zephyr/spinlock.h>
#include <zephyr/sys/device_mmio.h>
#include <errno.h>

#include <yakogl_drivers.h>
#include <yakogl_drv/imx8mp_media.h>
#include <yakogl_drv/yd_env.h>

LOG_MODULE_REGISTER(imx8mp_media, CONFIG_YAKOGL_DRV_DISPLAY_RUST_LOG_LEVEL);

/* ---- the block control: shared state ---------------------------------- */

#define DT_DRV_COMPAT yakogl_imx8mp_media_blk_ctrl

struct media_cfg {
	uintptr_t phys[4];
	size_t size[4];
};

struct media_data {
	struct yd_imx8mp_media_data yd;
	struct yd_env env;
	struct k_spinlock lock;
	bool mapped;
};

static int media_init(const struct device *dev)
{
	const struct media_cfg *cfg = dev->config;
	struct media_data *d = dev->data;

	device_map(&d->yd.blk_base, cfg->phys[0], cfg->size[0], K_MEM_CACHE_NONE);
	device_map(&d->yd.gpc_base, cfg->phys[1], cfg->size[1], K_MEM_CACHE_NONE);
	device_map(&d->yd.ccm_base, cfg->phys[2], cfg->size[2], K_MEM_CACHE_NONE);
	device_map(&d->yd.anatop_base, cfg->phys[3], cfg->size[3], K_MEM_CACHE_NONE);
	yd_env_init(&d->env, (void *)dev);
	d->mapped = true;
	LOG_INF("media: status 0x%08x at init", yd_imx8mp_media_status(&d->yd));
	return 0;
}

#define MEDIA_DEFINE(inst)                                                                     \
	static const struct media_cfg media_cfg_##inst = {                                     \
		.phys = { DT_INST_REG_ADDR_BY_NAME(inst, blkctrl),                             \
			  DT_INST_REG_ADDR_BY_NAME(inst, gpc),                                 \
			  DT_INST_REG_ADDR_BY_NAME(inst, ccm),                                 \
			  DT_INST_REG_ADDR_BY_NAME(inst, anatop) },                            \
		.size = { DT_INST_REG_SIZE_BY_NAME(inst, blkctrl),                             \
			  DT_INST_REG_SIZE_BY_NAME(inst, gpc),                                 \
			  DT_INST_REG_SIZE_BY_NAME(inst, ccm),                                 \
			  DT_INST_REG_SIZE_BY_NAME(inst, anatop) },                            \
	};                                                                                     \
	static struct media_data media_data_##inst;                                            \
	DEVICE_DT_INST_DEFINE(inst, media_init, NULL, &media_data_##inst, &media_cfg_##inst,   \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_INIT_PRIORITY, NULL);

DT_INST_FOREACH_STATUS_OKAY(MEDIA_DEFINE)

/*
 * The clock roots and gates Linux leaves gated on the way out: its i2c-imx
 * and pwm-imx drivers gate their clocks whenever the device is idle (runtime
 * PM), and a kexec'd image inherits that (clk_summary on the running board:
 * i2c2 at enable=0 while the display is on). Zephyr's I2C driver then faults
 * on its first register access at POST_KERNEL (measured 2026-09-20). So the
 * roots named in early-ccm-roots are enabled and the gates in
 * early-ccm-gates opened here, ahead of every driver (Rust:
 * soc::imx8mp::media::early_clocks).
 */
#if DT_HAS_COMPAT_STATUS_OKAY(DT_DRV_COMPAT) && !defined(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_NO_EARLY_CLOCKS)
static const uint32_t early_roots[] = DT_INST_PROP_OR(0, early_ccm_roots, {});
static const uint32_t early_gates[] = DT_INST_PROP_OR(0, early_ccm_gates, {});

static int media_early_clocks(void)
{
	mm_reg_t ccm;

	device_map(&ccm, DT_INST_REG_ADDR_BY_NAME(0, ccm), DT_INST_REG_SIZE_BY_NAME(0, ccm),
		   K_MEM_CACHE_NONE);
	(void)yd_imx8mp_ccm_early_clocks(ccm, early_roots, ARRAY_SIZE(early_roots), early_gates,
					 ARRAY_SIZE(early_gates));
	return 0;
}
SYS_INIT(media_early_clocks, PRE_KERNEL_1, 5);
#endif

#define FIRST_MEDIA DT_INST(0, yakogl_imx8mp_media_blk_ctrl)

uint32_t imx8mp_media_status(void)
{
#if DT_HAS_COMPAT_STATUS_OKAY(DT_DRV_COMPAT)
	const struct device *dev = DEVICE_DT_GET(FIRST_MEDIA);
	struct media_data *d = dev->data;

	if (!d->mapped) {
		return 0;
	}
	return yd_imx8mp_media_status(&d->yd);
#else
	return 0;
#endif
}

#undef DT_DRV_COMPAT

/* ---- the domains --------------------------------------------------------- */

#define DT_DRV_COMPAT yakogl_imx8mp_media_pd

struct pd_cfg {
	const struct device *media;
	uint32_t pd;
};

enum { OP_ON, OP_OFF, OP_RESET };

static int pd_run(const struct device *pd, int op)
{
	const struct pd_cfg *cfg = pd->config;
	struct media_data *d = cfg->media->data;
	k_spinlock_key_t key;
	int rc;

	if (!d->mapped) {
		return -ENODEV;
	}
	key = k_spin_lock(&d->lock);
	switch (op) {
	case OP_ON:
		rc = yd_imx8mp_media_pd_on(&d->yd, &d->env, cfg->pd);
		break;
	case OP_OFF:
		rc = yd_imx8mp_media_pd_off(&d->yd, &d->env, cfg->pd);
		break;
	default:
		rc = yd_imx8mp_media_pd_reset(&d->yd, &d->env, cfg->pd);
		break;
	}
	k_spin_unlock(&d->lock, key);
	return rc;
}

static int pd_action(const struct device *dev, enum pm_device_action action)
{
	switch (action) {
	case PM_DEVICE_ACTION_RESUME:
		return pd_run(dev, OP_ON);
	case PM_DEVICE_ACTION_SUSPEND:
		return pd_run(dev, OP_OFF);
	case PM_DEVICE_ACTION_TURN_ON:
	case PM_DEVICE_ACTION_TURN_OFF:
		return 0;
	default:
		return -ENOTSUP;
	}
}

static int pd_init(const struct device *dev)
{
	const struct pd_cfg *cfg = dev->config;
	int rc = pm_device_driver_init(dev, pd_action);

	/* A domain that does not come up is reported, not fatal: its consumer
	 * runs the same power-up again and reports the failure itself. */
	if (rc) {
		LOG_WRN("media: domain %u did not come up at init (%d), status 0x%08x", cfg->pd, rc,
			imx8mp_media_status());
	}
	return 0;
}

int imx8mp_media_pd_on(const struct device *pd)
{
	return pd_run(pd, OP_ON);
}

int imx8mp_media_pd_off(const struct device *pd)
{
	return pd_run(pd, OP_OFF);
}

int imx8mp_media_pd_reset(const struct device *pd)
{
	return pd_run(pd, OP_RESET);
}

int imx8mp_media_set_pixel_clock(const struct device *pd, uint32_t want_hz,
				 uint32_t video_pll1_hz, uint32_t *out_hz)
{
	const struct pd_cfg *cfg = pd->config;
	struct media_data *d = cfg->media->data;
	k_spinlock_key_t key;
	int rc;

	if (!d->mapped) {
		return -ENODEV;
	}
	key = k_spin_lock(&d->lock);
	rc = yd_imx8mp_media_set_pixel_clock(&d->yd, &d->env, want_hz, video_pll1_hz, out_hz);
	k_spin_unlock(&d->lock, key);
	return rc;
}

uint32_t imx8mp_media_pixel_clock(const struct device *pd)
{
	const struct pd_cfg *cfg = pd->config;
	struct media_data *d = cfg->media->data;

	return d->mapped ? yd_imx8mp_media_pixel_clock(&d->yd) : 0u;
}

uint32_t imx8mp_media_set_phy_ref_clock(const struct device *pd, uint32_t hz)
{
	const struct pd_cfg *cfg = pd->config;
	struct media_data *d = cfg->media->data;
	k_spinlock_key_t key;
	uint32_t rate;

	if (!d->mapped) {
		return 0;
	}
	key = k_spin_lock(&d->lock);
	rate = yd_imx8mp_media_set_phy_ref_clock(&d->yd, hz);
	k_spin_unlock(&d->lock, key);
	return rate;
}

#define PD_DEFINE(inst)                                                                        \
	BUILD_ASSERT(DT_NODE_HAS_COMPAT(DT_INST_PARENT(inst), yakogl_imx8mp_media_blk_ctrl),   \
		     "a yakogl,imx8mp-media-pd sits under a yakogl,imx8mp-media-blk-ctrl");    \
	static const struct pd_cfg pd_cfg_##inst = {                                           \
		.media = DEVICE_DT_GET(DT_INST_PARENT(inst)),                                  \
		.pd = DT_INST_ENUM_IDX(inst, domain),                                          \
	};                                                                                     \
	PM_DEVICE_DT_INST_DEFINE(inst, pd_action);                                             \
	DEVICE_DT_INST_DEFINE(inst, pd_init, PM_DEVICE_DT_INST_GET(inst), NULL, &pd_cfg_##inst, \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD_INIT_PRIORITY, NULL);

DT_INST_FOREACH_STATUS_OKAY(PD_DEFINE)
