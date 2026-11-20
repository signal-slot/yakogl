/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_tfu: does the Raspberry Pi 5's V3D 7.1.10 run a TFU job, though its
 * HUB_IDENT1 bit 17 ("TFU: no" in Linux's debugfs v3d_ident,
 * tools/v3d/pi5/v3d_ident.txt) is clear? Roadmap P10, one job:
 *
 *   M1  as rpi5_clear: the V3D clock through the mailbox, the power-on, the
 *       identity, with the raw HUB_IDENT1 and HUB_IDENT3 words and bit 17;
 *       the 7.1 TFU block's CS and SU (0x700, 0x704) read before anything
 *       is written there;
 *   M2  as rpi5_clear: one page table over the probe's static buffers, the
 *       MMU on;
 *   T1  a 64x64 RGBA8 raster source with a known pattern, and the texture's
 *       level 0 tiled by the CPU into the layout Mesa gives a sampled 64x64
 *       RGBA8 texture with 7 levels (v3d_tex_layout: Mesa's v3d_setup_slices,
 *       level 0 UIF), every other byte of the image a sentinel;
 *   T2  one TFU job packed by the driver's 7.1 fill (v3d_tfu71_pack, Mesa's
 *       v3dX(tfu) for 7.1): raster source in, the texture's level 0 as the
 *       destination, NUMMM 6 (levels 1 to 6, down to 1x1) and DIMTW, as
 *       tools/v3d/pi5/gl_mipgen.stdout's job (IOC 0x00086061 there); written
 *       in the order of driver/docs/rpi5-v3d71-facts.md section 7 (IIA, IIS,
 *       ICA, IUA, IOA, IOC, IOS, COEF0..3, ICFG last with its IOC bit: the
 *       ICFG write starts the job); completion polled on the hub's TFUC
 *       status and on TFU_CS (the conversion counter moved, not busy), 1 s at
 *       most;
 *   T3  every level read back and untiled with the same layout: level 0
 *       equal to the pattern (DIMTW: the TFU must not write it, or must
 *       write the same texels), levels 1 to 6 within 2 per channel of a 2x2
 *       box filter of the level above as the CPU computes it ((a+b+c+d+2)/4,
 *       each level from the CPU's previous level: 2 covers the TFU rounding
 *       otherwise at each level), and the bytes of the buffer outside the
 *       levels still the sentinel.
 *
 * One line per step with its raw values, then exactly one verdict line,
 * which tools/board.sh pi5 ends the run on:
 *
 *   v3d_tfu: PASS (levels 0-6 checked, 0 px wrong: ...)
 *   v3d_tfu: FAIL (<reason>)
 *
 * with the reasons, each its own words: the M1/M2 steps; "TFU registers
 * read all-ones" (CS before the job) or "TFU registers read 0" (the job's
 * registers read back after the writes, the job then not done);
 * "MMU fault"; "timeout ... CS ... SU ..."; "level 0 wrong"; "mips wrong";
 * "writes outside the levels". Nothing runs afterwards: the board layer's
 * PM watchdog (armed in the first PRE_KERNEL_1 hook, before main) resets the
 * SoC and tryboot brings Linux back.
 *
 * The caches as rpi5_clear: every buffer the GPU reads or writes is a
 * static cleaned and invalidated before the job, the texture invalidated
 * again before the CPU reads it back.
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
#include "v3d_tex.h"
#include "v3d_tfu71.h"
#include "rpi_mbox.h"

#define PAGE 4096u

/* The GPU's view, as rpi5_clear: one page table from VA 0, VA 0 unmapped. */
#define PT_ENTRIES 4096u
#define VA_FIRST   0x00100000u

/* The texture: 64x64 RGBA8, 7 levels (64 down to 1). */
#define TEX_W      64u
#define TEX_H      64u
#define TEX_CPP    4u
#define TEX_LEVELS 7u
#define SRC_STRIDE (TEX_W * TEX_CPP)
/* Mesa's layout of it is about 22 KiB (level 0 16 KiB at the top); the
 * buffer has room past it for the guard bytes. */
#define IMG_BYTES  (32u * 1024u)

#define SENTINEL_BYTE 0xa5u
#define SENTINEL      0xa5a5a5a5u
#define MIP_TOL       2u
#define WAIT_US       1000000u
#define POLL_US       10u

/* Mesa's own job for the same texture (gl_mipgen.stdout, in place from a
 * UIF level 0): only for the comparison line. */
#define MESA_IOC 0x00086061u

