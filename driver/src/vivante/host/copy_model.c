/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * copy_model.c: gc_draw_copy() and gc_draw_clear() (gc_draw.c and gc_cmd.c,
 * compiled unchanged against shim/) with the render service replaced by a
 * model of the resolve engine. gc_render_alloc() hands out blocks of a heap
 * "carveout" at the STM32MP257F-DK's gpu-reserved base (0xfa800000);
 * gc_render_submit() walks the caller's stream (LOAD_STATE, NOP, STALL,
 * DRAW), keeps the states by address and runs each RS_KICKER: the fill of
 * RS_FILL_VALUE(0) when RS_CLEAR_CONTROL's mode is ENABLED1, otherwise a
 * copy of RS_WINDOW_SIZE pixels from RS_PIPE_SOURCE_ADDR(0) at
 * RS_SOURCE_STRIDE to RS_PIPE_DEST_ADDR(0) at RS_DEST_STRIDE. A kick whose
 * states the model does not know (a TILED bit, a stride flag, another
 * format, a pipe offset, no single-buffer bracket, a width not a multiple
 * of 16, an address outside the carveout) fails the run.
 *
 * The register addresses are repeated below from rnndb's state_3d.xml.h /
 * state.xml.h on purpose, so a wrong constant in the driver cannot agree
 * with itself.
 *
 * The cases (gc_triangle's CONFIG_GC_TRIANGLE_LINEAR_OUT): a 256x128 target
 * filled with a per-pixel pattern copied into a buffer of its own size, into
 * a 1280x720 scan-out buffer, and into one whose pitch is padded past the
 * width; every pixel of the window must match the source and every pixel
 * outside it (and the guard rows) must keep the poison. Then the refusals:
 * a width not a multiple of 16, a height not a multiple of 4, a destination
 * smaller than the source, an unaligned address or pitch, each -EINVAL with
 * nothing submitted. Prints one line a case; exits 1 on a failure.
 *
 *   copy.sh [outdir]
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gc_internal.h"
#include "gc_draw.h"

/* --- the shim's log --- */
unsigned int model_log_errors;
bool model_log_quiet = true;

/* --- the GPU's registers this model knows --- */
#define FE_OP_MASK 0xf8000000u
#define FE_OP_LOAD_STATE 0x08000000u
#define FE_OP_END 0x10000000u
#define FE_OP_NOP 0x18000000u
#define FE_OP_DRAW_INSTANCED 0x60000000u
#define FE_OP_STALL 0x48000000u

#define GL_FLUSH_CACHE 0x380c
#define TS_MEM_CONFIG 0x1654
#define RS_CONFIG 0x1604
#define RS_SOURCE_STRIDE 0x160c
#define RS_DEST_STRIDE 0x1614
#define RS_WINDOW_SIZE 0x1620
#define RS_PIPE_SOURCE_ADDR0 0x16c0
#define RS_PIPE_DEST_ADDR0 0x16e0
#define RS_PIPE_OFFSET0 0x1700
#define RS_PIPE_OFFSET1 0x1704
#define RS_CLEAR_CONTROL 0x163c
#define RS_FILL_VALUE0 0x1640
#define RS_EXTRA_CONFIG 0x16a0
#define RS_KICKER 0x1600
#define RS_SINGLE_BUFFER 0x16b8

#define RS_FORMAT_A8R8G8B8 0x06u
#define RS_CONFIG_TILED_BITS (0x80u | 0x4000u) /* SOURCE_TILED, DEST_TILED */

#define CARVEOUT_PHYS 0xfa800000u
#define CARVEOUT_SIZE (16u << 20)

static uint8_t *s_carveout;
static size_t s_bump;
static unsigned int s_submits, s_kicks;
static int s_failures;

/* A state the stream wrote, and whether it wrote it since the last kick. */
static uint32_t s_state[0x10000 / 4];
static bool s_flushed_color_depth, s_ts_off;

#define MODEL_FAIL(...)                                                                            \
	do {                                                                                       \
		printf("  FAIL: ");                                                                \
		printf(__VA_ARGS__);                                                               \
		printf("\n");                                                                      \
		s_failures++;                                                                      \
	} while (0)

static uint8_t *cpu_of(uint32_t gpu, size_t bytes)
{
	if (gpu < CARVEOUT_PHYS || gpu - CARVEOUT_PHYS > CARVEOUT_SIZE - bytes) {
		return NULL;
	}
	return s_carveout + (gpu - CARVEOUT_PHYS);
}

/* --- the render service, as gc_draw.c sees it --- */
int gc_render_alloc(size_t size, size_t align, struct gc_bo *out)
{
	size_t a = align != 0 ? align : GC_ALIGN_DEFAULT;
	size_t at = ROUND_UP(s_bump, a);

	if (at + size > CARVEOUT_SIZE) {
		return -ENOMEM;
	}
	s_bump = at + size;
	memset(s_carveout + at, 0, size);
	*out = (struct gc_bo){ .cpu = s_carveout + at,
			       .gpu = CARVEOUT_PHYS + (uint32_t)at,
			       .phys = CARVEOUT_PHYS + at,
			       .size = size };
	return 0;
}

void gc_render_cache_invd(void *cpu, size_t n)
{
	ARG_UNUSED(cpu);
	ARG_UNUSED(n);
}

static uint32_t st(uint32_t reg)
{
	return s_state[reg / 4];
}

static void rs_kick(void)
{
	uint32_t cfg = st(RS_CONFIG), win = st(RS_WINDOW_SIZE);
	uint32_t w = win & 0xffffu, h = win >> 16;
	uint32_t dstride = st(RS_DEST_STRIDE), daddr = st(RS_PIPE_DEST_ADDR0);
	bool clear = (st(RS_CLEAR_CONTROL) & 0x00030000u) == 0x00010000u;
	uint8_t *d = cpu_of(daddr, (size_t)dstride * (h - 1) + w * 4u);

	s_kicks++;
	if (!s_flushed_color_depth) {
		MODEL_FAIL("kick without the colour and depth flush before it");
	}
	if (st(RS_SINGLE_BUFFER) != 1u) {
		MODEL_FAIL("kick outside the single-buffer bracket");
	}
	if ((cfg & 0x1fu) != RS_FORMAT_A8R8G8B8 || ((cfg >> 8) & 0x1fu) != RS_FORMAT_A8R8G8B8 ||
	    (cfg & ~(0x1fu | 0x1f00u)) != 0u) {
		MODEL_FAIL("RS_CONFIG 0x%08x: not A8R8G8B8 linear to linear", cfg);
	}
	if (cfg & RS_CONFIG_TILED_BITS) {
		MODEL_FAIL("RS_CONFIG 0x%08x has a TILED bit", cfg);
	}
	if (st(RS_PIPE_OFFSET0) || st(RS_PIPE_OFFSET1) || st(RS_EXTRA_CONFIG)) {
		MODEL_FAIL("pipe offsets or extra config not zero");
	}
	if (w == 0 || h == 0 || (w & 15u)) {
		MODEL_FAIL("window %ux%u: the width must be a non-zero multiple of 16", w, h);
		return;
	}
	if (dstride & ~0xfffffu || dstride < w * 4u || d == NULL) {
		MODEL_FAIL("destination 0x%08x stride 0x%x outside the carveout", daddr, dstride);
		return;
	}
	if (clear) {
		for (uint32_t y = 0; y < h; y++) {
			for (uint32_t x = 0; x < w; x++) {
				memcpy(d + (size_t)y * dstride + x * 4u, &s_state[RS_FILL_VALUE0 / 4], 4);
			}
		}
		return;
	}
	if (st(RS_CLEAR_CONTROL) != 0u) {
		MODEL_FAIL("a copy with RS_CLEAR_CONTROL 0x%08x", st(RS_CLEAR_CONTROL));
	}
	if (!s_ts_off) {
		MODEL_FAIL("a copy without TS_MEM_CONFIG 0 before it");
	}
	uint32_t sstride = st(RS_SOURCE_STRIDE), saddr = st(RS_PIPE_SOURCE_ADDR0);
	const uint8_t *s = cpu_of(saddr, (size_t)sstride * (h - 1) + w * 4u);

	if (sstride & ~0xfffffu || sstride < w * 4u || s == NULL) {
		MODEL_FAIL("source 0x%08x stride 0x%x outside the carveout", saddr, sstride);
		return;
	}
	for (uint32_t y = 0; y < h; y++) {
		memmove(d + (size_t)y * dstride, s + (size_t)y * sstride, (size_t)w * 4u);
	}
}

int gc_render_submit(const struct gc_submit *sub, uint32_t *fence_out)
{
	const uint32_t *p = (const uint32_t *)cpu_of(sub->cmd_gpu, sub->cmd_bytes);
	uint32_t n = sub->cmd_bytes / 4, i = 0;

	if (p == NULL || (sub->cmd_bytes & 7u)) {
		MODEL_FAIL("submit 0x%08x+%u", sub->cmd_gpu, sub->cmd_bytes);
		return -EINVAL;
	}
	s_flushed_color_depth = false;
	s_ts_off = false;
	while (i < n) {
		uint32_t hdr = p[i];

		switch (hdr & FE_OP_MASK) {
		case FE_OP_LOAD_STATE: {
			uint32_t count = (hdr >> 16) & 0x3ffu, reg = (hdr & 0xffffu) << 2;

			for (uint32_t k = 0; k < count; k++) {
				uint32_t r = reg + 4u * k, v = p[i + 1 + k];

				s_state[r / 4] = v;
				if (r == GL_FLUSH_CACHE && (v & 3u) == 3u) {
					s_flushed_color_depth = true;
				}
				if (r == TS_MEM_CONFIG && v == 0u) {
					s_ts_off = true;
				}
				if (r == RS_KICKER) {
					rs_kick();
				}
			}
			i += (1 + count + 1) & ~1u;
			break;
		}
		case FE_OP_NOP:
			i += (hdr == FE_OP_NOP && (i & 1u)) ? 1 : 2;
			break;
		case FE_OP_STALL:
			i += 2;
			break;
		case FE_OP_DRAW_INSTANCED:
			i += 4;
			break;
		default:
			MODEL_FAIL("unknown command 0x%08x at dword %u", hdr, i);
			return -EINVAL;
		}
	}
	*fence_out = ++s_submits;
	return 0;
}

/* --- the cases --- */
#define POISON 0x00a55a3cu
#define GUARD_ROWS 2u

static struct gc_target target(uint32_t w, uint32_t h, uint32_t stride)
{
	struct gc_bo bo;
	struct gc_target t = { .width = w, .height = h, .stride = stride };

	if (gc_render_alloc((size_t)stride * (h + GUARD_ROWS), 64, &bo)) {
		printf("  carveout full\n");
		exit(1);
	}
	t.gpu = bo.gpu;
	t.cpu = bo.cpu;
	for (uint32_t i = 0; i < stride / 4u * (h + GUARD_ROWS); i++) {
		((uint32_t *)t.cpu)[i] = POISON;
	}
	return t;
}

static uint32_t *px(const struct gc_target *t, uint32_t x, uint32_t y)
{
	return (uint32_t *)((uint8_t *)t->cpu + (size_t)y * t->stride) + x;
}

/* A target of 256x128 at stride 1024 cleared by the RS, a pattern drawn on
 * the CPU (the stand-in for the triangle), copied into `ow` x `oh` at
 * `pitch`. */
static void copy_case(uint32_t ow, uint32_t oh, uint32_t pitch)
{
	const int before = s_failures;
	struct gc_target src = target(256, 128, 1024);
	struct gc_target dst = target(ow, oh, pitch);
	uint32_t fence = 0, kicks = s_kicks, wrong = 0, outside = 0;
	int rc;

	rc = gc_draw_clear(&src, 0x00102030u, &fence);
	if (rc || s_kicks != kicks + 1 || *px(&src, 255, 127) != 0x00102030u ||
	    *px(&src, 0, 128) != POISON) {
		MODEL_FAIL("the clear: rc %d, kicks %u", rc, s_kicks - kicks);
	}
	for (uint32_t y = 32; y < 96; y++) {
		for (uint32_t x = 64; x < 192; x++) {
			*px(&src, x, y) = (y << 12) ^ (x * 0x9e37u);
		}
	}
	rc = gc_draw_copy(&src, &dst, &fence);
	if (rc || s_kicks != kicks + 2) {
		MODEL_FAIL("the copy: rc %d, kicks %u", rc, s_kicks - kicks);
	}
	for (uint32_t y = 0; y < oh + GUARD_ROWS; y++) {
		for (uint32_t x = 0; x < pitch / 4u; x++) {
			uint32_t v = *px(&dst, x, y);

			if (x < src.width && y < src.height) {
				wrong += v != *px(&src, x, y);
			} else {
				outside += v != POISON;
			}
		}
	}
	if (wrong || outside) {
		MODEL_FAIL("%u pixels of the window differ from the source, %u outside it lost the poison",
			   wrong, outside);
	}
	printf("%s copy 256x128 stride 1024 -> %ux%u pitch %u at 0x%08x\n",
	       s_failures == before ? "PASS" : "FAIL", ow, oh, pitch, dst.gpu);
}

static void refuse_case(const char *what, struct gc_target src, struct gc_target dst)
{
	const int before = s_failures;
	uint32_t fence = 0, submits = s_submits;
	int rc = gc_draw_copy(&src, &dst, &fence);

	if (rc != -EINVAL || s_submits != submits) {
		MODEL_FAIL("rc %d, %u submits", rc, s_submits - submits);
	}
	printf("%s refused: %s\n", s_failures == before ? "PASS" : "FAIL", what);
}

int main(void)
{
	s_carveout = aligned_alloc(4096, CARVEOUT_SIZE);
	if (s_carveout == NULL) {
		return 1;
	}

	copy_case(256, 128, 1024);
	copy_case(1280, 720, 5120);
	copy_case(272, 132, 1152);

	struct gc_target a = target(256, 128, 1024);
	struct gc_target b = target(256, 128, 1024);
	struct gc_target t;

	t = a;
	t.width = 248;
	refuse_case("source width 248, not a multiple of 16", t, b);
	t = a;
	t.height = 126;
	refuse_case("source height 126, not a multiple of 4", t, b);
	t = b;
	t.height = 64;
	refuse_case("destination 256x64 smaller than the source", a, t);
	t = b;
	t.gpu += 32;
	refuse_case("destination address not 64-byte aligned", a, t);
	t = b;
	t.stride = 1056 + 4;
	refuse_case("destination pitch not a multiple of 64", a, t);

	printf("copy_model: %s (%d failures, %u submits, %u kicks)\n",
	       s_failures ? "FAIL" : "PASS", s_failures, s_submits, s_kicks);
	free(s_carveout);
	return s_failures ? 1 : 0;
}
