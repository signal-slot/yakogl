/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_triangle: the first triangle on the Raspberry Pi 5's V3D 7.1,
 * milestone M5 of docs/notes/pre-study-rpi5.md (section 6; P6 of section 8),
 * the Pi 4's v3d_triangle (docs/notes/pi4-v3d-bringup.md) carried over:
 *
 *   M1  the V3D clock through the firmware mailbox, the power-on, the
 *       identity, as rpi5_clear (the firmware hands the V3D over powered, at
 *       500 MHz and out of reset);
 *   M2  one page table over the probe's static buffers, the MMU on;
 *   M5  the clear-then-triangle frame of the driver's frame code
 *       (v3d_frame_build_triangle, v3d_frame.c + v3d_frame71_tri.c at
 *       V3D_CL_VER 71; docs/notes/v3d71-triangle-frame.md), with Mesa's
 *       shaders (rpi5_triangle) or ours (rpi5_triangle_ours, src/shaders.c),
 *       binned with a 256 KiB overflow block in hand and rendered
 *       (docs/notes/v3d71-bin-submit.md);
 *       the 64x64 RGBA8888 raster target read back and every pixel
 *       classified inside / outside / edge to the tolerance of 3
 *       (tools/probes/rpi_5/common/tri_check/tri_check.h), the guard rows
 *       above and below unchanged.
 *
 * One line per step with its raw result, then a single verdict line at
 * column 0, `v3d_triangle: PASS (...)` or `v3d_triangle: FAIL (reason; ...)`,
 * which tools/board.sh pi5 ends the run on. Nothing runs afterwards: the
 * board layer's PM watchdog (armed in the first PRE_KERNEL_1 hook, before
 * main) resets the SoC and tryboot brings Linux back.
 *
 * Every buffer the GPU reads or writes is a static in the image, written
 * through the CPU's cache: each is cleaned and invalidated before the GPU
 * sees it, and the target invalidated again before the CPU reads it back.
 * A cache operation that fails ends the run.
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
#include "tri_check.h"
#include "shaders.h"

#define PAGE 4096u

/* The GPU's view: one page table of PT_ENTRIES 4 KiB pages from VA 0 (16 MiB);
 * VA 0 is never mapped, so a null GPU address faults. */
#define PT_ENTRIES 4096u
#define VA_FIRST   0x00100000u

/* The target: 64x64 RGBA8888 raster (one 64x64 tile, the board's Linux
 * reference size, tools/v3d/pi5/triangle.clif), with one guard row above and
 * one below in the same buffer. */
#define TGT_W      64u
#define TGT_H      64u
#define TGT_STRIDE (TGT_W * 4u)
#define TGT_BYTES  ROUND_UP((TGT_H + 2u) * TGT_STRIDE, PAGE)

#define SENTINEL   0xffff00ffu /* magenta: what the CPU writes, the GPU must overwrite */
#define GUARD_BYTE 0xa5u       /* the guard rows, the GPU must not touch */
/* the board's Linux reference clears to this (triangle.clif's
 * RENDER_TARGET_PART1 "Clear Color low bits" 4294901760): blue, R in the low
 * byte; the frame is that dump's apart from the target's layout */
#define CLEAR      0xffff0000u
#define TOL        3u
#define MARGIN     1.0f

#define CL_BYTES         PAGE
#define CODE_SLOT        0x200u /* FS, VS, CS each at a 512-byte slot of the code page */
#define TILE_ALLOC_BYTES (64u * 1024u)
#define TILE_STATE_BYTES (64u * 1024u)
/* the binner's overflow block: Linux's and Mesa's simulator's size
 * (v3d71-bin-submit.md section 4), mapped before the bin */
#define OVERFLOW_BYTES   (256u * 1024u)
#define WAIT_US          1000000u

/* registers v3d_regs.h has no name for (v3d71-bin-submit.md sections 6, 7;
 * Linux v6.18 v3d_regs.h): core MISCCFG and FDBGS, the 7.1 GMP pair */
