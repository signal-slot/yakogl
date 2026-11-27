/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rpi5_cube_ours: the first Pi 5 cube-map case (roadmap P10, pre-study U8,
 * acceptance c7 of plan #384) through the driver's V3D 7.1 job builder
 * (v3d_job71.c: the TMU config P0/P1 uniforms written by the builder from
 * the sampler binding, not by the probe), the 7.1 texture records
 * (v3d_tex71.c) and the 7.1 TFU fill (v3d_tfu71.c, IOC). The program is our
 * 7.1 online compiler's (src/shaders_ours.h, gen/gen_shaders.sh).
 *
 *   M1  as rpi5_tex: the V3D clock, power-on, identity; the core's IDENT1
 *       VPM_SIZE field (open from vpmsrc71) and the hub's IDENT1 word;
 *   M2  one page table over the probe's statics, the MMU on;
 *   C1  a 4x4 RGBA8 cube map of gl_cube's shape (six faces as layers, one
 *       colour per face), sampled LINEAR with wrap CLAMP in S, T and R
 *       (st/mesa's seamless rule, Mesa's tools/v3d/pi5/gl_cube.clif). One
 *       job, one draw of 54 quads (two triangles each): each quad fills one
 *       8x8 block of the 64x64 target and carries one direction at all six
 *       vertices: a face centre, a point a quarter texel inside each of the
 *       face's four edges and an eighth inside each of its four corners.
 *       Every pixel of every block is compared with the seamless prediction
 *       (the texel beyond the edge taken from the neighbouring face, a
 *       corner's missing texel the mean of the three faces); each block's
 *       centre also with the non-seamless one (the face clamped to its own
 *       edge texels). The 10 blocks no quad covers must keep the clear.
 *   C2  a second 4x4 RGBA8 cube map with two levels (Mesa's layout), level 0
 *       a per-face texel pattern; one 7.1 TFU job per face (v3d_tfu71_pack:
 *       in place, NUMMM 1, DIMTW; IOC written), as Mesa's glGenerateMipmap
 *       does per layer. Level 0 must be unchanged and level 1 within 2 per
 *       channel of the 2x2 box of level 0.
 *
 * The console then has exactly these verdict lines, in this order:
 *
 *   v3d_cube71 face <f> <name>: PASS|FAIL <n> of <m> px wrong   (six lines)
 *   v3d_cube71: PASS seamless=yes <n> of <m> px wrong
 *   | v3d_cube71: FAIL seamless=<yes|no|neither> <n> of <m> px wrong
 *   v3d_cube71_tfu: PASS|FAIL <n> of <m> px wrong (<what>)
 *   rpi5_cube_ours: done (C1 and C2 ran) | rpi5_cube_ours: FAIL (<step>)
 *
 * and, once the identity was read, one line before them:
 *
 *   rpi5_cube_ours: IDENT1 VPM_SIZE <v> (<v * 8192> bytes) CTL_IDENT1 0x... HUB_IDENT1 0x...
 *
 * A step that stops the probe before a part ran prints that part's lines as
 * FAIL with every px counted wrong (seamless=neither); the end line then
 * names the step. Nothing runs afterwards: the board layer's PM watchdog
 * (armed in the first PRE_KERNEL_1 hook) resets the SoC and tryboot brings
 * Linux back.
 *
 * Every buffer the GPU reads or writes is a static written through the CPU's
 * cache: cleaned and invalidated before the GPU sees it, invalidated again
 * before the CPU reads it. A cache call that fails ends the run.
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
#include "v3d_job71.h"
#include "v3d_tex.h"
#include "v3d_tex71.h"
#include "v3d_tfu71.h"
#include "rpi_mbox.h"
#include "shaders_ours.h"

#if V3D_CL_VER != 71
#error "rpi5_cube_ours: main.c is built at V3D_CL_VER 71 (CMakeLists.txt)"
#endif

#define PAGE 4096u

/* The GPU's view, as rpi5_tex: one page table from VA 0, VA 0 unmapped. */
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

/* C1: 4x4 RGBA8 faces, six layers, one level (gl_cube's record) */
#define CUBE_SIZE  4u
#define CUBE_BYTES PAGE
#define CUBE_TOL   8u
/* the fraction of the look-up's footprint beyond the face: an edge sample
 * a quarter texel, a corner sample an eighth (in each direction) */
#define EDGE_F      0.25f
#define CORNER_F    0.125f
#define BLOCK       8u /* px: one C1 sample per 8x8 block, 8x8 blocks */
#define BLOCKS_X    (TGT_W / BLOCK)
#define MAX_SAMPLES (BLOCKS_X * (TGT_H / BLOCK))
#define PER_FACE    9u                /* centre, 4 edges, 4 corners */
#define C1_SAMPLES  (6u * PER_FACE)   /* 54 */
#define FACE_PX     (PER_FACE * BLOCK * BLOCK)
#define C1_PX       (6u * FACE_PX)

/* C2: 4x4 RGBA8 faces, six layers, two levels (Mesa's layout: level 0 at
 * the top of a 4160-byte layer, about 25 KiB for six) */
#define MIP_LEVELS 2u
#define MIP_BYTES  (32u * 1024u)
#define MIP_TOL    2u
#define MIP_W1     (CUBE_SIZE / 2u)
#define C2_PX      (6u * MIP_W1 * MIP_W1)
#define TFU_WAIT_US 1000000u
#define POLL_US     10u

/* the words of a generated code array, counted as rpi5_tex counts them (Zephyr's ARRAY_SIZE
 * does not compile on them in this unit) */
#define CODE_WORDS(a) ((uint32_t)(sizeof(a) / sizeof((a)[0])))

#define CL_BYTES         PAGE
#define CODE_SLOT        0x200u /* FS, VS, CS each at a 512-byte slot */
#define TILE_ALLOC_BYTES (64u * 1024u)
#define TILE_STATE_BYTES (64u * 1024u)
#define OVERFLOW_BYTES   (256u * 1024u)
#define VTX_BYTES        (16u * 1024u)
#define WAIT_US          1000000u

/* Mesa's own gl_cube records for its LINEAR jobs (tools/v3d/pi5/README.md,
 * the texture record table): texture state words 1..3 (word 0, 4 and 5 hold
 * the base address) and sampler words 0..1. For the comparison line only. */
static const uint32_t MESA_CUBE_STATE_W1_3[3] = { 0x10000002u, 0x01800400u, 0x00b1a040u };
static const uint32_t MESA_CUBE_SAMPLER[2] = { 0x00100004u, 0x00490000u };

/* the six face colours (+X, -X, +Y, -Y, +Z, -Z; R in the low byte): any two
 * faces that share an edge differ by 255 in at least one channel */
static const uint32_t FACE_RGBA[6] = {
	0xff0000ffu, 0xffffff00u, 0xff00ff00u, 0xffff00ffu, 0xffff0000u, 0xff00ffffu,
};
static const char *const FACE_NAME[6] = { "+X", "-X", "+Y", "-Y", "+Z", "-Z" };

static uint32_t pt[PT_ENTRIES] __aligned(PAGE);
static uint8_t scratch[PAGE] __aligned(PAGE);
static uint8_t cls[3][CL_BYTES] __aligned(PAGE); /* binning, render, indirect */
static uint8_t code[PAGE] __aligned(PAGE);
static float vtx[VTX_BYTES / 4] __aligned(PAGE);
static uint8_t tile_alloc[TILE_ALLOC_BYTES] __aligned(PAGE);
static uint8_t tile_state[TILE_STATE_BYTES] __aligned(PAGE);
static uint8_t overflow[OVERFLOW_BYTES] __aligned(PAGE);
static uint32_t target[TGT_BYTES / 4] __aligned(PAGE);
static uint8_t cube[CUBE_BYTES] __aligned(PAGE);
static uint8_t mip[MIP_BYTES] __aligned(PAGE);
/* C1's texture record at 0x00, its sampler at 0x20 (32-byte aligned,
 * V3D_TEX71_RECORD_ALIGN) */
static uint8_t recs[PAGE] __aligned(PAGE);

static uint32_t va_next = VA_FIRST;
static struct v3d_regs r = { .soc = V3D_SOC_BCM2712 };
static struct v3d_identity id;
static uint32_t va_cl[3], va_code, va_vtx, va_ta, va_ts, va_ov, va_tgt, va_cube, va_mip, va_recs;
static char why[200];

#define CACHE_OPT                                                                                  \
	"CONFIG_CACHE_MANAGEMENT=" STRINGIFY(IS_ENABLED(CONFIG_CACHE_MANAGEMENT)) " CONFIG_DCACHE="  \
	STRINGIFY(IS_ENABLED(CONFIG_DCACHE)) " sys_cache_data_flush_and_invd_range"

/* ---- the verdict lines: each printed exactly once ---- */

static bool cube_said, tfu_said;

/* The six face lines and the C1 line. PASS only for seamless=yes with no
 * px wrong; a face passes with no px wrong. */
static void cube_verdict(const uint32_t face_wrong[6], const char *seamless, uint32_t wrong)
{
	if (cube_said)
		return;
	cube_said = true;
	for (int f = 0; f < 6; f++)
		printk("v3d_cube71 face %d %s: %s %u of %u px wrong\n", f, FACE_NAME[f],
		       face_wrong[f] ? "FAIL" : "PASS", face_wrong[f], FACE_PX);
	bool pass = !wrong && !strcmp(seamless, "yes");
	printk("v3d_cube71: %s seamless=%s %u of %u px wrong\n", pass ? "PASS" : "FAIL", seamless, wrong, C1_PX);
}

static void tfu_verdict(uint32_t wrong, const char *what)
{
	if (tfu_said)
		return;
	tfu_said = true;
	printk("v3d_cube71_tfu: %s %u of %u px wrong (%s)\n", wrong ? "FAIL" : "PASS", wrong, C2_PX, what);
}

/* ---- helpers, as rpi5_tex ---- */

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

static uint32_t hub_rd(uint32_t off)
{
	return sys_read32(r.hub + off);
}

static void hub_wr(uint32_t off, uint32_t v)
{
	sys_write32(v, r.hub + off);
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

/* One quad (two triangles, six vertices; the job culls nothing) over the
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

/* ---- the samples and their predictions (rpi5_tex's C1) ---- */

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

/* PER_FACE samples per face, face after face (sample i is face i / 9) */
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

/* ---- C1: the seamless cube through v3d_job71 ---- */

static struct v3d_fs_prog fs_prog;
static struct v3d_vs_prog vs_prog, cs_prog;

/* Place the code in the code page; the uniform streams stay the compiler's
 * (the TMU config words are v3d_job71's to fill from the sampler binding). */
static bool prog_place(void)
{
	const struct {
		const uint64_t *w;
		uint32_t n;
	} c[3] = {
		{ rpi5_cube_cube_fs_code, CODE_WORDS(rpi5_cube_cube_fs_code) },
		{ rpi5_cube_cube_vs_code, CODE_WORDS(rpi5_cube_cube_vs_code) },
		{ rpi5_cube_cube_cs_code, CODE_WORDS(rpi5_cube_cube_cs_code) },
	};

	memset(code, 0, sizeof(code));
	for (int s = 0; s < 3; s++) {
		if (c[s].n * 8u > CODE_SLOT) {
			printk("cube: stage %d has %u instructions, more than its slot\n", s, c[s].n);
			return false;
		}
		for (uint32_t i = 0; i < c[s].n; i++)
			for (int b = 0; b < 8; b++)
				code[s * CODE_SLOT + i * 8u + b] = (uint8_t)(c[s].w[i] >> (8 * b));
	}
	fs_prog = rpi5_cube_cube_fs;
	vs_prog = rpi5_cube_cube_vs;
	cs_prog = rpi5_cube_cube_cs;
	fs_prog.code = va_code;
	vs_prog.code = va_code + CODE_SLOT;
	cs_prog.code = va_code + 2u * CODE_SLOT;
	return true;
}

static bool c1_records(struct v3d_tex_layout *l, uint32_t *stride)
{
	if (!v3d_tex_layout(CUBE_SIZE, CUBE_SIZE, 1, 4, l)) {
		snprintk(why, sizeof(why), "C1 layout");
		return false;
	}
	*stride = v3d_tex_layer_stride(l);
	if (*stride * 6u > CUBE_BYTES) {
		snprintk(why, sizeof(why), "C1 layout: layer stride %u", *stride);
		return false;
	}
	memset(cube, GUARD_BYTE, sizeof(cube));
	for (int f = 0; f < 6; f++) {
		uint32_t face[CUBE_SIZE * CUBE_SIZE];

		for (uint32_t i = 0; i < CUBE_SIZE * CUBE_SIZE; i++)
			face[i] = FACE_RGBA[f];
		v3d_tex_store_level(cube + (uint32_t)f * *stride, l, 0, (const uint8_t *)face, CUBE_SIZE * 4u);
	}

	struct v3d_tex71_state s = { 0 };
	v3d_tex71_state_from_layout(&s, l);
	s.base = va_cube + l->slices[0].offset;
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
	memset(recs, 0, sizeof(recs));
	if (!v3d_tex71_pack_state(recs + 0x00, &s) || !v3d_tex71_pack_sampler(recs + 0x20, &sm)) {
		snprintk(why, sizeof(why), "C1 v3d_tex71 refused the records");
		return false;
	}
	bool st_same = true, sm_same = true;
	for (int w = 0; w < 3; w++)
		st_same &= rd32le(recs + 4 * (w + 1)) == MESA_CUBE_STATE_W1_3[w];
	for (int w = 0; w < 2; w++)
		sm_same &= rd32le(recs + 0x20 + 4 * w) == MESA_CUBE_SAMPLER[w];
	printk("C1 cube: 6 faces %ux%u RGBA8, layer stride %u, level 0 at +0x%x; state 0x%08x 0x%08x 0x%08x "
	       "0x%08x 0x%08x 0x%08x (words 1-3 %s Mesa's gl_cube); sampler 0x%08x 0x%08x (%s Mesa's "
	       "gl_cube LINEAR)\n",
	       CUBE_SIZE, CUBE_SIZE, *stride, l->slices[0].offset, rd32le(recs), rd32le(recs + 4),
	       rd32le(recs + 8), rd32le(recs + 12), rd32le(recs + 16), rd32le(recs + 20),
	       st_same ? "equal" : "differ from", rd32le(recs + 0x20), rd32le(recs + 0x24),
	       sm_same ? "equal" : "differ from");
	return true;
}

/* Build the job (one draw), bin and render it, make the target readable. */
static bool c1_job(uint32_t n)
{
	struct v3d_stall st;
	int rc;

	/* one 8x8 block per sample: pos at 0, the direction (vec4, w 0) after */
	const uint32_t pos_bytes = n * 6u * 16u;
	const struct v3d_vertex_attr attrs[2] = {
		{ .address = va_vtx, .stride = 16, .components = 4 },
		{ .address = va_vtx + pos_bytes, .stride = 16, .components = 4 },
	};
	struct v3d_job71_state js = { 0 };
	/* glViewport(0, 0, 64, 64), not flipped: window y is the buffer's row */
	v3d_viewport_gl(0, 0, (float)TGT_W, (float)TGT_H, 0, 1, false, TGT_H, &js.vp);
	js.front_ccw = false;
	js.line_width = 1.0f;
	js.depth_func = V3D_COMPARE_FUNC_ALWAYS;
	js.colormask = 0xf;
	js.sample_mask = 0xf;
	/* sampler binding 0 (the FS's P0/P1 data bits 24..31): the records */
	const struct v3d_job71_tmu tmu = {
		.unit = 0,
		.tex_state = va_recs + 0x00,
		.sampler = va_recs + 0x20,
		.width = CUBE_SIZE,
		.height = CUBE_SIZE,
		.depth = 1,
		.array_size = 1,
	};
	const struct v3d_job71_draw d = {
		.st = &js,
		.fs = &fs_prog,
		.vs = &vs_prog,
		.cs = &cs_prog,
		.attrs = attrs,
		.num_attrs = 2,
		.prim = V3D_PRIM_TRIANGLES,
		.count = n * 6u,
		.tmu = &tmu,
		.num_tmu = 1,
	};
	struct v3d_job71_target t = {
		.width = TGT_W,
		.height = TGT_H,
		.tile_alloc = va_ta,
		.rts = 1,
		.depth_type = V3D_INTERNAL_TYPE_DEPTH_32F, /* no zsbuf */
	};
	t.rt[0] = (struct v3d_job71_rt){
		.address = va_tgt + TGT_STRIDE, /* row 0 of the buffer is a guard row */
		.format = V3D_OUTPUT_IMAGE_FORMAT_RGBA8,
		.memory_format = V3D_MEMORY_FORMAT_RASTER,
		.height_in_ub_or_stride = TGT_STRIDE,
	};
	struct v3d_job71_clear c = { 0 };
	c.color[0][0] = CLEAR;
	c.z = 1.0f;

	struct v3d_cl bcl, rcl, ind;
	struct v3d_job71 j;
	struct v3d_job71_draw_layout lo = { 0 };
	uint32_t ls = 0, le = 0;
	memset(cls, 0, sizeof(cls));
	v3d_cl_init(&bcl, cls[0], va_cl[0], CL_BYTES);
	v3d_cl_init(&rcl, cls[1], va_cl[1], CL_BYTES);
	v3d_cl_init(&ind, cls[2], va_cl[2], CL_BYTES);
	if (!v3d_job71_begin(&j, &t, &bcl, &rcl, &ind)) {
		snprintk(why, sizeof(why), "C1 v3d_job71_begin refused the target");
		return false;
	}
	rc = v3d_job71_draw(&j, &d, &lo);
	if (rc != V3D_JOB71_OK) {
		snprintk(why, sizeof(why), "C1 v3d_job71_draw %s (BCL %u, indirect %u bytes)",
			 rc == V3D_JOB71_FULL ? "full" : "refused", bcl.used, ind.used);
		return false;
	}
	if (!v3d_job71_end(&j, &c, &ls, &le)) {
		snprintk(why, sizeof(why), "C1 v3d_job71_end refused (BCL %u, RCL %u, indirect %u bytes)",
			 bcl.used, rcl.used, ind.used);
		return false;
	}
	uint32_t ta_size, ts_size;
	v3d_job71_alloc_sizes(&j, PAGE, &ta_size, &ts_size);
	if (ta_size > TILE_ALLOC_BYTES || ts_size > TILE_STATE_BYTES) {
		snprintk(why, sizeof(why), "C1 job sizes: tile allocation %u, tile state %u", ta_size, ts_size);
		return false;
	}

	/* the TMU config words v3d_job71 wrote, against v3d_tex.h's P0/P1 of
	 * the same records (the compiler's data, binding 0) */
	const uint8_t *fsu = cls[2] + (lo.fs_uniforms - va_cl[2]);
	for (uint32_t i = 0; i < fs_prog.num_uniforms; i++) {
		const struct v3d_uniform u = fs_prog.uniforms[i];
		uint32_t want = 0;
		bool have = false;

		if (u.contents == V3D_QUNIFORM_TMU_CONFIG_P0)
			have = v3d_tex_p0(tmu.tex_state, u.data & 0xffffffu, &want);
		else if (u.contents == V3D_QUNIFORM_TMU_CONFIG_P1)
			have = v3d_tex_p1(tmu.sampler, u.data & 0xffffffu, &want);
		else
			continue;
		printk("C1 FS uniform %u: TMU config P%u 0x%08x (%s v3d_tex_p%u 0x%08x)\n", i,
		       u.contents - V3D_QUNIFORM_TMU_CONFIG_P0, rd32le(fsu + 4u * i),
		       have && rd32le(fsu + 4u * i) == want ? "equals" : "differs from",
		       u.contents - V3D_QUNIFORM_TMU_CONFIG_P0, want);
	}
	printk("C1 job: %u samples, %u vertices, tile %ux%u, BCL %u / RCL %u / indirect %u bytes, record 0x%08x, "
	       "tile list 0x%08x..0x%08x, tile alloc %u, tile state %u, code FS 0x%08x VS 0x%08x CS 0x%08x "
	       "(%u/%u/%u instructions)\n",
	       n, n * 6u, j.tiling.tile_w, j.tiling.tile_h, bcl.used, rcl.used, ind.used, lo.record, ls, le,
	       ta_size, ts_size, fs_prog.code, vs_prog.code, cs_prog.code,
	       CODE_WORDS(rpi5_cube_cube_fs_code), CODE_WORDS(rpi5_cube_cube_vs_code),
	       CODE_WORDS(rpi5_cube_cube_cs_code));

	memset(target, GUARD_BYTE, sizeof(target));
	for (uint32_t i = 0; i < TGT_W * TGT_H; i++)
		target[TGT_W + i] = SENTINEL;
	memset(tile_state, 0, sizeof(tile_state));
	memset(overflow, 0, sizeof(overflow));
	if (clean(cls, sizeof(cls), "lists") || clean(code, sizeof(code), "code") ||
	    clean(vtx, ROUND_UP(2u * pos_bytes, 64u), "vertices") ||
	    clean(tile_alloc, sizeof(tile_alloc), "tile allocation") ||
	    clean(tile_state, sizeof(tile_state), "tile state") ||
	    clean(overflow, sizeof(overflow), "overflow block") || clean(target, sizeof(target), "target") ||
	    clean(cube, sizeof(cube), "cube map") || clean(recs, sizeof(recs), "records")) {
		snprintk(why, sizeof(why), "C1 cache clean");
		return false;
	}
	v3d_caches_invalidate(&r);

	uint32_t ov_blocks[1] = { va_ov };
	struct v3d_bin_overflow ov = { .blocks = ov_blocks, .nblocks = 1, .block_size = OVERFLOW_BYTES, .used = 0 };
	v3d_submit_bin(&r, bcl.gpu, bcl.gpu + bcl.used, va_ta, ta_size, va_ts);
	rc = v3d_wait_bin(&r, WAIT_US, &st, &ov);
	printk("C1 bin: %d, overflow blocks used %u\n", rc, ov.used);
	if (rc) {
		print_stall("C1", &st);
		snprintk(why, sizeof(why), "C1 bin: %s",
			 rc == -ENOMEM ? "out of memory" : rc == -EFAULT ? "MMU fault" : "timeout");
		return false;
	}
	v3d_submit_render(&r, rcl.gpu, rcl.gpu + rcl.used);
	rc = v3d_wait(&r, V3D_CTL_0_INT_STS_INT_FRDONE_SET, WAIT_US, &st);
	printk("C1 render: %d, ERR_STAT 0x%08x\n", rc, sys_read32(r.core + V3D_ERR_0_STAT));
	if (rc) {
		print_stall("C1", &st);
		snprintk(why, sizeof(why), "C1 render: %s", rc == -EFAULT ? "MMU fault" : "timeout");
		return false;
	}
	rc = v3d_caches_clean(&r);
	if (rc) {
		snprintk(why, sizeof(why), "C1 GPU cache clean: %d", rc);
		return false;
	}
	rc = sys_cache_data_invd_range(target, sizeof(target));
	if (rc) {
		snprintk(why, sizeof(why), "C1 target cache invalidate: %d", rc);
		return false;
	}
	return true;
}

static bool c1_run(void)
{
	static struct sample smp[MAX_SAMPLES];
	struct v3d_tex_layout l;
	uint32_t stride, n = c1_samples(smp);

	if (n != C1_SAMPLES || n > MAX_SAMPLES) {
		snprintk(why, sizeof(why), "C1 samples: %u", n);
		return false;
	}
	if (!c1_records(&l, &stride))
		return false;
	if (!prog_place()) {
		snprintk(why, sizeof(why), "C1 program");
		return false;
	}

	const uint32_t pos_bytes = n * 6u * 16u;
	if (2u * pos_bytes > sizeof(vtx)) {
		snprintk(why, sizeof(why), "C1 vertices: %u bytes", 2u * pos_bytes);
		return false;
	}
	memset(vtx, 0, sizeof(vtx));
	float *dir = vtx + pos_bytes / 4u;
	for (uint32_t i = 0; i < n; i++) {
		const uint32_t bx = i % BLOCKS_X, by = i / BLOCKS_X;

		quad_pos(vtx + i * 24u, bx * BLOCK, by * BLOCK, (bx + 1u) * BLOCK, (by + 1u) * BLOCK);
		for (int v = 0; v < 6; v++)
			for (int c = 0; c < 3; c++)
				dir[i * 24u + v * 4 + c] = smp[i].dir[c];
	}
	if (!c1_job(n))
		return false;

	uint32_t face_wrong[6] = { 0 }, wrong = 0, as_seamless = 0, as_clamp = 0, off_edge = 0, n_edge = 0;
	for (uint32_t i = 0; i < n; i++) {
		const uint32_t bx = i % BLOCKS_X, by = i / BLOCKS_X;
		const uint32_t got = pixel(bx * BLOCK + BLOCK / 2u, by * BLOCK + BLOCK / 2u);
		const uint32_t ds = max_chan_diff(got, smp[i].seamless), dn = max_chan_diff(got, smp[i].clamp);
		const bool ok_s = ds <= CUBE_TOL, ok_n = dn <= CUBE_TOL;
		uint32_t bad = 0, maxd = 0;

		for (uint32_t y = by * BLOCK; y < (by + 1u) * BLOCK; y++) {
			for (uint32_t x = bx * BLOCK; x < (bx + 1u) * BLOCK; x++) {
				const uint32_t d = max_chan_diff(pixel(x, y), smp[i].seamless);

				maxd = MAX(maxd, d);
				bad += d > CUBE_TOL;
			}
		}
		face_wrong[smp[i].face] += bad;
		wrong += bad;
		if (strcmp(smp[i].kind, "centre")) {
			n_edge++;
			as_seamless += ok_s;
			as_clamp += ok_n;
			off_edge += !ok_s && !ok_n;
		}
		/* printk has no floats here: the direction in 1/10000 */
		printk("C1 sample %u face %u %s dir/10000 (%d %d %d): centre 0x%08x, seamless 0x%08x (diff %u), "
		       "non-seamless 0x%08x (diff %u); block %u of %u px off seamless (max %u)%s\n",
		       i, smp[i].face, smp[i].kind, (int)(smp[i].dir[0] * 10000.0f),
		       (int)(smp[i].dir[1] * 10000.0f), (int)(smp[i].dir[2] * 10000.0f), got, smp[i].seamless,
		       ds, smp[i].clamp, dn, bad, BLOCK * BLOCK, maxd, bad ? " WRONG" : "");
	}
	/* the blocks no quad covers keep the clear */
	uint32_t stray = 0;
	for (uint32_t i = n; i < MAX_SAMPLES; i++) {
		const uint32_t bx = i % BLOCKS_X, by = i / BLOCKS_X;

		for (uint32_t y = by * BLOCK; y < (by + 1u) * BLOCK; y++)
			for (uint32_t x = bx * BLOCK; x < (bx + 1u) * BLOCK; x++)
				stray += pixel(x, y) != CLEAR;
	}
	const uint32_t guard = guard_bytes_changed();
	const char *seamless = as_seamless == n_edge ? "yes" : as_clamp == n_edge ? "no" : "neither";
	printk("C1 read-back: %u of %u px off the seamless rule by more than %u; of %u edge and corner "
	       "samples %u match seamless, %u non-seamless, %u neither; %u of %u px outside the samples not "
	       "the clear; guard rows %u bytes changed\n",
	       wrong, C1_PX, CUBE_TOL, n_edge, as_seamless, as_clamp, off_edge, stray,
	       (MAX_SAMPLES - n) * BLOCK * BLOCK, guard);
	/* a stray px or a changed guard row makes the answer neither and
	 * counts one wrong px each kind */
	const bool spill = stray || guard;
	cube_verdict(face_wrong, spill ? "neither" : seamless, wrong + (stray ? 1u : 0u) + (guard ? 1u : 0u));
	return true;
}

/* ---- C2: level 1 of a cube by the 7.1 TFU, one job per face ---- */

/* level 0 texel (x, y) of face f: every channel moves with x, y or f */
static uint32_t mip_texel(uint32_t f, uint32_t x, uint32_t y)
{
	return (x * 64u + 16u) | (y * 64u + 16u) << 8 | (f * 40u + 20u) << 16 | (0xffu - x * 16u - y * 4u) << 24;
}

/* v3d_tex_tiling and v3d_tfu71_tiling are both Mesa's enum v3d_tiling_mode */
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

/* One TFU job, written as rpi5_tfu writes it (driver/docs/rpi5-v3d71-facts.md
 * section 7: ICFG last, with its IOC bit; the driver's v3d_tfu_run is the
 * 4.2 block's). False with `why` set on a fault or a timeout. */
static bool tfu_kick(uint32_t f, const struct v3d_tfu71_job *j)
{
	const uint32_t hub_mmu = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET |
				 V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;
	const uint32_t cs0 = hub_rd(V3D71_TFU_CS);

	if (cs0 == 0xffffffffu || (cs0 & V3D_TFU_CS_BUSY_SET)) {
		snprintk(why, sizeof(why), "C2 face %u: TFU CS 0x%08x before the job", f, cs0);
		return false;
	}
	hub_wr(V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET);
	hub_wr(V3D71_TFU_IIA, j->iia);
	hub_wr(V3D71_TFU_IIS, j->iis);
	hub_wr(V3D71_TFU_ICA, j->ica);
	hub_wr(V3D71_TFU_IUA, j->iua);
	hub_wr(V3D71_TFU_IOA, j->ioa);
	hub_wr(V3D71_TFU_IOC, j->ioc);
	hub_wr(V3D71_TFU_IOS, j->ios);
	hub_wr(V3D71_TFU_COEF0, j->coef[0]);
	hub_wr(V3D71_TFU_COEF1, j->coef[1]);
	hub_wr(V3D71_TFU_COEF2, j->coef[2]);
	hub_wr(V3D71_TFU_COEF3, j->coef[3]);
	hub_wr(V3D71_TFU_ICFG, j->icfg | V3D_TFU_ICFG_IOC_SET);

	uint32_t sts = 0, cs = cs0, t = 0;
	const char *by = NULL;
	for (;; t += POLL_US) {
		sts = hub_rd(V3D_HUB_CTL_INT_STS);
		cs = hub_rd(V3D71_TFU_CS);
		if (sts & hub_mmu) {
			snprintk(why, sizeof(why), "C2 face %u: TFU MMU fault: hub INT_STS 0x%08x VIO_ADDR 0x%08x", f,
				 sts, hub_rd(V3D_MMU0_VIO_ADDR));
			return false;
		}
		if (sts & V3D_HUB_CTL_INT_STS_INT_TFUC_SET) {
			by = "TFUC";
			break;
		}
		if (V3D_TFU_CS_CVTCT_GET(cs) != V3D_TFU_CS_CVTCT_GET(cs0) && !(cs & V3D_TFU_CS_BUSY_SET)) {
			by = "CS";
			break;
		}
		if (t >= TFU_WAIT_US)
			break;
		k_busy_wait(POLL_US);
	}
	printk("C2 face %u TFU: ICFG 0x%08x IIA 0x%08x IIS 0x%08x IOA 0x%08x IOS 0x%08x IOC 0x%08x; %s after %u us, "
	       "CS 0x%08x (CVTCT %u -> %u)\n",
	       f, j->icfg, j->iia, j->iis, j->ioa, j->ios, j->ioc, by ? by : "nothing", t, cs,
	       V3D_TFU_CS_CVTCT_GET(cs0), V3D_TFU_CS_CVTCT_GET(cs));
	hub_wr(V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET);
	if (!by) {
		snprintk(why, sizeof(why), "C2 face %u: TFU timeout, CS 0x%08x SU 0x%08x hub INT_STS 0x%08x", f, cs,
			 hub_rd(V3D71_TFU_SU), sts);
		return false;
	}
	return true;
}

static bool c2_run(void)
{
	struct v3d_tex_layout l;
	uint32_t lv0[CUBE_SIZE * CUBE_SIZE], got[CUBE_SIZE * CUBE_SIZE];

	if (!v3d_tex_layout(CUBE_SIZE, CUBE_SIZE, MIP_LEVELS, 4, &l)) {
		snprintk(why, sizeof(why), "C2 layout");
		return false;
	}
	const uint32_t stride = v3d_tex_layer_stride(&l);
	if (stride * 6u > MIP_BYTES) {
		snprintk(why, sizeof(why), "C2 layout: layer stride %u", stride);
		return false;
	}
	printk("C2 cube: 6 faces %ux%u RGBA8, %u levels, layer stride %u; level 0 at +0x%x (tiling %d), level 1 "
	       "at +0x%x (tiling %d)\n",
	       CUBE_SIZE, CUBE_SIZE, MIP_LEVELS, stride, l.slices[0].offset, l.slices[0].tiling,
	       l.slices[1].offset, l.slices[1].tiling);
	memset(mip, GUARD_BYTE, sizeof(mip));
	for (uint32_t f = 0; f < 6u; f++) {
		for (uint32_t y = 0; y < CUBE_SIZE; y++)
			for (uint32_t x = 0; x < CUBE_SIZE; x++)
				lv0[y * CUBE_SIZE + x] = mip_texel(f, x, y);
		v3d_tex_store_level(mip + f * stride, &l, 0, (const uint8_t *)lv0, CUBE_SIZE * 4u);
	}
	if (clean(mip, sizeof(mip), "mip cube")) {
		snprintk(why, sizeof(why), "C2 cache clean");
		return false;
	}
	v3d_caches_invalidate(&r);

	for (uint32_t f = 0; f < 6u; f++) {
		const uint32_t base = va_mip + f * stride + l.slices[0].offset;
		const struct v3d_tfu71_args a = {
			.ttype = V3D_TFU71_TTYPE_RGBA8,
			.cpp = 4,
			.width = CUBE_SIZE,
			.height = CUBE_SIZE,
			.num_mm = MIP_LEVELS - 1u,
			/* in place, as Mesa's glGenerateMipmap per layer */
			.src = { .addr = base, .tiling = tfu_tiling(l.slices[0].tiling),
				 .stride_or_padded_height = l.slices[0].padded_height },
			.dst = { .addr = base, .tiling = tfu_tiling(l.slices[0].tiling),
				 .stride_or_padded_height = l.slices[0].padded_height },
		};
		struct v3d_tfu71_job j;

		if (!v3d_tfu71_pack(&a, &j)) {
			snprintk(why, sizeof(why), "C2 face %u: v3d_tfu71_pack refused the job", f);
			return false;
		}
		if (!tfu_kick(f, &j))
			return false;
	}
	int rc = v3d_caches_clean(&r);
	if (rc) {
		snprintk(why, sizeof(why), "C2 GPU cache clean: %d", rc);
		return false;
	}
	rc = sys_cache_data_invd_range(mip, sizeof(mip));
	if (rc) {
		snprintk(why, sizeof(why), "C2 mip cube cache invalidate: %d", rc);
		return false;
	}

	uint32_t l0_bad = 0, l1_bad = 0, l1_max = 0, untouched = 0;
	for (uint32_t f = 0; f < 6u; f++) {
		uint32_t fb = 0;

		if (!v3d_tex_load_box(mip + f * stride, &l, 0, 0, 0, CUBE_SIZE, CUBE_SIZE, (uint8_t *)got,
				      CUBE_SIZE * 4u)) {
			snprintk(why, sizeof(why), "C2 level 0 untile refused");
			return false;
		}
		for (uint32_t y = 0; y < CUBE_SIZE; y++)
			for (uint32_t x = 0; x < CUBE_SIZE; x++)
				l0_bad += got[y * CUBE_SIZE + x] != mip_texel(f, x, y);
		if (!v3d_tex_load_box(mip + f * stride, &l, 1, 0, 0, MIP_W1, MIP_W1, (uint8_t *)got, MIP_W1 * 4u)) {
			snprintk(why, sizeof(why), "C2 level 1 untile refused");
			return false;
		}
		for (uint32_t y = 0; y < MIP_W1; y++) {
			for (uint32_t x = 0; x < MIP_W1; x++) {
				uint32_t want = 0;

				for (unsigned int c = 0; c < 4u; c++) {
					uint32_t s = chan(mip_texel(f, 2u * x, 2u * y), c) +
						     chan(mip_texel(f, 2u * x + 1u, 2u * y), c) +
						     chan(mip_texel(f, 2u * x, 2u * y + 1u), c) +
						     chan(mip_texel(f, 2u * x + 1u, 2u * y + 1u), c);
					want |= ((s + 2u) / 4u) << (8u * c);
				}
				const uint32_t g = got[y * MIP_W1 + x], d = max_chan_diff(g, want);

				untouched += g == 0xa5a5a5a5u;
				l1_max = MAX(l1_max, d);
				fb += d > MIP_TOL;
				if (x == 0 && y == 0)
					printk("C2 face %u level 1 (0,0): 0x%08x want 0x%08x\n", f, g, want);
			}
		}
		l1_bad += fb;
	}
	printk("C2 read-back: level 0 %u of %u px changed; level 1 %u of %u px off the 2x2 box by more than %u "
	       "(max %u), %u still the sentinel\n",
	       l0_bad, 6u * CUBE_SIZE * CUBE_SIZE, l1_bad, C2_PX, MIP_TOL, l1_max, untouched);
	snprintk(why, sizeof(why), "level 1 of 6 faces by 6 TFU jobs with IOC; level 0 %u px changed", l0_bad);
	/* a changed level 0 px counts one wrong px */
	tfu_verdict(l1_bad + (l0_bad ? 1u : 0u), why);
	why[0] = '\0';
	return true;
}

int main(void)
{
	struct v3d_clock_ops clk = { .set = clock_set, .ctx = NULL };
	const char *stop = NULL;
	uint32_t hz = 0;
	int rc;

	BUILD_ASSERT(RPI5_CUBE_CUBE_ATTRS == 2u);

	printk("rpi5_cube_ours: build %s, shaders ours (our 7.1 online compiler, src/shaders_ours.h), "
	       "job v3d_job71\n",
	       RPI5_CUBE_BUILD);
	printk("rpi5_cube_ours: cache: %s, data line %u bytes\n", CACHE_OPT,
	       (unsigned int)sys_cache_data_line_size_get());
	printk("rpi5_cube_ours: %s, V3D 7.1, C1 cube %ux%u LINEAR seamless (%u samples), C2 cube %ux%u level 1 "
	       "by the TFU, target %ux%u raster RGBA8888, watchdog armed at boot\n",
	       CONFIG_BOARD, CUBE_SIZE, CUBE_SIZE, C1_SAMPLES, CUBE_SIZE, CUBE_SIZE, TGT_W, TGT_H);

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
	/* VPM_SIZE is the core's IDENT1 field (bits 31:28, 8 KiB units as Mesa
	 * reads it, v3d_regs.h); the hub's IDENT1 word is printed beside it */
	printk("rpi5_cube_ours: IDENT1 VPM_SIZE %u (%u bytes) CTL_IDENT1 0x%08x HUB_IDENT1 0x%08x\n",
	       V3D_CTL_0_IDENT1_VPM_SIZE_GET(id.ctl_ident[1]), V3D_CTL_0_IDENT1_VPM_SIZE_GET(id.ctl_ident[1]) * 8192u,
	       id.ctl_ident[1], id.hub_ident[1]);
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
	va_cube = gpu_map(cube, CUBE_BYTES);
	va_mip = gpu_map(mip, MIP_BYTES);
	va_recs = gpu_map(recs, sizeof(recs));
	if (!va_cl[0] || !va_cl[1] || !va_cl[2] || !va_code || !va_vtx || !va_ta || !va_ts || !va_ov ||
	    !va_tgt || !va_cube || !va_mip || !va_recs) {
		stop = "M2 page table";
		goto out;
	}
	uint64_t pt_pa = k_mem_phys_addr(pt), scratch_pa = k_mem_phys_addr(scratch);
	printk("M2 page table: VA 0x%08x..0x%08x mapped; cube 0x%08x, mip cube 0x%08x, records 0x%08x\n", VA_FIRST,
	       va_next, va_cube, va_mip, va_recs);
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

	/* C1, then C2; a GPU stop in C1 ends the run before C2 */
	if (!c1_run()) {
		stop = why;
		goto out;
	}
	if (!c2_run()) {
		stop = why;
		goto out;
	}

out:
	/* every verdict line on every path; a part that did not run counts
	 * every px wrong */
	{
		const uint32_t all[6] = { FACE_PX, FACE_PX, FACE_PX, FACE_PX, FACE_PX, FACE_PX };

		cube_verdict(all, "neither", C1_PX);
	}
	tfu_verdict(C2_PX, stop ? stop : "did not run");
	if (stop)
		printk("rpi5_cube_ours: FAIL (%s)\n", stop);
	else
		printk("rpi5_cube_ours: done (C1 and C2 ran)\n");
	/* nothing more: the watchdog resets the SoC, tryboot brings Linux back */
	for (;;)
		k_sleep(K_SECONDS(10));
	return 0;
}
