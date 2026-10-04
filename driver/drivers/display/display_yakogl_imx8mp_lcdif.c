/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * display_yakogl_imx8mp_lcdif.c -- an i.MX 8M Plus LCDIFv3 as a Zephyr
 * display device: registration, the register mapping, a mutex, the
 * vertical-blanking interrupt and its semaphore. Every register access is
 * Rust (driver/rust/src/ip/lcdifv3.rs, from IMX8MPRM Rev. 3 13.3, through
 * src/ffi/imx8mp_dsi.rs).
 *
 * Serves yakogl,imx8mp-lcdif nodes and, as adopt-only instances, the old
 * safeui,imx8mp-lcdif-adopt ones (the Armadillo-X2's LCDIF3), so
 * CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST replaces both
 * src/display/imx8mp_lcdif.c and imx8mp_lcdif_adopt.c without a devicetree
 * change.
 *
 * Front ends:
 *  - Zephyr's display API (CONFIG_DISPLAY): write() of a whole frame flips
 *    to that buffer by address (no copy: physically contiguous,
 *    identity-mapped memory), adopting a running layer first on an
 *    adopt-only node; blanking_on() stops the layer (adopted: gives the
 *    previous owner's layer back); blanking_off() restarts it and then
 *    releases the dsi-host's data lanes (risk R5).
 *  - include/safeui/imx8mp_lcdif.h for a chain owner
 *    (src/display/imx8mp_lcdif_rust.c, the Verdin's imx8mp_display.c).
 *  - The imx8mp_display_* API, when no safeui,imx8mp-display node is enabled
 *    and the HDMI driver is not built: what imx8mp_lcdif_adopt.c provided,
 *    over the first adopt-only instance.
 *
 * Design and board plan: driver/docs/imx8mp-display-cold.md.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/irq.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/device_mmio.h>
#include <errno.h>
#include <string.h>

#if defined(CONFIG_DISPLAY)
#include <zephyr/drivers/display.h>
#endif

#include <yakogl_drivers.h>
#include <safeui/imx8mp_lcdif.h>
#include <safeui/yd_env.h>
#if defined(CONFIG_SAFEUI_DSIM_RUST)
#include <safeui/samsung_dsim.h>
#endif

#include "imx8mp_display.h"

LOG_MODULE_REGISTER(imx8mp_lcdif, CONFIG_SAFEUI_DISPLAY_RUST_LOG_LEVEL);

struct lcdif_cfg {
	uintptr_t phys;
	size_t size;
	struct yd_lcdif_config yd;
	const struct device *dsi; /* dsi-host, or NULL */
	void (*irq_connect)(void);
	int irq;                  /* -1: no interrupt */
};

struct lcdif_data {
	struct yd_lcdif_data yd;
	struct yd_env env;
	struct k_mutex lock;
	struct k_sem vblank;
	bool mapped;
};

static __maybe_unused void lcdif_isr(const struct device *dev)
{
	struct lcdif_data *d = dev->data;

	if (yd_lcdif_isr(&d->yd) & YD_LCDIF_IRQ_VS_BLANK) {
		k_sem_give(&d->vblank);
	}
}

static int lcdif_init(const struct device *dev)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;

	k_mutex_init(&d->lock);
	k_sem_init(&d->vblank, 0, 1);
	/* The registers sit in a power domain this device does not own: map,
	 * read nothing. */
	device_map(&d->yd.base, c->phys, c->size, K_MEM_CACHE_NONE);
	yd_env_init(&d->env, (void *)dev);
	d->mapped = true;
	if (c->irq_connect != NULL) {
		c->irq_connect();
	}
	return 0;
}

static void irq_on(const struct device *dev, bool on)
{
	const struct lcdif_cfg *c = dev->config;

	if (c->irq < 0) {
		return;
	}
	if (on) {
		irq_enable((unsigned int)c->irq);
	} else {
		irq_disable((unsigned int)c->irq);
	}
}

/* ---- include/safeui/imx8mp_lcdif.h ------------------------------------- */

int yakogl_imx8mp_lcdif_configure(const struct device *dev, const struct yd_lcdif_mode *mode,
				  uint64_t fb_phys, uint32_t pitch)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_lcdif_configure(&c->yd, &d->yd, mode, fb_phys, pitch);
	k_mutex_unlock(&d->lock);
	return rc;
}

int yakogl_imx8mp_lcdif_enable(const struct device *dev)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_lcdif_enable(&c->yd, &d->yd, &d->env, c->irq >= 0);
	k_mutex_unlock(&d->lock);
	if (rc == 0) {
		irq_on(dev, true);
	}
	return rc;
}