#define CORE_MISCCFG       0x00018
#define CORE_FDBGS         0x00f10
#define CORE_GMP_STATUS_71 0x00600
#define CORE_GMP_VIO_71    0x00608

/* gl_ref.py's triangle (tools/v3d/frame_check.c TRI71_VERTICES, the dump's
 * vertex buffer): three vec4 positions, then three vec4 colours */
#define COLOR_OFFSET 0x30u
static const float VERTICES[24] = {
	-0.75f, -0.75f, 0.0f, 1.0f, 0.75f, -0.75f, 0.0f, 1.0f, 0.0f, 0.75f, 0.0f, 1.0f,
	1.0f,   0.0f,   0.0f, 1.0f, 0.0f,  1.0f,   0.0f, 1.0f, 0.0f, 0.0f,  1.0f, 1.0f,
};

static uint32_t pt[PT_ENTRIES] __aligned(PAGE);
static uint8_t scratch[PAGE] __aligned(PAGE);
static uint8_t cls[3][CL_BYTES] __aligned(PAGE); /* binning, render, indirect */
static uint8_t code[PAGE] __aligned(PAGE);
static uint8_t vertex[PAGE] __aligned(PAGE);
static uint8_t tile_alloc[TILE_ALLOC_BYTES] __aligned(PAGE);
static uint8_t tile_state[TILE_STATE_BYTES] __aligned(PAGE);
static uint8_t overflow[OVERFLOW_BYTES] __aligned(PAGE);
static uint32_t target[TGT_BYTES / 4] __aligned(PAGE);

static uint32_t va_next = VA_FIRST;
static struct tri_shaders sh;

#define CACHE_OPT                                                                                  \
	"CONFIG_CACHE_MANAGEMENT=" STRINGIFY(IS_ENABLED(CONFIG_CACHE_MANAGEMENT)) " CONFIG_DCACHE="  \
	STRINGIFY(IS_ENABLED(CONFIG_DCACHE)) " sys_cache_data_flush_and_invd_range"

/* VA pages mapped so far, with the page table and the scratch page. */
static unsigned int pages_used(void)
{
	return (va_next - VA_FIRST) / PAGE + 2u;
}

/* The one verdict line. */
static void verdict(bool pass, const char *what)
{
	printk("v3d_triangle: %s (%s; shaders %s)\n", pass ? "PASS" : "FAIL", what, sh.name);
}

