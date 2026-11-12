/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * display_yakogl_stm32_ltdc.c -- the STM32MP25 LTDC as a Zephyr display device
 * (yakogl,stm32-ltdc). A compiling shell: the config is filled from the
 * devicetree and the display API returns -ENOSYS. The follow-up item adds the
 * FFI calls over ip::stm32_ltdc: layer 1 setup from hactive, vactive, the
 * format and the pitch, the frame buffer address, the shadow-reload flip and
 * blanking on/off, and the attach of the dsi-host.
 */
#define DT_DRV_COMPAT yakogl_stm32_ltdc

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/drivers/display.h>
#include <zephyr/logging/log.h>
#include <errno.h>
#include <string.h>

LOG_MODULE_REGISTER(stm32_ltdc, CONFIG_YAKOGL_DRV_STM32MP2_DISPLAY_LOG_LEVEL);

struct ltdc_cfg {
	uintptr_t phys;
	size_t size;
	uint16_t hactive;
	uint16_t vactive;
	enum display_pixel_format format;
	uint32_t pitch_bytes;
	uint32_t fb_size;
	bool hsync_low;
	bool vsync_low;
	bool de_low;
	bool pixclk_falling;
	const struct device *dsi; /* NULL: none */
};

struct ltdc_data {
	struct k_mutex lock;
};

/* pixel-format is the enum ["argb8888", "rgb888", "rgb565"] */
#define LTDC_FORMAT(inst)                                                                          \
	(DT_INST_ENUM_IDX(inst, pixel_format) == 0                                                 \
		 ? PIXEL_FORMAT_ARGB_8888                                                          \
		 : (DT_INST_ENUM_IDX(inst, pixel_format) == 1 ? PIXEL_FORMAT_RGB_888               \
							      : PIXEL_FORMAT_RGB_565))
#define LTDC_BPP(inst)                                                                             \
	(DT_INST_ENUM_IDX(inst, pixel_format) == 0 ? 4 : (DT_INST_ENUM_IDX(inst, pixel_format) == 1 ? 3 : 2))

static int ltdc_blanking_on(const struct device *dev)
{
	ARG_UNUSED(dev);
	return -ENOSYS;
}

static int ltdc_blanking_off(const struct device *dev)
{
	ARG_UNUSED(dev);
	return -ENOSYS;
}

static int ltdc_write(const struct device *dev, const uint16_t x, const uint16_t y,
		      const struct display_buffer_descriptor *desc, const void *buf)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(x);
	ARG_UNUSED(y);
	ARG_UNUSED(desc);
	ARG_UNUSED(buf);
	return -ENOSYS;
}

static void ltdc_get_capabilities(const struct device *dev, struct display_capabilities *caps)
{
	const struct ltdc_cfg *c = dev->config;

	memset(caps, 0, sizeof(*caps));
	caps->x_resolution = c->hactive;
	caps->y_resolution = c->vactive;
	caps->supported_pixel_formats = c->format;
	caps->current_pixel_format = c->format;
	caps->current_orientation = DISPLAY_ORIENTATION_NORMAL;
}

static int ltdc_set_pixel_format(const struct device *dev, const enum display_pixel_format pf)
{
	ARG_UNUSED(dev);
	ARG_UNUSED(pf);
	return -ENOSYS;
}

static DEVICE_API(display, ltdc_api) = {
	.blanking_on = ltdc_blanking_on,
	.blanking_off = ltdc_blanking_off,
	.write = ltdc_write,
	.get_capabilities = ltdc_get_capabilities,
	.set_pixel_format = ltdc_set_pixel_format,
};

static int ltdc_init(const struct device *dev)
{
	struct ltdc_data *d = dev->data;

	k_mutex_init(&d->lock);
	return 0;
}

#define LTDC_DSI(inst)                                                                             \
	COND_CODE_1(DT_INST_NODE_HAS_PROP(inst, dsi_host),                                         \
		    (DEVICE_DT_GET(DT_INST_PHANDLE(inst, dsi_host))), (NULL))

#define LTDC_DEFINE(inst)                                                                          \
	static const struct ltdc_cfg ltdc_cfg_##inst = {                                           \
		.phys = DT_INST_REG_ADDR(inst),                                                    \
		.size = DT_INST_REG_SIZE(inst),                                                    \
		.hactive = DT_INST_PROP(inst, hactive),                                            \
		.vactive = DT_INST_PROP(inst, vactive),                                            \
		.format = LTDC_FORMAT(inst),                                                       \
		.pitch_bytes = DT_INST_PROP_OR(inst, pitch_bytes,                                  \
					       DT_INST_PROP(inst, hactive) * LTDC_BPP(inst)),      \
		.fb_size = DT_INST_PROP_OR(inst, fb_size,                                          \
					   DT_INST_PROP(inst, hactive) * LTDC_BPP(inst) *          \
						   DT_INST_PROP(inst, vactive)),                   \
		.hsync_low = DT_INST_PROP(inst, hsync_active_low),                                 \
		.vsync_low = DT_INST_PROP(inst, vsync_active_low),                                 \
		.de_low = DT_INST_PROP(inst, de_active_low),                                       \
		.pixclk_falling = DT_INST_PROP(inst, pixclk_falling),                              \
		.dsi = LTDC_DSI(inst),                                                             \
	};                                                                                         \
	static struct ltdc_data ltdc_data_##inst;                                                  \
	DEVICE_DT_INST_DEFINE(inst, ltdc_init, NULL, &ltdc_data_##inst, &ltdc_cfg_##inst,          \
			      POST_KERNEL, CONFIG_YAKOGL_DRV_DISPLAY_STM32_LTDC_INIT_PRIORITY,     \
			      &ltdc_api);

DT_INST_FOREACH_STATUS_OKAY(LTDC_DEFINE)
