/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Internals shared by the blocks of the i.MX8MP display chain. One file per
 * block (LCDIFv3, DSIM); imx8mp_display.c owns the device and calls them in
 * order. The media power domains are Zephyr power-domain devices
 * (yakogl,imx8mp-media-pd, driver/drivers/power_domain/
 * power_domain_yakogl_imx8mp_media.c), the backlight a Zephyr PWM device
 * (yakogl,imx-pwm, driver/drivers/pwm/pwm_yakogl_imx.c) and the SN65DSI8x
 * bridge a device on I2C (yakogl,sn65dsi8x,
 * driver/drivers/display/bridge_sn65dsi8x.c).
 */
#ifndef IMX8MP_DISPLAY_INTERNAL_H
#define IMX8MP_DISPLAY_INTERNAL_H

#include <zephyr/device.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/pwm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>

#include "imx8mp_display.h"

/* The panel mode, in the same terms the device tree states it. Porches are
 * as Linux's panel-timing names them, not as DRM's hsync_start/_end. */
struct display_mode {
	uint32_t pixel_clock_hz;
	uint16_t hactive, hfront, hsync, hback;
	uint16_t vactive, vfront, vsync, vback;
	bool hsync_active_low;
	bool vsync_active_low;
	bool de_active_low;
};

static inline uint32_t display_htotal(const struct display_mode *m)
{
	return (uint32_t)m->hactive + m->hfront + m->hsync + m->hback;
}

static inline uint32_t display_vtotal(const struct display_mode *m)
{
	return (uint32_t)m->vactive + m->vfront + m->vsync + m->vback;
}

/* Everything the bring-up needs, resolved from the device tree once. */
struct display_ctx {
	mm_reg_t lcdif;
	mm_reg_t dsim;
	const struct device *pd_lcdif; /* yakogl,imx8mp-media-pd "lcdif1" */
	const struct device *pd_dsi;   /* yakogl,imx8mp-media-pd "mipi-dsi1" */

	const struct device *bridge; /* yakogl,sn65dsi8x */
	struct gpio_dt_spec backlight_enable;

	struct display_mode mode;
	uint8_t  lanes;          /* DSI data lanes */
	uint8_t  bpp;            /* bits per pixel on the DSI link (24) */
	uint32_t esc_clock_hz;   /* target DSIM escape clock */
	uint32_t pll_ref_hz;     /* MEDIA_MIPI_PHY1_REF, the DSIM PLL input */
	uint32_t burst_clock_hz; /* DSI HS clock to run; 0 = pixel * bpp / lanes */
	uint32_t video_pll1_hz;  /* program VIDEO_PLL1 to this and offer it as a
				  * pixel-clock parent; 0 = leave it alone */
	struct pwm_dt_spec backlight;   /* period from the pwms cell */
	uint8_t  backlight_percent;

	struct imx8mp_display_info info;
};

static inline uint32_t rd32(mm_reg_t base, uint32_t off)
{
	return sys_read32(base + off);
}

static inline void wr32(mm_reg_t base, uint32_t off, uint32_t val)
{
	sys_write32(val, base + off);
}

static inline void set32(mm_reg_t base, uint32_t off, uint32_t bits)
{
	sys_write32(sys_read32(base + off) | bits, base + off);
}

static inline void clr32(mm_reg_t base, uint32_t off, uint32_t bits)
{
	sys_write32(sys_read32(base + off) & ~bits, base + off);
}

/* LCDIFv3 (imx8mp_lcdif.c, or with CONFIG_SAFEUI_DISPLAY_IMX8MP_LCDIF_RUST
 * imx8mp_lcdif_rust.c over the yakogl,imx8mp-lcdif device) */
bool lcdif_is_running(mm_reg_t lcdif);
uint32_t lcdif_layer_control(mm_reg_t lcdif);
bool lcdif_read_scanout(mm_reg_t lcdif, uint32_t *w, uint32_t *h, uint32_t *pitch,
			uint32_t *bytes_pp, uint64_t *phys);
int  lcdif_configure(struct display_ctx *c, uint64_t fb_phys, uint32_t pitch);
int  lcdif_enable(struct display_ctx *c);
void lcdif_disable(struct display_ctx *c);
int  lcdif_set_fb(mm_reg_t lcdif, uint64_t fb_phys);

/* DSIM + D-PHY (imx8mp_dsim.c, or with CONFIG_SAFEUI_DSIM_RUST
 * imx8mp_dsim_rust.c over the yakogl,samsung-dsim device) */
int  dsim_enable(struct display_ctx *c);
void dsim_video_start(struct display_ctx *c);
void dsim_disable(struct display_ctx *c);

/* Live state, for imx8mp_display_health(): read from the hardware, not
 * from what the driver remembers writing. */
bool dsim_pll_is_stable(const struct display_ctx *c);
bool dsim_hs_clock_is_ready(const struct display_ctx *c);

#endif /* IMX8MP_DISPLAY_INTERNAL_H */
