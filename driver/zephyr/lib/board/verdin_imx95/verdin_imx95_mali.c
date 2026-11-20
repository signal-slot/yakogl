/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Mali OSAL's board half on the Verdin iMX95 (src/osal/zephyr_mali_board.h;
 * a CONFIG_YAKOGL_GPU_MALI image): two STUBS, each refusing with a line that
 * names what is missing, so that a GL image builds and links for the board
 * and stops at eglInitialize instead of touching a GPU that nothing has
 * powered.
 *
 * - osal_zephyr_mali_default_platform: no CSF firmware is embedded. The
 *   NanoPC-T6's image embeds the Mali-G610's (driver/src/fw/
 *   mali_csffw_blob.h); the Mali-G310's (the linux-firmware file panthor
 *   loads by the GPU_ID's architecture, arm/mali/arch<major>.<minor>/
 *   mali_csffw.bin) is not fetched or pinned (tools/cts/deps.toml) yet.
 * - osal_zephyr_mali_board_up: the GPU is not powered on and the group not
 *   brought up. Missing: the System Manager requests for the GPU's power
 *   domain and clocks (SCMI power domain protocol 0x11 POWER_STATE_SET, clock
 *   protocol 0x14 CLOCK_CONFIG_SET / CLOCK_RATE_SET, all three named by the
 *   gpu node of the board's devicetree), and a bring-up entry in
 *   yakogl-drivers that does not take the RK3588's blocks (yd_t6_osal_up
 *   powers the G610 through the RK3588's CRU, PMU and PMIC; board::
 *   nanopc_t6_osal is the only Mali OSAL kernel side there is).
 *
 * The register block comes from the devicetree (gpu@4d900000), not from a
 * constant here.
 */
#include <zephyr/devicetree.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>

#include "osal_internal.h" /* struct osal_zephyr_mali_init */
#include "zephyr_mali_board.h"
#include "yakogl_board.h"

#define GPU_NODE DT_NODELABEL(gpu)

BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(GPU_NODE, arm_mali_valhall_csf, okay),
	     "the Verdin iMX95's devicetree has the Mali node (gpu@4d900000)");

const struct osal_zephyr_mali_init *osal_zephyr_mali_default_platform(void)
{
	printk("verdin_imx95: STUB: no Mali-G310 CSF firmware is embedded in this image\n");
	return NULL;
}

int osal_zephyr_mali_board_up(const struct osal_zephyr_mali_init *fw,
			      const struct yd_t6_osal_mem *mem,
			      int (*clean)(uintptr_t addr, size_t len),
			      void (*delay_us)(uint32_t us),
			      void (*on_handover)(const struct yd_t6_gpu_up *h),
			      struct yd_t6_gpu_up *z1, struct yd_t6_submit_up *o)
{
	ARG_UNUSED(fw);
	ARG_UNUSED(mem);
	ARG_UNUSED(clean);
	ARG_UNUSED(delay_us);
	ARG_UNUSED(on_handover);
	ARG_UNUSED(z1);
	ARG_UNUSED(o);
	printk("verdin_imx95: STUB: the Mali at 0x%lx (%lu bytes) is not brought up: no "
	       "SCMI power-on of domain %u and clocks %u, %u, and no i.MX 95 bring-up in "
	       "yakogl-drivers\n",
	       (unsigned long)DT_REG_ADDR(GPU_NODE), (unsigned long)DT_REG_SIZE(GPU_NODE),
	       (unsigned int)DT_PHA_BY_IDX(GPU_NODE, power_domains, 0, name),
	       (unsigned int)DT_CLOCKS_CELL_BY_NAME(GPU_NODE, core, name),
	       (unsigned int)DT_CLOCKS_CELL_BY_NAME(GPU_NODE, coregroup, name));
	return -ENOTSUP;
}

void osal_zephyr_mali_board_feed(void)
{
	/* no watchdog (verdin_imx95_wdog_stub); the board layer's alive mark
	 * when there is one */
#ifdef CONFIG_YAKOGL_BOARD
	board_watchdog_feed();
#endif
}
