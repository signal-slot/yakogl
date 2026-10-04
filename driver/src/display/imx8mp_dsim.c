/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Samsung MIPI DSIM host and its D-PHY on the i.MX8MP (0x32e60000),
 * driving four data lanes in RGB888 video mode.
 *
 * Re-derived from mainline Linux drivers/gpu/drm/bridge/samsung-dsim.c and
 * include/drm/bridge/samsung-dsim.h. The i.MX8M parts (DSIM_TYPE_IMX8MP)
 * use exynos5433_reg_ofs[] for the register map, plltmr_reg 0xa0,
 * pll_p_offset 14, m in 64..1023, a VCO of 1050..2100 MHz, and
 * imx8mm_dsim_reg_values[] for the fixed PHYCTRL fields; every one of those
 * numbers is repeated below next to the name it has there.
 *
 * The order is samsung_dsim_init() for a non-Exynos part:
 *   SWRST -> PLL + CLKCTRL -> PHYCTRL/PHYTIMING -> init_link -> display mode
 *   -> MAIN_STAND_BY, all with FORCE_STOP_STATE still asserted, so the clock
 *   lane runs in HS but no video goes out. The SN65DSI84 is programmed in
 *   that window (its CSRs must be written while the DSI clock is in HS), and
 *   dsim_video_start() then releases FORCE_STOP_STATE
 *   (samsung_dsim_atomic_enable -> samsung_dsim_set_stop_state(false)).
 */

#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>

#include "display_internal.h"

LOG_MODULE_DECLARE(imx8mp_disp, CONFIG_SAFEUI_DISPLAY_IMX8MP_LOG_LEVEL);

/* exynos5433_reg_ofs[], which DSIM_TYPE_IMX8MP shares. */
#define DSIM_STATUS_REG      0x04
#define DSIM_SWRST_REG       0x0c
#define DSIM_CLKCTRL_REG     0x10
#define DSIM_TIMEOUT_REG     0x14
#define DSIM_CONFIG_REG      0x18
#define DSIM_ESCMODE_REG     0x1c
#define DSIM_MDRESOL_REG     0x20
#define DSIM_MVPORCH_REG     0x24
#define DSIM_MHPORCH_REG     0x28
#define DSIM_MSYNC_REG       0x2c
#define DSIM_INTSRC_REG      0x34
#define DSIM_FIFOCTRL_REG    0x4c
#define DSIM_PLLTMR_REG      0xa0   /* imx8mm_dsi_driver_data.plltmr_reg */
#define DSIM_PLLCTRL_REG     0x94
#define DSIM_PHYCTRL_REG     0xa4
#define DSIM_PHYTIMING_REG   0xb4
#define DSIM_PHYTIMING1_REG  0xb8
#define DSIM_PHYTIMING2_REG  0xbc

/* DSIM_STATUS_REG */
#define DSIM_STOP_STATE_DAT(x) (((x) & 0xfu) << 0)
#define DSIM_STOP_STATE_CLK    BIT(8)
#define DSIM_TX_READY_HS_CLK   BIT(10)
#define DSIM_PLL_STABLE        BIT(31)

/* DSIM_SWRST_REG */
#define DSIM_SWRST BIT(0)

/* DSIM_CLKCTRL_REG */
#define DSIM_ESC_PRESCALER(x)        (((x) & 0xffffu) << 0)
#define DSIM_ESC_PRESCALER_MASK      (0xffffu << 0)
#define DSIM_LANE_ESC_CLK_EN_CLK     BIT(19)
#define DSIM_LANE_ESC_CLK_EN_DATA(x) (((x) & 0xfu) << 20)
#define DSIM_LANE_ESC_CLK_EN_DATA_MASK (0xfu << 20)
#define DSIM_BYTE_CLKEN              BIT(24)
#define DSIM_BYTE_CLK_SRC(x)         (((x) & 0x3u) << 25)
#define DSIM_BYTE_CLK_SRC_MASK       (0x3u << 25)
#define DSIM_PLL_BYPASS              BIT(27)
#define DSIM_ESC_CLKEN               BIT(28)
#define DSIM_TX_REQUEST_HSCLK        BIT(31)

