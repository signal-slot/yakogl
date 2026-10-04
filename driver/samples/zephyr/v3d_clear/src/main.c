/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_clear: milestones M1..M4 of docs/rpi4-v3d-backend-plan.md in one image
 * for the Raspberry Pi 4. The only output is HDMI, so the firmware framebuffer
 * is both the console (top half) and the render target (bottom half):
 *
 *   M1  power the V3D behind its bridge and read its identity
 *   M2  a page table over the buffers below and the framebuffer; MMU on
 *   M3  one binning + render job, polled to FLDONE and FRDONE
 *   M4  the render is a TLB clear of the bottom half to green; every pixel of
 *       it is read back, and a guard row above it must be untouched
 *
 * Every step prints its raw result; the verdict line names what failed.
 * Nothing else runs afterwards: the image idles until the PM watchdog resets
 * the SoC (rpi_wdog.c), which after a tryboot brings the board back into Linux.
 */
#include <zephyr/kernel.h>
#include <zephyr/cache.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <string.h>

#include "v3d.h"
#include "v3d_regs.h"
#include "v3d_frame.h"
#include "rpi_mbox.h"
#include "fbcon.h"
#include "rpi_wdog.h"

#ifndef V3D_BUILD
#define V3D_BUILD "unknown"
#endif

#define FB_W 1280
#define FB_H 720

#define CLEAR_RGBA8 0xff00ff00u /* R 0, G 255, B 0, A 255 (R in the low byte) */
#define SENTINEL    0xffff00ffu /* magenta, written by the CPU beforehand */
#define GUARD       0xff0000ffu /* red, the row above the target */

/* GPU memory: the page table covers 8 MiB of GPU virtual address space. */
#define PT_ENTRIES 2048
static uint32_t pt[PT_ENTRIES] __aligned(4096);
static uint8_t scratch[4096] __aligned(4096);
static uint8_t cls[3][4096] __aligned(4096); /* BCL, RCL, per-tile list */
static uint8_t tile_alloc[64 * 1024] __aligned(4096);
static uint8_t tile_state[64 * 1024] __aligned(4096);

static uint32_t va_next = 0x00100000; /* VA 0 is never mapped */

/* Give `bytes` at physical address `pa` a GPU virtual address. */
static uint32_t gpu_map(uint64_t pa, uint32_t bytes, bool *ok)
{
	uint32_t va = va_next;
	uint32_t len = ROUND_UP(bytes, 4096);
	if (v3d_pte_map(pt, PT_ENTRIES, va, pa, len, true) != 0) {
		printk("gpu_map: refused pa 0x%llx len 0x%x at va 0x%x\n",
		       (unsigned long long)pa, len, va);
		*ok = false;
		return 0;
	}
	va_next += len;
	return va;
}

/*
 * The clock toggle of the power-on sequence. Linux toggles the CPRMAN V3D
 * clock the PM node names (bcm2711.dtsi:117, bcm2835-power.c:297-313); the
 * firmware clock 5 the V3D node names has no enable operation in Linux
 * (clk-raspberrypi.c:262-266), so whether SET_CLOCK_STATE on it does anything
 * is a board question. Its answer is printed and does not stop the run: the
 * identity read that follows is the test of whether the V3D is clocked.
 */
static int clk_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	int ret = rpi_clock_state(RPI_FW_CLK_V3D, on);
	printk("  fw clock 5 %s: %d%s\n", on ? "on" : "off", ret, ret ? " (not fatal)" : "");
	return 0;
}

static void verdict(bool pass, const char *what)
{
	printk("v3d_clear: %s (%s)\n", pass ? "PASS" : "FAIL", what);
}

static void print_stall(const char *what, int ret, const struct v3d_stall *s)
{
	printk("%s: %d core_sts 0x%x hub_sts 0x%x ct0 ca 0x%x ra 0x%x ct1 ca 0x%x ra 0x%x\n",
	       what, ret, s->core_int_sts, s->hub_int_sts, s->ct0ca, s->ct0ra, s->ct1ca, s->ct1ra);
	printk("  err_stat 0x%x mmu_ctl 0x%x vio_addr 0x%x\n", s->err_stat, s->mmu_ctl,
	       s->mmu_vio_addr);
}

