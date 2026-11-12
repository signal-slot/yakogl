/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_display_state: what ST's boot chain (TF-A, OP-TEE, U-Boot) leaves set
 * of the display path of the STM32MP257F-DK (README.md). It READS registers
 * and prints them, one line each:
 *
 *   mp2_display_state: <block> <name> 0x<addr> '=' 0x<val> (see LINE)
 *
 * and ends with "mp2_display_state: PASS: <n> registers read". Nothing is
 * written: the only accessor is sys_read32() below. A read that faults
 * (a bus error from a firewall) goes to the board layer's fatal-error
 * handler, which prints the fault and resets through PSCI; the last
 * "block <name> begin" line names the block, and there is no PASS line.
 *
 * Order: the RIFSC first (which blocks may the non-secure A35 touch), then
 * only what it allows; the groups of least certain address last.
 * RM0457 was not available when this was written: the chapter named at each
 * block is the manual's, the offsets are ST's Linux/U-Boot/OP-TEE
 * (docs/notes/stm32mp257f-dk-display.md), see README.md "Confidence".
 */
#include <stdio.h>

#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include <stm32mp2.h>

/* The line format; the "=" is an argument so that the audit grep for an
 * assignment of a hex literal stays meaningful for this file.
 */
#define LINE "mp2_display_state: %s %s 0x%08lx %c 0x%08x\n"

/* ---- RM0457 "RIFSC" (resource isolation framework security controller) ----
 * RIFSC 0x42080000. Per peripheral id: RISC_SECCFGR at 0x10 + 4 * (id / 32),
 * RISC_PRIVCFGR at 0x30 + 4 * (id / 32), RISC_RCFGLOCKR at 0x50, the
 * compartment filter RISC_PERx_CIDCFGR at 0x100 + 8 * id and its semaphore
 * RISC_PERx_SEMCR at 0x104 + 8 * id (OP-TEE stm32_rifsc.c). RIMU (master
 * isolation): RISC_RIMC_CR 0xC00, RISC_RIMC_ATTRx 0xC10 + 4 * x.
 * Ids: ST OP-TEE stm32mp25-rifsc.h.
 */
#define RIFSC_PHYS         0x42080000UL
#define RIFSC_SECCFGR(n)   (0x10 + 4 * (n))
#define RIFSC_PRIVCFGR(n)  (0x30 + 4 * (n))
#define RIFSC_CIDCFGR(id)  (0x100 + 8 * (id))
#define RIFSC_SEMCR(id)    (0x104 + 8 * (id))
#define RIFSC_RIMC_CR      0xC00
#define RIFSC_RIMC_ATTR(x) (0xC10 + 4 * (x))
#define RIFSC_RIMU_COUNT   16 /* the DK's OP-TEE table has sixteen entries */

#define RIF_I2C2     42
#define RIF_LTDC     80  /* common registers */
#define RIF_DSI      81
#define RIF_LTDC_L12 119 /* layers 1 and 2 */
#define RIF_LTDC_L3  120 /* layer 3 */
#define RIF_LTDC_ROT 121 /* rotation */

#define CIDCFGR_CFEN  (1u << 0)
#define CIDCFGR_SEMEN (1u << 1)
#define CIDCFGR_SCID(v) (((v) >> 4) & 7u)
#define CID_A35_NS    1u /* the CID ST's Linux runs with (OP-TEE ca35tdcid) */

/* ---- RM0457 "RCC": PERxCFGR registers, base 0x44200000 ----
 * Bit 0 reset (1 = held in reset), bit 1 clock enable, bit 2 enable in
 * sleep; DSICFGR bit 12 DSIBLSEL, bit 15 DSIPHYCKREFSEL. RCC_GPUCFGR 0x83C
 * is soc/stm32mp2/rcc.rs; LTDCCFGR/DSICFGR 0x840/0x844 are ST Linux
 * stm32mp25_rcc.h:439-440.
 */
#define RCC_PHYS     0x44200000UL
#define RCC_GPUCFGR  0x83C
#define RCC_LTDCCFGR 0x840
#define RCC_DSICFGR  0x844
#define RCC_PERCFGR_RST (1u << 0)
#define RCC_PERCFGR_EN  (1u << 1)

/* ---- RM0457 "LTDC": base 0x48010000 (stm32mp251.dtsi:2150). Global block
 * (ST Linux ltdc.c:69-94); layer 1 at 0x100 (LAY_OFS_1, ltdc.c:64-66).
 */
#define LTDC_PHYS 0x48010000UL
#define LTDC_IDR_EXPECT_A 0x00040100u /* HWVER_40100 */
#define LTDC_IDR_EXPECT_B 0x00040101u /* HWVER_40101 */

