/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_display: brings the DK's chain up through the LTDC display device (the
 * chain owner, display_yakogl_stm32_ltdc.c; the board layer powered the path
 * before, display_power_mp2.c), writes the eight colour bars (pattern.c,
 * what display_check/colorbars.py judges) line by line with display_write(),
 * turns blanking off, checks MP2_DISPLAY_FRAMES frames and prints
 *   mp2_display: PASS: <w>x<h> <fmt> <n> frames
 *   mp2_display: FAIL: <step> <err>
 * PASS only when the chain is up (LTDC on, DSI streaming, ADV7535 started)
 * and every frame came with no transfer or underrun error. It then keeps
 * scanning out CONFIG_MP2_DISPLAY_HOLD_S seconds (one frame checked a
 * second), so the runner's MP2_HOLD_S and the KVM can see the picture. The
 * board layer (stm32mp2.c) resets the board through PSCI at its lifetime.
 *
 * The LTDC's frame CRC (ip::stm32_ltdc) is not in the C ABI
 * (yakogl_mp25_display.h): the frame check is the LTDC's line interrupt and
 * error flags.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/display.h>
#include <zephyr/sys/printk.h>

#include <yakogl_mp25_display.h>
#include <yakogl_drv/adv7535.h>
#include <yakogl_drv/stm32_dsi.h>
#include <yakogl_drv/stm32_ltdc.h>

#include "pattern.h"

#define LTDC_NODE DT_NODELABEL(ltdc)
#define DSI_NODE  DT_PHANDLE(LTDC_NODE, dsi_host)
#define ADV_NODE  DT_PHANDLE(DSI_NODE, bridge)