static uint32_t pt[PT_ENTRIES] __aligned(PAGE);
static uint8_t scratch[PAGE] __aligned(PAGE);
static uint32_t src[TEX_W * TEX_H] __aligned(PAGE);
static uint8_t img[IMG_BYTES] __aligned(PAGE);
/* CPU only: one level read back, and the expected level and the one above */
static uint32_t got[TEX_W * TEX_H];
static uint32_t want[TEX_W * TEX_H];
static uint32_t prev[TEX_W * TEX_H];

static uint32_t va_next = VA_FIRST;

#define CACHE_OPT                                                                                  \
	"CONFIG_CACHE_MANAGEMENT=" STRINGIFY(IS_ENABLED(CONFIG_CACHE_MANAGEMENT)) " CONFIG_DCACHE="  \
	STRINGIFY(IS_ENABLED(CONFIG_DCACHE)) " sys_cache_data_flush_and_invd_range"

/* The one end line. Every path of main() ends here once. */
static bool ended;

static void verdict(bool pass, const char *what)
{
	if (ended)
		return;
	ended = true;
	printk("v3d_tfu: %s (%s)\n", pass ? "PASS" : "FAIL", what);
}

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

static uint32_t hub_rd(const struct v3d_regs *r, uint32_t off)
{
	return sys_read32(r->hub + off);
}

static void hub_wr(const struct v3d_regs *r, uint32_t off, uint32_t v)
{
	sys_write32(v, r->hub + off);
}

/* The source pattern: every channel moves with x or y, so a transposed,
 * shifted or channel-swapped level shows. R in the low byte (RGBA8). */
static uint32_t pattern(uint32_t x, uint32_t y)
{
	uint32_t rr = x * 4u + 2u, gg = y * 4u + 1u, bb = ((x ^ y) * 4u) & 0xffu;
	uint32_t aa = 0xffu - (x + y) * 2u;

	return rr | gg << 8 | bb << 16 | aa << 24;
}

static uint32_t chan(uint32_t v, unsigned int c)
{
	return (v >> (8u * c)) & 0xffu;
}

/* The 2x2 box filter of the CPU's previous level (pw x ph) at (x, y) of the
 * next one, rounded to nearest; a side of 1 repeats its texel. */
static uint32_t box(const uint32_t *p, uint32_t pw, uint32_t ph, uint32_t x, uint32_t y)
{
	uint32_t x0 = pw > 1u ? 2u * x : 0u, x1 = pw > 1u ? 2u * x + 1u : 0u;
	uint32_t y0 = ph > 1u ? 2u * y : 0u, y1 = ph > 1u ? 2u * y + 1u : 0u;
	uint32_t out = 0;

	for (unsigned int c = 0; c < 4u; c++) {
		uint32_t s = chan(p[y0 * pw + x0], c) + chan(p[y0 * pw + x1], c) +
			     chan(p[y1 * pw + x0], c) + chan(p[y1 * pw + x1], c);
		out |= ((s + 2u) / 4u) << (8u * c);
	}
	return out;
}

static uint32_t max_chan_diff(uint32_t a, uint32_t b)
{
	uint32_t m = 0;

	for (unsigned int c = 0; c < 4u; c++) {
		uint32_t ca = chan(a, c), cb = chan(b, c);
		uint32_t d = ca > cb ? ca - cb : cb - ca;
		m = d > m ? d : m;
	}
	return m;
}

static const char *tiling_name(enum v3d_tex_tiling t)
{
	switch (t) {
	case V3D_TEX_TILING_LINEARTILE:
		return "LT";
	case V3D_TEX_TILING_UBLINEAR_1_COLUMN:
		return "UBLINEAR_1";
	case V3D_TEX_TILING_UBLINEAR_2_COLUMN:
		return "UBLINEAR_2";
	case V3D_TEX_TILING_UIF_NO_XOR:
		return "UIF_NO_XOR";
	case V3D_TEX_TILING_UIF_XOR:
		return "UIF_XOR";
	}
	return "?";
}

