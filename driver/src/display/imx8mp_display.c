/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The i.MX8MP display pipeline as one Zephyr device: it maps the five
 * register blocks, resolves the panel mode from the device tree, and runs
 * the bring-up in the one order that works.
 *
 * Two starting states, both real on this board:
 *
 *   - The display is still running. That is the kexec crash route
 *     (kexec_zephyr zephyr.bin 0x60000000 --crash), which skips Linux's
 *     device_shutdown() and leaves the LCDIF scanning. Then the only thing
 *     to do is re-point the scan-out at our framebuffer, because every
 *     further write would blank a picture that is already correct -- and on
 *     this carrier the picture is the only console we have.
 *
 *   - The display is dark. That is the normal kexec route: device_shutdown()
 *     runs, the DRM drivers turn the backlight off and stop the LCDIF, and
 *     Zephyr arrives to a black panel (measured 2026-09-20). Then the whole
 *     chain has to be brought up: power domains, clocks, block resets,
 *     LCDIF, DSIM, bridge, backlight.
 *
 * The two are told apart by the LCDIF's own enable bits (CTRLDESCL0_5_EN
 * and DISP_PARA_DISP_ON), not by anything we were told at build time.
 *
 * Compile-tested only. No i.MX8MP board has run this.
 */

#define DT_DRV_COMPAT yakogl_imx8mp_display

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/pwm.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <yakogl_drv/imx8mp_media.h>
#include <yakogl_drivers.h>
#include <yakogl_drv/imx_pwm.h>
#include <yakogl_drv/sn65dsi8x.h>
#include <string.h>

#include "display_internal.h"

LOG_MODULE_REGISTER(imx8mp_disp, CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_LOG_LEVEL);

#if DT_HAS_COMPAT_STATUS_OKAY(DT_DRV_COMPAT)

struct display_dt_config {
	uintptr_t reg_phys[2];
	size_t reg_size[2];
	const struct device *pd_lcdif;
	const struct device *pd_dsi;
	const struct device *bridge;
	struct gpio_dt_spec backlight_enable;
	struct display_mode mode;
	uint8_t lanes;
	uint32_t esc_clock_hz;
	uint32_t pll_ref_hz;
	uint32_t burst_clock_hz;
	uint32_t video_pll1_hz;
	struct pwm_dt_spec backlight;
	uint8_t backlight_percent;
};

enum { R_LCDIF, R_DSIM };

static struct display_ctx g_ctx;
static bool g_mapped;

/* ------------------------------------------------------------------ */

/* The backlight is the node's `pwms` device (yakogl,imx-pwm): on is the
 * devicetree's period at backlight-brightness-percent, off is a 0 % pulse,
 * and "on" as the health bit reports it is read back from the PWM itself. */
static void backlight_on(struct display_ctx *c)
{
	uint32_t pulse = (uint32_t)((uint64_t)c->backlight.period * c->backlight_percent / 100u);
	int rc = pwm_set_pulse_dt(&c->backlight, pulse);

	if (rc) {
		LOG_ERR("backlight PWM: set %u/%u ns failed (%d)", pulse, c->backlight.period, rc);
	}
}

static void backlight_off(struct display_ctx *c)
{
	(void)pwm_set_pulse_dt(&c->backlight, 0);
}

static bool backlight_is_on(const struct display_ctx *c)
{
	return yakogl_imx_pwm_is_enabled(c->backlight.dev);
}

/* The bridge (yakogl,sn65dsi8x) is started with the DSI clock lane in HS and
 * no video flowing, which is where dsim_enable() stops: the panel's mode at
 * the pixel clock the CCM produces, and the bit rate per lane the DSIM PLL
 * runs (the bridge derives its DSI clock range and divider from it). */
