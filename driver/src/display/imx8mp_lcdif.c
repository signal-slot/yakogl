/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * LCDIFv3 (i.MX8MP "LCDIF1", 0x32e80000): one linear XRGB8888 layer scanned
 * out to the DSIM with the panel's timings.
 *
 * Register names, offsets and fields are those of mainline Linux
 * drivers/gpu/drm/mxsfb/lcdif_regs.h ("V8 register set"); the write order is
 * lcdif_kms.c: lcdif_reset_block() -> lcdif_set_formats() -> lcdif_set_mode()
 * -> address -> lcdif_enable_controller(), and a page flip is the address
 * pair plus CTRLDESCL0_5_SHADOW_LOAD_EN (lcdif_crtc_atomic_flush()).
 *
 * Sync polarity: LCDC_V8_CTRL is left at 0, i.e. HS, VS and DE active HIGH.
 * That is not the panel's polarity (the Riverdi timing says hsync-active 0)
 * but the polarity the i.MX8MP glue between LCDIFv3 and DSIM needs;
 * samsung-dsim.c forces exactly this in samsung_dsim_atomic_check() for
 * DSIM_TYPE_IMX8MP, and the panel's own polarity is applied further down by
 * the SN65DSI84's LVDS_FMT register.
 */

#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>

#include "display_internal.h"

LOG_MODULE_DECLARE(imx8mp_disp, CONFIG_SAFEUI_DISPLAY_IMX8MP_LOG_LEVEL);

#define REG_SET 4
#define REG_CLR 8

#define CTRL_INV_PXCK BIT(3)   /* lcdif_regs.h: sample pixels on the falling edge */
#define LCDC_V8_CTRL              0x000
#define LCDC_V8_DISP_PARA         0x010
#define LCDC_V8_DISP_SIZE         0x014
#define LCDC_V8_HSYN_PARA         0x018
#define LCDC_V8_VSYN_PARA         0x01c
#define LCDC_V8_VSYN_HSYN_WIDTH   0x020
#define LCDC_V8_INT_ENABLE_D1     0x034
#define LCDC_V8_CTRLDESCL0_1      0x200
#define LCDC_V8_CTRLDESCL0_3      0x208
#define LCDC_V8_CTRLDESCL_LOW0_4  0x20c
#define LCDC_V8_CTRLDESCL_HIGH0_4 0x210
#define LCDC_V8_CTRLDESCL0_5      0x214
#define LCDC_V8_CSC0_CTRL         0x21c
#define LCDC_V8_PANIC0_THRES      0x238

#define CTRL_SW_RESET             BIT(31)

#define DISP_PARA_DISP_ON              BIT(31)
#define DISP_PARA_LINE_PATTERN_RGB888  (0x0u << 26)

#define DISP_SIZE_DELTA_Y(n) (((n) & 0xffffu) << 16)
#define DISP_SIZE_DELTA_X(n) ((n) & 0xffffu)
#define HSYN_PARA_BP_H(n)    (((n) & 0xffffu) << 16)
#define HSYN_PARA_FP_H(n)    ((n) & 0xffffu)
#define VSYN_PARA_BP_V(n)    (((n) & 0xffffu) << 16)
#define VSYN_PARA_FP_V(n)    ((n) & 0xffffu)
#define VSYN_HSYN_WIDTH_PW_V(n) (((n) & 0xffffu) << 16)
#define VSYN_HSYN_WIDTH_PW_H(n) ((n) & 0xffffu)

#define INT_ENABLE_D1_PLANE_PANIC_EN BIT(0)
#define PANIC0_THRES_LOW_MASK  GENMASK(24, 16)
#define PANIC0_THRES_HIGH_MASK GENMASK(8, 0)
#define PANIC0_THRES_MAX       511

#define CTRLDESCL0_1_HEIGHT(n) (((n) & 0xffffu) << 16)
#define CTRLDESCL0_1_WIDTH(n)  ((n) & 0xffffu)
#define CTRLDESCL0_1_GET_HEIGHT(r) (((r) >> 16) & 0xffffu)
#define CTRLDESCL0_1_GET_WIDTH(r)  ((r) & 0xffffu)

/* P_SIZE and T_SIZE are the undocumented AXI burst sizes; 2 is the 256-byte
 * burst the downstream kernel uses and mainline copies. */
#define CTRLDESCL0_3_P_SIZE(n) (((n) << 20) & GENMASK(22, 20))
#define CTRLDESCL0_3_T_SIZE(n) (((n) << 16) & GENMASK(17, 16))
#define CTRLDESCL0_3_PITCH(n)  ((n) & 0xffffu)
#define CTRLDESCL0_3_GET_PITCH(r) ((r) & 0xffffu)