int yakogl_imx8mp_lcdif_disable(const struct device *dev)
{
	struct lcdif_data *d = dev->data;
	int rc;

	irq_on(dev, false);
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_lcdif_release(&d->yd, &d->env);
	k_mutex_unlock(&d->lock);
	return rc;
}

int yakogl_imx8mp_lcdif_adopt(const struct device *dev, uint64_t fb_phys, uint32_t pitch)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_lcdif_adopt(&c->yd, &d->yd, &d->env, fb_phys, pitch);
	k_mutex_unlock(&d->lock);
	return rc;
}

int yakogl_imx8mp_lcdif_set_fb(const struct device *dev, uint64_t fb_phys)
{
	struct lcdif_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = yd_lcdif_set_fb(&d->yd, &d->env, fb_phys);
	k_mutex_unlock(&d->lock);
	return rc;
}

int yakogl_imx8mp_lcdif_wait_vblank(const struct device *dev, k_timeout_t timeout)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;

	if (c->irq < 0) {
		return -ENOTSUP;
	}
	k_sem_reset(&d->vblank);
	return k_sem_take(&d->vblank, timeout);
}

const struct yd_lcdif_data *yakogl_imx8mp_lcdif_data(const struct device *dev)
{
	return &((const struct lcdif_data *)dev->data)->yd;
}

/* ---- Zephyr display API ------------------------------------------------ */

#if defined(CONFIG_DISPLAY)
static int lcdif_write(const struct device *dev, const uint16_t x, const uint16_t y,
		       const struct display_buffer_descriptor *desc, const void *buf)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;
	uint64_t fb = (uint64_t)(uintptr_t)buf;

	/* Whole frames only: the write is a flip to that buffer. */
	if (x != 0 || y != 0 || desc->width != c->yd.hactive || desc->height != c->yd.vactive ||
	    desc->frame_incomplete) {
		return -ENOTSUP;
	}
	if (!d->yd.on) {
		if ((c->yd.flags & YD_LCDIF_ADOPT_ONLY) == 0u) {
			/* the chain owner brings it up (imx8mp_display_on) */
			return -ENODEV;
		}
		return yakogl_imx8mp_lcdif_adopt(dev, fb, (uint32_t)desc->pitch * 4u);
	}
	return yakogl_imx8mp_lcdif_set_fb(dev, fb);
}

static int lcdif_blanking_on(const struct device *dev)
{
	return yakogl_imx8mp_lcdif_disable(dev);
}

/* Scanning resumes on the last buffer, then the DSI host's data lanes are
 * released: the Zephyr-API end of risk R5 (the chain owner calls the same
 * yakogl_samsung_dsim_video_start() on its cold path). */
static int lcdif_blanking_off(const struct device *dev)
{
	const struct lcdif_cfg *c = dev->config;
	struct lcdif_data *d = dev->data;
	int rc;

	if (d->yd.on) {
		return 0;
	}
	if (d->yd.fb == 0u) {
		return 0; /* nothing to show yet: the first write starts it */
	}
	if (d->yd.adopted) {
		rc = yakogl_imx8mp_lcdif_adopt(dev, d->yd.fb, d->yd.our_pitch);
	} else {
		rc = yakogl_imx8mp_lcdif_enable(dev);
	}
#if defined(CONFIG_SAFEUI_DSIM_RUST)
	if (rc == 0 && c->dsi != NULL) {
		rc = yakogl_samsung_dsim_video_start(c->dsi);
	}
#else
	ARG_UNUSED(c);
#endif
	return rc;
}

