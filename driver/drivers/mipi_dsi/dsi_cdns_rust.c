/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * dsi_cdns_rust.c -- the Cadence DSI host + D-PHY TX of TI K3 (the AM62P's
 * DSI0, yakogl,cdns-dsi-j721e) as a Zephyr MIPI-DSI device, with the
 * register logic in Rust: registration, the mipi_dsi API, the register
 * mappings and the TISCI power and clock requests stay here; every register
 * access is driver/rust/src/ip/cdns_dsi.rs and ip/cdns_dphy.rs (written from
 * the AM62Px TRM, SPRUJ83D, and where the TRM is silent from TI's vendored
 * Zephyr driver, third_party/zephyr-pr114866-2026-07-29-2473da6b/).
 * It replaced the GPL-derived dsi_cdns_j721e.c (deleted 2026-10-02 after the
 * board run), with the same binding and API.
 *
 * Design and board plan: driver/docs/am62p-display-cold.md.
 */
#define DT_DRV_COMPAT yakogl_cdns_dsi_j721e

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/util.h>
#include <zephyr/logging/log.h>
#include <yakogl_drv/tisci_poll.h>
#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>
#include <errno.h>

LOG_MODULE_REGISTER(cdns_dsi, CONFIG_YAKOGL_DRV_DSI_LOG_LEVEL);

/* reg-names of the binding */
enum { R_DSI, R_WRAP, R_DPHY, NREGS };

struct cdns_cfg {
	uintptr_t phys[NREGS];
	size_t size[NREGS];
	const struct device *tisci;
	uint32_t dsi_dev, dphy_dev;
	uint8_t dsi_pclk, dsi_sysclk, dphy_psm, dphy_pllref, dphy_pllref_parent;
	uint32_t pll_ref_hz, psm_clk_hz, pixel_clock_hz;
	uint8_t lanes;
	bool burst;
	bool eot;
};

struct cdns_data {
	struct yd_cdns_dsi_data yd;
	struct yd_env env;
	struct k_mutex lock;
};

static int cdns_power(const struct device *dev)
{
	const struct cdns_cfg *c = dev->config;
	int rc;

	if (c->tisci == NULL) {
		LOG_WRN("no TISCI: assuming DSI/DPHY powered and clocked");
		return 0;
	}
	if (!device_is_ready(c->tisci)) {
		return -ENODEV;
	}
	rc = yakogl_drv_tisci_device_ensure_on(c->tisci, c->dsi_dev);
	if (rc) {
		LOG_ERR("DSI device %u on failed (%d)", c->dsi_dev, rc);
		return rc;
	}
	rc = yakogl_drv_tisci_device_ensure_on(c->tisci, c->dphy_dev);
	if (rc) {
		LOG_ERR("DPHY device %u on failed (%d)", c->dphy_dev, rc);
		return rc;
	}
	/* The D-PHY PLL reference (pll-ref-hz, 25 MHz on the AM62P) */
	(void)yakogl_drv_tisci_clock_set_parent(c->tisci, c->dphy_dev, c->dphy_pllref,
					    c->dphy_pllref_parent);
	(void)yakogl_drv_tisci_clock_set_freq(c->tisci, c->dphy_dev, c->dphy_pllref, c->pll_ref_hz,
					  c->pll_ref_hz, c->pll_ref_hz);
	(void)yakogl_drv_tisci_clock_set_state(c->tisci, c->dphy_dev, c->dphy_pllref, true);
	(void)yakogl_drv_tisci_clock_set_state(c->tisci, c->dphy_dev, c->dphy_psm, true);
	(void)yakogl_drv_tisci_clock_set_state(c->tisci, c->dsi_dev, c->dsi_pclk, true);
	(void)yakogl_drv_tisci_clock_set_state(c->tisci, c->dsi_dev, c->dsi_sysclk, true);
	return 0;
}

/* The PSM clock: the devicetree's psm-clk-hz, else what TISCI says, else
 * 25 MHz with a warning (as the C driver). */
static uint32_t psm_hz(const struct device *dev)
{
	const struct cdns_cfg *c = dev->config;
	uint64_t hz = 0;

	if (c->psm_clk_hz) {
		return c->psm_clk_hz;
	}
	if (c->tisci && yakogl_drv_tisci_clock_get_freq(c->tisci, c->dphy_dev, c->dphy_psm, &hz) == 0 &&
	    hz && hz <= UINT32_MAX) {
		return (uint32_t)hz;
	}
	LOG_WRN("PSM clock rate unknown; assuming 25 MHz");
	return 25000000;
}

