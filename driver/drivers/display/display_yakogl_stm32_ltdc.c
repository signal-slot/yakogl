/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * display_yakogl_stm32_ltdc.c -- the STM32MP25 LTDC as a Zephyr display device
 * (yakogl,stm32-ltdc) and the owner of the STM32MP257F-DK's display chain,
 * LTDC -> DSI host (yakogl,stm32-dsi, its dsi-host) -> ADV7535 (that host's
 * bridge) -> HDMI. Every register access is Rust (driver/rust/src/ip/
 * stm32_ltdc.rs through src/ffi/mp25_display.rs and
 * include/yakogl_mp25_display.h); the DSI host and the bridge are reached
 * through their drivers (yakogl_drv/stm32_dsi.h, yakogl_drv/adv7535.h). The
 * i.MX8MP's chain (display_yakogl_imx8mp_lcdif.c) is the model.
 *
 *  - Zephyr's display API: get_capabilities() reports the chosen mode;
 *    write() copies pixels into the driver's frame buffer (the reserved
 *    memory /chosen yakogl,ltdc-fb names, mapped normal non-cacheable, so a
 *    barrier is the only maintenance); blanking_off() brings the chain up,
 *    blanking_on() takes it down (ADV7535 stop, DSI detach, LTDC disable).
 *  - yakogl_drv/stm32_ltdc.h: the bring-up's record, the frame check
 *    (stm32_ltdc_wait_frame), a flip to a foreign buffer (stm32_ltdc_set_fb),
 *    and the board hooks with their weak defaults here.
 *
 * The mode is chosen once, at the first call that needs it: ADV7535 init,
 * HPD (polled up to 2 s: the HDMI 5 V has just come on), EDID
 * block 0, yd_mp25_choose_mode(); 1920x1080@60 is taken when the EDID prefers
 * it and it fits the node's hactive x vactive and the buffer (the farm KVM's
 * preferred mode, tools/probes/stm32mp257f_dk/display_check/README.md), every
 * other answer becomes 1280x720@60, VIC 4. The node's hactive x vactive is
 * therefore the largest mode allowed, not the mode.
 *
 * Not this driver's: the clocks, resets, SCMI requests (HDMI 5 V), RIF and
 * pins of the path. The board layer does them before the drivers initialise
 * and answers stm32_ltdc_board_ready(); while it reports a failure nothing of
 * the chain is touched (an unclocked block can fault the bus).
 */
#define DT_DRV_COMPAT yakogl_stm32_ltdc

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/display.h>
#include <zephyr/drivers/i2c.h>
#include <zephyr/drivers/mipi_dsi.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include <yakogl_drivers.h>
#include <yakogl_mp25_display.h>
#include <yakogl_drv/adv7535.h>
#include <yakogl_drv/stm32_dsi.h>
#include <yakogl_drv/stm32_ltdc.h>
#include <yakogl_drv/yd_env.h>

LOG_MODULE_REGISTER(stm32_ltdc, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_DRV_DSI_STM32) && IS_ENABLED(CONFIG_YAKOGL_DRV_ADV7535),
	     "the LTDC owns the DK's chain: its DSI host and bridge drivers are needed");

/* The pitch ffi::mp25_display's tests and gc_triangle's scan-out use. */
#define LTDC_PITCH_ALIGN 64u
/* HPD after the 5 V came on: polled, 20 x 100 ms. */
#define LTDC_HPD_POLLS   20
#define LTDC_HPD_POLL_MS 100

/* The farm KVM's mode (CTA-861 VIC 16). */
#define LTDC_1080P_W  1920u
#define LTDC_1080P_H  1080u
#define LTDC_1080P_HZ 148500000u