/* ---- RM0457 "DSI": base 0x48000000 (stm32mp255.dtsi:123-126). Host
 * registers are the DesignWare map, 0x400-0x43c ST's wrapper
 * (dw_mipi_dsi-stm.c:37-107).
 */
#define DSI_PHYS 0x48000000UL

/* ---- RM0457 "GPIO": GPIOA 0x44240000, one bank per 0x10000. */
#define GPIOB_PHYS 0x44250000UL /* PB4 ADV7535 interrupt, PB6 ADV7535 reset */
#define GPIOF_PHYS 0x44290000UL /* PF0 I2C2 SDA, PF2 I2C2 SCL (AF9) */

/* ---- RM0457 "SYSCFG": DISPLAYCLKCR at SYSCFG 0x44230000 + 0x5000
 * (clk-stm32mp25-syscfg.c:13): 0 txbyteclk, 1 clk_pix_lvds, 2 ck_ker_ltdc.
 */
#define DISPCLK_PHYS 0x44235000UL

/* ---- RM0457 "RISAF" (DDR firewall, RISAF4): base 0x42100000 (unconfirmed).
 * RISAF_CR 0x00; region n (1-based) at 0x40 + 0x40 * (n - 1): CFGR +0,
 * STARTR +4, ENDR +8, CIDCFGR +0xC (OP-TEE stm32_risaf.c). Regions 13
 * (LTDC secure layers, 0xfe800000, 8 MiB), 14 (rotation, 0xff000000,
 * 16 MiB) and 10 (the GPU carveout, 0xfa800000) per the DK's RIF dtsi.
 */
#define RISAF4_PHYS 0x42100000UL
#define RISAF_REG(n) (0x40 + 0x40 * ((n) - 1u))

static unsigned int nread;

/* The one accessor of this image: a 32 bit read, printed. */
static uint32_t rd(const char *blk, const char *name, mm_reg_t va, uintptr_t pa, uint32_t off)
{
	uint32_t v = sys_read32(va + off);

	nread++;
	printk(LINE, blk, name, (unsigned long)(pa + off), '=', (unsigned int)v);
	return v;
}

static void begin(const char *blk)
{
	printk("mp2_display_state: block %s begin\n", blk);
}

static void skip(const char *blk, const char *why)
{
	printk("mp2_display_state: block %s SKIPPED: %s\n", blk, why);
}

static mm_reg_t map(uintptr_t pa)
{
	mm_reg_t va;

	device_map(&va, pa, 0x1000, K_MEM_CACHE_NONE);
	return va;
}

/* Reads the RIFSC entry of `id` and says whether the non-secure A35 (CID 1)
 * may touch the peripheral: not secure-only, and no compartment filter that
 * excludes CID 1 (a semaphore-mode entry needs the semaphore: skipped).
 */
static bool rif_open(mm_reg_t rifsc, const char *blk, uint32_t id)
{
	char name[24];
	uint32_t sec, priv, cid;

	snprintf(name, sizeof(name), "sec_id%u", (unsigned int)id);
	sec = rd("rifsc", name, rifsc, RIFSC_PHYS, RIFSC_SECCFGR(id / 32));
	snprintf(name, sizeof(name), "priv_id%u", (unsigned int)id);
	priv = rd("rifsc", name, rifsc, RIFSC_PHYS, RIFSC_PRIVCFGR(id / 32));
	snprintf(name, sizeof(name), "cid_id%u", (unsigned int)id);
	cid = rd("rifsc", name, rifsc, RIFSC_PHYS, RIFSC_CIDCFGR(id));
	snprintf(name, sizeof(name), "sem_id%u", (unsigned int)id);
	(void)rd("rifsc", name, rifsc, RIFSC_PHYS, RIFSC_SEMCR(id));

	if (sec & (1u << (id % 32))) {
		printk("mp2_display_state: %s id %u secure-only (RISC_SECCFGR)\n", blk,
		       (unsigned int)id);
		return false;
	}
	if (priv & (1u << (id % 32))) {
		printk("mp2_display_state: %s id %u privileged-only (EL1 is privileged: read)\n",
		       blk, (unsigned int)id);
	}
	if (cid & CIDCFGR_CFEN) {
		if (cid & CIDCFGR_SEMEN) {
			printk("mp2_display_state: %s id %u compartment semaphore mode\n", blk,
			       (unsigned int)id);
			return false;
		}
		if (CIDCFGR_SCID(cid) != CID_A35_NS) {
			printk("mp2_display_state: %s id %u static CID %u, not %u\n", blk,
			       (unsigned int)id, (unsigned int)CIDCFGR_SCID(cid),
			       (unsigned int)CID_A35_NS);
			return false;
		}
	}
	return true;
}