static int bridge_start(struct display_ctx *c)
{
	const struct display_mode *m = &c->mode;
	const struct yd_sn65dsi8x_mode bm = {
		.pixel_clock_hz = c->info.pixel_clock_hz,
		.hactive = m->hactive, .vactive = m->vactive,
		.hsync = m->hsync, .vsync = m->vsync,
		.hbp = m->hback, .vbp = m->vback,
		.hfp = m->hfront, .vfp = m->vfront,
	};
	const struct yd_sn65dsi8x_data *d;
	int rc = sn65dsi8x_start(c->bridge, &bm, c->info.dsi_hs_clock_hz);

	if (rc) {
		LOG_ERR("bridge: start failed (%d)", rc);
		return rc;
	}
	d = sn65dsi8x_data(c->bridge);
	c->info.lvds_clk_range = d->lvds_clk_range;
	c->info.dsi_clk_range = d->dsi_clk_range;
	c->info.dsi_clk_div = d->dsi_clk_div;
	memcpy(c->info.bridge_id, d->id, sizeof(c->info.bridge_id));
	return 0;
}

/* After the video started: the bridge's error status, cleared and read
 * again (data sheet Init seq 8-11). Anything left is a real link error. */
static void bridge_check(struct display_ctx *c)
{
	int rc = sn65dsi8x_check_link(c->bridge);

	c->info.bridge_irq_stat = rc >= 0 ? (uint8_t)rc : 0xffu;
	if (rc < 0) {
		LOG_ERR("bridge: link check failed (%d)", rc);
	}
}

const struct imx8mp_display_info *imx8mp_display_info(void)
{
	return &g_ctx.info;
}

/* The LCDIF and DSIM registers are inside MEDIAMIX: read them only while the
 * domain is up (mapped and its ADB400 acknowledged, read from the GPC, which
 * is always reachable). A power-down that failed half-way, or FORCE_COLD's
 * power-down, must not turn a health read into a bus hang. */
static bool media_powered(void)
{
	const uint32_t up = YD_MEDIA_ST_MEDIAMIX_MAPPED | YD_MEDIA_ST_ADB_ACK;

	return (imx8mp_media_status() & up) == up;
}

static bool media_up(void)
{
	return g_mapped && media_powered();
}

bool imx8mp_display_is_running(void)
{
	if (!media_up()) {
		return false;
	}
	return lcdif_is_running(g_ctx.lcdif);
}

unsigned int imx8mp_display_health(void)
{
	const struct display_ctx *c = &g_ctx;
	unsigned int h = 0;

	if (!g_mapped) {
		return 0;
	}
	if (media_up()) {
		if (lcdif_is_running(c->lcdif)) {
			h |= IMX8MP_DISPLAY_HEALTH_LCDIF_RUNNING;
		}
		if (dsim_pll_is_stable(c)) {
			h |= IMX8MP_DISPLAY_HEALTH_DSIM_PLL_STABLE;
		}
		if (dsim_hs_clock_is_ready(c)) {
			h |= IMX8MP_DISPLAY_HEALTH_DSIM_HS_CLOCK;
		}
	}
	/* Reads only: on the adopt path this asks a bridge Linux configured. */
	uint32_t b = sn65dsi8x_health(c->bridge);

	if (b & YD_SN65DSI8X_HEALTH_ANSWERS) {
		h |= IMX8MP_DISPLAY_HEALTH_BRIDGE_ANSWERS;
	}
	if (b & YD_SN65DSI8X_HEALTH_PLL_EN_STAT) {
		h |= IMX8MP_DISPLAY_HEALTH_BRIDGE_PLL_LOCK;
	}
	if (backlight_is_on(c)) {
		h |= IMX8MP_DISPLAY_HEALTH_BACKLIGHT_PWM;
	}
	return h;
}

int imx8mp_display_bridge_irq_stat(void)
{
	return g_mapped ? sn65dsi8x_irq_stat(g_ctx.bridge) : -ENODEV;
}

void imx8mp_display_geometry(uint32_t *width, uint32_t *height)
{
	if (width != NULL) {
		*width = g_ctx.mode.hactive;
	}
	if (height != NULL) {
		*height = g_ctx.mode.vactive;
	}
}

int imx8mp_display_set_fb(uintptr_t fb_phys)
{
	if (!media_up()) {
		return -ENODEV;
	}
	return lcdif_set_fb(g_ctx.lcdif, (uint64_t)fb_phys);
}

