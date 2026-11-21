/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Mali OSAL's board half on the Verdin iMX95 (src/osal/zephyr_mali_board.h;
 * a CONFIG_YAKOGL_GPU_MALI image): the Mali-G310's CSF firmware, the GPU's
 * bring-up (yd_imx95_osal_up) over the board's SCMI channel to the System
 * Manager (verdin_imx95_scmi.c).
 *
 * - The firmware is driver/src/fw/mali_csffw_blob.h, as on the NanoPC-T6
 *   (made by driver/tools/mkmalifw.sh from the pinned MALI_CSFFW,
 *   tools/cts/deps.toml; gitignored, never committed). The G310 is arch
 *   10.12, and linux-firmware's arm/mali/arch10.12/mali_csffw.bin is the
 *   same bytes as the arch10.8 file the T6 embeds (docs/notes/
 *   pre-study-imx95.md, section 2). The build stops at the include without
 *   it.
 * - osal_zephyr_mali_board_up maps the GPU's block from the devicetree
 *   (gpu@4d900000, 0x480000) and calls yd_imx95_osal_up
 *   (board::imx95_gpu in yakogl-drivers): GPU power domain 14, performance
 *   domain 9 and clock gate 174 over SCMI (soc::imx95::gpu_power), then
 *   GPU_ID, the soft reset, the L2, the firmware's boot and the group, the
 *   same code as the T6's after its power-on. The arch timer's rate is
 *   CNTFRQ_EL0 as TF-A set it.
 * - The SCMI channel is the board's one (verdin_imx95_scmi.c), which the
 *   display's requests share.
 *
 * The watchdog is the board glue's (WDOG3, verdin_imx95_wdog.c).
 */
#include <zephyr/devicetree.h>
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/arch/arm64/lib_helpers.h> /* read_cntfrq_el0; after kernel.h */
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <errno.h>

#include "osal_internal.h" /* struct osal_zephyr_mali_init */
#include "zephyr_mali_board.h"
#include "yakogl_board.h"
#include "verdin_imx95_scmi.h"
#include "fw/mali_csffw_blob.h"
#include <yakogl_drivers.h>

#define GPU_NODE   DT_NODELABEL(gpu)

BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(GPU_NODE, arm_mali_valhall_csf, okay),
	     "the Verdin iMX95's devicetree has the Mali node (gpu@4d900000)");
BUILD_ASSERT(DT_REG_SIZE(GPU_NODE) >= YD_IMX95_GPU_MAP_MIN,
	     "the GPU node covers the registers and doorbells the OSAL reaches");

int osal_zephyr_mali_board_up(const struct osal_zephyr_mali_init *fw,
			      const struct yd_t6_osal_mem *mem,
			      int (*clean)(uintptr_t addr, size_t len),
			      void (*delay_us)(uint32_t us),
			      void (*on_handover)(const struct yd_t6_gpu_up *h),
			      struct yd_t6_gpu_up *z1, struct yd_t6_submit_up *o)
{
	static struct yd_imx95_gpu_power power;
	uint32_t timer_hz = (uint32_t)read_cntfrq_el0();
	mm_reg_t gpu;
	int rc;

	device_map(&gpu, DT_REG_ADDR(GPU_NODE), DT_REG_SIZE(GPU_NODE), K_MEM_CACHE_NONE);
	osal_zephyr_mali_board_feed();
	rc = yd_imx95_osal_up(verdin_imx95_scmi, 0, gpu, DT_REG_SIZE(GPU_NODE), fw->fw, fw->fw_len,
			      timer_hz, mem, clean, delay_us, on_handover, z1, &power, o);
	if (power.on) {
		printk("verdin_imx95: GPU on over SCMI: level %u (%u kHz), gate %llu Hz; timer %u Hz\n",
		       power.level_index, power.level_khz,
		       (unsigned long long)power.rate_hz[1] << 32 | power.rate_hz[0], timer_hz);
	} else if (z1->stop == YD_IMX95_GPU_STOP_SCMI) {
		printk("verdin_imx95: the SCMI power-on stopped: refusal %u, message 0x%04x, "
		       "status %d, detail 0x%08x%08x\n",
		       power.refusal, power.message, power.status, power.detail[1],
		       power.detail[0]);
	}
	return rc;
}

void osal_zephyr_mali_board_feed(void)
{
	/* the board layer's alive mark, when there is one: WDOG3's feeder
	 * (verdin_imx95_wdog.c) reads it */
#ifdef CONFIG_YAKOGL_BOARD
	board_watchdog_feed();
#endif
}

static const struct osal_zephyr_mali_init vi95_mali_fw = {
	.fw = mali_csffw_blob,
	.fw_len = MALI_CSFFW_SIZE,
};

const struct osal_zephyr_mali_init *osal_zephyr_mali_default_platform(void)
{
	return &vi95_mali_fw;
}

static int vi95_mali_fw_report(void)
{
	printk("verdin_imx95: CSF firmware %u bytes embedded, built from a file with sha256 %s "
	       "(mkmalifw.sh's digest, not computed at run time)\n",
	       (unsigned int)MALI_CSFFW_SIZE, MALI_CSFFW_SHA256);
	return 0;
}
SYS_INIT(vi95_mali_fw_report, APPLICATION, 0);
