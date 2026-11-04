/* SPDX-License-Identifier: Apache-2.0 */
/*
 * The A53 cluster's clock (CONFIG_YAKOGL_BOARD_IMX8MP_A53_MHZ): this image
 * replaces U-Boot proper, so nothing raises the ARM PLL from what the SPL
 * left (2026-11-04, read back by board_route3_clkdiag.c: 1200 MHz, ARM PLL
 * FDIV_CTL0 0x0012c031, the core on arm_pll_out; VDD_ARM = PCA9450 BUCK2
 * 0x1c = 0.95 V).
 *
 * The rate is one of Linux's for this SoC (arch/arm64/boot/dts/freescale/
 * imx8mp.dtsi, a53_opp_table: 1200 MHz at 850 mV, 1600 MHz at 950 mV), at
 * the voltage the SPL already set, so the PMIC is not touched. The change is
 * Linux's clk-cpu sequence for the i.MX8M (drivers/clk/imx/clk-cpu.c,
 * clk-imx8mp.c): the core mux (CCM 0x9880 bit 24, "arm_a53_core") moves to
 * ARM_A53_CLK_ROOT fed from sys_pll1_800m, the ARM PLL (a PLL1416x,
 * CCM_ANALOG 0x84 GEN_CTRL / 0x88 FDIV_CTL0) is reprogrammed as
 * clk-pll14xx.c's clk_pll1416x_set_rate does (reset, bypass, divider, out of
 * reset, lock, bypass off; LOCK_SEL first), and the core mux moves back.
 * Checked against torvalds/linux master on 2026-11-04. Its 1600 MHz entry
 * in imx_pll1416x_tbl is m 200, p 3, s 0: 24 MHz x 200 / (3 x 2^0).
 *
 * At PRE_KERNEL_2, on core 0, before the other cores start. If the PLL does
 * not lock the core stays on the 800 MHz root, which is safe, and the result
 * is in board_imx8mp_a53_clock_result().
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/kernel/mm.h>
#include <errno.h>
#include "board_imx8mp_a53_clock.h"

#define CCM_ANALOG     0x30360000u
#define CCM            0x30380000u
#define ARM_PLL_GEN    0x84u
#define ARM_PLL_FDIV0  0x88u
#define GEN_LOCK       BIT(31)
#define GEN_LOCK_SEL   BIT(29) /* lock from the PLL output */
#define GEN_RST        BIT(9)  /* RESETB: 1 = out of reset */
#define GEN_BYPASS     BIT(4)
#define CCM_ROOT0      0x8000u /* ARM_A53_CLK_ROOT TARGET_ROOT */
#define CCM_CORE_SEL   0x9880u
#define CORE_SEL_PLL   BIT(24) /* 1: arm_pll_out, 0: ARM_A53_CLK_ROOT */
/* TARGET_ROOT: ENABLE bit 28, MUX 26:24 (imx8mp_a53_sels: 4 = sys_pll1_800m),
 * PRE/POST dividers 0 */
#define ROOT0_SYS_PLL1_800 (BIT(28) | (4u << 24))

static int result = 1; /* 1: not run */

int board_imx8mp_a53_clock_result(void)
{
	return result;
}

static uint32_t fdiv(uint32_t m, uint32_t p, uint32_t s)
{
	return (m << 12) | (p << 4) | s;
}

static int a53_clock_init(void)
{
	const uint32_t mhz = CONFIG_YAKOGL_BOARD_IMX8MP_A53_MHZ;
	mm_reg_t an, ccm;
	uint32_t want, gen;

	if (mhz == 1600u) {
		want = fdiv(200u, 3u, 0u);
	} else if (mhz == 1200u) {
		want = fdiv(300u, 3u, 1u);
	} else {
		result = -EINVAL;
		return 0;
	}
	device_map(&an, CCM_ANALOG, 0x1000u, K_MEM_CACHE_NONE);
	device_map(&ccm, CCM, 0x10000u, K_MEM_CACHE_NONE);
	if ((sys_read32(an + ARM_PLL_FDIV0) & 0x3FF3F7u) == want &&
	    (sys_read32(an + ARM_PLL_GEN) & GEN_LOCK) != 0u &&
	    (sys_read32(ccm + CCM_CORE_SEL) & CORE_SEL_PLL) != 0u) {
		result = 0; /* already there (a restart) */
		return 0;
	}

	/* the core onto ARM_A53_CLK_ROOT, fed from sys_pll1_800m */
	sys_write32(ROOT0_SYS_PLL1_800, ccm + CCM_ROOT0);
	k_busy_wait(1);
	sys_write32(sys_read32(ccm + CCM_CORE_SEL) & ~CORE_SEL_PLL, ccm + CCM_CORE_SEL);
	k_busy_wait(1);

	/* clk_pll1416x_set_rate */
	gen = sys_read32(an + ARM_PLL_GEN) | GEN_LOCK_SEL;
	sys_write32(gen, an + ARM_PLL_GEN);
	gen &= ~GEN_RST;
	sys_write32(gen, an + ARM_PLL_GEN);
	gen |= GEN_BYPASS;
	sys_write32(gen, an + ARM_PLL_GEN);
	sys_write32((sys_read32(an + ARM_PLL_FDIV0) & ~0x3FF3F7u) | want, an + ARM_PLL_FDIV0);
	k_busy_wait(3);
	gen |= GEN_RST;
	sys_write32(gen, an + ARM_PLL_GEN);
	result = -ETIMEDOUT;
	for (int i = 0; i < 10000; i++) { /* LOCK_TIMEOUT_US, clk-pll14xx.c */
		if ((sys_read32(an + ARM_PLL_GEN) & GEN_LOCK) != 0u) {
			result = 0;
			break;
		}
		k_busy_wait(1);
	}
	if (result != 0) {
		return 0; /* stays on the 800 MHz root */
	}
	gen &= ~GEN_BYPASS;
	sys_write32(gen, an + ARM_PLL_GEN);

	/* and back onto the PLL */
	sys_write32(sys_read32(ccm + CCM_CORE_SEL) | CORE_SEL_PLL, ccm + CCM_CORE_SEL);
	k_busy_wait(1);
	return 0;
}
SYS_INIT(a53_clock_init, PRE_KERNEL_2, 0);