/* v3d_tex_tiling and v3d_tfu71_tiling are both Mesa's enum v3d_tiling_mode. */
static enum v3d_tfu71_tiling tfu_tiling(enum v3d_tex_tiling t)
{
	switch (t) {
	case V3D_TEX_TILING_LINEARTILE:
		return V3D_TFU71_TILING_LINEARTILE;
	case V3D_TEX_TILING_UBLINEAR_1_COLUMN:
		return V3D_TFU71_TILING_UBLINEAR_1_COLUMN;
	case V3D_TEX_TILING_UBLINEAR_2_COLUMN:
		return V3D_TFU71_TILING_UBLINEAR_2_COLUMN;
	case V3D_TEX_TILING_UIF_NO_XOR:
		return V3D_TFU71_TILING_UIF_NO_XOR;
	case V3D_TEX_TILING_UIF_XOR:
		return V3D_TFU71_TILING_UIF_XOR;
	}
	return V3D_TFU71_TILING_RASTER; /* not a texture layout: the pack refuses it */
}

/* The verdict line's text. */
static char why[320];

int main(void)
{
	struct v3d_regs r = { .soc = V3D_SOC_BCM2712 };
	struct v3d_clock_ops clk = { .set = clock_set, .ctx = NULL };
	struct v3d_identity id;
	struct v3d_tex_layout l;
	uint32_t hz = 0;
	int rc;

	printk("v3d_tfu: build %s\n", RPI5_TFU_BUILD);
	printk("v3d_tfu: cache: %s, data line %u bytes\n", CACHE_OPT,
	       (unsigned int)sys_cache_data_line_size_get());
	printk("v3d_tfu: %s, V3D 7.1, one TFU job: %ux%u RGBA8 raster to Mesa's tiled layout, %u levels, "
	       "watchdog armed at boot\n",
	       CONFIG_BOARD, TEX_W, TEX_H, TEX_LEVELS);

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

	rc = v3d_power_on(&r, &clk);
	printk("M1 power_on: %d, PM_GRAFX 0x%08x\n", rc, sys_read32(r.pm + BCM2712_PM_GRAFX));
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
	const uint32_t ident1 = id.hub_ident[1];
	const unsigned int tfu_bit = (ident1 & V3D_HUB_CTL_IDENT1_WITH_TFU_SET) ? 1u : 0u;
	printk("M1 identity: HUB_IDENT1 0x%08x (bit 17, WITH_TFU: %u) HUB_IDENT3 0x%08x; V3D %u.%u: %d\n",
	       ident1, tfu_bit, id.hub_ident[3], id.ver / 10, id.ver % 10, rc);
	if (rc) {
		verdict(false, "M1 identity");
		goto out;
	}
	const uint32_t cs0 = hub_rd(&r, V3D71_TFU_CS), su0 = hub_rd(&r, V3D71_TFU_SU);
	printk("M1 TFU 7.1 block before the job: CS 0x%08x (CVTCT %u, NFREE %u, BUSY %u) SU 0x%08x\n", cs0,
	       V3D_TFU_CS_CVTCT_GET(cs0), V3D_TFU_CS_NFREE_GET(cs0), cs0 & V3D_TFU_CS_BUSY_SET, su0);
	if (cs0 == 0xffffffffu) {
		snprintk(why, sizeof(why),
			 "TFU registers read all-ones: CS 0x%08x SU 0x%08x before the job; HUB_IDENT1 0x%08x",
			 cs0, su0, ident1);
		verdict(false, why);
		goto out;
	}
	if (cs0 & V3D_TFU_CS_BUSY_SET) {
		snprintk(why, sizeof(why), "TFU busy before the job: CS 0x%08x SU 0x%08x; HUB_IDENT1 0x%08x",
			 cs0, su0, ident1);
		verdict(false, why);
		goto out;
	}

	/* M2 */
	memset(pt, 0, sizeof(pt));
	memset(scratch, 0, sizeof(scratch));
	const uint32_t va_src = gpu_map(src, sizeof(src));
	const uint32_t va_img = gpu_map(img, sizeof(img));
	if (!va_src || !va_img) {
		verdict(false, "M2 page table");
		goto out;
	}
	uint64_t pt_pa = k_mem_phys_addr(pt), scratch_pa = k_mem_phys_addr(scratch);
	printk("M2 page table: PA 0x%llx, source VA 0x%08x, texture VA 0x%08x\n", (unsigned long long)pt_pa,
	       va_src, va_img);
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

	/* T1 */
	if (!v3d_tex_layout(TEX_W, TEX_H, TEX_LEVELS, TEX_CPP, &l) || l.size > IMG_BYTES) {
		printk("T1 layout: %ux%u, %u levels: size %u of %u bytes\n", TEX_W, TEX_H, TEX_LEVELS, l.size,
		       IMG_BYTES);
		verdict(false, "T1 layout");
		goto out;
	}
	uint32_t lo = l.slices[0].offset;
	for (uint32_t i = 0; i < l.levels; i++) {
		const struct v3d_tex_slice *s = &l.slices[i];
		printk("T1 level %u: %ux%u, offset 0x%05x, %s, stride %u, padded height %u, size %u\n", i,
		       MAX(TEX_W >> i, 1u), MAX(TEX_H >> i, 1u), s->offset, tiling_name(s->tiling), s->stride,
		       s->padded_height, s->size);
		lo = MIN(lo, s->offset);
	}
	const uint32_t hi = l.slices[0].offset + l.slices[0].size; /* level 0 is the top */
	for (uint32_t y = 0; y < TEX_H; y++)
		for (uint32_t x = 0; x < TEX_W; x++)
			src[y * TEX_W + x] = pattern(x, y);
	memset(img, SENTINEL_BYTE, sizeof(img));
	v3d_tex_store_level(img, &l, 0, (const uint8_t *)src, SRC_STRIDE);
	if (clean(src, sizeof(src), "source") || clean(img, sizeof(img), "texture")) {
		verdict(false, "T1 cache clean");
		goto out;
	}

	/* T2 */
	const struct v3d_tfu71_args a = {
		.ttype = V3D_TFU71_TTYPE_RGBA8,
		.cpp = TEX_CPP,
		.width = TEX_W,
		.height = TEX_H,
		.num_mm = TEX_LEVELS - 1u,
		.src = { .addr = va_src, .tiling = V3D_TFU71_TILING_RASTER,
			 .stride_or_padded_height = SRC_STRIDE },
		.dst = { .addr = va_img + l.slices[0].offset, .tiling = tfu_tiling(l.slices[0].tiling),
			 .stride_or_padded_height = l.slices[0].padded_height },
	};
	struct v3d_tfu71_job j;
	if (!v3d_tfu71_pack(&a, &j)) {
		verdict(false, "T2 v3d_tfu71_pack refused the job");
		goto out;
	}
	printk("T2 job: ICFG 0x%08x IIA 0x%08x IIS 0x%08x ICA 0x%08x IUA 0x%08x IOC 0x%08x IOA 0x%08x "
	       "IOS 0x%08x COEF 0x%08x 0x%08x 0x%08x 0x%08x\n",
	       j.icfg, j.iia, j.iis, j.ica, j.iua, j.ioc, j.ioa, j.ios, j.coef[0], j.coef[1], j.coef[2],
	       j.coef[3]);
	printk("T2 job: IOC %s Mesa's gl_mipgen 0x%08x (its source is UIF in place, ours raster: ICFG and "
	       "IIS differ)\n",
	       j.ioc == MESA_IOC ? "equals" : "differs from", MESA_IOC);

	const uint32_t hub_mmu = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET |
				 V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;
	/* a stale completion must not end this wait */
	hub_wr(&r, V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET);
	/* facts file section 7 (Linux's v3d_tfu_job_run): ICFG last, its
	 * write starts the job */
	hub_wr(&r, V3D71_TFU_IIA, j.iia);
	hub_wr(&r, V3D71_TFU_IIS, j.iis);
	hub_wr(&r, V3D71_TFU_ICA, j.ica);
	hub_wr(&r, V3D71_TFU_IUA, j.iua);
	hub_wr(&r, V3D71_TFU_IOA, j.ioa);
	hub_wr(&r, V3D71_TFU_IOC, j.ioc);
	hub_wr(&r, V3D71_TFU_IOS, j.ios);
	hub_wr(&r, V3D71_TFU_COEF0, j.coef[0]);
	hub_wr(&r, V3D71_TFU_COEF1, j.coef[1]);
	hub_wr(&r, V3D71_TFU_COEF2, j.coef[2]);
	hub_wr(&r, V3D71_TFU_COEF3, j.coef[3]);
	/* what the registers hold before the start (whether they read back
	 * is itself the finding: a missing block reads 0 or all-ones) */
	const uint32_t rb[5] = { hub_rd(&r, V3D71_TFU_IIA), hub_rd(&r, V3D71_TFU_IIS), hub_rd(&r, V3D71_TFU_IOA),
				 hub_rd(&r, V3D71_TFU_IOC), hub_rd(&r, V3D71_TFU_IOS) };
	const uint32_t wr[5] = { j.iia, j.iis, j.ioa, j.ioc, j.ios };
	unsigned int rb_same = 0, rb_zero = 0, rb_ones = 0;
	for (unsigned int i = 0; i < 5u; i++) {
		rb_same += rb[i] == wr[i];
		rb_zero += rb[i] == 0u;
		rb_ones += rb[i] == 0xffffffffu;
	}
	printk("T2 read back before ICFG: IIA 0x%08x IIS 0x%08x IOA 0x%08x IOC 0x%08x IOS 0x%08x "
	       "(%u of 5 as written, %u zero, %u all-ones)\n",
	       rb[0], rb[1], rb[2], rb[3], rb[4], rb_same, rb_zero, rb_ones);
	hub_wr(&r, V3D71_TFU_ICFG, j.icfg | V3D_TFU_ICFG_IOC_SET);

	uint32_t sts = 0, cs = cs0, t = 0;
	const char *by = NULL;
	bool fault = false;
	for (;; t += POLL_US) {
		sts = hub_rd(&r, V3D_HUB_CTL_INT_STS);
		cs = hub_rd(&r, V3D71_TFU_CS);
		if (sts & hub_mmu) {
			by = "a hub MMU fault";
			fault = true;
			break;
		}
		if (sts & V3D_HUB_CTL_INT_STS_INT_TFUC_SET) {
			by = "TFUC";
			break;
		}
		if (V3D_TFU_CS_CVTCT_GET(cs) != V3D_TFU_CS_CVTCT_GET(cs0) && !(cs & V3D_TFU_CS_BUSY_SET)) {
			by = "CS (CVTCT moved, not busy; no TFUC)";
			break;
		}
		if (t >= WAIT_US)
			break;
		k_busy_wait(POLL_US);
	}
	const uint32_t su = hub_rd(&r, V3D71_TFU_SU);
	printk("T2 wait: %s after %u us: hub INT_STS 0x%08x CS 0x%08x (CVTCT %u -> %u, BUSY %u) SU 0x%08x\n",
	       by ? by : "nothing", t, sts, cs, V3D_TFU_CS_CVTCT_GET(cs0), V3D_TFU_CS_CVTCT_GET(cs),
	       cs & V3D_TFU_CS_BUSY_SET, su);
	if (fault) {
		const uint32_t vio_addr = hub_rd(&r, V3D_MMU0_VIO_ADDR), vio_id = hub_rd(&r, V3D_MMU0_VIO_ID);
		snprintk(why, sizeof(why),
			 "MMU fault: hub INT_STS 0x%08x VIO_ADDR 0x%08x VIO_ID 0x%08x CS 0x%08x SU 0x%08x; "
			 "HUB_IDENT1 0x%08x",
			 sts, vio_addr, vio_id, cs, su, ident1);
		verdict(false, why);
		goto out;
	}
	if (!by) {
		if (rb_zero == 5u || rb_ones == 5u)
			snprintk(why, sizeof(why),
				 "TFU registers read %s: the job's 5 read back so, not done in %u us, CS 0x%08x "
				 "SU 0x%08x; HUB_IDENT1 0x%08x",
				 rb_zero == 5u ? "0" : "all-ones", t, cs, su, ident1);
		else
			snprintk(why, sizeof(why),
				 "timeout: no TFUC and no CS count in %u us, CS 0x%08x SU 0x%08x hub INT_STS "
				 "0x%08x; HUB_IDENT1 0x%08x",
				 t, cs, su, sts, ident1);
		verdict(false, why);
		goto out;
	}
	hub_wr(&r, V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET);
	rc = v3d_caches_clean(&r);
	printk("T2 caches_clean: %d\n", rc);

	/* T3: what the TFU wrote, not a line the CPU still holds */
	rc = sys_cache_data_invd_range(img, sizeof(img));
	if (rc) {
		printk("cache invalidate of the texture: %d\n", rc);
		verdict(false, "T3 cache invalidate");
		goto out;
	}
	uint32_t l0_bad = 0, l0_first = 0;
	if (!v3d_tex_load_box(img, &l, 0, 0, 0, TEX_W, TEX_H, (uint8_t *)got, SRC_STRIDE)) {
		verdict(false, "T3 level 0 untile refused");
		goto out;
	}
	for (uint32_t i = 0; i < TEX_W * TEX_H; i++) {
		if (got[i] != src[i] && l0_bad++ == 0)
			l0_first = i;
	}
	printk("T3 level 0: %u of %u px differ%s", l0_bad, TEX_W * TEX_H, l0_bad ? "" : "\n");
	if (l0_bad)
		printk(" (first at %u,%u: 0x%08x, want 0x%08x)\n", l0_first % TEX_W, l0_first / TEX_W,
		       got[l0_first], src[l0_first]);

	uint32_t mip_bad = 0, mip_px = 0, mip_untouched = 0, mip_max = 0, bad_level = 0;
	memcpy(prev, src, sizeof(src));
	for (uint32_t lv = 1; lv < TEX_LEVELS; lv++) {
		const uint32_t pw = MAX(TEX_W >> (lv - 1u), 1u), ph = MAX(TEX_H >> (lv - 1u), 1u);
		const uint32_t w = MAX(TEX_W >> lv, 1u), h = MAX(TEX_H >> lv, 1u);
		uint32_t bad = 0, untouched = 0, maxd = 0, first = 0;

		if (!v3d_tex_load_box(img, &l, lv, 0, 0, w, h, (uint8_t *)got, w * TEX_CPP)) {
			snprintk(why, sizeof(why), "T3 level %u untile refused", lv);
			verdict(false, why);
			goto out;
		}
		for (uint32_t y = 0; y < h; y++) {
			for (uint32_t x = 0; x < w; x++) {
				const uint32_t e = box(prev, pw, ph, x, y), g = got[y * w + x];
				const uint32_t d = max_chan_diff(e, g);

				want[y * w + x] = e;
				untouched += g == SENTINEL;
				maxd = MAX(maxd, d);
				if (d > MIP_TOL && bad++ == 0)
					first = y * w + x;
			}
		}
		printk("T3 level %u: %ux%u, %u px off by more than %u (max %u), %u still the sentinel%s", lv, w, h,
		       bad, MIP_TOL, maxd, untouched, bad ? "" : "\n");
		if (bad)
			printk(" (first at %u,%u: 0x%08x, want 0x%08x)\n", first % w, first / w, got[first],
			       want[first]);
		if (bad && !mip_bad)
			bad_level = lv;
		mip_bad += bad;
		mip_px += w * h;
		mip_untouched += untouched;
		mip_max = MAX(mip_max, maxd);
		memcpy(prev, want, w * h * sizeof(uint32_t));
	}

	uint32_t guard_bad = 0;
	for (uint32_t i = 0; i < IMG_BYTES; i++) {
		if ((i < lo || i >= hi) && img[i] != SENTINEL_BYTE)
			guard_bad++;
	}
	printk("T3 outside the levels (bytes 0x0..0x%05x and 0x%05x..0x%05x): %u of %u changed\n", lo, hi,
	       IMG_BYTES, guard_bad, lo + (IMG_BYTES - hi));

	if (l0_bad)
		snprintk(why, sizeof(why),
			 "level 0 wrong: %u of %u px differ; mips %u of %u px off by more than %u; done by %s "
			 "in %u us; HUB_IDENT1 0x%08x",
			 l0_bad, TEX_W * TEX_H, mip_bad, mip_px, MIP_TOL, by, t, ident1);
	else if (mip_bad)
		snprintk(why, sizeof(why),
			 "mips wrong: %u of %u px of levels 1-%u off by more than %u (first level %u, max %u, "
			 "%u still the sentinel); level 0 exact; done by %s in %u us; HUB_IDENT1 0x%08x",
			 mip_bad, mip_px, TEX_LEVELS - 1u, MIP_TOL, bad_level, mip_max, mip_untouched, by, t,
			 ident1);
	else if (guard_bad)
		snprintk(why, sizeof(why),
			 "writes outside the levels: %u bytes changed; levels 0-%u right; done by %s in %u us; "
			 "HUB_IDENT1 0x%08x",
			 guard_bad, TEX_LEVELS - 1u, by, t, ident1);
	else
		snprintk(why, sizeof(why),
			 "levels 0-%u checked, 0 px wrong: level 0 exact, levels 1-%u (%u px) within %u of the "
			 "2x2 box (max %u); done by %s in %u us; IOC 0x%08x; HUB_IDENT1 0x%08x bit 17 %u",
			 TEX_LEVELS - 1u, TEX_LEVELS - 1u, mip_px, MIP_TOL, mip_max, by, t, j.ioc, ident1,
			 tfu_bit);
	verdict(!l0_bad && !mip_bad && !guard_bad, why);

out:
	/* every path above has printed its verdict; one more is a probe bug */
	if (!ended)
		verdict(false, "probe: no verdict");
	/* nothing more: the watchdog resets the SoC, tryboot brings Linux back */
	for (;;)
		k_sleep(K_SECONDS(10));
	return 0;
}
