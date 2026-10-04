/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx8mp_lcdif_adopt.c -- the imx8mp_display_* API over an LCDIFv3 that the
 * previous kernel left running (safeui,imx8mp-lcdif-adopt).
 *
 * The full pipeline (imx8mp_display.c) owns the Verdin's LCDIF1 -> DSIM ->
 * SN65DSI84 chain and can bring it up from cold. A board whose chain the
 * driver cannot yet bring up -- the Armadillo-X2's LCDIF3 -> PVI -> DW-HDMI
 * -> HDMI PHY -- still gets a picture on the crash route, because nothing is
 * shut down there: the chain keeps scanning whatever Linux last gave it, and
 * all an image has to do is point the layer at its own buffer. This file does
 * exactly that and nothing else. It never touches a clock, a power domain or
 * anything downstream of the LCDIF.
 *
 * Built instead of imx8mp_display.c (zephyr/CMakeLists.txt), so the samples
 * and the Qt glue call the same functions on either kind of board.
 */
#define DT_DRV_COMPAT safeui_imx8mp_lcdif_adopt

#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include "display_internal.h"

LOG_MODULE_REGISTER(imx8mp_disp, CONFIG_SAFEUI_DISPLAY_IMX8MP_LOG_LEVEL);

BUILD_ASSERT(DT_NUM_INST_STATUS_OKAY(DT_DRV_COMPAT) == 1,
	     "exactly one safeui,imx8mp-lcdif-adopt node");

#define NODE DT_DRV_INST(0)

/* The two LCDIFv3 layer registers the adopt path writes besides the address
 * (imx8mp_lcdif.c has the whole map; these two are all this file needs). */
#define LCDC_V8_CTRLDESCL0_3      0x208
#define LCDC_V8_CTRLDESCL0_5      0x214
#define CTRLDESCL0_3_PITCH_MASK   0xffffu
#define CTRLDESCL0_5_BPP_MASK     GENMASK(27, 24)
#define CTRLDESCL0_5_BPP_32_ARGB8888 (0x9u << 24)

static mm_reg_t s_lcdif;
static struct imx8mp_display_info s_info;
static uint64_t s_linux_fb;   /* what the previous kernel was scanning */
static uint32_t s_linux_desc3, s_linux_desc5;   /* ...and its pitch and format */
static uint32_t s_our_pitch;   /* the pitch of our buffers */
static bool s_layout_ours;     /* the layer has our pitch and format */

/* Our buffers are XRGB8888 whatever the previous client used (fbcon may run
 * RGB565), and their pitch is ours. Both go through the same shadow load as
 * the address that follows, so the panel switches in one frame. */
static void apply_our_layout(void)
{
	wr32(s_lcdif, LCDC_V8_CTRLDESCL0_3,
	     (s_linux_desc3 & ~CTRLDESCL0_3_PITCH_MASK) | s_our_pitch);
	wr32(s_lcdif, LCDC_V8_CTRLDESCL0_5,
	     (s_linux_desc5 & ~CTRLDESCL0_5_BPP_MASK) | CTRLDESCL0_5_BPP_32_ARGB8888);
	s_layout_ours = true;
}

static int adopt_init(const struct device *dev)
{
	ARG_UNUSED(dev);
	device_map(&s_lcdif, DT_REG_ADDR(NODE), DT_REG_SIZE(NODE), K_MEM_CACHE_NONE);
	return 0;
}
DEVICE_DT_INST_DEFINE(0, adopt_init, NULL, NULL, NULL, POST_KERNEL,
		      CONFIG_SAFEUI_DISPLAY_IMX8MP_INIT_PRIORITY, NULL);

const struct imx8mp_display_info *imx8mp_display_info(void)
{
	return &s_info;
}

bool imx8mp_display_is_running(void)
{
	return s_lcdif != 0 && lcdif_is_running(s_lcdif);
}

uint32_t imx8mp_display_layer_control(void)
{
	return s_lcdif != 0 ? lcdif_layer_control(s_lcdif) : 0u;
}

void imx8mp_display_geometry(uint32_t *width, uint32_t *height)
{
	*width = DT_PROP(NODE, hactive);
	*height = DT_PROP(NODE, vactive);
}

unsigned int imx8mp_display_health(void)
{
	return imx8mp_display_is_running() ? IMX8MP_DISPLAY_HEALTH_LCDIF_RUNNING : 0u;
}