uint64_t imx8mp_display_scanout(void)
{
	uint32_t w, h, pitch, bpp;
	uint64_t phys;

	if (!media_up() || !lcdif_read_scanout(g_ctx.lcdif, &w, &h, &pitch, &bpp, &phys)) {
		return 0;
	}
	return phys;
}

/*
 * CTRLDESCL0_5 as it stands: the layer enable, the shadow-load bit and the
 * pixel format in one word, for a report that has to say WHY it failed.
 *
 * It goes with imx8mp_display_is_running(), which already existed. The pair
 * matters because a shadow load latches an address whether or not the layer
 * is enabled, so imx8mp_display_scanout() can hand back exactly the buffer a
 * caller flipped to while the panel goes on showing whatever it last fetched.
 * A caller that wants to say "displayed" rather than "addressed" has to ask
 * both, and a caller that fails wants to say which one was wrong.
 */
uint32_t imx8mp_display_layer_control(void)
{
	return media_up() ? lcdif_layer_control(g_ctx.lcdif) : 0u;
}

void imx8mp_display_off(void)
{
	if (!media_up()) {
		return;
	}
	backlight_off(&g_ctx);
	(void)sn65dsi8x_stop(g_ctx.bridge);
	lcdif_disable(&g_ctx);
	dsim_disable(&g_ctx);
	g_ctx.info.on = false;
}

/* The refresh rate the programmed pixel clock gives, in milli-hertz, so a
 * console line can print "60.566 fps" without floating point. */
static uint32_t refresh_mhz(const struct display_ctx *c)
{
	uint64_t px = (uint64_t)display_htotal(&c->mode) * display_vtotal(&c->mode);

	if (px == 0) {
		return 0;
	}
	return (uint32_t)((uint64_t)c->info.pixel_clock_hz * 1000u / px);
}

/*
 * Bring-up bisection on a board with no console: stop after stage N with
 * -ECANCELED (the sample then reports FAIL and holds 40 s), so that the
 * time until Linux is back says whether the SoC reset happened before or
 * after that stage. 0 = never stop.
 */
#define STOP_AFTER(n)                                                          \
	do {                                                                   \
		if (CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_STOP_AFTER == (n)) {          \
			LOG_INF("stopping after stage %d", (n));               \
			return -ECANCELED;                                     \
		}                                                              \
	} while (0)