/* DSIM_CONFIG_REG */
#define DSIM_LANE_EN_CLK             BIT(0)
#define DSIM_LANE_EN(x)              (((x) & 0xfu) << 1)
#define DSIM_NUM_OF_DATA_LANE(x)     (((x) & 0x3u) << 5)
#define DSIM_MAIN_PIX_FORMAT_RGB888  (0x7u << 12)
#define DSIM_VIDEO_MODE              BIT(25)
#define DSIM_BURST_MODE              BIT(26)
#define DSIM_MFLUSH_VS               BIT(29)

/* DSIM_ESCMODE_REG */
#define DSIM_FORCE_STOP_STATE   BIT(20)
#define DSIM_STOP_STATE_CNT(x)  (((x) & 0x7ffu) << 21)
#define DSIM_STOP_STATE_CNT_MASK (0x7ffu << 21)

/* DSIM_MDRESOL_REG: 12 resolution bits on the i.MX8M (num_bits_resol). */
#define DSIM_MAIN_STAND_BY   BIT(31)
#define DSIM_MAIN_VRESOL(x)  (((x) & 0xfffu) << 16)
#define DSIM_MAIN_HRESOL(x)  (((x) & 0xfffu) << 0)

/* DSIM_MVPORCH_REG / DSIM_MHPORCH_REG / DSIM_MSYNC_REG */
#define DSIM_CMD_ALLOW(x)  ((uint32_t)(x) << 28)
#define DSIM_STABLE_VFP(x) ((uint32_t)(x) << 16)
#define DSIM_MAIN_VBP(x)   ((uint32_t)(x) << 0)
#define DSIM_MAIN_HFP(x)   ((uint32_t)(x) << 16)
#define DSIM_MAIN_HBP(x)   ((uint32_t)(x) << 0)
#define DSIM_MAIN_VSA(x)   ((uint32_t)(x) << 22)
#define DSIM_MAIN_HSA(x)   ((uint32_t)(x) << 0)

/* DSIM_TIMEOUT_REG */
#define DSIM_LPDR_TIMEOUT(x) ((uint32_t)(x) << 0)
#define DSIM_BTA_TIMEOUT(x)  ((uint32_t)(x) << 16)

/* DSIM_PLLCTRL_REG, with imx8mm_dsi_driver_data.pll_p_offset = 14. */
#define DSIM_PLL_EN     BIT(23)
#define DSIM_PLL_P(x)   ((uint32_t)(x) << 14)
#define DSIM_PLL_M(x)   ((uint32_t)(x) << 4)
#define DSIM_PLL_S(x)   ((uint32_t)(x) << 1)

/* DSIM_PHYTIMING_REG* */
#define DSIM_PHYTIMING_LPX(x)          ((uint32_t)(x) << 8)
#define DSIM_PHYTIMING_HS_EXIT(x)      ((uint32_t)(x) << 0)
#define DSIM_PHYTIMING1_CLK_PREPARE(x) ((uint32_t)(x) << 24)
#define DSIM_PHYTIMING1_CLK_ZERO(x)    ((uint32_t)(x) << 16)
#define DSIM_PHYTIMING1_CLK_POST(x)    ((uint32_t)(x) << 8)
#define DSIM_PHYTIMING1_CLK_TRAIL(x)   ((uint32_t)(x) << 0)
#define DSIM_PHYTIMING2_HS_PREPARE(x)  ((uint32_t)(x) << 16)
#define DSIM_PHYTIMING2_HS_ZERO(x)     ((uint32_t)(x) << 8)
#define DSIM_PHYTIMING2_HS_TRAIL(x)    ((uint32_t)(x) << 0)

