/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * v3d_triangle: milestone M5 of docs/rpi4-v3d-backend-plan.md for the
 * Raspberry Pi 4. The same bring-up as v3d_clear (M1 power and identity, M2
 * MMU, M3 one bin + render job), and the render is a clear followed by one
 * triangle with a colour per vertex, drawn by the three QPU programs v3d_shc
 * compiled from tools/shaders/passthrough.vert + interp_color.frag.
 *
 * The firmware framebuffer is the console (top half) and the render target
 * (bottom half). Every pixel of the target is read back and put in one of
 * three classes by its distance from the triangle's edges, measured at the
 * pixel centre in window coordinates:
 *
 *   inside  (at least EDGE_PX inside every edge): the interpolated colour,
 *           within COLOR_TOL of 255 per channel, alpha 255
 *   outside (at least EDGE_PX outside some edge): exactly the clear colour
 *   edge    (the band between): either the clear colour or within COLOR_TOL
 *           of the interpolated colour; counted separately
 *
 * and the row above the target (a guard the console never draws: it wraps at
 * 11 lines of 32 px, rows 0..351) must keep the value the CPU wrote.
 * COLOR_TOL is a stated guess, not a measurement: the value the V3D really
 * produces goes through a float16 pack (vfpack) and the TLB's conversion to
 * unorm8, and the first board run prints the largest difference it saw.
 *
 * Every step prints its raw result; the verdict line names what failed.
 *
 * The uniform variant (-DTRI_UNIFORMS=ON, shaders es100/utri) draws the same
 * triangle through a program that reads uniform storage (QUNIFORM_UNIFORM):
 * u_offset moves it (VS and CS), u_scale and u_bias change its colour (FS,
 * or wherever the compiler moved them). The storage is filled by name from
 * tri_uniform_values through the places the generated header lists, every
 * other word is a NaN, and the check expects the values' effect.
 *
 * The if variant (-DTRI_IF=ON, shaders es100/tri_if) draws it through an FS
 * `if (v_color.r > 0.5) c.g = 1.0; else c.b = 0.25;`: the threshold crosses
 * the triangle, so both arms are drawn, and each arm leaves the other's lane
 * at its value from before the if. The check expects each pixel's side and
 * requires pixels on both.
 *
 * The int variant (-DTRI_INT=ON, shaders es100/tri_int) draws it through an
 * FS of int and bool operations (F2i, Ineg, Imul, Iadd, I2f, Ieq, Ine, B2f,
 * sign, and a ternary read by the next add), each of which changes the
 * picture; the check expects each pixel's value and requires every band of
 * i and both sides of each comparison.
 *
 * The discard variant (-DTRI_DISCARD=ON, shaders es100/tri_discard) discards
 * where g > 0.25 and b < 0.25 (a combined condition: `&&`); those pixels keep the
 * clear colour. The check requires discarded and drawn pixels inside.
 *
 * The gl_FragCoord variant (-DTRI_FRAGCOORD=ON, shaders es100/tri_fragcoord,
 * our compiler's JSON) writes (x / 1280, y / 512, b, 1) in GL window
 * coordinates: green grows toward the top of the picture when the window
 * flip (f.wpos: FB_WPOS_Y_SCALE -1, _OFFSET the height) is right, toward the
 * bottom when it is wrong. The check counts inside pixels of both kinds and
 * requires the right one.
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
#include "tri_prog.h"
#include "tri_check.h"

/* A program that reads uniform storage needs the variant that fills it, and
 * the variant needs a program that reads it, or it proves nothing. */
#if TRI_USES_STORAGE && !defined(TRI_UNIFORMS)
#error "the program reads uniform storage: build with -DTRI_UNIFORMS=ON"
#endif
#if !TRI_USES_STORAGE && defined(TRI_UNIFORMS)
#error "TRI_UNIFORMS is set but the program reads no uniform storage"
#endif

#if (defined(TRI_IF) + defined(TRI_UNIFORMS) + defined(TRI_INT) + defined(TRI_DISCARD) + \
     defined(TRI_FRAGCOORD)) > 1
#error "TRI_IF, TRI_UNIFORMS, TRI_INT, TRI_DISCARD and TRI_FRAGCOORD are separate variants"
#endif

#ifndef V3D_BUILD
#define V3D_BUILD "unknown"
#endif

#define FB_W 1280
#define FB_H 720

#define CLEAR_RGBA8 0xff202020u /* dark grey, alpha 255 (R in the low byte) */
#define SENTINEL    0xffff00ffu /* magenta, written by the CPU beforehand */
#define GUARD       0xff0000ffu /* red, the row above the target */

