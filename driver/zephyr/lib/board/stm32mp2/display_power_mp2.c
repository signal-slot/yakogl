/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The STM32MP257F-DK's display path powered for its drivers (LTDC -> DSI ->
 * ADV7535 on I2C2, driver/drivers/display/display_yakogl_stm32_ltdc.c the
 * chain owner), before they initialise (POST_KERNEL,
 * CONFIG_YAKOGL_STM32MP2_DISPLAY_INIT_PRIORITY, ahead of the I2C controller's
 * 50). Plan #353, issues #7 and #1281.
 *
 * Through the crate's soc::stm32mp2::display (stm32mp2_display.h), in its
 * order: yd_mp2_display_rif_check (RIFSC entries of the LTDC, DSI, I2C2, read
 * only), yd_mp2_display_power_on (OP-TEE's SCMI: the HDMI 5 V on LDO5, the
 * GPIO bank clocks, FLEXGEN_12/28, FLEXGEN_27 at
 * CONFIG_YAKOGL_STM32MP2_DISPLAY_PIXEL_HZ; the RCC gates and resets;
 * DISPLAYCLKCR), yd_mp2_display_pins (I2C2 on PF2/PF0, PB4 input, PB6 high),
 * yd_mp2_display_adv_reset. Every SMC, OP-TEE frame and SCMI message is on
 * the console ("display_power: ...") before it is sent, as gc_power_mp2.c
 * does for the GPU.
 *
 * A refusal is printed and kept: stm32_ltdc_board_ready() then answers it and
 * the chain owner touches nothing of the path, so the probe prints its FAIL
 * line and the board layer's lifetime ends the run. Unlike the GPU's, the
 * path's state after a refused request is not one another master relies on.
 *
 * The board hooks of yakogl_drv/stm32_ltdc.h are here: board_ready, the
 * frame buffer's RISAF check, and the pixel clock (a mode whose clock is not
 * FLEXGEN_27's rate repeats the power-on with that rate: SCMI never
 * disables, so that is the only way to change it).
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include "stm32mp2_display.h"

#include <yakogl_drv/stm32_ltdc.h>

/* soc::stm32mp2::display_regs; the sizes are the probes' (mp2_display_state). */
#define MP2_RCC_PHYS    0x44200000UL /* RCC_I2C2CFGR 0x7a8, _LTDCCFGR 0x840, _DSICFGR 0x844 */
#define MP2_RCC_SIZE    0x1000UL
#define MP2_SYSCFG_PHYS 0x44230000UL /* DISPLAYCLKCR at +0x5000 */
#define MP2_SYSCFG_SIZE 0x6000UL
#define MP2_RIFSC_PHYS  0x42080000UL
#define MP2_RIFSC_SIZE  0x1000UL
#define MP2_GPIOB_PHYS  0x44250000UL
#define MP2_GPIOF_PHYS  0x44290000UL
#define MP2_GPIO_SIZE   0x1000UL

BUILD_ASSERT(IS_ENABLED(CONFIG_YAKOGL_STM32MP2_WDOG),
	     "an image that talks to OP-TEE's SCMI runs under OP-TEE's watchdog");
BUILD_ASSERT(CONFIG_YAKOGL_STM32MP2_DISPLAY_INIT_PRIORITY < CONFIG_YAKOGL_DRV_I2C_STM32MP2_INIT_PRIORITY,
	     "the path is powered before the I2C controller initialises");

static uint8_t shm[YD_MP2_DISPLAY_SHM_SIZE] __aligned(YD_MP2_DISPLAY_SHM_ALIGN);

static struct yd_mp2_display disp;
static struct yd_mp2_display_power power;
static int ready = -EAGAIN; /* 0 once the path is up */
static uint32_t rate_hz;    /* FLEXGEN_27 as read back */

static void log_line(void *ctx, const char *line, size_t len)
{
	ARG_UNUSED(ctx);
	printk("display_power: %.*s\n", (int)len, line);
}

static void delay_us(void *ctx, uint32_t us)
{
	ARG_UNUSED(ctx);
	k_busy_wait(us);
}

static void refused(const char *what, int32_t rc, const struct yd_mp2_display_err *e)
{
	printk("display_power: REFUSED %s rc=%d kind=%u step=%u id=%u a=0x%x b=0x%x\n", what,
	       (int)rc, (unsigned int)e->kind, (unsigned int)e->step, (unsigned int)e->id,
	       (unsigned int)e->a, (unsigned int)e->b);
}

static int power_on(uint32_t hz)
{
	int32_t rc;

	disp.ltdc_rate_hz = hz;
	rc = yd_mp2_display_power_on(&disp, &power);
	if (rc != 0) {
		refused("power-on", rc, &power.err);
		return rc;
	}
	rate_hz = power.rate_hz[4];
	printk("display_power: powered: hdmi 5v %u -> %u, supplies 0x%x, rates gpiob %u gpiof %u "
	       "i2c2 %u dsiphy %u ltdc %u\n",
	       (unsigned int)power.hdmi_before, (unsigned int)power.hdmi,
	       (unsigned int)power.supplies_on, (unsigned int)power.rate_hz[0],
	       (unsigned int)power.rate_hz[1], (unsigned int)power.rate_hz[2],
	       (unsigned int)power.rate_hz[3], (unsigned int)power.rate_hz[4]);
	printk("display_power: rcc i2c2 0x%08x -> 0x%08x, dsi 0x%08x -> 0x%08x, ltdc 0x%08x -> "
	       "0x%08x, displayclkcr 0x%x\n",
	       (unsigned int)power.rcc_before[0], (unsigned int)power.rcc[0],
	       (unsigned int)power.rcc_before[1], (unsigned int)power.rcc[1],
	       (unsigned int)power.rcc_before[2], (unsigned int)power.rcc[2],
	       (unsigned int)power.displayclkcr);
	return 0;
}