struct ltdc_cfg {
	uintptr_t phys;
	size_t size;
	uint32_t max_w;          /* hactive: the largest mode allowed */
	uint32_t max_h;          /* vactive */
	enum display_pixel_format format;
	uint32_t yd_format;      /* YD_MP25_LTDC_FMT_* */
	uint32_t bpp;
	uint32_t pitch_bytes;    /* pitch-bytes, 0: ROUND_UP(width * bpp, 64) */
	uint64_t fb_phys;        /* /chosen yakogl,ltdc-fb, 0 none */
	uint64_t fb_size;        /* min(that region, fb-size) */
	uint32_t sync_flags;     /* YD_MP25_MODE_* from the polarity properties */
	uint32_t ltdc_flags;     /* YD_MP25_LTDC_* */
	const struct device *dsi;
	uint8_t dsi_lanes;
	const struct device *adv;
	const struct device *i2c; /* the bridge's bus */
};

struct ltdc_data {
	struct k_mutex lock;
	struct yd_mp25_ltdc_config yd_cfg; /* with the mapped base */
	struct yd_env env;
	struct stm32_ltdc_status st;
	struct mipi_dsi_device mdev;
	uint8_t *fb;                       /* the mapped buffer, NULL none */
	bool dsi_attached;
	bool adv_up;                       /* adv7535_init() returned 0 */
};

static bool is_ltdc(const struct device *dev);

/* ---- board hooks: weak defaults ----------------------------------------- */

__weak int stm32_ltdc_board_ready(void)
{
	return 0;
}

__weak int stm32_ltdc_board_fb_check(uint64_t phys, uint64_t size)
{
	ARG_UNUSED(phys);
	ARG_UNUSED(size);
	return 0;
}

__weak int stm32_ltdc_board_pixel_clock(uint32_t hz, uint32_t *actual_hz)
{
	*actual_hz = hz;
	return -ENOSYS;
}

/* ---- the chain ------------------------------------------------------------ */

static int fail(struct ltdc_data *d, uint32_t step, int err)
{
	d->st.step = step;
	d->st.err = err;
	LOG_ERR("chain step %u failed (%d)", step, err);
	return err;
}

static uint32_t ltdc_pitch(const struct ltdc_cfg *c, uint32_t width)
{
	if (c->pitch_bytes != 0u && width == c->max_w) {
		return c->pitch_bytes;
	}
	return ROUND_UP(width * c->bpp, LTDC_PITCH_ALIGN);
}

static bool mode_fits(const struct ltdc_cfg *c, const struct yd_mp25_mode *m)
{
	return m->hactive <= c->max_w && m->vactive <= c->max_h &&
	       (uint64_t)ltdc_pitch(c, m->hactive) * m->vactive <= c->fb_size;
}

/* ADV7535 init, HPD, EDID, the mode: once, under the lock. */
static int ltdc_select(const struct device *dev)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;
	struct yd_mp25_mode mode;
	struct yd_mp25_hdmi_sink sink;
	int rc;

	if (d->st.selected) {
		return 0;
	}
	rc = stm32_ltdc_board_ready();
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_BOARD, rc);
	}
	if (d->fb == NULL) {
		return fail(d, STM32_LTDC_STEP_FB, -ENOMEM);
	}
	rc = stm32_ltdc_board_fb_check(c->fb_phys, c->fb_size);
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_FB, rc);
	}
	rc = adv7535_init(c->adv);
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_ADV_INIT, rc);
	}
	d->adv_up = true;

	for (int i = 0; i < LTDC_HPD_POLLS; i++) {
		rc = adv7535_hpd(c->adv);
		if (rc == 1) {
			break;
		}
		k_msleep(LTDC_HPD_POLL_MS);
	}
	d->st.hpd = rc == 1;
	d->st.edid_ok = 0;
	if (d->st.hpd) {
		rc = adv7535_read_edid(c->adv, d->st.edid);
		if (rc == 0) {
			d->st.edid_ok = 1;
		} else {
			LOG_WRN("EDID read failed (%d): 720p60", rc);
		}
	} else {
		LOG_WRN("no HPD: 720p60 without EDID");
	}

	if (d->st.edid_ok) {
		d->st.choice = adv7535_choose_mode(c->adv, d->st.edid, YD_MP25_EDID_BLOCK, &mode,
						   &sink);
		if (d->st.choice == YD_MP25_MODE_FROM_EDID && mode.hactive == LTDC_1080P_W &&
		    mode.vactive == LTDC_1080P_H && mode.pixel_clock_hz == LTDC_1080P_HZ &&
		    mode_fits(c, &mode)) {
			goto chosen;
		}
	}
	/* 1280x720@60; HDMI (VIC 4, AVI infoframe) as choose_mode's rule: an
	 * EDID with a CTA extension. */
	rc = adv7535_choose_mode(c->adv, NULL, 0, &mode, &sink);
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_MODE, rc);
	}
	if (!d->st.edid_ok) {
		d->st.choice = rc;
	} else if (d->st.edid[126] != 0u) {
		sink.hdmi = 1;
		sink.vic = 4;
		sink.aspect_16_9 = 1;
	}
	if (!mode_fits(c, &mode)) {
		return fail(d, STM32_LTDC_STEP_MODE, -EINVAL);
	}