/* GPU memory: the page table covers 8 MiB of GPU virtual address space. */
#define PT_ENTRIES 2048
static uint32_t pt[PT_ENTRIES] __aligned(4096);
static uint8_t scratch[4096] __aligned(4096);
static uint8_t cls[3][4096] __aligned(4096); /* BCL, RCL, indirect */
static uint8_t code[3][4096] __aligned(4096); /* FS, VS, CS */
static uint8_t vertices[4096] __aligned(4096);
static uint8_t tile_alloc[64 * 1024] __aligned(4096);
static uint8_t tile_state[64 * 1024] __aligned(4096);

static uint32_t va_next = 0x00100000; /* VA 0 is never mapped */

#ifdef TRI_UNIFORMS
/* Words no uniform covers: a NaN, so a read of one shows in the picture. */
#define STORAGE_POISON 0x7fc0deadu
static uint32_t vs_words[TRI_VS_STORAGE_WORDS ? TRI_VS_STORAGE_WORDS : 1];
static uint32_t fs_words[TRI_FS_STORAGE_WORDS ? TRI_FS_STORAGE_WORDS : 1];

static const float *uniform_value(const char *name)
{
	if (strcmp(name, "u_offset") == 0)
		return tri_uniform_values.offset;
	if (strcmp(name, "u_scale") == 0)
		return tri_uniform_values.scale;
	if (strcmp(name, "u_bias") == 0)
		return tri_uniform_values.bias;
	return NULL;
}

/* Fill both storages from the header's places; false names the uniform the
 * sample has no value for. */
static bool fill_storage(void)
{
	for (uint32_t i = 0; i < ARRAY_SIZE(vs_words); i++)
		vs_words[i] = STORAGE_POISON;
	for (uint32_t i = 0; i < ARRAY_SIZE(fs_words); i++)
		fs_words[i] = STORAGE_POISON;
	for (int i = 0; i < TRI_NUM_UNIFORM_PLACES; i++) {
		const struct tri_uniform_place *pl = &tri_uniform_places[i];
		const float *v = uniform_value(pl->name);
		uint32_t *words = pl->fs ? fs_words : vs_words;
		uint32_t n = pl->fs ? TRI_FS_STORAGE_WORDS : TRI_VS_STORAGE_WORDS;
		if (!v || pl->words > 4 || pl->word + pl->words > n) {
			printk("uniform %s (%s word %u+%u): %s\n", pl->name, pl->fs ? "fs" : "vs",
			       pl->word, pl->words, v ? "does not fit" : "the sample has no value");
			return false;
		}
		for (uint32_t c = 0; c < pl->words; c++) {
			union { float f; uint32_t u; } b = { .f = v[c] };
			words[pl->word + c] = b.u;
		}
	}
	printk("uniform storage: vs %d words, fs %d words\n", TRI_VS_STORAGE_WORDS,
	       TRI_FS_STORAGE_WORDS);
	for (int s = 0; s < 2; s++) {
		const uint32_t *w = s ? fs_words : vs_words;
		int n = s ? TRI_FS_STORAGE_WORDS : TRI_VS_STORAGE_WORDS;
		for (int i = 0; i < n; i += 4) {
			printk("  %s[%2d] %08x %08x %08x %08x\n", s ? "fs" : "vs", i, w[i],
			       i + 1 < n ? w[i + 1] : 0, i + 2 < n ? w[i + 2] : 0,
			       i + 3 < n ? w[i + 3] : 0);
		}
	}
	return true;
}
#endif

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

/* See v3d_clear: which clock the power-on toggle needs is a board question;
 * the firmware's answer is printed and is not fatal. */
static int clk_set(void *ctx, bool on)
{
	ARG_UNUSED(ctx);
	int ret = rpi_clock_state(RPI_FW_CLK_V3D, on);
	printk("  fw clock 5 %s: %d%s\n", on ? "on" : "off", ret, ret ? " (not fatal)" : "");
	return 0;
}