static void run(void)
{
	struct rpi_fb fb;
	int ret = rpi_mbox_init();
	if (ret == 0)
		ret = rpi_fb_alloc(FB_W, FB_H, &fb);
	if (ret) {
		/* no screen: nothing can be reported */
		return;
	}
	uint32_t half = fb.height / 2;
	/* scale 1: 22 text rows, so the first lines (M1, the clock replies) are
	 * not overwritten before the verdict */
	fbcon_attach(fb.cpu, fb.width, fb.pitch, half, 1);
	printk("v3d_clear: build %s\n", V3D_BUILD);
	rpi_wdog_announce();
	printk("fb %ux%u pitch %u phys 0x%08x size 0x%x\n", fb.width, fb.height, fb.pitch,
	       fb.phys, fb.size);

	uint32_t hz = 0, max_hz = 0;
	int r1 = rpi_clock_rate(RPI_FW_CLK_V3D, &hz);
	int r2 = rpi_clock_max_rate(RPI_FW_CLK_V3D, &max_hz);
	printk("V3D clock: %u Hz (%d), max %u Hz (%d)\n", hz, r1, max_hz, r2);

	struct v3d_regs regs;
	device_map(&regs.hub, BCM2711_V3D_HUB_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&regs.core, BCM2711_V3D_CORE0_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&regs.pm, BCM2711_PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	device_map(&regs.asb, BCM2711_V3D_ASB_BASE, 0x1000, K_MEM_CACHE_NONE);
	printk("PM_GRAFX 0x%08x ASB M 0x%08x S 0x%08x ID 0x%08x\n",
	       sys_read32(regs.pm + BCM2711_PM_GRAPHICS_CTL), sys_read32(regs.asb + BCM2711_ASB_V3D_MASTER_CTL),
	       sys_read32(regs.asb + BCM2711_ASB_V3D_SLAVE_CTL), sys_read32(regs.asb + BCM2711_ASB_ID));

	/* M1 */
	struct v3d_clock_ops clk = { .set = clk_set };
	ret = v3d_power_on(&regs, &clk);
	printk("M1 power_on: %d, PM_GRAFX 0x%08x\n", ret, sys_read32(regs.pm + BCM2711_PM_GRAPHICS_CTL));
	if (ret) {
		verdict(false, "M1 power-on");
		return;
	}
	if (max_hz && hz != max_hz) {
		ret = rpi_clock_set_rate(RPI_FW_CLK_V3D, max_hz);
		r1 = rpi_clock_rate(RPI_FW_CLK_V3D, &hz);
		printk("V3D clock set to max: %d, now %u Hz (%d)\n", ret, hz, r1);
	}
	struct v3d_identity id;
	ret = v3d_identify(&regs, &id);
	if (ret) {
		verdict(false, "M1 identity");
		return;
	}

	/* M2: map every buffer, then the framebuffer */
	bool ok = true;
	memset(pt, 0, sizeof(pt));
	uint32_t va_cls[3];
	for (int i = 0; i < 3; i++)
		va_cls[i] = gpu_map(k_mem_phys_addr(cls[i]), sizeof(cls[i]), &ok);
	uint32_t va_ta = gpu_map(k_mem_phys_addr(tile_alloc), sizeof(tile_alloc), &ok);
	uint32_t va_ts = gpu_map(k_mem_phys_addr(tile_state), sizeof(tile_state), &ok);
	uint32_t va_fb = gpu_map(fb.phys, fb.size, &ok);
	if (!ok) {
		verdict(false, "M2 page table");
		return;
	}
	sys_cache_data_flush_range(pt, sizeof(pt));
	v3d_irq_quiesce(&regs);
	v3d_caches_init(&regs);
	ret = v3d_mmu_init(&regs, k_mem_phys_addr(pt), k_mem_phys_addr(scratch));
	printk("M2 mmu_init: %d (pt 0x%lx, VA up to 0x%x)\n", ret,
	       (unsigned long)k_mem_phys_addr(pt), va_next);
	if (ret) {
		verdict(false, "M2 MMU");
		return;
	}

	/* M4 frame: clear the bottom half of the framebuffer */
	struct v3d_clear_frame f = {
		.width = fb.width,
		.height = fb.height - half,
		.clear_rgba8 = CLEAR_RGBA8,
		.target = va_fb + half * fb.pitch,
		.stride = fb.pitch,
		.tile_alloc = va_ta,
	};
	struct v3d_tiling t;
	uint32_t ta_size, ts_size;
	v3d_frame_tiling(f.width, f.height, &t);
	v3d_frame_alloc_sizes(&t, 0, 4096, &ta_size, &ts_size);
	if (ta_size > sizeof(tile_alloc) || ts_size > sizeof(tile_state)) {
		printk("tile alloc %u / state %u do not fit\n", ta_size, ts_size);
		verdict(false, "frame sizes");
		return;
	}
	struct v3d_cl bcl, rcl, list;
	v3d_cl_init(&bcl, cls[0], va_cls[0], sizeof(cls[0]));
	v3d_cl_init(&rcl, cls[1], va_cls[1], sizeof(cls[1]));
	v3d_cl_init(&list, cls[2], va_cls[2], sizeof(cls[2]));
	if (!v3d_frame_build_clear(&f, &bcl, &rcl, &list)) {
		verdict(false, "frame build");
		return;
	}
	printk("frame %ux%u: %ux%u tiles, BCL %u RCL %u list %u bytes, TA %u TS %u\n", f.width,
	       f.height, t.tiles_x, t.tiles_y, bcl.used, rcl.used, list.used, ta_size, ts_size);

	/* the target starts magenta and the row above it red */
	for (uint32_t y = half; y < fb.height; y++) {
		uint32_t *p = (uint32_t *)(fb.cpu + (size_t)y * fb.pitch);
		for (uint32_t x = 0; x < fb.width; x++)
			p[x] = SENTINEL;
	}
	uint32_t *guard = (uint32_t *)(fb.cpu + (size_t)(half - 1) * fb.pitch);
	for (uint32_t x = 0; x < fb.width; x++)
		guard[x] = GUARD;
	memset(tile_state, 0, sizeof(tile_state));
	sys_cache_data_flush_range(cls, sizeof(cls));
	sys_cache_data_flush_range(tile_state, sizeof(tile_state));

	/* M3 */
	struct v3d_stall st;
	v3d_submit_bin(&regs, bcl.gpu, bcl.gpu + bcl.used, va_ta, ta_size, va_ts);
	ret = v3d_wait(&regs, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000000, &st);
	if (ret) {
		print_stall("M3 bin", ret, &st);
		verdict(false, ret == -ENOMEM ? "M3 bin: out of memory" :
			       ret == -EFAULT ? "M3 bin: MMU fault" : "M3 bin: timeout");
		return;
	}
	printk("M3 bin: done\n");
	v3d_submit_render(&regs, rcl.gpu, rcl.gpu + rcl.used);
	ret = v3d_wait(&regs, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 1000000, &st);
	if (ret) {
		print_stall("M3 render", ret, &st);
		verdict(false, ret == -EFAULT ? "M3 render: MMU fault" : "M3 render: timeout");
		return;
	}
	ret = v3d_caches_clean(&regs);
	printk("M3 render: done, clean %d\n", ret);

	/* M4: every pixel of the target, and the guard row */
	uint32_t bad = 0, first_x = 0, first_y = 0, first_v = 0;
	for (uint32_t y = half; y < fb.height; y++) {
		const uint32_t *p = (const uint32_t *)(fb.cpu + (size_t)y * fb.pitch);
		for (uint32_t x = 0; x < fb.width; x++) {
			if (p[x] != CLEAR_RGBA8 && bad++ == 0) {
				first_x = x;
				first_y = y;
				first_v = p[x];
			}
		}
	}
	uint32_t guard_bad = 0;
	for (uint32_t x = 0; x < fb.width; x++)
		guard_bad += guard[x] != GUARD;
	uint32_t total = fb.width * (fb.height - half);
	printk("M4: %u of %u target pixels wrong", bad, total);
	if (bad)
		printk(" (first at %u,%u: 0x%08x)", first_x, first_y, first_v);
	printk(", guard row %u of %u changed\n", guard_bad, fb.width);
	verdict(bad == 0 && guard_bad == 0,
		bad ? "M4 pixels" : guard_bad ? "M4 guard row" : "M1-M4, every pixel read back");
}

int main(void)
{
	run();
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
