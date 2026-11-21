/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_tex: the first texture look-ups on the Raspberry Pi 5's V3D 7.1
 * (roadmap P10 and question U8 of docs/notes/pre-study-rpi5.md), before the
 * 7.1 GL table exists. Two frames of the driver's 7.1 clear-then-triangle
 * builder (v3d_frame_build_triangle, v3d_frame.c + v3d_frame71_tri.c at
 * V3D_CL_VER 71), each with one program of our 7.1 online compiler
 * (src/shaders_ours.h, gen/gen_shaders.sh) whose fragment shader does one
 * TMU look-up; the records it reads are v3d_tex71.c's:
 *
 *   M1, M2 as rpi5_triangle: the V3D clock, power-on, identity; one page
 *          table over the probe's statics, the MMU on.
 *   T1     a 64x64 RGBA8 2D texture with a per-texel pattern, tiled to
 *          Mesa's layout (v3d_tex_layout: level 0 UIF) by v3d_tex.c, drawn
 *          by a full-screen quad (texture2D, NEAREST, wrap REPEAT); the
 *          64x64 raster target read back and every pixel compared with its
 *          texel, within TEX_TOL per channel.
 *   C1     a 4x4 RGBA8 cube map (gl_cube's shape: six faces as layers, one
 *          colour per face) sampled with LINEAR filtering, wrap CLAMP in S,
 *          T and R (what st/mesa sets for a seamless cube, the sampler of
 *          Mesa's tools/v3d/pi5/gl_cube.clif), at 54 directions: each face's
 *          centre, a point a quarter texel inside each of its four edges and
 *          an eighth inside each of its four corners. Each direction fills
 *          one 8x8 block of the 64x64 target (two triangles whose three
 *          vertices carry the same direction); the block's centre pixel is
 *          compared with the seamless prediction (the texel beyond the edge
 *          taken from the neighbouring face, a corner's missing texel the
 *          mean of the three faces) and with the non-seamless one (the face
 *          clamped to its own edge texels).
 *
 * The console then has exactly these verdict lines, in this order:
 *
 *   v3d_tex: PASS <n> of <m> px wrong | v3d_tex: FAIL <n> of <m> px wrong
 *   v3d_cube: PASS seamless=yes <n> of <m> samples wrong
 *   | v3d_cube: FAIL seamless=<yes|no|neither> <n> of <m> samples wrong
 *   rpi5_tex: done (...) | rpi5_tex: FAIL (reason)
 *
 * A step that stops the probe before a part ran prints that part's line as
 * FAIL with every px or sample counted wrong (and seamless=neither), so both
 * lines are always there; the end line then names the step. Nothing runs
 * afterwards: the board layer's PM watchdog (armed in the first PRE_KERNEL_1
 * hook) resets the SoC and tryboot brings Linux back.
 *
 * Every buffer the GPU reads or writes is a static, written through the
 * CPU's cache: cleaned and invalidated before the GPU sees it, the target
 * invalidated again before the CPU reads it. A cache call that fails ends
 * the run.
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
#include "v3d_tex.h"
#include "v3d_tex71.h"
#include "rpi_mbox.h"
#include "shaders_ours.h"

#define PAGE 4096u

/* The GPU's view, as rpi5_triangle: one page table from VA 0, VA 0 unmapped. */
#define PT_ENTRIES 4096u
#define VA_FIRST   0x00100000u

/* The target: 64x64 RGBA8888 raster, one guard row above and one below. */
#define TGT_W      64u
#define TGT_H      64u
#define TGT_STRIDE (TGT_W * 4u)
#define TGT_BYTES  ROUND_UP((TGT_H + 2u) * TGT_STRIDE, PAGE)
#define SENTINEL   0xffff00ffu /* magenta: the clear must replace it */
#define GUARD_BYTE 0xa5u
#define CLEAR      0x00000000u

/* T1: 64x64 RGBA8, one level (Mesa's layout: level 0 UIF, 16 KiB) */
#define TEX_W     64u
#define TEX_H     64u
#define TEX_BYTES (32u * 1024u)
#define TEX_TOL   1u
/* C1: 4x4 RGBA8 faces, six layers (64 bytes each, gl_cube's record) */
#define CUBE_SIZE  4u
#define CUBE_BYTES PAGE
#define CUBE_TOL   8u
/* the fraction of the look-up's footprint beyond the face: an edge sample
 * a quarter texel, a corner sample an eighth (in each direction) */
#define EDGE_F     0.25f
#define CORNER_F   0.125f
#define BLOCK      8u /* px: one C1 sample per 8x8 block, 8x8 blocks */
#define MAX_SAMPLES ((TGT_W / BLOCK) * (TGT_H / BLOCK))
#define C1_SAMPLES  54u /* 6 faces x (centre, 4 edges, 4 corners) */

#define CL_BYTES         PAGE
#define CODE_SLOT        0x200u /* FS, VS, CS each at a 512-byte slot */
#define TILE_ALLOC_BYTES (64u * 1024u)
#define TILE_STATE_BYTES (64u * 1024u)
#define OVERFLOW_BYTES   (256u * 1024u)
#define VTX_BYTES        (16u * 1024u)
#define WAIT_US          1000000u

/* Mesa's own records (tools/v3d/pi5/README.md, the texture record table):
 * gl_tex2d job 1's texture state words 1..3 (word 0, 4 and 5 hold its base
 * address) and sampler words 0..1, gl_cube's for its LINEAR jobs. For the
 * comparison lines only. */