int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch)
{
	struct display_ctx *c = &g_ctx;
	uint32_t w, h, p, bpp;
	uint64_t linux_fb;
	int rc;

	if (!g_mapped) {
		LOG_ERR("no yakogl,imx8mp-display node, or it did not initialise");
		return -ENODEV;
	}

	/*
	 * --- already running: adopt it ---
	 *
	 * The decision is made on the enable bits alone. Reading the scan-out
	 * back can fail for a format we do not render into (24-bit RGB888,
	 * the YUV modes), and falling through to a cold bring-up then would
	 * blank a display that was working. So: if it is running we adopt it,
	 * and if we cannot make sense of it we say so and change nothing.
	 */
#ifdef CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_FORCE_COLD
	/* Diagnostic: take the cold path even on a running chain, so that a
	 * run does not depend on what Linux was showing at the jump. */
	if (media_powered() && lcdif_is_running(c->lcdif)) {
		LOG_INF("FORCE_COLD: tearing the running chain down first");
		imx8mp_display_off();
	}
#endif
	if (media_powered() && lcdif_is_running(c->lcdif)) {
		c->info.adopted = true;
		c->info.on = true;
		if (!lcdif_read_scanout(c->lcdif, &w, &h, &p, &bpp, &linux_fb)) {
			LOG_WRN("LCDIF is scanning in a layout we cannot draw into "
				"(CTRLDESCL0_5 0x%08x); leaving it alone",
				lcdif_layer_control(c->lcdif));
			return -ENOTSUP;
		}
		c->info.width = w;
		c->info.height = h;
		c->info.pitch = p;
		c->info.bytes_pp = bpp;
		/* What the CCM root is really producing, not what the device
		 * tree asked for: this pipeline was set up by Linux. If its
		 * parent is one of the PLLs we do not program (VIDEO_PLL1),
		 * fall back to the device tree's figure and say so. */
		c->info.pixel_clock_hz = imx8mp_media_pixel_clock(c->pd_lcdif);
		if (c->info.pixel_clock_hz == 0u) {
			c->info.pixel_clock_hz = c->mode.pixel_clock_hz;
			LOG_INF("pixel clock read back from an unknown parent; "
				"reporting the device tree's %u Hz", c->info.pixel_clock_hz);
		}
		c->info.refresh_mhz = refresh_mhz(c);
		LOG_INF("LCDIF was already scanning %ux%u, pitch %u, %u bpp, from 0x%llx",
			w, h, p, bpp * 8, (unsigned long long)linux_fb);
		/*
		 * Linux's device_shutdown() on the normal kexec route turns the
		 * backlight off (pwm_bl's shutdown) but leaves the LCDIF, the
		 * DSIM and the bridge running, so the picture is there and the
		 * panel is dark. Measured 2026-09-21 through the health bits:
		 * every stage alive, PWM3 disabled. The one thing not to touch
		 * on a live chain is the bridge; the backlight is ours.
		 */
		if (!backlight_is_on(c)) {
			LOG_INF("backlight was off; turning it on");
			backlight_on(c);
			if (c->backlight_enable.port != NULL) {
				gpio_pin_set_dt(&c->backlight_enable, 1);
			}
		}
		if (fb_phys == 0) {
			return 0;
		}
		rc = lcdif_set_fb(c->lcdif, (uint64_t)fb_phys);
		if (rc) {
			/* Put the old address back. A controller left pointing
			 * at a buffer it never latched is the one way this can
			 * cost us the picture. */
			(void)lcdif_set_fb(c->lcdif, linux_fb);
			LOG_ERR("the LCDIF did not latch our framebuffer; kept Linux's");
			return rc;
		}
		LOG_INF("scan-out moved to 0x%lx", (unsigned long)fb_phys);
		return 0;
	}

	/* --- dark: bring the whole chain up --- */
	LOG_INF("LCDIF is not scanning; bringing the display up from cold");
	STOP_AFTER(1);
	c->info.adopted = false;

	if (fb_phys == 0u) {
		/* Adopting can do without an address; a cold bring-up cannot,
		 * and scanning out of physical zero is not a useful default. */
		LOG_ERR("a cold bring-up needs a framebuffer address");
		return -EINVAL;
	}

#ifdef CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_FORCE_COLD_POWER_OFF
	/* FORCE_COLD also takes the power away, right before the power-up
	 * below, so the cold path powers MEDIAMIX up from off. Nothing may
	 * touch an LCDIF or DSIM register in between (the block is unpowered),
	 * which is why this is here and not with the teardown above: the first
	 * call of a run comes without a framebuffer and returns before this. */
	rc = imx8mp_media_pd_off(c->pd_dsi);
	if (rc == 0) {
		rc = imx8mp_media_pd_off(c->pd_lcdif);
	}
	if (rc) {
		LOG_ERR("FORCE_COLD: media domains off failed (%d), status 0x%08x", rc,
			imx8mp_media_status());
		return rc;
	}
#endif

	if (pitch == 0u) {
		pitch = (uint32_t)c->mode.hactive * 4u;
	}
	c->info.width = c->mode.hactive;
	c->info.height = c->mode.vactive;
	c->info.pitch = pitch;
	c->info.bytes_pp = 4;

	/* The power domains: MEDIAMIX (bus roots, gate, GPC, block control),
	 * then LCDIF1's and MIPI_DSI1's (with MIPI_PHY1) clocks and resets. */
	rc = imx8mp_media_pd_on(c->pd_lcdif);
	if (rc == 0) {
		rc = imx8mp_media_pd_on(c->pd_dsi);
	}
	if (rc) {
		LOG_ERR("media power: %d (status 0x%08x)", rc, imx8mp_media_status());
		return rc;
	}
	STOP_AFTER(2);

	/* The clock plan: VIDEO_PLL1 (when the device tree asks for it), the
	 * pixel clock root, the DSIM PLL's reference; then each block's resets
	 * pulsed so it starts on these clocks. */
	rc = imx8mp_media_set_pixel_clock(c->pd_lcdif, c->mode.pixel_clock_hz, c->video_pll1_hz,
					  &c->info.pixel_clock_hz);
	if (rc) {
		return rc;
	}
	c->info.refresh_mhz = refresh_mhz(c);
	if (imx8mp_media_set_phy_ref_clock(c->pd_dsi, c->pll_ref_hz) == 0u) {
		return -EINVAL;
	}
	rc = imx8mp_media_pd_reset(c->pd_lcdif);
	if (rc == 0) {
		rc = imx8mp_media_pd_reset(c->pd_dsi);
	}
	if (rc) {
		return rc;
	}
	STOP_AFTER(3);

	/* LCDIF first: the DSIM samples its sync signals, and the bridge
	 * needs a DSI clock that is derived from this pixel clock. */
	rc = lcdif_configure(c, (uint64_t)fb_phys, pitch);
	if (rc) {
		LOG_ERR("LCDIF: configure failed (%d)", rc);
		return rc;
	}
	STOP_AFTER(4);

	rc = dsim_enable(c);
	if (rc) {
		return rc;
	}
	STOP_AFTER(5);

	/* The bridge CSRs have to be written with the DSI clock lane in HS
	 * and no video flowing, which is exactly where dsim_enable() stops. */
	rc = bridge_start(c);
	if (rc) {
		return rc;
	}
	STOP_AFTER(6);

	rc = lcdif_enable(c);
	if (rc) {
		/* -ETIMEDOUT: no frame boundary within two frames, i.e. the
		 * pixel clock does not reach the LCDIF. */
		LOG_ERR("LCDIF: enable failed (%d)", rc);
		return rc;
	}
	dsim_video_start(c);
	bridge_check(c);
	STOP_AFTER(7);
	backlight_on(c);
	if (c->backlight_enable.port != NULL) {
		gpio_pin_set_dt(&c->backlight_enable, 1);
	}

	c->info.on = true;
	LOG_INF("display on: %ux%u at %u.%03u Hz, pixel clock %u Hz, DSI %u Hz",
		c->info.width, c->info.height, c->info.refresh_mhz / 1000u,
		c->info.refresh_mhz % 1000u, c->info.pixel_clock_hz,
		c->info.dsi_hs_clock_hz);
	return 0;
}