/* imx8mm_dsim_reg_values[] and imx8mm_dsi_driver_data. */
#define DSIM_PLL_TIMER        0x8000   /* the board's Linux: 0x8000, not mainline's 500 */
#define DSIM_STOP_STATE_COUNT 0        /* the board's Linux: ESCMODE reads 0; mainline uses 0xf */
#define DSIM_PHYCTRL_VALUE    0        /* ULPS_EXIT 0, VREG_LP 0, SLEW_UP 0 */
#define DSIM_PLL_M_MIN        64u
#define DSIM_PLL_M_MAX        1023u
#define DSIM_PLL_VCO_MIN_HZ   1050000000ull
#define DSIM_PLL_VCO_MAX_HZ   2100000000ull

/*
 * samsung_dsim_pll_find_pms(): pick p, m and s so that fin * m / (p << s) is
 * as close as possible to fout, with fin / p inside the PLL's input window,
 * m inside the part's range and the VCO (fin * m / p) inside it too.
 *
 * The input window is 2..30 MHz on this DSIM, not the 6..12 MHz this used
 * to assume: the board's Linux runs the panel with a 12 MHz reference and
 * p 5 (2.4 MHz into the PLL), m 711, s 2 -- 426.6 MHz exactly (PLLCTRL
 * 0x00816c74, live, 2026-09-23). With the narrower window the search could
 * only reach 426.0 MHz.
 */
static uint32_t pll_find_pms(uint32_t fin, uint32_t fout, uint8_t *pp, uint16_t *pm, uint8_t *ps)
{
	uint32_t best = 0, min_delta = 0xffffffffu;
	uint8_t p_min = (uint8_t)MAX(DIV_ROUND_UP(fin, MHZ(30)), 1u);
	uint8_t p_max = (uint8_t)MIN(fin / MHZ(2), 63u);

	for (uint8_t p = p_min; p <= p_max && p != 0; p++) {
		for (uint8_t s = 0; s <= 5; s++) {
			uint64_t tmp = (uint64_t)fout * ((uint32_t)p << s);
			uint32_t m = (uint32_t)(tmp / fin);
			uint64_t vco, f;
			uint32_t delta;

			if (m < DSIM_PLL_M_MIN || m > DSIM_PLL_M_MAX) {
				continue;
			}
			vco = (uint64_t)m * fin / p;
			if (vco < DSIM_PLL_VCO_MIN_HZ || vco > DSIM_PLL_VCO_MAX_HZ) {
				continue;
			}
			f = (uint64_t)m * fin / ((uint32_t)p << s);
			delta = (uint32_t)(f > fout ? f - fout : fout - f);
			if (delta < min_delta) {
				min_delta = delta;
				best = (uint32_t)f;
				*pp = p;
				*pm = (uint16_t)m;
				*ps = s;
			}
		}
	}
	return best;
}

static uint32_t dsim_set_pll(struct display_ctx *c, uint32_t freq)
{
	uint8_t p = 0, s = 0;
	uint16_t m = 0;
	uint32_t fout = pll_find_pms(c->pll_ref_hz, freq, &p, &m, &s);

	if (fout == 0) {
		LOG_ERR("DSIM: no PLL setting for %u Hz from a %u Hz reference", freq,
			c->pll_ref_hz);
		return 0;
	}
	wr32(c->dsim, DSIM_PLLTMR_REG, DSIM_PLL_TIMER);
	wr32(c->dsim, DSIM_PLLCTRL_REG,
	     DSIM_PLL_EN | DSIM_PLL_P(p) | DSIM_PLL_M(m) | DSIM_PLL_S(s));

	/* Linux spins on DSIM_PLL_STABLE with a 1000-iteration budget; give
	 * it a millisecond of real time instead. */
	for (uint32_t t = 0; t < 1000; t++) {
		if (rd32(c->dsim, DSIM_STATUS_REG) & DSIM_PLL_STABLE) {
			c->info.pll_p = p;
			c->info.pll_m = m;
			c->info.pll_s = s;
			c->info.dsi_hs_clock_hz = fout;
			LOG_INF("DSIM PLL: p %u m %u s %u -> %u Hz (wanted %u)", p, m, s,
				fout, freq);
			return fout;
		}
		k_busy_wait(1);
	}
	LOG_ERR("DSIM: PLL did not stabilise (STATUS 0x%08x)", rd32(c->dsim, DSIM_STATUS_REG));
	return 0;
}

