/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_display: gets the LTDC display device, writes the eight colour bars
 * (pattern.c, what display_check/colorbars.py judges), blanking off, and
 * prints
 *   mp2_display: PASS: <w>x<h> <fmt>
 *   mp2_display: FAIL: <step> <err>
 * then keeps scanning out CONFIG_MP2_DISPLAY_HOLD_S seconds, so the runner's
 * MP2_HOLD_S and the KVM can see the picture. The board layer (stm32mp2.c)
 * resets the board through PSCI at its lifetime.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/display.h>
#include <zephyr/sys/printk.h>

#include "pattern.h"

#define LTDC_NODE DT_NODELABEL(ltdc)

BUILD_ASSERT(DT_NODE_HAS_STATUS_OKAY(LTDC_NODE), "the overlay enables the ltdc node");
BUILD_ASSERT(CONFIG_MP2_DISPLAY_HOLD_S + 20 < CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the board layer's lifetime covers the hold");

/* A linear buffer the LTDC layer scans out; 64-byte aligned (cache line,
 * and the LTDC's bus alignment is the Q7 answer's).
 */
#define FB_W DT_PROP(LTDC_NODE, hactive)
#define FB_H DT_PROP(LTDC_NODE, vactive)
static uint8_t fb[FB_W * FB_H * 4] __aligned(64);

static int fail(const char *step, int err)
{
	printk("mp2_display: FAIL: %s %d\n", step, err);
	return err;
}

static void hold(void)
{
	for (int s = 0; s < CONFIG_MP2_DISPLAY_HOLD_S; s++) {
		k_sleep(K_SECONDS(1));
	}
}

static int show(const char **fmt_name, unsigned int *w, unsigned int *h)
{
	const struct device *dev = DEVICE_DT_GET(LTDC_NODE);
	struct display_capabilities caps;
	struct display_buffer_descriptor desc;
	enum pattern_format pf;
	int err;

	if (!device_is_ready(dev)) {
		return fail("ltdc-ready", -ENODEV);
	}
	display_get_capabilities(dev, &caps);
	switch (caps.current_pixel_format) {
	case PIXEL_FORMAT_ARGB_8888:
		pf = PATTERN_ARGB8888;
		*fmt_name = "argb8888";
		break;
	case PIXEL_FORMAT_RGB_888:
		pf = PATTERN_RGB888;
		*fmt_name = "rgb888";
		break;
	case PIXEL_FORMAT_RGB_565:
		pf = PATTERN_RGB565;
		*fmt_name = "rgb565";
		break;
	default:
		return fail("format", -ENOTSUP);
	}
	*w = caps.x_resolution;
	*h = caps.y_resolution;
	if (*w > FB_W || *h > FB_H || (size_t)*w * *h * pattern_bpp(pf) > sizeof(fb)) {
		return fail("size", -EINVAL);
	}

	err = pattern_fill(fb, *w, *h, (size_t)*w * pattern_bpp(pf), pf);
	if (err) {
		return fail("pattern", err);
	}
	desc.buf_size = (size_t)*w * *h * pattern_bpp(pf);
	desc.width = *w;
	desc.height = *h;
	desc.pitch = *w;
	desc.frame_incomplete = false;
	err = display_write(dev, 0, 0, &desc, fb);
	if (err) {
		return fail("write", err);
	}
	err = display_blanking_off(dev);
	if (err) {
		return fail("blanking-off", err);
	}
	return 0;
}

int main(void)
{
	const char *fmt = "?";
	unsigned int w = 0, h = 0;

	printk("mp2_display: build %s, hold %d s\n", MP2_DISPLAY_BUILD, CONFIG_MP2_DISPLAY_HOLD_S);
	if (show(&fmt, &w, &h) == 0) {
		printk("mp2_display: PASS: %ux%u %s\n", w, h, fmt);
	}
	/* The picture (or the failure) stays up for the runner and the KVM. */
	hold();
	return 0;
}
