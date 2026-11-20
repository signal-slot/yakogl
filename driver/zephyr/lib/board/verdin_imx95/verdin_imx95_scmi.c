/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin iMX95's SCMI channel to the System Manager as raw messages
 * (verdin_imx95_scmi.h), one for every request the Rust side builds itself:
 * the GPU's power-on (soc::imx95::gpu_power) and the display's power and
 * clocks (soc::imx95::display_power, ::dsi_power). Those build each command
 * themselves (the performance protocol 0x13 has no Zephyr driver at the pin,
 * and the drivers turn the SM's status into an errno, losing DENIED's name:
 * docs/notes/imx95-gpu-power.md, section 4). It goes out on the A55's one
 * channel (MU2, the shared memory at 0x445b1000) through the clock
 * protocol's instance, under the channel's lock and polled, as
 * scmi_send_message sends; Zephyr's own SCMI clients (the clock and pin
 * control drivers) take the same lock. The reply's length is read from the
 * shared memory's header first, since Zephyr's read takes only the length it
 * is told (a refusal's reply is shorter than the answer asked for).
 */
#include <zephyr/devicetree.h>
#include <zephyr/drivers/firmware/scmi/protocol.h>
#include <zephyr/drivers/firmware/scmi/shmem.h>
#include <zephyr/drivers/firmware/scmi/transport.h>
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <errno.h>
#include <stddef.h>
#include <string.h>

#include "verdin_imx95_scmi.h"

#define CLK_NODE   DT_NODELABEL(scmi_clk)
#define SHMEM_NODE DT_NODELABEL(scmi_shmem0)

BUILD_ASSERT(DT_NODE_HAS_COMPAT_STATUS(CLK_NODE, arm_scmi_clock, okay),
	     "the SCMI clock protocol's instance carries the raw messages");
BUILD_ASSERT(DT_REG_SIZE(SHMEM_NODE) >= sizeof(struct scmi_shmem_layout) + 8,
	     "the SCMI shared memory holds a header and a status");

/* The SM answers within microseconds; this bounds a channel that never
 * frees (an SM that died), so the caller's line says so instead of hanging. */
#define VI95_SCMI_TIMEOUT_US 100000U

static mm_reg_t vi95_shmem;

int verdin_imx95_scmi(const uint8_t *tx, size_t tx_len, uint8_t *rx, size_t rx_cap)
{
	const struct device *clk = DEVICE_DT_GET(CLK_NODE);
	struct scmi_protocol *proto = clk->data;
	struct scmi_message msg, reply;
	uint32_t hdr, len = 0;
	uint32_t waited = 0;
	int ret;

	if (tx_len < 4 || tx_len % 4 != 0 || rx_cap < 4)
		return -EINVAL;
	if (vi95_shmem == 0 || !device_is_ready(clk) || !proto || !proto->tx ||
	    !proto->tx->ready)
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

/* The shared memory's own mapping, for the reply's length: once, before
 * any caller (the GPU's bring-up runs at eglInitialize, the display's at the
 * first board_show). */
static int vi95_scmi_map(void)
{
	device_map(&vi95_shmem, DT_REG_ADDR(SHMEM_NODE), DT_REG_SIZE(SHMEM_NODE), K_MEM_CACHE_NONE);
	return 0;
}
SYS_INIT(vi95_scmi_map, POST_KERNEL, 0);