static int dsim_enable_clock(struct display_ctx *c)
{
	uint32_t pix = c->info.pixel_clock_hz;
	uint32_t hs, byte_clk, esc_div, esc_clk, reg;

	/* A fixed burst clock when the device tree names one (Linux's plan for
	 * this panel: 426.6 MHz for a 69.3 MHz pixel clock, 2.6 % above the
	 * video stream). Without one, the bare pixel * bpp / lanes leaves the
	 * link no room for packet overhead: the bridge's line buffer runs dry
	 * and the picture breaks up (the 2026-09-23 FORCE_COLD viewing). */
	hs = dsim_set_pll(c, c->burst_clock_hz != 0u
				   ? c->burst_clock_hz
				   : (uint32_t)DIV_ROUND_UP((uint64_t)pix * c->bpp, c->lanes));
	if (hs == 0) {
		return -EIO;
	}
	byte_clk = hs / 8u;
	if (c->esc_clock_hz == 0u) {
		LOG_ERR("DSIM: esc-clock-frequency is 0 in the device tree");
		return -EINVAL;
	}
	esc_div = DIV_ROUND_UP(byte_clk, c->esc_clock_hz);
	esc_clk = byte_clk / esc_div;
	if (esc_clk > MHZ(20)) {
		esc_div++;
		esc_clk = byte_clk / esc_div;
	}
	c->info.dsi_byte_clock_hz = byte_clk;
	c->info.dsi_esc_clock_hz = esc_clk;

	reg = rd32(c->dsim, DSIM_CLKCTRL_REG);
	reg &= ~(DSIM_ESC_PRESCALER_MASK | DSIM_LANE_ESC_CLK_EN_CLK |
		 DSIM_LANE_ESC_CLK_EN_DATA_MASK | DSIM_PLL_BYPASS | DSIM_BYTE_CLK_SRC_MASK);
	reg |= DSIM_ESC_CLKEN | DSIM_BYTE_CLKEN | DSIM_ESC_PRESCALER(esc_div) |
	       DSIM_LANE_ESC_CLK_EN_CLK | DSIM_LANE_ESC_CLK_EN_DATA(BIT(c->lanes) - 1) |
	       DSIM_BYTE_CLK_SRC(0) | DSIM_TX_REQUEST_HSCLK;
	wr32(c->dsim, DSIM_CLKCTRL_REG, reg);

	LOG_INF("DSIM clocks: hs %u, byte %u, esc %u (prescaler %u)", hs, byte_clk, esc_clk,
		esc_div);
	return 0;
}

/*
 * phy_mipi_dphy_get_default_config_for_hsclk() from
 * drivers/phy/phy-core-mipi-dphy.c, in picoseconds, with ui the unit
 * interval rounded up (ALIGN(PSEC_PER_SEC, rate) / rate).
 */
struct dphy_timing_ps {
	uint32_t lpx, hs_exit, clk_prepare, clk_zero, clk_post, clk_trail;
	uint32_t hs_prepare, hs_zero, hs_trail;
};

static void dphy_default_config(uint32_t hs_clk_hz, struct dphy_timing_ps *t)
{
	uint64_t ui = DIV_ROUND_UP(1000000000000ull, hs_clk_hz);

	t->lpx = 50000u;
	t->hs_exit = 100000u;
	t->clk_prepare = 38000u;
	t->clk_zero = 262000u;
	t->clk_post = (uint32_t)(60000u + 52u * ui);
	t->clk_trail = 60000u;
	t->hs_prepare = (uint32_t)(40000u + 4u * ui);
	t->hs_zero = (uint32_t)(105000u + 6u * ui);
	/*
	 * The D-PHY spec gives T_HS-TRAIL as max(n * 8 * ui, 60 + n * 4 * ui)
	 * with n = 1 forward and n = 4 reverse. phy_mipi_dphy_calc_config()
	 * has one setting and picks n = 4, the conservative one; so do we.
	 */
	t->hs_trail = (uint32_t)MAX(4u * 8u * ui, 60000u + 4u * 4u * ui);
}