chosen:
	d->st.mode = mode;
	d->st.mode.flags |= c->sync_flags;
	d->st.sink = sink;
	d->st.pitch = ltdc_pitch(c, mode.hactive);
	d->st.bpp = c->bpp;
	d->st.selected = 1;
	LOG_INF("mode %ux%u %u Hz (choice %d, hpd %u, edid %u), %s vic %u", mode.hactive,
		mode.vactive, mode.pixel_clock_hz, d->st.choice, d->st.hpd, d->st.edid_ok,
		sink.hdmi ? "hdmi" : "dvi", sink.vic);
	return 0;
}

/* After a re-power for a new pixel clock: I2C2 was reset. The controller's
 * driver re-initialises on a speed change, so step through another speed and
 * back; then the bridge again. */
static int ltdc_bus_again(const struct device *dev)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;
	uint32_t conf;
	uint32_t other;
	int rc;

	rc = i2c_get_config(c->i2c, &conf);
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_I2C, rc);
	}
	other = (conf & ~I2C_SPEED_MASK) |
		I2C_SPEED_SET(I2C_SPEED_GET(conf) == I2C_SPEED_STANDARD ? I2C_SPEED_FAST
									 : I2C_SPEED_STANDARD);
	rc = i2c_configure(c->i2c, other);
	if (rc == 0) {
		rc = i2c_configure(c->i2c, conf);
	}
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_I2C, rc);
	}
	rc = adv7535_init(c->adv);
	if (rc < 0) {
		d->adv_up = false;
		return fail(d, STM32_LTDC_STEP_ADV_INIT, rc);
	}
	return 0;
}

/* Reverse of ltdc_chain_up, of what is up. */
static void ltdc_chain_down(const struct device *dev)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;

	if (adv7535_data(c->adv) != NULL && adv7535_data(c->adv)->started) {
		(void)adv7535_stop(c->adv);
	}
	if (d->dsi_attached) {
		(void)mipi_dsi_detach(c->dsi, 0, &d->mdev);
		d->dsi_attached = false;
	}
	if (d->st.ltdc.on) {
		(void)yd_mp25_ltdc_disable(&d->yd_cfg, &d->st.ltdc, &d->env);
	}
	d->st.on = 0;
}