static const uint32_t MESA_TEX2D_STATE_W1_3[3] = { 0x00000200u, 0x00404001u, 0x00b1a840u };
static const uint32_t MESA_TEX2D_SAMPLER[2] = { 0x00100007u, 0x00000000u };
static const uint32_t MESA_CUBE_STATE_W1_3[3] = { 0x10000002u, 0x01800400u, 0x00b1a040u };
static const uint32_t MESA_CUBE_SAMPLER[2] = { 0x00100004u, 0x00490000u };

/* the six face colours (+X, -X, +Y, -Y, +Z, -Z; R in the low byte): any two
 * faces that share an edge differ by 255 in at least one channel */
static const uint32_t FACE_RGBA[6] = {
	0xff0000ffu, 0xffffff00u, 0xff00ff00u, 0xffff00ffu, 0xffff0000u, 0xff00ffffu,
};

static uint32_t pt[PT_ENTRIES] __aligned(PAGE);
static uint8_t scratch[PAGE] __aligned(PAGE);
static uint8_t cls[3][CL_BYTES] __aligned(PAGE); /* binning, render, indirect */
static uint8_t code[PAGE] __aligned(PAGE);
static float vtx[VTX_BYTES / 4] __aligned(PAGE);
static uint8_t tile_alloc[TILE_ALLOC_BYTES] __aligned(PAGE);
static uint8_t tile_state[TILE_STATE_BYTES] __aligned(PAGE);
static uint8_t overflow[OVERFLOW_BYTES] __aligned(PAGE);
static uint32_t target[TGT_BYTES / 4] __aligned(PAGE);
static uint8_t tex[TEX_BYTES] __aligned(PAGE);
static uint8_t cube[CUBE_BYTES] __aligned(PAGE);
/* the texture and sampler records: T1's at 0x00 and 0x20, C1's at 0x40 and
 * 0x60 (32-byte aligned, V3D_TEX71_RECORD_ALIGN) */
static uint8_t recs[PAGE] __aligned(PAGE);
/* CPU only: T1's source texels */
static uint32_t texels[TEX_W * TEX_H];

static uint32_t va_next = VA_FIRST;
static struct v3d_regs r = { .soc = V3D_SOC_BCM2712 };
static struct v3d_identity id;
static uint32_t va_cl[3], va_code, va_vtx, va_ta, va_ts, va_ov, va_tgt, va_tex, va_cube, va_recs;
static char why[200];

#define CACHE_OPT                                                                                  \
	"CONFIG_CACHE_MANAGEMENT=" STRINGIFY(IS_ENABLED(CONFIG_CACHE_MANAGEMENT)) " CONFIG_DCACHE="  \
	STRINGIFY(IS_ENABLED(CONFIG_DCACHE)) " sys_cache_data_flush_and_invd_range"

/* ---- the verdict lines: each printed exactly once ---- */

static bool tex_said, cube_said;

static void tex_verdict(uint32_t wrong, uint32_t total)
{
	if (tex_said)
		return;
	tex_said = true;
	printk("v3d_tex: %s %u of %u px wrong\n", wrong ? "FAIL" : "PASS", wrong, total);
}

/* PASS only for seamless=yes with no sample wrong */
static void cube_verdict(const char *seamless, uint32_t wrong, uint32_t total)
{
	if (cube_said)
		return;
	cube_said = true;
	bool pass = !wrong && !strcmp(seamless, "yes");
	printk("v3d_cube: %s seamless=%s %u of %u samples wrong\n", pass ? "PASS" : "FAIL", seamless, wrong,
	       total);
}

/* ---- helpers, as rpi5_triangle ---- */