static void verdict(bool pass, const char *what)
{
	/* The verdict names the shaders' compiler, so one photo of the screen
	 * says whose code passed or failed. */
#ifdef TRI_UNIFORMS
#define TRI_VARIANT ", uniform variant"
#elif defined(TRI_IF)
#define TRI_VARIANT ", if variant"
#elif defined(TRI_INT)
#define TRI_VARIANT ", int variant"
#elif defined(TRI_DISCARD)
#define TRI_VARIANT ", discard variant"
#elif defined(TRI_FRAGCOORD)
#define TRI_VARIANT ", gl_FragCoord variant"
#else
#define TRI_VARIANT ""
#endif
	printk("v3d_triangle: %s (%s) [shaders: %s%s]\n", pass ? "PASS" : "FAIL", what, TRI_COMPILER,
	       TRI_VARIANT);
	/* The console scrolls: the build line printed at boot is gone by now, so
	 * the verdict carries it again (one photo = one result row). */
	printk("v3d_triangle: build %s, json %.16s\n", V3D_BUILD, TRI_JSON_SHA256);
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
	if (ret)
		return; /* no screen: nothing can be reported */
	uint32_t half = fb.height / 2;
	/* scale 1: 22 text rows, so the first lines (M1, the clock replies) are
	 * not overwritten before the verdict */
	fbcon_attach(fb.cpu, fb.width, fb.pitch, half, 1);
	printk("v3d_triangle: build %s\n", V3D_BUILD);
	rpi_wdog_announce();
	printk("shaders compiled by: %s\n", TRI_COMPILER);
	printk("program json sha256 %.16s, VPM size %d (%s)\n", TRI_JSON_SHA256, TRI_VPM_SIZE,
	       TRI_VPM_SIZE_SOURCE);
	printk("assumed (absent from the JSON): %s\n", TRI_ASSUMED[0] ? TRI_ASSUMED : "none");
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
	/* The shaders were compiled for TRI_VPM_SIZE; the board's value is
	 * printed next to it. A difference is reported in the verdict, not
	 * hidden, and does not stop the run. */
	bool vpm_match = id.vpm_size == TRI_VPM_SIZE;
	printk("VPM: board %u bytes, compiled for %d: %s\n", id.vpm_size, TRI_VPM_SIZE,
	       vpm_match ? "same" : "DIFFERENT");

	/* M2: the lists, the code, the vertices, the tile buffers, the framebuffer */
	bool ok = true;
	memset(pt, 0, sizeof(pt));
	uint32_t va_cls[3], va_code[3];
	for (int i = 0; i < 3; i++)
		va_cls[i] = gpu_map(k_mem_phys_addr(cls[i]), sizeof(cls[i]), &ok);
	for (int i = 0; i < 3; i++)
		va_code[i] = gpu_map(k_mem_phys_addr(code[i]), sizeof(code[i]), &ok);
	uint32_t va_vtx = gpu_map(k_mem_phys_addr(vertices), sizeof(vertices), &ok);
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
	printk("M2 mmu_init: %d (VA up to 0x%x)\n", ret, va_next);
	if (ret) {
		verdict(false, "M2 MMU");
		return;
	}

	/* the program: code into its pages, prog_data from the header */
	BUILD_ASSERT(sizeof(tri_fs_code) <= sizeof(code[0]) &&
		     sizeof(tri_vs_code) <= sizeof(code[1]) &&
		     sizeof(tri_cs_code) <= sizeof(code[2]));
	memset(code, 0, sizeof(code));
	memcpy(code[0], tri_fs_code, sizeof(tri_fs_code));
	memcpy(code[1], tri_vs_code, sizeof(tri_vs_code));
	memcpy(code[2], tri_cs_code, sizeof(tri_cs_code));
	struct v3d_fs_prog fs;
	struct v3d_vs_prog vs, cs;
	tri_fs_prog(&fs, va_code[0]);
	tri_vs_prog(&vs, va_code[1]);
	tri_cs_prog(&cs, va_code[2]);

	/* interleaved vec4 position + vec4 colour, 32 bytes a vertex */
	BUILD_ASSERT(TRI_NUM_ATTRS == 2);
	memset(vertices, 0, sizeof(vertices));
	memcpy(vertices, tri_vertices, sizeof(tri_vertices));
	struct v3d_vertex_attr attrs[TRI_NUM_ATTRS];
	attrs[TRI_ATTR_A_POS_RECORD] = (struct v3d_vertex_attr){ va_vtx, 32, 4 };
	attrs[TRI_ATTR_A_COLOR_RECORD] = (struct v3d_vertex_attr){ va_vtx + 16, 32, 4 };

	struct v3d_triangle_frame f = {
		.clear = {
			.width = fb.width,
			.height = fb.height - half,
			.clear_rgba8 = CLEAR_RGBA8,
			.target = va_fb + half * fb.pitch,
			.stride = fb.pitch,
			.tile_alloc = va_ta,
		},
		.front_ccw = true,
		.fs = &fs,
		.vs = &vs,
		.cs = &cs,
		.attrs = attrs,
		.num_attrs = TRI_NUM_ATTRS,
		.first_vertex = 0,
		.vertex_count = 3,
	};
#ifdef TRI_UNIFORMS
	if (!fill_storage()) {
		verdict(false, "uniform storage");
		return;
	}
	if (TRI_VS_STORAGE_WORDS)
		f.vs_storage = (struct v3d_uniform_storage){ vs_words, TRI_VS_STORAGE_WORDS };
	if (TRI_FS_STORAGE_WORDS)
		f.fs_storage = (struct v3d_uniform_storage){ fs_words, TRI_FS_STORAGE_WORDS };
	const struct tri_xform *xf = &tri_uniform_values;
#else
	const struct tri_xform *xf = &tri_identity;
#endif
	printk("expected: offset %d/1000 %d/1000, colour x (%d %d %d %d)/1000 + (%d %d %d %d)/1000\n",
	       (int)(xf->offset[0] * 1000), (int)(xf->offset[1] * 1000),
	       (int)(xf->scale[0] * 1000), (int)(xf->scale[1] * 1000),
	       (int)(xf->scale[2] * 1000), (int)(xf->scale[3] * 1000),
	       (int)(xf->bias[0] * 1000), (int)(xf->bias[1] * 1000), (int)(xf->bias[2] * 1000),
	       (int)(xf->bias[3] * 1000));
	/* glViewport over the whole target, a window-system buffer (first row
	 * at the top), glDepthRange(0, 1) */
	v3d_viewport_gl(0, 0, (float)f.clear.width, (float)f.clear.height, 0.0f, 1.0f, true,
			f.clear.height, &f.vp);
	/* gl_FragCoord's window flip for the same target (first row at the top):
	 * written only for a program that asks for it (FB_WPOS_Y_*). */
	v3d_wpos_gl(true, f.clear.height, &f.wpos);

	struct v3d_tiling t;
	uint32_t ta_size, ts_size;
	v3d_frame_tiling(f.clear.width, f.clear.height, &t);
	v3d_frame_alloc_sizes(&t, 1, 4096, &ta_size, &ts_size);
	if (ta_size > sizeof(tile_alloc) || ts_size > sizeof(tile_state)) {
		printk("tile alloc %u / state %u do not fit\n", ta_size, ts_size);
		verdict(false, "frame sizes");
		return;
	}
	struct v3d_cl bcl, rcl, ind;
	v3d_cl_init(&bcl, cls[0], va_cls[0], sizeof(cls[0]));
	v3d_cl_init(&rcl, cls[1], va_cls[1], sizeof(cls[1]));
	v3d_cl_init(&ind, cls[2], va_cls[2], sizeof(cls[2]));
	struct v3d_triangle_layout lo;
	if (!v3d_frame_build_triangle(&f, &bcl, &rcl, &ind, &lo)) {
		verdict(false, "frame build");
		return;
	}
	printk("frame %ux%u: %ux%u tiles, BCL %u RCL %u indirect %u bytes, TA %u TS %u\n",
	       f.clear.width, f.clear.height, t.tiles_x, t.tiles_y, bcl.used, rcl.used, ind.used,
	       ta_size, ts_size);
	printk("record 0x%x, uniforms fs 0x%x vs 0x%x cs 0x%x, code fs 0x%x vs 0x%x cs 0x%x\n",
	       lo.record, lo.fs_uniforms, lo.vs_uniforms, lo.cs_uniforms, va_code[0], va_code[1],
	       va_code[2]);

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
	sys_cache_data_flush_range(code, sizeof(code));
	sys_cache_data_flush_range(vertices, sizeof(vertices));
	sys_cache_data_flush_range(tile_state, sizeof(tile_state));

	/* M3 */
	struct v3d_stall st;
	v3d_submit_bin(&regs, bcl.gpu, bcl.gpu + bcl.used, va_ta, ta_size, va_ts);
	ret = v3d_wait(&regs, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000000, &st);
	if (ret) {
		print_stall("bin", ret, &st);
		verdict(false, ret == -ENOMEM ? "bin: out of memory" :
			       ret == -EFAULT ? "bin: MMU fault" : "bin: timeout");
		return;
	}
	printk("bin: done\n");
	v3d_submit_render(&regs, rcl.gpu, rcl.gpu + rcl.used);
	ret = v3d_wait(&regs, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 1000000, &st);
	if (ret) {
		print_stall("render", ret, &st);
		verdict(false, ret == -EFAULT ? "render: MMU fault" : "render: timeout");
		return;
	}
	ret = v3d_caches_clean(&regs);
	printk("render: done, clean %d\n", ret);

	/* M5: every pixel of the target, in three classes, and the guard row */
	struct raster r;
	raster_setup(&f.vp, xf, &r);
#ifdef TRI_IF
	r.if_variant = true;
#endif
#ifdef TRI_INT
	r.int_variant = true;
#endif
#ifdef TRI_DISCARD
	r.discard_variant = true;
	r.clear = CLEAR_RGBA8;
#endif
#ifdef TRI_FRAGCOORD
	r.fragcoord_variant = true;
	r.fb_h = (float)f.clear.height;
#endif
	struct tri_result res;
	tri_check(&r, fb.cpu + (size_t)half * fb.pitch, fb.pitch, f.clear.width, f.clear.height,
		  CLEAR_RGBA8, &res);
	uint32_t guard_bad = 0;
	for (uint32_t x = 0; x < fb.width; x++)
		guard_bad += guard[x] != GUARD;

	/* a few named points: the centroid and a point near each vertex */
	static const float probe[4][3] = {
		{ 1.0f / 3, 1.0f / 3, 1.0f / 3 },
		{ 0.8f, 0.1f, 0.1f },
		{ 0.1f, 0.8f, 0.1f },
		{ 0.1f, 0.1f, 0.8f },
	};
	for (int i = 0; i < 4; i++) {
		float px = 0, py = 0;
		for (int k = 0; k < 3; k++) {
			px += probe[i][k] * r.x[k];
			py += probe[i][k] * r.y[k];
		}
		uint32_t x = (uint32_t)px, y = (uint32_t)py;
		float lambda[3];
		raster_at(&r, x + 0.5f, y + 0.5f, lambda);
		uint32_t cand[TRI_MAX_CANDIDATES];
		bool near;
		int nc = candidates(&r, lambda, x + 0.5f, y + 0.5f, cand, &near);
		uint32_t want = cand[0];
		uint32_t got = ((const uint32_t *)(fb.cpu + (size_t)(half + y) * fb.pitch))[x];
		uint32_t diff = max_diff(got, want);
		for (int k = 1; k < nc; k++)
			if (max_diff(got, cand[k]) < diff)
				diff = max_diff(got, cand[k]);
		printk("  probe %d at %u,%u: 0x%08x, expected 0x%08x%s (diff %u)\n", i, x, y, got,
		       want, near ? " or a band neighbour" : "", diff);
	}

	printk("M5: %u pixels = %u inside + %u outside + %u edge\n", res.total, res.n_in,
	       res.n_out, res.n_edge);
	printk("  inside wrong %u (largest channel diff %u, tolerance %u)\n", res.bad_in,
	       res.worst_in, COLOR_TOL);
	printk("  outside wrong %u, edge wrong %u (edge drawn %u, clear %u)\n", res.bad_out,
	       res.bad_edge, res.edge_drawn, res.n_edge - res.edge_drawn - res.bad_edge);
	if (res.have_first)
		printk("  first wrong at %u,%u: 0x%08x, expected 0x%08x\n", res.first_x,
		       res.first_y, res.first_got, res.first_want);
	printk("  guard row %u of %u changed\n", guard_bad, fb.width);
#ifdef TRI_IF
	printk("  if: inside %u then side, %u else side, %u in the band (either accepted)\n",
	       res.n_then, res.n_else, res.n_band);
#endif
#ifdef TRI_DISCARD
	printk("  discard: inside %u discarded, %u drawn, %u in the band (either accepted)\n",
	       res.n_discarded, res.n_drawn, res.n_band);
#endif
#ifdef TRI_FRAGCOORD
	printk("  gl_FragCoord: inside %u match the right flip, %u the wrong flip, %u in the band\n",
	       res.n_flip_right, res.n_flip_wrong, res.n_band);
#endif
#ifdef TRI_INT
	printk("  int: inside i=0..3 %u %u %u %u, g<0.5 %u g>0.5 %u, b>=0.5 %u b<0.5 %u, band %u\n",
	       res.n_int_i[0], res.n_int_i[1], res.n_int_i[2], res.n_int_i[3], res.n_int_g[0],
	       res.n_int_g[1], res.n_int_b[0], res.n_int_b[1], res.n_band);
#endif

	verdict(res.counted && tri_result_ok(&res) && !guard_bad,
		!res.counted ? "pixel classes do not add up" :
		res.bad_in   ? "M5 inside colour" :
		res.bad_out  ? "M5 outside not the clear colour" :
		res.bad_edge ? "M5 edge pixels" :
		guard_bad    ? "M5 guard row" :
		vpm_match    ? "M1-M5, every pixel read back" :
			       "M1-M5, every pixel read back; VPM size differs from the compile");
}

int main(void)
{
	run();
	for (;;)
		k_sleep(K_FOREVER);
	return 0;
}