static int ltdc_chain_up(const struct device *dev)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;
	const struct yd_mp25_mode *m = &d->st.mode;
	uint32_t hz = 0;
	int rc;

	if (d->st.on) {
		return 0;
	}
	d->st.step = STM32_LTDC_STEP_NONE;
	d->st.err = 0;
	rc = ltdc_select(dev);
	if (rc < 0) {
		return rc;
	}

	rc = stm32_ltdc_board_pixel_clock(m->pixel_clock_hz, &hz);
	if (rc == -ENOSYS) {
		hz = m->pixel_clock_hz;
	} else if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_CLOCK, rc);
	} else if (rc == 1) {
		rc = ltdc_bus_again(dev);
		if (rc < 0) {
			return rc;
		}
	}
	if (hz != m->pixel_clock_hz) {
		LOG_ERR("pixel clock %u Hz, the mode needs %u", hz, m->pixel_clock_hz);
		return fail(d, STM32_LTDC_STEP_CLOCK, -EIO);
	}
	d->st.pixel_clock_hz = hz;
	d->st.fb_phys = c->fb_phys;

	/* The picture written so far reaches DDR before the first fetch. */
	barrier_dsync_fence_full();
	rc = yd_mp25_ltdc_probe(&d->yd_cfg, &d->st.ltdc, &d->env);
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_LTDC_PROBE, rc);
	}
	rc = yd_mp25_ltdc_configure(&d->yd_cfg, &d->st.ltdc, m, c->fb_phys, d->st.pitch,
				    c->yd_format);
	if (rc < 0) {
		return fail(d, STM32_LTDC_STEP_LTDC_CONF, rc);
	}
	rc = yd_mp25_ltdc_enable(&d->yd_cfg, &d->st.ltdc, &d->env);
	if (rc < 0) {
		ltdc_chain_down(dev);
		return fail(d, STM32_LTDC_STEP_LTDC_ON, rc);
	}

	/* The LTDC scans (the pixel clock runs): the DSI host may stream. */
	rc = stm32_dsi_set_mode(c->dsi, m->pixel_clock_hz, m->flags);
	if (rc < 0) {
		ltdc_chain_down(dev);
		return fail(d, STM32_LTDC_STEP_DSI_MODE, rc);
	}
	d->mdev = (struct mipi_dsi_device){
		.data_lanes = c->dsi_lanes,
		.timings = {
			.hactive = m->hactive,
			.hfp = m->hfront,
			.hbp = m->hback,
			.hsync = m->hsync,
			.vactive = m->vactive,
			.vfp = m->vfront,
			.vbp = m->vback,
			.vsync = m->vsync,
		},
		.pixfmt = MIPI_DSI_PIXFMT_RGB888,
		/* adv7533.c's flags; no EoT packet (Zephyr's EOT bit clear). */
		.mode_flags = MIPI_DSI_MODE_VIDEO | MIPI_DSI_MODE_VIDEO_SYNC_PULSE |
			      MIPI_DSI_MODE_VIDEO_HSE,
	};
	rc = mipi_dsi_attach(c->dsi, 0, &d->mdev);
	if (rc < 0) {
		ltdc_chain_down(dev);
		return fail(d, STM32_LTDC_STEP_DSI, rc);
	}
	d->dsi_attached = true;

	/* Only now: the host streams video with the clock lane in HS. */
	rc = adv7535_start(c->adv, m, &d->st.sink);
	if (rc < 0) {
		ltdc_chain_down(dev);
		return fail(d, STM32_LTDC_STEP_ADV_START, rc);
	}

	(void)yd_mp25_ltdc_isr(&d->yd_cfg, &d->st.ltdc); /* the bring-up's bits cleared */
	rc = yd_mp25_ltdc_wait_frame(&d->yd_cfg, &d->env);
	if (rc < 0) {
		ltdc_chain_down(dev);
		return fail(d, STM32_LTDC_STEP_FRAME, rc);
	}
	d->st.on = 1;
	LOG_INF("chain up: ltdc 0x%06x, %ux%u pitch %u at 0x%llx", d->st.ltdc.hwver,
		m->hactive, m->vactive, d->st.pitch, (unsigned long long)c->fb_phys);
	return 0;
}

/* ---- yakogl_drv/stm32_ltdc.h ---------------------------------------------- */

const struct stm32_ltdc_status *stm32_ltdc_status(const struct device *dev)
{
	return is_ltdc(dev) ? &((const struct ltdc_data *)dev->data)->st : NULL;
}