static void print_rif(const struct yd_mp2_display_rif *r)
{
	for (size_t i = 0; i < ARRAY_SIZE(r->entries); i++) {
		const struct yd_mp2_display_rif_entry *e = &r->entries[i];

		printk("display_power: rif %u sec %u priv %u cid 0x%08x sem 0x%08x verdict %u\n",
		       (unsigned int)e->id, (unsigned int)e->secure, (unsigned int)e->privileged,
		       (unsigned int)e->cidcfgr, (unsigned int)e->semcr, (unsigned int)e->verdict);
	}
	printk("display_power: rif refused 0x%x, rimc 0x%08x 0x%08x 0x%08x, fb region %u verdict %u\n",
	       (unsigned int)r->refused, (unsigned int)r->rimc[0], (unsigned int)r->rimc[1],
	       (unsigned int)r->rimc[2], (unsigned int)r->fb_region, (unsigned int)r->fb_verdict);
}

static int mp2_display_power_init(void)
{
	struct yd_mp2_display_rif rif;
	struct yd_mp2_display_pins pins;
	mm_reg_t rcc, syscfg, rifsc, gpiob, gpiof;
	int32_t rc;

	device_map(&rcc, MP2_RCC_PHYS, MP2_RCC_SIZE, K_MEM_CACHE_NONE);
	device_map(&syscfg, MP2_SYSCFG_PHYS, MP2_SYSCFG_SIZE, K_MEM_CACHE_NONE);
	device_map(&rifsc, MP2_RIFSC_PHYS, MP2_RIFSC_SIZE, K_MEM_CACHE_NONE);
	device_map(&gpiob, MP2_GPIOB_PHYS, MP2_GPIO_SIZE, K_MEM_CACHE_NONE);
	device_map(&gpiof, MP2_GPIOF_PHYS, MP2_GPIO_SIZE, K_MEM_CACHE_NONE);
	disp = (struct yd_mp2_display){
		.rcc_base = rcc,
		.syscfg_base = syscfg,
		.rifsc_base = rifsc,
		.gpiob_base = gpiob,
		.gpiof_base = gpiof,
		.shm = shm,
		.shm_pa = k_mem_phys_addr(shm),
		.log = log_line,
		.log_ctx = NULL,
		.delay_us = delay_us,
		.delay_ctx = NULL,
		.ltdc_rate_hz = CONFIG_YAKOGL_STM32MP2_DISPLAY_PIXEL_HZ,
		.channel = 0,
		.cid = 0,
	};
	printk("display_power: shm pa 0x%llx, ltdc clock %u Hz\n", (unsigned long long)disp.shm_pa,
	       (unsigned int)CONFIG_YAKOGL_STM32MP2_DISPLAY_PIXEL_HZ);

	rc = yd_mp2_display_rif_check(&disp, 0, 0, &rif);
	print_rif(&rif);
	if (rc != 0) {
		refused("rif", rc, &rif.err);
		ready = rc;
		return 0;
	}
	rc = power_on(CONFIG_YAKOGL_STM32MP2_DISPLAY_PIXEL_HZ);
	if (rc != 0) {
		ready = rc;
		return 0;
	}
	rc = yd_mp2_display_pins(&disp, &pins);
	if (rc != 0) {
		refused("pins", rc, &pins.err);
		ready = rc;
		return 0;
	}
	rc = yd_mp2_display_adv_reset(&disp, &pins);
	if (rc != 0) {
		refused("adv-reset", rc, &pins.err);
		ready = rc;
		return 0;
	}
	printk("display_power: pins gpiob moder 0x%08x gpiof moder 0x%08x afrl 0x%08x, adv int %u\n",
	       (unsigned int)pins.gpiob_moder, (unsigned int)pins.gpiof_moder,
	       (unsigned int)pins.gpiof_afrl, (unsigned int)pins.adv_int);
	printk("display_power: up\n");
	ready = 0;
	return 0;
}

SYS_INIT(mp2_display_power_init, POST_KERNEL, CONFIG_YAKOGL_STM32MP2_DISPLAY_INIT_PRIORITY);

/* ---- yakogl_drv/stm32_ltdc.h board hooks --------------------------------- */

int stm32_ltdc_board_ready(void)
{
	return ready;
}

int stm32_ltdc_board_fb_check(uint64_t phys, uint64_t size)
{
	struct yd_mp2_display_rif rif;
	int32_t rc;

	if (ready != 0) {
		return ready;
	}
	rc = yd_mp2_display_rif_check(&disp, phys, size, &rif);
	printk("display_power: fb 0x%llx + 0x%llx: region %u verdict %u rc %d\n",
	       (unsigned long long)phys, (unsigned long long)size, (unsigned int)rif.fb_region,
	       (unsigned int)rif.fb_verdict, (int)rc);
	if (rc != 0) {
		refused("fb", rc, &rif.err);
	}
	return rc;
}

int stm32_ltdc_board_pixel_clock(uint32_t hz, uint32_t *actual_hz)
{
	int32_t rc;

	if (ready != 0) {
		return ready;
	}
	if (hz == rate_hz) {
		*actual_hz = rate_hz;
		return 0;
	}
	printk("display_power: ltdc clock %u Hz -> %u Hz: powering the path again\n",
	       (unsigned int)rate_hz, (unsigned int)hz);
	rc = power_on(hz);
	if (rc != 0) {
		ready = rc;
		return rc;
	}
	*actual_hz = rate_hz;
	return 1;
}
