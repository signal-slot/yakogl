/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_display: U3 on the Raspberry Pi 5 (driver/docs/rpi5-display.md).
 *
 *   D1  the property mailbox's framebuffer GET tags, before anything is
 *       asked: does the firmware answer them, and with a mode (route 2's
 *       first question; a mode with a pitch and an order is also a hint that
 *       the firmware's scanout is running, route 1);
 *   D2  display registers read raw, before the allocation: the HVS (first
 *       words of its block), pixel valve 0 and 1. Each block is announced
 *       before it is read, so a hang names the block (the PM watchdog then
 *       resets the board);
 *   D3  rpi_fb_alloc(): the address, size, pitch and pixel order the firmware
 *       granted;
 *   D4  the same registers again: how many words moved by the allocation
 *       (the scanout picked it up, or nothing happened);
 *   D5  the pattern written and read back through the CPU mapping (the bytes
 *       in memory, not yet the scanout), then the split screen of the Pi 4:
 *       the console on top, the pattern below.
 *
 * One line per fact, then `rpi5_display: PASS (...)` or `rpi5_display: FAIL
 * (reason)`. PASS means D1 to D5's memory check held; whether the panel shows
 * it is the HDMI capture's or the register readback's question, not this
 * line's. The PM watchdog (board layer, armed before main) resets the SoC
 * and tryboot returns to Linux.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/printk-hooks.h>
#include <zephyr/sys/sys_io.h>

#include <errno.h>
#include <stdbool.h>
#include <string.h>

#include "rpi5_screen.h"
#include "fbcon.h"

#define FB_W 1280u
#define FB_H 720u
#define TEXT_H 256u /* rows of the console, on top */

/* The display blocks (driver/docs/rpi5-v3d71-facts.md, section 10): bases
 * from bcm2712.dtsi under soc@107c000000. */
static const struct {
	const char *name;
	unsigned long long base;
	uint32_t words;
} blocks[] = {
	{ "hvs", 0x107c580000ull, 32 },
	{ "pixelvalve0", 0x107c410000ull, 16 },
	{ "pixelvalve1", 0x107c411000ull, 16 },
};
#define NBLOCKS ARRAY_SIZE(blocks)
#define MAXW    32u

static uint32_t before[NBLOCKS][MAXW], after[NBLOCKS][MAXW];

static printk_hook_fn_t uart_out, fb_out;

/* printk to the UART and the framebuffer both (fbcon_attach() replaces the
 * hook; keep it and put ours back, as the Pi 4's board layer does). */
static int console_out(int c)
{
	if (uart_out) {
		uart_out(c);
	}
	return fb_out ? fb_out(c) : c;
}

static void dump(const char *when, uint32_t (*v)[MAXW])
{
	for (size_t b = 0; b < NBLOCKS; b++) {
		mm_reg_t va;

		printk("rpi5_display: D2 %s reading %s\n", when, blocks[b].name);
		device_map(&va, blocks[b].base, 0x1000, K_MEM_CACHE_NONE);
		if (!va) {
			printk("rpi5_display: D2 %s %s: not mapped\n", when, blocks[b].name);
			continue;
		}
		for (uint32_t i = 0; i < blocks[b].words; i++) {
			v[b][i] = sys_read32(va + 4u * i);
		}
		for (uint32_t i = 0; i < blocks[b].words; i += 4) {
			printk("rpi5_display: D2 %s %s[0x%03x]: %08x %08x %08x %08x\n", when, blocks[b].name,
			       4u * i, v[b][i], v[b][i + 1], v[b][i + 2], v[b][i + 3]);
		}
	}
}

int main(void)
{
	struct rpi5_fw_fb q = { 0 };
	struct rpi_fb fb = { 0 };
	int rc;
	/* the `u3` lines tools/probes/rpi5_display/verdicts.sh reads */
	const char *mb_fb = "FAIL (not reached)", *mb_rb = "FAIL (not reached)";
	const char *split = "not drawn";
	uint32_t moved = 0;
	bool regs_read = false;
	char verdict[160] = "FAIL (not reached)";

	printk("rpi5_display: build %s\n", RPI5_DISPLAY_BUILD);
	rc = rpi_mbox_init();
	printk("rpi5_display: mailbox init %d\n", rc);
	if (rc != 0) {
		snprintk(verdict, sizeof(verdict), "FAIL (mailbox init)");
		goto hold;
	}

	/* D1 */
	int answered = rpi5_screen_query(&q);
	static const char *const names[RPI5_GET_COUNT] = { "physical", "virtual", "depth",
							   "order",    "pitch",   "offset" };
	for (int i = 0; i < RPI5_GET_COUNT; i++) {
		printk("rpi5_display: D1 get %s: rc %d code %08x values %u %u\n", names[i], q.rc[i], q.code[i],
		       q.val[i][0], q.val[i][1]);
	}
	printk("rpi5_display: D1 %d of %d tags answered, framebuffer reported: %s\n", answered, RPI5_GET_COUNT,
	       rpi5_screen_query_has_fb(&q) ? "yes" : "no");

	/* D2 */
	dump("before", before);
	regs_read = true;

	/* D3 */
	rc = rpi_fb_alloc(FB_W, FB_H, &fb);
	printk("rpi5_display: D3 alloc %ux%u: rc %d\n", FB_W, FB_H, rc);
	if (rc != 0) {
		snprintk(verdict, sizeof(verdict), "FAIL (D3 alloc %d)", rc);
		goto hold;
	}
	printk("rpi5_display: D3 granted %ux%u pitch %u size %u phys 0x%08x order code %08x value %u\n",
	       fb.width, fb.height, fb.pitch, fb.size, fb.phys, fb.order_code, fb.order);

	mb_fb = "PASS";
	/* D4 */
	dump("after", after);
	for (size_t b = 0; b < NBLOCKS; b++) {
		for (uint32_t i = 0; i < blocks[b].words; i++) {
			moved += before[b][i] != after[b][i];
		}
	}
	printk("rpi5_display: D4 %u register words changed by the allocation\n", moved);

	/* D5 */
	if (fb.order > 1u || fb.height < TEXT_H + 64u || fb.pitch < fb.width * 4u) {
		snprintk(verdict, sizeof(verdict), "FAIL (D5 unusable grant: order %u height %u pitch %u)", fb.order,
		       fb.height, fb.pitch);
		goto hold;
	}
	rpi5_pattern_fill(fb.cpu, fb.pitch, fb.order, fb.width, 0, fb.height);
	uint32_t first = 0, bad = rpi5_pattern_verify(fb.cpu, fb.pitch, fb.order, fb.width, 0, fb.height, &first);

	printk("rpi5_display: D5 pattern read back: %u wrong pixels of %u (first %u)\n", bad, fb.width * fb.height,
	       first);
	mb_rb = bad ? "FAIL (whole-frame pattern)" : "PASS";

	uart_out = __printk_get_hook();
	fbcon_attach(fb.cpu, fb.width, fb.pitch, TEXT_H, 2);
	fb_out = __printk_get_hook();
	__printk_hook_install(console_out);
	rpi5_pattern_fill(fb.cpu, fb.pitch, fb.order, fb.width, TEXT_H, fb.height);
	printk("rpi5_display: split screen: console rows 0..%u, pattern rows %u..%u\n", TEXT_H - 1u, TEXT_H,
	       fb.height - 1u);
	bad = rpi5_pattern_verify(fb.cpu, fb.pitch, fb.order, fb.width, TEXT_H, fb.height, &first);
	printk("rpi5_display: D5 lower pattern read back: %u wrong pixels (first %u)\n", bad, first);
	split = "drawn";
	if (bad != 0u) {
		snprintk(verdict, sizeof(verdict), "FAIL (D5 readback %u wrong)", bad);
	} else {
		snprintk(verdict, sizeof(verdict), "PASS (D1 %d tags, D3 phys 0x%08x pitch %u order %u, D4 %u words moved, "
		       "bytes read back)",
		       answered, fb.phys, fb.pitch, fb.order, moved);
	}

hold:
	/* Route A is not measured here: no display-list decoder in this image
	 * (hvs6_dlist.h is the host-side reader), so the plane is the mailbox's
	 * answer and the adopt read-back is reported as not attempted. */
	printk("u3 scanout: %s (pixelvalve0 CONTROL %08x)\n",
	       regs_read && (before[1][0] & 1u) ? "running" : "stopped", regs_read ? before[1][0] : 0u);
	printk("u3 plane: addr=0x%08x fmt=order%u pitch=%u w=%u h=%u\n", fb.phys, fb.order, fb.pitch,
	       fb.width, fb.height);
	printk("u3 adopt-readback: FAIL (not attempted: no display-list decoder in this image)\n");
	printk("u3 mailbox-fb: %s (%ux%u pitch %u order%u)\n", mb_fb, fb.width, fb.height, fb.pitch, fb.order);
	printk("u3 mailbox-readback: %s\n", mb_rb);
	printk("u3 scanout-follows-mailbox: %s (%u register words moved)\n", moved ? "yes" : "no", moved);
	printk("u3 split-screen: %s\n", split);
	printk("rpi5_display: %s\n", verdict);
	/* nothing more to do: the watchdog resets the SoC within its period */
	for (;;) {
		k_sleep(K_FOREVER);
	}
	return 0;
}