int stm32_ltdc_framebuffer(const struct device *dev, struct stm32_ltdc_fb *fb)
{
	const struct ltdc_cfg *c;
	struct ltdc_data *d;
	int rc;

	if (!is_ltdc(dev) || fb == NULL) {
		return -ENODEV;
	}
	c = dev->config;
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = ltdc_select(dev);
	if (rc == 0) {
		*fb = (struct stm32_ltdc_fb){
			.virt = d->fb,
			.phys = c->fb_phys,
			.size = c->fb_size,
			.pitch = d->st.pitch,
			.width = d->st.mode.hactive,
			.height = d->st.mode.vactive,
			.bpp = c->bpp,
		};
	}
	k_mutex_unlock(&d->lock);
	return rc;
}

int stm32_ltdc_wait_frame(const struct device *dev, uint32_t *isr)
{
	struct ltdc_data *d;
	uint32_t bits;
	int rc;

	if (!is_ltdc(dev)) {
		return -ENODEV;
	}
	d = dev->data;
	k_mutex_lock(&d->lock, K_FOREVER);
	if (!d->st.on) {
		k_mutex_unlock(&d->lock);
		return -ENODEV;
	}
	rc = yd_mp25_ltdc_wait_frame(&d->yd_cfg, &d->env);
	bits = yd_mp25_ltdc_isr(&d->yd_cfg, &d->st.ltdc);
	k_mutex_unlock(&d->lock);
	if (isr != NULL) {
		*isr = bits;
	}
	if (rc < 0) {
		return rc;
	}
	return (bits & STM32_LTDC_ISR_ERRORS) != 0u ? -EIO : 0;
}

int stm32_ltdc_set_fb(const struct device *dev, uint64_t fb_phys)
{
	const struct ltdc_cfg *c;
	struct ltdc_data *d;
	int rc;

	if (!is_ltdc(dev)) {
		return -ENODEV;
	}
	c = dev->config;
	d = dev->data;
	if (fb_phys == 0u) {
		fb_phys = c->fb_phys;
	}
	k_mutex_lock(&d->lock, K_FOREVER);
	if (!d->st.on) {
		k_mutex_unlock(&d->lock);
		return -ENODEV;
	}
	barrier_dsync_fence_full();
	rc = yd_mp25_ltdc_set_fb(&d->yd_cfg, &d->st.ltdc, &d->env, fb_phys);
	if (rc == 0) {
		d->st.fb_phys = fb_phys;
	}
	k_mutex_unlock(&d->lock);
	return rc;
}

/* ---- Zephyr display API ---------------------------------------------------- */

static int ltdc_blanking_on(const struct device *dev)
{
	struct ltdc_data *d = dev->data;

	k_mutex_lock(&d->lock, K_FOREVER);
	ltdc_chain_down(dev);
	k_mutex_unlock(&d->lock);
	return 0;
}

static int ltdc_blanking_off(const struct device *dev)
{
	struct ltdc_data *d = dev->data;
	int rc;

	k_mutex_lock(&d->lock, K_FOREVER);
	rc = ltdc_chain_up(dev);
	k_mutex_unlock(&d->lock);
	return rc;
}

/* A copy into the driver's buffer at (x, y); desc->pitch in pixels. */
static int ltdc_write(const struct device *dev, const uint16_t x, const uint16_t y,
		      const struct display_buffer_descriptor *desc, const void *buf)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;
	const uint8_t *src = buf;
	size_t row;
	int rc;

	if (desc == NULL || buf == NULL || desc->width == 0u || desc->height == 0u ||
	    desc->pitch < desc->width) {
		return -EINVAL;
	}
	k_mutex_lock(&d->lock, K_FOREVER);
	rc = ltdc_select(dev);
	if (rc < 0) {
		goto out;
	}
	if ((uint32_t)x + desc->width > d->st.mode.hactive ||
	    (uint32_t)y + desc->height > d->st.mode.vactive ||
	    (size_t)desc->pitch * c->bpp * (desc->height - 1u) + (size_t)desc->width * c->bpp >
		    desc->buf_size) {
		rc = -EINVAL;
		goto out;
	}
	row = (size_t)desc->width * c->bpp;
	for (uint32_t i = 0; i < desc->height; i++) {
		memcpy(d->fb + (size_t)(y + i) * d->st.pitch + (size_t)x * c->bpp,
		       src + (size_t)i * desc->pitch * c->bpp, row);
	}
	/* Non-cacheable: the barrier makes the rows DDR's before a fetch. */
	barrier_dsync_fence_full();
