/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * display_yakogl_imx8mp_hdmi.c -- the i.MX 8M Plus HDMI output (LCDIF3 ->
 * PVI -> DesignWare HDMI TX -> HDMI PHY) as a Zephyr display device:
 * registration only. The register logic is Rust
 * (driver/rust/src/soc/imx8mp/hdmi_chain.rs and the modules it calls, written
 * from IMX8MPRM Rev. 3, in libyakogl_drivers.a); this file maps the seven
 * register ranges, fills the devicetree configuration and serialises the
 * calls with a mutex (a bring-up and a flip wait for frames: no spinlock).
 *
 * Two front ends over the one device:
 *  - Zephyr's display API (with CONFIG_DISPLAY): write() of a whole frame
 *    flips to that buffer by address (no copy; the buffer must be
 *    physically contiguous, identity-mapped DMA memory), blanking on/off
 *    stops and starts the scan-out, get_capabilities reports the mode as
 *    ARGB8888.
 *  - The imx8mp_display_* facade (src/display/imx8mp_display.h) the samples
 *    and the Qt glue already call, so an image moves from the adopt driver
 *    (yakogl,imx8mp-lcdif-adopt) to this one by devicetree and Kconfig only.
 *
 * Binding: dts/bindings/display/yakogl,imx8mp-hdmi.yaml. Design and board
 * plan: driver/docs/x2-hdmi-cold.md.
 */
#define DT_DRV_COMPAT yakogl_imx8mp_hdmi

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/device_mmio.h>
#include <errno.h>
#include <string.h>

#if defined(CONFIG_DISPLAY)
#include <zephyr/drivers/display.h>
#endif

#include <yakogl_drivers.h>
#include <yakogl_drv/yd_env.h>

#include "imx8mp_display.h"

LOG_MODULE_REGISTER(imx8mp_hdmi, CONFIG_YAKOGL_DRV_DISPLAY_RUST_LOG_LEVEL);

BUILD_ASSERT(DT_NUM_INST_STATUS_OKAY(DT_DRV_COMPAT) == 1, "exactly one yakogl,imx8mp-hdmi node");

#define NREGS 7
/* reg-names order of the binding */
enum { R_HDMI, R_LCDIF, R_PHY, R_BLK, R_PVI, R_GPC, R_CCM };

struct hdmi_cfg {
	uintptr_t phys[NREGS];
	size_t size[NREGS];
	struct yd_imx8mp_hdmi_config yd;
};

struct hdmi_data {
	struct yd_imx8mp_hdmi_data yd;
	struct yd_env env;
	struct k_mutex lock;
	struct imx8mp_display_info info;
	bool mapped;
};

static int hdmi_init(const struct device *dev)
{
	const struct hdmi_cfg *cfg = dev->config;
	struct hdmi_data *d = dev->data;
	uintptr_t *base[NREGS] = {
		[R_HDMI] = &d->yd.hdmi, [R_LCDIF] = &d->yd.lcdif, [R_PHY] = &d->yd.phy,
		[R_BLK] = &d->yd.blk,   [R_PVI] = &d->yd.pvi,     [R_GPC] = &d->yd.gpc,
		[R_CCM] = &d->yd.ccm,
	};

	k_mutex_init(&d->lock);
	for (int i = 0; i < NREGS; i++) {
		device_map(base[i], cfg->phys[i], cfg->size[i], K_MEM_CACHE_NONE);
	}
	yd_env_init(&d->env, (void *)dev);
	d->mapped = true;
	/* Reads only what is reachable: HDMIMIX may well be off here. */
	LOG_INF("hdmi: health 0x%x at init", yd_imx8mp_hdmi_health(&d->yd));
	return 0;
}

static int hdmi_on(const struct device *dev, uint64_t fb_phys, uint32_t pitch)
{
	const struct hdmi_cfg *cfg = dev->config;
	struct hdmi_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_imx8mp_hdmi_on(&cfg->yd, &d->yd, &d->env, fb_phys, pitch);
	d->info.on = d->yd.on != 0;
	d->info.adopted = d->yd.adopted != 0;
	d->info.width = d->yd.width;
	d->info.height = d->yd.height;
	d->info.pitch = d->yd.pitch;
	d->info.bytes_pp = 4;
	d->info.pixel_clock_hz = d->yd.tmds_hz;
	d->info.refresh_mhz = d->yd.refresh_mhz;
	k_mutex_unlock(&d->lock);
	return rc;
}

static int hdmi_set_fb(const struct device *dev, uint64_t fb_phys)
{
	struct hdmi_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_imx8mp_hdmi_set_fb(&d->yd, &d->env, fb_phys);
	k_mutex_unlock(&d->lock);
	return rc;
}

static void hdmi_off(const struct device *dev)
{
	struct hdmi_data *d = dev->data;

	k_mutex_lock(&d->lock, K_FOREVER);
	yd_imx8mp_hdmi_off(&d->yd, &d->env);
	d->info.on = false;
	k_mutex_unlock(&d->lock);
}