/* PS_TO_CYCLE(ps, hz): picoseconds to cycles of `hz`, rounded to nearest. */
static uint32_t ps_to_cycle(uint32_t ps, uint32_t hz)
{
	uint64_t num = (uint64_t)ps * hz;

	return (uint32_t)((num + 500000000000ull) / 1000000000000ull);
}

static void dsim_set_phy_ctrl(struct display_ctx *c)
{
	uint32_t byte_clk = c->info.dsi_byte_clock_hz;
	struct dphy_timing_ps t;

	dphy_default_config(c->info.dsi_hs_clock_hz, &t);

	wr32(c->dsim, DSIM_PHYCTRL_REG, DSIM_PHYCTRL_VALUE);
	/*
	 * The board's Linux (NXP's sec-dsim) takes the D-PHY timings from a
	 * table rather than from the D-PHY minimums rounded to nearest, and
	 * at its 426.6 MHz HS clock it writes PHYTIMING1 0x03120904 and
	 * PHYTIMING2 0x00040507 (golden dump, 2026-09-21): CLK_PREPARE 3,
	 * CLK_ZERO 18, CLK_POST 9, CLK_TRAIL 4, HS_PREPARE 4, HS_ZERO 5,
	 * HS_TRAIL 7 byte clocks. The formula below gives 2/14/10/3 and
	 * 3/6/5, with CLK_PREPARE a hair under the 38 ns minimum. In the
	 * band the panel runs in, use what the hardware is known to work
	 * with; elsewhere the formula stands.
	 */
	if (c->info.dsi_hs_clock_hz >= 400000000u && c->info.dsi_hs_clock_hz <= 450000000u) {
		wr32(c->dsim, DSIM_PHYTIMING_REG,
		     DSIM_PHYTIMING_LPX(3) | DSIM_PHYTIMING_HS_EXIT(5));
		wr32(c->dsim, DSIM_PHYTIMING1_REG,
		     DSIM_PHYTIMING1_CLK_PREPARE(3) | DSIM_PHYTIMING1_CLK_ZERO(18) |
		     DSIM_PHYTIMING1_CLK_POST(9) | DSIM_PHYTIMING1_CLK_TRAIL(4));
		wr32(c->dsim, DSIM_PHYTIMING2_REG,
		     DSIM_PHYTIMING2_HS_PREPARE(4) | DSIM_PHYTIMING2_HS_ZERO(5) |
		     DSIM_PHYTIMING2_HS_TRAIL(7));
		LOG_DBG("DSIM PHYTIMING from the 426 MHz table");
		return;
	}
	wr32(c->dsim, DSIM_PHYTIMING_REG,
	     DSIM_PHYTIMING_LPX(ps_to_cycle(t.lpx, byte_clk)) |
	     DSIM_PHYTIMING_HS_EXIT(ps_to_cycle(t.hs_exit, byte_clk)));
	wr32(c->dsim, DSIM_PHYTIMING1_REG,
	     DSIM_PHYTIMING1_CLK_PREPARE(ps_to_cycle(t.clk_prepare, byte_clk)) |
	     DSIM_PHYTIMING1_CLK_ZERO(ps_to_cycle(t.clk_zero, byte_clk)) |
	     DSIM_PHYTIMING1_CLK_POST(ps_to_cycle(t.clk_post, byte_clk)) |
	     DSIM_PHYTIMING1_CLK_TRAIL(ps_to_cycle(t.clk_trail, byte_clk)));
	wr32(c->dsim, DSIM_PHYTIMING2_REG,
	     DSIM_PHYTIMING2_HS_PREPARE(ps_to_cycle(t.hs_prepare, byte_clk)) |
	     DSIM_PHYTIMING2_HS_ZERO(ps_to_cycle(t.hs_zero, byte_clk)) |
	     DSIM_PHYTIMING2_HS_TRAIL(ps_to_cycle(t.hs_trail, byte_clk)));

	LOG_DBG("DSIM PHYTIMING %08x %08x %08x", rd32(c->dsim, DSIM_PHYTIMING_REG),
		rd32(c->dsim, DSIM_PHYTIMING1_REG), rd32(c->dsim, DSIM_PHYTIMING2_REG));
}