out:
	k_mutex_unlock(&d->lock);
	return rc;
}

static void ltdc_get_capabilities(const struct device *dev, struct display_capabilities *caps)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;

	memset(caps, 0, sizeof(*caps));
	k_mutex_lock(&d->lock, K_FOREVER);
	if (ltdc_select(dev) == 0) {
		caps->x_resolution = (uint16_t)d->st.mode.hactive;
		caps->y_resolution = (uint16_t)d->st.mode.vactive;
	}
	k_mutex_unlock(&d->lock);
	caps->supported_pixel_formats = c->format;
	caps->current_pixel_format = c->format;
	caps->current_orientation = DISPLAY_ORIENTATION_NORMAL;
}

static int ltdc_set_pixel_format(const struct device *dev, const enum display_pixel_format pf)
{
	const struct ltdc_cfg *c = dev->config;

	return pf == c->format ? 0 : -ENOTSUP;
}

static DEVICE_API(display, ltdc_api) = {
	.blanking_on = ltdc_blanking_on,
	.blanking_off = ltdc_blanking_off,
	.write = ltdc_write,
	.get_capabilities = ltdc_get_capabilities,
	.set_pixel_format = ltdc_set_pixel_format,
};

static bool is_ltdc(const struct device *dev)
{
	return dev != NULL && dev->api == &ltdc_api && device_is_ready(dev);
}

static int ltdc_init(const struct device *dev)
{
	const struct ltdc_cfg *c = dev->config;
	struct ltdc_data *d = dev->data;

	k_mutex_init(&d->lock);
	if (!device_is_ready(c->dsi) || !device_is_ready(c->adv) || !device_is_ready(c->i2c)) {
		LOG_ERR("DSI host, bridge or its bus not ready");
		return -ENODEV;
	}
	device_map(&d->yd_cfg.base, c->phys, c->size, K_MEM_CACHE_NONE);
	d->yd_cfg.flags = c->ltdc_flags;
	d->yd_cfg.background = 0;
	d->yd_cfg.burst_bytes = 0;
	if (c->fb_size != 0u) {
		mm_reg_t va;

		/* Normal non-cacheable: memcpy may be unaligned (device memory
		 * would fault), and no line can be dirty when the LTDC reads.
		 * Identity-mapped: a 1080p frame is larger than the image's
		 * whole virtual region (CONFIG_KERNEL_VM_SIZE). */
		device_map(&va, (uintptr_t)c->fb_phys, (size_t)c->fb_size,
			   K_MEM_ARM_NORMAL_NC | K_MEM_DIRECT_MAP);
		d->fb = (uint8_t *)va;
	}
	yd_env_init(&d->env, (void *)dev);
	d->st.choice = -1;
	return 0;
}

/* pixel-format is the enum ["argb8888", "rgb888", "rgb565"]; the LTDC has no
 * 24-bit layer, the 32-bit one is scanned as XRGB8888 (alpha ignored). */
#define LTDC_FMT_IDX(inst) DT_INST_ENUM_IDX(inst, pixel_format)
#define LTDC_FORMAT(inst)                                                                          \
	(LTDC_FMT_IDX(inst) == 0 ? PIXEL_FORMAT_ARGB_8888 : PIXEL_FORMAT_RGB_565)
#define LTDC_YD_FORMAT(inst)                                                                       \
	(LTDC_FMT_IDX(inst) == 0 ? YD_MP25_LTDC_FMT_XRGB8888 : YD_MP25_LTDC_FMT_RGB565)