#define CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK 0xfu

#define CTRLDESCL0_5_EN              BIT(31)
#define CTRLDESCL0_5_SHADOW_LOAD_EN  BIT(30)
#define CTRLDESCL0_5_BPP_MASK        GENMASK(27, 24)
#define CTRLDESCL0_5_BPP_16_RGB565   (0x4u << 24)
#define CTRLDESCL0_5_BPP_32_ARGB8888 (0x9u << 24)
#define CTRLDESCL0_5_BPP_32_ABGR8888 (0xau << 24)

#define CSC0_CTRL_BYPASS BIT(0)

#define LCDIF_MIN_XRES 120
#define LCDIF_MIN_YRES 120
#define LCDIF_SANE_MAX 4096

/* lcdif_disable_controller() waits 36 ms, about two frames; so do we. */
#define SHADOW_LOAD_TIMEOUT_US 36000

bool lcdif_is_running(mm_reg_t lcdif)
{
	return (rd32(lcdif, LCDC_V8_CTRLDESCL0_5) & CTRLDESCL0_5_EN) != 0u &&
	       (rd32(lcdif, LCDC_V8_DISP_PARA) & DISP_PARA_DISP_ON) != 0u;
}

/* CTRLDESCL0_5 as it stands, for a diagnostic that wants the pixel format. */
uint32_t lcdif_layer_control(mm_reg_t lcdif)
{
	return rd32(lcdif, LCDC_V8_CTRLDESCL0_5);
}

bool lcdif_read_scanout(mm_reg_t lcdif, uint32_t *w, uint32_t *h, uint32_t *pitch,
			uint32_t *bytes_pp, uint64_t *phys)
{
	uint32_t desc5 = rd32(lcdif, LCDC_V8_CTRLDESCL0_5);
	uint32_t desc1, desc3, bpp;

	if (!lcdif_is_running(lcdif)) {
		return false;
	}
	switch (desc5 & CTRLDESCL0_5_BPP_MASK) {
	case CTRLDESCL0_5_BPP_32_ARGB8888:
	case CTRLDESCL0_5_BPP_32_ABGR8888:
		bpp = 4;
		break;
	case CTRLDESCL0_5_BPP_16_RGB565:
		bpp = 2;
		break;
	default:
		return false;
	}
	desc1 = rd32(lcdif, LCDC_V8_CTRLDESCL0_1);
	desc3 = rd32(lcdif, LCDC_V8_CTRLDESCL0_3);
	*w = CTRLDESCL0_1_GET_WIDTH(desc1);
	*h = CTRLDESCL0_1_GET_HEIGHT(desc1);
	*pitch = CTRLDESCL0_3_GET_PITCH(desc3);
	*bytes_pp = bpp;
	if (*w < LCDIF_MIN_XRES || *w > LCDIF_SANE_MAX || *h < LCDIF_MIN_YRES ||
	    *h > LCDIF_SANE_MAX || *pitch < *w * bpp) {
		return false;
	}
	*phys = ((uint64_t)(rd32(lcdif, LCDC_V8_CTRLDESCL_HIGH0_4) &
			    CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK) << 32) |
		rd32(lcdif, LCDC_V8_CTRLDESCL_LOW0_4);
	return true;
}

static void lcdif_write_addr(mm_reg_t lcdif, uint64_t phys)
{
	wr32(lcdif, LCDC_V8_CTRLDESCL_LOW0_4, (uint32_t)phys);
	wr32(lcdif, LCDC_V8_CTRLDESCL_HIGH0_4,
	     (uint32_t)(phys >> 32) & CTRLDESCL_HIGH0_4_ADDR_HIGH_MASK);
}

int lcdif_set_fb(mm_reg_t lcdif, uint64_t fb_phys)
{
	lcdif_write_addr(lcdif, fb_phys);
	set32(lcdif, LCDC_V8_CTRLDESCL0_5, CTRLDESCL0_5_SHADOW_LOAD_EN);

	for (uint32_t t = 0; t < SHADOW_LOAD_TIMEOUT_US; t += 50) {
		if ((rd32(lcdif, LCDC_V8_CTRLDESCL0_5) &
		     CTRLDESCL0_5_SHADOW_LOAD_EN) == 0u) {
			return 0;
		}
		k_busy_wait(50);
	}
	return -ETIMEDOUT;
}

