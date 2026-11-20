/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Mali-G610's CSF firmware for a NanoPC-T6 GL image
 * (CONFIG_YAKOGL_GPU_MALI): what every such image embeds, so each sample
 * (gl_test, deqp) does not carry its own copy.
 *
 * - The firmware is driver/src/fw/mali_csffw_blob.h (made by
 *   driver/tools/mkmalifw.sh from the board's mali_csffw.bin; gitignored,
 *   never committed). The build stops at the include without it.
 * - It is the Mali OSAL's default platform (src/osal/zephyr_mali.c, weak
 *   there): the GL core's init(NULL), from eglInitialize or gl_test.c's Mali
 *   arm, takes it, as the Pi 4's board layer gives the V3D OSAL its default.
 * - Its size goes to the console before main(), with the SHA-256 that
 *   mkmalifw.sh took of the file it made the header from. That digest is a
 *   constant of the header, not computed here over the embedded bytes, and
 *   the line says so: a record names the file the image was built with.
 * - The OSAL's board half (src/osal/zephyr_mali_board.h): the RK3588's
 *   blocks mapped and handed to yd_t6_osal_up, the bring-up of
 *   board::nanopc_t6_osal (Z1 to Z3, then the group), and WDT0's feed.
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>

#include "osal_internal.h" /* struct osal_zephyr_mali_init */
#include "zephyr_mali_board.h"
#include "fw/mali_csffw_blob.h"
#include <nanopc_t6.h>
#include <yakogl_drivers.h>

/* The RK3588's blocks Z1 to Z3 touch (soc::rk3588, board::nanopc_t6_gpu:
 * the CRU, the PMU, SPI2 to the RK806 PMIC, the GPU's QoS, the GPU), and
 * the GPU mapping's size: the doorbells are at 0x80000 and up. */
#define T6_CRU_PHYS  0xfd7c0000UL
#define T6_PMU_PHYS  0xfd8d8000UL
#define T6_SPI2_PHYS 0xfeb20000UL
#define T6_QOS_PHYS  0xfdf35000UL
#define T6_GPU_PHYS  0xfb000000UL
#define T6_GPU_MAP   0x280000
#define T6_TIMER_HZ  24000000 /* CNTFRQ_EL0 */

int osal_zephyr_mali_board_up(const struct osal_zephyr_mali_init *fw,
			      const struct yd_t6_osal_mem *mem,
			      int (*clean)(uintptr_t addr, size_t len),
			      void (*delay_us)(uint32_t us),
			      void (*on_handover)(const struct yd_t6_gpu_up *h),
			      struct yd_t6_gpu_up *z1, struct yd_t6_submit_up *o)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;

	device_map(&cru, T6_CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, T6_PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, T6_SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, T6_QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&gpu, T6_GPU_PHYS, T6_GPU_MAP, K_MEM_CACHE_NONE);
	t6_watchdog_feed();
	return yd_t6_osal_up(cru, pmu, spi2, qos, gpu, T6_GPU_MAP, fw->fw, fw->fw_len, T6_TIMER_HZ,
			     mem, clean, delay_us, on_handover, z1, o);
}

void osal_zephyr_mali_board_feed(void)
{
	t6_watchdog_feed();
}

static const struct osal_zephyr_mali_init t6_mali_fw = {
	.fw = mali_csffw_blob,
	.fw_len = MALI_CSFFW_SIZE,
};

const struct osal_zephyr_mali_init *osal_zephyr_mali_default_platform(void)
{
	return &t6_mali_fw;
}

static int t6_mali_fw_report(void)
{
	printk("t6: CSF firmware %u bytes embedded, built from a file with sha256 %s "
	       "(mkmalifw.sh's digest, not computed at run time)\n",
	       (unsigned int)MALI_CSFFW_SIZE, MALI_CSFFW_SHA256);
	return 0;
}
SYS_INIT(t6_mali_fw_report, APPLICATION, 0);