BUILD_ASSERT(DT_NODE_HAS_STATUS_OKAY(LTDC_NODE), "the overlay enables the ltdc node");
BUILD_ASSERT(CONFIG_MP2_DISPLAY_HOLD_S + 20 < CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the board layer's lifetime covers the hold");

/* Frames that must come clean before PASS: one second at 60 Hz. */
#define MP2_DISPLAY_FRAMES 60

/* One line of the largest mode the node allows; the bars are vertical, so
 * every line is the same. */
#define LINE_W DT_PROP(LTDC_NODE, hactive)
static uint8_t line[LINE_W * 4] __aligned(64);
static uint32_t rust_line[LINE_W];

static int fail(const char *step, int err)
{
	printk("mp2_display: FAIL: %s %d\n", step, err);
	return err;
}

static const char *const step_names[] = {
	"none", "board", "fb", "adv-init", "mode", "clock", "ltdc-probe", "ltdc-configure",
	"ltdc-enable", "dsi-mode", "dsi", "adv-start", "frame", "i2c",
};

static void print_record(const struct stm32_ltdc_status *st, const struct device *dsi,
			 const struct device *adv)
{
	const struct yd_mp25_dsi_data *dd = stm32_dsi_data(dsi);
	const struct yd_mp25_adv7535_data *ad = adv7535_data(adv);
	const struct yd_mp25_mode *m = &st->mode;

	printk("mp2_display: chain step %s err %d, hpd %u, edid %u (header %02x %02x, ext %u), "
	       "choice %d\n",
	       st->step < ARRAY_SIZE(step_names) ? step_names[st->step] : "?", (int)st->err,
	       st->hpd, st->edid_ok, st->edid[0], st->edid[1], st->edid[126], (int)st->choice);
	printk("mp2_display: mode %ux%u %u Hz h %u/%u/%u v %u/%u/%u flags 0x%x, %s vic %u\n",
	       m->hactive, m->vactive, m->pixel_clock_hz, m->hfront, m->hsync, m->hback,
	       m->vfront, m->vsync, m->vback, m->flags, st->sink.hdmi ? "hdmi" : "dvi",
	       st->sink.vic);
	printk("mp2_display: ltdc hwver 0x%06x bus %u layers %u on %u, pitch %u, fb 0x%llx, "
	       "clock %u Hz\n",
	       st->ltdc.hwver, st->ltdc.bus_bytes, st->ltdc.layers, st->ltdc.on, st->pitch,
	       (unsigned long long)st->fb_phys, st->pixel_clock_hz);
	if (dd != NULL) {
		printk("mp2_display: dsi on %u lanes %u lane %u kbit/s (timings %u Mbit/s) pll "
		       "idf %u ndiv %u odf %u, hline %u, failed step %u errno %d\n",
		       dd->on, dd->lanes, dd->lane_kbps, dd->lane_mbps, dd->pll_idf, dd->pll_ndiv,
		       dd->pll_odf, dd->hline, dd->failed_step, (int)dd->failed_errno);
	}
	if (ad != NULL) {
		printk("mp2_display: adv7535 rev 0x%02x hpd %u started %u\n", ad->revision, ad->hpd,
		       ad->started);
	}
}

/* pattern.c's line against the crate's yd_mp25_fill_color_bars (both claim
 * colorbars.py's order); a note only: pattern.c is the one the host test
 * judges. */
static void compare_bars(unsigned int w)
{
	const uint32_t *ours = (const uint32_t *)line;
	int rc = yd_mp25_fill_color_bars(rust_line, w, w, 1, w);

	if (rc != 0) {
		printk("mp2_display: bars: yd_mp25_fill_color_bars %d\n", rc);
		return;
	}
	for (unsigned int x = 0; x < w; x++) {
		if (ours[x] != rust_line[x]) {
			printk("mp2_display: bars: differ at x %u: 0x%08x, crate 0x%08x\n", x,
			       ours[x], rust_line[x]);
			return;
		}
	}
	printk("mp2_display: bars: equal to the crate's\n");
}

static int show(const struct device *dev, const char **fmt_name, unsigned int *w,
		unsigned int *h, int *frames)
{
	const struct device *dsi = DEVICE_DT_GET(DSI_NODE);
	const struct device *adv = DEVICE_DT_GET(ADV_NODE);
	const struct stm32_ltdc_status *st;
	const struct yd_mp25_dsi_data *dd;
	const struct yd_mp25_adv7535_data *ad;
	struct display_capabilities caps;
	struct display_buffer_descriptor desc;
	enum pattern_format pf;
	size_t row;
	uint32_t isr = 0;
	int err;

	if (!device_is_ready(dev)) {
		return fail("ltdc-ready", -ENODEV);
	}
	st = stm32_ltdc_status(dev);
	display_get_capabilities(dev, &caps);
	if (caps.x_resolution == 0u || caps.y_resolution == 0u) {
		print_record(st, dsi, adv);
		return fail(st->step < ARRAY_SIZE(step_names) ? step_names[st->step] : "select",
			    st->err != 0 ? st->err : -EIO);
	}
	switch (caps.current_pixel_format) {
	case PIXEL_FORMAT_ARGB_8888:
		pf = PATTERN_ARGB8888;
		*fmt_name = "argb8888";
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
	row = (size_t)*w * pattern_bpp(pf);
	if (*w > LINE_W || row > sizeof(line)) {
		return fail("size", -EINVAL);
	}

	err = pattern_fill(line, *w, 1, row, pf);
	if (err) {
		return fail("pattern", err);
	}
	if (pf == PATTERN_ARGB8888) {
		compare_bars(*w);
	}
	desc = (struct display_buffer_descriptor){
		.buf_size = row,
		.width = *w,
		.height = 1,
		.pitch = *w,
		.frame_incomplete = false,
	};
	for (unsigned int y = 0; y < *h; y++) {
		err = display_write(dev, 0, y, &desc, line);
		if (err) {
			return fail("write", err);
		}
	}

	err = display_blanking_off(dev);
	print_record(st, dsi, adv);
	if (err) {
		return fail(st->step < ARRAY_SIZE(step_names) ? step_names[st->step]
							     : "blanking-off",
			    err);
	}
	dd = stm32_dsi_data(dsi);
	ad = adv7535_data(adv);
	if (!st->on || !st->ltdc.on || dd == NULL || !dd->on || ad == NULL || !ad->started) {
		return fail("chain", -EIO);
	}

	/* The frame path: every frame's line interrupt, no error flag. */
	for (*frames = 0; *frames < MP2_DISPLAY_FRAMES; (*frames)++) {
		err = stm32_ltdc_wait_frame(dev, &isr);
		if (err) {
			printk("mp2_display: frame %d: isr 0x%x\n", *frames, isr);
			return fail("frame", err);
		}
	}
	return 0;
}

int main(void)
{
	const struct device *dev = DEVICE_DT_GET(LTDC_NODE);
	const char *fmt = "?";
	unsigned int w = 0, h = 0;
	int frames = 0;
	int good = 0, bad = 0;
	bool up;

	printk("mp2_display: build %s, hold %d s\n", MP2_DISPLAY_BUILD, CONFIG_MP2_DISPLAY_HOLD_S);
	up = show(dev, &fmt, &w, &h, &frames) == 0;
	if (up) {
		printk("mp2_display: PASS: %ux%u %s %d frames\n", w, h, fmt, frames);
	}
	/* The picture (or the failure) stays up for the runner and the KVM. */
	for (int s = 0; s < CONFIG_MP2_DISPLAY_HOLD_S; s++) {
		k_sleep(K_SECONDS(1));
		if (up) {
			uint32_t isr = 0;

			if (stm32_ltdc_wait_frame(dev, &isr) == 0) {
				good++;
			} else {
				bad++;
				printk("mp2_display: hold: frame at %d s failed, isr 0x%x\n", s, isr);
			}
		}
	}
	if (up) {
		printk("mp2_display: hold: %d frames ok, %d failed\n", good, bad);
	}
	return 0;
}