int lcdif_configure(struct display_ctx *c, uint64_t fb_phys, uint32_t pitch)
{
	const struct display_mode *m = &c->mode;

	/* "Mandatory eLCDIF reset as per the Reference Manual"
	 * (lcdif_reset_block()). */
	wr32(c->lcdif, LCDC_V8_CTRL + REG_SET, CTRL_SW_RESET);
	(void)rd32(c->lcdif, LCDC_V8_CTRL);
	wr32(c->lcdif, LCDC_V8_CTRL + REG_CLR, CTRL_SW_RESET);
	(void)rd32(c->lcdif, LCDC_V8_CTRL);

	/* lcdif_set_formats(): RGB888 on the bus, XRGB8888 in memory, CSC
	 * bypassed. Both writes replace the whole register, which is also
	 * how DISP_PARA_DISP_ON ends up clear until lcdif_enable(). */
	wr32(c->lcdif, LCDC_V8_DISP_PARA, DISP_PARA_LINE_PATTERN_RGB888);
	wr32(c->lcdif, LCDC_V8_CTRLDESCL0_5, CTRLDESCL0_5_BPP_32_ARGB8888);
	wr32(c->lcdif, LCDC_V8_CSC0_CTRL, CSC0_CTRL_BYPASS);

	/* lcdif_set_mode(). HS, VS and DE active high (see the file comment),
	 * and the pixel clock inverted: lcdif_kms.c sets CTRL_INV_PXCK from
	 * DRM_BUS_FLAG_PIXDATA_DRIVE_NEGEDGE on the DSIM's input bus, and the
	 * board's own Linux has CTRL = 0x08 while it displays (golden dump,
	 * 2026-09-21). With the bit clear the DSIM samples the pixel data on
	 * the wrong edge and nothing complains: PLLs lock, no picture. */
	wr32(c->lcdif, LCDC_V8_CTRL, CTRL_INV_PXCK);
	wr32(c->lcdif, LCDC_V8_DISP_SIZE,
	     DISP_SIZE_DELTA_Y(m->vactive) | DISP_SIZE_DELTA_X(m->hactive));
	wr32(c->lcdif, LCDC_V8_HSYN_PARA,
	     HSYN_PARA_BP_H(m->hback) | HSYN_PARA_FP_H(m->hfront));
	wr32(c->lcdif, LCDC_V8_VSYN_PARA,
	     VSYN_PARA_BP_V(m->vback) | VSYN_PARA_FP_V(m->vfront));
	wr32(c->lcdif, LCDC_V8_VSYN_HSYN_WIDTH,
	     VSYN_HSYN_WIDTH_PW_V(m->vsync) | VSYN_HSYN_WIDTH_PW_H(m->hsync));
	wr32(c->lcdif, LCDC_V8_CTRLDESCL0_1,
	     CTRLDESCL0_1_HEIGHT(m->vactive) | CTRLDESCL0_1_WIDTH(m->hactive));
	wr32(c->lcdif, LCDC_V8_CTRLDESCL0_3,
	     CTRLDESCL0_3_P_SIZE(2) | CTRLDESCL0_3_T_SIZE(2) | CTRLDESCL0_3_PITCH(pitch));

	/* Written before the layer is enabled so the first frame is not
	 * scanned out of whatever address the reset left behind. */
	lcdif_write_addr(c->lcdif, fb_phys);
	return 0;
}

int lcdif_enable(struct display_ctx *c)
{
	/* FIFO panic watermarks at 1/3 and 2/3, and the panic-driven NoC
	 * priority boost (it raises no interrupt). lcdif_enable_controller(). */
	wr32(c->lcdif, LCDC_V8_PANIC0_THRES,
	     FIELD_PREP(PANIC0_THRES_LOW_MASK, 1 * PANIC0_THRES_MAX / 3) |
	     FIELD_PREP(PANIC0_THRES_HIGH_MASK, 2 * PANIC0_THRES_MAX / 3));
	wr32(c->lcdif, LCDC_V8_INT_ENABLE_D1, INT_ENABLE_D1_PLANE_PANIC_EN);

	set32(c->lcdif, LCDC_V8_DISP_PARA, DISP_PARA_DISP_ON);
	set32(c->lcdif, LCDC_V8_CTRLDESCL0_5, CTRLDESCL0_5_EN);
	return 0;
}

void lcdif_disable(struct display_ctx *c)
{
	clr32(c->lcdif, LCDC_V8_CTRLDESCL0_5, CTRLDESCL0_5_EN);
	for (uint32_t t = 0; t < SHADOW_LOAD_TIMEOUT_US; t += 50) {
		if ((rd32(c->lcdif, LCDC_V8_CTRLDESCL0_5) & CTRLDESCL0_5_EN) == 0u) {
			break;
		}
		k_busy_wait(50);
	}
	clr32(c->lcdif, LCDC_V8_DISP_PARA, DISP_PARA_DISP_ON);
	wr32(c->lcdif, LCDC_V8_INT_ENABLE_D1, 0);
}