/* ---- Zephyr display API ------------------------------------------------ */

#if defined(CONFIG_DISPLAY)
static int hdmi_write(const struct device *dev, const uint16_t x, const uint16_t y,
		      const struct display_buffer_descriptor *desc, const void *buf)
{
	const struct hdmi_cfg *cfg = dev->config;
	struct hdmi_data *d = dev->data;

	/* Whole frames only: the write is a flip to that buffer. */
	if (x != 0 || y != 0 || desc->width != cfg->yd.hactive ||
	    desc->height != cfg->yd.vactive || desc->frame_incomplete) {
		return -ENOTSUP;
	}
	if (!d->yd.on) {
		return hdmi_on(dev, (uint64_t)(uintptr_t)buf, (uint32_t)desc->pitch * 4u);
	}
	return hdmi_set_fb(dev, (uint64_t)(uintptr_t)buf);
}

static int hdmi_blanking_on(const struct device *dev)
{
	hdmi_off(dev);
	return 0;
}

static int hdmi_blanking_off(const struct device *dev)
{
	struct hdmi_data *d = dev->data;

	/* Scanning resumes on the last buffer; before the first write there
	 * is none, and the first write starts the display. */
	return d->yd.fb != 0 ? hdmi_on(dev, d->yd.fb, d->yd.pitch) : 0;
}

static void hdmi_get_capabilities(const struct device *dev, struct display_capabilities *caps)
{
	const struct hdmi_cfg *cfg = dev->config;

	memset(caps, 0, sizeof(*caps));
	caps->x_resolution = (uint16_t)cfg->yd.hactive;
	caps->y_resolution = (uint16_t)cfg->yd.vactive;
	caps->supported_pixel_formats = PIXEL_FORMAT_ARGB_8888;
	caps->current_pixel_format = PIXEL_FORMAT_ARGB_8888;
	caps->current_orientation = DISPLAY_ORIENTATION_NORMAL;
}

static int hdmi_set_pixel_format(const struct device *dev, const enum display_pixel_format pf)
{
	ARG_UNUSED(dev);
	return pf == PIXEL_FORMAT_ARGB_8888 ? 0 : -ENOTSUP;
}

static DEVICE_API(display, hdmi_api) = {
	.blanking_on = hdmi_blanking_on,
	.blanking_off = hdmi_blanking_off,
	.write = hdmi_write,
	.get_capabilities = hdmi_get_capabilities,
	.set_pixel_format = hdmi_set_pixel_format,
};
#define HDMI_API (&hdmi_api)
#else
#define HDMI_API NULL
#endif

/* ---- the devicetree instance ------------------------------------------- */

#define NODE DT_DRV_INST(0)

#define FLAG_IF(cond, flag) ((cond) ? (flag) : 0u)
#define SEL_FLAGS(prop, set, one)                                                              \
	COND_CODE_1(DT_NODE_HAS_PROP(NODE, prop),                                              \
		    ((set) | FLAG_IF(DT_PROP(NODE, prop) != 0, (one))), (0u))

#define HDMI_FLAGS                                                                             \
	(FLAG_IF(DT_PROP(NODE, hsync_active), YD_IMX8MP_HDMI_HSYNC_HIGH) |                     \
	 FLAG_IF(DT_PROP(NODE, vsync_active), YD_IMX8MP_HDMI_VSYNC_HIGH) |                     \
	 FLAG_IF(DT_PROP(NODE, de_active), YD_IMX8MP_HDMI_DE_HIGH) |                           \
	 FLAG_IF(DT_PROP(NODE, lcdif_pixclk_rising), YD_IMX8MP_HDMI_PXCK_RISING) |             \
	 FLAG_IF(DT_PROP(NODE, force_cold), YD_IMX8MP_HDMI_FORCE_COLD) |                       \
	 FLAG_IF(DT_PROP(NODE, power_cycle), YD_IMX8MP_HDMI_POWER_CYCLE) |                     \
	 SEL_FLAGS(htx_pipe_clk_sel, YD_IMX8MP_HDMI_PIPE_SEL_SET, YD_IMX8MP_HDMI_PIPE_SEL_ONE) | \
	 SEL_FLAGS(htxphy_clk_sel, YD_IMX8MP_HDMI_PHY_SEL_SET, YD_IMX8MP_HDMI_PHY_SEL_ONE) |    \
	 COND_CODE_1(DT_NODE_HAS_PROP(NODE, dw_phy_conf0), (YD_IMX8MP_HDMI_CONF0_SET), (0u)) | \
	 COND_CODE_1(DT_NODE_HAS_PROP(NODE, dw_phy_rstz), (YD_IMX8MP_HDMI_PHYRSTZ_SET), (0u)) | \
	 FLAG_IF(DT_ENUM_IDX(NODE, test_picture) == 1, YD_IMX8MP_HDMI_TEST_FORCED) |           \
	 FLAG_IF(DT_ENUM_IDX(NODE, test_picture) == 2, YD_IMX8MP_HDMI_TEST_BGND))