uint64_t imx8mp_display_scanout(void)
{
	uint32_t w, h, pitch, bpp;
	uint64_t phys;

	if (s_lcdif == 0 || !lcdif_read_scanout(s_lcdif, &w, &h, &pitch, &bpp, &phys)) {
		return 0;
	}
	return phys;
}

int imx8mp_display_on(uintptr_t fb_phys, uint32_t pitch)
{
	uint32_t w, h, run_pitch, bpp;
	uint64_t phys;

	if (s_lcdif == 0) {
		return -ENODEV;
	}
	if (!lcdif_read_scanout(s_lcdif, &w, &h, &run_pitch, &bpp, &phys)) {
		LOG_ERR("the LCDIF is not scanning: nothing to adopt (this mode never "
			"brings a chain up)");
		return -ENODEV;
	}
	if (w != DT_PROP(NODE, hactive) || h != DT_PROP(NODE, vactive)) {
		LOG_ERR("the LCDIF runs %ux%u, the devicetree expects %ux%u", w, h,
			(unsigned int)DT_PROP(NODE, hactive),
			(unsigned int)DT_PROP(NODE, vactive));
		return -EINVAL;
	}
	if (pitch == 0u) {
		pitch = w * 4u;
	}
	/* What the previous kernel was scanning is captured once, at the first
	 * adopt: a second call finds our own buffer in the registers, and
	 * imx8mp_display_off() must hand back Linux's, not that. */
	if (s_linux_fb == 0u) {
		s_linux_fb = phys;
		s_linux_desc3 = rd32(s_lcdif, LCDC_V8_CTRLDESCL0_3);
		s_linux_desc5 = rd32(s_lcdif, LCDC_V8_CTRLDESCL0_5);
	}
	s_info.adopted = true;
	s_info.width = w;
	s_info.height = h;
	s_info.bytes_pp = 4;
	s_info.on = true;

	/*
	 * No address: adopt and change nothing, as imx8mp_display.c does
	 * ("adopting can do without an address"). The samples call
	 * imx8mp_display_on(0, 0) first, to own the display before any GL
	 * work, and the real buffer comes later. Writing 0 here instead -- as
	 * this file did until 2026-10-01 -- points the LCDIF's DMA at physical
	 * address zero: 9 MB of boot ROM, CAAM secure RAM, OCRAM_S and the
	 * M7's TCM, scanned 60 times a second. On the Armadillo-X2 that wedged
	 * the interconnect within a second of the call: the UART stopped
	 * mid-line and the watchdog took the board (tools/cts/RESULTS.md,
	 * "Armadillo-X2: first gc_slint run").
	 */
	if (fb_phys == 0u) {
		s_info.pitch = run_pitch;
		s_our_pitch = pitch;
		s_layout_ours = false;
		LOG_INF("adopting the LCDIF: %ux%u, %u bpp, pitch %u at 0x%llx, scan-out kept",
			w, h, bpp * 8u, run_pitch, (unsigned long long)phys);
		return 0;
	}

	s_info.pitch = pitch;
	s_our_pitch = pitch;
	LOG_INF("adopting the LCDIF: %ux%u, %u bpp, pitch %u at 0x%llx -> ours, pitch %u",
		w, h, bpp * 8u, run_pitch, (unsigned long long)phys, pitch);
	apply_our_layout();
	if (lcdif_set_fb(s_lcdif, fb_phys) != 0) {
		LOG_ERR("the LCDIF did not latch the first frame");
		return -ETIMEDOUT;
	}
	return 0;
}

int imx8mp_display_set_fb(uintptr_t fb_phys)
{
	if (!s_info.on) {
		return -ENODEV;
	}
	if (fb_phys == 0u) {
		return -EINVAL;   /* never scan out of physical zero; see above */
	}
	if (!s_layout_ours) {
		/* Adopted with no address: the first buffer of ours brings our
		 * pitch and format with it, in the same shadow load. */
		s_info.pitch = s_our_pitch;
		apply_our_layout();
	}
	return lcdif_set_fb(s_lcdif, fb_phys);
}

void imx8mp_display_off(void)
{
	/* Nothing downstream is ours to stop, so "off" gives the panel back
	 * the buffer, pitch and format the previous kernel was scanning. */
	if (s_info.on && s_linux_fb != 0u) {
		wr32(s_lcdif, LCDC_V8_CTRLDESCL0_3, s_linux_desc3);
		wr32(s_lcdif, LCDC_V8_CTRLDESCL0_5, s_linux_desc5);
		(void)lcdif_set_fb(s_lcdif, s_linux_fb);
	}
	s_layout_ours = false;
	s_info.on = false;
}
