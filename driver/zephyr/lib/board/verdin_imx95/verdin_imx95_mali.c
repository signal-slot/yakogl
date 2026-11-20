/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Mali OSAL's board half on the Verdin iMX95 (src/osal/zephyr_mali_board.h;
 * a CONFIG_YAKOGL_GPU_MALI image): the Mali-G310's CSF firmware, the GPU's
 * bring-up (yd_imx95_osal_up) and the System Manager's SCMI channel it
 * powers the GPU through.
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
 * - The SCMI channel: gpu_power builds each command itself (the
 *   performance protocol 0x13 has no Zephyr driver at the pin, and the
 *   drivers turn the SM's status into an errno, losing DENIED's name:
 *   docs/notes/imx95-gpu-power.md, section 4). It goes out on the A55's
 *   one channel (MU2, the shared memory at 0x445b1000) through the clock
 *   protocol's instance, under the channel's lock and polled, as
 *   scmi_send_message sends; the reply's length is read from the shared
 *   memory's header first, since Zephyr's read takes only the length it is
 *   told (a refusal's reply is shorter than the answer asked for).
 *
 * The watchdog is the board glue's (WDOG3, verdin_imx95_wdog.c).
 */
#include <zephyr/devicetree.h>
#include <zephyr/drivers/firmware/scmi/protocol.h>
#include <zephyr/drivers/firmware/scmi/shmem.h>
#include <zephyr/drivers/firmware/scmi/transport.h>
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/arch/arm64/lib_helpers.h> /* read_cntfrq_el0; after kernel.h */
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <errno.h>
#include <stddef.h>
#include <string.h>

#include "osal_internal.h" /* struct osal_zephyr_mali_init */
#include "zephyr_mali_board.h"
#include "yakogl_board.h"
#include "fw/mali_csffw_blob.h"
#include <yakogl_drivers.h>

#define GPU_NODE   DT_NODELABEL(gpu)
#define CLK_NODE   DT_NODELABEL(scmi_clk)
#define SHMEM_NODE DT_NODELABEL(scmi_shmem0)

BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(GPU_NODE, arm_mali_valhall_csf, okay),
	     "the Verdin iMX95's devicetree has the Mali node (gpu@4d900000)");
BUILD_ASSERT(DT_REG_SIZE(GPU_NODE) >= YD_IMX95_GPU_MAP_MIN,
	     "the GPU node covers the registers and doorbells the OSAL reaches");
BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(CLK_NODE, arm_scmi_clock, okay),
	     "the SCMI clock protocol's instance carries the GPU's messages");
BUILD_ASSERT(DT_REG_SIZE(SHMEM_NODE) >= sizeof(struct scmi_shmem_layout) + 8,
	     "the SCMI shared memory holds a header and a status");

/* The SM answers within microseconds; this bounds a channel that never
 * frees (an SM that died), so the OSAL's line says so instead of hanging. */
#define VI95_SCMI_TIMEOUT_US 100000U

static mm_reg_t vi95_shmem;

/* gpu_power's transport (yd_imx95_osal_up's scmi): one command `tx` (header
 * and payload words as built) out, the whole reply (header, status, return
 * values) into `rx`; its length, or a negative errno. */
static int vi95_scmi(const uint8_t *tx, size_t tx_len, uint8_t *rx, size_t rx_cap)
{
	const struct device *clk = DEVICE_DT_GET(CLK_NODE);
	struct scmi_protocol *proto = clk->data;
	struct scmi_message msg, reply;
	uint32_t hdr, len = 0;
	uint32_t waited = 0;
	int ret;

	if (tx_len < 4 || tx_len % 4 != 0 || rx_cap < 4)
		return -EINVAL;
	if (!device_is_ready(clk) || !proto || !proto->tx || !proto->tx->ready)
		return -ENODEV;
	memcpy(&hdr, tx, sizeof(hdr));
	msg.hdr = hdr;
	msg.len = tx_len - 4;
	msg.content = (void *)(tx + 4);

	ret = k_mutex_lock(&proto->tx->lock, K_USEC(VI95_SCMI_TIMEOUT_US));
	if (ret < 0)
		return ret;
	ret = scmi_transport_send_message(proto->transport, proto->tx, &msg, true);
	while (ret == 0 && !scmi_transport_channel_is_free(proto->transport, proto->tx)) {
		if (waited++ >= VI95_SCMI_TIMEOUT_US) {
			ret = -ETIMEDOUT;
			break;
		}
		k_busy_wait(1);
	}
	if (ret == 0) {
		/* the reply's header and payload, as the SM wrote them */
		len = sys_read32(vi95_shmem + offsetof(struct scmi_shmem_layout, len));
		if (len < 4 || len > rx_cap ||
		    sizeof(struct scmi_shmem_layout) + len - 4 > DT_REG_SIZE(SHMEM_NODE)) {
			ret = -EMSGSIZE;
		} else {
			reply.hdr = hdr;
			reply.len = len - 4;
			reply.content = rx + 4;
			ret = scmi_transport_read_message(proto->transport, proto->tx, &reply);
		}
	}
	k_mutex_unlock(&proto->tx->lock);
	if (ret < 0)
		return ret;
	/* the read checked that the reply's header is the command's */
	memcpy(rx, &hdr, sizeof(hdr));
	return (int)len;
}

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

	device_map(&vi95_shmem, DT_REG_ADDR(SHMEM_NODE), DT_REG_SIZE(SHMEM_NODE), K_MEM_CACHE_NONE);
	device_map(&gpu, DT_REG_ADDR(GPU_NODE), DT_REG_SIZE(GPU_NODE), K_MEM_CACHE_NONE);
	osal_zephyr_mali_board_feed();
	rc = yd_imx95_osal_up(vi95_scmi, 0, gpu, DT_REG_SIZE(GPU_NODE), fw->fw, fw->fw_len, timer_hz,
			      mem, clean, delay_us, on_handover, z1, &power, o);
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