static void lcdif_get_capabilities(const struct device *dev, struct display_capabilities *caps)
{
	const struct lcdif_cfg *c = dev->config;

	memset(caps, 0, sizeof(*caps));
	caps->x_resolution = (uint16_t)c->yd.hactive;
	caps->y_resolution = (uint16_t)c->yd.vactive;
	caps->supported_pixel_formats = PIXEL_FORMAT_ARGB_8888;
	caps->current_pixel_format = PIXEL_FORMAT_ARGB_8888;
	caps->current_orientation = DISPLAY_ORIENTATION_NORMAL;
}

static int lcdif_set_pixel_format(const struct device *dev, const enum display_pixel_format pf)
{
	ARG_UNUSED(dev);
	return pf == PIXEL_FORMAT_ARGB_8888 ? 0 : -ENOTSUP;
}

static DEVICE_API(display, lcdif_api) = {
	.blanking_on = lcdif_blanking_on,
	.blanking_off = lcdif_blanking_off,
	.write = lcdif_write,
	.get_capabilities = lcdif_get_capabilities,
	.set_pixel_format = lcdif_set_pixel_format,
};
#define LCDIF_API (&lcdif_api)
#else
#define LCDIF_API NULL
#endif

/* ---- devicetree instances ---------------------------------------------- */

#define FLAG_IF(cond, flag) ((cond) ? (flag) : 0u)

#define LCDIF_FLAGS(node, adopt)                                                                   \
	(FLAG_IF((adopt) || DT_PROP_OR(node, adopt_only, 0), YD_LCDIF_ADOPT_ONLY) |               \
	 FLAG_IF(DT_PROP_OR(node, pixclk_rising, 0), YD_LCDIF_PXCK_RISING) |                       \
	 FLAG_IF(DT_PROP_OR(node, hsync_active_low, 0), YD_LCDIF_HSYNC_LOW) |                      \
	 FLAG_IF(DT_PROP_OR(node, vsync_active_low, 0), YD_LCDIF_VSYNC_LOW) |                      \
	 FLAG_IF(DT_PROP_OR(node, de_active_low, 0), YD_LCDIF_DE_LOW) |                            \
	 FLAG_IF(DT_NODE_HAS_PROP(node, panic_thresholds), YD_LCDIF_PANIC))

#define LCDIF_IRQ_CONNECT(node)                                                                    \
	static void lcdif_irq_connect_##node(void)                                                 \
	{                                                                                          \
		IRQ_CONNECT(DT_IRQN(node), DT_IRQ(node, priority), lcdif_isr,                      \
			    DEVICE_DT_GET(node), 0);                                               \
	}

#define LCDIF_DEFINE(node, adopt)                                                                  \
	COND_CODE_1(DT_IRQ_HAS_IDX(node, 0), (LCDIF_IRQ_CONNECT(node)), ())                        \
	static const struct lcdif_cfg lcdif_cfg_##node = {                                         \
		.phys = DT_REG_ADDR(node),                                                         \
		.size = DT_REG_SIZE(node),                                                         \
		.yd = {                                                                            \
			.flags = LCDIF_FLAGS(node, adopt),                                         \
			.hactive = DT_PROP(node, hactive),                                         \
			.vactive = DT_PROP(node, vactive),                                         \
			.desc3_extra = DT_PROP_OR(node, desc3_extra, 0),                           \
			.panic_low = COND_CODE_1(DT_NODE_HAS_PROP(node, panic_thresholds),         \
						 (DT_PROP_BY_IDX(node, panic_thresholds, 0)), (0)), \
			.panic_high = COND_CODE_1(DT_NODE_HAS_PROP(node, panic_thresholds),        \
						  (DT_PROP_BY_IDX(node, panic_thresholds, 1)), (0)), \
		},                                                                                 \
		.dsi = COND_CODE_1(DT_NODE_HAS_PROP(node, dsi_host),                               \
				   (DEVICE_DT_GET(DT_PHANDLE(node, dsi_host))), (NULL)),           \
		.irq_connect = COND_CODE_1(DT_IRQ_HAS_IDX(node, 0), (lcdif_irq_connect_##node),    \
					   (NULL)),                                                \
		.irq = COND_CODE_1(DT_IRQ_HAS_IDX(node, 0), (DT_IRQN(node)), (-1)),               \
	};                                                                                         \
	static struct lcdif_data lcdif_data_##node;                                                \
	DEVICE_DT_DEFINE(node, lcdif_init, NULL, &lcdif_data_##node, &lcdif_cfg_##node,            \
			 POST_KERNEL, CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_INIT_PRIORITY, LCDIF_API);