/* ------------------------------------------------------------------ */

/* The same for display_init(): stop after init stage N with -ECANCELED (the
 * device is then not ready, the sample reports FAIL). Stage 3 adds 20 s when
 * the LCDIF was found stopped, so the return time also tells what it saw. */
#define INIT_STOP_AFTER(n)                                                     \
	do {                                                                   \
		if (CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_STOP_AFTER == (n)) {     \
			return -ECANCELED;                                     \
		}                                                              \
	} while (0)

static int display_init(const struct device *dev)
{
	const struct display_dt_config *cfg = dev->config;
	struct display_ctx *c = &g_ctx;
	bool running;

	INIT_STOP_AFTER(9);   /* the entry: nothing of this driver ran yet */

	/* The backlight: a PWM device of its own (it applies its pad's
	 * pinctrl and owns its clock), initialised before this one. */
	if (!pwm_is_ready_dt(&cfg->backlight)) {
		LOG_ERR("the backlight PWM device is not ready");
		return -ENODEV;
	}
	INIT_STOP_AFTER(1);

	device_map(&c->lcdif, cfg->reg_phys[R_LCDIF], cfg->reg_size[R_LCDIF],
		   K_MEM_CACHE_NONE);
	device_map(&c->dsim, cfg->reg_phys[R_DSIM], cfg->reg_size[R_DSIM],
		   K_MEM_CACHE_NONE);
	INIT_STOP_AFTER(2);

	if (!device_is_ready(cfg->pd_lcdif) || !device_is_ready(cfg->pd_dsi)) {
		LOG_ERR("the media power domains are not ready");
		return -ENODEV;
	}
	c->pd_lcdif = cfg->pd_lcdif;
	c->pd_dsi = cfg->pd_dsi;

	c->bridge = cfg->bridge;
	c->backlight_enable = cfg->backlight_enable;
	c->mode = cfg->mode;
	c->lanes = cfg->lanes;
	c->bpp = 24;
	c->esc_clock_hz = cfg->esc_clock_hz;
	c->pll_ref_hz = cfg->pll_ref_hz;
	c->burst_clock_hz = cfg->burst_clock_hz;
	c->video_pll1_hz = cfg->video_pll1_hz;
	c->backlight = cfg->backlight;
	c->backlight_percent = cfg->backlight_percent;

	/*
	 * Which way the enable lines are driven depends on what we found.
	 * If the LCDIF is already scanning we came in on the crash route and
	 * the bridge is live: driving its EN low here would reset it and
	 * black out the only console this carrier has. So claim the pin at
	 * the level it is already at.
	 */
	running = media_powered() && lcdif_is_running(c->lcdif);
	if (CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_STOP_AFTER == 3) {
		if (!running) {
			for (int i = 0; i < 20; i++) {
				k_busy_wait(1000000);
			}
		}
		return -ECANCELED;
	}

	if (!device_is_ready(c->bridge)) {
		LOG_ERR("the bridge device is not ready");
		return -ENODEV;
	}
	if (sn65dsi8x_claim_enable(c->bridge, running) != 0) {
		LOG_ERR("bridge EN gpio could not be claimed");
		return -ENODEV;
	}
	INIT_STOP_AFTER(4);
	if (c->backlight_enable.port != NULL) {
		if (!gpio_is_ready_dt(&c->backlight_enable)) {
			LOG_ERR("backlight enable gpio is not ready");
			return -ENODEV;
		}
		(void)gpio_pin_configure_dt(&c->backlight_enable,
					    running ? GPIO_OUTPUT_ACTIVE :
						      GPIO_OUTPUT_INACTIVE);
	}

	g_mapped = true;
	LOG_INF("display device ready: panel %ux%u, %u DSI lanes, LCDIF %s",
		c->mode.hactive, c->mode.vactive, c->lanes,
		running ? "already scanning" : "stopped");
	return 0;
}