#define REG_PHYS(name) DT_REG_ADDR_BY_NAME(NODE, name)
#define REG_SIZE(name) DT_REG_SIZE_BY_NAME(NODE, name)

static const struct hdmi_cfg hdmi_cfg0 = {
	.phys = {REG_PHYS(hdmi), REG_PHYS(lcdif), REG_PHYS(phy), REG_PHYS(blk), REG_PHYS(pvi),
		 REG_PHYS(gpc), REG_PHYS(ccm)},
	.size = {REG_SIZE(hdmi), REG_SIZE(lcdif), REG_SIZE(phy), REG_SIZE(blk), REG_SIZE(pvi),
		 REG_SIZE(gpc), REG_SIZE(ccm)},
	.yd = {
		.pixel_clock_hz = DT_PROP(NODE, clock_frequency),
		.hactive = DT_PROP(NODE, hactive),
		.hfront = DT_PROP(NODE, hfront_porch),
		.hsync = DT_PROP(NODE, hsync_len),
		.hback = DT_PROP(NODE, hback_porch),
		.vactive = DT_PROP(NODE, vactive),
		.vfront = DT_PROP(NODE, vfront_porch),
		.vsync = DT_PROP(NODE, vsync_len),
		.vback = DT_PROP(NODE, vback_porch),
		.flags = HDMI_FLAGS,
		.max_ppm = DT_PROP(NODE, max_ppm),
		.desc3_extra = DT_PROP(NODE, lcdif_desc3_extra),
		.noc_hurry = DT_PROP(NODE, lcdif_noc_hurry),
		.panic_low = COND_CODE_1(DT_NODE_HAS_PROP(NODE, lcdif_panic_thresholds),
					 (DT_PROP_BY_IDX(NODE, lcdif_panic_thresholds, 0)), (0)),
		.panic_high = COND_CODE_1(DT_NODE_HAS_PROP(NODE, lcdif_panic_thresholds),
					  (DT_PROP_BY_IDX(NODE, lcdif_panic_thresholds, 1)), (0)),
		.stop_after = DT_PROP(NODE, stop_after),
		.test_rgb = DT_PROP(NODE, test_rgb),
		.phy_conf0 = DT_PROP_OR(NODE, dw_phy_conf0, 0),
		.phyrstz = DT_PROP_OR(NODE, dw_phy_rstz, 0),
	},
};

static struct hdmi_data hdmi_data0;

DEVICE_DT_INST_DEFINE(0, hdmi_init, NULL, &hdmi_data0, &hdmi_cfg0, POST_KERNEL,
		      CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_HDMI_INIT_PRIORITY, HDMI_API);

/* ---- the imx8mp_display_* facade --------------------------------------- */

#define DEV DEVICE_DT_INST_GET(0)

int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch)
{
	int rc = hdmi_on(DEV, fb_phys, pitch);

	LOG_INF("hdmi: on = %d (stage %u, %s), PLL P %u M %u S %u -> %u Hz (%d ppm), "
		"%u.%03u Hz",
		rc, hdmi_data0.yd.stage, hdmi_data0.yd.adopted ? "adopted" : "cold",
		hdmi_data0.yd.pll_pms & 0xffu, (hdmi_data0.yd.pll_pms >> 8) & 0xffu,
		hdmi_data0.yd.pll_pms >> 16, hdmi_data0.yd.tmds_hz, hdmi_data0.yd.err_ppm,
		hdmi_data0.yd.refresh_mhz / 1000u, hdmi_data0.yd.refresh_mhz % 1000u);
	return rc;
}

int imx8mp_display_set_fb(uintptr_t fb_phys)
{
	return hdmi_set_fb(DEV, fb_phys);
}

uint64_t imx8mp_display_scanout(void)
{
	return hdmi_data0.mapped ? yd_imx8mp_hdmi_scanout(&hdmi_data0.yd) : 0;
}

void imx8mp_display_off(void)
{
	hdmi_off(DEV);
}

const struct imx8mp_display_info *imx8mp_display_info(void)
{
	return &hdmi_data0.info;
}

bool imx8mp_display_is_running(void)
{
	return hdmi_data0.mapped && yd_imx8mp_hdmi_is_running(&hdmi_data0.yd);
}

unsigned int imx8mp_display_health(void)
{
	return hdmi_data0.mapped ? yd_imx8mp_hdmi_health(&hdmi_data0.yd) : 0u;
}

/* No SN65DSI84 is this driver's to ask. */
int imx8mp_display_bridge_irq_stat(void)
{
	return -ENODEV;
}

void imx8mp_display_geometry(uint32_t *width, uint32_t *height)
{
	*width = hdmi_cfg0.yd.hactive;
	*height = hdmi_cfg0.yd.vactive;
}

uint32_t imx8mp_display_layer_control(void)
{
	return hdmi_data0.mapped ? yd_imx8mp_hdmi_layer_control(&hdmi_data0.yd) : 0u;
}