static void print_stall(const char *step, const struct v3d_stall *st, const struct v3d_regs *r,
			uint32_t va_width)
{
	/* VIO_ADDR holds the top 32 bits of a va_width-bit address */
	uint64_t vio = (uint64_t)st->mmu_vio_addr << (va_width > 32 ? va_width - 32 : 0);

	printk("%s: core_sts 0x%08x hub_sts 0x%08x ct0 0x%08x..0x%08x ct1 0x%08x..0x%08x\n", step,
	       st->core_int_sts, st->hub_int_sts, st->ct0ca, st->ct0ra, st->ct1ca, st->ct1ra);
	printk("%s: err_stat 0x%08x mmu_ctl 0x%08x vio_addr 0x%08x (VA 0x%09llx) vio_id 0x%08x (client 0x%02x)\n",
	       step, st->err_stat, st->mmu_ctl, st->mmu_vio_addr, (unsigned long long)vio, st->mmu_vio_id,
	       st->mmu_vio_id & 0x7fu);
	printk("%s: GMP_STATUS 0x%08x GMP_VIO_ADDR 0x%08x (raw)\n", step,
	       sys_read32(r->core + CORE_GMP_STATUS_71), sys_read32(r->core + CORE_GMP_VIO_71));
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

static uint8_t unorm8(float v)
{
	return v <= 0.0f ? 0u : v >= 1.0f ? 255u : (uint8_t)(v * 255.0f + 0.5f);
}

static void print_pixel(const char *name, uint32_t x, uint32_t y, const uint8_t *row0)
{
	const uint8_t *p = row0 + y * TGT_STRIDE + x * 4u;

	printk("M5 pixel %s (%u,%u): %02x %02x %02x %02x\n", name, x, y, p[0], p[1], p[2], p[3]);
}

int main(void)
{
	struct v3d_regs r = { .soc = V3D_SOC_BCM2712 };
	struct v3d_clock_ops clk = { .set = clock_set, .ctx = NULL };
	struct v3d_identity id;
	struct v3d_stall st;
	char why[160];
	uint32_t hz = 0;
	int rc;

	tri_shaders_get(&sh);
	printk("v3d_triangle: build %s, shaders %s (FS %u, VS %u, CS %u instructions)\n", RPI5_TRI_BUILD,
	       sh.name, sh.words[0], sh.words[1], sh.words[2]);
	printk("v3d_triangle: cache: %s, data line %u bytes\n", CACHE_OPT,
	       (unsigned int)sys_cache_data_line_size_get());
	printk("v3d_triangle: %s, V3D 7.1, target %ux%u raster RGBA8888, watchdog armed at boot\n",
	       CONFIG_BOARD, TGT_W, TGT_H);

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
	uint32_t va_code = gpu_map(code, sizeof(code));
	uint32_t va_vtx = gpu_map(vertex, sizeof(vertex));
	uint32_t va_ta = gpu_map(tile_alloc, TILE_ALLOC_BYTES);
	uint32_t va_ts = gpu_map(tile_state, TILE_STATE_BYTES);
	uint32_t va_ov = gpu_map(overflow, OVERFLOW_BYTES);
	uint32_t va_tgt = gpu_map(target, TGT_BYTES);
	if (!va_cl[0] || !va_cl[1] || !va_cl[2] || !va_code || !va_vtx || !va_ta || !va_ts || !va_ov ||
	    !va_tgt) {
		verdict(false, "M2 page table");
		goto out;
	}
	uint64_t pt_pa = k_mem_phys_addr(pt), scratch_pa = k_mem_phys_addr(scratch);
	printk("M2 page table: PA 0x%llx, %u entries, scratch PA 0x%llx, VA 0x%08x..0x%08x mapped, %u pages used\n",
	       (unsigned long long)pt_pa, PT_ENTRIES, (unsigned long long)scratch_pa, VA_FIRST, va_next,
	       pages_used());
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

	/* M5: the programs, FS, VS and CS each at its slot of the code page */
	memset(code, 0, sizeof(code));
	uint32_t code_va[3];
	for (int s = 0; s < 3; s++) {
		if (sh.words[s] * 8u > CODE_SLOT) {
			verdict(false, "frame build (a program is larger than its code slot)");
			goto out;
		}
		code_va[s] = va_code + (uint32_t)s * CODE_SLOT;
		for (uint32_t i = 0; i < sh.words[s]; i++)
			for (int b = 0; b < 8; b++)
				code[s * CODE_SLOT + i * 8u + b] = (uint8_t)(sh.code[s][i] >> (8 * b));
	}
	sh.fs.code = code_va[0];
	sh.vs.code = code_va[1];
	sh.cs.code = code_va[2];

	/* the vertex data: positions, then colours COLOR_OFFSET behind */
	memset(vertex, 0, sizeof(vertex));
	memcpy(vertex, VERTICES, sizeof(VERTICES));
	struct v3d_vertex_attr attrs[TRI_SHADER_ATTRS];
	for (uint32_t i = 0; i < TRI_SHADER_ATTRS; i++)
		attrs[i] = (struct v3d_vertex_attr){
			.address = va_vtx + (i ? COLOR_OFFSET : 0u),
			.stride = sh.attr_stride[i],
			.components = sh.attr_components[i],
		};

	struct v3d_triangle_frame f = {
		.clear = {
			.width = TGT_W,
			.height = TGT_H,
			.clear_rgba8 = CLEAR,
			.target = va_tgt + TGT_STRIDE, /* row 0 of the buffer is a guard row */
			.stride = TGT_STRIDE,
			.tile_alloc = va_ta,
		},
		.front_ccw = false, /* the dump's CFG_BITS "Clockwise Primitives: 0" */
		.fs = &sh.fs,
		.vs = &sh.vs,
		.cs = &sh.cs,
		.attrs = attrs,
		.num_attrs = TRI_SHADER_ATTRS,
		.first_vertex = 0,
		.vertex_count = 3,
	};
	/* glViewport(0, 0, 64, 64), not flipped: window y is the buffer's row */
	v3d_viewport_gl(0, 0, (float)TGT_W, (float)TGT_H, 0, 1, false, TGT_H, &f.vp);

	struct v3d_tiling t;
	uint32_t ta_size, ts_size;
	v3d_frame_tiling_gen(71, TGT_W, TGT_H, 1, false, V3D_INTERNAL_BPP_32, 4, &t);
	/* one draw: the tile allocation Mesa's job gives (16384 here) */
	v3d_frame_alloc_sizes(&t, 1, PAGE, &ta_size, &ts_size);
	if (ta_size > TILE_ALLOC_BYTES || ts_size > TILE_STATE_BYTES) {
		printk("frame: tile allocation %u bytes, tile state %u bytes\n", ta_size, ts_size);
		verdict(false, "frame sizes");
		goto out;
	}

	struct v3d_cl bcl, rcl, ind;
	struct v3d_triangle_layout lo;
	v3d_cl_init(&bcl, cls[0], va_cl[0], CL_BYTES);
	v3d_cl_init(&rcl, cls[1], va_cl[1], CL_BYTES);
	v3d_cl_init(&ind, cls[2], va_cl[2], CL_BYTES);
	if (!v3d_frame_build_triangle(&f, &bcl, &rcl, &ind, &lo)) {
		printk("frame: the builder refused the frame (BCL %u, RCL %u, indirect %u bytes written)\n",
		       bcl.used, rcl.used, ind.used);
		verdict(false, "frame build");
		goto out;
	}
	printk("frame %ux%u: tile %ux%u, %ux%u tiles, BCL %u / RCL %u / indirect %u bytes (record +0x%x, "
	       "attributes +0x%x, tile list +0x%x..+0x%x), tile alloc %u (draws 1), state %u\n",
	       TGT_W, TGT_H, t.tile_w, t.tile_h, t.tiles_x, t.tiles_y, bcl.used, rcl.used, ind.used,
	       lo.record - ind.gpu, lo.attr_records - ind.gpu, lo.tile_list_start - ind.gpu,
	       lo.tile_list_end - ind.gpu, ta_size, ts_size);
	printk("frame uniforms: FS +0x%x, VS +0x%x, CS +0x%x; code FS 0x%08x VS 0x%08x CS 0x%08x\n",
	       lo.fs_uniforms - ind.gpu, lo.vs_uniforms - ind.gpu, lo.cs_uniforms - ind.gpu, sh.fs.code,
	       sh.vs.code, sh.cs.code);
	{
		const uint8_t *rec = cls[2] + (lo.record - ind.gpu);
		const char *first;
		uint32_t bad = tri_shaders_rec_check(&lo.rec, &first);

		printk("frame record: first dword 0x%02x%02x%02x%02x, %u field(s) differ from the %s header%s%s\n",
		       rec[3], rec[2], rec[1], rec[0], bad, sh.name, bad ? ", first " : "",
		       bad ? first : "");
		if (bad) {
			snprintk(why, sizeof(why), "frame record: %u field(s) differ from the header, first %s",
				 bad, first);
			verdict(false, why);
			goto out;
		}
	}

	/* the target pre-filled: the guard rows, then the sentinel the clear must
	 * replace */
	memset(target, GUARD_BYTE, sizeof(target));
	for (uint32_t i = 0; i < TGT_W * TGT_H; i++)
		target[TGT_W + i] = SENTINEL;
	memset(tile_state, 0, sizeof(tile_state));
	memset(overflow, 0, sizeof(overflow));
	if (clean(cls, sizeof(cls), "lists") || clean(code, sizeof(code), "code") ||
	    clean(vertex, sizeof(vertex), "vertices") || clean(tile_alloc, sizeof(tile_alloc), "tile allocation") ||
	    clean(tile_state, sizeof(tile_state), "tile state") ||
	    clean(overflow, sizeof(overflow), "overflow block") || clean(target, sizeof(target), "target")) {
		verdict(false, "M5 cache clean");
		goto out;
	}

	/* M5: bin, with the overflow block in hand */
	uint32_t ov_blocks[1] = { va_ov };
	struct v3d_bin_overflow ov = {
		.blocks = ov_blocks, .nblocks = 1, .block_size = OVERFLOW_BYTES, .used = 0
	};
	printk("M5 before bin: MISCCFG 0x%08x UIFCFG 0x%08x ERR_STAT 0x%08x FDBGS 0x%08x core INT_STS 0x%08x hub INT_STS 0x%08x\n",
	       sys_read32(r.core + CORE_MISCCFG), sys_read32(r.hub + V3D_HUB_CTL_UIFCFG),
	       sys_read32(r.core + V3D_ERR_0_STAT), sys_read32(r.core + CORE_FDBGS),
	       sys_read32(r.core + V3D_CTL_0_INT_STS), sys_read32(r.hub + V3D_HUB_CTL_INT_STS));
	v3d_submit_bin(&r, bcl.gpu, bcl.gpu + bcl.used, va_ta, ta_size, va_ts);
	rc = v3d_wait_bin(&r, WAIT_US, &st, &ov);
	printk("M5 bin: %d, CT0CA 0x%08x CT0EA 0x%08x (end 0x%08x), BPCA 0x%08x BPCS 0x%08x, overflow blocks used %u\n",
	       rc, sys_read32(r.core + V3D_CLE_0_CT0CA), sys_read32(r.core + V3D_CLE_0_CT0EA),
	       bcl.gpu + bcl.used, sys_read32(r.core + V3D_PTB_0_BPCA), sys_read32(r.core + V3D_PTB_0_BPCS),
	       ov.used);
	if (rc) {
		print_stall("M5 bin", &st, &r, id.va_width);
		verdict(false, rc == -ENOMEM ? "M5 bin: out of memory" : rc == -EFAULT ? "M5 bin: MMU fault"
										     : "M5 bin: timeout");
		goto out;
	}
	v3d_submit_render(&r, rcl.gpu, rcl.gpu + rcl.used);
	rc = v3d_wait(&r, V3D_CTL_0_INT_STS_INT_FRDONE_SET, WAIT_US, &st);
	printk("M5 render: %d, CT1CA 0x%08x (end 0x%08x), ERR_STAT 0x%08x\n", rc,
	       sys_read32(r.core + V3D_CLE_0_CT1CA), rcl.gpu + rcl.used, sys_read32(r.core + V3D_ERR_0_STAT));
	if (rc) {
		print_stall("M5 render", &st, &r, id.va_width);
		verdict(false, rc == -EFAULT ? "M5 render: MMU fault" : "M5 render: timeout");
		goto out;
	}
	rc = v3d_caches_clean(&r);
	printk("M5 caches_clean: %d\n", rc);
	if (rc) {
		verdict(false, "M5 GPU cache clean");
		goto out;
	}

	/* the read-back: what the GPU wrote, not a line the CPU still holds */
	rc = sys_cache_data_invd_range(target, sizeof(target));
	if (rc) {
		printk("cache invalidate of the target: %d\n", rc);
		verdict(false, "M5 cache invalidate");
		goto out;
	}
	const uint8_t *buf = (const uint8_t *)target;
	const uint8_t *row0 = buf + TGT_STRIDE;
	struct tri_check_in in = {
		.pixels = row0,
		.width = TGT_W,
		.height = TGT_H,
		.stride = TGT_STRIDE,
		.tol = TOL,
		.margin = MARGIN,
	};
	for (int v = 0; v < 3; v++) {
		const float *pos = &VERTICES[v * 4];
		const float *col = &VERTICES[(COLOR_OFFSET / 4u) + v * 4];

		/* window = ndc * scale + translate, w = 1 */
		in.vx[v] = pos[0] * f.vp.scale[0] + f.vp.translate[0];
		in.vy[v] = pos[1] * f.vp.scale[1] + f.vp.translate[1];
		for (int c = 0; c < 4; c++)
			in.vc[v][c] = unorm8(col[c]);
	}
	for (int c = 0; c < 4; c++)
		in.clear[c] = (uint8_t)(CLEAR >> (8 * c));

	struct tri_check_out o;
	int res = tri_check_run(&in, &o);
	uint32_t guard_bad = tri_check_guard(buf, TGT_STRIDE, 0, TGT_STRIDE, GUARD_BYTE) +
			     tri_check_guard(buf, TGT_STRIDE, TGT_H + 1u, TGT_STRIDE, GUARD_BYTE);
	uint32_t wrong = o.bad_inside + o.bad_outside + o.bad_edge;

	printk("M5 vertices (window): (%d,%d) (%d,%d) (%d,%d), clear %02x %02x %02x %02x, tol %u, margin 1 px\n",
	       (int)in.vx[0], (int)in.vy[0], (int)in.vx[1], (int)in.vy[1], (int)in.vx[2], (int)in.vy[2],
	       in.clear[0], in.clear[1], in.clear[2], in.clear[3], TOL);
	print_pixel("centre", 32, 24, row0);
	print_pixel("near v0", 12, 10, row0);
	print_pixel("near v1", 51, 10, row0);
	print_pixel("near v2", 32, 50, row0);
	print_pixel("corner", 1, 62, row0);
	printk("M5 read-back: %d, %u inside (%u wrong, max diff %u), %u outside (%u wrong), %u edge (%u wrong), "
	       "guard rows %u of %u bytes changed\n",
	       res, o.n_inside, o.bad_inside, o.max_inside_diff, o.n_outside, o.bad_outside, o.n_edge,
	       o.bad_edge, guard_bad, 2u * TGT_STRIDE);
	if (res < 0) {
		verdict(false, "M5 read-back: the check refused its arguments");
		goto out;
	}
	if (wrong) {
		const uint8_t *p = row0 + o.first_bad_y * TGT_STRIDE + o.first_bad_x * 4u;
		struct tri_check_in flip = in;
		struct tri_check_out fo;

		printk("M5 first wrong pixel (%u,%u): %02x %02x %02x %02x\n", o.first_bad_x, o.first_bad_y,
		       p[0], p[1], p[2], p[3]);
		/* a diagnosis only: the same check with the rows upside down */
		for (int v = 0; v < 3; v++)
			flip.vy[v] = (float)TGT_H - in.vy[v];
		(void)tri_check_run(&flip, &fo);
		printk("M5 the same check with the rows flipped: %u wrong\n",
		       fo.bad_inside + fo.bad_outside + fo.bad_edge);
		snprintk(why, sizeof(why),
			 "%u of %u px differ by more than %u: %u inside, %u outside, %u edge, first at %u,%u",
			 wrong, TGT_W * TGT_H, TOL, o.bad_inside, o.bad_outside, o.bad_edge, o.first_bad_x,
			 o.first_bad_y);
		verdict(false, why);
	} else if (guard_bad) {
		snprintk(why, sizeof(why), "guard rows: %u of %u bytes changed", guard_bad, 2u * TGT_STRIDE);
		verdict(false, why);
	} else {
		snprintk(why, sizeof(why),
			 "%ux%u read back, 0 of %u px wrong: %u inside, %u outside, %u edge, tol %u", TGT_W,
			 TGT_H, TGT_W * TGT_H, o.n_inside, o.n_outside, o.n_edge, TOL);
		verdict(true, why);
	}

out:
	/* nothing more: the watchdog resets the SoC, tryboot brings Linux back */
	for (;;)
		k_sleep(K_SECONDS(10));
	return 0;
}