static int dsim_init_link(struct display_ctx *c)
{
	uint32_t lanes_mask = BIT(c->lanes) - 1;
	uint32_t reg;

	/* Initialise the FIFO pointers: the low five bits down, then up. */
	reg = rd32(c->dsim, DSIM_FIFOCTRL_REG) & ~0x1fu;
	wr32(c->dsim, DSIM_FIFOCTRL_REG, reg);
	k_sleep(K_MSEC(10));
	wr32(c->dsim, DSIM_FIFOCTRL_REG, reg | 0x1fu);
	k_sleep(K_MSEC(10));

	/*
	 * Video mode, RGB888, burst mode: CONFIG = 0x0600707f once the lanes
	 * are in, which is what the board's Linux (NXP's sec-dsim) has while
	 * it displays (golden dump, 2026-09-21). Mainline samsung-dsim would
	 * add MFLUSH_VS (bit 29) and the SN65DSI83 driver's NO_HFP/HBP/HSA/
	 * EOT bits; the hardware that works has none of them.
	 */
	reg = DSIM_VIDEO_MODE | DSIM_BURST_MODE | DSIM_MAIN_PIX_FORMAT_RGB888;
	wr32(c->dsim, DSIM_CONFIG_REG, reg);

	/* samsung_dsim_enable_lane() */
	set32(c->dsim, DSIM_CONFIG_REG,
	      DSIM_NUM_OF_DATA_LANE(c->lanes - 1) | DSIM_LANE_EN_CLK |
	      DSIM_LANE_EN(lanes_mask));

	/* Wait for every data lane to reach stop state and the clock lane to
	 * be either stopped or ready in HS. */
	for (uint32_t t = 0; t < 100; t++) {
		reg = rd32(c->dsim, DSIM_STATUS_REG);
		if ((reg & DSIM_STOP_STATE_DAT(lanes_mask)) ==
		    DSIM_STOP_STATE_DAT(lanes_mask) &&
		    (reg & (DSIM_STOP_STATE_CLK | DSIM_TX_READY_HS_CLK))) {
			goto lanes_ready;
		}
		k_busy_wait(100);
	}
	LOG_ERR("DSIM: lanes did not reach stop state (STATUS 0x%08x)",
		rd32(c->dsim, DSIM_STATUS_REG));
	return -EIO;

lanes_ready:
	reg = rd32(c->dsim, DSIM_ESCMODE_REG);
	reg &= ~DSIM_STOP_STATE_CNT_MASK;
	reg |= DSIM_STOP_STATE_CNT(DSIM_STOP_STATE_COUNT);
	/* Non-Exynos parts hold the link in forced stop state until the
	 * downstream bridge is programmed. */
	reg |= DSIM_FORCE_STOP_STATE;
	wr32(c->dsim, DSIM_ESCMODE_REG, reg);

	/* 0x00ff00ff on the board (mainline: LPDR 0xffff). */
	wr32(c->dsim, DSIM_TIMEOUT_REG, DSIM_BTA_TIMEOUT(0xff) | DSIM_LPDR_TIMEOUT(0xff));
	return 0;
}