static int cdns_attach(const struct device *dev, uint8_t channel, const struct mipi_dsi_device *mdev)
{
	const struct cdns_cfg *c = dev->config;
	struct cdns_data *d = dev->data;
	bool sync_pulse = !c->burst && !(mdev->mode_flags & MIPI_DSI_MODE_VIDEO_BURST);
	bool rgb565 = mdev->pixfmt == MIPI_DSI_PIXFMT_RGB565;
	struct yd_cdns_dsi_request req = {
		.pixel_clock_hz = c->pixel_clock_hz,
		.hactive = mdev->timings.hactive,
		.hfp = mdev->timings.hfp,
		.hsync = mdev->timings.hsync,
		.hbp = mdev->timings.hbp,
		.vactive = mdev->timings.vactive,
		.vfp = mdev->timings.vfp,
		.vsync = mdev->timings.vsync,
		.vbp = mdev->timings.vbp,
		.bpp = rgb565 ? 16 : 24,
		.lanes = mdev->data_lanes ? mdev->data_lanes : c->lanes,
		.flags = (sync_pulse ? YD_CDNS_DSI_SYNC_PULSE : 0) | (c->eot ? YD_CDNS_DSI_EOT : 0),
		.datatype = rgb565 ? MIPI_DSI_PACKED_PIXEL_STREAM_16 : MIPI_DSI_PACKED_PIXEL_STREAM_24,
		.pll_ref_hz = c->pll_ref_hz,
		.psm_hz = psm_hz(dev),
	};
	int rc;

	ARG_UNUSED(channel);
	if (mdev->pixfmt != MIPI_DSI_PIXFMT_RGB565 && mdev->pixfmt != MIPI_DSI_PIXFMT_RGB888) {
		return -ENOTSUP;
	}
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_cdns_dsi_attach(&d->yd, &d->env, &req);
	k_mutex_unlock(&d->lock);
	return rc;
}

static ssize_t cdns_transfer(const struct device *dev, uint8_t channel, struct mipi_dsi_msg *msg)
{
	struct cdns_data *d = dev->data;
	int rc;

	if (msg->rx_len) {
		return -ENOTSUP; /* reads not implemented */
	}
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_cdns_dsi_transfer(&d->yd, &d->env, channel, msg->type,
				  (msg->flags & MIPI_DSI_MSG_USE_LPM) != 0, msg->tx_buf, msg->tx_len);
	k_mutex_unlock(&d->lock);
	return rc;
}

static int cdns_detach(const struct device *dev, uint8_t channel, const struct mipi_dsi_device *mdev)
{
	struct cdns_data *d = dev->data;

	ARG_UNUSED(channel);
	ARG_UNUSED(mdev);
	k_mutex_lock(&d->lock, K_FOREVER);
	yd_cdns_dsi_detach(&d->yd);
	k_mutex_unlock(&d->lock);
	return 0;
}

static DEVICE_API(mipi_dsi, cdns_api) = {
	.attach = cdns_attach,
	.transfer = cdns_transfer,
	.detach = cdns_detach,
};

static int cdns_init(const struct device *dev)
{
	const struct cdns_cfg *c = dev->config;
	struct cdns_data *d = dev->data;
	uintptr_t *base[NREGS] = {[R_DSI] = &d->yd.dsi, [R_WRAP] = &d->yd.wrap, [R_DPHY] = &d->yd.dphy};
	int rc;

	k_mutex_init(&d->lock);
	for (int i = 0; i < NREGS; i++) {
#ifdef CONFIG_MMU
		device_map(base[i], c->phys[i], c->size[i], K_MEM_CACHE_NONE);
#else
		*base[i] = c->phys[i];
#endif
	}
	yd_env_init(&d->env, (void *)dev);
	rc = cdns_power(dev);
	if (rc) {
		return rc;
	}
	rc = yd_cdns_dsi_init(&d->yd, &d->env);
	if (rc) {
		LOG_ERR("DSI init failed (%d)", rc);
	}
	return rc;
}

#define CDNS_DEFINE(inst)                                                                          \
	static const struct cdns_cfg cdns_cfg_##inst = {                                           \
		.phys = {DT_INST_REG_ADDR_BY_NAME(inst, dsi), DT_INST_REG_ADDR_BY_NAME(inst, wrap), \
			 DT_INST_REG_ADDR_BY_NAME(inst, dphy)},                                    \
		.size = {DT_INST_REG_SIZE_BY_NAME(inst, dsi), DT_INST_REG_SIZE_BY_NAME(inst, wrap), \
			 DT_INST_REG_SIZE_BY_NAME(inst, dphy)},                                    \
		.tisci = DEVICE_DT_GET_OR_NULL(DT_INST_PHANDLE(inst, tisci)),                      \
		.dsi_dev = DT_INST_PROP(inst, tisci_dsi_device_id),                                \
		.dphy_dev = DT_INST_PROP(inst, tisci_dphy_device_id),                              \
		.dsi_pclk = DT_INST_PROP(inst, tisci_dsi_pclk_id),                                 \
		.dsi_sysclk = DT_INST_PROP(inst, tisci_dsi_sysclk_id),                             \
		.dphy_psm = DT_INST_PROP(inst, tisci_dphy_psm_clk_id),                             \
		.dphy_pllref = DT_INST_PROP(inst, tisci_dphy_pllref_clk_id),                       \
		.dphy_pllref_parent = DT_INST_PROP(inst, tisci_dphy_pllref_parent),                \
		.pll_ref_hz = DT_INST_PROP(inst, pll_ref_hz),                                      \
		.psm_clk_hz = DT_INST_PROP(inst, psm_clk_hz),                                      \
		.pixel_clock_hz = DT_INST_PROP(inst, pixel_clock_hz),                              \
		.lanes = DT_INST_PROP(inst, data_lanes),                                           \
		.burst = DT_INST_PROP(inst, burst_mode),                                           \
		.eot = DT_INST_PROP(inst, eot_packet),                                             \
	};                                                                                         \
	static struct cdns_data cdns_data_##inst;                                                  \
	DEVICE_DT_INST_DEFINE(inst, cdns_init, NULL, &cdns_data_##inst, &cdns_cfg_##inst,           \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_DSI_CDNS_J721E_INIT_PRIORITY, &cdns_api);

DT_INST_FOREACH_STATUS_OKAY(CDNS_DEFINE)