DT_FOREACH_STATUS_OKAY_VARGS(yakogl_imx8mp_lcdif, LCDIF_DEFINE, 0)
DT_FOREACH_STATUS_OKAY_VARGS(safeui_imx8mp_lcdif_adopt, LCDIF_DEFINE, 1)

/* ---- the imx8mp_display_* API over an adopt-only instance -------------- */

#if !DT_HAS_COMPAT_STATUS_OKAY(safeui_imx8mp_display) && !defined(CONFIG_SAFEUI_DISPLAY_IMX8MP_HDMI)

#if DT_HAS_COMPAT_STATUS_OKAY(safeui_imx8mp_lcdif_adopt)
#define FACADE_NODE DT_COMPAT_GET_ANY_STATUS_OKAY(safeui_imx8mp_lcdif_adopt)
#else
#define FACADE_NODE DT_COMPAT_GET_ANY_STATUS_OKAY(yakogl_imx8mp_lcdif)
BUILD_ASSERT(DT_PROP(FACADE_NODE, adopt_only),
	     "without a safeui,imx8mp-display chain, the imx8mp_display_* API needs an "
	     "adopt-only LCDIF");
#endif

#define FACADE DEVICE_DT_GET(FACADE_NODE)

static struct imx8mp_display_info facade_info;

static struct lcdif_data *facade_data(void)
{
	return FACADE->data;
}

static void facade_sync(void)
{
	const struct yd_lcdif_data *y = &facade_data()->yd;

	facade_info.on = y->on != 0;
	facade_info.adopted = y->adopted != 0;
	facade_info.width = y->width;
	facade_info.height = y->height;
	facade_info.pitch = y->pitch;
	facade_info.bytes_pp = 4;
}

const struct imx8mp_display_info *imx8mp_display_info(void)
{
	return &facade_info;
}

bool imx8mp_display_is_running(void)
{
	return facade_data()->mapped && yd_lcdif_is_running(facade_data()->yd.base);
}

uint32_t imx8mp_display_layer_control(void)
{
	return facade_data()->mapped ? yd_lcdif_layer_control(facade_data()->yd.base) : 0u;
}

void imx8mp_display_geometry(uint32_t *width, uint32_t *height)
{
	*width = DT_PROP(FACADE_NODE, hactive);
	*height = DT_PROP(FACADE_NODE, vactive);
}

unsigned int imx8mp_display_health(void)
{
	return imx8mp_display_is_running() ? IMX8MP_DISPLAY_HEALTH_LCDIF_RUNNING : 0u;
}

uint64_t imx8mp_display_scanout(void)
{
	struct yd_lcdif_scanout s;

	if (!facade_data()->mapped || !yd_lcdif_read_scanout(facade_data()->yd.base, &s)) {
		return 0;
	}
	return s.phys;
}

/* No address: adopt and change nothing (the samples own the display before
 * any GL work, the real buffer comes later); never scan physical zero. */
int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch)
{
	int rc;

	if (!facade_data()->mapped) {
		return -ENODEV;
	}
	rc = yakogl_imx8mp_lcdif_adopt(FACADE, fb_phys, pitch);
	if (rc == -ENODEV) {
		LOG_ERR("the LCDIF is not scanning: nothing to adopt (this mode never "
			"brings a chain up)");
	}
	facade_sync();
	return rc;
}

int imx8mp_display_set_fb(uintptr_t fb_phys)
{
	int rc;

	if (!facade_info.on) {
		return -ENODEV;
	}
	rc = yakogl_imx8mp_lcdif_set_fb(FACADE, fb_phys);
	facade_sync();
	return rc;
}

/* Nothing downstream is ours: "off" gives the previous kernel its layer back. */
void imx8mp_display_off(void)
{
	(void)yakogl_imx8mp_lcdif_disable(FACADE);
	facade_sync();
}

#endif /* the imx8mp_display_* API */
