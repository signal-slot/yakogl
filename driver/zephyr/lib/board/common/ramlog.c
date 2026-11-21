/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * ramlog: an opt-in RAM ring that mirrors every printk character
 * (CONFIG_YAKOGL_RAMLOG, the address CONFIG_YAKOGL_RAMLOG_ADDR, the size
 * CONFIG_YAKOGL_RAMLOG_SIZE). A blind run can read its console text back from
 * DDR after a reset (the region is mapped Normal non-cacheable, so the bytes
 * reach DDR without cache maintenance).
 *
 * Format (little endian). At ADDR a 32-byte header of eight u32:
 *   +0  magic     0x474C4B59 (the bytes 'Y','K','L','G'), written last at init
 *   +4  version   1
 *   +8  size      ring bytes = CONFIG_YAKOGL_RAMLOG_SIZE - 32
 *   +12 head      total bytes ever written, wrapping at 2^32
 *   +16 seq       0 at init, +1 on each line end '\n'
 *   +20 flags     bit0 = wrapped (the ring has been filled once)
 *   +24 reserved0 0
 *   +28 reserved1 0
 * The ring data follows the header: byte i of the stream is data[i % size].
 * Each character stores its data byte, then head (a dmb between). The hook
 * chains to the hook installed before it, so the UART console keeps working.
 */
#include <zephyr/kernel.h>
#include <zephyr/init.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/sys/device_mmio.h>
#include <stdint.h>

#define RAMLOG_MAGIC 0x474C4B59u
#define RAMLOG_HDR   32u
#define RAMLOG_SIZE  ((uint32_t)CONFIG_YAKOGL_RAMLOG_SIZE)
#define RAMLOG_RING  (RAMLOG_SIZE - RAMLOG_HDR)

struct ramlog_hdr {
	uint32_t magic, version, size, head, seq, flags, reserved0, reserved1;
};

BUILD_ASSERT(sizeof(struct ramlog_hdr) == RAMLOG_HDR, "ramlog header is 32 bytes");

static volatile struct ramlog_hdr *s_hdr;
static volatile uint8_t *s_data;
static printk_hook_fn_t s_prev;

static int ramlog_out(int c)
{
	volatile struct ramlog_hdr *h = s_hdr;
	uint32_t head = h->head;

	s_data[head % RAMLOG_RING] = (uint8_t)c;
	if (head % RAMLOG_RING == RAMLOG_RING - 1u) {
		h->flags = h->flags | 1u;
	}
	barrier_dmem_fence_full();
	h->head = head + 1u;
	if (c == '\n') {
		h->seq = h->seq + 1u;
	}
	return s_prev ? s_prev(c) : c;
}

static int ramlog_init(void)
{
	mm_reg_t va;

	device_map(&va, CONFIG_YAKOGL_RAMLOG_ADDR, RAMLOG_SIZE, K_MEM_ARM_NORMAL_NC);
	s_hdr = (volatile struct ramlog_hdr *)va;
	s_data = (volatile uint8_t *)(va + RAMLOG_HDR);

	s_hdr->version = 1;
	s_hdr->size = RAMLOG_RING;
	s_hdr->head = 0;
	s_hdr->seq = 0;
	s_hdr->flags = 0;
	s_hdr->reserved0 = 0;
	s_hdr->reserved1 = 0;
	barrier_dmem_fence_full();
	s_hdr->magic = RAMLOG_MAGIC;
	barrier_dmem_fence_full();

	/* After the UART console's own hook (PRE_KERNEL_1, priority 60): chain. */
	s_prev = __printk_get_hook();
	__printk_hook_install(ramlog_out);
	return 0;
}

SYS_INIT(ramlog_init, PRE_KERNEL_1, 90);