#define MODE_FROM_DT(inst)                                                             \
	{                                                                              \
		.pixel_clock_hz = DT_INST_PROP(inst, clock_frequency),                 \
		.hactive = DT_INST_PROP(inst, hactive),                                \
		.hfront = DT_INST_PROP(inst, hfront_porch),                            \
		.hsync = DT_INST_PROP(inst, hsync_len),                                \
		.hback = DT_INST_PROP(inst, hback_porch),                              \
		.vactive = DT_INST_PROP(inst, vactive),                                \
		.vfront = DT_INST_PROP(inst, vfront_porch),                            \
		.vsync = DT_INST_PROP(inst, vsync_len),                                \
		.vback = DT_INST_PROP(inst, vback_porch),                              \
		.hsync_active_low = DT_INST_PROP(inst, hsync_active) == 0,             \
		.vsync_active_low = DT_INST_PROP(inst, vsync_active) == 0,             \
		.de_active_low = DT_INST_PROP(inst, de_active) == 0,                   \
	}

#define DISPLAY_DEFINE(inst)                                                           \
	BUILD_ASSERT(DT_INST_PROP_LEN(inst, power_domains) == 2,                        \
		     "power-domains: the lcdif1 and the mipi-dsi1 domain, in that order"); \
	BUILD_ASSERT(DT_INST_PROP(inst, dsi_lanes) ==                                  \
		     DT_PROP(DT_INST_PHANDLE(inst, bridge), data_lanes),               \
		     "dsi-lanes and the bridge's data-lanes disagree");                \
	BUILD_ASSERT((uint64_t)DT_INST_PROP(inst, burst_clock_frequency) *              \
			     DT_INST_PROP(inst, dsi_lanes) * 1000u >=                          \
		     (uint64_t)DT_INST_PROP(inst, clock_frequency) * 24u *                     \
			     (1000u + YD_SAMSUNG_DSIM_LINK_MARGIN_PERMILLE),                   \
		     "burst-clock-frequency leaves the DSI link no margin for packet overhead: " \
		     "it must be at least clock-frequency x 24 / dsi-lanes x 1.025 "            \
		     "(YD_SAMSUNG_DSIM_LINK_MARGIN_PERMILLE; 71.1 MHz on 426.6 Mbit/s broke "   \
		     "the picture, issue #23)");                                               \
	BUILD_ASSERT(DT_NODE_HAS_COMPAT(DT_INST_PHANDLE(inst, bridge), yakogl_sn65dsi8x) && \
		     !DT_NODE_HAS_PROP(DT_INST_PHANDLE(inst, bridge), dsi_host),       \
		     "bridge: a yakogl,sn65dsi8x without a dsi-host");                 \
	static const struct display_dt_config display_cfg_##inst = {                   \
		.reg_phys = {                                                          \
			DT_INST_REG_ADDR_BY_NAME(inst, lcdif),                         \
			DT_INST_REG_ADDR_BY_NAME(inst, dsim),                          \
		},                                                                     \
		.reg_size = {                                                          \
			DT_INST_REG_SIZE_BY_NAME(inst, lcdif),                         \
			DT_INST_REG_SIZE_BY_NAME(inst, dsim),                          \
		},                                                                     \
		.pd_lcdif = DEVICE_DT_GET(DT_INST_PHANDLE_BY_IDX(inst, power_domains, 0)), \
		.pd_dsi = DEVICE_DT_GET(DT_INST_PHANDLE_BY_IDX(inst, power_domains, 1)), \
		.bridge = DEVICE_DT_GET(DT_INST_PHANDLE(inst, bridge)),                \
		.backlight_enable = GPIO_DT_SPEC_INST_GET_OR(                          \
			inst, backlight_enable_gpios, {0}),                            \
		.mode = MODE_FROM_DT(inst),                                            \
		.lanes = DT_INST_PROP(inst, dsi_lanes),                                \
		.esc_clock_hz = DT_INST_PROP(inst, esc_clock_frequency),               \
		.pll_ref_hz = DT_INST_PROP(inst, pll_ref_frequency),                   \
		.burst_clock_hz = DT_INST_PROP(inst, burst_clock_frequency),           \
		.video_pll1_hz = DT_INST_PROP(inst, video_pll1_frequency),             \
		.backlight = PWM_DT_SPEC_INST_GET(inst),                               \
		.backlight_percent = DT_INST_PROP(inst, backlight_brightness_percent), \
	};                                                                             \
	DEVICE_DT_INST_DEFINE(inst, display_init, NULL, NULL, &display_cfg_##inst,     \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_PRIORITY, \
			      NULL);

