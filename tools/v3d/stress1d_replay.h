/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * stress1d_replay.h - the carve-out allocations of the cluster sample's
 * STRESS1D run (records/pi4/record_STRESS1D_cluster_stress1d.txt, build
 * 526eb79c, 2026-09-26), replayed through src/osal/v3d_granules.h on the
 * host, so an allocator policy is judged by the free runs and the longest
 * run it leaves at each step. osal_granules_check.c includes it.
 *
 * What is replayed, and where each piece comes from:
 * - The carve-out: 48 MiB of 256-byte granules, the allocator's range
 *   starting past the 512 KiB page table and the scratch page (zephyr_v3d.c;
 *   the record's total 49,803,264). A block of a page or more is page
 *   aligned, a smaller one granule aligned (z_gpu_alloc).
 * - Step 1's start: the record's 20,971,520 bytes used as one block from
 *   the bottom with small holes, then the check frame's 64x64 texture
 *   (16,384 bytes), its tiled copy (20,480) and its vertex buffer (4,896),
 *   placed by the policy under test. The layout below them is not known.
 * - The steps (driver/samples/zephyr/v3d_gl_cluster/rust/src/stress.rs, deleted
 *   with v3d_gl_cluster in Step 5; in git history before that, on
 *   glue/stress1c): quads (vertex buffer only), layers (one 1920x720 RGBA
 *   texture: 5,529,600 bytes of storage and a 5,533,696-byte tiled copy),
 *   glyphs (runs of 20 cells, a pair of 240x20 textures per run: 19,200
 *   bytes of storage and a 28,672-byte tiled copy each; a tenth of the runs
 *   refill the texture the frame draws), uploads (256x256 textures,
 *   262,144 + a 266,240-byte copy, one refilled a frame). Frames per step:
 *   the record's fps x 5 s.
 * - The GL core and the backend (src/v3d/backend.rs at 526eb79c, the same
 *   rules on main): a whole-texture upload makes the tiled copy at once
 *   (v3d_texture_uploaded) and frees the texture's older copy; a draw of a
 *   texture with no copy makes one; at most 256 copies and 16 MiB of them,
 *   the least recently sampled given up first, and a copy sampled by the
 *   last two jobs is never given up for a newcomer (tiled into the job's
 *   arena instead, no allocation). femtovg's glBufferData each frame of
 *   (draws + 1) x 96 bytes takes fresh storage while the last job may read
 *   the old one (src/gl/buffer.rs).
 * - Frees (src/osal/v3d_frees.h): with a job open they wait for that job,
 *   with none open for the last one submitted, and go back once it has
 *   completed. A job completes at the next submit, or earlier when a
 *   kick_pending poll (glBufferData's) finds its render over: when the
 *   step's recorded render time is below its CPU draw time. Each step
 *   starts with every job completed (the sample's step report).
 * Fidelity (osal_granules_check.c pins the numbers): see RP_FIDELITY there.
 * Not replayed: allocations of the GL core and FemtoVG outside these
 * (programs, uniforms, scratch), and where the policy under test would
 * have put the 20 MiB present at step 1 (the same block for every policy).
 */
#ifndef STRESS1D_REPLAY_H
#define STRESS1D_REPLAY_H

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

#include "v3d_frees.h" /* v3d_free_runs */
#include "v3d_granules.h"

#define RP_GRANULE 256u
#define RP_N       (50331648u / RP_GRANULE)
#define RP_FIRST   ((512u * 1024 + 4096) / RP_GRANULE)
#define RP_STEPS   14

/* What a step leaves: bytes used and the high-water mark, the free runs
 * and the longest, the tiled copies kept. */
struct rp_step {
	uint32_t used, high, runs, longest, copies;
};

struct rp_result {
	struct rp_step step[RP_STEPS];
	uint32_t finds, reads_max;
	uint64_t reads;
	unsigned refused_alloc; /* allocations nothing fitted: must stay 0 */
	unsigned queue_full;    /* the replay's own free queue overflowed: must stay 0 */
};

struct rp_blk {
	uint32_t g, n; /* n == 0: none */
};

#define RP_TEXTURES  512
#define RP_ENTRIES   256                 /* backend.rs TILED_ENTRIES */
#define RP_BUDGET    (16u * 1024 * 1024) /* the record's "tiled copies kept up to" */
#define RP_TEX_ARENA (8u * 1024 * 1024)  /* backend.rs TEX_BYTES */
#define RP_HELD      4096

static struct {
	uint32_t map[RP_N / 32];
	struct v3d_granules gm;
	struct {
		struct rp_blk b;
		uint32_t fence;
	} held[RP_HELD];
	unsigned nheld;
	uint32_t submitted, completed;
	bool job_open;
	struct {
		uint32_t gen, image; /* content stamp, tiled image bytes */
	} tex[RP_TEXTURES];
	int ntex;
	struct {
		int tex; /* -1: empty */
		uint32_t gen, bytes, last;
		struct rp_blk mem;
	} copy[RP_ENTRIES];
	uint32_t copy_bytes, kicks;
	struct rp_blk vbo;
	uint32_t vbo_last;
	uint32_t frame;
	int quad, layer, runs[256][2], nruns, ups[2], nups;
	struct rp_result *out;
} rp;

static void rp_drain(void)
{
	unsigned j = 0;
	for (unsigned i = 0; i < rp.nheld; i++) {
		if (rp.held[i].fence <= rp.completed)
			v3d_granules_mark(&rp.gm, rp.held[i].b.g, rp.held[i].b.n, false);
		else
			rp.held[j++] = rp.held[i];
	}
	rp.nheld = j;
}

/* the job in flight has completed (a submit, a poll that found it over) */
static void rp_complete(void)
{
	rp.completed = rp.submitted;
	rp_drain();
}

static struct rp_blk rp_alloc(uint32_t bytes)
{
	uint32_t n = (bytes + RP_GRANULE - 1) / RP_GRANULE, align = bytes >= 4096 ? 4096 / RP_GRANULE : 1;
	int64_t g = v3d_granules_place(&rp.gm, n, align);
	if (g < 0) { /* z_gpu_alloc: finish the job in flight, drain, once more */
		rp_complete();
		g = v3d_granules_place(&rp.gm, n, align);
	}
	if (g < 0) {
		rp.out->refused_alloc++;
		return (struct rp_blk){ 0, 0 };
	}
	v3d_granules_mark(&rp.gm, (uint32_t)g, n, true);
	return (struct rp_blk){ (uint32_t)g, n };
}

static void rp_free(struct rp_blk b)
{
	if (!b.n)
		return;
	rp_drain();
	uint32_t fence = rp.job_open ? rp.submitted + 1 : rp.submitted;
	if (fence <= rp.completed) {
		v3d_granules_mark(&rp.gm, b.g, b.n, false);
		return;
	}
	if (rp.nheld == RP_HELD) {
		rp.out->queue_full++;
		return;
	}
	rp.held[rp.nheld].b = b;
	rp.held[rp.nheld].fence = fence;
	rp.nheld++;
}

static void rp_free_copy(int i)
{
	rp.copy_bytes -= rp.copy[i].bytes;
	rp_free(rp.copy[i].mem);
	rp.copy[i].tex = -1;
}

/* backend.rs tiled_copy: found, or made (giving up the least recently
 * sampled copy for room, never one the last two jobs sampled) */
static void rp_tiled_copy(int t)
{
	const uint32_t now = rp.kicks + 1;
	for (int i = 0; i < RP_ENTRIES; i++)
		if (rp.copy[i].tex == t && rp.copy[i].gen == rp.tex[t].gen) {
			rp.copy[i].last = now;
			return;
		}
	for (int i = 0; i < RP_ENTRIES; i++) /* another stamp of the same storage */
		if (rp.copy[i].tex == t)
			rp_free_copy(i);
	const uint32_t bytes = rp.tex[t].image + 4096;
	for (;;) {
		int slot = -1, victim = -1;
		for (int i = 0; i < RP_ENTRIES; i++) {
			if (rp.copy[i].tex < 0) {
				if (slot < 0)
					slot = i;
				continue;
			}
			int32_t a = (int32_t)(rp.copy[i].last - now), b = 0;
			if (victim >= 0)
				b = (int32_t)(rp.copy[victim].last - now);
			if (victim < 0 || a < b || (a == b && rp.copy[i].bytes < rp.copy[victim].bytes))
				victim = i;
		}
		if (slot >= 0 && rp.copy_bytes + bytes <= RP_BUDGET) {
			struct rp_blk m = rp_alloc(bytes);
			if (!m.n)
				return;
			rp.copy[slot].tex = t;
			rp.copy[slot].gen = rp.tex[t].gen;
			rp.copy[slot].bytes = bytes;
			rp.copy[slot].last = now;
			rp.copy[slot].mem = m;
			rp.copy_bytes += bytes;
			return;
		}
		if ((int32_t)(now - rp.copy[victim].last) <= 1 && bytes + 64 <= RP_TEX_ARENA)
			return; /* the working set: tiled into the job's arena */
		rp_free_copy(victim);
	}
}

static int rp_texture(uint32_t storage, uint32_t image)
{
	int t = rp.ntex++;
	(void)rp_alloc(storage); /* glTexImage2D(NULL): kept to the end */
	rp.tex[t].gen = 1;
	rp.tex[t].image = image;
	rp_tiled_copy(t); /* glTexSubImage2D of the whole: the copy at once */
	return t;
}

static void rp_refill(int t)
{
	rp.tex[t].gen++;
	rp_tiled_copy(t);
}

static void rp_frame(int arm, uint32_t n, bool over_at_poll)
{
	int drawn[160], nd = 0;
	uint32_t draws = 1;
	rp.job_open = false;
	if (arm == 0) { /* quads */
		draws = n;
		drawn[nd++] = rp.quad;
	} else if (arm == 1) { /* layers */
		if (rp.layer < 0)
			rp.layer = rp_texture(1920u * 720 * 4, 1920u * 720 * 4);
		draws = n;
		drawn[nd++] = rp.layer;
	} else if (arm == 2) { /* glyphs */
		const int nr = (int)((n + 19) / 20);
		while (rp.nruns < nr) {
			rp.runs[rp.nruns][0] = rp_texture(240 * 20 * 4, 24576);
			rp.runs[rp.nruns][1] = rp_texture(240 * 20 * 4, 24576);
			rp.nruns++;
		}
		const uint32_t parity = rp.frame & 1;
		for (int r = 0; r < nr; r++) {
			if (((uint32_t)r + rp.frame) % 10 == 0)
				rp_refill(rp.runs[r][parity]);
			drawn[nd++] = rp.runs[r][parity];
		}
		draws = (uint32_t)nr;
	} else { /* uploads */
		while (rp.nups < (int)n)
			rp.ups[rp.nups++] = rp_texture(256 * 256 * 4, 256 * 256 * 4);
		const int k = (int)(rp.frame % n);
		rp_refill(rp.ups[k]);
		drawn[nd++] = rp.ups[k];
	}
	/* the flush: glBufferData polls the last job first */
	if (over_at_poll)
		rp_complete();
	const uint32_t vb = (draws + 1) * 96;
	if (rp.vbo.n * RP_GRANULE < vb || rp.vbo_last > rp.completed) {
		rp_free(rp.vbo);
		rp.vbo = rp_alloc(vb);
	}
	rp.vbo_last = rp.submitted + 1;
	rp.job_open = true; /* the first draw opens the job */
	for (int i = 0; i < nd; i++)
		rp_tiled_copy(drawn[i]);
	rp.job_open = false; /* submit: the job in flight completes first */
	rp_complete();
	rp.submitted++;
	rp.kicks++;
	rp.frame++;
}

static void rp_snapshot(struct rp_step *st)
{
	int copies = 0;
	for (int i = 0; i < RP_ENTRIES; i++)
		copies += rp.copy[i].tex >= 0;
	v3d_free_runs(rp.map, RP_FIRST, RP_N, &st->longest, &st->runs);
	st->longest *= RP_GRANULE;
	st->used = rp.gm.used * RP_GRANULE;
	st->high = rp.gm.high * RP_GRANULE;
	st->copies = (uint32_t)copies;
}

/* STRESS1D's 14 steps with blocks below `top_below` bytes from the top (0 =
 * first fit for all); `print` prints a line a step. */
static void stress1d_replay(uint32_t top_below, bool print, struct rp_result *out)
{
	/* arm, units, frames (the record's fps x 5 s), CPU draw and render us */
	static const struct {
		int arm;
		uint32_t n, frames, draw_us, render_us;
	} steps[RP_STEPS] = {
		{ 0, 50, 2431, 1121, 1427 },    { 0, 200, 1003, 4300, 4310 },   { 0, 500, 392, 10617, 10574 },
		{ 0, 1000, 223, 21401, 21353 }, { 1, 1, 1236, 546, 3390 },      { 1, 2, 768, 569, 5806 },
		{ 1, 4, 436, 612, 10677 },      { 1, 6, 303, 654, 15535 },      { 1, 8, 233, 697, 20397 },
		{ 2, 200, 2794, 962, 1152 },    { 2, 1000, 1710, 2261, 2265 }, { 2, 3000, 497, 9389, 3656 },
		{ 3, 1, 1884, 2015, 1140 },     { 3, 2, 2144, 1692, 1653 },
	};
	static const char *const arm_name[4] = { "quads", "layers", "glyphs", "uploads" };

	*out = (struct rp_result){ 0 };
	rp.out = out;
	rp.nheld = 0;
	rp.submitted = rp.completed = 0;
	rp.job_open = false;
	rp.ntex = rp.nruns = rp.nups = 0;
	rp.copy_bytes = rp.kicks = rp.frame = 0;
	rp.vbo = (struct rp_blk){ 0, 0 };
	rp.vbo_last = 0;
	rp.layer = -1;
	for (int i = 0; i < RP_ENTRIES; i++)
		rp.copy[i].tex = -1;
	v3d_granules_init(&rp.gm, rp.map, RP_FIRST, RP_N);
	v3d_granules_top_below(&rp.gm, top_below / RP_GRANULE);

	/* step 1's start: 20,971,520 bytes used; below the check frame's three
	 * blocks (164 granules), one block from `first` with four holes of 12
	 * granules (the record: 5 free runs) */
	const uint32_t below = 20971520 / RP_GRANULE - 64 - 80 - 20;
	v3d_granules_mark(&rp.gm, RP_FIRST, below + 48, true);
	for (uint32_t h = 1; h <= 4; h++)
		v3d_granules_mark(&rp.gm, RP_FIRST + 20000 * h, 12, false);
	rp.quad = rp_texture(64 * 64 * 4, 64 * 64 * 4);
	rp.vbo = rp_alloc(51 * 96);
	rp.vbo_last = 0;

	for (int s = 0; s < RP_STEPS; s++) {
		rp_complete(); /* the step report: every job completed */
		const bool over_at_poll = steps[s].render_us < steps[s].draw_us;
		for (uint32_t f = 0; f < steps[s].frames; f++)
			rp_frame(steps[s].arm, steps[s].n, over_at_poll);
		struct rp_step *st = &out->step[s];
		rp_snapshot(st);
		if (print)
			printf("stress1d_replay (top_below %u): step %2d %-7s N=%-4u used %8u high %8u; free runs %3u, "
			       "longest %8u; copies %3u\n",
			       top_below, s + 1, arm_name[steps[s].arm], steps[s].n, st->used, st->high, st->runs,
			       st->longest, st->copies);
	}
	out->finds = rp.gm.finds;
	out->reads = rp.gm.reads;
	out->reads_max = rp.gm.reads_max;
	if (print)
		printf("stress1d_replay (top_below %u): find %u calls, %llu map reads (%llu a call, at most %u in one); "
		       "%u refused, %u free-queue overflows\n",
		       top_below, out->finds, (unsigned long long)out->reads,
		       (unsigned long long)(out->finds ? out->reads / out->finds : 0), out->reads_max,
		       out->refused_alloc, out->queue_full);
}

#endif