static void print_stall(const char *step, const struct v3d_stall *st)
{
	printk("%s: core_sts 0x%08x hub_sts 0x%08x ct0 0x%08x..0x%08x ct1 0x%08x..0x%08x\n", step,
	       st->core_int_sts, st->hub_int_sts, st->ct0ca, st->ct0ra, st->ct1ca, st->ct1ra);
	printk("%s: err_stat 0x%08x mmu_ctl 0x%08x vio_addr 0x%08x vio_id 0x%08x (client 0x%02x)\n", step,
	       st->err_stat, st->mmu_ctl, st->mmu_vio_addr, st->mmu_vio_id, st->mmu_vio_id & 0x7fu);
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

static uint32_t chan(uint32_t v, unsigned int c)
{
	return (v >> (8u * c)) & 0xffu;
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

static uint32_t rd32le(const uint8_t *p)
{
	return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

/* T1's texel pattern: every channel moves with x or y (rpi5_tfu's) */
static uint32_t pattern(uint32_t x, uint32_t y)
{
	uint32_t rr = x * 4u + 2u, gg = y * 4u + 1u, bb = ((x ^ y) * 4u) & 0xffu;
	uint32_t aa = 0xffu - (x + y) * 2u;

	return rr | gg << 8 | bb << 16 | aa << 24;
}

/* ---- one program and one frame ---- */

struct prog {
	const char *name;
	const uint64_t *code[3]; /* FS, VS, CS */
	uint32_t words[3];
	struct v3d_fs_prog fs;
	struct v3d_vs_prog vs, cs;
	/* the FS stream with the TMU P0/P1 words turned into constants */
	struct v3d_uniform fs_uni[8];
};

/* Place the code in the code page and resolve the FS's TMU words (our
 * compiler's contents 40/41, data = binding << 24 | the packed bits) with
 * the records' addresses, as v3d_job.c does for the 4.2 table (v3d_tex_p0 /
 * v3d_tex_p1). False, with the reason printed, for anything else. */
static bool prog_place(struct prog *p, uint32_t tex_state_va, uint32_t sampler_va)
{
	memset(code, 0, sizeof(code));
	for (int s = 0; s < 3; s++) {
		if (p->words[s] * 8u > CODE_SLOT) {
			printk("%s: stage %d has %u instructions, more than its slot\n", p->name, s, p->words[s]);
			return false;
		}
		for (uint32_t i = 0; i < p->words[s]; i++)
			for (int b = 0; b < 8; b++)
				code[s * CODE_SLOT + i * 8u + b] = (uint8_t)(p->code[s][i] >> (8 * b));
	}
	p->fs.code = va_code;
	p->vs.code = va_code + CODE_SLOT;
	p->cs.code = va_code + 2u * CODE_SLOT;
	if (p->fs.num_uniforms > (sizeof(p->fs_uni) / sizeof(p->fs_uni[0]))) {
		printk("%s: FS has %u uniforms\n", p->name, p->fs.num_uniforms);
		return false;
	}
	for (uint32_t i = 0; i < p->fs.num_uniforms; i++) {
		struct v3d_uniform u = p->fs.uniforms[i];
		uint32_t v;

		if (u.contents == 40u || u.contents == 41u) {
			bool ok = u.contents == 40u ? v3d_tex_p0(tex_state_va, u.data, &v)
						    : v3d_tex_p1(sampler_va, u.data, &v);
			if (!ok || (u.data >> 24) != 0u) {
				printk("%s: FS uniform %u (contents %u data 0x%08x) refused\n", p->name, i,
				       u.contents, u.data);
				return false;
			}
			printk("%s: FS uniform %u: TMU config P%u 0x%08x\n", p->name, i, u.contents - 40u, v);
			u = (struct v3d_uniform){ .contents = V3D_QUNIFORM_CONSTANT, .data = v };
		}
		p->fs_uni[i] = u;
	}
	p->fs.uniforms = p->fs_uni;
	return true;
}

/* Build, bin and render one clear-then-draw frame into the target, then
 * make the target readable. `tag` prefixes the step lines; on a failure
 * `why` holds the reason and false is returned. */
static bool run_frame(const char *tag, struct prog *p, uint32_t attr_bytes, uint32_t vertex_count,
		      const uint32_t *comps, const uint32_t *offsets, const uint32_t *strides)
{
	struct v3d_stall st;
	int rc;

	struct v3d_vertex_attr attrs[2];
	for (uint32_t i = 0; i < 2u; i++)
		attrs[i] = (struct v3d_vertex_attr){
			.address = va_vtx + offsets[i], .stride = strides[i], .components = comps[i]
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
		.front_ccw = false,
		.fs = &p->fs,
		.vs = &p->vs,
		.cs = &p->cs,
		.attrs = attrs,
		.num_attrs = 2,
		.first_vertex = 0,
		.vertex_count = vertex_count,
	};
	/* glViewport(0, 0, 64, 64), not flipped: window y is the buffer's row */
	v3d_viewport_gl(0, 0, (float)TGT_W, (float)TGT_H, 0, 1, false, TGT_H, &f.vp);

	struct v3d_tiling t;
	uint32_t ta_size, ts_size;
	v3d_frame_tiling_gen(71, TGT_W, TGT_H, 1, false, V3D_INTERNAL_BPP_32, 4, &t);
	v3d_frame_alloc_sizes(&t, 1, PAGE, &ta_size, &ts_size);
	if (ta_size > TILE_ALLOC_BYTES || ts_size > TILE_STATE_BYTES) {
		snprintk(why, sizeof(why), "%s frame sizes: tile allocation %u, tile state %u", tag, ta_size,
			 ts_size);
		return false;
	}

	struct v3d_cl bcl, rcl, ind;
	struct v3d_triangle_layout lo;
	memset(cls, 0, sizeof(cls));
	v3d_cl_init(&bcl, cls[0], va_cl[0], CL_BYTES);
	v3d_cl_init(&rcl, cls[1], va_cl[1], CL_BYTES);
	v3d_cl_init(&ind, cls[2], va_cl[2], CL_BYTES);
	if (!v3d_frame_build_triangle(&f, &bcl, &rcl, &ind, &lo)) {
		snprintk(why, sizeof(why), "%s frame build refused (BCL %u, RCL %u, indirect %u bytes)", tag,
			 bcl.used, rcl.used, ind.used);
		return false;
	}
	const uint8_t *rec = cls[2] + (lo.record - ind.gpu);
	printk("%s frame: %s, %u vertices, tile %ux%u, BCL %u / RCL %u / indirect %u bytes, record first "
	       "dword 0x%08x, code FS 0x%08x VS 0x%08x CS 0x%08x (%u/%u/%u instructions)\n",
	       tag, p->name, vertex_count, t.tile_w, t.tile_h, bcl.used, rcl.used, ind.used, rd32le(rec),
	       p->fs.code, p->vs.code, p->cs.code, p->words[0], p->words[1], p->words[2]);

	memset(target, GUARD_BYTE, sizeof(target));
	for (uint32_t i = 0; i < TGT_W * TGT_H; i++)
		target[TGT_W + i] = SENTINEL;
	memset(tile_state, 0, sizeof(tile_state));
	memset(overflow, 0, sizeof(overflow));
	if (clean(cls, sizeof(cls), "lists") || clean(code, sizeof(code), "code") ||
	    clean(vtx, ROUND_UP(attr_bytes, 64u), "vertices") ||
	    clean(tile_alloc, sizeof(tile_alloc), "tile allocation") ||
	    clean(tile_state, sizeof(tile_state), "tile state") ||
	    clean(overflow, sizeof(overflow), "overflow block") || clean(target, sizeof(target), "target") ||
	    clean(tex, sizeof(tex), "texture") || clean(cube, sizeof(cube), "cube map") ||
	    clean(recs, sizeof(recs), "records")) {
		snprintk(why, sizeof(why), "%s cache clean", tag);
		return false;
	}
	v3d_caches_invalidate(&r);

	uint32_t ov_blocks[1] = { va_ov };
	struct v3d_bin_overflow ov = { .blocks = ov_blocks, .nblocks = 1, .block_size = OVERFLOW_BYTES, .used = 0 };
	v3d_submit_bin(&r, bcl.gpu, bcl.gpu + bcl.used, va_ta, ta_size, va_ts);
	rc = v3d_wait_bin(&r, WAIT_US, &st, &ov);
	printk("%s bin: %d, overflow blocks used %u\n", tag, rc, ov.used);
	if (rc) {
		print_stall(tag, &st);
		snprintk(why, sizeof(why), "%s bin: %s", tag,
			 rc == -ENOMEM ? "out of memory" : rc == -EFAULT ? "MMU fault" : "timeout");
		return false;
	}
	v3d_submit_render(&r, rcl.gpu, rcl.gpu + rcl.used);
	rc = v3d_wait(&r, V3D_CTL_0_INT_STS_INT_FRDONE_SET, WAIT_US, &st);
	printk("%s render: %d, ERR_STAT 0x%08x\n", tag, rc, sys_read32(r.core + V3D_ERR_0_STAT));
	if (rc) {
		print_stall(tag, &st);
		snprintk(why, sizeof(why), "%s render: %s", tag, rc == -EFAULT ? "MMU fault" : "timeout");
		return false;
	}
	rc = v3d_caches_clean(&r);
	if (rc) {
		snprintk(why, sizeof(why), "%s GPU cache clean: %d", tag, rc);
		return false;
	}
	rc = sys_cache_data_invd_range(target, sizeof(target));
	if (rc) {
		snprintk(why, sizeof(why), "%s target cache invalidate: %d", tag, rc);
		return false;
	}
	return true;
}

static uint32_t guard_bytes_changed(void)
{
	const uint8_t *b = (const uint8_t *)target;
	uint32_t n = 0;

	for (uint32_t i = 0; i < TGT_STRIDE; i++) {
		n += b[i] != GUARD_BYTE;
		n += b[(TGT_H + 1u) * TGT_STRIDE + i] != GUARD_BYTE;
	}
	return n;
}

static uint32_t pixel(uint32_t x, uint32_t y)
{
	return target[TGT_W * (1u + y) + x];
}

/* One quad (two triangles, six vertices; the frame culls nothing) over the
 * window box [x0,x1) x [y0,y1): six vec4 positions at `pos`. */
static void quad_pos(float *pos, uint32_t x0, uint32_t y0, uint32_t x1, uint32_t y1)
{
	static const uint8_t corner[6][2] = { { 0, 0 }, { 1, 0 }, { 1, 1 }, { 0, 0 }, { 1, 1 }, { 0, 1 } };

	for (int v = 0; v < 6; v++) {
		float wx = (float)(corner[v][0] ? x1 : x0), wy = (float)(corner[v][1] ? y1 : y0);

		pos[v * 4 + 0] = wx / (float)(TGT_W / 2u) - 1.0f;
		pos[v * 4 + 1] = wy / (float)(TGT_H / 2u) - 1.0f;
		pos[v * 4 + 2] = 0.0f;
		pos[v * 4 + 3] = 1.0f;
	}
}

/* ---- T1 ---- */

static bool t1_run(struct prog *p)
{
	struct v3d_tex_layout l;

	if (!v3d_tex_layout(TEX_W, TEX_H, 1, 4, &l) || l.size > TEX_BYTES) {
		snprintk(why, sizeof(why), "T1 layout");
		return false;
	}
	for (uint32_t y = 0; y < TEX_H; y++)
		for (uint32_t x = 0; x < TEX_W; x++)
			texels[y * TEX_W + x] = pattern(x, y);
	memset(tex, GUARD_BYTE, sizeof(tex));
	v3d_tex_store_level(tex, &l, 0, (const uint8_t *)texels, TEX_W * 4u);

	struct v3d_tex71_state s = { 0 };
	v3d_tex71_state_from_layout(&s, &l);
	s.base = va_tex + l.slices[0].offset;
	s.width = TEX_W;
	s.height = TEX_H;
	s.depth = 1;
	s.type = V3D_TEXTURE_DATA_FORMAT_RGBA8;
	s.swizzle[0] = V3D_SWIZZLE_RED;
	s.swizzle[1] = V3D_SWIZZLE_GREEN;
	s.swizzle[2] = V3D_SWIZZLE_BLUE;
	s.swizzle[3] = V3D_SWIZZLE_ALPHA;
	s.transfer_func = V3D_TEX71_TRANSFER_NONE;
	struct v3d_tex71_sampler sm = {
		.wrap_s = V3D_TEX71_WRAP_REPEAT,
		.wrap_t = V3D_TEX71_WRAP_REPEAT,
		.wrap_r = V3D_TEX71_WRAP_REPEAT,
		.min_nearest = true,
		.mag_nearest = true,
		.mip_nearest = true,
		.mip_none = true,
		.min_lod = 0.0f,
		.max_lod = 1000.0f,
	};
	memset(recs, 0, sizeof(recs));
	if (!v3d_tex71_pack_state(recs + 0x00, &s) || !v3d_tex71_pack_sampler(recs + 0x20, &sm)) {
		snprintk(why, sizeof(why), "T1 v3d_tex71 refused the records");
		return false;
	}
	bool st_same = true, sm_same = true;
	for (int w = 0; w < 3; w++)
		st_same &= rd32le(recs + 4 * (w + 1)) == MESA_TEX2D_STATE_W1_3[w];
	for (int w = 0; w < 2; w++)
		sm_same &= rd32le(recs + 0x20 + 4 * w) == MESA_TEX2D_SAMPLER[w];
	printk("T1 texture: %ux%u RGBA8, level 0 at +0x%x (%s), image %u bytes; state 0x%08x 0x%08x 0x%08x "
	       "0x%08x 0x%08x 0x%08x (words 1-3 %s Mesa's gl_tex2d); sampler 0x%08x 0x%08x (%s Mesa's "
	       "gl_tex2d NEAREST)\n",
	       TEX_W, TEX_H, l.slices[0].offset,
	       l.slices[0].tiling == V3D_TEX_TILING_UIF_NO_XOR ? "UIF_NO_XOR" : "not UIF_NO_XOR", l.size,
	       rd32le(recs), rd32le(recs + 4), rd32le(recs + 8), rd32le(recs + 12), rd32le(recs + 16),
	       rd32le(recs + 20), st_same ? "equal" : "differ from", rd32le(recs + 0x20),
	       rd32le(recs + 0x24), sm_same ? "equal" : "differ from");

	if (!prog_place(p, va_recs + 0x00, va_recs + 0x20)) {
		snprintk(why, sizeof(why), "T1 program");
		return false;
	}

	/* the full-screen quad: pos at 0, uv at 0x100 (uv (0,0) at NDC (-1,-1)) */
	memset(vtx, 0, sizeof(vtx));
	quad_pos(vtx, 0, 0, TGT_W, TGT_H);
	float *uv = vtx + 0x100 / 4;
	for (int v = 0; v < 6; v++) {
		uv[v * 2 + 0] = (vtx[v * 4 + 0] + 1.0f) * 0.5f;
		uv[v * 2 + 1] = (vtx[v * 4 + 1] + 1.0f) * 0.5f;
	}
	const uint32_t comps[2] = { 4, 2 }, offs[2] = { 0, 0x100 }, strides[2] = { 16, 8 };
	if (!run_frame("T1", p, 0x200, 6, comps, offs, strides))
		return false;

	uint32_t wrong = 0, maxd = 0, first = 0;
	for (uint32_t y = 0; y < TGT_H; y++) {
		for (uint32_t x = 0; x < TGT_W; x++) {
			uint32_t d = max_chan_diff(pixel(x, y), texels[y * TEX_W + x]);

			maxd = MAX(maxd, d);
			if (d > TEX_TOL && wrong++ == 0)
				first = y * TGT_W + x;
		}
	}
	uint32_t guard = guard_bytes_changed();
	printk("T1 read-back: %u of %u px off by more than %u (max %u), guard rows %u bytes changed; "
	       "(0,0) 0x%08x want 0x%08x, (63,63) 0x%08x want 0x%08x\n",
	       wrong, TGT_W * TGT_H, TEX_TOL, maxd, guard, pixel(0, 0), texels[0], pixel(63, 63),
	       texels[TEX_W * TEX_H - 1u]);
	if (wrong)
		printk("T1 first wrong px (%u,%u): 0x%08x want 0x%08x\n", first % TGT_W, first / TGT_W,
		       pixel(first % TGT_W, first / TGT_W), texels[first]);
	/* a guard row the GPU wrote counts as one wrong px per changed row */
	tex_verdict(wrong + (guard ? 1u : 0u), TGT_W * TGT_H);
	return true;
}

/* ---- C1 ---- */

struct sample {
	float dir[3];
	uint32_t seamless, clamp; /* the two predictions */
	const char *kind;         /* "centre", "edge", "corner" */
	uint8_t face;
};

static uint32_t mix(const float w[4], const uint32_t c[4])
{
	uint32_t out = 0;

	for (unsigned int ch = 0; ch < 4u; ch++) {
		float v = 0.0f;

		for (int i = 0; i < 4; i++)
			v += w[i] * (float)chan(c[i], ch);
		out |= ((uint32_t)(v + 0.5f) & 0xffu) << (8u * ch);
	}
	return out;
}

/* face index of axis a (0 x, 1 y, 2 z) with sign s (+1, -1): GL's order */
static uint8_t face_of(int a, int s)
{
	return (uint8_t)(2 * a + (s < 0 ? 1 : 0));
}

static uint32_t c1_samples(struct sample *out)
{
	uint32_t n = 0;
	/* the distance from the edge, in direction units, that leaves a
	 * fraction f of the bilinear footprint beyond it on a 4-texel face:
	 * the in-face coordinate is 2 * delta texels from the edge, and the
	 * outside texel's weight is 0.5 - 2 * delta */
	const float de = (0.5f - EDGE_F) / 2.0f, dc = (0.5f - CORNER_F) / 2.0f;

	for (int a = 0; a < 3; a++) {
		for (int s = 1; s >= -1; s -= 2) {
			const int p = (a + 1) % 3, q = (a + 2) % 3;
			const uint8_t fa = face_of(a, s);
			const uint32_t A = FACE_RGBA[fa];
			struct sample c = { .face = fa, .kind = "centre" };

			c.dir[a] = (float)s;
			c.seamless = c.clamp = A;
			out[n++] = c;
			/* the four edges: toward +p, -p, +q, -q */
			for (int e = 0; e < 4; e++) {
				const int ax = e < 2 ? p : q, sg = (e & 1) ? -1 : 1;
				struct sample x = { .face = fa, .kind = "edge" };
				const float w[4] = { 1.0f - EDGE_F, EDGE_F, 0.0f, 0.0f };
				const uint32_t col[4] = { A, FACE_RGBA[face_of(ax, sg)], 0, 0 };

				x.dir[a] = (float)s;
				x.dir[ax] = (float)sg * (1.0f - de);
				x.seamless = mix(w, col);
				x.clamp = A;
				out[n++] = x;
			}
			/* the four corners */
			for (int k = 0; k < 4; k++) {
				const int sp = (k & 1) ? -1 : 1, sq = (k & 2) ? -1 : 1;
				const uint32_t B = FACE_RGBA[face_of(p, sp)], C = FACE_RGBA[face_of(q, sq)];
				const float f = CORNER_F, g = 1.0f - CORNER_F;
				/* the corner texel no face has: the mean of the three */
				const float w[4] = { g * g + f * f / 3.0f, f * g + f * f / 3.0f, f * g + f * f / 3.0f,
						     0.0f };
				const uint32_t col[4] = { A, B, C, 0 };
				struct sample x = { .face = fa, .kind = "corner" };

				x.dir[a] = (float)s;
				x.dir[p] = (float)sp * (1.0f - dc);
				x.dir[q] = (float)sq * (1.0f - dc);
				x.seamless = mix(w, col);
				x.clamp = A;
				out[n++] = x;
			}
		}
	}
	return n;
}

static bool c1_run(struct prog *p)
{
	static struct sample smp[MAX_SAMPLES];
	struct v3d_tex_layout l;
	uint32_t n = c1_samples(smp);

	if (!v3d_tex_layout(CUBE_SIZE, CUBE_SIZE, 1, 4, &l)) {
		snprintk(why, sizeof(why), "C1 layout");
		return false;
	}
	const uint32_t stride = v3d_tex_layer_stride(&l);
	if (stride * 6u > CUBE_BYTES) {
		snprintk(why, sizeof(why), "C1 layout: layer stride %u", stride);
		return false;
	}
	memset(cube, GUARD_BYTE, sizeof(cube));
	for (int f = 0; f < 6; f++) {
		uint32_t face[CUBE_SIZE * CUBE_SIZE];

		for (uint32_t i = 0; i < CUBE_SIZE * CUBE_SIZE; i++)
			face[i] = FACE_RGBA[f];
		v3d_tex_store_level(cube + (uint32_t)f * stride, &l, 0, (const uint8_t *)face, CUBE_SIZE * 4u);
	}

	struct v3d_tex71_state s = { 0 };
	v3d_tex71_state_from_layout(&s, &l);
	s.base = va_cube + l.slices[0].offset;
	s.width = CUBE_SIZE;
	s.height = CUBE_SIZE;
	s.depth = 6;
	s.type = V3D_TEXTURE_DATA_FORMAT_RGBA8;
	s.swizzle[0] = V3D_SWIZZLE_RED;
	s.swizzle[1] = V3D_SWIZZLE_GREEN;
	s.swizzle[2] = V3D_SWIZZLE_BLUE;
	s.swizzle[3] = V3D_SWIZZLE_ALPHA;
	s.transfer_func = V3D_TEX71_TRANSFER_NONE;
	/* gl_cube's LINEAR sampler: wrap CLAMP in all three (st/mesa's seamless
	 * rule), mag and min LINEAR, no mip filter */
	struct v3d_tex71_sampler sm = {
		.wrap_s = V3D_TEX71_WRAP_CLAMP,
		.wrap_t = V3D_TEX71_WRAP_CLAMP,
		.wrap_r = V3D_TEX71_WRAP_CLAMP,
		.min_nearest = false,
		.mag_nearest = false,
		.mip_nearest = true,
		.mip_none = true,
		.min_lod = 0.0f,
		.max_lod = 1000.0f,
	};
	if (!v3d_tex71_pack_state(recs + 0x40, &s) || !v3d_tex71_pack_sampler(recs + 0x60, &sm)) {
		snprintk(why, sizeof(why), "C1 v3d_tex71 refused the records");
		return false;
	}
	bool st_same = true, sm_same = true;
	for (int w = 0; w < 3; w++)
		st_same &= rd32le(recs + 0x40 + 4 * (w + 1)) == MESA_CUBE_STATE_W1_3[w];
	for (int w = 0; w < 2; w++)
		sm_same &= rd32le(recs + 0x60 + 4 * w) == MESA_CUBE_SAMPLER[w];
	printk("C1 cube: 6 faces %ux%u RGBA8, layer stride %u, level 0 at +0x%x; state 0x%08x 0x%08x 0x%08x "
	       "0x%08x 0x%08x 0x%08x (words 1-3 %s Mesa's gl_cube); sampler 0x%08x 0x%08x (%s Mesa's "
	       "gl_cube LINEAR)\n",
	       CUBE_SIZE, CUBE_SIZE, stride, l.slices[0].offset, rd32le(recs + 0x40), rd32le(recs + 0x44),
	       rd32le(recs + 0x48), rd32le(recs + 0x4c), rd32le(recs + 0x50), rd32le(recs + 0x54),
	       st_same ? "equal" : "differ from", rd32le(recs + 0x60), rd32le(recs + 0x64),
	       sm_same ? "equal" : "differ from");

	if (!prog_place(p, va_recs + 0x40, va_recs + 0x60)) {
		snprintk(why, sizeof(why), "C1 program");
		return false;
	}

	/* one 8x8 block per sample: pos at 0, the direction (vec4, w 0) after */
	const uint32_t pos_bytes = n * 6u * 16u;
	memset(vtx, 0, sizeof(vtx));
	float *dir = vtx + pos_bytes / 4u;
	if (2u * pos_bytes > sizeof(vtx) || n != C1_SAMPLES || n > MAX_SAMPLES) {
		snprintk(why, sizeof(why), "C1 vertices: %u samples", n);
		return false;
	}
	for (uint32_t i = 0; i < n; i++) {
		const uint32_t bx = i % (TGT_W / BLOCK), by = i / (TGT_W / BLOCK);

		quad_pos(vtx + i * 24u, bx * BLOCK, by * BLOCK, (bx + 1u) * BLOCK, (by + 1u) * BLOCK);
		for (int v = 0; v < 6; v++)
			for (int c = 0; c < 3; c++)
				dir[i * 24u + v * 4 + c] = smp[i].dir[c];
	}
	const uint32_t comps[2] = { 4, 4 }, offs[2] = { 0, pos_bytes }, strides[2] = { 16, 16 };
	if (!run_frame("C1", p, 2u * pos_bytes, n * 6u, comps, offs, strides))
		return false;

	uint32_t wrong = 0, as_seamless = 0, as_clamp = 0, off_edge = 0, n_edge = 0;
	for (uint32_t i = 0; i < n; i++) {
		const uint32_t bx = i % (TGT_W / BLOCK), by = i / (TGT_W / BLOCK);
		const uint32_t got = pixel(bx * BLOCK + BLOCK / 2u, by * BLOCK + BLOCK / 2u);
		const uint32_t ds = max_chan_diff(got, smp[i].seamless), dn = max_chan_diff(got, smp[i].clamp);
		const bool ok_s = ds <= CUBE_TOL, ok_n = dn <= CUBE_TOL;

		wrong += !ok_s;
		if (strcmp(smp[i].kind, "centre")) {
			n_edge++;
			as_seamless += ok_s;
			as_clamp += ok_n;
			off_edge += !ok_s && !ok_n;
		}
		/* printk has no floats here: the direction in 1/10000 */
		printk("C1 sample %u face %u %s dir/10000 (%d %d %d): got 0x%08x, seamless 0x%08x (diff %u), "
		       "non-seamless 0x%08x (diff %u)%s\n",
		       i, smp[i].face, smp[i].kind, (int)(smp[i].dir[0] * 10000.0f),
		       (int)(smp[i].dir[1] * 10000.0f), (int)(smp[i].dir[2] * 10000.0f), got, smp[i].seamless,
		       ds, smp[i].clamp, dn, ok_s ? "" : " WRONG");
	}
	uint32_t guard = guard_bytes_changed();
	const char *seamless = as_seamless == n_edge ? "yes" : as_clamp == n_edge ? "no" : "neither";
	printk("C1 read-back: %u of %u samples off the seamless rule by more than %u; of %u edge and corner "
	       "samples %u match seamless, %u non-seamless, %u neither; guard rows %u bytes changed\n",
	       wrong, n, CUBE_TOL, n_edge, as_seamless, as_clamp, off_edge, guard);
	cube_verdict(guard ? "neither" : seamless, wrong + (guard ? 1u : 0u), n);
	return true;
}

int main(void)
{
	struct v3d_clock_ops clk = { .set = clock_set, .ctx = NULL };
	static struct prog t1 = {
		.name = "tex2d",
		.code = { rpi5_tex_tex2d_fs_code, rpi5_tex_tex2d_vs_code, rpi5_tex_tex2d_cs_code },
		.words = { (uint32_t)(sizeof(rpi5_tex_tex2d_fs_code) / 8u), (uint32_t)(sizeof(rpi5_tex_tex2d_vs_code) / 8u),
			   (uint32_t)(sizeof(rpi5_tex_tex2d_cs_code) / 8u) },
	};
	static struct prog c1 = {
		.name = "cube",
		.code = { rpi5_tex_cube_fs_code, rpi5_tex_cube_vs_code, rpi5_tex_cube_cs_code },
		.words = { (uint32_t)(sizeof(rpi5_tex_cube_fs_code) / 8u), (uint32_t)(sizeof(rpi5_tex_cube_vs_code) / 8u),
			   (uint32_t)(sizeof(rpi5_tex_cube_cs_code) / 8u) },
	};
	const char *stop = NULL;
	uint32_t hz = 0;
	int rc;

	t1.fs = rpi5_tex_tex2d_fs;
	t1.vs = rpi5_tex_tex2d_vs;
	t1.cs = rpi5_tex_tex2d_cs;
	c1.fs = rpi5_tex_cube_fs;
	c1.vs = rpi5_tex_cube_vs;
	c1.cs = rpi5_tex_cube_cs;
	BUILD_ASSERT(RPI5_TEX_TEX2D_ATTRS == 2u && RPI5_TEX_CUBE_ATTRS == 2u);

	printk("rpi5_tex: build %s, shaders ours (our 7.1 online compiler, src/shaders_ours.h)\n", RPI5_TEX_BUILD);
	printk("rpi5_tex: cache: %s, data line %u bytes\n", CACHE_OPT,
	       (unsigned int)sys_cache_data_line_size_get());
	printk("rpi5_tex: %s, V3D 7.1, T1 2D RGBA8 %ux%u NEAREST, C1 cube %ux%u LINEAR, target %ux%u raster "
	       "RGBA8888, watchdog armed at boot\n",
	       CONFIG_BOARD, TEX_W, TEX_H, CUBE_SIZE, CUBE_SIZE, TGT_W, TGT_H);

	/* M1 */
	rc = rpi_mbox_init();
	printk("M1 mailbox: %d\n", rc);
	if (rc) {
		stop = "M1 mailbox";
		goto out;
	}
	device_map(&r.hub, BCM2712_V3D_HUB_BASE, 0x4000, K_MEM_CACHE_NONE);
	device_map(&r.core, BCM2712_V3D_CORE0_BASE, 0x6000, K_MEM_CACHE_NONE);
	device_map(&r.pm, BCM2712_PM_BASE, 0x1000, K_MEM_CACHE_NONE);
	device_map(&r.sms, BCM2712_V3D_SMS_BASE, 0x700, K_MEM_CACHE_NONE);
	rc = v3d_power_on(&r, &clk);
	printk("M1 power_on: %d, PM_GRAFX 0x%08x\n", rc, sys_read32(r.pm + BCM2712_PM_GRAFX));
	if (rc) {
		stop = "M1 power-on";
		goto out;
	}
	rc = rpi_clock_rate(RPI_FW_CLK_V3D, &hz);
	printk("M1 V3D clock: rate %d (%u Hz)\n", rc, hz);
	rc = v3d_identify(&r, &id);
	printk("M1 identity: HUB_IDENT0..3 0x%08x 0x%08x 0x%08x 0x%08x\n", id.hub_ident[0], id.hub_ident[1],
	       id.hub_ident[2], id.hub_ident[3]);
	printk("M1 identity: CTL_IDENT0..2 0x%08x 0x%08x 0x%08x MMU_DEBUG_INFO 0x%08x; V3D %u.%u: %d\n",
	       id.ctl_ident[0], id.ctl_ident[1], id.ctl_ident[2], id.mmu_debug_info, id.ver / 10, id.ver % 10,
	       rc);
	if (rc) {
		stop = "M1 identity";
		goto out;
	}

	/* M2 */
	memset(pt, 0, sizeof(pt));
	memset(scratch, 0, sizeof(scratch));
	for (int i = 0; i < 3; i++)
		va_cl[i] = gpu_map(cls[i], CL_BYTES);
	va_code = gpu_map(code, sizeof(code));
	va_vtx = gpu_map(vtx, sizeof(vtx));
	va_ta = gpu_map(tile_alloc, TILE_ALLOC_BYTES);
	va_ts = gpu_map(tile_state, TILE_STATE_BYTES);
	va_ov = gpu_map(overflow, OVERFLOW_BYTES);
	va_tgt = gpu_map(target, TGT_BYTES);
	va_tex = gpu_map(tex, TEX_BYTES);
	va_cube = gpu_map(cube, CUBE_BYTES);
	va_recs = gpu_map(recs, sizeof(recs));
	if (!va_cl[0] || !va_cl[1] || !va_cl[2] || !va_code || !va_vtx || !va_ta || !va_ts || !va_ov ||
	    !va_tgt || !va_tex || !va_cube || !va_recs) {
		stop = "M2 page table";
		goto out;
	}
	uint64_t pt_pa = k_mem_phys_addr(pt), scratch_pa = k_mem_phys_addr(scratch);
	printk("M2 page table: VA 0x%08x..0x%08x mapped; texture 0x%08x, cube 0x%08x, records 0x%08x\n",
	       VA_FIRST, va_next, va_tex, va_cube, va_recs);
	if (clean(pt, sizeof(pt), "page table") || clean(scratch, sizeof(scratch), "scratch page")) {
		stop = "M2 cache clean";
		goto out;
	}
	v3d_irq_quiesce(&r);
	v3d_caches_init(&r);
	rc = v3d_mmu_init(&r, pt_pa, scratch_pa);
	printk("M2 mmu_init: %d\n", rc);
	if (rc) {
		stop = "M2 MMU";
		goto out;
	}

	/* T1, then C1; a GPU stop in T1 ends the run before C1 (the GPU may
	 * still be busy) */
	if (!t1_run(&t1)) {
		stop = why;
		goto out;
	}
	if (!c1_run(&c1)) {
		stop = why;
		goto out;
	}

out:
	/* both verdict lines on every path; a part that did not run counts
	 * every px or sample wrong */
	tex_verdict(TGT_W * TGT_H, TGT_W * TGT_H);
	cube_verdict("neither", C1_SAMPLES, C1_SAMPLES);
	if (stop)
		printk("rpi5_tex: FAIL (%s)\n", stop);
	else
		printk("rpi5_tex: done (both parts ran)\n");
	/* nothing more: the watchdog resets the SoC, tryboot brings Linux back */
	for (;;)
		k_sleep(K_SECONDS(10));
	return 0;
}