/* The bring-up state is one static context, so a second display node would
 * silently share it. There is one LCDIF wired to a panel on this board. */
BUILD_ASSERT(DT_NUM_INST_STATUS_OKAY(DT_DRV_COMPAT) == 1,
	     "yakogl,imx8mp-display supports exactly one instance");

DT_INST_FOREACH_STATUS_OKAY(DISPLAY_DEFINE)

#else /* no enabled node */

const struct imx8mp_display_info *imx8mp_display_info(void)
{
	static const struct imx8mp_display_info none;

	return &none;
}
bool imx8mp_display_is_running(void) { return false; }
unsigned int imx8mp_display_health(void) { return 0; }
int imx8mp_display_bridge_irq_stat(void) { return -ENODEV; }
uint32_t imx8mp_display_layer_control(void) { return 0u; }
void imx8mp_display_geometry(uint32_t *w, uint32_t *h)
{
	if (w) { *w = 0; }
	if (h) { *h = 0; }
}
int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch)
{
	ARG_UNUSED(fb_phys);
	ARG_UNUSED(pitch);
	return -ENODEV;
}
int imx8mp_display_set_fb(uintptr_t fb_phys) { ARG_UNUSED(fb_phys); return -ENODEV; }
uint64_t imx8mp_display_scanout(void) { return 0; }
void imx8mp_display_off(void) {}

#endif /* DT_HAS_COMPAT_STATUS_OKAY(DT_DRV_COMPAT) */