static void read_ltdc(mm_reg_t rcc, mm_reg_t rifsc)
{
	static const struct {
		const char *name;
		uint16_t off;
	} regs[] = {
		{"idr", 0x00},   {"lcr", 0x04},    {"sscr", 0x08},   {"bpcr", 0x0c},
		{"awcr", 0x10},  {"twcr", 0x14},   {"gcr", 0x18},    {"gc1r", 0x1c},
		{"gc2r", 0x20},  {"srcr", 0x24},   {"gacr", 0x28},   {"bccr", 0x2c},
		{"ier", 0x34},   {"isr", 0x38},    {"lipcr", 0x40},  {"cpsr", 0x44},
		{"cdsr", 0x48},  {"edcr", 0x60},   {"fut", 0x90},
		/* layer 1 */
		{"l1c0r", 0x100}, {"l1c1r", 0x104}, {"l1rcr", 0x108}, {"l1cr", 0x10c},
		{"l1whpcr", 0x110}, {"l1wvpcr", 0x114}, {"l1pfcr", 0x11c},
		{"l1cfbar", 0x134}, {"l1cfblr", 0x138}, {"l1cfblnr", 0x13c},
	};
	uint32_t cfgr, idr;
	mm_reg_t ltdc;

	begin("ltdc");
	cfgr = rd("rcc", "ltdccfgr", rcc, RCC_PHYS, RCC_LTDCCFGR);
	if (!rif_open(rifsc, "ltdc", RIF_LTDC) || !rif_open(rifsc, "ltdc", RIF_LTDC_L12)) {
		skip("ltdc", "RIFSC does not open the LTDC to the non-secure A35");
		return;
	}
	(void)rif_open(rifsc, "ltdc", RIF_LTDC_L3);
	(void)rif_open(rifsc, "ltdc", RIF_LTDC_ROT);
	if (!(cfgr & RCC_PERCFGR_EN) || (cfgr & RCC_PERCFGR_RST)) {
		skip("ltdc", "RCC_LTDCCFGR: clock off or reset held");
		return;
	}
	ltdc = map(LTDC_PHYS);
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		uint32_t v = rd("ltdc", regs[i].name, ltdc, LTDC_PHYS, regs[i].off);

		if (i == 0) {
			idr = v;
			if (idr != LTDC_IDR_EXPECT_A && idr != LTDC_IDR_EXPECT_B) {
				printk("mp2_display_state: ltdc idr 0x%08x is neither 0x%08x nor "
				       "0x%08x (HWVER_40100/40101)\n",
				       (unsigned int)idr, (unsigned int)LTDC_IDR_EXPECT_A,
				       (unsigned int)LTDC_IDR_EXPECT_B);
			}
		}
	}
}

static void read_dsi(mm_reg_t rcc, mm_reg_t rifsc)
{
	static const struct {
		const char *name;
		uint16_t off;
	} regs[] = {
		/* DesignWare host */
		{"version", 0x00}, {"pwr_up", 0x04}, {"clkmgr_cfg", 0x08}, {"dpi_vcid", 0x0c},
		{"dpi_color_coding", 0x10}, {"dpi_cfg_pol", 0x14}, {"pckhdl_cfg", 0x2c},
		{"mode_cfg", 0x34}, {"vid_mode_cfg", 0x38}, {"vid_pkt_size", 0x3c},
		{"vid_hsa_time", 0x48}, {"vid_hbp_time", 0x4c}, {"vid_hline_time", 0x50},
		{"vid_vsa_lines", 0x54}, {"vid_vbp_lines", 0x58}, {"vid_vfp_lines", 0x5c},
		{"vid_vactive_lines", 0x60}, {"lpclk_ctrl", 0x94}, {"phy_tmr_lpclk_cfg", 0x98},
		{"phy_tmr_cfg", 0x9c}, {"pctlr", 0xa0}, {"phy_if_cfg", 0xa4}, {"psr", 0xb0},
		{"ptcr0", 0xb4},
		/* ST wrapper */
		{"wcfgr", 0x400}, {"wcr", 0x404}, {"wisr", 0x40c}, {"wpcr1", 0x430},
		{"wrpcr0", 0x434}, {"wrpcr1", 0x438}, {"wrpcr2", 0x43c},
	};
	uint32_t cfgr;
	mm_reg_t dsi;

	begin("dsi");
	cfgr = rd("rcc", "dsicfgr", rcc, RCC_PHYS, RCC_DSICFGR);
	if (!rif_open(rifsc, "dsi", RIF_DSI)) {
		skip("dsi", "RIFSC does not open the DSI to the non-secure A35");
		return;
	}
	if (!(cfgr & RCC_PERCFGR_EN) || (cfgr & RCC_PERCFGR_RST)) {
		skip("dsi", "RCC_DSICFGR: clock off or reset held");
		return;
	}
	dsi = map(DSI_PHYS);
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		(void)rd("dsi", regs[i].name, dsi, DSI_PHYS, regs[i].off);
	}
}

