/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_clear: the first clear on the Raspberry Pi 5's V3D 7.1, milestones M1
 * to M4 of docs/notes/pre-study-rpi5.md section 6, the Pi 4's v3d_clear
 * (docs/notes/v3d71-clear-frame.md section 3) carried over:
 *
 *   M1  the V3D clock through the firmware mailbox, the power-on
 *       (v3d_power_on, BCM2712: pm's reset and the SMS), the identity read
 *       and printed raw (hub and core IDENT, MMU_DEBUG_INFO) to compare with
 *       Linux's debugfs v3d_ident;
 *   M2  one page table over the probe's static buffers, the MMU on;
 *   M3  the clear-only frame (frame71.c: binning list, render list, generic
 *       tile list) binned and rendered, FLDONE and FRDONE polled;
 *   M4  the 128x64 raster target read back: every pixel the clear colour,
 *       the guard row above it unchanged.
 *
 * One line per step with its raw result, then a single verdict line,
 * `v3d_clear: PASS (...)` or `v3d_clear: FAIL (reason)`, which
 * tools/board.sh pi5 ends the run on. Nothing runs afterwards: the board
 * layer's PM watchdog (armed in the first PRE_KERNEL_1 hook, before main)
 * resets the SoC and tryboot brings Linux back.
 *
 * Every buffer the GPU reads or writes is a static in the image, written
 * through the CPU's cache: each is cleaned and invalidated before the GPU
 * sees it, and the target invalidated again before the CPU reads it back.
 * A cache operation that fails ends the run (CONFIG_CACHE_MANAGEMENT off
 * makes them all -ENOTSUP, and the GPU would read stale memory).
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
#include "rpi_mbox.h"
#include "frame71.h"

#define PAGE 4096u

/* The GPU's view: one page table of PT_ENTRIES 4 KiB pages from VA 0 (16 MiB);
 * VA 0 is never mapped, so a null GPU address faults. */
#define PT_ENTRIES 4096u
#define VA_FIRST   0x00100000u

/* The target: 128x64 RGBA8888 (two 64x64 tiles side by side), one guard row
 * in the same buffer right above its first row. */
#define TGT_W      128u
#define TGT_H      64u
#define TGT_STRIDE (TGT_W * 4u)
#define TGT_BYTES  ROUND_UP((TGT_H + 1u) * TGT_STRIDE, PAGE)

#define SENTINEL 0xffff00ffu /* magenta: what the CPU writes, the GPU must overwrite */
#define GUARD    0xff0000ffu /* red: the row above, the GPU must not touch */
#define CLEAR    0xff00ff00u /* green: R in the low byte */

#define CL_BYTES         PAGE
#define TILE_ALLOC_BYTES (64u * 1024u)
#define TILE_STATE_BYTES (64u * 1024u)
#define WAIT_US          1000000u

static uint32_t pt[PT_ENTRIES] __aligned(PAGE);
static uint8_t scratch[PAGE] __aligned(PAGE);
static uint8_t cls[3][CL_BYTES] __aligned(PAGE); /* binning, render, tile list */
static uint8_t tile_alloc[TILE_ALLOC_BYTES] __aligned(PAGE);
static uint8_t tile_state[TILE_STATE_BYTES] __aligned(PAGE);
static uint32_t target[TGT_BYTES / 4] __aligned(PAGE);

static uint32_t va_next = VA_FIRST;

/* The cache path the run used, printed at the start and with the verdict:
 * every sys_cache_data_*() below is this one. */
#define CACHE_OPT                                                                                  \
	"CONFIG_CACHE_MANAGEMENT=" STRINGIFY(IS_ENABLED(CONFIG_CACHE_MANAGEMENT)) " CONFIG_DCACHE="  \
	STRINGIFY(IS_ENABLED(CONFIG_DCACHE)) " sys_cache_data_flush_and_invd_range"

/* VA pages mapped so far, with the page table and the scratch page. */
static unsigned int pages_used(void)
{
	return (va_next - VA_FIRST) / PAGE + 2u;
}

static void verdict(bool pass, const char *what)
{
	printk("v3d_clear: %s (%s; %u pages used incl. page table and scratch, cache: %s)\n",
	       pass ? "PASS" : "FAIL", what, pages_used(), CACHE_OPT);
}

static void print_stall(const char *step, const struct v3d_stall *st)
{
	printk("%s: core_sts 0x%08x hub_sts 0x%08x ct0 0x%08x..0x%08x ct1 0x%08x..0x%08x\n", step,
	       st->core_int_sts, st->hub_int_sts, st->ct0ca, st->ct0ra, st->ct1ca, st->ct1ra);
	printk("%s: err_stat 0x%08x mmu_ctl 0x%08x vio_addr 0x%08x vio_id 0x%08x\n", step, st->err_stat,
	       st->mmu_ctl, st->mmu_vio_addr, st->mmu_vio_id);
}

/* Map `bytes` at `cpu` at the next free GPU VA; 0 when refused. */
static uint32_t gpu_map(void *cpu, uint32_t bytes)
{
	uint32_t va = va_next;
	uint64_t pa = k_mem_phys_addr(cpu);
	int rc = v3d_pte_map(pt, PT_ENTRIES, va, pa, bytes, true);

	if (rc) {
		printk("gpu_map: refused %u bytes at PA 0x%llx (VA 0x%08x): %d\n", bytes,
		       (unsigned long long)pa, va, rc);
		return 0;
	}
	va_next = va + ROUND_UP(bytes, PAGE);
	return va;
}

/* The CPU's writes out to memory and the lines dropped, so that the GPU reads
 * what the CPU wrote and no dirty line is later written over the GPU's. */
static int clean(void *p, size_t bytes, const char *name)
{
	int rc = sys_cache_data_flush_and_invd_range(p, bytes);

	if (rc)
		printk("cache clean of %s (%u bytes): %d\n", name, (unsigned int)bytes, rc);
	return rc;
}

static int clock_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	return rpi_clock_state(RPI_FW_CLK_V3D, on);
}