#define LTDC_BPP(inst) (LTDC_FMT_IDX(inst) == 0 ? 4u : 2u)

#define LTDC_DSI_NODE(inst) DT_INST_PHANDLE(inst, dsi_host)
#define LTDC_ADV_NODE(inst) DT_PHANDLE(LTDC_DSI_NODE(inst), bridge)

/* The frame buffer: the reserved memory /chosen yakogl,ltdc-fb names. */
#define LTDC_FB_NODE DT_CHOSEN(yakogl_ltdc_fb)
#define LTDC_FB_PHYS COND_CODE_1(DT_NODE_EXISTS(LTDC_FB_NODE), (DT_REG_ADDR(LTDC_FB_NODE)), (0))
#define LTDC_FB_REGION                                                                             \
	COND_CODE_1(DT_NODE_EXISTS(LTDC_FB_NODE), (DT_REG_SIZE(LTDC_FB_NODE)), (0))
#define LTDC_FB_SIZE(inst) MIN((uint64_t)LTDC_FB_REGION, (uint64_t)DT_INST_PROP_OR(inst, fb_size,  \
								     LTDC_FB_REGION))

#define LTDC_DEFINE(inst)                                                                          \
	BUILD_ASSERT(DT_INST_NODE_HAS_PROP(inst, dsi_host) &&                                      \
			     DT_NODE_HAS_PROP(LTDC_DSI_NODE(inst), bridge) &&                      \
			     DT_NODE_HAS_COMPAT(LTDC_ADV_NODE(inst), yakogl_adv7535),              \
		     "yakogl,stm32-ltdc: dsi-host names a yakogl,stm32-dsi whose bridge is a "     \
		     "yakogl,adv7535");                                                            \
	BUILD_ASSERT(LTDC_FMT_IDX(inst) != 1, "yakogl,stm32-ltdc: the LTDC has no rgb888 layer");  \
	static const struct ltdc_cfg ltdc_cfg_##inst = {                                           \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.max_w = DT_INST_PROP(inst, hactive),                                              \
		.max_h = DT_INST_PROP(inst, vactive),                                              \
		.format = LTDC_FORMAT(inst),                                                       \
		.yd_format = LTDC_YD_FORMAT(inst),                                                 \
		.bpp = LTDC_BPP(inst),                                                             \
		.pitch_bytes = DT_INST_PROP_OR(inst, pitch_bytes, 0),                              \
		.fb_phys = LTDC_FB_PHYS,                                                           \
		.fb_size = LTDC_FB_SIZE(inst),                                                     \
		.sync_flags = (DT_INST_PROP(inst, hsync_active_low) ? YD_MP25_MODE_HSYNC_LOW : 0u) | \
			      (DT_INST_PROP(inst, vsync_active_low) ? YD_MP25_MODE_VSYNC_LOW : 0u) | \
			      (DT_INST_PROP(inst, de_active_low) ? YD_MP25_MODE_DE_LOW : 0u),      \
		.ltdc_flags = DT_INST_PROP(inst, pixclk_falling) ? YD_MP25_LTDC_PIXCLK_FALLING : 0u, \
		.dsi = DEVICE_DT_GET(LTDC_DSI_NODE(inst)),                                         \
		.dsi_lanes = DT_PROP(LTDC_DSI_NODE(inst), data_lanes),                             \
		.adv = DEVICE_DT_GET(LTDC_ADV_NODE(inst)),                                         \
		.i2c = DEVICE_DT_GET(DT_BUS(LTDC_ADV_NODE(inst))),                                 \
	};                                                                                         \
	static struct ltdc_data ltdc_data_##inst;                                                  \
	DEVICE_DT_INST_DEFINE(inst, ltdc_init, NULL, &ltdc_data_##inst, &ltdc_cfg_##inst,          \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_DISPLAY_STM32_LTDC_INIT_PRIORITY,     \
			      &ltdc_api);

DT_INST_FOREACH_STATUS_OKAY(LTDC_DEFINE)