static void read_gpio(const char *blk, uintptr_t pa)
{
	static const struct {
		const char *name;
		uint8_t off;
	} regs[] = {
		{"moder", 0x00}, {"otyper", 0x04}, {"ospeedr", 0x08}, {"pupdr", 0x0c},
		{"idr", 0x10},   {"odr", 0x14},    {"lckr", 0x1c},    {"afrl", 0x20},
		{"afrh", 0x24},  {"seccfgr", 0x30}, {"privcfgr", 0x34},
	};
	mm_reg_t gpio;

	begin(blk);
	gpio = map(pa);
	for (size_t i = 0; i < ARRAY_SIZE(regs); i++) {
		(void)rd(blk, regs[i].name, gpio, pa, regs[i].off);
	}
}

int main(void)
{
	mm_reg_t rcc = map(RCC_PHYS);
	mm_reg_t rifsc = map(RIFSC_PHYS);

	printk("mp2_display_state: build %s\n", MP2_DISPLAY_STATE_BUILD);
	printk("mp2_display_state: read only; one line per register, then PASS or FAIL\n");

	/* The RIFSC: the compartment of the I2C2, and the RIMU (masters). */
	begin("rifsc");
	(void)rif_open(rifsc, "i2c2", RIF_I2C2);
	(void)rd("rifsc", "rimc_cr", rifsc, RIFSC_PHYS, RIFSC_RIMC_CR);
	for (int i = 0; i < RIFSC_RIMU_COUNT; i++) {
		char name[16];

		snprintf(name, sizeof(name), "rimc_attr%d", i);
		(void)rd("rifsc", name, rifsc, RIFSC_PHYS, RIFSC_RIMC_ATTR(i));
	}

	/* The RCC gates and resets of the display blocks (GPUCFGR as a
	 * cross-check against mp2_gpu).
	 */
	begin("rcc");
	(void)rd("rcc", "gpucfgr", rcc, RCC_PHYS, RCC_GPUCFGR);
	(void)rd("rcc", "ltdccfgr", rcc, RCC_PHYS, RCC_LTDCCFGR);
	(void)rd("rcc", "dsicfgr", rcc, RCC_PHYS, RCC_DSICFGR);

	read_ltdc(rcc, rifsc);
	read_dsi(rcc, rifsc);

#ifdef CONFIG_MP2_DISPLAY_STATE_RCC_RAW
	/* Flexgen, PLL4/PLL8, cross-bar, I2C2 and GPIO gates: raw, non-zero. */
	begin("rcc_raw");
	for (uint32_t off = 0; off < 0x1000; off += 4) {
		uint32_t v = sys_read32(rcc + off);

		if (v != 0) {
			nread++;
			printk(LINE, "rcc_raw", "word", (unsigned long)(RCC_PHYS + off), '=',
			       (unsigned int)v);
		}
	}
#endif
#ifdef CONFIG_MP2_DISPLAY_STATE_GPIO
	read_gpio("gpiob", GPIOB_PHYS);
	read_gpio("gpiof", GPIOF_PHYS);
#endif
#ifdef CONFIG_MP2_DISPLAY_STATE_SYSCFG
	begin("syscfg");
	(void)rd("syscfg", "displayclkcr", map(DISPCLK_PHYS), DISPCLK_PHYS, 0);
#endif
#ifdef CONFIG_MP2_DISPLAY_STATE_RISAF
	begin("risaf4");
	{
		mm_reg_t risaf = map(RISAF4_PHYS);
		static const uint8_t regions[] = {10, 13, 14};
		static const char *const names[3][4] = {
			{"reg10_cfgr", "reg10_startr", "reg10_endr", "reg10_cidcfgr"},
			{"reg13_cfgr", "reg13_startr", "reg13_endr", "reg13_cidcfgr"},
			{"reg14_cfgr", "reg14_startr", "reg14_endr", "reg14_cidcfgr"},
		};

		(void)rd("risaf4", "cr", risaf, RISAF4_PHYS, 0);
		for (size_t r = 0; r < ARRAY_SIZE(regions); r++) {
			uint32_t base = RISAF_REG(regions[r]);

			for (uint32_t k = 0; k < 4; k++) {
				(void)rd("risaf4", names[r][k], risaf, RISAF4_PHYS, base + 4 * k);
			}
		}
	}
#endif

	if (nread > 0) {
		printk("mp2_display_state: PASS: %u registers read\n", nread);
	} else {
		printk("mp2_display_state: FAIL: no register read\n");
	}
	mp2_reset_to_linux("mp2_display_state done");
}