static void dsim_set_display_mode(struct display_ctx *c)
{
	const struct display_mode *m = &c->mode;
	uint32_t byte_clk_khz = c->info.dsi_byte_clock_hz / 1000u;
	uint32_t pix_khz = c->info.pixel_clock_hz / 1000u;
	int hfp, hbp, hsa;

	/* Porches in DSI byte-clock units, minus the six bytes of packet
	 * overhead each (samsung_dsim_set_display_mode). */
	hfp = (int)((uint32_t)m->hfront * byte_clk_khz / pix_khz) - 6;
	hbp = (int)((uint32_t)m->hback * byte_clk_khz / pix_khz) - 6;
	hsa = (int)((uint32_t)m->hsync * byte_clk_khz / pix_khz) - 6;
	hfp = MAX(hfp, 0);
	hbp = MAX(hbp, 0);
	hsa = MAX(hsa, 0);

	/* CMD_ALLOW 0, not mainline's 0xf: the board's Linux (NXP's sec-dsim,
	 * not samsung-dsim) writes 0x00080018 here, and 15 command lines do
	 * not fit in this panel's 8-line front porch. */
	wr32(c->dsim, DSIM_MVPORCH_REG,
	     DSIM_CMD_ALLOW(0) | DSIM_STABLE_VFP(m->vfront) | DSIM_MAIN_VBP(m->vback));
	wr32(c->dsim, DSIM_MHPORCH_REG, DSIM_MAIN_HFP(hfp) | DSIM_MAIN_HBP(hbp));
	wr32(c->dsim, DSIM_MSYNC_REG, DSIM_MAIN_VSA(m->vsync) | DSIM_MAIN_HSA(hsa));
	wr32(c->dsim, DSIM_MDRESOL_REG,
	     DSIM_MAIN_HRESOL(m->hactive) | DSIM_MAIN_VRESOL(m->vactive));

	LOG_DBG("DSIM porches: hfp %d hbp %d hsa %d (byte clocks)", hfp, hbp, hsa);
}

int dsim_enable(struct display_ctx *c)
{
	int rc;

	/* samsung_dsim_reset(): RESET_TYPE is DSIM_SWRST on the i.MX8M, not
	 * DSIM_FUNCRST. */
	wr32(c->dsim, DSIM_SWRST_REG, DSIM_SWRST);
	k_busy_wait(100);
	wr32(c->dsim, DSIM_INTSRC_REG, rd32(c->dsim, DSIM_INTSRC_REG));

	rc = dsim_enable_clock(c);
	if (rc) {
		return rc;
	}
	/* wait_for_reset is 0 for this part, so no reset-release wait. */
	dsim_set_phy_ctrl(c);
	rc = dsim_init_link(c);
	if (rc) {
		return rc;
	}
	dsim_set_display_mode(c);
	set32(c->dsim, DSIM_MDRESOL_REG, DSIM_MAIN_STAND_BY);
	return 0;
}

void dsim_video_start(struct display_ctx *c)
{
	clr32(c->dsim, DSIM_ESCMODE_REG, DSIM_FORCE_STOP_STATE);
}

bool dsim_pll_is_stable(const struct display_ctx *c)
{
	return (rd32(c->dsim, DSIM_STATUS_REG) & DSIM_PLL_STABLE) != 0u;
}

bool dsim_hs_clock_is_ready(const struct display_ctx *c)
{
	return (rd32(c->dsim, DSIM_STATUS_REG) & DSIM_TX_READY_HS_CLK) != 0u;
}

void dsim_disable(struct display_ctx *c)
{
	set32(c->dsim, DSIM_ESCMODE_REG, DSIM_FORCE_STOP_STATE);
	clr32(c->dsim, DSIM_MDRESOL_REG, DSIM_MAIN_STAND_BY);
	clr32(c->dsim, DSIM_CLKCTRL_REG,
	      DSIM_LANE_ESC_CLK_EN_CLK | DSIM_LANE_ESC_CLK_EN_DATA_MASK |
	      DSIM_ESC_CLKEN | DSIM_BYTE_CLKEN);
	clr32(c->dsim, DSIM_PLLCTRL_REG, DSIM_PLL_EN);
}