int main(void)
{
	struct v3d_regs r = { .soc = V3D_SOC_BCM2712 };
	struct v3d_clock_ops clk = { .set = clock_set, .ctx = NULL };
	struct v3d_identity id;
	struct v3d_stall st;
	uint32_t hz = 0;
	int rc;

	printk("v3d_clear: build %s\n", RPI5_CLEAR_BUILD);
	printk("v3d_clear: cache: %s, data line %u bytes\n", CACHE_OPT,
	       (unsigned int)sys_cache_data_line_size_get());
	printk("v3d_clear: %s, V3D 7.1, target %ux%u raster RGBA8888, watchdog armed at boot\n", CONFIG_BOARD,
	       TGT_W, TGT_H);

	/* M1 */
	rc = rpi_mbox_init();
	printk("M1 mailbox: %d\n", rc);
	if (rc) {
		verdict(false, "M1 mailbox");
		goto out;
	}
	device_map(&r.hub, BCM2712_V3D_HUB_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&r.core, BCM2712_V3D_CORE0_BASE, 0x6000, K_MEM_CACHE_NONE);
	device_map(&r.pm, BCM2712_PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	device_map(&r.sms, BCM2712_V3D_SMS_BASE, 0x700, K_MEM_CACHE_NONE);
	printk("M1 before power-on: PM_GRAFX 0x%08x\n", sys_read32(r.pm + BCM2712_PM_GRAFX));

	rc = v3d_power_on(&r, &clk);
	printk("M1 power_on: %d, PM_GRAFX 0x%08x, SMS REE_CS 0x%08x TEE_CS 0x%08x\n", rc,
	       sys_read32(r.pm + BCM2712_PM_GRAFX), sys_read32(r.sms + V3D_SMS_REE_CS),
	       sys_read32(r.sms + V3D_SMS_TEE_CS));
	if (rc) {
		verdict(false, "M1 power-on");
		goto out;
	}
	rc = rpi_clock_rate(RPI_FW_CLK_V3D, &hz);
	printk("M1 V3D clock: rate %d (%u Hz)\n", rc, hz);

	rc = v3d_identify(&r, &id);
	printk("M1 identity: HUB_IDENT0..3 0x%08x 0x%08x 0x%08x 0x%08x\n", id.hub_ident[0], id.hub_ident[1],
	       id.hub_ident[2], id.hub_ident[3]);
	printk("M1 identity: CTL_IDENT0..2 0x%08x 0x%08x 0x%08x MMU_DEBUG_INFO 0x%08x\n", id.ctl_ident[0],
	       id.ctl_ident[1], id.ctl_ident[2], id.mmu_debug_info);
	printk("M1 identity: V3D %u.%u, %u core(s), %u QPUs, VPM %u bytes, PA %u / VA %u bits, MMU %s: %d\n",
	       id.ver / 10, id.ver % 10, id.ncores, id.qpu_count, id.vpm_size, id.pa_width, id.va_width,
	       id.has_mmu ? "yes" : "no", rc);
	if (rc) {
		verdict(false, "M1 identity");
		goto out;
	}

	/* M2 */
	memset(pt, 0, sizeof(pt));
	memset(scratch, 0, sizeof(scratch));
	uint32_t va_cl[3];
	for (int i = 0; i < 3; i++)
		va_cl[i] = gpu_map(cls[i], CL_BYTES);
	uint32_t va_ta = gpu_map(tile_alloc, TILE_ALLOC_BYTES);
	uint32_t va_ts = gpu_map(tile_state, TILE_STATE_BYTES);
	uint32_t va_tgt = gpu_map(target, TGT_BYTES);
	if (!va_cl[0] || !va_cl[1] || !va_cl[2] || !va_ta || !va_ts || !va_tgt) {
		verdict(false, "M2 page table");
		goto out;
	}
	uint64_t pt_pa = k_mem_phys_addr(pt), scratch_pa = k_mem_phys_addr(scratch);
	printk("M2 page table: PA 0x%llx, %u entries, scratch PA 0x%llx, VA 0x%08x..0x%08x mapped\n",
	       (unsigned long long)pt_pa, PT_ENTRIES, (unsigned long long)scratch_pa, VA_FIRST, va_next);
	if (clean(pt, sizeof(pt), "page table") || clean(scratch, sizeof(scratch), "scratch page")) {
		verdict(false, "M2 cache clean");
		goto out;
	}
	v3d_irq_quiesce(&r);
	v3d_caches_init(&r);
	rc = v3d_mmu_init(&r, pt_pa, scratch_pa);
	printk("M2 mmu_init: %d\n", rc);
	if (rc) {
		verdict(false, "M2 MMU");
		goto out;
	}

	/* the frame */
	struct frame71 f = {
		.width = TGT_W,
		.height = TGT_H,
		.clear_rgba8 = CLEAR,
		.target = va_tgt + TGT_STRIDE, /* row 0 of the buffer is the guard row */
		.stride = TGT_STRIDE,
		.tile_alloc = va_ta,
	};
	struct v3d_tiling t;
	uint32_t ta_size, ts_size;
	frame71_tiling(&f, &t);
	v3d_frame_alloc_sizes(&t, 0, PAGE, &ta_size, &ts_size);
	if (ta_size > TILE_ALLOC_BYTES || ts_size > TILE_STATE_BYTES) {
		printk("frame: tile allocation %u bytes, tile state %u bytes\n", ta_size, ts_size);
		verdict(false, "frame sizes");
		goto out;
	}
	struct cl71 bcl, rcl, tl;
	cl71_init(&bcl, cls[0], va_cl[0], CL_BYTES);
	cl71_init(&rcl, cls[1], va_cl[1], CL_BYTES);
	cl71_init(&tl, cls[2], va_cl[2], CL_BYTES);
	if (!frame71_build_clear(&f, &bcl, &rcl, &tl)) {
		verdict(false, "frame build");
		goto out;
	}
	printk("frame %ux%u: tile %ux%u, %ux%u tiles, lists %u / %u / %u bytes, tile alloc %u, state %u\n",
	       f.width, f.height, t.tile_w, t.tile_h, t.tiles_x, t.tiles_y, bcl.len, rcl.len, tl.len, ta_size,
	       ts_size);

	/* the target pre-filled: the guard row, then the sentinel the clear
	 * must replace */
	for (uint32_t x = 0; x < TGT_W; x++)
		target[x] = GUARD;
	for (uint32_t i = TGT_W; i < TGT_BYTES / 4u; i++)
		target[i] = SENTINEL;
	memset(tile_state, 0, sizeof(tile_state));
	if (clean(cls, sizeof(cls), "lists") || clean(tile_alloc, sizeof(tile_alloc), "tile allocation") ||
	    clean(tile_state, sizeof(tile_state), "tile state") || clean(target, sizeof(target), "target")) {
		verdict(false, "M3 cache clean");
		goto out;
	}

	/* M3 */
	v3d_submit_bin(&r, va_cl[0], cl71_end(&bcl), va_ta, ta_size, va_ts);
	rc = v3d_wait(&r, V3D_CTL_0_INT_STS_INT_FLDONE_SET, WAIT_US, &st);
	printk("M3 bin: %d\n", rc);
	if (rc) {
		print_stall("M3 bin", &st);
		verdict(false, rc == -ENOMEM ? "M3 bin: out of memory"
			       : rc == -EFAULT ? "M3 bin: MMU fault"
					       : "M3 bin: timeout");
		goto out;
	}
	v3d_submit_render(&r, va_cl[1], cl71_end(&rcl));
	rc = v3d_wait(&r, V3D_CTL_0_INT_STS_INT_FRDONE_SET, WAIT_US, &st);
	printk("M3 render: %d\n", rc);
	if (rc) {
		print_stall("M3 render", &st);
		verdict(false, rc == -EFAULT ? "M3 render: MMU fault" : "M3 render: timeout");
		goto out;
	}
	rc = v3d_caches_clean(&r);
	printk("M3 caches_clean: %d\n", rc);
	if (rc) {
		verdict(false, "M3 GPU cache clean");
		goto out;
	}

	/* M4: what the GPU wrote, not a line the CPU still holds */
	rc = sys_cache_data_invd_range(target, sizeof(target));
	if (rc) {
		printk("cache invalidate of the target: %d\n", rc);
		verdict(false, "M4 cache invalidate");
		goto out;
	}
	uint32_t bad = 0, guard_bad = 0, first_x = 0, first_y = 0, first_v = 0;
	for (uint32_t y = 0; y < TGT_H; y++) {
		for (uint32_t x = 0; x < TGT_W; x++) {
			uint32_t v = target[(y + 1) * TGT_W + x];
			if (v != CLEAR && bad++ == 0) {
				first_x = x;
				first_y = y;
				first_v = v;
			}
		}
	}
	for (uint32_t x = 0; x < TGT_W; x++)
		guard_bad += target[x] != GUARD;
	if (bad)
		printk("M4: %u of %u target pixels wrong (first at %u,%u: 0x%08x), guard row %u of %u changed\n",
		       bad, TGT_W * TGT_H, first_x, first_y, first_v, guard_bad, TGT_W);
	else
		printk("M4: %u of %u target pixels wrong, guard row %u of %u changed\n", bad, TGT_W * TGT_H,
		       guard_bad, TGT_W);
	if (bad)
		verdict(false, "M4 pixels");
	else if (guard_bad)
		verdict(false, "M4 guard row");
	else
		verdict(true, "M1 to M4, 128x64 cleared");

out:
	/* nothing more: the watchdog resets the SoC, tryboot brings Linux back */
	for (;;)
		k_sleep(K_SECONDS(10));
	return 0;
}
